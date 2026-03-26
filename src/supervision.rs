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
    pub primary_detail: Option<String>,
    pub evidence_fragments: Vec<String>,
    pub alignment: AlignmentSignal,
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
    let correlation = derive_correlation(intent.as_ref(), observed);
    let tactical = tactical_copy(record.status, status, observed, intent.as_ref(), &correlation);

    BattleCardViewModel {
        session_id: record.id,
        title: record.launch.name.clone(),
        subtitle: record.launch.subtitle.clone(),
        status,
        recency_label: recency_label(observed.idle_seconds, status),
        headline: tactical.headline,
        primary_detail: tactical.primary_detail,
        evidence_fragments: tactical.evidence_fragments,
        alignment: derive_alignment_signal(status, observed, intent.as_ref(), &correlation),
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
            } else if observed.active_command.is_none()
                && observed.dominant_process.is_none()
                && observed.recent_files.is_empty()
                && intent.is_some()
            {
                BattleCardStatus::Thinking
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

fn recency_label(idle_seconds: Option<u64>, status: BattleCardStatus) -> String {
    match (status, idle_seconds) {
        (BattleCardStatus::Idle, Some(seconds)) => format!("idle {seconds}s"),
        (_, Some(seconds)) if seconds < 5 => "active now".into(),
        (_, Some(seconds)) => format!("active {seconds}s ago"),
        _ => "recency unknown".into(),
    }
}

struct TacticalCopy {
    headline: String,
    primary_detail: Option<String>,
    evidence_fragments: Vec<String>,
}

fn tactical_copy(
    session_status: SessionStatus,
    status: BattleCardStatus,
    observed: &ObservedActivity,
    intent: Option<&IntentSummary>,
    _correlation: &CorrelationSummary,
) -> TacticalCopy {
    let intent_text = intent.map(|intent| intent.text.as_str());
    let command_text = observed.active_command.as_deref();
    let process_text = observed.dominant_process.as_deref();
    let output_text = observed.work_output_excerpt.as_deref();
    let file_text = (!observed.recent_files.is_empty()).then(|| summarize_files(&observed.recent_files));
    let shell_ready = matches!(command_text, Some("Interactive shell ready"));

    let mut tactical = match status {
        BattleCardStatus::Idle => TacticalCopy {
            headline: compact_fragment(
                intent_text
                    .or(command_text)
                    .or(output_text)
                    .unwrap_or("Waiting for the next meaningful step"),
            ),
            primary_detail: output_text
                .filter(|_| session_status != SessionStatus::Blocked)
                .map(compact_fragment)
                .or_else(|| file_text.clone().map(|files| format!("Last touched {files}"))),
            evidence_fragments: Vec::new(),
        },
        BattleCardStatus::Thinking => TacticalCopy {
            headline: compact_fragment(
                if shell_ready {
                    "Terminal is ready for direct intervention"
                } else {
                    intent_text
                        .or(command_text)
                        .or(process_text)
                        .unwrap_or("Working through the next step")
                }
            ),
            primary_detail: output_text.map(compact_fragment),
            evidence_fragments: Vec::new(),
        },
        BattleCardStatus::Working => TacticalCopy {
            headline: compact_fragment(
                command_text
                    .or(process_text)
                    .or(intent_text)
                    .unwrap_or("Concrete work is underway"),
            ),
            primary_detail: intent_text
                .filter(|intent| Some(*intent) != command_text)
                .map(compact_fragment)
                .or_else(|| output_text.map(compact_fragment)),
            evidence_fragments: Vec::new(),
        },
        BattleCardStatus::Blocked => TacticalCopy {
            headline: compact_fragment(
                output_text
                    .or(command_text)
                    .or(intent_text)
                    .unwrap_or("Needs operator input"),
            ),
            primary_detail: Some("Waiting on an explicit unblock".into()),
            evidence_fragments: Vec::new(),
        },
        BattleCardStatus::Failed => TacticalCopy {
            headline: compact_fragment(
                output_text
                    .or(command_text)
                    .or(process_text)
                    .unwrap_or("The last action failed"),
            ),
            primary_detail: Some(match session_status {
                SessionStatus::Failed(code) => format!("Exit code {code}"),
                _ => "Failure needs inspection".into(),
            }),
            evidence_fragments: Vec::new(),
        },
        BattleCardStatus::Complete => TacticalCopy {
            headline: compact_fragment(
                output_text
                    .or(intent_text)
                    .or(command_text)
                    .unwrap_or("Completed"),
            ),
            primary_detail: None,
            evidence_fragments: Vec::new(),
        },
        BattleCardStatus::Detached => TacticalCopy {
            headline: "Session detached".into(),
            primary_detail: Some("Runtime visibility is no longer healthy".into()),
            evidence_fragments: Vec::new(),
        },
    };

    let mut evidence_fragments = Vec::new();
    if let Some(files) = file_text.as_ref() {
        push_unique_fragment(
            &mut evidence_fragments,
            files.clone(),
            &[&tactical.headline, tactical.primary_detail.as_deref().unwrap_or("")],
        );
    }
    if let Some(output) = output_text.filter(|line| !line.is_empty()) {
        push_unique_fragment(
            &mut evidence_fragments,
            compact_fragment(output),
            &[&tactical.headline, tactical.primary_detail.as_deref().unwrap_or("")],
        );
    }
    if evidence_fragments.len() < 2 {
        if let Some(process) = process_text {
            push_unique_fragment(
                &mut evidence_fragments,
                compact_fragment(process),
                &[&tactical.headline, tactical.primary_detail.as_deref().unwrap_or("")],
            );
        }
    }
    evidence_fragments.truncate(2);
    tactical.evidence_fragments = evidence_fragments;

    if tactical
        .primary_detail
        .as_deref()
        .is_some_and(|detail| same_meaning(detail, &tactical.headline))
    {
        tactical.primary_detail = None;
    }

    tactical
}

fn derive_alignment_signal(
    status: BattleCardStatus,
    observed: &ObservedActivity,
    intent: Option<&IntentSummary>,
    correlation: &CorrelationSummary,
) -> AlignmentSignal {
    if correlation.suspicious_mismatch {
        return AlignmentSignal {
            text: compact_fragment(&correlation.narrative),
            tone: SignalTone::Alert,
        };
    }

    let has_files = !observed.recent_files.is_empty();
    let has_output = observed.work_output_excerpt.is_some();
    let has_runtime = observed.active_command.is_some() || observed.dominant_process.is_some();

    match status {
        BattleCardStatus::Blocked => AlignmentSignal {
            text: "Prompt or explicit unblock is visible".into(),
            tone: SignalTone::Alert,
        },
        BattleCardStatus::Failed => AlignmentSignal {
            text: "Failure is explicit in machine output".into(),
            tone: SignalTone::Alert,
        },
        BattleCardStatus::Working if has_files && has_output => AlignmentSignal {
            text: "Files and output both confirm forward progress".into(),
            tone: SignalTone::Calm,
        },
        BattleCardStatus::Working if has_files => AlignmentSignal {
            text: "Recent file changes support the current work".into(),
            tone: SignalTone::Calm,
        },
        BattleCardStatus::Working if has_output || has_runtime => AlignmentSignal {
            text: "Live runtime evidence supports the current work".into(),
            tone: SignalTone::Calm,
        },
        BattleCardStatus::Thinking
            if matches!(observed.active_command.as_deref(), Some("Interactive shell ready")) =>
        {
            AlignmentSignal {
                text: "Ready for direct control".into(),
                tone: SignalTone::Calm,
            }
        }
        BattleCardStatus::Thinking if intent.is_some() => AlignmentSignal {
            text: "Visible planning is ahead of concrete execution".into(),
            tone: SignalTone::Watch,
        },
        BattleCardStatus::Idle if has_files || has_output => AlignmentSignal {
            text: "Recent evidence exists, but progress has gone quiet".into(),
            tone: SignalTone::Watch,
        },
        BattleCardStatus::Idle => AlignmentSignal {
            text: "No concrete execution is visible right now".into(),
            tone: SignalTone::Watch,
        },
        _ => AlignmentSignal {
            text: "Machine evidence is limited in the overview".into(),
            tone: SignalTone::Watch,
        },
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

fn compact_fragment(text: &str) -> String {
    const LIMIT: usize = 72;
    let normalized = normalize_line(text);
    if normalized.chars().count() <= LIMIT {
        return normalized;
    }
    let mut shortened = normalized.chars().take(LIMIT - 1).collect::<String>();
    shortened.push('…');
    shortened
}

fn push_unique_fragment(fragments: &mut Vec<String>, candidate: String, avoid: &[&str]) {
    if candidate.is_empty()
        || fragments.iter().any(|fragment| same_meaning(fragment, &candidate))
        || avoid.iter().any(|item| same_meaning(item, &candidate))
    {
        return;
    }
    fragments.push(candidate);
}

fn same_meaning(left: &str, right: &str) -> bool {
    let normalize = |value: &str| {
        value
            .to_ascii_lowercase()
            .replace("files:", "")
            .replace("output:", "")
            .replace('…', "")
            .replace("last touched", "")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    };
    let left = normalize(left);
    let right = normalize(right);
    left == right || left.contains(&right) || right.contains(&left)
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
        IntentEngine, IntentSource, ObservedActivity, SignalTone,
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
        assert_eq!(card.recency_label, "idle 52s");
        assert!(card.headline.contains("rerunning the parser tests"));
        assert!(card
            .primary_detail
            .as_deref()
            .unwrap_or_default()
            .contains("3 parser tests still failing"));
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

        assert_eq!(card.alignment.tone, SignalTone::Alert);
    }
}
