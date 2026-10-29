//! Retry-safe session leases, suspension, and bounded resumption (security §6 and §7).
//!
//! A context hands another producer a session lease with `CREATE_SESSION_LEASE`
//! ([`SessionLeaseDefinition`]). The presenter's [`LeaseMachine`] runs the lease through its
//! states: it reserves the lease for one activation attempt, replays the same `WELCOME` for an
//! exact retry, suspends the session on unclean transport loss, and admits a resume at the current
//! generation. [`RevokeSessionLease`] ends a lease.

use std::fmt;

use sha2::{Digest, Sha256};

use crate::{
    auth::{ATTEMPT_ID_BYTES, NONCE_BYTES},
    cbor::Value,
    messages::{MessageError, PayloadMap, StrictMap, invalid_value, require_nonzero},
    registry,
    resource::ResourceContract,
    revision::ResumeGeneration,
};

/// Longest activation timeout a lease may request: 60 seconds.
pub const MAX_ACTIVATION_TIMEOUT_US: u64 = 60_000_000;

/// What happens to a lease's session when its transport closes (`CREATE_SESSION_LEASE` key 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
#[non_exhaustive]
pub enum CleanupPolicy {
    /// Clean up on every disconnect (`0`).
    Immediate = 0,
    /// Suspend on unclean transport loss so the session can resume (`1`).
    SuspendOnUncleanLoss = 1,
}

impl TryFrom<u64> for CleanupPolicy {
    type Error = MessageError;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Immediate),
            1 => Ok(Self::SuspendOnUncleanLoss),
            _ => Err(invalid_value(
                "CREATE_SESSION_LEASE",
                5,
                "has an unknown cleanup policy",
            )),
        }
    }
}

/// The `CREATE_SESSION_LEASE` request (security §6.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionLeaseDefinition {
    /// Owning context (key 0).
    pub context_id: u64,
    /// Nonzero lease ID, unique within the context (key 1).
    pub lease_id: u64,
    /// SHA-256 verifier of the activation secret; never the secret (key 2).
    pub activation_verifier: [u8; 32],
    /// Nonzero activation timeout, at most 60 seconds (key 3).
    pub activation_timeout_us: u64,
    /// Requested disconnect grace in microseconds (key 4).
    pub requested_disconnect_grace_us: u64,
    /// Cleanup policy (key 5).
    pub cleanup_policy: CleanupPolicy,
    /// Profiles the session may use, sorted, unique, and closed under prerequisites (key 6).
    pub permitted_profiles: Vec<String>,
    /// Resource contract reserved for the session (key 7).
    pub requested_contract: ResourceContract,
    /// Client public key for deployment-specific proof of possession (key 8).
    pub client_public_key: Option<Vec<u8>>,
}

impl SessionLeaseDefinition {
    /// Checks the lease's IDs, activation timeout, and permitted profiles.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError::InvalidValue`] when the context or lease ID is zero, the activation
    /// timeout is outside 1 through 60 seconds, or the permitted profiles are not sorted, unique,
    /// prerequisite-closed, and inclusive of the core profile.
    pub fn validate(&self) -> Result<(), MessageError> {
        require_nonzero("CREATE_SESSION_LEASE", 0, self.context_id)?;
        require_nonzero("CREATE_SESSION_LEASE", 1, self.lease_id)?;
        if self.activation_timeout_us == 0 || self.activation_timeout_us > MAX_ACTIVATION_TIMEOUT_US
        {
            return Err(invalid_value(
                "CREATE_SESSION_LEASE",
                3,
                "is outside 1..=60 seconds",
            ));
        }
        registry::validate_profile_set(self.permitted_profiles.iter().map(String::as_str))
            .map_err(|_| {
                invalid_value(
                    "CREATE_SESSION_LEASE",
                    6,
                    "is not sorted, unique, and prerequisite-closed",
                )
            })?;
        if !self
            .permitted_profiles
            .iter()
            .any(|profile| profile == registry::CORE_CONTROL)
        {
            return Err(invalid_value(
                "CREATE_SESSION_LEASE",
                6,
                "does not include the core profile",
            ));
        }
        Ok(())
    }

