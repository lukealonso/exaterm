use crate::routes::AppState;
use axum::extract::ws::{Message, WebSocket};
use axum::extract::{Path, State, WebSocketUpgrade};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::IntoResponse;
use exaterm_core::daemon::connect_session_stream_socket;
use exaterm_types::proto::{ClientMessage, ServerMessage, WorkspaceSnapshot};
use futures::{SinkExt, StreamExt};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Reject WebSocket upgrades from cross-origin pages.
fn origin_allowed(headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get("origin").and_then(|v| v.to_str().ok()) else {
        // No Origin header means a non-browser client (curl, etc.) which is
        // not subject to cross-origin restrictions. This is safe because the
        // web UI is designed for localhost use and any client that can reach
        // the socket already has local access.
        return true;
    };
    // Parse as a URI and match the host component exactly to prevent
    // prefix-based bypasses (e.g. localhost.evil.com).
    let Ok(uri) = origin.parse::<Uri>() else {
        return false;
    };
    matches!(uri.scheme_str(), Some("http" | "https"))
        && matches!(
            uri.host(),
            Some("localhost" | "127.0.0.1" | "[::1]" | "::1")
        )
}

/// Strip internal filesystem paths from the snapshot before sending to the
/// browser.  The frontend only uses `raw_stream_socket_name` as a truthy
/// check, so we replace the real path with a harmless placeholder.
fn sanitize_snapshot(snapshot: &WorkspaceSnapshot) -> WorkspaceSnapshot {
    let sessions = snapshot
        .sessions
        .iter()
        .map(|s| {
            let mut s = s.clone();
            if s.raw_stream_socket_name.is_some() {
                s.raw_stream_socket_name = Some("available".into());
            }
            s
        })
        .collect();
    WorkspaceSnapshot { sessions }
}

// --- Control WebSocket: JSON snapshot/command relay ---

