//! Presenter-side desktop input grants.
//!
//! Desktop §4 keeps three states distinct: the producer's *desired* binding, the presenter's
//! *eligibility* — focus, consent, policy, device capability — and the *effective* grant, which is
//! a presenter grant for the intersection of the two. Nothing here reinstates a grant when
//! eligibility returns; the producer must issue a strictly greater epoch.
//!
//! [`InputGate`] already owns epoch monotonicity, exact-retry detection, and the checked
//! session-wide grant generation, so this type adds only what a presenter needs on top: deriving
//! effective classes from eligibility, scheduling renewals, and building the four wire payloads.
//! A browser presenter compiled to WebAssembly drives the same code, which is the point — the
//! generation and watchdog rules are exactly where two implementations would otherwise drift.

use crate::{
    cbor::Value,
    input::{
        ActiveGrant, BindingOutcome, INPUT_CLASS_KNOWN_MASK, InputBinding, InputEvent, InputGate,
        MAX_WATCHDOG_US, MIN_WATCHDOG_US,
    },
    messages::{MessageError, PayloadMap},
    revision::SurfaceGeneration,
    time::Monotonic,
};

/// Reasons a presenter reports on `INPUT_BOUND`, `INPUT_REVOKED`, and `INPUT_RESET`, desktop §6.
pub mod reason {
    /// The surface lost focus.
    pub const FOCUS_LOSS: u64 = 1;
    /// Local policy or user consent withdrew input.
    pub const LOCAL_POLICY: u64 = 2;
    /// The surface is unavailable.
    pub const SURFACE_UNAVAILABLE: u64 = 3;
    /// The surface generation changed.
    pub const GENERATION_CHANGE: u64 = 4;
    /// The owning context or lease lost authority.
    pub const AUTHORITY_LOSS: u64 = 5;
    /// The interactive lane was lost.
    pub const LANE_LOSS: u64 = 6;
    /// The session was suspended.
    pub const SUSPENSION: u64 = 7;
    /// The grant's watchdog expired.
    pub const WATCHDOG: u64 = 8;
    /// The input queue or injector failed.
    pub const INJECTOR_FAILURE: u64 = 9;
    /// The presenter is shutting down.
    pub const PRESENTER_SHUTDOWN: u64 = 10;
}

/// `INPUT_BOUND` state 0 (desktop §5.2 key 6): input is disabled.
pub const STATE_DISABLED: u64 = 0;
/// `INPUT_BOUND` state 1: input is enabled.
pub const STATE_ENABLED: u64 = 1;
/// `INPUT_BOUND` state 2: the presenter denied input.
pub const STATE_DENIED: u64 = 2;

/// A presenter's default watchdog. Desktop §5.2 recommends two seconds.
pub const DEFAULT_WATCHDOG_US: u64 = 2_000_000;

/// Everything the presenter knows about whether it *may* grant input right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent presenter conditions, each reported as-is"
)]
pub struct Eligibility {
    /// The presentation target has keyboard focus.
    pub focused: bool,
    /// Local policy and user consent permit injection.
    pub consented: bool,
    /// The named surface exists, is active, and is a desktop-content surface.
    pub surface_present: bool,
    /// Its current generation, which the binding must name exactly.
    pub surface_generation: SurfaceGeneration,
    /// Classes the producer declared it can inject for this surface.
    pub capability_mask: u64,
    /// Whether the active primary-video track has milestone bit 5.
    ///
    /// The bit must be set on the track's current channel generation: first presentation for this
    /// surface generation. Desktop §5.1 makes it a precondition of enabling.
    pub presented: bool,
    /// A live interactive lane, which is where every input record travels.
    pub lane_live: bool,
    /// Context operation class bit 4.
    pub may_receive_input: bool,
}

impl Eligibility {
    /// Why this eligibility refuses a binding, if it does.
    fn refusal(&self, binding: &InputBinding) -> Option<u64> {
        if !self.lane_live {
            return Some(reason::LANE_LOSS);
        }
        if !self.may_receive_input {
            return Some(reason::AUTHORITY_LOSS);
        }
        if !self.surface_present {
            return Some(reason::SURFACE_UNAVAILABLE);
        }
        if binding.surface_generation != self.surface_generation {
            return Some(reason::GENERATION_CHANGE);
        }
        if !self.presented {
            // Nothing has been shown for this surface generation, so a pointer coordinate would
            // name a mapping the user has never seen.
            return Some(reason::SURFACE_UNAVAILABLE);
        }
        if !self.focused {
            return Some(reason::FOCUS_LOSS);
        }
        if !self.consented {
            return Some(reason::LOCAL_POLICY);
        }
        if binding.requested_classes & self.capability_mask == 0 {
            return Some(reason::LOCAL_POLICY);
        }
        None
    }
}