    /// Encodes the `CREATE_SESSION_LEASE` payload.
    ///
    /// # Errors
    ///
    /// Returns any error from [`SessionLeaseDefinition::validate`].
    pub fn payload(&self) -> Result<PayloadMap, MessageError> {
        self.validate()?;
        let mut fields = vec![
            (0, Value::Unsigned(self.context_id)),
            (1, Value::Unsigned(self.lease_id)),
            (2, Value::Bytes(self.activation_verifier.to_vec())),
            (3, Value::Unsigned(self.activation_timeout_us)),
            (4, Value::Unsigned(self.requested_disconnect_grace_us)),
            (5, Value::Unsigned(self.cleanup_policy as u64)),
            (
                6,
                Value::Array(
                    self.permitted_profiles
                        .iter()
                        .cloned()
                        .map(Value::Text)
                        .collect(),
                ),
            ),
            (7, self.requested_contract.to_value()),
        ];
        if let Some(public_key) = &self.client_public_key {
            fields.push((8, Value::Bytes(public_key.clone())));
        }
        Ok(fields)
    }

    /// Decodes a `CREATE_SESSION_LEASE` payload.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError`] when `payload` is not a strict `CREATE_SESSION_LEASE` map, a field
    /// is missing or has the wrong type, the profile list holds a non-text entry, the cleanup
    /// policy or resource contract is invalid, or the definition fails
    /// [`SessionLeaseDefinition::validate`].
    pub fn decode(payload: &Value) -> Result<Self, MessageError> {
        let map = StrictMap::new(
            "CREATE_SESSION_LEASE",
            payload,
            &[0, 1, 2, 3, 4, 5, 6, 7, 8],
        )?;
        let profiles = map
            .required(6)?
            .as_array()
            .ok_or_else(|| invalid_value("CREATE_SESSION_LEASE", 6, "is not an array"))?
            .iter()
            .map(|value| {
                value.as_text().map(ToOwned::to_owned).ok_or_else(|| {
                    invalid_value("CREATE_SESSION_LEASE", 6, "contains a non-text profile")
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let definition = Self {
            context_id: map.required_u64(0)?,
            lease_id: map.required_u64(1)?,
            activation_verifier: map.required_fixed_bytes(2)?,
            activation_timeout_us: map.required_u64(3)?,
            requested_disconnect_grace_us: map.required_u64(4)?,
            cleanup_policy: CleanupPolicy::try_from(map.required_u64(5)?)?,
            permitted_profiles: profiles,
            requested_contract: ResourceContract::from_value(map.required(7)?)?,
            client_public_key: map.optional_bytes(8)?.map(ToOwned::to_owned),
        };
        definition.validate()?;
        Ok(definition)
    }
}

/// A session lease's state (security §6.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
pub enum LeaseState {
    /// Created; no activation yet.
    Issued = 1,
    /// One activation attempt holds the lease until its `WELCOME` is committed.
    Reserved = 2,
    /// A session is using the lease.
    Active = 3,
    /// The session lost its transport and may resume within the grace.
    Suspended = 4,
    /// The session closed cleanly.
    Closed = 5,
    /// The owner or a parent revoked the lease.
    Revoked = 6,
    /// The activation deadline or disconnect grace passed.
    Expired = 7,
}

#[derive(Clone, PartialEq, Eq)]
struct Attempt {
    id: [u8; 16],
    client_nonce: [u8; 32],
    hello_hash: [u8; 32],
    session_id: u64,
    server_nonce: [u8; 32],
    welcome: Vec<u8>,
    transport_live: bool,
    post_hello_admitted: bool,
    resume_from: Option<ResumeGeneration>,
}

impl Attempt {
    fn exact(&self, attempt_id: &[u8; 16], client_nonce: &[u8; 32], hello_bytes: &[u8]) -> bool {
        self.id == *attempt_id
            && self.client_nonce == *client_nonce
            && self.hello_hash == Sha256::digest(hello_bytes).as_slice()
    }
}

/// One `HELLO`'s bid to activate or resume a lease, with the reply prepared for it.
///
/// An exact retry repeats the attempt ID, client nonce, and `HELLO` bytes of the attempt the lease
/// holds; it is answered with that attempt's stored reply instead of `welcome`.
#[derive(Clone, PartialEq, Eq)]
pub struct EstablishmentAttempt<'a> {
    /// `HELLO`'s attempt ID, which identifies retries of one attempt.
    pub attempt_id: [u8; ATTEMPT_ID_BYTES],
    /// `HELLO`'s client nonce.
    pub client_nonce: [u8; NONCE_BYTES],
    /// The complete `HELLO` body; only its digest is kept.
    pub hello_bytes: &'a [u8],
    /// A digest of the negotiated profile set, which every retry and resume must match.
    pub profile_fingerprint: [u8; 32],
    /// The logical session the lease belongs to; nonzero.
    pub session_id: u64,
    /// The server nonce of the `WELCOME` prepared for this attempt.
    pub server_nonce: [u8; NONCE_BYTES],
    /// The prepared `WELCOME` body, replayed to an exact retry.
    pub welcome: Vec<u8>,
}

impl fmt::Debug for EstablishmentAttempt<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EstablishmentAttempt")
            .field("session_id", &self.session_id)
            .field("hello_len", &self.hello_bytes.len())
            .field("welcome_len", &self.welcome.len())
            .finish_non_exhaustive()
    }
}

/// How a lease answers an activation or resume attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttemptDecision {
    /// A new attempt: send this `WELCOME`.
    Fresh {
        /// The logical session ID.
        session_id: u64,
        /// The server nonce.
        server_nonce: [u8; 32],
        /// The complete `WELCOME` body.
        welcome: Vec<u8>,
    },
    /// An exact retry: send the original `WELCOME` again.
    ExactReplay {
        /// The logical session ID.
        session_id: u64,
        /// The server nonce.
        server_nonce: [u8; 32],
        /// The complete `WELCOME` body.
        welcome: Vec<u8>,
    },
}

