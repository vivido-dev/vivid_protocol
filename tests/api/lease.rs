//! Public-API tests for `vivid_protocol::lease`.
use vivid_protocol::lease::*;
use vivid_protocol::revision::ResumeGeneration;

#[test]
fn lost_welcome_retries_same_nonsecret_outcome() {
    let mut lease = LeaseMachine::new(CleanupPolicy::SuspendOnUncleanLoss, 1_000_000);
    let first = lease
        .begin_activation(EstablishmentAttempt {
            attempt_id: [1; 16],
            client_nonce: [2; 32],
            hello_bytes: b"hello",
            profile_fingerprint: [4; 32],
            session_id: 7,
            server_nonce: [3; 32],
            welcome: b"welcome".to_vec(),
        })
        .unwrap();
    let retry = lease
        .begin_activation(EstablishmentAttempt {
            attempt_id: [1; 16],
            client_nonce: [2; 32],
            hello_bytes: b"hello",
            profile_fingerprint: [4; 32],
            session_id: 99,
            server_nonce: [9; 32],
            welcome: b"other".to_vec(),
        })
        .unwrap();
    assert!(matches!(
        first,
        AttemptDecision::Fresh { session_id: 7, .. }
    ));
    assert!(matches!(
        retry,
        AttemptDecision::ExactReplay { session_id: 7, .. }
    ));
    assert_eq!(
        lease.begin_activation(EstablishmentAttempt {
            attempt_id: [4; 16],
            client_nonce: [2; 32],
            hello_bytes: b"hello",
            profile_fingerprint: [4; 32],
            session_id: 8,
            server_nonce: [3; 32],
            welcome: Vec::new()
        }),
        Err(LeaseTransitionError::AuthenticationFailed)
    );
}

#[test]
fn active_lease_suspends_and_resume_generation_advances_once() {
    let mut lease = LeaseMachine::new(CleanupPolicy::SuspendOnUncleanLoss, 1_000_000);
    lease
        .begin_activation(EstablishmentAttempt {
            attempt_id: [1; 16],
            client_nonce: [2; 32],
            hello_bytes: b"hello",
            profile_fingerprint: [4; 32],
            session_id: 7,
            server_nonce: [3; 32],
            welcome: Vec::new(),
        })
        .unwrap();
    lease.commit_welcome().unwrap();
    lease.admit_post_hello().unwrap();
    assert_eq!(
        lease.confirm_transport_lost(false).unwrap(),
        LeaseState::Suspended
    );
    lease
        .begin_resume(
            ResumeGeneration::ZERO,
            EstablishmentAttempt {
                attempt_id: [4; 16],
                client_nonce: [5; 32],
                hello_bytes: b"resume",
                profile_fingerprint: [4; 32],
                session_id: 7,
                server_nonce: [6; 32],
                welcome: Vec::new(),
            },
        )
        .unwrap();
    assert_eq!(lease.resume_generation(), ResumeGeneration::ONE);
    assert_eq!(
        lease.begin_resume(
            ResumeGeneration::ZERO,
            EstablishmentAttempt {
                attempt_id: [7; 16],
                client_nonce: [8; 32],
                hello_bytes: b"different",
                profile_fingerprint: [4; 32],
                session_id: 7,
                server_nonce: [9; 32],
                welcome: Vec::new()
            }
        ),
        Err(LeaseTransitionError::AuthenticationFailed)
    );
}

#[test]
fn transport_loss_before_post_hello_allows_only_exact_activation_retry() {
    let mut lease = LeaseMachine::new(CleanupPolicy::SuspendOnUncleanLoss, 1_000_000);
    lease
        .begin_activation(EstablishmentAttempt {
            attempt_id: [1; 16],
            client_nonce: [2; 32],
            hello_bytes: b"hello",
            profile_fingerprint: [4; 32],
            session_id: 7,
            server_nonce: [3; 32],
            welcome: b"welcome".to_vec(),
        })
        .unwrap();
    lease.commit_welcome().unwrap();
    assert_eq!(
        lease.confirm_transport_lost(false).unwrap(),
        LeaseState::Active
    );
    assert!(matches!(
        lease
            .begin_activation(EstablishmentAttempt {
                attempt_id: [1; 16],
                client_nonce: [2; 32],
                hello_bytes: b"hello",
                profile_fingerprint: [4; 32],
                session_id: 99,
                server_nonce: [9; 32],
                welcome: Vec::new()
            })
            .unwrap(),
        AttemptDecision::ExactReplay { session_id: 7, .. }
    ));
}