/// What a `SET_INPUT_BINDING` produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GrantOutcome {
    /// The binding asked to disable, and it is disabled.
    Disabled,
    /// A grant for the intersection of desired classes and eligibility.
    Enabled,
    /// Eligible in form but refused, with the reason the presenter reports.
    Denied {
        /// Why the presenter refused, as a [`reason`] value.
        reason: u64,
    },
    /// A byte-identical retry of the current epoch, which returns the current logical result.
    ExactRetry,
}

/// One renewal the presenter owes its producer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Renewal {
    /// Renewal sequence; strictly increases for the grant (key 5).
    pub sequence: u64,
    /// Watchdog timeout in microseconds (key 6).
    pub watchdog_us: u64,
}

/// The presenter's view of one session's input grant.
#[derive(Debug, Default, Clone)]
pub struct InputGrant {
    gate: InputGate,
    watchdog_us: u64,
    renewal_sequence: u64,
    last_reason: u64,
    last_state: u64,
    /// The last generation observed, so it stays reportable after a revocation clears the grant.
    generation: u64,
}

impl InputGrant {
    /// Creates a session's input state with no grant.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the active grant, if any.
    pub fn active(&self) -> Option<&ActiveGrant> {
        self.gate.active()
    }

    /// Returns the last grant generation, which stays reportable after a revocation.
    pub fn grant_generation(&self) -> u64 {
        self.gate.active().map_or(self.generation, |grant| {
            grant.binding.grant_generation.get()
        })
    }

    /// The state the most recent binding produced: `STATE_DISABLED`, `STATE_ENABLED`, or
    /// `STATE_DENIED`. A presenter needs this after the outcome itself is forgotten, because the
    /// browser binding drives its own UI from the same machine a native presenter reads directly.
    pub fn state(&self) -> u64 {
        self.last_state
    }

    /// The transition reason reported with `state()`.
    pub fn reason(&self) -> u64 {
        self.last_reason
    }

    /// Apply a producer binding under the presenter's current eligibility.
    ///
    /// # Errors
    ///
    /// Returns any error from [`crate::input::InputGate::apply_binding`].
    pub fn apply(
        &mut self,
        binding: &InputBinding,
        eligibility: &Eligibility,
        now: Monotonic,
    ) -> Result<GrantOutcome, MessageError> {
        if binding.disabled() {
            let outcome = self.gate.apply_binding(binding.clone(), 0, 0, now)?;
            self.watchdog_us = 0;
            self.renewal_sequence = 0;
            self.last_state = STATE_DISABLED;
            self.last_reason = binding.reason;
            return Ok(match outcome {
                BindingOutcome::ExactRetry => GrantOutcome::ExactRetry,
                _ => GrantOutcome::Disabled,
            });
        }

        // A presenter narrows requested classes; it never broadens them (desktop §5.2).
        let refusal = eligibility.refusal(binding);
        let classes = match refusal {
            Some(_) => 0,
            None => {
                binding.requested_classes & eligibility.capability_mask & INPUT_CLASS_KNOWN_MASK
            }
        };
        let watchdog = binding
            .requested_watchdog_us
            .clamp(MIN_WATCHDOG_US, MAX_WATCHDOG_US);
        let outcome = self
            .gate
            .apply_binding(binding.clone(), classes, watchdog, now)?;
        Ok(match outcome {
            BindingOutcome::ExactRetry => GrantOutcome::ExactRetry,
            BindingOutcome::Enabled(tuple) => {
                self.generation = tuple.grant_generation.get();
                self.watchdog_us = watchdog;
                self.renewal_sequence = 0;
                self.last_state = STATE_ENABLED;
                self.last_reason = binding.reason;
                GrantOutcome::Enabled
            }
            BindingOutcome::Denied(tuple) => {
                self.generation = tuple.grant_generation.get();
                self.watchdog_us = 0;
                self.last_state = STATE_DENIED;
                let reason = refusal.unwrap_or(reason::LOCAL_POLICY);
                self.last_reason = reason;
                GrantOutcome::Denied { reason }
            }
            BindingOutcome::Disabled => {
                self.watchdog_us = 0;
                self.last_state = STATE_DISABLED;
                self.last_reason = binding.reason;
                GrantOutcome::Disabled
            }
        })
    }

    /// End the current grant, advancing the generation.
    ///
    /// Returns the payload identity for `INPUT_REVOKED`, or `None` when nothing was granted.
    pub fn revoke(&mut self, reason: u64) -> Option<PayloadMap> {
        let active = self.gate.active()?.binding;
        let generation = self.gate.revoke().ok()?;
        self.generation = generation.get();
        self.watchdog_us = 0;
        self.renewal_sequence = 0;
        self.last_state = STATE_DISABLED;
        self.last_reason = reason;
        Some(vec![
            (0, Value::Unsigned(active.producer_epoch.get())),
            (1, Value::Unsigned(generation.get())),
            (2, Value::Unsigned(active.context_id)),
            (3, Value::Unsigned(active.surface_id)),
            (4, Value::Unsigned(active.surface_generation.get())),
            (5, Value::Unsigned(reason)),
        ])
    }

