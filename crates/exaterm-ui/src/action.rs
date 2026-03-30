use exaterm_types::model::{SessionId, SessionLaunch};

/// Platform-independent UI actions that both GTK and macOS clients emit.
#[derive(Debug)]
pub enum UiAction {
    SelectSession(SessionId),
    EnterFocusMode(SessionId),
    ReturnToBattlefield,
    SendTerminalInput(SessionId, Vec<u8>),
    ResizeTerminal(SessionId, u16, u16),
    RequestNewSession(SessionLaunch),
    CopySelection(SessionId),
    PasteClipboard(SessionId),
}
