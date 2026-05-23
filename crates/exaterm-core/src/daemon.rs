use crate::config::{
    apply_app_config_environment, apply_terminal_assist_config_environment, load_app_config,
    TerminalAssistConfig,
};
use crate::file_watch::{spawn_repo_watch, RepoWatchHandle};
use crate::mcp::{
    McpServer, ServerInfo, ToolCallError, ToolCallOutcome, ToolCallResult, ToolDefinition,
};
use crate::model::{
    command_launch, user_shell_launch, SessionId, SessionKind, SessionLaunch, WorkspaceStore,
};
use crate::observation::{
    apply_file_activity, apply_observation_refresh, apply_stream_update, clear_file_activity,
    compute_observation_refresh, find_git_worktree_root, record_terminal_input_activity,
    terminal_assist_history, SessionObservation,
};
use crate::proto::{
    ClientMessage, InputSyncScope, ObservationSnapshot, ServerMessage, SessionSnapshot,
    TerminalDisplayCapabilities, WorkspaceSnapshot,
};
use crate::runtime::{spawn_headless_runtime, RuntimeEvent, SessionRuntime};
use crate::synthesis::{
    ProviderCallResult, ProviderPreferences, SynthesisBackendRegistry, TerminalAssistEvidence,
};
use exaterm_types::model::{
    GroupId, SupervisedGroupRecord, SupervisorActionRecord, SupervisorProvider, WorkspaceItem,
};
use portable_pty::PtySize;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::process::{Command, ExitCode, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

const CONTROL_SOCKET_NAME: &str = "beachhead-control.sock";
const MCP_SOCKET_NAME: &str = "beachhead-mcp.sock";
const CANONICAL_TERMINAL_ROWS: u16 = 40;
const CANONICAL_TERMINAL_COLS: u16 = 120;
const REPLAY_BYTES_LIMIT: usize = 8 * 1024 * 1024;
const REFRESH_INTERVAL: Duration = Duration::from_millis(300);
const CONTROL_EVENTS_PER_TICK: usize = 128;
const TERMINAL_RETURN_DELAY: Duration = Duration::from_millis(35);

struct AssistWorker {
    requests: mpsc::Sender<AssistJob>,
    responses: mpsc::Receiver<AssistResult>,
}

struct AssistJob {
    request_id: u64,
    origin_session_id: SessionId,
    evidence: crate::synthesis::TerminalAssistEvidence,
    preferences: ProviderPreferences,
}

struct AssistResult {
    request_id: u64,
    origin_session_id: SessionId,
    suggestion: ProviderCallResult<exaterm_types::synthesis::TerminalAssistSuggestion>,
}

struct ObservationWorker {
    requests: mpsc::Sender<ObservationJob>,
    responses: mpsc::Receiver<ObservationResult>,
}

struct ObservationJob {
    session_id: SessionId,
    session: crate::model::SessionRecord,
}

struct ObservationResult {
    session_id: SessionId,
    session: crate::model::SessionRecord,
    refresh: crate::observation::ObservationRefreshResult,
}

#[derive(Clone)]
struct ControlNotifier {
    tx: mpsc::Sender<ClientControl>,
    wake: std::sync::Arc<std::sync::Mutex<UnixStream>>,
}

impl ControlNotifier {
    fn send(&self, control: ClientControl) -> Result<(), mpsc::SendError<ClientControl>> {
        self.tx.send(control)?;
        self.wake();
        Ok(())
    }

    fn wake(&self) {
        let Ok(mut wake) = self.wake.lock() else {
            return;
        };
        match wake.write(&[1]) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(_) => {}
        }
    }
}

struct SupervisorActionEntry {
    sequence: u64,
    summary: String,
    at: Instant,
}

struct SupervisedGroupState {
    id: GroupId,
    name: String,
    member_session_ids: Vec<SessionId>,
    supervisor_session_id: Option<SessionId>,
    provider: Option<SupervisorProvider>,
    goal: Option<String>,
    summary_markdown: String,
    supervisor_visible: bool,
    summary_updated_at: Option<Instant>,
    actions: Vec<SupervisorActionEntry>,
}

struct ObservationCacheEntry {
    last_attempt: Option<Instant>,
    in_flight: bool,
}

impl ObservationCacheEntry {
    fn new() -> Self {
        Self {
            last_attempt: None,
            in_flight: false,
        }
    }
}

struct DaemonState {
    workspace: WorkspaceStore,
    workspace_items: Vec<WorkspaceItem>,
    next_group_id: u32,
    next_supervisor_action_sequence: u64,
    groups: BTreeMap<GroupId, SupervisedGroupState>,
    observations: BTreeMap<SessionId, SessionObservation>,
    observation_worker: Option<ObservationWorker>,
    observation_cache: BTreeMap<SessionId, ObservationCacheEntry>,
    runtimes: BTreeMap<SessionId, SessionRuntime>,
    replay_buffers: BTreeMap<SessionId, Vec<u8>>,
    session_streams: BTreeMap<SessionId, SessionStreamState>,
    repo_watches: BTreeMap<PathBuf, RepoWatchState>,
    session_repo_roots: BTreeMap<SessionId, PathBuf>,
    assist_worker: Option<AssistWorker>,
    forwarded_sessions: BTreeSet<SessionId>,
    input_sync_enabled: bool,
    input_sync_scope: InputSyncScope,
    terminal_display_capabilities: TerminalDisplayCapabilities,
    snapshot_dirty: bool,
}

struct SessionStreamState {
    socket_name: String,
    socket_path: PathBuf,
    listener: UnixListener,
    writer: std::sync::Arc<std::sync::Mutex<Option<UnixStream>>>,
}

struct RepoWatchState {
    sessions: BTreeSet<SessionId>,
    handle: RepoWatchHandle,
}

fn apply_terminal_display_env(
    launch: &mut SessionLaunch,
    capabilities: &TerminalDisplayCapabilities,
) {
    let mut protocols = Vec::new();

    if capabilities.kitty_graphics {
        upsert_launch_env(&mut launch.env, "KITTY_WINDOW_ID", "1");
        protocols.push("kitty");
    }

    if capabilities.sixel {
        if let Some(vte_version) = capabilities.vte_version.as_deref() {
            upsert_launch_env(&mut launch.env, "VTE_VERSION", vte_version);
        }
        protocols.push("sixel");
    }

    if !protocols.is_empty() {
        upsert_launch_env(
            &mut launch.env,
            "EXATERM_TERMINAL_IMAGE_PROTOCOLS",
            &protocols.join(","),
        );
    }
}

fn upsert_launch_env(env: &mut Vec<(String, String)>, key: &str, value: &str) {
    if let Some((_, existing_value)) = env
        .iter_mut()
        .rev()
        .find(|(existing_key, _)| existing_key == key)
    {
        *existing_value = value.to_string();
        return;
    }
    env.push((key.to_string(), value.to_string()));
}

impl DaemonState {
    fn new() -> Self {
        Self {
            workspace: WorkspaceStore::new(),
            workspace_items: Vec::new(),
            next_group_id: 1,
            next_supervisor_action_sequence: 1,
            groups: BTreeMap::new(),
            observations: BTreeMap::new(),
            observation_worker: spawn_observation_worker(),
            observation_cache: BTreeMap::new(),
            runtimes: BTreeMap::new(),
            replay_buffers: BTreeMap::new(),
            session_streams: BTreeMap::new(),
            repo_watches: BTreeMap::new(),
            session_repo_roots: BTreeMap::new(),
            assist_worker: spawn_assist_worker(),
            forwarded_sessions: BTreeSet::new(),
            input_sync_enabled: false,
            input_sync_scope: InputSyncScope::RootVisible,
            terminal_display_capabilities: TerminalDisplayCapabilities::default(),
            snapshot_dirty: false,
        }
    }

    fn ensure_default_workspace(&mut self) -> Result<(), String> {
        if !self.workspace.sessions().is_empty() {
            return Ok(());
        }

        let launch = user_shell_launch("Shell 1", "Generic command session");
        self.add_shell_session_without_watch(launch)?;
        self.snapshot_dirty = true;
        Ok(())
    }

    fn add_shell_session_without_watch(
        &mut self,
        launch: SessionLaunch,
    ) -> Result<SessionId, String> {
        self.add_shell_session_with_visibility(launch, true)
    }

    fn add_shell_session_with_visibility(
        &mut self,
        mut launch: SessionLaunch,
        top_level: bool,
    ) -> Result<SessionId, String> {
        let idx = self.workspace.sessions().len();
        apply_terminal_display_env(&mut launch, &self.terminal_display_capabilities);
        launch.env.push(("EXATERM_IDX".into(), idx.to_string()));
        launch
            .env
            .push(("EXATERM_IDX_1".into(), (idx + 1).to_string()));
        let session_id = self.workspace.add_session(launch.clone());
        if top_level {
            self.workspace_items
                .push(WorkspaceItem::Session(session_id));
        }
        self.observations
            .insert(session_id, SessionObservation::new());
        self.observation_cache
            .insert(session_id, ObservationCacheEntry::new());
        self.replay_buffers.insert(session_id, Vec::new());
        self.session_streams
            .insert(session_id, create_session_stream_state(session_id)?);
        let size = PtySize {
            rows: CANONICAL_TERMINAL_ROWS,
            cols: CANONICAL_TERMINAL_COLS,
            pixel_width: 0,
            pixel_height: 0,
        };
        let runtime = spawn_headless_runtime(&launch, size)?;
        if let Some(pid) = runtime.pid {
            self.workspace.mark_spawned(session_id, pid);
        }
        self.runtimes.insert(session_id, runtime.session_runtime);
        Ok(session_id)
    }

    fn attach_repo_watch(
        &mut self,
        session_id: SessionId,
        launch: &SessionLaunch,
        control_tx: &ControlNotifier,
    ) -> Result<(), String> {
        let Some(cwd) = launch.cwd.as_deref() else {
            if let Some(observation) = self.observations.get_mut(&session_id) {
                clear_file_activity(observation);
            }
            return Ok(());
        };
        let Some(repo_root) = find_git_worktree_root(cwd) else {
            if let Some(observation) = self.observations.get_mut(&session_id) {
                clear_file_activity(observation);
            }
            return Ok(());
        };

        self.session_repo_roots
            .insert(session_id, repo_root.clone());
        if let Some(watch) = self.repo_watches.get_mut(&repo_root) {
            watch.sessions.insert(session_id);
            return Ok(());
        }

        let notifier = control_tx.clone();
        let repo_root_for_thread = repo_root.clone();
        let handle = spawn_repo_watch(repo_root.clone(), move |relative_path| {
            let _ = notifier.send(ClientControl::FileActivity {
                repo_root: repo_root_for_thread.clone(),
                relative_path,
            });
        })?;
        let mut sessions = BTreeSet::new();
        sessions.insert(session_id);
        self.repo_watches
            .insert(repo_root, RepoWatchState { sessions, handle });
        Ok(())
    }

    fn workspace_snapshot(&self) -> WorkspaceSnapshot {
        WorkspaceSnapshot {
            items: self.visible_workspace_items(),
            sessions: self
                .workspace
                .sessions()
                .iter()
                .cloned()
                .map(|record| {
                    let record_id = record.id;
                    let observation = self
                        .observations
                        .get(&record_id)
                        .map(observation_snapshot)
                        .unwrap_or_default();
                    SessionSnapshot {
                        record,
                        observation,
                        raw_stream_socket_name: self
                            .session_streams
                            .get(&record_id)
                            .map(|stream| stream.socket_name.clone()),
                    }
                })
                .collect(),
            groups: self.group_snapshots(),
        }
    }

