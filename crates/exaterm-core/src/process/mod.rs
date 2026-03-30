use std::collections::{BTreeMap, BTreeSet};
use std::io;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessEntry {
    pub pid: u32,
    pub ppid: u32,
    pub command: String,
    pub state: char,
}

pub trait ProcessTableReader: Send + Sync {
    fn read_process_table(&self) -> io::Result<BTreeMap<u32, ProcessEntry>>;
}

pub fn format_process_tree(reader: &dyn ProcessTableReader, root_pid: u32) -> io::Result<String> {
    let entries = reader.read_process_table()?;
    Ok(format_process_tree_from_entries(&entries, root_pid))
}

pub fn dominant_child_command(
    reader: &dyn ProcessTableReader,
    root_pid: u32,
) -> io::Result<Option<String>> {
    let entries = reader.read_process_table()?;
    Ok(dominant_child_command_from_entries(&entries, root_pid))
}

pub fn direct_child_command(
    reader: &dyn ProcessTableReader,
    root_pid: u32,
) -> io::Result<Option<String>> {
    let entries = reader.read_process_table()?;
    Ok(direct_child_command_from_entries(&entries, root_pid))
}

pub fn format_process_tree_from_entries(
    entries: &BTreeMap<u32, ProcessEntry>,
    root_pid: u32,
) -> String {
    let Some(root) = entries.get(&root_pid) else {
        return format!("Process {root_pid} is no longer running.");
    };

    let children = build_children_map(entries);

    let mut lines = Vec::new();
    let mut visited = BTreeSet::new();
    write_tree(root.pid, 0, entries, &children, &mut visited, &mut lines);
    lines.join("\n")
}

pub fn dominant_child_command_from_entries(
    entries: &BTreeMap<u32, ProcessEntry>,
    root_pid: u32,
) -> Option<String> {
    let root = entries.get(&root_pid)?;
    let children = build_children_map(entries);

    let mut stack = children.get(&root.pid).cloned().unwrap_or_default();
    while let Some(pid) = stack.pop() {
        let Some(entry) = entries.get(&pid) else {
            continue;
        };
        if is_significant_command(&entry.command) {
            return Some(entry.command.clone());
        }
        if let Some(child_pids) = children.get(&pid) {
            for child in child_pids.iter().rev() {
                stack.push(*child);
            }
        }
    }

    None
}

pub fn direct_child_command_from_entries(
    entries: &BTreeMap<u32, ProcessEntry>,
    root_pid: u32,
) -> Option<String> {
    let root = entries.get(&root_pid)?;

    let mut child_pids = entries
        .values()
        .filter(|entry| entry.ppid == root.pid)
        .map(|entry| entry.pid)
        .collect::<Vec<_>>();
    child_pids.sort_unstable();

    for pid in child_pids {
        let Some(entry) = entries.get(&pid) else {
            continue;
        };
        if is_significant_command(&entry.command) {
            return Some(entry.command.clone());
        }
    }

    None
}

fn build_children_map(entries: &BTreeMap<u32, ProcessEntry>) -> BTreeMap<u32, Vec<u32>> {
    let mut children: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for entry in entries.values() {
        children.entry(entry.ppid).or_default().push(entry.pid);
    }
    for pids in children.values_mut() {
        pids.sort_unstable();
    }
    children
}

fn write_tree(
    pid: u32,
    depth: usize,
    entries: &BTreeMap<u32, ProcessEntry>,
    children: &BTreeMap<u32, Vec<u32>>,
    visited: &mut BTreeSet<u32>,
    lines: &mut Vec<String>,
) {
    if !visited.insert(pid) {
        return;
    }
    let Some(entry) = entries.get(&pid) else {
        return;
    };

    let indent = if depth == 0 {
        String::new()
    } else {
        "  ".repeat(depth)
    };
    lines.push(format!(
        "{indent}{} [{}] pid={} ppid={}",
        entry.command, entry.state, entry.pid, entry.ppid
    ));

    if let Some(child_pids) = children.get(&pid) {
        for child_pid in child_pids {
            write_tree(*child_pid, depth + 1, entries, children, visited, lines);
        }
    }
}

fn is_significant_command(command: &str) -> bool {
    !matches!(
        command,
        "bash" | "sh" | "dash" | "env" | "sleep" | "timeout" | "systemd" | "date"
    )
}

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::LinuxProcessTableReader;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::MacosProcessTableReader;

/// Returns the platform-default process table reader.
pub fn default_reader() -> Box<dyn ProcessTableReader> {
    #[cfg(target_os = "linux")]
    {
        Box::new(LinuxProcessTableReader)
    }
    #[cfg(target_os = "macos")]
    {
        Box::new(MacosProcessTableReader)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        Box::new(NullProcessTableReader)
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
struct NullProcessTableReader;

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
impl ProcessTableReader for NullProcessTableReader {
    fn read_process_table(&self) -> io::Result<BTreeMap<u32, ProcessEntry>> {
        Ok(BTreeMap::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockReader {
        entries: BTreeMap<u32, ProcessEntry>,
    }

    impl ProcessTableReader for MockReader {
        fn read_process_table(&self) -> io::Result<BTreeMap<u32, ProcessEntry>> {
            Ok(self.entries.clone())
        }
    }

    fn entry(pid: u32, ppid: u32, command: &str) -> ProcessEntry {
        ProcessEntry {
            pid,
            ppid,
            command: command.into(),
            state: 'S',
        }
    }

    fn mock(entries: Vec<ProcessEntry>) -> MockReader {
        MockReader {
            entries: entries.iter().map(|e| (e.pid, e.clone())).collect(),
        }
    }

    #[test]
    fn format_tree_reports_missing_root() {
        let reader = mock(vec![]);
        let output = format_process_tree(&reader, 999).unwrap();
        assert!(output.contains("no longer running"));
    }

    #[test]
    fn format_tree_renders_parent_and_child() {
        let reader = mock(vec![entry(100, 1, "bash"), entry(200, 100, "cargo")]);
        let output = format_process_tree(&reader, 100).unwrap();
        assert!(output.contains("bash [S] pid=100"));
        assert!(output.contains("cargo [S] pid=200"));
    }

    #[test]
    fn dominant_child_skips_shell_wrappers() {
        let reader = mock(vec![
            entry(100, 1, "bash"),
            entry(200, 100, "bash"),
            entry(300, 200, "cargo"),
        ]);
        let result = dominant_child_command(&reader, 100).unwrap();
        assert_eq!(result, Some("cargo".into()));
    }

    #[test]
    fn dominant_child_returns_none_for_only_shells() {
        let reader = mock(vec![entry(100, 1, "bash"), entry(200, 100, "sh")]);
        let result = dominant_child_command(&reader, 100).unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn direct_child_picks_significant_command() {
        let reader = mock(vec![
            entry(100, 1, "bash"),
            entry(200, 100, "bash"),
            entry(300, 100, "codex"),
            entry(400, 200, "cargo"),
        ]);
        let result = direct_child_command(&reader, 100).unwrap();
        assert_eq!(result, Some("codex".into()));
    }

    #[test]
    fn direct_child_returns_none_when_no_significant_children() {
        let reader = mock(vec![entry(100, 1, "bash"), entry(200, 100, "sh")]);
        let result = direct_child_command(&reader, 100).unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn default_reader_returns_some_implementation() {
        let reader = default_reader();
        // Just verify it doesn't panic — the actual table contents are OS-dependent
        let _ = reader.read_process_table();
    }
}
