//! Exhaustive finite-state exploration of the composed authority machine — spec §7's proof
//! obligation: *"Before a presenter advertises Vivid 1.5, its composed lease, suspension,
//! channel-generation, input-binding, and revocation state machines MUST be subjected to
//! exhaustive finite-state exploration or a stronger model-checking method."*
//!
//! The model drives the *real* `LeaseMachine`, `InputGrant`, `InputGate`, and `ChannelFlow` from
//! this crate, composed exactly as a presenter and its producer compose them: the presenter's
//! grant decides bindings from eligibility, the producer's gate admits or rejects injection, and
//! revocations propagate atomically, as the actor loop applies them. Lane and channel transport
//! slots are abstract, with generations collapsed to small buckets so the reachable space is
//! finite; every transition that must not wait on another is offered independently so the BFS
//! explores every interleaving.
//!
//! The invariants are written from the spec text, not from the implementation's behaviour, and
//! `the_model_catches_a_producer_gate_that_misses_a_revocation` proves the exploration notices
//! when one is broken.

use std::collections::{HashSet, VecDeque};

use vivid_protocol::grant::{
    Eligibility, GrantOutcome, InputGrant, Renewal, reason as grant_reason,
};
use vivid_protocol::input::{
    BindingOutcome, INPUT_CLASS_KEYBOARD, InputBinding, InputEvent, InputGate, InputTuple,
};
use vivid_protocol::lease::{AttemptDecision, CleanupPolicy, LeaseMachine, LeaseState};
use vivid_protocol::resource::ChannelFlow;
use vivid_protocol::revision::{GrantGeneration, InputEpoch, SurfaceGeneration};
use vivid_protocol::time::Monotonic;

// The minimum legal watchdog gives the fewest distinct deadline bands: armed-first-half,
// armed-second-half, expired. One time step crosses exactly one band.
const WATCHDOG_US: u64 = 250_000;
const TIME_STEP_US: u64 = WATCHDOG_US / 2;

// Generation and epoch buckets: small enough to bound the space, large enough that "old" and
// "new" generations can be alive at once and one further advance is always visible.
const MAX_EPOCH: u64 = 2;
const MAX_LANE_GENERATION: u64 = 2;
const MAX_CHANNEL_GENERATION: u64 = 2;
const MAX_SURFACE_GENERATION: u64 = 2;
const MAX_RESUME_GENERATION: u64 = 2;

// Every object ID is 1, deliberately: owner B reuses each of them, because scoped-identity bugs
// only show when the numbers collide.
const CONTEXT: u64 = 1;
const SURFACE: u64 = 1;
const SESSION: u64 = 1;

const FLOW_WINDOW_BYTES: u64 = 200;
const FLOW_WINDOW_RECORDS: u64 = 2;
const FLOW_RAISED_BYTES: u64 = 400;
const FLOW_RAISED_RECORDS: u64 = 4;
const MEDIA_BODY: u32 = 60;

const ATTEMPT: [u8; 16] = [0xa; 16];
const ATTEMPT_OTHER: [u8; 16] = [0xb; 16];
const CLIENT_NONCE: [u8; 32] = [0xc; 32];
const SERVER_NONCE: [u8; 32] = [0xd; 32];
const FINGERPRINT: [u8; 32] = [0xe; 32];
const HELLO_BODY: &[u8] = b"model-hello";
const WELCOME_BODY: &[u8] = b"model-welcome";

/// The transition alphabet. Every variant is offered from every state where it could apply, and
/// the exploration asserts each fires at least once.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum T {
    Activate,
    ActivateRetryExact,
    ActivateRetryDifferent,
    WelcomeCommitted,
    PostHelloAdmitted,
    ControlLostClean,
    ControlLostUnclean,
    ResumeValid,
    ResumeStale,
    GraceExpiry,
    LeaseRevoke,
    ParentRevoke,
    OldControlRecord,
    LaneOpen,
    LaneDuplicate,
    LaneLoss,
    LaneRetry,
    LaneStaleRecord,
    ChannelOpen,
    ChannelDuplicate,
    ChannelAccept,
    ChannelAcceptConfirmed,
    ChannelAcceptLost,
    ChannelAdvance,
    MediaAccept,
    MediaParseBegin,
    MediaParseEnd,
    FlowUpdateBegin,
    FlowUpdateComplete,
    ChannelLoss,
    OldTransportMedia,
    BindEnable,
    BindDisable,
    BindLowerEpoch,
    BindRetrySame,
    BindRetryDifferent,
    RenewSend,
    RenewDeliver,
    TimePass,
    WatchdogFire,
    FocusLoss,
    FocusGain,
    SurfaceGenerationChange,
    EventPress,
    EventRelease,
    EventOldTuple,
    EventWrongSurfaceGeneration,
    OtherActivate,
    OtherBind,
    OtherTeardown,
    CrossInject,
}

const ALL_TRANSITIONS: &[T] = &[
    T::Activate,
    T::ActivateRetryExact,
    T::ActivateRetryDifferent,
    T::WelcomeCommitted,
    T::PostHelloAdmitted,
    T::ControlLostClean,
    T::ControlLostUnclean,
    T::ResumeValid,
    T::ResumeStale,
    T::GraceExpiry,
    T::LeaseRevoke,
    T::ParentRevoke,
    T::OldControlRecord,
    T::LaneOpen,
    T::LaneDuplicate,
    T::LaneLoss,
    T::LaneRetry,
    T::LaneStaleRecord,
    T::ChannelOpen,
    T::ChannelDuplicate,
    T::ChannelAccept,
    T::ChannelAcceptConfirmed,
    T::ChannelAcceptLost,
    T::ChannelAdvance,
    T::MediaAccept,
    T::MediaParseBegin,
    T::MediaParseEnd,
    T::FlowUpdateBegin,
    T::FlowUpdateComplete,
    T::ChannelLoss,
    T::OldTransportMedia,
    T::BindEnable,
    T::BindDisable,
    T::BindLowerEpoch,
    T::BindRetrySame,
    T::BindRetryDifferent,
    T::RenewSend,
    T::RenewDeliver,
    T::TimePass,
    T::WatchdogFire,
    T::FocusLoss,
    T::FocusGain,
    T::SurfaceGenerationChange,
    T::EventPress,
    T::EventRelease,
    T::EventOldTuple,
    T::EventWrongSurfaceGeneration,
    T::OtherActivate,
    T::OtherBind,
    T::OtherTeardown,
    T::CrossInject,
];

