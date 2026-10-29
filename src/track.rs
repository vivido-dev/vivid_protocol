//! Media tracks: configuration, channel generations, and readiness (media §2 through §8).
//!
//! A [`TrackConfiguration`] is the immutable `CREATE_TRACK` schema, with a [`KindConfiguration`]
//! for each media kind. A track's media travels on an authenticated channel; [`ChannelOpenState`]
//! decides how to answer each `CHANNEL_OPEN`, and [`TrackState`] holds the generation, milestone
//! bits, and flow. Control records such as [`ActivateTrack`], [`AdvanceChannel`], and
//! [`WaitTrack`] have typed codecs here.

use crate::{
    HARD_MAX_RECORD_BODY,
    cbor::Value,
    media,
    messages::{
        LaneClass, MessageError, PayloadMap, StrictMap, TrackKind, invalid_value, require_nonzero,
        validate_header_object,
    },
    resource::{ChannelFlow, ResourceError},
    revision::{ChannelGeneration, SurfaceRevision, TrackRevision},
};
use sha2::{Digest, Sha256};

/// Milestone bit 0: the channel was accepted (media §7).
pub const MILESTONE_CHANNEL_ACCEPTED: u64 = 1 << 0;
/// Milestone bit 1: the first media record was accepted.
pub const MILESTONE_FIRST_MEDIA: u64 = 1 << 1;
/// Milestone bit 2: the decoder is initialized.
pub const MILESTONE_DECODER_INITIALIZED: u64 = 1 << 2;
/// Milestone bit 3: a random-access video unit, or a full raster or image, was accepted.
pub const MILESTONE_RANDOM_ACCESS: u64 = 1 << 3;
/// Milestone bit 4: the first decoded or composed output is ready.
pub const MILESTONE_OUTPUT_READY: u64 = 1 << 4;
/// Milestone bit 5: first presentation for the current surface generation.
///
/// It requires an eligible node to be visible.
pub const MILESTONE_PRESENTED: u64 = 1 << 5;
/// Milestone bit 6: the timed playback clock started.
pub const MILESTONE_CLOCK_STARTED: u64 = 1 << 6;
/// Milestone bit 7: end of stream was accepted.
pub const MILESTONE_EOS_ACCEPTED: u64 = 1 << 7;
/// Milestone bit 8: buffered playback ended.
pub const MILESTONE_BUFFERED_ENDED: u64 = 1 << 8;
/// Milestone bit 9: the channel is detached; it survives a generation advance.
pub const MILESTONE_CHANNEL_DETACHED: u64 = 1 << 9;
/// Milestone bit 10: the track is lost; it survives a generation advance.
pub const MILESTONE_TRACK_LOST: u64 = 1 << 10;
/// Every assigned milestone bit.
pub const MILESTONE_KNOWN_MASK: u64 = (1 << 11) - 1;

/// Unsigned Q32.32 linear amplitude used by `audio-gain-v1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct AudioGain(u64);

impl AudioGain {
    /// Zero gain.
    pub const SILENT: Self = Self(0);
    /// Gain of one: the audio plays unchanged.
    pub const UNITY: Self = Self(1_u64 << 32);
    /// The largest gain, two (200 percent).
    pub const MAX: Self = Self(2_u64 << 32);

    /// Returns the gain for Q32.32 value `raw`, or `None` above [`AudioGain::MAX`].
    pub const fn new(raw: u64) -> Option<Self> {
        if raw <= Self::MAX.0 {
            Some(Self(raw))
        } else {
            None
        }
    }

    /// Returns the Q32.32 value.
    pub const fn raw(self) -> u64 {
        self.0
    }

    /// Returns the gain for `percent`, or `None` above 200.
    pub fn from_percent(percent: u32) -> Option<Self> {
        if percent > 200 {
            return None;
        }
        Some(Self((u64::from(percent) << 32) / 100))
    }

    #[expect(
        clippy::cast_precision_loss,
        reason = "a presentation ratio; f32 precision is the intent"
    )]
    /// Returns the gain as a linear amplitude factor.
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

