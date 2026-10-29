//! Deterministic Vivid 1.5 control-message codecs.
//!
//! All records in this module use the common envelope. Allocation and mutation payloads are
//! strict; negotiation payloads preserve unknown values because canonical CBOR re-encoding is
//! byte-stable.

use std::{collections::BTreeSet, fmt, io};

use zeroize::{Zeroize, Zeroizing};

use crate::{
    HARD_MAX_RECORD_BODY, VIVID_MAJOR, VIVID_MINOR,
    auth::{
        self, ATTEMPT_ID_BYTES, ChannelKey, HandshakePrk, NONCE_BYTES, ResumeBinding, ResumeKey,
        Secret32,
    },
    cbor::{self, Value},
    registry::{self, CORE_CONTROL},
    resource::{ResourceContract, ResourceError},
};

// Record types are re-exported by name so that each assignment reaches this module
// deliberately; add new registry::record constants here as well. Retired codes stay in the
// registry, reachable only through `is_retired_record`, so they cannot be sent by name.
#[doc(inline)]
pub use crate::registry::record::{
    ABORT_TXN, ACCEPT_FILE_DROP, ACTIVATE_TRACK, ADVANCE_CHANNEL, ADVANCE_FILE_TRANSFER,
    ANCHOR_GONE, ANCHOR_READY, ANCHOR_STATUS, AUDIO_PACKET, BEGIN_TXN, CANCEL_FILE_DROP,
    CANCEL_WAIT, CAPS_CHANGED, CHANNEL_ACCEPTED, CHANNEL_ADVANCED, CHANNEL_EOS, CHANNEL_OPEN,
    COMMIT_TXN, CONTEXT_CHANGED, CONTEXT_READY, CREATE_CONTEXT, CREATE_NODE, CREATE_SESSION_LEASE,
    CREATE_SURFACE, CREATE_TRACK, DELETE_NODE, DESTROY_SURFACE, DESTROY_TRACK, DRAIN, ERROR,
    FILE_DATA, FILE_DROP_ACCEPTED, FILE_DROP_BOUND, FILE_DROP_CANCELLED, FILE_DROP_OFFER,
    FILE_DROP_STATUS, FILE_FINISH, FILE_RESULT, FILE_TRANSFER_ABORT, FILE_TRANSFER_ACCEPTED,
    FILE_TRANSFER_ADVANCED, FILE_TRANSFER_OPEN, FLUSH, GOODBYE, HELLO, IMAGE_DATA, INPUT_BOUND,
    INPUT_LEASE_RENEW, INPUT_RESET, INPUT_REVOKED, KEY_INPUT, LANE_ACCEPTED, LANE_OPEN,
    MAX_CHANNEL_DATA, MAX_FILE_DATA, MEASURE_OVERLAY_TEXT, MEASURE_OVERLAY_TEXT_BATCH,
    NEED_FULL_FRAME, NEED_KEYFRAME, OBSERVATION_GAP, OK, OVERLAY_ACTION, OVERLAY_ENV_CHANGED,
    OVERLAY_INPUT_CAPTURE, OVERLAY_INPUT_EVENT, OVERLAY_INPUT_RENEW, OVERLAY_STATUS,
    OVERLAY_SUBMISSION_OUTCOME, OVERLAY_TEXT_BATCH_MEASURED, OVERLAY_TEXT_MEASURED,
    OVERLAY_VIEWPORT_CHANGED, OVERLAY_WINDOW_READY, PAUSE, PING, PLAY, PLAYBACK_HOLD,
    PLAYBACK_STATE, POINTER_AXIS, POINTER_BUTTON, POINTER_MOTION, PONG, PROBE_TRACK_CONFIG,
    QUERY_ANCHOR, QUERY_FILE_DROP, QUERY_OVERLAY, QUERY_SCENE, QUERY_SESSION, QUERY_SURFACE,
    QUERY_TRACK, RASTER_FRAME, RELEASE_OVERLAY_TEXT_LAYOUTS, REVOKE_CONTEXT, REVOKE_SESSION_LEASE,
    SCENE_CHANGED, SCENE_PRESENTED, SCENE_STATUS, SESSION_LEASE_CHANGED, SESSION_LEASE_READY,
    SESSION_STATUS, SET_AUDIO_GAIN, SET_FILE_DROP_BINDING, SET_INPUT_BINDING, SET_OBSERVATION,
    SET_OVERLAY_CLIPBOARD, SET_OVERLAY_EDITOR, SET_OVERLAY_SEMANTICS, SET_OVERLAY_WINDOW,
    SURFACE_CHANGED, SURFACE_READY, SURFACE_STATUS, TARGET_CHANGED, TRACK_ACTIVATED, TRACK_CHANGED,
    TRACK_LOST, TRACK_READY, TRACK_STATUS, TRACK_SUPPORT, UPDATE_NODE, UPDATE_SURFACE,
    VECTOR_ASSET, VECTOR_ASSET_RELEASE, VECTOR_FRAME, VIDEO_PACKET, WAIT_SATISFIED, WAIT_TRACK,
    WELCOME,
};

/// Whether `record_type` is a retired assignment that a 1.5 peer must not send.
pub const fn is_retired_record(record_type: u16) -> bool {
    registry::record::is_retired(record_type)
}

/// Whether `record_type` lies in the range reserved for explicitly negotiated experiments.
pub const fn is_experimental_record(record_type: u16) -> bool {
    registry::record::is_experimental(record_type)
}

/// Whether `record_type` lies in the vendor range, which stays disabled without negotiation.
pub const fn is_vendor_record(record_type: u16) -> bool {
    registry::record::is_vendor(record_type)
}

/// Whether `code` is an `ERROR` code the 1.5 registry assigns.
pub const fn is_registered_error(code: u64) -> bool {
    registry::error::is_registered(code)
}

