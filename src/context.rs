//! Context authority and finite delegated resource schemas.

use crate::{
    cbor::Value,
    identity::{ContextIdentity, SessionIdentity},
    messages::{
        MessageError, PayloadMap, StrictMap, invalid_value, require_nonzero, validate_header_object,
    },
    resource::{ReservationLedger, ResourceContract, ResourceError},
    revision::ContextRevision,
};

pub const OP_OBSERVE: u64 = 1 << 0;
pub const OP_SURFACE_TRACK_MEDIA: u64 = 1 << 1;
pub const OP_SCENE: u64 = 1 << 2;
pub const OP_TERMINAL_ANCHOR: u64 = 1 << 3;
pub const OP_DESKTOP_INPUT: u64 = 1 << 4;
pub const OP_DELEGATE: u64 = 1 << 5;
pub const OP_RECEIVE_FILE_DROP: u64 = 1 << 6;
pub const OP_KNOWN_MASK: u64 = (1 << 7) - 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextDefinition {
    pub context_id: u64,
    pub parent_context_id: u64,
    pub operation_classes: u64,
    pub label: String,
    pub lifetime_us: u64,
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
            requested_contract: ResourceContract::from_value(map.required(5)?)
                .map_err(|error| MessageError::Cbor(error.to_string()))?,
        };
        definition.validate(header_object_id)?;
        Ok(definition)
    }
}

#[derive(Debug)]
pub struct ContextState {
    pub identity: ContextIdentity,
    pub parent_context_id: Option<u64>,
    pub operation_classes: u64,
    pub revision: ContextRevision,
    pub contract: ResourceContract,
    reservations: ReservationLedger,
    pub lifecycle: ContextLifecycle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
pub enum ContextLifecycle {
    Active = 1,
    Expired = 2,
    Revoked = 3,
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
    /// [`MessageError::Cbor`] carrying the [`crate::resource::ResourceError`] text when resource
    /// accounting overflows.
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
            .reserve(&definition.requested_contract, policy)
            .map_err(resource_error)?;
        self.advance_revision()?;
        Ok((classes, contract))
    }

    pub const fn context_id(&self) -> u64 {
        self.identity.context_id
    }

    /// Returns a released child's resources to this context's budget.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError::Cbor`] carrying the [`crate::resource::ResourceError`] text when
    /// `contract` exceeds what is reserved here, and [`MessageError::InvalidValue`] when the
    /// context revision is exhausted.
    pub fn release_child(&mut self, contract: &ResourceContract) -> Result<(), MessageError> {
        self.reservations
            .release(contract)
            .map_err(resource_error)?;
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

fn resource_error(error: ResourceError) -> MessageError {
    MessageError::Cbor(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::PresenterInstanceId;
    use crate::resource::Resource;

    #[test]
    fn sibling_contracts_reserve_parent_capacity() {
        let mut contract = ResourceContract::denied();
        contract.set(Resource::Surfaces, 4);
        let session = SessionIdentity::new(PresenterInstanceId([1; 16]), 1).unwrap();
        let mut root = ContextState::root(session, 1, OP_DELEGATE, contract.clone()).unwrap();
        let mut requested = ResourceContract::denied();
        requested.set(Resource::Surfaces, 3);
        let definition = ContextDefinition {
            context_id: 2,
            parent_context_id: 1,
            operation_classes: 0,
            label: String::new(),
            lifetime_us: 0,
            requested_contract: requested,
        };
        let (_, first) = root.reserve_child(&definition, &contract).unwrap();
        let (_, second) = root.reserve_child(&definition, &contract).unwrap();
        assert_eq!(first.get(Resource::Surfaces), 3);
        assert_eq!(second.get(Resource::Surfaces), 1);
    }
}
