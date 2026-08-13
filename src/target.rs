//! Presentation-target descriptors and the target-generation advance rules.
//!
//! A session has exactly one presentation target. Its descriptor rides in `WELCOME` key 5 and in
//! every `TARGET_CHANGED`, and its shape is profile-specific. This module owns the
//! `desktop-surface-v1` descriptor (desktop §1) and the generation rules that are common to every
//! profile, so a native presenter and a browser presenter agree byte for byte.

use crate::{
    cbor::Value,
    geometry::{Rotation, TargetExtent},
    messages::{MessageError, PayloadMap, StrictMap, invalid_value},
    revision::TargetGeneration,
};

/// Reason bits on `TARGET_CHANGED`, desktop §1.
pub mod reason {
    pub const VIRTUAL_BOUNDS: u64 = 1 << 0;
    pub const OUTPUT_SET: u64 = 1 << 1;
    pub const OUTPUT_GEOMETRY: u64 = 1 << 2;
    pub const SCALE_OR_ROTATION: u64 = 1 << 3;
    pub const PRESENTATION_WINDOW: u64 = 1 << 4;
    pub const TARGET_RECREATION: u64 = 1 << 5;
    pub const KNOWN_MASK: u64 = (1 << 6) - 1;
}

/// One output in a desktop topology.
///
/// Output IDs are session-local and are not device identities. Desktop §1 forbids monitor serial
/// numbers, user names, desktop names, window titles, and login-session identifiers anywhere in a
/// topology, which is why this type has no room for them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputDescriptor {
    pub output_id: u64,
    pub origin_x: i32,
    pub origin_y: i32,
    pub width: u32,
    pub height: u32,
    pub scale_numerator: u32,
    pub scale_denominator: u32,
    pub rotation: Rotation,
    pub primary: bool,
}

impl OutputDescriptor {
    pub fn encode(&self) -> Value {
        Value::Map(vec![
            (0, Value::Unsigned(self.output_id)),
            (1, signed(i64::from(self.origin_x))),
            (2, signed(i64::from(self.origin_y))),
            (3, Value::Unsigned(u64::from(self.width))),
            (4, Value::Unsigned(u64::from(self.height))),
            (5, Value::Unsigned(u64::from(self.scale_numerator))),
            (6, Value::Unsigned(u64::from(self.scale_denominator))),
            (7, Value::Unsigned(self.rotation as u64)),
            (8, Value::Bool(self.primary)),
        ])
    }

    pub fn decode(value: &Value) -> Result<Self, MessageError> {
        let map = StrictMap::new("output descriptor", value, &[0, 1, 2, 3, 4, 5, 6, 7, 8])?;
        let descriptor = Self {
            output_id: map.required_u64(0)?,
            origin_x: required_i32(&map, 1)?,
            origin_y: required_i32(&map, 2)?,
            width: map.required_u32(3)?,
            height: map.required_u32(4)?,
            scale_numerator: map.required_u32(5)?,
            scale_denominator: map.required_u32(6)?,
            rotation: Rotation::try_from(map.required_u64(7)?)?,
            primary: map.required_bool(8)?,
        };
        if descriptor.width == 0 || descriptor.height == 0 {
            return Err(invalid_value(
                "output descriptor",
                3,
                "output extent must be nonzero",
            ));
        }
        if descriptor.scale_numerator == 0 || descriptor.scale_denominator == 0 {
            return Err(invalid_value(
                "output descriptor",
                5,
                "output scale must be a positive ratio",
            ));
        }
        Ok(descriptor)
    }

    /// Right and bottom edges in virtual logical pixels, with checked arithmetic.
    fn extent(&self) -> Result<(i64, i64), MessageError> {
        let right = i64::from(self.origin_x)
            .checked_add(i64::from(self.width))
            .ok_or_else(|| invalid_value("output descriptor", 1, "origin plus width overflows"))?;
        let bottom = i64::from(self.origin_y)
            .checked_add(i64::from(self.height))
            .ok_or_else(|| invalid_value("output descriptor", 2, "origin plus height overflows"))?;
        Ok((right, bottom))
    }
}

/// The `desktop-surface-v1` target descriptor, desktop §1 keys 0 through 6.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopTarget {
    pub origin_x: i32,
    pub origin_y: i32,
    pub width: u32,
    pub height: u32,
    pub outputs: Vec<OutputDescriptor>,
    /// False while a resize or reconfiguration is still in flight.
    pub settled: bool,
    pub topology_revision: u64,
}

impl DesktopTarget {
    pub fn extent(&self) -> TargetExtent {
        TargetExtent::new(self.width, self.height)
    }

