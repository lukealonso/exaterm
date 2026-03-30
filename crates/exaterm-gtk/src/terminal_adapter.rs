use crate::beachhead::RawSessionConnector;
use exaterm_core::model::launch_argv;
use exaterm_core::runtime::{RuntimeEvent, SessionRuntime, SpawnedRuntime, StreamRuntimeUpdate};
use exaterm_core::terminal_stream::TerminalStreamProcessor;
use exaterm_types::model::SessionId;
use exaterm_types::model::SessionLaunch;
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use vte4 as vte;
use vte4::prelude::*;

const DEFAULT_PROXY_ROWS: u16 = 40;
const DEFAULT_PROXY_COLS: u16 = 160;

pub struct ClientDisplayRuntime {
    pub display_resize_target: Arc<Mutex<File>>,
    pub output_writer: Arc<Mutex<File>>,
    pub last_size: Option<(u16, u16)>,
}

pub fn spawn_runtime(
    terminal: &vte::Terminal,
    launch: &SessionLaunch,
    size: PtySize,
) -> Result<SpawnedRuntime, String> {
    if direct_pty_mode_enabled() {
        spawn_direct_runtime(terminal, launch, size)
    } else {
        spawn_proxy_runtime(terminal, launch, size)
    }
}

pub fn attach_display_runtime(
    terminal: &vte::Terminal,
    size: PtySize,
) -> Result<(ClientDisplayRuntime, mpsc::Receiver<Vec<u8>>), String> {
    let (display_pty, mut display_reader, display_writer, display_resizer) =
        create_display_pty(size)?;
    terminal.set_pty(Some(&display_pty));

    let output_writer = Arc::new(Mutex::new(display_writer));
    let resize_target = Arc::new(Mutex::new(display_resizer));
    let (input_tx, input_rx) = mpsc::channel::<Vec<u8>>();

    spawn_display_input_capture_thread(&mut display_reader, input_tx);

    Ok((
        ClientDisplayRuntime {
            display_resize_target: resize_target,
            output_writer,
            last_size: Some((size.rows, size.cols)),
        },
        input_rx,
    ))
}

pub fn write_display_output(writer: &Arc<Mutex<File>>, bytes: &[u8]) -> std::io::Result<()> {
    if bytes.is_empty() {
        return Ok(());
    }
    let mut writer = writer
        .lock()
        .map_err(|_| std::io::Error::other("display writer lock poisoned"))?;
    writer.write_all(bytes)
}

