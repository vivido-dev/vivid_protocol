//! Bounded, coalescing observation delivery.
//!
//! Core §10 makes observations *non-actionable*: they may be coalesced latest-wins and dropped
//! under bounded writer pressure, and current truth is recovered by query. `OBSERVATION_GAP` names
//! the first lost sequence and the affected classes so a producer knows to re-query rather than
//! believing a stale value.
//!
//! The actionable set — `TARGET_CHANGED`, `TRACK_LOST`, `NEED_KEYFRAME`, `NEED_FULL_FRAME`,
//! `MAX_CHANNEL_DATA`, the input records, `CONTEXT_CHANGED`, `SESSION_LEASE_CHANGED`, `PING`,
//! `PONG`, and fatal `ERROR` — never enters this queue. [`is_actionable`] is the guard, so a
//! caller cannot quietly coalesce something that must be delivered.

use std::collections::VecDeque;

use crate::{cbor::Value, messages, messages::PayloadMap};

/// Observation classes, core §10's `SET_OBSERVATION` mask.
pub mod class {
    pub const SURFACE: u64 = 1 << 0;
    pub const TRACK: u64 = 1 << 1;
    pub const SCENE: u64 = 1 << 2;
    pub const PLAYBACK: u64 = 1 << 3;
    pub const AUTHORITY: u64 = 1 << 4;
    pub const KNOWN_MASK: u64 = (1 << 5) - 1;
}

/// Records that must be delivered rather than coalesced.
pub fn is_actionable(record_type: u16) -> bool {
    matches!(
        record_type,
        messages::TARGET_CHANGED
            | messages::TRACK_LOST
            | messages::NEED_KEYFRAME
            | messages::NEED_FULL_FRAME
            | messages::MAX_CHANNEL_DATA
            | messages::INPUT_BOUND
            | messages::INPUT_LEASE_RENEW
            | messages::INPUT_REVOKED
            | messages::INPUT_RESET
            | messages::CONTEXT_CHANGED
            | messages::SESSION_LEASE_CHANGED
            | messages::PING
            | messages::PONG
            | messages::ERROR
    )
}

/// What an observation coalesces on: the same record about the same object replaces the earlier one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObservationKey {
    pub record_type: u16,
    pub context_id: u64,
    pub object_id: u64,
}

#[derive(Debug, Clone)]
struct Entry {
    key: ObservationKey,
    class: u64,
    payload: PayloadMap,
}

/// One observation ready to write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observation {
    pub record_type: u16,
    pub object_id: u64,
    pub payload: PayloadMap,
}

/// A session's observation subscription and its bounded queue.
#[derive(Debug)]
pub struct ObservationQueue {
    mask: u64,
    capacity: usize,
    sequence: u64,
    entries: VecDeque<Entry>,
    /// The first sequence lost since the last gap was reported, and the classes it spanned.
    lost: Option<(u64, u64)>,
}

impl ObservationQueue {
    pub fn new(capacity: u64) -> Self {
        Self {
            mask: 0,
            capacity: usize::try_from(capacity).unwrap_or(usize::MAX).max(1),
            sequence: 0,
            entries: VecDeque::new(),
            lost: None,
        }
    }

    /// Replace the subscription mask, core §10's `SET_OBSERVATION`.
    ///
    /// Unsubscribing discards anything already queued for the classes that were dropped: a
    /// producer that stopped listening should not receive them later.
    pub fn subscribe(&mut self, mask: u64) -> Result<(), messages::MessageError> {
        if mask & !class::KNOWN_MASK != 0 {
            return Err(messages::invalid_value(
                "SET_OBSERVATION",
                0,
                "has unassigned observation class bits",
            ));
        }
        self.mask = mask;
        self.entries.retain(|entry| entry.class & mask != 0);
        Ok(())
    }

    pub fn mask(&self) -> u64 {
        self.mask
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty() && self.lost.is_none()
    }

    /// Queue one observation, coalescing latest-wins on its key.
    ///
    /// Returns false when the class is not subscribed, which is not a loss.
    pub fn push(&mut self, class: u64, key: ObservationKey, payload: PayloadMap) -> bool {
        debug_assert!(
            !is_actionable(key.record_type),
            "an actionable record must not be coalesced"
        );
        if self.mask & class == 0 {
            return false;
        }
        if let Some(existing) = self.entries.iter_mut().find(|entry| entry.key == key) {
            existing.payload = payload;
            return true;
        }
        if self.entries.len() >= self.capacity {
            // Dropping the oldest is what "dropped under bounded writer pressure" means; the gap
            // tells the producer to re-query rather than trust what it last saw.
            let dropped = self.entries.pop_front();
            let classes = dropped.map(|entry| entry.class).unwrap_or(0);
            self.note_loss(classes);
        }
        self.entries.push_back(Entry {
            key,
            class,
            payload,
        });
        true
    }

    fn note_loss(&mut self, classes: u64) {
        match &mut self.lost {
            Some((_, seen)) => *seen |= classes,
            None => self.lost = Some((self.sequence.saturating_add(1), classes)),
        }
    }

    /// Take the next record to write, or `None` when the queue is drained.
    ///
    /// A pending gap is reported first, because it describes everything before what follows.
    pub fn drain_next(&mut self) -> Option<Observation> {
        if let Some((first_lost, classes)) = self.lost.take() {
            self.sequence = self.sequence.saturating_add(1);
            return Some(Observation {
                record_type: messages::OBSERVATION_GAP,
                object_id: 0,
                payload: vec![
                    (0, Value::Unsigned(first_lost)),
                    (1, Value::Unsigned(classes)),
                    (2, Value::Unsigned(self.sequence)),
                ],
            });
        }
        let entry = self.entries.pop_front()?;
        self.sequence = self.sequence.saturating_add(1);
        let mut payload = entry.payload;
        payload.push((OBSERVATION_SEQUENCE_KEY, Value::Unsigned(self.sequence)));
        Some(Observation {
            record_type: entry.key.record_type,
            object_id: entry.key.object_id,
            payload,
        })
    }

    /// Everything currently queued, in order.
    pub fn drain(&mut self) -> Vec<Observation> {
        let mut drained = Vec::new();
        while let Some(observation) = self.drain_next() {
            drained.push(observation);
        }
        drained
    }
}

/// Where the observation sequence rides in every observation payload.
pub const OBSERVATION_SEQUENCE_KEY: u64 = 90;

#[cfg(test)]
mod tests {
    use super::*;

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
}
