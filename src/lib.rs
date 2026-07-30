//! Framing, deterministic control-message codecs, and media record layouts for the Vivid terminal
//! media protocol.
//!
//! This crate is transport- and renderer-independent. Producers and presenters share it to keep
//! the Vivid version, numeric registries, validation limits, and binary layouts synchronized.

#![forbid(unsafe_code)]

pub mod anchor;
pub mod auth;
pub mod cbor;
pub mod context;
#[cfg(feature = "native")]
pub mod discovery;
pub mod idempotency;
pub mod identity;
pub mod input;
pub mod lease;
pub mod media;
pub mod messages;
pub mod registry;
pub mod resource;
pub mod revision;
pub mod scene;
pub mod surface;
#[cfg(feature = "native")]
pub mod trace;
pub mod track;
pub mod wire;

/// Version of the Vivid wire protocol, used by both the connection preface and HELLO/WELCOME.
pub const VIVID_MAJOR: u8 = 1;
pub const VIVID_MINOR: u8 = 5;
pub const CONTROL_MAX_RECORD_BODY: u32 = 1024 * 1024;
pub const DEFAULT_MAX_RECORD_BODY: u32 = 64 * 1024 * 1024;
pub const HARD_MAX_RECORD_BODY: u32 = 64 * 1024 * 1024;
/// Maximum timeout carried by one correlated `WAIT_TRACK` request.
pub const MAX_TRACK_WAIT_TIMEOUT_US: u64 = 30_000_000;
