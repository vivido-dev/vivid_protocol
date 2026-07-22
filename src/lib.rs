//! Framing, deterministic control-message codecs, and media record layouts for the Vivid terminal
//! media protocol.
//!
//! This crate is transport- and renderer-independent. Producers and presenters share it to keep
//! the Vivid version, numeric registries, validation limits, and binary layouts synchronized.

#![forbid(unsafe_code)]

pub mod anchor;
pub mod cbor;
pub mod media;
pub mod messages;
pub mod wire;

/// Version of the Vivid wire protocol, used by both the connection preface and HELLO/WELCOME.
pub const VIVID_MAJOR: u8 = 1;
pub const VIVID_MINOR: u8 = 0;

/// Compatibility alias for [`VIVID_MAJOR`].
#[deprecated(since = "1.2.1", note = "use VIVID_MAJOR")]
pub const FRAMING_MAJOR: u8 = VIVID_MAJOR;
/// Compatibility alias for [`VIVID_MINOR`].
#[deprecated(since = "1.2.1", note = "use VIVID_MINOR")]
pub const FRAMING_MINOR: u8 = VIVID_MINOR;
/// Compatibility alias for [`VIVID_MAJOR`].
#[deprecated(since = "1.2.1", note = "use VIVID_MAJOR")]
pub const PROTOCOL_MAJOR: u8 = VIVID_MAJOR;
/// Compatibility alias for [`VIVID_MINOR`].
#[deprecated(since = "1.2.1", note = "use VIVID_MINOR")]
pub const PROTOCOL_MINOR: u8 = VIVID_MINOR;
pub const CONTROL_MAX_RECORD_BODY: u32 = 1024 * 1024;
pub const DEFAULT_MAX_RECORD_BODY: u32 = 64 * 1024 * 1024;
pub const HARD_MAX_RECORD_BODY: u32 = 64 * 1024 * 1024;
