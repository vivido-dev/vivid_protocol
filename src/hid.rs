//! The HID keyboard-page usages desktop §7 carries, keyed by the web's physical-key name.
//!
//! The canonical name for a physical key on the wire side is the browser `KeyboardEvent.code`
//! string — winit's `KeyCode` variants use the same names for the same reason — so one table
//! serves a native presenter mapping from `KeyCode` and a browser presenter mapping from
//! `code` without either needing to know the other exists.

/// The first usage desktop §7 accepts.
pub const KEYBOARD_PAGE_MIN: u16 = 0x04;
/// The last usage desktop §7 accepts.
pub const KEYBOARD_PAGE_MAX: u16 = 0xe7;

/// The HID keyboard-page usage for a named physical key, or `None` for one the page does not name.
///
/// Names are exactly the web `KeyboardEvent.code` values. A key with no keyboard-page usage maps
/// to nothing rather than being guessed at, because the producer's held-key set is balanced by
/// usage and a guessed usage would leak a held key.
pub fn usage(code: &str) -> Option<u16> {
    let usage = match code {
        "KeyA" => 0x04,
        "KeyB" => 0x05,
        "KeyC" => 0x06,
        "KeyD" => 0x07,
        "KeyE" => 0x08,
        "KeyF" => 0x09,
        "KeyG" => 0x0a,
        "KeyH" => 0x0b,
        "KeyI" => 0x0c,
        "KeyJ" => 0x0d,
        "KeyK" => 0x0e,
        "KeyL" => 0x0f,
        "KeyM" => 0x10,
        "KeyN" => 0x11,
        "KeyO" => 0x12,
        "KeyP" => 0x13,
        "KeyQ" => 0x14,
        "KeyR" => 0x15,
        "KeyS" => 0x16,
        "KeyT" => 0x17,
        "KeyU" => 0x18,
        "KeyV" => 0x19,
        "KeyW" => 0x1a,
        "KeyX" => 0x1b,
        "KeyY" => 0x1c,
        "KeyZ" => 0x1d,
        "Digit1" => 0x1e,
        "Digit2" => 0x1f,
        "Digit3" => 0x20,
        "Digit4" => 0x21,
        "Digit5" => 0x22,
        "Digit6" => 0x23,
        "Digit7" => 0x24,
        "Digit8" => 0x25,
        "Digit9" => 0x26,
        "Digit0" => 0x27,
        "Enter" => 0x28,
        "Escape" => 0x29,
        "Backspace" => 0x2a,
        "Tab" => 0x2b,
        "Space" => 0x2c,
        "Minus" => 0x2d,
        "Equal" => 0x2e,
        "BracketLeft" => 0x2f,
        "BracketRight" => 0x30,
        "Backslash" => 0x31,
        "Semicolon" => 0x33,
        "Quote" => 0x34,
        "Backquote" => 0x35,
        "Comma" => 0x36,
        "Period" => 0x37,
        "Slash" => 0x38,
        "CapsLock" => 0x39,
        "F1" => 0x3a,
        "F2" => 0x3b,
        "F3" => 0x3c,
        "F4" => 0x3d,
        "F5" => 0x3e,
        "F6" => 0x3f,
        "F7" => 0x40,
        "F8" => 0x41,
        "F9" => 0x42,
        "F10" => 0x43,
        "F11" => 0x44,
        "F12" => 0x45,
        "PrintScreen" => 0x46,
        "ScrollLock" => 0x47,
        "Pause" => 0x48,
        "Insert" => 0x49,
        "Home" => 0x4a,
        "PageUp" => 0x4b,
        "Delete" => 0x4c,
        "End" => 0x4d,
        "PageDown" => 0x4e,
        "ArrowRight" => 0x4f,
        "ArrowLeft" => 0x50,
        "ArrowDown" => 0x51,
        "ArrowUp" => 0x52,
        "NumLock" => 0x53,
        "NumpadDivide" => 0x54,
        "NumpadMultiply" => 0x55,
        "NumpadSubtract" => 0x56,
        "NumpadAdd" => 0x57,
        "NumpadEnter" => 0x58,
        "Numpad1" => 0x59,
        "Numpad2" => 0x5a,
        "Numpad3" => 0x5b,
        "Numpad4" => 0x5c,
        "Numpad5" => 0x5d,
        "Numpad6" => 0x5e,
        "Numpad7" => 0x5f,
        "Numpad8" => 0x60,
        "Numpad9" => 0x61,
        "Numpad0" => 0x62,
        "NumpadDecimal" => 0x63,
        "ContextMenu" => 0x65,
        "ControlLeft" => 0xe0,
        "ShiftLeft" => 0xe1,
        "AltLeft" => 0xe2,
        "MetaLeft" | "SuperLeft" | "OSLeft" => 0xe3,
        "ControlRight" => 0xe4,
        "ShiftRight" => 0xe5,
        "AltRight" => 0xe6,
        "MetaRight" | "SuperRight" | "OSRight" => 0xe7,
        _ => return None,
    };
    Some(usage)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NAMES: &[&str] = &[
        "KeyA",
        "KeyZ",
        "Digit1",
        "Digit0",
        "Enter",
        "Escape",
        "Backspace",
        "Tab",
        "Space",
        "Minus",
        "Equal",
        "BracketLeft",
        "BracketRight",
        "Backslash",
        "Semicolon",
        "Quote",
        "Backquote",
        "Comma",
        "Period",
        "Slash",
        "CapsLock",
        "F1",
        "F12",
        "PrintScreen",
        "ScrollLock",
        "Pause",
        "Insert",
        "Home",
        "PageUp",
        "Delete",
        "End",
        "PageDown",
        "ArrowRight",
        "ArrowLeft",
        "ArrowDown",
        "ArrowUp",
        "NumLock",
        "NumpadDivide",
        "NumpadMultiply",
        "NumpadSubtract",
        "NumpadAdd",
        "NumpadEnter",
        "Numpad0",
        "Numpad9",
        "NumpadDecimal",
        "ContextMenu",
        "ControlLeft",
        "ShiftLeft",
        "AltLeft",
        "ControlRight",
        "ShiftRight",
        "AltRight",
    ];

    #[test]
    fn every_named_key_stays_on_the_keyboard_page() {
        // Desktop §7 accepts `0x04..=0xe7`; a mapping outside it would be rejected on the wire.
        for name in NAMES {
            let usage = usage(name).expect("a listed key maps");
            assert!(
                (KEYBOARD_PAGE_MIN..=KEYBOARD_PAGE_MAX).contains(&usage),
                "{name} mapped outside the keyboard page"
            );
        }
    }

    #[test]
    fn the_mapping_is_injective_across_distinct_physical_keys() {
        // Two physical keys sharing a usage would unbalance the producer's held-key set. The OS
        // key aliases are deliberately excluded: they share a usage by design.
        let mut seen = Vec::new();
        for name in NAMES {
            let usage = usage(name).expect("a listed key maps");
            assert!(!seen.contains(&usage), "{name} reuses usage {usage:#04x}");
            seen.push(usage);
        }
    }

    #[test]
    fn the_os_key_aliases_agree_on_one_usage() {
        // Browsers disagree on the name of the platform key; all of them are the left/right GUI
        // usages, matching what the native presenter emits for Super.
        assert_eq!(usage("MetaLeft"), usage("SuperLeft"));
        assert_eq!(usage("OSLeft"), usage("SuperLeft"));
        assert_eq!(usage("MetaRight"), usage("SuperRight"));
        assert_eq!(usage("OSRight"), usage("SuperRight"));
        assert_ne!(usage("SuperLeft"), usage("SuperRight"));
    }

    #[test]
    fn an_unnamed_key_maps_to_nothing() {
        assert!(usage("Fn").is_none());
        assert!(usage("").is_none());
        assert!(usage("keyA").is_none(), "names are case-sensitive");
    }
}
