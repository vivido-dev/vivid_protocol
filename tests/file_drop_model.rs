//! Exhaustive finite-state exploration of the bounded `file-drop-v1` lifecycle.
//!
//! The two owners deliberately reuse every numeric identifier.  Transport loss, lost replies,
//! retries, revocation, cancellation, and expiry are independent transitions so the search covers
//! their interleavings.  File contents and offsets are collapsed to three finite buckets; the
//! protocol codec and flow-controller tests cover the concrete integer boundaries.

use std::collections::{HashSet, VecDeque};

#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, Eq)]
enum Phase {
    #[default]
    Unbound,
    Bound,
    Offered,
    Accepted,
    Transferring,
    Committed,
    Cancelled,
    Failed,
}

#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, Eq)]
struct DropState {
    phase: Phase,
    generation: u8,
    offset: u8,
    source_open: bool,
    temporary_visible: bool,
    final_visible: bool,
    result_cached: bool,
}

impl DropState {
    fn check(self) {
        assert_eq!(self.final_visible, self.phase == Phase::Committed);
        assert!(!self.temporary_visible || self.phase == Phase::Transferring);
        assert!(
            !self.source_open
                || matches!(
                    self.phase,
                    Phase::Offered | Phase::Accepted | Phase::Transferring
                )
        );
        assert_eq!(
            self.generation == 0,
            !matches!(
                self.phase,
                Phase::Accepted | Phase::Transferring | Phase::Committed
            )
        );
        assert!(
            !self.result_cached
                || matches!(
                    self.phase,
                    Phase::Committed | Phase::Cancelled | Phase::Failed
                )
        );
        assert!(self.offset <= 2);
    }
}

#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, Eq)]
struct Model {
    // A and B use the same context, surface, drop, transfer, and generation numbers.  Their array
    // slot represents the complete owner/session identity that must scope all lifecycle actions.
    owners: [DropState; 2],
}

#[derive(Clone, Copy, Debug)]
enum Transition {
    Bind,
    Offer,
    Accept,
    LoseAcceptReply,
    Open,
    GiveCreditAndData,
    LoseConnection,
    AdvanceGeneration,
    StaleConnectionData,
    FinishValid,
    FinishCorrupt,
    LoseResultReply,
    Query,
    Cancel,
    Revoke,
    Timeout,
    ExpireResult,
}

const TRANSITIONS: &[Transition] = &[
    Transition::Bind,
    Transition::Offer,
    Transition::Accept,
    Transition::LoseAcceptReply,
    Transition::Open,
    Transition::GiveCreditAndData,
    Transition::LoseConnection,
    Transition::AdvanceGeneration,
    Transition::StaleConnectionData,
    Transition::FinishValid,
    Transition::FinishCorrupt,
    Transition::LoseResultReply,
    Transition::Query,
    Transition::Cancel,
    Transition::Revoke,
    Transition::Timeout,
    Transition::ExpireResult,
];

fn apply(mut model: Model, owner: usize, transition: Transition) -> Option<Model> {
    let state = &mut model.owners[owner];
    match transition {
        Transition::Bind if state.phase == Phase::Unbound => state.phase = Phase::Bound,
        Transition::Offer if state.phase == Phase::Bound => {
            state.phase = Phase::Offered;
            state.source_open = true;
        }
        Transition::Accept if state.phase == Phase::Offered => {
            state.phase = Phase::Accepted;
            state.generation = 1;
        }
        // A named acceptance reply may be lost.  Retrying or querying observes the same IDs and
        // generation; it never creates a second logical transfer.
        Transition::LoseAcceptReply if state.phase == Phase::Accepted => {}
        Transition::Open if state.phase == Phase::Accepted => {
            state.phase = Phase::Transferring;
            state.temporary_visible = true;
        }
        Transition::GiveCreditAndData if state.phase == Phase::Transferring && state.offset < 2 => {
            state.offset += 1;
        }
        // Losing a bulk connection leaves the logical transfer and its committed prefix alive.
        Transition::LoseConnection if state.phase == Phase::Transferring => {
            state.phase = Phase::Accepted;
            state.temporary_visible = false;
        }
        Transition::AdvanceGeneration if state.phase == Phase::Accepted && state.generation < 2 => {
            state.generation += 1;
        }
        // Records from an old live connection are rejected without mutating the current transfer.
        Transition::StaleConnectionData if state.generation == 2 => {}
        Transition::FinishValid if state.phase == Phase::Transferring && state.offset == 2 => {
            state.phase = Phase::Committed;
            state.source_open = false;
            state.temporary_visible = false;
            state.final_visible = true;
            state.result_cached = true;
        }
        Transition::FinishCorrupt if state.phase == Phase::Transferring => {
            state.phase = Phase::Failed;
            state.generation = 0;
            state.source_open = false;
            state.temporary_visible = false;
            state.result_cached = true;
        }
        // Losing a terminal reply preserves the cached outcome for QUERY/retry reconciliation.
        Transition::LoseResultReply if state.result_cached => {}
        Transition::Query if state.result_cached => {}
        Transition::Cancel
            if matches!(
                state.phase,
                Phase::Offered | Phase::Accepted | Phase::Transferring
            ) =>
        {
            terminate(state, Phase::Cancelled);
        }
        Transition::Revoke
            if matches!(
                state.phase,
                Phase::Bound | Phase::Offered | Phase::Accepted | Phase::Transferring
            ) =>
        {
            if state.phase == Phase::Bound {
                *state = DropState::default();
            } else {
                terminate(state, Phase::Cancelled);
            }
        }
        Transition::Timeout
            if matches!(
                state.phase,
                Phase::Offered | Phase::Accepted | Phase::Transferring
            ) =>
        {
            terminate(state, Phase::Cancelled);
        }
        Transition::ExpireResult if state.result_cached => *state = DropState::default(),
        _ => return None,
    }
    Some(model)
}

fn terminate(state: &mut DropState, phase: Phase) {
    state.phase = phase;
    state.generation = 0;
    state.source_open = false;
    state.temporary_visible = false;
    state.result_cached = true;
}

#[test]
fn exhaustive_file_drop_lifecycle_is_owner_scoped_and_atomic() {
    let initial = Model::default();
    let mut queue = VecDeque::from([initial]);
    let mut visited = HashSet::from([initial]);
    let mut saw_resume = false;
    let mut saw_commit = false;
    let mut saw_isolated_cleanup = false;

    while let Some(model) = queue.pop_front() {
        for state in model.owners {
            state.check();
        }
        saw_resume |= model.owners.iter().any(|state| state.generation == 2);
        saw_commit |= model
            .owners
            .iter()
            .any(|state| state.phase == Phase::Committed);
        saw_isolated_cleanup |= model.owners[0].phase == Phase::Unbound
            && matches!(
                model.owners[1].phase,
                Phase::Offered | Phase::Accepted | Phase::Transferring | Phase::Committed
            );

        for owner in 0..2 {
            for &transition in TRANSITIONS {
                if let Some(next) = apply(model, owner, transition) {
                    // An operation scoped to one complete identity cannot alter the other owner,
                    // even though all of their local numeric IDs collide.
                    assert_eq!(next.owners[1 - owner], model.owners[1 - owner]);
                    if visited.insert(next) {
                        queue.push_back(next);
                    }
                }
            }
        }
    }

    assert!(saw_resume, "generation recovery was unreachable");
    assert!(saw_commit, "atomic terminal commit was unreachable");
    assert!(saw_isolated_cleanup, "owner-scoped cleanup was unreachable");
    assert!(visited.len() > 100, "state space was unexpectedly small");
}
