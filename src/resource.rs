//! Finite resource contracts, reservations, and flow accounting (security §4 and §5).
//!
//! A [`ResourceContract`] bounds every [`Resource`] a context may use; zero denies it, and there is
//! no unlimited value. A [`ReservationLedger`] reserves contracts against a fixed capacity and
//! releases them. [`ChannelFlow`] enforces a channel's absolute byte and record allowance, and
//! [`TokenBucket`] enforces a sustained rate.

use std::{fmt, time::Duration};

use crate::cbor::Value;

/// Number of entries in a resource contract.
pub const RESOURCE_COUNT: usize = 33;

/// One entry of a resource contract; the discriminant is its contract key (security §4.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
#[non_exhaustive]
pub enum Resource {
    /// Maximum surfaces (key 0).
    Surfaces = 0,
    /// Maximum tracks, aggregate (key 1).
    Tracks = 1,
    /// Maximum nodes (key 2).
    Nodes = 2,
    /// Maximum video tracks (key 3).
    VideoTracks = 3,
    /// Maximum audio tracks (key 4).
    AudioTracks = 4,
    /// Maximum raster tracks (key 5).
    RasterTracks = 5,
    /// Maximum encoded-image tracks (key 6).
    ImageTracks = 6,
    /// Maximum decoder instances (key 7).
    DecoderInstances = 7,
    /// Maximum coded pixels per video/raster track (key 8).
    CodedPixelsPerTrack = 8,
    /// Maximum reserved decoded pixels per second (key 9).
    DecodedPixelsPerSecond = 9,
    /// Maximum reserved encoded bits per second (key 10).
    EncodedBitsPerSecond = 10,
    /// Maximum reserved media records per second (key 11).
    MediaRecordsPerSecond = 11,
    /// Maximum audio sample rate per track (key 12).
    AudioSampleRate = 12,
    /// Maximum audio channels per track (key 13).
    AudioChannelsPerTrack = 13,
    /// Maximum aggregate in-flight media body bytes (key 14).
    InflightMediaBytes = 14,
    /// Maximum concurrent track connections (key 15).
    TrackConnections = 15,
    /// Maximum retained decoded/poster pixels (key 16).
    RetainedPixels = 16,
    /// Maximum media-record body (key 17).
    MediaRecordBody = 17,
    /// Maximum control-record body (key 18).
    ControlRecordBody = 18,
    /// Maximum pending correlated requests (key 19).
    PendingRequests = 19,
    /// Maximum registered waits (key 20).
    RegisteredWaits = 20,
    /// Maximum idempotency entries (key 21).
    IdempotencyEntries = 21,
    /// Maximum child session leases (key 22).
    ChildSessionLeases = 22,
    /// Maximum disconnect grace in microseconds (key 23).
    DisconnectGraceUs = 23,
    /// Maximum input events per second (key 24).
    InputEventsPerSecond = 24,
    /// Maximum observation-queue entries (key 25).
    ObservationQueueEntries = 25,
    /// Maximum encoded-image cache bytes (key 26).
    ImageCacheBytes = 26,
    /// Maximum open scene transactions (key 27).
    OpenSceneTransactions = 27,
    /// Maximum child contexts (key 28).
    ChildContexts = 28,
    /// Maximum suspended child sessions (key 29).
    SuspendedChildSessions = 29,
    /// Maximum pending channel-open attempts (key 30).
    PendingChannelOpenAttempts = 30,
    /// Maximum active terminal anchors (key 31).
    ActiveTerminalAnchors = 31,
    /// Maximum seen terminal anchor IDs (key 32).
    SeenTerminalAnchorIds = 32,
}

/// A resource contract or accounting rule a value broke.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ResourceError {
    /// The contract is not a CBOR map.
    NotAMap,
    /// The contract omits this key.
    MissingKey(u64),
    /// The contract has a key that is not assigned.
    UnknownKey(u64),
    /// The value at this key is not an unsigned integer.
    InvalidValue(u64),
    /// Checked arithmetic overflowed.
    Overflow,
    /// The request exceeds what the contract still allows for this resource.
    ExceedsContract(Resource),
    /// Traffic exceeds the channel's absolute allowance.
    FlowControl,
}

impl fmt::Display for ResourceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAMap => formatter.write_str("resource contract is not a map"),
            Self::MissingKey(key) => write!(formatter, "resource contract omits key {key}"),
            Self::UnknownKey(key) => write!(formatter, "resource contract has unknown key {key}"),
            Self::InvalidValue(key) => {
                write!(
                    formatter,
                    "resource contract key {key} is not an unsigned integer"
                )
            }
            Self::Overflow => formatter.write_str("resource accounting overflow"),
            Self::ExceedsContract(resource) => {
                write!(
                    formatter,
                    "resource request exceeds contract for {resource:?}"
                )
            }
            Self::FlowControl => formatter.write_str("absolute channel allowance exceeded"),
        }
    }
}

impl std::error::Error for ResourceError {}

/// Finite limits on every resource a context may use; zero denies that resource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceContract {
    values: [u64; RESOURCE_COUNT],
}

