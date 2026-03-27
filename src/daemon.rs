use crate::model::{SessionId, SessionLaunch, WorkspaceState};
use crate::observation::{
    apply_stream_update, build_naming_evidence, build_nudge_evidence, build_tactical_evidence,
    refresh_observation as refresh_session_observation, SessionObservation,
};
use crate::proto::{
    ClientMessage, ObservationSnapshot, ServerMessage, SessionSnapshot, WorkspaceSnapshot,
};
use crate::runtime::{spawn_headless_runtime, RuntimeEvent, SessionRuntime};
use crate::synthesis::{
    name_signature, nudge_signature, suggest_name_blocking, suggest_nudge_blocking,
    summary_signature, summarize_blocking, NameSuggestion, NamingEvidence, NudgeEvidence,
    NudgeSuggestion, OpenAiNamingConfig, OpenAiNudgeConfig, OpenAiSynthesisConfig,
    TacticalState, TacticalSynthesis,
};
use glib::ExitCode;
use portable_pty::PtySize;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::env;
use std::fs::File;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

const CONTROL_SOCKET_NAME: &str = "beachhead-control.sock";
const RAW_SOCKET_NAME: &str = "beachhead-stream.sock";
const CANONICAL_TERMINAL_ROWS: u16 = 40;
const CANONICAL_TERMINAL_COLS: u16 = 120;
const REPLAY_BYTES_LIMIT: usize = 8 * 1024 * 1024;
const REFRESH_INTERVAL: Duration = Duration::from_millis(900);

struct SummaryWorker {
    requests: mpsc::Sender<SummaryJob>,
    responses: mpsc::Receiver<SummaryResult>,
}

struct SummaryJob {
    session_id: SessionId,
    signature: String,
    evidence: crate::synthesis::TacticalEvidence,
}

struct SummaryResult {
    session_id: SessionId,
    signature: String,
    summary: Result<TacticalSynthesis, String>,
}

struct NamingWorker {
    requests: mpsc::Sender<NamingJob>,
    responses: mpsc::Receiver<NamingResult>,
}

struct NamingJob {
    session_id: SessionId,
    signature: String,
    evidence: NamingEvidence,
}

struct NamingResult {
    session_id: SessionId,
    signature: String,
    suggestion: Result<NameSuggestion, String>,
}

struct NudgeWorker {
    requests: mpsc::Sender<NudgeJob>,
    responses: mpsc::Receiver<NudgeResult>,
}

struct NudgeJob {
    session_id: SessionId,
    signature: String,
    evidence: NudgeEvidence,
}

struct NudgeResult {
    session_id: SessionId,
    signature: String,
    suggestion: Result<NudgeSuggestion, String>,
}

struct SummaryCacheEntry {
    first_seen: Instant,
    completed_signature: Option<String>,
    requested_signature: Option<String>,
    last_summary: Option<TacticalSynthesis>,
    last_attempt: Option<Instant>,
    in_flight: bool,
}

struct NamingCacheEntry {
    completed_signature: Option<String>,
    requested_signature: Option<String>,
    last_attempt: Option<Instant>,
    in_flight: bool,
}

struct NudgeCacheEntry {
    enabled: bool,
    completed_signature: Option<String>,
    requested_signature: Option<String>,
    last_nudge: Option<String>,
    last_attempt: Option<Instant>,
    last_sent: Option<Instant>,
    in_flight: bool,
}

impl SummaryCacheEntry {
    fn new() -> Self {
        Self {
            first_seen: Instant::now(),
            completed_signature: None,
            requested_signature: None,
            last_summary: None,
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
        }
    }
}

struct DaemonState {
    workspace: WorkspaceState,
    observations: BTreeMap<SessionId, SessionObservation>,
    runtimes: BTreeMap<SessionId, SessionRuntime>,
    replay_buffers: BTreeMap<SessionId, Vec<u8>>,
    summary_worker: Option<SummaryWorker>,
    summary_cache: BTreeMap<SessionId, SummaryCacheEntry>,
    naming_worker: Option<NamingWorker>,
    naming_cache: BTreeMap<SessionId, NamingCacheEntry>,
    nudge_worker: Option<NudgeWorker>,
    nudge_cache: BTreeMap<SessionId, NudgeCacheEntry>,
    forwarded_sessions: BTreeSet<SessionId>,
    pending_input_traces: VecDeque<PendingInputTrace>,
    snapshot_dirty: bool,
}

struct PendingInputTrace {
    trace_id: u64,
    sent_at_us: u64,
    len: usize,
}

