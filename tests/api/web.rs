//! Public-API tests for `vivid_protocol::web`.
use vivid_protocol::web::*;

#[test]
fn the_ceilings_match_the_specification_table() {
    // Web §5.2 is normative: read its table instead of restating the numbers here.
    let spec = include_str!("../../vivid-protocol-1.5-web-bindings.md");
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
    assert!(!fits_control(vivid_protocol::CONTROL_MAX_RECORD_BODY));
    assert!(!fits_media(vivid_protocol::HARD_MAX_RECORD_BODY));
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