/// What [`ChannelOpenState::open`] decided about a `CHANNEL_OPEN` (media §5.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChannelOpenDecision {
    /// Accept the channel with this `CHANNEL_ACCEPTED` payload.
    Fresh(PayloadMap),
    /// Rebind an exact retry after transport loss with the original acceptance.
    ExactReplay(PayloadMap),
    /// Reject with `CHANNEL_BUSY`: the accepted transport is still live.
    Busy,
    /// Reject with `BAD_MESSAGE`: the generation was opened with other bytes.
    DifferentBytes,
    /// Reject: the open names a generation other than the current one, or media already arrived.
    StaleGeneration,
}

/// One `CHANNEL_OPEN` a presenter is deciding on, for [`ChannelOpenState::open`].
#[derive(Clone, PartialEq, Eq)]
pub struct ChannelOpenAttempt<'a> {
    /// The channel generation the open names.
    pub generation: ChannelGeneration,
    /// The open's client nonce, which identifies a retry.
    pub nonce: [u8; 16],
    /// The complete `CHANNEL_OPEN` record body; only its digest is kept.
    pub complete_open_bytes: &'a [u8],
    /// The `CHANNEL_ACCEPTED` payload prepared for this open, replayed to an exact retry.
    pub acceptance: PayloadMap,
}

impl std::fmt::Debug for ChannelOpenAttempt<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ChannelOpenAttempt")
            .field("generation", &self.generation)
            .field("open_len", &self.complete_open_bytes.len())
            .finish_non_exhaustive()
    }
}

/// One received media record, as [`TrackState::admit_media`] checks it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReceivedMedia {
    /// The channel generation the record arrived on.
    pub generation: ChannelGeneration,
    /// The record body length, charged against the channel's flow allowance.
    pub body_length: u32,
    /// The media epoch from the record prefix.
    pub epoch: u32,
    /// The media ID from the record prefix; nonzero and increasing.
    pub media_id: u64,
    /// Whether the record is a random-access unit, such as a key frame.
    pub random_access: bool,
}

/// A presenter's channel-open retry state for one track identity.
#[derive(Debug, Default)]
pub struct ChannelOpenState {
    accepted: Option<AcceptedOpen>,
}