impl DaemonState {
    fn new() -> Self {
        Self {
            workspace: WorkspaceState::new(),
            observations: BTreeMap::new(),
            runtimes: BTreeMap::new(),
            replay_buffers: BTreeMap::new(),
            summary_worker: spawn_summary_worker(),
            summary_cache: BTreeMap::new(),
            naming_worker: spawn_naming_worker(),
            naming_cache: BTreeMap::new(),
            nudge_worker: spawn_nudge_worker(),
            nudge_cache: BTreeMap::new(),
            forwarded_sessions: BTreeSet::new(),
            pending_input_traces: VecDeque::new(),
            snapshot_dirty: false,
        }
    }

    fn ensure_default_workspace(&mut self) -> Result<(), String> {
        if !self.workspace.sessions().is_empty() {
            return Ok(());
        }

        let launch = SessionLaunch::user_shell("Shell 1", "Generic command session");
        let session_id = self.workspace.add_session(launch.clone());
        self.observations.insert(session_id, SessionObservation::new());
        self.nudge_cache.insert(session_id, NudgeCacheEntry::new());
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
        self.replay_buffers.insert(session_id, Vec::new());
        self.snapshot_dirty = true;
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
                    let observation = self
                        .observations
                        .get(&record.id)
                        .map(observation_snapshot)
                        .unwrap_or_default();
                    let summary = self
                        .summary_cache
                        .get(&record.id)
                        .and_then(|entry| entry.last_summary.clone());
                    let nudge = self.nudge_cache.get(&record.id);
                    SessionSnapshot {
                        record,
                        observation,
                        summary,
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
        self.replay_buffers.clear();
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
    StreamDisconnected,
    RuntimeEvent(SessionId, RuntimeEvent),
}

pub struct LocalBeachheadClient {
    pub commands: mpsc::Sender<ClientMessage>,
    pub events: mpsc::Receiver<ServerMessage>,
    pub raw_writer: std::sync::Arc<std::sync::Mutex<UnixStream>>,
    pub raw_reader: std::sync::Arc<std::sync::Mutex<Option<UnixStream>>>,
}

impl LocalBeachheadClient {
    pub fn connect_or_spawn() -> Result<Self, String> {
        let (control, raw_stream) = connect_or_spawn_sockets()?;
        let control_reader = control
            .try_clone()
            .map_err(|error| format!("failed to clone beachhead socket: {error}"))?;
        let control_writer = control;
        let stream_reader = raw_stream
            .try_clone()
            .map_err(|error| format!("failed to clone beachhead raw socket: {error}"))?;
        let stream_writer = raw_stream
            .try_clone()
            .map_err(|error| format!("failed to clone beachhead raw writer: {error}"))?;

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
            raw_writer: std::sync::Arc::new(std::sync::Mutex::new(stream_writer)),
            raw_reader: std::sync::Arc::new(std::sync::Mutex::new(Some(stream_reader))),
        })
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
    let raw_socket_path = raw_socket_path()?;
    if let Some(parent) = control_socket_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create daemon socket dir: {error}"))?;
    }
    if control_socket_path.exists() {
        let _ = fs::remove_file(&control_socket_path);
    }
    if raw_socket_path.exists() {
        let _ = fs::remove_file(&raw_socket_path);
    }

    let control_listener = UnixListener::bind(&control_socket_path)
        .map_err(|error| format!("failed to bind daemon control socket: {error}"))?;
    control_listener
        .set_nonblocking(true)
        .map_err(|error| format!("failed to set daemon control socket nonblocking: {error}"))?;
    let raw_listener = UnixListener::bind(&raw_socket_path)
        .map_err(|error| format!("failed to bind daemon raw socket: {error}"))?;
    raw_listener
        .set_nonblocking(true)
        .map_err(|error| format!("failed to set daemon raw socket nonblocking: {error}"))?;

    let (control_tx, control_rx) = mpsc::channel::<ClientControl>();
    let mut client_writer: Option<UnixStream> = None;
    let raw_writer = std::sync::Arc::new(std::sync::Mutex::new(None::<UnixStream>));
    let live_input_writer =
        std::sync::Arc::new(std::sync::Mutex::new(None::<std::sync::Arc<std::sync::Mutex<File>>>));
    let mut state = DaemonState::new();
    let mut last_refresh = Instant::now() - REFRESH_INTERVAL;
    let mut should_exit = false;

