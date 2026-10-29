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
    let size = [SizeError::Overflow, SizeError::TooLarge, SizeError::Empty].map(message);
    let all: Vec<&String> = completion
        .iter()
        .chain(&lease)
        .chain(&injection)
        .chain(&size)
        .collect();
    for (index, text) in all.iter().enumerate() {
        assert!(text.contains(' '), "{text:?} is not a sentence");
        assert!(!all[..index].contains(text), "{text:?} is not distinct");
    }
}