/// Length of an envelope idempotency key (envelope key 5).
pub const IDEMPOTENCY_KEY_BYTES: usize = 16;
/// Length of an envelope causation ID (envelope key 6).
pub const CAUSATION_ID_BYTES: usize = 16;
/// Length of the non-secret session tag in `WELCOME` key 1.
pub const SESSION_TAG_BYTES: usize = 16;

/// `HELLO` authentication kind for root authentication (security §3.1).
pub const AUTHENTICATION_ROOT: u64 = 0;
/// `HELLO` authentication kind for first lease activation (security §6.3).
pub const AUTHENTICATION_LEASE_ACTIVATION: u64 = 1;
/// `HELLO` authentication kind for resuming a suspended session (security §7.2).
pub const AUTHENTICATION_RESUME: u64 = 2;

/// `AUTH_FAILED`: Authentication or proof failed.
pub const ERROR_AUTH_FAILED: u64 = registry::error::AUTH_FAILED;
/// `UNSUPPORTED_VERSION`: Preface version unsupported.
pub const ERROR_UNSUPPORTED_VERSION: u64 = registry::error::UNSUPPORTED_VERSION;
/// `UNSUPPORTED_PROFILE`: Required profile or complete profile behavior unavailable.
pub const ERROR_UNSUPPORTED_PROFILE: u64 = registry::error::UNSUPPORTED_PROFILE;
/// `UNSUPPORTED_CONFIG`: Exact surface/track/device configuration unsupported.
pub const ERROR_UNSUPPORTED_CONFIG: u64 = registry::error::UNSUPPORTED_CONFIG;
/// `BAD_MESSAGE`: Structurally or semantically invalid message.
pub const ERROR_BAD_MESSAGE: u64 = registry::error::BAD_MESSAGE;
/// `BAD_STATE`: Valid message invalid in current state.
pub const ERROR_BAD_STATE: u64 = registry::error::BAD_STATE;
/// `DUPLICATE_ID`: Complete scoped identity already live.
pub const ERROR_DUPLICATE_ID: u64 = registry::error::DUPLICATE_ID;
/// `NOT_FOUND`: Object absent or outside caller authority.
pub const ERROR_NOT_FOUND: u64 = registry::error::NOT_FOUND;
/// `LIMIT_EXCEEDED`: Static count, size, or allocation limit exceeded.
pub const ERROR_LIMIT_EXCEEDED: u64 = registry::error::LIMIT_EXCEEDED;
/// `NO_MEMORY`: Bounded allocation failed after admission.
pub const ERROR_NO_MEMORY: u64 = registry::error::NO_MEMORY;
/// `FLOW_CONTROL`: Absolute byte/record allowance exceeded.
pub const ERROR_FLOW_CONTROL: u64 = registry::error::FLOW_CONTROL;
/// `HASH_MISMATCH`: Declared and computed image hash differ.
pub const ERROR_HASH_MISMATCH: u64 = registry::error::HASH_MISMATCH;
/// `NEED_KEYFRAME`: Random-access unit required.
pub const ERROR_NEED_KEYFRAME: u64 = registry::error::NEED_KEYFRAME;
/// `STALE_EPOCH`: Media epoch older than accepted epoch.
pub const ERROR_STALE_EPOCH: u64 = registry::error::STALE_EPOCH;
/// `STALE_TARGET_GENERATION`: Scene target generation mismatch.
pub const ERROR_STALE_TARGET_GENERATION: u64 = registry::error::STALE_TARGET_GENERATION;
/// `ANCHOR_INVALIDATED`: Terminal anchor no longer exists.
pub const ERROR_ANCHOR_INVALIDATED: u64 = registry::error::ANCHOR_INVALIDATED;
/// `AUTHORITY_REVOKED`: Context or lease authority ended.
pub const ERROR_AUTHORITY_REVOKED: u64 = registry::error::AUTHORITY_REVOKED;
/// `DECODER`: Decoder failed within a track boundary.
pub const ERROR_DECODER: u64 = registry::error::DECODER;
/// `DEVICE_LOST`: Required render or audio device lost.
pub const ERROR_DEVICE_LOST: u64 = registry::error::DEVICE_LOST;
/// `TIMEOUT`: Bounded deadline elapsed.
pub const ERROR_TIMEOUT: u64 = registry::error::TIMEOUT;
/// `PRECONDITION_FAILED`: Typed expected value did not match.
pub const ERROR_PRECONDITION_FAILED: u64 = registry::error::PRECONDITION_FAILED;
/// `ALREADY_APPLIED`: Mutation already applied but no replayable secret/result exists.
pub const ERROR_ALREADY_APPLIED: u64 = registry::error::ALREADY_APPLIED;
/// `NOT_VISIBLE`: Presentation condition cannot be satisfied at this hop.
pub const ERROR_NOT_VISIBLE: u64 = registry::error::NOT_VISIBLE;
/// `CANCELLED`: Pending work cancelled.
pub const ERROR_CANCELLED: u64 = registry::error::CANCELLED;
/// `UNKNOWN_OUTCOME`: Resume cannot prove a non-idempotent pre-loss outcome.
pub const ERROR_UNKNOWN_OUTCOME: u64 = registry::error::UNKNOWN_OUTCOME;
/// `CHANNEL_BUSY`: Another transport is active for the generation.
pub const ERROR_CHANNEL_BUSY: u64 = registry::error::CHANNEL_BUSY;
/// `STALE_CHANNEL_GENERATION`: Open or media record targets an old generation.
pub const ERROR_STALE_CHANNEL_GENERATION: u64 = registry::error::STALE_CHANNEL_GENERATION;
/// `LEASE_SUSPENDED`: Mutation is unavailable while logical session is suspended.
pub const ERROR_LEASE_SUSPENDED: u64 = registry::error::LEASE_SUSPENDED;
/// `RATE_LIMITED`: Contractual sustained rate exceeded.
pub const ERROR_RATE_LIMITED: u64 = registry::error::RATE_LIMITED;
/// `INTEGRITY_FAILED`: Authenticated channel or transcript integrity failed.
pub const ERROR_INTEGRITY_FAILED: u64 = registry::error::INTEGRITY_FAILED;

