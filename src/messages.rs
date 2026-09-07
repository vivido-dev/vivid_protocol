//! Deterministic Vivid 1.5 control-message codecs.
//!
//! All records in this module use the common envelope. Allocation and mutation payloads are
//! strict; negotiation payloads preserve unknown values because canonical CBOR re-encoding is
//! byte-stable.

use std::{collections::BTreeSet, fmt, io};

use zeroize::{Zeroize, Zeroizing};

use crate::{
    HARD_MAX_RECORD_BODY, VIVID_MAJOR, VIVID_MINOR,
    auth::{self, ATTEMPT_ID_BYTES, HandshakePrk, NONCE_BYTES, Secret32},
    cbor::{self, Value},
    registry::{self, CORE_CONTROL},
    resource::ResourceContract,
};

pub use crate::identity::{NodeIdentity, SurfaceIdentity, TrackIdentity};
pub use crate::registry::record::*;

pub const IDEMPOTENCY_KEY_BYTES: usize = 16;
pub const CAUSATION_ID_BYTES: usize = 16;
pub const SESSION_TAG_BYTES: usize = 16;

pub const AUTHENTICATION_ROOT: u64 = 0;
pub const AUTHENTICATION_LEASE_ACTIVATION: u64 = 1;
pub const AUTHENTICATION_RESUME: u64 = 2;

pub const ERROR_AUTH_FAILED: u64 = registry::error::AUTH_FAILED;
pub const ERROR_UNSUPPORTED_VERSION: u64 = registry::error::UNSUPPORTED_VERSION;
pub const ERROR_UNSUPPORTED_PROFILE: u64 = registry::error::UNSUPPORTED_PROFILE;
pub const ERROR_UNSUPPORTED_CONFIG: u64 = registry::error::UNSUPPORTED_CONFIG;
pub const ERROR_BAD_MESSAGE: u64 = registry::error::BAD_MESSAGE;
pub const ERROR_BAD_STATE: u64 = registry::error::BAD_STATE;
pub const ERROR_DUPLICATE_ID: u64 = registry::error::DUPLICATE_ID;
pub const ERROR_NOT_FOUND: u64 = registry::error::NOT_FOUND;
pub const ERROR_LIMIT_EXCEEDED: u64 = registry::error::LIMIT_EXCEEDED;
pub const ERROR_NO_MEMORY: u64 = registry::error::NO_MEMORY;
pub const ERROR_FLOW_CONTROL: u64 = registry::error::FLOW_CONTROL;
pub const ERROR_HASH_MISMATCH: u64 = registry::error::HASH_MISMATCH;
pub const ERROR_NEED_KEYFRAME: u64 = registry::error::NEED_KEYFRAME;
pub const ERROR_STALE_EPOCH: u64 = registry::error::STALE_EPOCH;
pub const ERROR_STALE_TARGET_GENERATION: u64 = registry::error::STALE_TARGET_GENERATION;
pub const ERROR_ANCHOR_INVALIDATED: u64 = registry::error::ANCHOR_INVALIDATED;
pub const ERROR_AUTHORITY_REVOKED: u64 = registry::error::AUTHORITY_REVOKED;
pub const ERROR_DECODER: u64 = registry::error::DECODER;
pub const ERROR_DEVICE_LOST: u64 = registry::error::DEVICE_LOST;
pub const ERROR_TIMEOUT: u64 = registry::error::TIMEOUT;
pub const ERROR_PRECONDITION_FAILED: u64 = registry::error::PRECONDITION_FAILED;
pub const ERROR_ALREADY_APPLIED: u64 = registry::error::ALREADY_APPLIED;
pub const ERROR_NOT_VISIBLE: u64 = registry::error::NOT_VISIBLE;
pub const ERROR_CANCELLED: u64 = registry::error::CANCELLED;
pub const ERROR_UNKNOWN_OUTCOME: u64 = registry::error::UNKNOWN_OUTCOME;
pub const ERROR_CHANNEL_BUSY: u64 = registry::error::CHANNEL_BUSY;
pub const ERROR_STALE_CHANNEL_GENERATION: u64 = registry::error::STALE_CHANNEL_GENERATION;
pub const ERROR_LEASE_SUSPENDED: u64 = registry::error::LEASE_SUSPENDED;
pub const ERROR_RATE_LIMITED: u64 = registry::error::RATE_LIMITED;
pub const ERROR_INTEGRITY_FAILED: u64 = registry::error::INTEGRITY_FAILED;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageError {
    Cbor(String),
    ExpectedMap(&'static str),
    MissingKey {
        schema: &'static str,
        key: u64,
    },
    UnknownKey {
        schema: &'static str,
        key: u64,
    },
    WrongType {
        schema: &'static str,
        key: u64,
    },
    InvalidValue {
        schema: &'static str,
        key: u64,
        reason: &'static str,
    },
    HeaderObjectMismatch {
        header: u64,
        payload: u64,
    },
}

impl fmt::Display for MessageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cbor(message) => write!(formatter, "CBOR error: {message}"),
            Self::ExpectedMap(schema) => write!(formatter, "{schema} is not a map"),
            Self::MissingKey { schema, key } => write!(formatter, "{schema} omits key {key}"),
            Self::UnknownKey { schema, key } => {
                write!(formatter, "{schema} contains unknown key {key}")
            }
            Self::WrongType { schema, key } => {
                write!(formatter, "{schema} key {key} has the wrong type")
            }
            Self::InvalidValue {
                schema,
                key,
                reason,
            } => write!(formatter, "{schema} key {key} {reason}"),
            Self::HeaderObjectMismatch { header, payload } => write!(
                formatter,
                "record object ID {header} does not match payload object ID {payload}"
            ),
        }
    }
}

impl std::error::Error for MessageError {}

impl From<MessageError> for io::Error {
    fn from(value: MessageError) -> Self {
        io::Error::new(io::ErrorKind::InvalidData, value)
    }
}

impl From<cbor::EncodeError> for MessageError {
    fn from(value: cbor::EncodeError) -> Self {
        Self::Cbor(value.to_string())
    }
}

impl From<cbor::DecodeError> for MessageError {
    fn from(value: cbor::DecodeError) -> Self {
        Self::Cbor(value.to_string())
    }
}

pub type PayloadMap = Vec<(u64, Value)>;

#[derive(Debug, Clone, PartialEq, Eq, Zeroize)]
pub struct Envelope {
    pub request_id: u64,
    pub transaction_id: Option<u64>,
    pub expected_target_generation: Option<u64>,
    pub payload: PayloadMap,
    pub preconditions: PayloadMap,
    pub idempotency_key: Option<[u8; IDEMPOTENCY_KEY_BYTES]>,
    pub causation_id: Option<[u8; CAUSATION_ID_BYTES]>,
}

impl Envelope {
    pub fn new(request_id: u64, payload: PayloadMap) -> Self {
        Self {
            request_id,
            transaction_id: None,
            expected_target_generation: None,
            payload,
            preconditions: Vec::new(),
            idempotency_key: None,
            causation_id: None,
        }
    }

