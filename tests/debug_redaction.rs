//! `Debug` output of public types never renders key material, tags, proofs, or payload bytes.
use vivid_protocol::{
    anchor::AnchorKey,
    auth::{self, Secret32},
    cbor::Value,
    lease::{CleanupPolicy, LeaseMachine},
    media::{AudioPacket, VideoPacket},
    messages::{ChannelOpen, LaneClass, LaneOpen, StrictMap, TrackKind, WelcomeAuthentication},
    wire::{BorrowedRecord, Record},
};

/// Distinct, non-repeating bytes so a leak cannot hide behind a coincidental match.
fn secret<const N: usize>(seed: u8) -> [u8; N] {
    std::array::from_fn(|index| seed.wrapping_add((index as u8).wrapping_mul(37)))
}

/// Asserts `rendered` holds no recognizable rendering of `bytes`.
fn assert_redacted(rendered: &str, bytes: &[u8]) {
    let window = &bytes[..bytes.len().min(4)];
    let decimal = format!("{window:?}");
    let decimal = decimal.trim_matches(['[', ']']);
    let hex: String = window.iter().map(|byte| format!("{byte:02x}")).collect();
    assert!(!rendered.contains(decimal), "{rendered} leaks {decimal}");
    assert!(!rendered.contains(&hex), "{rendered} leaks {hex}");
}

#[test]
fn derived_keys_are_opaque() {
    let prk = auth::extract_handshake_prk(
        &Secret32::new(secret(1)),
        &secret(2),
        &secret(3),
        &secret(4),
    );
    let (keys, anchor) = auth::derive_session_keys(&prk, 7, 1, &secret(5));

    assert_eq!(format!("{prk:?}"), "HandshakePrk([REDACTED])");
    assert_eq!(format!("{keys:?}"), "SessionKeys([REDACTED])");
    assert_eq!(format!("{anchor:?}"), "AnchorKey([REDACTED])");
    assert_eq!(
        format!("{:?}", AnchorKey::new(secret(6))),
        "AnchorKey([REDACTED])"
    );
}

#[test]
fn lease_machine_hides_fingerprint_and_attempt() {
    let attempt_id = secret::<16>(10);
    let client_nonce = secret::<32>(11);
    let fingerprint = secret::<32>(12);
    let server_nonce = secret::<32>(13);
    let welcome = secret::<32>(14).to_vec();
    let mut lease = LeaseMachine::new(CleanupPolicy::Immediate, 0);
    lease
        .begin_activation(
            attempt_id,
            client_nonce,
            b"hello",
            fingerprint,
            9,
            server_nonce,
            welcome.clone(),
        )
        .unwrap();

    let rendered = format!("{lease:?}");
    for bytes in [
        &attempt_id[..],
        &client_nonce,
        &fingerprint,
        &server_nonce,
        &welcome,
    ] {
        assert_redacted(&rendered, bytes);
    }
    assert!(rendered.contains("has_profile_fingerprint: true"));
    assert!(rendered.contains("has_attempt: true"));
}

#[test]
fn authentication_messages_hide_confirmations_and_tags() {
    let confirmation = secret::<32>(20);
    let welcome = WelcomeAuthentication {
        kind: 0,
        confirmation,
        lease_state: 0,
        activation_attempt_status: 0,
    };
    let rendered = format!("{welcome:?}");
    assert!(rendered.contains("[REDACTED]"));
    assert_redacted(&rendered, &confirmation);

    let lane_tag = secret::<16>(21);
    let lane = LaneOpen {
        session_id: 1,
        lane_generation: 1,
        client_nonce: [0; 16],
        authentication_tag: lane_tag,
    };
    let rendered = format!("{lane:?}");
    assert!(rendered.contains("[REDACTED]"));
    assert_redacted(&rendered, &lane_tag);

    let channel_tag = secret::<16>(22);
    let channel = ChannelOpen {
        session_id: 1,
        context_id: 2,
        surface_id: 3,
        track_id: 4,
        channel_generation: 1,
        track_kind: TrackKind::Video,
        lane: LaneClass::Realtime,
        client_nonce: [0; 16],
        authentication_tag: channel_tag,
    };
    let rendered = format!("{channel:?}");
    assert!(rendered.contains("[REDACTED]"));
    assert_redacted(&rendered, &channel_tag);
}

#[test]
fn records_and_packets_report_lengths_not_bytes() {
    let body = secret::<64>(30);
    let record = Record {
        record_type: 0x0100,
        flags: 0,
        object_id: 5,
        sequence: 6,
        body: body.to_vec(),
    };
    let rendered = format!("{record:?}");
    assert!(rendered.contains("body_len: 64"), "{rendered}");
    assert_redacted(&rendered, &body);

    let borrowed = BorrowedRecord {
        record_type: 0x0100,
        flags: 0,
        object_id: 5,
        sequence: 6,
        body: &body,
    };
    let rendered = format!("{borrowed:?}");
    assert!(rendered.contains("body_len: 64"), "{rendered}");
    assert_redacted(&rendered, &body);

    let video = VideoPacket {
        epoch: 1,
        packet_id: 2,
        pts_us: 3,
        dts_us: 3,
        duration_us: 4,
        key: true,
        data: &body,
    };
    let rendered = format!("{video:?}");
    assert!(rendered.contains("data_len: 64"), "{rendered}");
    assert_redacted(&rendered, &body);

    let audio = AudioPacket {
        epoch: 1,
        packet_id: 2,
        pts_us: 3,
        dts_us: 3,
        duration_us: 4,
        trim_start_samples: 0,
        trim_end_samples: 0,
        data: &body,
    };
    let rendered = format!("{audio:?}");
    assert!(rendered.contains("data_len: 64"), "{rendered}");
    assert_redacted(&rendered, &body);
}

#[test]
fn strict_map_hides_decoded_values() {
    let proof = secret::<32>(40);
    let value = Value::Map(vec![(0, Value::Bytes(proof.to_vec()))]);
    let map = StrictMap::new("test", &value, &[0]).unwrap();
    let rendered = format!("{map:?}");
    assert!(rendered.contains("entries: 1"), "{rendered}");
    assert_redacted(&rendered, &proof);
}
