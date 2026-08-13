//! Generation-checked desktop input schemas and final-injection gating.

use crate::time::Monotonic;

use crate::{
    cbor::Value,
    messages::{
        MessageError, PayloadMap, StrictMap, invalid_value, require_nonzero, validate_header_object,
    },
    revision::{GrantGeneration, InputEpoch, SurfaceGeneration},
};

pub const INPUT_CLASS_KEYBOARD: u64 = 1 << 0;
pub const INPUT_CLASS_POINTER_MOTION: u64 = 1 << 1;
pub const INPUT_CLASS_POINTER_BUTTON: u64 = 1 << 2;
pub const INPUT_CLASS_POINTER_AXIS: u64 = 1 << 3;
pub const INPUT_CLASS_KNOWN_MASK: u64 = (1 << 4) - 1;

pub const MIN_WATCHDOG_US: u64 = 250_000;
pub const MAX_WATCHDOG_US: u64 = 5_000_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputBinding {
    pub producer_epoch: InputEpoch,
    pub context_id: u64,
    pub surface_id: u64,
    pub surface_generation: SurfaceGeneration,
    pub requested_classes: u64,
    pub reason: u64,
    pub requested_watchdog_us: u64,
}

impl InputBinding {
    pub fn disabled(&self) -> bool {
        self.context_id == 0
            && self.surface_id == 0
            && self.surface_generation == SurfaceGeneration::ZERO
            && self.requested_classes == 0
    }

    pub fn validate(&self, header_object_id: u64) -> Result<(), MessageError> {
        self.producer_epoch
            .require_nonzero()
            .map_err(|_| invalid_value("SET_INPUT_BINDING", 0, "must be nonzero"))?;
        validate_header_object(header_object_id, self.surface_id)?;
        if self.reason > 7 {
            return Err(invalid_value(
                "SET_INPUT_BINDING",
                5,
                "has an unknown transition reason",
            ));
        }
        if self.disabled() {
            if self.requested_watchdog_us != 0 {
                return Err(invalid_value(
                    "SET_INPUT_BINDING",
                    6,
                    "must be zero when disabling",
                ));
            }
            return Ok(());
        }
        require_nonzero("SET_INPUT_BINDING", 1, self.context_id)?;
        require_nonzero("SET_INPUT_BINDING", 2, self.surface_id)?;
        self.surface_generation
            .require_nonzero()
            .map_err(|_| invalid_value("SET_INPUT_BINDING", 3, "must be nonzero"))?;
        if self.requested_classes == 0 || self.requested_classes & !INPUT_CLASS_KNOWN_MASK != 0 {
            return Err(invalid_value(
                "SET_INPUT_BINDING",
                4,
                "contains no class or unknown classes",
            ));
        }
        if !(MIN_WATCHDOG_US..=MAX_WATCHDOG_US).contains(&self.requested_watchdog_us) {
            return Err(invalid_value(
                "SET_INPUT_BINDING",
                6,
                "is outside the watchdog range",
            ));
        }
        Ok(())
    }

    pub fn payload(&self) -> PayloadMap {
        vec![
            (0, Value::Unsigned(self.producer_epoch.get())),
            (1, Value::Unsigned(self.context_id)),
            (2, Value::Unsigned(self.surface_id)),
            (3, Value::Unsigned(self.surface_generation.get())),
            (4, Value::Unsigned(self.requested_classes)),
            (5, Value::Unsigned(self.reason)),
            (6, Value::Unsigned(self.requested_watchdog_us)),
        ]
    }

