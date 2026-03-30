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

/// Translates a platform key event into terminal bytes or a UI action.
///
/// When `app_cursor` is true the arrow keys emit SS3 sequences (`\x1bO{A-D}`)
/// instead of CSI sequences (`\x1b[{A-D}`), matching DECCKM application mode.
pub fn key_event_to_action(input: &KeyInput, app_cursor: bool) -> KeyAction {
    let mods = &input.modifiers;

    // 1. Command modifier — clipboard or passthrough.
    if mods.command {
        return match (input.key_code, input.characters.as_deref()) {
            (8, Some("c")) => KeyAction::Copy,
            (9, Some("v")) => KeyAction::Paste,
            _ => KeyAction::None,
        };
    }

    // 2. Control modifier — map single ASCII letter to control character.
    if mods.control {
        if let Some(ref chars) = input.characters {
            if chars.len() == 1 {
                let ch = chars.as_bytes()[0];
                if ch.is_ascii_lowercase() {
                    let ctrl_byte = ch - b'a' + 1;
                    return KeyAction::Bytes(vec![ctrl_byte]);
                }
            }
        }
    }

    // 3. Arrow keys by key_code.
    if app_cursor {
        match input.key_code {
            126 => return KeyAction::Bytes(b"\x1bOA".to_vec()),
            125 => return KeyAction::Bytes(b"\x1bOB".to_vec()),
            124 => return KeyAction::Bytes(b"\x1bOC".to_vec()),
            123 => return KeyAction::Bytes(b"\x1bOD".to_vec()),
            _ => {}
        }
    } else {
        match input.key_code {
            126 => return KeyAction::Bytes(b"\x1b[A".to_vec()),
            125 => return KeyAction::Bytes(b"\x1b[B".to_vec()),
            124 => return KeyAction::Bytes(b"\x1b[C".to_vec()),
            123 => return KeyAction::Bytes(b"\x1b[D".to_vec()),
            _ => {}
        }
    }

    // 4. Special keys.
    match input.key_code {
        36 => return KeyAction::Bytes(vec![0x0d]),
        48 => return KeyAction::Bytes(vec![0x09]),
        53 => return KeyAction::Bytes(vec![0x1b]),
        51 => return KeyAction::Bytes(vec![0x7f]),
        _ => {}
    }

    // 5. Otherwise: pass through character bytes.
    match input.characters.as_deref() {
        Some(s) if !s.is_empty() => KeyAction::Bytes(s.as_bytes().to_vec()),
        _ => KeyAction::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: u16, chars: &str, mods: Modifiers) -> KeyInput {
        KeyInput {
            key_code: code,
            modifiers: mods,
            characters: Some(chars.into()),
        }
    }

    fn ctrl() -> Modifiers {
        Modifiers {
            control: true,
            ..Default::default()
        }
    }

    fn cmd() -> Modifiers {
        Modifiers {
            command: true,
            ..Default::default()
        }
    }

    #[test]
    fn regular_character_produces_utf8_bytes() {
        let action = key_event_to_action(&key(0, "a", Modifiers::default()), false);
        assert_eq!(action, KeyAction::Bytes(b"a".to_vec()));
    }

    #[test]
    fn ctrl_c_produces_etx() {
        let action = key_event_to_action(&key(8, "c", ctrl()), false);
        assert_eq!(action, KeyAction::Bytes(vec![0x03]));
    }

    #[test]
    fn ctrl_d_produces_eot() {
        let action = key_event_to_action(&key(2, "d", ctrl()), false);
        assert_eq!(action, KeyAction::Bytes(vec![0x04]));
    }

    #[test]
    fn ctrl_z_produces_sub() {
        let action = key_event_to_action(&key(6, "z", ctrl()), false);
        assert_eq!(action, KeyAction::Bytes(vec![0x1a]));
    }

    #[test]
    fn up_arrow_produces_csi_a() {
        // macOS key code 126 = up arrow
        let action = key_event_to_action(&key(126, "", Modifiers::default()), false);
        assert_eq!(action, KeyAction::Bytes(b"\x1b[A".to_vec()));
    }

    #[test]
    fn down_arrow_produces_csi_b() {
        let action = key_event_to_action(&key(125, "", Modifiers::default()), false);
        assert_eq!(action, KeyAction::Bytes(b"\x1b[B".to_vec()));
    }

    #[test]
    fn right_arrow_produces_csi_c() {
        let action = key_event_to_action(&key(124, "", Modifiers::default()), false);
        assert_eq!(action, KeyAction::Bytes(b"\x1b[C".to_vec()));
    }

    #[test]
    fn left_arrow_produces_csi_d() {
        let action = key_event_to_action(&key(123, "", Modifiers::default()), false);
        assert_eq!(action, KeyAction::Bytes(b"\x1b[D".to_vec()));
    }

    #[test]
    fn cmd_v_is_paste() {
        let action = key_event_to_action(&key(9, "v", cmd()), false);
        assert_eq!(action, KeyAction::Paste);
    }

    #[test]
    fn cmd_c_is_copy() {
        let action = key_event_to_action(&key(8, "c", cmd()), false);
        assert_eq!(action, KeyAction::Copy);
    }

    #[test]
    fn return_key_produces_cr() {
        let action = key_event_to_action(&key(36, "\r", Modifiers::default()), false);
        assert_eq!(action, KeyAction::Bytes(vec![0x0d]));
    }

    #[test]
    fn tab_key_produces_ht() {
        let action = key_event_to_action(&key(48, "\t", Modifiers::default()), false);
        assert_eq!(action, KeyAction::Bytes(vec![0x09]));
    }

    #[test]
    fn escape_key_produces_esc() {
        let action = key_event_to_action(&key(53, "\x1b", Modifiers::default()), false);
        assert_eq!(action, KeyAction::Bytes(vec![0x1b]));
    }

    #[test]
    fn backspace_produces_del() {
        // macOS key code 51 = delete (backspace)
        let action = key_event_to_action(&key(51, "\x7f", Modifiers::default()), false);
        assert_eq!(action, KeyAction::Bytes(vec![0x7f]));
    }

    #[test]
    fn up_arrow_app_cursor_produces_ss3_a() {
        let action = key_event_to_action(&key(126, "", Modifiers::default()), true);
        assert_eq!(action, KeyAction::Bytes(b"\x1bOA".to_vec()));
    }

    #[test]
    fn regular_character_unaffected_by_app_cursor() {
        let action = key_event_to_action(&key(0, "a", Modifiers::default()), true);
        assert_eq!(action, KeyAction::Bytes(b"a".to_vec()));
    }
}
