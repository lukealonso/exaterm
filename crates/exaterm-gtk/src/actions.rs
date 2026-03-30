use crate::ui::{AppContext, NudgeCacheEntry, refresh_runtime_and_cards, update_nudge_widgets};
use exaterm_types::model::SessionId;
use exaterm_types::proto::ClientMessage;
use std::fs::File;
use std::io::Write;
use std::os::unix::net::UnixStream;
use std::rc::Rc;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

pub(crate) fn toggle_auto_nudge(context: &Rc<AppContext>, session_id: SessionId) {
    let enabled = {
        let mut cache = context.nudge_cache.borrow_mut();
        let entry = cache.entry(session_id).or_insert_with(NudgeCacheEntry::new);
        entry.enabled = !entry.enabled;
        if !entry.enabled {
            entry.in_flight = false;
            entry.requested_signature = None;
            entry.hovered = false;
        }
        entry.enabled
    };
    if let Some(beachhead) = context.beachhead.as_ref() {
        let _ = beachhead.commands().send(ClientMessage::ToggleAutoNudge {
            session_id,
            enabled,
        });
    }
    update_nudge_widgets(context, session_id);
    if enabled {
        refresh_runtime_and_cards(context);
    }
}

pub(crate) fn set_auto_nudge_hover(context: &Rc<AppContext>, session_id: SessionId, hovered: bool) {
    let changed = {
        let mut cache = context.nudge_cache.borrow_mut();
        let entry = cache.entry(session_id).or_insert_with(NudgeCacheEntry::new);
        let changed = entry.hovered != hovered;
        entry.hovered = hovered;
        changed
    };
    if changed {
        update_nudge_widgets(context, session_id);
    }
}

pub(crate) fn insert_terminal_number(
    context: &Rc<AppContext>,
    source_session: SessionId,
    one_based: bool,
) {
    let ordered_session_ids = context
        .state
        .borrow()
        .sessions()
        .iter()
        .map(|session| session.id)
        .collect::<Vec<_>>();
    if ordered_session_ids.is_empty() {
        return;
    }

    let session_numbers = ordered_session_ids
        .iter()
        .enumerate()
        .map(|(index, session_id)| {
            let number = if one_based { index + 1 } else { index };
            (*session_id, number.to_string())
        })
        .collect::<Vec<_>>();

    if context.sync_inputs_enabled.load(Ordering::Relaxed) {
        for (session_id, text) in session_numbers {
            let _ = send_session_input_text(context, session_id, &text);
        }
        return;
    }

    if let Some((session_id, text)) = session_numbers
        .into_iter()
        .find(|(session_id, _)| *session_id == source_session)
    {
        let _ = send_session_input_text(context, session_id, &text);
    }
}

pub(crate) fn send_runtime_input_line(
    context: &Rc<AppContext>,
    session_id: SessionId,
    line: &str,
) -> std::io::Result<()> {
    let mut bytes = line.as_bytes().to_vec();
    bytes.push(b'\n');
    send_session_input_bytes(context, session_id, &bytes)
}

pub(crate) fn send_session_input_text(
    context: &Rc<AppContext>,
    session_id: SessionId,
    text: &str,
) -> std::io::Result<()> {
    send_session_input_bytes(context, session_id, text.as_bytes())
}

pub(crate) fn send_session_input_bytes(
    context: &Rc<AppContext>,
    session_id: SessionId,
    bytes: &[u8],
) -> std::io::Result<()> {
    if let Some(writer) = context
        .raw_input_writers
        .lock()
        .ok()
        .and_then(|writers| writers.get(&session_id).cloned())
    {
        return write_raw_session_input(&writer, bytes);
    }

    let writer = {
        let runtimes = context.runtimes.borrow();
        runtimes
            .get(&session_id)
            .and_then(|runtime| runtime.input_writer.as_ref().cloned())
    }
    .ok_or_else(|| std::io::Error::other("session runtime input writer missing"))?;

    write_runtime_input(&writer, bytes)
}

fn write_raw_session_input(writer: &Arc<Mutex<UnixStream>>, bytes: &[u8]) -> std::io::Result<()> {
    let mut writer = writer
        .lock()
        .map_err(|_| std::io::Error::other("raw session writer lock poisoned"))?;
    writer.write_all(bytes)
}

fn write_runtime_input(writer: &Arc<Mutex<File>>, bytes: &[u8]) -> std::io::Result<()> {
    let mut writer = writer
        .lock()
        .map_err(|_| std::io::Error::other("runtime input writer lock poisoned"))?;
    let mut offset = 0usize;

    while offset < bytes.len() {
        match writer.write(&bytes[offset..]) {
            Ok(0) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::WriteZero,
                    "short write to runtime input",
                ));
            }
            Ok(n) => offset += n,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                let mut fds = [libc::pollfd {
                    fd: std::os::fd::AsRawFd::as_raw_fd(&*writer),
                    events: libc::POLLOUT,
                    revents: 0,
                }];
                let poll_result =
                    unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, 1000) };
                if poll_result < 0 {
                    let poll_error = std::io::Error::last_os_error();
                    if poll_error.kind() == std::io::ErrorKind::Interrupted {
                        continue;
                    }
                    return Err(poll_error);
                }
                if poll_result == 0 {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "timed out waiting for runtime input writer",
                    ));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }

    Ok(())
}
