//! The finite ceilings a browser carrier imposes, web §5.2.
//!
//! A classic WebSocket gives the receiver no byte-stream backpressure, so the browser binding
//! replaces it with hard bounds: the bridge closes a track rather than buffering past them. These
//! are the *maxima* — a deployment may advertise lower values, never higher ones — and they belong
//! here rather than in `vvweb` because a native presenter has to be able to offer a contract a
//! browser peer can actually honour without knowing anything about JavaScript.

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ceilings_match_the_specification_table() {
        // Web §5.2 is normative: read its table instead of restating the numbers here.
        let spec = include_str!("../vivid-protocol-1.5-web-bindings.md");
        let table = &spec[spec.find("### 5.2 Mandatory web ceilings").unwrap()..];
        let cell = |bound: &str| -> &str {
            let prefix = format!("| {bound} |");
            let row = table
                .lines()
                .find(|line| line.starts_with(&prefix))
                .unwrap_or_else(|| panic!("web §5.2 has no {bound:?} row"));
            row.split('|').nth(2).unwrap().trim()
        };
        let bytes = |bound: &str| -> u64 {
            let value = cell(bound).trim_end_matches(" bytes").replace(',', "");
            value.parse().unwrap()
        };
        assert_eq!(
            u64::from(MAX_SOCKET_CHUNK),
            bytes("WebSocket binary message chunk")
        );
        assert_eq!(
            u64::from(MAX_CONTROL_RECORD_BODY),
            bytes("Control Vivid record body")
        );
        assert_eq!(
            u64::from(MAX_INTERACTIVE_RECORD_BODY),
            bytes("Interactive record body")
        );
        assert_eq!(
            u64::from(MAX_MEDIA_RECORD_BODY),
            bytes("Media Vivid record body")
        );
        assert_eq!(
            MAX_AGGREGATE_REASSEMBLY,
            bytes("Aggregate browser reassembly and queued decoded input")
        );
        assert_eq!(
            u64::from(MAX_PENDING_CHUNKS),
            bytes("Pending browser event chunks per connection")
        );
        assert_eq!(
            cell("Per-connection incomplete-record reassembly"),
            format!("One accepted record plus {REASSEMBLY_HEADROOM} bytes")
        );
    }

    #[test]
    fn the_native_defaults_do_not_fit_a_browser() {
        // The reason a web-compatible contract has to exist at all: a presenter advertising its
        // native ceilings would hand a browser peer limits the carrier closes on.
        assert!(!fits_control(crate::CONTROL_MAX_RECORD_BODY));
        assert!(!fits_media(crate::HARD_MAX_RECORD_BODY));
    }

    #[test]
    fn a_chunked_record_fits_within_the_aggregate() {
        // One in-flight media record plus its header must still leave the aggregate budget room
        // for the other connections of the same carrier.
        let worst_case = u64::from(MAX_MEDIA_RECORD_BODY + REASSEMBLY_HEADROOM);
        assert!(worst_case * 2 <= MAX_AGGREGATE_REASSEMBLY);
    }

    #[test]
    fn the_pending_chunk_budget_bounds_one_media_record() {
        // A receive pump holding the full pending-chunk allowance stays inside the media ceiling,
        // so a stalled event loop cannot exceed the per-connection bound before the aggregate one.
        assert!(
            u64::from(MAX_PENDING_CHUNKS) * u64::from(MAX_SOCKET_CHUNK) <= MAX_AGGREGATE_REASSEMBLY
        );
    }
}
