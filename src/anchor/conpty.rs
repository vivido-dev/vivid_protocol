//! Bounded recovery of the printable anchor envelope from ConPTY's VT output.
//!
//! At the bottom of the screen ConPTY splits a soft-wrapped write with CR LF and sometimes
//! CUP and a repaint of the last glyph. These bytes describe the marker's own glyphs,
//! not application terminal movement.
//! Remove only those known transport insertions, then apply the ordinary marker grammar.
//! The presenter must still authenticate the recovered body and enforce replay protection.

use super::parse_conpty_marker;

/// Bounds raw transport bytes, including wrap controls. The canonical envelope remains 99 bytes.
pub const MAX_TRANSPORT_BYTES: usize = 2048;
const BODY_BYTES: usize = 10 + 22 + 1 + 16 + 1 + 16 + 1 + 22;
const SUFFIX: &[u8] = b";VIVID-END";
const ENVELOPE_BYTES: usize = BODY_BYTES + SUFFIX.len();

#[derive(Debug, PartialEq, Eq)]
pub enum Scan {
    /// Exactly `consumed` raw bytes belong to this zero-width envelope.
    Complete {
        consumed: usize,
        body: String,
    },
    Incomplete,
    Invalid,
}

/// Inspect a candidate starting at `V`, including a prefix or suffix split by a ConPTY wrap.
/// Invalid input must be preserved byte-for-byte by the caller; never normalize arbitrary VT.
pub fn scan(bytes: &[u8]) -> Scan {
    let input = &bytes[..bytes.len().min(MAX_TRANSPORT_BYTES)];
    let mut canonical = [0; ENVELOPE_BYTES];
    let mut length = 0;
    let mut cursor = 0;
    while cursor < input.len() {
        if length != 0 && input[cursor] == b'\r' {
            let Some(&next) = input.get(cursor + 1) else {
                return incomplete(bytes);
            };
            if next != b'\n' {
                return Scan::Invalid;
            }
            cursor += 2;
            if input.get(cursor) == Some(&b'\x1b') {
                match wrap_cursor(&input[cursor..]) {
                    Cursor::Complete(consumed) => {
                        cursor += consumed;
                        let Some(&repaint) = input.get(cursor) else {
                            return incomplete(bytes);
                        };
                        // ConPTY repaints the previous row's last cell to recreate soft wrap.
                        if repaint != canonical[length - 1] {
                            return Scan::Invalid;
                        }
                        cursor += 1;
                    }
                    Cursor::Incomplete => return incomplete(bytes),
                    Cursor::Invalid => return Scan::Invalid,
                }
            }
            // A read may end after CR LF, before the optional CUP. Re-scan this bounded
            // candidate once more bytes arrive rather than committing a partial transform.
            if cursor == input.len() {
                return incomplete(bytes);
            }
        }
        let byte = input[cursor];
        if !expected_byte(length, byte) {
            return Scan::Invalid;
        }
        canonical[length] = byte;
        length += 1;
        cursor += 1;
        if length == ENVELOPE_BYTES {
            let envelope = std::str::from_utf8(&canonical).expect("marker grammar is ASCII");
            return if parse_conpty_marker(envelope).is_ok() {
                Scan::Complete {
                    consumed: cursor,
                    body: envelope[..BODY_BYTES].to_owned(),
                }
            } else {
                Scan::Invalid
            };
        }
    }
    incomplete(bytes)
}

fn incomplete(bytes: &[u8]) -> Scan {
    if bytes.len() >= MAX_TRANSPORT_BYTES {
        Scan::Invalid
    } else {
        Scan::Incomplete
    }
}

fn expected_byte(index: usize, byte: u8) -> bool {
    match index {
        0..10 => byte == b"VIVID;3;A;"[index],
        10..32 | 67..89 => byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'),
        32 | 49 | 66 => byte == b';',
        33..49 | 50..66 => byte.is_ascii_hexdigit(),
        _ => byte == SUFFIX[index - BODY_BYTES],
    }
}

enum Cursor {
    Complete(usize),
    Incomplete,
    Invalid,
}

