//! Finite resource contracts and channel-local absolute flow accounting.

use std::{fmt, time::Duration};

use crate::cbor::Value;

pub const RESOURCE_COUNT: usize = 33;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
pub enum Resource {
    Surfaces = 0,
    Tracks = 1,
    Nodes = 2,
    VideoTracks = 3,
    AudioTracks = 4,
    RasterTracks = 5,
    ImageTracks = 6,
    DecoderInstances = 7,
    CodedPixelsPerTrack = 8,
    DecodedPixelsPerSecond = 9,
    EncodedBitsPerSecond = 10,
    MediaRecordsPerSecond = 11,
    AudioSampleRate = 12,
    AudioChannelsPerTrack = 13,
    InflightMediaBytes = 14,
    TrackConnections = 15,
    RetainedPixels = 16,
    MediaRecordBody = 17,
    ControlRecordBody = 18,
    PendingRequests = 19,
    RegisteredWaits = 20,
    IdempotencyEntries = 21,
    ChildSessionLeases = 22,
    DisconnectGraceUs = 23,
    InputEventsPerSecond = 24,
    ObservationQueueEntries = 25,
    ImageCacheBytes = 26,
    OpenSceneTransactions = 27,
    ChildContexts = 28,
    SuspendedChildSessions = 29,
    PendingChannelOpenAttempts = 30,
    ActiveTerminalAnchors = 31,
    SeenTerminalAnchorIds = 32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceError {
    NotAMap,
    MissingKey(u64),
    UnknownKey(u64),
    InvalidValue(u64),
    Overflow,
    ExceedsContract(Resource),
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceContract {
    values: [u64; RESOURCE_COUNT],
}

impl ResourceContract {
    pub const fn new(values: [u64; RESOURCE_COUNT]) -> Self {
        Self { values }
    }

    pub const fn denied() -> Self {
        Self::new([0; RESOURCE_COUNT])
    }

    pub const fn get(&self, resource: Resource) -> u64 {
        self.values[resource as usize]
    }

    pub fn set(&mut self, resource: Resource, value: u64) {
        self.values[resource as usize] = value;
    }

    pub fn component_min(&self, other: &Self) -> Self {
        let mut values = [0; RESOURCE_COUNT];
        for (index, value) in values.iter_mut().enumerate() {
            *value = self.values[index].min(other.values[index]);
        }
        Self::new(values)
    }

    pub fn to_value(&self) -> Value {
        Value::Map(
            self.values
                .iter()
                .enumerate()
                .map(|(key, value)| (key as u64, Value::Unsigned(*value)))
                .collect(),
        )
    }

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

#[derive(Debug, Clone)]
pub struct ReservationLedger {
    capacity: ResourceContract,
    reserved: [u64; RESOURCE_COUNT],
}

impl ReservationLedger {
    pub fn new(capacity: ResourceContract) -> Self {
        Self {
            capacity,
            reserved: [0; RESOURCE_COUNT],
        }
    }

    pub fn available(&self) -> ResourceContract {
        let mut values = [0; RESOURCE_COUNT];
        for (index, value) in values.iter_mut().enumerate() {
            *value = self.capacity.values[index] - self.reserved[index];
        }
        ResourceContract::new(values)
    }

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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ChannelFlow {
    pub sent_body_bytes: u64,
    pub sent_media_records: u64,
    pub maximum_body_bytes: u64,
    pub maximum_media_records: u64,
}

impl ChannelFlow {
    pub fn new(maximum_body_bytes: u64, maximum_media_records: u64) -> Self {
        Self {
            maximum_body_bytes,
            maximum_media_records,
            ..Self::default()
        }
    }

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

    pub fn raise_maxima(&mut self, body_bytes: u64, records: u64) {
        self.maximum_body_bytes = self.maximum_body_bytes.max(body_bytes);
        self.maximum_media_records = self.maximum_media_records.max(records);
    }
}

#[derive(Debug, Clone)]
pub struct TokenBucket {
    rate_per_second: u64,
    capacity: u64,
    tokens: u64,
    remainder_nanos: u64,
}

impl TokenBucket {
    pub fn new(rate_per_second: u64, maximum_record_charge: u64) -> Self {
        let capacity = rate_per_second.max(maximum_record_charge);
        Self {
            rate_per_second,
            capacity,
            tokens: capacity,
            remainder_nanos: 0,
        }
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_bucket_reports_the_exact_wait_for_a_charge_it_cannot_yet_admit() {
        let mut bucket = TokenBucket::new(1_000, 1_000);
        assert_eq!(
            bucket.time_until(1_000),
            Ok(None),
            "a full bucket admits its capacity now"
        );
        bucket.charge(1_000).unwrap();
        // Empty, earning 1000 tokens per second: 250 tokens is a quarter of a second.
        assert_eq!(bucket.time_until(250), Ok(Some(Duration::from_millis(250))));
        // Waiting exactly that long must be enough, not one token short.
        bucket.replenish(Duration::from_millis(250)).unwrap();
        assert_eq!(bucket.time_until(250), Ok(None));
        assert!(bucket.charge(250).is_ok());
    }

    #[test]
    fn a_charge_larger_than_the_bucket_is_refused_rather_than_waited_out() {
        let bucket = TokenBucket::new(1_000, 1_000);
        assert!(bucket.time_until(1_001).is_err());
        // A bucket that earns nothing can never admit a charge it does not already hold.
        let mut idle = TokenBucket::new(0, 10);
        assert_eq!(idle.time_until(10), Ok(None));
        idle.charge(10).unwrap();
        assert!(idle.time_until(1).is_err());
    }

    #[test]
    fn contract_requires_all_fields() {
        let value = ResourceContract::denied().to_value();
        assert_eq!(
            ResourceContract::from_value(&value).unwrap(),
            ResourceContract::denied()
        );
        assert!(matches!(
            ResourceContract::from_value(&Value::Map(vec![])),
            Err(ResourceError::MissingKey(0))
        ));
    }

    #[test]
    fn flow_is_absolute_and_lower_updates_are_ignored() {
        let mut flow = ChannelFlow::new(10, 2);
        flow.admit(7).unwrap();
        assert_eq!(flow.admit(4), Err(ResourceError::FlowControl));
        flow.raise_maxima(20, 3);
        flow.admit(4).unwrap();
        flow.raise_maxima(1, 1);
        assert_eq!(flow.maximum_body_bytes, 20);
        assert_eq!(flow.maximum_media_records, 3);
    }
}
