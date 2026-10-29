//! Public-API tests for `vivid_protocol::time`.
use vivid_protocol::time::*;

#[test]
fn arithmetic_is_checked_at_both_ends() {
    let start = Monotonic::from_micros(1_000);
    assert_eq!(start.checked_add_micros(500).unwrap().as_micros(), 1_500);
    assert_eq!(start.checked_sub_micros(500).unwrap().as_micros(), 500);
    assert!(
        Monotonic::from_micros(u64::MAX)
            .checked_add_micros(1)
            .is_none()
    );
    assert!(start.checked_sub_micros(1_001).is_none());
}

#[test]
fn elapsed_saturates_rather_than_wrapping() {
    let earlier = Monotonic::from_micros(10);
    let later = Monotonic::from_micros(30);
    assert_eq!(later.saturating_elapsed_since(earlier), 20);
    assert_eq!(
        earlier.saturating_elapsed_since(later),
        0,
        "a point before the origin reports no elapsed time rather than wrapping"
    );
}

#[test]
fn ordering_follows_the_underlying_microseconds() {
    assert!(Monotonic::from_micros(1) < Monotonic::from_micros(2));
    assert_eq!(Monotonic::ZERO.as_micros(), 0);
}
