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
    /// `TARGET_CHANGED` reason bit 0: the virtual-desktop bounds changed.
    pub const VIRTUAL_BOUNDS: u64 = 1 << 0;
    /// Reason bit 1: an output was added or removed.
    pub const OUTPUT_SET: u64 = 1 << 1;
    /// Reason bit 2: an output's geometry changed.
    pub const OUTPUT_GEOMETRY: u64 = 1 << 2;
    /// Reason bit 3: an output's scale or rotation changed.
    pub const SCALE_OR_ROTATION: u64 = 1 << 3;
    /// Reason bit 4: the presentation window changed.
    pub const PRESENTATION_WINDOW: u64 = 1 << 4;
    /// Reason bit 5: the target was recreated.
    pub const TARGET_RECREATION: u64 = 1 << 5;
    /// Every assigned reason bit.
    pub const KNOWN_MASK: u64 = (1 << 6) - 1;
}

/// One output in a desktop topology.
///
/// Output IDs are session-local and are not device identities. Desktop §1 forbids monitor serial
/// numbers, user names, desktop names, window titles, and login-session identifiers anywhere in a
/// topology, which is why this type has no room for them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputDescriptor {
    /// Session-local output ID; never a device identity (key 0).
    pub output_id: u64,
    /// Origin X in virtual logical pixels (key 1).
    pub origin_x: i32,
    /// Origin Y in virtual logical pixels (key 2).
    pub origin_y: i32,
    /// Logical width (key 3).
    pub width: u32,
    /// Logical height (key 4).
    pub height: u32,
    /// Scale numerator (key 5).
    pub scale_numerator: u32,
    /// Scale denominator (key 6).
    pub scale_denominator: u32,
    /// Clockwise rotation (key 7).
    pub rotation: Rotation,
    /// Whether this is the primary output (key 8).
    pub primary: bool,
}

impl OutputDescriptor {
    /// Encodes the output as its descriptor map.
    pub fn encode(&self) -> Value {
        Value::Map(vec![
            (0, Value::Unsigned(self.output_id)),
            (1, Value::from(i64::from(self.origin_x))),
            (2, Value::from(i64::from(self.origin_y))),
            (3, Value::Unsigned(u64::from(self.width))),
            (4, Value::Unsigned(u64::from(self.height))),
            (5, Value::Unsigned(u64::from(self.scale_numerator))),
            (6, Value::Unsigned(u64::from(self.scale_denominator))),
            (7, Value::Unsigned(self.rotation as u64)),
            (8, Value::Bool(self.primary)),
        ])
    }

    /// Decodes one output descriptor of a desktop topology.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError`] when `value` is not a strict output map, a field is missing, has
    /// the wrong type, or does not fit its integer type, the rotation is unregistered, the extent
    /// is zero, or the scale is not a positive ratio.
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
    /// Virtual-desktop origin X in logical pixels (key 0).
    pub origin_x: i32,
    /// Virtual-desktop origin Y in logical pixels (key 1).
    pub origin_y: i32,
    /// Virtual-desktop width in logical pixels (key 2).
    pub width: u32,
    /// Virtual-desktop height in logical pixels (key 3).
    pub height: u32,
    /// Output descriptors; exactly one is primary when the list is nonempty (key 4).
    pub outputs: Vec<OutputDescriptor>,
    /// False while a resize or reconfiguration is still in flight.
    pub settled: bool,
    /// Topology revision (key 6).
    pub topology_revision: u64,
}

impl DesktopTarget {
    /// Returns the virtual-desktop size.
    pub fn extent(&self) -> TargetExtent {
        TargetExtent::new(self.width, self.height)
    }

    /// Encodes the descriptor as its `WELCOME` key 5 map.
    pub fn encode(&self) -> PayloadMap {
        vec![
            (0, Value::from(i64::from(self.origin_x))),
            (1, Value::from(i64::from(self.origin_y))),
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

    /// Decodes the desktop target descriptor.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError`] when `map` is not a strict desktop-target map, a field is missing,
    /// has the wrong type, or does not fit its integer type, the output list is not an array of
    /// valid outputs, or the target fails [`DesktopTarget::validate`].
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

    /// Checks desktop §1's structural rules.
    ///
    /// The virtual extent is nonzero, exactly one output is primary when the list is nonempty,
    /// output IDs are unique, and every output lies inside the virtual rectangle after checked
    /// transform arithmetic.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError::InvalidValue`] when the virtual extent is zero, a nonempty output
    /// list does not have exactly one primary output, output IDs repeat, or an edge overflows.
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

    /// Returns the primary output, if there is one.
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
    Settled {
        /// The target generation producers now hold.
        generation: TargetGeneration,
    },
    /// A real change: the generation advances and the reason mask says what moved.
    Advanced {
        /// The target generation producers now hold.
        generation: TargetGeneration,
        /// `TARGET_CHANGED` reason bits, such as [`reason::OUTPUT_SET`].
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
    /// Starts tracking `target` at generation one.
    ///
    /// # Errors
    ///
    /// Returns any error from [`DesktopTarget::validate`].
    pub fn new(target: DesktopTarget) -> Result<Self, MessageError> {
        target.validate()?;
        Ok(Self {
            current: target,
            generation: TargetGeneration::ONE,
        })
    }

    /// Returns the current descriptor.
    pub fn current(&self) -> &DesktopTarget {
        &self.current
    }

    /// Returns the current target generation.
    pub fn generation(&self) -> TargetGeneration {
        self.generation
    }

    /// Offer a new descriptor and learn what the session owes its producers.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError::InvalidValue`] when `next` fails [`DesktopTarget::validate`] or the
    /// target generation is exhausted.
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
