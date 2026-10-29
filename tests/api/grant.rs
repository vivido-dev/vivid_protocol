//! Public-API tests for `vivid_protocol::grant`.
use vivid_protocol::grant::*;
use vivid_protocol::input::{
    INPUT_CLASS_KEYBOARD, INPUT_CLASS_POINTER_AXIS, INPUT_CLASS_POINTER_MOTION,
};
use vivid_protocol::revision::InputEpoch;
use vivid_protocol::{
    input::{InputBinding, MAX_WATCHDOG_US, MIN_WATCHDOG_US},
    revision::SurfaceGeneration,
    time::Monotonic,
};

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
    grant
        .apply(
            &binding(2, INPUT_CLASS_KEYBOARD),
            &eligible(),
            Monotonic::from_micros(1_000_000),
        )
        .unwrap_err();
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
fn a_stalled_presenter_observes_watchdog_expiry_instead_of_renewing_late() {
    let start = Monotonic::from_micros(1_000_000);
    let mut grant = InputGrant::new();
    grant
        .apply(&binding(1, INPUT_CLASS_KEYBOARD), &eligible(), start)
        .unwrap();
    assert!(!grant.watchdog_expired(start.checked_add_micros(DEFAULT_WATCHDOG_US - 1).unwrap()));
    assert!(grant.watchdog_expired(start.checked_add_micros(DEFAULT_WATCHDOG_US).unwrap()));
    assert!(
        grant
            .due_renewal(start.checked_add_micros(DEFAULT_WATCHDOG_US).unwrap())
            .is_none()
    );
}

#[test]
fn a_watchdog_outside_the_registered_range_is_refused() {
    // Desktop §5.2 fixes the range at 250 ms to 5 s, and the binding schema enforces it, so a
    // presenter rejects an out-of-range request rather than quietly clamping it.
    let mut grant = InputGrant::new();
    let mut fast = binding(1, INPUT_CLASS_KEYBOARD);
    fast.requested_watchdog_us = 1;
    grant
        .apply(&fast, &eligible(), Monotonic::from_micros(1_000_000))
        .unwrap_err();

    let mut slow = binding(1, INPUT_CLASS_KEYBOARD);
    slow.requested_watchdog_us = u64::MAX;
    grant
        .apply(&slow, &eligible(), Monotonic::from_micros(1_000_000))
        .unwrap_err();
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
