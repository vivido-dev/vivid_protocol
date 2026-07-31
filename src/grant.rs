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
    pub const FOCUS_LOSS: u64 = 1;
    pub const LOCAL_POLICY: u64 = 2;
    pub const SURFACE_UNAVAILABLE: u64 = 3;
    pub const GENERATION_CHANGE: u64 = 4;
    pub const AUTHORITY_LOSS: u64 = 5;
    pub const LANE_LOSS: u64 = 6;
    pub const SUSPENSION: u64 = 7;
    pub const WATCHDOG: u64 = 8;
    pub const INJECTOR_FAILURE: u64 = 9;
    pub const PRESENTER_SHUTDOWN: u64 = 10;
}

/// `INPUT_BOUND` states, desktop §5.2 key 6.
pub const STATE_DISABLED: u64 = 0;
pub const STATE_ENABLED: u64 = 1;
pub const STATE_DENIED: u64 = 2;

/// A presenter's default watchdog. Desktop §5.2 recommends two seconds.
pub const DEFAULT_WATCHDOG_US: u64 = 2_000_000;

/// Everything the presenter knows about whether it *may* grant input right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    /// Milestone bit 5 on the active primary-video track's *current* channel generation: first
    /// presentation for this surface generation. Desktop §5.1 makes it a precondition of enabling.
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
    Denied { reason: u64 },
    /// A byte-identical retry of the current epoch, which returns the current logical result.
    ExactRetry,
}

