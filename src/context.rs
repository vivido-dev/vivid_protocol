//! Contexts: delegated authority with a finite resource contract (security §4).
//!
//! Every session has a root context. A context with [`OP_DELEGATE`] creates children with
//! `CREATE_CONTEXT` ([`ContextDefinition`]); each child holds a subset of its parent's operation
//! classes and a contract reserved from the parent's. [`ContextState`] is a presenter's record of
//! one context, and [`RevokeContext`] ends a child and everything it owns.

use crate::{
    cbor::Value,
    identity::{ContextIdentity, SessionIdentity},
    messages::{
        MessageError, PayloadMap, StrictMap, invalid_value, require_nonzero, validate_header_object,
    },
    resource::{ReservationLedger, ResourceContract},
    revision::ContextRevision,
};

/// Operation class 0: observe owned objects and sanitized status.
pub const OP_OBSERVE: u64 = 1 << 0;
/// Operation class 1: create surfaces and tracks and submit media.
pub const OP_SURFACE_TRACK_MEDIA: u64 = 1 << 1;
/// Operation class 2: create and mutate owned scene nodes.
pub const OP_SCENE: u64 = 1 << 2;
/// Operation class 3: create terminal anchors.
pub const OP_TERMINAL_ANCHOR: u64 = 1 << 3;
/// Operation class 4: establish and receive desktop input for owned surfaces.
pub const OP_DESKTOP_INPUT: u64 = 1 << 4;
/// Operation class 5: create child contexts and session leases.
pub const OP_DELEGATE: u64 = 1 << 5;
/// Operation class 6: receive user-initiated file drops on owned contexts or surfaces.
pub const OP_RECEIVE_FILE_DROP: u64 = 1 << 6;
/// Every assigned operation-class bit.
pub const OP_KNOWN_MASK: u64 = (1 << 7) - 1;

/// The `CREATE_CONTEXT` request (security §4.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextDefinition {
    /// Nonzero child context ID (key 0).
    pub context_id: u64,
    /// Parent context (key 1).
    pub parent_context_id: u64,
    /// Requested operation-class bits (key 2).
    pub operation_classes: u64,
    /// Diagnostic label, at most 64 UTF-8 bytes (key 3).
    pub label: String,
    /// Lifetime from acceptance in microseconds; zero means the parent's (key 4).
    pub lifetime_us: u64,
    /// Requested resource contract (key 5).
    pub requested_contract: ResourceContract,
}

impl ContextDefinition {
    /// Checks the context's IDs, operation classes, and label against `header_object_id`.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError`] when the context or parent ID is zero, the context ID differs from
    /// `header_object_id`, the operation classes have unknown bits, or the label exceeds 64 UTF-8
    /// bytes.
    pub fn validate(&self, header_object_id: u64) -> Result<(), MessageError> {
        require_nonzero("CREATE_CONTEXT", 0, self.context_id)?;
        require_nonzero("CREATE_CONTEXT", 1, self.parent_context_id)?;
        validate_header_object(header_object_id, self.context_id)?;
        if self.operation_classes & !OP_KNOWN_MASK != 0 {
            return Err(invalid_value(
                "CREATE_CONTEXT",
                2,
                "has unknown operation-class bits",
            ));
        }
        if self.label.len() > 64 {
            return Err(invalid_value(
                "CREATE_CONTEXT",
                3,
                "label exceeds 64 UTF-8 bytes",
            ));
        }
        Ok(())
    }

    /// Encodes the `CREATE_CONTEXT` payload.
    ///
    /// # Errors
    ///
    /// Returns any error from [`ContextDefinition::validate`].
    pub fn payload(&self) -> Result<PayloadMap, MessageError> {
        self.validate(self.context_id)?;
        Ok(vec![
            (0, Value::Unsigned(self.context_id)),
            (1, Value::Unsigned(self.parent_context_id)),
            (2, Value::Unsigned(self.operation_classes)),
            (3, Value::Text(self.label.clone())),
            (4, Value::Unsigned(self.lifetime_us)),
            (5, self.requested_contract.to_value()),
        ])
    }

    /// Decodes a `CREATE_CONTEXT` payload.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError`] when `payload` is not a strict `CREATE_CONTEXT` map, a field is
    /// missing or has the wrong type, the resource contract is invalid, or the definition fails
    /// [`ContextDefinition::validate`].
    pub fn decode(header_object_id: u64, payload: &Value) -> Result<Self, MessageError> {
        let map = StrictMap::new("CREATE_CONTEXT", payload, &[0, 1, 2, 3, 4, 5])?;
        let definition = Self {
            context_id: map.required_u64(0)?,
            parent_context_id: map.required_u64(1)?,
            operation_classes: map.required_u64(2)?,
            label: map.required_text(3)?.to_owned(),
            lifetime_us: map.required_u64(4)?,
            requested_contract: ResourceContract::from_value(map.required(5)?)?,
        };
        definition.validate(header_object_id)?;
        Ok(definition)
    }
}