/// A control message that does not satisfy its Vivid 1.5 schema.
///
/// New variants may be added within 1.5 as the protocol registers new checks.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum MessageError {
    /// The body is not canonical Vivid CBOR.
    Decode(cbor::DecodeError),
    /// A value could not be encoded as canonical Vivid CBOR.
    Encode(cbor::EncodeError),
    /// A resource contract inside the message is invalid.
    Resource(ResourceError),
    /// The named schema's value is not a CBOR map.
    ExpectedMap(&'static str),
    /// A required key is absent.
    MissingKey {
        /// The record or map whose schema failed.
        schema: &'static str,
        /// The offending key.
        key: u64,
    },
    /// A key the schema does not define is present.
    UnknownKey {
        /// The record or map whose schema failed.
        schema: &'static str,
        /// The offending key.
        key: u64,
    },
    /// A key holds a value of the wrong CBOR type.
    WrongType {
        /// The record or map whose schema failed.
        schema: &'static str,
        /// The offending key.
        key: u64,
    },
    /// A value has the right type but violates the schema.
    InvalidValue {
        /// The record or map whose schema failed.
        schema: &'static str,
        /// The offending key.
        key: u64,
        /// What the value violates.
        reason: &'static str,
    },
    /// The record header's object ID differs from the ID the payload names.
    HeaderObjectMismatch {
        /// Object ID in the record header.
        header: u64,
        /// Object ID the payload names.
        payload: u64,
    },
}

impl fmt::Display for MessageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Decode(error) => write!(formatter, "invalid CBOR: {error}"),
            Self::Encode(error) => write!(formatter, "CBOR encoding failed: {error}"),
            Self::Resource(error) => write!(formatter, "invalid resource contract: {error}"),
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

impl std::error::Error for MessageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Decode(error) => Some(error),
            Self::Encode(error) => Some(error),
            Self::Resource(error) => Some(error),
            _ => None,
        }
    }
}

impl From<MessageError> for io::Error {
    fn from(value: MessageError) -> Self {
        io::Error::new(io::ErrorKind::InvalidData, value)
    }
}

impl From<cbor::EncodeError> for MessageError {
    fn from(value: cbor::EncodeError) -> Self {
        Self::Encode(value)
    }
}

impl From<cbor::DecodeError> for MessageError {
    fn from(value: cbor::DecodeError) -> Self {
        Self::Decode(value)
    }
}

impl From<ResourceError> for MessageError {
    fn from(value: ResourceError) -> Self {
        Self::Resource(value)
    }
}

/// A CBOR map with integer keys, in strictly increasing key order once encoded.
pub type PayloadMap = Vec<(u64, Value)>;

/// The deterministic CBOR value every control, lane, and track-control record carries (core §5).
#[derive(Debug, Clone, PartialEq, Eq, Zeroize)]
pub struct Envelope {
    /// Nonzero for a correlated request; zero for an unsolicited event (key 0).
    pub request_id: u64,
    /// Scene transaction the record belongs to (key 1).
    pub transaction_id: Option<u64>,
    /// Presentation-target generation the request expects (key 2).
    pub expected_target_generation: Option<u64>,
    /// The record's own payload (key 3).
    pub payload: PayloadMap,
    /// Typed preconditions checked before the record applies (key 4).
    pub preconditions: PayloadMap,
    /// Key that lets a retried mutation replay its first result (key 5).
    pub idempotency_key: Option<[u8; IDEMPOTENCY_KEY_BYTES]>,
    /// Non-secret ID that ties a record to its cause in traces (key 6).
    pub causation_id: Option<[u8; CAUSATION_ID_BYTES]>,
}

impl Envelope {
    /// Creates an envelope with only a request ID and payload.
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

    /// Builds a request envelope, which needs a nonzero request ID.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError::InvalidValue`] when `request_id` is zero.
    pub fn correlated(request_id: u64, payload: PayloadMap) -> Result<Self, MessageError> {
        if request_id == 0 {
            return Err(invalid("envelope", 0, "must be nonzero for a request"));
        }
        Ok(Self::new(request_id, payload))
    }

    /// Encodes the envelope as deterministic CBOR.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError`] when the payload or precondition keys are not strictly increasing,
    /// or when CBOR encoding fails.
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

    /// Checks that this envelope carries a request, which has a nonzero request ID.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError::InvalidValue`] when the request ID is zero.
    pub fn validate_request(&self) -> Result<(), MessageError> {
        if self.request_id == 0 {
            Err(invalid("envelope", 0, "must be nonzero for a request"))
        } else {
            Ok(())
        }
    }

