//! Public error types are `std::error::Error` with stable, readable messages.
use std::error::Error;

use vivid_protocol::{
    idempotency::CompletionError, input::InjectionRejection, lease::LeaseTransitionError,
    media::SizeError,
};

fn message(error: impl Error) -> String {
    error.to_string()
}

#[test]
fn state_machine_errors_render_readable_messages() {
    let completion = [
        CompletionError::NotReserved,
        CompletionError::AlreadyComplete,
    ]
    .map(message);
    let lease = [
        LeaseTransitionError::AuthenticationFailed,
        LeaseTransitionError::BadState,
        LeaseTransitionError::StaleResumeGeneration,
        LeaseTransitionError::Exhausted,
    ]
    .map(message);
    let injection = [
        InjectionRejection::NoActiveGrant,
        InjectionRejection::StaleTuple,
        InjectionRejection::SurfaceGenerationChanged,
        InjectionRejection::WatchdogExpired,
        InjectionRejection::ClassNotGranted,
    ]
    .map(message);
    let all: Vec<&String> = completion.iter().chain(&lease).chain(&injection).collect();
    for (index, text) in all.iter().enumerate() {
        assert!(text.contains(' '), "{text:?} is not a sentence");
        assert!(!all[..index].contains(text), "{text:?} is not distinct");
    }
}

/// `vivi` reports oversized images through `io::Error::other(SizeError)` and matches on the
/// variant name, so this text must not change without updating it.
#[test]
fn size_error_renders_its_variant_name() {
    assert_eq!(SizeError::TooLarge.to_string(), "TooLarge");
    assert_eq!(SizeError::Overflow.to_string(), "Overflow");
    assert_eq!(SizeError::Empty.to_string(), "Empty");
}