    pub fn correlated(request_id: u64, payload: PayloadMap) -> Result<Self, MessageError> {
        if request_id == 0 {
            return Err(invalid("envelope", 0, "must be nonzero for a request"));
        }
        Ok(Self::new(request_id, payload))
    }

    pub fn encode(&self) -> Result<Vec<u8>, MessageError> {
        validate_sorted_map("payload", &self.payload)?;
        validate_sorted_map("preconditions", &self.preconditions)?;
        let mut map = Zeroizing::new(vec![(0, Value::Unsigned(self.request_id))]);
        if let Some(transaction_id) = self.transaction_id {
            map.push((1, Value::Unsigned(transaction_id)));
        }
        if let Some(generation) = self.expected_target_generation {
            map.push((2, Value::Unsigned(generation)));
        }
        map.push((3, Value::Map(self.payload.clone())));
        if !self.preconditions.is_empty() {
            map.push((4, Value::Map(self.preconditions.clone())));
        }
        if let Some(key) = self.idempotency_key {
            map.push((5, Value::Bytes(key.to_vec())));
        }
        if let Some(id) = self.causation_id {
            map.push((6, Value::Bytes(id.to_vec())));
        }
        let value = Zeroizing::new(Value::Map(std::mem::take(&mut *map)));
        Ok(cbor::encode(&value)?)
    }

    pub fn validate_request(&self) -> Result<(), MessageError> {
        if self.request_id == 0 {
            Err(invalid("envelope", 0, "must be nonzero for a request"))
        } else {
            Ok(())
        }
    }

    pub fn validate_event(&self) -> Result<(), MessageError> {
        if self.request_id != 0 {
            Err(invalid("envelope", 0, "must be zero for an event"))
        } else {
            Ok(())
        }
    }
}

/// Decode an owned envelope. The caller owns the returned payload and the borrowed input;
/// sensitive users should wrap the envelope in `zeroize::Zeroizing` until ownership transfers.
pub fn decode_control(body: &[u8]) -> Result<Envelope, MessageError> {
    let mut value = Zeroizing::new(cbor::decode(body)?);
    let map = StrictMap::new("envelope", &value, &[0, 1, 2, 3, 4, 5, 6])?;
    validate_sorted_map("payload", map.required_map(3)?)?;
    let preconditions = map.optional_map(4)?.unwrap_or_default();
    validate_sorted_map("preconditions", preconditions)?;
    for (key, _) in preconditions {
        if *key > 9 {
            return Err(MessageError::UnknownKey {
                schema: "preconditions",
                key: *key,
            });
        }
    }
    // Finish every fallible step while the entire decoded tree is still guarded. Move the
    // validated maps into the result rather than cloning secret-bearing subtrees.
    let mut envelope = Envelope {
        request_id: map.required_u64(0)?,
        transaction_id: map.optional_u64(1)?,
        expected_target_generation: map.optional_u64(2)?,
        idempotency_key: map.optional_fixed_bytes(5)?,
        causation_id: map.optional_fixed_bytes(6)?,
        payload: Vec::new(),
        preconditions: Vec::new(),
    };
    if let Value::Map(fields) = &mut *value {
        for (key, value) in fields {
            if let Value::Map(entries) = value {
                match key {
                    3 => envelope.payload = std::mem::take(entries),
                    4 => envelope.preconditions = std::mem::take(entries),
                    _ => {}
                }
            }
        }
    }
    Ok(envelope)
}

pub fn encode_payload(request_id: u64, payload: PayloadMap) -> Result<Vec<u8>, MessageError> {
    Zeroizing::new(Envelope::new(request_id, payload)).encode()
}

pub fn empty(request_id: u64) -> Vec<u8> {
    encode_payload(request_id, vec![]).expect("empty payload is valid")
}

pub fn ok(request_id: u64) -> Vec<u8> {
    empty(request_id)
}

pub enum HelloAuthentication {
    Root {
        proof: [u8; 32],
    },
    LeaseActivation {
        context_id: u64,
        lease_id: u64,
        activation_secret: Secret32,
        attempt_id: [u8; ATTEMPT_ID_BYTES],
        proof_of_possession: Option<Vec<u8>>,
    },
    Resume {
        context_id: u64,
        lease_id: u64,
        session_id: u64,
        resume_generation: u64,
        attempt_id: [u8; ATTEMPT_ID_BYTES],
        proof: [u8; 32],
    },
}

impl fmt::Debug for HelloAuthentication {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Root { .. } => formatter
                .debug_struct("Root")
                .field("proof", &"[REDACTED]")
                .finish(),
            Self::LeaseActivation {
                context_id,
                lease_id,
                attempt_id,
                proof_of_possession,
                ..
            } => formatter
                .debug_struct("LeaseActivation")
                .field("context_id", context_id)
                .field("lease_id", lease_id)
                .field("activation_secret", &"[REDACTED]")
                .field("attempt_id", attempt_id)
                .field(
                    "proof_of_possession",
                    &proof_of_possession.as_ref().map(|value| value.len()),
                )
                .finish(),
            Self::Resume {
                context_id,
                lease_id,
                session_id,
                resume_generation,
                attempt_id,
                ..
            } => formatter
                .debug_struct("Resume")
                .field("context_id", context_id)
                .field("lease_id", lease_id)
                .field("session_id", session_id)
                .field("resume_generation", resume_generation)
                .field("attempt_id", attempt_id)
                .field("proof", &"[REDACTED]")
                .finish(),
        }
    }
}

impl Drop for HelloAuthentication {
    fn drop(&mut self) {
        match self {
            Self::Root { proof } => proof.zeroize(),
            Self::LeaseActivation {
                proof_of_possession,
                ..
            } => {
                if let Some(proof) = proof_of_possession {
                    proof.zeroize();
                }
            }
            Self::Resume { proof, .. } => proof.zeroize(),
        }
    }
}

impl HelloAuthentication {
    pub const fn kind(&self) -> u64 {
        match self {
            Self::Root { .. } => AUTHENTICATION_ROOT,
            Self::LeaseActivation { .. } => AUTHENTICATION_LEASE_ACTIVATION,
            Self::Resume { .. } => AUTHENTICATION_RESUME,
        }
    }

    fn to_value(&self, omit_proof: bool) -> Value {
        let entries = match self {
            Self::Root { proof } => {
                let mut entries = vec![(0, Value::Unsigned(AUTHENTICATION_ROOT))];
                if !omit_proof {
                    entries.push((1, Value::Bytes(proof.to_vec())));
                }
                entries
            }
            Self::LeaseActivation {
                context_id,
                lease_id,
                activation_secret,
                attempt_id,
                proof_of_possession,
            } => {
                let mut entries = vec![
                    (0, Value::Unsigned(AUTHENTICATION_LEASE_ACTIVATION)),
                    (1, Value::Unsigned(*context_id)),
                    (2, Value::Unsigned(*lease_id)),
                    (3, Value::Bytes(activation_secret.expose().to_vec())),
                    (4, Value::Bytes(attempt_id.to_vec())),
                ];
                if let Some(proof) = proof_of_possession {
                    entries.push((5, Value::Bytes(proof.clone())));
                }
                entries
            }
            Self::Resume {
                context_id,
                lease_id,
                session_id,
                resume_generation,
                attempt_id,
                proof,
            } => {
                let mut entries = vec![
                    (0, Value::Unsigned(AUTHENTICATION_RESUME)),
                    (1, Value::Unsigned(*context_id)),
                    (2, Value::Unsigned(*lease_id)),
                    (3, Value::Unsigned(*session_id)),
                    (4, Value::Unsigned(*resume_generation)),
                    (5, Value::Bytes(attempt_id.to_vec())),
                ];
                if !omit_proof {
                    entries.push((6, Value::Bytes(proof.to_vec())));
                }
                entries
            }
        };
        Value::Map(entries)
    }