    pub fn encode(&self) -> PayloadMap {
        vec![
            (0, signed(i64::from(self.origin_x))),
            (1, signed(i64::from(self.origin_y))),
            (2, Value::Unsigned(u64::from(self.width))),
            (3, Value::Unsigned(u64::from(self.height))),
            (
                4,
                Value::Array(self.outputs.iter().map(OutputDescriptor::encode).collect()),
            ),
            (5, Value::Bool(self.settled)),
            (6, Value::Unsigned(self.topology_revision)),
        ]
    }

    pub fn decode(map: &PayloadMap) -> Result<Self, MessageError> {
        let value = Value::Map(map.clone());
        let strict = StrictMap::new("desktop target", &value, &[0, 1, 2, 3, 4, 5, 6])?;
        let outputs = match strict.required(4)? {
            Value::Array(entries) => entries
                .iter()
                .map(OutputDescriptor::decode)
                .collect::<Result<Vec<_>, _>>()?,
            _ => {
                return Err(invalid_value(
                    "desktop target",
                    4,
                    "outputs must be an array",
                ));
            }
        };
        let target = Self {
            origin_x: required_i32(&strict, 0)?,
            origin_y: required_i32(&strict, 1)?,
            width: strict.required_u32(2)?,
            height: strict.required_u32(3)?,
            outputs,
            settled: strict.required_bool(5)?,
            topology_revision: strict.required_u64(6)?,
        };
        target.validate()?;
        Ok(target)
    }

    /// Desktop §1's structural rules: nonzero virtual extent, exactly one primary output when the
    /// list is nonempty, unique output IDs, and every output inside the virtual rectangle after
    /// checked transform arithmetic.
    pub fn validate(&self) -> Result<(), MessageError> {
        if self.width == 0 || self.height == 0 {
            return Err(invalid_value(
                "desktop target",
                2,
                "virtual desktop extent must be nonzero",
            ));
        }
        if self.outputs.is_empty() {
            return Ok(());
        }
        if self.outputs.iter().filter(|output| output.primary).count() != 1 {
            return Err(invalid_value(
                "desktop target",
                4,
                "exactly one output is primary when the list is nonempty",
            ));
        }
        let virtual_right = i64::from(self.origin_x)
            .checked_add(i64::from(self.width))
            .ok_or_else(|| {
                invalid_value("desktop target", 0, "virtual origin plus width overflows")
            })?;
        let virtual_bottom = i64::from(self.origin_y)
            .checked_add(i64::from(self.height))
            .ok_or_else(|| {
                invalid_value("desktop target", 1, "virtual origin plus height overflows")
            })?;
        for (index, output) in self.outputs.iter().enumerate() {
            if self.outputs[..index]
                .iter()
                .any(|earlier| earlier.output_id == output.output_id)
            {
                return Err(invalid_value(
                    "desktop target",
                    4,
                    "output IDs must be unique within a topology",
                ));
            }
            let (right, bottom) = output.extent()?;
            if i64::from(output.origin_x) < i64::from(self.origin_x)
                || i64::from(output.origin_y) < i64::from(self.origin_y)
                || right > virtual_right
                || bottom > virtual_bottom
            {
                return Err(invalid_value(
                    "desktop target",
                    4,
                    "output lies outside the virtual desktop rectangle",
                ));
            }
        }
        Ok(())
    }

    pub fn primary(&self) -> Option<&OutputDescriptor> {
        self.outputs.iter().find(|output| output.primary)
    }

    /// Which reason bits distinguish this descriptor from `previous`.
    ///
    /// The settled flag is deliberately not a reason: an unsettled-to-settled transition with no
    /// other change is not a target change at all, per [`TargetTransition`].
    pub fn reason_against(&self, previous: &Self) -> u64 {
        let mut mask = 0;
        if self.origin_x != previous.origin_x
            || self.origin_y != previous.origin_y
            || self.width != previous.width
            || self.height != previous.height
        {
            mask |= reason::VIRTUAL_BOUNDS;
        }
        let previous_ids: Vec<u64> = previous.outputs.iter().map(|out| out.output_id).collect();
        let current_ids: Vec<u64> = self.outputs.iter().map(|out| out.output_id).collect();
        if previous_ids != current_ids {
            mask |= reason::OUTPUT_SET;
        }
        for output in &self.outputs {
            let Some(before) = previous
                .outputs
                .iter()
                .find(|candidate| candidate.output_id == output.output_id)
            else {
                continue;
            };
            if before.origin_x != output.origin_x
                || before.origin_y != output.origin_y
                || before.width != output.width
                || before.height != output.height
            {
                mask |= reason::OUTPUT_GEOMETRY;
            }
            if before.scale_numerator != output.scale_numerator
                || before.scale_denominator != output.scale_denominator
                || before.rotation != output.rotation
            {
                mask |= reason::SCALE_OR_ROTATION;
            }
        }
        mask
    }
}

