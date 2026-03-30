use crate::terminal_state::TerminalState;
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::sync::mpsc;

use exaterm_core::daemon::connect_session_stream_socket;
use exaterm_types::model::SessionId;

/// Bidirectional I/O bridge for a single session's raw PTY stream.
pub struct SessionIO {
    output_rx: mpsc::Receiver<Vec<u8>>,
    input_writer: UnixStream,
    shutdown_stream: UnixStream,
    pub terminal: TerminalState,
}

impl SessionIO {
    /// Connect to a session's raw stream socket and spawn a background reader thread.
    pub fn connect(socket_name: &str, rows: u16, cols: u16) -> Result<Self, String> {
        let stream = connect_session_stream_socket(socket_name)?;
        let reader_stream = stream
            .try_clone()
            .map_err(|e| format!("clone stream: {e}"))?;
        let shutdown_stream = stream
            .try_clone()
            .map_err(|e| format!("clone stream for shutdown: {e}"))?;
        let output_rx = spawn_output_reader(reader_stream);
        Ok(Self {
            output_rx,
            input_writer: stream,
            shutdown_stream,
            terminal: TerminalState::new(rows, cols),
        })
    }

    /// Drain all pending output from the reader thread and feed it to the terminal emulator.
    /// Returns `true` if any bytes were consumed.
    pub fn drain_output(&mut self) -> bool {
        let mut changed = false;
        while let Ok(bytes) = self.output_rx.try_recv() {
            self.terminal.write_output(&bytes);
            changed = true;
        }
        changed
    }

    /// Send keyboard input bytes to the PTY.
    pub fn write_input(&mut self, bytes: &[u8]) {
        let _ = self.input_writer.write_all(bytes);
    }

    /// Extract the last `max_lines` non-empty trimmed lines from the terminal scrollback.
    pub fn scrollback_lines(&self, max_lines: usize) -> Vec<String> {
        let snap = self.terminal.grid_snapshot();
        scrollback_from_snapshot(&snap, max_lines)
    }

