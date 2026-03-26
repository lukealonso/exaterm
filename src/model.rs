use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SessionId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionKind {
    WaitingShell,
    RunningStream,
    BlockingPrompt,
    FailingTask,
}

impl SessionKind {
    pub fn default_status(self) -> SessionStatus {
        match self {
            SessionKind::WaitingShell => SessionStatus::Waiting,
            SessionKind::RunningStream => SessionStatus::Running,
            SessionKind::BlockingPrompt => SessionStatus::Blocked,
            SessionKind::FailingTask => SessionStatus::Running,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionLaunch {
    pub name: String,
    pub subtitle: String,
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub kind: SessionKind,
}

impl SessionLaunch {
    pub fn shell(
        name: impl Into<String>,
        subtitle: impl Into<String>,
        banner: impl Into<String>,
    ) -> Self {
        let banner = banner.into().replace('\'', r"'\''");
        Self {
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

    pub fn running_stream(
        name: impl Into<String>,
        subtitle: impl Into<String>,
        script: impl Into<String>,
    ) -> Self {
        Self::command(
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

    pub fn blocking_prompt(
        name: impl Into<String>,
        subtitle: impl Into<String>,
        prompt: impl Into<String>,
    ) -> Self {
        let prompt = prompt.into().replace('\'', r"'\''");
        Self::command(
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

    pub fn failing_task(
        name: impl Into<String>,
        subtitle: impl Into<String>,
        message: impl Into<String>,
        exit_code: i32,
    ) -> Self {
        let message = message.into().replace('\'', r"'\''");
        Self::command(
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

    pub fn command(
        name: impl Into<String>,
        subtitle: impl Into<String>,
        kind: SessionKind,
        program: impl Into<String>,
        args: Vec<String>,
    ) -> Self {
        Self {
            name: name.into(),
            subtitle: subtitle.into(),
            program: program.into(),
            args,
            cwd: None,
            kind,
        }
    }

    pub fn argv(&self) -> Vec<String> {
        std::iter::once(self.program.clone())
            .chain(self.args.iter().cloned())
            .collect()
    }

    pub fn status_hint(&self, status: SessionStatus) -> String {
        match status {
            SessionStatus::Launching => "Starting session".into(),
            SessionStatus::Running => match self.kind {
                SessionKind::FailingTask => "Running until failure signal".into(),
                _ => "Actively producing terminal activity".into(),
            },
            SessionStatus::Waiting => "Ready for direct intervention".into(),
            SessionStatus::Blocked => "Waiting for operator input".into(),
            SessionStatus::Failed(code) => format!("Exited with code {code}"),
            SessionStatus::Complete => "Process exited cleanly".into(),
            SessionStatus::Detached => "Runtime disconnected".into(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionStatus {
    Launching,
    Running,
    Waiting,
    Blocked,
    Failed(i32),
    Complete,
    Detached,
}

impl SessionStatus {
    pub fn chip_label(self) -> String {
        match self {
            SessionStatus::Launching => "Launching".into(),
            SessionStatus::Running => "Running".into(),
            SessionStatus::Waiting => "Waiting".into(),
            SessionStatus::Blocked => "Blocked".into(),
            SessionStatus::Failed(_) => "Failed".into(),
            SessionStatus::Complete => "Complete".into(),
            SessionStatus::Detached => "Detached".into(),
        }
    }

    pub fn css_class(self) -> &'static str {
        match self {
            SessionStatus::Launching => "status-launching",
            SessionStatus::Running => "status-running",
            SessionStatus::Waiting => "status-waiting",
            SessionStatus::Blocked => "status-blocked",
            SessionStatus::Failed(_) => "status-failed",
            SessionStatus::Complete => "status-complete",
            SessionStatus::Detached => "status-detached",
        }
    }

    pub fn needs_attention(self) -> bool {
        matches!(
            self,
            SessionStatus::Blocked | SessionStatus::Failed(_) | SessionStatus::Detached
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionRecord {
    pub id: SessionId,
    pub launch: SessionLaunch,
    pub status: SessionStatus,
    pub pid: Option<u32>,
}

#[derive(Debug, Default)]
pub struct WorkspaceState {
    next_session_id: u32,
    sessions: Vec<SessionRecord>,
    selected_session: Option<SessionId>,
    focused_terminal: Option<SessionId>,
}

impl WorkspaceState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn load_workspace(&mut self, launches: Vec<SessionLaunch>) -> Vec<SessionId> {
        self.next_session_id = 1;
        self.sessions.clear();
        self.selected_session = None;
        self.focused_terminal = None;

        let mut ids = Vec::with_capacity(launches.len());
        for launch in launches {
            ids.push(self.add_session(launch));
        }
        ids
    }

    pub fn add_session(&mut self, launch: SessionLaunch) -> SessionId {
        let id = SessionId(self.next_session_id);
        self.next_session_id += 1;

        self.sessions.push(SessionRecord {
            id,
            launch,
            status: SessionStatus::Launching,
            pid: None,
        });

        self.selected_session.get_or_insert(id);
        id
    }

    pub fn sessions(&self) -> &[SessionRecord] {
        &self.sessions
    }

    pub fn selected_session(&self) -> Option<SessionId> {
        self.selected_session
    }

    pub fn focused_terminal(&self) -> Option<SessionId> {
        self.focused_terminal
    }

    pub fn select_session(&mut self, session_id: SessionId) {
        if self.sessions.iter().any(|session| session.id == session_id) {
            self.selected_session = Some(session_id);
        }
    }

    pub fn set_terminal_focus(&mut self, session_id: Option<SessionId>) {
        self.focused_terminal =
            session_id.filter(|id| self.sessions.iter().any(|session| session.id == *id));
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
            if self.focused_terminal == Some(session_id) {
                self.focused_terminal = None;
            }
        }
    }

    pub fn tile_position(index: usize, columns: usize) -> (i32, i32) {
        let columns = columns.max(1);
        let row = index / columns;
        let col = index % columns;
        (col as i32, row as i32)
    }
}

#[cfg(test)]
mod tests {
    use super::{SessionKind, SessionLaunch, SessionStatus, WorkspaceState};

    #[test]
    fn loading_workspace_selects_first_session() {
        let mut state = WorkspaceState::new();
        let ids = state.load_workspace(vec![
            SessionLaunch::shell("One", "shell", "banner"),
            SessionLaunch::shell("Two", "shell", "banner"),
        ]);

        assert_eq!(state.sessions().len(), 2);
        assert_eq!(state.selected_session(), Some(ids[0]));
        assert_eq!(state.sessions()[0].status, SessionStatus::Launching);
    }

    #[test]
    fn add_session_preserves_existing_selection() {
        let mut state = WorkspaceState::new();
        let first = state.add_session(SessionLaunch::shell("One", "shell", "banner"));
        let second = state.add_session(SessionLaunch::shell("Two", "shell", "banner"));

        assert_eq!(state.selected_session(), Some(first));
        assert_eq!(state.sessions()[1].id, second);
    }

    #[test]
    fn mark_spawned_uses_kind_default_status() {
        let mut state = WorkspaceState::new();
        let shell = state.add_session(SessionLaunch::shell("One", "shell", "banner"));
        let blocked = state.add_session(SessionLaunch::blocking_prompt("Two", "prompt", "approve"));

        state.mark_spawned(shell, 4242);
        state.mark_spawned(blocked, 4343);

        assert_eq!(state.sessions()[0].status, SessionStatus::Waiting);
        assert_eq!(state.sessions()[1].status, SessionStatus::Blocked);
    }

    #[test]
    fn non_zero_exit_is_failed_and_zero_exit_is_complete() {
        let mut state = WorkspaceState::new();
        let first = state.add_session(SessionLaunch::shell("One", "shell", "banner"));
        let second = state.add_session(SessionLaunch::failing_task("Two", "task", "boom", 2));

        state.mark_spawned(first, 1);
        state.mark_spawned(second, 2);
        state.set_terminal_focus(Some(second));
        state.mark_exited(first, 0);
        state.mark_exited(second, 2);

        assert_eq!(state.sessions()[0].status, SessionStatus::Complete);
        assert_eq!(state.sessions()[1].status, SessionStatus::Failed(2));
        assert_eq!(state.focused_terminal(), None);
    }

    #[test]
    fn status_hints_reflect_supervisory_meaning() {
        let prompt = SessionLaunch::blocking_prompt("Approval", "prompt", "approve");
        let failed = SessionLaunch::failing_task("Task", "task", "boom", 9);

        assert_eq!(
            prompt.status_hint(SessionStatus::Blocked),
            "Waiting for operator input"
        );
        assert_eq!(
            failed.status_hint(SessionStatus::Failed(9)),
            "Exited with code 9"
        );
    }

    #[test]
    fn shell_launch_is_deterministic() {
        let shell = SessionLaunch::shell("A", "shell", "banner");

        assert_eq!(shell.kind, SessionKind::WaitingShell);
        assert!(shell.args.contains(&"--noprofile".to_string()));
        assert!(shell.args.contains(&"--norc".to_string()));
    }

    #[test]
    fn tile_positions_fill_rows_left_to_right() {
        assert_eq!(WorkspaceState::tile_position(0, 2), (0, 0));
        assert_eq!(WorkspaceState::tile_position(1, 2), (1, 0));
        assert_eq!(WorkspaceState::tile_position(2, 2), (0, 1));
        assert_eq!(WorkspaceState::tile_position(5, 3), (2, 1));
    }
}