impl ResourceContract {
    /// Creates a contract from all values in key order.
    pub const fn new(values: [u64; RESOURCE_COUNT]) -> Self {
        Self { values }
    }

    /// Returns a contract that denies every resource.
    pub const fn denied() -> Self {
        Self::new([0; RESOURCE_COUNT])
    }

    /// Returns the limit for `resource`.
    pub const fn get(&self, resource: Resource) -> u64 {
        self.values[resource as usize]
    }

    /// Sets the limit for `resource`.
    pub fn set(&mut self, resource: Resource, value: u64) {
        self.values[resource as usize] = value;
    }

    /// Returns the smaller of each pair of limits.
    #[must_use]
    pub fn component_min(&self, other: &Self) -> Self {
        let mut values = [0; RESOURCE_COUNT];
        for (index, value) in values.iter_mut().enumerate() {
            *value = self.values[index].min(other.values[index]);
        }
        Self::new(values)
    }

    /// Encodes the contract as its CBOR map, with every key present.
    pub fn to_value(&self) -> Value {
        Value::Map(
            self.values
                .iter()
                .enumerate()
                .map(|(key, value)| (key as u64, Value::Unsigned(*value)))
                .collect(),
        )
    }

    /// Decodes a resource contract from its CBOR map of every registered resource.
    ///
    /// # Errors
    ///
    /// Returns [`ResourceError::NotAMap`] when `value` is not a map, [`ResourceError::UnknownKey`]
    /// for a key outside the registry, [`ResourceError::InvalidValue`] for a non-integer amount,
    /// and [`ResourceError::MissingKey`] when a registered resource is absent.
    pub fn from_value(value: &Value) -> Result<Self, ResourceError> {
        let Value::Map(entries) = value else {
            return Err(ResourceError::NotAMap);
        };
        let mut values = [0; RESOURCE_COUNT];
        let mut seen = [false; RESOURCE_COUNT];
        for (key, value) in entries {
            let index = usize::try_from(*key).map_err(|_| ResourceError::UnknownKey(*key))?;
            if index >= RESOURCE_COUNT {
                return Err(ResourceError::UnknownKey(*key));
            }
            values[index] = value.as_u64().ok_or(ResourceError::InvalidValue(*key))?;
            seen[index] = true;
        }
        if let Some(index) = seen.iter().position(|present| !present) {
            return Err(ResourceError::MissingKey(index as u64));
        }
        Ok(Self::new(values))
    }
}

/// Static reservations made against a fixed capacity.
#[derive(Debug, Clone)]
pub struct ReservationLedger {
    capacity: ResourceContract,
    reserved: [u64; RESOURCE_COUNT],
}

impl ReservationLedger {
    /// Creates a ledger with nothing reserved.
    pub fn new(capacity: ResourceContract) -> Self {
        Self {
            capacity,
            reserved: [0; RESOURCE_COUNT],
        }
    }

    /// Returns what remains of each resource.
    pub fn available(&self) -> ResourceContract {
        let mut values = [0; RESOURCE_COUNT];
        for (index, value) in values.iter_mut().enumerate() {
            *value = self.capacity.values[index] - self.reserved[index];
        }
        ResourceContract::new(values)
    }

    /// Reserves `requested`, clamped to `policy` and to what remains, returning the granted
    /// amounts.
    ///
    /// # Errors
    ///
    /// Returns [`ResourceError::Overflow`] when a reserved total would overflow.
    pub fn reserve(
        &mut self,
        requested: &ResourceContract,
        policy: &ResourceContract,
    ) -> Result<ResourceContract, ResourceError> {
        let effective = requested
            .component_min(policy)
            .component_min(&self.available());
        let mut reserved = self.reserved;
        for (index, amount) in effective.values.iter().copied().enumerate() {
            reserved[index] = reserved[index]
                .checked_add(amount)
                .ok_or(ResourceError::Overflow)?;
        }
        self.reserved = reserved;
        Ok(effective)
    }

    /// Returns `contract`'s amounts to the ledger.
    ///
    /// # Errors
    ///
    /// Returns [`ResourceError::Overflow`] when `contract` exceeds what is reserved; the ledger is
    /// unchanged.
    pub fn release(&mut self, contract: &ResourceContract) -> Result<(), ResourceError> {
        let mut reserved = self.reserved;
        for (index, amount) in contract.values.iter().copied().enumerate() {
            reserved[index] = reserved[index]
                .checked_sub(amount)
                .ok_or(ResourceError::Overflow)?;
        }
        self.reserved = reserved;
        Ok(())
    }
}

/// Absolute, cumulative flow accounting for one channel generation (media §6).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ChannelFlow {
    /// Media body bytes sent so far.
    pub sent_body_bytes: u64,
    /// Media records sent so far.
    pub sent_media_records: u64,
    /// Cumulative body bytes the receiver allows.
    pub maximum_body_bytes: u64,
    /// Cumulative media records the receiver allows.
    pub maximum_media_records: u64,
}

