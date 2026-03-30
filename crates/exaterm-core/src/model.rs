pub use exaterm_types::model::{
    SessionEvent, SessionId, SessionKind, SessionLaunch, SessionRecord, SessionStatus,
};

use std::path::Path;

pub fn shell_launch(
    name: impl Into<String>,
    subtitle: impl Into<String>,
    banner: impl Into<String>,
) -> SessionLaunch {
    let banner = banner.into().replace('\'', r"'\''");
    SessionLaunch {
        name: name.into(),
        subtitle: subtitle.into(),
        program: "/usr/bin/env".into(),
        args: vec![
            "bash".into(),
            "--noprofile".into(),
            "--norc".into(),
            "-ic".into(),
            format!("printf '%s\\r\\n' '{banner}'; exec bash --noprofile --norc -i"),
        ],
        cwd: None,
        kind: SessionKind::WaitingShell,
    }
}

pub fn user_shell_launch(name: impl Into<String>, subtitle: impl Into<String>) -> SessionLaunch {
    let (program, args) = preferred_user_shell_launch();
    SessionLaunch {
        name: name.into(),
        subtitle: subtitle.into(),
        program,
        args,
        cwd: std::env::current_dir().ok(),
        kind: SessionKind::WaitingShell,
    }
}

pub fn ssh_shell_launch(
    name: impl Into<String>,
    subtitle: impl Into<String>,
    target: impl Into<String>,
) -> SessionLaunch {
    SessionLaunch {
        name: name.into(),
        subtitle: subtitle.into(),
        program: "/usr/bin/env".into(),
        args: vec!["ssh".into(), target.into()],
        cwd: None,
        kind: SessionKind::WaitingShell,
    }
}

pub fn running_stream_launch(
    name: impl Into<String>,
    subtitle: impl Into<String>,
    script: impl Into<String>,
) -> SessionLaunch {
    command_launch(
        name,
        subtitle,
        SessionKind::RunningStream,
        "/usr/bin/env",
        vec![
            "bash".into(),
            "--noprofile".into(),
            "--norc".into(),
            "-lc".into(),
            script.into(),
        ],
    )
}

pub fn planning_stream_launch(
    name: impl Into<String>,
    subtitle: impl Into<String>,
    script: impl Into<String>,
) -> SessionLaunch {
    command_launch(
        name,
        subtitle,
        SessionKind::PlanningStream,
        "/usr/bin/env",
        vec![
            "bash".into(),
            "--noprofile".into(),
            "--norc".into(),
            "-lc".into(),
            script.into(),
        ],
    )
}

pub fn blocking_prompt_launch(
    name: impl Into<String>,
    subtitle: impl Into<String>,
    prompt: impl Into<String>,
) -> SessionLaunch {
    let prompt = prompt.into().replace('\'', r"'\''");
    command_launch(
        name,
        subtitle,
        SessionKind::BlockingPrompt,
        "/usr/bin/env",
        vec![
            "bash".into(),
            "--noprofile".into(),
            "--norc".into(),
            "-ic".into(),
            format!(
                "printf '%s\\r\\n' '{prompt}'; read -r approval; printf 'Approved: %s\\r\\n' \"$approval\"; exec bash --noprofile --norc -i"
            ),
        ],
    )
}

pub fn failing_task_launch(
    name: impl Into<String>,
    subtitle: impl Into<String>,
    message: impl Into<String>,
    exit_code: i32,
) -> SessionLaunch {
    let message = message.into().replace('\'', r"'\''");
    command_launch(
        name,
        subtitle,
        SessionKind::FailingTask,
        "/usr/bin/env",
        vec![
            "bash".into(),
            "--noprofile".into(),
            "--norc".into(),
            "-lc".into(),
            format!("printf '%s\\r\\n' '{message}'; exit {exit_code}"),
        ],
    )
}

