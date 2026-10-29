//! The finite ceilings a browser carrier imposes, web §5.2.
//!
//! A classic WebSocket gives the receiver no byte-stream backpressure, so the browser binding
//! replaces it with hard bounds: the bridge closes a track rather than buffering past them. These
//! are the *maxima* — a deployment may advertise lower values, never higher ones. A native
//! presenter uses them to offer a contract a browser peer can honor.

/// The largest WebSocket binary message a sender may produce.
///
/// Message boundaries carry no Vivid meaning; this only bounds what the receiver must hold while
/// parsing incrementally.
pub const MAX_SOCKET_CHUNK: u32 = 64 * 1024;

/// The largest control record body a browser carrier accepts.
pub const MAX_CONTROL_RECORD_BODY: u32 = 256 * 1024;

/// The largest interactive-lane record body a browser carrier accepts.
pub const MAX_INTERACTIVE_RECORD_BODY: u32 = 64 * 1024;

/// The largest media record body a browser carrier accepts.
pub const MAX_MEDIA_RECORD_BODY: u32 = 8 * 1024 * 1024;

/// Reassembly plus queued decoded input across every connection of one browser carrier.
///
/// Channel windows are granted against this, not against each connection separately: a presenter
/// must not grant a window whose worst case drives the whole pipeline over the aggregate.
pub const MAX_AGGREGATE_REASSEMBLY: u64 = 32 * 1024 * 1024;

/// Undelivered receive-pump chunks a single connection may hold.
pub const MAX_PENDING_CHUNKS: u32 = 128;

/// Bytes beyond one accepted record that per-connection incomplete-record reassembly may hold.
///
/// Web §5.2 bounds it at "one accepted record plus 24 bytes" — the frame header, so that a header
/// split across two messages still parses.
pub const REASSEMBLY_HEADROOM: u32 = 24;

/// Whether a record ceiling can be carried by a browser on the given connection kind.
///
/// Take the argument from the ceiling actually negotiated, not from a request: the point is to
/// decide whether an already-agreed limit is deliverable, and a bridge that answers "yes" wrongly
/// closes the carrier mid-session instead of refusing up front.
pub const fn fits_control(body: u32) -> bool {
    body <= MAX_CONTROL_RECORD_BODY
}

/// Whether an interactive record ceiling fits the browser bound.
pub const fn fits_interactive(body: u32) -> bool {
    body <= MAX_INTERACTIVE_RECORD_BODY
}

/// Whether a media record ceiling fits the browser bound.
pub const fn fits_media(body: u32) -> bool {
    body <= MAX_MEDIA_RECORD_BODY
}
