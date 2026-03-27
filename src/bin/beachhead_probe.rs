use exaterm::proto::{ClientMessage, ServerMessage};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn main() {
    if let Err(error) = run() {
        eprintln!("beachhead_probe failed: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let runtime_dir = unique_runtime_dir("probe");
    std::env::set_var("EXATERM_RUNTIME_DIR", &runtime_dir);

    let daemon_thread = thread::spawn(exaterm::run_local_daemon);
    wait_for_socket_pair(&runtime_dir, Duration::from_secs(5))?;

    let control_path = runtime_dir.join("exaterm").join("beachhead-control.sock");
    let raw_path = runtime_dir.join("exaterm").join("beachhead-stream.sock");
    let mut control = UnixStream::connect(&control_path).map_err(|error| error.to_string())?;
    let mut control_reader = BufReader::new(
        control
            .try_clone()
            .map_err(|error| format!("clone control socket: {error}"))?,
    );
    let mut raw = UnixStream::connect(&raw_path).map_err(|error| error.to_string())?;
    raw.set_nonblocking(true)
        .map_err(|error| format!("set raw nonblocking: {error}"))?;

    write_json_line(&mut control, &ClientMessage::AttachClient)?;
    drain_until_snapshot(&mut control_reader, false)?;
    write_json_line(&mut control, &ClientMessage::CreateOrResumeDefaultWorkspace)?;
    let session_count = drain_until_snapshot(&mut control_reader, true)?;
    println!("attached to isolated beachhead with {session_count} live session(s)");

    // Let the shell settle and drain any prompt/banner backlog.
    thread::sleep(Duration::from_millis(200));
    drain_raw(&mut raw)?;

    let sample_count = 25u64;
    let mut echo_roundtrip_us = Vec::new();
    for sample in 1..=sample_count {
        let start = Instant::now();
        raw.write_all(b"a")
            .map_err(|error| format!("raw write failed: {error}"))?;
        wait_for_echo(&mut raw, b'a', Duration::from_secs(2))?;
        let elapsed = start.elapsed().as_micros();
        echo_roundtrip_us.push(elapsed);
        println!("sample #{sample:02} raw echo roundtrip={elapsed}us");

        // Remove the typed char so each sample starts from a clean prompt.
        raw.write_all(&[0x7f])
            .map_err(|error| format!("raw backspace failed: {error}"))?;
        thread::sleep(Duration::from_millis(25));
        let _ = drain_raw(&mut raw);
    }

    println!();
    print_summary("raw_echo_roundtrip", &echo_roundtrip_us);

    write_json_line(&mut control, &ClientMessage::TerminateWorkspace)?;
    drop(raw);
    drop(control);
    let _ = daemon_thread.join();
    let _ = std::fs::remove_dir_all(runtime_dir);
    Ok(())
}

fn drain_until_snapshot(
    reader: &mut BufReader<UnixStream>,
    expect_nonempty: bool,
) -> Result<usize, String> {
    loop {
        match read_server_message(reader)? {
            ServerMessage::WorkspaceSnapshot { snapshot } => {
                let count = snapshot.sessions.len();
                if !expect_nonempty || count > 0 {
                    return Ok(count);
                }
            }
            ServerMessage::Error { message } => return Err(message),
            ServerMessage::TraceInputAck { .. } => {}
        }
    }
}

fn read_server_message(reader: &mut BufReader<UnixStream>) -> Result<ServerMessage, String> {
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .map_err(|error| format!("read control message: {error}"))?;
    serde_json::from_str(line.trim()).map_err(|error| format!("parse control message: {error}"))
}

fn write_json_line<W: Write, T: serde::Serialize>(writer: &mut W, value: &T) -> Result<(), String> {
    serde_json::to_writer(&mut *writer, value).map_err(|error| error.to_string())?;
    writer.write_all(b"\n").map_err(|error| error.to_string())?;
    writer.flush().map_err(|error| error.to_string())
}

fn wait_for_echo(raw: &mut UnixStream, byte: u8, timeout: Duration) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    let mut buf = [0u8; 8192];
    while Instant::now() < deadline {
        match raw.read(&mut buf) {
            Ok(0) => return Err("raw stream closed while waiting for echo".into()),
            Ok(n) => {
                if buf[..n].contains(&byte) {
                    return Ok(());
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(1));
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(format!("raw read failed: {error}")),
        }
    }
    Err("timed out waiting for raw echo".into())
}

fn drain_raw(raw: &mut UnixStream) -> Result<usize, String> {
    let mut buf = [0u8; 8192];
    let mut total = 0usize;
    loop {
        match raw.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => total += n,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(format!("raw drain failed: {error}")),
        }
    }
    Ok(total)
}

fn print_summary(label: &str, samples: &[u128]) {
    if samples.is_empty() {
        println!("{label}: no samples");
        return;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let min = sorted[0];
    let max = *sorted.last().unwrap_or(&min);
    let median = sorted[sorted.len() / 2];
    let mean = sorted.iter().copied().sum::<u128>() / sorted.len() as u128;
    println!(
        "{label}: min={min}us median={median}us mean={mean}us max={max}us over {} samples",
        sorted.len()
    );
}

fn unique_runtime_dir(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be after epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("exaterm-{label}-{nanos}"))
}

fn wait_for_socket_pair(runtime_dir: &PathBuf, timeout: Duration) -> Result<(), String> {
    let control = runtime_dir.join("exaterm").join("beachhead-control.sock");
    let raw = runtime_dir.join("exaterm").join("beachhead-stream.sock");
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if control.exists() && raw.exists() {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(25));
    }
    Err("timed out waiting for beachhead sockets".into())
}