/// One renewal the presenter owes its producer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Renewal {
    pub sequence: u64,
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
    pub fn new() -> Self {
        Self::default()
    }

    pub fn active(&self) -> Option<&ActiveGrant> {
        self.gate.active()
    }

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::{
        INPUT_CLASS_KEYBOARD, INPUT_CLASS_POINTER_AXIS, INPUT_CLASS_POINTER_MOTION,
    };
    use crate::revision::InputEpoch;

    fn eligible() -> Eligibility {
        Eligibility {
            focused: true,
            consented: true,
            surface_present: true,
            surface_generation: SurfaceGeneration::ONE,
            capability_mask: INPUT_CLASS_KEYBOARD | INPUT_CLASS_POINTER_MOTION,
            presented: true,
            lane_live: true,
            may_receive_input: true,
        }
    }

    fn binding(epoch: u64, classes: u64) -> InputBinding {
        InputBinding {
            producer_epoch: InputEpoch::new(epoch),
            context_id: 1,
            surface_id: 2,
            surface_generation: SurfaceGeneration::ONE,
            requested_classes: classes,
            reason: 6,
            requested_watchdog_us: DEFAULT_WATCHDOG_US,
        }
    }

    fn disabled(epoch: u64) -> InputBinding {
        InputBinding {
            producer_epoch: InputEpoch::new(epoch),
            context_id: 0,
            surface_id: 0,
            surface_generation: SurfaceGeneration::ZERO,
            requested_classes: 0,
            reason: 5,
            requested_watchdog_us: 0,
        }
    }

    #[test]
    fn an_eligible_binding_is_granted_and_narrowed() {
        let mut grant = InputGrant::new();
        let requested = INPUT_CLASS_KEYBOARD | INPUT_CLASS_POINTER_AXIS;
        let outcome = grant
            .apply(
                &binding(1, requested),
                &eligible(),
                Monotonic::from_micros(1_000_000),
            )
            .unwrap();
        assert_eq!(outcome, GrantOutcome::Enabled);
        // Axis was requested but is outside the surface capability mask, so it is narrowed away.
        assert_eq!(grant.effective_classes(), INPUT_CLASS_KEYBOARD);
        assert!(!grant.admits(INPUT_CLASS_POINTER_AXIS));
    }

    #[test]
    fn a_presenter_never_broadens_requested_classes() {
        let mut grant = InputGrant::new();
        grant
            .apply(
                &binding(1, INPUT_CLASS_KEYBOARD),
                &eligible(),
                Monotonic::from_micros(1_000_000),
            )
            .unwrap();
        assert_eq!(grant.effective_classes(), INPUT_CLASS_KEYBOARD);
    }

    #[test]
    fn every_precondition_denies_with_its_own_reason() {
        for (mutate, expected) in [
            (
                (|e: &mut Eligibility| e.lane_live = false) as fn(&mut Eligibility),
                reason::LANE_LOSS,
            ),
            (|e| e.may_receive_input = false, reason::AUTHORITY_LOSS),
            (|e| e.surface_present = false, reason::SURFACE_UNAVAILABLE),
            (|e| e.presented = false, reason::SURFACE_UNAVAILABLE),
            (|e| e.focused = false, reason::FOCUS_LOSS),
            (|e| e.consented = false, reason::LOCAL_POLICY),
        ] {
            let mut eligibility = eligible();
            mutate(&mut eligibility);
            let mut grant = InputGrant::new();
            let outcome = grant
                .apply(
                    &binding(1, INPUT_CLASS_KEYBOARD),
                    &eligibility,
                    Monotonic::from_micros(1_000_000),
                )
                .unwrap();
            assert_eq!(outcome, GrantOutcome::Denied { reason: expected });
            assert_eq!(grant.effective_classes(), 0);
        }
    }

    #[test]
    fn a_binding_naming_the_wrong_surface_generation_is_denied() {
        let mut grant = InputGrant::new();
        let mut wrong = binding(1, INPUT_CLASS_KEYBOARD);
        wrong.surface_generation = SurfaceGeneration::new(2);
        let outcome = grant
            .apply(&wrong, &eligible(), Monotonic::from_micros(1_000_000))
            .unwrap();
        assert_eq!(
            outcome,
            GrantOutcome::Denied {
                reason: reason::GENERATION_CHANGE
            }
        );
    }

    #[test]
    fn a_denied_grant_needs_a_greater_epoch_and_is_not_reinstated() {
        // Desktop §4: eligibility returning does not restore a grant.
        let mut grant = InputGrant::new();
        let mut ineligible = eligible();
        ineligible.focused = false;
        grant
            .apply(
                &binding(1, INPUT_CLASS_KEYBOARD),
                &ineligible,
                Monotonic::from_micros(1_000_000),
            )
            .unwrap();
        assert_eq!(grant.effective_classes(), 0);

        // The same epoch with the same bytes is an exact retry, not a new attempt.
        let outcome = grant
            .apply(
                &binding(1, INPUT_CLASS_KEYBOARD),
                &eligible(),
                Monotonic::from_micros(1_000_000),
            )
            .unwrap();
        assert_eq!(outcome, GrantOutcome::ExactRetry);
        assert_eq!(
            grant.effective_classes(),
            0,
            "an exact retry changes nothing"
        );

        // A strictly greater epoch does grant.
        let outcome = grant
            .apply(
                &binding(2, INPUT_CLASS_KEYBOARD),
                &eligible(),
                Monotonic::from_micros(1_000_000),
            )
            .unwrap();
        assert_eq!(outcome, GrantOutcome::Enabled);
    }

    #[test]
    fn a_lower_epoch_is_refused() {
        let mut grant = InputGrant::new();
        grant
            .apply(
                &binding(3, INPUT_CLASS_KEYBOARD),
                &eligible(),
                Monotonic::from_micros(1_000_000),
            )
            .unwrap();
        assert!(
            grant
                .apply(
                    &binding(2, INPUT_CLASS_KEYBOARD),
                    &eligible(),
                    Monotonic::from_micros(1_000_000)
                )
                .is_err()
        );
    }

    #[test]
    fn the_grant_generation_advances_on_every_change() {
        let mut grant = InputGrant::new();
        grant
            .apply(
                &binding(1, INPUT_CLASS_KEYBOARD),
                &eligible(),
                Monotonic::from_micros(1_000_000),
            )
            .unwrap();
        let first = grant.grant_generation();
        grant.revoke(reason::FOCUS_LOSS).unwrap();
        let after_revoke = grant.grant_generation();
        assert!(after_revoke > first, "revocation advances the generation");

        grant
            .apply(
                &binding(2, INPUT_CLASS_KEYBOARD),
                &eligible(),
                Monotonic::from_micros(1_000_000),
            )
            .unwrap();
        assert!(grant.grant_generation() > after_revoke);
    }

    #[test]
    fn revoking_without_a_grant_reports_nothing() {
        let mut grant = InputGrant::new();
        assert!(grant.revoke(reason::FOCUS_LOSS).is_none());
    }

    #[test]
    fn disabling_reports_disabled_and_clears_the_watchdog() {
        let mut grant = InputGrant::new();
        grant
            .apply(
                &binding(1, INPUT_CLASS_KEYBOARD),
                &eligible(),
                Monotonic::from_micros(1_000_000),
            )
            .unwrap();
        let outcome = grant
            .apply(&disabled(2), &eligible(), Monotonic::from_micros(1_000_000))
            .unwrap();
        assert_eq!(outcome, GrantOutcome::Disabled);
        assert_eq!(grant.effective_classes(), 0);
        let payload = grant.bound_payload(2);
        assert_eq!(payload[6].1.as_u64(), Some(STATE_DISABLED));
        assert_eq!(payload[8].1.as_u64(), Some(0));
    }

    #[test]
    fn a_renewal_is_due_at_half_the_effective_timeout() {
        let start = Monotonic::from_micros(1_000_000);
        let mut grant = InputGrant::new();
        grant
            .apply(&binding(1, INPUT_CLASS_KEYBOARD), &eligible(), start)
            .unwrap();
        assert!(
            grant.due_renewal(start).is_none(),
            "nothing is due immediately"
        );

        let half = DEFAULT_WATCHDOG_US / 2;
        let renewal = grant
            .due_renewal(start.checked_add_micros(half).unwrap())
            .expect("a renewal falls due");
        assert_eq!(renewal.sequence, 1);
        assert_eq!(renewal.watchdog_us, DEFAULT_WATCHDOG_US);
        // Renewing moves the deadline, so the next is due another half period on.
        assert!(
            grant
                .due_renewal(start.checked_add_micros(half).unwrap())
                .is_none()
        );
        let next = grant
            .due_renewal(start.checked_add_micros(half * 2).unwrap())
            .expect("the next renewal");
        assert_eq!(next.sequence, 2, "renewal sequences strictly increase");
    }

    #[test]
    fn a_watchdog_outside_the_registered_range_is_refused() {
        // Desktop §5.2 fixes the range at 250 ms to 5 s, and the binding schema enforces it, so a
        // presenter rejects an out-of-range request rather than quietly clamping it.
        let mut grant = InputGrant::new();
        let mut fast = binding(1, INPUT_CLASS_KEYBOARD);
        fast.requested_watchdog_us = 1;
        assert!(
            grant
                .apply(&fast, &eligible(), Monotonic::from_micros(1_000_000))
                .is_err()
        );

        let mut slow = binding(1, INPUT_CLASS_KEYBOARD);
        slow.requested_watchdog_us = u64::MAX;
        assert!(
            grant
                .apply(&slow, &eligible(), Monotonic::from_micros(1_000_000))
                .is_err()
        );
    }

    #[test]
    fn a_watchdog_at_the_range_edges_is_accepted_verbatim() {
        for requested in [MIN_WATCHDOG_US, MAX_WATCHDOG_US] {
            let mut grant = InputGrant::new();
            let mut edge = binding(1, INPUT_CLASS_KEYBOARD);
            edge.requested_watchdog_us = requested;
            grant
                .apply(&edge, &eligible(), Monotonic::from_micros(1_000_000))
                .unwrap();
            assert_eq!(grant.bound_payload(1)[8].1.as_u64(), Some(requested));
        }
    }

    #[test]
    fn an_event_is_tagged_only_for_a_granted_class() {
        let mut grant = InputGrant::new();
        grant
            .apply(
                &binding(1, INPUT_CLASS_KEYBOARD),
                &eligible(),
                Monotonic::from_micros(1_000_000),
            )
            .unwrap();
        let tag = grant
            .tag(INPUT_CLASS_KEYBOARD)
            .expect("a granted class is taggable");
        assert_eq!(tag.len(), 5, "the complete binding tuple");
        assert_eq!(tag[0].1.as_u64(), Some(1), "producer epoch");
        assert!(
            grant.tag(INPUT_CLASS_POINTER_MOTION).is_none(),
            "a class outside the grant is not taggable"
        );

        grant.revoke(reason::FOCUS_LOSS);
        assert!(
            grant.tag(INPUT_CLASS_KEYBOARD).is_none(),
            "revocation stops tagging"
        );
    }

    #[test]
    fn a_revocation_payload_names_the_grant_it_ended() {
        let mut grant = InputGrant::new();
        grant
            .apply(
                &binding(4, INPUT_CLASS_KEYBOARD),
                &eligible(),
                Monotonic::from_micros(1_000_000),
            )
            .unwrap();
        let payload = grant.revoke(reason::WATCHDOG).unwrap();
        assert_eq!(payload[0].1.as_u64(), Some(4), "the revoked producer epoch");
        assert_eq!(payload[3].1.as_u64(), Some(2), "the surface it named");
        assert_eq!(payload[5].1.as_u64(), Some(reason::WATCHDOG));
    }
}
