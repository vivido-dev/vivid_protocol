//! Public-API tests for `vivid_protocol::observation`.
use vivid_protocol::observation::*;
use vivid_protocol::{cbor::Value, messages, messages::PayloadMap};

fn key(record_type: u16, object_id: u64) -> ObservationKey {
    ObservationKey {
        record_type,
        context_id: 1,
        object_id,
    }
}

fn payload(value: u64) -> PayloadMap {
    vec![(0, Value::Unsigned(value))]
}

fn subscribed(capacity: u64, mask: u64) -> ObservationQueue {
    let mut queue = ObservationQueue::new(capacity);
    queue.subscribe(mask).unwrap();
    queue
}

#[test]
fn an_unsubscribed_class_is_not_queued() {
    let mut queue = subscribed(8, class::TRACK);
    assert!(!queue.push(
        class::SURFACE,
        key(messages::SURFACE_CHANGED, 1),
        payload(1)
    ));
    assert!(queue.is_empty());
}

#[test]
fn unassigned_mask_bits_are_refused() {
    let mut queue = ObservationQueue::new(8);
    assert!(queue.subscribe(1 << 9).is_err());
    assert_eq!(queue.mask(), 0);
}

#[test]
fn the_same_object_coalesces_latest_wins() {
    let mut queue = subscribed(8, class::SURFACE);
    queue.push(
        class::SURFACE,
        key(messages::SURFACE_CHANGED, 1),
        payload(1),
    );
    queue.push(
        class::SURFACE,
        key(messages::SURFACE_CHANGED, 1),
        payload(2),
    );
    let drained = queue.drain();
    assert_eq!(drained.len(), 1, "one entry per object");
    assert_eq!(
        drained[0].payload[0].1.as_u64(),
        Some(2),
        "the latest value wins"
    );
}

#[test]
fn different_objects_do_not_coalesce() {
    let mut queue = subscribed(8, class::SURFACE);
    queue.push(
        class::SURFACE,
        key(messages::SURFACE_CHANGED, 1),
        payload(1),
    );
    queue.push(
        class::SURFACE,
        key(messages::SURFACE_CHANGED, 2),
        payload(2),
    );
    assert_eq!(queue.drain().len(), 2);
}

#[test]
fn every_observation_carries_a_strictly_increasing_sequence() {
    let mut queue = subscribed(8, class::SURFACE);
    for object in 1..=3 {
        queue.push(
            class::SURFACE,
            key(messages::SURFACE_CHANGED, object),
            payload(object),
        );
    }
    let sequences = queue
        .drain()
        .into_iter()
        .map(|observation| {
            observation
                .payload
                .iter()
                .find(|entry| entry.0 == OBSERVATION_SEQUENCE_KEY)
                .and_then(|entry| entry.1.as_u64())
                .unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(sequences, vec![1, 2, 3]);
}

#[test]
fn overflow_drops_and_reports_a_gap_before_what_follows() {
    let mut queue = subscribed(2, class::SURFACE | class::TRACK);
    queue.push(
        class::SURFACE,
        key(messages::SURFACE_CHANGED, 1),
        payload(1),
    );
    queue.push(class::TRACK, key(messages::TRACK_CHANGED, 2), payload(2));
    // The third entry evicts the first.
    queue.push(
        class::SURFACE,
        key(messages::SURFACE_CHANGED, 3),
        payload(3),
    );

    let drained = queue.drain();
    assert_eq!(
        drained[0].record_type,
        messages::OBSERVATION_GAP,
        "the gap comes first"
    );
    assert_eq!(
        drained[0].payload[0].1.as_u64(),
        Some(1),
        "the first lost sequence"
    );
    assert_eq!(
        drained[0].payload[1].1.as_u64(),
        Some(class::SURFACE),
        "the classes the loss spanned"
    );
    assert_eq!(drained.len(), 3, "the gap plus the two survivors");
}

#[test]
fn repeated_losses_collapse_into_one_gap_naming_every_class() {
    let mut queue = subscribed(1, class::SURFACE | class::TRACK);
    queue.push(
        class::SURFACE,
        key(messages::SURFACE_CHANGED, 1),
        payload(1),
    );
    queue.push(class::TRACK, key(messages::TRACK_CHANGED, 2), payload(2));
    queue.push(
        class::SURFACE,
        key(messages::SURFACE_CHANGED, 3),
        payload(3),
    );
    let drained = queue.drain();
    assert_eq!(drained[0].record_type, messages::OBSERVATION_GAP);
    assert_eq!(
        drained[0].payload[1].1.as_u64(),
        Some(class::SURFACE | class::TRACK),
        "one gap names every class that lost an entry"
    );
    assert_eq!(drained.len(), 2, "the gap plus the one survivor");
}

#[test]
fn unsubscribing_discards_what_was_queued_for_that_class() {
    let mut queue = subscribed(8, class::SURFACE | class::TRACK);
    queue.push(
        class::SURFACE,
        key(messages::SURFACE_CHANGED, 1),
        payload(1),
    );
    queue.push(class::TRACK, key(messages::TRACK_CHANGED, 2), payload(2));
    queue.subscribe(class::TRACK).unwrap();
    let drained = queue.drain();
    assert_eq!(drained.len(), 1);
    assert_eq!(drained[0].record_type, messages::TRACK_CHANGED);
}

#[test]
fn the_actionable_set_is_recognised() {
    for record in [
        messages::TARGET_CHANGED,
        messages::TRACK_LOST,
        messages::NEED_KEYFRAME,
        messages::NEED_FULL_FRAME,
        messages::MAX_CHANNEL_DATA,
        messages::INPUT_REVOKED,
        messages::CONTEXT_CHANGED,
        messages::SESSION_LEASE_CHANGED,
        messages::ERROR,
    ] {
        assert!(
            is_actionable(record),
            "{record:#06x} must never be coalesced"
        );
    }
    for record in [
        messages::SURFACE_CHANGED,
        messages::TRACK_CHANGED,
        messages::SCENE_CHANGED,
    ] {
        assert!(!is_actionable(record));
    }
}

#[test]
fn a_drained_queue_reports_empty() {
    let mut queue = subscribed(4, class::SURFACE);
    queue.push(
        class::SURFACE,
        key(messages::SURFACE_CHANGED, 1),
        payload(1),
    );
    assert!(!queue.is_empty());
    queue.drain();
    assert!(queue.is_empty());
    assert!(queue.drain_next().is_none());
}