/// The cases spec §7 enumerates by name, plus the recovery paths the desktop chapters add.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Case {
    SimultaneousActivation,
    LostWelcome,
    LostChannelAccepted,
    OldControlRecordIgnored,
    OldLaneRecordIgnored,
    OldChannelMediaIgnored,
    ClosedClean,
    SuspendedOnUncleanLoss,
    RetryableActivationLoss,
    Resumed,
    ExplicitRevoke,
    ParentRevoked,
    GraceExpired,
    InputRejectedBeforeEnable,
    InputInjected,
    InputRejectedAfterRevoke,
    InputAfterRenewal,
    RenewalLateRejected,
    ChannelLossDuringParse,
    ChannelLossDuringFlowUpdate,
    ChargedWhileSuspended,
    TwoOwnersCrossInjection,
    TwoOwnersIsolated,
    WatchdogRevocation,
    FocusRevocation,
    NoImplicitRestoration,
    StaleResumeRejected,
    DuplicateLaneBusy,
    LaneRetryRefusedAfterInput,
    DuplicateChannelReplayed,
    DuplicateChannelRefusedAfterMedia,
    DeniedWhileLaneDown,
    DeniedWhileUnfocused,
}

const ALL_CASES: &[Case] = &[
    Case::SimultaneousActivation,
    Case::LostWelcome,
    Case::LostChannelAccepted,
    Case::OldControlRecordIgnored,
    Case::OldLaneRecordIgnored,
    Case::OldChannelMediaIgnored,
    Case::ClosedClean,
    Case::SuspendedOnUncleanLoss,
    Case::RetryableActivationLoss,
    Case::Resumed,
    Case::ExplicitRevoke,
    Case::ParentRevoked,
    Case::GraceExpired,
    Case::InputRejectedBeforeEnable,
    Case::InputInjected,
    Case::InputRejectedAfterRevoke,
    Case::InputAfterRenewal,
    Case::RenewalLateRejected,
    Case::ChannelLossDuringParse,
    Case::ChannelLossDuringFlowUpdate,
    Case::ChargedWhileSuspended,
    Case::TwoOwnersCrossInjection,
    Case::TwoOwnersIsolated,
    Case::WatchdogRevocation,
    Case::FocusRevocation,
    Case::NoImplicitRestoration,
    Case::StaleResumeRejected,
    Case::DuplicateLaneBusy,
    Case::LaneRetryRefusedAfterInput,
    Case::DuplicateChannelReplayed,
    Case::DuplicateChannelRefusedAfterMedia,
    Case::DeniedWhileLaneDown,
    Case::DeniedWhileUnfocused,
];

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
struct Lane {
    generation: u64,
    live: bool,
    admitted_input: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum ChannelPhase {
    None,
    Attaching,
    AcceptedUnconfirmed,
    Established,
    Lost,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
struct Channel {
    generation: u64,
    phase: ChannelPhase,
    media_admitted: bool,
    parsing: bool,
    flow_update: bool,
}

impl Channel {
    const fn none() -> Self {
        Self {
            generation: 0,
            phase: ChannelPhase::None,
            media_admitted: false,
            parsing: false,
            flow_update: false,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Deadline {
    None,
    Early,
    Late,
    Expired,
}

#[derive(PartialEq, Eq, Hash)]
struct Key {
    lease_state: u8,
    resume_generation: u64,
    post_hello_admitted: bool,
    charged: bool,
    lane: Option<Lane>,
    channel: Channel,
    flow: (u64, u64),
    focused: bool,
    surface_generation: u64,
    deadline: Deadline,
    epoch: u64,
    grant_state: u64,
    gate_active: bool,
    has_retired: bool,
    held: bool,
    renewed: bool,
    has_pending_renewal: bool,
    generation_bucket: u64,
    other: (u8, bool, bool),
}

/// The second owner: an independent lease and gate driven by its own small alphabet, sharing
/// every numeric ID with owner A.
#[derive(Clone)]
struct OwnerB {
    lease: LeaseMachine,
    gate: InputGate,
    bound: bool,
}

#[derive(Clone)]
struct Model {
    // Owner A: the real machines, paired presenter-side and producer-side.
    lease: LeaseMachine,
    grant: InputGrant,
    gate: InputGate,
    lane: Option<Lane>,
    channel: Channel,
    flow: ChannelFlow,
    // Eligibility feeds.
    focused: bool,
    surface_generation: u64,
    now_us: u64,
    // Input bookkeeping.
    epoch: u64,
    last_binding: Option<InputBinding>,
    retired_tuple: Option<InputTuple>,
    held: bool,
    renewed: bool,
    /// A renewal the presenter sent that has not reached the producer's gate yet.
    pending_renewal: Option<Renewal>,
    // Lease bookkeeping.
    post_hello_admitted: bool,
    charged: bool,
    // The largest grant generation either side has reported on this path.
    max_generation_seen: u64,
    other: OwnerB,
    /// Injected regression: the producer's gate never learns about presenter revocations.
    broken_revocation: bool,
}

impl Model {
    fn new(broken_revocation: bool) -> Self {
        Self {
            lease: LeaseMachine::new(CleanupPolicy::SuspendOnUncleanLoss, 5_000_000),
            grant: InputGrant::new(),
            gate: InputGate::default(),
            lane: None,
            channel: Channel::none(),
            flow: ChannelFlow::default(),
            focused: true,
            surface_generation: 1,
            now_us: 0,
            epoch: 0,
            last_binding: None,
            retired_tuple: None,
            held: false,
            renewed: false,
            pending_renewal: None,
            post_hello_admitted: false,
            // A lease reservation is charged from the moment the lease is issued.
            charged: true,
            max_generation_seen: 0,
            other: OwnerB {
                lease: LeaseMachine::new(CleanupPolicy::SuspendOnUncleanLoss, 5_000_000),
                gate: InputGate::default(),
                bound: false,
            },
            broken_revocation,
        }
    }

    fn now(&self) -> Monotonic {
        Monotonic::from_micros(self.now_us)
    }

    fn deadline(&self) -> Deadline {
        let Some(active) = self.gate.active() else {
            return Deadline::None;
        };
        if self.now() >= active.watchdog_deadline {
            return Deadline::Expired;
        }
        if active
            .watchdog_deadline
            .saturating_elapsed_since(self.now())
            > WATCHDOG_US / 2
        {
            Deadline::Early
        } else {
            Deadline::Late
        }
    }

    /// The deduplication key: every abstract bucket, and nothing path-dependent. Concrete
    /// machines travel with the state; nothing the transitions consult is absent from the key
    /// unless it is a function of the key itself.
    fn key(&self) -> Key {
        Key {
            lease_state: self.lease.state() as u8,
            resume_generation: self
                .lease
                .resume_generation()
                .get()
                .min(MAX_RESUME_GENERATION),
            post_hello_admitted: self.post_hello_admitted,
            charged: self.charged,
            lane: self.lane,
            channel: self.channel,
            flow: (self.flow.sent_body_bytes, self.flow.sent_media_records),
            focused: self.focused,
            surface_generation: self.surface_generation,
            deadline: self.deadline(),
            epoch: self.epoch,
            grant_state: self.grant.state(),
            gate_active: self.gate.active().is_some(),
            has_retired: self.retired_tuple.is_some(),
            held: self.held,
            renewed: self.renewed,
            has_pending_renewal: self.pending_renewal.is_some(),
            generation_bucket: self.max_generation_seen.min(MAX_EPOCH * 4),
            other: (
                self.other.lease.state() as u8,
                self.other.gate.active().is_some(),
                self.other.bound,
            ),
        }
    }

    fn eligibility(&self) -> Eligibility {
        Eligibility {
            focused: self.focused,
            consented: true,
            surface_present: true,
            surface_generation: SurfaceGeneration::new(self.surface_generation),
            capability_mask: INPUT_CLASS_KEYBOARD,
            presented: true,
            lane_live: self.lane.is_some_and(|lane| lane.live),
            may_receive_input: true,
        }
    }

    fn enabled_binding(&self, epoch: u64) -> InputBinding {
        InputBinding {
            producer_epoch: InputEpoch::new(epoch),
            context_id: CONTEXT,
            surface_id: SURFACE,
            surface_generation: SurfaceGeneration::new(self.surface_generation),
            requested_classes: INPUT_CLASS_KEYBOARD,
            reason: 0,
            requested_watchdog_us: WATCHDOG_US,
        }
    }

    fn disabled_binding(&self, epoch: u64) -> InputBinding {
        InputBinding {
            producer_epoch: InputEpoch::new(epoch),
            context_id: 0,
            surface_id: 0,
            surface_generation: SurfaceGeneration::ZERO,
            requested_classes: 0,
            reason: 0,
            requested_watchdog_us: 0,
        }
    }

    fn key_event(&self, tuple: InputTuple, pressed: bool) -> InputEvent {
        InputEvent::Key {
            binding: tuple,
            usage: 0x04,
            pressed,
        }
    }

    /// The tuple the producer would stamp now: its active grant's, or a placeholder naming the
    /// current epoch and generation so a never-granted producer still has something to try.
    fn current_tuple(&self) -> InputTuple {
        self.gate.active().map_or(
            InputTuple {
                producer_epoch: InputEpoch::new(self.epoch.max(1)),
                grant_generation: GrantGeneration::new(self.max_generation_seen.max(1)),
                context_id: CONTEXT,
                surface_id: SURFACE,
                surface_generation: SurfaceGeneration::new(self.surface_generation),
            },
            |active| active.binding,
        )
    }

    /// Record a generation either side reported, asserting strict monotonicity along the path.
    fn observe_generation(&mut self, generation: u64, via: T, violations: &mut Vec<String>) {
        if generation <= self.max_generation_seen && generation != 0 {
            violations.push(format!(
                "I6 grant generation moved backwards: {generation} after {} (via {via:?})",
                self.max_generation_seen
            ));
        }
        self.max_generation_seen = self.max_generation_seen.max(generation);
    }

    /// The presenter revokes, and the producer's gate hears about it — unless the regression
    /// knob removed that propagation. Composite because the actor applies the loss in one pass:
    /// no interleaving exists between "the presenter learned" and "the grant died".
    fn presenter_revoke(&mut self, reason: u64, via: T, violations: &mut Vec<String>) {
        self.retired_tuple = self
            .gate
            .active()
            .map(|active| active.binding)
            .or(self.retired_tuple);
        self.pending_renewal = None;
        if self.grant.revoke(reason).is_some() && !self.broken_revocation {
            match self.gate.revoke() {
                Ok(generation) => {
                    if generation.get() != self.grant.grant_generation() {
                        violations.push(format!(
                            "I11 presenter/producer generations diverged on revoke: {} vs {} (via {via:?})",
                            generation.get(),
                            self.grant.grant_generation()
                        ));
                    }
                    self.observe_generation(generation.get(), via, violations);
                }
                Err(error) => violations.push(format!("gate revoke failed: {error} (via {via:?})")),
            }
        }
        if !self.broken_revocation {
            self.held = false;
        }
    }

    /// Everything that dies with the control transport.
    fn teardown_transports(&mut self) {
        self.lane = None;
        self.channel = Channel::none();
        self.flow = ChannelFlow::default();
    }

    fn attempt(&self) -> Result<AttemptDecision, vivid_protocol::lease::LeaseTransitionError> {
        let mut lease = self.lease.clone();
        lease.begin_activation(
            ATTEMPT,
            CLIENT_NONCE,
            HELLO_BODY,
            FINGERPRINT,
            SESSION,
            SERVER_NONCE,
            WELCOME_BODY.to_vec(),
        )
    }

    fn fire(&self, kind: T, report: &mut Report) -> Option<Model> {
        let mut next = self.clone();
        let mut violations = Vec::new();
        let result = (|| {
            match kind {
                T::Activate => {
                    if self.lease.state() != LeaseState::Issued {
                        return None;
                    }
                    match next.lease.begin_activation(
                        ATTEMPT,
                        CLIENT_NONCE,
                        HELLO_BODY,
                        FINGERPRINT,
                        SESSION,
                        SERVER_NONCE,
                        WELCOME_BODY.to_vec(),
                    ) {
                        Ok(AttemptDecision::Fresh { .. }) => Some(()),
                        other => {
                            violations.push(format!("activation was not fresh: {other:?}"));
                            Some(())
                        }
                    }
                }
                T::ActivateRetryExact => {
                    if !matches!(
                        self.lease.state(),
                        LeaseState::Reserved | LeaseState::Active
                    ) {
                        return None;
                    }
                    if matches!(next.attempt(), Ok(AttemptDecision::ExactReplay { .. })) {
                        if self.lease.state() == LeaseState::Active {
                            report.cases.insert(Case::RetryableActivationLoss);
                        }
                        // The retried attempt is fresh: admission must recur on it.
                        next.post_hello_admitted = false;
                    }
                    Some(())
                }
                T::ActivateRetryDifferent => {
                    if !matches!(
                        self.lease.state(),
                        LeaseState::Reserved | LeaseState::Active
                    ) || report.cases.contains(&Case::SimultaneousActivation)
                    {
                        return None;
                    }
                    report.cases.insert(Case::SimultaneousActivation);
                    let mut racing = next.lease.clone();
                    if racing
                        .begin_activation(
                            ATTEMPT_OTHER,
                            CLIENT_NONCE,
                            HELLO_BODY,
                            FINGERPRINT,
                            SESSION,
                            SERVER_NONCE,
                            WELCOME_BODY.to_vec(),
                        )
                        .is_ok()
                    {
                        violations.push("a different attempt displaced the pending one".to_owned());
                    }
                    Some(())
                }
                T::WelcomeCommitted => {
                    if self.lease.state() != LeaseState::Reserved {
                        return None;
                    }
                    if next.lease.commit_welcome().is_err() {
                        violations.push("welcome commit failed from Reserved".to_owned());
                    }
                    if self.lease.resume_generation().get() >= 1 {
                        report.cases.insert(Case::Resumed);
                    }
                    Some(())
                }
                T::PostHelloAdmitted => {
                    if self.lease.state() != LeaseState::Active || self.post_hello_admitted {
                        return None;
                    }
                    if next.lease.admit_post_hello().is_err() {
                        violations.push("post-hello admit failed from Active".to_owned());
                    }
                    next.post_hello_admitted = true;
                    Some(())
                }
                T::ControlLostClean => {
                    if !matches!(
                        self.lease.state(),
                        LeaseState::Reserved | LeaseState::Active
                    ) {
                        return None;
                    }
                    report.cases.insert(Case::ClosedClean);
                    next.presenter_revoke(grant_reason::PRESENTER_SHUTDOWN, kind, &mut violations);
                    if next.lease.confirm_transport_lost(true) != Ok(LeaseState::Closed) {
                        violations.push("a clean loss did not close the lease".to_owned());
                    }
                    next.charged = false;
                    next.teardown_transports();
                    Some(())
                }
                T::ControlLostUnclean => {
                    if !matches!(
                        self.lease.state(),
                        LeaseState::Reserved | LeaseState::Active
                    ) {
                        return None;
                    }
                    match (self.lease.state(), self.post_hello_admitted) {
                        (LeaseState::Reserved, _) => {
                            report.cases.insert(Case::LostWelcome);
                        }
                        (LeaseState::Active, false) => {
                            report.cases.insert(Case::RetryableActivationLoss);
                        }
                        (LeaseState::Active, true) => {
                            report.cases.insert(Case::SuspendedOnUncleanLoss);
                        }
                        _ => {}
                    }
                    next.presenter_revoke(grant_reason::SUSPENSION, kind, &mut violations);
                    let outcome = next.lease.confirm_transport_lost(false);
                    match (self.lease.state(), self.post_hello_admitted) {
                        (LeaseState::Reserved, _) | (LeaseState::Active, false) => {
                            // The loss is retryable: the machine stays put for an exact retry.
                            if outcome != Ok(self.lease.state()) {
                                violations
                                    .push(format!("a retryable loss moved the lease: {outcome:?}"));
                            }
                        }
                        _ => {
                            if outcome != Ok(LeaseState::Suspended) {
                                violations.push(format!(
                                    "an unclean post-admission loss did not suspend: {outcome:?}"
                                ));
                            } else {
                                report.cases.insert(Case::ChargedWhileSuspended);
                            }
                        }
                    }
                    next.teardown_transports();
                    Some(())
                }
                T::ResumeValid => {
                    if self.lease.state() != LeaseState::Suspended
                        || self.lease.resume_generation().get() >= MAX_RESUME_GENERATION
                    {
                        return None;
                    }
                    let generation = self.lease.resume_generation();
                    match next.lease.begin_resume(
                        generation,
                        ATTEMPT,
                        CLIENT_NONCE,
                        HELLO_BODY,
                        FINGERPRINT,
                        SESSION,
                        SERVER_NONCE,
                        WELCOME_BODY.to_vec(),
                    ) {
                        Ok(AttemptDecision::Fresh { .. }) => {
                            // The resumed attempt is fresh: admission must recur on it.
                            next.post_hello_admitted = false;
                        }
                        other => violations.push(format!("a valid resume was refused: {other:?}")),
                    }
                    Some(())
                }
                T::ResumeStale => {
                    if self.lease.state() != LeaseState::Suspended
                        || self.lease.resume_generation().get() == 0
                        || report.cases.contains(&Case::StaleResumeRejected)
                    {
                        return None;
                    }
                    report.cases.insert(Case::StaleResumeRejected);
                    let stale = self.lease.resume_generation().get() - 1;
                    if next
                        .lease
                        .begin_resume(
                            vivid_protocol::revision::ResumeGeneration::new(stale),
                            ATTEMPT,
                            CLIENT_NONCE,
                            HELLO_BODY,
                            FINGERPRINT,
                            SESSION,
                            SERVER_NONCE,
                            WELCOME_BODY.to_vec(),
                        )
                        .is_ok()
                    {
                        violations.push("a stale resume generation was accepted".to_owned());
                    }
                    Some(())
                }
                T::GraceExpiry => {
                    if self.lease.state() != LeaseState::Suspended {
                        return None;
                    }
                    report.cases.insert(Case::GraceExpired);
                    if next.lease.expire().is_err() {
                        violations.push("grace expiry failed from Suspended".to_owned());
                    }
                    next.charged = false;
                    Some(())
                }
                T::LeaseRevoke | T::ParentRevoke => {
                    if matches!(
                        self.lease.state(),
                        LeaseState::Closed | LeaseState::Revoked | LeaseState::Expired
                    ) || (kind == T::ParentRevoke && self.lease.state() != LeaseState::Active)
                    {
                        return None;
                    }
                    report.cases.insert(if kind == T::ParentRevoke {
                        Case::ParentRevoked
                    } else {
                        Case::ExplicitRevoke
                    });
                    // The other owner's machines must not notice this owner's teardown.
                    let probe = self.other_probe();
                    next.presenter_revoke(grant_reason::AUTHORITY_LOSS, kind, &mut violations);
                    if next.lease.revoke().is_err() {
                        violations.push("lease revoke failed".to_owned());
                    }
                    next.charged = false;
                    next.teardown_transports();
                    if next.other_probe() != probe {
                        violations
                            .push("one owner's teardown disturbed the other's state".to_owned());
                    } else {
                        report.cases.insert(Case::TwoOwnersIsolated);
                    }
                    Some(())
                }
                T::OldControlRecord => {
                    // A record arrives on the transport a suspension replaced. It changes
                    // nothing: the resumed session answers only its live transport.
                    if self.lease.state() != LeaseState::Active
                        || self.lease.resume_generation().get() == 0
                        || report.cases.contains(&Case::OldControlRecordIgnored)
                    {
                        return None;
                    }
                    report.cases.insert(Case::OldControlRecordIgnored);
                    Some(())
                }
                T::LaneOpen => {
                    if self.lease.state() != LeaseState::Active {
                        return None;
                    }
                    let generation = self.lane.map_or(1, |lane| lane.generation + 1);
                    if generation > MAX_LANE_GENERATION {
                        return None;
                    }
                    // A replacement kills the previous transport, and input with it.
                    if self.lane.is_some() {
                        next.presenter_revoke(grant_reason::LANE_LOSS, kind, &mut violations);
                    }
                    next.lane = Some(Lane {
                        generation,
                        live: true,
                        admitted_input: false,
                    });
                    Some(())
                }
                T::LaneDuplicate => {
                    if !self.lane.is_some_and(|lane| lane.live)
                        || report.cases.contains(&Case::DuplicateLaneBusy)
                    {
                        return None;
                    }
                    report.cases.insert(Case::DuplicateLaneBusy);
                    Some(())
                }
                T::LaneLoss => {
                    if !self.lane.is_some_and(|lane| lane.live) {
                        return None;
                    }
                    next.lane.as_mut().unwrap().live = false;
                    next.presenter_revoke(grant_reason::LANE_LOSS, kind, &mut violations);
                    Some(())
                }
                T::LaneRetry => {
                    let lane = self.lane?;
                    if lane.live {
                        return None;
                    }
                    if lane.admitted_input {
                        // A generation that carried input can never be reopened.
                        if report.cases.contains(&Case::LaneRetryRefusedAfterInput) {
                            return None;
                        }
                        report.cases.insert(Case::LaneRetryRefusedAfterInput);
                        return Some(());
                    }
                    next.lane.as_mut().unwrap().live = true;
                    Some(())
                }
                T::LaneStaleRecord => {
                    let lane = self.lane?;
                    if lane.generation < 2 || report.cases.contains(&Case::OldLaneRecordIgnored) {
                        return None;
                    }
                    report.cases.insert(Case::OldLaneRecordIgnored);
                    Some(())
                }
                T::ChannelOpen => {
                    if self.lease.state() != LeaseState::Active
                        || self.channel.phase != ChannelPhase::None
                    {
                        return None;
                    }
                    next.channel = Channel {
                        generation: 1,
                        phase: ChannelPhase::Attaching,
                        media_admitted: false,
                        parsing: false,
                        flow_update: false,
                    };
                    Some(())
                }
                T::ChannelDuplicate => {
                    if !matches!(
                        self.channel.phase,
                        ChannelPhase::Attaching
                            | ChannelPhase::AcceptedUnconfirmed
                            | ChannelPhase::Established
                    ) || (self.channel.media_admitted
                        && report
                            .cases
                            .contains(&Case::DuplicateChannelRefusedAfterMedia))
                        || (!self.channel.media_admitted
                            && report.cases.contains(&Case::DuplicateChannelReplayed))
                    {
                        return None;
                    }
                    if self.channel.media_admitted {
                        // An attachment that carried media is consumed; a re-open is refused.
                        report.cases.insert(Case::DuplicateChannelRefusedAfterMedia);
                    } else {
                        report.cases.insert(Case::DuplicateChannelReplayed);
                    }
                    Some(())
                }
                T::ChannelAccept => {
                    if self.channel.phase != ChannelPhase::Attaching {
                        return None;
                    }
                    next.channel.phase = ChannelPhase::AcceptedUnconfirmed;
                    next.flow = ChannelFlow::new(FLOW_WINDOW_BYTES, FLOW_WINDOW_RECORDS);
                    Some(())
                }
                T::ChannelAcceptConfirmed => {
                    if self.channel.phase != ChannelPhase::AcceptedUnconfirmed {
                        return None;
                    }
                    next.channel.phase = ChannelPhase::Established;
                    Some(())
                }
                T::ChannelAcceptLost => {
                    // CHANNEL_ACCEPTED left the presenter but the transport died before the
                    // producer saw it.
                    if self.channel.phase != ChannelPhase::AcceptedUnconfirmed {
                        return None;
                    }
                    report.cases.insert(Case::LostChannelAccepted);
                    next.channel.phase = ChannelPhase::Lost;
                    next.channel.parsing = false;
                    next.channel.flow_update = false;
                    Some(())
                }
                T::ChannelAdvance => {
                    if self.lease.state() != LeaseState::Active
                        || !matches!(
                            self.channel.phase,
                            ChannelPhase::Established | ChannelPhase::Lost
                        )
                        || self.channel.generation >= MAX_CHANNEL_GENERATION
                    {
                        return None;
                    }
                    next.channel = Channel {
                        generation: self.channel.generation + 1,
                        phase: ChannelPhase::Attaching,
                        media_admitted: false,
                        parsing: false,
                        flow_update: false,
                    };
                    Some(())
                }
                T::MediaAccept => {
                    if self.channel.phase != ChannelPhase::Established {
                        return None;
                    }
                    let admitted = next.flow.admit(MEDIA_BODY).is_ok();
                    if !admitted {
                        return None;
                    }
                    next.channel.media_admitted = true;
                    Some(())
                }
                T::MediaParseBegin => {
                    if !self.channel.media_admitted || self.channel.parsing {
                        return None;
                    }
                    next.channel.parsing = true;
                    Some(())
                }
                T::MediaParseEnd => {
                    if !self.channel.parsing {
                        return None;
                    }
                    next.channel.parsing = false;
                    Some(())
                }
                T::FlowUpdateBegin => {
                    if self.channel.phase != ChannelPhase::Established || self.channel.flow_update {
                        return None;
                    }
                    next.channel.flow_update = true;
                    Some(())
                }
                T::FlowUpdateComplete => {
                    if !self.channel.flow_update {
                        return None;
                    }
                    next.flow
                        .raise_maxima(FLOW_RAISED_BYTES, FLOW_RAISED_RECORDS);
                    next.channel.flow_update = false;
                    Some(())
                }
                T::ChannelLoss => {
                    if self.channel.phase != ChannelPhase::Established {
                        return None;
                    }
                    if self.channel.parsing {
                        report.cases.insert(Case::ChannelLossDuringParse);
                    }
                    if self.channel.flow_update {
                        report.cases.insert(Case::ChannelLossDuringFlowUpdate);
                    }
                    next.channel.phase = ChannelPhase::Lost;
                    next.channel.parsing = false;
                    next.channel.flow_update = false;
                    Some(())
                }
                T::OldTransportMedia => {
                    // Bytes on the transport a channel advance replaced: rejected, and the new
                    // generation's flow window must not move.
                    if self.channel.phase != ChannelPhase::Established
                        || self.channel.generation < 2
                        || report.cases.contains(&Case::OldChannelMediaIgnored)
                    {
                        return None;
                    }
                    report.cases.insert(Case::OldChannelMediaIgnored);
                    Some(())
                }
                T::BindEnable => {
                    if self.lease.state() != LeaseState::Active || self.epoch >= MAX_EPOCH {
                        return None;
                    }
                    let binding = next.enabled_binding(self.epoch + 1);
                    let eligibility = next.eligibility();
                    let outcome = match next.grant.apply(&binding, &eligibility, next.now()) {
                        Ok(outcome) => outcome,
                        Err(error) => {
                            violations.push(format!("a well-formed binding errored: {error}"));
                            return Some(());
                        }
                    };
                    next.retired_tuple = next
                        .gate
                        .active()
                        .map(|active| active.binding)
                        .or(next.retired_tuple);
                    match outcome {
                        GrantOutcome::Enabled => {
                            match next.gate.apply_binding(
                                binding.clone(),
                                INPUT_CLASS_KEYBOARD,
                                WATCHDOG_US,
                                next.now(),
                            ) {
                                Ok(BindingOutcome::Enabled(tuple)) => {
                                    if tuple.grant_generation.get() != next.grant.grant_generation()
                                    {
                                        violations.push(format!(
                                            "I11 generations diverged on enable: producer {} presenter {}",
                                            tuple.grant_generation.get(),
                                            next.grant.grant_generation()
                                        ));
                                    }
                                    next.observe_generation(
                                        tuple.grant_generation.get(),
                                        kind,
                                        &mut violations,
                                    );
                                }
                                other => violations.push(format!(
                                    "the producer gate refused an enabled binding: {other:?}"
                                )),
                            }
                            next.epoch += 1;
                            next.last_binding = Some(binding);
                            next.renewed = false;
                            next.held = false;
                        }
                        GrantOutcome::Denied { .. } => {
                            if !self.lane.is_some_and(|lane| lane.live) {
                                report.cases.insert(Case::DeniedWhileLaneDown);
                            }
                            if !self.focused {
                                report.cases.insert(Case::DeniedWhileUnfocused);
                            }
                            match next.gate.apply_binding(binding.clone(), 0, 0, next.now()) {
                                Ok(BindingOutcome::Denied(tuple)) => next.observe_generation(
                                    tuple.grant_generation.get(),
                                    kind,
                                    &mut violations,
                                ),
                                other => violations.push(format!(
                                    "the producer gate mishandled a denial: {other:?}"
                                )),
                            }
                            next.epoch += 1;
                            next.last_binding = Some(binding);
                            next.held = false;
                        }
                        other => violations
                            .push(format!("a fresh epoch was not a fresh outcome: {other:?}")),
                    }
                    Some(())
                }
                T::BindDisable => {
                    if self.last_binding.is_none() || self.epoch >= MAX_EPOCH {
                        return None;
                    }
                    let binding = next.disabled_binding(self.epoch + 1);
                    match next.grant.apply(&binding, &next.eligibility(), next.now()) {
                        Ok(GrantOutcome::Disabled) => {}
                        other => violations.push(format!("a disable was not disabled: {other:?}")),
                    }
                    next.retired_tuple = next
                        .gate
                        .active()
                        .map(|active| active.binding)
                        .or(next.retired_tuple);
                    match next.gate.apply_binding(binding.clone(), 0, 0, next.now()) {
                        Ok(BindingOutcome::Disabled) => {}
                        other => violations
                            .push(format!("the producer gate mishandled a disable: {other:?}")),
                    }
                    next.epoch += 1;
                    next.last_binding = Some(binding);
                    next.held = false;
                    next.renewed = false;
                    Some(())
                }
                T::BindLowerEpoch => {
                    if self.epoch < 2 {
                        return None;
                    }

                    let binding = next.enabled_binding(self.epoch - 1);
                    if next
                        .grant
                        .apply(&binding, &next.eligibility(), next.now())
                        .is_ok()
                    {
                        violations.push("a lower input epoch was accepted".to_owned());
                    }
                    Some(())
                }
                T::BindRetrySame => {
                    let binding = self.last_binding.clone()?;
                    match next.grant.apply(&binding, &next.eligibility(), next.now()) {
                        Ok(GrantOutcome::ExactRetry) => {}
                        Ok(GrantOutcome::Denied { .. })
                            if !self.lane.is_some_and(|lane| lane.live) || !self.focused =>
                        {
                            // A retry under *changed* eligibility is re-decided; that is the
                            // eligibility rule, not a retry failure.
                        }
                        other => {
                            violations.push(format!("an exact retry was not recognised: {other:?}"))
                        }
                    }
                    Some(())
                }
                T::BindRetryDifferent => {
                    let mut binding = self.last_binding.clone()?;
                    if binding.requested_classes == 0 {
                        return None;
                    }
                    binding.requested_classes = INPUT_CLASS_KEYBOARD << 1;
                    if next
                        .grant
                        .apply(&binding, &next.eligibility(), next.now())
                        .is_ok()
                    {
                        violations.push("an epoch was reused with different bytes".to_owned());
                    }
                    Some(())
                }
                T::RenewSend => {
                    // The presenter owes a renewal when its cadence says so. Sending extends the
                    // presenter's own bookkeeping immediately; delivery is a separate step, so
                    // the producer's deadline can legitimately expire while one is in flight.
                    if self.gate.active().is_none() || self.pending_renewal.is_some() {
                        return None;
                    }
                    let renewal = next.grant.due_renewal(next.now())?;
                    next.pending_renewal = Some(renewal);
                    Some(())
                }
                T::RenewDeliver => {
                    let renewal = self.pending_renewal?;
                    next.pending_renewal = None;
                    let Some(tuple) = next.gate.active().map(|active| active.binding) else {
                        // The grant died while the renewal was in flight.
                        report.cases.insert(Case::RenewalLateRejected);
                        return Some(());
                    };
                    match next
                        .gate
                        .renew(tuple, renewal.sequence, renewal.watchdog_us, next.now())
                    {
                        Ok(()) => next.renewed = true,
                        Err(_) => {
                            // Stale, late, or inconsistent: the producer refuses it, and the
                            // watchdog transition is what happens next if time ran out.
                            report.cases.insert(Case::RenewalLateRejected);
                        }
                    }
                    Some(())
                }
                T::TimePass => {
                    if self.gate.active().is_none() || self.deadline() == Deadline::Expired {
                        return None;
                    }
                    next.now_us += TIME_STEP_US;
                    Some(())
                }
                T::WatchdogFire => {
                    if self.gate.active().is_none() || self.deadline() != Deadline::Expired {
                        return None;
                    }
                    report.cases.insert(Case::WatchdogRevocation);
                    next.presenter_revoke(grant_reason::WATCHDOG, kind, &mut violations);
                    Some(())
                }
                T::FocusLoss => {
                    if !self.focused {
                        return None;
                    }
                    next.focused = false;
                    if self.gate.active().is_some() {
                        report.cases.insert(Case::FocusRevocation);
                        next.presenter_revoke(grant_reason::FOCUS_LOSS, kind, &mut violations);
                    }
                    Some(())
                }
                T::FocusGain => {
                    if self.focused {
                        return None;
                    }
                    next.focused = true;
                    // Spec §8: no implicit input restoration. Focus returning changes eligibility
                    // only; the producer must bind again with a fresh epoch.
                    if self.epoch >= 1 && next.gate.active().is_none() {
                        report.cases.insert(Case::NoImplicitRestoration);
                    }
                    Some(())
                }
                T::SurfaceGenerationChange => {
                    if self.surface_generation >= MAX_SURFACE_GENERATION || self.epoch == 0 {
                        return None;
                    }
                    next.surface_generation += 1;
                    // A binding names an exact surface generation; a new generation revokes it.
                    next.presenter_revoke(grant_reason::GENERATION_CHANGE, kind, &mut violations);
                    Some(())
                }
                T::EventPress | T::EventRelease => {
                    let pressed = kind == T::EventPress;
                    if pressed == self.held && self.gate.active().is_some() {
                        // Repeat transitions are suppressed at the source, and a release with
                        // nothing held is a no-op.
                        return None;
                    }
                    if self.gate.active().is_none()
                        && self.retired_tuple.is_none()
                        && report.cases.contains(&Case::InputRejectedBeforeEnable)
                    {
                        return None;
                    }
                    let tuple = next.current_tuple();
                    let event = next.key_event(tuple, pressed);
                    let surface_generation = SurfaceGeneration::new(next.surface_generation);
                    match next.gate.authorize(event, surface_generation, next.now()) {
                        Ok(()) => {
                            // I4: injection is authorized only while the whole eligibility
                            // chain holds — this is where a broken revocation is caught.
                            if !next.lane.is_some_and(|lane| lane.live) {
                                violations.push(format!(
                                    "I4 injected with the lane down ({}, via {kind:?})",
                                    if pressed { "press" } else { "release" }
                                ));
                            }
                            if next.deadline() == Deadline::Expired {
                                violations.push("I4 injected past the watchdog".to_owned());
                            }
                            if next.lease.state() != LeaseState::Active {
                                violations.push("I4 injected without an active lease".to_owned());
                            }
                            if pressed && self.gate.active().is_none() {
                                violations.push("I4 injected with no grant".to_owned());
                            }
                            // The live transport admitted ordinary input: this generation can
                            // never be replayed after a loss.
                            if let Some(lane) = next.lane.as_mut() {
                                lane.admitted_input = true;
                            }
                            next.held = pressed;
                            report.cases.insert(if self.renewed {
                                Case::InputAfterRenewal
                            } else {
                                Case::InputInjected
                            });
                        }
                        Err(_) => {
                            if self.gate.active().is_none() && self.retired_tuple.is_none() {
                                report.cases.insert(Case::InputRejectedBeforeEnable);
                            }
                        }
                    }
                    Some(())
                }
                T::EventOldTuple => {
                    let tuple = self.retired_tuple?;
                    let event = next.key_event(tuple, true);
                    let surface_generation = SurfaceGeneration::new(next.surface_generation);
                    if next
                        .gate
                        .authorize(event, surface_generation, next.now())
                        .is_ok()
                    {
                        violations.push("an old-tuple event was injected".to_owned());
                    } else {
                        report.cases.insert(Case::InputRejectedAfterRevoke);
                    }
                    Some(())
                }
                T::EventWrongSurfaceGeneration => {
                    next.gate.active()?;
                    let mut tuple = next.current_tuple();
                    tuple.surface_generation =
                        SurfaceGeneration::new(if self.surface_generation == 1 { 2 } else { 1 });
                    let event = next.key_event(tuple, true);
                    if next
                        .gate
                        .authorize(
                            event,
                            SurfaceGeneration::new(next.surface_generation),
                            next.now(),
                        )
                        .is_ok()
                    {
                        violations.push(
                            "an event for a stale surface generation was injected".to_owned(),
                        );
                    }
                    Some(())
                }
                T::OtherActivate => {
                    if next.other.lease.state() != LeaseState::Issued {
                        return None;
                    }
                    if next
                        .other
                        .lease
                        .begin_activation(
                            ATTEMPT,
                            CLIENT_NONCE,
                            HELLO_BODY,
                            FINGERPRINT,
                            SESSION,
                            SERVER_NONCE,
                            WELCOME_BODY.to_vec(),
                        )
                        .is_err()
                    {
                        violations.push("the other owner's activation failed".to_owned());
                    }
                    if next.other.lease.commit_welcome().is_err() {
                        violations.push("the other owner's welcome commit failed".to_owned());
                    }
                    Some(())
                }
                T::OtherBind => {
                    if next.other.lease.state() != LeaseState::Active || next.other.bound {
                        return None;
                    }
                    let binding = next.enabled_binding(1);
                    match next.other.gate.apply_binding(
                        binding,
                        INPUT_CLASS_KEYBOARD,
                        WATCHDOG_US,
                        next.now(),
                    ) {
                        Ok(BindingOutcome::Enabled(_)) => next.other.bound = true,
                        other => {
                            violations.push(format!("the other owner's binding failed: {other:?}"))
                        }
                    }
                    Some(())
                }
                T::OtherTeardown => {
                    if next.other.lease.state() != LeaseState::Active {
                        return None;
                    }
                    if next.other.lease.revoke().is_err() {
                        violations.push("the other owner's revoke failed".to_owned());
                    }
                    if next.other.gate.revoke().is_err() {
                        violations.push("the other owner's gate revoke failed".to_owned());
                    }
                    Some(())
                }
                T::CrossInject => {
                    if next.other.lease.state() != LeaseState::Active {
                        return None;
                    }
                    // An event stamped with this owner's tuple meets the other owner's gate,
                    // whose IDs are identical. It may pass only when the other owner
                    // independently granted that exact tuple — anything else is a leak.
                    let tuple = self.current_tuple();
                    let event = next.key_event(tuple, true);
                    let outcome = next.other.gate.authorize(
                        event,
                        SurfaceGeneration::new(next.surface_generation),
                        next.now(),
                    );
                    match (outcome, next.other.gate.active()) {
                        (Ok(()), Some(active)) if active.binding == tuple => {}
                        (Ok(()), _) => violations.push(
                            "one owner's event was authorized by the other's gate".to_owned(),
                        ),
                        (Err(_), _) => {}
                    }
                    report.cases.insert(Case::TwoOwnersCrossInjection);
                    Some(())
                }
            }
        })();
        report.violations.extend(
            violations
                .into_iter()
                .map(|violation| format!("{violation} (via {kind:?})")),
        );
        result.map(|()| next)
    }

    fn other_probe(&self) -> (LeaseState, bool, bool) {
        (
            self.other.lease.state(),
            self.other.gate.active().is_some(),
            self.other.bound,
        )
    }
}

#[derive(Default)]
struct Report {
    fired: HashSet<T>,
    cases: HashSet<Case>,
    violations: Vec<String>,
    states_explored: usize,
}

/// The invariants, from the spec text. Every generated state is checked, whether or not its key
/// was already seen — different concrete paths to one abstract state are different evidence.
fn check_invariants(state: &Model, via: T, report: &mut Report) {
    let lease_state = state.lease.state();
    let lane_live = state.lane.is_some_and(|lane| lane.live);

    // I1: a live interactive lane exists only on an active lease.
    if lane_live && lease_state != LeaseState::Active {
        report.violations.push(format!(
            "I1 lane live while lease is {lease_state:?} (via {via:?})"
        ));
    }
    // I2/I3: an active injection grant requires a live lane and an active lease.
    if state.gate.active().is_some() {
        if !lane_live {
            report
                .violations
                .push(format!("I2 grant active with the lane down (via {via:?})"));
        }
        if lease_state != LeaseState::Active {
            report.violations.push(format!(
                "I3 grant active while lease is {lease_state:?} (via {via:?})"
            ));
        }
    }
    // I12: presenter and producer agree on whether a grant exists at all.
    if state.grant.active().is_some() != state.gate.active().is_some() {
        report.violations.push(format!(
            "I12 presenter/producer grant presence diverged (via {via:?})"
        ));
    }
    // I5: a held key must still be releasable — anything that makes the release
    // non-injectable must have cleared the held set in the same atomic step. The one physical
    // exception is the watchdog race: time passes between the deadline and the revocation,
    // and the watchdog transition is what unsticks the held set.
    if state.held {
        let event = state.key_event(state.current_tuple(), false);
        if state
            .gate
            .authorize(
                event,
                SurfaceGeneration::new(state.surface_generation),
                state.now(),
            )
            .is_err()
            && state.deadline() != Deadline::Expired
        {
            report.violations.push(format!(
                "I5 a held key is stuck for a reason other than the watchdog race (via {via:?})"
            ));
        }
    }
    // I7: the reservation is charged in every non-terminal lease state, suspended
    // included, and never in a terminal one.
    let terminal = matches!(
        lease_state,
        LeaseState::Closed | LeaseState::Revoked | LeaseState::Expired
    );
    if state.charged == terminal {
        report.violations.push(format!(
            "I7 charging is {} while the lease is {lease_state:?} (via {via:?})",
            if state.charged { "on" } else { "off" }
        ));
    }
    if !terminal && lease_state == LeaseState::Suspended && state.charged {
        report.cases.insert(Case::ChargedWhileSuspended);
    }
    // I8: the flow window bounds what was admitted.
    if state.flow.sent_body_bytes > state.flow.maximum_body_bytes
        || state.flow.sent_media_records > state.flow.maximum_media_records
    {
        report
            .violations
            .push(format!("I8 flow window exceeded (via {via:?})"));
    }
    // I9: at most one live lane transport per generation — structural in this model,
    // asserted so a future multi-slot lane cannot silently break it.
    if state
        .lane
        .is_some_and(|lane| lane.live && lane.generation == 0)
    {
        report
            .violations
            .push(format!("I9 lane generation is zero (via {via:?})"));
    }
}

fn explore(broken_revocation: bool) -> Report {
    let mut report = Report::default();
    let initial = Model::new(broken_revocation);
    let mut seen = HashSet::new();
    let mut queue = VecDeque::new();
    check_invariants(&initial, T::Activate, &mut report);
    seen.insert(initial.key());
    queue.push_back(initial);
    while let Some(state) = queue.pop_front() {
        report.states_explored += 1;
        for kind in ALL_TRANSITIONS.iter().copied() {
            if let Some(next) = state.fire(kind, &mut report) {
                report.fired.insert(kind);
                check_invariants(&next, kind, &mut report);
                if seen.insert(next.key()) {
                    queue.push_back(next);
                }
            }
        }
        // The regression run exists to find one violation; the clean run must see everything.
        if broken_revocation && !report.violations.is_empty() {
            break;
        }
    }
    report
}

#[test]
fn the_composed_authority_machine_is_exhaustively_safe() {
    let report = explore(false);
    assert!(
        report.violations.is_empty(),
        "{} invariant violations:\n{}",
        report.violations.len(),
        report.violations[..report.violations.len().min(12)].join("\n")
    );
    for kind in ALL_TRANSITIONS {
        assert!(
            report.fired.contains(kind),
            "transition {kind:?} never fired; the exploration does not cover the alphabet"
        );
    }
    for case in ALL_CASES {
        assert!(
            report.cases.contains(case),
            "case {case:?} was never reached; spec §7's enumeration is not covered"
        );
    }
    eprintln!(
        "explored {} states, {} transitions fired, {} cases reached",
        report.states_explored,
        report.fired.len(),
        report.cases.len()
    );
}

#[test]
fn the_model_catches_a_producer_gate_that_misses_a_revocation() {
    // The injected regression: presenter revocations never reach the producer's injection gate,
    // so it keeps authorizing after the lane died. The exploration must notice.
    let report = explore(true);
    assert!(
        !report.violations.is_empty(),
        "the model failed to catch a producer gate injecting after revocation"
    );
    assert!(
        report
            .violations
            .iter()
            .any(|violation| violation.starts_with("I4 ") || violation.starts_with("I12 ")),
        "the caught violation should be about injection after revocation, got:
{}",
        report.violations[..report.violations.len().min(8)].join("\n")
    );
}
