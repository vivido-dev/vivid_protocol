//! Public-API tests for `vivid_protocol::track`.
use vivid_protocol::track::*;
use vivid_protocol::{cbor::Value, media, messages::LaneClass, revision::ChannelGeneration};

/// The exact keys and order of a `MAX_CHANNEL_DATA` body.
///
/// Pinned against a literal rather than against another call to the same function: this is the
/// regression the three hand-written copies could not have, because each was its own authority
/// on what the map should contain.
#[test]
fn a_flow_grant_carries_the_owner_tuple_then_the_two_maxima() {
    let address = TrackAddress {
        context_id: 1,
        surface_id: 2,
        track_id: 3,
        channel_generation: ChannelGeneration::new(4),
    };

    assert_eq!(
        max_channel_data_payload(address, 65_536, 128),
        vec![
            (0, Value::Unsigned(1)),
            (1, Value::Unsigned(2)),
            (2, Value::Unsigned(3)),
            (3, Value::Unsigned(4)),
            (4, Value::Unsigned(65_536)),
            (5, Value::Unsigned(128)),
        ]
    );
}

#[test]
fn a_keyframe_request_carries_its_minimum_epoch_and_reason() {
    let address = TrackAddress {
        context_id: 7,
        surface_id: 8,
        track_id: 9,
        channel_generation: ChannelGeneration::new(2),
    };

    assert_eq!(
        need_keyframe_payload(address, 5, 2),
        vec![
            (0, Value::Unsigned(7)),
            (1, Value::Unsigned(8)),
            (2, Value::Unsigned(9)),
            (3, Value::Unsigned(2)),
            (4, Value::Unsigned(5)),
            (5, Value::Unsigned(2)),
        ]
    );
}

#[test]
fn a_full_frame_request_takes_its_reason_rather_than_assuming_one() {
    // One presenter parameterised this reason and another hardcoded 1. Same value, but the
    // shared encoder has to accept it or the two cannot both use this.
    let address = TrackAddress {
        context_id: 1,
        surface_id: 1,
        track_id: 1,
        channel_generation: ChannelGeneration::new(1),
    };

    assert_eq!(
        need_full_frame_payload(address, 1).last(),
        Some(&(4, Value::Unsigned(1)))
    );
    assert_eq!(
        need_full_frame_payload(address, 3).last(),
        Some(&(4, Value::Unsigned(3)))
    );
}

#[test]
fn every_channel_notification_opens_with_the_same_owner_tuple() {
    // The property that makes a wrong-track notification impossible to misread: all three
    // answer "which track" in keys 0 through 3, in one order.
    let address = TrackAddress {
        context_id: 11,
        surface_id: 12,
        track_id: 13,
        channel_generation: ChannelGeneration::new(14),
    };
    let expected = [
        (0, Value::Unsigned(11)),
        (1, Value::Unsigned(12)),
        (2, Value::Unsigned(13)),
        (3, Value::Unsigned(14)),
    ];

    for payload in [
        max_channel_data_payload(address, 1, 1),
        need_keyframe_payload(address, 0, 0),
        need_full_frame_payload(address, 1),
    ] {
        assert_eq!(&payload[..4], &expected[..], "payload: {payload:?}");
    }
}

#[test]
fn audio_gain_covers_the_kitim_volume_range() {
    assert_eq!(AudioGain::from_percent(0), Some(AudioGain::SILENT));
    assert_eq!(AudioGain::from_percent(100), Some(AudioGain::UNITY));
    assert_eq!(AudioGain::from_percent(200), Some(AudioGain::MAX));
    assert_eq!(AudioGain::from_percent(201), None);
    assert_eq!(AudioGain::new(AudioGain::MAX.raw() + 1), None);
}

#[test]
fn channel_generation_resets_generation_local_state() {
    let mut state = TrackState::new();
    state
        .accept_channel(ChannelGeneration::ONE, 100, 4, 100)
        .unwrap();
    state
        .admit_media(ReceivedMedia {
            generation: ChannelGeneration::ONE,
            body_length: 20,
            epoch: 1,
            media_id: 1,
            random_access: true,
        })
        .unwrap();
    state
        .advance_channel(ChannelGeneration::ONE, ChannelGeneration::new(2))
        .unwrap();
    assert_eq!(state.milestones, 0);
    assert_eq!(state.flow.sent_body_bytes, 0);
    assert_eq!(state.last_media_id, 1);
}

