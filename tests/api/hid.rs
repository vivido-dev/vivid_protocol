//! Public-API tests for `vivid_protocol::hid`.
use vivid_protocol::hid::*;

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
