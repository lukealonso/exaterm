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

#[cfg(test)]
mod tests {
    use super::{format_process_tree, parse_stat_line, ProcessEntry};
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
}