    fn from_value(value: &Value) -> Result<Self, MessageError> {
        value
            .as_map()
            .ok_or(MessageError::ExpectedMap("HELLO authentication"))?;
        let kind = StrictMap::new("HELLO authentication", value, &[0, 1, 2, 3, 4, 5, 6])?
            .required_u64(0)?;
        match kind {
            AUTHENTICATION_ROOT => {
                let map = StrictMap::new("root authentication", value, &[0, 1])?;
                Ok(Self::Root {
                    proof: map.required_fixed_bytes(1)?,
                })
            }
            AUTHENTICATION_LEASE_ACTIVATION => {
                let map = StrictMap::new(
                    "lease activation authentication",
                    value,
                    &[0, 1, 2, 3, 4, 5],
                )?;
                Ok(Self::LeaseActivation {
                    context_id: nonzero(
                        "lease activation authentication",
                        1,
                        map.required_u64(1)?,
                    )?,
                    lease_id: nonzero("lease activation authentication", 2, map.required_u64(2)?)?,
                    activation_secret: Secret32::new(map.required_fixed_bytes(3)?),
                    attempt_id: map.required_fixed_bytes(4)?,
                    proof_of_possession: map.optional_bytes(5)?.map(ToOwned::to_owned),
                })
            }
            AUTHENTICATION_RESUME => {
                let map = StrictMap::new("resume authentication", value, &[0, 1, 2, 3, 4, 5, 6])?;
                Ok(Self::Resume {
                    context_id: nonzero("resume authentication", 1, map.required_u64(1)?)?,
                    lease_id: nonzero("resume authentication", 2, map.required_u64(2)?)?,
                    session_id: nonzero("resume authentication", 3, map.required_u64(3)?)?,
                    resume_generation: map.required_u64(4)?,
                    attempt_id: map.required_fixed_bytes(5)?,
                    proof: map.required_fixed_bytes(6)?,
                })
            }
            _ => Err(invalid(
                "HELLO authentication",
                0,
                "is not a registered authentication kind",
            )),
        }
    }
}

#[derive(Debug)]
pub struct Hello {
    pub producer_name: String,
    pub producer_version: String,
    pub required_profiles: Vec<String>,
    pub optional_profiles: Vec<String>,
    pub maximum_control_body: u32,
    pub client_nonce: [u8; NONCE_BYTES],
    pub authentication: HelloAuthentication,
    pub target_profile: String,
    /// Canonical extension values. Keys 0 through 7 are reserved to the base schema.
    pub extensions: PayloadMap,
}

impl Hello {
    pub fn authenticate_root(
        &mut self,
        root_secret: &Secret32,
        preface: &[u8; 16],
    ) -> Result<(), MessageError> {
        if !matches!(self.authentication, HelloAuthentication::Root { .. }) {
            return Err(invalid(
                "HELLO authentication",
                0,
                "is not root authentication",
            ));
        }
        let authless = self.authless_payload()?;
        let proof = auth::root_hello_proof(root_secret, preface, &authless);
        self.authentication = HelloAuthentication::Root { proof };
        Ok(())
    }

    pub fn authenticate_resume(
        &mut self,
        prior_resume_key: &[u8; 32],
        preface: &[u8; 16],
    ) -> Result<(), MessageError> {
        let (lease_id, session_id, resume_generation, attempt_id) = match &self.authentication {
            HelloAuthentication::Resume {
                lease_id,
                session_id,
                resume_generation,
                attempt_id,
                ..
            } => (*lease_id, *session_id, *resume_generation, *attempt_id),
            _ => {
                return Err(invalid(
                    "HELLO authentication",
                    0,
                    "is not resume authentication",
                ));
            }
        };
        let authless = self.authless_payload()?;
        let proof = auth::resume_hello_proof(
            prior_resume_key,
            preface,
            lease_id,
            session_id,
            resume_generation,
            &attempt_id,
            &authless,
        );
        if let HelloAuthentication::Resume {
            proof: stored_proof,
            ..
        } = &mut self.authentication
        {
            *stored_proof = proof;
        }
        Ok(())
    }

    pub fn validate(&self) -> Result<(), MessageError> {
        bounded_text("HELLO", 0, &self.producer_name, 256)?;
        bounded_text("HELLO", 1, &self.producer_version, 128)?;
        validate_sorted_profiles("HELLO", 2, &self.required_profiles)?;
        validate_sorted_profiles("HELLO", 3, &self.optional_profiles)?;
        if self
            .required_profiles
            .iter()
            .any(|profile| self.optional_profiles.contains(profile))
        {
            return Err(invalid("HELLO", 3, "overlaps the required profile list"));
        }
        let required: BTreeSet<_> = self.required_profiles.iter().map(String::as_str).collect();
        if !required.contains(CORE_CONTROL) || !required.contains(self.target_profile.as_str()) {
            return Err(invalid(
                "HELLO",
                2,
                "must require the core and selected target profiles",
            ));
        }
        let offered: BTreeSet<_> = self
            .required_profiles
            .iter()
            .chain(&self.optional_profiles)
            .map(String::as_str)
            .collect();
        for profile in &self.required_profiles {
            let prerequisites = registry::prerequisites(profile)
                .ok_or_else(|| invalid("HELLO", 2, "contains an unknown required profile"))?;
            if prerequisites
                .iter()
                .any(|prerequisite| !offered.contains(prerequisite))
            {
                return Err(invalid("HELLO", 2, "is not prerequisite-closed"));
            }
        }
        for profile in &self.optional_profiles {
            if let Some(prerequisites) = registry::prerequisites(profile) {
                if prerequisites
                    .iter()
                    .any(|prerequisite| !offered.contains(prerequisite))
                {
                    return Err(invalid("HELLO", 3, "is not prerequisite-closed"));
                }
            }
        }
        if self.maximum_control_body == 0 || self.maximum_control_body > HARD_MAX_RECORD_BODY {
            return Err(invalid("HELLO", 4, "is outside the legal body range"));
        }
        if self.extensions.iter().any(|(key, _)| *key <= 7) {
            return Err(invalid("HELLO extensions", 0, "overlap a registered field"));
        }
        validate_sorted_map("HELLO extensions", &self.extensions)
    }

