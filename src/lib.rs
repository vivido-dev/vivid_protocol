//! Framing, deterministic control-message codecs, and media record layouts for the Vivid terminal
//! media protocol.
//!
//! This crate is transport- and renderer-independent. Producers and presenters share it to keep
//! protocol versions, numeric registries, validation limits, and binary layouts synchronized.

#![forbid(unsafe_code)]

pub mod cbor;
pub mod media;
pub mod messages;
pub mod wire;

pub const PROTOCOL_MAJOR: u8 = 1;
pub const PROTOCOL_MINOR: u8 = 0;
pub const CONTROL_MAX_RECORD_BODY: u32 = 1024 * 1024;
pub const DEFAULT_MAX_RECORD_BODY: u32 = 64 * 1024 * 1024;
pub const HARD_MAX_RECORD_BODY: u32 = 64 * 1024 * 1024;