pub fn spawn_daemon_display_bridge(
    connector: RawSessionConnector,
    session_id: SessionId,
    socket_name: String,
    output_writer: Arc<Mutex<File>>,
    raw_input_writers: Arc<Mutex<std::collections::BTreeMap<SessionId, Arc<Mutex<UnixStream>>>>>,
    sync_inputs_enabled: Arc<AtomicBool>,
    input_events: mpsc::Receiver<Vec<u8>>,
) {
    thread::spawn(move || {
        let Ok(raw_reader) = connector.connect_raw_session(session_id, &socket_name) else {
            return;
        };
        let Ok(raw_writer_stream) = raw_reader.try_clone() else {
            return;
        };
        let raw_writer = Arc::new(Mutex::new(raw_writer_stream));
        if let Ok(mut writers) = raw_input_writers.lock() {
            writers.insert(session_id, raw_writer.clone());
        }
        let input_raw_writer = raw_writer.clone();
        let fanout_writers = raw_input_writers.clone();
        thread::spawn(move || {
            while let Ok(bytes) = input_events.recv() {
                if sync_inputs_enabled.load(Ordering::Relaxed) {
                    let targets = fanout_writers
                        .lock()
                        .map(|writers| {
                            writers
                                .iter()
                                .map(|(target_session, writer)| (*target_session, writer.clone()))
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    let mut failed = Vec::new();
                    for (target_session, writer) in targets {
                        let Ok(mut writer) = writer.lock() else {
                            failed.push(target_session);
                            continue;
                        };
                        if writer.write_all(&bytes).is_err() {
                            failed.push(target_session);
                        }
                    }
                    if !failed.is_empty() {
                        if let Ok(mut writers) = fanout_writers.lock() {
                            for target_session in failed {
                                writers.remove(&target_session);
                            }
                        }
                    }
                } else {
                    let Ok(mut writer) = input_raw_writer.lock() else {
                        break;
                    };
                    if writer.write_all(&bytes).is_err() {
                        break;
                    }
                }
            }
        });

        let mut raw_reader = raw_reader;
        let mut buf = [0u8; 8192];
        loop {
            match raw_reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if write_display_output(&output_writer, &buf[..n]).is_err() {
                        break;
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
        if let Ok(mut writers) = raw_input_writers.lock() {
            writers.remove(&session_id);
        }
    });
}

pub fn terminal_size_hint(terminal: &vte::Terminal) -> PtySize {
    let rows = match terminal.row_count() {
        rows if rows > 0 => rows as u16,
        _ => DEFAULT_PROXY_ROWS,
    };
    let cols = match terminal.column_count() {
        cols if cols > 0 => cols as u16,
        _ => DEFAULT_PROXY_COLS,
    };
    PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    }
}

pub fn measured_terminal_size_hint(terminal: &vte::Terminal) -> Option<PtySize> {
    measured_terminal_size(terminal.row_count(), terminal.column_count())
}

fn measured_terminal_size(rows: i64, cols: i64) -> Option<PtySize> {
    (rows > 0 && cols > 0).then_some(PtySize {
        rows: rows as u16,
        cols: cols as u16,
        pixel_width: 0,
        pixel_height: 0,
    })
}

fn direct_pty_mode_enabled() -> bool {
    std::env::var("EXATERM_DIRECT_PTY")
        .ok()
        .is_some_and(|value| matches!(value.as_str(), "1" | "true" | "yes"))
}

fn spawn_direct_runtime(
    terminal: &vte::Terminal,
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
    let Some(master_fd) = pair.master.as_raw_fd() else {
        return Err("agent pty master did not expose a file descriptor".into());
    };
    let foreign_fd = unsafe { libc::dup(master_fd) };
    let input_fd = unsafe { libc::dup(master_fd) };
    if foreign_fd < 0 || input_fd < 0 {
        unsafe {
            if foreign_fd >= 0 {
                libc::close(foreign_fd);
            }
            if input_fd >= 0 {
                libc::close(input_fd);
            }
        }
        return Err(std::io::Error::last_os_error().to_string());
    }
    let master = unsafe { OwnedFd::from_raw_fd(foreign_fd) };
    let input_writer = unsafe { File::from_raw_fd(input_fd) };
    let pty = vte::Pty::foreign_sync(master, None::<&gio::Cancellable>)
        .map_err(|error| error.to_string())?;
    terminal.set_pty(Some(&pty));

    let resize_target = Arc::new(Mutex::new(pair.master));
    let (event_tx, event_rx) = mpsc::channel::<RuntimeEvent>();
    let stop_flag = Arc::new(AtomicBool::new(false));
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

fn spawn_proxy_runtime(
    terminal: &vte::Terminal,
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
    let agent_writer_fd = unsafe { libc::dup(agent_master_fd) };
    let input_writer_fd = unsafe { libc::dup(agent_master_fd) };
    if agent_reader_fd < 0 || agent_writer_fd < 0 || input_writer_fd < 0 {
        unsafe {
            if agent_reader_fd >= 0 {
                libc::close(agent_reader_fd);
            }
            if agent_writer_fd >= 0 {
                libc::close(agent_writer_fd);
            }
            if input_writer_fd >= 0 {
                libc::close(input_writer_fd);
            }
        }
        return Err(std::io::Error::last_os_error().to_string());
    }
    let mut agent_reader = unsafe { File::from_raw_fd(agent_reader_fd) };
    let mut agent_writer = unsafe { File::from_raw_fd(agent_writer_fd) };
    let input_writer = unsafe { File::from_raw_fd(input_writer_fd) };
    let resize_target = Arc::new(Mutex::new(pair.master));
    let (display_pty, mut display_reader, mut display_writer, display_resizer) =
        create_display_pty(size)?;
    terminal.set_pty(Some(&display_pty));

    let (event_tx, event_rx) = mpsc::channel::<RuntimeEvent>();
    let stop_flag = Arc::new(AtomicBool::new(false));

    spawn_proxy_relay_thread(
        &mut agent_reader,
        &mut agent_writer,
        &mut display_reader,
        &mut display_writer,
        event_tx.clone(),
        stop_flag.clone(),
    );
    spawn_wait_thread(child, event_tx, stop_flag);

    Ok(SpawnedRuntime {
        pid,
        session_runtime: SessionRuntime {
            resize_target,
            display_resize_target: Some(Arc::new(Mutex::new(display_resizer))),
            input_writer: Some(Arc::new(Mutex::new(input_writer))),
            events: event_rx,
            last_size: Some((size.rows, size.cols)),
        },
    })
}

fn spawn_proxy_relay_thread(
    agent_reader: &mut File,
    agent_writer: &mut File,
    display_reader: &mut File,
    display_writer: &mut File,
    event_tx: mpsc::Sender<RuntimeEvent>,
    stop_flag: Arc<AtomicBool>,
) {
    const RELAY_BUF_SIZE: usize = 16 * 1024;
    let mut agent_reader = agent_reader
        .try_clone()
        .expect("agent reader clone should succeed");
    let mut agent_writer = agent_writer
        .try_clone()
        .expect("agent writer clone should succeed");
    let mut display_reader = display_reader
        .try_clone()
        .expect("display reader clone should succeed");
    let mut display_writer = display_writer
        .try_clone()
        .expect("display slave writer clone should succeed");

    set_nonblocking(agent_reader.as_raw_fd()).expect("agent reader should support nonblocking");
    set_nonblocking(agent_writer.as_raw_fd()).expect("agent writer should support nonblocking");
    set_nonblocking(display_reader.as_raw_fd()).expect("display reader should support nonblocking");
    set_nonblocking(display_writer.as_raw_fd()).expect("display writer should support nonblocking");

    thread::spawn(move || {
        let mut processor = TerminalStreamProcessor::default();
        let mut to_display = Vec::<u8>::with_capacity(RELAY_BUF_SIZE);
        let mut to_agent = Vec::<u8>::with_capacity(RELAY_BUF_SIZE);
        let mut scratch = [0u8; 8192];

        loop {
            let mut fds = [
                libc::pollfd {
                    fd: display_reader.as_raw_fd(),
                    events: if to_agent.len() < RELAY_BUF_SIZE {
                        libc::POLLIN
                    } else {
                        0
                    },
                    revents: 0,
                },
                libc::pollfd {
                    fd: display_writer.as_raw_fd(),
                    events: if to_display.is_empty() {
                        0
                    } else {
                        libc::POLLOUT
                    },
                    revents: 0,
                },
                libc::pollfd {
                    fd: agent_reader.as_raw_fd(),
                    events: if to_display.len() < RELAY_BUF_SIZE {
                        libc::POLLIN
                    } else {
                        0
                    },
                    revents: 0,
                },
                libc::pollfd {
                    fd: agent_writer.as_raw_fd(),
                    events: if to_agent.is_empty() {
                        0
                    } else {
                        libc::POLLOUT
                    },
                    revents: 0,
                },
            ];

            let poll_result =
                unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, -1) };
            if poll_result < 0 {
                let error = std::io::Error::last_os_error();
                if error.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                break;
            }

            if (fds[0].revents | fds[1].revents | fds[2].revents | fds[3].revents)
                & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL)
                != 0
            {
                break;
            }

            if fds[0].revents & libc::POLLIN != 0 {
                let remaining = RELAY_BUF_SIZE.saturating_sub(to_agent.len());
                let read_len = remaining.min(scratch.len());
                match display_reader.read(&mut scratch[..read_len]) {
                    Ok(0) => break,
                    Ok(n) => to_agent.extend_from_slice(&scratch[..n]),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(_) => break,
                }
            }

            if fds[2].revents & libc::POLLIN != 0 {
                let remaining = RELAY_BUF_SIZE.saturating_sub(to_display.len());
                let read_len = remaining.min(scratch.len());
                match agent_reader.read(&mut scratch[..read_len]) {
                    Ok(0) => break,
                    Ok(n) => {
                        let chunk = &scratch[..n];
                        to_display.extend_from_slice(chunk);
                        let update = processor.ingest(chunk);
                        if !update.is_empty() || !chunk.is_empty() {
                            let _ = event_tx.send(RuntimeEvent::Stream(StreamRuntimeUpdate {
                                output_bytes: chunk.to_vec(),
                                semantic_lines: update.semantic_lines,
                                painted_line: update.painted_line,
                            }));
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(error) if error.raw_os_error() == Some(libc::EIO) => break,
                    Err(_) => break,
                }
            }

            if fds[1].revents & libc::POLLOUT != 0 && !to_display.is_empty() {
                match display_writer.write(&to_display) {
                    Ok(0) => break,
                    Ok(n) => consume_relay_buffer(&mut to_display, n),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(_) => break,
                }
            }

            if fds[3].revents & libc::POLLOUT != 0 && !to_agent.is_empty() {
                match agent_writer.write(&to_agent) {
                    Ok(0) => break,
                    Ok(n) => consume_relay_buffer(&mut to_agent, n),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(_) => break,
                }
            }
        }
        stop_flag.store(true, Ordering::Relaxed);
    });
}

fn consume_relay_buffer(buffer: &mut Vec<u8>, amount: usize) {
    if amount == 0 || amount > buffer.len() {
        return;
    }
    buffer.drain(0..amount);
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

fn spawn_display_input_capture_thread(display_reader: &mut File, input_tx: mpsc::Sender<Vec<u8>>) {
    let mut display_reader = display_reader
        .try_clone()
        .expect("display reader clone should succeed");
    thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match display_reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if input_tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
    });
}

fn create_display_pty(size: PtySize) -> Result<(vte::Pty, File, File, File), String> {
    let mut master_fd = -1;
    let mut slave_fd = -1;
    let winsize = libc::winsize {
        ws_row: size.rows,
        ws_col: size.cols,
        ws_xpixel: size.pixel_width,
        ws_ypixel: size.pixel_height,
    };
    let result = unsafe {
        libc::openpty(
            &mut master_fd,
            &mut slave_fd,
            std::ptr::null_mut(),
            std::ptr::null(),
            &winsize,
        )
    };
    if result != 0 {
        return Err(format!(
            "failed to create display pty: {}",
            std::io::Error::last_os_error()
        ));
    }

    if let Err(error) = set_raw_display_slave(slave_fd) {
        unsafe {
            libc::close(master_fd);
            libc::close(slave_fd);
        }
        return Err(format!("failed to configure display pty: {error}"));
    }

    let reader_fd = unsafe { libc::dup(slave_fd) };
    let writer_fd = unsafe { libc::dup(slave_fd) };
    let resize_fd = unsafe { libc::dup(master_fd) };
    if reader_fd < 0 || writer_fd < 0 || resize_fd < 0 {
        unsafe {
            if reader_fd >= 0 {
                libc::close(reader_fd);
            }
            if writer_fd >= 0 {
                libc::close(writer_fd);
            }
            if resize_fd >= 0 {
                libc::close(resize_fd);
            }
            libc::close(master_fd);
            libc::close(slave_fd);
        }
        return Err(std::io::Error::last_os_error().to_string());
    }

    unsafe {
        libc::close(slave_fd);
    }

    let master = unsafe { OwnedFd::from_raw_fd(master_fd) };
    let reader = unsafe { File::from_raw_fd(reader_fd) };
    let writer = unsafe { File::from_raw_fd(writer_fd) };
    let resizer = unsafe { File::from_raw_fd(resize_fd) };
    let pty = vte::Pty::foreign_sync(master, None::<&gio::Cancellable>)
        .map_err(|error| error.to_string())?;
    Ok((pty, reader, writer, resizer))
}

fn set_raw_display_slave(fd: i32) -> std::io::Result<()> {
    let mut termios = unsafe { std::mem::zeroed::<libc::termios>() };
    if unsafe { libc::tcgetattr(fd, &mut termios) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    unsafe {
        libc::cfmakeraw(&mut termios);
    }
    termios.c_cc[libc::VMIN] = 1;
    termios.c_cc[libc::VTIME] = 0;
    if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &termios) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

fn set_nonblocking(fd: i32) -> std::io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 {
        return Err(std::io::Error::last_os_error());
    }
    if unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::measured_terminal_size;

    #[test]
    fn measured_terminal_size_requires_positive_dimensions() {
        assert!(measured_terminal_size(0, 80).is_none());
        assert!(measured_terminal_size(24, 0).is_none());
        let size = measured_terminal_size(24, 80).expect("size should exist");
        assert_eq!(size.rows, 24);
        assert_eq!(size.cols, 80);
    }
}
