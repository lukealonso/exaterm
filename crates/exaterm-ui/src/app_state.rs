use crate::presentation::{
    AttentionPresentation, attention_presentation, combined_focus_summary_text, status_chip_label,
};
use crate::supervision::{
    BattleCardStatus, ObservedActivity, build_battle_card, derive_battle_card_status,
};
use crate::workspace_view::WorkspaceViewState;
use exaterm_types::model::SessionId;
use exaterm_types::proto::WorkspaceSnapshot;
use exaterm_types::synthesis::{AttentionLevel, TacticalSynthesis};
use std::collections::BTreeMap;

/// Data needed to render a single card in the battlefield view.
#[derive(Clone, Debug)]
pub struct CardRenderData {
    pub id: SessionId,
    pub title: String,
    pub status: BattleCardStatus,
    pub status_label: String,
    pub recency: String,
    pub scrollback: Vec<String>,
    /// One-line synthesis headline (e.g. "Parser pass completed").
    pub headline: String,
    pub combined_headline: String,
    /// Optional detailed synthesis text.
    pub detail: Option<String>,
    /// Optional alert text (operator action recommendation).
    pub alert: Option<String>,
    pub attention: Option<AttentionPresentation>,
    pub attention_reason: Option<String>,
    /// Whether auto-nudge is enabled for this session.
    pub auto_nudge_enabled: bool,
    /// Most recent auto-nudge text, if any.
    pub last_nudge: Option<String>,
}

#[derive(Clone, Debug)]
pub struct FocusRenderData {
    pub id: SessionId,
    pub title: String,
    pub status: BattleCardStatus,
    pub status_label: String,
    pub combined_headline: String,
    pub attention: Option<AttentionPresentation>,
    pub attention_reason: Option<String>,
}

/// Extract headline, detail, and alert strings from an optional `TacticalSynthesis`.
///
/// - `headline`: the synthesis headline, or empty if absent.
/// - `detail`: the tactical state brief, if present.
/// - `alert`: the attention brief, if the attention level requires intervention.
pub fn extract_synthesis_fields(
    synthesis: Option<&TacticalSynthesis>,
) -> (String, Option<String>, Option<String>) {
    match synthesis {
        Some(s) => {
            let headline = s.headline.clone().unwrap_or_default();
            let detail = s.tactical_state_brief.clone();
            let alert = match s.attention_level {
                AttentionLevel::Autopilot | AttentionLevel::Monitor => None,
                _ => s.attention_brief.clone(),
            };
            (headline, detail, alert)
        }
        None => (String::new(), None, None),
    }
}