impl ChannelFlow {
    /// Creates accounting with nothing sent and the given initial allowance.
    pub fn new(maximum_body_bytes: u64, maximum_media_records: u64) -> Self {
        Self {
            maximum_body_bytes,
            maximum_media_records,
            ..Self::default()
        }
    }

    /// Admits one record of `body_length` bytes against the channel's absolute allowance.
    ///
    /// # Errors
    ///
    /// Returns [`ResourceError::Overflow`] when a counter would overflow, and
    /// [`ResourceError::FlowControl`] when the record exceeds the allowance.
    pub fn admit(&mut self, body_length: u32) -> Result<(), ResourceError> {
        let body_bytes = self
            .sent_body_bytes
            .checked_add(u64::from(body_length))
            .ok_or(ResourceError::Overflow)?;
        let records = self
            .sent_media_records
            .checked_add(1)
            .ok_or(ResourceError::Overflow)?;
        if body_bytes > self.maximum_body_bytes || records > self.maximum_media_records {
            return Err(ResourceError::FlowControl);
        }
        self.sent_body_bytes = body_bytes;
        self.sent_media_records = records;
        Ok(())
    }

    /// Raises the allowance; lower values, such as reordered updates, are ignored.
    pub fn raise_maxima(&mut self, body_bytes: u64, records: u64) {
        self.maximum_body_bytes = self.maximum_body_bytes.max(body_bytes);
        self.maximum_media_records = self.maximum_media_records.max(records);
    }
}

/// Enforces a sustained per-second rate with a burst of one second.
#[derive(Debug, Clone)]
pub struct TokenBucket {
    rate_per_second: u64,
    capacity: u64,
    tokens: u64,
    remainder_nanos: u64,
}

impl TokenBucket {
    /// Creates a full bucket for `rate_per_second`, holding at least one `maximum_record_charge`.
    pub fn new(rate_per_second: u64, maximum_record_charge: u64) -> Self {
        let capacity = rate_per_second.max(maximum_record_charge);
        Self {
            rate_per_second,
            capacity,
            tokens: capacity,
            remainder_nanos: 0,
        }
    }

    /// Adds the tokens earned over `elapsed`, up to the bucket capacity.
    ///
    /// # Errors
    ///
    /// Returns [`ResourceError::Overflow`] when the earned amount overflows.
    pub fn replenish(&mut self, elapsed: Duration) -> Result<(), ResourceError> {
        let nanos = elapsed.as_nanos();
        let earned_numerator = u128::from(self.rate_per_second)
            .checked_mul(nanos)
            .and_then(|value| value.checked_add(u128::from(self.remainder_nanos)))
            .ok_or(ResourceError::Overflow)?;
        let earned = earned_numerator / 1_000_000_000;
        self.remainder_nanos = (earned_numerator % 1_000_000_000) as u64;
        let earned = u64::try_from(earned).map_err(|_| ResourceError::Overflow)?;
        let missing = self.capacity - self.tokens;
        self.tokens = if earned >= missing {
            self.capacity
        } else {
            self.tokens
                .checked_add(earned)
                .ok_or(ResourceError::Overflow)?
        };
        Ok(())
    }

    /// How long until `units` can be charged.
    ///
    /// `Ok(None)` means now. `Err` means never: a charge larger than the bucket can ever hold is
    /// something the caller has to reject, not wait out — waiting would be an unbounded stall on
    /// whatever thread is shaping the stream.
    ///
    /// # Errors
    ///
    /// Returns [`ResourceError::ExceedsContract`] when `units` exceeds the capacity or the rate is
    /// zero, and [`ResourceError::Overflow`] when the wait cannot be represented.
    pub fn time_until(&self, units: u64) -> Result<Option<Duration>, ResourceError> {
        if units <= self.tokens {
            return Ok(None);
        }
        if units > self.capacity || self.rate_per_second == 0 {
            return Err(ResourceError::ExceedsContract(
                Resource::MediaRecordsPerSecond,
            ));
        }
        let deficit = u128::from(units - self.tokens);
        // Tokens already part-earned since the last replenish count toward the deficit.
        let owed_nanos = deficit
            .checked_mul(1_000_000_000)
            .ok_or(ResourceError::Overflow)?
            .saturating_sub(u128::from(self.remainder_nanos));
        let rate = u128::from(self.rate_per_second);
        // Round up: waiting the floor would wake with the charge still one token short.
        let nanos = owed_nanos.div_ceil(rate);
        Ok(Some(Duration::from_nanos(
            u64::try_from(nanos).map_err(|_| ResourceError::Overflow)?,
        )))
    }

    /// Spends `units` tokens.
    ///
    /// # Errors
    ///
    /// Returns [`ResourceError::ExceedsContract`] when fewer than `units` tokens are available; the
    /// bucket is unchanged.
    pub fn charge(&mut self, units: u64) -> Result<(), ResourceError> {
        self.tokens = self
            .tokens
            .checked_sub(units)
            .ok_or(ResourceError::ExceedsContract(
                Resource::MediaRecordsPerSecond,
            ))?;
        Ok(())
    }
}