    /// Checks that this envelope carries an unsolicited event, which has request ID zero.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError::InvalidValue`] when the request ID is nonzero.
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
///
/// # Errors
///
/// Returns [`MessageError`] when `body` is not canonical CBOR, is not an envelope map, contains an
/// unknown envelope key, has a field of the wrong type, or has payload or precondition keys that
/// are not strictly increasing.
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

/// Encodes `payload` in an envelope with `request_id`.
///
/// # Errors
///
/// Returns [`MessageError`] when the payload keys are not strictly increasing or CBOR encoding
/// fails.
pub fn encode_payload(request_id: u64, payload: PayloadMap) -> Result<Vec<u8>, MessageError> {
    Zeroizing::new(Envelope::new(request_id, payload)).encode()
}

/// Encodes an envelope with an empty payload, the body of `OK` and similar acknowledgements.
///
/// # Panics
///
/// Never in practice: an envelope with an empty payload always encodes.
pub fn empty(request_id: u64) -> Vec<u8> {
    encode_payload(request_id, vec![]).expect("empty payload is valid")
}

/// Encodes an `OK` reply body for `request_id`.
pub fn ok(request_id: u64) -> Vec<u8> {
    empty(request_id)
}

/// The authentication request in `HELLO` key 6.
#[non_exhaustive]
pub enum HelloAuthentication {
    /// Root authentication with a proof over the preface and `HELLO` (security §3.1).
    Root {
        /// HMAC proof over the preface and the `HELLO` without this field.
        proof: [u8; 32],
    },
    /// First activation of a session lease (security §6.3).
    LeaseActivation {
        /// The context that issued the lease.
        context_id: u64,
        /// The lease being used.
        lease_id: u64,
        /// The lease's activation secret.
        activation_secret: Secret32,
        /// Attempt ID that makes a retried establishment recognizable.
        attempt_id: [u8; ATTEMPT_ID_BYTES],
        /// Proof of possession for the lease's bound key, when it has one.
        proof_of_possession: Option<Vec<u8>>,
    },
    /// Resumption of a suspended session (security §7.2).
    Resume {
        /// The context that issued the lease.
        context_id: u64,
        /// The lease being used.
        lease_id: u64,
        /// The suspended session.
        session_id: u64,
        /// The lease's current resume generation.
        resume_generation: u64,
        /// Attempt ID that makes a retried establishment recognizable.
        attempt_id: [u8; ATTEMPT_ID_BYTES],
        /// HMAC proof over the preface and the `HELLO` without this field.
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
                    &proof_of_possession.as_ref().map(std::vec::Vec::len),
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
            Self::Root { proof } | Self::Resume { proof, .. } => proof.zeroize(),
            Self::LeaseActivation {
                proof_of_possession,
                ..
            } => {
                if let Some(proof) = proof_of_possession {
                    proof.zeroize();
                }
            }
        }
    }
}

impl HelloAuthentication {
    /// Returns the wire authentication kind, such as [`AUTHENTICATION_ROOT`].
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

/// The producer's `HELLO`, the first control record of a session (core §6.1).
///
/// # Examples
///
/// ```
/// use vivid_protocol::auth::Secret32;
/// use vivid_protocol::messages::{Hello, HelloAuthentication};
/// use vivid_protocol::registry::{CORE_CONTROL, DESKTOP_SURFACE};
/// use vivid_protocol::wire::{ConnectionKind, encode_preface};
///
/// let mut hello = Hello {
///     producer_name: "example".into(),
///     producer_version: "1.0".into(),
///     required_profiles: vec![DESKTOP_SURFACE.into(), CORE_CONTROL.into()],
///     optional_profiles: vec![],
///     maximum_control_body: vivid_protocol::CONTROL_MAX_RECORD_BODY,
///     client_nonce: [1; 32],
///     authentication: HelloAuthentication::Root { proof: [0; 32] },
///     target_profile: DESKTOP_SURFACE.into(),
///     extensions: vec![],
/// };
/// let preface = encode_preface(ConnectionKind::Control, vivid_protocol::CONTROL_MAX_RECORD_BODY);
/// hello.authenticate_root(&Secret32::new([7; 32]), &preface)?;
///
/// let body = hello.encode(1)?;
/// let (request_id, decoded) = Hello::decode(&body)?;
/// assert_eq!((request_id, decoded.target_profile.as_str()), (1, DESKTOP_SURFACE));
/// # Ok::<(), vivid_protocol::messages::MessageError>(())
/// ```
#[derive(Debug)]
pub struct Hello {
    /// Producer name, at most 256 UTF-8 bytes (key 0).
    pub producer_name: String,
    /// Producer version, at most 128 UTF-8 bytes (key 1).
    pub producer_version: String,
    /// Required profiles, sorted and unique (key 2).
    pub required_profiles: Vec<String>,
    /// Optional profiles, sorted and unique (key 3).
    pub optional_profiles: Vec<String>,
    /// Largest control-record body the producer accepts (key 4).
    pub maximum_control_body: u32,
    /// Fresh client nonce (key 5).
    pub client_nonce: [u8; NONCE_BYTES],
    /// Authentication request (key 6).
    pub authentication: HelloAuthentication,
    /// Requested presentation-target profile (key 7).
    pub target_profile: String,
    /// Canonical extension values. Keys 0 through 7 are reserved to the base schema.
    pub extensions: PayloadMap,
}

impl Hello {
    /// Signs this `HELLO` with root authentication over `preface` and the unauthenticated payload.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError::InvalidValue`] when the authentication is not root authentication,
    /// and any error from [`Hello::validate`].
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

    /// Signs this `HELLO` for a lease resume with the prior session's resume key.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError::InvalidValue`] when the authentication is not resume authentication,
    /// and any error from [`Hello::validate`].
    pub fn authenticate_resume(
        &mut self,
        prior_resume_key: &ResumeKey,
        preface: &[u8; 16],
    ) -> Result<(), MessageError> {
        let Some(binding) = self.resume_binding() else {
            return Err(invalid(
                "HELLO authentication",
                0,
                "is not resume authentication",
            ));
        };
        let authless = self.authless_payload()?;
        let proof = auth::resume_hello_proof(prior_resume_key, preface, &binding, &authless);
        if let HelloAuthentication::Resume {
            proof: stored_proof,
            ..
        } = &mut self.authentication
        {
            *stored_proof = proof;
        }
        Ok(())
    }