/// A lease transition the state machine refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum LeaseTransitionError {
    /// Another attempt already holds the lease, or the retry differs.
    AuthenticationFailed,
    /// The lease's state does not allow the transition.
    BadState,
    /// The resume names a generation other than the current one.
    StaleResumeGeneration,
    /// The lease revision or resume generation cannot advance.
    Exhausted,
}

impl std::fmt::Display for LeaseTransitionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::AuthenticationFailed => "lease authentication failed",
            Self::BadState => "lease transition is not allowed in the current state",
            Self::StaleResumeGeneration => "resume generation is stale",
            Self::Exhausted => "lease revision or resume generation is exhausted",
        })
    }
}

impl std::error::Error for LeaseTransitionError {}

/// A presenter's state machine for one session lease.
#[derive(Clone)]
pub struct LeaseMachine {
    state: LeaseState,
    revision: u64,
    resume_generation: ResumeGeneration,
    cleanup_policy: CleanupPolicy,
    disconnect_grace_us: u64,
    logical_session_id: Option<u64>,
    profile_fingerprint: Option<[u8; 32]>,
    attempt: Option<Attempt>,
}

impl std::fmt::Debug for LeaseMachine {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LeaseMachine")
            .field("state", &self.state)
            .field("revision", &self.revision)
            .field("resume_generation", &self.resume_generation)
            .field("cleanup_policy", &self.cleanup_policy)
            .field("disconnect_grace_us", &self.disconnect_grace_us)
            .field("logical_session_id", &self.logical_session_id)
            .field(
                "has_profile_fingerprint",
                &self.profile_fingerprint.is_some(),
            )
            .field("has_attempt", &self.attempt.is_some())
            .finish()
    }
}

impl LeaseMachine {
    /// Creates a machine in [`LeaseState::Issued`].
    pub fn new(cleanup_policy: CleanupPolicy, disconnect_grace_us: u64) -> Self {
        Self {
            state: LeaseState::Issued,
            revision: 1,
            resume_generation: ResumeGeneration::ZERO,
            cleanup_policy,
            disconnect_grace_us,
            logical_session_id: None,
            profile_fingerprint: None,
            attempt: None,
        }
    }

