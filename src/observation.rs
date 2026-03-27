use crate::model::{SessionKind, SessionLaunch, SessionRecord};
use crate::runtime::StreamRuntimeUpdate;
use crate::synthesis::{NamingEvidence, NudgeEvidence, TacticalEvidence, TacticalSynthesis};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[derive(Clone)]
pub struct SessionObservation {
    pub last_change: Instant,
    pub recent_lines: Vec<String>,
    pub terminal_activity: Vec<TerminalActivityEntry>,
    pub painted_line: Option<String>,
    pub shell_child_command: Option<String>,
    pub active_command: Option<String>,
    pub dominant_process: Option<String>,
    pub process_tree_excerpt: Option<String>,
    pub recent_files: Vec<String>,
    pub recent_file_activity: BTreeMap<String, Instant>,
    pub work_output_excerpt: Option<String>,
    pub file_fingerprints: BTreeMap<PathBuf, (u64, u64)>,
}

#[derive(Clone)]
pub struct TerminalActivityEntry {
    pub at: Instant,
    pub text: String,
}

impl SessionObservation {
    pub fn new() -> Self {
        Self {
            last_change: Instant::now(),
            recent_lines: Vec::new(),
            terminal_activity: Vec::new(),
            painted_line: None,
            shell_child_command: None,
            active_command: None,
            dominant_process: None,
            process_tree_excerpt: None,
            recent_files: Vec::new(),
            recent_file_activity: BTreeMap::new(),
            work_output_excerpt: None,
            file_fingerprints: BTreeMap::new(),
        }
    }
}

pub fn apply_stream_update(observation: &mut SessionObservation, update: StreamRuntimeUpdate) {
    append_recent_lines(&mut observation.recent_lines, &update.semantic_lines);
    append_terminal_activity(&mut observation.terminal_activity, &update.semantic_lines);
    if let Some(painted_line) = update.painted_line {
        let changed = observation.painted_line.as_ref() != Some(&painted_line);
        observation.painted_line = Some(painted_line);
        if changed {
            observation.last_change = Instant::now();
        }
    } else if !update.semantic_lines.is_empty() && observation.painted_line.is_none() {
        observation.last_change = Instant::now();
    }
}

pub fn refresh_observation(
    observation: &mut SessionObservation,
    session: &SessionRecord,
    remote_mode: bool,
) {
    let shell_child_command = if remote_mode {
        None
    } else {
        session.pid.and_then(read_shell_child_command)
    };
    let dominant_process = if remote_mode {
        None
    } else {
        session.pid.and_then(read_dominant_process_hint)
    };
    let process_tree_excerpt = if remote_mode {
        None
    } else {
        session.pid.and_then(read_process_tree_hint)
    };

    let active_command = infer_active_command_from_lines(&observation.recent_lines)
        .or(shell_child_command.clone())
        .or(dominant_process.clone())
        .or_else(|| launch_command_hint(&session.launch));
    observation.shell_child_command = shell_child_command;
    observation.dominant_process = dominant_process;
    observation.active_command = active_command;
    observation.process_tree_excerpt = process_tree_excerpt;
    observation.work_output_excerpt = observation.painted_line.clone().or_else(|| {
        observation
            .recent_lines
            .iter()
            .rev()
            .find(|line| is_meaningful_output_line(line))
            .cloned()
    });
    let changed_files = if remote_mode {
        Vec::new()
    } else {
        session
            .launch
            .cwd
            .as_deref()
            .map(|cwd| scan_recent_files(cwd, &mut observation.file_fingerprints))
            .unwrap_or_default()
    };
    let now = Instant::now();
    for file in changed_files {
        observation.recent_file_activity.insert(file, now);
    }
    observation
        .recent_file_activity
        .retain(|_, seen_at| seen_at.elapsed() <= Duration::from_secs(12));
    let mut recent_files = observation
        .recent_file_activity
        .iter()
        .map(|(path, seen_at)| (path.clone(), *seen_at))
        .collect::<Vec<_>>();
    recent_files.sort_by_key(|(_, seen_at)| std::cmp::Reverse(*seen_at));
    observation.recent_files = recent_files
        .into_iter()
        .map(|(path, _)| path)
        .take(2)
        .collect();
}

