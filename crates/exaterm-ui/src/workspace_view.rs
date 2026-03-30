use exaterm_types::model::{SessionEvent, SessionId, SessionLaunch, SessionRecord, SessionStatus};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PresentationMode {
    Battlefield,
    Focused(SessionId),
}

impl Default for PresentationMode {
    fn default() -> Self {
        Self::Battlefield
    }
}

#[derive(Debug, Default)]
pub struct WorkspaceViewState {
    next_session_id: u32,
    next_event_sequence: u64,
    sessions: Vec<SessionRecord>,
    selected_session: Option<SessionId>,
    focused_terminal: Option<SessionId>,
    presentation_mode: PresentationMode,
}

impl WorkspaceViewState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn replace_sessions(&mut self, sessions: Vec<SessionRecord>) {
        let previous_selected = self.selected_session;
        let previous_focus = self.focused_terminal;
        let previous_presentation = self.presentation_mode;

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

        self.selected_session = previous_selected
            .filter(|session_id| {
                self.sessions
                    .iter()
                    .any(|session| session.id == *session_id)
            })
            .or_else(|| self.sessions.first().map(|session| session.id));
        self.focused_terminal = previous_focus.filter(|session_id| {
            self.sessions
                .iter()
                .any(|session| session.id == *session_id)
        });
        self.presentation_mode = match previous_presentation {
            PresentationMode::Focused(session_id)
                if self.sessions.iter().any(|session| session.id == session_id) =>
            {
                PresentationMode::Focused(session_id)
            }
            _ => PresentationMode::Battlefield,
        };
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

    pub fn focused_session(&self) -> Option<SessionId> {
        match self.presentation_mode {
            PresentationMode::Battlefield => None,
            PresentationMode::Focused(session_id) => Some(session_id),
        }
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

    pub fn select_session(&mut self, session_id: SessionId) {
        if self.sessions.iter().any(|session| session.id == session_id) {
            self.selected_session = Some(session_id);
        }
    }

    pub fn set_terminal_focus(&mut self, session_id: Option<SessionId>) {
        self.focused_terminal =
            session_id.filter(|id| self.sessions.iter().any(|session| session.id == *id));
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
    use super::WorkspaceViewState;
    use exaterm_core::model::shell_launch;

    #[test]
    fn focus_mode_preserves_selection_when_returning_to_battlefield() {
        let mut state = WorkspaceViewState::new();
        let first = state.add_session(shell_launch("One", "shell", "banner"));
        let second = state.add_session(shell_launch("Two", "shell", "banner"));

        state.enter_focus_mode(second);
        state.return_to_battlefield();

        assert_eq!(state.focused_session(), None);
        assert_eq!(state.selected_session(), Some(second));
        assert_ne!(state.selected_session(), Some(first));
    }

    #[test]
    fn replacing_sessions_keeps_focus_when_session_survives() {
        let mut state = WorkspaceViewState::new();
        let session_id = state.add_session(shell_launch("One", "shell", "banner"));
        state.enter_focus_mode(session_id);
        let sessions = state.sessions().to_vec();

        state.replace_sessions(sessions);

        assert_eq!(state.focused_session(), Some(session_id));
        assert_eq!(state.selected_session(), Some(session_id));
    }
}