/// The outcome of offering a new target descriptor to a live session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetTransition {
    /// Nothing changed; no `TARGET_CHANGED` is owed.
    Unchanged,
    /// Only the settled flag flipped from false to true. Core §2.2 is explicit that this repeats
    /// the current generation exactly rather than advancing it, so a producer can distinguish
    /// "the resize finished" from "the target moved again".
    Settled { generation: TargetGeneration },
    /// A real change: the generation advances and the reason mask says what moved.
    Advanced {
        generation: TargetGeneration,
        reason: u64,
    },
}

/// Owns the current descriptor and applies core §2.2's generation rules.
#[derive(Debug, Clone)]
pub struct DesktopTargetState {
    current: DesktopTarget,
    generation: TargetGeneration,
}

impl DesktopTargetState {
    pub fn new(target: DesktopTarget) -> Result<Self, MessageError> {
        target.validate()?;
        Ok(Self {
            current: target,
            generation: TargetGeneration::ONE,
        })
    }

    pub fn current(&self) -> &DesktopTarget {
        &self.current
    }

    pub fn generation(&self) -> TargetGeneration {
        self.generation
    }

    /// Offer a new descriptor and learn what the session owes its producers.
    pub fn offer(&mut self, next: DesktopTarget) -> Result<TargetTransition, MessageError> {
        next.validate()?;
        if next == self.current {
            return Ok(TargetTransition::Unchanged);
        }
        // The settled-repeat exception: identical in every field except a false-to-true settled
        // flag. Compare with the flag normalized so an accompanying geometry change still counts
        // as a real change.
        let mut normalized = next.clone();
        normalized.settled = self.current.settled;
        if normalized == self.current && !self.current.settled && next.settled {
            self.current = next;
            return Ok(TargetTransition::Settled {
                generation: self.generation,
            });
        }
        let reason = next.reason_against(&self.current);
        self.generation = self
            .generation
            .advance()
            .map_err(|_| invalid_value("desktop target", 6, "target generation is exhausted"))?;
        self.current = next;
        Ok(TargetTransition::Advanced {
            generation: self.generation,
            reason,
        })
    }
}

fn required_i32(map: &StrictMap<'_>, key: u64) -> Result<i32, MessageError> {
    map.required(key)?
        .as_i64()
        .and_then(|value| i32::try_from(value).ok())
        .ok_or_else(|| invalid_value("desktop target", key, "must fit in a signed 32-bit integer"))
}

