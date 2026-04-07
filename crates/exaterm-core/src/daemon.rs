use crate::file_watch::{RepoWatchHandle, spawn_repo_watch};
use crate::model::{SessionId, SessionLaunch, WorkspaceStore, user_shell_launch};
use crate::observation::{
    SessionObservation, apply_file_activity, apply_observation_refresh, apply_stream_update,
    build_naming_evidence, build_nudge_evidence, build_tactical_evidence, clear_file_activity,
    compute_observation_refresh, find_git_worktree_root, is_bare_waiting_shell,
    record_terminal_input_activity,
};
use crate::proto::{
    ClientMessage, ObservationSnapshot, ServerMessage, SessionSnapshot, WorkspaceSnapshot,
};
use crate::runtime::{RuntimeEvent, SessionRuntime, spawn_headless_runtime};
use crate::synthesis::{
    NameSuggestion, NamingEvidence, NudgeEvidence, NudgeSuggestion, ProviderCallResult,
    ProviderPreferences, SynthesisBackendRegistry, TacticalState, TacticalSynthesis,
    name_signature, nudge_signature, should_skip_repeated_paused_summary, summary_signature,
    summary_substantive_signature,
};
use portable_pty::PtySize;
use serde::Serialize;
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
const CANONICAL_TERMINAL_ROWS: u16 = 40;
const CANONICAL_TERMINAL_COLS: u16 = 120;
const REPLAY_BYTES_LIMIT: usize = 8 * 1024 * 1024;
const REFRESH_INTERVAL: Duration = Duration::from_millis(900);
const CONTROL_EVENTS_PER_TICK: usize = 128;
const TERMINAL_RETURN_DELAY: Duration = Duration::from_millis(35);
const MIN_IDLE_SECS_FOR_NUDGE: u64 = 20;
const PROVIDER_DEMOTION_COOLDOWN: Duration = Duration::from_secs(300);

struct SummaryWorker {
    requests: mpsc::Sender<SummaryJob>,
    responses: mpsc::Receiver<SummaryResult>,
}

struct SummaryJob {
    session_id: SessionId,
    signature: String,
    substantive_signature: String,
    evidence: crate::synthesis::TacticalEvidence,
    preferences: ProviderPreferences,
}

struct SummaryResult {
    session_id: SessionId,
    signature: String,
    substantive_signature: String,
    summary: ProviderCallResult<TacticalSynthesis>,
}

struct NamingWorker {
    requests: mpsc::Sender<NamingJob>,
    responses: mpsc::Receiver<NamingResult>,
}

struct NamingJob {
    session_id: SessionId,
    signature: String,
    evidence: NamingEvidence,
    preferences: ProviderPreferences,
}

struct NamingResult {
    session_id: SessionId,
    signature: String,
    suggestion: ProviderCallResult<NameSuggestion>,
}

struct NudgeWorker {
    requests: mpsc::Sender<NudgeJob>,
    responses: mpsc::Receiver<NudgeResult>,
}

