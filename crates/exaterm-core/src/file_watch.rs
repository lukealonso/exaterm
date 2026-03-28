use std::collections::BTreeMap;
use std::ffi::{CString, OsStr};
use std::fs::{self, File};
use std::io::Read;
use std::os::fd::{FromRawFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

const WATCH_MASK: u32 = libc::IN_CLOSE_WRITE
    | libc::IN_CREATE
    | libc::IN_MOVED_TO
    | libc::IN_DELETE
    | libc::IN_ATTRIB
    | libc::IN_DELETE_SELF
    | libc::IN_MOVE_SELF;

pub struct RepoWatchHandle {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl RepoWatchHandle {
    pub fn stop(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub fn spawn_repo_watch<F>(root: PathBuf, mut on_event: F) -> Result<RepoWatchHandle, String>
where
    F: FnMut(String) + Send + 'static,
{
    let fd = unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) };
    if fd < 0 {
        return Err(format!(
            "failed to initialize inotify: {}",
            std::io::Error::last_os_error()
        ));
    }
    let file = unsafe { File::from_raw_fd(fd) };
    let mut watch_roots = BTreeMap::new();
    add_recursive_watches(fd, &root, &mut watch_roots)?;

    let stop = Arc::new(AtomicBool::new(false));
    let stop_thread = stop.clone();
    let thread = thread::spawn(move || {
        let mut file = file;
        let mut buf = vec![0u8; 64 * 1024];

        while !stop_thread.load(Ordering::Relaxed) {
            let mut pollfd = libc::pollfd {
                fd,
                events: libc::POLLIN,
                revents: 0,
            };
            let poll_result = unsafe { libc::poll(&mut pollfd, 1, 250) };
            if poll_result < 0 {
                let error = std::io::Error::last_os_error();
                if error.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                break;
            }
            if poll_result == 0 || (pollfd.revents & libc::POLLIN) == 0 {
                continue;
            }

            loop {
                match file.read(&mut buf) {
                    Ok(0) => return,
                    Ok(n) => process_events(&root, &buf[..n], fd, &mut watch_roots, &mut on_event),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => return,
                }
            }
        }
    });

    Ok(RepoWatchHandle {
        stop,
        thread: Some(thread),
    })
}

fn process_events<F>(
    root: &Path,
    buf: &[u8],
    fd: RawFd,
    watch_roots: &mut BTreeMap<i32, PathBuf>,
    on_event: &mut F,
) where
    F: FnMut(String),
{
    let mut offset = 0usize;
    while offset + std::mem::size_of::<libc::inotify_event>() <= buf.len() {
        let event = unsafe {
            &*(buf[offset..].as_ptr() as *const libc::inotify_event)
        };
        let event_size = std::mem::size_of::<libc::inotify_event>() + event.len as usize;
        if offset + event_size > buf.len() {
            break;
        }

        let dir_path = watch_roots.get(&event.wd).cloned();
        let name_bytes =
            &buf[offset + std::mem::size_of::<libc::inotify_event>()..offset + event_size];
        let name_len = name_bytes
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(name_bytes.len());
        let name = OsStr::from_bytes(&name_bytes[..name_len]);
        let is_dir = event.mask & libc::IN_ISDIR != 0;

        if event.mask & libc::IN_IGNORED != 0 {
            watch_roots.remove(&event.wd);
            offset += event_size;
            continue;
        }

        let Some(dir_path) = dir_path else {
            offset += event_size;
            continue;
        };
        let path = if name.is_empty() {
            dir_path.clone()
        } else {
            dir_path.join(name)
        };

        if is_dir {
            if is_ignored_dir_name(name) {
                offset += event_size;
                continue;
            }
            if event.mask & (libc::IN_CREATE | libc::IN_MOVED_TO) != 0 {
                let _ = add_recursive_watches(fd, &path, watch_roots);
            }
            if event.mask & (libc::IN_DELETE_SELF | libc::IN_MOVE_SELF) != 0 {
                watch_roots.remove(&event.wd);
            }
            offset += event_size;
            continue;
        }

        if is_ignored_dir_name(name) {
            offset += event_size;
            continue;
        }

        if event.mask & (libc::IN_CLOSE_WRITE | libc::IN_CREATE | libc::IN_MOVED_TO | libc::IN_DELETE | libc::IN_ATTRIB) != 0 {
            if let Ok(relative) = path.strip_prefix(root) {
                on_event(relative.display().to_string());
            }
        }

        offset += event_size;
    }
}

fn add_recursive_watches(
    fd: RawFd,
    root: &Path,
    watch_roots: &mut BTreeMap<i32, PathBuf>,
) -> Result<(), String> {
    add_watch(fd, root, watch_roots)?;
    let Ok(entries) = fs::read_dir(root) else {
        return Ok(());
    };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() || !file_type.is_dir() {
            continue;
        }
        if is_ignored_dir_name(&entry.file_name()) {
            continue;
        }
        add_recursive_watches(fd, &entry.path(), watch_roots)?;
    }
    Ok(())
}

fn add_watch(fd: RawFd, path: &Path, watch_roots: &mut BTreeMap<i32, PathBuf>) -> Result<(), String> {
    let c_path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| format!("watch path contains interior null byte: {}", path.display()))?;
    let wd = unsafe { libc::inotify_add_watch(fd, c_path.as_ptr(), WATCH_MASK) };
    if wd < 0 {
        return Err(format!(
            "failed to add inotify watch for {}: {}",
            path.display(),
            std::io::Error::last_os_error()
        ));
    }
    watch_roots.insert(wd, path.to_path_buf());
    Ok(())
}