#[test]
fn lower_flow_update_is_harmless() {
    let mut state = TrackState::new();
    state
        .accept_channel(ChannelGeneration::ONE, 100, 4, 100)
        .unwrap();
    state.flow.raise_maxima(50, 2);
    assert_eq!(state.flow.maximum_body_bytes, 100);
    assert_eq!(state.flow.maximum_media_records, 4);
}

#[test]
fn uncertain_channel_open_replays_only_before_media() {
    let mut open = ChannelOpenState::default();
    let acceptance = vec![(0, Value::Unsigned(1))];
    assert!(matches!(
        open.open(
            ChannelGeneration::ONE,
            ChannelOpenAttempt {
                generation: ChannelGeneration::ONE,
                nonce: [1; 16],
                complete_open_bytes: b"open",
                acceptance: acceptance.clone()
            }
        ),
        ChannelOpenDecision::Fresh(_)
    ));
    assert_eq!(
        open.open(
            ChannelGeneration::ONE,
            ChannelOpenAttempt {
                generation: ChannelGeneration::ONE,
                nonce: [1; 16],
                complete_open_bytes: b"open",
                acceptance: acceptance.clone()
            }
        ),
        ChannelOpenDecision::Busy
    );
    open.transport_lost(ChannelGeneration::ONE);
    assert_eq!(
        open.open(
            ChannelGeneration::ONE,
            ChannelOpenAttempt {
                generation: ChannelGeneration::ONE,
                nonce: [1; 16],
                complete_open_bytes: b"open",
                acceptance
            }
        ),
        ChannelOpenDecision::ExactReplay(vec![(0, Value::Unsigned(1))])
    );
    open.admit_media(ChannelGeneration::ONE).unwrap();
    open.transport_lost(ChannelGeneration::ONE);
    assert_eq!(
        open.open(
            ChannelGeneration::ONE,
            ChannelOpenAttempt {
                generation: ChannelGeneration::ONE,
                nonce: [1; 16],
                complete_open_bytes: b"open",
                acceptance: vec![]
            }
        ),
        ChannelOpenDecision::StaleGeneration
    );
}

#[test]
fn strict_raster_track_configuration_round_trips() {
    let configuration = TrackConfiguration {
        direction: TrackDirection::default(),
        context_id: 1,
        surface_id: 2,
        track_id: 3,
        slot: 0,
        mode: TrackMode::Live,
        lane: LaneClass::Bulk,
        maximum_record_body: 72 + 64 * 64 * 4,
        maximum_rate_millihertz: 60_000,
        maximum_encoded_bits_per_second: 100_000_000,
        maximum_records_per_second: 60,
        maximum_inflight_body_bytes: 1_000_000,
        kind: KindConfiguration::Raster(RasterConfiguration {
            width: 64,
            height: 64,
            alpha_mode: 1,
            delta_enabled: true,
            maximum_delta_operations: 16,
            zstd_enabled: true,
        }),
        target_latency_us: 50_000,
        maximum_latency_us: 250_000,
        retained_pixel_charge: 64 * 64,
    };
    let payload = Value::Map(configuration.payload(false).unwrap());
    assert_eq!(
        TrackConfiguration::decode(3, &payload, false).unwrap(),
        configuration
    );
}

#[test]
fn canonical_audio_track_configuration_round_trips() {
    let mut opus_head = b"OpusHead".to_vec();
    opus_head.extend_from_slice(&[1, 2, 0, 0, 0x80, 0xbb, 0, 0, 0, 0, 0]);
    let maximum_record_body = media::audio_body_len(4_096).unwrap();
    let configuration = TrackConfiguration {
        direction: TrackDirection::default(),
        context_id: 1,
        surface_id: 2,
        track_id: 3,
        slot: 2,
        mode: TrackMode::Timed,
        lane: LaneClass::Realtime,
        maximum_record_body,
        maximum_rate_millihertz: 50_000,
        maximum_encoded_bits_per_second: 512_000,
        maximum_records_per_second: 50,
        maximum_inflight_body_bytes: u64::from(maximum_record_body) * 4,
        kind: KindConfiguration::Audio(AudioConfiguration {
            codec: "opus".into(),
            packetization: media::AUDIO_PACKETIZATION_OPUS.into(),
            extradata: opus_head,
            sample_rate: 48_000,
            channels: 2,
            channel_mask: 3,
            maximum_access_unit_bytes: 4_096,
            codec_string: Some("opus".into()),
        }),
        target_latency_us: 0,
        maximum_latency_us: 2_000_000,
        retained_pixel_charge: 0,
    };
    let payload = Value::Map(configuration.payload(false).unwrap());
    assert_eq!(
        TrackConfiguration::decode(3, &payload, false).unwrap(),
        configuration
    );
}
