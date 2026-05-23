use exaterm_types::model::{SessionId, SessionRecord, SessionStatus};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionTileStatus {
    Idle,
    Stopped,
    Active,
    Thinking,
    Working,
    Blocked,
    Failed,
    Complete,
    Detached,
}

impl SessionTileStatus {
    pub fn label(self) -> &'static str {
        match self {
            SessionTileStatus::Idle => "Idle",
            SessionTileStatus::Stopped => "Stopped",
            SessionTileStatus::Active => "Active",
            SessionTileStatus::Thinking => "Thinking",
            SessionTileStatus::Working => "Working",
            SessionTileStatus::Blocked => "Blocked",
            SessionTileStatus::Failed => "Failed",
            SessionTileStatus::Complete => "Complete",
            SessionTileStatus::Detached => "Detached",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ObservedActivity {
    pub idle_seconds: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionTileViewModel {
    pub session_id: SessionId,
    pub title: String,
    pub subtitle: String,
    pub status: SessionTileStatus,
    pub recency_label: String,
}

pub fn build_session_tile(
    record: &SessionRecord,
    observed: &ObservedActivity,
) -> SessionTileViewModel {
    let status = derive_session_tile_status(record.status, observed);

    SessionTileViewModel {
        session_id: record.id,
        title: record.launch.name.clone(),
        subtitle: record.launch.subtitle.clone(),
        status,
        recency_label: recency_label(observed.idle_seconds, status),
    }
}

pub fn derive_session_tile_status(
    session_status: SessionStatus,
    observed: &ObservedActivity,
) -> SessionTileStatus {
    match session_status {
        SessionStatus::Blocked => SessionTileStatus::Active,
        SessionStatus::Failed(_) => SessionTileStatus::Failed,
        SessionStatus::Complete => SessionTileStatus::Complete,
        SessionStatus::Detached => SessionTileStatus::Detached,
        SessionStatus::Launching => SessionTileStatus::Active,
        SessionStatus::Waiting => {
            if observed.idle_seconds.unwrap_or_default() >= 30 {
                SessionTileStatus::Idle
            } else {
                SessionTileStatus::Active
            }
        }
        SessionStatus::Running => {
            if observed.idle_seconds.unwrap_or_default() >= 30 {
                SessionTileStatus::Idle
            } else {
                SessionTileStatus::Active
            }
        }
    }
}

fn recency_label(idle_seconds: Option<u64>, status: SessionTileStatus) -> String {
    match (status, idle_seconds) {
        (SessionTileStatus::Idle, Some(seconds)) => format!("idle {seconds}s"),
        (_, Some(seconds)) if seconds < 5 => "active now".into(),
        (_, Some(seconds)) => format!("active {seconds}s ago"),
        _ => "recency unknown".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        build_session_tile, derive_session_tile_status, ObservedActivity, SessionTileStatus,
    };
    use exaterm_core::model::user_shell_launch;
    use exaterm_types::model::{SessionId, SessionRecord, SessionStatus};

    fn session(status: SessionStatus) -> SessionRecord {
        SessionRecord {
            id: SessionId(1),
            launch: user_shell_launch("Shell 1", "Terminal"),
            pid: None,
            status,
            display_name: None,
            events: Vec::new(),
        }
    }

    #[test]
    fn waiting_shell_with_no_runtime_evidence_turns_idle_after_threshold() {
        let observed = ObservedActivity {
            idle_seconds: Some(35),
            ..ObservedActivity::default()
        };

        assert_eq!(
            derive_session_tile_status(SessionStatus::Waiting, &observed),
            SessionTileStatus::Idle
        );
    }

    #[test]
    fn waiting_shell_before_idle_threshold_stays_active() {
        let observed = ObservedActivity {
            idle_seconds: Some(5),
            ..ObservedActivity::default()
        };

        assert_eq!(
            derive_session_tile_status(SessionStatus::Waiting, &observed),
            SessionTileStatus::Active
        );
    }

    #[test]
    fn blocked_session_without_summary_shows_active() {
        // The daemon's Blocked status means a blocking prompt (read, passwd, etc.)
        // which looks like active work without LLM context. This must stay Active
        // to match the GTK client behavior.
        let observed = ObservedActivity::default();
        assert_eq!(
            derive_session_tile_status(SessionStatus::Blocked, &observed),
            SessionTileStatus::Active
        );
    }

    #[test]
    fn build_session_tile_sets_identity_and_status() {
        let card = build_session_tile(
            &session(SessionStatus::Running),
            &ObservedActivity::default(),
        );
        assert_eq!(card.session_id, SessionId(1));
        assert_eq!(card.title, "Shell 1");
        assert_eq!(card.status, SessionTileStatus::Active);
    }
}
