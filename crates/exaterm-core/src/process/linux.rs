use super::{ProcessEntry, ProcessTableReader};
use std::collections::BTreeMap;
use std::fs;
use std::io;

pub struct LinuxProcessTableReader;

impl ProcessTableReader for LinuxProcessTableReader {
    fn read_process_table(&self) -> io::Result<BTreeMap<u32, ProcessEntry>> {
        read_process_table_from("/proc")
    }
}

pub(crate) fn read_process_table_from(proc_root: &str) -> io::Result<BTreeMap<u32, ProcessEntry>> {
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

pub(crate) fn parse_stat_line(stat: &str) -> Option<ProcessEntry> {
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
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn parses_proc_stat_lines() {
        let entry = parse_stat_line(
            "4242 (bash) S 100 200 300 0 -1 4194560 102 0 0 0 0 0 0 0 20 0 1 0 1 1 1 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0",
        )
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
    fn reads_process_table_from_temp_proc_dir() {
        let root = tempdir_path("exaterm-procfs-read");
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

        let entries =
            read_process_table_from(root.to_str().expect("utf8 path")).expect("table should read");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[&101].command, "bash");
        assert_eq!(entries[&202].command, "cargo");

        let _ = fs::remove_dir_all(root);
    }

    fn tempdir_path(prefix: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("{prefix}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        path
    }
}