fn signed(value: i64) -> Value {
    if value >= 0 {
        Value::Unsigned(value as u64)
    } else {
        Value::Negative(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output(id: u64, origin_x: i32, width: u32, primary: bool) -> OutputDescriptor {
        OutputDescriptor {
            output_id: id,
            origin_x,
            origin_y: 0,
            width,
            height: 1080,
            scale_numerator: 1,
            scale_denominator: 1,
            rotation: Rotation::None,
            primary,
        }
    }

    fn single() -> DesktopTarget {
        DesktopTarget {
            origin_x: 0,
            origin_y: 0,
            width: 1920,
            height: 1080,
            outputs: vec![output(1, 0, 1920, true)],
            settled: true,
            topology_revision: 1,
        }
    }

    fn dual() -> DesktopTarget {
        DesktopTarget {
            width: 3840,
            outputs: vec![output(1, 0, 1920, true), output(2, 1920, 1920, false)],
            ..single()
        }
    }

    #[test]
    fn descriptor_round_trips() {
        for target in [single(), dual()] {
            assert_eq!(DesktopTarget::decode(&target.encode()).unwrap(), target);
        }
    }

    #[test]
    fn a_descriptor_carries_no_device_identity() {
        // Desktop §1 forbids serials, user names, desktop names, window titles, and login-session
        // identifiers. The encoding is all integers and one boolean, so there is nowhere to put
        // them — assert that structurally rather than by scanning strings.
        for (_, value) in dual().encode() {
            let leaves = match value {
                Value::Array(entries) => entries,
                other => vec![other],
            };
            for leaf in leaves {
                match leaf {
                    Value::Map(entries) => assert!(
                        entries
                            .iter()
                            .all(|(_, item)| !matches!(item, Value::Text(_) | Value::Bytes(_))),
                        "an output descriptor carried free-form data"
                    ),
                    Value::Text(_) | Value::Bytes(_) => panic!("target descriptor carried text"),
                    _ => {}
                }
            }
        }
    }

    #[test]
    fn outputs_must_lie_inside_the_virtual_rectangle() {
        let mut target = single();
        target.outputs[0].width = 2560;
        assert!(target.validate().is_err());
    }

    #[test]
    fn exactly_one_output_is_primary() {
        let mut none_primary = dual();
        none_primary.outputs[0].primary = false;
        assert!(none_primary.validate().is_err());

        let mut both_primary = dual();
        both_primary.outputs[1].primary = true;
        assert!(both_primary.validate().is_err());
    }

    #[test]
    fn output_ids_are_unique_and_extents_are_nonzero() {
        let mut duplicate = dual();
        duplicate.outputs[1].output_id = 1;
        assert!(duplicate.validate().is_err());

        let mut empty = single();
        empty.outputs[0].width = 0;
        assert!(DesktopTarget::decode(&empty.encode()).is_err());

        let mut zero_scale = single();
        zero_scale.outputs[0].scale_denominator = 0;
        assert!(DesktopTarget::decode(&zero_scale.encode()).is_err());
    }

    #[test]
    fn an_empty_topology_is_allowed() {
        let headless = DesktopTarget {
            outputs: Vec::new(),
            ..single()
        };
        assert!(headless.validate().is_ok());
    }

    #[test]
    fn settling_repeats_the_generation_exactly() {
        let unsettled = DesktopTarget {
            settled: false,
            ..single()
        };
        let mut state = DesktopTargetState::new(unsettled).unwrap();
        assert_eq!(state.generation(), TargetGeneration::ONE);

        // Core §2.2: the final unsettled-to-settled transition of an otherwise byte-identical
        // descriptor repeats the current generation rather than advancing it.
        let transition = state.offer(single()).unwrap();
        assert_eq!(
            transition,
            TargetTransition::Settled {
                generation: TargetGeneration::ONE
            }
        );
        assert_eq!(state.generation(), TargetGeneration::ONE);
    }

    #[test]
    fn settling_together_with_a_real_change_still_advances() {
        let unsettled = DesktopTarget {
            settled: false,
            ..single()
        };
        let mut state = DesktopTargetState::new(unsettled).unwrap();
        let grown = DesktopTarget {
            settled: true,
            ..dual()
        };
        let transition = state.offer(grown).unwrap();
        match transition {
            TargetTransition::Advanced { generation, reason } => {
                assert_eq!(generation.get(), 2);
                assert!(reason & reason::VIRTUAL_BOUNDS != 0);
                assert!(reason & reason::OUTPUT_SET != 0);
            }
            other => panic!("expected an advance, got {other:?}"),
        }
    }

    #[test]
    fn an_identical_descriptor_owes_nothing() {
        let mut state = DesktopTargetState::new(single()).unwrap();
        assert_eq!(state.offer(single()).unwrap(), TargetTransition::Unchanged);
        assert_eq!(state.generation(), TargetGeneration::ONE);
    }

    #[test]
    fn scale_and_rotation_changes_are_reported_separately_from_geometry() {
        let mut state = DesktopTargetState::new(single()).unwrap();
        let scaled = DesktopTarget {
            outputs: vec![OutputDescriptor {
                scale_numerator: 3,
                scale_denominator: 2,
                ..output(1, 0, 1920, true)
            }],
            ..single()
        };
        match state.offer(scaled).unwrap() {
            TargetTransition::Advanced { reason, .. } => {
                assert_eq!(
                    reason & reason::SCALE_OR_ROTATION,
                    reason::SCALE_OR_ROTATION
                );
                assert_eq!(reason & reason::OUTPUT_GEOMETRY, 0);
                assert_eq!(reason & reason::VIRTUAL_BOUNDS, 0);
            }
            other => panic!("expected an advance, got {other:?}"),
        }
    }

    #[test]
    fn a_rotated_output_reports_scale_or_rotation() {
        let mut state = DesktopTargetState::new(single()).unwrap();
        let rotated = DesktopTarget {
            outputs: vec![OutputDescriptor {
                rotation: Rotation::Ninety,
                ..output(1, 0, 1920, true)
            }],
            ..single()
        };
        match state.offer(rotated).unwrap() {
            TargetTransition::Advanced { reason, .. } => {
                assert!(reason & reason::SCALE_OR_ROTATION != 0);
            }
            other => panic!("expected an advance, got {other:?}"),
        }
    }

    #[test]
    fn an_invalid_offer_leaves_the_state_untouched() {
        let mut state = DesktopTargetState::new(single()).unwrap();
        let mut broken = dual();
        broken.outputs[1].primary = true;
        assert!(state.offer(broken).is_err());
        assert_eq!(state.current(), &single());
        assert_eq!(state.generation(), TargetGeneration::ONE);
    }

    #[test]
    fn the_extent_feeds_normalized_projection() {
        let state = DesktopTargetState::new(dual()).unwrap();
        assert_eq!(state.current().extent(), TargetExtent::new(3840, 1080));
        assert_eq!(state.current().primary().unwrap().output_id, 1);
    }

    #[test]
    fn decoding_rejects_unknown_keys() {
        let mut map = single().encode();
        map.push((7, Value::Unsigned(0)));
        assert!(DesktopTarget::decode(&map).is_err());
    }
}