    /// Checks this resume `HELLO`'s proof against the prior session's resume key in constant time.
    ///
    /// Returns `false` for any other authentication kind.
    ///
    /// # Errors
    ///
    /// Returns any error from [`Hello::authless_payload`].
    pub fn verify_resume(
        &self,
        prior_resume_key: &ResumeKey,
        preface: &[u8; 16],
    ) -> Result<bool, MessageError> {
        let (Some(binding), HelloAuthentication::Resume { proof, .. }) =
            (self.resume_binding(), &self.authentication)
        else {
            return Ok(false);
        };
        let authless = self.authless_payload()?;
        let expected = auth::resume_hello_proof(prior_resume_key, preface, &binding, &authless);
        Ok(auth::verify_proof(&expected, proof))
    }

    fn resume_binding(&self) -> Option<ResumeBinding> {
        match &self.authentication {
            HelloAuthentication::Resume {
                lease_id,
                session_id,
                resume_generation,
                attempt_id,
                ..
            } => Some(ResumeBinding {
                lease_id: *lease_id,
                session_id: *session_id,
                resume_generation: *resume_generation,
                attempt_id: *attempt_id,
            }),
            _ => None,
        }
    }

    /// Checks every `HELLO` field against the core specification's limits.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError::InvalidValue`] when the producer name or version is too long, a
    /// profile list is unsorted, overlapping, unknown, or not closed under prerequisites, the
    /// control-body limit is zero or above [`crate::HARD_MAX_RECORD_BODY`], or an extension reuses
    /// a registered field.
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
            if let Some(prerequisites) = registry::prerequisites(profile)
                && prerequisites
                    .iter()
                    .any(|prerequisite| !offered.contains(prerequisite))
            {
                return Err(invalid("HELLO", 3, "is not prerequisite-closed"));
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

    /// Returns the complete `HELLO` payload, including the authentication proof.
    ///
    /// # Errors
    ///
    /// Returns any error from [`Hello::validate`].
    pub fn payload_value(&self) -> Result<Value, MessageError> {
        self.payload_value_inner(false)
    }

    /// Returns the `HELLO` payload with the authentication proof omitted, as the proof signs it.
    ///
    /// # Errors
    ///
    /// Returns any error from [`Hello::validate`].
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

    /// Encodes the proof-free `HELLO` payload that the authentication proof signs.
    ///
    /// # Errors
    ///
    /// Returns any error from [`Hello::validate`], or a CBOR encoding error.
    pub fn authless_payload(&self) -> Result<Vec<u8>, MessageError> {
        let value = Zeroizing::new(self.authless_payload_value()?);
        Ok(cbor::encode(&value)?)
    }

    /// Encodes this `HELLO` as a request envelope with `request_id`.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError`] when the payload fails [`Hello::validate`] or `request_id` is zero.
    pub fn encode(&self, request_id: u64) -> Result<Vec<u8>, MessageError> {
        let Value::Map(payload) = self.payload_value()? else {
            unreachable!("a HELLO payload is always a map")
        };
        let envelope = Zeroizing::new(Envelope::new(request_id, payload));
        envelope.validate_request()?;
        envelope.encode()
    }

    /// Decodes a `HELLO` request, returning its request ID and payload.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError`] when `body` is not a valid envelope, is not a request, lacks a
    /// `HELLO` field or has one of the wrong type, or fails [`Hello::validate`].
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

/// The authentication confirmation in `WELCOME` key 9.
#[derive(Clone, PartialEq, Eq)]
pub struct WelcomeAuthentication {
    /// Authentication kind, matching the `HELLO` request (subkey 0).
    pub kind: u64,
    /// Server confirmation the producer verifies before sending anything else (subkey 1).
    pub confirmation: [u8; 32],
    /// Lease state, or zero for root (subkey 2).
    pub lease_state: u64,
    /// Fresh (`0`) or exact replay (`1`) of an activation attempt (subkey 3).
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
    /// Checks the authentication kind, attempt status, and lease state for consistency.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError::InvalidValue`] for an unregistered authentication kind, an attempt
    /// status other than fresh or replayed, or a lease state that does not fit the authentication
    /// kind.
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

/// The presenter's `WELCOME` reply that establishes a session (core §6.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Welcome {
    /// Nonzero session ID (key 0).
    pub session_id: u64,
    /// Non-secret session tag; an identifier, never authority (key 1).
    pub session_tag: [u8; SESSION_TAG_BYTES],
    /// Root context visible to this principal (key 2).
    pub root_context_id: u64,
    /// Presentation-target generation (key 3).
    pub target_generation: u64,
    /// Selected presentation-target profile (key 4).
    pub target_profile: String,
    /// Target-profile descriptor (key 5).
    pub target_descriptor: PayloadMap,
    /// Accepted profiles, sorted and unique (key 6).
    pub accepted_profiles: Vec<String>,
    /// Largest control-record body the presenter accepts (key 7).
    pub maximum_control_body: u32,
    /// Fresh server nonce (key 8).
    pub server_nonce: [u8; NONCE_BYTES],
    /// Authentication confirmation and lease state (key 9).
    pub authentication: WelcomeAuthentication,
    /// Initial or resumed session revision (key 10).
    pub session_revision: u64,
    /// Current scene revision (key 11).
    pub scene_revision: u64,
    /// Effective resource contract (key 12).
    pub resource_contract: ResourceContract,
    /// New (`0`) or resumed (`1`) session (key 13).
    pub establishment_state: u64,
    /// Lease resume generation; zero for a non-resumable root session (key 14).
    pub resume_generation: u64,
    /// Canonical extension entries after key 14, such as the vector limits map at key 15.
    pub extensions: PayloadMap,
}

impl Welcome {
    /// Sets the `WELCOME` confirmation from the handshake key and the unconfirmed payload.
    ///
    /// # Errors
    ///
    /// Returns any error from [`Welcome::validate`].
    pub fn confirm(&mut self, prk: &HandshakePrk) -> Result<(), MessageError> {
        let unconfirmed = self.unconfirmed_payload()?;
        self.authentication.confirmation = auth::welcome_confirmation(prk, &unconfirmed);
        Ok(())
    }

    /// Checks every `WELCOME` field against the core specification's limits.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError::InvalidValue`] when the authentication fields are inconsistent, a
    /// session, root-context, or target ID is zero, the accepted profiles are invalid, negotiated
    /// vector limits are missing or invalid, or the control-body limit is out of range.
    pub fn validate(&self) -> Result<(), MessageError> {
        self.authentication.validate()?;
        nonzero("WELCOME", 0, self.session_id)?;
        nonzero("WELCOME", 2, self.root_context_id)?;
        nonzero("WELCOME", 3, self.target_generation)?;
        validate_profiles("WELCOME", 6, &self.accepted_profiles)?;
        if self
            .accepted_profiles
            .iter()
            .any(|profile| profile == registry::VECTOR_SCENE)
        {
            let limits = self
                .extensions
                .iter()
                .find(|(key, _)| *key == 15)
                .ok_or_else(|| {
                    invalid("WELCOME", 15, "vector-scene-v1 requires negotiated limits")
                })?;
            crate::vector::Limits::from_value(&limits.1)
                .map_err(|_| invalid("WELCOME", 15, "invalid vector limits"))?;
        }

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

    /// Encodes the `WELCOME` payload without its confirmation, as the confirmation signs it.
    ///
    /// # Errors
    ///
    /// Returns any error from [`Welcome::validate`], or a CBOR encoding error.
    pub fn unconfirmed_payload(&self) -> Result<Vec<u8>, MessageError> {
        let value = Zeroizing::new(self.payload_value_inner(true)?);
        Ok(cbor::encode(&value)?)
    }

    /// Encodes this `WELCOME` as a reply envelope with `request_id`.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError`] when the payload fails [`Welcome::validate`] or `request_id` is
    /// zero.
    pub fn encode(&self, request_id: u64) -> Result<Vec<u8>, MessageError> {
        let Value::Map(payload) = self.payload_value_inner(false)? else {
            unreachable!("a WELCOME payload is always a map")
        };
        let envelope = Zeroizing::new(Envelope::new(request_id, payload));
        envelope.validate_request()?;
        envelope.encode()
    }

    /// Decodes a `WELCOME` reply, returning its request ID and payload.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError`] when `body` is not a valid envelope, has a zero request ID, lacks a
    /// `WELCOME` field or has one of the wrong type, carries an invalid resource contract, or fails
    /// [`Welcome::validate`].
    pub fn decode(body: &[u8]) -> Result<(u64, Self), MessageError> {
        let mut envelope = Zeroizing::new(decode_control(body)?);
        envelope.validate_request()?;
        let value = Zeroizing::new(Value::Map(std::mem::take(&mut envelope.payload)));
        let welcome = {
            let map = StrictMap::preserving("WELCOME", &value, 14)?;
            let contract = ResourceContract::from_value(map.required(12)?)?;
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

/// The structured detail map in `ERROR` key 2 (core §11).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorDetail {
    /// Detail entries, such as limit identifier (0) or current session revision (3).
    pub fields: PayloadMap,
}

impl ErrorDetail {
    /// Builds error detail from `fields`, checking them first.
    ///
    /// # Errors
    ///
    /// Returns any error from [`ErrorDetail::validate`].
    pub fn new(fields: PayloadMap) -> Result<Self, MessageError> {
        let detail = Self { fields };
        detail.validate()?;
        Ok(detail)
    }

    /// Checks detail fields against the registered `ERROR` detail keys.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError`] when keys are not strictly increasing or exceed 18, a field has the
    /// wrong type, the idempotent-result field is unregistered, or the encoded detail exceeds 4,096
    /// bytes.
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

    /// Returns the `UNSUPPORTED_VERSION` detail naming Vivid 1.5 (keys 14 and 15).
    pub fn supported_version() -> Self {
        Self {
            fields: vec![
                (14, Value::Unsigned(u64::from(VIVID_MAJOR))),
                (15, Value::Unsigned(u64::from(VIVID_MINOR))),
            ],
        }
    }

    /// Returns the supported major and minor version (keys 14 and 15), if both are present.
    pub fn supported_version_tuple(&self) -> Option<(u64, u64)> {
        Some((
            self.fields.iter().find(|(key, _)| *key == 14)?.1.as_u64()?,
            self.fields.iter().find(|(key, _)| *key == 15)?.1.as_u64()?,
        ))
    }
}

/// A decoded `ERROR` reply (core §11).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorReply {
    /// Registered error code, such as [`ERROR_BAD_MESSAGE`] (key 0).
    pub code: u64,
    /// The failed request's ID (key 1).
    pub request_id: u64,
    /// Structured detail (key 2).
    pub detail: ErrorDetail,
    /// Whether the error ends the connection (key 3).
    pub fatal: bool,
    /// Untrusted human diagnostic, at most 4,096 UTF-8 bytes (key 4).
    pub diagnostic: String,
}

impl ErrorReply {
    /// Encodes this `ERROR` reply as deterministic CBOR.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError::InvalidValue`] for an unregistered error code or a diagnostic over
    /// 4,096 bytes, and any error from [`ErrorDetail::validate`].
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

/// Decodes an `ERROR` reply body.
///
/// # Errors
///
/// Returns [`MessageError`] when `body` is not a valid envelope, has an unknown or mistyped field,
/// carries an unregistered error code, names a failed request ID that differs from the envelope's,
/// or has invalid detail or an over-long diagnostic.
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

/// Encodes the fatal `UNSUPPORTED_VERSION` error body that names this crate's protocol version.
///
/// # Panics
///
/// Never in practice: every field is a fixed, valid value.
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

/// The quality-of-service lane a connection or track uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
#[non_exhaustive]
pub enum LaneClass {
    /// The control connection.
    Control = 0,
    /// The interactive lane that carries input.
    Interactive = 1,
    /// Latency-sensitive media.
    Realtime = 2,
    /// Throughput-oriented media and file transfer.
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

/// The `LANE_OPEN` record that authenticates an interactive lane (core §7).
#[derive(Clone, PartialEq, Eq)]
pub struct LaneOpen {
    /// The session that owns the channel key (key 0).
    pub session_id: u64,
    /// Lane generation, starting at one (key 2).
    pub lane_generation: u64,
    /// Fresh client nonce (key 3).
    pub client_nonce: [u8; 16],
    /// Truncated HMAC over the other fields (key 4).
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
    /// Sets `authentication_tag` from `session_channel_key` and every other field.
    ///
    /// Sign after the binding fields are final; any later field change invalidates the tag.
    pub fn sign(&mut self, session_channel_key: &ChannelKey) {
        self.authentication_tag = auth::lane_tag(session_channel_key, self);
    }

    /// Checks `authentication_tag` against `session_channel_key` in constant time.
    pub fn verify(&self, session_channel_key: &ChannelKey) -> bool {
        auth::verify_tag(
            &auth::lane_tag(session_channel_key, self),
            &self.authentication_tag,
        )
    }

    /// Encodes the `LANE_OPEN` payload; key 1 is always the interactive class.
    pub fn payload(&self) -> PayloadMap {
        vec![
            (0, Value::Unsigned(self.session_id)),
            (1, Value::Unsigned(LaneClass::Interactive as u64)),
            (2, Value::Unsigned(self.lane_generation)),
            (3, Value::Bytes(self.client_nonce.to_vec())),
            (4, Value::Bytes(self.authentication_tag.to_vec())),
        ]
    }

    /// Decodes a `LANE_OPEN` request.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError`] when `body` is not a valid request envelope, has an unknown,
    /// missing, or mistyped field, names a lane other than interactive, or has a zero session ID or
    /// lane generation.
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

/// The media a track carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
#[non_exhaustive]
pub enum TrackKind {
    /// Coded video access units.
    Video = 1,
    /// Coded audio packets.
    Audio = 2,
    /// Uncompressed or zstd-compressed RGBA frames.
    Raster = 3,
    /// Whole encoded images, such as PNG.
    EncodedImage = 4,
    /// Vector display lists for overlays.
    VectorScene = 5,
}

impl TryFrom<u64> for TrackKind {
    type Error = MessageError;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::Video),
            2 => Ok(Self::Audio),
            3 => Ok(Self::Raster),
            4 => Ok(Self::EncodedImage),
            5 => Ok(Self::VectorScene),
            _ => Err(invalid("track", 5, "has an unknown kind")),
        }
    }
}

/// The `CHANNEL_OPEN` record that authenticates a track connection (media §5.1).
#[derive(Clone, PartialEq, Eq)]
pub struct ChannelOpen {
    /// The session that owns the channel key (key 0).
    pub session_id: u64,
    /// Owning context (key 1).
    pub context_id: u64,
    /// Owning surface (key 2).
    pub surface_id: u64,
    /// The track; also the record object ID (key 3).
    pub track_id: u64,
    /// The channel generation this connection carries (key 4).
    pub channel_generation: u64,
    /// The track's kind (key 5).
    pub track_kind: TrackKind,
    /// The lane the connection uses (key 6).
    pub lane: LaneClass,
    /// Fresh client channel nonce (key 7).
    pub client_nonce: [u8; 16],
    /// Truncated HMAC over the other fields (key 8).
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
    /// Sets `authentication_tag` from `session_channel_key` and every other field.
    ///
    /// Sign after the binding fields are final; any later field change invalidates the tag.
    pub fn sign(&mut self, session_channel_key: &ChannelKey) {
        self.authentication_tag = auth::channel_tag(session_channel_key, self);
    }

    /// Checks `authentication_tag` against `session_channel_key` in constant time.
    pub fn verify(&self, session_channel_key: &ChannelKey) -> bool {
        auth::verify_tag(
            &auth::channel_tag(session_channel_key, self),
            &self.authentication_tag,
        )
    }

    /// Encodes the `CHANNEL_OPEN` payload.
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

    /// Decodes a `CHANNEL_OPEN` request whose record header named `header_object_id`.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError`] when `body` is not a valid request envelope, has an unknown,
    /// missing, or mistyped field, has a zero ID or generation, names an unregistered track kind or
    /// a lane other than realtime or bulk, or its track ID differs from `header_object_id`.
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

/// Checks that a record header and its payload name the same object.
///
/// # Errors
///
/// Returns [`MessageError::HeaderObjectMismatch`] when the two IDs differ.
pub fn validate_header_object(header_object_id: u64, payload_id: u64) -> Result<(), MessageError> {
    if header_object_id == payload_id {
        Ok(())
    } else {
        Err(MessageError::HeaderObjectMismatch {
            header: header_object_id,
            payload: payload_id,
        })
    }
}

/// A decoded CBOR map checked against one schema's known keys.
///
/// Debug output reports the schema and entry count, never decoded values that may carry proofs.
pub struct StrictMap<'a> {
    schema: &'static str,
    entries: &'a [(u64, Value)],
}

impl fmt::Debug for StrictMap<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StrictMap")
            .field("schema", &self.schema)
            .field("entries", &self.entries.len())
            .finish()
    }
}

impl<'a> StrictMap<'a> {
    /// Wraps `value` as a strict map that may only contain the `allowed` keys.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError::ExpectedMap`] when `value` is not a map, and
    /// [`MessageError::UnknownKey`] when it holds a key outside `allowed`.
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

    /// Wraps `value` as a map that requires keys `0..=last_known_key` and keeps any later keys.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError::ExpectedMap`] when `value` is not a map, and
    /// [`MessageError::MissingKey`] when a key up to `last_known_key` is absent.
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

    /// Returns the value stored under `key`.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError::MissingKey`] when `key` is absent.
    pub fn required(&self, key: u64) -> Result<&'a Value, MessageError> {
        self.entries
            .iter()
            .find_map(|(entry_key, value)| (*entry_key == key).then_some(value))
            .ok_or(MessageError::MissingKey {
                schema: self.schema,
                key,
            })
    }

    /// Returns the value at `key`, if present.
    pub fn optional(&self, key: u64) -> Option<&'a Value> {
        self.entries
            .iter()
            .find_map(|(entry_key, value)| (*entry_key == key).then_some(value))
    }

