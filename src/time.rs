//! A monotonic instant the protocol never reads for itself.
//!
//! Every deadline in this crate is computed from a value the caller supplies, and nothing here
//! calls a clock. That is deliberate: the input watchdog and the lease grace have to run
//! identically in a native presenter driven by `std::time::Instant` and in a browser presenter
//! driven by `performance.now()`, and `Instant` cannot be constructed from a JavaScript number.
//!
//! The origin is the caller's, not this crate's. Two values are only comparable when they came
//! from the same clock, which is the caller's responsibility exactly as it is with `Instant`.

/// A monotonic point in time, in microseconds from a caller-chosen origin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Hash)]
pub struct Monotonic(u64);

impl Monotonic {
    pub const ZERO: Self = Self(0);

    pub const fn from_micros(micros: u64) -> Self {
        Self(micros)
    }

    pub const fn as_micros(self) -> u64 {
        self.0
    }

    /// Move forward, refusing to wrap.
    pub const fn checked_add_micros(self, micros: u64) -> Option<Self> {
        match self.0.checked_add(micros) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    /// Move back, refusing to go before the origin.
    pub const fn checked_sub_micros(self, micros: u64) -> Option<Self> {
        match self.0.checked_sub(micros) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    /// Microseconds from `earlier` to here, or zero when `earlier` is later.
    ///
    /// Saturating rather than checked because a caller comparing two points from one monotonic
    /// clock cannot meaningfully act on "negative elapsed"; it means the two came from different
    /// origins, and a deadline computed from it would be nonsense either way.
    pub const fn saturating_elapsed_since(self, earlier: Self) -> u64 {
        self.0.saturating_sub(earlier.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