    while !should_exit {
        loop {
            match control_listener.accept() {
                Ok((stream, _)) => {
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
                    spawn_client_reader(reader, control_tx.clone());
                    client_writer = Some(stream);
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(error) => return Err(format!("daemon control accept failed: {error}")),
            }
        }

        loop {
            match raw_listener.accept() {
                Ok((stream, _)) => {
                    if raw_writer.lock().ok().and_then(|guard| guard.as_ref().map(|_| ())).is_some() {
                        drop(stream);
                        continue;
                    }
                    let reader = stream
                        .try_clone()
                        .map_err(|error| format!("failed to clone raw stream: {error}"))?;
                    spawn_raw_stream_reader(reader, live_input_writer.clone(), control_tx.clone());
                    if let Ok(mut guard) = raw_writer.lock() {
                        *guard = Some(stream);
                        if let Some(writer) = guard.as_mut() {
                            for replay in state.replay_buffers.values() {
                                if replay.is_empty() {
                                    continue;
                                }
                                let _ = writer.write_all(replay);
                            }
                        }
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(error) => return Err(format!("daemon raw accept failed: {error}")),
            }
        }

        while let Ok(control) = control_rx.try_recv() {
            match control {
                ClientControl::Message(message) => {
                    if handle_client_message(
                        &mut state,
                        &mut client_writer,
                        &raw_writer,
                        &live_input_writer,
                        &control_tx,
                        message,
                    )? {
                        should_exit = true;
                    }
                }
                ClientControl::ControlDisconnected => {
                    if beachhead_debug_enabled() {
                        eprintln!("[beachhead-debug] control disconnected");
                    }
                    client_writer = None;
                }
                ClientControl::StreamDisconnected => {
                    if beachhead_debug_enabled() {
                        eprintln!("[beachhead-debug] raw stream disconnected");
                    }
                    if let Ok(mut guard) = raw_writer.lock() {
                        *guard = None;
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

        thread::sleep(Duration::from_millis(5));
    }

    let _ = fs::remove_file(&control_socket_path);
    let _ = fs::remove_file(&raw_socket_path);
    Ok(())
}

fn handle_client_message(
    state: &mut DaemonState,
    client_writer: &mut Option<UnixStream>,
    raw_writer: &std::sync::Arc<std::sync::Mutex<Option<UnixStream>>>,
    live_input_writer: &std::sync::Arc<
        std::sync::Mutex<Option<std::sync::Arc<std::sync::Mutex<File>>>>,
    >,
    control_tx: &mpsc::Sender<ClientControl>,
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
            if let Some(session_id) = state.workspace.sessions().first().map(|session| session.id) {
                ensure_runtime_forwarder(state, session_id, raw_writer.clone(), control_tx.clone());
                if let Some(writer) = state
                    .runtimes
                    .get(&session_id)
                    .and_then(|runtime| runtime.input_writer.as_ref().cloned())
                {
                    if let Ok(mut live_writer) = live_input_writer.lock() {
                        *live_writer = Some(writer);
                    }
                }
            }
            state.snapshot_dirty = true;
            Ok(false)
        }
        ClientMessage::TraceInputSample {
            trace_id,
            sent_at_us,
            len,
        } => {
            if beachhead_debug_enabled() {
                eprintln!(
                    "[beachhead-debug] trace sample id={} sent_at_us={} len={}",
                    trace_id, sent_at_us, len
                );
            }
            state.pending_input_traces.push_back(PendingInputTrace {
                trace_id,
                sent_at_us,
                len,
            });
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
        {
            let observation = state
                .observations
                .entry(session.id)
                .or_insert_with(SessionObservation::new);
            refresh_session_observation(observation, session, false);
        }
        let Some(observation) = state.observations.get(&session.id).cloned() else {
            continue;
        };

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

fn ensure_runtime_forwarder(
    state: &mut DaemonState,
    session_id: SessionId,
    raw_writer: std::sync::Arc<std::sync::Mutex<Option<UnixStream>>>,
    control_tx: mpsc::Sender<ClientControl>,
) {
    if !state.forwarded_sessions.insert(session_id) {
        return;
    }
    let Some(runtime) = state.runtimes.get_mut(&session_id) else {
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
        evidence: evidence.clone(),
    });
}

fn maybe_queue_name(
    state: &mut DaemonState,
    session_id: SessionId,
    evidence: &NamingEvidence,
) {
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
    if summary.tactical_state != Some(TacticalState::Stopped) {
        return;
    }
    let Some(shell_child_command) = observation.shell_child_command.as_deref() else {
        return;
    };
    if !looks_like_coding_agent(shell_child_command) {
        return;
    }
    if observation.last_change.elapsed().as_secs() < 20 {
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
    });
}

fn drain_worker_results(state: &mut DaemonState) -> bool {
    let mut changed = false;

    if let Some(worker) = state.summary_worker.as_ref() {
        while let Ok(result) = worker.responses.try_recv() {
            let entry = state
                .summary_cache
                .entry(result.session_id)
                .or_insert_with(SummaryCacheEntry::new);
            entry.in_flight = false;
            entry.requested_signature = None;
            match result.summary {
                Ok(summary) => {
                    entry.completed_signature = Some(result.signature);
                    entry.last_summary = Some(summary);
                    changed = true;
                }
                Err(_) => {}
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
            if let Ok(suggestion) = result.suggestion {
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

    if let Some(worker) = state.nudge_worker.as_ref() {
        while let Ok(result) = worker.responses.try_recv() {
            let mut suggestion_text = None::<String>;
            {
                let entry = state
                    .nudge_cache
                    .entry(result.session_id)
                    .or_insert_with(NudgeCacheEntry::new);
                entry.in_flight = false;
                entry.requested_signature = None;
                if let Ok(suggestion) = result.suggestion {
                    entry.completed_signature = Some(result.signature);
                    entry.last_nudge =
                        (!suggestion.text.is_empty()).then_some(suggestion.text.clone());
                    suggestion_text = (!suggestion.text.is_empty()).then_some(suggestion.text);
                    changed = true;
                }
            }
            if let Some(text) = suggestion_text {
                if send_runtime_input_line(state, result.session_id, &text).is_ok() {
                    if let Some(entry) = state.nudge_cache.get_mut(&result.session_id) {
                        entry.last_sent = Some(Instant::now());
                    }
                }
            }
        }
    }

    changed
}

fn handle_runtime_event(
    state: &mut DaemonState,
    client_writer: &mut Option<UnixStream>,
    session_id: SessionId,
    event: RuntimeEvent,
) {
    match event {
        RuntimeEvent::Stream(update) => {
            append_replay_buffer(
                state
                    .replay_buffers
                    .entry(session_id)
                    .or_default(),
                &update.output_bytes,
            );
            let observation = state
                .observations
                .entry(session_id)
                .or_insert_with(SessionObservation::new);
            apply_stream_update(observation, update);
            state.snapshot_dirty = true;

            if let Some(trace) = state.pending_input_traces.pop_front() {
                if let Some(writer) = client_writer.as_mut() {
                    let now = now_unix_us();
                    let _ = write_json_line(
                        writer,
                        &ServerMessage::TraceInputAck {
                            trace_id: trace.trace_id,
                            sent_at_us: trace.sent_at_us,
                            daemon_recv_at_us: now,
                            daemon_write_done_at_us: now,
                            len: trace.len,
                        },
                    );
                }
            }
        }
        RuntimeEvent::Exited(exit_code) => {
            state.workspace.mark_exited(session_id, exit_code);
            state.snapshot_dirty = true;
        }
    }
}

fn send_runtime_input_line(
    state: &DaemonState,
    session_id: SessionId,
    line: &str,
) -> std::io::Result<()> {
    let mut bytes = line.as_bytes().to_vec();
    bytes.push(b'\n');
    send_runtime_input_bytes(state, session_id, &bytes)
}

fn send_runtime_input_bytes(
    state: &DaemonState,
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
    writer.write_all(bytes)
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
    let config = OpenAiSynthesisConfig::from_env()?;
    let (request_tx, request_rx) = mpsc::channel::<SummaryJob>();
    let (result_tx, result_rx) = mpsc::channel::<SummaryResult>();
    thread::spawn(move || {
        while let Ok(job) = request_rx.recv() {
            let summary = summarize_blocking(&config, &job.evidence);
            let _ = result_tx.send(SummaryResult {
                session_id: job.session_id,
                signature: job.signature,
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
    let config = OpenAiNamingConfig::from_env()?;
    let (request_tx, request_rx) = mpsc::channel::<NamingJob>();
    let (result_tx, result_rx) = mpsc::channel::<NamingResult>();
    thread::spawn(move || {
        while let Ok(job) = request_rx.recv() {
            let suggestion = suggest_name_blocking(&config, &job.evidence);
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
    let config = OpenAiNudgeConfig::from_env()?;
    let (request_tx, request_rx) = mpsc::channel::<NudgeJob>();
    let (result_tx, result_rx) = mpsc::channel::<NudgeResult>();
    thread::spawn(move || {
        while let Ok(job) = request_rx.recv() {
            let suggestion = suggest_nudge_blocking(&config, &job.evidence);
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

fn spawn_client_reader(stream: UnixStream, control_tx: mpsc::Sender<ClientControl>) {
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
    control_tx: mpsc::Sender<ClientControl>,
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
    live_input_writer: std::sync::Arc<
        std::sync::Mutex<Option<std::sync::Arc<std::sync::Mutex<File>>>>,
    >,
    control_tx: mpsc::Sender<ClientControl>,
) {
    thread::spawn(move || {
        let mut reader = stream;
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => {
                    let _ = control_tx.send(ClientControl::StreamDisconnected);
                    break;
                }
                Ok(n) => {
                    let maybe_writer = live_input_writer
                        .lock()
                        .ok()
                        .and_then(|guard| guard.as_ref().cloned());
                    if let Some(writer) = maybe_writer {
                        let Ok(mut writer) = writer.lock() else {
                            break;
                        };
                        if writer.write_all(&buf[..n]).is_err() {
                            break;
                        }
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => {
                    let _ = control_tx.send(ClientControl::StreamDisconnected);
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

fn connect_or_spawn_sockets() -> Result<(UnixStream, UnixStream), String> {
    if let Ok(sockets) = connect_sockets() {
        return Ok(sockets);
    }

    let current_exe = std::env::current_exe()
        .map_err(|error| format!("failed to resolve current executable: {error}"))?;
    Command::new(current_exe)
        .arg("--beachhead-daemon")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("failed to spawn local beachhead daemon: {error}"))?;

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match connect_sockets() {
            Ok(sockets) => return Ok(sockets),
            Err(error) if Instant::now() < deadline => {
                let _ = error;
                thread::sleep(Duration::from_millis(100));
            }
            Err(error) => return Err(error),
        }
    }
}

fn connect_sockets() -> Result<(UnixStream, UnixStream), String> {
    let control = UnixStream::connect(control_socket_path()?)
        .map_err(|error| format!("failed to connect control socket: {error}"))?;
    let raw = UnixStream::connect(raw_socket_path()?)
        .map_err(|error| format!("failed to connect raw socket: {error}"))?;
    Ok((control, raw))
}

fn control_socket_path() -> Result<PathBuf, String> {
    let runtime_dir = daemon_runtime_dir();
    Ok(runtime_dir.join("exaterm").join(CONTROL_SOCKET_NAME))
}

fn raw_socket_path() -> Result<PathBuf, String> {
    let runtime_dir = daemon_runtime_dir();
    Ok(runtime_dir.join("exaterm").join(RAW_SOCKET_NAME))
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

fn now_unix_us() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as u64
}

fn beachhead_debug_enabled() -> bool {
    env::var("EXATERM_BEACHHEAD_DEBUG")
        .ok()
        .is_some_and(|value| matches!(value.as_str(), "1" | "true" | "yes"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn unique_runtime_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("exaterm-test-{label}-{nanos}"))
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
        let raw_path = raw_socket_path().expect("raw socket path");
        assert_eq!(control_path, runtime_dir.join("exaterm").join(CONTROL_SOCKET_NAME));
        assert_eq!(raw_path, runtime_dir.join("exaterm").join(RAW_SOCKET_NAME));
        std::env::remove_var("EXATERM_RUNTIME_DIR");
    }

    #[test]
    fn local_daemon_attach_create_and_terminate_workspace() {
        let _guard = env_lock().lock().expect("env lock");
        let runtime_dir = unique_runtime_dir("daemon-flow");
        std::env::set_var("EXATERM_RUNTIME_DIR", &runtime_dir);

        let handle = thread::spawn(run_local_daemon_inner);

        let deadline = Instant::now() + Duration::from_secs(5);
        let control_path = control_socket_path().expect("control socket path");
        let raw_path = raw_socket_path().expect("raw socket path");
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
        let _raw_stream = UnixStream::connect(&raw_path).expect("connect raw stream");
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
        let _guard = env_lock().lock().expect("env lock");
        let runtime_dir = unique_runtime_dir("daemon-reject");
        std::env::set_var("EXATERM_RUNTIME_DIR", &runtime_dir);

        let handle = thread::spawn(run_local_daemon_inner);

        let deadline = Instant::now() + Duration::from_secs(5);
        let control_path = control_socket_path().expect("control socket path");
        let raw_path = raw_socket_path().expect("raw socket path");
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
        let _first_raw = UnixStream::connect(&raw_path).expect("connect first raw");
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
}