pub async fn ws_control(
    headers: HeaderMap,
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    if !origin_allowed(&headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let relay = state.relay.clone();
    ws.on_upgrade(move |socket| handle_control(socket, relay)).into_response()
}

async fn handle_control(
    mut socket: WebSocket,
    relay: Arc<crate::relay::DaemonRelay>,
) {
    // Send the current snapshot from the same receiver we'll subscribe to,
    // so no update can be lost between the initial send and the change loop.
    let mut snapshots = relay.snapshots.clone();
    let snapshot = sanitize_snapshot(&snapshots.borrow_and_update().clone());
    let msg = ServerMessage::WorkspaceSnapshot { snapshot };
    if let Ok(json) = serde_json::to_string(&msg) {
        if socket.send(Message::Text(json.into())).await.is_err() {
            return;
        }
    }

    loop {
        tokio::select! {
            result = snapshots.changed() => {
                if result.is_err() {
                    break;
                }
                let snapshot = sanitize_snapshot(&snapshots.borrow_and_update().clone());
                let msg = ServerMessage::WorkspaceSnapshot { snapshot };
                if let Ok(json) = serde_json::to_string(&msg) {
                    if socket.send(Message::Text(json.into())).await.is_err() {
                        break;
                    }
                }
            }
            result = socket.recv() => {
                match result {
                    Some(Ok(Message::Text(text))) => {
                        match serde_json::from_str::<ClientMessage>(&text) {
                            Ok(cmd) => { let _ = relay.commands.send(cmd); }
                            Err(e) => eprintln!("invalid client message: {e}"),
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    _ => {}
                }
            }
        }
    }
}

// --- Stream WebSocket: raw PTY byte relay ---

pub async fn ws_stream(
    headers: HeaderMap,
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<u32>,
) -> impl IntoResponse {
    if !origin_allowed(&headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let relay = state.relay.clone();
    ws.on_upgrade(move |socket| handle_stream(socket, relay, session_id)).into_response()
}

async fn handle_stream(
    mut socket: WebSocket,
    relay: Arc<crate::relay::DaemonRelay>,
    session_id: u32,
) {
    // Retry connecting to the daemon's per-session socket. The socket may
    // not be ready yet if the session was just created.
    const MAX_CONNECT_ATTEMPTS: usize = 20;
    const CONNECT_RETRY_MS: u64 = 250;
    let mut unix_stream = None;
    for attempt in 0..MAX_CONNECT_ATTEMPTS {
        let socket_name = {
            let snapshot = relay.snapshot();
            snapshot
                .sessions
                .iter()
                .find(|s| s.record.id.0 == session_id)
                .and_then(|s| s.raw_stream_socket_name.clone())
        };
        if let Some(socket_name) = socket_name {
            match connect_session_stream_socket(&socket_name) {
                Ok(stream) => {
                    unix_stream = Some(stream);
                    break;
                }
                Err(_) if attempt < MAX_CONNECT_ATTEMPTS - 1 => {
                    tokio::time::sleep(std::time::Duration::from_millis(CONNECT_RETRY_MS)).await;
                }
                Err(_) => {}
            }
        } else if attempt < MAX_CONNECT_ATTEMPTS - 1 {
            tokio::time::sleep(std::time::Duration::from_millis(CONNECT_RETRY_MS)).await;
        }
    }
    let Some(unix_stream) = unix_stream else {
        let _ = socket
            .send(Message::Close(Some(axum::extract::ws::CloseFrame {
                code: 1011,
                reason: "failed to connect to session stream".into(),
            })))
            .await;
        return;
    };
    unix_stream.set_nonblocking(true).ok();
    let unix_stream = match tokio::net::UnixStream::from_std(unix_stream) {
        Ok(stream) => stream,
        Err(_) => return,
    };

    let (mut unix_reader, mut unix_writer) = unix_stream.into_split();
    let (mut ws_sink, mut ws_stream) = socket.split();

    // Daemon -> Browser: raw PTY output as binary frames.
    let to_browser = async move {
        let mut buf = [0u8; 8192];
        loop {
            match unix_reader.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if ws_sink
                        .send(Message::Binary(buf[..n].to_vec().into()))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
            }
        }
    };

    // Browser -> Daemon: keyboard input as raw bytes.
    let to_daemon = async move {
        while let Some(Ok(msg)) = ws_stream.next().await {
            match msg {
                Message::Binary(data) => {
                    if unix_writer.write_all(&data).await.is_err() {
                        break;
                    }
                }
                Message::Close(_) => break,
                _ => {}
            }
        }
    };

    tokio::select! {
        _ = to_browser => {}
        _ = to_daemon => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers_with_origin(origin: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert("origin", origin.parse().unwrap());
        headers
    }

    #[test]
    fn no_origin_header_is_allowed() {
        assert!(origin_allowed(&HeaderMap::new()));
    }

    #[test]
    fn localhost_http_is_allowed() {
        assert!(origin_allowed(&headers_with_origin("http://localhost")));
        assert!(origin_allowed(&headers_with_origin("http://localhost:9800")));
    }

    #[test]
    fn localhost_https_is_allowed() {
        assert!(origin_allowed(&headers_with_origin("https://localhost")));
        assert!(origin_allowed(&headers_with_origin("https://localhost:9800")));
    }

    #[test]
    fn loopback_ipv4_is_allowed() {
        assert!(origin_allowed(&headers_with_origin("http://127.0.0.1")));
        assert!(origin_allowed(&headers_with_origin("http://127.0.0.1:9800")));
        assert!(origin_allowed(&headers_with_origin("https://127.0.0.1:9800")));
    }

    #[test]
    fn loopback_ipv6_is_allowed() {
        assert!(origin_allowed(&headers_with_origin("http://[::1]")));
        assert!(origin_allowed(&headers_with_origin("http://[::1]:9800")));
        assert!(origin_allowed(&headers_with_origin("https://[::1]:9800")));
    }

    #[test]
    fn localhost_subdomain_is_rejected() {
        assert!(!origin_allowed(&headers_with_origin("http://localhost.evil.com")));
    }

    #[test]
    fn loopback_subdomain_is_rejected() {
        assert!(!origin_allowed(&headers_with_origin("http://127.0.0.1.evil.com")));
    }

    #[test]
    fn external_origin_is_rejected() {
        assert!(!origin_allowed(&headers_with_origin("http://evil.com")));
        assert!(!origin_allowed(&headers_with_origin("https://example.org")));
    }

    #[test]
    fn empty_origin_is_rejected() {
        assert!(!origin_allowed(&headers_with_origin("")));
    }

    #[test]
    fn sanitize_snapshot_replaces_socket_paths() {
        use exaterm_types::model::{SessionId, SessionRecord, SessionLaunch, SessionKind, SessionStatus};
        use exaterm_types::proto::{SessionSnapshot, ObservationSnapshot};

        let snapshot = WorkspaceSnapshot {
            sessions: vec![SessionSnapshot {
                record: SessionRecord {
                    id: SessionId(1),
                    launch: SessionLaunch {
                        name: "test".into(),
                        subtitle: "".into(),
                        program: "/bin/sh".into(),
                        args: vec![],
                        cwd: None,
                        env: vec![],
                        kind: SessionKind::WaitingShell,
                    },
                    display_name: None,
                    status: SessionStatus::Running,
                    pid: Some(1234),
                    events: vec![],
                },
                observation: ObservationSnapshot::default(),
                summary: None,
                raw_stream_socket_name: Some("/tmp/exaterm-abc/session-1.sock".into()),
                auto_nudge_enabled: false,
                last_nudge: None,
                last_sent_age_secs: None,
            }],
        };

        let sanitized = sanitize_snapshot(&snapshot);
        assert_eq!(sanitized.sessions.len(), 1);
        // Real path must be replaced with placeholder.
        assert_eq!(
            sanitized.sessions[0].raw_stream_socket_name.as_deref(),
            Some("available")
        );
    }

    #[test]
    fn sanitize_snapshot_preserves_none_socket() {
        let snapshot = WorkspaceSnapshot {
            sessions: vec![],
        };
        let sanitized = sanitize_snapshot(&snapshot);
        assert!(sanitized.sessions.is_empty());
    }
}
