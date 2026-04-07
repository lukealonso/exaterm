use exaterm_core::daemon::LocalBeachheadClient;
use exaterm_types::proto::{ClientMessage, ServerMessage, WorkspaceSnapshot};
use std::thread;
use std::time::Duration;
use tokio::sync::{mpsc, watch};

const RECONNECT_DELAY: Duration = Duration::from_secs(2);
const MAX_RECONNECT_DELAY: Duration = Duration::from_secs(30);
const EVENT_POLL_INTERVAL: Duration = Duration::from_millis(50);

pub struct DaemonRelay {
    pub snapshots: watch::Receiver<WorkspaceSnapshot>,
    pub commands: mpsc::Sender<ClientMessage>,
}

impl DaemonRelay {
    pub fn start() -> Self {
        let (snapshot_tx, snapshot_rx) = watch::channel(WorkspaceSnapshot::default());
        let (command_tx, command_rx) = mpsc::channel::<ClientMessage>(256);
        thread::spawn(move || relay_loop(snapshot_tx, command_rx));
        Self {
            snapshots: snapshot_rx,
            commands: command_tx,
        }
    }

    pub fn snapshot(&self) -> WorkspaceSnapshot {
        self.snapshots.borrow().clone()
    }
}

fn relay_loop(
    snapshot_tx: watch::Sender<WorkspaceSnapshot>,
    mut command_rx: mpsc::Receiver<ClientMessage>,
) {
    let mut delay = RECONNECT_DELAY;

    loop {
        let client = match LocalBeachheadClient::connect_or_spawn() {
            Ok(client) => {
                delay = RECONNECT_DELAY;
                client
            }
            Err(error) => {
                eprintln!("daemon connection failed: {error}, retrying in {delay:?}");
                thread::sleep(delay);
                delay = (delay * 2).min(MAX_RECONNECT_DELAY);
                continue;
            }
        };

        let _ = client
            .commands
            .send(ClientMessage::CreateOrResumeDefaultWorkspace);

        loop {
            // Drain pending commands from web clients (non-blocking).
            loop {
                match command_rx.try_recv() {
                    Ok(message) => {
                        if client.commands.send(message).is_err() {
                            break;
                        }
                    }
                    Err(mpsc::error::TryRecvError::Empty) => break,
                    Err(mpsc::error::TryRecvError::Disconnected) => return,
                }
            }

            // Wait briefly for daemon events.
            match client.events.recv_timeout(EVENT_POLL_INTERVAL) {
                Ok(ServerMessage::WorkspaceSnapshot { snapshot }) => {
                    let _ = snapshot_tx.send(snapshot);
                }
                Ok(ServerMessage::Error { message }) => {
                    eprintln!("daemon error: {message}");
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    eprintln!("daemon disconnected, reconnecting");
                    break;
                }
            }
        }
    }
}
