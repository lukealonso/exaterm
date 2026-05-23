use exaterm_types::model::{
    GroupId, SessionEvent, SessionId, SessionLaunch, SessionRecord, SessionStatus,
    SupervisedGroupRecord, WorkspaceItem,
};

#[derive(Debug, Default)]
pub struct WorkspaceViewState {
    next_session_id: u32,
    next_event_sequence: u64,
    sessions: Vec<SessionRecord>,
    groups: Vec<SupervisedGroupRecord>,
    item_order: Vec<WorkspaceItem>,
    session_order: Vec<SessionId>,
    selected_session: Option<SessionId>,
    selected_workspace_item: Option<WorkspaceItem>,
}

impl WorkspaceViewState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn replace_sessions(&mut self, sessions: Vec<SessionRecord>) {
        let previous_selected = self.selected_session;
        let previous_selected_item = self.selected_workspace_item;

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
        self.groups.clear();

        // Maintain session_order: prune removed, append new.
        self.session_order
            .retain(|id| self.sessions.iter().any(|s| s.id == *id));
        for session in &self.sessions {
            if !self.session_order.contains(&session.id) {
                self.session_order.push(session.id);
            }
        }
        self.item_order = self
            .session_order
            .iter()
            .copied()
            .map(WorkspaceItem::Session)
            .collect();

        self.selected_session = previous_selected
            .filter(|session_id| {
                self.sessions
                    .iter()
                    .any(|session| session.id == *session_id)
            })
            .or_else(|| self.sessions.first().map(|session| session.id));
        self.selected_workspace_item = previous_selected_item
            .filter(|item| self.item_exists(*item))
            .or_else(|| self.ordered_visible_items().first().copied())
            .or_else(|| self.selected_session.map(WorkspaceItem::Session));
    }

    pub fn replace_workspace(
        &mut self,
        sessions: Vec<SessionRecord>,
        groups: Vec<SupervisedGroupRecord>,
        items: Vec<WorkspaceItem>,
    ) {
        let previous_selected = self.selected_session;
        let previous_selected_item = self.selected_workspace_item;

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
        self.groups = groups;
        self.item_order = self.valid_item_order(items);

        self.session_order = self
            .item_order
            .iter()
            .filter_map(|item| match item {
                WorkspaceItem::Session(session_id) => Some(*session_id),
                WorkspaceItem::Group(_) => None,
            })
            .collect();
        for session in &self.sessions {
            if !self.session_order.contains(&session.id) {
                self.session_order.push(session.id);
            }
        }

        self.selected_session = previous_selected
            .filter(|session_id| self.session_exists(*session_id))
            .or_else(|| self.session_order.first().copied());
        self.selected_workspace_item = previous_selected_item
            .filter(|item| self.item_exists(*item))
            .or_else(|| self.ordered_visible_items().first().copied())
            .or_else(|| self.selected_session.map(WorkspaceItem::Session));
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
        self.session_order.push(id);
        self.item_order.push(WorkspaceItem::Session(id));

        self.selected_session.get_or_insert(id);
        self.selected_workspace_item
            .get_or_insert(WorkspaceItem::Session(id));
        self.push_event(id, "Session added to workspace");
        id
    }

    pub fn remove_session(&mut self, session_id: SessionId) {
        self.sessions.retain(|s| s.id != session_id);
        self.session_order.retain(|id| *id != session_id);
        self.item_order
            .retain(|item| *item != WorkspaceItem::Session(session_id));
        if self.selected_session == Some(session_id) {
            self.selected_session = self.sessions.first().map(|s| s.id);
        }
        if self.selected_workspace_item == Some(WorkspaceItem::Session(session_id)) {
            self.selected_workspace_item = self
                .ordered_visible_items()
                .first()
                .copied()
                .or_else(|| self.selected_session.map(WorkspaceItem::Session));
        }
    }

    pub fn sessions(&self) -> &[SessionRecord] {
        &self.sessions
    }

    pub fn groups(&self) -> &[SupervisedGroupRecord] {
        &self.groups
    }

    /// Sessions in user-arranged display order.
    pub fn ordered_session_ids(&self) -> &[SessionId] {
        &self.session_order
    }

    pub fn ordered_visible_items(&self) -> &[WorkspaceItem] {
        &self.item_order
    }

    /// Move `session_id` to `target_index` in the display order.
    pub fn move_session(&mut self, session_id: SessionId, target_index: usize) {
        if let Some(from) = self.session_order.iter().position(|id| *id == session_id) {
            let id = self.session_order.remove(from);
            let to = target_index.min(self.session_order.len());
            self.session_order.insert(to, id);
        }
        if let Some(from) = self
            .item_order
            .iter()
            .position(|item| *item == WorkspaceItem::Session(session_id))
        {
            let item = self.item_order.remove(from);
            let to = target_index.min(self.item_order.len());
            self.item_order.insert(to, item);
        }
    }

    pub fn selected_session(&self) -> Option<SessionId> {
        self.selected_session
    }

    pub fn selected_workspace_item(&self) -> Option<WorkspaceItem> {
        self.selected_workspace_item
    }

    pub fn session(&self, session_id: SessionId) -> Option<&SessionRecord> {
        self.sessions
            .iter()
            .find(|session| session.id == session_id)
    }

    pub fn group(&self, group_id: GroupId) -> Option<&SupervisedGroupRecord> {
        self.groups.iter().find(|group| group.id == group_id)
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
            self.selected_workspace_item = Some(WorkspaceItem::Session(session_id));
        }
    }

    pub fn select_workspace_item(&mut self, item: WorkspaceItem) {
        if self.item_exists(item) {
            self.selected_workspace_item = Some(item);
            if let WorkspaceItem::Session(session_id) = item {
                self.selected_session = Some(session_id);
            }
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

    fn valid_item_order(&self, items: Vec<WorkspaceItem>) -> Vec<WorkspaceItem> {
        let has_explicit_order = !items.is_empty();
        let mut order = Vec::new();
        for item in items {
            if self.item_exists(item) && !order.contains(&item) {
                order.push(item);
            }
        }

        if !has_explicit_order || order.is_empty() {
            order.extend(
                self.sessions
                    .iter()
                    .map(|session| WorkspaceItem::Session(session.id)),
            );
            order.extend(
                self.groups
                    .iter()
                    .map(|group| WorkspaceItem::Group(group.id)),
            );
        }

        order
    }

    fn item_exists(&self, item: WorkspaceItem) -> bool {
        match item {
            WorkspaceItem::Session(session_id) => self.session_exists(session_id),
            WorkspaceItem::Group(group_id) => self.group(group_id).is_some(),
        }
    }

    fn session_exists(&self, session_id: SessionId) -> bool {
        self.sessions.iter().any(|session| session.id == session_id)
    }
}
