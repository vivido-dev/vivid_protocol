//! Desktop input binding, events, and final-injection checks (`desktop-input-v1`).
//!
//! A producer asks for input with an [`InputBinding`] (`SET_INPUT_BINDING`). An [`InputGate`]
//! tracks the latest binding and the active grant, and checks every [`InputEvent`] against the
//! grant's tuple, classes, surface generation, and watchdog before it is injected.
//! [`InjectionRejection`] says why an event was refused.

use crate::time::Monotonic;

use crate::{
    cbor::Value,
    messages::{
        MessageError, PayloadMap, StrictMap, invalid_value, require_nonzero, validate_header_object,
    },
    revision::{GrantGeneration, InputEpoch, SurfaceGeneration},
};

/// Input class bit for `KEY_INPUT`.
pub const INPUT_CLASS_KEYBOARD: u64 = 1 << 0;
/// Input class bit for `POINTER_MOTION`.
pub const INPUT_CLASS_POINTER_MOTION: u64 = 1 << 1;
/// Input class bit for `POINTER_BUTTON`.
pub const INPUT_CLASS_POINTER_BUTTON: u64 = 1 << 2;
/// Input class bit for `POINTER_AXIS`.
pub const INPUT_CLASS_POINTER_AXIS: u64 = 1 << 3;
/// Every assigned input class bit.
pub const INPUT_CLASS_KNOWN_MASK: u64 = (1 << 4) - 1;

/// Shortest watchdog timeout a binding may request, in microseconds.
pub const MIN_WATCHDOG_US: u64 = 250_000;
/// Longest watchdog timeout a binding may request, in microseconds.
pub const MAX_WATCHDOG_US: u64 = 5_000_000;

/// The producer's `SET_INPUT_BINDING` request (desktop §5.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputBinding {
    /// Nonzero epoch that increases with each binding (key 0).
    pub producer_epoch: InputEpoch,
    /// Context, or zero to disable (key 1).
    pub context_id: u64,
    /// Surface, or zero to disable (key 2).
    pub surface_id: u64,
    /// Expected surface generation, or zero to disable (key 3).
    pub surface_generation: SurfaceGeneration,
    /// Requested input class mask, or zero to disable (key 4).
    pub requested_classes: u64,
    /// Transition reason (key 5).
    pub reason: u64,
    /// Requested watchdog timeout in microseconds (key 6).
    pub requested_watchdog_us: u64,
}

impl InputBinding {
    /// Returns whether this request disables input.
    pub fn disabled(&self) -> bool {
        self.context_id == 0
            && self.surface_id == 0
            && self.surface_generation == SurfaceGeneration::ZERO
            && self.requested_classes == 0
    }

    /// Checks the binding's epoch, reason, scope, classes, and watchdog against `header_object_id`.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError`] when the producer epoch is zero, the surface ID differs from
    /// `header_object_id`, the reason is unregistered, a disabling binding carries classes or
    /// scope, or an enabling binding has a zero ID or generation, no or unknown input classes, or a
    /// watchdog outside the supported range.
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

    /// Encodes the `SET_INPUT_BINDING` payload.
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

    /// Decodes a `SET_INPUT_BINDING` payload.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError`] when `payload` is not a strict `SET_INPUT_BINDING` map, a field is
    /// missing or has the wrong type, or the binding fails [`InputBinding::validate`].
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

/// The binding identity every desktop input event carries (desktop §7, keys 0 through 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputTuple {
    /// The binding's producer input epoch.
    pub producer_epoch: InputEpoch,
    /// The presenter grant generation.
    pub grant_generation: GrantGeneration,
    /// Owning context.
    pub context_id: u64,
    /// Target surface; also the event's record object ID.
    pub surface_id: u64,
    /// Surface generation the coordinates are in.
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

/// The canonical size of a desktop surface, which bounds every pointer position on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SurfaceExtent {
    /// Width in canonical surface units.
    pub width: u64,
    /// Height in canonical surface units.
    pub height: u64,
}