    pub fn payload_value(&self) -> Result<Value, MessageError> {
        self.payload_value_inner(false)
    }

    pub fn authless_payload_value(&self) -> Result<Value, MessageError> {
        self.payload_value_inner(true)
    }

    fn payload_value_inner(&self, omit_proof: bool) -> Result<Value, MessageError> {
        self.validate()?;
        let mut fields = vec![
            (0, Value::Text(self.producer_name.clone())),
            (1, Value::Text(self.producer_version.clone())),
            (2, text_array(&self.required_profiles)),
            (3, text_array(&self.optional_profiles)),
            (4, Value::Unsigned(u64::from(self.maximum_control_body))),
            (5, Value::Bytes(self.client_nonce.to_vec())),
            (6, self.authentication.to_value(omit_proof)),
            (7, Value::Text(self.target_profile.clone())),
        ];
        fields.extend(self.extensions.clone());
        Ok(Value::Map(fields))
    }

    pub fn authless_payload(&self) -> Result<Vec<u8>, MessageError> {
        let value = Zeroizing::new(self.authless_payload_value()?);
        Ok(cbor::encode(&value)?)
    }

    pub fn encode(&self, request_id: u64) -> Result<Vec<u8>, MessageError> {
        let Value::Map(payload) = self.payload_value()? else {
            unreachable!()
        };
        let envelope = Zeroizing::new(Envelope::new(request_id, payload));
        envelope.validate_request()?;
        envelope.encode()
    }

    pub fn decode(body: &[u8]) -> Result<(u64, Self), MessageError> {
        let mut envelope = Zeroizing::new(decode_control(body)?);
        envelope.validate_request()?;
        let value = Zeroizing::new(Value::Map(std::mem::take(&mut envelope.payload)));
        let hello = {
            let map = StrictMap::preserving("HELLO", &value, 7)?;
            Self {
                producer_name: map.required_text(0)?.to_owned(),
                producer_version: map.required_text(1)?.to_owned(),
                required_profiles: map.required_text_array(2)?,
                optional_profiles: map.required_text_array(3)?,
                maximum_control_body: map.required_u32(4)?,
                client_nonce: map.required_fixed_bytes(5)?,
                authentication: HelloAuthentication::from_value(map.required(6)?)?,
                target_profile: map.required_text(7)?.to_owned(),
                extensions: map.extensions_after(7),
            }
        };
        hello.validate()?;
        Ok((envelope.request_id, hello))
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct WelcomeAuthentication {
    pub kind: u64,
    pub confirmation: [u8; 32],
    pub lease_state: u64,
    pub activation_attempt_status: u64,
}

impl fmt::Debug for WelcomeAuthentication {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WelcomeAuthentication")
            .field("kind", &self.kind)
            .field("confirmation", &"[REDACTED]")
            .field("lease_state", &self.lease_state)
            .field("activation_attempt_status", &self.activation_attempt_status)
            .finish()
    }
}

impl Drop for WelcomeAuthentication {
    fn drop(&mut self) {
        self.confirmation.zeroize();
    }
}

impl WelcomeAuthentication {
    pub fn validate(&self) -> Result<(), MessageError> {
        if self.kind > AUTHENTICATION_RESUME {
            return Err(invalid(
                "WELCOME authentication",
                0,
                "is not a registered authentication kind",
            ));
        }
        if self.activation_attempt_status > 1 {
            return Err(invalid(
                "WELCOME authentication",
                3,
                "is not fresh or replayed",
            ));
        }
        if (self.kind == AUTHENTICATION_ROOT && self.lease_state != 0)
            || (self.kind != AUTHENTICATION_ROOT && !(1..=7).contains(&self.lease_state))
        {
            return Err(invalid(
                "WELCOME authentication",
                2,
                "is not a valid lease state for this authentication kind",
            ));
        }
        Ok(())
    }

    fn to_value(&self, omit_confirmation: bool) -> Value {
        let mut fields = vec![(0, Value::Unsigned(self.kind))];
        if !omit_confirmation {
            fields.push((1, Value::Bytes(self.confirmation.to_vec())));
        }
        fields.push((2, Value::Unsigned(self.lease_state)));
        fields.push((3, Value::Unsigned(self.activation_attempt_status)));
        Value::Map(fields)
    }