    /// Returns the current state.
    pub const fn state(&self) -> LeaseState {
        self.state
    }

    /// Returns the lease revision; it advances on each transition.
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    /// Returns the current resume generation.
    pub const fn resume_generation(&self) -> ResumeGeneration {
        self.resume_generation
    }

    /// Reserves an issued lease for a fresh activation attempt, or recognizes an exact retry.
    ///
    /// # Errors
    ///
    /// Returns [`LeaseTransitionError::BadState`] when the attempt's session ID is zero,
    /// [`LeaseTransitionError::AuthenticationFailed`] when a resume attempt is pending, the lease
    /// is not issued and this is not an exact retry of the reserved or active attempt, or the
    /// profile fingerprint differs, and [`LeaseTransitionError::Exhausted`] when the revision
    /// cannot advance.
    pub fn begin_activation(
        &mut self,
        attempt: EstablishmentAttempt<'_>,
    ) -> Result<AttemptDecision, LeaseTransitionError> {
        let EstablishmentAttempt {
            attempt_id,
            client_nonce,
            hello_bytes,
            profile_fingerprint,
            session_id,
            server_nonce,
            welcome,
        } = attempt;
        if session_id == 0 {
            return Err(LeaseTransitionError::BadState);
        }
        if self
            .attempt
            .as_ref()
            .is_some_and(|attempt| attempt.resume_from.is_some())
        {
            return Err(LeaseTransitionError::AuthenticationFailed);
        }
        match self.state {
            LeaseState::Issued => {
                self.state = LeaseState::Reserved;
                self.advance_revision()?;
                self.logical_session_id = Some(session_id);
                self.profile_fingerprint = Some(profile_fingerprint);
                self.attempt = Some(Attempt {
                    id: attempt_id,
                    client_nonce,
                    hello_hash: Sha256::digest(hello_bytes).into(),
                    session_id,
                    server_nonce,
                    welcome: welcome.clone(),
                    transport_live: true,
                    post_hello_admitted: false,
                    resume_from: None,
                });
                Ok(AttemptDecision::Fresh {
                    session_id,
                    server_nonce,
                    welcome,
                })
            }
            LeaseState::Reserved => {
                if self.profile_fingerprint != Some(profile_fingerprint) {
                    return Err(LeaseTransitionError::AuthenticationFailed);
                }
                let decision = self.exact_retry(attempt_id, client_nonce, hello_bytes)?;
                self.attempt
                    .as_mut()
                    .ok_or(LeaseTransitionError::BadState)?
                    .transport_live = true;
                Ok(decision)
            }
            LeaseState::Active => {
                if self.profile_fingerprint != Some(profile_fingerprint) {
                    return Err(LeaseTransitionError::AuthenticationFailed);
                }
                let retryable = self
                    .attempt
                    .as_ref()
                    .is_some_and(|attempt| !attempt.transport_live && !attempt.post_hello_admitted);
                if retryable {
                    let decision = self.exact_retry(attempt_id, client_nonce, hello_bytes)?;
                    if let Some(attempt) = &mut self.attempt {
                        attempt.transport_live = true;
                    }
                    Ok(decision)
                } else {
                    Err(LeaseTransitionError::AuthenticationFailed)
                }
            }
            _ => Err(LeaseTransitionError::AuthenticationFailed),
        }
    }

    /// Marks the reserved lease active once its `WELCOME` is committed.
    ///
    /// # Errors
    ///
    /// Returns [`LeaseTransitionError::BadState`] when the lease is not reserved, and
    /// [`LeaseTransitionError::Exhausted`] when the revision cannot advance.
    pub fn commit_welcome(&mut self) -> Result<(), LeaseTransitionError> {
        if self.state != LeaseState::Reserved {
            return Err(LeaseTransitionError::BadState);
        }
        self.state = LeaseState::Active;
        self.advance_revision()
    }

