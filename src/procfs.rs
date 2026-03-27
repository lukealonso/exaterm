use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessEntry {
    pub pid: u32,
    pub ppid: u32,
    pub command: String,
    pub state: char,
}

pub fn format_process_tree(root_pid: u32) -> io::Result<String> {
    let entries = read_process_table("/proc")?;
    let Some(root) = entries.get(&root_pid) else {
        return Ok(format!("Process {root_pid} is no longer running."));
    };

    let mut children: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for entry in entries.values() {
        children.entry(entry.ppid).or_default().push(entry.pid);
    }
    for pids in children.values_mut() {
        pids.sort_unstable();
    }

    let mut lines = Vec::new();
    let mut visited = BTreeSet::new();
    write_tree(root.pid, 0, &entries, &children, &mut visited, &mut lines);
    Ok(lines.join("\n"))
}

pub fn dominant_child_command(root_pid: u32) -> io::Result<Option<String>> {
    let entries = read_process_table("/proc")?;
    Ok(dominant_child_command_from_entries(&entries, root_pid))
}

pub fn direct_child_command(root_pid: u32) -> io::Result<Option<String>> {
    let entries = read_process_table("/proc")?;
    Ok(direct_child_command_from_entries(&entries, root_pid))
}

fn dominant_child_command_from_entries(
    entries: &BTreeMap<u32, ProcessEntry>,
    root_pid: u32,
) -> Option<String> {
    let Some(root) = entries.get(&root_pid) else {
        return None;
    };

    let mut children: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for entry in entries.values() {
        children.entry(entry.ppid).or_default().push(entry.pid);
    }
    for pids in children.values_mut() {
        pids.sort_unstable();
    }

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

fn direct_child_command_from_entries(
    entries: &BTreeMap<u32, ProcessEntry>,
    root_pid: u32,
) -> Option<String> {
    let Some(root) = entries.get(&root_pid) else {
        return None;
    };

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

fn read_process_table(proc_root: &str) -> io::Result<BTreeMap<u32, ProcessEntry>> {
    let mut entries = BTreeMap::new();
    for item in fs::read_dir(proc_root)? {
        let item = item?;
        let name = item.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Ok(pid) = name.parse::<u32>() else {
            continue;
        };
        let stat_path = item.path().join("stat");
        let Ok(stat) = fs::read_to_string(&stat_path) else {
            continue;
        };
        if let Some(entry) = parse_stat_line(&stat) {
            entries.insert(pid, entry);
        }
    }
    Ok(entries)
}

fn parse_stat_line(stat: &str) -> Option<ProcessEntry> {
    let open = stat.find('(')?;
    let close = stat.rfind(')')?;
    let pid = stat[..open].trim().parse().ok()?;
    let command = stat[open + 1..close].to_string();
    let rest = stat[close + 1..].trim();
    let mut parts = rest.split_whitespace();
    let state = parts.next()?.chars().next()?;
    let ppid = parts.next()?.parse().ok()?;
    Some(ProcessEntry {
        pid,
        ppid,
        command,
        state,
    })
}

fn is_significant_command(command: &str) -> bool {
    !matches!(
        command,
        "bash" | "sh" | "dash" | "env" | "sleep" | "timeout" | "systemd" | "date"
    )
}

#[cfg(test)]
mod tests {
    use super::{
        direct_child_command_from_entries, dominant_child_command_from_entries, format_process_tree,
        parse_stat_line, ProcessEntry,
    };
    use std::fs;
    use std::path::PathBuf;

    #[test]
    fn parses_proc_stat_lines() {
        let entry = parse_stat_line("4242 (bash) S 100 200 300 0 -1 4194560 102 0 0 0 0 0 0 0 20 0 1 0 1 1 1 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0")
            .expect("line should parse");

        assert_eq!(
            entry,
            ProcessEntry {
                pid: 4242,
                ppid: 100,
                command: "bash".into(),
                state: 'S',
            }
        );
    }

    #[test]
    fn renders_a_small_process_tree() {
        let root = tempdir_path("exaterm-procfs-tree");
        fs::create_dir_all(root.join("101")).expect("root proc dir");
        fs::create_dir_all(root.join("202")).expect("child proc dir");
        fs::write(
            root.join("101").join("stat"),
            "101 (bash) S 1 1 1 0 -1 0 0 0 0 0 0 0 0 0 20 0 1 0 1 1 1 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0",
        )
        .expect("write root stat");
        fs::write(
            root.join("202").join("stat"),
            "202 (cargo) R 101 1 1 0 -1 0 0 0 0 0 0 0 0 0 20 0 1 0 1 1 1 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0",
        )
        .expect("write child stat");

        let rendered = super::read_process_table(root.to_str().expect("utf8 path"))
            .expect("table should read");
        assert_eq!(rendered.len(), 2);

        let output = {
            let backup = PathBuf::from("/proc");
            let _ = backup;
            let mut lines = Vec::new();
            let entries = rendered;
            let mut children = std::collections::BTreeMap::new();
            for entry in entries.values() {
                children.entry(entry.ppid).or_insert_with(Vec::new).push(entry.pid);
            }
            for pids in children.values_mut() {
                pids.sort_unstable();
            }
            let mut visited = std::collections::BTreeSet::new();
            super::write_tree(101, 0, &entries, &children, &mut visited, &mut lines);
            lines.join("\n")
        };

        assert!(output.contains("bash [S] pid=101"));
        assert!(output.contains("cargo [R] pid=202"));

        let _ = fs::remove_dir_all(root);
    }

    fn tempdir_path(prefix: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("{prefix}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        path
    }

    #[test]
    fn missing_root_process_is_reported() {
        let message = format_process_tree(u32::MAX).expect("function should not fail");
        assert!(message.contains("no longer running"));
    }

    #[test]
    fn picks_first_significant_child_command() {
        let root = tempdir_path("exaterm-procfs-dominant");
        fs::create_dir_all(root.join("101")).expect("root proc dir");
        fs::create_dir_all(root.join("202")).expect("shell child dir");
        fs::create_dir_all(root.join("303")).expect("tool child dir");
        fs::write(
            root.join("101").join("stat"),
            "101 (bash) S 1 1 1 0 -1 0 0 0 0 0 0 0 0 0 20 0 1 0 1 1 1 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0",
        )
        .expect("write root stat");
        fs::write(
            root.join("202").join("stat"),
            "202 (bash) S 101 1 1 0 -1 0 0 0 0 0 0 0 0 0 20 0 1 0 1 1 1 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0",
        )
        .expect("write shell stat");
        fs::write(
            root.join("303").join("stat"),
            "303 (cargo) R 202 1 1 0 -1 0 0 0 0 0 0 0 0 0 20 0 1 0 1 1 1 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0",
        )
        .expect("write tool stat");

        let entries = super::read_process_table(root.to_str().expect("utf8 path"))
            .expect("table should read");
        assert_eq!(
            dominant_child_command_from_entries(&entries, 101),
            Some("cargo".into())
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn picks_first_significant_direct_child_command() {
        let root = tempdir_path("exaterm-procfs-direct");
        fs::create_dir_all(root.join("101")).expect("root proc dir");
        fs::create_dir_all(root.join("202")).expect("wrapper child dir");
        fs::create_dir_all(root.join("303")).expect("agent child dir");
        fs::create_dir_all(root.join("404")).expect("nested tool dir");
        fs::write(
            root.join("101").join("stat"),
            "101 (bash) S 1 1 1 0 -1 0 0 0 0 0 0 0 0 0 20 0 1 0 1 1 1 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0",
        )
        .expect("write root stat");
        fs::write(
            root.join("202").join("stat"),
            "202 (bash) S 101 1 1 0 -1 0 0 0 0 0 0 0 0 0 20 0 1 0 1 1 1 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0",
        )
        .expect("write wrapper stat");
        fs::write(
            root.join("303").join("stat"),
            "303 (codex) S 101 1 1 0 -1 0 0 0 0 0 0 0 0 0 20 0 1 0 1 1 1 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0",
        )
        .expect("write direct agent stat");
        fs::write(
            root.join("404").join("stat"),
            "404 (cargo) R 202 1 1 0 -1 0 0 0 0 0 0 0 0 0 20 0 1 0 1 1 1 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0",
        )
        .expect("write nested tool stat");

        let entries = super::read_process_table(root.to_str().expect("utf8 path"))
            .expect("table should read");
        assert_eq!(
            direct_child_command_from_entries(&entries, 101),
            Some("codex".into())
        );

        let _ = fs::remove_dir_all(root);
    }
}
