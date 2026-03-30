use exaterm_types::model::SessionId;
use exaterm_types::proto::ClientMessage;
use objc2::rc::Retained;
use objc2::runtime::NSObject;
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::NSApplicationDelegate;
use objc2_foundation::NSObjectProtocol;
use std::cell::RefCell;
use std::sync::mpsc;

thread_local! {
    static CMD_SENDER: RefCell<Option<mpsc::Sender<ClientMessage>>> = const { RefCell::new(None) };
    static FIRST_SESSION: RefCell<Option<SessionId>> = const { RefCell::new(None) };
}

/// Store the command sender so the AppDelegate can send messages to the daemon.
pub fn set_command_sender(sender: mpsc::Sender<ClientMessage>) {
    CMD_SENDER.with(|s| *s.borrow_mut() = Some(sender));
}

/// Update the first session ID (needed for AddTerminals).
pub fn set_first_session(id: Option<SessionId>) {
    FIRST_SESSION.with(|s| *s.borrow_mut() = id);
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
    }
);

impl AppDelegate {
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        unsafe { msg_send![Self::alloc(mtm), init] }
    }
}