    /// Records that the active attempt has admitted its first post-`HELLO` record.
    ///
    /// # Errors
    ///
    /// Returns [`LeaseTransitionError::BadState`] when the lease is not active or has no attempt.
    pub fn admit_post_hello(&mut self) -> Result<(), LeaseTransitionError> {
        if self.state != LeaseState::Active {
            return Err(LeaseTransitionError::BadState);
        }
        self.attempt
            .as_mut()
            .ok_or(LeaseTransitionError::BadState)?
            .post_hello_admitted = true;
        Ok(())
    }

    /// Handles loss of the attempt's transport, closing or suspending the lease.
    ///
    /// # Errors
    ///
    /// Returns [`LeaseTransitionError::BadState`] when the lease is neither reserved nor active,
    /// and [`LeaseTransitionError::Exhausted`] when the revision cannot advance.
    pub fn confirm_transport_lost(
        &mut self,
        clean: bool,
    ) -> Result<LeaseState, LeaseTransitionError> {
        if !matches!(self.state, LeaseState::Reserved | LeaseState::Active) {
            return Err(LeaseTransitionError::BadState);
        }
        if let Some(attempt) = &mut self.attempt {
            attempt.transport_live = false;
        }
        if !clean {
            match self.state {
                LeaseState::Reserved => return Ok(self.state),
                LeaseState::Active
                    if self
                        .attempt
                        .as_ref()
                        .is_some_and(|attempt| !attempt.post_hello_admitted) =>
                {
                    return Ok(self.state);
                }
                _ => {}
            }
        }
        if clean || self.cleanup_policy == CleanupPolicy::Immediate || self.disconnect_grace_us == 0
        {
            self.state = LeaseState::Closed;
            self.logical_session_id = None;
            self.profile_fingerprint = None;
            self.attempt = None;
        } else {
            self.state = LeaseState::Suspended;
            self.attempt = None;
        }
        self.advance_revision()?;
        Ok(self.state)
    }

    /// Reserves a suspended lease for a resume attempt, or recognizes an exact retry.
    ///
    /// # Errors
    ///
    /// Returns [`LeaseTransitionError::BadState`] when the lease is not suspended and this is not a
    /// retry, [`LeaseTransitionError::StaleResumeGeneration`] when `expected_generation` is not
    /// current, [`LeaseTransitionError::AuthenticationFailed`] for a non-retryable attempt or a
    /// fingerprint mismatch, and [`LeaseTransitionError::Exhausted`] when the revision or resume
    /// generation cannot advance.
    pub fn begin_resume(
        &mut self,
        expected_generation: ResumeGeneration,
        attempt: EstablishmentAttempt<'_>,
    ) -> Result<AttemptDecision, LeaseTransitionError> {
        let EstablishmentAttempt {
            attempt_id,
            client_nonce,
            hello_bytes,
            profile_fingerprint,
            session_id,
            server_nonce,
            welcome,
        } = attempt;
        if matches!(self.state, LeaseState::Reserved | LeaseState::Active) {
            if self.profile_fingerprint != Some(profile_fingerprint)
                || self.logical_session_id != Some(session_id)
            {
                return Err(LeaseTransitionError::AuthenticationFailed);
            }
            let retryable = self.attempt.as_ref().is_some_and(|attempt| {
                attempt.resume_from == Some(expected_generation)
                    && !attempt.post_hello_admitted
                    && (self.state == LeaseState::Reserved || !attempt.transport_live)
            });
            if !retryable {
                return Err(LeaseTransitionError::AuthenticationFailed);
            }
            let decision = self.exact_retry(attempt_id, client_nonce, hello_bytes)?;
            self.attempt
                .as_mut()
                .ok_or(LeaseTransitionError::BadState)?
                .transport_live = true;
            return Ok(decision);
        }
        if self.state != LeaseState::Suspended {
            return Err(LeaseTransitionError::BadState);
        }
        if expected_generation != self.resume_generation {
            return Err(LeaseTransitionError::StaleResumeGeneration);
        }
        if self.logical_session_id != Some(session_id)
            || self.profile_fingerprint != Some(profile_fingerprint)
        {
            return Err(LeaseTransitionError::AuthenticationFailed);
        }
        let next_revision = self
            .revision
            .checked_add(1)
            .ok_or(LeaseTransitionError::Exhausted)?;
        self.resume_generation = self
            .resume_generation
            .advance()
            .map_err(|_| LeaseTransitionError::Exhausted)?;
        self.state = LeaseState::Reserved;
        self.revision = next_revision;
        self.attempt = Some(Attempt {
            id: attempt_id,
            client_nonce,
            hello_hash: Sha256::digest(hello_bytes).into(),
            session_id,
            server_nonce,
            welcome: welcome.clone(),
            transport_live: true,
            post_hello_admitted: false,
            resume_from: Some(expected_generation),
        });
        Ok(AttemptDecision::Fresh {
            session_id,
            server_nonce,
            welcome,
        })
    }

