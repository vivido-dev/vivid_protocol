//! Framing, deterministic control-message codecs, and media record layouts for the Vivid terminal
//! media protocol.
//!
//! This crate is transport- and renderer-independent. Producers and presenters share it to keep
//! the Vivid version, numeric registries, validation limits, and binary layouts synchronized.

#![forbid(unsafe_code)]

pub mod anchor;
pub mod audio_input;
pub mod auth;
pub mod cbor;
pub mod context;
#[cfg(any(feature = "native", feature = "native-transport"))]
pub mod discovery;
pub mod file_drop;
pub mod geometry;
pub mod grant;
pub mod hid;
pub mod idempotency;
pub mod identity;
pub mod input;
pub mod lease;
pub mod media;
pub mod messages;
pub mod observation;
pub mod overlay;
pub mod registry;
pub mod resource;
pub mod revision;
pub mod scene;
pub mod surface;
pub mod target;
pub mod time;
pub mod timed;
#[cfg(any(feature = "native", feature = "native-transport"))]
pub mod trace;
pub mod track;
pub mod vector;
pub mod web;
pub mod wire;

/// Narrows a compile-time size to `u32`, failing const evaluation if it does not fit.
///
/// Call it inside `const { ... }` so an oversized constant is a build error, not a runtime panic.
pub(crate) const fn const_u32(value: usize) -> u32 {
    assert!(value <= u32::MAX as usize, "size constant exceeds u32");
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the assert above bounds the value"
    )]
    let narrowed = value as u32;
    narrowed
}

/// Version of the Vivid wire protocol, used by both the connection preface and HELLO/WELCOME.
pub const VIVID_MAJOR: u8 = 1;
pub const VIVID_MINOR: u8 = 5;
pub const CONTROL_MAX_RECORD_BODY: u32 = 1024 * 1024;
pub const DEFAULT_MAX_RECORD_BODY: u32 = 64 * 1024 * 1024;
pub const HARD_MAX_RECORD_BODY: u32 = 64 * 1024 * 1024;
/// Ceiling an interactive lane may grant one record. The lane must stay responsive while bulk
/// media is saturated, so it carries input and small snapshots rather than payloads.
pub const LANE_MAX_RECORD_BODY: u32 = 64 * 1024;
/// Maximum timeout carried by one correlated `WAIT_TRACK` request.
pub const MAX_TRACK_WAIT_TIMEOUT_US: u64 = 30_000_000;
