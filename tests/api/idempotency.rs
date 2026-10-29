//! Public-API tests for `vivid_protocol::idempotency`.
use vivid_protocol::idempotency::*;

#[test]
fn exact_retry_replays_without_double_admission() {
    let mut cache = IdempotencyCache::new(1);
    assert_eq!(cache.begin([1; 16], b"request"), BeginResult::Fresh);
    cache.complete([1; 16], b"result".to_vec()).unwrap();
    assert_eq!(
        cache.begin([1; 16], b"request"),
        BeginResult::Replay(b"result".to_vec())
    );
    assert_eq!(
        cache.begin([1; 16], b"different"),
        BeginResult::ConflictingReuse
    );
    assert_eq!(cache.begin([2; 16], b"request"), BeginResult::LimitExceeded);
}

#[test]
fn uncertain_pending_result_is_explicit_after_resume() {
    let mut cache = IdempotencyCache::new(1);
    assert_eq!(cache.begin([1; 16], b"request"), BeginResult::Fresh);
    cache.mark_unknown([1; 16]).unwrap();
    assert_eq!(
        cache.begin([1; 16], b"request"),
        BeginResult::UnknownOutcome
    );
}
