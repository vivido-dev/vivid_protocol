//! Checked Vivid 1.5 revision and generation domains.

use std::{fmt, io};

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

            pub fn advance(self) -> io::Result<Self> {
                self.0.checked_add(1).map(Self).ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidData, concat!($label, " exhausted"))
                })
            }

            pub fn require_nonzero(self) -> io::Result<Self> {
                if self.0 == 0 {
                    Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        concat!($label, " must be nonzero"),
                    ))
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
    fn exhaustion_is_an_error() {
        assert_eq!(
            ChannelGeneration::new(u64::MAX)
                .advance()
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
    }
}
