use exaterm_core::daemon::{connect_session_stream_socket, LocalBeachheadClient};
use exaterm_types::model::SessionId;
use exaterm_types::proto::{ClientMessage, ServerMessage};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const REMOTE_STATE_SUBDIR: &str = ".local/state/exaterm";
const REMOTE_RUNTIME_SUBDIR: &str = ".local/state/exaterm/runtime";
const REMOTE_BIN_SUBDIR: &str = ".local/state/exaterm/bin";
const CONTROL_SOCKET_NAME: &str = "beachhead-control.sock";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunMode {
    Local,
    Ssh { target: String },
}

impl RunMode {
    pub fn workspace_id(&self) -> Option<&str> {
        None
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedArgs {
    pub mode: RunMode,
    pub workspace: Option<WorkspaceArg>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkspaceArg {
    New(String),
    Resume(String),
}

impl WorkspaceArg {
    pub fn id(&self) -> &str {
        match self {
            WorkspaceArg::New(id) | WorkspaceArg::Resume(id) => id,
        }
    }
}

#[derive(Clone, Debug)]
pub enum BeachheadTarget {
    Local,
    Ssh(String),
}

impl BeachheadTarget {
    pub fn from_parsed(args: &ParsedArgs) -> Self {
        match &args.mode {
            RunMode::Local => Self::Local,
            RunMode::Ssh { target } => Self::Ssh(target.clone()),
        }
    }
}

impl From<&RunMode> for BeachheadTarget {
    fn from(value: &RunMode) -> Self {
        match value {
            RunMode::Local => Self::Local,
            RunMode::Ssh { target } => Self::Ssh(target.clone()),
        }
    }
}

pub fn parse_run_mode(args: impl IntoIterator<Item = String>) -> Result<ParsedArgs, String> {
    let args: Vec<String> = args.into_iter().collect();
    let mut mode = RunMode::Local;
    let mut workspace: Option<WorkspaceArg> = None;
    let mut i = 0;

    while i < args.len() {
        match args[i].as_str() {
            "--ssh" => {
                let target = args.get(i + 1).ok_or("--ssh requires a target like user@host")?;
                mode = RunMode::Ssh {
                    target: target.clone(),
                };
                i += 2;
            }
            "--new" => {
                let id = args
                    .get(i + 1)
                    .ok_or("--new requires a workspace id")?;
                workspace = Some(WorkspaceArg::New(id.clone()));
                i += 2;
            }
            "--resume" => {
                let id = args
                    .get(i + 1)
                    .ok_or("--resume requires a workspace id")?;
                workspace = Some(WorkspaceArg::Resume(id.clone()));
                i += 2;
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }

    Ok(ParsedArgs { mode, workspace })
}

#[derive(Clone)]
pub enum RawSessionConnector {
    Local,
    Remote(Arc<RemoteRawSessionConnector>),
}

impl RawSessionConnector {
    pub fn connect_raw_session(
        &self,
        session_id: SessionId,
        socket_name: &str,
    ) -> Result<UnixStream, String> {
        match self {
            RawSessionConnector::Local => connect_session_stream_socket(socket_name),
            RawSessionConnector::Remote(bridge) => {
                bridge.connect_raw_session(session_id, socket_name)
            }
        }
    }
}

pub struct BeachheadConnection {
    client: LocalBeachheadClient,
    raw_sessions: RawSessionConnector,
    _remote_bridge: Option<RemoteBeachheadBridge>,
}

impl BeachheadConnection {
    pub fn connect(target: &BeachheadTarget) -> Result<Self, String> {
        match target {
            BeachheadTarget::Local => Ok(Self {
                client: LocalBeachheadClient::connect_or_spawn()?,
                raw_sessions: RawSessionConnector::Local,
                _remote_bridge: None,
            }),
            BeachheadTarget::Ssh(target) => {
                let (client, bridge) = connect_remote(target)?;
                Ok(Self {
                    client,
                    raw_sessions: RawSessionConnector::Remote(bridge.raw_connector()),
                    _remote_bridge: Some(bridge),
                })
            }
        }
    }

    pub fn commands(&self) -> &mpsc::Sender<ClientMessage> {
        &self.client.commands
    }

    pub fn events(&self) -> &crossbeam_channel::Receiver<ServerMessage> {
        &self.client.events
    }

    pub fn event_wake_fd(&self) -> i32 {
        self.client.event_wake_fd()
    }

    pub fn drain_event_wake(&self) {
        self.client.drain_event_wake();
    }

    pub fn raw_session_connector(&self) -> RawSessionConnector {
        self.raw_sessions.clone()
    }
}

pub struct RemoteBeachheadBridge {
    control_forward_process: Child,
    raw_connector: Arc<RemoteRawSessionConnector>,
}

struct SessionForward {
    process: Child,
    local_socket_path: PathBuf,
    remote_socket_name: String,
}

pub struct RemoteRawSessionConnector {
    target: String,
    local_socket_dir: PathBuf,
    remote_socket_dir: String,
    session_forwards: Mutex<BTreeMap<SessionId, SessionForward>>,
}

impl Drop for RemoteBeachheadBridge {
    fn drop(&mut self) {
        let _ = self.control_forward_process.kill();
        let _ = self.control_forward_process.wait();
        self.raw_connector.shutdown();
        let _ = fs::remove_dir_all(&self.raw_connector.local_socket_dir);
    }
}

struct RemoteHostInfo {
    os: String,
    arch: String,
    home: String,
}

pub fn connect_remote(
    target: &str,
) -> Result<(LocalBeachheadClient, RemoteBeachheadBridge), String> {
    let info = probe_remote_host(target)?;
    ensure_supported_remote(&info)?;

    let local_exatermd = local_exatermd_path()?;
    let remote_root = format!("{}/{}", info.home, REMOTE_STATE_SUBDIR);
    let remote_bin_dir = format!("{}/{}", info.home, REMOTE_BIN_SUBDIR);
    let remote_runtime_dir = format!("{}/{}", info.home, REMOTE_RUNTIME_SUBDIR);
    let remote_socket_dir = match std::env::var("EXATERM_WORKSPACE") {
        Ok(ws) if !ws.is_empty() => format!("{remote_runtime_dir}/exaterm/{ws}"),
        _ => format!("{remote_runtime_dir}/exaterm"),
    };
    let remote_bin = format!("{remote_bin_dir}/exatermd");
    let remote_control = format!("{remote_socket_dir}/{CONTROL_SOCKET_NAME}");

    ensure_remote_dirs(target, &remote_root, &remote_bin_dir, &remote_runtime_dir)?;
    upload_remote_exatermd(target, &local_exatermd, &remote_bin)?;
    launch_remote_daemon(target, &remote_bin, &remote_runtime_dir, &remote_control)?;

    let local_socket_dir = unique_local_socket_dir("ssh-bridge");
    fs::create_dir_all(&local_socket_dir)
        .map_err(|error| format!("create local socket dir: {error}"))?;
    let local_control = local_socket_dir.join("control.sock");

    let mut forward = Command::new("ssh");
    forward
        .arg("-o")
        .arg("ExitOnForwardFailure=yes")
        .arg("-o")
        .arg("StreamLocalBindUnlink=yes")
        .arg("-N")
        .arg("-L")
        .arg(format!("{}:{}", local_control.display(), remote_control))
        .arg(target)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let forward_process = forward
        .spawn()
        .map_err(|error| format!("failed to start SSH socket forwarder: {error}"))?;

    struct ForwarderCleanup {
        forward_process: Option<Child>,
        local_socket_dir: Option<PathBuf>,
    }

    impl Drop for ForwarderCleanup {
        fn drop(&mut self) {
            if let Some(mut forward_process) = self.forward_process.take() {
                let _ = forward_process.kill();
                let _ = forward_process.wait();
            }
            if let Some(local_socket_dir) = self.local_socket_dir.take() {
                let _ = fs::remove_dir_all(local_socket_dir);
            }
        }
    }

    let mut cleanup = ForwarderCleanup {
        forward_process: Some(forward_process),
        local_socket_dir: Some(local_socket_dir),
    };

    let control = wait_for_forwarded_control_socket(
        &local_control,
        cleanup
            .forward_process
            .as_mut()
            .expect("forwarder should exist"),
    )?;
    let client = LocalBeachheadClient::connect_control(control)?;
    let forward_process = cleanup
        .forward_process
        .take()
        .expect("forwarder should exist");
    let local_socket_dir = cleanup
        .local_socket_dir
        .take()
        .expect("socket dir should exist");
    let raw_connector = Arc::new(RemoteRawSessionConnector {
        target: target.to_string(),
        local_socket_dir,
        remote_socket_dir,
        session_forwards: Mutex::new(BTreeMap::new()),
    });
    Ok((
        client,
        RemoteBeachheadBridge {
            control_forward_process: forward_process,
            raw_connector,
        },
    ))
}

impl RemoteBeachheadBridge {
    pub fn raw_connector(&self) -> Arc<RemoteRawSessionConnector> {
        self.raw_connector.clone()
    }
}

impl RemoteRawSessionConnector {
    pub fn connect_raw_session(
        &self,
        session_id: SessionId,
        socket_name: &str,
    ) -> Result<UnixStream, String> {
        let local_socket_path = self
            .local_socket_dir
            .join(format!("session-{}.sock", session_id.0));
        let remote_socket_path = format!("{}/{}", self.remote_socket_dir, socket_name);
        let needs_refresh = {
            let forwards = self
                .session_forwards
                .lock()
                .map_err(|_| "remote session forward lock poisoned".to_string())?;
            forwards
                .get(&session_id)
                .is_none_or(|forward| forward.remote_socket_name != socket_name)
        };

        if needs_refresh {
            let mut forwards = self
                .session_forwards
                .lock()
                .map_err(|_| "remote session forward lock poisoned".to_string())?;
            if let Some(mut existing) = forwards.remove(&session_id) {
                let _ = existing.process.kill();
                let _ = existing.process.wait();
                let _ = fs::remove_file(existing.local_socket_path);
            }

            let mut forward = Command::new("ssh");
            forward
                .arg("-o")
                .arg("ExitOnForwardFailure=yes")
                .arg("-o")
                .arg("StreamLocalBindUnlink=yes")
                .arg("-N")
                .arg("-L")
                .arg(format!(
                    "{}:{}",
                    local_socket_path.display(),
                    remote_socket_path
                ))
                .arg(&self.target)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            let process = forward
                .spawn()
                .map_err(|error| format!("failed to start SSH raw-session forwarder: {error}"))?;
            forwards.insert(
                session_id,
                SessionForward {
                    process,
                    local_socket_path: local_socket_path.clone(),
                    remote_socket_name: socket_name.to_string(),
                },
            );
        }

        wait_for_forwarded_control_socket(
            &local_socket_path,
            &mut self
                .session_forwards
                .lock()
                .map_err(|_| "remote session forward lock poisoned".to_string())?
                .get_mut(&session_id)
                .expect("session forward should exist")
                .process,
        )
    }

    fn shutdown(&self) {
        let forwards = self.session_forwards.lock();
        if let Ok(mut forwards) = forwards {
            for (_, mut forward) in std::mem::take(&mut *forwards) {
                let _ = forward.process.kill();
                let _ = forward.process.wait();
                let _ = fs::remove_file(forward.local_socket_path);
            }
        }
    }
}

fn probe_remote_host(target: &str) -> Result<RemoteHostInfo, String> {
    let output = run_remote_shell(
        target,
        "printf '%s\\t%s\\t%s\\n' \"$(uname -s)\" \"$(uname -m)\" \"$HOME\"",
    )?;
    if !output.status.success() {
        return Err(format!(
            "remote probe failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut parts = stdout.trim().split('\t');
    let os = parts.next().unwrap_or_default().to_string();
    let arch = parts.next().unwrap_or_default().to_string();
    let home = parts.next().unwrap_or_default().to_string();
    if os.is_empty() || arch.is_empty() || home.is_empty() {
        return Err("remote probe returned incomplete host info".into());
    }
    Ok(RemoteHostInfo { os, arch, home })
}

fn ensure_supported_remote(info: &RemoteHostInfo) -> Result<(), String> {
    if info.os != "Linux" {
        return Err(format!(
            "remote beachhead currently supports Linux only, got {}",
            info.os
        ));
    }
    let local_arch = std::env::consts::ARCH;
    if info.arch != local_arch {
        return Err(format!(
            "remote architecture {} does not match local exatermd build architecture {}",
            info.arch, local_arch
        ));
    }
    Ok(())
}

fn local_exatermd_path() -> Result<PathBuf, String> {
    let current_exe = std::env::current_exe()
        .map_err(|error| format!("failed to resolve current executable: {error}"))?;
    let candidate = current_exe.with_file_name("exatermd");
    if candidate.exists() {
        return Ok(candidate);
    }
    Err(format!(
        "missing sibling exatermd at {}; build it first with `make`",
        candidate.display()
    ))
}

fn ensure_remote_dirs(
    target: &str,
    remote_root: &str,
    remote_bin_dir: &str,
    remote_runtime_dir: &str,
) -> Result<(), String> {
    let script = format!(
        "set -eu; mkdir -p {} {} {}",
        shell_quote(remote_root),
        shell_quote(remote_bin_dir),
        shell_quote(remote_runtime_dir),
    );
    run_remote_shell(target, &script).map(|_| ())
}

fn upload_remote_exatermd(
    target: &str,
    local_exatermd: &Path,
    remote_bin: &str,
) -> Result<(), String> {
    let remote_tmp = format!("{remote_bin}.upload");
    let output = Command::new("scp")
        .arg(local_exatermd)
        .arg(format!("{target}:{remote_tmp}"))
        .output()
        .map_err(|error| format!("failed to upload remote exatermd: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let detail = if stderr.is_empty() {
            format!("scp exited with status {}", output.status)
        } else {
            stderr
        };
        return Err(format!("scp upload of remote exatermd failed: {detail}"));
    }

    let finalize = format!(
        "set -eu; chmod +x {tmp}; mv -f {tmp} {bin}",
        tmp = shell_quote(&remote_tmp),
        bin = shell_quote(remote_bin),
    );
    run_remote_shell(target, &finalize).map(|_| ())
}

fn launch_remote_daemon(
    target: &str,
    remote_bin: &str,
    remote_runtime_dir: &str,
    remote_control: &str,
) -> Result<(), String> {
    let mut exports = vec![format!(
        "export EXATERM_RUNTIME_DIR={}",
        shell_quote(remote_runtime_dir)
    )];
    exports.push("export EXATERM_SHELL_MODE='login'".to_string());
    for key in [
        "OPENAI_API_KEY",
        "EXATERM_OPENAI_BASE_URL",
        "OPENAI_BASE_URL",
        "EXATERM_SUMMARY_MODEL",
        "EXATERM_NAMING_MODEL",
        "EXATERM_NUDGE_MODEL",
        "EXATERM_WORKSPACE",
    ] {
        if let Some(value) = std::env::var_os(key) {
            exports.push(format!("export {key}={}", shell_quote_os(&value)));
        }
    }

    let script = format!(
        "set -eu; chmod +x {bin}; {exports}; nohup {bin} {log_redirection} < /dev/null & \
         i=0; while [ \"$i\" -lt 50 ]; do \
           if [ -S {control} ]; then exit 0; fi; \
           i=$((i+1)); sleep 0.1; \
        done; \
         echo 'remote beachhead control socket did not appear' >&2; exit 1",
        bin = shell_quote(remote_bin),
        exports = exports.join("; "),
        log_redirection = ">/dev/null 2>&1",
        control = shell_quote(remote_control),
    );
    run_remote_shell(target, &script).map(|_| ())
}

fn run_remote_shell(target: &str, script: &str) -> Result<std::process::Output, String> {
    let remote_command = ssh_shell_command(script);
    let output = Command::new("ssh")
        .arg(target)
        .arg(remote_command)
        .output()
        .map_err(|error| format!("failed to run remote command: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "remote command failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(output)
}

fn ssh_shell_command(script: &str) -> String {
    format!("sh -lc {}", shell_quote(script))
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

fn shell_quote_os(value: &OsString) -> String {
    shell_quote(&value.to_string_lossy())
}

fn unique_local_socket_dir(prefix: &str) -> PathBuf {
    let pid = std::process::id();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    std::env::temp_dir().join(format!("exaterm-{prefix}-{pid}-{nanos}"))
}

fn wait_for_forwarded_control_socket(
    path: &Path,
    process: &mut Child,
) -> Result<UnixStream, String> {
    let start = Instant::now();
    loop {
        match UnixStream::connect(path) {
            Ok(stream) => return Ok(stream),
            Err(error) if start.elapsed() >= Duration::from_secs(5) => {
                let _ = process.try_wait();
                return Err(format!(
                    "timed out waiting for forwarded remote beachhead control socket: {error}"
                ));
            }
            Err(_) => {
                let _ = process.try_wait();
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_run_mode, shell_quote, ssh_shell_command, RunMode, WorkspaceArg};

    #[test]
    fn parses_local_run_mode_without_args() {
        let parsed = parse_run_mode(Vec::<String>::new()).unwrap();
        assert_eq!(parsed.mode, RunMode::Local);
        assert!(parsed.workspace.is_none());
    }

    #[test]
    fn parses_ssh_run_mode() {
        let parsed = parse_run_mode(vec!["--ssh".into(), "user@example.com".into()]).unwrap();
        assert_eq!(
            parsed.mode,
            RunMode::Ssh {
                target: "user@example.com".into(),
            }
        );
    }

    #[test]
    fn parses_new_workspace() {
        let parsed = parse_run_mode(vec!["--new".into(), "mywork".into()]).unwrap();
        assert_eq!(parsed.mode, RunMode::Local);
        assert_eq!(parsed.workspace, Some(WorkspaceArg::New("mywork".into())));
    }

    #[test]
    fn parses_resume_workspace() {
        let parsed = parse_run_mode(vec!["--resume".into(), "mywork".into()]).unwrap();
        assert_eq!(parsed.mode, RunMode::Local);
        assert_eq!(
            parsed.workspace,
            Some(WorkspaceArg::Resume("mywork".into()))
        );
    }

    #[test]
    fn parses_ssh_with_workspace() {
        let parsed = parse_run_mode(vec![
            "--ssh".into(),
            "user@host".into(),
            "--new".into(),
            "remote-work".into(),
        ])
        .unwrap();
        assert_eq!(
            parsed.mode,
            RunMode::Ssh {
                target: "user@host".into()
            }
        );
        assert_eq!(
            parsed.workspace,
            Some(WorkspaceArg::New("remote-work".into()))
        );
    }

    #[test]
    fn missing_ssh_target_is_error() {
        assert!(parse_run_mode(vec!["--ssh".into()]).is_err());
    }

    #[test]
    fn missing_workspace_id_is_error() {
        assert!(parse_run_mode(vec!["--new".into()]).is_err());
        assert!(parse_run_mode(vec!["--resume".into()]).is_err());
    }

    #[test]
    fn ssh_shell_command_quotes_script_as_single_argument() {
        assert_eq!(
            ssh_shell_command("printf '%s\\n' \"$HOME\""),
            "sh -lc 'printf '\\''%s\\n'\\'' \"$HOME\"'"
        );
    }

    #[test]
    fn shell_quote_escapes_single_quotes() {
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
    }
}
