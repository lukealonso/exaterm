use exaterm_core::daemon::LocalBeachheadClient;
use exaterm_types::proto::{ClientMessage, ServerMessage};
use std::sync::mpsc;

pub struct BeachheadConnection {
    client: LocalBeachheadClient,
}

impl BeachheadConnection {
    pub fn connect() -> Result<Self, String> {
        let client = LocalBeachheadClient::connect_or_spawn()?;
        Ok(Self { client })
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
}
