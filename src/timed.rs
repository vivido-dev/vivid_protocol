//! Negotiated synchronized timed playback. Positions are clock observations, never ingress ACKs.
use crate::{
    cbor::Value,
    messages::{PayloadMap, StrictMap},
    track::TrackAddress,
};
use std::io;

pub const PRESENTATION_RESUMED: u64 = 6;
pub const HOLD_NOT_VISIBLE: u64 = 1;
pub const HOLD_DETACHED: u64 = 2;
pub const HOLD_DOWNSTREAM: u64 = 4;
pub const HOLD_POLICY: u64 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NeedKeyframe {
    pub address: TrackAddress,
    pub minimum_epoch: u32,
    pub reason: u64,
    pub presentation_pts_us: Option<i64>,
    pub hold_serial: Option<u64>,
}

impl NeedKeyframe {
    pub fn decode(value: &Value) -> io::Result<Self> {
        let map = StrictMap::new("NEED_KEYFRAME", value, &[0, 1, 2, 3, 4, 5, 6, 7, 8])?;
        map.optional_u64(6)?;
        let request = Self {
            address: TrackAddress {
                context_id: map.required_u64(0)?,
                surface_id: map.required_u64(1)?,
                track_id: map.required_u64(2)?,
                channel_generation: crate::revision::ChannelGeneration::new(map.required_u64(3)?),
            },
            minimum_epoch: map.required_u32(4)?,
            reason: map.required_u64(5)?,
            presentation_pts_us: map
                .optional(7)
                .map(|v| v.as_i64().ok_or_else(|| invalid("invalid recovery PTS")))
                .transpose()?,
            hold_serial: map.optional_u64(8)?,
        };
        if request.address.context_id == 0
            || request.address.surface_id == 0
            || request.address.track_id == 0
            || request.address.channel_generation.get() == 0
            || !(1..=PRESENTATION_RESUMED).contains(&request.reason)
            || request.hold_serial == Some(0)
            || (request.reason == PRESENTATION_RESUMED && request.hold_serial.is_none())
            || (request.reason != PRESENTATION_RESUMED
                && (request.hold_serial.is_some() || request.presentation_pts_us.is_some()))
        {
            return Err(invalid("invalid recovery reason or correlation"));
        }
        Ok(request)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StartPolicy {
    #[default]
    AfterMinimumBuffer,
    Synchronized,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayOptions {
    pub start_pts_us: i64,
    pub minimum_buffer_us: u64,
    pub maximum_latency_us: u64,
    pub start_policy: StartPolicy,
    /// Correlates a resume with the current surface hold; absent for ordinary starts/seeks.
    pub hold_serial: Option<u64>,
}

impl PlayOptions {
    pub fn decode(value: &Value) -> io::Result<(TrackAddress, Self)> {
        let map = StrictMap::new("PLAY", value, &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11])?;
        let address = TrackAddress {
            context_id: map.required_u64(0)?,
            surface_id: map.required_u64(1)?,
            track_id: map.required_u64(2)?,
            channel_generation: crate::revision::ChannelGeneration::new(map.required_u64(10)?),
        };
        if map.required_u64(6)? != 1 << 32 || map.required_u64(7)? != 1 || map.required_u64(8)? != 0
        {
            return Err(invalid("unsupported PLAY rate, late policy or loop count"));
        }
        let options = Self {
            start_pts_us: map
                .required(3)?
                .as_i64()
                .ok_or_else(|| invalid("invalid PLAY PTS"))?,
            minimum_buffer_us: map.required_u64(4)?,
            maximum_latency_us: map.required_u64(5)?,
            start_policy: match map.required_u64(9)? {
                1 => StartPolicy::AfterMinimumBuffer,
                2 => StartPolicy::Synchronized,
                _ => return Err(invalid("unsupported PLAY start policy")),
            },
            hold_serial: map.optional_u64(11)?,
        };
        options.payload(address)?;
        Ok((address, options))
    }

    pub fn payload(self, address: TrackAddress) -> io::Result<PayloadMap> {
        if address.context_id == 0
            || address.surface_id == 0
            || address.track_id == 0
            || address.channel_generation.get() == 0
            || self.minimum_buffer_us > self.maximum_latency_us
            || self.hold_serial == Some(0)
            || (self.hold_serial.is_some() && self.start_policy != StartPolicy::Synchronized)
        {
            return Err(invalid("invalid timed playback options"));
        }
        let mut fields = vec![
            (0, Value::Unsigned(address.context_id)),
            (1, Value::Unsigned(address.surface_id)),
            (2, Value::Unsigned(address.track_id)),
        ];
        fields.extend([
            (3, signed(self.start_pts_us)),
            (4, Value::Unsigned(self.minimum_buffer_us)),
            (5, Value::Unsigned(self.maximum_latency_us)),
            (6, Value::Unsigned(1 << 32)),
            (7, Value::Unsigned(1)),
            (8, Value::Unsigned(0)),
            (
                9,
                Value::Unsigned(match self.start_policy {
                    StartPolicy::AfterMinimumBuffer => 1,
                    StartPolicy::Synchronized => 2,
                }),
            ),
            (10, Value::Unsigned(address.channel_generation.get())),
        ]);
        if let Some(serial) = self.hold_serial {
            fields.push((11, Value::Unsigned(serial)));
        }
        Ok(fields)
    }
}

/// The clock identity qualifies the PTS across independent audio and video epochs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeldPosition {
    pub track_id: u64,
    pub channel_generation: u64,
    pub epoch: u32,
    pub pts_us: i64,
    pub estimated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaybackHold {
    pub context_id: u64,
    pub surface_id: u64,
    pub serial: u64,
    pub held: bool,
    pub reasons: u64,
    pub playing_intent: bool,
    pub recovery_required: bool,
    pub position: Option<HeldPosition>,
}

impl PlaybackHold {
    pub fn payload(&self) -> io::Result<PayloadMap> {
        if self.context_id == 0
            || self.surface_id == 0
            || self.serial == 0
            || self.reasons & !15 != 0
            || (self.held && self.reasons == 0)
            || (!self.held && self.reasons != 0)
            || self
                .position
                .is_some_and(|p| p.track_id == 0 || p.channel_generation == 0)
        {
            return Err(invalid("invalid playback hold"));
        }
        let mut fields = vec![
            (0, Value::Unsigned(self.context_id)),
            (1, Value::Unsigned(self.surface_id)),
            (2, Value::Unsigned(self.serial)),
            (3, Value::Bool(self.held)),
            (4, Value::Unsigned(self.reasons)),
            (5, Value::Bool(self.playing_intent)),
            (6, Value::Bool(self.recovery_required)),
        ];
        if let Some(p) = self.position {
            fields.push((
                7,
                Value::Map(vec![
                    (0, Value::Unsigned(p.track_id)),
                    (1, Value::Unsigned(p.channel_generation)),
                    (2, Value::Unsigned(u64::from(p.epoch))),
                    (3, signed(p.pts_us)),
                    (4, Value::Bool(p.estimated)),
                ]),
            ));
        }
        Ok(fields)
    }

    pub fn decode(value: &Value) -> io::Result<Self> {
        let map = StrictMap::new("PLAYBACK_HOLD", value, &[0, 1, 2, 3, 4, 5, 6, 7])?;
        let position = map
            .optional(7)
            .map(|value| -> io::Result<_> {
                let p = StrictMap::new("held position", value, &[0, 1, 2, 3, 4])?;
                Ok(HeldPosition {
                    track_id: p.required_u64(0)?,
                    channel_generation: p.required_u64(1)?,
                    epoch: p.required_u32(2)?,
                    pts_us: p
                        .required(3)?
                        .as_i64()
                        .ok_or_else(|| invalid("invalid held PTS"))?,
                    estimated: p.required_bool(4)?,
                })
            })
            .transpose()?;
        let hold = Self {
            context_id: map.required_u64(0)?,
            surface_id: map.required_u64(1)?,
            serial: map.required_u64(2)?,
            held: map.required_bool(3)?,
            reasons: map.required_u64(4)?,
            playing_intent: map.required_bool(5)?,
            recovery_required: map.required_bool(6)?,
            position,
        };
        hold.payload()?;
        Ok(hold)
    }
}

fn signed(value: i64) -> Value {
    match u64::try_from(value) {
        Ok(value) => Value::Unsigned(value),
        Err(_) => Value::Negative(value),
    }
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resume_correlation_is_distinct_from_decoder_failure() {
        let address = TrackAddress {
            context_id: 1,
            surface_id: 2,
            track_id: 3,
            channel_generation: crate::revision::ChannelGeneration::new(4),
        };
        let mut fields = crate::track::need_keyframe_payload(address, 5, PRESENTATION_RESUMED);
        assert!(NeedKeyframe::decode(&Value::Map(fields.clone())).is_err());
        fields.extend([(7, signed(123)), (8, Value::Unsigned(9))]);
        let request = NeedKeyframe::decode(&Value::Map(fields.clone())).unwrap();
        assert_eq!(request.presentation_pts_us, Some(123));
        assert_eq!(request.hold_serial, Some(9));
        fields.iter_mut().find(|(key, _)| *key == 5).unwrap().1 = Value::Unsigned(2);
        assert!(NeedKeyframe::decode(&Value::Map(fields)).is_err());
        let mut play = PlayOptions {
            start_pts_us: 123,
            minimum_buffer_us: 1,
            maximum_latency_us: 2,
            start_policy: StartPolicy::Synchronized,
            hold_serial: Some(9),
        };
        let encoded = Value::Map(play.payload(address).unwrap());
        assert_eq!(PlayOptions::decode(&encoded).unwrap(), (address, play));
        let mut invalid_fields = play.payload(address).unwrap();
        invalid_fields
            .iter_mut()
            .find(|(key, _)| *key == 6)
            .unwrap()
            .1 = Value::Unsigned(2 << 32);
        assert!(PlayOptions::decode(&Value::Map(invalid_fields)).is_err());
        play.start_policy = StartPolicy::AfterMinimumBuffer;
        assert!(play.payload(address).is_err());
    }

    #[test]
    fn held_position_is_optional_and_qualified() {
        let mut hold = PlaybackHold {
            context_id: 1,
            surface_id: 2,
            serial: 1,
            held: true,
            reasons: HOLD_NOT_VISIBLE,
            playing_intent: true,
            recovery_required: true,
            position: None,
        };
        for position in [
            None,
            Some(HeldPosition {
                track_id: 3,
                channel_generation: 4,
                epoch: 5,
                pts_us: -6,
                estimated: true,
            }),
        ] {
            hold.position = position;
            let payload = hold.payload().unwrap();
            assert_eq!(PlaybackHold::decode(&Value::Map(payload)).unwrap(), hold);
        }
        hold.serial = 0;
        assert!(hold.payload().is_err());
    }

    #[test]
    fn hold_rejects_unknown_fields_and_invalid_clock_identity() {
        let hold = PlaybackHold {
            context_id: 1,
            surface_id: 2,
            serial: 1,
            held: true,
            reasons: HOLD_NOT_VISIBLE,
            playing_intent: false,
            recovery_required: true,
            position: None,
        };
        let mut fields = hold.payload().unwrap();
        fields.push((8, Value::Bool(true)));
        assert!(PlaybackHold::decode(&Value::Map(fields)).is_err());
        let mut bad = hold;
        bad.position = Some(HeldPosition {
            track_id: 0,
            channel_generation: 1,
            epoch: 0,
            pts_us: 0,
            estimated: false,
        });
        assert!(bad.payload().is_err());
    }
}
