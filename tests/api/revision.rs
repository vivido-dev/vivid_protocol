//! Public-API tests for `vivid_protocol::revision`.
use std::io;
use vivid_protocol::revision::*;

#[test]
fn domains_advance_independently() {
    assert_eq!(SessionRevision::ONE.advance().unwrap().get(), 2);
    assert_eq!(SurfaceGeneration::ONE.advance().unwrap().get(), 2);
    assert_eq!(TrackRevision::ONE.advance().unwrap().get(), 2);
}

#[test]
fn exhaustion_and_zero_are_distinct_errors() {
    let exhausted = ChannelGeneration::new(u64::MAX).advance().unwrap_err();
    assert!(exhausted.is_exhausted() && !exhausted.is_zero());
    assert_eq!(exhausted.domain(), "channel generation");
    assert!(
        exhausted
            .to_string()
            .starts_with("channel generation exhausted")
    );

    let zero = SurfaceGeneration::ZERO.require_nonzero().unwrap_err();
    assert!(zero.is_zero() && !zero.is_exhausted());
    assert_eq!(io::Error::from(zero).kind(), io::ErrorKind::InvalidData);
}