    fn visible_workspace_items(&self) -> Vec<WorkspaceItem> {
        let session_ids = self
            .workspace
            .sessions()
            .iter()
            .map(|session| session.id)
            .collect::<BTreeSet<_>>();
        let group_ids = self.groups.keys().copied().collect::<BTreeSet<_>>();
        self.workspace_items
            .iter()
            .copied()
            .filter(|item| match item {
                WorkspaceItem::Session(session_id) => session_ids.contains(session_id),
                WorkspaceItem::Group(group_id) => group_ids.contains(group_id),
            })
            .collect()
    }

    fn group_snapshots(&self) -> Vec<SupervisedGroupRecord> {
        self.groups
            .values()
            .map(|group| {
                let latest_action_age_secs = group
                    .actions
                    .last()
                    .map(|action| action.at.elapsed().as_secs());
                SupervisedGroupRecord {
                    id: group.id,
                    name: group.name.clone(),
                    member_session_ids: group.member_session_ids.clone(),
                    supervisor_session_id: group.supervisor_session_id,
                    provider: group.provider,
                    goal: group.goal.clone(),
                    summary_markdown: group.summary_markdown.clone(),
                    supervisor_visible: group.supervisor_visible,
                    summary_age_secs: group
                        .summary_updated_at
                        .map(|updated| updated.elapsed().as_secs()),
                    latest_action_age_secs,
                    actions: group
                        .actions
                        .iter()
                        .rev()
                        .take(8)
                        .map(|action| SupervisorActionRecord {
                            sequence: action.sequence,
                            summary: action.summary.clone(),
                            age_secs: action.at.elapsed().as_secs(),
                        })
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .collect(),
                }
            })
            .collect()
    }

    fn shutdown_workspace(&mut self) {
        self.runtimes.clear();
        self.workspace_items.clear();
        self.groups.clear();
        self.observations.clear();
        self.observation_cache.clear();
        self.replay_buffers.clear();
        for stream in self.session_streams.values() {
            let _ = fs::remove_file(&stream.socket_path);
        }
        self.session_streams.clear();
        for (_, watch) in std::mem::take(&mut self.repo_watches) {
            watch.handle.stop();
        }
        self.session_repo_roots.clear();
        self.forwarded_sessions.clear();
        self.input_sync_enabled = false;
        self.input_sync_scope = InputSyncScope::RootVisible;
        self.workspace.replace_sessions(Vec::new());
        self.snapshot_dirty = true;
    }
}

enum ClientControl {
    Message(ClientMessage),
    McpToolCall {
        name: String,
        arguments: Value,
        response: mpsc::Sender<ToolCallOutcome>,
    },
    ControlDisconnected,
    StreamDisconnected(SessionId),
    TerminalInputBytes {
        source_session: SessionId,
        bytes: Vec<u8>,
    },
    FileActivity {
        repo_root: PathBuf,
        relative_path: String,
    },
    RuntimeEvent(SessionId, RuntimeEvent),
}

pub struct LocalBeachheadClient {
    pub commands: mpsc::Sender<ClientMessage>,
    pub events: crossbeam_channel::Receiver<ServerMessage>,
    event_wake_reader: std::sync::Mutex<UnixStream>,
}

impl LocalBeachheadClient {
    pub fn connect_or_spawn() -> Result<Self, String> {
        let control = connect_or_spawn_control_socket()?;
        Self::connect_control(control)
    }

    pub fn connect_control(control: UnixStream) -> Result<Self, String> {
        let control_writer = control
            .try_clone()
            .map_err(|error| format!("failed to clone host session socket: {error}"))?;
        let control_reader = control;
        let (event_wake_reader, mut event_wake_writer) = UnixStream::pair()
            .map_err(|error| format!("failed to create event wake socket: {error}"))?;
        event_wake_reader
            .set_nonblocking(true)
            .map_err(|error| format!("failed to set event wake reader nonblocking: {error}"))?;
        event_wake_writer
            .set_nonblocking(true)
            .map_err(|error| format!("failed to set event wake writer nonblocking: {error}"))?;

        let (command_tx, command_rx) = mpsc::channel::<ClientMessage>();
        let (event_tx, event_rx) = crossbeam_channel::unbounded::<ServerMessage>();

        thread::spawn(move || {
            let mut writer = control_writer;
            while let Ok(message) = command_rx.recv() {
                if write_json_line(&mut writer, &message).is_err() {
                    break;
                }
            }
        });

        thread::spawn(move || {
            let mut reader = BufReader::new(control_reader);
            loop {
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) => break,
                    Ok(_) => {
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }
                        match serde_json::from_str::<ServerMessage>(trimmed) {
                            Ok(message) => {
                                if event_tx.send(message).is_err() {
                                    break;
                                }
                                match event_wake_writer.write(&[1]) {
                                    Ok(_) => {}
                                    Err(error)
                                        if error.kind() == std::io::ErrorKind::WouldBlock => {}
                                    Err(_) => break,
                                }
                            }
                            Err(_) => break,
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        let _ = command_tx.send(ClientMessage::AttachClient);

        Ok(Self {
            commands: command_tx,
            events: event_rx,
            event_wake_reader: std::sync::Mutex::new(event_wake_reader),
        })
    }

    pub fn event_wake_fd(&self) -> i32 {
        self.event_wake_reader
            .lock()
            .expect("event wake reader lock poisoned")
            .as_raw_fd()
    }

    pub fn drain_event_wake(&self) {
        let Ok(mut reader) = self.event_wake_reader.lock() else {
            return;
        };
        let mut buf = [0u8; 256];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
    }
}

fn spawn_mcp_client(stream: UnixStream, control_tx: ControlNotifier) {
    thread::spawn(move || {
        let reader = match stream.try_clone() {
            Ok(reader) => BufReader::new(reader),
            Err(_) => return,
        };
        let writer = stream;
        let dispatcher = move |name: &str, arguments: Value| -> ToolCallOutcome {
            let (response_tx, response_rx) = mpsc::channel();
            control_tx
                .send(ClientControl::McpToolCall {
                    name: name.to_string(),
                    arguments,
                    response: response_tx,
                })
                .map_err(|_| ToolCallError::new("Exaterm daemon is not accepting MCP calls"))?;
            response_rx
                .recv_timeout(Duration::from_secs(30))
                .map_err(|_| ToolCallError::new("Exaterm MCP tool call timed out"))?
        };
        let server = McpServer::new(
            ServerInfo::new("exaterm-persistent-sessions", env!("CARGO_PKG_VERSION")),
            exaterm_mcp_tools(),
            dispatcher,
        );
        let _ = server.serve(reader, writer);
    });
}

fn exaterm_mcp_tools() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition::new(
            "exaterm_list_groups",
            "List supervised groups and their worker sessions.",
            json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        ),
        ToolDefinition::new(
            "exaterm_get_group",
            "Read a supervised group's summary, members, observations, and recent supervisor actions.",
            json!({
                "type": "object",
                "properties": {
                    "group_id": { "type": "integer", "minimum": 1 }
                },
                "required": ["group_id"],
                "additionalProperties": false
            }),
        ),
        ToolDefinition::new(
            "exaterm_send_message_to_agent",
            "Send a direct message into one worker terminal in a supervised group.",
            json!({
                "type": "object",
                "properties": {
                    "group_id": { "type": "integer", "minimum": 1 },
                    "session_id": { "type": "integer", "minimum": 1 },
                    "message": { "type": "string", "minLength": 1 }
                },
                "required": ["group_id", "session_id", "message"],
                "additionalProperties": false
            }),
        ),
        ToolDefinition::new(
            "exaterm_update_group_summary",
            "Replace the operator-facing Markdown summary for a supervised group. Use natural Markdown, including tables when useful.",
            json!({
                "type": "object",
                "properties": {
                    "group_id": { "type": "integer", "minimum": 1 },
                    "markdown": { "type": "string" }
                },
                "required": ["group_id", "markdown"],
                "additionalProperties": false
            }),
        ),
    ]
}

pub fn run_local_daemon() -> ExitCode {
    match run_local_daemon_inner() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(1)
        }
    }
}

