//! Complete owner-scoped identities for Vivid 1.5 state and cleanup.

use std::fmt;

use crate::revision::{ChannelGeneration, GrantGeneration, InputEpoch, SurfaceGeneration};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IdentityError(&'static str);

impl fmt::Display for IdentityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} must be nonzero", self.0)
    }
}

impl std::error::Error for IdentityError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PresenterInstanceId(pub [u8; 16]);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SessionIdentity {
    pub presenter: PresenterInstanceId,
    pub session_id: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContextIdentity {
    pub session: SessionIdentity,
    pub context_id: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SurfaceIdentity {
    pub context: ContextIdentity,
    pub surface_id: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TrackIdentity {
    pub surface: SurfaceIdentity,
    pub track_id: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ChannelIdentity {
    pub track: TrackIdentity,
    pub generation: ChannelGeneration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeIdentity {
    pub context: ContextIdentity,
    pub node_id: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TransactionIdentity {
    pub context: ContextIdentity,
    pub transaction_id: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AnchorIdentity {
    pub context: ContextIdentity,
    pub anchor_id: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LeaseIdentity {
    pub context: ContextIdentity,
    pub lease_id: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InputGrantIdentity {
    pub surface: SurfaceIdentity,
    pub producer_epoch: InputEpoch,
    pub surface_generation: SurfaceGeneration,
    pub grant_generation: GrantGeneration,
}

impl SessionIdentity {
    pub fn new(presenter: PresenterInstanceId, session_id: u64) -> Result<Self, IdentityError> {
        Ok(Self {
            presenter,
            session_id: nonzero("session ID", session_id)?,
        })
    }

    pub fn context(self, context_id: u64) -> Result<ContextIdentity, IdentityError> {
        Ok(ContextIdentity {
            session: self,
            context_id: nonzero("context ID", context_id)?,
        })
    }
}

impl ContextIdentity {
    pub fn surface(self, surface_id: u64) -> Result<SurfaceIdentity, IdentityError> {
        Ok(SurfaceIdentity {
            context: self,
            surface_id: nonzero("surface ID", surface_id)?,
        })
    }

    pub fn node(self, node_id: u64) -> Result<NodeIdentity, IdentityError> {
        Ok(NodeIdentity {
            context: self,
            node_id: nonzero("node ID", node_id)?,
        })
    }

    pub fn transaction(self, transaction_id: u64) -> Result<TransactionIdentity, IdentityError> {
        Ok(TransactionIdentity {
            context: self,
            transaction_id: nonzero("transaction ID", transaction_id)?,
        })
    }

    pub fn anchor(self, anchor_id: u64) -> Result<AnchorIdentity, IdentityError> {
        Ok(AnchorIdentity {
            context: self,
            anchor_id: nonzero("anchor ID", anchor_id)?,
        })
    }

    pub fn lease(self, lease_id: u64) -> Result<LeaseIdentity, IdentityError> {
        Ok(LeaseIdentity {
            context: self,
            lease_id: nonzero("lease ID", lease_id)?,
        })
    }
}

impl SurfaceIdentity {
    pub fn track(self, track_id: u64) -> Result<TrackIdentity, IdentityError> {
        Ok(TrackIdentity {
            surface: self,
            track_id: nonzero("track ID", track_id)?,
        })
    }
}

impl TrackIdentity {
    pub fn channel(self, generation: ChannelGeneration) -> Result<ChannelIdentity, IdentityError> {
        generation
            .require_nonzero()
            .map_err(|_| IdentityError("channel generation"))?;
        Ok(ChannelIdentity {
            track: self,
            generation,
        })
    }
}

fn nonzero(label: &'static str, value: u64) -> Result<u64, IdentityError> {
    if value == 0 {
        Err(IdentityError(label))
    } else {
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reused_local_numbers_do_not_alias_across_owners_or_presenters() {
        let presenter_a = PresenterInstanceId([1; 16]);
        let presenter_b = PresenterInstanceId([2; 16]);
        let first = SessionIdentity::new(presenter_a, 1)
            .unwrap()
            .context(2)
            .unwrap()
            .surface(3)
            .unwrap()
            .track(4)
            .unwrap();
        let sibling = SessionIdentity::new(presenter_a, 1)
            .unwrap()
            .context(5)
            .unwrap()
            .surface(3)
            .unwrap()
            .track(4)
            .unwrap();
        let other_presenter = SessionIdentity::new(presenter_b, 1)
            .unwrap()
            .context(2)
            .unwrap()
            .surface(3)
            .unwrap()
            .track(4)
            .unwrap();
        assert_ne!(first, sibling);
        assert_ne!(first, other_presenter);
    }
}