struct NudgeJob {
    session_id: SessionId,
    signature: String,
    evidence: NudgeEvidence,
    preferences: ProviderPreferences,
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

struct NudgeResult {
    session_id: SessionId,
    signature: String,
    suggestion: ProviderCallResult<NudgeSuggestion>,
}

struct SummaryCacheEntry {
    first_seen: Instant,
    completed_signature: Option<String>,
    completed_substantive_signature: Option<String>,
    requested_signature: Option<String>,
    last_summary: Option<TacticalSynthesis>,
    last_attempt: Option<Instant>,
    in_flight: bool,
    skipped_providers: BTreeMap<crate::synthesis::SynthesisProvider, Instant>,
}

struct ObservationCacheEntry {
    last_attempt: Option<Instant>,
    in_flight: bool,
}

struct NamingCacheEntry {
    completed_signature: Option<String>,
    requested_signature: Option<String>,
    last_attempt: Option<Instant>,
    in_flight: bool,
    skipped_providers: BTreeMap<crate::synthesis::SynthesisProvider, Instant>,
}

struct NudgeCacheEntry {
    enabled: bool,
    completed_signature: Option<String>,
    requested_signature: Option<String>,
    last_nudge: Option<String>,
    last_attempt: Option<Instant>,
    last_sent: Option<Instant>,
    in_flight: bool,
    skipped_providers: BTreeMap<crate::synthesis::SynthesisProvider, Instant>,
}

impl SummaryCacheEntry {
    fn new() -> Self {
        Self {
            first_seen: Instant::now(),
            completed_signature: None,
            completed_substantive_signature: None,
            requested_signature: None,
            last_summary: None,
            last_attempt: None,
            in_flight: false,
            skipped_providers: BTreeMap::new(),
        }
    }
}

impl ObservationCacheEntry {
    fn new() -> Self {
        Self {
            last_attempt: None,
            in_flight: false,
        }
    }
}

impl NamingCacheEntry {
    fn new() -> Self {
        Self {
            completed_signature: None,
            requested_signature: None,
            last_attempt: None,
            in_flight: false,
            skipped_providers: BTreeMap::new(),
        }
    }
}

impl NudgeCacheEntry {
    fn new() -> Self {
        Self {
            enabled: false,
            completed_signature: None,
            requested_signature: None,
            last_nudge: None,
            last_attempt: None,
            last_sent: None,
            in_flight: false,
            skipped_providers: BTreeMap::new(),
        }
    }
}

struct DaemonState {
    workspace: WorkspaceStore,
    observations: BTreeMap<SessionId, SessionObservation>,
    observation_worker: Option<ObservationWorker>,
    observation_cache: BTreeMap<SessionId, ObservationCacheEntry>,
    runtimes: BTreeMap<SessionId, SessionRuntime>,
    replay_buffers: BTreeMap<SessionId, Vec<u8>>,
    session_streams: BTreeMap<SessionId, SessionStreamState>,
    repo_watches: BTreeMap<PathBuf, RepoWatchState>,
    session_repo_roots: BTreeMap<SessionId, PathBuf>,
    summary_worker: Option<SummaryWorker>,
    summary_cache: BTreeMap<SessionId, SummaryCacheEntry>,
    naming_worker: Option<NamingWorker>,
    naming_cache: BTreeMap<SessionId, NamingCacheEntry>,
    nudge_worker: Option<NudgeWorker>,
    nudge_cache: BTreeMap<SessionId, NudgeCacheEntry>,
    forwarded_sessions: BTreeSet<SessionId>,
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

impl DaemonState {
    fn new() -> Self {
        Self {
            workspace: WorkspaceStore::new(),
            observations: BTreeMap::new(),
            observation_worker: spawn_observation_worker(),
            observation_cache: BTreeMap::new(),
            runtimes: BTreeMap::new(),
            replay_buffers: BTreeMap::new(),
            session_streams: BTreeMap::new(),
            repo_watches: BTreeMap::new(),
            session_repo_roots: BTreeMap::new(),
            summary_worker: spawn_summary_worker(),
            summary_cache: BTreeMap::new(),
            naming_worker: spawn_naming_worker(),
            naming_cache: BTreeMap::new(),
            nudge_worker: spawn_nudge_worker(),
            nudge_cache: BTreeMap::new(),
            forwarded_sessions: BTreeSet::new(),
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
        let session_id = self.workspace.add_session(launch.clone());
        self.observations
            .insert(session_id, SessionObservation::new());
        self.observation_cache
            .insert(session_id, ObservationCacheEntry::new());
        self.nudge_cache.insert(session_id, NudgeCacheEntry::new());
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
                    let summary = self
                        .summary_cache
                        .get(&record_id)
                        .and_then(|entry| entry.last_summary.clone());
                    let nudge = self.nudge_cache.get(&record_id);
                    SessionSnapshot {
                        record,
                        observation,
                        summary,
                        raw_stream_socket_name: self
                            .session_streams
                            .get(&record_id)
                            .map(|stream| stream.socket_name.clone()),
                        auto_nudge_enabled: nudge.is_some_and(|entry| entry.enabled),
                        last_nudge: nudge.and_then(|entry| entry.last_nudge.clone()),
                        last_sent_age_secs: nudge
                            .and_then(|entry| entry.last_sent.map(|sent| sent.elapsed().as_secs())),
                    }
                })
                .collect(),
        }
    }

