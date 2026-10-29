//! Public-API tests for `vivid_protocol::anchor`.
use vivid_protocol::anchor::*;

#[test]
fn marker_v3_round_trip_and_owner_binding() {
    let key = AnchorKey::new([0x42; 32]);
    let tag = [0x24; 16];
    let encoded = encode_marker(&key, &tag, 3, 7).unwrap();
    assert!(encoded.len() <= MAX_MARKER_BYTES);
    let marker = parse_marker(&encoded[2..encoded.len() - 2]).unwrap();
    assert!(verify_marker(&key, &marker));
    let conpty = encode_conpty_marker(&key, &tag, 3, 7).unwrap();
    assert_eq!(parse_conpty_marker(&conpty).unwrap(), marker);

    let mut wrong_owner = marker;
    wrong_owner.context_id = 4;
    assert!(!verify_marker(&key, &wrong_owner));
}
