//! Complete owner-scoped identities for Vivid 1.5 state and cleanup.

use std::fmt;

use crate::revision::ChannelGeneration;

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

/// A live session on one presenter instance.
///
/// Fields are crate-private so code outside this crate builds every identity in this module
/// through the checked, nonzero constructors; a struct literal cannot smuggle in a zero ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SessionIdentity {
    pub(crate) presenter: PresenterInstanceId,
    pub(crate) session_id: u64,
}

/// A context owned by one session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContextIdentity {
    pub(crate) session: SessionIdentity,
    pub(crate) context_id: u64,
}

/// A surface owned by one context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SurfaceIdentity {
    pub(crate) context: ContextIdentity,
    pub(crate) surface_id: u64,
}

/// A track owned by one surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TrackIdentity {
    pub(crate) surface: SurfaceIdentity,
    pub(crate) track_id: u64,
}

/// One nonzero channel generation of a track.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ChannelIdentity {
    pub(crate) track: TrackIdentity,
    pub(crate) generation: ChannelGeneration,
}

/// A scene node owned by one context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeIdentity {
    pub(crate) context: ContextIdentity,
    pub(crate) node_id: u64,
}

/// A scene transaction owned by one context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TransactionIdentity {
    pub(crate) context: ContextIdentity,
    pub(crate) transaction_id: u64,
}

/// A terminal anchor owned by one context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AnchorIdentity {
    pub(crate) context: ContextIdentity,
    pub(crate) anchor_id: u64,
}

/// A lease owned by one context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LeaseIdentity {
    pub(crate) context: ContextIdentity,
    pub(crate) lease_id: u64,
}

impl SessionIdentity {
    /// Identifies session `session_id` on presenter instance `presenter`.
    ///
    /// # Errors
    ///
    /// Returns [`IdentityError`] when `session_id` is zero.
    pub fn new(presenter: PresenterInstanceId, session_id: u64) -> Result<Self, IdentityError> {
        Ok(Self {
            presenter,
            session_id: nonzero("session ID", session_id)?,
        })
    }

    pub const fn presenter(&self) -> PresenterInstanceId {
        self.presenter
    }

    pub const fn session_id(&self) -> u64 {
        self.session_id
    }

    /// Identifies context `context_id` owned by this session.
    ///
    /// # Errors
    ///
    /// Returns [`IdentityError`] when `context_id` is zero.
    pub fn context(self, context_id: u64) -> Result<ContextIdentity, IdentityError> {
        Ok(ContextIdentity {
            session: self,
            context_id: nonzero("context ID", context_id)?,
        })
    }
}

impl ContextIdentity {
    pub const fn session(&self) -> SessionIdentity {
        self.session
    }

    pub const fn context_id(&self) -> u64 {
        self.context_id
    }

    /// Identifies surface `surface_id` owned by this context.
    ///
    /// # Errors
    ///
    /// Returns [`IdentityError`] when `surface_id` is zero.
    pub fn surface(self, surface_id: u64) -> Result<SurfaceIdentity, IdentityError> {
        Ok(SurfaceIdentity {
            context: self,
            surface_id: nonzero("surface ID", surface_id)?,
        })
    }

    /// Identifies scene node `node_id` owned by this context.
    ///
    /// # Errors
    ///
    /// Returns [`IdentityError`] when `node_id` is zero.
    pub fn node(self, node_id: u64) -> Result<NodeIdentity, IdentityError> {
        Ok(NodeIdentity {
            context: self,
            node_id: nonzero("node ID", node_id)?,
        })
    }

    /// Identifies transaction `transaction_id` owned by this context.
    ///
    /// # Errors
    ///
    /// Returns [`IdentityError`] when `transaction_id` is zero.
    pub fn transaction(self, transaction_id: u64) -> Result<TransactionIdentity, IdentityError> {
        Ok(TransactionIdentity {
            context: self,
            transaction_id: nonzero("transaction ID", transaction_id)?,
        })
    }

    /// Identifies anchor `anchor_id` owned by this context.
    ///
    /// # Errors
    ///
    /// Returns [`IdentityError`] when `anchor_id` is zero.
    pub fn anchor(self, anchor_id: u64) -> Result<AnchorIdentity, IdentityError> {
        Ok(AnchorIdentity {
            context: self,
            anchor_id: nonzero("anchor ID", anchor_id)?,
        })
    }

    /// Identifies lease `lease_id` owned by this context.
    ///
    /// # Errors
    ///
    /// Returns [`IdentityError`] when `lease_id` is zero.
    pub fn lease(self, lease_id: u64) -> Result<LeaseIdentity, IdentityError> {
        Ok(LeaseIdentity {
            context: self,
            lease_id: nonzero("lease ID", lease_id)?,
        })
    }
}

impl SurfaceIdentity {
    pub const fn context(&self) -> ContextIdentity {
        self.context
    }

    pub const fn surface_id(&self) -> u64 {
        self.surface_id
    }

    /// Identifies track `track_id` owned by this surface.
    ///
    /// # Errors
    ///
    /// Returns [`IdentityError`] when `track_id` is zero.
    pub fn track(self, track_id: u64) -> Result<TrackIdentity, IdentityError> {
        Ok(TrackIdentity {
            surface: self,
            track_id: nonzero("track ID", track_id)?,
        })
    }
}

impl TrackIdentity {
    pub const fn surface(&self) -> SurfaceIdentity {
        self.surface
    }

    pub const fn track_id(&self) -> u64 {
        self.track_id
    }

    /// Identifies channel `generation` of this track.
    ///
    /// # Errors
    ///
    /// Returns [`IdentityError`] when `generation` is zero.
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

impl ChannelIdentity {
    pub const fn track(&self) -> TrackIdentity {
        self.track
    }

    pub const fn generation(&self) -> ChannelGeneration {
        self.generation
    }
}

macro_rules! context_child_accessors {
    ($($name:ident => $id:ident),* $(,)?) => {
        $(
            impl $name {
                pub const fn context(&self) -> ContextIdentity {
                    self.context
                }

                pub const fn $id(&self) -> u64 {
                    self.$id
                }
            }
        )*
    };
}

context_child_accessors! {
    NodeIdentity => node_id,
    TransactionIdentity => transaction_id,
    AnchorIdentity => anchor_id,
    LeaseIdentity => lease_id,
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