    pub fn decode(header_object_id: u64, payload: &Value) -> Result<Self, MessageError> {
        let map = StrictMap::new("SET_INPUT_BINDING", payload, &[0, 1, 2, 3, 4, 5, 6])?;
        let binding = Self {
            producer_epoch: InputEpoch::new(map.required_u64(0)?),
            context_id: map.required_u64(1)?,
            surface_id: map.required_u64(2)?,
            surface_generation: SurfaceGeneration::new(map.required_u64(3)?),
            requested_classes: map.required_u64(4)?,
            reason: map.required_u64(5)?,
            requested_watchdog_us: map.required_u64(6)?,
        };
        binding.validate(header_object_id)?;
        Ok(binding)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputTuple {
    pub producer_epoch: InputEpoch,
    pub grant_generation: GrantGeneration,
    pub context_id: u64,
    pub surface_id: u64,
    pub surface_generation: SurfaceGeneration,
}

impl InputTuple {
    fn payload_prefix(self) -> PayloadMap {
        vec![
            (0, Value::Unsigned(self.producer_epoch.get())),
            (1, Value::Unsigned(self.grant_generation.get())),
            (2, Value::Unsigned(self.context_id)),
            (3, Value::Unsigned(self.surface_id)),
            (4, Value::Unsigned(self.surface_generation.get())),
        ]
    }

    fn decode(map: &StrictMap<'_>) -> Result<Self, MessageError> {
        Ok(Self {
            producer_epoch: InputEpoch::new(map.required_u64(0)?),
            grant_generation: GrantGeneration::new(map.required_u64(1)?),
            context_id: require_nonzero("input event", 2, map.required_u64(2)?)?,
            surface_id: require_nonzero("input event", 3, map.required_u64(3)?)?,
            surface_generation: SurfaceGeneration::new(require_nonzero(
                "input event",
                4,
                map.required_u64(4)?,
            )?),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputEvent {
    Key {
        binding: InputTuple,
        usage: u16,
        pressed: bool,
    },
    PointerMotion {
        binding: InputTuple,
        x: u64,
        y: u64,
    },
    PointerButton {
        binding: InputTuple,
        button: u8,
        pressed: bool,
        x: u64,
        y: u64,
    },
    PointerAxis {
        binding: InputTuple,
        horizontal: i64,
        vertical: i64,
        x: u64,
        y: u64,
    },
}

impl InputEvent {
    pub const fn binding(self) -> InputTuple {
        match self {
            Self::Key { binding, .. }
            | Self::PointerMotion { binding, .. }
            | Self::PointerButton { binding, .. }
            | Self::PointerAxis { binding, .. } => binding,
        }
    }

    /// The record type this event travels as on the interactive lane.
    pub const fn record_type(self) -> u16 {
        match self {
            Self::Key { .. } => crate::messages::KEY_INPUT,
            Self::PointerMotion { .. } => crate::messages::POINTER_MOTION,
            Self::PointerButton { .. } => crate::messages::POINTER_BUTTON,
            Self::PointerAxis { .. } => crate::messages::POINTER_AXIS,
        }
    }

    pub const fn class(self) -> u64 {
        match self {
            Self::Key { .. } => INPUT_CLASS_KEYBOARD,
            Self::PointerMotion { .. } => INPUT_CLASS_POINTER_MOTION,
            Self::PointerButton { .. } => INPUT_CLASS_POINTER_BUTTON,
            Self::PointerAxis { .. } => INPUT_CLASS_POINTER_AXIS,
        }
    }

    pub fn payload(self) -> PayloadMap {
        let mut payload = self.binding().payload_prefix();
        match self {
            Self::Key { usage, pressed, .. } => {
                payload.push((5, Value::Unsigned(u64::from(usage))));
                payload.push((6, Value::Bool(pressed)));
            }
            Self::PointerMotion { x, y, .. } => {
                payload.push((5, Value::Unsigned(x)));
                payload.push((6, Value::Unsigned(y)));
            }
            Self::PointerButton {
                button,
                pressed,
                x,
                y,
                ..
            } => {
                payload.push((5, Value::Unsigned(u64::from(button))));
                payload.push((6, Value::Bool(pressed)));
                payload.push((7, Value::Unsigned(x)));
                payload.push((8, Value::Unsigned(y)));
            }
            Self::PointerAxis {
                horizontal,
                vertical,
                x,
                y,
                ..
            } => {
                payload.push((5, signed(horizontal)));
                payload.push((6, signed(vertical)));
                payload.push((7, Value::Unsigned(x)));
                payload.push((8, Value::Unsigned(y)));
            }
        }
        payload
    }

    pub fn decode_key(header_object_id: u64, payload: &Value) -> Result<Self, MessageError> {
        let map = StrictMap::new("KEY_INPUT", payload, &[0, 1, 2, 3, 4, 5, 6])?;
        let binding = InputTuple::decode(&map)?;
        validate_header_object(header_object_id, binding.surface_id)?;
        let usage = u16::try_from(map.required_u64(5)?)
            .map_err(|_| invalid_value("KEY_INPUT", 5, "does not fit in u16"))?;
        if !(0x04..=0xe7).contains(&usage) {
            return Err(invalid_value(
                "KEY_INPUT",
                5,
                "is outside the keyboard usage range",
            ));
        }
        Ok(Self::Key {
            binding,
            usage,
            pressed: map.required_bool(6)?,
        })
    }

    pub fn decode_motion(
        header_object_id: u64,
        payload: &Value,
        width: u64,
        height: u64,
    ) -> Result<Self, MessageError> {
        let map = StrictMap::new("POINTER_MOTION", payload, &[0, 1, 2, 3, 4, 5, 6])?;
        let binding = InputTuple::decode(&map)?;
        validate_header_object(header_object_id, binding.surface_id)?;
        let x = map.required_u64(5)?;
        let y = map.required_u64(6)?;
        validate_coordinates(x, y, width, height)?;
        Ok(Self::PointerMotion { binding, x, y })
    }

    pub fn decode_button(
        header_object_id: u64,
        payload: &Value,
        width: u64,
        height: u64,
    ) -> Result<Self, MessageError> {
        let map = StrictMap::new("POINTER_BUTTON", payload, &[0, 1, 2, 3, 4, 5, 6, 7, 8])?;
        let binding = InputTuple::decode(&map)?;
        validate_header_object(header_object_id, binding.surface_id)?;
        let button = u8::try_from(map.required_u64(5)?)
            .map_err(|_| invalid_value("POINTER_BUTTON", 5, "does not fit in u8"))?;
        if button > 4 {
            return Err(invalid_value("POINTER_BUTTON", 5, "has an unknown button"));
        }
        let x = map.required_u64(7)?;
        let y = map.required_u64(8)?;
        validate_coordinates(x, y, width, height)?;
        Ok(Self::PointerButton {
            binding,
            button,
            pressed: map.required_bool(6)?,
            x,
            y,
        })
    }

    pub fn decode_axis(
        header_object_id: u64,
        payload: &Value,
        width: u64,
        height: u64,
    ) -> Result<Self, MessageError> {
        let map = StrictMap::new("POINTER_AXIS", payload, &[0, 1, 2, 3, 4, 5, 6, 7, 8])?;
        let binding = InputTuple::decode(&map)?;
        validate_header_object(header_object_id, binding.surface_id)?;
        let horizontal = required_i64(&map, 5)?;
        let vertical = required_i64(&map, 6)?;
        if !(-12_000..=12_000).contains(&horizontal) || !(-12_000..=12_000).contains(&vertical) {
            return Err(invalid_value(
                "POINTER_AXIS",
                5,
                "is outside -12000..=12000",
            ));
        }
        let x = map.required_u64(7)?;
        let y = map.required_u64(8)?;
        validate_coordinates(x, y, width, height)?;
        Ok(Self::PointerAxis {
            binding,
            horizontal,
            vertical,
            x,
            y,
        })
    }
}

#[derive(Debug, Clone)]
pub struct ActiveGrant {
    pub binding: InputTuple,
    pub effective_classes: u64,
    /// The effective watchdog in microseconds.
    pub watchdog_timeout_us: u64,
    pub watchdog_deadline: Monotonic,
    pub renewal_sequence: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingOutcome {
    Disabled,
    Enabled(InputTuple),
    Denied(InputTuple),
    ExactRetry,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InjectionRejection {
    NoActiveGrant,
    StaleTuple,
    SurfaceGenerationChanged,
    WatchdogExpired,
    ClassNotGranted,
}

#[derive(Debug, Default, Clone)]
pub struct InputGate {
    latest_binding: Option<InputBinding>,
    grant_generation: GrantGeneration,
    active: Option<ActiveGrant>,
    release_generation: u64,
}

impl InputGate {
    pub fn active(&self) -> Option<&ActiveGrant> {
        self.active.as_ref()
    }

    pub const fn release_generation(&self) -> u64 {
        self.release_generation
    }

    pub fn apply_binding(
        &mut self,
        binding: InputBinding,
        effective_classes: u64,
        effective_watchdog_us: u64,
        now: Monotonic,
    ) -> Result<BindingOutcome, MessageError> {
        binding.validate(binding.surface_id)?;
        if let Some(previous) = &self.latest_binding {
            if binding.producer_epoch < previous.producer_epoch {
                return Err(invalid_value(
                    "SET_INPUT_BINDING",
                    0,
                    "moves the input epoch backward",
                ));
            }
            if binding.producer_epoch == previous.producer_epoch {
                return if binding == *previous {
                    Ok(BindingOutcome::ExactRetry)
                } else {
                    Err(invalid_value(
                        "SET_INPUT_BINDING",
                        0,
                        "reuses an epoch with different bytes",
                    ))
                };
            }
        }
        self.latest_binding = Some(binding.clone());
        self.advance_grant_generation()?;
        self.release_active()?;
        if binding.disabled() {
            return Ok(BindingOutcome::Disabled);
        }
        let tuple = InputTuple {
            producer_epoch: binding.producer_epoch,
            grant_generation: self.grant_generation,
            context_id: binding.context_id,
            surface_id: binding.surface_id,
            surface_generation: binding.surface_generation,
        };
        let effective_classes = effective_classes & binding.requested_classes;
        if effective_classes == 0
            || !(MIN_WATCHDOG_US..=MAX_WATCHDOG_US).contains(&effective_watchdog_us)
        {
            return Ok(BindingOutcome::Denied(tuple));
        }
        let watchdog_deadline = now
            .checked_add_micros(effective_watchdog_us)
            .ok_or_else(|| invalid_value("INPUT_BOUND", 8, "overflows local time"))?;
        self.active = Some(ActiveGrant {
            binding: tuple,
            effective_classes,
            watchdog_timeout_us: effective_watchdog_us,
            watchdog_deadline,
            renewal_sequence: 0,
        });
        Ok(BindingOutcome::Enabled(tuple))
    }

    pub fn renew(
        &mut self,
        binding: InputTuple,
        renewal_sequence: u64,
        watchdog_us: u64,
        received_at: Monotonic,
    ) -> Result<(), MessageError> {
        let active = self
            .active
            .as_mut()
            .ok_or_else(|| invalid_value("INPUT_LEASE_RENEW", 0, "has no active grant"))?;
        if active.binding != binding
            || renewal_sequence <= active.renewal_sequence
            || received_at >= active.watchdog_deadline
            || watchdog_us != active.watchdog_timeout_us
        {
            return Err(invalid_value(
                "INPUT_LEASE_RENEW",
                5,
                "is stale, late, or inconsistent",
            ));
        }
        active.renewal_sequence = renewal_sequence;
        active.watchdog_deadline = received_at
            .checked_add_micros(active.watchdog_timeout_us)
            .ok_or_else(|| invalid_value("INPUT_LEASE_RENEW", 6, "overflows local time"))?;
        Ok(())
    }

    pub fn revoke(&mut self) -> Result<GrantGeneration, MessageError> {
        self.advance_grant_generation()?;
        self.release_active()?;
        Ok(self.grant_generation)
    }

    pub fn authorize(
        &self,
        event: InputEvent,
        current_surface_generation: SurfaceGeneration,
        now: Monotonic,
    ) -> Result<(), InjectionRejection> {
        let active = self
            .active
            .as_ref()
            .ok_or(InjectionRejection::NoActiveGrant)?;
        if event.binding() != active.binding {
            return Err(InjectionRejection::StaleTuple);
        }
        if event.binding().surface_generation != current_surface_generation {
            return Err(InjectionRejection::SurfaceGenerationChanged);
        }
        if now >= active.watchdog_deadline {
            return Err(InjectionRejection::WatchdogExpired);
        }
        if active.effective_classes & event.class() == 0 {
            return Err(InjectionRejection::ClassNotGranted);
        }
        Ok(())
    }

    /// Perform the final generation check and OS-target operation under one exclusive gate.
    ///
    /// Callers that share this gate between threads place it behind the same mutex used by
    /// revocation and surface-target replacement. The closure must invoke only the currently
    /// selected target and must not enqueue work elsewhere.
    pub fn dispatch<R>(
        &mut self,
        event: InputEvent,
        current_surface_generation: SurfaceGeneration,
        now: Monotonic,
        operation: impl FnOnce(InputEvent) -> R,
    ) -> Result<R, InjectionRejection> {
        self.authorize(event, current_surface_generation, now)?;
        Ok(operation(event))
    }

    fn release_active(&mut self) -> Result<(), MessageError> {
        if self.active.take().is_some() {
            self.release_generation = self
                .release_generation
                .checked_add(1)
                .ok_or_else(|| invalid_value("input release", 0, "exhausted"))?;
        }
        Ok(())
    }

    fn advance_grant_generation(&mut self) -> Result<(), MessageError> {
        self.grant_generation = self
            .grant_generation
            .advance()
            .map_err(|_| invalid_value("input grant", 1, "exhausted"))?;
        Ok(())
    }
}

fn validate_coordinates(x: u64, y: u64, width: u64, height: u64) -> Result<(), MessageError> {
    let maximum_x = width
        .checked_mul(1_u64 << 32)
        .ok_or_else(|| invalid_value("pointer input", 5, "width overflows 32.32"))?;
    let maximum_y = height
        .checked_mul(1_u64 << 32)
        .ok_or_else(|| invalid_value("pointer input", 6, "height overflows 32.32"))?;
    if x >= maximum_x || y >= maximum_y {
        return Err(invalid_value(
            "pointer input",
            5,
            "is outside the canonical surface",
        ));
    }
    Ok(())
}

fn required_i64(map: &StrictMap<'_>, key: u64) -> Result<i64, MessageError> {
    map.required(key)?
        .as_i64()
        .ok_or_else(|| invalid_value("input event", key, "is not an integer"))
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

    fn binding(epoch: u64) -> InputBinding {
        InputBinding {
            producer_epoch: InputEpoch::new(epoch),
            context_id: 2,
            surface_id: 3,
            surface_generation: SurfaceGeneration::ONE,
            requested_classes: INPUT_CLASS_KEYBOARD,
            reason: 6,
            requested_watchdog_us: 1_000_000,
        }
    }

    #[test]
    fn queued_old_event_fails_after_revocation() {
        let now = Monotonic::from_micros(1_000_000);
        let mut gate = InputGate::default();
        let BindingOutcome::Enabled(tuple) = gate
            .apply_binding(binding(1), INPUT_CLASS_KEYBOARD, 1_000_000, now)
            .unwrap()
        else {
            panic!("grant was not enabled");
        };
        let event = InputEvent::Key {
            binding: tuple,
            usage: 4,
            pressed: true,
        };
        gate.authorize(event, SurfaceGeneration::ONE, now).unwrap();
        gate.revoke().unwrap();
        assert_eq!(
            gate.authorize(event, SurfaceGeneration::ONE, now),
            Err(InjectionRejection::NoActiveGrant)
        );
        assert_eq!(gate.release_generation(), 1);
    }

    #[test]
    fn generation_change_rejects_at_final_gate() {
        let now = Monotonic::from_micros(1_000_000);
        let mut gate = InputGate::default();
        let BindingOutcome::Enabled(tuple) = gate
            .apply_binding(binding(1), INPUT_CLASS_KEYBOARD, 1_000_000, now)
            .unwrap()
        else {
            panic!("grant was not enabled");
        };
        let event = InputEvent::Key {
            binding: tuple,
            usage: 4,
            pressed: true,
        };
        assert_eq!(
            gate.authorize(event, SurfaceGeneration::new(2), now),
            Err(InjectionRejection::SurfaceGenerationChanged)
        );
    }

    #[test]
    fn old_epoch_never_auto_restores() {
        let now = Monotonic::from_micros(1_000_000);
        let mut gate = InputGate::default();
        gate.apply_binding(binding(2), INPUT_CLASS_KEYBOARD, 1_000_000, now)
            .unwrap();
        gate.revoke().unwrap();
        assert!(
            gate.apply_binding(binding(1), INPUT_CLASS_KEYBOARD, 1_000_000, now)
                .is_err()
        );
    }
}
