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
}

impl SessionIO {
    /// Connect to a session's raw stream socket and spawn a background reader thread.
    pub fn connect(socket_name: &str) -> Result<Self, String> {
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
        })
    }

    /// Drain all pending output from the reader thread, returning the raw bytes.
    pub fn drain_raw_output(&mut self) -> Vec<u8> {
        let mut buf = Vec::new();
        while let Ok(bytes) = self.output_rx.try_recv() {
            buf.extend_from_slice(&bytes);
        }
        buf
    }

    /// Send keyboard input bytes to the PTY.
    pub fn write_input(&mut self, bytes: &[u8]) {
        let _ = self.input_writer.write_all(bytes);
    }
}

impl Drop for SessionIO {
    fn drop(&mut self) {
        let _ = self.shutdown_stream.shutdown(Shutdown::Both);
    }
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
    pub fn connect_new_sessions(&mut self, raw_socket_names: &BTreeMap<SessionId, String>) {
        for (id, socket_name) in raw_socket_names {
            if self.connected_sockets.contains_key(id) {
                continue;
            }
            self.connected_sockets.insert(*id, socket_name.clone());
            match SessionIO::connect(socket_name) {
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

    /// Drain raw output for a specific session. Returns empty vec if not connected.
    pub fn drain_session_output(&mut self, id: &SessionId) -> Vec<u8> {
        self.sessions
            .get_mut(id)
            .map(|s| s.drain_raw_output())
            .unwrap_or_default()
    }

    /// Drain raw output for all sessions, returning a map of session id to bytes.
    pub fn drain_all_output(&mut self) -> BTreeMap<SessionId, Vec<u8>> {
        let mut result = BTreeMap::new();
        for (id, session_io) in self.sessions.iter_mut() {
            let bytes = session_io.drain_raw_output();
            if !bytes.is_empty() {
                result.insert(*id, bytes);
            }
        }
        result
    }

    /// Get the first connected session id (for single-session display).
    pub fn first_session_id(&self) -> Option<SessionId> {
        self.sessions.keys().next().copied()
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
    fn retain_sessions_removes_absent_ids() {
        let mut map = SessionIOMap::new();
        map.connected_sockets.insert(SessionId(1), "sock1".into());
        map.connected_sockets.insert(SessionId(2), "sock2".into());

        map.retain_sessions(&[SessionId(1)]);
        assert!(map.connected_sockets.contains_key(&SessionId(1)));
        assert!(!map.connected_sockets.contains_key(&SessionId(2)));
    }

    #[test]
    fn drain_all_output_empty_map() {
        let mut map = SessionIOMap::new();
        let result = map.drain_all_output();
        assert!(result.is_empty());
    }
}
