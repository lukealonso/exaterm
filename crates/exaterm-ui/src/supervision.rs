use exaterm_types::model::{SessionId, SessionRecord, SessionStatus};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BattleCardStatus {
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

impl BattleCardStatus {
    pub fn label(self) -> &'static str {
        match self {
            BattleCardStatus::Idle => "Idle",
            BattleCardStatus::Stopped => "Stopped",
            BattleCardStatus::Active => "Active",
            BattleCardStatus::Thinking => "Thinking",
            BattleCardStatus::Working => "Working",
            BattleCardStatus::Blocked => "Blocked",
            BattleCardStatus::Failed => "Failed",
            BattleCardStatus::Complete => "Complete",
            BattleCardStatus::Detached => "Detached",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ObservedActivity {
    pub active_command: Option<String>,
    pub dominant_process: Option<String>,
    pub recent_files: Vec<String>,
    pub work_output_excerpt: Option<String>,
    pub idle_seconds: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignalTone {
    Calm,
    Watch,
    Alert,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AlignmentSignal {
    pub text: String,
    pub tone: SignalTone,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BattleCardViewModel {
    pub session_id: SessionId,
    pub title: String,
    pub subtitle: String,
    pub status: BattleCardStatus,
    pub recency_label: String,
    pub headline: String,
    pub evidence_fragments: Vec<String>,
    pub alignment: AlignmentSignal,
}

pub fn build_battle_card(
    record: &SessionRecord,
    observed: &ObservedActivity,
) -> BattleCardViewModel {
    let status = derive_battle_card_status(record.status, observed);

    BattleCardViewModel {
        session_id: record.id,
        title: record.launch.name.clone(),
        subtitle: record.launch.subtitle.clone(),
        status,
        recency_label: recency_label(observed.idle_seconds, status),
        headline: String::new(),
        evidence_fragments: Vec::new(),
        alignment: AlignmentSignal {
            text: String::new(),
            tone: SignalTone::Calm,
        },
    }
}

pub fn derive_battle_card_status(
    session_status: SessionStatus,
    observed: &ObservedActivity,
) -> BattleCardStatus {
    let shell_ready = matches!(
        observed.active_command.as_deref(),
        Some("Interactive shell ready")
    );
    let has_runtime_evidence = observed
        .active_command
        .as_deref()
        .is_some_and(|command| command != "Interactive shell ready")
        || observed.dominant_process.is_some()
        || observed.work_output_excerpt.is_some()
        || !observed.recent_files.is_empty();
    match session_status {
        SessionStatus::Blocked => BattleCardStatus::Active,
        SessionStatus::Failed(_) => BattleCardStatus::Failed,
        SessionStatus::Complete => BattleCardStatus::Complete,
        SessionStatus::Detached => BattleCardStatus::Detached,
        SessionStatus::Launching => BattleCardStatus::Active,
        SessionStatus::Waiting => {
            if has_runtime_evidence {
                BattleCardStatus::Active
            } else if shell_ready || observed.idle_seconds.unwrap_or_default() >= 30 {
                BattleCardStatus::Idle
            } else {
                BattleCardStatus::Active
            }
        }
        SessionStatus::Running => {
            if observed.idle_seconds.unwrap_or_default() >= 30
                && observed.active_command.is_none()
                && observed.dominant_process.is_none()
            {
                BattleCardStatus::Idle
            } else {
                BattleCardStatus::Active
            }
        }
    }
}

fn recency_label(idle_seconds: Option<u64>, status: BattleCardStatus) -> String {
    match (status, idle_seconds) {
        (BattleCardStatus::Idle, Some(seconds)) => format!("idle {seconds}s"),
        (_, Some(seconds)) if seconds < 5 => "active now".into(),
        (_, Some(seconds)) => format!("active {seconds}s ago"),
        _ => "recency unknown".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::{BattleCardStatus, ObservedActivity, build_battle_card, derive_battle_card_status};
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
            derive_battle_card_status(SessionStatus::Waiting, &observed),
            BattleCardStatus::Idle
        );
    }

    #[test]
    fn waiting_shell_with_runtime_evidence_stays_active() {
        let observed = ObservedActivity {
            dominant_process: Some("codex".into()),
            idle_seconds: Some(35),
            ..ObservedActivity::default()
        };

        assert_eq!(
            derive_battle_card_status(SessionStatus::Waiting, &observed),
            BattleCardStatus::Active
        );
    }

    #[test]
    fn build_battle_card_leaves_text_fields_blank() {
        let card = build_battle_card(
            &session(SessionStatus::Running),
            &ObservedActivity::default(),
        );
        assert!(card.headline.is_empty());
        assert!(card.evidence_fragments.is_empty());
        assert!(card.alignment.text.is_empty());
    }
}
