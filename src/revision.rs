//! Strongly typed, checked revision domains used by Vivid 1.1 observability.

use std::error::Error;
use std::fmt::{self, Display, Formatter};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RevisionExhausted {
    domain: &'static str,
}

impl RevisionExhausted {
    pub const fn domain(self) -> &'static str {
        self.domain
    }
}

impl Display for RevisionExhausted {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} exhausted", self.domain)
    }
}

impl Error for RevisionExhausted {}

macro_rules! revision_type {
    ($name:ident, $domain:literal) => {
        #[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(u64);

        impl $name {
            pub const ZERO: Self = Self(0);

            pub const fn new(value: u64) -> Self {
                Self(value)
            }

            pub const fn get(self) -> u64 {
                self.0
            }

            pub fn advance(self) -> Result<Self, RevisionExhausted> {
                self.0
                    .checked_add(1)
                    .map(Self)
                    .ok_or(RevisionExhausted { domain: $domain })
            }
        }

        impl From<u64> for $name {
            fn from(value: u64) -> Self {
                Self::new(value)
            }
        }

        impl From<$name> for u64 {
            fn from(value: $name) -> Self {
                value.get()
            }
        }
    };
}

revision_type!(SceneRevision, "scene revision");
revision_type!(SourceRevision, "source revision");
revision_type!(ObservationSequence, "observation sequence");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revision_domains_advance_without_aliasing() {
        assert_eq!(SceneRevision::ZERO.advance().unwrap().get(), 1);
        assert_eq!(SourceRevision::new(4).advance().unwrap().get(), 5);
        assert_eq!(ObservationSequence::new(8).advance().unwrap().get(), 9);
    }

    #[test]
    fn revision_exhaustion_is_typed_and_never_wraps() {
        for error in [
            SceneRevision::new(u64::MAX).advance().unwrap_err(),
            SourceRevision::new(u64::MAX).advance().unwrap_err(),
            ObservationSequence::new(u64::MAX).advance().unwrap_err(),
        ] {
            assert!(error.to_string().ends_with("exhausted"));
        }
        assert_eq!(
            SceneRevision::new(u64::MAX).advance().unwrap_err().domain(),
            "scene revision"
        );
    }
}
