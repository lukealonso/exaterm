use crate::presentation::status_chip_label;
use crate::supervision::{
    build_session_tile, derive_session_tile_status, ObservedActivity, SessionTileStatus,
};
use crate::workspace_view::WorkspaceViewState;
use exaterm_types::model::SessionId;
use exaterm_types::proto::WorkspaceSnapshot;
use std::collections::BTreeMap;

/// Data needed to render a single card in the battlefield view.
#[derive(Clone, Debug)]
pub struct CardRenderData {
    pub id: SessionId,
    pub title: String,
    pub status: SessionTileStatus,
    pub status_label: String,
    pub recency: String,
}

pub struct AppState {
    pub workspace: WorkspaceViewState,
    pub observations: BTreeMap<SessionId, ObservedActivity>,
    pub raw_socket_names: BTreeMap<SessionId, String>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            workspace: WorkspaceViewState::new(),
            observations: BTreeMap::new(),
            raw_socket_names: BTreeMap::new(),
        }
    }

    /// Process a workspace snapshot from the daemon.
    pub fn apply_snapshot(&mut self, snapshot: &WorkspaceSnapshot) {
        // Update observation stubs for new sessions, mapping ObservationSnapshot fields.
        for session in &snapshot.sessions {
            let obs = self.observations.entry(session.record.id).or_default();
            let snap_obs = &session.observation;
            obs.idle_seconds = Some(snap_obs.last_change_age_secs);
        }

        // Track raw stream socket names for each session.
        for session in &snapshot.sessions {
            if let Some(ref name) = session.raw_stream_socket_name {
                self.raw_socket_names
                    .insert(session.record.id, name.clone());
            } else {
                self.raw_socket_names.remove(&session.record.id);
            }
        }

        // Remove observations and socket names for sessions no longer present.
        let session_ids: Vec<_> = snapshot.sessions.iter().map(|s| s.record.id).collect();
        self.observations.retain(|id, _| session_ids.contains(id));
        self.raw_socket_names
            .retain(|id, _| session_ids.contains(id));

        // Update workspace state with the latest records and daemon-owned item order.
        let records = snapshot.sessions.iter().map(|s| s.record.clone()).collect();
        self.workspace
            .replace_workspace(records, snapshot.groups.clone(), snapshot.items.clone());
    }

    /// Build card render data for the battlefield view.
    pub fn card_render_data(&self) -> Vec<CardRenderData> {
        self.workspace
            .ordered_session_ids()
            .iter()
            .filter_map(|session_id| {
                let session = self.workspace.session(*session_id)?;
                let observed = self
                    .observations
                    .get(session_id)
                    .cloned()
                    .unwrap_or_default();
                let card = build_session_tile(session, &observed);
                let title = session
                    .display_name
                    .as_deref()
                    .unwrap_or(&card.title)
                    .to_string();
                let status_label = status_chip_label(card.status, &card.recency_label);
                Some(CardRenderData {
                    id: session.id,
                    title,
                    status: card.status,
                    status_label,
                    recency: card.recency_label,
                })
            })
            .collect()
    }

    /// Select the next session in the list (wrapping around).
    pub fn select_next_session(&mut self) {
        let sessions = self.workspace.ordered_session_ids();
        if sessions.is_empty() {
            return;
        }
        let current = self.workspace.selected_session();
        let next = match current {
            Some(id) => {
                let idx = sessions
                    .iter()
                    .position(|session_id| *session_id == id)
                    .unwrap_or(0);
                let next_idx = (idx + 1) % sessions.len();
                sessions[next_idx]
            }
            None => sessions[0],
        };
        self.workspace.select_session(next);
    }

    /// Select the previous session in the list (wrapping around).
    pub fn select_previous_session(&mut self) {
        let sessions = self.workspace.ordered_session_ids();
        if sessions.is_empty() {
            return;
        }
        let current = self.workspace.selected_session();
        let prev = match current {
            Some(id) => {
                let idx = sessions
                    .iter()
                    .position(|session_id| *session_id == id)
                    .unwrap_or(0);
                let prev_idx = if idx == 0 {
                    sessions.len() - 1
                } else {
                    idx - 1
                };
                sessions[prev_idx]
            }
            None => sessions[sessions.len() - 1],
        };
        self.workspace.select_session(prev);
    }

    /// Build a summary line for display in the window.
    pub fn session_summaries(&self) -> Vec<(SessionId, String, SessionTileStatus)> {
        self.workspace
            .ordered_session_ids()
            .iter()
            .filter_map(|session_id| {
                let session = self.workspace.session(*session_id)?;
                let observed = self
                    .observations
                    .get(session_id)
                    .cloned()
                    .unwrap_or_default();
                let status = derive_session_tile_status(session.status, &observed);
                let display_name = session
                    .display_name
                    .as_deref()
                    .unwrap_or(&session.launch.name);
                Some((session.id, display_name.to_string(), status))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use exaterm_types::model::{
        GroupId, SessionId, SessionRecord, SessionStatus, SupervisedGroupRecord, WorkspaceItem,
    };
    use exaterm_types::proto::{ObservationSnapshot, SessionSnapshot, WorkspaceSnapshot};

    fn make_snapshot(sessions: Vec<SessionSnapshot>) -> WorkspaceSnapshot {
        WorkspaceSnapshot {
            items: Vec::new(),
            sessions,
            groups: Vec::new(),
        }
    }

    fn make_snapshot_with_items(
        items: Vec<WorkspaceItem>,
        sessions: Vec<SessionSnapshot>,
        groups: Vec<SupervisedGroupRecord>,
    ) -> WorkspaceSnapshot {
        WorkspaceSnapshot {
            items,
            sessions,
            groups,
        }
    }

    fn make_session_snapshot(id: u32, name: &str, status: SessionStatus) -> SessionSnapshot {
        SessionSnapshot {
            record: SessionRecord {
                id: SessionId(id),
                launch: exaterm_core::model::user_shell_launch(name, "Terminal"),
                pid: None,
                status,
                display_name: None,
                events: Vec::new(),
            },
            observation: ObservationSnapshot::default(),
            raw_stream_socket_name: None,
        }
    }

    fn make_group(id: u32, name: &str, members: Vec<SessionId>) -> SupervisedGroupRecord {
        SupervisedGroupRecord {
            id: GroupId(id),
            name: name.into(),
            member_session_ids: members,
            supervisor_session_id: None,
            provider: None,
            goal: None,
            summary_markdown: String::new(),
            supervisor_visible: false,
            summary_age_secs: None,
            latest_action_age_secs: None,
            actions: Vec::new(),
        }
    }

    #[test]
    fn apply_snapshot_populates_sessions() {
        let mut state = AppState::new();
        let snapshot = make_snapshot(vec![
            make_session_snapshot(1, "Shell 1", SessionStatus::Running),
            make_session_snapshot(2, "Shell 2", SessionStatus::Waiting),
        ]);

        state.apply_snapshot(&snapshot);

        assert_eq!(state.workspace.sessions().len(), 2);
        assert_eq!(state.observations.len(), 2);
    }

    #[test]
    fn apply_snapshot_preserves_workspace_items_and_groups() {
        let mut state = AppState::new();
        let sessions = vec![
            make_session_snapshot(1, "Shell 1", SessionStatus::Running),
            make_session_snapshot(2, "Shell 2", SessionStatus::Running),
        ];
        let group = make_group(7, "Review group", vec![SessionId(1), SessionId(2)]);
        let snapshot = make_snapshot_with_items(
            vec![
                WorkspaceItem::Group(GroupId(7)),
                WorkspaceItem::Session(SessionId(2)),
                WorkspaceItem::Session(SessionId(1)),
            ],
            sessions,
            vec![group],
        );

        state.apply_snapshot(&snapshot);

        assert_eq!(
            state.workspace.ordered_visible_items(),
            &[
                WorkspaceItem::Group(GroupId(7)),
                WorkspaceItem::Session(SessionId(2)),
                WorkspaceItem::Session(SessionId(1)),
            ]
        );
        assert_eq!(
            state.workspace.ordered_session_ids(),
            &[SessionId(2), SessionId(1)]
        );
        assert_eq!(
            state
                .workspace
                .group(GroupId(7))
                .map(|group| group.name.as_str()),
            Some("Review group")
        );
        assert_eq!(state.workspace.selected_session(), Some(SessionId(2)));
        assert_eq!(
            state.workspace.selected_workspace_item(),
            Some(WorkspaceItem::Group(GroupId(7)))
        );
    }

    #[test]
    fn apply_snapshot_prunes_stale_groups_and_group_items() {
        let mut state = AppState::new();
        let initial = make_snapshot_with_items(
            vec![
                WorkspaceItem::Group(GroupId(7)),
                WorkspaceItem::Session(SessionId(1)),
            ],
            vec![make_session_snapshot(1, "Shell 1", SessionStatus::Running)],
            vec![make_group(7, "Old group", vec![SessionId(1)])],
        );
        state.apply_snapshot(&initial);
        state
            .workspace
            .select_workspace_item(WorkspaceItem::Group(GroupId(7)));

        let updated = make_snapshot_with_items(
            vec![
                WorkspaceItem::Group(GroupId(7)),
                WorkspaceItem::Session(SessionId(1)),
            ],
            vec![make_session_snapshot(1, "Shell 1", SessionStatus::Running)],
            Vec::new(),
        );
        state.apply_snapshot(&updated);

        assert!(state.workspace.groups().is_empty());
        assert!(state.workspace.group(GroupId(7)).is_none());
        assert_eq!(
            state.workspace.ordered_visible_items(),
            &[WorkspaceItem::Session(SessionId(1))]
        );
        assert_eq!(
            state.workspace.selected_workspace_item(),
            Some(WorkspaceItem::Session(SessionId(1)))
        );
    }

    #[test]
    fn apply_snapshot_removes_stale_observations() {
        let mut state = AppState::new();

        // First snapshot with two sessions.
        let snapshot = make_snapshot(vec![
            make_session_snapshot(1, "Shell 1", SessionStatus::Running),
            make_session_snapshot(2, "Shell 2", SessionStatus::Running),
        ]);
        state.apply_snapshot(&snapshot);
        assert_eq!(state.observations.len(), 2);

        // Second snapshot drops session 2.
        let snapshot = make_snapshot(vec![make_session_snapshot(
            1,
            "Shell 1",
            SessionStatus::Running,
        )]);
        state.apply_snapshot(&snapshot);
        assert_eq!(state.observations.len(), 1);
        assert!(state.observations.contains_key(&SessionId(1)));
        assert!(!state.observations.contains_key(&SessionId(2)));
    }

    #[test]
    fn apply_snapshot_updates_raw_socket_name_when_it_changes() {
        let mut state = AppState::new();
        let mut snap = make_session_snapshot(1, "Shell 1", SessionStatus::Running);
        snap.raw_stream_socket_name = Some("session-1.sock".into());
        state.apply_snapshot(&make_snapshot(vec![snap]));

        let mut updated = make_session_snapshot(1, "Shell 1", SessionStatus::Running);
        updated.raw_stream_socket_name = Some("session-1-new.sock".into());
        state.apply_snapshot(&make_snapshot(vec![updated]));

        assert_eq!(
            state
                .raw_socket_names
                .get(&SessionId(1))
                .map(String::as_str),
            Some("session-1-new.sock")
        );
    }

    #[test]
    fn apply_snapshot_clears_raw_socket_name_when_it_disappears() {
        let mut state = AppState::new();
        let mut snap = make_session_snapshot(1, "Shell 1", SessionStatus::Running);
        snap.raw_stream_socket_name = Some("session-1.sock".into());
        state.apply_snapshot(&make_snapshot(vec![snap]));

        let mut updated = make_session_snapshot(1, "Shell 1", SessionStatus::Running);
        updated.raw_stream_socket_name = None;
        state.apply_snapshot(&make_snapshot(vec![updated]));

        assert!(!state.raw_socket_names.contains_key(&SessionId(1)));
    }

    #[test]
    fn apply_snapshot_maps_snapshot_visible_observation_fields() {
        let mut state = AppState::new();
        let mut snap = make_session_snapshot(1, "Shell", SessionStatus::Running);
        snap.observation.last_change_age_secs = 5;

        state.apply_snapshot(&make_snapshot(vec![snap]));

        let obs = state.observations.get(&SessionId(1)).unwrap();
        assert_eq!(obs.idle_seconds, Some(5));
    }

    #[test]
    fn session_summaries_uses_display_name_when_present() {
        let mut state = AppState::new();
        let mut snap = make_session_snapshot(1, "Shell 1", SessionStatus::Running);
        snap.record.display_name = Some("My Custom Name".into());

        state.apply_snapshot(&make_snapshot(vec![snap]));

        let summaries = state.session_summaries();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].1, "My Custom Name");
    }

    #[test]
    fn session_summaries_falls_back_to_launch_name() {
        let mut state = AppState::new();
        let snap = make_session_snapshot(1, "Shell 1", SessionStatus::Running);

        state.apply_snapshot(&make_snapshot(vec![snap]));

        let summaries = state.session_summaries();
        assert_eq!(summaries[0].1, "Shell 1");
    }

    #[test]
    fn empty_snapshot_clears_state() {
        let mut state = AppState::new();
        let snapshot = make_snapshot(vec![make_session_snapshot(
            1,
            "Shell",
            SessionStatus::Running,
        )]);
        state.apply_snapshot(&snapshot);
        assert_eq!(state.workspace.sessions().len(), 1);

        state.apply_snapshot(&make_snapshot(vec![]));
        assert_eq!(state.workspace.sessions().len(), 0);
        assert_eq!(state.observations.len(), 0);
    }

    #[test]
    fn card_render_data_returns_titles_and_statuses() {
        let mut state = AppState::new();
        let snapshot = make_snapshot(vec![
            make_session_snapshot(1, "Shell 1", SessionStatus::Running),
            make_session_snapshot(2, "Shell 2", SessionStatus::Waiting),
        ]);
        state.apply_snapshot(&snapshot);

        let cards = state.card_render_data();
        assert_eq!(cards.len(), 2);
        assert_eq!(cards[0].id, SessionId(1));
        assert_eq!(cards[0].title, "Shell 1");
        assert_eq!(cards[1].id, SessionId(2));
        assert_eq!(cards[1].title, "Shell 2");
    }

    #[test]
    fn card_render_data_uses_display_name() {
        let mut state = AppState::new();
        let mut snap = make_session_snapshot(1, "Shell 1", SessionStatus::Running);
        snap.record.display_name = Some("My Project".into());
        state.apply_snapshot(&make_snapshot(vec![snap]));

        let cards = state.card_render_data();
        assert_eq!(cards[0].title, "My Project");
    }

    #[test]
    fn select_next_session_cycles_forward() {
        let mut state = AppState::new();
        let snapshot = make_snapshot(vec![
            make_session_snapshot(1, "Shell 1", SessionStatus::Running),
            make_session_snapshot(2, "Shell 2", SessionStatus::Running),
            make_session_snapshot(3, "Shell 3", SessionStatus::Running),
        ]);
        state.apply_snapshot(&snapshot);

        // Initially selects first session.
        assert_eq!(state.workspace.selected_session(), Some(SessionId(1)));

        state.select_next_session();
        assert_eq!(state.workspace.selected_session(), Some(SessionId(2)));

        state.select_next_session();
        assert_eq!(state.workspace.selected_session(), Some(SessionId(3)));

        // Wraps around.
        state.select_next_session();
        assert_eq!(state.workspace.selected_session(), Some(SessionId(1)));
    }

    #[test]
    fn select_previous_session_cycles_backward() {
        let mut state = AppState::new();
        let snapshot = make_snapshot(vec![
            make_session_snapshot(1, "Shell 1", SessionStatus::Running),
            make_session_snapshot(2, "Shell 2", SessionStatus::Running),
        ]);
        state.apply_snapshot(&snapshot);

        assert_eq!(state.workspace.selected_session(), Some(SessionId(1)));

        // Wraps to last.
        state.select_previous_session();
        assert_eq!(state.workspace.selected_session(), Some(SessionId(2)));

        state.select_previous_session();
        assert_eq!(state.workspace.selected_session(), Some(SessionId(1)));
    }

    #[test]
    fn select_next_noop_on_empty() {
        let mut state = AppState::new();
        state.select_next_session();
        assert_eq!(state.workspace.selected_session(), None);
    }

    #[test]
    fn session_summaries_derive_correct_status_for_idle() {
        let mut state = AppState::new();
        let mut snap = make_session_snapshot(1, "Shell", SessionStatus::Running);
        snap.observation.last_change_age_secs = 60;

        state.apply_snapshot(&make_snapshot(vec![snap]));

        let summaries = state.session_summaries();
        assert_eq!(summaries[0].2, SessionTileStatus::Idle);
    }
}