    /// Returns the unsigned integer stored under `key`.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError::MissingKey`] when `key` is absent and [`MessageError::WrongType`]
    /// when its value has another type.
    pub fn required_u64(&self, key: u64) -> Result<u64, MessageError> {
        self.required(key)?.as_u64().ok_or(MessageError::WrongType {
            schema: self.schema,
            key,
        })
    }

    /// Returns the unsigned integer stored under `key` as a `u32`.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError::MissingKey`] when `key` is absent, [`MessageError::WrongType`] when
    /// the value is not an unsigned integer, and [`MessageError::InvalidValue`] when it does not
    /// fit in `u32`.
    pub fn required_u32(&self, key: u64) -> Result<u32, MessageError> {
        u32::try_from(self.required_u64(key)?).map_err(|_| MessageError::InvalidValue {
            schema: self.schema,
            key,
            reason: "does not fit in u32",
        })
    }

    /// Returns the unsigned integer stored under `key`, if present.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError::WrongType`] when the value is present but is not an unsigned
    /// integer.
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

    /// Returns the boolean stored under `key`.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError::MissingKey`] when `key` is absent and [`MessageError::WrongType`]
    /// when its value has another type.
    pub fn required_bool(&self, key: u64) -> Result<bool, MessageError> {
        self.required(key)?
            .as_bool()
            .ok_or(MessageError::WrongType {
                schema: self.schema,
                key,
            })
    }