impl ChannelOpenState {
    /// Decides how to answer a `CHANNEL_OPEN` for the track at `current_generation`.
    pub fn open(
        &mut self,
        current_generation: ChannelGeneration,
        attempt: ChannelOpenAttempt<'_>,
    ) -> ChannelOpenDecision {
        let ChannelOpenAttempt {
            generation,
            nonce,
            complete_open_bytes,
            acceptance,
        } = attempt;
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

    /// Admits a media record on the producer side's accepted channel.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError::InvalidValue`] when no channel is accepted, or `generation` is not
    /// the accepted channel or its transport is down.
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

    /// Records that the accepted transport for `generation` closed.
    pub fn transport_lost(&mut self, generation: ChannelGeneration) {
        if let Some(accepted) = &mut self.accepted
            && accepted.generation == generation
        {
            accepted.transport_live = false;
        }
    }

    /// Forgets the accepted open after the track moves to a new generation.
    pub fn advance(&mut self) {
        self.accepted = None;
    }
}

/// How a track presents media (`CREATE_TRACK` key 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
#[non_exhaustive]
pub enum TrackMode {
    /// Present media as soon as it is ready.
    Live = 1,
    /// Present media at exact timestamps under `timed-media-v1`.
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

/// Immutable video configuration (media §3.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoConfiguration {
    /// Canonical codec name, such as `h264` (key 0).
    pub codec: String,
    /// Packetization identifier, such as `h264-annexb-au-v1` (key 1).
    pub packetization: String,
    /// Codec extradata, at most 65,536 bytes (key 2).
    pub extradata: Vec<u8>,
    /// Coded width, `1..=8192` (key 3).
    pub coded_width: u32,
    /// Coded height, `1..=8192` (key 4).
    pub coded_height: u32,
    /// Codec profile (key 5).
    pub profile: i32,
    /// Codec level (key 6).
    pub level: i32,
    /// Most frames the decoder may reorder, at most 64 (key 8).
    pub maximum_reorder_depth: u8,
    /// Color primaries code (key 10).
    pub color_primaries: u64,
    /// Transfer characteristic code (key 11).
    pub transfer: u64,
    /// Matrix coefficients code (key 12).
    pub matrix: u64,
    /// Signal range code (key 13).
    pub signal_range: u64,
    /// Nonzero sample-aspect-ratio numerator (key 14).
    pub aspect_numerator: u64,
    /// Nonzero sample-aspect-ratio denominator (key 15).
    pub aspect_denominator: u64,
    /// Largest encoded access unit, nonzero (key 16).
    pub maximum_access_unit_bytes: u32,
    /// RFC 6381 codec string, at most 64 printable ASCII bytes (key 17).
    pub codec_string: Option<String>,
    /// Matching decoder configuration body, at most 4,096 bytes (key 18).
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
            (5, Value::from(i64::from(self.profile))),
            (6, Value::from(i64::from(self.level))),
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

/// Immutable audio configuration (media §3.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioConfiguration {
    /// Canonical codec name, such as `opus` (key 0).
    pub codec: String,
    /// Packetization identifier, such as `opus-packet-v1` (key 1).
    pub packetization: String,
    /// Codec extradata, at most 65,536 bytes (key 2).
    pub extradata: Vec<u8>,
    /// Sample rate, `8000..=192000` (key 3).
    pub sample_rate: u32,
    /// Channel count, `1..=8` (key 4).
    pub channels: u8,
    /// Channel mask, or zero for the default layout (key 5).
    pub channel_mask: u64,
    /// Largest encoded access unit, `1..=1048576` (key 6).
    pub maximum_access_unit_bytes: u32,
    /// RFC 6381 codec string (key 8).
    pub codec_string: Option<String>,
}

impl AudioConfiguration {
    fn validate(&self) -> Result<(), MessageError> {
        media::validate_audio_initialization(media::AudioInitialization {
            codec: &self.codec,
            packetization: &self.packetization,
            extradata: &self.extradata,
            sample_rate: self.sample_rate,
            channels: u16::from(self.channels),
        })
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

/// Immutable raster configuration (media §3.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RasterConfiguration {
    /// Width, `1..=8192` (key 0).
    pub width: u32,
    /// Height, `1..=8192` (key 1).
    pub height: u32,
    /// Straight (`1`) or premultiplied (`2`) alpha (key 3).
    pub alpha_mode: u64,
    /// Whether delta frames may follow full frames (key 4).
    pub delta_enabled: bool,
    /// Most operations in one delta frame, `1..=16` (key 5).
    pub maximum_delta_operations: u8,
    /// Whether frames may be zstd-compressed (key 6).
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

/// Immutable encoded-image configuration (media §3.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageConfiguration {
    /// PNG (`1`) or JPEG (`2`) (key 0).
    pub encoding: u64,
    /// Decoded width, `1..=8192` (key 1).
    pub width: u32,
    /// Decoded height, `1..=8192` (key 2).
    pub height: u32,
    /// Exact encoded byte length (key 3).
    pub encoded_length: u32,
    /// SHA-256 of the exact encoded bytes (key 4).
    pub sha256: Option<[u8; 32]>,
    /// Requests a lookup in the context's immutable image cache (key 6).
    pub cache_lookup: bool,
}

impl ImageConfiguration {
    /// Checks the encoding, dimensions, and declared length against the media specification.
    ///
    /// A producer can call it to reject a container before building a track configuration.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError::InvalidValue`] when the encoding is unknown, a dimension is out of
    /// range, the encoded length is invalid, or a required SHA-256 value is missing.
    pub fn validate(&self) -> Result<(), MessageError> {
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

/// Immutable logical extent and byte ceiling of a retained vector track.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VectorConfiguration {
    /// Logical width, `1..=16384` (key 0).
    pub width: u32,
    /// Logical height, `1..=16384` (key 1).
    pub height: u32,
    /// Largest encoded scene (key 2).
    pub maximum_scene_bytes: u32,
}
impl VectorConfiguration {
    fn to_value(&self) -> Result<Value, MessageError> {
        if self.width == 0
            || self.height == 0
            || self.width > 16384
            || self.height > 16384
            || self.maximum_scene_bytes == 0
            || self.maximum_scene_bytes as usize > crate::vector::MAX_SCENE_BYTES
        {
            return Err(invalid_value(
                "vector configuration",
                0,
                "exceeds vector limits",
            ));
        }
        Ok(Value::Map(vec![
            (0, Value::Unsigned(u64::from(self.width))),
            (1, Value::Unsigned(u64::from(self.height))),
            (2, Value::Unsigned(u64::from(self.maximum_scene_bytes))),
        ]))
    }
    fn from_value(value: &Value) -> Result<Self, MessageError> {
        let map = StrictMap::new("vector configuration", value, &[0, 1, 2])?;
        let read = |key| {
            u32::try_from(map.required_u64(key)?)
                .map_err(|_| invalid_value("vector configuration", key, "exceeds u32"))
        };
        let config = Self {
            width: read(0)?,
            height: read(1)?,
            maximum_scene_bytes: read(2)?,
        };
        config.to_value()?;
        Ok(config)
    }
}

/// The kind-specific configuration in `CREATE_TRACK` key 12.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum KindConfiguration {
    /// A video track.
    Video(VideoConfiguration),
    /// An audio track.
    Audio(AudioConfiguration),
    /// A raster track.
    Raster(RasterConfiguration),
    /// An encoded-image track.
    EncodedImage(ImageConfiguration),
    /// A vector-scene track.
    VectorScene(VectorConfiguration),
}

impl KindConfiguration {
    /// Returns the track kind this configuration is for.
    pub const fn kind(&self) -> TrackKind {
        match self {
            Self::Video(_) => TrackKind::Video,
            Self::Audio(_) => TrackKind::Audio,
            Self::Raster(_) => TrackKind::Raster,
            Self::EncodedImage(_) => TrackKind::EncodedImage,
            Self::VectorScene(_) => TrackKind::VectorScene,
        }
    }