    /// Render the terminal grid as a plain-text string (one line per row, trimmed).
    pub fn render_grid_text(&self) -> String {
        let snap = self.terminal.grid_snapshot();
        snap.cells
            .iter()
            .map(|row| {
                let line: String = row.iter().map(|c| c.character).collect();
                line.trim_end().to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

impl Drop for SessionIO {
    fn drop(&mut self) {
        let _ = self.shutdown_stream.shutdown(Shutdown::Both);
    }
}

/// Extract the last `max_lines` non-empty trimmed lines from a grid snapshot.
pub fn scrollback_from_snapshot(
    snap: &crate::terminal_state::GridSnapshot,
    max_lines: usize,
) -> Vec<String> {
    let lines: Vec<String> = snap
        .cells
        .iter()
        .map(|row| {
            let line: String = row.iter().map(|c| c.character).collect();
            line.trim_end().to_string()
        })
        .collect();
    // Collect non-empty lines from the bottom.
    lines
        .into_iter()
        .rev()
        .filter(|l| !l.is_empty())
        .take(max_lines)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

/// Holds all active session I/O bridges, keyed by session id.
pub struct SessionIOMap {
    sessions: BTreeMap<SessionId, SessionIO>,
    /// Socket names for which we have already attempted (or completed) a connection.
    connected_sockets: BTreeMap<SessionId, String>,
}

impl SessionIOMap {
    pub fn new() -> Self {
        Self {
            sessions: BTreeMap::new(),
            connected_sockets: BTreeMap::new(),
        }
    }

    /// Attempt to connect any sessions whose raw socket name is known but not yet connected.
    pub fn connect_new_sessions(
        &mut self,
        raw_socket_names: &BTreeMap<SessionId, String>,
        rows: u16,
        cols: u16,
    ) {
        for (id, socket_name) in raw_socket_names {
            if self.connected_sockets.contains_key(id) {
                continue;
            }
            self.connected_sockets.insert(*id, socket_name.clone());
            match SessionIO::connect(socket_name, rows, cols) {
                Ok(session_io) => {
                    self.sessions.insert(*id, session_io);
                }
                Err(e) => {
                    eprintln!("exaterm: failed to connect session {}: {e}", id.0);
                }
            }
        }
    }

    /// Remove sessions that are no longer present.
    pub fn retain_sessions(&mut self, active_ids: &[SessionId]) {
        self.sessions.retain(|id, _| active_ids.contains(id));
        self.connected_sockets
            .retain(|id, _| active_ids.contains(id));
    }

    /// Drain output for all sessions. Returns true if any session had new output.
    pub fn drain_all_output(&mut self) -> bool {
        let mut changed = false;
        for session_io in self.sessions.values_mut() {
            if session_io.drain_output() {
                changed = true;
            }
        }
        changed
    }

    /// Get a mutable reference to a specific session's I/O bridge.
    pub fn get_mut(&mut self, id: &SessionId) -> Option<&mut SessionIO> {
        self.sessions.get_mut(id)
    }

    /// Get the first connected session id (for single-session display).
    pub fn first_session_id(&self) -> Option<SessionId> {
        self.sessions.keys().next().copied()
    }

    /// Render the first connected session's terminal grid as text.
    /// Falls back to the given default string if no sessions are connected.
    pub fn render_first_session(&self, default: &str) -> String {
        match self.sessions.values().next() {
            Some(session_io) => session_io.render_grid_text(),
            None => default.to_string(),
        }
    }

    /// Return a grid snapshot from the first connected session, if any.
    pub fn first_session_snapshot(&self) -> Option<crate::terminal_state::GridSnapshot> {
        self.sessions
            .values()
            .next()
            .map(|s| s.terminal.grid_snapshot())
    }

    /// Return a grid snapshot for a specific session, if connected.
    pub fn session_snapshot(&self, id: &SessionId) -> Option<crate::terminal_state::GridSnapshot> {
        self.sessions.get(id).map(|s| s.terminal.grid_snapshot())
    }

    /// Return scrollback lines for a specific session, if connected.
    pub fn session_scrollback(&self, id: &SessionId, max_lines: usize) -> Vec<String> {
        self.sessions
            .get(id)
            .map(|s| s.scrollback_lines(max_lines))
            .unwrap_or_default()
    }

    /// Check whether a session's terminal is in application cursor key mode.
    pub fn session_app_cursor(&self, id: &SessionId) -> Option<bool> {
        self.sessions.get(id).map(|s| s.terminal.app_cursor_mode())
    }

    /// Write input bytes to a specific session.
    pub fn write_input(&mut self, id: &SessionId, bytes: &[u8]) {
        if let Some(session_io) = self.sessions.get_mut(id) {
            session_io.write_input(bytes);
        }
    }

    /// Write input bytes to the first connected session.
    pub fn write_input_first(&mut self, bytes: &[u8]) {
        if let Some(session_io) = self.sessions.values_mut().next() {
            session_io.write_input(bytes);
        }
    }
}

/// Spawn a background thread that reads from the PTY stream and sends chunks
/// over an mpsc channel.
fn spawn_output_reader(stream: UnixStream) -> mpsc::Receiver<Vec<u8>> {
    let (tx, rx) = mpsc::channel();
    let mut reader = stream;
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_io_map_starts_empty() {
        let map = SessionIOMap::new();
        assert!(map.first_session_id().is_none());
    }

    #[test]
    fn render_first_session_returns_default_when_empty() {
        let map = SessionIOMap::new();
        assert_eq!(map.render_first_session("no sessions"), "no sessions");
    }

    #[test]
    fn scrollback_lines_returns_last_n_non_empty() {
        let mut ts = crate::terminal_state::TerminalState::new(24, 80);
        ts.write_output(b"line1\r\nline2\r\nline3");
        let snap = ts.grid_snapshot();
        let result = scrollback_from_snapshot(&snap, 2);
        assert_eq!(result, vec!["line2", "line3"]);
    }

    #[test]
    fn scrollback_lines_with_fewer_lines_than_requested() {
        let mut ts = crate::terminal_state::TerminalState::new(24, 80);
        ts.write_output(b"only");
        let snap = ts.grid_snapshot();
        let result = scrollback_from_snapshot(&snap, 5);
        assert_eq!(result, vec!["only"]);
    }

    #[test]
    fn scrollback_lines_empty_terminal() {
        let ts = crate::terminal_state::TerminalState::new(24, 80);
        let snap = ts.grid_snapshot();
        let result = scrollback_from_snapshot(&snap, 3);
        assert!(result.is_empty());
    }

    #[test]
    fn retain_sessions_removes_absent_ids() {
        let mut map = SessionIOMap::new();
        // Manually insert a connected_sockets entry to test retention.
        map.connected_sockets.insert(SessionId(1), "sock1".into());
        map.connected_sockets.insert(SessionId(2), "sock2".into());

        map.retain_sessions(&[SessionId(1)]);
        assert!(map.connected_sockets.contains_key(&SessionId(1)));
        assert!(!map.connected_sockets.contains_key(&SessionId(2)));
    }
}
