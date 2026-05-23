use crate::model::{GroupId, SessionId, SessionRecord, SupervisedGroupRecord, WorkspaceItem};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    AttachClient,
    SetTerminalDisplayCapabilities {
        capabilities: TerminalDisplayCapabilities,
    },
    CreateOrResumeDefaultWorkspace,
    AddTerminals {
        source_session: SessionId,
    },
    AddTerminalsTo {
        source_session: SessionId,
        target_total: usize,
    },
    AddOneTerminal {
        source_session: SessionId,
    },
    ResizeTerminal {
        session_id: SessionId,
        rows: u16,
        cols: u16,
    },
    CloseSession {
        session_id: SessionId,
    },
    CreateSupervisedGroup {
        name: String,
        session_ids: Vec<SessionId>,
        goal: Option<String>,
    },
    SetGroupSupervisorVisible {
        group_id: GroupId,
        visible: bool,
    },
    SetInputSync {
        enabled: bool,
        scope: InputSyncScope,
    },
    SendMessageToAgent {
        group_id: GroupId,
        session_id: SessionId,
        message: String,
    },
    UpdateGroupSummary {
        group_id: GroupId,
        markdown: String,
    },
    ConfigureTerminalAssist {
        openai_api_key: Option<String>,
        openai_base_url: String,
        model: String,
    },
    RequestTerminalAssist {
        request_id: u64,
        session_id: SessionId,
        prompt: String,
    },
    DetachClient {
        keep_alive: bool,
    },
    TerminateWorkspace,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalDisplayCapabilities {
    #[serde(default)]
    pub kitty_graphics: bool,
    #[serde(default)]
    pub sixel: bool,
    #[serde(default)]
    pub vte_version: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InputSyncScope {
    RootVisible,
    GroupMembers { group_id: GroupId },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    WorkspaceSnapshot {
        snapshot: WorkspaceSnapshot,
    },
    TerminalAssistCompleted {
        request_id: u64,
        session_id: SessionId,
        inserted: bool,
        error: Option<String>,
    },
    Error {
        message: String,
    },
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct WorkspaceSnapshot {
    #[serde(default)]
    pub items: Vec<WorkspaceItem>,
    pub sessions: Vec<SessionSnapshot>,
    #[serde(default)]
    pub groups: Vec<SupervisedGroupRecord>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionSnapshot {
    pub record: SessionRecord,
    pub observation: ObservationSnapshot,
    pub raw_stream_socket_name: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ObservationSnapshot {
    pub last_change_age_secs: u64,
    pub recent_lines: Vec<String>,
    pub painted_line: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_message_round_trips_through_json() {
        let message = ClientMessage::ResizeTerminal {
            session_id: SessionId(7),
            rows: 31,
            cols: 97,
        };
        let json = serde_json::to_string(&message).expect("serialize client message");
        let decoded: ClientMessage =
            serde_json::from_str(&json).expect("deserialize client message");
        match decoded {
            ClientMessage::ResizeTerminal {
                session_id,
                rows,
                cols,
            } => {
                assert_eq!(session_id, SessionId(7));
                assert_eq!(rows, 31);
                assert_eq!(cols, 97);
            }
            other => panic!("unexpected decoded message: {other:?}"),
        }
    }

    #[test]
    fn add_one_terminal_round_trips_through_json() {
        let message = ClientMessage::AddOneTerminal {
            source_session: SessionId(42),
        };
        let json = serde_json::to_string(&message).expect("serialize add_one_terminal");
        let decoded: ClientMessage =
            serde_json::from_str(&json).expect("deserialize add_one_terminal");
        match decoded {
            ClientMessage::AddOneTerminal { source_session } => {
                assert_eq!(source_session, SessionId(42));
            }
            other => panic!("unexpected decoded message: {other:?}"),
        }
    }

    #[test]
    fn terminal_display_capabilities_round_trip_through_json() {
        let message = ClientMessage::SetTerminalDisplayCapabilities {
            capabilities: TerminalDisplayCapabilities {
                kitty_graphics: true,
                sixel: true,
                vte_version: Some("8400".into()),
            },
        };
        let json = serde_json::to_string(&message).expect("serialize terminal capabilities");
        let decoded: ClientMessage =
            serde_json::from_str(&json).expect("deserialize terminal capabilities");
        match decoded {
            ClientMessage::SetTerminalDisplayCapabilities { capabilities } => {
                assert!(capabilities.kitty_graphics);
                assert!(capabilities.sixel);
                assert_eq!(capabilities.vte_version.as_deref(), Some("8400"));
            }
            other => panic!("unexpected decoded message: {other:?}"),
        }
    }

    #[test]
    fn create_group_round_trips_through_json() {
        let message = ClientMessage::CreateSupervisedGroup {
            name: "Research".into(),
            session_ids: vec![SessionId(1), SessionId(2)],
            goal: Some("Watch the benchmark loop".into()),
        };
        let json = serde_json::to_string(&message).expect("serialize create group");
        let decoded: ClientMessage = serde_json::from_str(&json).expect("deserialize create group");
        match decoded {
            ClientMessage::CreateSupervisedGroup {
                name,
                session_ids,
                goal,
            } => {
                assert_eq!(name, "Research");
                assert_eq!(session_ids, vec![SessionId(1), SessionId(2)]);
                assert_eq!(goal.as_deref(), Some("Watch the benchmark loop"));
            }
            other => panic!("unexpected decoded message: {other:?}"),
        }
    }

    #[test]
    fn input_sync_round_trips_through_json() {
        let message = ClientMessage::SetInputSync {
            enabled: true,
            scope: InputSyncScope::GroupMembers {
                group_id: GroupId(3),
            },
        };
        let json = serde_json::to_string(&message).expect("serialize input sync");
        let decoded: ClientMessage = serde_json::from_str(&json).expect("deserialize input sync");
        match decoded {
            ClientMessage::SetInputSync { enabled, scope } => {
                assert!(enabled);
                assert_eq!(
                    scope,
                    InputSyncScope::GroupMembers {
                        group_id: GroupId(3)
                    }
                );
            }
            other => panic!("unexpected decoded message: {other:?}"),
        }
    }

    #[test]
    fn terminal_assist_result_round_trips_through_json() {
        let message = ServerMessage::TerminalAssistCompleted {
            request_id: 9,
            session_id: SessionId(3),
            inserted: true,
            error: None,
        };
        let json = serde_json::to_string(&message).expect("serialize assist result");
        let decoded: ServerMessage =
            serde_json::from_str(&json).expect("deserialize assist result");
        match decoded {
            ServerMessage::TerminalAssistCompleted {
                request_id,
                session_id,
                inserted,
                error,
            } => {
                assert_eq!(request_id, 9);
                assert_eq!(session_id, SessionId(3));
                assert!(inserted);
                assert!(error.is_none());
            }
            other => panic!("unexpected decoded message: {other:?}"),
        }
    }

    #[test]
    fn server_message_round_trips_through_json() {
        let message = ServerMessage::WorkspaceSnapshot {
            snapshot: WorkspaceSnapshot::default(),
        };
        let json = serde_json::to_string(&message).expect("serialize server message");
        let decoded: ServerMessage =
            serde_json::from_str(&json).expect("deserialize server message");
        match decoded {
            ServerMessage::WorkspaceSnapshot { snapshot } => {
                assert!(snapshot.sessions.is_empty());
            }
            other => panic!("unexpected decoded server message: {other:?}"),
        }
    }
}