    /// The renewal due at `now`, if one is.
    ///
    /// Desktop §6 requires a renewal no less often than half the effective timeout, and ordinary
    /// input events never extend the watchdog — only a renewal does.
    pub fn due_renewal(&mut self, now: Monotonic) -> Option<Renewal> {
        let active = self.gate.active()?;
        let elapsed_deadline = active
            .watchdog_deadline
            .checked_sub_micros(self.watchdog_us / 2)?;
        if now < elapsed_deadline {
            return None;
        }
        self.renewal_sequence = self.renewal_sequence.checked_add(1)?;
        let renewal = Renewal {
            sequence: self.renewal_sequence,
            watchdog_us: self.watchdog_us,
        };
        // Renewing locally moves our own deadline forward so the next one is due half a period on.
        let binding = active.binding;
        self.gate
            .renew(binding, renewal.sequence, self.watchdog_us, now)
            .ok()?;
        Some(renewal)
    }

    /// Whether the active grant's local watchdog deadline has elapsed.
    ///
    /// Presenters use this before attempting a renewal after a stalled event loop: a renewal that
    /// reaches the producer at or after the deadline is invalid, so continuing to emit events with
    /// that tuple would only create stale input traffic.
    pub fn watchdog_expired(&self, now: Monotonic) -> bool {
        self.gate
            .active()
            .is_some_and(|active| now >= active.watchdog_deadline)
    }

    /// `INPUT_BOUND`, desktop §5.2.
    pub fn bound_payload(&self, producer_epoch: u64) -> PayloadMap {
        let active = self.gate.active();
        let enabled = active.is_some() && self.last_state == STATE_ENABLED;
        let (context_id, surface_id, surface_generation, classes) = match active {
            Some(grant) if enabled => (
                grant.binding.context_id,
                grant.binding.surface_id,
                grant.binding.surface_generation.get(),
                grant.effective_classes,
            ),
            _ => (0, 0, 0, 0),
        };
        vec![
            (0, Value::Unsigned(producer_epoch)),
            (1, Value::Unsigned(self.grant_generation())),
            (2, Value::Unsigned(context_id)),
            (3, Value::Unsigned(surface_id)),
            (4, Value::Unsigned(surface_generation)),
            (5, Value::Unsigned(classes)),
            (6, Value::Unsigned(self.last_state)),
            (7, Value::Unsigned(self.last_reason)),
            (
                8,
                Value::Unsigned(if enabled { self.watchdog_us } else { 0 }),
            ),
        ]
    }

    /// `INPUT_LEASE_RENEW`, desktop §6.
    pub fn renewal_payload(&self, renewal: Renewal) -> Option<PayloadMap> {
        let active = self.gate.active()?;
        Some(vec![
            (0, Value::Unsigned(active.binding.producer_epoch.get())),
            (1, Value::Unsigned(active.binding.grant_generation.get())),
            (2, Value::Unsigned(active.binding.context_id)),
            (3, Value::Unsigned(active.binding.surface_id)),
            (4, Value::Unsigned(active.binding.surface_generation.get())),
            (5, Value::Unsigned(renewal.sequence)),
            (6, Value::Unsigned(renewal.watchdog_us)),
        ])
    }

    /// Tag an outgoing event with the current grant, or refuse it.
    ///
    /// An event the presenter cannot tag is one it must not send: the producer's final injection
    /// gate would discard it anyway, and sending it would only widen the window in which a stale
    /// tuple exists.
    pub fn tag(&self, class: u64) -> Option<PayloadMap> {
        let active = self.gate.active()?;
        if active.effective_classes & class == 0 {
            return None;
        }
        let binding = active.binding;
        Some(vec![
            (0, Value::Unsigned(binding.producer_epoch.get())),
            (1, Value::Unsigned(binding.grant_generation.get())),
            (2, Value::Unsigned(binding.context_id)),
            (3, Value::Unsigned(binding.surface_id)),
            (4, Value::Unsigned(binding.surface_generation.get())),
        ])
    }

    /// The classes currently granted, for a caller deciding whether to source an event at all.
    pub fn effective_classes(&self) -> u64 {
        self.gate
            .active()
            .map_or(0, |grant| grant.effective_classes)
    }

    /// Whether an event of `class` would be admitted right now.
    pub fn admits(&self, class: u64) -> bool {
        self.effective_classes() & class != 0
    }
}

/// Build one ordinary input event's payload from a grant tag.
pub fn event_payload(tag: PayloadMap, event: &InputEvent) -> PayloadMap {
    let mut payload = tag;
    payload.extend(event.payload().into_iter().filter(|(key, _)| *key >= 5));
    payload
}
