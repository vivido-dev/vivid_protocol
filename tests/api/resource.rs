//! Public-API tests for `vivid_protocol::resource`.
use std::time::Duration;
use vivid_protocol::cbor::Value;
use vivid_protocol::resource::*;

#[test]
fn a_token_bucket_reports_the_exact_wait_for_a_charge_it_cannot_yet_admit() {
    let mut bucket = TokenBucket::new(1_000, 1_000);
    assert_eq!(
        bucket.time_until(1_000),
        Ok(None),
        "a full bucket admits its capacity now"
    );
    bucket.charge(1_000).unwrap();
    // Empty, earning 1000 tokens per second: 250 tokens is a quarter of a second.
    assert_eq!(bucket.time_until(250), Ok(Some(Duration::from_millis(250))));
    // Waiting exactly that long must be enough, not one token short.
    bucket.replenish(Duration::from_millis(250)).unwrap();
    assert_eq!(bucket.time_until(250), Ok(None));
    bucket.charge(250).unwrap();
}

#[test]
fn a_charge_larger_than_the_bucket_is_refused_rather_than_waited_out() {
    let bucket = TokenBucket::new(1_000, 1_000);
    bucket.time_until(1_001).unwrap_err();
    // A bucket that earns nothing can never admit a charge it does not already hold.
    let mut idle = TokenBucket::new(0, 10);
    assert_eq!(idle.time_until(10), Ok(None));
    idle.charge(10).unwrap();
    idle.time_until(1).unwrap_err();
}

#[test]
fn contract_requires_all_fields() {
    let value = ResourceContract::denied().to_value();
    assert_eq!(
        ResourceContract::from_value(&value).unwrap(),
        ResourceContract::denied()
    );
    assert!(matches!(
        ResourceContract::from_value(&Value::Map(vec![])),
        Err(ResourceError::MissingKey(0))
    ));
}

#[test]
fn flow_is_absolute_and_lower_updates_are_ignored() {
    let mut flow = ChannelFlow::new(10, 2);
    flow.admit(7).unwrap();
    assert_eq!(flow.admit(4), Err(ResourceError::FlowControl));
    flow.raise_maxima(20, 3);
    flow.admit(4).unwrap();
    flow.raise_maxima(1, 1);
    assert_eq!(flow.maximum_body_bytes, 20);
    assert_eq!(flow.maximum_media_records, 3);
}