    fn to_value(&self) -> Result<Value, MessageError> {
        match self {
            Self::Video(value) => value.to_value(),
            Self::Audio(value) => value.to_value(),
            Self::Raster(value) => value.to_value(),
            Self::EncodedImage(value) => value.to_value(),
            Self::VectorScene(value) => value.to_value(),
        }
    }

    fn from_value(kind: TrackKind, value: &Value) -> Result<Self, MessageError> {
        match kind {
            TrackKind::VectorScene => {
                Ok(Self::VectorScene(VectorConfiguration::from_value(value)?))
            }
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
    /// Producer to presenter, the default.
    #[default]
    Downlink = 0,
    /// Presenter to producer, such as microphone input (`audio-input-v1`).
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

/// The `CREATE_TRACK` and `PROBE_TRACK_CONFIG` schema (media §2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackConfiguration {
    /// Immutable direction; omitted on the wire for downlink (key 16).
    pub direction: TrackDirection,
    /// Owning context (key 0).
    pub context_id: u64,
    /// Owning surface (key 1).
    pub surface_id: u64,
    /// Track ID; zero for a probe (key 2).
    pub track_id: u64,
    /// Intended surface slot (key 4).
    pub slot: u64,
    /// Live or timed presentation (key 5).
    pub mode: TrackMode,
    /// Realtime or bulk lane (key 6).
    pub lane: LaneClass,
    /// Largest media-record body, nonzero (key 7).
    pub maximum_record_body: u32,
    /// Largest frame or access-unit rate in millihertz (key 8).
    pub maximum_rate_millihertz: u64,
    /// Largest encoded bit rate (key 9).
    pub maximum_encoded_bits_per_second: u64,
    /// Largest media record rate (key 10).
    pub maximum_records_per_second: u64,
    /// Requested in-flight body-byte allowance (key 11).
    pub maximum_inflight_body_bytes: u64,
    /// Kind-specific configuration; it also sets the track kind (keys 3 and 12).
    pub kind: KindConfiguration,
    /// Target latency in microseconds, for live mode (key 13).
    pub target_latency_us: u64,
    /// Largest latency in microseconds (key 14).
    pub maximum_latency_us: u64,
    /// Requested retained-pixel charge (key 15).
    pub retained_pixel_charge: u64,
}

impl TrackConfiguration {
    /// Checks the configuration; `probe` selects the `PROBE_TRACK_CONFIG` rules, where the track ID
    /// is zero.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError::InvalidValue`] when a required ID is zero (or a probe's track ID is
    /// nonzero), an uplink track is not live realtime audio without a slot or retained pixels, a
    /// vector scene is not on live bulk slot 5, the lane is not realtime or bulk, the body or flow
    /// limits cannot carry one maximum legal record or exceed the hard limit, the target latency
    /// exceeds the maximum, or the kind-specific configuration is invalid.
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
        if matches!(self.kind, KindConfiguration::VectorScene(_))
            && (self.slot != 5 || self.mode != TrackMode::Live || self.lane != LaneClass::Bulk)
        {
            return Err(invalid_value(
                "track configuration",
                3,
                "vector scenes require live bulk visual slot 5",
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
            KindConfiguration::VectorScene(configuration) => {
                Ok(configuration.maximum_scene_bytes + 12)
            }
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

    /// Encodes the `CREATE_TRACK` payload, or the `PROBE_TRACK_CONFIG` payload when `probe` is set.
    ///
    /// # Errors
    ///
    /// Returns any error from [`TrackConfiguration::validate`].
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

    /// Decodes a `CREATE_TRACK` payload, or a `PROBE_TRACK_CONFIG` payload when `probe` is set.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError`] when `payload` is not a strict track map, a field is missing or has
    /// the wrong type, the track ID differs from `header_object_id`, the kind, direction, mode, or
    /// lane is unregistered, the kind configuration is invalid, or the result fails
    /// [`TrackConfiguration::validate`].
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

/// A track's mutable presenter state.
#[derive(Debug, Clone)]
pub struct TrackState {
    /// Track revision; advances on each change.
    pub revision: TrackRevision,
    /// Current channel generation.
    pub channel_generation: ChannelGeneration,
    /// Milestone bits for the current generation, such as [`MILESTONE_OUTPUT_READY`].
    pub milestones: u64,
    /// Current media epoch.
    pub media_epoch: u32,
    /// ID of the last accepted media record.
    pub last_media_id: u64,
    /// Flow accounting for the current generation.
    pub flow: ChannelFlow,
    /// Whether the presenter lost the track.
    pub lost: bool,
}

impl TrackState {
    /// Creates the state of a newly created track.
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

    /// Accepts a channel for `generation` with the given credit.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError::InvalidValue`] when the track is lost, `generation` is not the live
    /// channel generation, the credit cannot admit one maximum legal record, or the track revision
    /// is exhausted.
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

    /// Admits one received media record against the channel's credit and ordering.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError::InvalidValue`] when the track is lost or the channel generation is
    /// stale, the record exceeds the flow allowance or overflows accounting, the media ID is zero
    /// or not increasing, the epoch moved backward, or a new epoch does not begin with a
    /// random-access unit.
    pub fn admit_media(&mut self, media: ReceivedMedia) -> Result<(), MessageError> {
        let ReceivedMedia {
            generation,
            body_length,
            epoch,
            media_id,
            random_access,
        } = media;
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

    /// Moves the track from channel generation `expected` to `next`.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError::InvalidValue`] when `expected` is not current, `next` is not exactly
    /// one more, or the track revision is exhausted.
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

    /// Records that the track's channel detached.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError::InvalidValue`] when the track revision is exhausted.
    pub fn detach(&mut self) -> Result<(), MessageError> {
        self.milestones |= MILESTONE_CHANNEL_DETACHED;
        self.advance_revision("channel detach")
    }

    /// Marks the track lost.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError::InvalidValue`] when the track revision is exhausted.
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
    if (1..=8192).contains(&value) {
        Ok(())
    } else {
        Err(invalid_value(schema, key, "is outside 1..=8192"))
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

/// The `ADVANCE_CHANNEL` payload: reattach a track on a new channel generation (media §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdvanceChannel {
    /// The owning context.
    pub context_id: u64,
    /// The track's surface.
    pub surface_id: u64,
    /// The track to advance.
    pub track_id: u64,
    /// The channel generation the producer believes is current.
    pub current_generation: ChannelGeneration,
    /// The new generation, exactly `current_generation` plus one.
    pub next_generation: ChannelGeneration,
    /// Why the channel advances, a `channel_advance_reason` registry value.
    pub reason: u64,
}

impl AdvanceChannel {
    /// Encodes the payload map.
    pub fn payload(&self) -> PayloadMap {
        vec![
            (0, Value::Unsigned(self.context_id)),
            (1, Value::Unsigned(self.surface_id)),
            (2, Value::Unsigned(self.track_id)),
            (3, Value::Unsigned(self.current_generation.get())),
            (4, Value::Unsigned(self.next_generation.get())),
            (5, Value::Unsigned(self.reason)),
        ]
    }

    /// Decodes an `ADVANCE_CHANNEL` payload; generation and reason checks need track state.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError`] when `value` is not a strict `ADVANCE_CHANNEL` map or a field is
    /// missing or not an unsigned integer.
    pub fn decode(value: &Value) -> Result<Self, MessageError> {
        let map = StrictMap::new("ADVANCE_CHANNEL", value, &[0, 1, 2, 3, 4, 5])?;
        Ok(Self {
            context_id: map.required_u64(0)?,
            surface_id: map.required_u64(1)?,
            track_id: map.required_u64(2)?,
            current_generation: ChannelGeneration::new(map.required_u64(3)?),
            next_generation: ChannelGeneration::new(map.required_u64(4)?),
            reason: map.required_u64(5)?,
        })
    }
}

/// The `SET_AUDIO_GAIN` payload: an audio track's linear output gain (media §15.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetAudioGain {
    /// The owning context.
    pub context_id: u64,
    /// The track's surface.
    pub surface_id: u64,
    /// The audio track.
    pub track_id: u64,
    /// The new gain, zero through 2.0.
    pub gain: AudioGain,
}

impl SetAudioGain {
    /// Encodes the payload map.
    pub fn payload(&self) -> PayloadMap {
        vec![
            (0, Value::Unsigned(self.context_id)),
            (1, Value::Unsigned(self.surface_id)),
            (2, Value::Unsigned(self.track_id)),
            (3, Value::Unsigned(self.gain.raw())),
        ]
    }

    /// Decodes a `SET_AUDIO_GAIN` payload.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError`] when `value` is not a strict `SET_AUDIO_GAIN` map, a field is
    /// missing or not an unsigned integer, or the gain exceeds 2.0.
    pub fn decode(value: &Value) -> Result<Self, MessageError> {
        let map = StrictMap::new("SET_AUDIO_GAIN", value, &[0, 1, 2, 3])?;
        Ok(Self {
            context_id: map.required_u64(0)?,
            surface_id: map.required_u64(1)?,
            track_id: map.required_u64(2)?,
            gain: AudioGain::new(map.required_u64(3)?)
                .ok_or_else(|| invalid_value("SET_AUDIO_GAIN", 3, "exceeds 2.0"))?,
        })
    }
}

/// One slot of an `ACTIVATE_TRACK` request (media §9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlotBinding {
    /// The visual or audio slot to update.
    pub slot: u64,
    /// The track to activate in the slot, or zero to clear it.
    pub track_id: u64,
    /// The channel generation the track must be on.
    pub channel_generation: ChannelGeneration,
    /// The `MILESTONE_*` bit the track's current generation must have reached.
    pub required_milestone: u64,
}

impl SlotBinding {
    fn value(&self) -> Value {
        Value::Map(vec![
            (0, Value::Unsigned(self.slot)),
            (1, Value::Unsigned(self.track_id)),
            (2, Value::Unsigned(self.channel_generation.get())),
            (3, Value::Unsigned(self.required_milestone)),
        ])
    }

    fn decode(value: &Value) -> Result<Self, MessageError> {
        let map = StrictMap::new("slot binding", value, &[0, 1, 2, 3])?;
        Ok(Self {
            slot: map.required_u64(0)?,
            track_id: map.required_u64(1)?,
            channel_generation: ChannelGeneration::new(map.required_u64(2)?),
            required_milestone: map.required_u64(3)?,
        })
    }
}

/// The `ACTIVATE_TRACK` payload: atomically rebind one or more of a surface's slots (media §9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivateTrack {
    /// The owning context.
    pub context_id: u64,
    /// The surface whose slots change.
    pub surface_id: u64,
    /// The slots to update, all applied or none.
    pub bindings: Vec<SlotBinding>,
    /// The surface revision the activation expects.
    pub expected_surface_revision: SurfaceRevision,
}

impl ActivateTrack {
    /// Encodes the payload map.
    pub fn payload(&self) -> PayloadMap {
        vec![
            (0, Value::Unsigned(self.context_id)),
            (1, Value::Unsigned(self.surface_id)),
            (
                2,
                Value::Array(self.bindings.iter().map(SlotBinding::value).collect()),
            ),
            (3, Value::Unsigned(self.expected_surface_revision.get())),
        ]
    }

    /// Decodes an `ACTIVATE_TRACK` payload; slot and track checks need surface state.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError`] when `value` is not a strict `ACTIVATE_TRACK` map, a field is
    /// missing or mistyped, or a binding is not a strict four-key map.
    pub fn decode(value: &Value) -> Result<Self, MessageError> {
        let map = StrictMap::new("ACTIVATE_TRACK", value, &[0, 1, 2, 3])?;
        let bindings = map
            .required(2)?
            .as_array()
            .ok_or_else(|| invalid_value("ACTIVATE_TRACK", 2, "is not an array"))?
            .iter()
            .map(SlotBinding::decode)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            context_id: map.required_u64(0)?,
            surface_id: map.required_u64(1)?,
            bindings,
            expected_surface_revision: SurfaceRevision::new(map.required_u64(3)?),
        })
    }
}

/// The `WAIT_TRACK` payload: a bounded, correlated wait for one track condition (media §12).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WaitTrack {
    /// The owning context.
    pub context_id: u64,
    /// The track's surface.
    pub surface_id: u64,
    /// The track to watch.
    pub track_id: u64,
    /// The wait condition, 1 through 9.
    ///
    /// 1 track revision above the value, 2 milestone set, 3 raster frame presented, 4 video PTS
    /// presented, 5 playback started, 6 playback ended, 7 channel accepted, 8 channel closed, 9
    /// track lost.
    pub condition: u64,
    /// The threshold conditions 1 through 4 compare against; absent for 5 through 9.
    pub condition_value: Option<u64>,
    /// How long to wait, nonzero and at most [`crate::MAX_TRACK_WAIT_TIMEOUT_US`].
    pub timeout_us: u64,
    /// The channel generation the condition applies to; nonzero.
    pub channel_generation: ChannelGeneration,
}

impl WaitTrack {
    /// Encodes the payload map.
    pub fn payload(&self) -> PayloadMap {
        let mut payload = vec![
            (0, Value::Unsigned(self.context_id)),
            (1, Value::Unsigned(self.surface_id)),
            (2, Value::Unsigned(self.track_id)),
            (3, Value::Unsigned(self.condition)),
        ];
        if let Some(value) = self.condition_value {
            payload.push((4, Value::Unsigned(value)));
        }
        payload.push((5, Value::Unsigned(self.timeout_us)));
        payload.push((6, Value::Unsigned(self.channel_generation.get())));
        payload
    }

    /// Decodes a `WAIT_TRACK` payload, checking the condition, value, timeout, and generation.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError`] when `value` is not a strict `WAIT_TRACK` map, a field is missing
    /// or mistyped, the condition is unknown or its value is missing or superfluous, the timeout is
    /// zero or above [`crate::MAX_TRACK_WAIT_TIMEOUT_US`], or the generation is zero.
    pub fn decode(value: &Value) -> Result<Self, MessageError> {
        let map = StrictMap::new("WAIT_TRACK", value, &[0, 1, 2, 3, 4, 5, 6])?;
        let wait = Self {
            context_id: map.required_u64(0)?,
            surface_id: map.required_u64(1)?,
            track_id: map.required_u64(2)?,
            condition: map.required_u64(3)?,
            condition_value: map.optional_u64(4)?,
            timeout_us: map.required_u64(5)?,
            channel_generation: ChannelGeneration::new(map.required_u64(6)?),
        };
        let value_is_valid = match wait.condition {
            1..=4 => wait.condition_value.is_some(),
            5..=9 => wait.condition_value.is_none(),
            _ => false,
        };
        if !value_is_valid {
            return Err(invalid_value(
                "WAIT_TRACK",
                3,
                "is unknown or disagrees with its value",
            ));
        }
        if wait.timeout_us == 0 || wait.timeout_us > crate::MAX_TRACK_WAIT_TIMEOUT_US {
            return Err(invalid_value(
                "WAIT_TRACK",
                5,
                "is zero or above 30 seconds",
            ));
        }
        if wait.channel_generation.get() == 0 {
            return Err(invalid_value("WAIT_TRACK", 6, "must be nonzero"));
        }
        Ok(wait)
    }
}

/// The `FLUSH` payload: discard queued media and start a new epoch (media §15).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Flush {
    /// The owning context.
    pub context_id: u64,
    /// The track's surface.
    pub surface_id: u64,
    /// The track to flush.
    pub track_id: u64,
    /// The new media epoch, which must exceed the current one.
    pub epoch: u32,
}

impl Flush {
    /// Encodes the payload map.
    pub fn payload(&self) -> PayloadMap {
        vec![
            (0, Value::Unsigned(self.context_id)),
            (1, Value::Unsigned(self.surface_id)),
            (2, Value::Unsigned(self.track_id)),
            (3, Value::Unsigned(u64::from(self.epoch))),
        ]
    }