    /// Revokes the lease from any live state.
    ///
    /// # Errors
    ///
    /// Returns [`LeaseTransitionError::BadState`] when the lease is already closed, revoked, or
    /// expired, and [`LeaseTransitionError::Exhausted`] when the revision cannot advance.
    pub fn revoke(&mut self) -> Result<(), LeaseTransitionError> {
        if matches!(
            self.state,
            LeaseState::Closed | LeaseState::Revoked | LeaseState::Expired
        ) {
            return Err(LeaseTransitionError::BadState);
        }
        self.state = LeaseState::Revoked;
        self.logical_session_id = None;
        self.profile_fingerprint = None;
        self.attempt = None;
        self.advance_revision()
    }

    /// Expires the lease from any live state.
    ///
    /// # Errors
    ///
    /// Returns [`LeaseTransitionError::BadState`] when the lease is already closed, revoked, or
    /// expired, and [`LeaseTransitionError::Exhausted`] when the revision cannot advance.
    pub fn expire(&mut self) -> Result<(), LeaseTransitionError> {
        if matches!(
            self.state,
            LeaseState::Closed | LeaseState::Revoked | LeaseState::Expired
        ) {
            return Err(LeaseTransitionError::BadState);
        }
        self.state = LeaseState::Expired;
        self.logical_session_id = None;
        self.profile_fingerprint = None;
        self.attempt = None;
        self.advance_revision()
    }

    fn exact_retry(
        &self,
        attempt_id: [u8; 16],
        client_nonce: [u8; 32],
        hello_bytes: &[u8],
    ) -> Result<AttemptDecision, LeaseTransitionError> {
        let attempt = self
            .attempt
            .as_ref()
            .ok_or(LeaseTransitionError::AuthenticationFailed)?;
        if !attempt.exact(&attempt_id, &client_nonce, hello_bytes) {
            return Err(LeaseTransitionError::AuthenticationFailed);
        }
        Ok(AttemptDecision::ExactReplay {
            session_id: attempt.session_id,
            server_nonce: attempt.server_nonce,
            welcome: attempt.welcome.clone(),
        })
    }

    fn advance_revision(&mut self) -> Result<(), LeaseTransitionError> {
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or(LeaseTransitionError::Exhausted)?;
        Ok(())
    }
}

/// The `REVOKE_SESSION_LEASE` payload: end a lease and its child session (security §8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RevokeSessionLease {
    /// The context that issued the lease.
    pub context_id: u64,
    /// The lease to revoke.
    pub lease_id: u64,
}

impl RevokeSessionLease {
    /// Encodes the payload map.
    pub fn payload(&self) -> PayloadMap {
        vec![
            (0, Value::Unsigned(self.context_id)),
            (1, Value::Unsigned(self.lease_id)),
        ]
    }

    /// Decodes a `REVOKE_SESSION_LEASE` payload.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError`] when `value` is not a strict `REVOKE_SESSION_LEASE` map or a field
    /// is missing or not an unsigned integer.
    pub fn decode(value: &Value) -> Result<Self, MessageError> {
        let map = StrictMap::new("REVOKE_SESSION_LEASE", value, &[0, 1])?;
        Ok(Self {
            context_id: map.required_u64(0)?,
            lease_id: map.required_u64(1)?,
        })
    }
}
