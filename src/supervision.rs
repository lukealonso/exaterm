use crate::model::{SessionEvent, SessionId, SessionRecord, SessionStatus};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntentSource {
    Stated,
    Inferred,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IntentSummary {
    pub source: IntentSource,
    pub text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BattleCardStatus {
    Idle,
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CorrelationSummary {
    pub narrative: String,
    pub suspicious_mismatch: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BattleCardViewModel {
    pub session_id: SessionId,
    pub title: String,
    pub subtitle: String,
    pub status: BattleCardStatus,
    pub recency_label: String,
    pub intent: Option<IntentSummary>,
    pub observed_summary: String,
    pub file_summary: Option<String>,
    pub output_summary: Option<String>,
    pub correlation: CorrelationSummary,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct IntentContext {
    pub recent_terminal_lines: Vec<String>,
    pub recent_events: Vec<SessionEvent>,
    pub active_command: Option<String>,
    pub dominant_process: Option<String>,
    pub work_output_excerpt: Option<String>,
    pub idle_seconds: Option<u64>,
}

pub trait IntentEngine {
    fn determine_intent(&self, context: &IntentContext) -> Option<IntentSummary>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct DeterministicIntentEngine;

impl IntentEngine for DeterministicIntentEngine {
    fn determine_intent(&self, context: &IntentContext) -> Option<IntentSummary> {
        for line in context.recent_terminal_lines.iter().rev() {
            let normalized = normalize_line(line);
            if normalized.is_empty() || !looks_like_narrative(&normalized) {
                continue;
            }
            return Some(IntentSummary {
                source: IntentSource::Stated,
                text: normalized,
            });
        }

        if let Some(command) = context.active_command.as_ref().filter(|s| !s.is_empty()) {
            return Some(IntentSummary {
                source: IntentSource::Inferred,
                text: format!("Running {command}"),
            });
        }

        if let Some(process) = context.dominant_process.as_ref().filter(|s| !s.is_empty()) {
            return Some(IntentSummary {
                source: IntentSource::Inferred,
                text: format!("Working in {process}"),
            });
        }

        if let Some(idle) = context.idle_seconds {
            return Some(IntentSummary {
                source: IntentSource::Inferred,
                text: format!("No recent visible progress for {}s", idle),
            });
        }

        None
    }
}

pub fn build_battle_card(
    record: &SessionRecord,
    observed: &ObservedActivity,
    terminal_lines: &[String],
    intent_engine: &dyn IntentEngine,
) -> BattleCardViewModel {
    let intent_context = IntentContext {
        recent_terminal_lines: terminal_lines.to_vec(),
        recent_events: record.events.clone(),
        active_command: observed.active_command.clone(),
        dominant_process: observed.dominant_process.clone(),
        work_output_excerpt: observed.work_output_excerpt.clone(),
        idle_seconds: observed.idle_seconds,
    };
    let intent = intent_engine.determine_intent(&intent_context);
    let status = derive_battle_card_status(record.status, observed, intent.as_ref());

    let observed_summary = observed_summary(observed);
    let file_summary = (!observed.recent_files.is_empty())
        .then(|| format!("Files: {}", summarize_files(&observed.recent_files)));
    let output_summary = observed
        .work_output_excerpt
        .as_ref()
        .map(|excerpt| format!("Output: {excerpt}"));
    let correlation = derive_correlation(intent.as_ref(), observed);

    BattleCardViewModel {
        session_id: record.id,
        title: record.launch.name.clone(),
        subtitle: record.launch.subtitle.clone(),
        status,
        recency_label: recency_label(observed.idle_seconds, status),
        intent,
        observed_summary,
        file_summary,
        output_summary,
        correlation,
    }
}

pub fn derive_battle_card_status(
    session_status: SessionStatus,
    observed: &ObservedActivity,
    intent: Option<&IntentSummary>,
) -> BattleCardStatus {
    match session_status {
        SessionStatus::Blocked => BattleCardStatus::Blocked,
        SessionStatus::Failed(_) => BattleCardStatus::Failed,
        SessionStatus::Complete => BattleCardStatus::Complete,
        SessionStatus::Detached => BattleCardStatus::Detached,
        SessionStatus::Launching => BattleCardStatus::Thinking,
        SessionStatus::Waiting => {
            if observed.idle_seconds.unwrap_or_default() >= 30 {
                BattleCardStatus::Idle
            } else {
                BattleCardStatus::Thinking
            }
        }
        SessionStatus::Running => {
            if observed.idle_seconds.unwrap_or_default() >= 30
                && observed.active_command.is_none()
                && observed.dominant_process.is_none()
                && observed.recent_files.is_empty()
            {
                BattleCardStatus::Idle
            } else if observed.active_command.is_some()
                || observed.dominant_process.is_some()
                || observed.work_output_excerpt.is_some()
                || !observed.recent_files.is_empty()
            {
                BattleCardStatus::Working
            } else if intent.is_some() {
                BattleCardStatus::Thinking
            } else {
                BattleCardStatus::Working
            }
        }
    }
}

fn observed_summary(observed: &ObservedActivity) -> String {
    if let Some(command) = observed.active_command.as_ref() {
        return format!("Reality: {command}");
    }
    if let Some(process) = observed.dominant_process.as_ref() {
        return format!("Reality: {process}");
    }
    if let Some(idle) = observed.idle_seconds {
        return format!("Reality: no meaningful activity for {}s", idle);
    }
    "Reality: insufficient runtime evidence yet".into()
}

fn recency_label(idle_seconds: Option<u64>, status: BattleCardStatus) -> String {
    match (status, idle_seconds) {
        (BattleCardStatus::Idle, Some(seconds)) => format!("idle {seconds}s"),
        (_, Some(seconds)) if seconds < 5 => "active now".into(),
        (_, Some(seconds)) => format!("active {seconds}s ago"),
        _ => "recency unknown".into(),
    }
}

fn derive_correlation(
    intent: Option<&IntentSummary>,
    observed: &ObservedActivity,
) -> CorrelationSummary {
    let Some(intent) = intent else {
        return CorrelationSummary {
            narrative: "No recent intent extracted from visible evidence".into(),
            suspicious_mismatch: false,
        };
    };

    let intent_lower = intent.text.to_ascii_lowercase();
    let observed_lower = format!(
        "{} {} {}",
        observed.active_command.as_deref().unwrap_or(""),
        observed.dominant_process.as_deref().unwrap_or(""),
        observed.work_output_excerpt.as_deref().unwrap_or("")
    )
    .to_ascii_lowercase();

    let suspicious_mismatch =
        mentions_test_work(&intent_lower) && !mentions_test_work(&observed_lower)
            || mentions_editing(&intent_lower)
                && observed.recent_files.is_empty()
                && observed.idle_seconds.unwrap_or_default() >= 30;

    let narrative = if suspicious_mismatch {
        format!("Intent and observed activity appear misaligned: {}", intent.text)
    } else {
        format!("Intent and observed activity are plausibly aligned: {}", intent.text)
    };

    CorrelationSummary {
        narrative,
        suspicious_mismatch,
    }
}

fn summarize_files(files: &[String]) -> String {
    const LIMIT: usize = 3;
    if files.len() <= LIMIT {
        return files.join(", ");
    }
    format!("{}, +{} more", files[..LIMIT].join(", "), files.len() - LIMIT)
}

fn looks_like_narrative(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    if line.starts_with('$')
        || line.starts_with("bash-")
        || lower.starts_with("error:")
        || lower.starts_with("warning:")
        || lower.starts_with("running ")
        || lower.contains("heartbeat")
    {
        return false;
    }

    lower.starts_with("intent:")
        || lower.starts_with("now ")
        || lower.starts_with("investigating ")
        || lower.starts_with("updating ")
        || lower.starts_with("fixing ")
        || lower.starts_with("checking ")
        || lower.starts_with("inspecting ")
        || lower.starts_with("need to ")
        || lower.starts_with("i need to ")
        || lower.starts_with("i'm ")
        || lower.starts_with("i am ")
        || lower.starts_with("next:")
        || lower.contains("going to")
}

fn normalize_line(line: &str) -> String {
    let trimmed = strip_prompt_prefix(line.trim());
    trimmed
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_matches('"')
        .to_string()
}

fn strip_prompt_prefix(line: &str) -> &str {
    if let Some(index) = line.rfind("$ ") {
        let suffix = &line[index + 2..];
        if suffix.starts_with("i ")
            || suffix.starts_with("I ")
            || suffix.starts_with("now ")
            || suffix.starts_with("Intent:")
        {
            return suffix;
        }
    }
    line
}

fn mentions_test_work(text: &str) -> bool {
    text.contains("test") || text.contains("pytest") || text.contains("cargo test")
}

fn mentions_editing(text: &str) -> bool {
    text.contains("edit") || text.contains("updat") || text.contains("fix")
}

#[cfg(test)]
mod tests {
    use super::{
        build_battle_card, derive_battle_card_status, BattleCardStatus, DeterministicIntentEngine,
        IntentEngine, IntentSource, ObservedActivity,
    };
    use crate::model::{SessionId, SessionKind, SessionLaunch, SessionRecord, SessionStatus};

    fn record(status: SessionStatus) -> SessionRecord {
        SessionRecord {
            id: SessionId(1),
            launch: SessionLaunch::command(
                "Agent 1",
                "Parser fix",
                SessionKind::RunningStream,
                "/usr/bin/env",
                vec!["bash".into()],
            ),
            status,
            pid: Some(4242),
            events: Vec::new(),
        }
    }

    #[test]
    fn deterministic_engine_prefers_recent_stated_intent() {
        let engine = DeterministicIntentEngine;
        let intent = engine
            .determine_intent(&super::IntentContext {
                recent_terminal_lines: vec![
                    "heartbeat 001".into(),
                    "Now rerunning the parser tests after the last fix.".into(),
                ],
                ..Default::default()
            })
            .expect("intent should be extracted");

        assert_eq!(intent.source, IntentSource::Stated);
        assert!(intent.text.contains("rerunning the parser tests"));
    }

    #[test]
    fn deterministic_engine_falls_back_to_command_inference() {
        let engine = DeterministicIntentEngine;
        let intent = engine
            .determine_intent(&super::IntentContext {
                active_command: Some("cargo test parser".into()),
                ..Default::default()
            })
            .expect("inferred intent should exist");

        assert_eq!(intent.source, IntentSource::Inferred);
        assert_eq!(intent.text, "Running cargo test parser");
    }

    #[test]
    fn battle_card_status_becomes_idle_after_quiet_running_period() {
        let status = derive_battle_card_status(
            SessionStatus::Running,
            &ObservedActivity {
                idle_seconds: Some(48),
                ..Default::default()
            },
            None,
        );

        assert_eq!(status, BattleCardStatus::Idle);
    }

    #[test]
    fn battle_card_status_marks_blocked_and_failed_directly() {
        assert_eq!(
            derive_battle_card_status(SessionStatus::Blocked, &ObservedActivity::default(), None),
            BattleCardStatus::Blocked
        );
        assert_eq!(
            derive_battle_card_status(
                SessionStatus::Failed(2),
                &ObservedActivity::default(),
                None
            ),
            BattleCardStatus::Failed
        );
    }

    #[test]
    fn battle_card_view_model_captures_mismatch_signal() {
        let card = build_battle_card(
            &record(SessionStatus::Running),
            &ObservedActivity {
                idle_seconds: Some(52),
                work_output_excerpt: Some("3 parser tests still failing".into()),
                ..Default::default()
            },
            &["Now rerunning the parser tests.".into()],
            &DeterministicIntentEngine,
        );

        assert_eq!(card.status, BattleCardStatus::Idle);
        assert!(card.correlation.narrative.contains("aligned"));
        assert_eq!(card.recency_label, "idle 52s");
        assert_eq!(
            card.intent.expect("intent should exist").source,
            IntentSource::Stated
        );
    }

    #[test]
    fn battle_card_view_model_flags_editing_claim_without_file_activity() {
        let card = build_battle_card(
            &record(SessionStatus::Running),
            &ObservedActivity {
                idle_seconds: Some(61),
                ..Default::default()
            },
            &["Updating parser.rs and rerunning checks.".into()],
            &DeterministicIntentEngine,
        );

        assert!(card.correlation.suspicious_mismatch);
    }
}