/// One desktop input event on the interactive lane (desktop §7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum InputEvent {
    /// A physical key transition (`KEY_INPUT`).
    Key {
        /// Returns the binding the event was granted under.
        binding: InputTuple,
        /// USB HID keyboard-page usage, `0x04..=0xe7`.
        usage: u16,
        /// Whether the key or button went down.
        pressed: bool,
    },
    /// Pointer motion (`POINTER_MOTION`).
    PointerMotion {
        /// Returns the binding the event was granted under.
        binding: InputTuple,
        /// Pointer X in 32.32 canonical surface units.
        x: u64,
        /// Pointer Y in 32.32 canonical surface units.
        y: u64,
    },
    /// A pointer button transition (`POINTER_BUTTON`).
    PointerButton {
        /// Returns the binding the event was granted under.
        binding: InputTuple,
        /// Primary `0`, auxiliary `1`, secondary `2`, back `3`, or forward `4`.
        button: u8,
        /// Whether the key or button went down.
        pressed: bool,
        /// Pointer X in 32.32 canonical surface units.
        x: u64,
        /// Pointer Y in 32.32 canonical surface units.
        y: u64,
    },
    /// A scroll (`POINTER_AXIS`).
    PointerAxis {
        /// Returns the binding the event was granted under.
        binding: InputTuple,
        /// Horizontal delta in 1/120-detent units, `-12000..=12000`.
        horizontal: i64,
        /// Vertical delta in 1/120-detent units, `-12000..=12000`.
        vertical: i64,
        /// Pointer X in 32.32 canonical surface units.
        x: u64,
        /// Pointer Y in 32.32 canonical surface units.
        y: u64,
    },
}

impl InputEvent {
    /// Returns the binding the event was granted under.
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

    /// Returns the event's input class bit, such as [`INPUT_CLASS_KEYBOARD`].
    pub const fn class(self) -> u64 {
        match self {
            Self::Key { .. } => INPUT_CLASS_KEYBOARD,
            Self::PointerMotion { .. } => INPUT_CLASS_POINTER_MOTION,
            Self::PointerButton { .. } => INPUT_CLASS_POINTER_BUTTON,
            Self::PointerAxis { .. } => INPUT_CLASS_POINTER_AXIS,
        }
    }

    /// Encodes the event's payload.
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
                payload.push((5, Value::from(horizontal)));
                payload.push((6, Value::from(vertical)));
                payload.push((7, Value::Unsigned(x)));
                payload.push((8, Value::Unsigned(y)));
            }
        }
        payload
    }

    /// Decodes a `KEY_INPUT` payload.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError`] when `payload` is not a strict `KEY_INPUT` map, the input tuple is
    /// missing, mistyped, or has a zero context, surface, or surface generation, the surface ID
    /// differs from `header_object_id`, or the usage is outside the keyboard usage range.
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

    /// Decodes a `POINTER_MOTION` payload on a surface of `extent`.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError`] when `payload` is not a strict `POINTER_MOTION` map, the input
    /// tuple is missing, mistyped, or has a zero context, surface, or surface generation, the
    /// surface ID differs from `header_object_id`, or the position is outside `extent`.
    pub fn decode_motion(
        header_object_id: u64,
        payload: &Value,
        extent: SurfaceExtent,
    ) -> Result<Self, MessageError> {
        let map = StrictMap::new("POINTER_MOTION", payload, &[0, 1, 2, 3, 4, 5, 6])?;
        let binding = InputTuple::decode(&map)?;
        validate_header_object(header_object_id, binding.surface_id)?;
        let x = map.required_u64(5)?;
        let y = map.required_u64(6)?;
        validate_coordinates(x, y, extent)?;
        Ok(Self::PointerMotion { binding, x, y })
    }

    /// Decodes a `POINTER_BUTTON` payload on a surface of `extent`.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError`] when `payload` is not a strict `POINTER_BUTTON` map, the input
    /// tuple is missing, mistyped, or has a zero context, surface, or surface generation, the
    /// surface ID differs from `header_object_id`, the button is unknown, or the position is
    /// outside `extent`.
    pub fn decode_button(
        header_object_id: u64,
        payload: &Value,
        extent: SurfaceExtent,
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
        validate_coordinates(x, y, extent)?;
        Ok(Self::PointerButton {
            binding,
            button,
            pressed: map.required_bool(6)?,
            x,
            y,
        })
    }

    /// Decodes a `POINTER_AXIS` payload on a surface of `extent`.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError`] when `payload` is not a strict `POINTER_AXIS` map, the input tuple
    /// is missing, mistyped, or has a zero context, surface, or surface generation, the surface ID
    /// differs from `header_object_id`, a scroll delta is outside -12,000 through 12,000, or the
    /// position is outside `extent`.
    pub fn decode_axis(
        header_object_id: u64,
        payload: &Value,
        extent: SurfaceExtent,
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
        validate_coordinates(x, y, extent)?;
        Ok(Self::PointerAxis {
            binding,
            horizontal,
            vertical,
            x,
            y,
        })
    }
}

/// The input grant currently in effect.
#[derive(Debug, Clone)]
pub struct ActiveGrant {
    /// The binding identity events must carry.
    pub binding: InputTuple,
    /// Input classes the grant allows.
    pub effective_classes: u64,
    /// The effective watchdog in microseconds.
    pub watchdog_timeout_us: u64,
    /// When the grant lapses without a renewal.
    pub watchdog_deadline: Monotonic,
    /// Sequence of the last accepted renewal; renewals must increase it.
    pub renewal_sequence: u64,
}