pub struct AppState {
    pub workspace: WorkspaceViewState,
    pub observations: BTreeMap<SessionId, ObservedActivity>,
    pub recent_lines: BTreeMap<SessionId, Vec<String>>,
    pub raw_socket_names: BTreeMap<SessionId, String>,
    pub summaries: BTreeMap<SessionId, TacticalSynthesis>,
    pub auto_nudge_enabled: BTreeMap<SessionId, bool>,
    pub last_nudges: BTreeMap<SessionId, String>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            workspace: WorkspaceViewState::new(),
            observations: BTreeMap::new(),
            recent_lines: BTreeMap::new(),
            raw_socket_names: BTreeMap::new(),
            summaries: BTreeMap::new(),
            auto_nudge_enabled: BTreeMap::new(),
            last_nudges: BTreeMap::new(),
        }
    }

    /// Process a workspace snapshot from the daemon.
    pub fn apply_snapshot(&mut self, snapshot: &WorkspaceSnapshot) {
        // Update observation stubs for new sessions, mapping ObservationSnapshot fields.
        for session in &snapshot.sessions {
            let obs = self.observations.entry(session.record.id).or_default();
            let snap_obs = &session.observation;
            obs.active_command = snap_obs.active_command.clone();
            obs.dominant_process = snap_obs.dominant_process.clone();
            obs.recent_files = snap_obs.recent_files.clone();
            obs.work_output_excerpt = snap_obs.work_output_excerpt.clone();
            obs.idle_seconds = Some(snap_obs.last_change_age_secs);
        }

        // Store recent terminal output lines for card scrollback.
        for session in &snapshot.sessions {
            self.recent_lines
                .insert(session.record.id, session.observation.recent_lines.clone());
        }

        // Track raw stream socket names for each session.
        for session in &snapshot.sessions {
            if let Some(ref name) = session.raw_stream_socket_name {
                self.raw_socket_names
                    .entry(session.record.id)
                    .or_insert_with(|| name.clone());
            }
        }

        // Store synthesis summaries from the snapshot.
        for session in &snapshot.sessions {
            if let Some(ref synthesis) = session.summary {
                self.summaries.insert(session.record.id, synthesis.clone());
            } else {
                self.summaries.remove(&session.record.id);
            }
        }

        for session in &snapshot.sessions {
            self.auto_nudge_enabled
                .insert(session.record.id, session.auto_nudge_enabled);
            if let Some(ref last_nudge) = session.last_nudge {
                self.last_nudges
                    .insert(session.record.id, last_nudge.clone());
            } else {
                self.last_nudges.remove(&session.record.id);
            }
        }

        // Remove observations, socket names, and summaries for sessions no longer present.
        let session_ids: Vec<_> = snapshot.sessions.iter().map(|s| s.record.id).collect();
        self.observations.retain(|id, _| session_ids.contains(id));
        self.recent_lines.retain(|id, _| session_ids.contains(id));
        self.raw_socket_names
            .retain(|id, _| session_ids.contains(id));
        self.summaries.retain(|id, _| session_ids.contains(id));
        self.auto_nudge_enabled
            .retain(|id, _| session_ids.contains(id));
        self.last_nudges.retain(|id, _| session_ids.contains(id));

        // Update workspace state with the latest session records.
        let records = snapshot.sessions.iter().map(|s| s.record.clone()).collect();
        self.workspace.replace_sessions(records);
    }

    /// Build card render data for the battlefield view.
    pub fn card_render_data(&self) -> Vec<CardRenderData> {
        self.workspace
            .sessions()
            .iter()
            .map(|session| {
                let observed = self
                    .observations
                    .get(&session.id)
                    .cloned()
                    .unwrap_or_default();
                let card = build_battle_card(session, &observed);
                let scrollback = self
                    .recent_lines
                    .get(&session.id)
                    .map(|lines| {
                        let taken: Vec<String> = lines
                            .iter()
                            .rev()
                            .map(|l| l.trim().to_string())
                            .filter(|l| !l.is_empty())
                            .take(4)
                            .collect();
                        taken.into_iter().rev().collect()
                    })
                    .unwrap_or_default();
                let title = session
                    .display_name
                    .as_deref()
                    .unwrap_or(&card.title)
                    .to_string();
                let (headline, detail, alert) =
                    extract_synthesis_fields(self.summaries.get(&session.id));
                let status_label = status_chip_label(card.status, &card.recency_label);
                let (attention, attention_reason) =
                    attention_presentation(self.summaries.get(&session.id))
                        .map(|(presentation, reason)| (Some(presentation), reason))
                        .unwrap_or((None, None));
                CardRenderData {
                    id: session.id,
                    title,
                    status: card.status,
                    status_label,
                    recency: card.recency_label,
                    scrollback,
                    combined_headline: combined_focus_summary_text(
                        &headline,
                        self.summaries
                            .get(&session.id)
                            .and_then(|summary| summary.attention_brief.as_deref()),
                    ),
                    headline,
                    detail,
                    alert,
                    attention,
                    attention_reason,
                    auto_nudge_enabled: self
                        .auto_nudge_enabled
                        .get(&session.id)
                        .copied()
                        .unwrap_or(false),
                    last_nudge: self.last_nudges.get(&session.id).cloned(),
                }
            })
            .collect()
    }

    pub fn focus_render_data(&self, session_id: SessionId) -> Option<FocusRenderData> {
        let session = self.workspace.session(session_id)?;
        let observed = self
            .observations
            .get(&session_id)
            .cloned()
            .unwrap_or_default();
        let card = build_battle_card(session, &observed);
        let title = session
            .display_name
            .as_deref()
            .unwrap_or(&card.title)
            .to_string();
        let summary = self.summaries.get(&session_id);
        let status_label = status_chip_label(card.status, &card.recency_label);
        let (headline, _, _) = extract_synthesis_fields(summary);
        let (attention, attention_reason) = attention_presentation(summary)
            .map(|(presentation, reason)| (Some(presentation), reason))
            .unwrap_or((None, None));
        Some(FocusRenderData {
            id: session_id,
            title,
            status: card.status,
            status_label,
            combined_headline: combined_focus_summary_text(
                &headline,
                summary.and_then(|s| s.attention_brief.as_deref()),
            ),
            attention,
            attention_reason,
        })
    }

    /// Select the next session in the list (wrapping around).
    pub fn select_next_session(&mut self) {
        let sessions = self.workspace.sessions();
        if sessions.is_empty() {
            return;
        }
        let current = self.workspace.selected_session();
        let next = match current {
            Some(id) => {
                let idx = sessions.iter().position(|s| s.id == id).unwrap_or(0);
                let next_idx = (idx + 1) % sessions.len();
                sessions[next_idx].id
            }
            None => sessions[0].id,
        };
        self.workspace.select_session(next);
    }

    /// Select the previous session in the list (wrapping around).
    pub fn select_previous_session(&mut self) {
        let sessions = self.workspace.sessions();
        if sessions.is_empty() {
            return;
        }
        let current = self.workspace.selected_session();
        let prev = match current {
            Some(id) => {
                let idx = sessions.iter().position(|s| s.id == id).unwrap_or(0);
                let prev_idx = if idx == 0 {
                    sessions.len() - 1
                } else {
                    idx - 1
                };
                sessions[prev_idx].id
            }
            None => sessions[sessions.len() - 1].id,
        };
        self.workspace.select_session(prev);
    }

    /// Build a summary line for display in the window.
    pub fn session_summaries(&self) -> Vec<(SessionId, String, BattleCardStatus)> {
        self.workspace
            .sessions()
            .iter()
            .map(|session| {
                let observed = self
                    .observations
                    .get(&session.id)
                    .cloned()
                    .unwrap_or_default();
                let status = derive_battle_card_status(session.status, &observed);
                let display_name = session
                    .display_name
                    .as_deref()
                    .unwrap_or(&session.launch.name);
                (session.id, display_name.to_string(), status)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use exaterm_types::model::{SessionId, SessionRecord, SessionStatus};
    use exaterm_types::proto::{ObservationSnapshot, SessionSnapshot, WorkspaceSnapshot};

    fn make_snapshot(sessions: Vec<SessionSnapshot>) -> WorkspaceSnapshot {
        WorkspaceSnapshot { sessions }
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
            summary: None,
            raw_stream_socket_name: None,
            auto_nudge_enabled: false,
            last_nudge: None,
            last_sent_age_secs: None,
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
    fn apply_snapshot_maps_observation_fields() {
        let mut state = AppState::new();
        let mut snap = make_session_snapshot(1, "Shell", SessionStatus::Running);
        snap.observation.active_command = Some("cargo build".into());
        snap.observation.dominant_process = Some("rustc".into());
        snap.observation.last_change_age_secs = 5;
        snap.observation.recent_files = vec!["main.rs".into()];
        snap.observation.work_output_excerpt = Some("Compiling...".into());

        state.apply_snapshot(&make_snapshot(vec![snap]));

        let obs = state.observations.get(&SessionId(1)).unwrap();
        assert_eq!(obs.active_command.as_deref(), Some("cargo build"));
        assert_eq!(obs.dominant_process.as_deref(), Some("rustc"));
        assert_eq!(obs.idle_seconds, Some(5));
        assert_eq!(obs.recent_files, vec!["main.rs"]);
        assert_eq!(obs.work_output_excerpt.as_deref(), Some("Compiling..."));
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
    fn card_render_data_scrollback_empty_without_ios() {
        let mut state = AppState::new();
        let snapshot = make_snapshot(vec![make_session_snapshot(
            1,
            "Shell 1",
            SessionStatus::Running,
        )]);
        state.apply_snapshot(&snapshot);

        let cards = state.card_render_data();
        assert!(cards[0].scrollback.is_empty());
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
    fn extract_synthesis_fields_none() {
        let (headline, detail, alert) = extract_synthesis_fields(None);
        assert!(headline.is_empty());
        assert!(detail.is_none());
        assert!(alert.is_none());
    }

    #[test]
    fn extract_synthesis_fields_with_headline() {
        use exaterm_types::synthesis::{AttentionLevel, TacticalState, TacticalSynthesis};
        let synth = TacticalSynthesis {
            tactical_state: TacticalState::Working,
            tactical_state_brief: Some("Steady progress".into()),
            attention_level: AttentionLevel::Autopilot,
            attention_brief: None,
            headline: Some("Build passing".into()),
        };
        let (headline, detail, alert) = extract_synthesis_fields(Some(&synth));
        assert_eq!(headline, "Build passing");
        assert_eq!(detail.as_deref(), Some("Steady progress"));
        assert!(
            alert.is_none(),
            "Autopilot attention should produce no alert"
        );
    }

    #[test]
    fn extract_synthesis_fields_with_alert() {
        use exaterm_types::synthesis::{AttentionLevel, TacticalState, TacticalSynthesis};
        let synth = TacticalSynthesis {
            tactical_state: TacticalState::Blocked,
            tactical_state_brief: None,
            attention_level: AttentionLevel::Intervene,
            attention_brief: Some("Process stuck, needs input".into()),
            headline: Some("Blocked on user".into()),
        };
        let (headline, _detail, alert) = extract_synthesis_fields(Some(&synth));
        assert_eq!(headline, "Blocked on user");
        assert_eq!(alert.as_deref(), Some("Process stuck, needs input"));
    }

    #[test]
    fn apply_snapshot_stores_summaries() {
        use exaterm_types::synthesis::{AttentionLevel, TacticalState, TacticalSynthesis};
        let mut state = AppState::new();
        let mut snap = make_session_snapshot(1, "Shell", SessionStatus::Running);
        snap.summary = Some(TacticalSynthesis {
            tactical_state: TacticalState::Working,
            tactical_state_brief: None,
            attention_level: AttentionLevel::Monitor,
            attention_brief: None,
            headline: Some("Tests passing".into()),
        });

        state.apply_snapshot(&make_snapshot(vec![snap]));

        assert!(state.summaries.contains_key(&SessionId(1)));
        let synth = state.summaries.get(&SessionId(1)).unwrap();
        assert_eq!(synth.headline.as_deref(), Some("Tests passing"));
    }

    #[test]
    fn apply_snapshot_clears_summary_when_absent() {
        use exaterm_types::synthesis::{AttentionLevel, TacticalState, TacticalSynthesis};
        let mut state = AppState::new();

        // First snapshot with a summary.
        let mut snap = make_session_snapshot(1, "Shell", SessionStatus::Running);
        snap.summary = Some(TacticalSynthesis {
            tactical_state: TacticalState::Working,
            tactical_state_brief: None,
            attention_level: AttentionLevel::Monitor,
            attention_brief: None,
            headline: Some("Active".into()),
        });
        state.apply_snapshot(&make_snapshot(vec![snap]));
        assert!(state.summaries.contains_key(&SessionId(1)));

        // Second snapshot without summary.
        let snap2 = make_session_snapshot(1, "Shell", SessionStatus::Running);
        state.apply_snapshot(&make_snapshot(vec![snap2]));
        assert!(!state.summaries.contains_key(&SessionId(1)));
    }

    #[test]
    fn card_render_data_includes_synthesis_fields() {
        use exaterm_types::synthesis::{AttentionLevel, TacticalState, TacticalSynthesis};
        let mut state = AppState::new();
        let mut snap = make_session_snapshot(1, "Shell", SessionStatus::Running);
        snap.summary = Some(TacticalSynthesis {
            tactical_state: TacticalState::Working,
            tactical_state_brief: Some("Good momentum".into()),
            attention_level: AttentionLevel::Guide,
            attention_brief: Some("Monitor closely".into()),
            headline: Some("Compiling".into()),
        });

        state.apply_snapshot(&make_snapshot(vec![snap]));

        let cards = state.card_render_data();
        assert_eq!(cards[0].headline, "Compiling");
        assert_eq!(cards[0].detail.as_deref(), Some("Good momentum"));
        assert_eq!(cards[0].alert.as_deref(), Some("Monitor closely"));
    }

    #[test]
    fn card_render_data_uses_daemon_recent_lines() {
        let mut state = AppState::new();
        let mut snap = make_session_snapshot(1, "Shell", SessionStatus::Running);
        snap.observation.recent_lines = vec![
            "$ cargo build".into(),
            "   Compiling exaterm v0.1.0".into(),
            "    Finished dev".into(),
        ];
        state.apply_snapshot(&make_snapshot(vec![snap]));

        let cards = state.card_render_data();
        assert_eq!(
            cards[0].scrollback,
            vec!["$ cargo build", "Compiling exaterm v0.1.0", "Finished dev",]
        );
    }

    #[test]
    fn card_render_data_scrollback_trims_and_filters() {
        let mut state = AppState::new();
        let mut snap = make_session_snapshot(1, "Shell", SessionStatus::Running);
        snap.observation.recent_lines = vec![
            "line1".into(),
            "  ".into(), // whitespace-only, should be filtered
            "".into(),   // empty, should be filtered
            "line2".into(),
            "  line3  ".into(), // should be trimmed
            "line4".into(),
            "line5".into(),
            "line6".into(), // exceeds limit of 4
        ];
        state.apply_snapshot(&make_snapshot(vec![snap]));

        let cards = state.card_render_data();
        // Takes last 4 non-empty trimmed lines
        assert_eq!(
            cards[0].scrollback,
            vec!["line3", "line4", "line5", "line6"]
        );
    }

    #[test]
    fn session_summaries_derive_correct_status_for_idle() {
        let mut state = AppState::new();
        let mut snap = make_session_snapshot(1, "Shell", SessionStatus::Running);
        snap.observation.last_change_age_secs = 60;

        state.apply_snapshot(&make_snapshot(vec![snap]));

        let summaries = state.session_summaries();
        assert_eq!(summaries[0].2, BattleCardStatus::Idle);
    }
}
