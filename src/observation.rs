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
    /// Observation class 0: surface changes.
    pub const SURFACE: u64 = 1 << 0;
    /// Observation class 1: track changes.
    pub const TRACK: u64 = 1 << 1;
    /// Observation class 2: scene changes.
    pub const SCENE: u64 = 1 << 2;
    /// Observation class 3: playback transitions.
    pub const PLAYBACK: u64 = 1 << 3;
    /// Observation class 4: lease and context changes visible to the principal.
    pub const AUTHORITY: u64 = 1 << 4;
    /// Every assigned observation class bit.
    pub const KNOWN_MASK: u64 = (1 << 5) - 1;
}

/// Records that must be delivered rather than coalesced.
pub fn is_actionable(record_type: u16) -> bool {
    matches!(
        record_type,
        messages::TARGET_CHANGED
            | messages::PLAYBACK_HOLD
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

/// The identity an observation coalesces on.
///
/// The same record about the same object replaces the earlier one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObservationKey {
    /// The observation's record type.
    pub record_type: u16,
    /// The context the object belongs to.
    pub context_id: u64,
    /// The object the observation is about.
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
    /// Record type to write.
    pub record_type: u16,
    /// Record object ID.
    pub object_id: u64,
    /// Envelope payload; the queue adds the observation sequence.
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
    /// Creates an unsubscribed queue holding at most `capacity` observations, and at least one.
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
    ///
    /// # Errors
    ///
    /// Returns [`messages::MessageError::InvalidValue`] when `mask` has unassigned class bits.
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

    /// Returns the subscribed observation classes.
    pub fn mask(&self) -> u64 {
        self.mask
    }

    /// Returns whether nothing, not even a gap report, is waiting.
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
            let classes = dropped.map_or(0, |entry| entry.class);
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

/// The `SET_OBSERVATION` payload: replace the session's observation mask (core §10).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetObservation {
    /// The [`class`] bits to subscribe to; [`ObservationQueue::subscribe`] rejects unknown bits.
    pub mask: u64,
}

impl SetObservation {
    /// Encodes the payload map.
    pub fn payload(&self) -> PayloadMap {
        vec![(0, Value::Unsigned(self.mask))]
    }

    /// Decodes a `SET_OBSERVATION` payload.
    ///
    /// # Errors
    ///
    /// Returns [`messages::MessageError`] when `value` is not a strict `SET_OBSERVATION` map or the
    /// mask is missing or not an unsigned integer.
    pub fn decode(value: &Value) -> Result<Self, messages::MessageError> {
        let map = messages::StrictMap::new("SET_OBSERVATION", value, &[0])?;
        Ok(Self {
            mask: map.required_u64(0)?,
        })
    }
}