    fn shutdown_workspace(&mut self) {
        self.runtimes.clear();
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
        self.summary_cache.clear();
        self.naming_cache.clear();
        self.nudge_cache.clear();
        self.forwarded_sessions.clear();
        self.workspace.replace_sessions(Vec::new());
        self.snapshot_dirty = true;
    }
}

enum ClientControl {
    Message(ClientMessage),
    ControlDisconnected,
    StreamDisconnected(SessionId),
    TerminalInput(SessionId),
    FileActivity {
        repo_root: PathBuf,
        relative_path: String,
    },
    RuntimeEvent(SessionId, RuntimeEvent),
}

pub struct LocalBeachheadClient {
    pub commands: mpsc::Sender<ClientMessage>,
    pub events: mpsc::Receiver<ServerMessage>,
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
            .map_err(|error| format!("failed to clone beachhead socket: {error}"))?;
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
        let (event_tx, event_rx) = mpsc::channel::<ServerMessage>();

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
    let control_socket_path = control_socket_path()?;
    if let Some(parent) = control_socket_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create daemon socket dir: {error}"))?;
    }
    clear_stale_control_socket(&control_socket_path)?;

    let control_listener = UnixListener::bind(&control_socket_path)
        .map_err(|error| format!("failed to bind daemon control socket: {error}"))?;
    control_listener
        .set_nonblocking(true)
        .map_err(|error| format!("failed to set daemon control socket nonblocking: {error}"))?;

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
            wake_ready = pollfds[1].revents & libc::POLLIN != 0;
            for (index, session_id) in session_ids.into_iter().enumerate() {
                if pollfds[index + 2].revents & libc::POLLIN != 0 {
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
                ClientControl::TerminalInput(session_id) => {
                    note_terminal_input_activity(&mut state, session_id);
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
                        state.snapshot_dirty = true;
                    }
                }
                ClientControl::RuntimeEvent(session_id, event) => {
                    handle_runtime_event(&mut state, &mut client_writer, session_id, event);
                }
            }
        }

        let runtime_changed = false;
        let worker_changed = drain_worker_results(&mut state);
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
            let additions = additions_for_session_count(state.workspace.sessions().len());
            if additions == 0 {
                return Ok(false);
            }
            let cwd = state
                .workspace
                .session(source_session)
                .and_then(|session| session.launch.cwd.clone());
            for _ in 0..additions {
                let number = state.workspace.sessions().len() + 1;
                let mut launch =
                    user_shell_launch(format!("Shell {number}"), "Generic command session");
                if let Some(cwd) = cwd.clone() {
                    launch = launch.with_cwd(cwd);
                }
                let session_id = state.add_shell_session_without_watch(launch.clone())?;
                state.attach_repo_watch(session_id, &launch, control_tx)?;
                ensure_runtime_forwarder(state, session_id, control_tx.clone());
            }
            state.snapshot_dirty = true;
            Ok(false)
        }
        ClientMessage::AddTerminalsTo {
            source_session,
            target_total,
        } => {
            let current_total = state.workspace.sessions().len();
            if target_total <= current_total || !supported_terminal_target(target_total) {
                return Ok(false);
            }
            let additions = target_total - current_total;
            let cwd = state
                .workspace
                .session(source_session)
                .and_then(|session| session.launch.cwd.clone());
            for _ in 0..additions {
                let number = state.workspace.sessions().len() + 1;
                let mut launch =
                    user_shell_launch(format!("Shell {number}"), "Generic command session");
                if let Some(cwd) = cwd.clone() {
                    launch = launch.with_cwd(cwd);
                }
                let session_id = state.add_shell_session_without_watch(launch.clone())?;
                state.attach_repo_watch(session_id, &launch, control_tx)?;
                ensure_runtime_forwarder(state, session_id, control_tx.clone());
            }
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
        ClientMessage::ToggleAutoNudge {
            session_id,
            enabled,
        } => {
            let entry = state
                .nudge_cache
                .entry(session_id)
                .or_insert_with(NudgeCacheEntry::new);
            entry.enabled = enabled;
            if !enabled {
                entry.in_flight = false;
                entry.requested_signature = None;
            }
            state.snapshot_dirty = true;
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

fn refresh_state(state: &mut DaemonState) {
    let sessions = state.workspace.sessions().to_vec();
    for session in &sessions {
        maybe_queue_observation_refresh(state, session);
        let Some(observation) = state.observations.get(&session.id).cloned() else {
            continue;
        };

        if is_bare_waiting_shell(session, &observation) {
            continue;
        }

        let evidence = build_tactical_evidence(session, &observation);
        maybe_queue_summary(state, session.id, &evidence);

        let naming = build_naming_evidence(session, &observation);
        maybe_queue_name(state, session.id, &naming);

        let summary = state
            .summary_cache
            .get(&session.id)
            .and_then(|entry| entry.last_summary.as_ref())
            .cloned();
        if let Some(summary) = summary.as_ref() {
            maybe_queue_nudge(state, session, &observation, summary);
        }
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

fn maybe_queue_summary(
    state: &mut DaemonState,
    session_id: SessionId,
    evidence: &crate::synthesis::TacticalEvidence,
) {
    let Some(worker) = state.summary_worker.as_ref() else {
        return;
    };
    let signature = summary_signature(evidence);
    let substantive_signature = summary_substantive_signature(evidence);
    let entry = state
        .summary_cache
        .entry(session_id)
        .or_insert_with(SummaryCacheEntry::new);
    if entry.completed_signature.as_deref() == Some(signature.as_str())
        || entry.requested_signature.as_deref() == Some(signature.as_str())
        || entry.in_flight
    {
        return;
    }
    if should_skip_repeated_paused_summary(
        entry.last_summary.as_ref(),
        entry.completed_substantive_signature.as_deref(),
        &substantive_signature,
    ) {
        return;
    }
    let refresh_interval = summary_refresh_interval(entry.first_seen.elapsed());
    if entry
        .last_attempt
        .is_some_and(|attempt| attempt.elapsed() < refresh_interval)
    {
        return;
    }
    entry.in_flight = true;
    entry.last_attempt = Some(Instant::now());
    entry.requested_signature = Some(signature.clone());
    let _ = worker.requests.send(SummaryJob {
        session_id,
        signature,
        substantive_signature,
        evidence: evidence.clone(),
        preferences: active_provider_preferences(&entry.skipped_providers),
    });
}

fn maybe_queue_name(state: &mut DaemonState, session_id: SessionId, evidence: &NamingEvidence) {
    let Some(worker) = state.naming_worker.as_ref() else {
        return;
    };
    let signature = name_signature(evidence);
    let entry = state
        .naming_cache
        .entry(session_id)
        .or_insert_with(NamingCacheEntry::new);
    if entry.completed_signature.as_deref() == Some(signature.as_str())
        || entry.requested_signature.as_deref() == Some(signature.as_str())
        || entry.in_flight
    {
        return;
    }
    if entry
        .last_attempt
        .is_some_and(|attempt| attempt.elapsed() < Duration::from_secs(20))
    {
        return;
    }
    entry.in_flight = true;
    entry.last_attempt = Some(Instant::now());
    entry.requested_signature = Some(signature.clone());
    let _ = worker.requests.send(NamingJob {
        session_id,
        signature,
        evidence: evidence.clone(),
        preferences: active_provider_preferences(&entry.skipped_providers),
    });
}

fn maybe_queue_nudge(
    state: &mut DaemonState,
    session: &crate::model::SessionRecord,
    observation: &SessionObservation,
    summary: &TacticalSynthesis,
) {
    let Some(worker) = state.nudge_worker.as_ref() else {
        return;
    };
    if summary.tactical_state != TacticalState::Stopped {
        return;
    }
    let Some(shell_child_command) = observation.shell_child_command.as_deref() else {
        return;
    };
    if !looks_like_coding_agent(shell_child_command) {
        return;
    }
    if observation.last_change.elapsed().as_secs() < MIN_IDLE_SECS_FOR_NUDGE {
        return;
    }
    let entry = state
        .nudge_cache
        .entry(session.id)
        .or_insert_with(NudgeCacheEntry::new);
    if !entry.enabled || entry.in_flight {
        return;
    }
    if entry
        .last_sent
        .is_some_and(|sent| sent.elapsed() < Duration::from_secs(120))
    {
        return;
    }
    if entry
        .last_attempt
        .is_some_and(|attempt| attempt.elapsed() < Duration::from_secs(10))
    {
        return;
    }

    let evidence = build_nudge_evidence(session, observation, summary);
    let signature = nudge_signature(&evidence);
    if entry.requested_signature.as_deref() == Some(signature.as_str()) {
        return;
    }
    entry.in_flight = true;
    entry.last_attempt = Some(Instant::now());
    entry.requested_signature = Some(signature.clone());
    let _ = worker.requests.send(NudgeJob {
        session_id: session.id,
        signature,
        evidence,
        preferences: active_provider_preferences(&entry.skipped_providers),
    });
}

fn drain_worker_results(state: &mut DaemonState) -> bool {
    let mut changed = false;

    if let Some(worker) = state.observation_worker.as_ref() {
        while let Ok(result) = worker.responses.try_recv() {
            let entry = state
                .observation_cache
                .entry(result.session_id)
                .or_insert_with(ObservationCacheEntry::new);
            entry.in_flight = false;
            let observation = state.observations.entry(result.session_id).or_default();
            let before = (
                observation.shell_child_command.clone(),
                observation.dominant_process.clone(),
                observation.process_tree_excerpt.clone(),
                observation.recent_files.clone(),
                observation.work_output_excerpt.clone(),
                observation.active_command.clone(),
            );
            apply_observation_refresh(observation, &result.session, result.refresh);
            let after = (
                observation.shell_child_command.clone(),
                observation.dominant_process.clone(),
                observation.process_tree_excerpt.clone(),
                observation.recent_files.clone(),
                observation.work_output_excerpt.clone(),
                observation.active_command.clone(),
            );
            if before != after {
                changed = true;
            }
        }
    }

    if let Some(worker) = state.summary_worker.as_ref() {
        while let Ok(result) = worker.responses.try_recv() {
            let entry = state
                .summary_cache
                .entry(result.session_id)
                .or_insert_with(SummaryCacheEntry::new);
            entry.in_flight = false;
            entry.requested_signature = None;
            if let Some(provider) = result.summary.demoted_provider {
                record_provider_demotion(&mut entry.skipped_providers, provider);
            }
            if let Ok(summary) = result.summary.value {
                entry.completed_signature = Some(result.signature);
                entry.completed_substantive_signature = Some(result.substantive_signature);
                entry.last_summary = Some(summary);
                changed = true;
            }
        }
    }

    if let Some(worker) = state.naming_worker.as_ref() {
        while let Ok(result) = worker.responses.try_recv() {
            let entry = state
                .naming_cache
                .entry(result.session_id)
                .or_insert_with(NamingCacheEntry::new);
            entry.in_flight = false;
            entry.requested_signature = None;
            if let Some(provider) = result.suggestion.demoted_provider {
                record_provider_demotion(&mut entry.skipped_providers, provider);
            }
            if let Ok(suggestion) = result.suggestion.value {
                entry.completed_signature = Some(result.signature);
                if !suggestion.name.is_empty() {
                    state
                        .workspace
                        .set_display_name(result.session_id, Some(suggestion.name));
                    changed = true;
                }
            }
        }
    }

    loop {
        let result = {
            let Some(worker) = state.nudge_worker.as_ref() else {
                break;
            };
            worker.responses.try_recv().ok()
        };
        let Some(result) = result else {
            break;
        };

        let mut suggestion_text = None::<String>;
        {
            let entry = state
                .nudge_cache
                .entry(result.session_id)
                .or_insert_with(NudgeCacheEntry::new);
            entry.in_flight = false;
            entry.requested_signature = None;
            if let Some(provider) = result.suggestion.demoted_provider {
                record_provider_demotion(&mut entry.skipped_providers, provider);
            }
            if let Ok(suggestion) = result.suggestion.value {
                entry.completed_signature = Some(result.signature);
                entry.last_nudge = (!suggestion.text.is_empty()).then_some(suggestion.text.clone());
                suggestion_text = (!suggestion.text.is_empty()).then_some(suggestion.text);
                changed = true;
            }
        }
        if let Some(text) = suggestion_text {
            if session_still_accepts_nudge(state, result.session_id)
                && send_runtime_input_line(state, result.session_id, &text).is_ok()
            {
                if let Some(entry) = state.nudge_cache.get_mut(&result.session_id) {
                    entry.last_sent = Some(Instant::now());
                }
            }
        }
    }

    changed
}

fn active_provider_preferences(
    skipped_providers: &BTreeMap<crate::synthesis::SynthesisProvider, Instant>,
) -> ProviderPreferences {
    let skipped_providers = skipped_providers
        .iter()
        .filter_map(|(provider, demoted_at)| {
            (demoted_at.elapsed() < PROVIDER_DEMOTION_COOLDOWN).then_some(*provider)
        })
        .collect();
    ProviderPreferences { skipped_providers }
}

fn record_provider_demotion(
    skipped_providers: &mut BTreeMap<crate::synthesis::SynthesisProvider, Instant>,
    provider: crate::synthesis::SynthesisProvider,
) {
    skipped_providers.insert(provider, Instant::now());
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

fn session_still_accepts_nudge(state: &DaemonState, session_id: SessionId) -> bool {
    state
        .observations
        .get(&session_id)
        .is_some_and(|observation| {
            observation.last_change.elapsed().as_secs() >= MIN_IDLE_SECS_FOR_NUDGE
        })
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

fn note_terminal_input_activity(state: &mut DaemonState, session_id: SessionId) {
    let observation = state.observations.entry(session_id).or_default();
    record_terminal_input_activity(observation);
    state.snapshot_dirty = true;
}

fn looks_like_coding_agent(command: &str) -> bool {
    matches!(
        command,
        "codex" | "claude" | "claude-code" | "aider" | "opencode" | "goose" | "gemini"
    )
}

fn observation_snapshot(observation: &SessionObservation) -> ObservationSnapshot {
    ObservationSnapshot {
        last_change_age_secs: observation.last_change.elapsed().as_secs(),
        recent_lines: observation.recent_lines.clone(),
        painted_line: observation.painted_line.clone(),
        shell_child_command: observation.shell_child_command.clone(),
        active_command: observation.active_command.clone(),
        dominant_process: observation.dominant_process.clone(),
        process_tree_excerpt: observation.process_tree_excerpt.clone(),
        recent_files: observation.recent_files.clone(),
        work_output_excerpt: observation.work_output_excerpt.clone(),
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

fn summary_refresh_interval(session_age: Duration) -> Duration {
    let seconds = session_age.as_secs();
    if seconds < 60 {
        Duration::from_secs(5)
    } else if seconds < 180 {
        Duration::from_secs(10)
    } else if seconds < 300 {
        Duration::from_secs(20)
    } else {
        Duration::from_secs(30)
    }
}

fn spawn_summary_worker() -> Option<SummaryWorker> {
    let registry = SynthesisBackendRegistry::from_env()?;
    let (request_tx, request_rx) = mpsc::channel::<SummaryJob>();
    let (result_tx, result_rx) = mpsc::channel::<SummaryResult>();
    thread::spawn(move || {
        while let Ok(job) = request_rx.recv() {
            let summary = registry.summarize_blocking(&job.preferences, &job.evidence);
            let _ = result_tx.send(SummaryResult {
                session_id: job.session_id,
                signature: job.signature,
                substantive_signature: job.substantive_signature,
                summary,
            });
        }
    });
    Some(SummaryWorker {
        requests: request_tx,
        responses: result_rx,
    })
}

fn spawn_naming_worker() -> Option<NamingWorker> {
    let registry = SynthesisBackendRegistry::from_env()?;
    let (request_tx, request_rx) = mpsc::channel::<NamingJob>();
    let (result_tx, result_rx) = mpsc::channel::<NamingResult>();
    thread::spawn(move || {
        while let Ok(job) = request_rx.recv() {
            let suggestion = registry.suggest_name_blocking(&job.preferences, &job.evidence);
            let _ = result_tx.send(NamingResult {
                session_id: job.session_id,
                signature: job.signature,
                suggestion,
            });
        }
    });
    Some(NamingWorker {
        requests: request_tx,
        responses: result_rx,
    })
}

fn spawn_nudge_worker() -> Option<NudgeWorker> {
    let registry = SynthesisBackendRegistry::from_env()?;
    let (request_tx, request_rx) = mpsc::channel::<NudgeJob>();
    let (result_tx, result_rx) = mpsc::channel::<NudgeResult>();
    thread::spawn(move || {
        while let Ok(job) = request_rx.recv() {
            let suggestion = registry.suggest_nudge_blocking(&job.preferences, &job.evidence);
            let _ = result_tx.send(NudgeResult {
                session_id: job.session_id,
                signature: job.signature,
                suggestion,
            });
        }
    });
    Some(NudgeWorker {
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
        loop {
            match reader.read(&mut buf) {
                Ok(0) => {
                    let _ = control_tx.send(ClientControl::StreamDisconnected(session_id));
                    break;
                }
                Ok(n) => {
                    let Ok(mut writer) = input_writer.lock() else {
                        break;
                    };
                    if writer.write_all(&buf[..n]).is_err() {
                        break;
                    }
                    let _ = control_tx.send(ClientControl::TerminalInput(session_id));
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
        .map_err(|error| format!("failed to spawn local beachhead daemon: {error}"))
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
        "EXATERM_SUMMARY_MODEL",
        "EXATERM_NAMING_MODEL",
        "EXATERM_NUDGE_MODEL",
        "EXATERM_CODEX_CLI_MODEL",
        "EXATERM_CLAUDE_CLI_MODEL",
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
        Ok(_) => Err("beachhead daemon already running".into()),
        Err(_) => fs::remove_file(control_socket_path)
            .map_err(|error| format!("failed to remove stale daemon control socket: {error}")),
    }
}

pub fn connect_session_stream_socket(socket_name: &str) -> Result<UnixStream, String> {
    UnixStream::connect(session_raw_socket_path(socket_name)?)
        .map_err(|error| format!("failed to connect session raw socket: {error}"))
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
    let runtime_dir = daemon_runtime_dir();
    Ok(runtime_dir.join("exaterm").join(CONTROL_SOCKET_NAME))
}

pub fn session_raw_socket_path(socket_name: &str) -> Result<PathBuf, String> {
    let runtime_dir = daemon_runtime_dir();
    Ok(runtime_dir.join("exaterm").join(socket_name))
}

fn daemon_runtime_dir() -> PathBuf {
    env::var_os("EXATERM_RUNTIME_DIR")
        .map(PathBuf::from)
        .or_else(|| env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from))
        .unwrap_or_else(|| {
            let uid = unsafe { libc::geteuid() };
            PathBuf::from(format!("/tmp/exaterm-{uid}"))
        })
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
    fn nudge_only_applies_if_session_is_still_idle() {
        let mut state = DaemonState::new();
        let session_id = SessionId(7);

        let mut stale = SessionObservation::new();
        stale.last_change = Instant::now() - Duration::from_secs(MIN_IDLE_SECS_FOR_NUDGE + 5);
        state.observations.insert(session_id, stale);
        assert!(session_still_accepts_nudge(&state, session_id));

        let mut fresh = SessionObservation::new();
        fresh.last_change = Instant::now() - Duration::from_secs(MIN_IDLE_SECS_FOR_NUDGE - 1);
        state.observations.insert(session_id, fresh);
        assert!(!session_still_accepts_nudge(&state, session_id));
    }

    #[test]
    fn terminal_input_activity_blocks_a_pending_nudge() {
        let mut state = DaemonState::new();
        let session_id = SessionId(7);

        let mut stale = SessionObservation::new();
        stale.last_change = Instant::now() - Duration::from_secs(MIN_IDLE_SECS_FOR_NUDGE + 5);
        state.observations.insert(session_id, stale);
        assert!(session_still_accepts_nudge(&state, session_id));

        note_terminal_input_activity(&mut state, session_id);

        assert!(!session_still_accepts_nudge(&state, session_id));
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

    #[test]
    fn drain_worker_results_persists_provider_demotions_per_session() {
        let mut state = DaemonState::new();

        let (summary_request_tx, _summary_request_rx) = mpsc::channel();
        let (summary_result_tx, summary_result_rx) = mpsc::channel();
        state.summary_worker = Some(SummaryWorker {
            requests: summary_request_tx,
            responses: summary_result_rx,
        });
        summary_result_tx
            .send(SummaryResult {
                session_id: SessionId(1),
                signature: "summary".into(),
                substantive_signature: "summary-substantive".into(),
                summary: ProviderCallResult {
                    provider: Some(crate::synthesis::SynthesisProvider::ClaudeCli),
                    value: Err("openai failed".into()),
                    demoted_provider: Some(crate::synthesis::SynthesisProvider::OpenAi),
                },
            })
            .expect("send summary result");

        let (naming_request_tx, _naming_request_rx) = mpsc::channel();
        let (naming_result_tx, naming_result_rx) = mpsc::channel();
        state.naming_worker = Some(NamingWorker {
            requests: naming_request_tx,
            responses: naming_result_rx,
        });
        naming_result_tx
            .send(NamingResult {
                session_id: SessionId(2),
                signature: "naming".into(),
                suggestion: ProviderCallResult {
                    provider: Some(crate::synthesis::SynthesisProvider::ClaudeCli),
                    value: Err("claude failed".into()),
                    demoted_provider: Some(crate::synthesis::SynthesisProvider::ClaudeCli),
                },
            })
            .expect("send naming result");

        let (nudge_request_tx, _nudge_request_rx) = mpsc::channel();
        let (nudge_result_tx, nudge_result_rx) = mpsc::channel();
        state.nudge_worker = Some(NudgeWorker {
            requests: nudge_request_tx,
            responses: nudge_result_rx,
        });
        nudge_result_tx
            .send(NudgeResult {
                session_id: SessionId(3),
                signature: "nudge".into(),
                suggestion: ProviderCallResult {
                    provider: Some(crate::synthesis::SynthesisProvider::ClaudeCli),
                    value: Err("codex failed".into()),
                    demoted_provider: Some(crate::synthesis::SynthesisProvider::CodexCli),
                },
            })
            .expect("send nudge result");

        let changed = drain_worker_results(&mut state);
        assert!(!changed);
        assert!(
            state
                .summary_cache
                .get(&SessionId(1))
                .unwrap()
                .skipped_providers
                .contains_key(&crate::synthesis::SynthesisProvider::OpenAi)
        );
        assert!(
            state
                .naming_cache
                .get(&SessionId(2))
                .unwrap()
                .skipped_providers
                .contains_key(&crate::synthesis::SynthesisProvider::ClaudeCli)
        );
        assert!(
            state
                .nudge_cache
                .get(&SessionId(3))
                .unwrap()
                .skipped_providers
                .contains_key(&crate::synthesis::SynthesisProvider::CodexCli)
        );
    }

    #[test]
    fn maybe_queue_summary_retries_expired_provider_demotions() {
        let mut state = DaemonState::new();

        let (request_tx, request_rx) = mpsc::channel();
        let (response_tx, response_rx) = mpsc::channel();
        state.summary_worker = Some(SummaryWorker {
            requests: request_tx,
            responses: response_rx,
        });

        let session_id = SessionId(7);
        let mut entry = SummaryCacheEntry::new();
        entry.skipped_providers.insert(
            crate::synthesis::SynthesisProvider::OpenAi,
            Instant::now() - PROVIDER_DEMOTION_COOLDOWN - Duration::from_secs(1),
        );
        state.summary_cache.insert(session_id, entry);

        let evidence = crate::synthesis::TacticalEvidence {
            session_name: "Shell".into(),
            task_label: "Retry".into(),
            dominant_process: None,
            process_tree_excerpt: None,
            recent_files: vec![],
            terminal_status_line: None,
            terminal_status_line_age: None,
            recent_terminal_activity: vec![],
            recent_events: vec![],
        };

        maybe_queue_summary(&mut state, session_id, &evidence);

        let job = request_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("summary job should be queued after cooldown");
        assert!(job.preferences.skipped_providers.is_empty());

        drop(response_tx);
    }
}
