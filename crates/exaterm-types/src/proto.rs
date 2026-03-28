use crate::model::{SessionId, SessionRecord};
use crate::synthesis::TacticalSynthesis;
use base64::Engine;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    AttachClient,
    CreateOrResumeDefaultWorkspace,
    AddTerminals {
        source_session: SessionId,
    },
    ResizeTerminal {
        session_id: SessionId,
        rows: u16,
        cols: u16,
    },
    ToggleAutoNudge {
        session_id: SessionId,
        enabled: bool,
    },
    DetachClient {
        keep_alive: bool,
    },
    TerminateWorkspace,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    WorkspaceSnapshot {
        snapshot: WorkspaceSnapshot,
    },
    Error {
        message: String,
    },
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct WorkspaceSnapshot {
    pub sessions: Vec<SessionSnapshot>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionSnapshot {
    pub record: SessionRecord,
    pub observation: ObservationSnapshot,
    pub summary: Option<TacticalSynthesis>,
    pub raw_stream_socket_name: Option<String>,
    pub auto_nudge_enabled: bool,
    pub last_nudge: Option<String>,
    pub last_sent_age_secs: Option<u64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ObservationSnapshot {
    pub last_change_age_secs: u64,
    pub recent_lines: Vec<String>,
    pub painted_line: Option<String>,
    pub shell_child_command: Option<String>,
    pub active_command: Option<String>,
    pub dominant_process: Option<String>,
    pub process_tree_excerpt: Option<String>,
    pub recent_files: Vec<String>,
    pub work_output_excerpt: Option<String>,
}

pub fn encode_bytes(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

pub fn decode_bytes(bytes_b64: &str) -> Result<Vec<u8>, String> {
    base64::engine::general_purpose::STANDARD
        .decode(bytes_b64)
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_round_trip_through_base64_helpers() {
        let source = b"\x1b[2K\rWorking...\n";
        let encoded = encode_bytes(source);
        let decoded = decode_bytes(&encoded).expect("decode bytes");
        assert_eq!(decoded, source);
    }

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