pub fn effective_display_name(session: &SessionRecord) -> String {
    session
        .display_name
        .clone()
        .unwrap_or_else(|| "New Session".into())
}

pub fn build_tactical_evidence(
    session: &SessionRecord,
    observation: &SessionObservation,
) -> TacticalEvidence {
    TacticalEvidence {
        session_name: effective_display_name(session),
        task_label: session.launch.subtitle.clone(),
        dominant_process: observation.dominant_process.clone(),
        process_tree_excerpt: observation.process_tree_excerpt.clone(),
        recent_files: observation.recent_files.clone(),
        work_output_excerpt: observation.painted_line.clone(),
        current_time: Some("now".into()),
        idle_seconds: Some(observation.last_change.elapsed().as_secs()),
        last_update_age: Some(relative_age_label(observation.last_change.elapsed())),
        recent_terminal_activity: synthesis_terminal_activity(observation),
        recent_events: session
            .events
            .iter()
            .rev()
            .filter(|event| is_runtime_event(&event.summary))
            .take(4)
            .map(|event| event.summary.clone())
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect(),
    }
}

pub fn build_naming_evidence(
    session: &SessionRecord,
    observation: &SessionObservation,
) -> NamingEvidence {
    NamingEvidence {
        current_name: session.display_name.clone().unwrap_or_default(),
        recent_terminal_history: naming_terminal_history(observation),
    }
}

pub fn build_nudge_evidence(
    session: &SessionRecord,
    observation: &SessionObservation,
    summary: &TacticalSynthesis,
) -> NudgeEvidence {
    NudgeEvidence {
        session_name: effective_display_name(session),
        shell_child_command: observation.shell_child_command.clone(),
        idle_seconds: Some(observation.last_change.elapsed().as_secs()),
        tactical_state_brief: summary.tactical_state_brief.clone(),
        progress_state_brief: summary.progress_state_brief.clone(),
        momentum_state_brief: summary.momentum_state_brief.clone(),
        terse_operator_summary: summary.terse_operator_summary.clone(),
        recent_terminal_history: nudge_terminal_history(observation),
    }
}

pub fn synthesis_terminal_activity(observation: &SessionObservation) -> Vec<String> {
    const SUMMARY_ACTIVITY_HISTORY_WINDOW: usize = 100;

    let mut entries = Vec::new();
    let now = Instant::now();

    entries.extend(
        observation
            .terminal_activity
            .iter()
            .rev()
            .take(SUMMARY_ACTIVITY_HISTORY_WINDOW)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .map(|entry| format_terminal_activity_entry(entry, now)),
    );

    if let Some(painted) = observation.painted_line.as_deref() {
        let trimmed = painted.trim();
        if !trimmed.is_empty() {
            entries.push(format!("[most recent updated line] {trimmed}"));
        }
    }

    entries
}

