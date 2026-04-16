use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SessionId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionKind {
    WaitingShell,
    PlanningStream,
    RunningStream,
    BlockingPrompt,
    FailingTask,
}

impl SessionKind {
    pub fn default_status(self) -> SessionStatus {
        match self {
            SessionKind::WaitingShell => SessionStatus::Waiting,
            SessionKind::PlanningStream
            | SessionKind::RunningStream
            | SessionKind::BlockingPrompt
            | SessionKind::FailingTask => SessionStatus::Running,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionLaunch {
    pub name: String,
    pub subtitle: String,
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    #[serde(default)]
    pub env: Vec<(String, String)>,
    pub kind: SessionKind,
}

impl SessionLaunch {
    pub fn with_cwd(mut self, cwd: impl Into<PathBuf>) -> Self {
        self.cwd = Some(cwd.into());
        self
    }

    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionStatus {
    Launching,
    Running,
    Waiting,
    Blocked,
    Failed(i32),
    Complete,
    Detached,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionEvent {
    pub sequence: u64,
    pub summary: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionRecord {
    pub id: SessionId,
    pub launch: SessionLaunch,
    pub display_name: Option<String>,
    pub status: SessionStatus,
    pub pid: Option<u32>,
    pub events: Vec<SessionEvent>,
}