fn run_local_daemon_inner() -> Result<(), String> {
    apply_app_config_environment(&load_app_config());

    let control_socket_path = control_socket_path()?;
    let mcp_socket_path = mcp_socket_path()?;
    if let Some(parent) = control_socket_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create daemon socket dir: {error}"))?;
    }
    clear_stale_control_socket(&control_socket_path)?;
    clear_stale_mcp_socket(&mcp_socket_path)?;

    let control_listener = UnixListener::bind(&control_socket_path)
        .map_err(|error| format!("failed to bind daemon control socket: {error}"))?;
    control_listener
        .set_nonblocking(true)
        .map_err(|error| format!("failed to set daemon control socket nonblocking: {error}"))?;
    let mcp_listener = UnixListener::bind(&mcp_socket_path)
        .map_err(|error| format!("failed to bind daemon MCP socket: {error}"))?;
    mcp_listener
        .set_nonblocking(true)
        .map_err(|error| format!("failed to set daemon MCP socket nonblocking: {error}"))?;

    let (control_tx, control_rx) = mpsc::channel::<ClientControl>();
    let (mut wake_reader, wake_writer) = UnixStream::pair()
        .map_err(|error| format!("failed to create daemon wake socket: {error}"))?;
    wake_reader
        .set_nonblocking(true)
        .map_err(|error| format!("failed to set daemon wake reader nonblocking: {error}"))?;
    wake_writer
        .set_nonblocking(true)
        .map_err(|error| format!("failed to set daemon wake writer nonblocking: {error}"))?;
    let control_notifier = ControlNotifier {
        tx: control_tx,
        wake: std::sync::Arc::new(std::sync::Mutex::new(wake_writer)),
    };
    let mut client_writer: Option<UnixStream> = None;
    let mut state = DaemonState::new();
    let mut last_refresh = Instant::now() - REFRESH_INTERVAL;
    let mut should_exit = false;

    while !should_exit {
        let control_ready;
        let mcp_ready;
        let wake_ready;
        let mut ready_session_ids = Vec::new();
        {
            let timeout = refresh_timeout_ms(last_refresh.elapsed());
            let session_ids = state.session_streams.keys().copied().collect::<Vec<_>>();
            let mut pollfds = vec![
                libc::pollfd {
                    fd: control_listener.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                },
                libc::pollfd {
                    fd: mcp_listener.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                },
                libc::pollfd {
                    fd: wake_reader.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                },
            ];
            for session_id in &session_ids {
                let stream = state
                    .session_streams
                    .get(session_id)
                    .expect("session stream should exist while polling");
                pollfds.push(libc::pollfd {
                    fd: stream.listener.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                });
            }

            let poll_result =
                unsafe { libc::poll(pollfds.as_mut_ptr(), pollfds.len() as libc::nfds_t, timeout) };
            if poll_result < 0 {
                let error = std::io::Error::last_os_error();
                if error.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(format!("daemon poll failed: {error}"));
            }

            control_ready = pollfds[0].revents & libc::POLLIN != 0;
            mcp_ready = pollfds[1].revents & libc::POLLIN != 0;
            wake_ready = pollfds[2].revents & libc::POLLIN != 0;
            for (index, session_id) in session_ids.into_iter().enumerate() {
                if pollfds[index + 3].revents & libc::POLLIN != 0 {
                    ready_session_ids.push(session_id);
                }
            }
        }

        if control_ready {
            loop {
                match control_listener.accept() {
                    Ok((stream, _)) => {
                        // Accepted sockets inherit non-blocking on macOS; reset to blocking
                        // so the client reader thread can use blocking read_line.
                        let _ = stream.set_nonblocking(false);
                        if client_writer.is_some() {
                            let mut stream = stream;
                            let _ = write_json_line(
                                &mut stream,
                                &ServerMessage::Error {
                                    message: "another Exaterm client is already attached".into(),
                                },
                            );
                            continue;
                        }
                        let reader = stream
                            .try_clone()
                            .map_err(|error| format!("failed to clone client stream: {error}"))?;
                        spawn_client_reader(reader, control_notifier.clone());
                        client_writer = Some(stream);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(error) => return Err(format!("daemon control accept failed: {error}")),
                }
            }
        }

        if mcp_ready {
            loop {
                match mcp_listener.accept() {
                    Ok((stream, _)) => {
                        let _ = stream.set_nonblocking(false);
                        spawn_mcp_client(stream, control_notifier.clone());
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(error) => return Err(format!("daemon MCP accept failed: {error}")),
                }
            }
        }

        if wake_ready {
            drain_wake_socket(&mut wake_reader);
        }

        for session_id in ready_session_ids {
            let Some(stream) = state.session_streams.get(&session_id) else {
                continue;
            };
            loop {
                match stream.listener.accept() {
                    Ok((socket, _)) => {
                        // Accepted sockets inherit non-blocking on macOS; reset to blocking.
                        let _ = socket.set_nonblocking(false);
                        let reader = socket.try_clone().map_err(|error| {
                            format!("failed to clone session raw stream: {error}")
                        })?;
                        let Some(input_writer) = state
                            .runtimes
                            .get(&session_id)
                            .and_then(|runtime| runtime.input_writer.as_ref().cloned())
                        else {
                            continue;
                        };
                        spawn_raw_stream_reader(
                            reader,
                            input_writer,
                            control_notifier.clone(),
                            session_id,
                        );
                        if let Ok(mut guard) = stream.writer.lock() {
                            *guard = Some(socket);
                            if let Some(writer) = guard.as_mut() {
                                if let Some(replay) = state.replay_buffers.get(&session_id) {
                                    if !replay.is_empty() {
                                        let _ = writer.write_all(replay);
                                    }
                                }
                            }
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(error) => {
                        return Err(format!(
                            "daemon raw accept failed for session {:?}: {error}",
                            session_id
                        ));
                    }
                }
            }
        }

        for _ in 0..CONTROL_EVENTS_PER_TICK {
            let Ok(control) = control_rx.try_recv() else {
                break;
            };
            match control {
                ClientControl::Message(message) => {
                    if handle_client_message(
                        &mut state,
                        &mut client_writer,
                        &control_notifier,
                        message,
                    )? {
                        should_exit = true;
                    }
                }
                ClientControl::McpToolCall {
                    name,
                    arguments,
                    response,
                } => {
                    let result =
                        handle_mcp_tool_call(&mut state, &control_notifier, &name, arguments);
                    let _ = response.send(result);
                }
                ClientControl::ControlDisconnected => {
                    client_writer = None;
                }
                ClientControl::StreamDisconnected(session_id) => {
                    if let Some(stream) = state.session_streams.get(&session_id) {
                        if let Ok(mut guard) = stream.writer.lock() {
                            *guard = None;
                        }
                    }
                }
                ClientControl::TerminalInputBytes {
                    source_session,
                    bytes,
                } => {
                    note_terminal_input_activity(&mut state, source_session);
                    fanout_synced_terminal_input(&mut state, source_session, &bytes);
                }
                ClientControl::FileActivity {
                    repo_root,
                    relative_path,
                } => {
                    if let Some(watch) = state.repo_watches.get(&repo_root) {
                        let now = Instant::now();
                        for session_id in &watch.sessions {
                            if let Some(observation) = state.observations.get_mut(session_id) {
                                apply_file_activity(observation, relative_path.clone(), now);
                            }
                        }
                    }
                }
                ClientControl::RuntimeEvent(session_id, event) => {
                    handle_runtime_event(&mut state, &mut client_writer, session_id, event);
                }
            }
        }

        let runtime_changed = false;
        let worker_changed = drain_worker_results(&mut state, &mut client_writer);
        if runtime_changed || worker_changed {
            state.snapshot_dirty = true;
        }

        if last_refresh.elapsed() >= REFRESH_INTERVAL {
            refresh_state(&mut state);
            last_refresh = Instant::now();
        }

        if state.snapshot_dirty {
            if let Some(writer) = client_writer.as_mut() {
                let snapshot = state.workspace_snapshot();
                let _ = write_json_line(writer, &ServerMessage::WorkspaceSnapshot { snapshot });
            }
            state.snapshot_dirty = false;
        }
    }

    let _ = fs::remove_file(&control_socket_path);
    let _ = fs::remove_file(&mcp_socket_path);
    Ok(())
}

fn handle_client_message(
    state: &mut DaemonState,
    client_writer: &mut Option<UnixStream>,
    control_tx: &ControlNotifier,
    message: ClientMessage,
) -> Result<bool, String> {
    match message {
        ClientMessage::AttachClient => {
            if let Some(writer) = client_writer.as_mut() {
                let snapshot = state.workspace_snapshot();
                let _ = write_json_line(writer, &ServerMessage::WorkspaceSnapshot { snapshot });
            }
            Ok(false)
        }
        ClientMessage::SetTerminalDisplayCapabilities { capabilities } => {
            state.terminal_display_capabilities = capabilities;
            Ok(false)
        }
        ClientMessage::CreateOrResumeDefaultWorkspace => {
            state.ensure_default_workspace()?;
            if let Some(session) = state.workspace.sessions().first().cloned() {
                state.attach_repo_watch(session.id, &session.launch, control_tx)?;
                let session_id = session.id;
                ensure_runtime_forwarder(state, session_id, control_tx.clone());
            }
            state.snapshot_dirty = true;
            Ok(false)
        }
        ClientMessage::AddTerminals { source_session } => {
            let count = additions_for_session_count(state.workspace.sessions().len());
            if count > 0 {
                add_n_terminals(state, source_session, count, control_tx)?;
            }
            Ok(false)
        }
        ClientMessage::AddTerminalsTo {
            source_session,
            target_total,
        } => {
            let current_total = state.workspace.sessions().len();
            if target_total > current_total && supported_terminal_target(target_total) {
                add_n_terminals(
                    state,
                    source_session,
                    target_total - current_total,
                    control_tx,
                )?;
            }
            Ok(false)
        }
        ClientMessage::AddOneTerminal { source_session } => {
            add_n_terminals(state, source_session, 1, control_tx)?;
            Ok(false)
        }
        ClientMessage::CloseSession { session_id } => {
            state.runtimes.remove(&session_id);
            state.observations.remove(&session_id);
            state.observation_cache.remove(&session_id);
            state.replay_buffers.remove(&session_id);
            state
                .workspace_items
                .retain(|item| *item != WorkspaceItem::Session(session_id));
            let mut empty_groups = Vec::new();
            for group in state.groups.values_mut() {
                group.member_session_ids.retain(|id| *id != session_id);
                if group.supervisor_session_id == Some(session_id) {
                    group.supervisor_session_id = None;
                }
                if group.member_session_ids.is_empty() {
                    empty_groups.push(group.id);
                }
            }
            for group_id in empty_groups {
                state.groups.remove(&group_id);
                state
                    .workspace_items
                    .retain(|item| *item != WorkspaceItem::Group(group_id));
            }
            if let Some(stream) = state.session_streams.remove(&session_id) {
                let _ = fs::remove_file(&stream.socket_path);
            }
            if let Some(root) = state.session_repo_roots.remove(&session_id) {
                let still_used = state.session_repo_roots.values().any(|r| r == &root);
                if !still_used {
                    if let Some(watch) = state.repo_watches.remove(&root) {
                        watch.handle.stop();
                    }
                }
            }
            state.forwarded_sessions.remove(&session_id);
            state.workspace.remove_session(session_id);
            state.snapshot_dirty = true;
            Ok(false)
        }
        ClientMessage::ResizeTerminal {
            session_id,
            rows,
            cols,
        } => {
            if let Some(runtime) = state.runtimes.get_mut(&session_id) {
                let size = PtySize {
                    rows,
                    cols,
                    pixel_width: 0,
                    pixel_height: 0,
                };
                if runtime.last_size != Some((rows, cols)) {
                    if let Ok(master) = runtime.resize_target.lock() {
                        let _ = master.resize(size);
                    }
                    runtime.last_size = Some((rows, cols));
                }
            }
            Ok(false)
        }
        ClientMessage::CreateSupervisedGroup {
            name,
            session_ids,
            goal,
        } => {
            create_supervised_group(state, control_tx, name, session_ids, goal)?;
            state.snapshot_dirty = true;
            Ok(false)
        }
        ClientMessage::SetGroupSupervisorVisible { group_id, visible } => {
            let group = state
                .groups
                .get_mut(&group_id)
                .ok_or_else(|| format!("unknown supervised group {}", group_id.0))?;
            group.supervisor_visible = visible;
            state.snapshot_dirty = true;
            Ok(false)
        }
        ClientMessage::SetInputSync { enabled, scope } => {
            state.input_sync_enabled = enabled;
            state.input_sync_scope = scope;
            Ok(false)
        }
        ClientMessage::SendMessageToAgent {
            group_id,
            session_id,
            message,
        } => {
            send_group_message_to_agent(state, group_id, session_id, &message)?;
            state.snapshot_dirty = true;
            Ok(false)
        }
        ClientMessage::UpdateGroupSummary { group_id, markdown } => {
            update_group_summary(state, group_id, &markdown)?;
            state.snapshot_dirty = true;
            Ok(false)
        }
        ClientMessage::ConfigureTerminalAssist {
            openai_api_key,
            openai_base_url,
            model,
        } => {
            apply_terminal_assist_config_environment(&TerminalAssistConfig {
                openai_api_key: openai_api_key.unwrap_or_default(),
                openai_base_url,
                model,
            });
            state.assist_worker = spawn_assist_worker();
            Ok(false)
        }
        ClientMessage::RequestTerminalAssist {
            request_id,
            session_id,
            prompt,
        } => {
            if let Err(error) = queue_terminal_assist(state, request_id, session_id, &prompt) {
                if let Some(writer) = client_writer.as_mut() {
                    let _ = write_json_line(
                        writer,
                        &ServerMessage::TerminalAssistCompleted {
                            request_id,
                            session_id,
                            inserted: false,
                            error: Some(error),
                        },
                    );
                }
            }
            Ok(false)
        }
        ClientMessage::DetachClient { keep_alive } => {
            *client_writer = None;
            if keep_alive {
                Ok(false)
            } else {
                state.shutdown_workspace();
                Ok(true)
            }
        }
        ClientMessage::TerminateWorkspace => {
            state.shutdown_workspace();
            Ok(true)
        }
    }
}

fn handle_mcp_tool_call(
    state: &mut DaemonState,
    _control_tx: &ControlNotifier,
    name: &str,
    arguments: Value,
) -> ToolCallOutcome {
    match name {
        "exaterm_list_groups" => {
            let groups = state.group_snapshots();
            let structured = json!({
                "visible_items": state.visible_workspace_items(),
                "groups": groups,
            });
            let text = if state.groups.is_empty() {
                "No supervised groups are active.".to_string()
            } else {
                state
                    .groups
                    .values()
                    .map(|group| {
                        format!(
                            "Group {}: {} ({} worker terminals)",
                            group.id.0,
                            group.name,
                            group.member_session_ids.len()
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            Ok(ToolCallResult::structured(structured, text))
        }
        "exaterm_get_group" => {
            let group_id = GroupId(argument_u32(&arguments, "group_id")?);
            let structured = mcp_group_detail(state, group_id)?;
            let name = structured
                .get("group")
                .and_then(|group| group.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("group")
                .to_string();
            Ok(ToolCallResult::structured(
                structured,
                format!("Snapshot for group {} ({name})", group_id.0),
            ))
        }
        "exaterm_send_message_to_agent" => {
            let group_id = GroupId(argument_u32(&arguments, "group_id")?);
            let session_id = SessionId(argument_u32(&arguments, "session_id")?);
            let message = argument_string(&arguments, "message")?;
            send_group_message_to_agent(state, group_id, session_id, &message)
                .map_err(ToolCallError::new)?;
            state.snapshot_dirty = true;
            Ok(ToolCallResult::structured(
                json!({
                    "group_id": group_id.0,
                    "session_id": session_id.0,
                    "sent": true
                }),
                format!("Sent message to session {}", session_id.0),
            ))
        }
        "exaterm_update_group_summary" => {
            let group_id = GroupId(argument_u32(&arguments, "group_id")?);
            let markdown = argument_string(&arguments, "markdown")?;
            update_group_summary(state, group_id, &markdown).map_err(ToolCallError::new)?;
            state.snapshot_dirty = true;
            Ok(ToolCallResult::structured(
                json!({
                    "group_id": group_id.0,
                    "updated": true
                }),
                format!("Updated summary for group {}", group_id.0),
            ))
        }
        _ => Err(ToolCallError::new(format!(
            "unknown Exaterm MCP tool: {name}"
        ))),
    }
}

fn mcp_group_detail(state: &DaemonState, group_id: GroupId) -> Result<Value, ToolCallError> {
    let group = state
        .groups
        .get(&group_id)
        .ok_or_else(|| ToolCallError::new(format!("unknown supervised group {}", group_id.0)))?;
    let group_record = state
        .group_snapshots()
        .into_iter()
        .find(|record| record.id == group_id)
        .ok_or_else(|| ToolCallError::new(format!("unknown supervised group {}", group_id.0)))?;
    let sessions = group
        .member_session_ids
        .iter()
        .filter_map(|session_id| {
            let record = state.workspace.session(*session_id)?;
            let observation = state
                .observations
                .get(session_id)
                .map(observation_snapshot)
                .unwrap_or_default();
            Some(json!({
                "session_id": session_id.0,
                "name": record.launch.name.clone(),
                "display_name": record.display_name.clone(),
                "status": record.status,
                "pid": record.pid,
                "observation": observation,
            }))
        })
        .collect::<Vec<_>>();

    Ok(json!({
        "group": group_record,
        "worker_sessions": sessions,
    }))
}

fn argument_u32(arguments: &Value, key: &str) -> Result<u32, ToolCallError> {
    let camel = to_camel_case(key);
    arguments
        .get(key)
        .or_else(|| arguments.get(camel.as_str()))
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| ToolCallError::new(format!("missing or invalid integer argument `{key}`")))
}

fn argument_string(arguments: &Value, key: &str) -> Result<String, ToolCallError> {
    let camel = to_camel_case(key);
    arguments
        .get(key)
        .or_else(|| arguments.get(camel.as_str()))
        .and_then(Value::as_str)
        .map(|value| value.to_string())
        .ok_or_else(|| ToolCallError::new(format!("missing or invalid string argument `{key}`")))
}

fn to_camel_case(key: &str) -> String {
    let mut output = String::new();
    let mut uppercase_next = false;
    for ch in key.chars() {
        if ch == '_' {
            uppercase_next = true;
        } else if uppercase_next {
            output.extend(ch.to_uppercase());
            uppercase_next = false;
        } else {
            output.push(ch);
        }
    }
    output
}

fn create_supervised_group(
    state: &mut DaemonState,
    control_tx: &ControlNotifier,
    name: String,
    session_ids: Vec<SessionId>,
    goal: Option<String>,
) -> Result<GroupId, String> {
    let mut members = Vec::new();
    for session_id in session_ids {
        if members.contains(&session_id) {
            continue;
        }
        if state.workspace.session(session_id).is_some() {
            members.push(session_id);
        }
    }
    if members.is_empty() {
        return Err("cannot create supervised group without valid member sessions".into());
    }

    let group_id = GroupId(state.next_group_id);
    state.next_group_id = state.next_group_id.saturating_add(1);
    let name = sanitize_group_name(&name).unwrap_or_else(|| format!("Group {}", group_id.0));
    let provider = detect_supervisor_provider(state, &members);
    let supervisor_session_id = Some(spawn_supervisor_session(
        state,
        control_tx,
        group_id,
        &name,
        provider,
        goal.as_deref(),
    )?);

    let insert_at = state
        .workspace_items
        .iter()
        .position(|item| match item {
            WorkspaceItem::Session(session_id) => members.contains(session_id),
            WorkspaceItem::Group(_) => false,
        })
        .unwrap_or(state.workspace_items.len());
    state.workspace_items.retain(
        |item| !matches!(item, WorkspaceItem::Session(session_id) if members.contains(session_id)),
    );
    state.workspace_items.insert(
        insert_at.min(state.workspace_items.len()),
        WorkspaceItem::Group(group_id),
    );

    state.groups.insert(
        group_id,
        SupervisedGroupState {
            id: group_id,
            name,
            member_session_ids: members,
            supervisor_session_id,
            provider,
            goal,
            summary_markdown: "Supervisor is starting. No summary yet.".into(),
            supervisor_visible: false,
            summary_updated_at: None,
            actions: Vec::new(),
        },
    );

    Ok(group_id)
}

fn sanitize_group_name(name: &str) -> Option<String> {
    let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
    let name = name.trim();
    (!name.is_empty()).then(|| name.chars().take(60).collect())
}

fn detect_supervisor_provider(
    state: &DaemonState,
    member_session_ids: &[SessionId],
) -> Option<SupervisorProvider> {
    let mut codex = 0usize;
    let mut claude = 0usize;
    for session_id in member_session_ids {
        let Some(observation) = state.observations.get(session_id) else {
            continue;
        };
        for value in [
            observation.shell_child_command.as_deref(),
            observation.dominant_process.as_deref(),
            observation.active_command.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            let lower = value.to_ascii_lowercase();
            if lower.contains("codex") {
                codex += 1;
            }
            if lower.contains("claude") {
                claude += 1;
            }
        }
    }
    if codex > 0 || claude > 0 {
        return if codex >= claude {
            Some(SupervisorProvider::Codex)
        } else {
            Some(SupervisorProvider::Claude)
        };
    }
    if command_exists("codex") {
        Some(SupervisorProvider::Codex)
    } else if command_exists("claude") {
        Some(SupervisorProvider::Claude)
    } else {
        Some(SupervisorProvider::Other)
    }
}

fn command_exists(program: &str) -> bool {
    Command::new("sh")
        .arg("-lc")
        .arg(format!("command -v {}", shell_quote(program)))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

fn spawn_supervisor_session(
    state: &mut DaemonState,
    control_tx: &ControlNotifier,
    group_id: GroupId,
    group_name: &str,
    provider: Option<SupervisorProvider>,
    goal: Option<&str>,
) -> Result<SessionId, String> {
    let prompt = supervisor_prompt(group_id, group_name, goal);
    let mut launch = match provider {
        Some(SupervisorProvider::Codex) if command_exists("codex") => command_launch(
            format!("{group_name} Supervisor"),
            "Supervisor agent",
            SessionKind::RunningStream,
            "/usr/bin/env",
            vec![
                "codex".into(),
                "--dangerously-bypass-approvals-and-sandbox".into(),
                prompt,
            ],
        ),
        Some(SupervisorProvider::Claude) if command_exists("claude") => command_launch(
            format!("{group_name} Supervisor"),
            "Supervisor agent",
            SessionKind::RunningStream,
            "/usr/bin/env",
            vec![
                "claude".into(),
                "--dangerously-skip-permissions".into(),
                prompt,
            ],
        ),
        _ => command_launch(
            format!("{group_name} Supervisor"),
            "Supervisor shell",
            SessionKind::WaitingShell,
            "/usr/bin/env",
            vec![
                "bash".into(),
                "--noprofile".into(),
                "--norc".into(),
                "-ic".into(),
                format!(
                    "printf '%s\\r\\n' {}; exec bash --noprofile --norc -i",
                    shell_quote(&prompt)
                ),
            ],
        ),
    };
    launch
        .env
        .push(("EXATERM_SUPERVISED_GROUP_ID".into(), group_id.0.to_string()));
    launch.env.push((
        "EXATERM_MCP_SOCKET".into(),
        mcp_socket_path()?.to_string_lossy().into_owned(),
    ));
    launch
        .env
        .push(("EXATERM_MCP_TRANSPORT".into(), "unix-jsonrpc-lines".into()));
    let session_id = state.add_shell_session_with_visibility(launch, false)?;
    ensure_runtime_forwarder(state, session_id, control_tx.clone());
    Ok(session_id)
}

fn supervisor_prompt(group_id: GroupId, group_name: &str, goal: Option<&str>) -> String {
    let goal = goal.unwrap_or("Supervise this terminal group with a light touch.");
    format!(
        "You are the Exaterm supervisor for group {id} ({name}). Goal: {goal}\n\
	Use Exaterm MCP tools as your source of truth when available. The MCP endpoint is advertised in EXATERM_MCP_SOCKET and speaks newline-delimited JSON-RPC over a Unix socket. \
	Keep the operator-facing summary in natural Markdown, using tables when they help. \
	Use exaterm_send_message_to_agent when a worker needs a prod, redirect, or cross-pollinated idea. Use exaterm_update_group_summary separately when the user-facing summary should change. \
	Assess the overall group conservatively: Active means useful work is still moving; Stalling means that despite supervisor efforts forward progress is not being made; Blocked means a substantial proportion of the agents cannot proceed at all. \
	Do not call the group Blocked for ordinary compile errors, failing tests, one worker being stuck while others can continue, or visible debugging work.",
        id = group_id.0,
        name = group_name
    )
}

fn send_group_message_to_agent(
    state: &mut DaemonState,
    group_id: GroupId,
    session_id: SessionId,
    message: &str,
) -> Result<(), String> {
    let message = sanitize_agent_message(message)?;
    let group = state
        .groups
        .get(&group_id)
        .ok_or_else(|| format!("unknown supervised group {}", group_id.0))?;
    if !group.member_session_ids.contains(&session_id) {
        return Err(format!(
            "session {} is not a worker in supervised group {}",
            session_id.0, group_id.0
        ));
    }
    send_runtime_input_line(state, session_id, &message).map_err(|error| {
        format!(
            "failed to send message to session {}: {error}",
            session_id.0
        )
    })?;
    record_group_action(
        state,
        group_id,
        format!("Sent message to session {}: {}", session_id.0, message),
    );
    Ok(())
}

fn sanitize_agent_message(message: &str) -> Result<String, String> {
    let collapsed = message.trim().replace('\r', "\n");
    if collapsed.is_empty() {
        return Err("message cannot be empty".into());
    }
    Ok(collapsed.chars().take(2000).collect())
}

fn update_group_summary(
    state: &mut DaemonState,
    group_id: GroupId,
    markdown: &str,
) -> Result<(), String> {
    let group = state
        .groups
        .get_mut(&group_id)
        .ok_or_else(|| format!("unknown supervised group {}", group_id.0))?;
    let markdown = markdown.trim();
    group.summary_markdown = if markdown.is_empty() {
        "No supervisor summary yet.".into()
    } else {
        markdown.chars().take(50_000).collect()
    };
    group.summary_updated_at = Some(Instant::now());
    Ok(())
}

fn record_group_action(state: &mut DaemonState, group_id: GroupId, summary: impl Into<String>) {
    let Some(group) = state.groups.get_mut(&group_id) else {
        return;
    };
    let sequence = state.next_supervisor_action_sequence;
    state.next_supervisor_action_sequence = state.next_supervisor_action_sequence.saturating_add(1);
    group.actions.push(SupervisorActionEntry {
        sequence,
        summary: summary.into(),
        at: Instant::now(),
    });
    const MAX_ACTIONS: usize = 64;
    if group.actions.len() > MAX_ACTIONS {
        let extra = group.actions.len() - MAX_ACTIONS;
        group.actions.drain(0..extra);
    }
}

fn queue_terminal_assist(
    state: &mut DaemonState,
    request_id: u64,
    session_id: SessionId,
    prompt: &str,
) -> Result<(), String> {
    let Some(worker) = state.assist_worker.as_ref() else {
        return Err("no terminal assist provider is available".into());
    };
    let prompt = prompt.trim();
    if prompt.is_empty() {
        return Err("assist prompt cannot be empty".into());
    }
    let evidence = build_terminal_assist_evidence(state, session_id, prompt)?;
    let _ = worker.requests.send(AssistJob {
        request_id,
        origin_session_id: session_id,
        evidence,
        preferences: ProviderPreferences::default(),
    });
    Ok(())
}

fn build_terminal_assist_evidence(
    state: &DaemonState,
    session_id: SessionId,
    prompt: &str,
) -> Result<TerminalAssistEvidence, String> {
    let session = state
        .workspace
        .session(session_id)
        .ok_or_else(|| format!("unknown session {}", session_id.0))?;
    let observation = state
        .observations
        .get(&session_id)
        .ok_or_else(|| format!("missing observation for session {}", session_id.0))?;
    Ok(TerminalAssistEvidence {
        session_name: session
            .display_name
            .clone()
            .unwrap_or_else(|| session.launch.name.clone()),
        operator_prompt: prompt.to_string(),
        current_input: observation.painted_line.clone().unwrap_or_default(),
        working_directory: session
            .launch
            .cwd
            .as_ref()
            .map(|path| path.display().to_string()),
        shell_child_command: observation.shell_child_command.clone(),
        active_command: observation.active_command.clone(),
        dominant_process: observation.dominant_process.clone(),
        process_tree_excerpt: observation.process_tree_excerpt.clone(),
        recent_files: observation.recent_files.clone(),
        terminal_status_line: observation.painted_line.clone(),
        recent_terminal_history: terminal_assist_history(observation),
    })
}

fn refresh_state(state: &mut DaemonState) {
    let sessions = state.workspace.sessions().to_vec();
    for session in &sessions {
        maybe_queue_observation_refresh(state, session);
    }
}

fn maybe_queue_observation_refresh(state: &mut DaemonState, session: &crate::model::SessionRecord) {
    let Some(worker) = state.observation_worker.as_ref() else {
        return;
    };
    let entry = state
        .observation_cache
        .entry(session.id)
        .or_insert_with(ObservationCacheEntry::new);
    if entry.in_flight {
        return;
    }
    if entry
        .last_attempt
        .is_some_and(|attempt| attempt.elapsed() < REFRESH_INTERVAL)
    {
        return;
    }
    entry.in_flight = true;
    entry.last_attempt = Some(Instant::now());
    let _ = worker.requests.send(ObservationJob {
        session_id: session.id,
        session: session.clone(),
    });
}

fn ensure_runtime_forwarder(
    state: &mut DaemonState,
    session_id: SessionId,
    control_tx: ControlNotifier,
) {
    if !state.forwarded_sessions.insert(session_id) {
        return;
    }
    let Some(runtime) = state.runtimes.get_mut(&session_id) else {
        return;
    };
    let Some(raw_writer) = state
        .session_streams
        .get(&session_id)
        .map(|stream| stream.writer.clone())
    else {
        return;
    };
    let (_dead_tx, dead_rx) = mpsc::channel();
    let events = std::mem::replace(&mut runtime.events, dead_rx);
    spawn_runtime_forwarder(session_id, events, raw_writer, control_tx);
}

fn drain_worker_results(state: &mut DaemonState, client_writer: &mut Option<UnixStream>) -> bool {
    if let Some(worker) = state.observation_worker.as_ref() {
        while let Ok(result) = worker.responses.try_recv() {
            let entry = state
                .observation_cache
                .entry(result.session_id)
                .or_insert_with(ObservationCacheEntry::new);
            entry.in_flight = false;
            let observation = state.observations.entry(result.session_id).or_default();
            apply_observation_refresh(observation, &result.session, result.refresh);
        }
    }

    loop {
        let result = {
            let Some(worker) = state.assist_worker.as_ref() else {
                break;
            };
            worker.responses.try_recv().ok()
        };
        let Some(result) = result else {
            break;
        };

        let (inserted, error) = match result.suggestion.value {
            Ok(suggestion) if !suggestion.insert_text.trim().is_empty() => {
                match send_terminal_assist_insert_bytes(
                    state,
                    result.origin_session_id,
                    suggestion.insert_text.as_bytes(),
                ) {
                    Ok(()) => (true, None),
                    Err(error) => (false, Some(error.to_string())),
                }
            }
            Ok(_) => (false, Some("terminal assist returned no insertion".into())),
            Err(error) => (false, Some(error)),
        };
        if let Some(writer) = client_writer.as_mut() {
            let _ = write_json_line(
                writer,
                &ServerMessage::TerminalAssistCompleted {
                    request_id: result.request_id,
                    session_id: result.origin_session_id,
                    inserted,
                    error,
                },
            );
        }
    }

    false
}

fn handle_runtime_event(
    state: &mut DaemonState,
    _client_writer: &mut Option<UnixStream>,
    session_id: SessionId,
    event: RuntimeEvent,
) {
    match event {
        RuntimeEvent::Stream(update) => {
            append_replay_buffer(
                state.replay_buffers.entry(session_id).or_default(),
                &update.output_bytes,
            );
            let observation = state.observations.entry(session_id).or_default();
            apply_stream_update(observation, update);
            state.snapshot_dirty = true;
        }
        RuntimeEvent::Exited(exit_code) => {
            state.workspace.mark_exited(session_id, exit_code);
            state.snapshot_dirty = true;
        }
    }
}

fn send_runtime_input_line(
    state: &mut DaemonState,
    session_id: SessionId,
    line: &str,
) -> std::io::Result<()> {
    send_runtime_input_bytes(state, session_id, line.as_bytes())?;
    thread::sleep(TERMINAL_RETURN_DELAY);
    send_runtime_input_bytes(state, session_id, b"\r")
}

fn send_runtime_input_bytes(
    state: &mut DaemonState,
    session_id: SessionId,
    bytes: &[u8],
) -> std::io::Result<()> {
    let writer = state
        .runtimes
        .get(&session_id)
        .and_then(|runtime| runtime.input_writer.as_ref().cloned())
        .ok_or_else(|| std::io::Error::other("runtime input writer missing"))?;
    let mut writer = writer
        .lock()
        .map_err(|_| std::io::Error::other("runtime input writer lock poisoned"))?;
    writer.write_all(bytes)?;
    note_terminal_input_activity(state, session_id);
    Ok(())
}

fn send_terminal_assist_insert_bytes(
    state: &mut DaemonState,
    origin_session: SessionId,
    bytes: &[u8],
) -> std::io::Result<()> {
    let targets = terminal_assist_insert_targets(state, origin_session);
    send_runtime_input_bytes(state, origin_session, bytes)?;
    for target_session in targets {
        if target_session == origin_session {
            continue;
        }
        let _ = send_runtime_input_bytes(state, target_session, bytes);
    }
    Ok(())
}

fn terminal_assist_insert_targets(
    state: &DaemonState,
    origin_session: SessionId,
) -> BTreeSet<SessionId> {
    if !state.input_sync_enabled {
        return BTreeSet::from([origin_session]);
    }

    let targets = input_sync_targets(state);
    if targets.contains(&origin_session) {
        targets
    } else {
        BTreeSet::from([origin_session])
    }
}

fn fanout_synced_terminal_input(state: &mut DaemonState, source_session: SessionId, bytes: &[u8]) {
    if bytes.is_empty() || !state.input_sync_enabled {
        return;
    }
    let targets = input_sync_targets(state);
    if !targets.contains(&source_session) {
        return;
    }
    for target_session in targets {
        if target_session == source_session {
            continue;
        }
        if write_runtime_input_bytes(state, target_session, bytes).is_ok() {
            note_terminal_input_activity(state, target_session);
        }
    }
}

fn write_runtime_input_bytes(
    state: &DaemonState,
    session_id: SessionId,
    bytes: &[u8],
) -> std::io::Result<()> {
    let input_writer = state
        .runtimes
        .get(&session_id)
        .and_then(|runtime| runtime.input_writer.as_ref().cloned())
        .ok_or_else(|| std::io::Error::other("runtime input writer missing"))?;
    let mut writer = input_writer
        .lock()
        .map_err(|_| std::io::Error::other("runtime input writer lock poisoned"))?;
    writer.write_all(bytes)
}

fn input_sync_targets(state: &DaemonState) -> BTreeSet<SessionId> {
    let session_ids = state
        .workspace
        .sessions()
        .iter()
        .map(|session| session.id)
        .collect::<BTreeSet<_>>();
    match state.input_sync_scope {
        InputSyncScope::RootVisible => {
            let supervisor_ids = state
                .groups
                .values()
                .filter_map(|group| group.supervisor_session_id)
                .collect::<BTreeSet<_>>();
            state
                .visible_workspace_items()
                .into_iter()
                .filter_map(|item| match item {
                    WorkspaceItem::Session(session_id)
                        if session_ids.contains(&session_id)
                            && !supervisor_ids.contains(&session_id) =>
                    {
                        Some(session_id)
                    }
                    _ => None,
                })
                .collect()
        }
        InputSyncScope::GroupMembers { group_id } => state
            .groups
            .get(&group_id)
            .map(|group| {
                group
                    .member_session_ids
                    .iter()
                    .copied()
                    .filter(|session_id| session_ids.contains(session_id))
                    .collect()
            })
            .unwrap_or_default(),
    }
}

fn note_terminal_input_activity(state: &mut DaemonState, session_id: SessionId) {
    let observation = state.observations.entry(session_id).or_default();
    record_terminal_input_activity(observation);
    state.snapshot_dirty = true;
}

fn observation_snapshot(observation: &SessionObservation) -> ObservationSnapshot {
    ObservationSnapshot {
        last_change_age_secs: observation.last_change.elapsed().as_secs(),
        recent_lines: observation.recent_lines.clone(),
        painted_line: observation.painted_line.clone(),
    }
}

fn append_replay_buffer(buffer: &mut Vec<u8>, chunk: &[u8]) {
    if chunk.is_empty() {
        return;
    }
    buffer.extend_from_slice(chunk);
    if buffer.len() > REPLAY_BYTES_LIMIT {
        let overflow = buffer.len() - REPLAY_BYTES_LIMIT;
        buffer.drain(0..overflow);
    }
}

fn spawn_assist_worker() -> Option<AssistWorker> {
    let registry = SynthesisBackendRegistry::from_env()?;
    let (request_tx, request_rx) = mpsc::channel::<AssistJob>();
    let (result_tx, result_rx) = mpsc::channel::<AssistResult>();
    thread::spawn(move || {
        while let Ok(job) = request_rx.recv() {
            let suggestion =
                registry.suggest_terminal_assist_blocking(&job.preferences, &job.evidence);
            let _ = result_tx.send(AssistResult {
                request_id: job.request_id,
                origin_session_id: job.origin_session_id,
                suggestion,
            });
        }
    });
    Some(AssistWorker {
        requests: request_tx,
        responses: result_rx,
    })
}

fn spawn_observation_worker() -> Option<ObservationWorker> {
    let (request_tx, request_rx) = mpsc::channel::<ObservationJob>();
    let (result_tx, result_rx) = mpsc::channel::<ObservationResult>();

    thread::spawn(move || {
        while let Ok(job) = request_rx.recv() {
            let refresh = compute_observation_refresh(&job.session, false);
            let _ = result_tx.send(ObservationResult {
                session_id: job.session_id,
                session: job.session,
                refresh,
            });
        }
    });

    Some(ObservationWorker {
        requests: request_tx,
        responses: result_rx,
    })
}

fn spawn_client_reader(stream: UnixStream, control_tx: ControlNotifier) {
    thread::spawn(move || {
        let mut reader = BufReader::new(stream);
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) => {
                    let _ = control_tx.send(ClientControl::ControlDisconnected);
                    break;
                }
                Ok(_) => {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    match serde_json::from_str::<ClientMessage>(trimmed) {
                        Ok(message) => {
                            if control_tx.send(ClientControl::Message(message)).is_err() {
                                break;
                            }
                        }
                        Err(_) => {
                            let _ = control_tx.send(ClientControl::ControlDisconnected);
                            break;
                        }
                    }
                }
                Err(_) => {
                    let _ = control_tx.send(ClientControl::ControlDisconnected);
                    break;
                }
            }
        }
    });
}

fn spawn_runtime_forwarder(
    session_id: SessionId,
    events: mpsc::Receiver<RuntimeEvent>,
    raw_writer: std::sync::Arc<std::sync::Mutex<Option<UnixStream>>>,
    control_tx: ControlNotifier,
) {
    thread::spawn(move || {
        while let Ok(event) = events.recv() {
            if let RuntimeEvent::Stream(update) = &event {
                if !update.output_bytes.is_empty() {
                    if let Ok(mut guard) = raw_writer.lock() {
                        if let Some(writer) = guard.as_mut() {
                            let _ = writer.write_all(&update.output_bytes);
                        }
                    }
                }
            }
            if control_tx
                .send(ClientControl::RuntimeEvent(session_id, event))
                .is_err()
            {
                break;
            }
        }
    });
}

fn spawn_raw_stream_reader(
    stream: UnixStream,
    input_writer: std::sync::Arc<std::sync::Mutex<File>>,
    control_tx: ControlNotifier,
    session_id: SessionId,
) {
    thread::spawn(move || {
        let mut reader = stream;
        let mut buf = [0u8; 8192];
        let mut response_filter = TerminalResponseInputFilter::default();
        loop {
            match reader.read(&mut buf) {
                Ok(0) => {
                    let _ = control_tx.send(ClientControl::StreamDisconnected(session_id));
                    break;
                }
                Ok(n) => {
                    let bytes = {
                        let Ok(mut writer) = input_writer.lock() else {
                            break;
                        };
                        let filter_responses = terminal_echoes_canonical_input(&writer);
                        let bytes = response_filter.filter(&buf[..n], filter_responses);
                        if bytes.is_empty() {
                            continue;
                        }
                        if writer.write_all(&bytes).is_err() {
                            break;
                        }
                        bytes
                    };
                    let _ = control_tx.send(ClientControl::TerminalInputBytes {
                        source_session: session_id,
                        bytes,
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => {
                    let _ = control_tx.send(ClientControl::StreamDisconnected(session_id));
                    break;
                }
            }
        }
    });
}

fn terminal_echoes_canonical_input(pty_master: &File) -> bool {
    let mut termios = unsafe { std::mem::zeroed::<libc::termios>() };
    if unsafe { libc::tcgetattr(pty_master.as_raw_fd(), &mut termios) } != 0 {
        return false;
    }
    let interactive_echo = libc::ECHO | libc::ICANON;
    termios.c_lflag & interactive_echo == interactive_echo
}

#[derive(Default)]
struct TerminalResponseInputFilter {
    state: TerminalResponseInputState,
}

#[derive(Default)]
enum TerminalResponseInputState {
    #[default]
    Ground,
    Escape,
    Csi(Vec<u8>),
    Osc,
    OscEscape,
    Dcs,
    DcsEscape,
}

impl TerminalResponseInputFilter {
    fn filter(&mut self, bytes: &[u8], filter_responses: bool) -> Vec<u8> {
        if !filter_responses && matches!(self.state, TerminalResponseInputState::Ground) {
            return bytes.to_vec();
        }

        let mut output = Vec::with_capacity(bytes.len());
        for &byte in bytes {
            if filter_responses || !matches!(self.state, TerminalResponseInputState::Ground) {
                self.filter_byte(byte, &mut output);
            } else {
                output.push(byte);
            }
        }
        output
    }

    fn filter_byte(&mut self, byte: u8, output: &mut Vec<u8>) {
        match &mut self.state {
            TerminalResponseInputState::Ground => {
                if byte == 0x1b {
                    self.state = TerminalResponseInputState::Escape;
                } else {
                    output.push(byte);
                }
            }
            TerminalResponseInputState::Escape => match byte {
                b']' => self.state = TerminalResponseInputState::Osc,
                b'P' => self.state = TerminalResponseInputState::Dcs,
                b'[' => self.state = TerminalResponseInputState::Csi(Vec::new()),
                _ => {
                    output.push(0x1b);
                    output.push(byte);
                    self.state = TerminalResponseInputState::Ground;
                }
            },
            TerminalResponseInputState::Csi(sequence) => {
                sequence.push(byte);
                if is_csi_final_byte(byte) {
                    if !is_terminal_response_csi(sequence) {
                        output.push(0x1b);
                        output.push(b'[');
                        output.extend_from_slice(sequence);
                    }
                    self.state = TerminalResponseInputState::Ground;
                } else if sequence.len() > 128 {
                    output.push(0x1b);
                    output.push(b'[');
                    output.extend_from_slice(sequence);
                    self.state = TerminalResponseInputState::Ground;
                }
            }
            TerminalResponseInputState::Osc => match byte {
                0x07 => self.state = TerminalResponseInputState::Ground,
                0x1b => self.state = TerminalResponseInputState::OscEscape,
                _ => {}
            },
            TerminalResponseInputState::OscEscape => {
                if byte == b'\\' {
                    self.state = TerminalResponseInputState::Ground;
                } else if byte != 0x1b {
                    self.state = TerminalResponseInputState::Osc;
                }
            }
            TerminalResponseInputState::Dcs => {
                if byte == 0x1b {
                    self.state = TerminalResponseInputState::DcsEscape;
                }
            }
            TerminalResponseInputState::DcsEscape => {
                if byte == b'\\' {
                    self.state = TerminalResponseInputState::Ground;
                } else if byte != 0x1b {
                    self.state = TerminalResponseInputState::Dcs;
                }
            }
        }
    }
}

fn is_csi_final_byte(byte: u8) -> bool {
    (0x40..=0x7e).contains(&byte)
}

fn is_terminal_response_csi(sequence: &[u8]) -> bool {
    let Some((&final_byte, body)) = sequence.split_last() else {
        return false;
    };
    match final_byte {
        b'R' => body
            .iter()
            .all(|byte| byte.is_ascii_digit() || *byte == b';'),
        b'c' => matches!(body.first(), Some(b'?' | b'>')),
        b'y' => body.contains(&b'$'),
        b'n' => body
            .iter()
            .all(|byte| byte.is_ascii_digit() || *byte == b';'),
        _ => false,
    }
}

fn write_json_line<W: Write, T: Serialize>(writer: &mut W, value: &T) -> std::io::Result<()> {
    serde_json::to_writer(&mut *writer, value)?;
    writer.write_all(b"\n")?;
    writer.flush()
}

fn connect_or_spawn_control_socket() -> Result<UnixStream, String> {
    if let Ok(control) = connect_control_socket() {
        return Ok(control);
    }

    let current_exe = std::env::current_exe()
        .map_err(|error| format!("failed to resolve current executable: {error}"))?;
    spawn_local_daemon_process(&current_exe)?;

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match connect_control_socket() {
            Ok(control) => return Ok(control),
            Err(error) if Instant::now() < deadline => {
                let _ = error;
                thread::sleep(Duration::from_millis(100));
            }
            Err(error) => return Err(error),
        }
    }
}

fn spawn_local_daemon_process(current_exe: &std::path::Path) -> Result<(), String> {
    if let Some(exatermd_path) = exatermd_sibling_path(current_exe) {
        if exatermd_path.exists() {
            let mut command = Command::new(exatermd_path);
            command
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            inherit_beachhead_env(&mut command);
            return command
                .spawn()
                .map(|_| ())
                .map_err(|error| format!("failed to spawn exatermd: {error}"));
        }
    }

    let mut command = Command::new(current_exe);
    command
        .arg("--beachhead-daemon")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    inherit_beachhead_env(&mut command);
    command
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("failed to start local host sessions: {error}"))
}

fn exatermd_sibling_path(current_exe: &std::path::Path) -> Option<PathBuf> {
    let file_name = current_exe.file_name()?.to_str()?;
    let sibling = if let Some(stripped) = file_name.strip_suffix(".exe") {
        format!("{stripped}d.exe")
    } else {
        "exatermd".to_string()
    };
    Some(current_exe.with_file_name(sibling))
}

fn inherit_beachhead_env(command: &mut Command) {
    for key in [
        "OPENAI_API_KEY",
        "EXATERM_OPENAI_BASE_URL",
        "OPENAI_BASE_URL",
        "EXATERM_TERMINAL_ASSIST_MODEL",
        "EXATERM_WORKSPACE",
    ] {
        if let Some(value) = env::var_os(key) {
            command.env(key, value);
        }
    }
}

fn connect_control_socket() -> Result<UnixStream, String> {
    UnixStream::connect(control_socket_path()?)
        .map_err(|error| format!("failed to connect control socket: {error}"))
}

fn clear_stale_control_socket(control_socket_path: &PathBuf) -> Result<(), String> {
    if !control_socket_path.exists() {
        return Ok(());
    }

    match UnixStream::connect(control_socket_path) {
        Ok(_) => Err("host session service already running".into()),
        Err(_) => fs::remove_file(control_socket_path)
            .map_err(|error| format!("failed to remove stale daemon control socket: {error}")),
    }
}

fn clear_stale_mcp_socket(mcp_socket_path: &PathBuf) -> Result<(), String> {
    if !mcp_socket_path.exists() {
        return Ok(());
    }
    fs::remove_file(mcp_socket_path)
        .map_err(|error| format!("failed to remove stale daemon MCP socket: {error}"))
}

pub fn connect_session_stream_socket(socket_name: &str) -> Result<UnixStream, String> {
    UnixStream::connect(session_raw_socket_path(socket_name)?)
        .map_err(|error| format!("failed to connect session raw socket: {error}"))
}

fn add_n_terminals(
    state: &mut DaemonState,
    source_session: SessionId,
    count: usize,
    control_tx: &ControlNotifier,
) -> Result<(), String> {
    let cwd = state
        .workspace
        .session(source_session)
        .and_then(|session| session.launch.cwd.clone());
    for _ in 0..count {
        let number = state.workspace.sessions().len() + 1;
        let mut launch = user_shell_launch(format!("Shell {number}"), "Generic command session");
        if let Some(cwd) = cwd.clone() {
            launch = launch.with_cwd(cwd);
        }
        let session_id = state.add_shell_session_without_watch(launch.clone())?;
        state.attach_repo_watch(session_id, &launch, control_tx)?;
        ensure_runtime_forwarder(state, session_id, control_tx.clone());
    }
    state.snapshot_dirty = true;
    Ok(())
}

fn additions_for_session_count(count: usize) -> usize {
    match count {
        1 => 1,
        2 | 4 | 6 => 2,
        8 => 1,
        9 => 3,
        12 => 4,
        _ => 0,
    }
}

fn supported_terminal_target(count: usize) -> bool {
    matches!(count, 1 | 2 | 4 | 6 | 8 | 9 | 12 | 16)
}

fn create_session_stream_state(session_id: SessionId) -> Result<SessionStreamState, String> {
    let socket_name = session_raw_socket_name(session_id);
    let socket_path = session_raw_socket_path(&socket_name)?;
    if socket_path.exists() {
        let _ = fs::remove_file(&socket_path);
    }
    let listener = UnixListener::bind(&socket_path).map_err(|error| {
        format!(
            "failed to bind session raw socket {:?}: {error}",
            session_id
        )
    })?;
    listener.set_nonblocking(true).map_err(|error| {
        format!(
            "failed to set session raw socket nonblocking {:?}: {error}",
            session_id
        )
    })?;
    Ok(SessionStreamState {
        socket_name,
        socket_path,
        listener,
        writer: std::sync::Arc::new(std::sync::Mutex::new(None)),
    })
}

fn session_raw_socket_name(session_id: SessionId) -> String {
    format!("session-{}-stream.sock", session_id.0)
}

pub fn control_socket_path() -> Result<PathBuf, String> {
    Ok(daemon_socket_dir().join(CONTROL_SOCKET_NAME))
}

pub fn mcp_socket_path() -> Result<PathBuf, String> {
    Ok(daemon_socket_dir().join(MCP_SOCKET_NAME))
}

pub fn session_raw_socket_path(socket_name: &str) -> Result<PathBuf, String> {
    Ok(daemon_socket_dir().join(socket_name))
}

fn daemon_socket_dir() -> PathBuf {
    let base = env::var_os("EXATERM_RUNTIME_DIR")
        .map(PathBuf::from)
        .or_else(|| env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from))
        .unwrap_or_else(|| {
            let uid = unsafe { libc::geteuid() };
            PathBuf::from(format!("/tmp/exaterm-{uid}"))
        });
    let dir = base.join("exaterm");
    match env::var_os("EXATERM_WORKSPACE") {
        Some(workspace) if !workspace.is_empty() => dir.join(PathBuf::from(workspace)),
        _ => dir,
    }
}

fn refresh_timeout_ms(elapsed: Duration) -> i32 {
    if elapsed >= REFRESH_INTERVAL {
        return 0;
    }
    let remaining = REFRESH_INTERVAL - elapsed;
    remaining
        .as_millis()
        .min(i32::MAX as u128)
        .try_into()
        .unwrap_or(i32::MAX)
}

fn drain_wake_socket(reader: &mut UnixStream) {
    let mut buf = [0u8; 256];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn add_terminals_follows_staged_density_growth() {
        assert_eq!(additions_for_session_count(1), 1);
        assert_eq!(additions_for_session_count(2), 2);
        assert_eq!(additions_for_session_count(4), 2);
        assert_eq!(additions_for_session_count(6), 2);
        assert_eq!(additions_for_session_count(8), 1);
        assert_eq!(additions_for_session_count(9), 3);
        assert_eq!(additions_for_session_count(12), 4);
    }

    #[test]
    fn add_terminals_stops_outside_supported_breakpoints() {
        assert_eq!(additions_for_session_count(3), 0);
        assert_eq!(additions_for_session_count(5), 0);
        assert_eq!(additions_for_session_count(10), 0);
        assert_eq!(additions_for_session_count(16), 0);
    }

    #[test]
    fn supported_terminal_targets_include_nine_tile_layout() {
        assert!(supported_terminal_target(9));
        assert!(supported_terminal_target(16));
        assert!(!supported_terminal_target(10));
        assert!(!supported_terminal_target(11));
    }

    #[test]
    fn terminal_display_env_only_advertises_confirmed_sixel() {
        let mut launch = user_shell_launch("Shell", "Generic command session");
        apply_terminal_display_env(
            &mut launch,
            &TerminalDisplayCapabilities {
                kitty_graphics: false,
                sixel: false,
                vte_version: Some("8400".into()),
            },
        );
        assert!(launch.env.iter().all(|(key, _)| key != "VTE_VERSION"));

        apply_terminal_display_env(
            &mut launch,
            &TerminalDisplayCapabilities {
                kitty_graphics: false,
                sixel: true,
                vte_version: Some("8400".into()),
            },
        );
        assert!(launch
            .env
            .iter()
            .any(|(key, value)| key == "VTE_VERSION" && value == "8400"));
        assert!(launch
            .env
            .iter()
            .any(|(key, value)| { key == "EXATERM_TERMINAL_IMAGE_PROTOCOLS" && value == "sixel" }));
    }

    #[test]
    fn terminal_display_env_advertises_kitty_bridge() {
        let mut launch = user_shell_launch("Shell", "Generic command session");
        apply_terminal_display_env(
            &mut launch,
            &TerminalDisplayCapabilities {
                kitty_graphics: true,
                sixel: false,
                vte_version: None,
            },
        );

        assert!(launch
            .env
            .iter()
            .any(|(key, value)| key == "KITTY_WINDOW_ID" && value == "1"));
        assert!(launch
            .env
            .iter()
            .any(|(key, value)| { key == "EXATERM_TERMINAL_IMAGE_PROTOCOLS" && value == "kitty" }));
    }

    #[test]
    fn terminal_response_filter_drops_osc_and_dcs_replies() {
        let mut filter = TerminalResponseInputFilter::default();
        let bytes = b"a\x1b]10;rgb:ffff/ffff/ffff\x07b\x1bP1+r6b75=1B4F41\x1b\\c";

        assert_eq!(filter.filter(bytes, true), b"abc");
    }

    #[test]
    fn terminal_response_filter_handles_split_sequences() {
        let mut filter = TerminalResponseInputFilter::default();

        assert_eq!(filter.filter(b"a\x1b]10;rgb", true), b"a");
        assert_eq!(filter.filter(b":ffff/ffff/ffff\x07b", true), b"b");
        assert_eq!(filter.filter(b"\x1bP1+r6b75=1B4F41", true), b"");
        assert_eq!(filter.filter(b"\x1b\\c", true), b"c");
    }

    #[test]
    fn terminal_response_filter_drops_response_csi_but_preserves_keys() {
        let mut filter = TerminalResponseInputFilter::default();

        assert_eq!(filter.filter(b"\x1b[?12;4$y", true), b"");
        assert_eq!(filter.filter(b"\x1b[24;80R", true), b"");
        assert_eq!(
            filter.filter(b"\x1b[A\x1bOB\x1b[6~", true),
            b"\x1b[A\x1bOB\x1b[6~"
        );
    }

    #[test]
    fn terminal_response_filter_forwards_everything_when_not_filtering() {
        let mut filter = TerminalResponseInputFilter::default();
        let bytes = b"\x1b]10;rgb:ffff/ffff/ffff\x07\x1bP1+r6b75=1B4F41\x1b\\";

        assert_eq!(filter.filter(bytes, false), bytes);
    }

    #[test]
    fn input_sync_root_targets_visible_non_supervisor_sessions() {
        let mut state = DaemonState::new();
        let worker = state
            .workspace
            .add_session(user_shell_launch("Worker", "root worker"));
        let supervisor = state
            .workspace
            .add_session(user_shell_launch("Supervisor", "hidden supervisor"));
        let grouped_worker = state
            .workspace
            .add_session(user_shell_launch("Grouped", "grouped worker"));
        let group_id = GroupId(1);
        state.workspace_items = vec![
            WorkspaceItem::Session(worker),
            WorkspaceItem::Session(supervisor),
            WorkspaceItem::Group(group_id),
        ];
        state.groups.insert(
            group_id,
            SupervisedGroupState {
                id: group_id,
                name: "Group".into(),
                member_session_ids: vec![grouped_worker],
                supervisor_session_id: Some(supervisor),
                provider: None,
                goal: None,
                summary_markdown: String::new(),
                supervisor_visible: true,
                summary_updated_at: None,
                actions: Vec::new(),
            },
        );
        state.input_sync_scope = InputSyncScope::RootVisible;

        let targets = input_sync_targets(&state);

        assert_eq!(targets, BTreeSet::from([worker]));
    }

    #[test]
    fn input_sync_group_targets_group_members_only() {
        let mut state = DaemonState::new();
        let root = state
            .workspace
            .add_session(user_shell_launch("Root", "root worker"));
        let member_a = state
            .workspace
            .add_session(user_shell_launch("A", "group member"));
        let member_b = state
            .workspace
            .add_session(user_shell_launch("B", "group member"));
        let supervisor = state
            .workspace
            .add_session(user_shell_launch("Supervisor", "group supervisor"));
        let group_id = GroupId(7);
        state.workspace_items = vec![WorkspaceItem::Session(root), WorkspaceItem::Group(group_id)];
        state.groups.insert(
            group_id,
            SupervisedGroupState {
                id: group_id,
                name: "Group".into(),
                member_session_ids: vec![member_a, member_b],
                supervisor_session_id: Some(supervisor),
                provider: None,
                goal: None,
                summary_markdown: String::new(),
                supervisor_visible: true,
                summary_updated_at: None,
                actions: Vec::new(),
            },
        );
        state.input_sync_scope = InputSyncScope::GroupMembers { group_id };

        let targets = input_sync_targets(&state);

        assert_eq!(targets, BTreeSet::from([member_a, member_b]));
    }

    #[test]
    fn terminal_assist_insert_targets_origin_when_sync_disabled() {
        let mut state = DaemonState::new();
        let session = state
            .workspace
            .add_session(user_shell_launch("Shell", "single shell"));
        state.input_sync_enabled = false;
        state.input_sync_scope = InputSyncScope::RootVisible;

        let targets = terminal_assist_insert_targets(&state, session);

        assert_eq!(targets, BTreeSet::from([session]));
    }

    #[test]
    fn terminal_assist_insert_targets_group_when_sync_enabled() {
        let mut state = DaemonState::new();
        let root = state
            .workspace
            .add_session(user_shell_launch("Root", "root worker"));
        let member_a = state
            .workspace
            .add_session(user_shell_launch("A", "group member"));
        let member_b = state
            .workspace
            .add_session(user_shell_launch("B", "group member"));
        let group_id = GroupId(11);
        state.workspace_items = vec![WorkspaceItem::Session(root), WorkspaceItem::Group(group_id)];
        state.groups.insert(
            group_id,
            SupervisedGroupState {
                id: group_id,
                name: "Group".into(),
                member_session_ids: vec![member_a, member_b],
                supervisor_session_id: None,
                provider: None,
                goal: None,
                summary_markdown: String::new(),
                supervisor_visible: false,
                summary_updated_at: None,
                actions: Vec::new(),
            },
        );
        state.input_sync_enabled = true;
        state.input_sync_scope = InputSyncScope::GroupMembers { group_id };

        let targets = terminal_assist_insert_targets(&state, member_a);

        assert_eq!(targets, BTreeSet::from([member_a, member_b]));
    }

    #[test]
    fn terminal_assist_insert_targets_origin_when_origin_outside_sync_scope() {
        let mut state = DaemonState::new();
        let root = state
            .workspace
            .add_session(user_shell_launch("Root", "root worker"));
        let member = state
            .workspace
            .add_session(user_shell_launch("A", "group member"));
        let group_id = GroupId(12);
        state.workspace_items = vec![WorkspaceItem::Session(root), WorkspaceItem::Group(group_id)];
        state.groups.insert(
            group_id,
            SupervisedGroupState {
                id: group_id,
                name: "Group".into(),
                member_session_ids: vec![member],
                supervisor_session_id: None,
                provider: None,
                goal: None,
                summary_markdown: String::new(),
                supervisor_visible: false,
                summary_updated_at: None,
                actions: Vec::new(),
            },
        );
        state.input_sync_enabled = true;
        state.input_sync_scope = InputSyncScope::GroupMembers { group_id };

        let targets = terminal_assist_insert_targets(&state, root);

        assert_eq!(targets, BTreeSet::from([root]));
    }

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    /// Returns false when Unix socket creation is blocked (e.g. inside the
    /// Claude Code sandbox).  Tests that start a real daemon process or rely
    /// on FSEvents delivery use this as an early-exit guard so `cargo test`
    /// passes in restricted environments without skipping anything on CI.
    fn can_bind_unix_sockets() -> bool {
        use std::os::unix::net::UnixListener;
        use std::sync::atomic::{AtomicU64, Ordering};
        // Combine PID (cross-process isolation) with a per-process atomic
        // counter (cross-thread isolation) so parallel test threads never race
        // on the same probe path.
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let probe = std::env::temp_dir().join(format!(
            ".exaterm-sock-probe-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed),
        ));
        let ok = UnixListener::bind(&probe).is_ok();
        let _ = fs::remove_file(&probe);
        ok
    }

    fn unique_runtime_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        let suffix = format!("exaterm-test-{label}-{nanos}");
        // Unix socket paths have a strict length limit (104 bytes on macOS).
        // The socket lives at <dir>/exaterm/beachhead-control.sock (+31 chars).
        // On macOS CI, TMPDIR expands to a long /var/folders/… path that pushes
        // us over; in that case fall back to /tmp which is always short.
        const SOCKET_SUFFIX_LEN: usize = "/exaterm/beachhead-control.sock".len();
        const LIMIT: usize = 104;
        let base = std::env::temp_dir();
        let candidate = base.join(&suffix);
        if candidate.as_os_str().len() + SOCKET_SUFFIX_LEN > LIMIT {
            PathBuf::from("/tmp").join(suffix)
        } else {
            candidate
        }
    }

    fn read_server_message(reader: &mut BufReader<UnixStream>) -> ServerMessage {
        let mut line = String::new();
        reader.read_line(&mut line).expect("read daemon message");
        serde_json::from_str(line.trim()).expect("parse daemon message")
    }

    #[test]
    fn replay_buffer_trims_to_limit() {
        let mut buffer = Vec::new();
        append_replay_buffer(&mut buffer, &vec![b'x'; REPLAY_BYTES_LIMIT + 128]);
        assert_eq!(buffer.len(), REPLAY_BYTES_LIMIT);
        assert!(buffer.iter().all(|byte| *byte == b'x'));
    }

    #[test]
    fn socket_paths_use_override_runtime_dir() {
        let _guard = env_lock().lock().expect("env lock");
        let runtime_dir = unique_runtime_dir("socket");
        std::env::set_var("EXATERM_RUNTIME_DIR", &runtime_dir);
        let control_path = control_socket_path().expect("control socket path");
        assert_eq!(
            control_path,
            runtime_dir.join("exaterm").join(CONTROL_SOCKET_NAME)
        );
        assert_eq!(
            session_raw_socket_path("session-7-stream.sock").expect("session raw socket path"),
            runtime_dir.join("exaterm").join("session-7-stream.sock")
        );
        std::env::remove_var("EXATERM_RUNTIME_DIR");
    }

    #[test]
    fn local_daemon_attach_create_and_terminate_workspace() {
        if !can_bind_unix_sockets() {
            return;
        }
        let _guard = env_lock().lock().expect("env lock");
        let runtime_dir = unique_runtime_dir("daemon-flow");
        std::env::set_var("EXATERM_RUNTIME_DIR", &runtime_dir);

        let handle = thread::spawn(run_local_daemon_inner);

        let deadline = Instant::now() + Duration::from_secs(5);
        let control_path = control_socket_path().expect("control socket path");
        let mut stream = loop {
            match UnixStream::connect(&control_path) {
                Ok(stream) => break stream,
                Err(error) if Instant::now() < deadline => {
                    let _ = error;
                    thread::sleep(Duration::from_millis(50));
                }
                Err(error) => panic!("failed to connect daemon: {error}"),
            }
        };
        let reader_stream = stream.try_clone().expect("clone stream");
        let mut reader = BufReader::new(reader_stream);

        write_json_line(&mut stream, &ClientMessage::AttachClient).expect("attach client");
        match read_server_message(&mut reader) {
            ServerMessage::WorkspaceSnapshot { snapshot } => {
                assert!(snapshot.sessions.is_empty());
            }
            other => panic!("unexpected first message: {other:?}"),
        }

        write_json_line(&mut stream, &ClientMessage::CreateOrResumeDefaultWorkspace)
            .expect("create workspace");
        let snapshot = match read_server_message(&mut reader) {
            ServerMessage::WorkspaceSnapshot { snapshot } => snapshot,
            other => panic!("unexpected second message: {other:?}"),
        };
        assert_eq!(snapshot.sessions.len(), 1);
        assert_eq!(snapshot.sessions[0].record.launch.name, "Shell 1");

        write_json_line(&mut stream, &ClientMessage::TerminateWorkspace).expect("terminate");
        drop(stream);
        let result = handle.join().expect("daemon thread should join");
        assert!(result.is_ok(), "daemon should exit cleanly: {result:?}");

        std::env::remove_var("EXATERM_RUNTIME_DIR");
        let _ = fs::remove_dir_all(runtime_dir);
    }

    #[test]
    fn daemon_rejects_second_attached_client() {
        if !can_bind_unix_sockets() {
            return;
        }
        let _guard = env_lock().lock().expect("env lock");
        let runtime_dir = unique_runtime_dir("daemon-reject");
        std::env::set_var("EXATERM_RUNTIME_DIR", &runtime_dir);

        let handle = thread::spawn(run_local_daemon_inner);

        let deadline = Instant::now() + Duration::from_secs(5);
        let control_path = control_socket_path().expect("control socket path");
        let mut first = loop {
            match UnixStream::connect(&control_path) {
                Ok(stream) => break stream,
                Err(error) if Instant::now() < deadline => {
                    let _ = error;
                    thread::sleep(Duration::from_millis(50));
                }
                Err(error) => panic!("failed to connect daemon: {error}"),
            }
        };
        write_json_line(&mut first, &ClientMessage::AttachClient).expect("attach first");

        let second = UnixStream::connect(&control_path).expect("connect second");
        let mut second_reader = BufReader::new(second);
        match read_server_message(&mut second_reader) {
            ServerMessage::Error { message } => {
                assert!(message.contains("already attached"));
            }
            other => panic!("unexpected second-client message: {other:?}"),
        }

        write_json_line(&mut first, &ClientMessage::TerminateWorkspace).expect("terminate");
        drop(first);
        let result = handle.join().expect("daemon thread should join");
        assert!(result.is_ok(), "daemon should exit cleanly: {result:?}");

        std::env::remove_var("EXATERM_RUNTIME_DIR");
        let _ = fs::remove_dir_all(runtime_dir);
    }

    #[test]
    fn repo_watch_events_update_observation_recent_files() {
        if !can_bind_unix_sockets() {
            return;
        }
        let root = unique_runtime_dir("repo-watch");
        let repo_root = root.join("repo");
        let nested = repo_root.join("src");
        fs::create_dir_all(repo_root.join(".git")).expect("git dir");
        fs::create_dir_all(&nested).expect("src dir");
        let tracked = nested.join("lib.rs");

        let (tx, rx) = mpsc::channel();
        let (wake_reader, wake_writer) = UnixStream::pair().expect("wake pair");
        let notifier = ControlNotifier {
            tx,
            wake: std::sync::Arc::new(std::sync::Mutex::new(wake_writer)),
        };

        let mut state = DaemonState::new();
        let launch = user_shell_launch("Shell", "watch test").with_cwd(nested.clone());
        let session_id = state.workspace.add_session(launch.clone());
        state
            .observations
            .insert(session_id, SessionObservation::new());
        state
            .observation_cache
            .insert(session_id, ObservationCacheEntry::new());

        state
            .attach_repo_watch(session_id, &launch, &notifier)
            .expect("attach repo watch");

        fs::write(&tracked, "pub fn watched() {}\n").expect("write watched file");
        let control = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("watcher should publish file activity");
        drop(wake_reader);

        match control {
            ClientControl::FileActivity {
                repo_root: event_root,
                relative_path,
            } => {
                assert_eq!(event_root, repo_root);
                assert_eq!(relative_path, "src/lib.rs");
                let now = Instant::now();
                let sessions = state
                    .repo_watches
                    .get(&event_root)
                    .expect("watch should still exist")
                    .sessions
                    .clone();
                for watched_session in sessions {
                    let observation = state
                        .observations
                        .get_mut(&watched_session)
                        .expect("observation exists");
                    apply_file_activity(observation, relative_path.clone(), now);
                }
                assert_eq!(
                    state
                        .observations
                        .get(&session_id)
                        .expect("observation exists")
                        .recent_files,
                    vec!["src/lib.rs".to_string()]
                );
            }
            _ => panic!("unexpected control message"),
        }

        state.shutdown_workspace();
        let _ = fs::remove_dir_all(root);
    }
}
