use crate::model::{SessionLaunch, launch_argv};
use crate::terminal_stream::TerminalStreamProcessor;
use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::fs::File;
use std::io::Read;
use std::os::fd::FromRawFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;

pub struct SessionRuntime {
    pub resize_target: Arc<Mutex<Box<dyn MasterPty + Send>>>,
    pub display_resize_target: Option<Arc<Mutex<File>>>,
    pub input_writer: Option<Arc<Mutex<File>>>,
    pub events: mpsc::Receiver<RuntimeEvent>,
    pub last_size: Option<(u16, u16)>,
}

pub enum RuntimeEvent {
    Stream(StreamRuntimeUpdate),
    Exited(i32),
}

pub struct StreamRuntimeUpdate {
    pub output_bytes: Vec<u8>,
    pub semantic_lines: Vec<String>,
    pub painted_line: Option<String>,
}

pub struct SpawnedRuntime {
    pub pid: Option<u32>,
    pub session_runtime: SessionRuntime,
}

pub fn spawn_headless_runtime(
    launch: &SessionLaunch,
    size: PtySize,
) -> Result<SpawnedRuntime, String> {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(size)
        .map_err(|error| format!("failed to create agent pty: {error}"))?;

    let builder = command_builder(launch);
    let child = pair
        .slave
        .spawn_command(builder)
        .map_err(|error| format!("failed to spawn command: {error}"))?;
    drop(pair.slave);

    let pid = child.process_id();
    let Some(agent_master_fd) = pair.master.as_raw_fd() else {
        return Err("agent pty master did not expose a file descriptor".into());
    };
    let agent_reader_fd = unsafe { libc::dup(agent_master_fd) };
    let input_writer_fd = unsafe { libc::dup(agent_master_fd) };
    if agent_reader_fd < 0 || input_writer_fd < 0 {
        unsafe {
            if agent_reader_fd >= 0 {
                libc::close(agent_reader_fd);
            }
            if input_writer_fd >= 0 {
                libc::close(input_writer_fd);
            }
        }
        return Err(std::io::Error::last_os_error().to_string());
    }

    let agent_reader = unsafe { File::from_raw_fd(agent_reader_fd) };
    let input_writer = unsafe { File::from_raw_fd(input_writer_fd) };
    let resize_target = Arc::new(Mutex::new(pair.master));
    let (event_tx, event_rx) = mpsc::channel::<RuntimeEvent>();
    let stop_flag = Arc::new(AtomicBool::new(false));

    spawn_headless_output_thread(agent_reader, event_tx.clone(), stop_flag.clone());
    spawn_wait_thread(child, event_tx, stop_flag);

    Ok(SpawnedRuntime {
        pid,
        session_runtime: SessionRuntime {
            resize_target,
            display_resize_target: None,
            input_writer: Some(Arc::new(Mutex::new(input_writer))),
            events: event_rx,
            last_size: Some((size.rows, size.cols)),
        },
    })
}

fn command_builder(launch: &SessionLaunch) -> CommandBuilder {
    let argv_owned = launch_argv(launch);
    let mut builder = CommandBuilder::new(&argv_owned[0]);
    for arg in argv_owned.iter().skip(1) {
        builder.arg(arg);
    }
    builder.env("TERM", "xterm-256color");
    builder.env("COLORTERM", "truecolor");
    builder.env("TERM_PROGRAM", "exaterm");
    if let Some(cwd) = launch.cwd.as_ref() {
        builder.cwd(cwd);
    }
    builder
}

fn spawn_headless_output_thread(
    mut agent_reader: File,
    event_tx: mpsc::Sender<RuntimeEvent>,
    stop_flag: Arc<AtomicBool>,
) {
    thread::spawn(move || {
        let mut processor = TerminalStreamProcessor::default();
        let mut buf = [0u8; 8192];
        loop {
            match agent_reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let chunk = &buf[..n];
                    let update = processor.ingest(chunk);
                    let _ = event_tx.send(RuntimeEvent::Stream(StreamRuntimeUpdate {
                        output_bytes: chunk.to_vec(),
                        semantic_lines: update.semantic_lines,
                        painted_line: update.painted_line,
                    }));
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) if error.raw_os_error() == Some(libc::EIO) => break,
                Err(_) => break,
            }
        }
        stop_flag.store(true, Ordering::Relaxed);
    });
}

fn spawn_wait_thread(
    mut child: Box<dyn portable_pty::Child + Send + Sync>,
    event_tx: mpsc::Sender<RuntimeEvent>,
    stop_flag: Arc<AtomicBool>,
) {
    thread::spawn(move || {
        let exit_code = child
            .wait()
            .map(|status| status.exit_code() as i32)
            .unwrap_or(-1);
        stop_flag.store(true, Ordering::Relaxed);
        let _ = event_tx.send(RuntimeEvent::Exited(exit_code));
    });
}