    /// Decodes a `FLUSH` payload; whether the epoch advances needs track state.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError`] when `value` is not a strict `FLUSH` map, a field is missing or
    /// not an unsigned integer, or the epoch does not fit in `u32`.
    pub fn decode(value: &Value) -> Result<Self, MessageError> {
        let map = StrictMap::new("FLUSH", value, &[0, 1, 2, 3])?;
        Ok(Self {
            context_id: map.required_u64(0)?,
            surface_id: map.required_u64(1)?,
            track_id: map.required_u64(2)?,
            epoch: map.required_u32(3)?,
        })
    }
}

/// The `CHANNEL_EOS` payload: the ordered end of a track generation's media (media §14).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelEos {
    /// The owning context.
    pub context_id: u64,
    /// The track's surface.
    pub surface_id: u64,
    /// The track whose ingress closes.
    pub track_id: u64,
    /// The channel generation that ends.
    pub channel_generation: ChannelGeneration,
    /// The media epoch of the last media record.
    pub epoch: u32,
    /// The track-connection sequence of the last media record, or zero when none was sent.
    pub last_media_sequence: u64,
}

impl ChannelEos {
    /// Encodes the payload map.
    pub fn payload(&self) -> PayloadMap {
        vec![
            (0, Value::Unsigned(self.context_id)),
            (1, Value::Unsigned(self.surface_id)),
            (2, Value::Unsigned(self.track_id)),
            (3, Value::Unsigned(self.channel_generation.get())),
            (4, Value::Unsigned(u64::from(self.epoch))),
            (5, Value::Unsigned(self.last_media_sequence)),
        ]
    }

    /// Decodes a `CHANNEL_EOS` payload; identity and sequence checks need channel state.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError`] when `value` is not a strict `CHANNEL_EOS` map, a field is missing
    /// or not an unsigned integer, or the epoch does not fit in `u32`.
    pub fn decode(value: &Value) -> Result<Self, MessageError> {
        let map = StrictMap::new("CHANNEL_EOS", value, &[0, 1, 2, 3, 4, 5])?;
        Ok(Self {
            context_id: map.required_u64(0)?,
            surface_id: map.required_u64(1)?,
            track_id: map.required_u64(2)?,
            channel_generation: ChannelGeneration::new(map.required_u64(3)?),
            epoch: map.required_u32(4)?,
            last_media_sequence: map.required_u64(5)?,
        })
    }
}

/// One track's complete owner tuple, as a presenter names it when notifying its producer.
///
/// The three channel notifications below all begin with it, in the same key order, because they all
/// answer "which track" before they answer anything else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrackAddress {
    /// Owning context.
    pub context_id: u64,
    /// Owning surface.
    pub surface_id: u64,
    /// The track.
    pub track_id: u64,
    /// The channel generation the notification is about.
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
