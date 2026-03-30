use exaterm_types::model::SessionId;
use exaterm_types::proto::ClientMessage;
use objc2::rc::Retained;
use objc2::runtime::NSObject;
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSAlertSecondButtonReturn, NSAlertStyle, NSApplication,
    NSApplicationDelegate, NSApplicationTerminateReply,
};
use objc2_foundation::NSObjectProtocol;
use objc2_foundation::ns_string;
use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;

thread_local! {
    static CMD_SENDER: RefCell<Option<mpsc::Sender<ClientMessage>>> = const { RefCell::new(None) };
    static FIRST_SESSION: RefCell<Option<SessionId>> = const { RefCell::new(None) };
    static SELECTED_SESSION: RefCell<Option<SessionId>> = const { RefCell::new(None) };
    static SELECTED_AUTO_NUDGE: RefCell<bool> = const { RefCell::new(false) };
    static HAS_SESSIONS: RefCell<bool> = const { RefCell::new(false) };
    static SYNC_INPUTS: RefCell<Option<Arc<AtomicBool>>> = const { RefCell::new(None) };
}

/// Store the command sender so the AppDelegate can send messages to the daemon.
pub fn set_command_sender(sender: mpsc::Sender<ClientMessage>) {
    CMD_SENDER.with(|s| *s.borrow_mut() = Some(sender));
}

/// Update the first session ID (needed for AddTerminals).
pub fn set_first_session(id: Option<SessionId>) {
    FIRST_SESSION.with(|s| *s.borrow_mut() = id);
}

pub fn set_selected_session(id: Option<SessionId>) {
    SELECTED_SESSION.with(|s| *s.borrow_mut() = id);
}

pub fn set_selected_auto_nudge(enabled: bool) {
    SELECTED_AUTO_NUDGE.with(|s| *s.borrow_mut() = enabled);
}

pub fn set_has_sessions(has_sessions: bool) {
    HAS_SESSIONS.with(|s| *s.borrow_mut() = has_sessions);
}

pub fn set_sync_inputs_state(sync_inputs: Arc<AtomicBool>) {
    SYNC_INPUTS.with(|s| *s.borrow_mut() = Some(sync_inputs));
}

pub fn send_add_terminals() {
    let source = FIRST_SESSION.with(|s| *s.borrow());
    if let Some(source_session) = source {
        CMD_SENDER.with(|s| {
            if let Some(sender) = s.borrow().as_ref() {
                let _ = sender.send(ClientMessage::AddTerminals { source_session });
            }
        });
    }
}

fn toggle_auto_nudge() {
    let session = SELECTED_SESSION.with(|s| *s.borrow());
    let enabled = !SELECTED_AUTO_NUDGE.with(|s| *s.borrow());
    if let Some(session_id) = session {
        CMD_SENDER.with(|s| {
            if let Some(sender) = s.borrow().as_ref() {
                let _ = sender.send(ClientMessage::ToggleAutoNudge {
                    session_id,
                    enabled,
                });
            }
        });
        set_selected_auto_nudge(enabled);
    }
}

fn toggle_sync_inputs() {
    SYNC_INPUTS.with(|s| {
        if let Some(state) = s.borrow().as_ref() {
            let next = !state.load(Ordering::Relaxed);
            state.store(next, Ordering::Relaxed);
        }
    });
}

fn confirm_termination() -> NSApplicationTerminateReply {
    let has_sessions = HAS_SESSIONS.with(|s| *s.borrow());
    if !has_sessions {
        return NSApplicationTerminateReply::TerminateNow;
    }

    let alert = NSAlert::new(MainThreadMarker::new().expect("main thread"));
    alert.setMessageText(ns_string!("Keep terminals alive?"));
    alert.setInformativeText(ns_string!(
        "Closing Exaterm can leave the local or remote beachhead running so you can reconnect to the same live terminals later."
    ));
    alert.setAlertStyle(NSAlertStyle::Warning);
    alert.addButtonWithTitle(ns_string!("Keep Alive"));
    alert.addButtonWithTitle(ns_string!("Terminate"));
    alert.addButtonWithTitle(ns_string!("Cancel"));

    match alert.runModal() {
        value if value == NSAlertFirstButtonReturn => {
            CMD_SENDER.with(|s| {
                if let Some(sender) = s.borrow().as_ref() {
                    let _ = sender.send(ClientMessage::DetachClient { keep_alive: true });
                }
            });
            NSApplicationTerminateReply::TerminateNow
        }
        value if value == NSAlertSecondButtonReturn => {
            CMD_SENDER.with(|s| {
                if let Some(sender) = s.borrow().as_ref() {
                    let _ = sender.send(ClientMessage::DetachClient { keep_alive: false });
                }
            });
            NSApplicationTerminateReply::TerminateNow
        }
        _ => NSApplicationTerminateReply::TerminateCancel,
    }
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and we don't implement Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "ExatermAppDelegate"]
    pub struct AppDelegate;

    unsafe impl NSObjectProtocol for AppDelegate {}

    unsafe impl NSApplicationDelegate for AppDelegate {}

    impl AppDelegate {
        #[unsafe(method(newShell:))]
        fn _new_shell(&self, _sender: Option<&NSObject>) {
            send_add_terminals();
        }

        #[unsafe(method(toggleAutoNudge:))]
        fn _toggle_auto_nudge(&self, _sender: Option<&NSObject>) {
            toggle_auto_nudge();
        }

        #[unsafe(method(toggleSyncInputs:))]
        fn _toggle_sync_inputs(&self, _sender: Option<&NSObject>) {
            toggle_sync_inputs();
        }

        #[unsafe(method(applicationShouldTerminateAfterLastWindowClosed:))]
        fn application_should_terminate_after_last_window_closed(
            &self,
            _sender: &NSApplication,
        ) -> bool {
            true
        }

        #[unsafe(method(applicationShouldTerminate:))]
        fn application_should_terminate(
            &self,
            _sender: &NSApplication,
        ) -> NSApplicationTerminateReply {
            confirm_termination()
        }
    }
);

impl AppDelegate {
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        unsafe { msg_send![Self::alloc(mtm), init] }
    }
}
