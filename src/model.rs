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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProbeLens {
    Output,
    Events,
    Process,
}

impl ProbeLens {
    pub fn title(self) -> &'static str {
        match self {
            ProbeLens::Output => "Output",
            ProbeLens::Events => "Events",
            ProbeLens::Process => "Process",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProbeMode {
    Peek,
    Pinned,
}

impl ProbeMode {
    pub fn action_label(self) -> &'static str {
        match self {
            ProbeMode::Peek => "Pin",
            ProbeMode::Pinned => "Unpin",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PresentationMode {
    Battlefield,
    Focused(SessionId),
}

impl Default for PresentationMode {
    fn default() -> Self {
        Self::Battlefield
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProbeState {
    pub session_id: SessionId,
    pub lens: ProbeLens,
    pub mode: ProbeMode,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionEvent {
    pub sequence: u64,
    pub summary: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionRecord {
    pub id: SessionId,
    pub launch: SessionLaunch,
    pub status: SessionStatus,
    pub pid: Option<u32>,
    pub events: Vec<SessionEvent>,
}

#[derive(Debug, Default)]
pub struct WorkspaceState {
    next_session_id: u32,
    next_event_sequence: u64,
    sessions: Vec<SessionRecord>,
    selected_session: Option<SessionId>,
    focused_terminal: Option<SessionId>,
    presentation_mode: PresentationMode,
    open_probe: Option<ProbeState>,
}

impl WorkspaceState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn load_workspace(&mut self, launches: Vec<SessionLaunch>) -> Vec<SessionId> {
        self.next_session_id = 1;
        self.next_event_sequence = 1;
        self.sessions.clear();
        self.selected_session = None;
        self.focused_terminal = None;
        self.presentation_mode = PresentationMode::Battlefield;
        self.open_probe = None;

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
            events: Vec::new(),
        });

        self.selected_session.get_or_insert(id);
        self.push_event(id, "Session added to workspace");
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

    pub fn presentation_mode(&self) -> PresentationMode {
        self.presentation_mode
    }

    pub fn focused_session(&self) -> Option<SessionId> {
        match self.presentation_mode {
            PresentationMode::Battlefield => None,
            PresentationMode::Focused(session_id) => Some(session_id),
        }
    }

    pub fn open_probe(&self) -> Option<ProbeState> {
        self.open_probe
    }

    pub fn session(&self, session_id: SessionId) -> Option<&SessionRecord> {
        self.sessions.iter().find(|session| session.id == session_id)
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

    pub fn activate_session(&mut self, session_id: SessionId) {
        self.enter_focus_mode(session_id);
    }

    pub fn enter_focus_mode(&mut self, session_id: SessionId) {
        if self.sessions.iter().any(|session| session.id == session_id) {
            self.selected_session = Some(session_id);
            self.focused_terminal = Some(session_id);
            self.presentation_mode = PresentationMode::Focused(session_id);
            self.push_event(session_id, "Entered focused terminal view");
        }
    }

    pub fn return_to_battlefield(&mut self) {
        if let Some(session_id) = self.focused_session() {
            self.push_event(session_id, "Returned to battlefield view");
        }
        self.presentation_mode = PresentationMode::Battlefield;
        self.focused_terminal = None;
    }

    pub fn show_probe(&mut self, session_id: SessionId) {
        if self.sessions.iter().any(|session| session.id == session_id) {
            self.selected_session = Some(session_id);
            let probe = ProbeState {
                session_id,
                lens: self
                    .open_probe
                    .filter(|probe| probe.session_id == session_id)
                    .map(|probe| probe.lens)
                    .unwrap_or(ProbeLens::Output),
                mode: self
                    .open_probe
                    .filter(|probe| probe.session_id == session_id)
                    .map(|probe| probe.mode)
                    .unwrap_or(ProbeMode::Peek),
            };
            self.open_probe = Some(probe);
            self.push_event(session_id, "Probe opened");
        }
    }

    pub fn close_probe(&mut self) {
        if let Some(probe) = self.open_probe {
            self.push_event(probe.session_id, "Probe closed");
        }
        self.open_probe = None;
    }

    pub fn set_probe_lens(&mut self, lens: ProbeLens) {
        if let Some(mut probe) = self.open_probe {
            probe.lens = lens;
            self.open_probe = Some(probe);
            self.push_event(probe.session_id, format!("Probe switched to {}", lens.title()));
        }
    }

    pub fn toggle_probe_pin(&mut self) {
        if let Some(mut probe) = self.open_probe {
            probe.mode = match probe.mode {
                ProbeMode::Peek => ProbeMode::Pinned,
                ProbeMode::Pinned => ProbeMode::Peek,
            };
            let label = match probe.mode {
                ProbeMode::Peek => "Probe returned to peek mode",
                ProbeMode::Pinned => "Probe pinned for ongoing watch",
            };
            self.open_probe = Some(probe);
            self.push_event(probe.session_id, label);
        }
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
            if self.focused_terminal == Some(session_id) {
                self.focused_terminal = None;
            }
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

    pub fn tile_position(index: usize, columns: usize) -> (i32, i32) {
        let columns = columns.max(1);
        let row = index / columns;
        let col = index % columns;
        (col as i32, row as i32)
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
        PresentationMode, ProbeLens, ProbeMode, SessionKind, SessionLaunch, SessionStatus,
        WorkspaceState,
    };

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

    #[test]
    fn activate_session_moves_selection_and_terminal_focus_together() {
        let mut state = WorkspaceState::new();
        let first = state.add_session(SessionLaunch::shell("One", "shell", "banner"));
        let second = state.add_session(SessionLaunch::shell("Two", "shell", "banner"));

        state.activate_session(second);

        assert_eq!(state.selected_session(), Some(second));
        assert_eq!(state.focused_terminal(), Some(second));
        assert_eq!(state.presentation_mode(), PresentationMode::Focused(second));
        assert_ne!(state.selected_session(), Some(first));
    }

    #[test]
    fn return_to_battlefield_clears_focus_mode_but_preserves_selection() {
        let mut state = WorkspaceState::new();
        let first = state.add_session(SessionLaunch::shell("One", "shell", "banner"));
        let second = state.add_session(SessionLaunch::shell("Two", "shell", "banner"));

        state.enter_focus_mode(second);
        state.return_to_battlefield();

        assert_eq!(state.presentation_mode(), PresentationMode::Battlefield);
        assert_eq!(state.focused_session(), None);
        assert_eq!(state.focused_terminal(), None);
        assert_eq!(state.selected_session(), Some(second));
        assert_ne!(state.selected_session(), Some(first));
    }

    #[test]
    fn showing_and_closing_probe_tracks_selected_session() {
        let mut state = WorkspaceState::new();
        let first = state.add_session(SessionLaunch::shell("One", "shell", "banner"));
        let second = state.add_session(SessionLaunch::shell("Two", "shell", "banner"));

        state.show_probe(second);
        assert_eq!(state.selected_session(), Some(second));
        assert_eq!(state.open_probe().map(|probe| probe.session_id), Some(second));

        state.close_probe();
        assert_eq!(state.open_probe(), None);
        assert_eq!(state.selected_session(), Some(second));

        state.show_probe(first);
        assert_eq!(state.open_probe().map(|probe| probe.session_id), Some(first));
    }

    #[test]
    fn probe_can_switch_lenses_and_modes() {
        let mut state = WorkspaceState::new();
        let first = state.add_session(SessionLaunch::shell("One", "shell", "banner"));

        state.show_probe(first);
        state.set_probe_lens(ProbeLens::Process);
        state.toggle_probe_pin();

        let probe = state.open_probe().expect("probe should stay open");
        assert_eq!(probe.session_id, first);
        assert_eq!(probe.lens, ProbeLens::Process);
        assert_eq!(probe.mode, ProbeMode::Pinned);
    }

    #[test]
    fn events_capture_supervision_transitions() {
        let mut state = WorkspaceState::new();
        let session_id = state.add_session(SessionLaunch::shell("One", "shell", "banner"));

        state.mark_spawned(session_id, 4242);
        state.activate_session(session_id);
        state.show_probe(session_id);
        state.set_probe_lens(ProbeLens::Events);

        let session = state.session(session_id).expect("session must exist");
        let summaries: Vec<&str> = session.events.iter().map(|event| event.summary.as_str()).collect();

        assert!(summaries.iter().any(|summary| summary.contains("Spawned process 4242")));
        assert!(summaries
            .iter()
            .any(|summary| summary.contains("Entered focused terminal view")));
        assert!(summaries.iter().any(|summary| summary.contains("Probe opened")));
        assert!(summaries
            .iter()
            .any(|summary| summary.contains("Probe switched to Events")));
    }
}
