//! The wire model of Vivid Protocol 1.5: framing, control codecs, and media layouts.
//!
//! Vivid carries surfaces, scenes, and media from a *producer*, such as a program running in a
//! terminal, to a *presenter*, such as the terminal emulator, over authenticated local
//! connections. This crate is transport- and renderer-independent. Producers, presenters,
//! gateways, and browser bindings share it so they agree on the protocol version, the numeric
//! registries, every validation limit, and every binary layout. The normative specification is
//! `vivid-protocol-1.5-spec.md` in this crate's repository.
//!
//! Binary-layout codecs report [`CodecError`]; control-message codecs report
//! [`messages::MessageError`]. Both convert into [`std::io::Error`] for callers that use `?` in
//! I/O code.
//!
//! # Modules
//!
//! - **Connections:** [`wire`] framing and transport, `discovery` of endpoints, and [`cbor`].
//! - **Sessions:** [`messages`] control records, [`auth`] proofs and keys, [`lease`], [`context`],
//!   [`resource`] contracts, and the [`registry`] of numeric assignments.
//! - **Presentation:** [`target`], [`surface`], [`scene`], [`geometry`], and terminal [`anchor`]s.
//! - **Media:** [`track`], [`media`] record layouts, [`timed`] playback, [`audio_input`], and
//!   [`vector`] display lists.
//! - **Input and files:** desktop [`input`], presenter [`grant`]s, [`hid`] usages, and
//!   [`file_drop`].
//! - **Overlays:** [`overlay`] windows and their [`overlay::wire`] records.
//! - **Bookkeeping:** [`identity`], [`revision`], [`idempotency`], [`observation`], [`time`],
//!   `trace`, and [`web`] carrier limits.
//!
//! `wire::Connection`, `discovery`, and `trace` need the default `native` feature or
//! `native-transport`; everything else, including wasm32 builds, works without them.
//!
//! # Quick start
//!
//! A producer opens a control connection, which writes the preface, and sends a root-authenticated
//! `HELLO`. A presenter checks the preface, the first record, and the proof before allocating
//! anything for the session. Here both sides share an in-memory buffer instead of a socket:
//!
//! ```
//! use std::io::{self, Write};
//! use std::sync::{Arc, Mutex};
//!
//! use vivid_protocol::auth::{self, Secret32};
//! use vivid_protocol::messages::{self, Hello, HelloAuthentication};
//! use vivid_protocol::registry;
//! use vivid_protocol::wire::{
//!     self, Connection, ConnectionKind, HEADER_SIZE, PREFACE_SIZE, RecordHeader,
//! };
//!
//! /// Bytes the producer wrote, for the presenter half of the example to read.
//! #[derive(Clone, Default)]
//! struct Buffer(Arc<Mutex<Vec<u8>>>);
//!
//! impl Write for Buffer {
//!     fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
//!         self.0.lock().unwrap().extend_from_slice(bytes);
//!         Ok(bytes.len())
//!     }
//!     fn flush(&mut self) -> io::Result<()> {
//!         Ok(())
//!     }
//! }
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let root_secret = Secret32::new([7; 32]);
//!
//! // Producer: the proof covers the exact preface the connection writes.
//! let buffer = Buffer::default();
//! let kind = ConnectionKind::Control;
//! let connection = Connection::from_streams(io::empty(), buffer.clone(), kind)?;
//! let preface = wire::encode_preface(kind, vivid_protocol::CONTROL_MAX_RECORD_BODY);
//! let mut hello = Hello {
//!     producer_name: "example".into(),
//!     producer_version: "1.0".into(),
//!     required_profiles: vec![registry::TERMINAL_SURFACE.into(), registry::CORE_CONTROL.into()],
//!     optional_profiles: vec![],
//!     maximum_control_body: vivid_protocol::CONTROL_MAX_RECORD_BODY,
//!     client_nonce: [1; 32],
//!     authentication: HelloAuthentication::Root { proof: [0; 32] },
//!     target_profile: registry::TERMINAL_SURFACE.into(),
//!     extensions: vec![],
//! };
//! hello.authenticate_root(&root_secret, &preface)?;
//! connection.writer().write_record(messages::HELLO, 0, 0, &hello.encode(1)?)?;
//!
//! // Presenter: validate the preface and first record, then the proof.
//! let bytes = buffer.0.lock().unwrap().clone();
//! let (preface_bytes, records) = bytes.split_at(PREFACE_SIZE);
//! let preface_bytes: [u8; PREFACE_SIZE] = preface_bytes.try_into()?;
//! let accepted = wire::accept_preface(preface_bytes, &mut io::sink())?;
//! let header = RecordHeader::decode(records[..HEADER_SIZE].try_into()?);
//! accepted.kind.validate_first_record(&header)?;
//! let (_request_id, received) = Hello::decode(&records[HEADER_SIZE..])?;
//! let HelloAuthentication::Root { proof } = &received.authentication else {
//!     return Err("expected root authentication".into());
//! };
//! assert!(auth::verify_root_hello_proof(
//!     &root_secret,
//!     &preface_bytes,
//!     &received.authless_payload()?,
//!     proof,
//! ));
//! # Ok(())
//! # }
//! ```
//!
//! A real producer then reads `WELCOME`, verifies its confirmation with [`auth`], and creates
//! surfaces, tracks, and scene nodes with the records in [`messages`], [`surface`], [`track`], and
//! [`scene`].

#![forbid(unsafe_code)]

mod codec;

#[doc(inline)]
pub use codec::CodecError;

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
