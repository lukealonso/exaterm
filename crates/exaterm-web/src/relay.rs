use crossbeam_channel as channel;
use exaterm_core::daemon::LocalBeachheadClient;
use exaterm_types::proto::{ClientMessage, ServerMessage, WorkspaceSnapshot};
use std::thread;
use std::time::Duration;
use tokio::sync::watch;

const RECONNECT_DELAY: Duration = Duration::from_secs(2);
const MAX_RECONNECT_DELAY: Duration = Duration::from_secs(30);

pub struct DaemonRelay {
    pub snapshots: watch::Receiver<WorkspaceSnapshot>,
    pub commands: channel::Sender<ClientMessage>,
}

impl DaemonRelay {
    pub fn start() -> Self {
        let (snapshot_tx, snapshot_rx) = watch::channel(WorkspaceSnapshot::default());
        let (command_tx, command_rx) = channel::unbounded::<ClientMessage>();
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
    command_rx: channel::Receiver<ClientMessage>,
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

        // Block on both channels simultaneously — zero polling, zero latency.
        'connected: loop {
            channel::select! {
                recv(command_rx) -> msg => {
                    match msg {
                        Ok(message) => {
                            if client.commands.send(message).is_err() {
                                eprintln!("daemon command channel disconnected, reconnecting");
                                break 'connected;
                            }
                        }
                        Err(_) => return, // all senders dropped
                    }
                }
                recv(client.events) -> msg => {
                    match msg {
                        Ok(ServerMessage::WorkspaceSnapshot { snapshot }) => {
                            let _ = snapshot_tx.send(snapshot);
                        }
                        Ok(ServerMessage::Error { message }) => {
                            eprintln!("daemon error: {message}");
                        }
                        Err(_) => {
                            eprintln!("daemon disconnected, reconnecting");
                            let _ = snapshot_tx.send(WorkspaceSnapshot::default());
                            break 'connected;
                        }
                    }
                }
            }
        }
    }
}