/// What [`InputGate`] did with a binding request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingOutcome {
    /// The request disabled input.
    Disabled,
    /// Input is enabled for this tuple.
    Enabled(InputTuple),
    /// The presenter denied the binding for this tuple.
    Denied(InputTuple),
    /// The request repeats the latest binding exactly.
    ExactRetry,
}

/// Why the presenter refused to inject an input event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum InjectionRejection {
    /// No input grant is active.
    NoActiveGrant,
    /// The event names a binding other than the active one.
    StaleTuple,
    /// The surface generation changed since the grant.
    SurfaceGenerationChanged,
    /// The grant's watchdog lapsed.
    WatchdogExpired,
    /// The grant does not allow the event's class.
    ClassNotGranted,
}

impl std::fmt::Display for InjectionRejection {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::NoActiveGrant => "no input grant is active",
            Self::StaleTuple => "input event names a stale binding",
            Self::SurfaceGenerationChanged => "surface generation changed since the grant",
            Self::WatchdogExpired => "input grant watchdog expired",
            Self::ClassNotGranted => "input class was not granted",
        })
    }
}

impl std::error::Error for InjectionRejection {}

/// Desktop input state for one binding: the latest binding, the active grant, and event checks.
///
/// Presenters keep one per session, and producers check it immediately before OS injection.
#[derive(Debug, Default, Clone)]
pub struct InputGate {
    latest_binding: Option<InputBinding>,
    grant_generation: GrantGeneration,
    active: Option<ActiveGrant>,
    release_generation: u64,
}

impl InputGate {
    /// Returns the active grant, if any.
    pub fn active(&self) -> Option<&ActiveGrant> {
        self.active.as_ref()
    }

    /// Returns the release generation, which advances each time a binding change ends an active
    /// grant.
    pub const fn release_generation(&self) -> u64 {
        self.release_generation
    }

    /// Applies a `SET_INPUT_BINDING`, granting at most `effective_classes` from `now`.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError`] when the binding fails [`InputBinding::validate`], moves the input
    /// epoch backward, reuses the current epoch with different contents, or a grant generation is
    /// exhausted.
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
        let grant_generation = self.next_grant_generation()?;
        let release_generation = self.next_release_generation()?;
        let tuple = InputTuple {
            producer_epoch: binding.producer_epoch,
            grant_generation,
            context_id: binding.context_id,
            surface_id: binding.surface_id,
            surface_generation: binding.surface_generation,
        };
        let effective_classes = effective_classes & binding.requested_classes;
        let (outcome, active) = if binding.disabled() {
            (BindingOutcome::Disabled, None)
        } else if effective_classes == 0
            || !(MIN_WATCHDOG_US..=MAX_WATCHDOG_US).contains(&effective_watchdog_us)
        {
            (BindingOutcome::Denied(tuple), None)
        } else {
            let watchdog_deadline = now
                .checked_add_micros(effective_watchdog_us)
                .ok_or_else(|| invalid_value("INPUT_BOUND", 8, "overflows local time"))?;
            (
                BindingOutcome::Enabled(tuple),
                Some(ActiveGrant {
                    binding: tuple,
                    effective_classes,
                    watchdog_timeout_us: effective_watchdog_us,
                    watchdog_deadline,
                    renewal_sequence: 0,
                }),
            )
        };
        // Commit only after every fallible generation and deadline check succeeds.
        self.latest_binding = Some(binding);
        self.grant_generation = grant_generation;
        self.release_generation = release_generation;
        self.active = active;
        Ok(outcome)
    }

    /// Extends the active grant's watchdog for an `INPUT_LEASE_RENEW` received at `received_at`.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError::InvalidValue`] when there is no active grant, the renewal names
    /// another binding, repeats or reorders a sequence number, arrives after the watchdog expired,
    /// changes the watchdog, or the new deadline overflows.
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
        let watchdog_deadline = received_at
            .checked_add_micros(active.watchdog_timeout_us)
            .ok_or_else(|| invalid_value("INPUT_LEASE_RENEW", 6, "overflows local time"))?;
        active.renewal_sequence = renewal_sequence;
        active.watchdog_deadline = watchdog_deadline;
        Ok(())
    }

    /// Revokes any active grant, advancing the grant generation.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError::InvalidValue`] when a grant generation is exhausted.
    pub fn revoke(&mut self) -> Result<GrantGeneration, MessageError> {
        let grant_generation = self.next_grant_generation()?;
        let release_generation = self.next_release_generation()?;
        self.grant_generation = grant_generation;
        self.release_generation = release_generation;
        self.active = None;
        Ok(grant_generation)
    }

    /// Checks whether `event` may be injected now under the active grant.
    ///
    /// # Errors
    ///
    /// Returns an [`InjectionRejection`] when there is no active grant, the event's tuple is stale,
    /// the surface generation changed, the watchdog expired, or the event's input class was not
    /// granted.
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
    ///
    /// # Errors
    ///
    /// Returns the [`InjectionRejection`] from [`InputGate::authorize`] without running
    /// `operation`.
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

    fn next_release_generation(&self) -> Result<u64, MessageError> {
        if self.active.is_some() {
            self.release_generation
                .checked_add(1)
                .ok_or_else(|| invalid_value("input release", 0, "exhausted"))
        } else {
            Ok(self.release_generation)
        }
    }

    fn next_grant_generation(&self) -> Result<GrantGeneration, MessageError> {
        self.grant_generation
            .advance()
            .map_err(|_| invalid_value("input grant", 1, "exhausted"))
    }
}

