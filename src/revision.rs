//! Checked Vivid 1.5 revision and generation domains.
//!
//! Each domain is its own type, so a surface revision cannot be passed where a track revision is
//! expected. Values never wrap: [`advance`](SessionRevision::advance) fails at `u64::MAX`, and
//! zero is reserved for "none yet", which [`require_nonzero`](SessionRevision::require_nonzero)
//! rejects. Both return [`CounterError`].

use std::backtrace::{Backtrace, BacktraceStatus};
use std::{error::Error, fmt, io};

/// A revision or generation that is exhausted or zero where a live value is required.
#[derive(Debug)]
pub struct CounterError {
    domain: &'static str,
    exhausted: bool,
    backtrace: Backtrace,
}

impl CounterError {
    fn new(domain: &'static str, exhausted: bool) -> Self {
        Self {
            domain,
            exhausted,
            backtrace: Backtrace::capture(),
        }
    }

    /// Whether the counter was at `u64::MAX` and cannot advance.
    pub fn is_exhausted(&self) -> bool {
        self.exhausted
    }

    /// Whether the value was zero, which no live value may carry.
    pub fn is_zero(&self) -> bool {
        !self.exhausted
    }

    /// The domain that failed, such as `"surface revision"`.
    pub fn domain(&self) -> &'static str {
        self.domain
    }

    /// The backtrace captured when the error was created; usually disabled.
    pub fn backtrace(&self) -> &Backtrace {
        &self.backtrace
    }
}

impl fmt::Display for CounterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.exhausted {
            write!(formatter, "{} exhausted", self.domain)?;
        } else {
            write!(formatter, "{} must be nonzero", self.domain)?;
        }
        if self.backtrace.status() == BacktraceStatus::Captured {
            write!(formatter, "\n\nbacktrace:\n{}", self.backtrace)?;
        }
        Ok(())
    }
}

impl Error for CounterError {}

impl From<CounterError> for io::Error {
    fn from(error: CounterError) -> Self {
        io::Error::new(io::ErrorKind::InvalidData, error)
    }
}

macro_rules! counter_type {
    ($name:ident, $label:literal) => {
        #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(u64);

        impl $name {
            pub const ZERO: Self = Self(0);
            pub const ONE: Self = Self(1);

            pub const fn new(value: u64) -> Self {
                Self(value)
            }

            pub const fn get(self) -> u64 {
                self.0
            }

            /// Returns the next value in this domain.
            ///
            /// # Errors
            ///
            /// Returns an exhausted [`CounterError`] when the counter is at `u64::MAX`; a domain
            /// never wraps.
            pub fn advance(self) -> Result<Self, CounterError> {
                self.0
                    .checked_add(1)
                    .map(Self)
                    .ok_or_else(|| CounterError::new($label, true))
            }

            /// Returns `self` when it is a valid live value.
            ///
            /// # Errors
            ///
            /// Returns a zero [`CounterError`] for zero, which no live value in this domain may
            /// carry.
            pub fn require_nonzero(self) -> Result<Self, CounterError> {
                if self.0 == 0 {
                    Err(CounterError::new($label, false))
                } else {
                    Ok(self)
                }
            }
        }

        impl From<$name> for u64 {
            fn from(value: $name) -> Self {
                value.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }
    };
}

counter_type!(SessionRevision, "session revision");
counter_type!(ContextRevision, "context revision");
counter_type!(SurfaceRevision, "surface revision");
counter_type!(SurfaceGeneration, "surface generation");
counter_type!(TrackRevision, "track revision");
counter_type!(SceneRevision, "scene revision");
counter_type!(ObservationSequence, "observation sequence");
counter_type!(TargetGeneration, "target generation");
counter_type!(CapabilityGeneration, "capability generation");
counter_type!(ResumeGeneration, "lease resume generation");
counter_type!(ChannelGeneration, "channel generation");
counter_type!(InputEpoch, "input epoch");
counter_type!(GrantGeneration, "presenter grant generation");
counter_type!(FileDropEpoch, "file-drop epoch");
counter_type!(FileDropGrantGeneration, "file-drop grant generation");
counter_type!(FileTransferGeneration, "file-transfer generation");
counter_type!(MediaEpoch, "media epoch");

#[cfg(test)]
mod tests {
    use super::*;

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
}
