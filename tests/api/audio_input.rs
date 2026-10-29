//! Public-API tests for `vivid_protocol::audio_input`.
use vivid_protocol::audio_input::*;
use vivid_protocol::cbor::Value;
use vivid_protocol::messages::LaneClass;
use vivid_protocol::track::{TrackConfiguration, TrackDirection, TrackMode};

#[test]
fn uplink_roundtrip_and_constraints() {
    let config = configuration(1, 2, 3);
    let payload = Value::Map(config.payload(false).unwrap());
    assert_eq!(
        TrackConfiguration::decode(3, &payload, false).unwrap(),
        config
    );
    let mut invalid = config.clone();
    invalid.slot = 2;
    assert!(invalid.validate(false).is_err());
    invalid = config.clone();
    invalid.mode = TrackMode::Timed;
    assert!(invalid.validate(false).is_err());
    invalid = config.clone();
    invalid.lane = LaneClass::Bulk;
    assert!(invalid.validate(false).is_err());
    invalid = config;
    invalid.direction = TrackDirection::Downlink;
    invalid.slot = 2;
    let payload = invalid.payload(false).unwrap();
    assert!(!payload.iter().any(|(key, _)| *key == 16));
    assert_eq!(
        TrackConfiguration::decode(3, &Value::Map(payload), false).unwrap(),
        invalid
    );
}

#[test]
fn pcm_shape_is_checked_before_copying() {
    let packet = InputPacket {
        epoch: 1,
        packet_id: 1,
        pts_us: 0,
        pcm: [0; PCM_BYTES],
    };
    let body = packet.encode().unwrap();
    assert_eq!(body.len(), BODY_BYTES as usize);
    InputPacket::decode(&body[..body.len() - 2]).unwrap_err();
    assert_eq!(InputPacket::decode(&body).unwrap().pcm, packet.pcm);
}
