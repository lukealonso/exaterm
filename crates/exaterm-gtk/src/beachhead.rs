use crate::remote::{RemoteBeachheadBridge, RemoteRawSessionConnector, connect_remote};
use exaterm_core::daemon::{LocalBeachheadClient, connect_session_stream_socket};
use exaterm_types::model::SessionId;
use exaterm_types::proto::{ClientMessage, ServerMessage};
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::mpsc;

#[derive(Clone, Debug)]
pub enum BeachheadTarget {
    Local,
    Ssh(String),
}

#[derive(Clone)]
pub enum RawSessionConnector {
    Local,
    Remote(Arc<RemoteRawSessionConnector>),
}

impl RawSessionConnector {
    pub fn connect_raw_session(
        &self,
        session_id: SessionId,
        socket_name: &str,
    ) -> Result<UnixStream, String> {
        match self {
            RawSessionConnector::Local => connect_session_stream_socket(socket_name),
            RawSessionConnector::Remote(bridge) => {
                bridge.connect_raw_session(session_id, socket_name)
            }
        }
    }
}

pub struct BeachheadConnection {
    client: LocalBeachheadClient,
    raw_sessions: RawSessionConnector,
    _remote_bridge: Option<RemoteBeachheadBridge>,
}

impl BeachheadConnection {
    pub fn connect(target: &BeachheadTarget) -> Result<Self, String> {
        match target {
            BeachheadTarget::Local => Ok(Self {
                client: LocalBeachheadClient::connect_or_spawn()?,
                raw_sessions: RawSessionConnector::Local,
                _remote_bridge: None,
            }),
            BeachheadTarget::Ssh(target) => {
                let (client, bridge) = connect_remote(target)?;
                Ok(Self {
                    client,
                    raw_sessions: RawSessionConnector::Remote(bridge.raw_connector()),
                    _remote_bridge: Some(bridge),
                })
            }
        }
    }

    pub fn commands(&self) -> &mpsc::Sender<ClientMessage> {
        &self.client.commands
    }

    pub fn events(&self) -> &mpsc::Receiver<ServerMessage> {
        &self.client.events
    }

    pub fn event_wake_fd(&self) -> i32 {
        self.client.event_wake_fd()
    }

    pub fn drain_event_wake(&self) {
        self.client.drain_event_wake();
    }

    pub fn raw_session_connector(&self) -> RawSessionConnector {
        self.raw_sessions.clone()
    }
}