/// Only the absolute row/column CUP emitted after ConPTY's wrap is transport syntax.
/// Bound decimal parameters to the positive signed-16-bit ConPTY geometry range.
fn wrap_cursor(bytes: &[u8]) -> Cursor {
    if bytes.len() < 2 {
        return Cursor::Incomplete;
    }
    if !bytes.starts_with(b"\x1b[") {
        return Cursor::Invalid;
    }
    let mut cursor = 2;
    for delimiter in *b";H" {
        let start = cursor;
        let mut value = 0_u32;
        while let Some(byte) = bytes.get(cursor).filter(|byte| byte.is_ascii_digit()) {
            if cursor - start == 5 {
                return Cursor::Invalid;
            }
            value = value * 10 + u32::from(byte - b'0');
            cursor += 1;
        }
        let Some(&byte) = bytes.get(cursor) else {
            return Cursor::Incomplete;
        };
        if cursor == start || value == 0 || value > i16::MAX as u32 || byte != delimiter {
            return Cursor::Invalid;
        }
        cursor += 1;
    }
    Cursor::Complete(cursor)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::anchor::{AnchorKey, encode_conpty_marker, parse_marker, verify_marker};

    #[test]
    fn wraps_in_every_field_prefix_and_suffix_preserve_authentication() {
        let key = AnchorKey::new([42; 32]);
        let other_key = AnchorKey::new([43; 32]);
        let marker = encode_conpty_marker(&key, &[24; 16], 3, 7).unwrap();
        assert_eq!(marker.len(), ENVELOPE_BYTES);
        for insertion in 1..marker.len() {
            for cup in [false, true] {
                let wrap = if cup {
                    format!(
                        "\r\n\x1b[23;40H{}",
                        char::from(marker.as_bytes()[insertion - 1])
                    )
                } else {
                    "\r\n".to_owned()
                };
                let input = format!("{}{wrap}{}", &marker[..insertion], &marker[insertion..]);
                for end in 0..input.len() {
                    assert_eq!(scan(&input.as_bytes()[..end]), Scan::Incomplete);
                }
                let Scan::Complete { consumed, body } = scan(input.as_bytes()) else {
                    panic!("wrapped marker was rejected at {insertion}");
                };
                assert_eq!(consumed, input.len());
                let decoded = parse_marker(&body).unwrap();
                assert!(verify_marker(&key, &decoded));
                assert!(!verify_marker(&other_key, &decoded));
            }
        }
    }

    #[test]
    fn unrelated_controls_and_oversized_cursor_parameters_are_rejected() {
        let marker = encode_conpty_marker(&AnchorKey::new([42; 32]), &[24; 16], 3, 7).unwrap();
        for control in [
            "\n",
            "\rX",
            "\x1b[23;40H",
            "\r\n\x1b[2J",
            "\r\n\x1b[0;1H",
            "\r\n\x1b[32768;1H",
            "\r\n\x1b[123456;1H",
            "\r\n\x1b]0;title\x07",
        ] {
            let input = format!("{}{control}{}", &marker[..40], &marker[40..]);
            assert_eq!(scan(input.as_bytes()), Scan::Invalid);
        }
    }

    #[test]
    fn incomplete_candidates_have_a_raw_byte_bound() {
        // An attacker cannot extend a partial marker indefinitely with transport controls.
        let mut input = b"V".to_vec();
        input.extend(std::iter::repeat_n(b'\r', MAX_TRANSPORT_BYTES));
        assert_eq!(scan(&input), Scan::Invalid);
        assert_eq!(incomplete(&vec![0; MAX_TRANSPORT_BYTES]), Scan::Invalid);
    }

    #[test]
    fn captured_bottom_row_output_recovers_exact_fields() {
        let raw = b"VIVID;3;A;AAAAAAAAAAAAAAAAAAAAAA;0000000\r\n\x1b[23;40H0000000003;0000000000000007;AAAAAAAAAAAAA\r\n\x1b[23;40HAAAAAAAAAA;VIVID-END";
        let expected = "VIVID;3;A;AAAAAAAAAAAAAAAAAAAAAA;0000000000000003;0000000000000007;AAAAAAAAAAAAAAAAAAAAAA";
        assert_eq!(
            scan(raw),
            Scan::Complete {
                consumed: raw.len(),
                body: expected.to_owned()
            }
        );
        let mut wrong_repaint = raw.to_vec();
        let repaint = raw.windows(5).position(|bytes| bytes == b"3;40H").unwrap() + 5;
        wrong_repaint[repaint] = b'X';
        assert_eq!(scan(&wrong_repaint), Scan::Invalid);
    }
}