pub fn naming_terminal_history(observation: &SessionObservation) -> Vec<String> {
    let now = Instant::now();
    observation
        .terminal_activity
        .iter()
        .rev()
        .take(80)
        .map(|entry| format_terminal_activity_entry(entry, now))
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

pub fn nudge_terminal_history(observation: &SessionObservation) -> Vec<String> {
    let now = Instant::now();
    observation
        .terminal_activity
        .iter()
        .rev()
        .take(120)
        .map(|entry| format_terminal_activity_entry(entry, now))
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

pub fn scrollback_fragments(observation: &SessionObservation) -> Vec<String> {
    observation
        .recent_lines
        .iter()
        .rev()
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
        .take(3)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

pub fn append_recent_lines(recent_lines: &mut Vec<String>, candidate_lines: &[String]) {
    for line in candidate_lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if recent_lines
            .last()
            .is_some_and(|existing| existing == trimmed)
        {
            continue;
        }
        recent_lines.push(trimmed.to_string());
    }

    const MAX_RECENT_LINES_WINDOW: usize = 24;
    if recent_lines.len() > MAX_RECENT_LINES_WINDOW {
        let extra = recent_lines.len() - MAX_RECENT_LINES_WINDOW;
        recent_lines.drain(0..extra);
    }
}

fn append_terminal_activity(activity: &mut Vec<TerminalActivityEntry>, candidate_lines: &[String]) {
    if candidate_lines.is_empty() {
        return;
    }

    let trailing_payloads = activity
        .iter()
        .rev()
        .take(candidate_lines.len())
        .map(|entry| entry.text.clone())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>();

    if trailing_payloads == candidate_lines {
        return;
    }

    for line in candidate_lines {
        activity.push(TerminalActivityEntry {
            at: Instant::now(),
            text: line.trim().to_string(),
        });
    }

    const MAX_ACTIVITY_LINES: usize = 320;
    if activity.len() > MAX_ACTIVITY_LINES {
        let extra = activity.len() - MAX_ACTIVITY_LINES;
        activity.drain(0..extra);
    }
}

fn scan_recent_files(root: &Path, fingerprints: &mut BTreeMap<PathBuf, (u64, u64)>) -> Vec<String> {
    let mut current = BTreeMap::new();
    let mut changed = Vec::new();
    collect_file_changes(root, root, fingerprints, &mut current, &mut changed);
    *fingerprints = current;
    changed.truncate(2);
    changed
}

fn collect_file_changes(
    root: &Path,
    path: &Path,
    previous: &BTreeMap<PathBuf, (u64, u64)>,
    current: &mut BTreeMap<PathBuf, (u64, u64)>,
    changed: &mut Vec<String>,
) {
    let Ok(entries) = fs::read_dir(path) else {
        return;
    };

    for entry in entries.flatten() {
        let entry_path = entry.path();
        let Ok(metadata) = entry.metadata() else {
            continue;
        };

        if metadata.is_dir() {
            collect_file_changes(root, &entry_path, previous, current, changed);
            continue;
        }

        let modified = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs())
            .unwrap_or_default();
        let signature = (modified, metadata.len());
        current.insert(entry_path.clone(), signature);

        let changed_now = previous
            .get(&entry_path)
            .map(|existing| *existing != signature)
            .unwrap_or(true);

        if changed_now {
            if let Ok(relative) = entry_path.strip_prefix(root) {
                changed.push(relative.display().to_string());
            }
        }
    }
}

fn format_terminal_activity_entry(entry: &TerminalActivityEntry, now: Instant) -> String {
    format!("[{}] {}", relative_age_label(now.duration_since(entry.at)), entry.text)
}

fn relative_age_label(duration: Duration) -> String {
    let seconds = duration.as_secs();
    match seconds {
        0..=59 => format!("{seconds}s ago"),
        60..=3599 => format!("{}m ago", seconds / 60),
        _ => format!("{}h ago", seconds / 3600),
    }
}

fn is_runtime_event(summary: &str) -> bool {
    !matches!(
        summary,
        "Entered focused terminal view"
            | "Returned to battlefield view"
            | "Probe opened"
            | "Probe closed"
            | "Probe pinned for ongoing watch"
            | "Probe returned to peek mode"
    ) && !summary.starts_with("Probe switched to ")
}

fn launch_command_hint(launch: &SessionLaunch) -> Option<String> {
    match launch.kind {
        SessionKind::WaitingShell => Some("Interactive shell ready".into()),
        SessionKind::PlanningStream => None,
        SessionKind::BlockingPrompt => Some("Waiting on approval prompt".into()),
        SessionKind::RunningStream => Some("cargo test parser".into()),
        SessionKind::FailingTask => Some("Task exited after failure".into()),
    }
}

fn infer_active_command_from_lines(lines: &[String]) -> Option<String> {
    lines.iter().rev().find_map(|line| {
        let trimmed = line.trim();
        if let Some(command) = trimmed.strip_prefix("$ ") {
            let command = command.trim();
            return (!command.is_empty()).then(|| command.to_string());
        }
        None
    })
}

fn read_dominant_process_hint(pid: u32) -> Option<String> {
    crate::procfs::dominant_child_command(pid)
        .ok()
        .flatten()
        .map(|command| command.replace("  ", " ").trim().to_string())
}

fn read_shell_child_command(pid: u32) -> Option<String> {
    crate::procfs::direct_child_command(pid)
        .ok()
        .flatten()
        .map(|command| command.replace("  ", " ").trim().to_string())
}

fn read_process_tree_hint(pid: u32) -> Option<String> {
    crate::procfs::format_process_tree(pid)
        .ok()
        .map(|tree| {
            tree.lines()
                .take(4)
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .collect::<Vec<_>>()
                .join(" | ")
        })
        .filter(|tree| !tree.is_empty())
}

fn is_meaningful_output_line(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    !line.starts_with("bash-") && !line.starts_with('$') && !lower.starts_with("intent:")
}

#[cfg(test)]
mod tests {
    use super::{
        append_recent_lines, effective_display_name, naming_terminal_history,
        synthesis_terminal_activity, SessionObservation, TerminalActivityEntry,
    };
    use crate::model::{SessionId, SessionKind, SessionLaunch, SessionRecord, SessionStatus};
    use std::time::{Duration, Instant};

    #[test]
    fn recent_lines_accumulate_semantic_output_without_duplicates() {
        let mut recent = vec!["first".to_string()];
        append_recent_lines(
            &mut recent,
            &["first".to_string(), "second".to_string(), "second".to_string()],
        );
        assert_eq!(recent, vec!["first".to_string(), "second".to_string()]);
    }

    #[test]
    fn synthesis_activity_contains_terminal_history_and_most_recent_updated_line() {
        let mut observation = SessionObservation::new();
        let now = Instant::now();
        observation.terminal_activity = vec![
            TerminalActivityEntry {
                at: now - Duration::from_secs(2),
                text: "• Ran cargo test".to_string(),
            },
            TerminalActivityEntry {
                at: now - Duration::from_secs(1),
                text: "test result: ok".to_string(),
            },
        ];
        observation.painted_line = Some("Working 7".to_string());

        let history = synthesis_terminal_activity(&observation);
        assert_eq!(history.len(), 3);
        assert!(history[0].ends_with("• Ran cargo test"));
        assert!(history[1].ends_with("test result: ok"));
        assert_eq!(history[2], "[most recent updated line] Working 7");
    }

    #[test]
    fn synthesis_activity_uses_large_history_window() {
        let mut observation = SessionObservation::new();
        let now = Instant::now();
        observation.terminal_activity = (0..120)
            .map(|index| TerminalActivityEntry {
                at: now - Duration::from_secs((120 - index) as u64),
                text: format!("line {index}"),
            })
            .collect();

        let history = synthesis_terminal_activity(&observation);
        assert_eq!(history.len(), 100);
        assert!(history.first().is_some_and(|line| line.ends_with("line 20")));
        assert!(history.last().is_some_and(|line| line.ends_with("line 119")));
    }

    #[test]
    fn naming_history_uses_large_timestamped_window() {
        let mut observation = SessionObservation::new();
        let now = Instant::now();
        observation.terminal_activity = (0..100)
            .map(|index| TerminalActivityEntry {
                at: now - Duration::from_secs((100 - index) as u64),
                text: format!("line {index}"),
            })
            .collect();

        let history = naming_terminal_history(&observation);
        assert_eq!(history.len(), 80);
        assert!(history.first().is_some_and(|line| line.ends_with("line 20")));
        assert!(history.last().is_some_and(|line| line.ends_with("line 99")));
    }

    #[test]
    fn effective_display_name_prefers_override_then_new_session() {
        let launch = SessionLaunch {
            name: "Shell 1".into(),
            subtitle: "Main".into(),
            program: "/bin/bash".into(),
            args: Vec::new(),
            cwd: None,
            kind: SessionKind::WaitingShell,
        };
        let session = SessionRecord {
            id: SessionId(7),
            launch: launch.clone(),
            display_name: None,
            status: SessionStatus::Waiting,
            pid: None,
            events: Vec::new(),
        };
        assert_eq!(effective_display_name(&session), "New Session");

        let named_session = SessionRecord {
            display_name: Some("Parser repair".into()),
            ..session
        };
        assert_eq!(effective_display_name(&named_session), "Parser repair");
    }
}
