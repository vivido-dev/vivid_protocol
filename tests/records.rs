//! Typed control records decode exactly what they encode and reject what their schema forbids.
use vivid_protocol::{
    anchor::QueryAnchor,
    cbor::Value,
    context::RevokeContext,
    lease::RevokeSessionLease,
    messages::CancelWait,
    observation::SetObservation,
    revision::{ChannelGeneration, SurfaceGeneration, SurfaceRevision},
    scene::{BeginTxn, CommitTxn, DeleteNode},
    surface::{SurfaceDescriptor, SurfaceRole, UpdateSurface},
    track::{
        ActivateTrack, AdvanceChannel, AudioGain, ChannelEos, Flush, SetAudioGain, SlotBinding,
        WaitTrack,
    },
};

fn map(payload: Vec<(u64, Value)>) -> Value {
    Value::Map(payload)
}

fn wait(condition: u64, condition_value: Option<u64>) -> WaitTrack {
    WaitTrack {
        context_id: 1,
        surface_id: 2,
        track_id: 3,
        condition,
        condition_value,
        timeout_us: 1_000,
        channel_generation: ChannelGeneration::ONE,
    }
}

#[test]
fn every_record_round_trips() {
    let advance = AdvanceChannel {
        context_id: 1,
        surface_id: 2,
        track_id: 3,
        current_generation: ChannelGeneration::new(4),
        next_generation: ChannelGeneration::new(5),
        reason: 3,
    };
    assert_eq!(
        AdvanceChannel::decode(&map(advance.payload())).unwrap(),
        advance
    );

    let gain = SetAudioGain {
        context_id: 1,
        surface_id: 2,
        track_id: 3,
        gain: AudioGain::UNITY,
    };
    assert_eq!(SetAudioGain::decode(&map(gain.payload())).unwrap(), gain);

    let activate = ActivateTrack {
        context_id: 1,
        surface_id: 2,
        bindings: vec![
            SlotBinding {
                slot: 1,
                track_id: 3,
                channel_generation: ChannelGeneration::ONE,
                required_milestone: 4,
            },
            SlotBinding {
                slot: 2,
                track_id: 0,
                channel_generation: ChannelGeneration::ZERO,
                required_milestone: 0,
            },
        ],
        expected_surface_revision: SurfaceRevision::new(7),
    };
    assert_eq!(
        ActivateTrack::decode(&map(activate.payload())).unwrap(),
        activate
    );

    for record in [wait(1, Some(9)), wait(5, None)] {
        assert_eq!(WaitTrack::decode(&map(record.payload())).unwrap(), record);
    }

    let flush = Flush {
        context_id: 1,
        surface_id: 2,
        track_id: 3,
        epoch: 4,
    };
    assert_eq!(Flush::decode(&map(flush.payload())).unwrap(), flush);

    let eos = ChannelEos {
        context_id: 1,
        surface_id: 2,
        track_id: 3,
        channel_generation: ChannelGeneration::ONE,
        epoch: 4,
        last_media_sequence: 0,
    };
    assert_eq!(ChannelEos::decode(&map(eos.payload())).unwrap(), eos);

    let begin = BeginTxn {
        context_id: 1,
        transaction_id: 2,
    };
    assert_eq!(BeginTxn::decode(&map(begin.payload())).unwrap(), begin);
    let delete = DeleteNode {
        context_id: 1,
        node_id: 2,
    };
    assert_eq!(DeleteNode::decode(&map(delete.payload())).unwrap(), delete);
    let commit = CommitTxn {
        presentation_mode: 0,
    };
    assert_eq!(CommitTxn::decode(&map(commit.payload())).unwrap(), commit);

    let anchor = QueryAnchor {
        context_id: 1,
        anchor_id: 2,
    };
    assert_eq!(QueryAnchor::decode(&map(anchor.payload())).unwrap(), anchor);
    let cancel = CancelWait { request_id: 9 };
    assert_eq!(CancelWait::decode(&map(cancel.payload())).unwrap(), cancel);
    let observe = SetObservation { mask: 0b1_0101 };
    assert_eq!(
        SetObservation::decode(&map(observe.payload())).unwrap(),
        observe
    );
    let lease = RevokeSessionLease {
        context_id: 1,
        lease_id: 2,
    };
    assert_eq!(
        RevokeSessionLease::decode(&map(lease.payload())).unwrap(),
        lease
    );
    for context in [
        RevokeContext {
            context_id: 2,
            parent_context_id: Some(1),
        },
        RevokeContext {
            context_id: 2,
            parent_context_id: None,
        },
    ] {
        assert_eq!(
            RevokeContext::decode(&map(context.payload())).unwrap(),
            context
        );
    }

    let update = UpdateSurface {
        context_id: 1,
        surface_id: 2,
        expected_revision: SurfaceRevision::new(3),
        expected_generation: SurfaceGeneration::new(4),
        logical_width: 640,
        logical_height: 480,
        scale_numerator: 1,
        scale_denominator: 1,
        rotation: 90,
        descriptor: SurfaceDescriptor {
            role: SurfaceRole::Desktop,
            title: "desktop".into(),
            semantic_content_revision: 0,
            semantic_availability: 0,
            locator_hint: String::new(),
        },
        policy: 0,
        profile_parameters: vec![(0, Value::Unsigned(5))],
    };
    let decoded = UpdateSurface::decode(&map(update.payload().unwrap())).unwrap();
    assert_eq!(decoded, update);
}

#[test]
fn waits_reject_bad_conditions_timeouts_and_generations() {
    for invalid in [
        wait(1, None),
        wait(5, Some(1)),
        wait(10, None),
        WaitTrack {
            timeout_us: 0,
            ..wait(5, None)
        },
        WaitTrack {
            timeout_us: vivid_protocol::MAX_TRACK_WAIT_TIMEOUT_US + 1,
            ..wait(5, None)
        },
        WaitTrack {
            channel_generation: ChannelGeneration::ZERO,
            ..wait(5, None)
        },
    ] {
        WaitTrack::decode(&map(invalid.payload())).unwrap_err();
    }
}

#[test]
fn out_of_range_fields_and_unknown_keys_are_rejected() {
    let gain = map(vec![
        (0, Value::Unsigned(1)),
        (1, Value::Unsigned(2)),
        (2, Value::Unsigned(3)),
        (3, Value::Unsigned(AudioGain::MAX.raw() + 1)),
    ]);
    SetAudioGain::decode(&gain).unwrap_err();

    let flush = map(vec![
        (0, Value::Unsigned(1)),
        (1, Value::Unsigned(2)),
        (2, Value::Unsigned(3)),
        (3, Value::Unsigned(u64::from(u32::MAX) + 1)),
    ]);
    Flush::decode(&flush).unwrap_err();

    let mut extra = CancelWait { request_id: 1 }.payload();
    extra.push((1, Value::Unsigned(2)));
    CancelWait::decode(&map(extra)).unwrap_err();
}