fn is_ignored_dir_name(name: &OsStr) -> bool {
    matches!(
        name.to_str(),
        Some(".git" | "node_modules" | "target" | ".venv" | ".direnv")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    #[test]
    fn watcher_reports_modified_file_relative_to_repo_root() {
        let root = tempdir_path("exaterm-inotify-basic");
        fs::create_dir_all(root.join(".git")).expect("git dir");
        fs::create_dir_all(root.join("src")).expect("src dir");
        let file = root.join("src/main.rs");
        fs::write(&file, "fn main() {}\n").expect("seed file");

        let (tx, rx) = mpsc::channel();
        let handle = spawn_repo_watch(root.clone(), move |path| {
            let _ = tx.send(path);
        })
        .expect("spawn watcher");

        fs::write(&file, "fn main() { println!(\"hi\"); }\n").expect("rewrite file");
        let event = recv_event(&rx, Duration::from_secs(2));
        handle.stop();

        assert_eq!(event.as_deref(), Some("src/main.rs"));
    }

    #[test]
    fn watcher_tracks_files_in_new_nested_directories() {
        let root = tempdir_path("exaterm-inotify-nested");
        fs::create_dir_all(root.join(".git")).expect("git dir");

        let (tx, rx) = mpsc::channel();
        let handle = spawn_repo_watch(root.clone(), move |path| {
            let _ = tx.send(path);
        })
        .expect("spawn watcher");

        let nested_dir = root.join("pkg/src");
        fs::create_dir_all(&nested_dir).expect("nested dir");
        let file = nested_dir.join("lib.rs");
        std::thread::sleep(Duration::from_millis(100));
        fs::write(&file, "pub fn x() {}\n").expect("write nested file");
        let event = recv_event(&rx, Duration::from_secs(1)).or_else(|| {
            fs::write(&file, "pub fn y() {}\n").expect("rewrite nested file");
            recv_event(&rx, Duration::from_secs(1))
        });
        handle.stop();

        assert_eq!(event.as_deref(), Some("pkg/src/lib.rs"));
    }

    #[test]
    fn watcher_ignores_dot_git_activity() {
        let root = tempdir_path("exaterm-inotify-git-ignore");
        let git_dir = root.join(".git");
        fs::create_dir_all(&git_dir).expect("git dir");
        let head = git_dir.join("HEAD");
        fs::write(&head, "ref: refs/heads/main\n").expect("seed head");

        let (tx, rx) = mpsc::channel();
        let handle = spawn_repo_watch(root.clone(), move |path| {
            let _ = tx.send(path);
        })
        .expect("spawn watcher");

        fs::write(&head, "ref: refs/heads/dev\n").expect("rewrite head");
        let event = recv_event(&rx, Duration::from_millis(300));
        handle.stop();

        assert!(event.is_none(), "expected no .git event, got {event:?}");
    }

    #[test]
    fn watcher_ignores_target_directory_activity() {
        let root = tempdir_path("exaterm-inotify-target-ignore");
        fs::create_dir_all(root.join(".git")).expect("git dir");
        let target_dir = root.join("target/debug");
        fs::create_dir_all(&target_dir).expect("target dir");

        let (tx, rx) = mpsc::channel();
        let handle = spawn_repo_watch(root.clone(), move |path| {
            let _ = tx.send(path);
        })
        .expect("spawn watcher");

        fs::write(target_dir.join("build.log"), "compiled\n").expect("write target log");
        let event = recv_event(&rx, Duration::from_millis(300));
        handle.stop();

        assert!(event.is_none(), "expected no target event, got {event:?}");
    }

    fn recv_event(rx: &mpsc::Receiver<String>, timeout: Duration) -> Option<String> {
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
