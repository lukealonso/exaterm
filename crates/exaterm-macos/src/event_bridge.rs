use exaterm_types::proto::{ServerMessage, WorkspaceSnapshot};
use exaterm_ui::workspace_view::WorkspaceViewState;
use std::sync::mpsc;

/// Result of draining pending daemon events.
pub struct DrainResult {
    pub events: Vec<ServerMessage>,
    pub disconnected: bool,
}

/// Bridges daemon events to the UI refresh cycle.
pub struct EventBridge {
    events_rx: mpsc::Receiver<ServerMessage>,
}

impl EventBridge {
    pub fn new(events_rx: mpsc::Receiver<ServerMessage>) -> Self {
        Self { events_rx }
    }

    /// Drain all pending daemon events from the channel.
    pub fn drain_events(&self) -> DrainResult {
        let mut events = Vec::new();
        let mut disconnected = false;
        loop {
            match self.events_rx.try_recv() {
                Ok(message) => events.push(message),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            }
        }
        DrainResult {
            events,
            disconnected,
        }
    }

    /// Process a workspace snapshot, updating the view state to match.
    pub fn apply_snapshot(snapshot: &WorkspaceSnapshot, state: &mut WorkspaceViewState) {
        let records: Vec<_> = snapshot
            .sessions
            .iter()
            .map(|session| session.record.clone())
            .collect();
        state.replace_sessions(records);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use exaterm_core::model::shell_launch;
    use exaterm_types::model::{SessionId, SessionStatus};
    use exaterm_types::proto::{ObservationSnapshot, SessionSnapshot};

    fn make_session_snapshot(id: u32, name: &str) -> SessionSnapshot {
        use exaterm_types::model::SessionRecord;

        SessionSnapshot {
            record: SessionRecord {
                id: SessionId(id),
                launch: shell_launch(name, "shell", "banner"),
                display_name: None,
                status: SessionStatus::Waiting,
                pid: Some(100 + id),
                events: Vec::new(),
            },
            observation: ObservationSnapshot::default(),
            summary: None,
            raw_stream_socket_name: None,
            auto_nudge_enabled: false,
            last_nudge: None,
            last_sent_age_secs: None,
        }
    }

    #[test]
    fn drain_events_empty_channel_returns_empty_vec() {
        let (_tx, rx) = mpsc::channel::<ServerMessage>();
        let bridge = EventBridge::new(rx);

        let result = bridge.drain_events();

        assert!(result.events.is_empty());
        assert!(!result.disconnected);
    }

    #[test]
    fn drain_events_returns_all_pending_messages() {
        let (tx, rx) = mpsc::channel();
        let bridge = EventBridge::new(rx);

        tx.send(ServerMessage::WorkspaceSnapshot {
            snapshot: WorkspaceSnapshot::default(),
        })
        .unwrap();
        tx.send(ServerMessage::Error {
            message: "boom".into(),
        })
        .unwrap();
        tx.send(ServerMessage::WorkspaceSnapshot {
            snapshot: WorkspaceSnapshot::default(),
        })
        .unwrap();

        let result = bridge.drain_events();

        assert_eq!(result.events.len(), 3);
        assert!(!result.disconnected);
        assert!(matches!(
            result.events[0],
            ServerMessage::WorkspaceSnapshot { .. }
        ));
        assert!(matches!(result.events[1], ServerMessage::Error { .. }));
        assert!(matches!(
            result.events[2],
            ServerMessage::WorkspaceSnapshot { .. }
        ));
    }

    #[test]
    fn apply_snapshot_with_two_sessions_updates_state() {
        let mut state = WorkspaceViewState::new();
        let snapshot = WorkspaceSnapshot {
            sessions: vec![
                make_session_snapshot(1, "Alpha"),
                make_session_snapshot(2, "Beta"),
            ],
        };

        EventBridge::apply_snapshot(&snapshot, &mut state);

        assert_eq!(state.sessions().len(), 2);
        assert_eq!(state.sessions()[0].id, SessionId(1));
        assert_eq!(state.sessions()[1].id, SessionId(2));
        assert_eq!(state.sessions()[0].launch.name, "Alpha");
        assert_eq!(state.sessions()[1].launch.name, "Beta");
        assert_eq!(state.selected_session(), Some(SessionId(1)));
    }

    #[test]
    fn apply_snapshot_preserves_focus_mode_when_focused_session_survives() {
        let mut state = WorkspaceViewState::new();

        // Seed initial sessions into state via a first snapshot.
        let initial = WorkspaceSnapshot {
            sessions: vec![
                make_session_snapshot(1, "Alpha"),
                make_session_snapshot(2, "Beta"),
            ],
        };
        EventBridge::apply_snapshot(&initial, &mut state);

        // Enter focus mode on session 2.
        state.enter_focus_mode(SessionId(2));
        assert_eq!(state.focused_session(), Some(SessionId(2)));

        // Apply a new snapshot that still contains session 2.
        let updated = WorkspaceSnapshot {
            sessions: vec![
                make_session_snapshot(1, "Alpha"),
                make_session_snapshot(2, "Beta-updated"),
            ],
        };
        EventBridge::apply_snapshot(&updated, &mut state);

        assert_eq!(state.focused_session(), Some(SessionId(2)));
        assert_eq!(state.selected_session(), Some(SessionId(2)));
    }
}
