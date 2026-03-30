use crate::model::{SessionKind, SessionLaunch, SessionRecord};
use crate::runtime::StreamRuntimeUpdate;
use crate::synthesis::{NamingEvidence, NudgeEvidence, TacticalEvidence, TacticalSynthesis};
use std::collections::BTreeMap;
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
        }
    }
}

impl Default for SessionObservation {
    fn default() -> Self {
        Self::new()
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

pub fn apply_file_activity(
    observation: &mut SessionObservation,
    relative_path: String,
    seen_at: Instant,
) {
    observation
        .recent_file_activity
        .insert(relative_path, seen_at);
    observation
        .recent_file_activity
        .retain(|_, at| seen_at.duration_since(*at) <= Duration::from_secs(12));
    let mut recent_files = observation
        .recent_file_activity
        .iter()
        .map(|(path, at)| (path.clone(), *at))
        .collect::<Vec<_>>();
    recent_files.sort_by_key(|(_, at)| std::cmp::Reverse(*at));
    observation.recent_files = recent_files
        .into_iter()
        .map(|(path, _)| path)
        .take(2)
        .collect();
}

pub fn clear_file_activity(observation: &mut SessionObservation) {
    observation.recent_files.clear();
    observation.recent_file_activity.clear();
}

pub fn is_bare_waiting_shell(session: &SessionRecord, observation: &SessionObservation) -> bool {
    session.launch.kind == SessionKind::WaitingShell && observation.shell_child_command.is_none()
}

pub fn refresh_observation(
    observation: &mut SessionObservation,
    session: &SessionRecord,
    remote_mode: bool,
) {
    let refresh = compute_observation_refresh(session, remote_mode);
    apply_observation_refresh(observation, session, refresh);
}

pub fn apply_observation_refresh(
    observation: &mut SessionObservation,
    session: &SessionRecord,
    refresh: ObservationRefreshResult,
) {
    observation.shell_child_command = refresh.shell_child_command;
    observation.dominant_process = refresh.dominant_process;
    observation.process_tree_excerpt = refresh.process_tree_excerpt;
    observation.active_command = infer_active_command_from_lines(&observation.recent_lines)
        .or(observation.shell_child_command.clone())
        .or(observation.dominant_process.clone())
        .or_else(|| launch_command_hint(&session.launch));
    observation.work_output_excerpt = observation.painted_line.clone().or_else(|| {
        observation
            .recent_lines
            .iter()
            .rev()
            .find(|line| is_meaningful_output_line(line))
            .cloned()
    });
}

#[derive(Clone)]
pub struct ObservationRefreshResult {
    pub shell_child_command: Option<String>,
    pub dominant_process: Option<String>,
    pub process_tree_excerpt: Option<String>,
}

pub fn compute_observation_refresh(
    session: &SessionRecord,
    remote_mode: bool,
) -> ObservationRefreshResult {
    let (dominant_process, shell_child_command, process_tree_excerpt) = if remote_mode {
        (None, None, None)
    } else {
        session
            .pid
            .map(read_process_hints)
            .unwrap_or((None, None, None))
    };

    ObservationRefreshResult {
        shell_child_command,
        dominant_process,
        process_tree_excerpt,
    }
}

pub fn effective_display_name(session: &SessionRecord) -> String {
    session
        .display_name
        .clone()
        .unwrap_or_else(|| session.launch.name.clone())
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
        terminal_status_line: observation.painted_line.clone(),
        terminal_status_line_age: Some(relative_age_label(observation.last_change.elapsed())),
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
        attention_brief: summary.attention_brief.clone(),
        headline: summary.headline.clone(),
        recent_terminal_history: nudge_terminal_history(observation),
    }
}

pub fn synthesis_terminal_activity(observation: &SessionObservation) -> Vec<String> {
    let mut entries = model_terminal_history_window(observation);

    if let Some(painted) = observation.painted_line.as_deref() {
        let trimmed = painted.trim();
        if !trimmed.is_empty() {
            entries.push(format!("[most recent updated line] {trimmed}"));
        }
    }

    entries
}

pub fn naming_terminal_history(observation: &SessionObservation) -> Vec<String> {
    model_terminal_history_window(observation)
}

pub fn nudge_terminal_history(observation: &SessionObservation) -> Vec<String> {
    model_terminal_history_window(observation)
}

pub fn scrollback_fragments(observation: &SessionObservation, limit: usize) -> Vec<String> {
    observation
        .recent_lines
        .iter()
        .rev()
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
        .take(limit)
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

    const MAX_ACTIVITY_LINES: usize = 4096;
    if activity.len() > MAX_ACTIVITY_LINES {
        let extra = activity.len() - MAX_ACTIVITY_LINES;
        activity.drain(0..extra);
    }
}

fn model_terminal_history_window(observation: &SessionObservation) -> Vec<String> {
    const MODEL_HISTORY_MIN_LINES: usize = 256;
    const MODEL_HISTORY_MIN_AGE: Duration = Duration::from_secs(5 * 60);

    let now = Instant::now();
    let total = observation.terminal_activity.len();
    if total == 0 {
        return Vec::new();
    }

    let line_start = total.saturating_sub(MODEL_HISTORY_MIN_LINES);
    let time_start = observation
        .terminal_activity
        .iter()
        .position(|entry| now.duration_since(entry.at) <= MODEL_HISTORY_MIN_AGE)
        .unwrap_or(total.saturating_sub(1));
    let start = line_start.min(time_start);

    observation.terminal_activity[start..]
        .iter()
        .map(|entry| format_terminal_activity_entry(entry, now))
        .collect()
}

pub fn find_git_worktree_root(start: &Path) -> Option<PathBuf> {
    let mut current = if start.is_absolute() {
        start.to_path_buf()
    } else {
        std::env::current_dir().ok()?.join(start)
    };

    loop {
        let dot_git = current.join(".git");
        if dot_git.is_dir() || dot_git.is_file() {
            return Some(current);
        }
        if !current.pop() {
            return None;
        }
    }
}

fn format_terminal_activity_entry(entry: &TerminalActivityEntry, now: Instant) -> String {
    format!(
        "[{}] {}",
        relative_age_label(now.duration_since(entry.at)),
        entry.text
    )
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

fn read_process_hints(pid: u32) -> (Option<String>, Option<String>, Option<String>) {
    let reader = crate::process::default_reader();
    let entries = match reader.read_process_table() {
        Ok(e) => e,
        Err(_) => return (None, None, None),
    };
    let dominant = crate::process::dominant_child_command_from_entries(&entries, pid)
        .map(|c| c.replace("  ", " ").trim().to_string());
    let direct = crate::process::direct_child_command_from_entries(&entries, pid)
        .map(|c| c.replace("  ", " ").trim().to_string());
    let tree = crate::process::format_process_tree_from_entries(&entries, pid);
    let tree = {
        let t = tree
            .lines()
            .take(4)
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect::<Vec<_>>()
            .join(" | ");
        if t.is_empty() { None } else { Some(t) }
    };
    (dominant, direct, tree)
}

fn is_meaningful_output_line(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    !line.starts_with("bash-") && !line.starts_with('$') && !lower.starts_with("intent:")
}

#[cfg(test)]
mod tests {
    use super::{
        SessionObservation, TerminalActivityEntry, append_recent_lines, apply_file_activity,
        compute_observation_refresh, effective_display_name, find_git_worktree_root,
        is_bare_waiting_shell, naming_terminal_history, synthesis_terminal_activity,
    };
    use crate::model::{
        SessionId, SessionKind, SessionLaunch, SessionRecord, SessionStatus, user_shell_launch,
    };
    use std::fs;
    use std::path::PathBuf;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    #[test]
    fn recent_lines_accumulate_semantic_output_without_duplicates() {
        let mut recent = vec!["first".to_string()];
        append_recent_lines(
            &mut recent,
            &[
                "first".to_string(),
                "second".to_string(),
                "second".to_string(),
            ],
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
    fn synthesis_activity_keeps_at_least_256_lines_when_recent_activity_is_short() {
        let mut observation = SessionObservation::new();
        let now = Instant::now();
        observation.terminal_activity = (0..300)
            .map(|index| TerminalActivityEntry {
                at: now - Duration::from_secs(((300 - index) * 2) as u64),
                text: format!("line {index}"),
            })
            .collect();

        let history = synthesis_terminal_activity(&observation);
        assert_eq!(history.len(), 256);
        assert!(
            history
                .first()
                .is_some_and(|line| line.ends_with("line 44"))
        );
        assert!(
            history
                .last()
                .is_some_and(|line| line.ends_with("line 299"))
        );
    }

    #[test]
    fn model_history_keeps_more_than_256_lines_when_five_minutes_is_larger() {
        let mut observation = SessionObservation::new();
        let now = Instant::now();
        observation.terminal_activity = (0..400)
            .map(|index| TerminalActivityEntry {
                at: now - Duration::from_secs((400 - index) as u64),
                text: format!("line {index}"),
            })
            .collect();

        let history = naming_terminal_history(&observation);
        assert_eq!(history.len(), 299);
        assert!(
            history
                .first()
                .is_some_and(|line| line.ends_with("line 101"))
        );
        assert!(
            history
                .last()
                .is_some_and(|line| line.ends_with("line 399"))
        );
    }

    #[test]
    fn effective_display_name_prefers_override_then_launch_name() {
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
        assert_eq!(effective_display_name(&session), "Shell 1");

        let named_session = SessionRecord {
            display_name: Some("Parser repair".into()),
            ..session
        };
        assert_eq!(effective_display_name(&named_session), "Parser repair");
    }

    #[test]
    fn finds_git_directory_root_from_nested_workspace_path() {
        let root = tempdir_path("exaterm-observation-git-dir");
        let nested = root.join("src/lib");
        fs::create_dir_all(&nested).expect("nested dir");
        fs::create_dir_all(root.join(".git")).expect("git dir");

        assert_eq!(find_git_worktree_root(&nested), Some(root));
    }

    #[test]
    fn finds_git_file_root_for_worktree_style_layout() {
        let root = tempdir_path("exaterm-observation-git-file");
        let nested = root.join("pkg/app");
        fs::create_dir_all(&nested).expect("nested dir");
        fs::write(root.join(".git"), "gitdir: /tmp/fake-worktree").expect("git file");

        assert_eq!(find_git_worktree_root(&nested), Some(root));
    }

    #[test]
    fn returns_none_when_path_is_not_inside_git_worktree() {
        let root = tempdir_path("exaterm-observation-no-git");
        let nested = root.join("plain/home");
        fs::create_dir_all(&nested).expect("nested dir");

        assert_eq!(find_git_worktree_root(&nested), None);
    }

    #[test]
    fn apply_file_activity_keeps_most_recent_two_paths() {
        let mut observation = SessionObservation::new();
        let base = Instant::now();
        apply_file_activity(&mut observation, "one.rs".to_string(), base);
        apply_file_activity(
            &mut observation,
            "two.rs".to_string(),
            base + Duration::from_secs(1),
        );
        apply_file_activity(
            &mut observation,
            "three.rs".to_string(),
            base + Duration::from_secs(2),
        );

        assert_eq!(
            observation.recent_files,
            vec!["three.rs".to_string(), "two.rs".to_string()]
        );
    }

    #[test]
    fn compute_observation_refresh_has_no_file_activity_payload() {
        let session =
            session_record_with_cwd(SessionId(42), tempdir_path("exaterm-observation-refresh"));
        let refresh = compute_observation_refresh(&session, false);
        assert!(refresh.shell_child_command.is_none());
        assert!(refresh.dominant_process.is_none());
    }

    #[test]
    fn bare_waiting_shell_detects_shell_without_subprocesses() {
        let session = session_record_with_cwd(
            SessionId(42),
            tempdir_path("exaterm-observation-bare-shell"),
        );
        let observation = SessionObservation::new();

        assert!(is_bare_waiting_shell(&session, &observation));
    }

    #[test]
    fn bare_waiting_shell_rejects_shell_with_direct_child() {
        let session = session_record_with_cwd(
            SessionId(42),
            tempdir_path("exaterm-observation-active-shell"),
        );
        let mut observation = SessionObservation::new();
        observation.shell_child_command = Some("codex".into());

        assert!(!is_bare_waiting_shell(&session, &observation));
    }

    #[test]
    fn bare_waiting_shell_ignores_terminal_evidence_without_subprocesses() {
        let session = session_record_with_cwd(
            SessionId(42),
            tempdir_path("exaterm-observation-terminal-evidence"),
        );
        let mut observation = SessionObservation::new();
        observation.active_command = Some("codex".into());
        observation.work_output_excerpt = Some("Updating parser".into());

        assert!(is_bare_waiting_shell(&session, &observation));
    }

    #[test]
    fn apply_file_activity_deduplicates_path_to_latest_timestamp() {
        let mut observation = SessionObservation::new();
        let base = Instant::now();
        apply_file_activity(&mut observation, "same.rs".to_string(), base);
        apply_file_activity(
            &mut observation,
            "same.rs".to_string(),
            base + Duration::from_secs(1),
        );

        assert_eq!(observation.recent_files, vec!["same.rs".to_string()]);
        assert_eq!(observation.recent_file_activity.len(), 1);
    }

    #[test]
    fn apply_file_activity_prunes_stale_entries() {
        let mut observation = SessionObservation::new();
        let base = Instant::now();
        apply_file_activity(
            &mut observation,
            "old.rs".to_string(),
            base - Duration::from_secs(13),
        );
        apply_file_activity(&mut observation, "fresh.rs".to_string(), base);

        assert_eq!(observation.recent_files, vec!["fresh.rs".to_string()]);
        assert!(!observation.recent_file_activity.contains_key("old.rs"));
    }

    fn tempdir_path(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should be monotonic enough for temp dir names")
            .as_nanos();
        let unique = format!("{}-{}-{}", prefix, std::process::id(), nanos);
        let path = std::env::temp_dir().join(unique);
        fs::create_dir_all(&path).expect("temp dir");
        path
    }

    fn session_record_with_cwd(session_id: SessionId, cwd: PathBuf) -> SessionRecord {
        let launch = user_shell_launch("Shell", "test shell").with_cwd(cwd);
        SessionRecord {
            id: session_id,
            launch,
            display_name: None,
            status: SessionStatus::Waiting,
            pid: None,
            events: Vec::new(),
        }
    }
}