fn validate_coordinates(x: u64, y: u64, extent: SurfaceExtent) -> Result<(), MessageError> {
    let SurfaceExtent { width, height } = extent;
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

    fn assert_unchanged(actual: &InputGate, before: &InputGate) {
        assert_eq!(actual.latest_binding, before.latest_binding);
        assert_eq!(actual.grant_generation, before.grant_generation);
        assert_eq!(actual.release_generation, before.release_generation);
        let snapshot = |gate: &InputGate| {
            gate.active().map(|a| {
                (
                    a.binding,
                    a.effective_classes,
                    a.watchdog_timeout_us,
                    a.watchdog_deadline,
                    a.renewal_sequence,
                )
            })
        };
        assert_eq!(snapshot(actual), snapshot(before));
    }

    #[test]
    fn failed_binding_and_revocation_preserve_both_owners() {
        let now = Monotonic::from_micros(0);
        let mut gate = InputGate::default();
        gate.apply_binding(binding(1), INPUT_CLASS_KEYBOARD, 1_000_000, now)
            .unwrap();
        let mut other = InputGate::default();
        let mut other_binding = binding(1); // Same local surface and epoch, another context.
        other_binding.context_id = 9;
        other
            .apply_binding(other_binding, INPUT_CLASS_KEYBOARD, 1_000_000, now)
            .unwrap();
        let other_before = other.clone();
        for boundary in 0..3 {
            let mut failed = gate.clone();
            let time = match boundary {
                0 => Monotonic::from_micros(u64::MAX),
                1 => {
                    failed.grant_generation = GrantGeneration::new(u64::MAX);
                    now
                }
                _ => {
                    failed.release_generation = u64::MAX;
                    now
                }
            };
            let before = failed.clone();
            for _ in 0..2 {
                failed
                    .apply_binding(binding(2), INPUT_CLASS_KEYBOARD, 1_000_000, time)
                    .unwrap_err();
                assert_unchanged(&failed, &before);
            }
            if boundary != 0 {
                failed.revoke().unwrap_err();
                assert_unchanged(&failed, &before);
            } else {
                assert!(matches!(
                    failed
                        .apply_binding(binding(2), INPUT_CLASS_KEYBOARD, 1_000_000, now)
                        .unwrap(),
                    BindingOutcome::Enabled(_)
                ));
            }
            assert_unchanged(&other, &other_before);
            let active = other.active().unwrap();
            other
                .authorize(
                    InputEvent::Key {
                        binding: active.binding,
                        usage: 4,
                        pressed: true,
                    },
                    SurfaceGeneration::ONE,
                    now,
                )
                .unwrap();
        }
    }

    #[test]
    fn overflowing_renewal_does_not_consume_sequence() {
        let mut gate = InputGate::default();
        gate.apply_binding(
            binding(1),
            INPUT_CLASS_KEYBOARD,
            1_000_000,
            Monotonic::from_micros(u64::MAX - 1_000_000),
        )
        .unwrap();
        let before = gate.clone();
        let tuple = gate.active().unwrap().binding;
        for _ in 0..2 {
            assert!(
                gate.renew(tuple, 1, 1_000_000, Monotonic::from_micros(u64::MAX - 1))
                    .is_err()
            );
            assert_unchanged(&gate, &before);
        }
        gate.renew(
            tuple,
            1,
            1_000_000,
            Monotonic::from_micros(u64::MAX - 1_000_000),
        )
        .unwrap();
        assert_eq!(gate.active().unwrap().renewal_sequence, 1);
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
        gate.apply_binding(binding(1), INPUT_CLASS_KEYBOARD, 1_000_000, now)
            .unwrap_err();
    }
}
