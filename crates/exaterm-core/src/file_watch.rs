use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::PathBuf;
use std::sync::mpsc;

pub struct RepoWatchHandle {
    _watcher: Option<RecommendedWatcher>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl RepoWatchHandle {
    pub fn stop(mut self) {
        // Drop the watcher first to close the sender, unblocking the receiver thread
        drop(self._watcher.take());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for RepoWatchHandle {
    fn drop(&mut self) {
        // Drop the watcher first to close the sender, unblocking the receiver thread
        drop(self._watcher.take());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn is_directory_event(kind: &EventKind) -> bool {
    matches!(
        kind,
        EventKind::Create(notify::event::CreateKind::Folder)
            | EventKind::Remove(notify::event::RemoveKind::Folder)
    )
}

pub fn spawn_repo_watch<F>(root: PathBuf, mut on_event: F) -> Result<RepoWatchHandle, String>
where
    F: FnMut(String) + Send + 'static,
{
    let (tx, rx) = mpsc::channel::<notify::Result<Event>>();

    let mut watcher = RecommendedWatcher::new(tx, notify::Config::default())
        .map_err(|e| format!("failed to create watcher: {e}"))?;

    watcher
        .watch(&root, RecursiveMode::Recursive)
        .map_err(|e| format!("failed to watch {}: {e}", root.display()))?;

    // Canonicalize so strip_prefix works when the OS reports canonical paths
    // (e.g., macOS resolves /tmp → /private/tmp in FSEvents)
    let original_root = root.clone();
    let watch_root = root.canonicalize().unwrap_or(root);
    let thread = std::thread::spawn(move || {
        for result in rx {
            let Ok(event) = result else {
                continue;
            };
            if !is_relevant_event(&event.kind) {
                continue;
            }
            // Skip directory-specific events — only report files
            if is_directory_event(&event.kind) {
                continue;
            }
            for path in &event.paths {
                if path
                    .components()
                    .any(|c| is_ignored_dir_name(c.as_os_str()))
                {
                    continue;
                }
                // Canonicalize the event path too, since on some platforms
                // the event path may use a different form than the watch root
                let canonical_path = path.canonicalize().unwrap_or_else(|_| path.clone());
                let relative = canonical_path
                    .strip_prefix(&watch_root)
                    .or_else(|_| canonical_path.strip_prefix(&original_root));
                if let Ok(relative) = relative {
                    let rel_str = relative.display().to_string();
                    if !rel_str.is_empty() {
                        on_event(rel_str);
                    }
                }
            }
        }
    });

    Ok(RepoWatchHandle {
        _watcher: Some(watcher),
        thread: Some(thread),
    })
}

fn is_relevant_event(kind: &EventKind) -> bool {
    matches!(
        kind,
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
    )
}

fn is_ignored_dir_name(name: &std::ffi::OsStr) -> bool {
    matches!(
        name.to_str(),
        Some(".git" | "node_modules" | "target" | ".venv" | ".direnv")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::mpsc as std_mpsc;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    #[test]
    fn watcher_reports_modified_file_relative_to_repo_root() {
        let root = tempdir_path("exaterm-notify-basic");
        fs::create_dir_all(root.join(".git")).expect("git dir");
        fs::create_dir_all(root.join("src")).expect("src dir");
        let file = root.join("src/main.rs");
        fs::write(&file, "fn main() {}\n").expect("seed file");

        let (tx, rx) = std_mpsc::channel();
        let handle = spawn_repo_watch(root.clone(), move |path| {
            let _ = tx.send(path);
        })
        .expect("spawn watcher");

        // FSEvents on macOS needs time to register the watch
        std::thread::sleep(Duration::from_millis(500));
        fs::write(&file, "fn main() { println!(\"hi\"); }\n").expect("rewrite file");
        let event = recv_event(&rx, Duration::from_secs(10));
        handle.stop();

        assert_eq!(event.as_deref(), Some("src/main.rs"));
    }

    #[test]
    fn watcher_tracks_files_in_new_nested_directories() {
        let root = tempdir_path("exaterm-notify-nested");
        fs::create_dir_all(root.join(".git")).expect("git dir");

        let (tx, rx) = std_mpsc::channel();
        let handle = spawn_repo_watch(root.clone(), move |path| {
            let _ = tx.send(path);
        })
        .expect("spawn watcher");

        std::thread::sleep(Duration::from_millis(500));
        let nested_dir = root.join("pkg/src");
        fs::create_dir_all(&nested_dir).expect("nested dir");
        let file = nested_dir.join("lib.rs");
        std::thread::sleep(Duration::from_millis(500));
        fs::write(&file, "pub fn x() {}\n").expect("write nested file");
        let event = recv_event(&rx, Duration::from_secs(10)).or_else(|| {
            fs::write(&file, "pub fn y() {}\n").expect("rewrite nested file");
            recv_event(&rx, Duration::from_secs(10))
        });
        handle.stop();

        assert_eq!(event.as_deref(), Some("pkg/src/lib.rs"));
    }

    #[test]
    fn watcher_ignores_dot_git_activity() {
        let root = tempdir_path("exaterm-notify-git-ignore");
        let git_dir = root.join(".git");
        fs::create_dir_all(&git_dir).expect("git dir");
        let head = git_dir.join("HEAD");
        fs::write(&head, "ref: refs/heads/main\n").expect("seed head");

        let (tx, rx) = std_mpsc::channel();
        let handle = spawn_repo_watch(root.clone(), move |path| {
            let _ = tx.send(path);
        })
        .expect("spawn watcher");

        std::thread::sleep(Duration::from_millis(100));
        fs::write(&head, "ref: refs/heads/dev\n").expect("rewrite head");
        let event = recv_event(&rx, Duration::from_millis(500));
        handle.stop();

        assert!(event.is_none(), "expected no .git event, got {event:?}");
    }

    #[test]
    fn watcher_ignores_target_directory_activity() {
        let root = tempdir_path("exaterm-notify-target-ignore");
        fs::create_dir_all(root.join(".git")).expect("git dir");
        let target_dir = root.join("target/debug");
        fs::create_dir_all(&target_dir).expect("target dir");

        let (tx, rx) = std_mpsc::channel();
        let handle = spawn_repo_watch(root.clone(), move |path| {
            let _ = tx.send(path);
        })
        .expect("spawn watcher");

        std::thread::sleep(Duration::from_millis(100));
        fs::write(target_dir.join("build.log"), "compiled\n").expect("write target log");
        let event = recv_event(&rx, Duration::from_millis(500));
        handle.stop();

        assert!(event.is_none(), "expected no target event, got {event:?}");
    }

    fn recv_event(rx: &std_mpsc::Receiver<String>, timeout: Duration) -> Option<String> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if let Ok(event) = rx.recv_timeout(Duration::from_millis(50)) {
                return Some(event);
            }
        }
        None
    }

    fn tempdir_path(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should be after epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("{prefix}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&path).expect("temp dir");
        path
    }
}
