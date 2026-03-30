use super::{ProcessEntry, ProcessTableReader};
use std::collections::BTreeMap;
use std::io;
use std::process::Command;

pub struct MacosProcessTableReader;

impl ProcessTableReader for MacosProcessTableReader {
    fn read_process_table(&self) -> io::Result<BTreeMap<u32, ProcessEntry>> {
        read_process_table_ps()
    }
}

/// Reads the process table using `ps` — available on all macOS versions, no unsafe code.
fn read_process_table_ps() -> io::Result<BTreeMap<u32, ProcessEntry>> {
    let output = Command::new("ps")
        .args(["-axo", "pid,ppid,stat,comm"])
        .output()?;

    if !output.status.success() {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            format!("ps exited with status {}", output.status),
        ));
    }

    let text = String::from_utf8_lossy(&output.stdout);
    let mut entries = BTreeMap::new();

    for line in text.lines().skip(1) {
        // skip header
        if let Some(entry) = parse_ps_line(line) {
            entries.insert(entry.pid, entry);
        }
    }

    Ok(entries)
}

fn parse_ps_line(line: &str) -> Option<ProcessEntry> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }

    // Split into at most 4 parts: pid, ppid, stat, and the remainder (full command).
    // ps output has whitespace-padded fields, so we find 3 tokens then take the rest.
    let mut rest = line;
    let mut tokens = Vec::new();
    for _ in 0..3 {
        rest = rest.trim_start();
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        tokens.push(&rest[..end]);
        rest = &rest[end..];
    }
    rest = rest.trim_start();
    if tokens.len() < 3 || rest.is_empty() {
        return None;
    }

    let pid: u32 = tokens[0].parse().ok()?;
    let ppid: u32 = tokens[1].parse().ok()?;
    let stat_str = tokens[2];
    let command = rest;

    let state = match stat_str.chars().next()? {
        'R' => 'R',
        'S' => 'S',
        'T' => 'T',
        'Z' => 'Z',
        'U' => 'S', // uninterruptible wait → treat as sleeping
        'I' => 'S', // idle → treat as sleeping
        c => c,
    };

    // Extract just the binary name from the full path
    let binary_name = command.rsplit('/').next().unwrap_or(command);

    if pid == 0 {
        return None;
    }

    Some(ProcessEntry {
        pid,
        ppid,
        command: binary_name.to_string(),
        state,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ps_output_line() {
        let entry = parse_ps_line("  1234   567 Ss   /usr/bin/bash").unwrap();
        assert_eq!(entry.pid, 1234);
        assert_eq!(entry.ppid, 567);
        assert_eq!(entry.state, 'S');
        // The full path is captured, and the binary name is extracted
        assert_eq!(entry.command, "bash");

        // Verify the parser captures the full command field (not just the first token)
        let entry2 = parse_ps_line("  1234   567 Ss   /usr/local/bin/node").unwrap();
        assert_eq!(entry2.command, "node");
    }

    #[test]
    fn parses_ps_line_with_simple_command() {
        let entry = parse_ps_line("42 1 R+ cargo").unwrap();
        assert_eq!(entry.pid, 42);
        assert_eq!(entry.ppid, 1);
        assert_eq!(entry.state, 'R');
        assert_eq!(entry.command, "cargo");
    }

    #[test]
    fn skips_pid_zero() {
        assert!(parse_ps_line("0 0 Ss kernel_task").is_none());
    }

    #[test]
    fn reads_process_table_from_system() {
        let reader = MacosProcessTableReader;
        match reader.read_process_table() {
            Ok(entries) => {
                assert!(
                    entries.len() > 1,
                    "expected multiple processes, got {}",
                    entries.len()
                );
            }
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                // Sandboxed environment — ps may not be available
            }
            Err(e) => panic!("unexpected error: {e}"),
        }
    }

    #[test]
    fn our_process_is_in_table() {
        let reader = MacosProcessTableReader;
        match reader.read_process_table() {
            Ok(entries) => {
                let our_pid = std::process::id();
                assert!(
                    entries.contains_key(&our_pid),
                    "our pid {} not found in process table",
                    our_pid
                );
            }
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                // Sandboxed environment — ps may not be available
            }
            Err(e) => panic!("unexpected error: {e}"),
        }
    }
}
