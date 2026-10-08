//! Owner-scoped retained scenes with atomic transactions.

use std::collections::BTreeMap;

use crate::{
    cbor::Value,
    identity::{NodeIdentity, SessionIdentity},
    messages::{
        MessageError, PayloadMap, StrictMap, invalid_value, require_nonzero, validate_header_object,
    },
    revision::{SceneRevision, TargetGeneration},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
pub enum Fit {
    Fill = 1,
    Contain = 2,
    Cover = 3,
    None = 4,
}

impl TryFrom<u64> for Fit {
    type Error = MessageError;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::Fill),
            2 => Ok(Self::Contain),
            3 => Ok(Self::Cover),
            4 => Ok(Self::None),
            _ => Err(invalid_value("scene node", 5, "has an unknown fit")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneNode {
    pub owning_context_id: u64,
    pub node_id: u64,
    pub surface_context_id: u64,
    pub surface_id: u64,
    pub geometry: PayloadMap,
    pub fit: Fit,
    pub linear_sampling: bool,
    pub z_index: i64,
    pub visible: bool,
    pub opacity: u16,
    pub clip: Option<PayloadMap>,
}

impl SceneNode {
    /// Checks that the node's owning context, node, and surface IDs are nonzero.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError::InvalidValue`] when any of the four IDs is zero.
    pub fn validate(&self) -> Result<(), MessageError> {
        require_nonzero("scene node", 0, self.owning_context_id)?;
        require_nonzero("scene node", 1, self.node_id)?;
        require_nonzero("scene node", 2, self.surface_context_id)?;
        require_nonzero("scene node", 3, self.surface_id)?;
        Ok(())
    }

    /// Encodes the node as a `CREATE_NODE` or `UPDATE_NODE` payload.
    ///
    /// # Errors
    ///
    /// Returns any error from [`SceneNode::validate`].
    pub fn payload(&self) -> Result<PayloadMap, MessageError> {
        self.validate()?;
        let mut fields = vec![
            (0, Value::Unsigned(self.owning_context_id)),
            (1, Value::Unsigned(self.node_id)),
            (2, Value::Unsigned(self.surface_context_id)),
            (3, Value::Unsigned(self.surface_id)),
            (4, Value::Map(self.geometry.clone())),
            (5, Value::Unsigned(self.fit as u64)),
            (6, Value::Unsigned(u64::from(self.linear_sampling))),
            (7, Value::from(self.z_index)),
            (8, Value::Unsigned(0)),
            (9, Value::Bool(self.visible)),
            (10, Value::Unsigned(u64::from(self.opacity))),
        ];
        if let Some(clip) = &self.clip {
            fields.push((11, Value::Map(clip.clone())));
        }
        Ok(fields)
    }

    /// Decodes a `CREATE_NODE` or `UPDATE_NODE` payload.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError`] when `payload` is not a strict node map, a field is missing or has
    /// the wrong type, sampling or blend semantics are unknown, the opacity exceeds 65,535, the fit
    /// is unregistered, the node fails [`SceneNode::validate`], or its node ID differs from
    /// `header_object_id`.
    pub fn decode(header_object_id: u64, payload: &Value) -> Result<Self, MessageError> {
        let map = StrictMap::new(
            "scene node",
            payload,
            &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11],
        )?;
        let sampling = map.required_u64(6)?;
        if sampling > 1 || map.required_u64(8)? != 0 {
            return Err(invalid_value(
                "scene node",
                6,
                "has unknown sampling or blend semantics",
            ));
        }
        let opacity = u16::try_from(map.required_u64(10)?)
            .map_err(|_| invalid_value("scene node", 10, "exceeds 65535"))?;
        let node = Self {
            owning_context_id: map.required_u64(0)?,
            node_id: map.required_u64(1)?,
            surface_context_id: map.required_u64(2)?,
            surface_id: map.required_u64(3)?,
            geometry: map.required_map(4)?.to_vec(),
            fit: Fit::try_from(map.required_u64(5)?)?,
            linear_sampling: sampling == 1,
            z_index: map
                .required(7)?
                .as_i64()
                .ok_or_else(|| invalid_value("scene node", 7, "is not an integer"))?,
            visible: map.required_bool(9)?,
            opacity,
            clip: map.optional_map(11)?.map(ToOwned::to_owned),
        };
        node.validate()?;
        validate_header_object(header_object_id, node.node_id)?;
        Ok(node)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Mutation {
    Create(SceneNode),
    Update(SceneNode),
    Delete { context_id: u64, node_id: u64 },
}

#[derive(Debug, Default)]
struct Transaction {
    mutations: Vec<Mutation>,
}

#[derive(Debug)]
pub struct Scene {
    session: SessionIdentity,
    revision: SceneRevision,
    target_generation: TargetGeneration,
    nodes: BTreeMap<(u64, u64), SceneNode>,
    transactions: BTreeMap<(u64, u64), Transaction>,
}

impl Scene {
    /// Creates an empty scene for `session` at `target_generation`.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError::InvalidValue`] when `target_generation` is zero.
    pub fn new(
        session: SessionIdentity,
        target_generation: TargetGeneration,
    ) -> Result<Self, MessageError> {
        target_generation
            .require_nonzero()
            .map_err(|_| invalid_value("scene", 1, "target generation must be nonzero"))?;
        Ok(Self {
            session,
            revision: SceneRevision::ZERO,
            target_generation,
            nodes: BTreeMap::new(),
            transactions: BTreeMap::new(),
        })
    }

    pub const fn revision(&self) -> SceneRevision {
        self.revision
    }

    pub const fn target_generation(&self) -> TargetGeneration {
        self.target_generation
    }

    /// Opens transaction `transaction_id` in `context_id`.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError::InvalidValue`] when either ID is zero or the transaction is already
    /// open in that context.
    pub fn begin(&mut self, context_id: u64, transaction_id: u64) -> Result<(), MessageError> {
        require_nonzero("BEGIN_TXN", 0, context_id)?;
        require_nonzero("BEGIN_TXN", 1, transaction_id)?;
        if self
            .transactions
            .contains_key(&(context_id, transaction_id))
        {
            return Err(invalid_value(
                "BEGIN_TXN",
                1,
                "duplicates a live transaction ID in this context",
            ));
        }
        self.transactions
            .insert((context_id, transaction_id), Transaction::default());
        Ok(())
    }

    /// Stages a `CREATE_NODE` in an open transaction.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError::InvalidValue`] when the node's owning context is not `context_id` or
    /// the transaction does not exist.
    pub fn create(
        &mut self,
        context_id: u64,
        transaction_id: u64,
        node: SceneNode,
    ) -> Result<(), MessageError> {
        if node.owning_context_id != context_id {
            return Err(invalid_value(
                "CREATE_NODE",
                0,
                "does not match transaction ownership",
            ));
        }
        self.transaction_mut(context_id, transaction_id)?
            .mutations
            .push(Mutation::Create(node));
        Ok(())
    }

    /// Stages an `UPDATE_NODE` in an open transaction.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError::InvalidValue`] when the node's owning context is not `context_id` or
    /// the transaction does not exist.
    pub fn update(
        &mut self,
        context_id: u64,
        transaction_id: u64,
        node: SceneNode,
    ) -> Result<(), MessageError> {
        if node.owning_context_id != context_id {
            return Err(invalid_value(
                "UPDATE_NODE",
                0,
                "does not match transaction ownership",
            ));
        }
        self.transaction_mut(context_id, transaction_id)?
            .mutations
            .push(Mutation::Update(node));
        Ok(())
    }

    /// Stages a `DELETE_NODE` in an open transaction.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError::InvalidValue`] when `node_id` is zero or the transaction does not
    /// exist.
    pub fn delete(
        &mut self,
        context_id: u64,
        transaction_id: u64,
        node_id: u64,
    ) -> Result<(), MessageError> {
        require_nonzero("DELETE_NODE", 1, node_id)?;
        self.transaction_mut(context_id, transaction_id)?
            .mutations
            .push(Mutation::Delete {
                context_id,
                node_id,
            });
        Ok(())
    }

    pub fn abort(&mut self, context_id: u64, transaction_id: u64) -> bool {
        self.transactions
            .remove(&(context_id, transaction_id))
            .is_some()
    }

    /// Applies an open transaction atomically, advancing the scene revision.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError::InvalidValue`] when the target generation or expected scene revision
    /// is stale, the transaction is not live, a create duplicates an existing node, an update or
    /// delete names a missing node, or the scene revision is exhausted. A failed commit leaves the
    /// scene unchanged.
    pub fn commit(
        &mut self,
        context_id: u64,
        transaction_id: u64,
        expected_target_generation: TargetGeneration,
        expected_scene_revision: Option<SceneRevision>,
    ) -> Result<SceneRevision, MessageError> {
        if expected_target_generation != self.target_generation {
            return Err(invalid_value(
                "COMMIT_TXN",
                2,
                "uses a stale target generation",
            ));
        }
        if expected_scene_revision.is_some_and(|value| value != self.revision) {
            return Err(invalid_value(
                "COMMIT_TXN",
                0,
                "uses a stale scene revision",
            ));
        }
        let transaction = self
            .transactions
            .get(&(context_id, transaction_id))
            .ok_or_else(|| invalid_value("COMMIT_TXN", 1, "does not name a live transaction"))?;
        let mut candidate = self.nodes.clone();
        for mutation in &transaction.mutations {
            match mutation {
                Mutation::Create(node) => {
                    let key = (node.owning_context_id, node.node_id);
                    if candidate.contains_key(&key) {
                        return Err(invalid_value(
                            "CREATE_NODE",
                            1,
                            "duplicates a complete node identity",
                        ));
                    }
                    candidate.insert(key, node.clone());
                }
                Mutation::Update(node) => {
                    let key = (node.owning_context_id, node.node_id);
                    if !candidate.contains_key(&key) {
                        return Err(invalid_value(
                            "UPDATE_NODE",
                            1,
                            "does not name an existing node",
                        ));
                    }
                    candidate.insert(key, node.clone());
                }
                Mutation::Delete {
                    context_id,
                    node_id,
                } => {
                    if candidate.remove(&(*context_id, *node_id)).is_none() {
                        return Err(invalid_value(
                            "DELETE_NODE",
                            1,
                            "does not name an existing node",
                        ));
                    }
                }
            }
        }
        let next = self
            .revision
            .advance()
            .map_err(|_| invalid_value("COMMIT_TXN", 0, "exhausted scene revision"))?;
        self.nodes = candidate;
        self.revision = next;
        self.transactions.remove(&(context_id, transaction_id));
        Ok(next)
    }

    /// Removes every node that displays the given surface, returning how many were removed.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError::InvalidValue`] when nodes were removed and the scene revision is
    /// exhausted.
    pub fn remove_surface_references(
        &mut self,
        surface_context_id: u64,
        surface_id: u64,
    ) -> Result<usize, MessageError> {
        let before = self.nodes.len();
        self.nodes.retain(|_, node| {
            node.surface_context_id != surface_context_id || node.surface_id != surface_id
        });
        let removed = before - self.nodes.len();
        if removed != 0 {
            self.revision = self
                .revision
                .advance()
                .map_err(|_| invalid_value("scene cleanup", 0, "exhausted scene revision"))?;
        }
        Ok(removed)
    }

    pub fn node(&self, identity: NodeIdentity) -> Option<&SceneNode> {
        if identity.context.session != self.session {
            return None;
        }
        self.nodes
            .get(&(identity.context.context_id, identity.node_id))
    }

    fn transaction_mut(
        &mut self,
        context_id: u64,
        transaction_id: u64,
    ) -> Result<&mut Transaction, MessageError> {
        self.transactions
            .get_mut(&(context_id, transaction_id))
            .ok_or_else(|| invalid_value("transaction", 1, "does not exist"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::PresenterInstanceId;

    fn node(owner: u64, surface_owner: u64) -> SceneNode {
        SceneNode {
            owning_context_id: owner,
            node_id: 7,
            surface_context_id: surface_owner,
            surface_id: 9,
            geometry: vec![],
            fit: Fit::Contain,
            linear_sampling: true,
            z_index: 0,
            visible: true,
            opacity: u16::MAX,
            clip: None,
        }
    }

    fn session() -> SessionIdentity {
        SessionIdentity::new(PresenterInstanceId([1; 16]), 1).unwrap()
    }

    fn node_identity(context_id: u64) -> NodeIdentity {
        session().context(context_id).unwrap().node(7).unwrap()
    }

    #[test]
    fn failed_transaction_is_atomic() {
        let mut scene = Scene::new(session(), TargetGeneration::ONE).unwrap();
        scene.begin(2, 1).unwrap();
        scene.create(2, 1, node(2, 2)).unwrap();
        scene.create(2, 1, node(2, 2)).unwrap();
        scene.commit(2, 1, TargetGeneration::ONE, None).unwrap_err();
        assert_eq!(scene.revision(), SceneRevision::ZERO);
        assert!(scene.node(node_identity(2)).is_none());
    }

    #[test]
    fn cleanup_is_scoped_when_numeric_ids_are_reused() {
        let mut scene = Scene::new(session(), TargetGeneration::ONE).unwrap();
        for context in [2, 3] {
            scene.begin(context, 1).unwrap();
            scene.create(context, 1, node(context, context)).unwrap();
            scene
                .commit(context, 1, TargetGeneration::ONE, None)
                .unwrap();
        }
        assert_eq!(scene.remove_surface_references(2, 9).unwrap(), 1);
        assert!(scene.node(node_identity(2)).is_none());
        assert!(scene.node(node_identity(3)).is_some());
    }
}
