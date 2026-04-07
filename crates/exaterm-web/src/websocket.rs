use crate::routes::AppState;
use axum::extract::ws::{Message, WebSocket};
use axum::extract::{Path, State, WebSocketUpgrade};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use exaterm_core::daemon::connect_session_stream_socket;
use exaterm_types::proto::{ClientMessage, ServerMessage};
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
    // Allow localhost/127.0.0.1 on any port.
    let origin_lower = origin.to_lowercase();
    origin_lower.starts_with("http://localhost")
        || origin_lower.starts_with("https://localhost")
        || origin_lower.starts_with("http://127.0.0.1")
        || origin_lower.starts_with("https://127.0.0.1")
        || origin_lower.starts_with("http://[::1]")
        || origin_lower.starts_with("https://[::1]")
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
    // Send current snapshot immediately.
    let snapshot = relay.snapshot();
    let msg = ServerMessage::WorkspaceSnapshot { snapshot };
    if let Ok(json) = serde_json::to_string(&msg) {
        if socket.send(Message::Text(json.into())).await.is_err() {
            return;
        }
    }

    let mut snapshots = relay.snapshots.clone();

    loop {
        tokio::select! {
            result = snapshots.changed() => {
                if result.is_err() {
                    break;
                }
                let snapshot = snapshots.borrow_and_update().clone();
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
                            Ok(cmd) => { let _ = relay.commands.send(cmd).await; }
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
    socket: WebSocket,
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
