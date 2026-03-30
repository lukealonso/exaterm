/// Keyboard input from the platform (macOS NSEvent fields).
#[derive(Debug, Clone)]
pub struct KeyInput {
    pub key_code: u16,
    pub modifiers: Modifiers,
    pub characters: Option<String>,
}

/// Modifier key state.
#[derive(Debug, Clone, Copy, Default)]
pub struct Modifiers {
    pub shift: bool,
    pub control: bool,
    pub option: bool,
    pub command: bool,
}

/// Result of mapping a key event — either terminal bytes or a UI-level action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyAction {
    /// Bytes to send to the terminal PTY.
    Bytes(Vec<u8>),
    /// Paste from clipboard (Cmd+V).
    Paste,
    /// Copy selection (Cmd+C).
    Copy,
    /// No action for this key combination.
    None,
}
