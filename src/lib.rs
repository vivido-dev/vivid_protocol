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

/// Copies the `N` bytes at `offset` into an array.
///
/// Callers check the bounds first; every use reads a fixed field from a length-checked buffer.
pub(crate) fn array_at<const N: usize>(bytes: &[u8], offset: usize) -> [u8; N] {
    let mut array = [0; N];
    array.copy_from_slice(&bytes[offset..offset + N]);
    array
}

/// Major version of the Vivid wire protocol, carried by the preface and HELLO/WELCOME.
pub const VIVID_MAJOR: u8 = 1;
/// Minor version of the Vivid wire protocol; 1.5 peers reject any other minor version.
pub const VIVID_MINOR: u8 = 5;
/// Ceiling on one control record body, the core specification's 1 MiB control limit (§4.3).
///
/// The effective ceiling is the minimum of this, the preface limit, and the peer's advertised
/// limit; raising it here does not let a peer accept larger bodies.
pub const CONTROL_MAX_RECORD_BODY: u32 = 1024 * 1024;
/// Body limit a non-control connection advertises in its preface when nothing narrower applies.
///
/// It equals [`HARD_MAX_RECORD_BODY`] today. It is a separate constant because it is a default
/// that callers may lower, while the hard ceiling is a protocol bound no value may exceed.
pub const DEFAULT_MAX_RECORD_BODY: u32 = 64 * 1024 * 1024;
/// Absolute ceiling on any record body: the specification's 64 MiB media hard limit (§4.3).
///
/// Every size check in this crate rejects larger bodies before allocating, so this bounds the
/// memory one record can demand from a receiver.
pub const HARD_MAX_RECORD_BODY: u32 = 64 * 1024 * 1024;
/// Ceiling an interactive lane may grant one record. The lane must stay responsive while bulk
/// media is saturated, so it carries input and small snapshots rather than payloads.
pub const LANE_MAX_RECORD_BODY: u32 = 64 * 1024;
/// Maximum timeout carried by one correlated `WAIT_TRACK` request.
pub const MAX_TRACK_WAIT_TIMEOUT_US: u64 = 30_000_000;
