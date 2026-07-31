//! Retry-safe delegated session leases, suspension, and bounded resume state.

use sha2::{Digest, Sha256};

use crate::{
    cbor::Value,
    messages::{MessageError, PayloadMap, StrictMap, invalid_value, require_nonzero},
    registry,
    resource::ResourceContract,
    revision::ResumeGeneration,
};

pub const MAX_ACTIVATION_TIMEOUT_US: u64 = 60_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
pub enum CleanupPolicy {
    Immediate = 0,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionLeaseDefinition {
    pub context_id: u64,
    pub lease_id: u64,
    pub activation_verifier: [u8; 32],
    pub activation_timeout_us: u64,
    pub requested_disconnect_grace_us: u64,
    pub cleanup_policy: CleanupPolicy,
    pub permitted_profiles: Vec<String>,
    pub requested_contract: ResourceContract,
    pub client_public_key: Option<Vec<u8>>,
}

impl SessionLeaseDefinition {
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
            requested_contract: ResourceContract::from_value(map.required(7)?)
                .map_err(|error| MessageError::Cbor(error.to_string()))?,
            client_public_key: map.optional_bytes(8)?.map(ToOwned::to_owned),
        };
        definition.validate()?;
        Ok(definition)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
pub enum LeaseState {
    Issued = 1,
    Reserved = 2,
    Active = 3,
    Suspended = 4,
    Closed = 5,
    Revoked = 6,
    Expired = 7,
}

#[derive(Clone, PartialEq, Eq)]
struct Attempt {
    attempt_id: [u8; 16],
    client_nonce: [u8; 32],
    hello_hash: [u8; 32],
    session_id: u64,
    server_nonce: [u8; 32],
    welcome: Vec<u8>,
    transport_live: bool,
    post_hello_admitted: bool,
}

impl Attempt {
    fn exact(&self, attempt_id: &[u8; 16], client_nonce: &[u8; 32], hello_bytes: &[u8]) -> bool {
        self.attempt_id == *attempt_id
            && self.client_nonce == *client_nonce
            && self.hello_hash == Sha256::digest(hello_bytes).as_slice()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttemptDecision {
    Fresh {
        session_id: u64,
        server_nonce: [u8; 32],
        welcome: Vec<u8>,
    },
    ExactReplay {
        session_id: u64,
        server_nonce: [u8; 32],
        welcome: Vec<u8>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeaseTransitionError {
    AuthenticationFailed,
    BadState,
    StaleResumeGeneration,
    Exhausted,
}

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

    pub const fn state(&self) -> LeaseState {
        self.state
    }

    pub const fn revision(&self) -> u64 {
        self.revision
    }

    pub const fn resume_generation(&self) -> ResumeGeneration {
        self.resume_generation
    }

    #[allow(clippy::too_many_arguments)]
    pub fn begin_activation(
        &mut self,
        attempt_id: [u8; 16],
        client_nonce: [u8; 32],
        hello_bytes: &[u8],
        profile_fingerprint: [u8; 32],
        session_id: u64,
        server_nonce: [u8; 32],
        welcome: Vec<u8>,
    ) -> Result<AttemptDecision, LeaseTransitionError> {
        if session_id == 0 {
            return Err(LeaseTransitionError::BadState);
        }
        match self.state {
            LeaseState::Issued => {
                self.state = LeaseState::Reserved;
                self.advance_revision()?;
                self.logical_session_id = Some(session_id);
                self.profile_fingerprint = Some(profile_fingerprint);
                self.attempt = Some(Attempt {
                    attempt_id,
                    client_nonce,
                    hello_hash: Sha256::digest(hello_bytes).into(),
                    session_id,
                    server_nonce,
                    welcome: welcome.clone(),
                    transport_live: true,
                    post_hello_admitted: false,
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
                self.exact_retry(attempt_id, client_nonce, hello_bytes)
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

    pub fn commit_welcome(&mut self) -> Result<(), LeaseTransitionError> {
        if self.state != LeaseState::Reserved {
            return Err(LeaseTransitionError::BadState);
        }
        self.state = LeaseState::Active;
        self.advance_revision()
    }

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

    #[allow(clippy::too_many_arguments)]
    pub fn begin_resume(
        &mut self,
        expected_generation: ResumeGeneration,
        attempt_id: [u8; 16],
        client_nonce: [u8; 32],
        hello_bytes: &[u8],
        profile_fingerprint: [u8; 32],
        session_id: u64,
        server_nonce: [u8; 32],
        welcome: Vec<u8>,
    ) -> Result<AttemptDecision, LeaseTransitionError> {
        if self.state == LeaseState::Reserved {
            if self.profile_fingerprint != Some(profile_fingerprint)
                || self.logical_session_id != Some(session_id)
            {
                return Err(LeaseTransitionError::AuthenticationFailed);
            }
            return self.exact_retry(attempt_id, client_nonce, hello_bytes);
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
        self.resume_generation = self
            .resume_generation
            .advance()
            .map_err(|_| LeaseTransitionError::Exhausted)?;
        self.state = LeaseState::Reserved;
        self.advance_revision()?;
        self.attempt = Some(Attempt {
            attempt_id,
            client_nonce,
            hello_hash: Sha256::digest(hello_bytes).into(),
            session_id,
            server_nonce,
            welcome: welcome.clone(),
            transport_live: true,
            post_hello_admitted: false,
        });
        Ok(AttemptDecision::Fresh {
            session_id,
            server_nonce,
            welcome,
        })
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lost_welcome_retries_same_nonsecret_outcome() {
        let mut lease = LeaseMachine::new(CleanupPolicy::SuspendOnUncleanLoss, 1_000_000);
        let first = lease
            .begin_activation(
                [1; 16],
                [2; 32],
                b"hello",
                [4; 32],
                7,
                [3; 32],
                b"welcome".to_vec(),
            )
            .unwrap();
        let retry = lease
            .begin_activation(
                [1; 16],
                [2; 32],
                b"hello",
                [4; 32],
                99,
                [9; 32],
                b"other".to_vec(),
            )
            .unwrap();
        assert!(matches!(
            first,
            AttemptDecision::Fresh { session_id: 7, .. }
        ));
        assert!(matches!(
            retry,
            AttemptDecision::ExactReplay { session_id: 7, .. }
        ));
        assert_eq!(
            lease.begin_activation([4; 16], [2; 32], b"hello", [4; 32], 8, [3; 32], Vec::new()),
            Err(LeaseTransitionError::AuthenticationFailed)
        );
    }

    #[test]
    fn active_lease_suspends_and_resume_generation_advances_once() {
        let mut lease = LeaseMachine::new(CleanupPolicy::SuspendOnUncleanLoss, 1_000_000);
        lease
            .begin_activation([1; 16], [2; 32], b"hello", [4; 32], 7, [3; 32], Vec::new())
            .unwrap();
        lease.commit_welcome().unwrap();
        lease.admit_post_hello().unwrap();
        assert_eq!(
            lease.confirm_transport_lost(false).unwrap(),
            LeaseState::Suspended
        );
        lease
            .begin_resume(
                ResumeGeneration::ZERO,
                [4; 16],
                [5; 32],
                b"resume",
                [4; 32],
                7,
                [6; 32],
                Vec::new(),
            )
            .unwrap();
        assert_eq!(lease.resume_generation(), ResumeGeneration::ONE);
        assert_eq!(
            lease.begin_resume(
                ResumeGeneration::ZERO,
                [7; 16],
                [8; 32],
                b"different",
                [4; 32],
                7,
                [9; 32],
                Vec::new(),
            ),
            Err(LeaseTransitionError::AuthenticationFailed)
        );
    }

    #[test]
    fn transport_loss_before_post_hello_allows_only_exact_activation_retry() {
        let mut lease = LeaseMachine::new(CleanupPolicy::SuspendOnUncleanLoss, 1_000_000);
        lease
            .begin_activation(
                [1; 16],
                [2; 32],
                b"hello",
                [4; 32],
                7,
                [3; 32],
                b"welcome".to_vec(),
            )
            .unwrap();
        lease.commit_welcome().unwrap();
        assert_eq!(
            lease.confirm_transport_lost(false).unwrap(),
            LeaseState::Active
        );
        assert!(matches!(
            lease
                .begin_activation([1; 16], [2; 32], b"hello", [4; 32], 99, [9; 32], Vec::new(),)
                .unwrap(),
            AttemptDecision::ExactReplay { session_id: 7, .. }
        ));
    }
}