pub fn command_launch(
    name: impl Into<String>,
    subtitle: impl Into<String>,
    kind: SessionKind,
    program: impl Into<String>,
    args: Vec<String>,
) -> SessionLaunch {
    SessionLaunch {
        name: name.into(),
        subtitle: subtitle.into(),
        program: program.into(),
        args,
        cwd: None,
        kind,
    }
}

pub fn launch_argv(launch: &SessionLaunch) -> Vec<String> {
    std::iter::once(launch.program.clone())
        .chain(launch.args.iter().cloned())
        .collect()
}

pub fn session_status_hint(launch: &SessionLaunch, status: SessionStatus) -> String {
    match status {
        SessionStatus::Launching => "Starting session".into(),
        SessionStatus::Running => match launch.kind {
            SessionKind::FailingTask => "Running until failure signal".into(),
            SessionKind::PlanningStream => "Visible planning narrative".into(),
            _ => "Actively producing terminal activity".into(),
        },
        SessionStatus::Waiting => "Interactive shell ready".into(),
        SessionStatus::Blocked => "Session stopped pending human intervention".into(),
        SessionStatus::Failed(code) => format!("Exited with code {code}"),
        SessionStatus::Complete => "Process exited cleanly".into(),
        SessionStatus::Detached => "Runtime disconnected".into(),
    }
}

fn preferred_user_shell_launch() -> (String, Vec<String>) {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".into());
    let shell_name = Path::new(&shell)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("bash");

    let mode = std::env::var("EXATERM_SHELL_MODE").unwrap_or_else(|_| "interactive".into());
    let args = preferred_user_shell_args(shell_name, &mode);

    (shell, args)
}

fn preferred_user_shell_args(shell_name: &str, mode: &str) -> Vec<String> {
    match (shell_name, mode) {
        ("bash", "login") => vec!["-il".into()],
        ("zsh", "login") => vec!["-il".into()],
        ("fish", "login") => vec!["--interactive".into(), "--login".into()],
        ("bash", _) => vec!["-i".into()],
        ("zsh", _) => vec!["-i".into()],
        ("fish", _) => vec!["--interactive".into()],
        (_, _) => vec!["-i".into()],
    }
}

#[derive(Debug, Default)]
pub struct WorkspaceStore {
    next_session_id: u32,
    next_event_sequence: u64,
    sessions: Vec<SessionRecord>,
}