    /// Returns the text stored under `key`.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError::MissingKey`] when `key` is absent and [`MessageError::WrongType`]
    /// when its value has another type.
    pub fn required_text(&self, key: u64) -> Result<&'a str, MessageError> {
        self.required(key)?
            .as_text()
            .ok_or(MessageError::WrongType {
                schema: self.schema,
                key,
            })
    }

    /// Returns the text stored under `key`, if present.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError::WrongType`] when the value is present but is not text.
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

    /// Returns the byte string stored under `key`.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError::MissingKey`] when `key` is absent and [`MessageError::WrongType`]
    /// when its value has another type.
    pub fn required_bytes(&self, key: u64) -> Result<&'a [u8], MessageError> {
        self.required(key)?
            .as_bytes()
            .ok_or(MessageError::WrongType {
                schema: self.schema,
                key,
            })
    }

    /// Returns the byte string stored under `key`, if present.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError::WrongType`] when the value is present but is not a byte string.
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

    /// Returns the byte string stored under `key` as an array of exactly `N` bytes.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError::MissingKey`] when `key` is absent, [`MessageError::WrongType`] when
    /// the value is not a byte string, and [`MessageError::InvalidValue`] when its length is not
    /// `N`.
    pub fn required_fixed_bytes<const N: usize>(&self, key: u64) -> Result<[u8; N], MessageError> {
        self.required_bytes(key)?
            .try_into()
            .map_err(|_| MessageError::InvalidValue {
                schema: self.schema,
                key,
                reason: "has the wrong byte length",
            })
    }

    /// Returns the byte string at `key` as exactly `N` bytes, if present.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError::WrongType`] when the value is present but is not a byte string, and
    /// [`MessageError::InvalidValue`] when its length is not `N`.
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

    /// Returns the map stored under `key`.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError::MissingKey`] when `key` is absent and [`MessageError::WrongType`]
    /// when its value has another type.
    pub fn required_map(&self, key: u64) -> Result<&'a [(u64, Value)], MessageError> {
        self.required(key)?.as_map().ok_or(MessageError::WrongType {
            schema: self.schema,
            key,
        })
    }

    /// Returns the map stored under `key`, if present.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError::WrongType`] when the value is present but is not a map.
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

    /// Returns the array of text values stored under `key`.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError::MissingKey`] when `key` is absent, and [`MessageError::WrongType`]
    /// when the value is not an array or holds a non-text element.
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

    /// Returns the entries whose keys follow `last_known_key`, for preserving schemas.
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

/// Builds a [`MessageError::InvalidValue`] for `schema` at `key`.
pub fn invalid_value(schema: &'static str, key: u64, reason: &'static str) -> MessageError {
    invalid(schema, key, reason)
}

/// Returns `value` when it is nonzero, for codecs outside this module.
///
/// # Errors
///
/// Returns [`MessageError::InvalidValue`] naming `schema` and `key` when `value` is zero.
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

/// The `CANCEL_WAIT` payload: abandon a pending wait (core §10).
///
/// Cancelling a wait that already completed is not an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CancelWait {
    /// The request ID of the wait to cancel.
    pub request_id: u64,
}

impl CancelWait {
    /// Encodes the payload map.
    pub fn payload(&self) -> PayloadMap {
        vec![(0, Value::Unsigned(self.request_id))]
    }

    /// Decodes a `CANCEL_WAIT` payload.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError`] when `value` is not a strict `CANCEL_WAIT` map or the request ID is
    /// missing or not an unsigned integer.
    pub fn decode(value: &Value) -> Result<Self, MessageError> {
        let map = StrictMap::new("CANCEL_WAIT", value, &[0])?;
        Ok(Self {
            request_id: map.required_u64(0)?,
        })
    }
}
