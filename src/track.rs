//! Immutable track configuration, channel generations, and readiness state.

use crate::{
    HARD_MAX_RECORD_BODY,
    cbor::Value,
    media,
    messages::{
        LaneClass, MessageError, PayloadMap, StrictMap, TrackKind, invalid_value, require_nonzero,
        validate_header_object,
    },
    resource::{ChannelFlow, ResourceError},
    revision::{ChannelGeneration, TrackRevision},
};
use sha2::{Digest, Sha256};

pub const MILESTONE_CHANNEL_ACCEPTED: u64 = 1 << 0;
pub const MILESTONE_FIRST_MEDIA: u64 = 1 << 1;
pub const MILESTONE_DECODER_INITIALIZED: u64 = 1 << 2;
pub const MILESTONE_RANDOM_ACCESS: u64 = 1 << 3;
pub const MILESTONE_OUTPUT_READY: u64 = 1 << 4;
pub const MILESTONE_PRESENTED: u64 = 1 << 5;
pub const MILESTONE_CLOCK_STARTED: u64 = 1 << 6;
pub const MILESTONE_EOS_ACCEPTED: u64 = 1 << 7;
pub const MILESTONE_BUFFERED_ENDED: u64 = 1 << 8;
pub const MILESTONE_CHANNEL_DETACHED: u64 = 1 << 9;
pub const MILESTONE_TRACK_LOST: u64 = 1 << 10;
pub const MILESTONE_KNOWN_MASK: u64 = (1 << 11) - 1;

/// Unsigned Q32.32 linear amplitude used by `audio-gain-v1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct AudioGain(u64);

impl AudioGain {
    pub const SILENT: Self = Self(0);
    pub const UNITY: Self = Self(1_u64 << 32);
    pub const MAX: Self = Self(2_u64 << 32);

    pub const fn new(raw: u64) -> Option<Self> {
        if raw <= Self::MAX.0 {
            Some(Self(raw))
        } else {
            None
        }
    }

    pub const fn raw(self) -> u64 {
        self.0
    }

    pub fn from_percent(percent: u32) -> Option<Self> {
        if percent > 200 {
            return None;
        }
        Some(Self((u64::from(percent) << 32) / 100))
    }