impl WorkspaceStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn replace_sessions(&mut self, sessions: Vec<SessionRecord>) {
        self.next_session_id = sessions
            .iter()
            .map(|session| session.id.0)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        self.next_event_sequence = sessions
            .iter()
            .flat_map(|session| session.events.iter().map(|event| event.sequence))
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        self.sessions = sessions;
    }

    pub fn add_session(&mut self, launch: SessionLaunch) -> SessionId {
        let id = SessionId(self.next_session_id);
        self.next_session_id += 1;

        self.sessions.push(SessionRecord {
            id,
            launch,
            display_name: None,
            status: SessionStatus::Launching,
            pid: None,
            events: Vec::new(),
        });
        self.push_event(id, "Session added to workspace");
        id
    }

    pub fn sessions(&self) -> &[SessionRecord] {
        &self.sessions
    }

    pub fn session(&self, session_id: SessionId) -> Option<&SessionRecord> {
        self.sessions
            .iter()
            .find(|session| session.id == session_id)
    }

    pub fn set_display_name(&mut self, session_id: SessionId, display_name: Option<String>) {
        let Some(session) = self
            .sessions
            .iter_mut()
            .find(|session| session.id == session_id)
        else {
            return;
        };

        session.display_name = display_name.and_then(|name| {
            let trimmed = name.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        });
    }

    pub fn mark_spawned(&mut self, session_id: SessionId, pid: u32) {
        if let Some(session) = self
            .sessions
            .iter_mut()
            .find(|session| session.id == session_id)
        {
            session.status = session.launch.kind.default_status();
            session.pid = Some(pid);
        }
        self.push_event(session_id, format!("Spawned process {pid}"));
    }

    pub fn mark_exited(&mut self, session_id: SessionId, exit_code: i32) {
        if let Some(session) = self
            .sessions
            .iter_mut()
            .find(|session| session.id == session_id)
        {
            session.status = if exit_code == 0 {
                SessionStatus::Complete
            } else {
                SessionStatus::Failed(exit_code)
            };
            session.pid = None;
        }
        self.push_event(
            session_id,
            if exit_code == 0 {
                "Process exited cleanly".into()
            } else {
                format!("Process exited with code {exit_code}")
            },
        );
    }

    fn push_event(&mut self, session_id: SessionId, summary: impl Into<String>) {
        let Some(session) = self
            .sessions
            .iter_mut()
            .find(|session| session.id == session_id)
        else {
            return;
        };

        session.events.push(SessionEvent {
            sequence: self.next_event_sequence,
            summary: summary.into(),
        });
        self.next_event_sequence += 1;

        const MAX_EVENTS: usize = 16;
        if session.events.len() > MAX_EVENTS {
            let extra = session.events.len() - MAX_EVENTS;
            session.events.drain(0..extra);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        SessionStatus, WorkspaceStore, blocking_prompt_launch, launch_argv,
        preferred_user_shell_args, session_status_hint, shell_launch, ssh_shell_launch,
        user_shell_launch,
    };

    #[test]
    fn user_shell_launch_tracks_current_shell() {
        let launch = user_shell_launch("Shell", "Generic command session");
        let argv = launch_argv(&launch);
        assert_eq!(argv[0], launch.program);
        assert!(argv.len() >= 2);
    }

    #[test]
    fn shell_launch_uses_banner_script() {
        let launch = shell_launch("Smoke", "shell", "hello from smoke");
        let argv = launch_argv(&launch);
        assert!(
            argv.last()
                .is_some_and(|arg| arg.contains("hello from smoke"))
        );
    }

    #[test]
    fn workspace_store_tracks_spawn_and_exit() {
        let mut state = WorkspaceStore::new();
        let session_id = state.add_session(shell_launch("A", "shell", "a"));

        state.mark_spawned(session_id, 99);
        assert_eq!(state.sessions()[0].status, SessionStatus::Waiting);

        state.mark_exited(session_id, 7);
        assert_eq!(state.sessions()[0].status, SessionStatus::Failed(7));
    }

    #[test]
    fn status_hints_describe_terminal_state() {
        let prompt = blocking_prompt_launch("Approval", "prompt", "approve");
        let failed = super::failing_task_launch("Task", "task", "boom", 9);

        assert_eq!(
            session_status_hint(&prompt, SessionStatus::Blocked),
            "Session stopped pending human intervention"
        );
        assert_eq!(
            session_status_hint(&failed, SessionStatus::Failed(9)),
            "Exited with code 9"
        );
    }

    #[test]
    fn ssh_launch_uses_ssh_target() {
        let launch = ssh_shell_launch("A", "shell", "user@example.com");
        assert_eq!(
            launch.args,
            vec!["ssh".to_string(), "user@example.com".to_string()]
        );
    }

    #[test]
    fn preferred_shell_args_favor_interactive_rc_loading() {
        assert_eq!(preferred_user_shell_args("bash", "interactive"), vec!["-i"]);
        assert_eq!(preferred_user_shell_args("zsh", "interactive"), vec!["-i"]);
        assert_eq!(
            preferred_user_shell_args("fish", "interactive"),
            vec!["--interactive"]
        );
    }

    #[test]
    fn login_shell_mode_requests_login_args() {
        assert_eq!(preferred_user_shell_args("bash", "login"), vec!["-il"]);
        assert_eq!(preferred_user_shell_args("zsh", "login"), vec!["-il"]);
        assert_eq!(
            preferred_user_shell_args("fish", "login"),
            vec!["--interactive", "--login"]
        );
    }
}