/// A presenter's state for one context.
#[derive(Debug)]
pub struct ContextState {
    /// The context's complete identity.
    pub identity: ContextIdentity,
    /// The parent context, or `None` for the root.
    pub parent_context_id: Option<u64>,
    /// Granted operation-class bits, such as [`OP_SCENE`].
    pub operation_classes: u64,
    /// Context revision; advances on each change.
    pub revision: ContextRevision,
    /// Effective resource contract.
    pub contract: ResourceContract,
    reservations: ReservationLedger,
    /// Whether the context is live.
    pub lifecycle: ContextLifecycle,
}

/// Whether a context is live, and if not, why it ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
pub enum ContextLifecycle {
    /// Live.
    Active = 1,
    /// Its lifetime ran out.
    Expired = 2,
    /// Its owner or a parent revoked it.
    Revoked = 3,
    /// Its session closed.
    Closed = 4,
}

impl ContextState {
    /// Creates the session's root context with `contract` as its resource budget.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError::InvalidValue`] when `context_id` is zero or `operation_classes` has
    /// unknown bits.
    pub fn root(
        session: SessionIdentity,
        context_id: u64,
        operation_classes: u64,
        contract: ResourceContract,
    ) -> Result<Self, MessageError> {
        let identity = session
            .context(context_id)
            .map_err(|_| invalid_value("root context", 1, "has a zero context ID"))?;
        if operation_classes & !OP_KNOWN_MASK != 0 {
            return Err(invalid_value(
                "root context",
                2,
                "has unknown operation-class bits",
            ));
        }
        Ok(Self {
            identity,
            parent_context_id: None,
            operation_classes,
            revision: ContextRevision::ONE,
            reservations: ReservationLedger::new(contract.clone()),
            contract,
            lifecycle: ContextLifecycle::Active,
        })
    }

    /// Reserves resources for a child, returning its effective operation classes and contract.
    ///
    /// The granted contract is the request clamped to `policy` and to this context's remaining
    /// budget, so an oversized request is narrowed rather than rejected.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError::InvalidValue`] when this context is not active, is not the child's
    /// parent, or lacks delegation authority, or the context revision is exhausted. Returns
    /// [`MessageError::Resource`] when resource accounting overflows.
    pub fn reserve_child(
        &mut self,
        definition: &ContextDefinition,
        policy: &ResourceContract,
    ) -> Result<(u64, ResourceContract), MessageError> {
        if self.lifecycle != ContextLifecycle::Active
            || definition.parent_context_id != self.context_id()
            || self.operation_classes & OP_DELEGATE == 0
        {
            return Err(invalid_value(
                "CREATE_CONTEXT",
                1,
                "is outside active delegation authority",
            ));
        }
        let classes = definition.operation_classes & self.operation_classes;
        let contract = self
            .reservations
            .reserve(&definition.requested_contract, policy)?;
        self.advance_revision()?;
        Ok((classes, contract))
    }

    /// Returns the local context ID.
    pub const fn context_id(&self) -> u64 {
        self.identity.context_id
    }

    /// Returns a released child's resources to this context's budget.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError::Resource`] when `contract` exceeds what is reserved here, and
    /// [`MessageError::InvalidValue`] when the context revision is exhausted.
    pub fn release_child(&mut self, contract: &ResourceContract) -> Result<(), MessageError> {
        self.reservations.release(contract)?;
        self.advance_revision()
    }

    /// Revokes an active context.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError::InvalidValue`] when the context is not active, or the context
    /// revision is exhausted.
    pub fn revoke(&mut self) -> Result<(), MessageError> {
        if self.lifecycle != ContextLifecycle::Active {
            return Err(invalid_value("REVOKE_CONTEXT", 0, "context is not active"));
        }
        self.lifecycle = ContextLifecycle::Revoked;
        self.advance_revision()
    }

    fn advance_revision(&mut self) -> Result<(), MessageError> {
        self.revision = self
            .revision
            .advance()
            .map_err(|_| invalid_value("context", 0, "exhausted its revision"))?;
        Ok(())
    }
}

/// The `REVOKE_CONTEXT` payload: tear down a delegated context and its descendants (security §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RevokeContext {
    /// The context to revoke.
    pub context_id: u64,
    /// The parent context that authorizes the revocation, when the request names it.
    pub parent_context_id: Option<u64>,
}

impl RevokeContext {
    /// Encodes the payload map.
    pub fn payload(&self) -> PayloadMap {
        let mut payload = vec![(0, Value::Unsigned(self.context_id))];
        if let Some(parent) = self.parent_context_id {
            payload.push((1, Value::Unsigned(parent)));
        }
        payload
    }

    /// Decodes a `REVOKE_CONTEXT` payload.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError`] when `value` is not a strict `REVOKE_CONTEXT` map, the context is
    /// missing, or a field is not an unsigned integer.
    pub fn decode(value: &Value) -> Result<Self, MessageError> {
        let map = StrictMap::new("REVOKE_CONTEXT", value, &[0, 1])?;
        Ok(Self {
            context_id: map.required_u64(0)?,
            parent_context_id: map.optional_u64(1)?,
        })
    }
}