    pub fn as_f32(self) -> f32 {
        self.0 as f32 / (1_u64 << 32) as f32
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AcceptedOpen {
    generation: ChannelGeneration,
    nonce: [u8; 16],
    open_hash: [u8; 32],
    acceptance: PayloadMap,
    transport_live: bool,
    media_admitted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChannelOpenDecision {
    Fresh(PayloadMap),
    ExactReplay(PayloadMap),
    Busy,
    DifferentBytes,
    StaleGeneration,
}

#[derive(Debug, Default)]
pub struct ChannelOpenState {
    accepted: Option<AcceptedOpen>,
}

impl ChannelOpenState {
    pub fn open(
        &mut self,
        current_generation: ChannelGeneration,
        generation: ChannelGeneration,
        nonce: [u8; 16],
        complete_open_bytes: &[u8],
        acceptance: PayloadMap,
    ) -> ChannelOpenDecision {
        if generation != current_generation {
            return ChannelOpenDecision::StaleGeneration;
        }
        let open_hash: [u8; 32] = Sha256::digest(complete_open_bytes).into();
        if let Some(accepted) = &mut self.accepted {
            if accepted.generation != generation {
                self.accepted = None;
            } else if accepted.nonce != nonce || accepted.open_hash != open_hash {
                return ChannelOpenDecision::DifferentBytes;
            } else if accepted.transport_live {
                return ChannelOpenDecision::Busy;
            } else if accepted.media_admitted {
                return ChannelOpenDecision::StaleGeneration;
            } else {
                accepted.transport_live = true;
                return ChannelOpenDecision::ExactReplay(accepted.acceptance.clone());
            }
        }
        self.accepted = Some(AcceptedOpen {
            generation,
            nonce,
            open_hash,
            acceptance: acceptance.clone(),
            transport_live: true,
            media_admitted: false,
        });
        ChannelOpenDecision::Fresh(acceptance)
    }

    pub fn admit_media(&mut self, generation: ChannelGeneration) -> Result<(), MessageError> {
        let accepted = self
            .accepted
            .as_mut()
            .ok_or_else(|| invalid_value("media record", 0, "has no accepted channel"))?;
        if accepted.generation != generation || !accepted.transport_live {
            return Err(invalid_value(
                "media record",
                0,
                "does not use the accepted live channel",
            ));
        }
        accepted.media_admitted = true;
        Ok(())
    }

    pub fn transport_lost(&mut self, generation: ChannelGeneration) {
        if let Some(accepted) = &mut self.accepted {
            if accepted.generation == generation {
                accepted.transport_live = false;
            }
        }
    }

    pub fn advance(&mut self) {
        self.accepted = None;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
pub enum TrackMode {
    Live = 1,
    Timed = 2,
}

impl TryFrom<u64> for TrackMode {
    type Error = MessageError;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::Live),
            2 => Ok(Self::Timed),
            _ => Err(invalid_value(
                "track configuration",
                5,
                "has an unknown mode",
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoConfiguration {
    pub codec: String,
    pub packetization: String,
    pub extradata: Vec<u8>,
    pub coded_width: u32,
    pub coded_height: u32,
    pub profile: i32,
    pub level: i32,
    pub maximum_reorder_depth: u8,
    pub color_primaries: u64,
    pub transfer: u64,
    pub matrix: u64,
    pub signal_range: u64,
    pub aspect_numerator: u64,
    pub aspect_denominator: u64,
    pub maximum_access_unit_bytes: u32,
    pub codec_string: Option<String>,
    pub decoder_configuration: Option<Vec<u8>>,
}

impl VideoConfiguration {
    fn validate(&self) -> Result<(), MessageError> {
        if !media::is_portable_packetization(&self.codec, &self.packetization) {
            return Err(invalid_value("video configuration", 1, "is not canonical"));
        }
        if self.extradata.len() > 65_536 {
            return Err(invalid_value(
                "video configuration",
                2,
                "extradata exceeds 65536 bytes",
            ));
        }
        dimension("video configuration", 3, self.coded_width)?;
        dimension("video configuration", 4, self.coded_height)?;
        if self.maximum_reorder_depth > 64 {
            return Err(invalid_value(
                "video configuration",
                8,
                "reorder depth exceeds 64",
            ));
        }
        if !(1..=4).contains(&self.color_primaries)
            || !(1..=2).contains(&self.transfer)
            || self.matrix > 3
            || !(1..=2).contains(&self.signal_range)
        {
            return Err(invalid_value(
                "video configuration",
                10,
                "has unsupported colorimetry",
            ));
        }
        require_nonzero("video configuration", 14, self.aspect_numerator)?;
        require_nonzero("video configuration", 15, self.aspect_denominator)?;
        media::video_body_len(self.maximum_access_unit_bytes)
            .map_err(|_| invalid_value("video configuration", 16, "exceeds the body ceiling"))?;
        if self.codec_string.as_ref().is_some_and(|value| {
            value.len() > 64 || !value.bytes().all(|byte| (0x20..=0x7e).contains(&byte))
        }) {
            return Err(invalid_value(
                "video configuration",
                17,
                "is not at most 64 printable ASCII bytes",
            ));
        }
        if self
            .decoder_configuration
            .as_ref()
            .is_some_and(|value| value.len() > 4096)
        {
            return Err(invalid_value(
                "video configuration",
                18,
                "exceeds 4096 bytes",
            ));
        }
        Ok(())
    }

    fn to_value(&self) -> Result<Value, MessageError> {
        self.validate()?;
        let mut fields = vec![
            (0, Value::Text(self.codec.clone())),
            (1, Value::Text(self.packetization.clone())),
            (2, Value::Bytes(self.extradata.clone())),
            (3, Value::Unsigned(u64::from(self.coded_width))),
            (4, Value::Unsigned(u64::from(self.coded_height))),
            (5, Value::from_i64(i64::from(self.profile))),
            (6, Value::from_i64(i64::from(self.level))),
            (7, Value::Unsigned(0)),
            (8, Value::Unsigned(u64::from(self.maximum_reorder_depth))),
            (9, Value::Text("source-timebase-us".into())),
            (10, Value::Unsigned(self.color_primaries)),
            (11, Value::Unsigned(self.transfer)),
            (12, Value::Unsigned(self.matrix)),
            (13, Value::Unsigned(self.signal_range)),
            (14, Value::Unsigned(self.aspect_numerator)),
            (15, Value::Unsigned(self.aspect_denominator)),
            (
                16,
                Value::Unsigned(u64::from(self.maximum_access_unit_bytes)),
            ),
        ];
        if let Some(codec_string) = &self.codec_string {
            fields.push((17, Value::Text(codec_string.clone())));
        }
        if let Some(configuration) = &self.decoder_configuration {
            fields.push((18, Value::Bytes(configuration.clone())));
        }
        Ok(Value::Map(fields))
    }

    fn from_value(value: &Value) -> Result<Self, MessageError> {
        let map = StrictMap::new(
            "video configuration",
            value,
            &[
                0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18,
            ],
        )?;
        if map.required_u64(7)? != 0 || map.required_text(9)? != "source-timebase-us" {
            return Err(invalid_value(
                "video configuration",
                7,
                "uses an unsupported alpha mode or timeline",
            ));
        }
        let configuration = Self {
            codec: map.required_text(0)?.to_owned(),
            packetization: map.required_text(1)?.to_owned(),
            extradata: map.required_bytes(2)?.to_vec(),
            coded_width: map.required_u32(3)?,
            coded_height: map.required_u32(4)?,
            profile: required_i32(&map, 5)?,
            level: required_i32(&map, 6)?,
            maximum_reorder_depth: u8::try_from(map.required_u64(8)?)
                .map_err(|_| invalid_value("video configuration", 8, "does not fit in u8"))?,
            color_primaries: map.required_u64(10)?,
            transfer: map.required_u64(11)?,
            matrix: map.required_u64(12)?,
            signal_range: map.required_u64(13)?,
            aspect_numerator: map.required_u64(14)?,
            aspect_denominator: map.required_u64(15)?,
            maximum_access_unit_bytes: map.required_u32(16)?,
            codec_string: map
                .optional(17)
                .map(|value| {
                    value
                        .as_text()
                        .map(ToOwned::to_owned)
                        .ok_or_else(|| invalid_value("video configuration", 17, "is not text"))
                })
                .transpose()?,
            decoder_configuration: map.optional_bytes(18)?.map(ToOwned::to_owned),
        };
        configuration.validate()?;
        Ok(configuration)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioConfiguration {
    pub codec: String,
    pub packetization: String,
    pub extradata: Vec<u8>,
    pub sample_rate: u32,
    pub channels: u8,
    pub channel_mask: u64,
    pub maximum_access_unit_bytes: u32,
    pub codec_string: Option<String>,
}

impl AudioConfiguration {
    fn validate(&self) -> Result<(), MessageError> {
        media::validate_audio_initialization(
            &self.codec,
            &self.packetization,
            &self.extradata,
            self.sample_rate,
            u16::from(self.channels),
        )
        .map_err(|_| invalid_value("audio configuration", 1, "is not canonical"))?;
        if self.extradata.len() > 65_536
            || !(8_000..=192_000).contains(&self.sample_rate)
            || !(1..=8).contains(&self.channels)
            || self.maximum_access_unit_bytes == 0
            || self.maximum_access_unit_bytes > 1_048_576
        {
            return Err(invalid_value(
                "audio configuration",
                2,
                "has an out-of-range field",
            ));
        }
        if self.channel_mask != 0 && self.channel_mask.count_ones() != u32::from(self.channels) {
            return Err(invalid_value(
                "audio configuration",
                5,
                "does not contain the declared channel count",
            ));
        }
        if self
            .codec_string
            .as_ref()
            .is_some_and(|value| value.len() > 64)
        {
            return Err(invalid_value(
                "audio configuration",
                8,
                "codec string exceeds 64 bytes",
            ));
        }
        Ok(())
    }

    fn to_value(&self) -> Result<Value, MessageError> {
        self.validate()?;
        let mut fields = vec![
            (0, Value::Text(self.codec.clone())),
            (1, Value::Text(self.packetization.clone())),
            (2, Value::Bytes(self.extradata.clone())),
            (3, Value::Unsigned(u64::from(self.sample_rate))),
            (4, Value::Unsigned(u64::from(self.channels))),
            (5, Value::Unsigned(self.channel_mask)),
            (
                6,
                Value::Unsigned(u64::from(self.maximum_access_unit_bytes)),
            ),
            (7, Value::Text("source-timebase-us".into())),
        ];
        if let Some(codec_string) = &self.codec_string {
            fields.push((8, Value::Text(codec_string.clone())));
        }
        Ok(Value::Map(fields))
    }

    fn from_value(value: &Value) -> Result<Self, MessageError> {
        let map = StrictMap::new("audio configuration", value, &[0, 1, 2, 3, 4, 5, 6, 7, 8])?;
        if map.required_text(7)? != "source-timebase-us" {
            return Err(invalid_value(
                "audio configuration",
                7,
                "has an unsupported timeline",
            ));
        }
        let configuration = Self {
            codec: map.required_text(0)?.to_owned(),
            packetization: map.required_text(1)?.to_owned(),
            extradata: map.required_bytes(2)?.to_vec(),
            sample_rate: map.required_u32(3)?,
            channels: u8::try_from(map.required_u64(4)?)
                .map_err(|_| invalid_value("audio configuration", 4, "does not fit in u8"))?,
            channel_mask: map.required_u64(5)?,
            maximum_access_unit_bytes: map.required_u32(6)?,
            codec_string: map
                .optional(8)
                .map(|value| {
                    value
                        .as_text()
                        .map(ToOwned::to_owned)
                        .ok_or_else(|| invalid_value("audio configuration", 8, "is not text"))
                })
                .transpose()?,
        };
        configuration.validate()?;
        Ok(configuration)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RasterConfiguration {
    pub width: u32,
    pub height: u32,
    pub alpha_mode: u64,
    pub delta_enabled: bool,
    pub maximum_delta_operations: u8,
    pub zstd_enabled: bool,
}

impl RasterConfiguration {
    fn validate(&self) -> Result<(), MessageError> {
        dimension("raster configuration", 0, self.width)?;
        dimension("raster configuration", 1, self.height)?;
        if !(1..=2).contains(&self.alpha_mode)
            || !(1..=16).contains(&self.maximum_delta_operations)
            || (!self.delta_enabled && self.maximum_delta_operations != 1)
        {
            return Err(invalid_value(
                "raster configuration",
                3,
                "has invalid alpha or delta settings",
            ));
        }
        media::rgba8_raw_frame_body_len(self.width, self.height).map_err(|_| {
            invalid_value(
                "raster configuration",
                0,
                "raw full frame exceeds the body ceiling",
            )
        })?;
        Ok(())
    }

    fn to_value(&self) -> Result<Value, MessageError> {
        self.validate()?;
        Ok(Value::Map(vec![
            (0, Value::Unsigned(u64::from(self.width))),
            (1, Value::Unsigned(u64::from(self.height))),
            (2, Value::Unsigned(1)),
            (3, Value::Unsigned(self.alpha_mode)),
            (4, Value::Unsigned(u64::from(self.delta_enabled))),
            (5, Value::Unsigned(u64::from(self.maximum_delta_operations))),
            (6, Value::Unsigned(u64::from(self.zstd_enabled))),
            (7, Value::Unsigned(1)),
        ]))
    }

    fn from_value(value: &Value) -> Result<Self, MessageError> {
        let map = StrictMap::new("raster configuration", value, &[0, 1, 2, 3, 4, 5, 6, 7])?;
        if map.required_u64(2)? != 1 || map.required_u64(7)? != 1 {
            return Err(invalid_value(
                "raster configuration",
                2,
                "requires RGBA8 and sRGB",
            ));
        }
        let update_mode = map.required_u64(4)?;
        let compression = map.required_u64(6)?;
        if update_mode > 1 || compression > 1 {
            return Err(invalid_value(
                "raster configuration",
                4,
                "has an unknown update or compression mode",
            ));
        }
        let configuration = Self {
            width: map.required_u32(0)?,
            height: map.required_u32(1)?,
            alpha_mode: map.required_u64(3)?,
            delta_enabled: update_mode == 1,
            maximum_delta_operations: u8::try_from(map.required_u64(5)?)
                .map_err(|_| invalid_value("raster configuration", 5, "does not fit in u8"))?,
            zstd_enabled: compression == 1,
        };
        configuration.validate()?;
        Ok(configuration)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageConfiguration {
    pub encoding: u64,
    pub width: u32,
    pub height: u32,
    pub encoded_length: u32,
    pub sha256: Option<[u8; 32]>,
    pub cache_lookup: bool,
}

impl ImageConfiguration {
    fn validate(&self) -> Result<(), MessageError> {
        if !(1..=2).contains(&self.encoding) {
            return Err(invalid_value(
                "image configuration",
                0,
                "has an unknown encoding",
            ));
        }
        dimension("image configuration", 1, self.width)?;
        dimension("image configuration", 2, self.height)?;
        if self.encoded_length == 0 || self.encoded_length > HARD_MAX_RECORD_BODY {
            return Err(invalid_value(
                "image configuration",
                3,
                "has an invalid encoded length",
            ));
        }
        if self.cache_lookup && self.sha256.is_none() {
            return Err(invalid_value(
                "image configuration",
                6,
                "requires a SHA-256 value",
            ));
        }
        Ok(())
    }

    fn to_value(&self) -> Result<Value, MessageError> {
        self.validate()?;
        let mut fields = vec![
            (0, Value::Unsigned(self.encoding)),
            (1, Value::Unsigned(u64::from(self.width))),
            (2, Value::Unsigned(u64::from(self.height))),
            (3, Value::Unsigned(u64::from(self.encoded_length))),
        ];
        if let Some(hash) = self.sha256 {
            fields.push((4, Value::Bytes(hash.to_vec())));
        }
        fields.push((5, Value::Unsigned(1)));
        fields.push((6, Value::Bool(self.cache_lookup)));
        Ok(Value::Map(fields))
    }

    fn from_value(value: &Value) -> Result<Self, MessageError> {
        let map = StrictMap::new("image configuration", value, &[0, 1, 2, 3, 4, 5, 6])?;
        if map.required_u64(5)? != 1 {
            return Err(invalid_value(
                "image configuration",
                5,
                "requires sRGB output",
            ));
        }
        let configuration = Self {
            encoding: map.required_u64(0)?,
            width: map.required_u32(1)?,
            height: map.required_u32(2)?,
            encoded_length: map.required_u32(3)?,
            sha256: map.optional_fixed_bytes(4)?,
            cache_lookup: map.required_bool(6)?,
        };
        configuration.validate()?;
        Ok(configuration)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KindConfiguration {
    Video(VideoConfiguration),
    Audio(AudioConfiguration),
    Raster(RasterConfiguration),
    EncodedImage(ImageConfiguration),
}

impl KindConfiguration {
    pub const fn kind(&self) -> TrackKind {
        match self {
            Self::Video(_) => TrackKind::Video,
            Self::Audio(_) => TrackKind::Audio,
            Self::Raster(_) => TrackKind::Raster,
            Self::EncodedImage(_) => TrackKind::EncodedImage,
        }
    }

    fn to_value(&self) -> Result<Value, MessageError> {
        match self {
            Self::Video(value) => value.to_value(),
            Self::Audio(value) => value.to_value(),
            Self::Raster(value) => value.to_value(),
            Self::EncodedImage(value) => value.to_value(),
        }
    }

    fn from_value(kind: TrackKind, value: &Value) -> Result<Self, MessageError> {
        match kind {
            TrackKind::Video => Ok(Self::Video(VideoConfiguration::from_value(value)?)),
            TrackKind::Audio => Ok(Self::Audio(AudioConfiguration::from_value(value)?)),
            TrackKind::Raster => Ok(Self::Raster(RasterConfiguration::from_value(value)?)),
            TrackKind::EncodedImage => {
                Ok(Self::EncodedImage(ImageConfiguration::from_value(value)?))
            }
        }
    }
}

/// Media byte direction, independent of which role creates the track.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[repr(u64)]
pub enum TrackDirection {
    #[default]
    Downlink = 0,
    Uplink = 1,
}

impl TryFrom<u64> for TrackDirection {
    type Error = MessageError;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Downlink),
            1 => Ok(Self::Uplink),
            _ => Err(invalid_value(
                "track configuration",
                16,
                "unknown direction",
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackConfiguration {
    pub direction: TrackDirection,
    pub context_id: u64,
    pub surface_id: u64,
    pub track_id: u64,
    pub slot: u64,
    pub mode: TrackMode,
    pub lane: LaneClass,
    pub maximum_record_body: u32,
    pub maximum_rate_millihertz: u64,
    pub maximum_encoded_bits_per_second: u64,
    pub maximum_records_per_second: u64,
    pub maximum_inflight_body_bytes: u64,
    pub kind: KindConfiguration,
    pub target_latency_us: u64,
    pub maximum_latency_us: u64,
    pub retained_pixel_charge: u64,
}

impl TrackConfiguration {
    pub fn validate(&self, probe: bool) -> Result<(), MessageError> {
        if self.direction == TrackDirection::Uplink
            && (!matches!(self.kind, KindConfiguration::Audio(_))
                || self.mode != TrackMode::Live
                || self.lane != LaneClass::Realtime
                || self.slot != 0
                || self.retained_pixel_charge != 0)
        {
            return Err(invalid_value(
                "track configuration",
                16,
                "uplink requires live realtime audio without a surface slot or retained pixels",
            ));
        }
        require_nonzero("track configuration", 0, self.context_id)?;
        require_nonzero("track configuration", 1, self.surface_id)?;
        if probe {
            if self.track_id != 0 {
                return Err(invalid_value(
                    "track configuration",
                    2,
                    "must be zero for a probe",
                ));
            }
        } else {
            require_nonzero("track configuration", 2, self.track_id)?;
            for (key, value) in [
                (7, u64::from(self.maximum_record_body)),
                (8, self.maximum_rate_millihertz),
                (9, self.maximum_encoded_bits_per_second),
                (10, self.maximum_records_per_second),
                (11, self.maximum_inflight_body_bytes),
            ] {
                require_nonzero("track configuration", key, value)?;
            }
        }
        if self.maximum_record_body > HARD_MAX_RECORD_BODY {
            return Err(invalid_value(
                "track configuration",
                7,
                "exceeds the hard body limit",
            ));
        }
        if !matches!(self.lane, LaneClass::Realtime | LaneClass::Bulk) {
            return Err(invalid_value(
                "track configuration",
                6,
                "must use realtime or bulk",
            ));
        }
        if self.target_latency_us > self.maximum_latency_us {
            return Err(invalid_value(
                "track configuration",
                13,
                "exceeds maximum latency",
            ));
        }
        self.kind.to_value()?;
        let required_body = match &self.kind {
            KindConfiguration::Video(configuration) => {
                media::video_body_len(configuration.maximum_access_unit_bytes)
            }
            KindConfiguration::Audio(configuration) => {
                media::audio_body_len(configuration.maximum_access_unit_bytes)
            }
            KindConfiguration::Raster(configuration) => {
                media::rgba8_raw_frame_body_len(configuration.width, configuration.height)
            }
            KindConfiguration::EncodedImage(configuration) => Ok(configuration.encoded_length),
        }
        .map_err(|_| {
            invalid_value(
                "track configuration",
                7,
                "cannot carry its maximum legal media body",
            )
        })?;
        if self.maximum_record_body < required_body
            || self.maximum_inflight_body_bytes < u64::from(self.maximum_record_body)
        {
            return Err(invalid_value(
                "track configuration",
                11,
                "cannot admit one maximum legal media record",
            ));
        }
        Ok(())
    }

    pub fn payload(&self, probe: bool) -> Result<PayloadMap, MessageError> {
        self.validate(probe)?;
        let mut payload = vec![
            (0, Value::Unsigned(self.context_id)),
            (1, Value::Unsigned(self.surface_id)),
            (2, Value::Unsigned(self.track_id)),
            (3, Value::Unsigned(self.kind.kind() as u64)),
            (4, Value::Unsigned(self.slot)),
            (5, Value::Unsigned(self.mode as u64)),
            (6, Value::Unsigned(self.lane as u64)),
            (7, Value::Unsigned(u64::from(self.maximum_record_body))),
            (8, Value::Unsigned(self.maximum_rate_millihertz)),
            (9, Value::Unsigned(self.maximum_encoded_bits_per_second)),
            (10, Value::Unsigned(self.maximum_records_per_second)),
            (11, Value::Unsigned(self.maximum_inflight_body_bytes)),
            (12, self.kind.to_value()?),
            (13, Value::Unsigned(self.target_latency_us)),
            (14, Value::Unsigned(self.maximum_latency_us)),
            (15, Value::Unsigned(self.retained_pixel_charge)),
        ];
        if self.direction != TrackDirection::Downlink {
            payload.push((16, Value::Unsigned(self.direction as u64)));
        }
        Ok(payload)
    }

    pub fn decode(
        header_object_id: u64,
        payload: &Value,
        probe: bool,
    ) -> Result<Self, MessageError> {
        let map = StrictMap::new(
            "track configuration",
            payload,
            &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16],
        )?;
        let track_id = map.required_u64(2)?;
        validate_header_object(header_object_id, track_id)?;
        let kind = TrackKind::try_from(map.required_u64(3)?)?;
        let configuration = Self {
            direction: TrackDirection::try_from(map.optional_u64(16)?.unwrap_or(0))?,
            context_id: map.required_u64(0)?,
            surface_id: map.required_u64(1)?,
            track_id,
            slot: map.required_u64(4)?,
            mode: TrackMode::try_from(map.required_u64(5)?)?,
            lane: LaneClass::try_from(map.required_u64(6)?)?,
            maximum_record_body: map.required_u32(7)?,
            maximum_rate_millihertz: map.required_u64(8)?,
            maximum_encoded_bits_per_second: map.required_u64(9)?,
            maximum_records_per_second: map.required_u64(10)?,
            maximum_inflight_body_bytes: map.required_u64(11)?,
            kind: KindConfiguration::from_value(kind, map.required(12)?)?,
            target_latency_us: map.required_u64(13)?,
            maximum_latency_us: map.required_u64(14)?,
            retained_pixel_charge: map.required_u64(15)?,
        };
        configuration.validate(probe)?;
        Ok(configuration)
    }
}

#[derive(Debug, Clone)]
pub struct TrackState {
    pub revision: TrackRevision,
    pub channel_generation: ChannelGeneration,
    pub milestones: u64,
    pub media_epoch: u32,
    pub last_media_id: u64,
    pub flow: ChannelFlow,
    pub lost: bool,
}

impl TrackState {
    pub fn new() -> Self {
        Self {
            revision: TrackRevision::ONE,
            channel_generation: ChannelGeneration::ONE,
            milestones: 0,
            media_epoch: 0,
            last_media_id: 0,
            flow: ChannelFlow::default(),
            lost: false,
        }
    }

    pub fn accept_channel(
        &mut self,
        generation: ChannelGeneration,
        maximum_body_bytes: u64,
        maximum_records: u64,
        maximum_record_body: u32,
    ) -> Result<(), MessageError> {
        if self.lost || generation != self.channel_generation {
            return Err(invalid_value(
                "CHANNEL_ACCEPTED",
                3,
                "does not match the live channel generation",
            ));
        }
        if maximum_body_bytes < u64::from(maximum_record_body) || maximum_records == 0 {
            return Err(invalid_value(
                "CHANNEL_ACCEPTED",
                4,
                "does not admit one maximum legal record",
            ));
        }
        self.flow = ChannelFlow::new(maximum_body_bytes, maximum_records);
        self.milestones = MILESTONE_CHANNEL_ACCEPTED;
        self.advance_revision("CHANNEL_ACCEPTED")?;
        Ok(())
    }

    pub fn admit_media(
        &mut self,
        generation: ChannelGeneration,
        body_length: u32,
        epoch: u32,
        media_id: u64,
        random_access: bool,
    ) -> Result<(), MessageError> {
        if self.lost || generation != self.channel_generation {
            return Err(invalid_value(
                "media record",
                0,
                "uses a stale channel generation",
            ));
        }
        self.flow.admit(body_length).map_err(|error| match error {
            ResourceError::FlowControl => {
                invalid_value("media record", 0, "exceeds absolute flow allowance")
            }
            _ => invalid_value("media record", 0, "overflows accounting"),
        })?;
        if media_id == 0 || media_id <= self.last_media_id || epoch < self.media_epoch {
            return Err(invalid_value(
                "media record",
                0,
                "has a stale epoch or non-increasing media ID",
            ));
        }
        if (self.last_media_id == 0 || epoch > self.media_epoch) && !random_access {
            return Err(invalid_value(
                "media record",
                0,
                "must begin the epoch with a recovery unit",
            ));
        }
        self.media_epoch = epoch;
        self.last_media_id = media_id;
        self.milestones |= MILESTONE_FIRST_MEDIA;
        if random_access {
            self.milestones |= MILESTONE_RANDOM_ACCESS;
        }
        Ok(())
    }

    pub fn advance_channel(
        &mut self,
        expected: ChannelGeneration,
        next: ChannelGeneration,
    ) -> Result<(), MessageError> {
        if expected != self.channel_generation
            || next.get() != expected.get().checked_add(1).unwrap_or(0)
        {
            return Err(invalid_value(
                "ADVANCE_CHANNEL",
                4,
                "is not exactly the current generation plus one",
            ));
        }
        self.channel_generation = next;
        self.milestones = 0;
        self.flow = ChannelFlow::default();
        self.advance_revision("ADVANCE_CHANNEL")
    }

    pub fn detach(&mut self) -> Result<(), MessageError> {
        self.milestones |= MILESTONE_CHANNEL_DETACHED;
        self.advance_revision("channel detach")
    }

    pub fn lose(&mut self) -> Result<(), MessageError> {
        self.lost = true;
        self.milestones |= MILESTONE_TRACK_LOST;
        self.advance_revision("TRACK_LOST")
    }

    fn advance_revision(&mut self, schema: &'static str) -> Result<(), MessageError> {
        self.revision = self
            .revision
            .advance()
            .map_err(|_| invalid_value(schema, 0, "exhausted the track revision"))?;
        Ok(())
    }
}

impl Default for TrackState {
    fn default() -> Self {
        Self::new()
    }
}

fn dimension(schema: &'static str, key: u64, value: u32) -> Result<(), MessageError> {
    if !(1..=8192).contains(&value) {
        Err(invalid_value(schema, key, "is outside 1..=8192"))
    } else {
        Ok(())
    }
}

fn required_i32(map: &StrictMap<'_>, key: u64) -> Result<i32, MessageError> {
    let value = map
        .required(key)?
        .as_i64()
        .ok_or_else(|| invalid_value("track configuration", key, "is not an integer"))?;
    i32::try_from(value)
        .map_err(|_| invalid_value("track configuration", key, "does not fit in i32"))
}

trait SignedValue {
    fn from_i64(value: i64) -> Self;
}

impl SignedValue for Value {
    fn from_i64(value: i64) -> Self {
        if value >= 0 {
            Self::Unsigned(value as u64)
        } else {
            Self::Negative(value)
        }
    }
}

#[cfg(test)]
mod tests {

    /// The exact keys and order of a `MAX_CHANNEL_DATA` body.
    ///
    /// Pinned against a literal rather than against another call to the same function: this is the
    /// regression the three hand-written copies could not have, because each was its own authority
    /// on what the map should contain.
    #[test]
    fn a_flow_grant_carries_the_owner_tuple_then_the_two_maxima() {
        let address = TrackAddress {
            context_id: 1,
            surface_id: 2,
            track_id: 3,
            channel_generation: ChannelGeneration::new(4),
        };

        assert_eq!(
            max_channel_data_payload(address, 65_536, 128),
            vec![
                (0, Value::Unsigned(1)),
                (1, Value::Unsigned(2)),
                (2, Value::Unsigned(3)),
                (3, Value::Unsigned(4)),
                (4, Value::Unsigned(65_536)),
                (5, Value::Unsigned(128)),
            ]
        );
    }

    #[test]
    fn a_keyframe_request_carries_its_minimum_epoch_and_reason() {
        let address = TrackAddress {
            context_id: 7,
            surface_id: 8,
            track_id: 9,
            channel_generation: ChannelGeneration::new(2),
        };

        assert_eq!(
            need_keyframe_payload(address, 5, 2),
            vec![
                (0, Value::Unsigned(7)),
                (1, Value::Unsigned(8)),
                (2, Value::Unsigned(9)),
                (3, Value::Unsigned(2)),
                (4, Value::Unsigned(5)),
                (5, Value::Unsigned(2)),
            ]
        );
    }

    #[test]
    fn a_full_frame_request_takes_its_reason_rather_than_assuming_one() {
        // One presenter parameterised this reason and another hardcoded 1. Same value, but the
        // shared encoder has to accept it or the two cannot both use this.
        let address = TrackAddress {
            context_id: 1,
            surface_id: 1,
            track_id: 1,
            channel_generation: ChannelGeneration::new(1),
        };

        assert_eq!(
            need_full_frame_payload(address, 1).last(),
            Some(&(4, Value::Unsigned(1)))
        );
        assert_eq!(
            need_full_frame_payload(address, 3).last(),
            Some(&(4, Value::Unsigned(3)))
        );
    }

    #[test]
    fn every_channel_notification_opens_with_the_same_owner_tuple() {
        // The property that makes a wrong-track notification impossible to misread: all three
        // answer "which track" in keys 0 through 3, in one order.
        let address = TrackAddress {
            context_id: 11,
            surface_id: 12,
            track_id: 13,
            channel_generation: ChannelGeneration::new(14),
        };
        let expected = [
            (0, Value::Unsigned(11)),
            (1, Value::Unsigned(12)),
            (2, Value::Unsigned(13)),
            (3, Value::Unsigned(14)),
        ];

        for payload in [
            max_channel_data_payload(address, 1, 1),
            need_keyframe_payload(address, 0, 0),
            need_full_frame_payload(address, 1),
        ] {
            assert_eq!(&payload[..4], &expected[..], "payload: {payload:?}");
        }
    }
    use super::*;

    #[test]
    fn audio_gain_covers_the_kitim_volume_range() {
        assert_eq!(AudioGain::from_percent(0), Some(AudioGain::SILENT));
        assert_eq!(AudioGain::from_percent(100), Some(AudioGain::UNITY));
        assert_eq!(AudioGain::from_percent(200), Some(AudioGain::MAX));
        assert_eq!(AudioGain::from_percent(201), None);
        assert_eq!(AudioGain::new(AudioGain::MAX.raw() + 1), None);
    }

    #[test]
    fn channel_generation_resets_generation_local_state() {
        let mut state = TrackState::new();
        state
            .accept_channel(ChannelGeneration::ONE, 100, 4, 100)
            .unwrap();
        state
            .admit_media(ChannelGeneration::ONE, 20, 1, 1, true)
            .unwrap();
        state
            .advance_channel(ChannelGeneration::ONE, ChannelGeneration::new(2))
            .unwrap();
        assert_eq!(state.milestones, 0);
        assert_eq!(state.flow.sent_body_bytes, 0);
        assert_eq!(state.last_media_id, 1);
    }

    #[test]
    fn lower_flow_update_is_harmless() {
        let mut state = TrackState::new();
        state
            .accept_channel(ChannelGeneration::ONE, 100, 4, 100)
            .unwrap();
        state.flow.raise_maxima(50, 2);
        assert_eq!(state.flow.maximum_body_bytes, 100);
        assert_eq!(state.flow.maximum_media_records, 4);
    }

    #[test]
    fn uncertain_channel_open_replays_only_before_media() {
        let mut open = ChannelOpenState::default();
        let acceptance = vec![(0, Value::Unsigned(1))];
        assert!(matches!(
            open.open(
                ChannelGeneration::ONE,
                ChannelGeneration::ONE,
                [1; 16],
                b"open",
                acceptance.clone()
            ),
            ChannelOpenDecision::Fresh(_)
        ));
        assert_eq!(
            open.open(
                ChannelGeneration::ONE,
                ChannelGeneration::ONE,
                [1; 16],
                b"open",
                acceptance.clone()
            ),
            ChannelOpenDecision::Busy
        );
        open.transport_lost(ChannelGeneration::ONE);
        assert_eq!(
            open.open(
                ChannelGeneration::ONE,
                ChannelGeneration::ONE,
                [1; 16],
                b"open",
                acceptance
            ),
            ChannelOpenDecision::ExactReplay(vec![(0, Value::Unsigned(1))])
        );
        open.admit_media(ChannelGeneration::ONE).unwrap();
        open.transport_lost(ChannelGeneration::ONE);
        assert_eq!(
            open.open(
                ChannelGeneration::ONE,
                ChannelGeneration::ONE,
                [1; 16],
                b"open",
                vec![]
            ),
            ChannelOpenDecision::StaleGeneration
        );
    }

    #[test]
    fn strict_raster_track_configuration_round_trips() {
        let configuration = TrackConfiguration {
            direction: Default::default(),
            context_id: 1,
            surface_id: 2,
            track_id: 3,
            slot: 0,
            mode: TrackMode::Live,
            lane: LaneClass::Bulk,
            maximum_record_body: 72 + 64 * 64 * 4,
            maximum_rate_millihertz: 60_000,
            maximum_encoded_bits_per_second: 100_000_000,
            maximum_records_per_second: 60,
            maximum_inflight_body_bytes: 1_000_000,
            kind: KindConfiguration::Raster(RasterConfiguration {
                width: 64,
                height: 64,
                alpha_mode: 1,
                delta_enabled: true,
                maximum_delta_operations: 16,
                zstd_enabled: true,
            }),
            target_latency_us: 50_000,
            maximum_latency_us: 250_000,
            retained_pixel_charge: 64 * 64,
        };
        let payload = Value::Map(configuration.payload(false).unwrap());
        assert_eq!(
            TrackConfiguration::decode(3, &payload, false).unwrap(),
            configuration
        );
    }

    #[test]
    fn canonical_audio_track_configuration_round_trips() {
        let mut opus_head = b"OpusHead".to_vec();
        opus_head.extend_from_slice(&[1, 2, 0, 0, 0x80, 0xbb, 0, 0, 0, 0, 0]);
        let maximum_record_body = media::audio_body_len(4_096).unwrap();
        let configuration = TrackConfiguration {
            direction: Default::default(),
            context_id: 1,
            surface_id: 2,
            track_id: 3,
            slot: 2,
            mode: TrackMode::Timed,
            lane: LaneClass::Realtime,
            maximum_record_body,
            maximum_rate_millihertz: 50_000,
            maximum_encoded_bits_per_second: 512_000,
            maximum_records_per_second: 50,
            maximum_inflight_body_bytes: u64::from(maximum_record_body) * 4,
            kind: KindConfiguration::Audio(AudioConfiguration {
                codec: "opus".into(),
                packetization: media::AUDIO_PACKETIZATION_OPUS.into(),
                extradata: opus_head,
                sample_rate: 48_000,
                channels: 2,
                channel_mask: 3,
                maximum_access_unit_bytes: 4_096,
                codec_string: Some("opus".into()),
            }),
            target_latency_us: 0,
            maximum_latency_us: 2_000_000,
            retained_pixel_charge: 0,
        };
        let payload = Value::Map(configuration.payload(false).unwrap());
        assert_eq!(
            TrackConfiguration::decode(3, &payload, false).unwrap(),
            configuration
        );
    }
}

/// One track's complete owner tuple, as a presenter names it when notifying its producer.
///
/// The three channel notifications below all begin with it, in the same key order, because they all
/// answer "which track" before they answer anything else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrackAddress {
    pub context_id: u64,
    pub surface_id: u64,
    pub track_id: u64,
    pub channel_generation: ChannelGeneration,
}

impl TrackAddress {
    fn prefix(&self) -> PayloadMap {
        vec![
            (0, Value::Unsigned(self.context_id)),
            (1, Value::Unsigned(self.surface_id)),
            (2, Value::Unsigned(self.track_id)),
            (3, Value::Unsigned(self.channel_generation.get())),
        ]
    }
}

/// The body of `MAX_CHANNEL_DATA`: the absolute cumulative maxima a channel may now reach.
///
/// These are the ceilings, not an increment. A presenter that sends a smaller maximum than it sent
/// before is telling its producer nothing new, and media §6 makes the maxima monotonic for exactly
/// that reason.
pub fn max_channel_data_payload(
    address: TrackAddress,
    maximum_body_bytes: u64,
    maximum_media_records: u64,
) -> PayloadMap {
    let mut payload = address.prefix();
    payload.push((4, Value::Unsigned(maximum_body_bytes)));
    payload.push((5, Value::Unsigned(maximum_media_records)));
    payload
}

/// The body of `NEED_KEYFRAME`, media §13.
///
/// `minimum_epoch` is the epoch the presenter will accept a keyframe at or after; zero means the
/// current one. `reason` distinguishes a decoder reset from a transport loss, and only the latter
/// hands the replacement channel a fresh epoch.
pub fn need_keyframe_payload(address: TrackAddress, minimum_epoch: u32, reason: u64) -> PayloadMap {
    let mut payload = address.prefix();
    payload.push((4, Value::Unsigned(u64::from(minimum_epoch))));
    payload.push((5, Value::Unsigned(reason)));
    payload
}

/// The body of `NEED_FULL_FRAME`, media §13: a raster delta chain that cannot continue.
pub fn need_full_frame_payload(address: TrackAddress, reason: u64) -> PayloadMap {
    let mut payload = address.prefix();
    payload.push((4, Value::Unsigned(reason)));
    payload
}
