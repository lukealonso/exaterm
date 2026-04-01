use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::sync::mpsc;

use exaterm_types::model::SessionId;
use exaterm_ui::beachhead::RawSessionConnector;

pub(crate) trait SessionConnector {
    fn connect_raw_session(
        &self,
        session_id: SessionId,
        socket_name: &str,
    ) -> Result<UnixStream, String>;
}

impl SessionConnector for RawSessionConnector {
    fn connect_raw_session(
        &self,
        session_id: SessionId,
        socket_name: &str,
    ) -> Result<UnixStream, String> {
        RawSessionConnector::connect_raw_session(self, session_id, socket_name)
    }
}

/// Bidirectional I/O bridge for a single session's raw PTY stream.
pub struct SessionIO {
    output_rx: mpsc::Receiver<Vec<u8>>,
    input_writer: UnixStream,
    shutdown_stream: UnixStream,
}

impl SessionIO {
    /// Connect to a session's raw stream socket and spawn a background reader thread.
    pub(crate) fn connect<C: SessionConnector>(
        connector: &C,
        session_id: SessionId,
        socket_name: &str,
    ) -> Result<Self, String> {
        let stream = connector.connect_raw_session(session_id, socket_name)?;
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
    pub(crate) fn connect_new_sessions<C: SessionConnector>(
        &mut self,
        connector: &C,
        raw_socket_names: &BTreeMap<SessionId, String>,
    ) {
        for (id, socket_name) in raw_socket_names {
            let needs_refresh = self
                .connected_sockets
                .get(id)
                .map(|existing| existing != socket_name)
                .unwrap_or(true)
                || !self.sessions.contains_key(id);
            if !needs_refresh {
                continue;
            }

            match SessionIO::connect(connector, *id, socket_name) {
                Ok(session_io) => {
                    self.sessions.insert(*id, session_io);
                    self.connected_sockets.insert(*id, socket_name.clone());
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

    /// Write input bytes to a specific session.
    pub fn write_input(&mut self, id: &SessionId, bytes: &[u8]) {
        if let Some(session_io) = self.sessions.get_mut(id) {
            session_io.write_input(bytes);
        }
    }

    pub fn write_input_all(&mut self, bytes: &[u8]) {
        for session_io in self.sessions.values_mut() {
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
    fn retain_sessions_removes_absent_ids() {
        let mut map = SessionIOMap::new();
        map.connected_sockets.insert(SessionId(1), "sock1".into());
        map.connected_sockets.insert(SessionId(2), "sock2".into());

        map.retain_sessions(&[SessionId(1)]);
        assert!(map.connected_sockets.contains_key(&SessionId(1)));
        assert!(!map.connected_sockets.contains_key(&SessionId(2)));
    }

    struct MockConnector {
        calls: std::cell::RefCell<Vec<(SessionId, String)>>,
        peers: std::cell::RefCell<Vec<UnixStream>>,
    }

    impl MockConnector {
        fn new() -> Self {
            Self {
                calls: std::cell::RefCell::new(Vec::new()),
                peers: std::cell::RefCell::new(Vec::new()),
            }
        }
    }

    impl SessionConnector for MockConnector {
        fn connect_raw_session(
            &self,
            session_id: SessionId,
            socket_name: &str,
        ) -> Result<UnixStream, String> {
            let (left, right) = UnixStream::pair().map_err(|error| error.to_string())?;
            self.calls
                .borrow_mut()
                .push((session_id, socket_name.to_string()));
            self.peers.borrow_mut().push(right);
            Ok(left)
        }
    }

    #[test]
    fn reconnects_when_socket_name_changes() {
        let mut map = SessionIOMap::new();
        let connector = MockConnector::new();

        let mut names = BTreeMap::new();
        names.insert(SessionId(1), "sock1".to_string());
        map.connect_new_sessions(&connector, &names);

        names.insert(SessionId(1), "sock2".to_string());
        map.connect_new_sessions(&connector, &names);

        let calls = connector.calls.borrow();
        assert_eq!(
            calls
                .iter()
                .map(|(_, name)| name.as_str())
                .collect::<Vec<_>>(),
            vec!["sock1", "sock2"]
        );
        assert_eq!(
            map.connected_sockets.get(&SessionId(1)).map(String::as_str),
            Some("sock2")
        );
        assert_eq!(map.sessions.len(), 1);
    }

    #[test]
    fn drain_all_output_empty_map() {
        let mut map = SessionIOMap::new();
        let result = map.drain_all_output();
        assert!(result.is_empty());
    }
}