    fn from_value(value: &Value) -> Result<Self, MessageError> {
        let map = StrictMap::new("WELCOME authentication", value, &[0, 1, 2, 3])?;
        let authentication = Self {
            kind: map.required_u64(0)?,
            confirmation: map.required_fixed_bytes(1)?,
            lease_state: map.required_u64(2)?,
            activation_attempt_status: map.required_u64(3)?,
        };
        authentication.validate()?;
        Ok(authentication)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Welcome {
    pub session_id: u64,
    pub session_tag: [u8; SESSION_TAG_BYTES],
    pub root_context_id: u64,
    pub target_generation: u64,
    pub target_profile: String,
    pub target_descriptor: PayloadMap,
    pub accepted_profiles: Vec<String>,
    pub maximum_control_body: u32,
    pub server_nonce: [u8; NONCE_BYTES],
    pub authentication: WelcomeAuthentication,
    pub session_revision: u64,
    pub scene_revision: u64,
    pub resource_contract: ResourceContract,
    pub establishment_state: u64,
    pub resume_generation: u64,
    pub extensions: PayloadMap,
}

impl Welcome {
    pub fn confirm(&mut self, prk: &HandshakePrk) -> Result<(), MessageError> {
        let unconfirmed = self.unconfirmed_payload()?;
        self.authentication.confirmation = auth::welcome_confirmation(prk, &unconfirmed);
        Ok(())
    }

    pub fn validate(&self) -> Result<(), MessageError> {
        self.authentication.validate()?;
        nonzero("WELCOME", 0, self.session_id)?;
        nonzero("WELCOME", 2, self.root_context_id)?;
        nonzero("WELCOME", 3, self.target_generation)?;
        validate_profiles("WELCOME", 6, &self.accepted_profiles)?;
        if !self
            .accepted_profiles
            .iter()
            .any(|value| value == CORE_CONTROL)
            || !self
                .accepted_profiles
                .iter()
                .any(|value| value == &self.target_profile)
        {
            return Err(invalid(
                "WELCOME",
                6,
                "does not contain the core and target profiles",
            ));
        }
        if self.maximum_control_body == 0 || self.maximum_control_body > HARD_MAX_RECORD_BODY {
            return Err(invalid("WELCOME", 7, "is outside the legal body range"));
        }
        if self.establishment_state > 1 {
            return Err(invalid("WELCOME", 13, "is not new or resumed"));
        }
        if self.establishment_state == 0
            && self.authentication.kind == AUTHENTICATION_ROOT
            && self.resume_generation != 0
        {
            return Err(invalid(
                "WELCOME",
                14,
                "must be zero for a non-resumable root session",
            ));
        }
        if self.extensions.iter().any(|(key, _)| *key <= 14) {
            return Err(invalid(
                "WELCOME extensions",
                0,
                "overlap a registered field",
            ));
        }
        validate_sorted_map("WELCOME extensions", &self.extensions)
    }

    fn payload_value_inner(&self, omit_confirmation: bool) -> Result<Value, MessageError> {
        self.validate()?;
        let mut fields = vec![
            (0, Value::Unsigned(self.session_id)),
            (1, Value::Bytes(self.session_tag.to_vec())),
            (2, Value::Unsigned(self.root_context_id)),
            (3, Value::Unsigned(self.target_generation)),
            (4, Value::Text(self.target_profile.clone())),
            (5, Value::Map(self.target_descriptor.clone())),
            (6, text_array(&self.accepted_profiles)),
            (7, Value::Unsigned(u64::from(self.maximum_control_body))),
            (8, Value::Bytes(self.server_nonce.to_vec())),
            (9, self.authentication.to_value(omit_confirmation)),
            (10, Value::Unsigned(self.session_revision)),
            (11, Value::Unsigned(self.scene_revision)),
            (12, self.resource_contract.to_value()),
            (13, Value::Unsigned(self.establishment_state)),
            (14, Value::Unsigned(self.resume_generation)),
        ];
        fields.extend(self.extensions.clone());
        Ok(Value::Map(fields))
    }

    pub fn unconfirmed_payload(&self) -> Result<Vec<u8>, MessageError> {
        let value = Zeroizing::new(self.payload_value_inner(true)?);
        Ok(cbor::encode(&value)?)
    }

    pub fn encode(&self, request_id: u64) -> Result<Vec<u8>, MessageError> {
        let Value::Map(payload) = self.payload_value_inner(false)? else {
            unreachable!()
        };
        let envelope = Zeroizing::new(Envelope::new(request_id, payload));
        envelope.validate_request()?;
        envelope.encode()
    }

    pub fn decode(body: &[u8]) -> Result<(u64, Self), MessageError> {
        let mut envelope = Zeroizing::new(decode_control(body)?);
        envelope.validate_request()?;
        let value = Zeroizing::new(Value::Map(std::mem::take(&mut envelope.payload)));
        let welcome = {
            let map = StrictMap::preserving("WELCOME", &value, 14)?;
            let contract = ResourceContract::from_value(map.required(12)?)
                .map_err(|error| MessageError::Cbor(error.to_string()))?;
            Self {
                session_id: map.required_u64(0)?,
                session_tag: map.required_fixed_bytes(1)?,
                root_context_id: map.required_u64(2)?,
                target_generation: map.required_u64(3)?,
                target_profile: map.required_text(4)?.to_owned(),
                target_descriptor: map.required_map(5)?.to_vec(),
                accepted_profiles: map.required_text_array(6)?,
                maximum_control_body: map.required_u32(7)?,
                server_nonce: map.required_fixed_bytes(8)?,
                authentication: WelcomeAuthentication::from_value(map.required(9)?)?,
                session_revision: map.required_u64(10)?,
                scene_revision: map.required_u64(11)?,
                resource_contract: contract,
                establishment_state: map.required_u64(13)?,
                resume_generation: map.required_u64(14)?,
                extensions: map.extensions_after(14),
            }
        };
        welcome.validate()?;
        Ok((envelope.request_id, welcome))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorDetail {
    pub fields: PayloadMap,
}

impl ErrorDetail {
    pub fn new(fields: PayloadMap) -> Result<Self, MessageError> {
        let detail = Self { fields };
        detail.validate()?;
        Ok(detail)
    }

    pub fn validate(&self) -> Result<(), MessageError> {
        let fields = &self.fields;
        validate_sorted_map("ERROR detail", fields)?;
        if let Some((key, _)) = fields.iter().find(|(key, _)| *key > 18) {
            return Err(MessageError::UnknownKey {
                schema: "ERROR detail",
                key: *key,
            });
        }
        for (key, value) in fields {
            if (*key == 10 && value.as_bool().is_none()) || (*key != 10 && value.as_u64().is_none())
            {
                return Err(MessageError::WrongType {
                    schema: "ERROR detail",
                    key: *key,
                });
            }
            if *key == 12 && value.as_u64().is_some_and(|value| value > 2) {
                return Err(invalid(
                    "ERROR detail",
                    12,
                    "is not a registered idempotent result",
                ));
            }
        }
        let encoded = cbor::encode(&Value::Map(fields.clone()))?;
        if encoded.len() > 4096 {
            return Err(invalid("ERROR detail", 0, "exceeds 4096 encoded bytes"));
        }
        Ok(())
    }

    pub fn supported_version() -> Self {
        Self {
            fields: vec![
                (14, Value::Unsigned(u64::from(VIVID_MAJOR))),
                (15, Value::Unsigned(u64::from(VIVID_MINOR))),
            ],
        }
    }

    pub fn supported_version_tuple(&self) -> Option<(u64, u64)> {
        Some((
            self.fields.iter().find(|(key, _)| *key == 14)?.1.as_u64()?,
            self.fields.iter().find(|(key, _)| *key == 15)?.1.as_u64()?,
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorReply {
    pub code: u64,
    pub request_id: u64,
    pub detail: ErrorDetail,
    pub fatal: bool,
    pub diagnostic: String,
}

impl ErrorReply {
    pub fn encode(&self) -> Result<Vec<u8>, MessageError> {
        if !registry::error::is_registered(self.code) {
            return Err(invalid("ERROR", 0, "is not a registered error code"));
        }
        self.detail.validate()?;
        bounded_text("ERROR", 4, &self.diagnostic, 4096)?;
        encode_payload(
            self.request_id,
            vec![
                (0, Value::Unsigned(self.code)),
                (1, Value::Unsigned(self.request_id)),
                (2, Value::Map(self.detail.fields.clone())),
                (3, Value::Bool(self.fatal)),
                (4, Value::Text(self.diagnostic.clone())),
            ],
        )
    }
}

pub fn parse_error_reply(body: &[u8]) -> Result<ErrorReply, MessageError> {
    let envelope = decode_control(body)?;
    let value = Value::Map(envelope.payload);
    let map = StrictMap::new("ERROR", &value, &[0, 1, 2, 3, 4])?;
    let code = map.required_u64(0)?;
    if !registry::error::is_registered(code) {
        return Err(invalid("ERROR", 0, "is not a registered error code"));
    }
    let failed_request_id = map.required_u64(1)?;
    if failed_request_id != envelope.request_id {
        return Err(invalid(
            "ERROR",
            1,
            "does not match the envelope request ID",
        ));
    }
    let detail = ErrorDetail::new(map.required_map(2)?.to_vec())?;
    let diagnostic = map.required_text(4)?.to_owned();
    bounded_text("ERROR", 4, &diagnostic, 4096)?;
    Ok(ErrorReply {
        code,
        request_id: failed_request_id,
        detail,
        fatal: map.required_bool(3)?,
        diagnostic,
    })
}

pub fn unsupported_version_error() -> Vec<u8> {
    ErrorReply {
        code: ERROR_UNSUPPORTED_VERSION,
        request_id: 0,
        detail: ErrorDetail::supported_version(),
        fatal: true,
        diagnostic: "unsupported Vivid protocol version".into(),
    }
    .encode()
    .expect("static version error is valid")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum LaneClass {
    Control = 0,
    Interactive = 1,
    Realtime = 2,
    Bulk = 3,
}

impl TryFrom<u64> for LaneClass {
    type Error = MessageError;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Control),
            1 => Ok(Self::Interactive),
            2 => Ok(Self::Realtime),
            3 => Ok(Self::Bulk),
            _ => Err(invalid("lane", 1, "is not a registered lane class")),
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct LaneOpen {
    pub session_id: u64,
    pub lane_generation: u64,
    pub client_nonce: [u8; 16],
    pub authentication_tag: [u8; 16],
}

impl fmt::Debug for LaneOpen {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LaneOpen")
            .field("session_id", &self.session_id)
            .field("lane_generation", &self.lane_generation)
            .field("client_nonce", &self.client_nonce)
            .field("authentication_tag", &"[REDACTED]")
            .finish()
    }
}

impl Drop for LaneOpen {
    fn drop(&mut self) {
        self.authentication_tag.zeroize();
    }
}

impl LaneOpen {
    pub fn payload(&self) -> PayloadMap {
        vec![
            (0, Value::Unsigned(self.session_id)),
            (1, Value::Unsigned(LaneClass::Interactive as u64)),
            (2, Value::Unsigned(self.lane_generation)),
            (3, Value::Bytes(self.client_nonce.to_vec())),
            (4, Value::Bytes(self.authentication_tag.to_vec())),
        ]
    }

    pub fn decode(body: &[u8]) -> Result<Self, MessageError> {
        let mut envelope = Zeroizing::new(decode_control(body)?);
        envelope.validate_request()?;
        let value = Zeroizing::new(Value::Map(std::mem::take(&mut envelope.payload)));
        let map = StrictMap::new("LANE_OPEN", &value, &[0, 1, 2, 3, 4])?;
        let lane = LaneClass::try_from(map.required_u64(1)?)?;
        if lane != LaneClass::Interactive {
            return Err(invalid("LANE_OPEN", 1, "must be interactive"));
        }
        Ok(Self {
            session_id: nonzero("LANE_OPEN", 0, map.required_u64(0)?)?,
            lane_generation: nonzero("LANE_OPEN", 2, map.required_u64(2)?)?,
            client_nonce: map.required_fixed_bytes(3)?,
            authentication_tag: map.required_fixed_bytes(4)?,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum TrackKind {
    Video = 1,
    Audio = 2,
    Raster = 3,
    EncodedImage = 4,
}

impl TryFrom<u64> for TrackKind {
    type Error = MessageError;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::Video),
            2 => Ok(Self::Audio),
            3 => Ok(Self::Raster),
            4 => Ok(Self::EncodedImage),
            _ => Err(invalid("track", 5, "has an unknown kind")),
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ChannelOpen {
    pub session_id: u64,
    pub context_id: u64,
    pub surface_id: u64,
    pub track_id: u64,
    pub channel_generation: u64,
    pub track_kind: TrackKind,
    pub lane: LaneClass,
    pub client_nonce: [u8; 16],
    pub authentication_tag: [u8; 16],
}

impl fmt::Debug for ChannelOpen {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ChannelOpen")
            .field("session_id", &self.session_id)
            .field("context_id", &self.context_id)
            .field("surface_id", &self.surface_id)
            .field("track_id", &self.track_id)
            .field("channel_generation", &self.channel_generation)
            .field("track_kind", &self.track_kind)
            .field("lane", &self.lane)
            .field("client_nonce", &self.client_nonce)
            .field("authentication_tag", &"[REDACTED]")
            .finish()
    }
}

impl Drop for ChannelOpen {
    fn drop(&mut self) {
        self.authentication_tag.zeroize();
    }
}

impl ChannelOpen {
    pub fn payload(&self) -> PayloadMap {
        vec![
            (0, Value::Unsigned(self.session_id)),
            (1, Value::Unsigned(self.context_id)),
            (2, Value::Unsigned(self.surface_id)),
            (3, Value::Unsigned(self.track_id)),
            (4, Value::Unsigned(self.channel_generation)),
            (5, Value::Unsigned(self.track_kind as u64)),
            (6, Value::Unsigned(self.lane as u64)),
            (7, Value::Bytes(self.client_nonce.to_vec())),
            (8, Value::Bytes(self.authentication_tag.to_vec())),
        ]
    }

    pub fn decode(header_object_id: u64, body: &[u8]) -> Result<Self, MessageError> {
        let mut envelope = Zeroizing::new(decode_control(body)?);
        envelope.validate_request()?;
        let value = Zeroizing::new(Value::Map(std::mem::take(&mut envelope.payload)));
        let map = StrictMap::new("CHANNEL_OPEN", &value, &[0, 1, 2, 3, 4, 5, 6, 7, 8])?;
        let track_id = nonzero("CHANNEL_OPEN", 3, map.required_u64(3)?)?;
        validate_header_object(header_object_id, track_id)?;
        let lane = LaneClass::try_from(map.required_u64(6)?)?;
        if !matches!(lane, LaneClass::Realtime | LaneClass::Bulk) {
            return Err(invalid(
                "CHANNEL_OPEN",
                6,
                "must use the realtime or bulk lane",
            ));
        }
        Ok(Self {
            session_id: nonzero("CHANNEL_OPEN", 0, map.required_u64(0)?)?,
            context_id: nonzero("CHANNEL_OPEN", 1, map.required_u64(1)?)?,
            surface_id: nonzero("CHANNEL_OPEN", 2, map.required_u64(2)?)?,
            track_id,
            channel_generation: nonzero("CHANNEL_OPEN", 4, map.required_u64(4)?)?,
            track_kind: TrackKind::try_from(map.required_u64(5)?)?,
            lane,
            client_nonce: map.required_fixed_bytes(7)?,
            authentication_tag: map.required_fixed_bytes(8)?,
        })
    }
}

pub fn validate_header_object(header_object_id: u64, payload_id: u64) -> Result<(), MessageError> {
    if header_object_id != payload_id {
        Err(MessageError::HeaderObjectMismatch {
            header: header_object_id,
            payload: payload_id,
        })
    } else {
        Ok(())
    }
}

pub struct StrictMap<'a> {
    schema: &'static str,
    entries: &'a [(u64, Value)],
}

impl<'a> StrictMap<'a> {
    pub fn new(
        schema: &'static str,
        value: &'a Value,
        allowed: &[u64],
    ) -> Result<Self, MessageError> {
        let Value::Map(entries) = value else {
            return Err(MessageError::ExpectedMap(schema));
        };
        if let Some((key, _)) = entries.iter().find(|(key, _)| !allowed.contains(key)) {
            return Err(MessageError::UnknownKey { schema, key: *key });
        }
        Ok(Self { schema, entries })
    }

    pub fn preserving(
        schema: &'static str,
        value: &'a Value,
        last_known_key: u64,
    ) -> Result<Self, MessageError> {
        let Value::Map(entries) = value else {
            return Err(MessageError::ExpectedMap(schema));
        };
        for expected in 0..=last_known_key {
            if !entries.iter().any(|(key, _)| *key == expected) {
                return Err(MessageError::MissingKey {
                    schema,
                    key: expected,
                });
            }
        }
        Ok(Self { schema, entries })
    }

    pub fn required(&self, key: u64) -> Result<&'a Value, MessageError> {
        self.entries
            .iter()
            .find_map(|(entry_key, value)| (*entry_key == key).then_some(value))
            .ok_or(MessageError::MissingKey {
                schema: self.schema,
                key,
            })
    }

    pub fn optional(&self, key: u64) -> Option<&'a Value> {
        self.entries
            .iter()
            .find_map(|(entry_key, value)| (*entry_key == key).then_some(value))
    }

    pub fn required_u64(&self, key: u64) -> Result<u64, MessageError> {
        self.required(key)?.as_u64().ok_or(MessageError::WrongType {
            schema: self.schema,
            key,
        })
    }

    pub fn required_u32(&self, key: u64) -> Result<u32, MessageError> {
        u32::try_from(self.required_u64(key)?).map_err(|_| MessageError::InvalidValue {
            schema: self.schema,
            key,
            reason: "does not fit in u32",
        })
    }

    pub fn optional_u64(&self, key: u64) -> Result<Option<u64>, MessageError> {
        self.optional(key)
            .map(|value| {
                value.as_u64().ok_or(MessageError::WrongType {
                    schema: self.schema,
                    key,
                })
            })
            .transpose()
    }

    pub fn required_bool(&self, key: u64) -> Result<bool, MessageError> {
        self.required(key)?
            .as_bool()
            .ok_or(MessageError::WrongType {
                schema: self.schema,
                key,
            })
    }

    pub fn required_text(&self, key: u64) -> Result<&'a str, MessageError> {
        self.required(key)?
            .as_text()
            .ok_or(MessageError::WrongType {
                schema: self.schema,
                key,
            })
    }

    pub fn optional_text(&self, key: u64) -> Result<Option<&'a str>, MessageError> {
        self.optional(key)
            .map(|value| {
                value.as_text().ok_or(MessageError::WrongType {
                    schema: self.schema,
                    key,
                })
            })
            .transpose()
    }

    pub fn required_bytes(&self, key: u64) -> Result<&'a [u8], MessageError> {
        self.required(key)?
            .as_bytes()
            .ok_or(MessageError::WrongType {
                schema: self.schema,
                key,
            })
    }

    pub fn optional_bytes(&self, key: u64) -> Result<Option<&'a [u8]>, MessageError> {
        self.optional(key)
            .map(|value| {
                value.as_bytes().ok_or(MessageError::WrongType {
                    schema: self.schema,
                    key,
                })
            })
            .transpose()
    }

    pub fn required_fixed_bytes<const N: usize>(&self, key: u64) -> Result<[u8; N], MessageError> {
        self.required_bytes(key)?
            .try_into()
            .map_err(|_| MessageError::InvalidValue {
                schema: self.schema,
                key,
                reason: "has the wrong byte length",
            })
    }

    pub fn optional_fixed_bytes<const N: usize>(
        &self,
        key: u64,
    ) -> Result<Option<[u8; N]>, MessageError> {
        self.optional_bytes(key)?
            .map(|value| {
                value.try_into().map_err(|_| MessageError::InvalidValue {
                    schema: self.schema,
                    key,
                    reason: "has the wrong byte length",
                })
            })
            .transpose()
    }

    pub fn required_map(&self, key: u64) -> Result<&'a [(u64, Value)], MessageError> {
        self.required(key)?.as_map().ok_or(MessageError::WrongType {
            schema: self.schema,
            key,
        })
    }

    pub fn optional_map(&self, key: u64) -> Result<Option<&'a [(u64, Value)]>, MessageError> {
        self.optional(key)
            .map(|value| {
                value.as_map().ok_or(MessageError::WrongType {
                    schema: self.schema,
                    key,
                })
            })
            .transpose()
    }

    pub fn required_text_array(&self, key: u64) -> Result<Vec<String>, MessageError> {
        let values = self
            .required(key)?
            .as_array()
            .ok_or(MessageError::WrongType {
                schema: self.schema,
                key,
            })?;
        values
            .iter()
            .map(|value| {
                value
                    .as_text()
                    .map(ToOwned::to_owned)
                    .ok_or(MessageError::WrongType {
                        schema: self.schema,
                        key,
                    })
            })
            .collect()
    }

    pub fn extensions_after(&self, last_known_key: u64) -> PayloadMap {
        self.entries
            .iter()
            .filter(|(key, _)| *key > last_known_key)
            .cloned()
            .collect()
    }
}

fn text_array(values: &[String]) -> Value {
    Value::Array(values.iter().cloned().map(Value::Text).collect())
}

fn validate_profiles(
    schema: &'static str,
    key: u64,
    profiles: &[String],
) -> Result<(), MessageError> {
    if profiles
        .windows(2)
        .any(|pair| pair[0].as_str() >= pair[1].as_str())
    {
        return Err(invalid(schema, key, "is not sorted and unique"));
    }
    registry::validate_profile_set(profiles.iter().map(String::as_str))
        .map_err(|_| invalid(schema, key, "is not prerequisite-closed"))
}

fn validate_sorted_profiles(
    schema: &'static str,
    key: u64,
    profiles: &[String],
) -> Result<(), MessageError> {
    if profiles
        .windows(2)
        .any(|pair| pair[0].as_str() >= pair[1].as_str())
    {
        Err(invalid(schema, key, "is not sorted and unique"))
    } else {
        Ok(())
    }
}

fn validate_sorted_map(schema: &'static str, map: &[(u64, Value)]) -> Result<(), MessageError> {
    if map.windows(2).any(|pair| pair[0].0 >= pair[1].0) {
        Err(invalid(schema, 0, "keys are not strictly increasing"))
    } else {
        Ok(())
    }
}

fn bounded_text(
    schema: &'static str,
    key: u64,
    value: &str,
    maximum: usize,
) -> Result<(), MessageError> {
    if value.len() > maximum {
        Err(invalid(schema, key, "exceeds its UTF-8 byte limit"))
    } else {
        Ok(())
    }
}

fn nonzero(schema: &'static str, key: u64, value: u64) -> Result<u64, MessageError> {
    if value == 0 {
        Err(invalid(schema, key, "must be nonzero"))
    } else {
        Ok(value)
    }
}

fn invalid(schema: &'static str, key: u64, reason: &'static str) -> MessageError {
    MessageError::InvalidValue {
        schema,
        key,
        reason,
    }
}

pub fn invalid_value(schema: &'static str, key: u64, reason: &'static str) -> MessageError {
    invalid(schema, key, reason)
}

pub fn require_nonzero(schema: &'static str, key: u64, value: u64) -> Result<u64, MessageError> {
    nonzero(schema, key, value)
}

trait ValueMapExt {
    fn as_map(&self) -> Option<&[(u64, Value)]>;
}

impl ValueMapExt for Value {
    fn as_map(&self) -> Option<&[(u64, Value)]> {
        match self {
            Self::Map(entries) => Some(entries),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        CONTROL_MAX_RECORD_BODY, auth,
        registry::{DESKTOP_SURFACE, LIVE_MEDIA},
        resource::ResourceContract,
        wire::{ConnectionKind, encode_preface},
    };

    fn root_hello() -> Hello {
        Hello {
            producer_name: "test producer".into(),
            producer_version: "1.5.0".into(),
            required_profiles: vec![
                DESKTOP_SURFACE.into(),
                LIVE_MEDIA.into(),
                CORE_CONTROL.into(),
            ],
            optional_profiles: vec![],
            maximum_control_body: CONTROL_MAX_RECORD_BODY,
            client_nonce: [1; 32],
            authentication: HelloAuthentication::Root { proof: [2; 32] },
            target_profile: DESKTOP_SURFACE.into(),
            extensions: vec![(100, Value::Text("preserved".into()))],
        }
    }

    #[test]
    fn envelope_round_trip_includes_execution_metadata() {
        let mut envelope = Envelope::correlated(9, vec![(0, Value::Unsigned(3))]).unwrap();
        envelope.transaction_id = Some(4);
        envelope.expected_target_generation = Some(7);
        envelope.preconditions = vec![(0, Value::Unsigned(8))];
        envelope.idempotency_key = Some([5; 16]);
        envelope.causation_id = Some([6; 16]);
        assert_eq!(
            decode_control(&envelope.encode().unwrap()).unwrap(),
            envelope
        );
    }

    #[test]
    fn hello_has_no_version_range_and_preserves_extensions() {
        let hello = root_hello();
        let body = hello.encode(1).unwrap();
        let (request_id, parsed) = Hello::decode(&body).unwrap();
        assert_eq!(request_id, 1);
        assert_eq!(parsed.extensions, hello.extensions);
        assert_eq!(parsed.target_profile, DESKTOP_SURFACE);
    }

    #[test]
    fn hello_accepts_zero_as_the_initial_resume_generation() {
        let mut hello = root_hello();
        hello.authentication = HelloAuthentication::Resume {
            context_id: 1,
            lease_id: 2,
            session_id: 3,
            resume_generation: 0,
            attempt_id: [4; 16],
            proof: [5; 32],
        };

        let (_, parsed) = Hello::decode(&hello.encode(1).unwrap()).unwrap();
        assert!(matches!(
            parsed.authentication,
            HelloAuthentication::Resume {
                resume_generation: 0,
                ..
            }
        ));
    }

    #[test]
    fn hello_rejects_unclosed_profiles() {
        let mut hello = root_hello();
        hello.required_profiles = vec!["desktop-input-v1".into(), CORE_CONTROL.into()];
        assert!(hello.validate().is_err());
    }

    #[test]
    fn hello_ignores_unknown_optional_profile_syntax() {
        let mut hello = root_hello();
        hello.optional_profiles = vec!["future-profile-v9".into()];
        hello.validate().unwrap();
    }

    #[test]
    fn hello_root_builder_matches_transcript_verifier_and_redacts_debug() {
        let secret = Secret32::new([7; 32]);
        let preface = encode_preface(ConnectionKind::Control, CONTROL_MAX_RECORD_BODY);
        let mut hello = root_hello();
        hello.authenticate_root(&secret, &preface).unwrap();
        let authless = hello.authless_payload().unwrap();
        let HelloAuthentication::Root { proof } = &hello.authentication else {
            panic!("not root authentication");
        };
        assert!(auth::verify_root_hello_proof(
            &secret, &preface, &authless, proof
        ));
        assert!(format!("{:?}", hello.authentication).contains("[REDACTED]"));
        assert!(!format!("{:?}", hello.authentication).contains(&hex(proof)));
    }

    #[test]
    fn welcome_round_trip_carries_all_resource_fields() {
        let welcome = Welcome {
            session_id: 1,
            session_tag: [2; 16],
            root_context_id: 3,
            target_generation: 1,
            target_profile: DESKTOP_SURFACE.into(),
            target_descriptor: vec![],
            accepted_profiles: vec![DESKTOP_SURFACE.into(), CORE_CONTROL.into()],
            maximum_control_body: CONTROL_MAX_RECORD_BODY,
            server_nonce: [4; 32],
            authentication: WelcomeAuthentication {
                kind: AUTHENTICATION_ROOT,
                confirmation: [5; 32],
                lease_state: 0,
                activation_attempt_status: 0,
            },
            session_revision: 1,
            scene_revision: 0,
            resource_contract: ResourceContract::denied(),
            establishment_state: 0,
            resume_generation: 0,
            extensions: vec![],
        };
        let body = welcome.encode(9).unwrap();
        let (request_id, parsed) = Welcome::decode(&body).unwrap();
        assert_eq!(request_id, 9);
        assert_eq!(parsed, welcome);
    }

    #[test]
    fn lane_and_track_open_schemas_bind_complete_owner_fields() {
        let lane = LaneOpen {
            session_id: 1,
            lane_generation: 1,
            client_nonce: [2; 16],
            authentication_tag: [3; 16],
        };
        let lane_body = encode_payload(1, lane.payload()).unwrap();
        assert_eq!(LaneOpen::decode(&lane_body).unwrap().session_id, 1);

        let channel = ChannelOpen {
            session_id: 1,
            context_id: 2,
            surface_id: 3,
            track_id: 4,
            channel_generation: 1,
            track_kind: TrackKind::Video,
            lane: LaneClass::Realtime,
            client_nonce: [5; 16],
            authentication_tag: [6; 16],
        };
        let channel_body = encode_payload(1, channel.payload()).unwrap();
        let parsed = ChannelOpen::decode(4, &channel_body).unwrap();
        assert_eq!(
            (parsed.context_id, parsed.surface_id, parsed.track_id),
            (2, 3, 4)
        );
        assert!(ChannelOpen::decode(5, &channel_body).is_err());
    }

    #[test]
    fn typed_version_error_round_trip() {
        let parsed = parse_error_reply(&unsupported_version_error()).unwrap();
        assert_eq!(parsed.code, ERROR_UNSUPPORTED_VERSION);
        assert!(parsed.fatal);
        assert_eq!(
            parsed.detail.supported_version_tuple(),
            Some((u64::from(VIVID_MAJOR), u64::from(VIVID_MINOR)))
        );
    }

    #[test]
    fn complete_identity_does_not_alias_reused_local_ids() {
        let session =
            crate::identity::SessionIdentity::new(crate::identity::PresenterInstanceId([1; 16]), 1)
                .unwrap();
        let first = session
            .context(10)
            .unwrap()
            .surface(20)
            .unwrap()
            .track(7)
            .unwrap();
        let second = session
            .context(11)
            .unwrap()
            .surface(20)
            .unwrap()
            .track(7)
            .unwrap();
        assert_ne!(first, second);
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }
}
