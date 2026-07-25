//! Normative Vivid 1.0 numeric registry and deterministic control schemas.

#![allow(dead_code)]

use std::collections::{HashMap, HashSet};
use std::io;

use super::cbor::{self, Encoder, Value};
use super::{VIVID_MAJOR, VIVID_MINOR};

pub const HELLO: u16 = 0x0001;
pub const WELCOME: u16 = 0x0002;
pub const OK: u16 = 0x0003;
pub const ERROR: u16 = 0x0004;
pub const PING: u16 = 0x0005;
pub const PONG: u16 = 0x0006;
pub const GOODBYE: u16 = 0x0007;
pub const DISPLAY_CHANGED: u16 = 0x0008;
pub const CAPS_CHANGED: u16 = 0x0009;

pub const PROBE_VIDEO_CONFIG: u16 = 0x0100;
pub const VIDEO_SUPPORT: u16 = 0x0101;
pub const CREATE_IMAGE: u16 = 0x0102;
pub const CREATE_VIDEO: u16 = 0x0103;
pub const CREATE_RASTER: u16 = 0x0104;
pub const SOURCE_READY: u16 = 0x0105;
pub const RECONFIGURE_SOURCE: u16 = 0x0106;
pub const DESTROY_SOURCE: u16 = 0x0107;
pub const SOURCE_LOST: u16 = 0x0108;
pub const PROBE_AUDIO_CONFIG: u16 = 0x0109;
pub const AUDIO_SUPPORT: u16 = 0x010a;
pub const CREATE_AUDIO: u16 = 0x010b;

pub const BEGIN_TXN: u16 = 0x0200;
pub const CREATE_NODE: u16 = 0x0201;
pub const UPDATE_NODE: u16 = 0x0202;
pub const DELETE_NODE: u16 = 0x0203;
pub const COMMIT_TXN: u16 = 0x0204;
pub const ABORT_TXN: u16 = 0x0205;
pub const PRESENTED: u16 = 0x0206;
pub const ANCHOR_READY: u16 = 0x0207;
pub const ANCHOR_GONE: u16 = 0x0208;
pub const BARRIER_REACHED: u16 = 0x0209;

pub const PLAY: u16 = 0x0300;
pub const PAUSE: u16 = 0x0301;
pub const STEP: u16 = 0x0302;
pub const FLUSH: u16 = 0x0303;
pub const DRAIN: u16 = 0x0304;
pub const EOS: u16 = 0x0305;
pub const PLAYBACK_STATE: u16 = 0x0306;

pub const CREDIT: u16 = 0x0400;
pub const FEEDBACK: u16 = 0x0401;
pub const VISIBILITY: u16 = 0x0402;
pub const QUALITY_HINT: u16 = 0x0403;
pub const NEED_KEYFRAME: u16 = 0x0404;

pub const BLOB_OFFER: u16 = 0x0500;
pub const BLOB_HAVE: u16 = 0x0501;
pub const BLOB_NEED: u16 = 0x0502;
pub const BLOB_COMPLETE: u16 = 0x0503;
pub const CACHE_EVICTED: u16 = 0x0504;

pub const CREATE_CONTEXT: u16 = 0x0600;
pub const DELEGATE_CONTEXT: u16 = 0x0601;
pub const REVOKE_CONTEXT: u16 = 0x0602;
pub const CONTEXT_CHANGED: u16 = 0x0603;

pub const KEY_INPUT: u16 = 0x7000;
pub const POINTER_MOTION: u16 = 0x7001;
pub const POINTER_BUTTON: u16 = 0x7002;
pub const POINTER_AXIS: u16 = 0x7003;
pub const INPUT_RESET: u16 = 0x7004;

pub const ATTACH_CHANNEL: u16 = 0x8000;
pub const VIDEO_PACKET: u16 = 0x8001;
pub const VIDEO_FRAGMENT: u16 = 0x8002;
pub const RASTER_FRAME: u16 = 0x8003;
pub const BLOB_CHUNK: u16 = 0x8004;
pub const BUFFER_SUBMIT: u16 = 0x8005;
pub const IMAGE_DATA: u16 = 0x8006;
pub const AUDIO_PACKET: u16 = 0x8007;

pub const FEATURE_RASTER_RGBA8: u64 = 1;
pub const FEATURE_RETIRED_VIDEO_FFMPEG_PACKET_V0: u64 = 2;
pub const FEATURE_SCENE_TRANSACTIONS: u64 = 3;
pub const FEATURE_GRID_CELL_NODES: u64 = 4;
pub const FEATURE_CREDIT_FLOW_CONTROL: u64 = 5;
pub const FEATURE_RETIRED_TEXT_ANCHORS_V1: u64 = 6;
pub const FEATURE_ENCODED_IMAGE_V1: u64 = 7;
pub const FEATURE_RASTER_ZSTD_V1: u64 = 8;
pub const FEATURE_RASTER_PREMULTIPLIED_ALPHA: u64 = 9;
pub const FEATURE_VISIBILITY_EVENTS_V1: u64 = 10;
pub const FEATURE_VIDEO_ACCESS_UNIT_V1: u64 = 11;
pub const FEATURE_VIDEO_CONTROL_V1: u64 = 12;
pub const FEATURE_TEXT_ANCHORS_V2: u64 = 13;
pub const FEATURE_AUDIO_ACCESS_UNIT_V1: u64 = 14;
pub const FEATURE_NODE_CLIP_RECT_V1: u64 = 15;
pub const FEATURE_DECODER_DESCRIPTION_V1: u64 = 16;
pub const FEATURE_DESKTOP_INPUT_V1: u64 = 17;

/// Negotiate a HELLO feature request against a presenter's supported set.
///
/// Every required feature must be supported; the first unsupported one is returned as the error
/// (answer it with `ERROR_UNSUPPORTED_FEATURE`). The accepted set is the supported union of
/// required and optional features, sorted and deduplicated as WELCOME demands.
pub fn negotiate_features(
    required: &[u64],
    optional: &[u64],
    mut supported: impl FnMut(u64) -> bool,
) -> Result<Vec<u64>, u64> {
    if let Some(missing) = required.iter().find(|feature| !supported(**feature)) {
        return Err(*missing);
    }
    let mut accepted: Vec<u64> = required
        .iter()
        .chain(optional.iter())
        .copied()
        .filter(|feature| supported(*feature))
        .collect();
    accepted.sort_unstable();
    accepted.dedup();
    Ok(accepted)
}

pub const PROFILE_RASTER_RGBA8: &str = "raster-rgba8-full-v1";
pub const PROFILE_RASTER_ZSTD: &str = "raster-zstd-full-v1";
pub const PROFILE_IMAGE_PNG_JPEG: &str = "image-png-jpeg-v1";
pub const PROFILE_VIDEO_ACCESS_UNIT: &str = "video-access-unit-v1";
pub const PROFILE_TEXT_ANCHOR_V2: &str = "text-anchor-cell-v2";
pub const PROFILE_VISIBILITY: &str = "visibility-source-v1";
pub const PROFILE_AUDIO_ACCESS_UNIT: &str = "audio-access-unit-v1";
pub const PROFILE_NODE_CLIP_RECT: &str = "node-clip-rect-v1";
pub const PROFILE_DESKTOP_INPUT: &str = "desktop-input-v1";

pub const HID_KEYBOARD_USAGE_MIN: u16 = 0x04;
pub const HID_KEYBOARD_USAGE_MAX: u16 = 0xe7;
pub const POINTER_BUTTON_MAX: u8 = 4;
pub const POINTER_AXIS_MAX: i32 = 12_000;

pub const MAX_AUDIO_EXTRADATA: usize = 64 * 1024;
pub const MAX_CODEC_STRING: usize = 64;
pub const MAX_DECODER_CONFIG: usize = 4096;
pub const MAX_AUDIO_ACCESS_UNIT_BYTES: u32 = 1024 * 1024;
pub const AUDIO_PACKETIZATION_OPUS: &str = "opus-packet-v1";
pub const AUDIO_PACKETIZATION_VORBIS: &str = "vorbis-packet-v1";
pub const AUDIO_PACKETIZATION_FLAC: &str = "flac-frame-v1";

pub const ERROR_AUTH_FAILED: u64 = 1;
pub const ERROR_UNSUPPORTED_VERSION: u64 = 2;
pub const ERROR_UNSUPPORTED_FEATURE: u64 = 3;
pub const ERROR_UNSUPPORTED_CONFIG: u64 = 4;
pub const ERROR_BAD_MESSAGE: u64 = 5;
pub const ERROR_BAD_STATE: u64 = 6;
pub const ERROR_DUPLICATE_ID: u64 = 7;
pub const ERROR_NOT_FOUND: u64 = 8;
pub const ERROR_LIMIT_EXCEEDED: u64 = 9;
pub const ERROR_NO_MEMORY: u64 = 10;
pub const ERROR_FLOW_CONTROL: u64 = 11;
pub const ERROR_HASH_MISMATCH: u64 = 12;
pub const ERROR_NEED_KEYFRAME: u64 = 13;
pub const ERROR_STALE_EPOCH: u64 = 14;

/// `NEED_KEYFRAME` reason codes (specification section 7.8).
pub const KEYFRAME_REASON_INITIAL: u64 = 1;
pub const KEYFRAME_REASON_DECODER_ERROR: u64 = 2;
pub const KEYFRAME_REASON_EPOCH_DISCONTINUITY: u64 = 3;
pub const KEYFRAME_REASON_DEVICE_RESET: u64 = 4;
pub const ERROR_STALE_DISPLAY_GENERATION: u64 = 15;
pub const ERROR_ANCHOR_GONE: u64 = 16;
pub const ERROR_CONTEXT_REVOKED: u64 = 17;
pub const ERROR_DECODER: u64 = 18;
pub const ERROR_DEVICE_LOST: u64 = 19;
pub const ERROR_TIMEOUT: u64 = 20;

pub const PIXEL_FORMAT_RGBA8: u64 = 1;
pub const ALPHA_STRAIGHT: u64 = 1;
pub const ALPHA_PREMULTIPLIED: u64 = 2;
pub const RASTER_FULL_FRAME: u64 = 0;
pub const COMPRESSION_NONE: u64 = 0;
pub const COMPRESSION_RAW_OR_ZSTD: u64 = 1;
pub const RETENTION_NONE: u64 = 0;
pub const RETENTION_DECODED_SOURCE: u64 = 1;
pub const IMAGE_PNG: u64 = 1;
pub const IMAGE_JPEG: u64 = 2;
pub const COLOR_SPACE_SRGB: u64 = 1;
pub const COORDINATE_GRID_CELL: u64 = 1;
pub const COORDINATE_ANCHOR_CELL: u64 = 3;
pub const FIT_CONTAIN: u64 = 2;
pub const SAMPLING_LINEAR: u64 = 1;
pub const TEXT_LAYER_BETWEEN_BACKGROUND_AND_GLYPH: u64 = 1;
pub const BLEND_SOURCE_OVER: u64 = 0;
pub const PRESENT_NEXT_COMPOSITOR_FRAME: u64 = 0;
pub const START_AFTER_MINIMUM_BUFFER: u64 = 1;
pub const LATE_DROP_PRESENTATION: u64 = 1;

/// Conservatively size initial PLAY buffering from one clean control-path RTT sample.
pub fn minimum_buffer_for_rtt(requested_us: u64, rtt_us: Option<u64>) -> u64 {
    rtt_us.map_or(requested_us, |rtt_us| {
        requested_us
            .max(rtt_us.saturating_mul(2).saturating_add(25_000))
            .min(500_000)
    })
}

/// Complete Vivid 1.0 PLAY payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayRequest {
    pub source_id: u64,
    pub start_pts_us: i64,
    pub minimum_buffer_us: u64,
    pub maximum_latency_us: u64,
    pub rate_32_32: i64,
    pub late_policy: u64,
    pub loop_count: u64,
    pub start_policy: u64,
}

impl PlayRequest {
    pub fn baseline(source_id: u64, minimum_buffer_us: u64) -> Self {
        Self {
            source_id,
            start_pts_us: 0,
            minimum_buffer_us,
            maximum_latency_us: 500_000,
            rate_32_32: 1_i64 << 32,
            late_policy: LATE_DROP_PRESENTATION,
            loop_count: 0,
            start_policy: START_AFTER_MINIMUM_BUFFER,
        }
    }

    pub fn validate(self) -> io::Result<Self> {
        if self.source_id == 0 {
            return Err(invalid("PLAY source ID is zero"));
        }
        if self.maximum_latency_us < self.minimum_buffer_us {
            return Err(invalid("PLAY maximum latency is below its minimum buffer"));
        }
        if self.rate_32_32 != 1_i64 << 32
            || self.late_policy != LATE_DROP_PRESENTATION
            || self.loop_count != 0
            || self.start_policy != START_AFTER_MINIMUM_BUFFER
        {
            return Err(invalid("PLAY contains a non-baseline playback policy"));
        }
        Ok(self)
    }
}

#[derive(Debug, Clone)]
pub struct Welcome {
    pub session_id: u64,
    pub session_tag: Vec<u8>,
    pub root_context_id: u64,
    pub display_generation: u64,
    pub viewport_width: u32,
    pub viewport_height: u32,
    pub grid_columns: u64,
    pub grid_rows: u64,
    pub cell_width: u32,
    pub cell_height: u32,
    pub maximum_control_body: u32,
    pub accepted_profiles: Vec<String>,
    pub selected_major: u64,
    pub selected_minor: u64,
    pub accepted_features: Vec<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplayChanged {
    pub display_generation: u64,
    pub viewport_width: u32,
    pub viewport_height: u32,
    pub grid_columns: u32,
    pub grid_rows: u32,
    pub cell_width: u32,
    pub cell_height: u32,
}

#[derive(Debug, Clone)]
pub struct SourceReady {
    pub source_id: u64,
    pub media_ticket: Vec<u8>,
    pub byte_credits: u64,
    pub packet_credits: u64,
    pub fragment_credits: u64,
    pub max_media_body: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorReply {
    pub code: u64,
    pub request_id: u64,
    pub diagnostic: String,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Credits {
    pub bytes: u64,
    pub packets: u64,
    pub fragments: u64,
}

/// Per-source credit ledger shared by producers, presenters, and bridges so grant/consume
/// arithmetic stays checked and identical on every side of the wire.
///
/// One media record costs one packet credit plus its body length in byte credits. Fragmented
/// profiles additionally consume the declared number of fragment slots.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CreditLedger {
    pub bytes: u64,
    pub packets: u64,
    pub fragments: u64,
    lost: bool,
}

impl CreditLedger {
    pub fn new(initial: Credits) -> Self {
        Self {
            bytes: initial.bytes,
            packets: initial.packets,
            fragments: initial.fragments,
            lost: false,
        }
    }

    /// Permanently close this source's credit window after `SOURCE_LOST` or local cancellation.
    pub fn mark_lost(&mut self) {
        self.lost = true;
    }

    pub fn is_lost(&self) -> bool {
        self.lost
    }

    /// Apply a `CREDIT` grant with checked arithmetic; overflow is a protocol violation.
    pub fn grant(&mut self, credits: Credits) -> io::Result<()> {
        if self.lost {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "cannot grant credit to a lost source",
            ));
        }
        let bytes = self
            .bytes
            .checked_add(credits.bytes)
            .ok_or_else(|| invalid("byte credit overflow"))?;
        let packets = self
            .packets
            .checked_add(credits.packets)
            .ok_or_else(|| invalid("packet credit overflow"))?;
        let fragments = self
            .fragments
            .checked_add(credits.fragments)
            .ok_or_else(|| invalid("fragment credit overflow"))?;
        self.bytes = bytes;
        self.packets = packets;
        self.fragments = fragments;
        Ok(())
    }

    /// Whether one unfragmented media record with `body_length` bytes can be sent now.
    pub fn can_consume(&self, body_length: u64) -> bool {
        self.can_consume_with_fragments(body_length, 0)
    }

    /// Whether one media record and its logical fragments fit the current credit window.
    pub fn can_consume_with_fragments(&self, body_length: u64, fragments: u64) -> bool {
        !self.lost && self.bytes >= body_length && self.packets > 0 && self.fragments >= fragments
    }

    /// Consume the credit for one unfragmented media record.
    pub fn consume(&mut self, body_length: u64) -> io::Result<()> {
        self.consume_with_fragments(body_length, 0)
    }

    /// Consume one packet, its body bytes, and `fragments` fragment slots atomically.
    pub fn consume_with_fragments(&mut self, body_length: u64, fragments: u64) -> io::Result<()> {
        if self.lost {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "cannot consume credit for a lost source",
            ));
        }
        if !self.can_consume_with_fragments(body_length, fragments) {
            return Err(invalid("media record exceeds the granted credit window"));
        }
        self.bytes -= body_length;
        self.packets -= 1;
        self.fragments -= fragments;
        Ok(())
    }
}

pub struct VideoSourceConfig<'a> {
    pub source_id: u64,
    pub codec: &'a str,
    pub packetization: &'a str,
    pub extradata: &'a [u8],
    pub width: u32,
    pub height: u32,
    pub profile: i32,
    pub level: i32,
    pub bitrate: i64,
    pub color_primaries: u64,
    pub transfer: u64,
    pub matrix: u64,
    pub range: u64,
    pub sar_num: u32,
    pub sar_den: u32,
    pub max_access_unit_bytes: u32,
    /// Optional RFC 6381 codec string (`decoder-description-v1`). Send only when the presenter
    /// accepted [`FEATURE_DECODER_DESCRIPTION_V1`].
    pub codec_string: Option<&'a str>,
    /// Optional ISO-BMFF decoder configuration box body matching the codec (avcC/hvcC/vpcC/av1C).
    /// Send only when the presenter accepted [`FEATURE_DECODER_DESCRIPTION_V1`].
    pub decoder_config: Option<&'a [u8]>,
}

pub struct AudioSourceConfig<'a> {
    pub source_id: u64,
    pub linked_video_source_id: Option<u64>,
    pub codec: &'a str,
    pub packetization: &'a str,
    pub extradata: &'a [u8],
    pub sample_rate: u32,
    pub channels: u16,
    pub channel_mask: u64,
    pub bitrate: i64,
    pub max_access_unit_bytes: u32,
    /// Optional RFC 6381 codec string (`decoder-description-v1`). Send only when the presenter
    /// accepted [`FEATURE_DECODER_DESCRIPTION_V1`].
    pub codec_string: Option<&'a str>,
}

pub struct NodeConfig {
    pub node_id: u64,
    pub source_id: u64,
    pub context_id: u64,
    pub columns: u32,
    pub rows: u32,
    pub anchor_id: Option<u64>,
}

/// A rectangle in signed 32.32 cell coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClipRect {
    pub x: i64,
    pub y: i64,
    pub width: i64,
    pub height: i64,
}

pub const MAX_SCENE_NODES: usize = 256;
pub const MAX_SCENE_FRAGMENTS_PER_NODE: usize = 8;

/// Stable owner-scoped object identity used by the shared scene snapshot validator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SceneValidationKey {
    pub owner_id: u64,
    pub object_id: u64,
}

/// Source linkage needed for structural scene validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SceneValidationSource {
    pub key: SceneValidationKey,
    pub is_video: bool,
    pub linked_video: Option<SceneValidationKey>,
}

/// One projected fragment of a logical scene node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SceneValidationNode {
    pub owner_id: u64,
    pub node_id: u64,
    pub fragment_id: u64,
    pub source: SceneValidationKey,
    pub x: i64,
    pub y: i64,
    pub width: i64,
    pub height: i64,
    pub clip: Option<ClipRect>,
}

/// Validate one signed 32.32 scene rectangle: positive extent and non-overflowing edges, per the
/// specification's checked-arithmetic rule for scene geometry. Shared by presenters and bridges
/// so every consumer rejects the same degenerate rectangles.
pub fn validate_scene_rect(x: i64, y: i64, width: i64, height: i64) -> io::Result<()> {
    if width <= 0
        || height <= 0
        || x.checked_add(width).is_none()
        || y.checked_add(height).is_none()
    {
        return Err(invalid(
            "scene rectangle has non-positive or overflowing geometry",
        ));
    }
    Ok(())
}

/// Validate the consumer-independent structure of one authoritative scene snapshot.
///
/// Session capability and anchor existence remain consumer-owned checks. This function owns the
/// cross-consumer limits and relationships: unique sources/fragments, linked audio scope,
/// fragment count, source ownership, and checked signed 32.32 geometry.
pub fn validate_scene_snapshot(
    sources: &[SceneValidationSource],
    nodes: &[SceneValidationNode],
) -> io::Result<()> {
    if nodes.len() > MAX_SCENE_NODES {
        return Err(invalid("scene snapshot exceeds the node limit"));
    }

    let source_kinds = sources
        .iter()
        .map(|source| (source.key, source.is_video))
        .collect::<HashMap<_, _>>();
    if source_kinds.len() != sources.len() {
        return Err(invalid("scene snapshot repeats a source key"));
    }
    for source in sources {
        if let Some(video) = source.linked_video
            && (video.owner_id != source.key.owner_id
                || source_kinds.get(&video).copied() != Some(true))
        {
            return Err(invalid(
                "scene audio source references a missing or foreign video source",
            ));
        }
    }

    let mut fragment_keys = HashSet::new();
    let mut logical_counts = HashMap::<(u64, u64), usize>::new();
    for node in nodes {
        if !fragment_keys.insert((node.owner_id, node.node_id, node.fragment_id)) {
            return Err(invalid("scene snapshot repeats a fragment key"));
        }
        let count = logical_counts
            .entry((node.owner_id, node.node_id))
            .or_default();
        *count += 1;
        if *count > MAX_SCENE_FRAGMENTS_PER_NODE {
            return Err(invalid(
                "scene snapshot exceeds the fragment limit for a logical node",
            ));
        }
        if node.owner_id != node.source.owner_id || !source_kinds.contains_key(&node.source) {
            return Err(invalid("scene node references a missing or foreign source"));
        }
        validate_scene_rect(node.x, node.y, node.width, node.height)?;
        if let Some(clip) = node.clip {
            validate_scene_rect(clip.x, clip.y, clip.width, clip.height)?;
        }
    }
    Ok(())
}

/// Complete scene-node configuration for callers that need positioning, ordering, visibility, or
/// clipping. [`NodeConfig`] remains the compact compatibility helper.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SceneNodeConfig {
    pub node_id: u64,
    pub source_id: u64,
    pub context_id: u64,
    pub x: i64,
    pub y: i64,
    pub width: i64,
    pub height: i64,
    pub text_layer: u64,
    pub z_index: i64,
    pub visible: bool,
    pub anchor_id: Option<u64>,
    pub clip: Option<ClipRect>,
}

#[derive(Debug, Clone)]
pub struct ControlEnvelope {
    pub request_id: u64,
    pub transaction_id: Option<u64>,
    pub expected_generation: Option<u64>,
    pub payload: Value,
}

#[derive(Debug, Clone)]
pub struct Hello {
    pub minimum_major: u64,
    pub minimum_minor: u64,
    pub maximum_major: u64,
    pub maximum_minor: u64,
    pub token: String,
    pub producer: String,
    pub producer_version: String,
    pub required_features: Vec<u64>,
    pub optional_features: Vec<u64>,
    pub maximum_record_body: u32,
}

/// Configurable HELLO encoder input. This is used by bridges and presenters that are not Vivi and
/// must advertise their exact feature set.
pub struct HelloConfig<'a> {
    pub minimum_major: u64,
    pub minimum_minor: u64,
    pub maximum_major: u64,
    pub maximum_minor: u64,
    pub token: &'a str,
    pub producer: &'a str,
    pub producer_version: &'a str,
    pub required_features: &'a [u64],
    pub optional_features: &'a [u64],
    pub maximum_record_body: u32,
}

/// Configurable WELCOME encoder input for virtual or alternate presenters.
pub struct WelcomeConfig<'a> {
    pub session_id: u64,
    pub session_tag: &'a [u8; 16],
    pub root_context_id: u64,
    pub capability_generation: u64,
    pub display: DisplayChanged,
    pub maximum_control_body: u32,
    pub accepted_profiles: &'a [&'a str],
    pub selected_major: u64,
    pub selected_minor: u64,
    pub accepted_features: &'a [u64],
}

#[derive(Debug, Clone)]
pub struct RasterSourceConfig {
    pub source_id: u64,
    pub width: u32,
    pub height: u32,
    pub alpha_mode: u64,
    pub compression_mode: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageSourceConfig {
    pub source_id: u64,
    pub encoding: u64,
    pub width: u32,
    pub height: u32,
    pub encoded_length: u32,
    pub sha256: Option<[u8; 32]>,
}

#[derive(Debug, Clone)]
pub struct ParsedVideoSourceConfig {
    pub source_id: u64,
    pub codec: String,
    pub packetization: String,
    pub extradata: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub profile: i32,
    pub level: i32,
    pub bitrate: u64,
    pub color_primaries: u64,
    pub transfer: u64,
    pub matrix: u64,
    pub range: u64,
    pub sar_num: u32,
    pub sar_den: u32,
    pub max_access_unit_bytes: u32,
    pub codec_string: Option<String>,
    pub decoder_config: Option<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedAudioSourceConfig {
    pub source_id: u64,
    pub linked_video_source_id: Option<u64>,
    pub codec: String,
    pub packetization: String,
    pub extradata: Vec<u8>,
    pub sample_rate: u32,
    pub channels: u16,
    pub channel_mask: u64,
    pub bitrate: u64,
    pub max_access_unit_bytes: u32,
    pub codec_string: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Visibility {
    pub visible: bool,
    pub reasons: u64,
    pub display_generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NeedKeyframe {
    pub source_id: u64,
    pub minimum_epoch: u32,
    pub reason: u64,
    pub last_packet_id: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceLost {
    pub source_id: u64,
    pub code: u64,
    pub diagnostic: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyInput {
    pub usage: u16,
    pub pressed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PointerMotion {
    pub source_id: u64,
    pub x: u32,
    pub y: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PointerButton {
    pub source_id: u64,
    pub button: u8,
    pub pressed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PointerAxis {
    pub source_id: u64,
    pub horizontal_120: i32,
    pub vertical_120: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedNodeConfig {
    pub node_id: u64,
    pub source_id: u64,
    pub context_id: u64,
    pub x: i64,
    pub y: i64,
    pub width: i64,
    pub height: i64,
    pub text_layer: u64,
    pub z_index: i64,
    pub visible: bool,
    pub anchor_id: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedSceneNode {
    pub node: ParsedNodeConfig,
    pub clip: Option<ClipRect>,
}

pub fn hello(request_id: u64, token: &str) -> Vec<u8> {
    const REQUIRED: &[u64] = &[
        FEATURE_RASTER_RGBA8,
        FEATURE_SCENE_TRANSACTIONS,
        FEATURE_GRID_CELL_NODES,
        FEATURE_CREDIT_FLOW_CONTROL,
        FEATURE_TEXT_ANCHORS_V2,
    ];
    const OPTIONAL: &[u64] = &[
        FEATURE_ENCODED_IMAGE_V1,
        FEATURE_RASTER_ZSTD_V1,
        FEATURE_RASTER_PREMULTIPLIED_ALPHA,
        FEATURE_VISIBILITY_EVENTS_V1,
        FEATURE_VIDEO_ACCESS_UNIT_V1,
        FEATURE_VIDEO_CONTROL_V1,
        FEATURE_AUDIO_ACCESS_UNIT_V1,
        FEATURE_DECODER_DESCRIPTION_V1,
    ];
    encode_hello(
        request_id,
        &HelloConfig {
            minimum_major: u64::from(VIVID_MAJOR),
            minimum_minor: u64::from(VIVID_MINOR),
            maximum_major: u64::from(VIVID_MAJOR),
            maximum_minor: u64::from(VIVID_MINOR),
            token,
            producer: "vivi",
            producer_version: env!("CARGO_PKG_VERSION"),
            required_features: REQUIRED,
            optional_features: OPTIONAL,
            maximum_record_body: super::CONTROL_MAX_RECORD_BODY,
        },
    )
}

pub fn encode_hello(request_id: u64, config: &HelloConfig<'_>) -> Vec<u8> {
    envelope(request_id, None, None, |encoder| {
        encoder.map(10);
        key_u64(encoder, 0, config.minimum_major);
        key_u64(encoder, 1, config.minimum_minor);
        key_u64(encoder, 2, config.maximum_major);
        key_u64(encoder, 3, config.maximum_minor);
        encoder.u64(4);
        encoder.text(config.token);
        encoder.u64(5);
        encoder.text(config.producer);
        encoder.u64(6);
        encoder.text(config.producer_version);
        encoder.u64(7);
        encoder.array(config.required_features.len());
        for feature in config.required_features {
            encoder.u64(*feature);
        }
        encoder.u64(8);
        encoder.array(config.optional_features.len());
        for feature in config.optional_features {
            encoder.u64(*feature);
        }
        key_u64(encoder, 9, u64::from(config.maximum_record_body));
    })
}

pub fn create_raster(request_id: u64, source_id: u64, width: u32, height: u32) -> Vec<u8> {
    create_raster_config(
        request_id,
        &RasterSourceConfig {
            source_id,
            width,
            height,
            alpha_mode: ALPHA_STRAIGHT,
            compression_mode: COMPRESSION_RAW_OR_ZSTD,
        },
    )
}

pub fn create_raster_config(request_id: u64, config: &RasterSourceConfig) -> Vec<u8> {
    envelope(request_id, None, None, |encoder| {
        encoder.map(9);
        key_u64(encoder, 0, config.source_id);
        key_u64(encoder, 1, u64::from(config.width));
        key_u64(encoder, 2, u64::from(config.height));
        key_u64(encoder, 3, PIXEL_FORMAT_RGBA8);
        key_u64(encoder, 4, config.alpha_mode);
        key_u64(encoder, 5, RASTER_FULL_FRAME);
        key_u64(encoder, 6, 1);
        key_u64(encoder, 7, config.compression_mode);
        key_u64(encoder, 8, RETENTION_NONE);
    })
}

pub fn create_image(request_id: u64, config: &ImageSourceConfig) -> Vec<u8> {
    envelope(request_id, None, None, |encoder| {
        encoder.map(if config.sha256.is_some() { 8 } else { 7 });
        key_u64(encoder, 0, config.source_id);
        key_u64(encoder, 1, config.encoding);
        key_u64(encoder, 2, u64::from(config.width));
        key_u64(encoder, 3, u64::from(config.height));
        key_u64(encoder, 4, u64::from(config.encoded_length));
        if let Some(hash) = config.sha256 {
            encoder.u64(5);
            encoder.bytes(&hash);
        }
        key_u64(encoder, 6, COLOR_SPACE_SRGB);
        key_u64(encoder, 7, RETENTION_DECODED_SOURCE);
    })
}

pub fn create_video(request_id: u64, config: &VideoSourceConfig<'_>) -> Vec<u8> {
    envelope(request_id, None, None, |encoder| {
        let optional = usize::from(config.codec_string.is_some())
            + usize::from(config.decoder_config.is_some());
        encoder.map(21 + optional);
        key_u64(encoder, 0, config.source_id);
        encoder.u64(1);
        encoder.text(config.codec);
        encoder.u64(2);
        encoder.text(config.packetization);
        encoder.u64(3);
        encoder.bytes(config.extradata);
        key_u64(encoder, 4, u64::from(config.width));
        key_u64(encoder, 5, u64::from(config.height));
        key_i64(encoder, 6, i64::from(config.profile));
        key_i64(encoder, 7, i64::from(config.level));
        key_u64(encoder, 8, 0); // alpha: none
        key_u64(encoder, 9, 0); // latency: normal playback
        key_u64(encoder, 10, RETENTION_NONE);
        key_i64(encoder, 11, config.bitrate.max(0));
        key_u64(encoder, 12, 16); // maximum reorder depth
        encoder.u64(13);
        encoder.text("source-timebase-us");
        key_u64(encoder, 14, config.color_primaries);
        key_u64(encoder, 15, config.transfer);
        key_u64(encoder, 16, config.matrix);
        key_u64(encoder, 17, config.range);
        key_u64(encoder, 18, u64::from(config.sar_num));
        key_u64(encoder, 19, u64::from(config.sar_den));
        key_u64(encoder, 20, u64::from(config.max_access_unit_bytes));
        if let Some(codec_string) = config.codec_string {
            encoder.u64(21);
            encoder.text(codec_string);
        }
        if let Some(decoder_config) = config.decoder_config {
            encoder.u64(22);
            encoder.bytes(decoder_config);
        }
    })
}

pub fn probe_video_config(request_id: u64, config: &VideoSourceConfig<'_>) -> Vec<u8> {
    create_video(request_id, config)
}

pub fn create_audio(request_id: u64, config: &AudioSourceConfig<'_>) -> Vec<u8> {
    envelope(request_id, None, None, |encoder| {
        encoder.map(11 + usize::from(config.codec_string.is_some()));
        key_u64(encoder, 0, config.source_id);
        key_u64(encoder, 1, config.linked_video_source_id.unwrap_or(0));
        encoder.u64(2);
        encoder.text(config.codec);
        encoder.u64(3);
        encoder.text(config.packetization);
        encoder.u64(4);
        encoder.bytes(config.extradata);
        key_u64(encoder, 5, u64::from(config.sample_rate));
        key_u64(encoder, 6, u64::from(config.channels));
        key_u64(encoder, 7, config.channel_mask);
        key_i64(encoder, 8, config.bitrate.max(0));
        key_u64(encoder, 9, u64::from(config.max_access_unit_bytes));
        encoder.u64(10);
        encoder.text("source-timebase-us");
        if let Some(codec_string) = config.codec_string {
            encoder.u64(11);
            encoder.text(codec_string);
        }
    })
}

pub fn probe_audio_config(request_id: u64, config: &AudioSourceConfig<'_>) -> Vec<u8> {
    create_audio(request_id, config)
}

pub fn audio_support(request_id: u64, supported: bool, decoder: &str) -> Vec<u8> {
    video_support(request_id, supported, decoder)
}

pub fn parse_audio_support(body: &[u8]) -> io::Result<bool> {
    parse_video_support(body)
}

pub fn video_support(request_id: u64, supported: bool, decoder: &str) -> Vec<u8> {
    envelope(request_id, None, None, |encoder| {
        encoder.map(2);
        encoder.u64(0);
        encoder.bool(supported);
        encoder.u64(1);
        encoder.text(decoder);
    })
}

pub fn parse_video_support(body: &[u8]) -> io::Result<bool> {
    let (_, payload) = decode_envelope(body)?;
    payload
        .map_value(0)
        .and_then(Value::as_bool)
        .ok_or_else(|| invalid("VIDEO_SUPPORT is missing its supported flag"))
}

pub fn begin_transaction(request_id: u64, transaction_id: u64) -> Vec<u8> {
    envelope(request_id, Some(transaction_id), None, |encoder| {
        encoder.map(1);
        key_u64(encoder, 0, transaction_id);
    })
}

pub fn create_node(request_id: u64, transaction_id: u64, node: NodeConfig) -> Vec<u8> {
    create_node_at(request_id, transaction_id, node, 0, 0)
}

pub fn create_node_at(
    request_id: u64,
    transaction_id: u64,
    node: NodeConfig,
    x: i64,
    y: i64,
) -> Vec<u8> {
    create_scene_node(
        request_id,
        transaction_id,
        &SceneNodeConfig {
            node_id: node.node_id,
            source_id: node.source_id,
            context_id: node.context_id,
            x,
            y,
            width: fixed_cells(node.columns),
            height: fixed_cells(node.rows),
            text_layer: TEXT_LAYER_BETWEEN_BACKGROUND_AND_GLYPH,
            z_index: 0,
            visible: true,
            anchor_id: node.anchor_id,
            clip: None,
        },
    )
}

pub fn create_scene_node(request_id: u64, transaction_id: u64, node: &SceneNodeConfig) -> Vec<u8> {
    envelope(request_id, Some(transaction_id), None, |encoder| {
        let field_count =
            14 + usize::from(node.anchor_id.is_some()) + 4 * usize::from(node.clip.is_some());
        encoder.map(field_count);
        key_u64(encoder, 0, node.node_id);
        key_u64(encoder, 1, node.source_id);
        key_u64(encoder, 2, node.context_id);
        key_u64(
            encoder,
            3,
            if node.anchor_id.is_some() {
                COORDINATE_ANCHOR_CELL
            } else {
                COORDINATE_GRID_CELL
            },
        );
        key_i64(encoder, 4, node.x);
        key_i64(encoder, 5, node.y);
        key_i64(encoder, 6, node.width);
        key_i64(encoder, 7, node.height);
        key_u64(encoder, 8, FIT_CONTAIN);
        key_u64(encoder, 9, SAMPLING_LINEAR);
        key_u64(encoder, 10, node.text_layer);
        key_i64(encoder, 11, node.z_index);
        key_u64(encoder, 12, BLEND_SOURCE_OVER);
        encoder.u64(13);
        encoder.bool(node.visible);
        if let Some(anchor_id) = node.anchor_id {
            key_u64(encoder, 14, anchor_id);
        }
        if let Some(clip) = node.clip {
            key_i64(encoder, 15, clip.x);
            key_i64(encoder, 16, clip.y);
            key_i64(encoder, 17, clip.width);
            key_i64(encoder, 18, clip.height);
        }
    })
}

pub fn anchor_event(anchor_id: u64) -> Vec<u8> {
    envelope(0, None, None, |encoder| {
        encoder.map(1);
        key_u64(encoder, 0, anchor_id);
    })
}

pub fn delete_node(request_id: u64, transaction_id: u64, node_id: u64) -> Vec<u8> {
    envelope(request_id, Some(transaction_id), None, |encoder| {
        encoder.map(1);
        key_u64(encoder, 0, node_id);
    })
}

pub fn destroy_source(request_id: u64, source_id: u64) -> Vec<u8> {
    envelope(request_id, None, None, |encoder| {
        encoder.map(1);
        key_u64(encoder, 0, source_id);
    })
}

pub fn abort_transaction(request_id: u64, transaction_id: u64) -> Vec<u8> {
    envelope(request_id, Some(transaction_id), None, |encoder| {
        encoder.map(1);
        key_u64(encoder, 0, transaction_id);
    })
}

pub fn commit_transaction(
    request_id: u64,
    transaction_id: u64,
    display_generation: u64,
) -> Vec<u8> {
    envelope(
        request_id,
        Some(transaction_id),
        Some(display_generation),
        |encoder| {
            encoder.map(2);
            key_u64(encoder, 0, PRESENT_NEXT_COMPOSITOR_FRAME);
            encoder.u64(1);
            encoder.bool(true);
        },
    )
}

pub fn play_request(request_id: u64, request: &PlayRequest) -> Vec<u8> {
    envelope(request_id, None, None, |encoder| {
        encoder.map(8);
        key_u64(encoder, 0, request.source_id);
        key_i64(encoder, 1, request.start_pts_us);
        key_u64(encoder, 2, request.minimum_buffer_us);
        key_u64(encoder, 3, request.maximum_latency_us);
        key_i64(encoder, 4, request.rate_32_32);
        key_u64(encoder, 5, request.late_policy);
        key_u64(encoder, 6, request.loop_count);
        key_u64(encoder, 7, request.start_policy);
    })
}

pub fn play(request_id: u64, source_id: u64, minimum_buffer_us: u64) -> Vec<u8> {
    play_request(
        request_id,
        &PlayRequest::baseline(source_id, minimum_buffer_us),
    )
}

pub fn eos(request_id: u64, source_id: u64, epoch: u32) -> Vec<u8> {
    envelope(request_id, None, None, |encoder| {
        encoder.map(2);
        key_u64(encoder, 0, source_id);
        key_u64(encoder, 1, u64::from(epoch));
    })
}

pub fn drain(request_id: u64, source_id: u64) -> Vec<u8> {
    envelope(request_id, None, None, |encoder| {
        encoder.map(1);
        key_u64(encoder, 0, source_id);
    })
}

pub fn goodbye(request_id: u64) -> Vec<u8> {
    envelope(request_id, None, None, |encoder| {
        encoder.map(0);
    })
}

pub fn attach_channel(ticket: &[u8]) -> Vec<u8> {
    envelope(0, None, None, |encoder| {
        encoder.map(1);
        encoder.u64(0);
        encoder.bytes(ticket);
    })
}

pub fn welcome(
    request_id: u64,
    session_id: u64,
    session_tag: &[u8; 16],
    root_context_id: u64,
    display: DisplayChanged,
    accepted_features: &[u64],
) -> Vec<u8> {
    const BASE_PROFILES: &[&str] = &[
        PROFILE_AUDIO_ACCESS_UNIT,
        PROFILE_IMAGE_PNG_JPEG,
        PROFILE_RASTER_RGBA8,
        PROFILE_RASTER_ZSTD,
        PROFILE_TEXT_ANCHOR_V2,
        PROFILE_VIDEO_ACCESS_UNIT,
        PROFILE_VISIBILITY,
    ];
    let mut profiles = BASE_PROFILES.to_vec();
    if accepted_features.contains(&FEATURE_DESKTOP_INPUT_V1) {
        profiles.insert(1, PROFILE_DESKTOP_INPUT);
    }
    if accepted_features.contains(&FEATURE_NODE_CLIP_RECT_V1) {
        let index = profiles
            .binary_search(&PROFILE_NODE_CLIP_RECT)
            .unwrap_or_else(|index| index);
        profiles.insert(index, PROFILE_NODE_CLIP_RECT);
    }
    encode_welcome(
        request_id,
        &WelcomeConfig {
            session_id,
            session_tag,
            root_context_id,
            capability_generation: 1,
            display,
            maximum_control_body: super::CONTROL_MAX_RECORD_BODY,
            accepted_profiles: &profiles,
            selected_major: u64::from(VIVID_MAJOR),
            selected_minor: u64::from(VIVID_MINOR),
            accepted_features,
        },
    )
}

pub fn encode_welcome(request_id: u64, config: &WelcomeConfig<'_>) -> Vec<u8> {
    envelope(request_id, None, None, |encoder| {
        encoder.map(16);
        key_u64(encoder, 0, config.session_id);
        encoder.u64(1);
        encoder.bytes(config.session_tag);
        key_u64(encoder, 2, config.root_context_id);
        key_u64(encoder, 3, config.capability_generation);
        key_u64(encoder, 4, config.display.display_generation);
        key_u64(encoder, 5, u64::from(config.display.viewport_width));
        key_u64(encoder, 6, u64::from(config.display.viewport_height));
        key_u64(encoder, 7, u64::from(config.display.grid_columns));
        key_u64(encoder, 8, u64::from(config.display.grid_rows));
        key_u64(encoder, 9, u64::from(config.display.cell_width));
        key_u64(encoder, 10, u64::from(config.display.cell_height));
        key_u64(encoder, 11, u64::from(config.maximum_control_body));
        encoder.u64(12);
        encoder.array(config.accepted_profiles.len());
        for profile in config.accepted_profiles {
            encoder.text(profile);
        }
        key_u64(encoder, 13, config.selected_major);
        key_u64(encoder, 14, config.selected_minor);
        encoder.u64(15);
        encoder.array(config.accepted_features.len());
        for feature in config.accepted_features {
            encoder.u64(*feature);
        }
    })
}

pub fn display_changed(request_id: u64, display: DisplayChanged) -> Vec<u8> {
    envelope(request_id, None, None, |encoder| {
        encoder.map(7);
        key_u64(encoder, 0, display.display_generation);
        key_u64(encoder, 1, u64::from(display.viewport_width));
        key_u64(encoder, 2, u64::from(display.viewport_height));
        key_u64(encoder, 3, u64::from(display.grid_columns));
        key_u64(encoder, 4, u64::from(display.grid_rows));
        key_u64(encoder, 5, u64::from(display.cell_width));
        key_u64(encoder, 6, u64::from(display.cell_height));
    })
}

pub fn source_ready(
    request_id: u64,
    source_id: u64,
    ticket: &[u8],
    credits: Credits,
    max_media_body: u32,
) -> Vec<u8> {
    envelope(request_id, None, None, |encoder| {
        encoder.map(6);
        key_u64(encoder, 0, source_id);
        encoder.u64(1);
        encoder.bytes(ticket);
        key_u64(encoder, 2, credits.bytes);
        key_u64(encoder, 3, credits.packets);
        key_u64(encoder, 4, credits.fragments);
        key_u64(encoder, 5, u64::from(max_media_body));
    })
}

pub fn pause(request_id: u64, source_id: u64) -> Vec<u8> {
    envelope(request_id, None, None, |encoder| {
        encoder.map(1);
        key_u64(encoder, 0, source_id);
    })
}

pub fn flush(request_id: u64, source_id: u64, epoch: u32) -> Vec<u8> {
    envelope(request_id, None, None, |encoder| {
        encoder.map(2);
        key_u64(encoder, 0, source_id);
        key_u64(encoder, 1, u64::from(epoch));
    })
}

pub fn visibility(source_id: u64, visible: bool, reasons: u64, generation: u64) -> Vec<u8> {
    let mut output = Vec::new();
    visibility_into(&mut output, source_id, visible, reasons, generation);
    output
}

pub fn visibility_into(
    output: &mut Vec<u8>,
    source_id: u64,
    visible: bool,
    reasons: u64,
    generation: u64,
) {
    let _ = source_id;
    envelope_into(output, 0, None, None, |encoder| {
        encoder.map(3);
        encoder.u64(0);
        encoder.bool(visible);
        key_u64(encoder, 1, reasons);
        key_u64(encoder, 2, generation);
    })
}

pub fn need_keyframe(
    source_id: u64,
    minimum_epoch: u32,
    reason: u64,
    last_packet_id: Option<u64>,
) -> Vec<u8> {
    envelope(0, None, None, |encoder| {
        encoder.map(if last_packet_id.is_some() { 4 } else { 3 });
        key_u64(encoder, 0, source_id);
        key_u64(encoder, 1, u64::from(minimum_epoch));
        key_u64(encoder, 2, reason);
        if let Some(id) = last_packet_id {
            key_u64(encoder, 3, id);
        }
    })
}

pub fn source_lost(source_id: u64, code: u64, diagnostic: &str) -> Vec<u8> {
    envelope(0, None, None, |encoder| {
        encoder.map(3);
        key_u64(encoder, 0, source_id);
        key_u64(encoder, 1, code);
        encoder.u64(2);
        encoder.text(truncate_utf8(diagnostic, 4096));
    })
}

pub fn ok(request_id: u64) -> Vec<u8> {
    let mut output = Vec::new();
    ok_into(&mut output, request_id);
    output
}

pub fn ok_into(output: &mut Vec<u8>, request_id: u64) {
    envelope_into(output, request_id, None, None, |encoder| encoder.map(0));
}

pub fn error(request_id: u64, code: u64, diagnostic: &str) -> Vec<u8> {
    envelope(request_id, None, None, |encoder| {
        encoder.map(4);
        key_u64(encoder, 0, code);
        key_u64(encoder, 1, request_id);
        encoder.u64(4);
        encoder.bool(false);
        encoder.u64(5);
        encoder.text(truncate_utf8(diagnostic, 4096));
    })
}

pub fn credit(bytes: u64, packets: u64, fragments: u64) -> Vec<u8> {
    let mut output = Vec::new();
    credit_into(&mut output, bytes, packets, fragments);
    output
}

pub fn credit_into(output: &mut Vec<u8>, bytes: u64, packets: u64, fragments: u64) {
    envelope_into(output, 0, None, None, |encoder| {
        encoder.map(3);
        key_u64(encoder, 0, bytes);
        key_u64(encoder, 1, packets);
        key_u64(encoder, 2, fragments);
    });
}

pub fn ping(request_id: u64) -> Vec<u8> {
    let mut output = Vec::new();
    ping_into(&mut output, request_id);
    output
}

pub fn ping_into(output: &mut Vec<u8>, request_id: u64) {
    envelope_into(output, request_id, None, None, |encoder| encoder.map(0));
}

pub fn pong(request_id: u64) -> Vec<u8> {
    let mut output = Vec::new();
    pong_into(&mut output, request_id);
    output
}

pub fn pong_into(output: &mut Vec<u8>, request_id: u64) {
    envelope_into(output, request_id, None, None, |encoder| encoder.map(0));
}

pub fn key_input(usage: u16, pressed: bool) -> Vec<u8> {
    envelope(0, None, None, |encoder| {
        encoder.map(2);
        key_u64(encoder, 0, u64::from(usage));
        encoder.u64(1);
        encoder.bool(pressed);
    })
}

pub fn pointer_motion(source_id: u64, x: u32, y: u32) -> Vec<u8> {
    envelope(0, None, None, |encoder| {
        encoder.map(3);
        key_u64(encoder, 0, source_id);
        key_u64(encoder, 1, u64::from(x));
        key_u64(encoder, 2, u64::from(y));
    })
}

pub fn pointer_button(source_id: u64, button: u8, pressed: bool) -> Vec<u8> {
    envelope(0, None, None, |encoder| {
        encoder.map(3);
        key_u64(encoder, 0, source_id);
        key_u64(encoder, 1, u64::from(button));
        encoder.u64(2);
        encoder.bool(pressed);
    })
}

pub fn pointer_axis(source_id: u64, horizontal_120: i32, vertical_120: i32) -> Vec<u8> {
    envelope(0, None, None, |encoder| {
        encoder.map(3);
        key_u64(encoder, 0, source_id);
        key_i64(encoder, 1, i64::from(horizontal_120));
        key_i64(encoder, 2, i64::from(vertical_120));
    })
}

pub fn input_reset() -> Vec<u8> {
    envelope(0, None, None, |encoder| encoder.map(0))
}

pub fn decode_control(body: &[u8]) -> io::Result<ControlEnvelope> {
    let value = cbor::decode(body).map_err(invalid_data)?;
    if !matches!(value, Value::Map(_)) {
        return Err(invalid("control envelope is not a map"));
    }
    let payload = value
        .map_value(3)
        .cloned()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing control payload"))?;
    if !matches!(payload, Value::Map(_)) {
        return Err(invalid("control payload is not a map"));
    }
    reject_unknown_fields(&value, &[0, 1, 2, 3])?;
    Ok(ControlEnvelope {
        request_id: required_u64(&value, 0, "request ID")?,
        transaction_id: value
            .map_value(1)
            .map(|value| {
                value.as_u64().ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidData, "transaction ID is not unsigned")
                })
            })
            .transpose()?,
        expected_generation: value
            .map_value(2)
            .map(|value| {
                value.as_u64().ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        "display generation is not unsigned",
                    )
                })
            })
            .transpose()?,
        payload,
    })
}

pub fn parse_hello(body: &[u8]) -> io::Result<(u64, Hello)> {
    let envelope = decode_control(body)?;
    let payload = &envelope.payload;
    reject_unknown_fields(payload, &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9])?;
    let maximum_record_body = u32::try_from(required_u64(payload, 9, "maximum record body")?)
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "maximum record body exceeds u32",
            )
        })?;
    let hello = Hello {
        minimum_major: required_u64(payload, 0, "minimum major version")?,
        minimum_minor: required_u64(payload, 1, "minimum minor version")?,
        maximum_major: required_u64(payload, 2, "maximum major version")?,
        maximum_minor: required_u64(payload, 3, "maximum minor version")?,
        token: required_text(payload, 4, "authentication token")?.to_owned(),
        producer: bounded_text(payload, 5, "producer name", 256)?.to_owned(),
        producer_version: bounded_text(payload, 6, "producer version", 128)?.to_owned(),
        required_features: feature_array(payload, 7, "required features")?,
        optional_features: feature_array(payload, 8, "optional features")?,
        maximum_record_body,
    };
    if (hello.minimum_major, hello.minimum_minor) > (hello.maximum_major, hello.maximum_minor) {
        return Err(invalid("HELLO Vivid version range is reversed"));
    }
    if hello.token.len() != 64 || !hello.token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(invalid(
            "HELLO token is not exactly 64 hexadecimal characters",
        ));
    }
    if hello
        .required_features
        .iter()
        .any(|feature| hello.optional_features.binary_search(feature).is_ok())
    {
        return Err(invalid("HELLO required and optional feature sets overlap"));
    }
    Ok((envelope.request_id, hello))
}

pub fn parse_create_raster(body: &[u8]) -> io::Result<(ControlEnvelope, RasterSourceConfig)> {
    let envelope = decode_control(body)?;
    let payload = &envelope.payload;
    reject_unknown_fields(payload, &[0, 1, 2, 3, 4, 5, 6, 7, 8])?;
    let config = RasterSourceConfig {
        source_id: required_u64(payload, 0, "source ID")?,
        width: required_u32(payload, 1, "raster width")?,
        height: required_u32(payload, 2, "raster height")?,
        alpha_mode: required_u64(payload, 4, "alpha mode")?,
        compression_mode: required_u64(payload, 7, "compression")?,
    };
    if config.source_id == 0 {
        return Err(invalid("raster source ID is zero"));
    }
    if required_u64(payload, 3, "pixel format")? != PIXEL_FORMAT_RGBA8
        || !matches!(config.alpha_mode, ALPHA_STRAIGHT | ALPHA_PREMULTIPLIED)
        || required_u64(payload, 5, "raster mode")? != RASTER_FULL_FRAME
        || required_u64(payload, 6, "rectangle limit")? != 1
        || !matches!(
            config.compression_mode,
            COMPRESSION_NONE | COMPRESSION_RAW_OR_ZSTD
        )
        || required_u64(payload, 8, "retention")? != RETENTION_NONE
    {
        return Err(invalid("unsupported raster configuration"));
    }
    if config.width == 0 || config.height == 0 || config.width > 8192 || config.height > 8192 {
        return Err(invalid("raster dimensions are outside Vivid v1 limits"));
    }
    Ok((envelope, config))
}

pub fn parse_create_image(body: &[u8]) -> io::Result<(ControlEnvelope, ImageSourceConfig)> {
    let envelope = decode_control(body)?;
    let payload = &envelope.payload;
    reject_unknown_fields(payload, &[0, 1, 2, 3, 4, 5, 6, 7])?;
    let hash = payload
        .map_value(5)
        .map(|value| {
            let bytes = value
                .as_bytes()
                .ok_or_else(|| invalid("image hash is not bytes"))?;
            bytes
                .try_into()
                .map_err(|_| invalid("image hash is not 32 bytes"))
        })
        .transpose()?;
    let config = ImageSourceConfig {
        source_id: required_u64(payload, 0, "source ID")?,
        encoding: required_u64(payload, 1, "image encoding")?,
        width: required_u32(payload, 2, "image width")?,
        height: required_u32(payload, 3, "image height")?,
        encoded_length: required_u32(payload, 4, "encoded image length")?,
        sha256: hash,
    };
    if config.source_id == 0
        || !matches!(config.encoding, IMAGE_PNG | IMAGE_JPEG)
        || config.width == 0
        || config.height == 0
        || config.width > 8192
        || config.height > 8192
        || config.encoded_length == 0
        || required_u64(payload, 6, "color space")? != COLOR_SPACE_SRGB
        || required_u64(payload, 7, "retention")? != RETENTION_DECODED_SOURCE
    {
        return Err(invalid("unsupported encoded-image configuration"));
    }
    Ok((envelope, config))
}

pub fn parse_create_video(body: &[u8]) -> io::Result<(ControlEnvelope, ParsedVideoSourceConfig)> {
    let envelope = decode_control(body)?;
    let payload = &envelope.payload;
    reject_unknown_fields(
        payload,
        &[
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22,
        ],
    )?;
    let profile = required_i64(payload, 6, "video profile")?;
    let level = required_i64(payload, 7, "video level")?;
    let codec_string = match payload.map_value(21) {
        None => None,
        Some(value) => Some(
            value
                .as_text()
                .ok_or_else(|| invalid("video codec string is not text"))?
                .to_owned(),
        ),
    };
    let decoder_config = match payload.map_value(22) {
        None => None,
        Some(value) => Some(
            value
                .as_bytes()
                .ok_or_else(|| invalid("video decoder configuration is not bytes"))?
                .to_vec(),
        ),
    };
    let config = ParsedVideoSourceConfig {
        source_id: required_u64(payload, 0, "source ID")?,
        codec: required_text(payload, 1, "codec")?.to_owned(),
        packetization: required_text(payload, 2, "packetization")?.to_owned(),
        extradata: required_bytes(payload, 3, "extradata")?.to_vec(),
        width: required_u32(payload, 4, "coded width")?,
        height: required_u32(payload, 5, "coded height")?,
        profile: i32::try_from(profile).map_err(|_| invalid("video profile exceeds i32"))?,
        level: i32::try_from(level).map_err(|_| invalid("video level exceeds i32"))?,
        bitrate: required_u64(payload, 11, "video bitrate")?,
        color_primaries: required_u64(payload, 14, "color primaries")?,
        transfer: required_u64(payload, 15, "transfer characteristic")?,
        matrix: required_u64(payload, 16, "matrix coefficients")?,
        range: required_u64(payload, 17, "signal range")?,
        sar_num: required_u32(payload, 18, "sample aspect ratio numerator")?,
        sar_den: required_u32(payload, 19, "sample aspect ratio denominator")?,
        max_access_unit_bytes: required_u32(payload, 20, "maximum access-unit bytes")?,
        codec_string,
        decoder_config,
    };
    if let Some(codec_string) = &config.codec_string {
        validate_video_codec_string(&config.codec, codec_string)?;
    }
    if let Some(decoder_config) = &config.decoder_config {
        validate_video_decoder_config(&config.codec, decoder_config)?;
    }
    if config.width == 0 || config.height == 0 || config.width > 8192 || config.height > 8192 {
        return Err(invalid("video dimensions are outside Vivid v1 limits"));
    }
    if required_u64(payload, 8, "alpha mode")? != 0
        || required_u64(payload, 9, "latency mode")? != 0
        || required_u64(payload, 10, "retention mode")? != RETENTION_NONE
        || required_u64(payload, 12, "reorder depth")? > 64
        || required_text(payload, 13, "timeline name")? != "source-timebase-us"
        || config.sar_num == 0
        || config.sar_den == 0
        || config.max_access_unit_bytes == 0
        || !matches!(config.color_primaries, 1..=4)
        || !matches!(config.transfer, 1..=2)
        || !matches!(config.matrix, 0..=3)
        || !matches!(config.range, 1..=2)
    {
        return Err(invalid("unsupported video configuration"));
    }
    Ok((envelope, config))
}

pub fn parse_create_audio(body: &[u8]) -> io::Result<(ControlEnvelope, ParsedAudioSourceConfig)> {
    let envelope = decode_control(body)?;
    let payload = &envelope.payload;
    reject_unknown_fields(payload, &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11])?;
    let channels = required_u32(payload, 6, "audio channel count")?;
    let linked = required_u64(payload, 1, "linked video source ID")?;
    let codec_string = match payload.map_value(11) {
        None => None,
        Some(value) => Some(
            value
                .as_text()
                .ok_or_else(|| invalid("audio codec string is not text"))?
                .to_owned(),
        ),
    };
    let config = ParsedAudioSourceConfig {
        source_id: required_u64(payload, 0, "source ID")?,
        linked_video_source_id: (linked != 0).then_some(linked),
        codec: bounded_text(payload, 2, "audio codec", 64)?.to_owned(),
        packetization: bounded_text(payload, 3, "audio packetization", 64)?.to_owned(),
        extradata: required_bytes(payload, 4, "audio extradata")?.to_vec(),
        sample_rate: required_u32(payload, 5, "audio sample rate")?,
        channels: u16::try_from(channels)
            .map_err(|_| invalid("audio channel count exceeds u16"))?,
        channel_mask: required_u64(payload, 7, "audio channel mask")?,
        bitrate: required_u64(payload, 8, "audio bitrate")?,
        max_access_unit_bytes: required_u32(payload, 9, "maximum audio access-unit bytes")?,
        codec_string,
    };
    if config.linked_video_source_id == Some(config.source_id) {
        return Err(invalid("audio source cannot link to itself"));
    }
    if required_text(payload, 10, "audio timeline")? != "source-timebase-us" {
        return Err(invalid("unsupported audio configuration"));
    }
    if let Some(codec_string) = &config.codec_string {
        validate_audio_codec_string(&config.codec, codec_string)?;
    }
    Ok((envelope, config))
}

pub fn audio_config_supported(config: &ParsedAudioSourceConfig) -> bool {
    (8_000..=192_000).contains(&config.sample_rate)
        && (1..=8).contains(&config.channels)
        && (config.channel_mask == 0
            || config.channel_mask.count_ones() == u32::from(config.channels))
        && config.extradata.len() <= MAX_AUDIO_EXTRADATA
        && config.max_access_unit_bytes > 0
        && config.max_access_unit_bytes <= MAX_AUDIO_ACCESS_UNIT_BYTES
        && validate_audio_initialization(
            &config.codec,
            &config.packetization,
            &config.extradata,
            config.sample_rate,
            config.channels,
        )
        .is_ok()
}

pub fn valid_audio_packetization(codec: &str, packetization: &str) -> bool {
    match codec {
        "mp3" => packetization == "mp3-frame-v1",
        "aac" => packetization == "aac-raw-au-v1",
        "alac" => packetization == "alac-frame-v1",
        "opus" => packetization == AUDIO_PACKETIZATION_OPUS,
        "vorbis" => packetization == AUDIO_PACKETIZATION_VORBIS,
        "flac" => packetization == AUDIO_PACKETIZATION_FLAC,
        "pcm_u8" | "pcm_s16le" | "pcm_s24le" | "pcm_s32le" | "pcm_f32le" | "pcm_f64le"
        | "pcm_mulaw" | "pcm_alaw" => packetization == "pcm-packet-v1",
        _ => false,
    }
}

/// Validate an optional RFC 6381 codec string against a CREATE_VIDEO codec key
/// (`decoder-description-v1`).
pub fn validate_video_codec_string(codec: &str, codec_string: &str) -> io::Result<()> {
    validate_codec_string_shape(codec_string)?;
    let family_matches = match codec {
        "h264" => codec_string.starts_with("avc1.") || codec_string.starts_with("avc3."),
        "hevc" => codec_string.starts_with("hvc1.") || codec_string.starts_with("hev1."),
        "vp9" => codec_string.starts_with("vp09."),
        "av1" => codec_string.starts_with("av01."),
        _ => false,
    };
    if !family_matches {
        return Err(invalid(
            "video codec string family does not match the codec",
        ));
    }
    Ok(())
}

/// Validate an optional ISO-BMFF decoder configuration body against a CREATE_VIDEO codec key
/// (`decoder-description-v1`): avcC for `h264`, hvcC for `hevc`, vpcC for `vp9`, av1C for `av1`.
pub fn validate_video_decoder_config(codec: &str, decoder_config: &[u8]) -> io::Result<()> {
    if decoder_config.is_empty() || decoder_config.len() > MAX_DECODER_CONFIG {
        return Err(invalid("video decoder configuration is empty or oversized"));
    }
    let version_matches = match codec {
        // These minimum lengths cover the fixed header through the first codec-specific field.
        "h264" => decoder_config.len() >= 7 && decoder_config[0] == 1,
        "hevc" => decoder_config.len() >= 23 && decoder_config[0] == 1,
        "vp9" => decoder_config.len() >= 12 && decoder_config[0] == 1,
        // av1C begins with marker (1) << 7 | version (1).
        "av1" => decoder_config.len() >= 4 && decoder_config[0] == 0x81,
        _ => false,
    };
    if !version_matches {
        return Err(invalid(
            "video decoder configuration does not match the codec",
        ));
    }
    Ok(())
}

/// Validate an optional RFC 6381 codec string against a CREATE_AUDIO codec key
/// (`decoder-description-v1`).
pub fn validate_audio_codec_string(codec: &str, codec_string: &str) -> io::Result<()> {
    validate_codec_string_shape(codec_string)?;
    let family_matches = match codec {
        "aac" => codec_string.starts_with("mp4a.40."),
        "mp3" => codec_string == "mp3" || codec_string.eq_ignore_ascii_case("mp4a.6b"),
        "opus" | "vorbis" | "flac" | "alac" => codec_string == codec,
        "pcm_mulaw" => codec_string == "ulaw",
        "pcm_alaw" => codec_string == "alaw",
        _ => codec.starts_with("pcm_") && codec_string.starts_with("pcm-"),
    };
    if !family_matches {
        return Err(invalid(
            "audio codec string family does not match the codec",
        ));
    }
    Ok(())
}

fn validate_codec_string_shape(codec_string: &str) -> io::Result<()> {
    if codec_string.is_empty()
        || codec_string.len() > MAX_CODEC_STRING
        || !codec_string.bytes().all(|byte| byte.is_ascii_graphic())
    {
        return Err(invalid(
            "codec string is empty, oversized, or not printable ASCII",
        ));
    }
    Ok(())
}

/// Validate the canonical, container-independent initialization carried by CREATE_AUDIO.
pub fn validate_audio_initialization(
    codec: &str,
    packetization: &str,
    extradata: &[u8],
    sample_rate: u32,
    channels: u16,
) -> io::Result<()> {
    if extradata.len() > MAX_AUDIO_EXTRADATA {
        return Err(invalid("audio initialization data is too large"));
    }
    if !valid_audio_packetization(codec, packetization) {
        return Err(invalid("unsupported audio codec/packetization pair"));
    }
    match codec {
        "opus" => validate_opus_head(extradata, sample_rate, channels),
        "vorbis" => validate_vorbis_headers(extradata, sample_rate, channels),
        "flac" => validate_flac_streaminfo(extradata, sample_rate, channels),
        "aac" => validate_aac_audio_specific_config(extradata, sample_rate, channels),
        _ => Ok(()),
    }
}

/// MPEG-4 samplingFrequencyIndex table (ISO/IEC 14496-3); indexes 13 and 14 are reserved.
const AAC_SAMPLE_RATES: [u32; 13] = [
    96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350,
];

/// Validate an AAC AudioSpecificConfig against the declared CREATE_AUDIO rate and channel count.
///
/// Accepts the HE-AAC convention where the base configuration declares half the output sample
/// rate. `channelConfiguration == 0` (program config element) is accepted for any channel count.
pub fn validate_aac_audio_specific_config(
    config: &[u8],
    sample_rate: u32,
    channels: u16,
) -> io::Result<()> {
    let mut reader = AscBitReader::new(config);
    let audio_object_type = reader
        .read_audio_object_type()
        .ok_or_else(|| invalid("AudioSpecificConfig is truncated"))?;
    if audio_object_type == 0 {
        return Err(invalid("AudioSpecificConfig audio object type is null"));
    }
    let frequency = reader
        .read_sampling_frequency()
        .ok_or_else(|| invalid("AudioSpecificConfig sampling frequency is invalid"))?;
    let channel_configuration = reader
        .read_bits(4)
        .ok_or_else(|| invalid("AudioSpecificConfig is truncated"))?;
    if frequency != sample_rate && frequency.checked_mul(2) != Some(sample_rate) {
        return Err(invalid(
            "AudioSpecificConfig sampling frequency does not match the declared rate",
        ));
    }
    let declared = u32::from(channels);
    let configured = match channel_configuration {
        0 => declared, // program config element carries the layout
        7 => 8,
        other => other,
    };
    if configured != declared {
        return Err(invalid(
            "AudioSpecificConfig channel configuration does not match the declared channels",
        ));
    }
    Ok(())
}

/// Minimal big-endian bit reader for AudioSpecificConfig validation.
struct AscBitReader<'a> {
    data: &'a [u8],
    position: usize,
}

impl<'a> AscBitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, position: 0 }
    }

    fn read_bits(&mut self, count: u32) -> Option<u32> {
        debug_assert!(count <= 24);
        let mut value = 0_u32;
        for _ in 0..count {
            let byte = *self.data.get(self.position / 8)?;
            let bit = (byte >> (7 - (self.position % 8))) & 1;
            value = (value << 1) | u32::from(bit);
            self.position += 1;
        }
        Some(value)
    }

    fn read_audio_object_type(&mut self) -> Option<u32> {
        let base = self.read_bits(5)?;
        if base == 31 {
            Some(32 + self.read_bits(6)?)
        } else {
            Some(base)
        }
    }

    fn read_sampling_frequency(&mut self) -> Option<u32> {
        match self.read_bits(4)? {
            15 => {
                let explicit = self.read_bits(24)?;
                (explicit > 0).then_some(explicit)
            }
            13 | 14 => None, // reserved indexes
            index => AAC_SAMPLE_RATES.get(index as usize).copied(),
        }
    }
}

pub fn validate_opus_head(head: &[u8], sample_rate: u32, channels: u16) -> io::Result<()> {
    if sample_rate != 48_000 || head.len() < 19 || &head[..8] != b"OpusHead" {
        return Err(invalid(
            "invalid OpusHead signature, length, or decode rate",
        ));
    }
    if head[8] > 15 || u16::from(head[9]) != channels || channels == 0 {
        return Err(invalid("OpusHead version or channel count is unsupported"));
    }
    match head[18] {
        0 if head.len() == 19 && channels <= 2 => Ok(()),
        1 => {
            let expected = 21_usize
                .checked_add(usize::from(channels))
                .ok_or_else(|| invalid("OpusHead channel mapping length overflow"))?;
            if head.len() != expected {
                return Err(invalid(
                    "OpusHead family-1 channel mapping length is invalid",
                ));
            }
            let streams = head[19];
            let coupled = head[20];
            let coded_channels = streams.checked_add(coupled).unwrap_or(0);
            if streams == 0
                || coupled > streams
                || coded_channels == 0
                || u16::from(coded_channels) > channels
                || head[21..]
                    .iter()
                    .any(|mapping| *mapping != 255 && *mapping >= coded_channels)
            {
                return Err(invalid("OpusHead family-1 mapping is invalid"));
            }
            Ok(())
        }
        _ => Err(invalid("unsupported OpusHead mapping family")),
    }
}

pub fn validate_vorbis_headers(private: &[u8], sample_rate: u32, channels: u16) -> io::Result<()> {
    if private.first() != Some(&2) {
        return Err(invalid(
            "Vorbis initialization is not three-header Xiph lacing",
        ));
    }
    let mut cursor = 1_usize;
    let first = xiph_laced_length(private, &mut cursor)?;
    let second = xiph_laced_length(private, &mut cursor)?;
    let header_bytes = first
        .checked_add(second)
        .and_then(|length| length.checked_add(cursor))
        .ok_or_else(|| invalid("Vorbis header lengths overflow"))?;
    if header_bytes >= private.len() {
        return Err(invalid("Vorbis headers are truncated"));
    }
    let first_end = cursor + first;
    let second_end = first_end + second;
    let identification = &private[cursor..first_end];
    let comments = &private[first_end..second_end];
    let setup = &private[second_end..];
    let block_sizes = identification.get(28).copied().unwrap_or(0);
    let small_block = block_sizes & 0x0f;
    let large_block = block_sizes >> 4;
    if identification.len() != 30
        || !identification.starts_with(b"\x01vorbis")
        || !comments.starts_with(b"\x03vorbis")
        || !setup.starts_with(b"\x05vorbis")
        || identification[7..11] != [0, 0, 0, 0]
        || u16::from(identification[11]) != channels
        || u32::from_le_bytes(identification[12..16].try_into().unwrap()) != sample_rate
        || !(6..=13).contains(&small_block)
        || !(small_block..=13).contains(&large_block)
        || identification[29] != 1
        || !valid_vorbis_comment_header(comments)
    {
        return Err(invalid(
            "Vorbis identification or header signatures are invalid",
        ));
    }
    Ok(())
}

fn valid_vorbis_comment_header(header: &[u8]) -> bool {
    let mut cursor = 7_usize;
    let Some(vendor_length) = take_vorbis_length(header, &mut cursor) else {
        return false;
    };
    let Some(after_vendor) = cursor.checked_add(vendor_length) else {
        return false;
    };
    if after_vendor > header.len() {
        return false;
    }
    cursor = after_vendor;
    let Some(comment_count) = take_vorbis_length(header, &mut cursor) else {
        return false;
    };
    if comment_count > header.len().saturating_sub(cursor).saturating_sub(1) / 4 {
        return false;
    }
    for _ in 0..comment_count {
        let Some(length) = take_vorbis_length(header, &mut cursor) else {
            return false;
        };
        let Some(next) = cursor.checked_add(length) else {
            return false;
        };
        if next > header.len() {
            return false;
        }
        cursor = next;
    }
    header.get(cursor) == Some(&1) && cursor + 1 == header.len()
}

fn take_vorbis_length(bytes: &[u8], cursor: &mut usize) -> Option<usize> {
    let end = cursor.checked_add(4)?;
    let length = u32::from_le_bytes(bytes.get(*cursor..end)?.try_into().ok()?);
    *cursor = end;
    usize::try_from(length).ok()
}

fn xiph_laced_length(bytes: &[u8], cursor: &mut usize) -> io::Result<usize> {
    let mut length = 0_usize;
    loop {
        let value = *bytes
            .get(*cursor)
            .ok_or_else(|| invalid("truncated Xiph-laced length"))?;
        *cursor += 1;
        length = length
            .checked_add(usize::from(value))
            .ok_or_else(|| invalid("Xiph-laced length overflow"))?;
        if value != 255 {
            return Ok(length);
        }
    }
}

pub fn validate_flac_streaminfo(
    streaminfo: &[u8],
    sample_rate: u32,
    channels: u16,
) -> io::Result<()> {
    if streaminfo.len() != 34 {
        return Err(invalid(
            "FLAC initialization is not a raw 34-byte STREAMINFO",
        ));
    }
    let minimum_block = u16::from_be_bytes(streaminfo[0..2].try_into().unwrap());
    let maximum_block = u16::from_be_bytes(streaminfo[2..4].try_into().unwrap());
    let packed = u64::from_be_bytes(streaminfo[10..18].try_into().unwrap());
    let header_rate = ((packed >> 44) & 0x000f_ffff) as u32;
    let header_channels = ((packed >> 41) & 0x7) as u16 + 1;
    let bits_per_sample = ((packed >> 36) & 0x1f) as u8 + 1;
    if minimum_block < 16
        || maximum_block < minimum_block
        || header_rate == 0
        || header_rate != sample_rate
        || header_channels != channels
        || !(4..=32).contains(&bits_per_sample)
    {
        return Err(invalid("FLAC STREAMINFO configuration is invalid"));
    }
    Ok(())
}

pub fn parse_create_node(body: &[u8]) -> io::Result<(ControlEnvelope, ParsedNodeConfig)> {
    let (envelope, scene_node) = parse_scene_node(body)?;
    if scene_node.clip.is_some() {
        return Err(invalid(
            "clipped node requires parse_scene_node; refusing to discard clip fields",
        ));
    }
    Ok((envelope, scene_node.node))
}

pub fn parse_scene_node(body: &[u8]) -> io::Result<(ControlEnvelope, ParsedSceneNode)> {
    let envelope = decode_control(body)?;
    let payload = &envelope.payload;
    let coordinate_space = required_u64(payload, 3, "coordinate space")?;
    let anchor_id = payload.map_value(14).and_then(Value::as_u64);
    if !matches!(
        coordinate_space,
        COORDINATE_GRID_CELL | COORDINATE_ANCHOR_CELL
    ) || (coordinate_space == COORDINATE_ANCHOR_CELL) != anchor_id.is_some()
        || required_u64(payload, 8, "fit mode")? != FIT_CONTAIN
        || required_u64(payload, 9, "sampling mode")? != SAMPLING_LINEAR
        || required_u64(payload, 10, "text layer")? != TEXT_LAYER_BETWEEN_BACKGROUND_AND_GLYPH
        || required_u64(payload, 12, "blend mode")? != BLEND_SOURCE_OVER
    {
        return Err(invalid("unsupported node configuration"));
    }
    let parsed = ParsedNodeConfig {
        node_id: required_u64(payload, 0, "node ID")?,
        source_id: required_u64(payload, 1, "source ID")?,
        context_id: required_u64(payload, 2, "context ID")?,
        x: required_i64(payload, 4, "node x")?,
        y: required_i64(payload, 5, "node y")?,
        width: required_i64(payload, 6, "node width")?,
        height: required_i64(payload, 7, "node height")?,
        text_layer: required_u64(payload, 10, "text layer")?,
        z_index: required_i64(payload, 11, "z index")?,
        visible: payload
            .map_value(13)
            .and_then(Value::as_bool)
            .unwrap_or(true),
        anchor_id,
    };
    if parsed.node_id == 0 || parsed.source_id == 0 {
        return Err(invalid("node or source ID is zero"));
    }
    if parsed.width <= 0 || parsed.height <= 0 {
        return Err(invalid("node dimensions must be positive"));
    }
    let clip_values = [15_u64, 16, 17, 18].map(|key| payload.map_value(key));
    let clip_count = clip_values.iter().filter(|value| value.is_some()).count();
    let clip = match clip_count {
        0 => None,
        4 => {
            let clip = ClipRect {
                x: required_i64(payload, 15, "clip x")?,
                y: required_i64(payload, 16, "clip y")?,
                width: required_i64(payload, 17, "clip width")?,
                height: required_i64(payload, 18, "clip height")?,
            };
            if clip.width <= 0 || clip.height <= 0 {
                return Err(invalid("clip dimensions must be positive"));
            }
            if clip.x.checked_add(clip.width).is_none() || clip.y.checked_add(clip.height).is_none()
            {
                return Err(invalid("clip rectangle overflows coordinate space"));
            }
            Some(clip)
        }
        _ => return Err(invalid("clip rectangle is incomplete")),
    };
    Ok((envelope, ParsedSceneNode { node: parsed, clip }))
}

pub fn parse_anchor_event(body: &[u8]) -> io::Result<u64> {
    let (_, payload) = decode_envelope(body)?;
    required_u64(&payload, 0, "anchor ID")
}

pub fn parse_update_node(body: &[u8]) -> io::Result<(ControlEnvelope, ParsedNodeConfig)> {
    parse_create_node(body)
}

pub fn parse_update_scene_node(body: &[u8]) -> io::Result<(ControlEnvelope, ParsedSceneNode)> {
    parse_scene_node(body)
}

pub fn parse_object_id(body: &[u8], description: &str) -> io::Result<(ControlEnvelope, u64)> {
    let envelope = decode_control(body)?;
    let object_id = required_u64(&envelope.payload, 0, description)?;
    Ok((envelope, object_id))
}

pub fn parse_play(body: &[u8]) -> io::Result<(ControlEnvelope, PlayRequest)> {
    let envelope = decode_control(body)?;
    let payload = &envelope.payload;
    reject_unknown_fields(payload, &[0, 1, 2, 3, 4, 5, 6, 7])?;
    let request = PlayRequest {
        source_id: required_u64(payload, 0, "PLAY source ID")?,
        start_pts_us: required_i64(payload, 1, "PLAY start PTS")?,
        minimum_buffer_us: required_u64(payload, 2, "PLAY minimum buffer")?,
        maximum_latency_us: required_u64(payload, 3, "PLAY maximum latency")?,
        rate_32_32: required_i64(payload, 4, "PLAY rate")?,
        late_policy: required_u64(payload, 5, "PLAY late policy")?,
        loop_count: required_u64(payload, 6, "PLAY loop count")?,
        start_policy: required_u64(payload, 7, "PLAY start policy")?,
    }
    .validate()?;
    Ok((envelope, request))
}

pub fn parse_eos(body: &[u8]) -> io::Result<(ControlEnvelope, u64, u32)> {
    parse_source_epoch(body)
}

/// `FLUSH` shares the source-ID/epoch body layout with `EOS` but has distinct semantics; parse it
/// under its own name so call sites stay legible.
pub fn parse_flush(body: &[u8]) -> io::Result<(ControlEnvelope, u64, u32)> {
    parse_source_epoch(body)
}

fn parse_source_epoch(body: &[u8]) -> io::Result<(ControlEnvelope, u64, u32)> {
    let envelope = decode_control(body)?;
    let source_id = required_u64(&envelope.payload, 0, "source ID")?;
    let epoch = required_u32(&envelope.payload, 1, "source epoch")?;
    Ok((envelope, source_id, epoch))
}

pub fn parse_attach_channel(body: &[u8]) -> io::Result<Vec<u8>> {
    let envelope = decode_control(body)?;
    Ok(required_bytes(&envelope.payload, 0, "media ticket")?.to_vec())
}

pub fn parse_welcome(body: &[u8]) -> io::Result<Welcome> {
    let (_, payload) = decode_envelope(body)?;
    reject_unknown_fields(
        &payload,
        &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
    )?;
    let welcome = Welcome {
        session_id: required_u64(&payload, 0, "session ID")?,
        session_tag: required_bytes(&payload, 1, "session tag")?.to_vec(),
        root_context_id: required_u64(&payload, 2, "root context ID")?,
        display_generation: required_u64(&payload, 4, "display generation")?,
        viewport_width: required_u32(&payload, 5, "viewport width")?,
        viewport_height: required_u32(&payload, 6, "viewport height")?,
        grid_columns: required_u64(&payload, 7, "grid columns")?,
        grid_rows: required_u64(&payload, 8, "grid rows")?,
        cell_width: required_u32(&payload, 9, "cell width")?,
        cell_height: required_u32(&payload, 10, "cell height")?,
        maximum_control_body: required_u32(&payload, 11, "maximum control body")?,
        accepted_profiles: text_array(&payload, 12, "accepted profiles")?,
        selected_major: required_u64(&payload, 13, "selected Vivid major version")?,
        selected_minor: required_u64(&payload, 14, "selected Vivid minor version")?,
        accepted_features: feature_array(&payload, 15, "accepted features")?,
    };
    if welcome.session_id == 0
        || welcome.root_context_id == 0
        || welcome.session_tag.len() != 16
        || welcome.viewport_width == 0
        || welcome.viewport_height == 0
        || welcome.grid_columns == 0
        || welcome.grid_rows == 0
        || welcome.cell_width == 0
        || welcome.cell_height == 0
        || welcome.maximum_control_body == 0
        || welcome.maximum_control_body > super::CONTROL_MAX_RECORD_BODY
        || (welcome.selected_major, welcome.selected_minor)
            != (u64::from(VIVID_MAJOR), u64::from(VIVID_MINOR))
    {
        return Err(invalid("WELCOME contains an invalid mandatory 1.0 field"));
    }
    Ok(welcome)
}

pub fn parse_display_changed(body: &[u8]) -> io::Result<DisplayChanged> {
    let (_, payload) = decode_envelope(body)?;
    Ok(DisplayChanged {
        display_generation: required_u64(&payload, 0, "display generation")?,
        viewport_width: required_u32(&payload, 1, "viewport width")?,
        viewport_height: required_u32(&payload, 2, "viewport height")?,
        grid_columns: required_u32(&payload, 3, "grid columns")?,
        grid_rows: required_u32(&payload, 4, "grid rows")?,
        cell_width: required_u32(&payload, 5, "cell width")?,
        cell_height: required_u32(&payload, 6, "cell height")?,
    })
}

pub fn parse_source_ready(body: &[u8]) -> io::Result<SourceReady> {
    let (_, payload) = decode_envelope(body)?;
    reject_unknown_fields(&payload, &[0, 1, 2, 3, 4, 5])?;
    let ready = SourceReady {
        source_id: required_u64(&payload, 0, "source ID")?,
        media_ticket: required_bytes(&payload, 1, "media ticket")?.to_vec(),
        byte_credits: required_u64(&payload, 2, "byte credits")?,
        packet_credits: required_u64(&payload, 3, "packet credits")?,
        fragment_credits: payload.map_value(4).and_then(Value::as_u64).unwrap_or(0),
        max_media_body: required_u32(&payload, 5, "maximum media body")?,
    };
    if ready.source_id == 0
        || ready.media_ticket.len() != 32
        || ready.max_media_body == 0
        || ready.max_media_body > super::HARD_MAX_RECORD_BODY
        || ready.byte_credits < u64::from(ready.max_media_body)
        || ready.packet_credits == 0
    {
        return Err(invalid("SOURCE_READY contains invalid limits or credits"));
    }
    Ok(ready)
}

pub fn parse_visibility(body: &[u8]) -> io::Result<Visibility> {
    let (_, payload) = decode_envelope(body)?;
    Ok(Visibility {
        visible: payload
            .map_value(0)
            .and_then(Value::as_bool)
            .ok_or_else(|| invalid("missing visibility state"))?,
        reasons: required_u64(&payload, 1, "visibility reasons")?,
        display_generation: required_u64(&payload, 2, "visibility generation")?,
    })
}

pub fn parse_need_keyframe(body: &[u8]) -> io::Result<NeedKeyframe> {
    let (_, payload) = decode_envelope(body)?;
    Ok(NeedKeyframe {
        source_id: required_u64(&payload, 0, "source ID")?,
        minimum_epoch: required_u32(&payload, 1, "minimum epoch")?,
        reason: required_u64(&payload, 2, "keyframe reason")?,
        last_packet_id: payload
            .map_value(3)
            .map(|value| {
                value
                    .as_u64()
                    .ok_or_else(|| invalid("last packet ID is not unsigned"))
            })
            .transpose()?,
    })
}

pub fn parse_source_lost(body: &[u8]) -> io::Result<SourceLost> {
    let (_, payload) = decode_envelope(body)?;
    reject_unknown_fields(&payload, &[0, 1, 2])?;
    Ok(SourceLost {
        source_id: required_u64(&payload, 0, "source ID")?,
        code: required_u64(&payload, 1, "source-loss code")?,
        diagnostic: bounded_text(&payload, 2, "source-loss diagnostic", 4096)?.to_owned(),
    })
}

pub fn parse_credit(body: &[u8]) -> io::Result<Credits> {
    let (_, payload) = decode_envelope(body)?;
    Ok(Credits {
        bytes: required_u64(&payload, 0, "byte credits")?,
        packets: required_u64(&payload, 1, "packet credits")?,
        fragments: payload.map_value(2).and_then(Value::as_u64).unwrap_or(0),
    })
}

pub fn parse_key_input(body: &[u8]) -> io::Result<KeyInput> {
    let envelope = parse_unsolicited(body, &[0, 1])?;
    let usage = u16::try_from(required_u64(&envelope.payload, 0, "keyboard usage")?)
        .map_err(|_| invalid("keyboard usage exceeds u16"))?;
    if !(HID_KEYBOARD_USAGE_MIN..=HID_KEYBOARD_USAGE_MAX).contains(&usage) {
        return Err(invalid("keyboard usage is outside the HID keyboard page"));
    }
    let pressed = envelope
        .payload
        .map_value(1)
        .and_then(Value::as_bool)
        .ok_or_else(|| invalid("missing keyboard state"))?;
    Ok(KeyInput { usage, pressed })
}

pub fn parse_pointer_motion(body: &[u8]) -> io::Result<PointerMotion> {
    let envelope = parse_unsolicited(body, &[0, 1, 2])?;
    let motion = PointerMotion {
        source_id: required_u64(&envelope.payload, 0, "pointer source ID")?,
        x: required_u32(&envelope.payload, 1, "pointer x")?,
        y: required_u32(&envelope.payload, 2, "pointer y")?,
    };
    if motion.source_id == 0 {
        return Err(invalid("pointer source ID is zero"));
    }
    Ok(motion)
}

pub fn parse_pointer_button(body: &[u8]) -> io::Result<PointerButton> {
    let envelope = parse_unsolicited(body, &[0, 1, 2])?;
    let source_id = required_u64(&envelope.payload, 0, "pointer source ID")?;
    let button = u8::try_from(required_u64(&envelope.payload, 1, "pointer button")?)
        .map_err(|_| invalid("pointer button exceeds u8"))?;
    let pressed = envelope
        .payload
        .map_value(2)
        .and_then(Value::as_bool)
        .ok_or_else(|| invalid("missing pointer button state"))?;
    if source_id == 0 || button > POINTER_BUTTON_MAX {
        return Err(invalid("pointer button event is outside its bounds"));
    }
    Ok(PointerButton {
        source_id,
        button,
        pressed,
    })
}

pub fn parse_pointer_axis(body: &[u8]) -> io::Result<PointerAxis> {
    let envelope = parse_unsolicited(body, &[0, 1, 2])?;
    let source_id = required_u64(&envelope.payload, 0, "pointer source ID")?;
    let horizontal_120 = i32::try_from(required_i64(
        &envelope.payload,
        1,
        "horizontal pointer axis",
    )?)
    .map_err(|_| invalid("horizontal pointer axis exceeds i32"))?;
    let vertical_120 = i32::try_from(required_i64(&envelope.payload, 2, "vertical pointer axis")?)
        .map_err(|_| invalid("vertical pointer axis exceeds i32"))?;
    if source_id == 0
        || horizontal_120.unsigned_abs() > POINTER_AXIS_MAX as u32
        || vertical_120.unsigned_abs() > POINTER_AXIS_MAX as u32
    {
        return Err(invalid("pointer axis event is outside its bounds"));
    }
    Ok(PointerAxis {
        source_id,
        horizontal_120,
        vertical_120,
    })
}

pub fn parse_input_reset(body: &[u8]) -> io::Result<()> {
    parse_unsolicited(body, &[]).map(|_| ())
}

pub fn parse_error(body: &[u8]) -> io::Result<String> {
    let error = parse_error_reply(body)?;
    Ok(format!(
        "presenter error {}: {}",
        error.code, error.diagnostic
    ))
}

pub fn parse_error_reply(body: &[u8]) -> io::Result<ErrorReply> {
    let (envelope_request_id, payload) = decode_envelope(body)?;
    reject_unknown_fields(&payload, &[0, 1, 4, 5])?;
    let code = required_u64(&payload, 0, "error code")?;
    let request_id = required_u64(&payload, 1, "failed request ID")?;
    if request_id != envelope_request_id {
        return Err(invalid("ERROR request IDs do not match"));
    }
    let diagnostic = payload
        .map_value(5)
        .map(|_| bounded_text(&payload, 5, "error diagnostic", 4096))
        .transpose()?
        .unwrap_or("no diagnostic");
    Ok(ErrorReply {
        code,
        request_id,
        diagnostic: diagnostic.to_owned(),
    })
}

pub fn request_id(body: &[u8]) -> io::Result<u64> {
    let (request_id, _) = decode_envelope(body)?;
    Ok(request_id)
}

pub fn name(record_type: u16) -> &'static str {
    match record_type {
        HELLO => "HELLO",
        WELCOME => "WELCOME",
        OK => "OK",
        ERROR => "ERROR",
        PING => "PING",
        PONG => "PONG",
        GOODBYE => "GOODBYE",
        DISPLAY_CHANGED => "DISPLAY_CHANGED",
        CAPS_CHANGED => "CAPS_CHANGED",
        PROBE_VIDEO_CONFIG => "PROBE_VIDEO_CONFIG",
        VIDEO_SUPPORT => "VIDEO_SUPPORT",
        CREATE_IMAGE => "CREATE_IMAGE",
        CREATE_VIDEO => "CREATE_VIDEO",
        CREATE_RASTER => "CREATE_RASTER",
        SOURCE_READY => "SOURCE_READY",
        RECONFIGURE_SOURCE => "RECONFIGURE_SOURCE",
        DESTROY_SOURCE => "DESTROY_SOURCE",
        SOURCE_LOST => "SOURCE_LOST",
        PROBE_AUDIO_CONFIG => "PROBE_AUDIO_CONFIG",
        AUDIO_SUPPORT => "AUDIO_SUPPORT",
        CREATE_AUDIO => "CREATE_AUDIO",
        BEGIN_TXN => "BEGIN_TXN",
        CREATE_NODE => "CREATE_NODE",
        UPDATE_NODE => "UPDATE_NODE",
        DELETE_NODE => "DELETE_NODE",
        COMMIT_TXN => "COMMIT_TXN",
        ABORT_TXN => "ABORT_TXN",
        PRESENTED => "PRESENTED",
        ANCHOR_READY => "ANCHOR_READY",
        ANCHOR_GONE => "ANCHOR_GONE",
        BARRIER_REACHED => "BARRIER_REACHED",
        PLAY => "PLAY",
        PAUSE => "PAUSE",
        STEP => "STEP",
        FLUSH => "FLUSH",
        DRAIN => "DRAIN",
        EOS => "EOS",
        PLAYBACK_STATE => "PLAYBACK_STATE",
        CREDIT => "CREDIT",
        FEEDBACK => "FEEDBACK",
        VISIBILITY => "VISIBILITY",
        QUALITY_HINT => "QUALITY_HINT",
        NEED_KEYFRAME => "NEED_KEYFRAME",
        BLOB_OFFER => "BLOB_OFFER",
        BLOB_HAVE => "BLOB_HAVE",
        BLOB_NEED => "BLOB_NEED",
        BLOB_COMPLETE => "BLOB_COMPLETE",
        CACHE_EVICTED => "CACHE_EVICTED",
        CREATE_CONTEXT => "CREATE_CONTEXT",
        DELEGATE_CONTEXT => "DELEGATE_CONTEXT",
        REVOKE_CONTEXT => "REVOKE_CONTEXT",
        CONTEXT_CHANGED => "CONTEXT_CHANGED",
        KEY_INPUT => "KEY_INPUT",
        POINTER_MOTION => "POINTER_MOTION",
        POINTER_BUTTON => "POINTER_BUTTON",
        POINTER_AXIS => "POINTER_AXIS",
        INPUT_RESET => "INPUT_RESET",
        ATTACH_CHANNEL => "ATTACH_CHANNEL",
        VIDEO_PACKET => "VIDEO_PACKET",
        VIDEO_FRAGMENT => "VIDEO_FRAGMENT",
        RASTER_FRAME => "RASTER_FRAME",
        BLOB_CHUNK => "BLOB_CHUNK",
        BUFFER_SUBMIT => "BUFFER_SUBMIT",
        IMAGE_DATA => "IMAGE_DATA",
        AUDIO_PACKET => "AUDIO_PACKET",
        _ => "UNKNOWN",
    }
}

fn parse_unsolicited(body: &[u8], fields: &[u64]) -> io::Result<ControlEnvelope> {
    let envelope = decode_control(body)?;
    if envelope.request_id != 0
        || envelope.transaction_id.is_some()
        || envelope.expected_generation.is_some()
    {
        return Err(invalid(
            "unsolicited input has request or transaction state",
        ));
    }
    reject_unknown_fields(&envelope.payload, fields)?;
    Ok(envelope)
}

fn envelope(
    request_id: u64,
    transaction_id: Option<u64>,
    expected_generation: Option<u64>,
    payload: impl FnOnce(&mut Encoder),
) -> Vec<u8> {
    let mut output = Vec::new();
    envelope_into(
        &mut output,
        request_id,
        transaction_id,
        expected_generation,
        payload,
    );
    output
}

fn envelope_into(
    output: &mut Vec<u8>,
    request_id: u64,
    transaction_id: Option<u64>,
    expected_generation: Option<u64>,
    payload: impl FnOnce(&mut Encoder),
) {
    let mut encoder = Encoder::from_vec(std::mem::take(output));
    encoder.clear();
    let field_count =
        2 + usize::from(transaction_id.is_some()) + usize::from(expected_generation.is_some());
    encoder.map(field_count);
    key_u64(&mut encoder, 0, request_id);
    if let Some(transaction_id) = transaction_id {
        key_u64(&mut encoder, 1, transaction_id);
    }
    if let Some(expected_generation) = expected_generation {
        key_u64(&mut encoder, 2, expected_generation);
    }
    encoder.u64(3);
    payload(&mut encoder);
    *output = encoder.into_vec();
}

fn decode_envelope(body: &[u8]) -> io::Result<(u64, Value)> {
    let envelope = decode_control(body)?;
    Ok((envelope.request_id, envelope.payload))
}

fn required_u64(value: &Value, key: u64, description: &str) -> io::Result<u64> {
    value
        .map_value(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, format!("missing {description}")))
}

fn reject_unknown_fields(value: &Value, allowed: &[u64]) -> io::Result<()> {
    let Value::Map(entries) = value else {
        return Err(invalid("schema value is not a map"));
    };
    if entries.iter().any(|(key, _)| !allowed.contains(key)) {
        return Err(invalid("schema contains a reserved field"));
    }
    Ok(())
}

fn required_bytes<'a>(value: &'a Value, key: u64, description: &str) -> io::Result<&'a [u8]> {
    value
        .map_value(key)
        .and_then(Value::as_bytes)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, format!("missing {description}")))
}

fn required_text<'a>(value: &'a Value, key: u64, description: &str) -> io::Result<&'a str> {
    value
        .map_value(key)
        .and_then(Value::as_text)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, format!("missing {description}")))
}

fn bounded_text<'a>(
    value: &'a Value,
    key: u64,
    description: &str,
    maximum: usize,
) -> io::Result<&'a str> {
    let text = required_text(value, key, description)?;
    if text.len() > maximum {
        return Err(invalid("text field exceeds its schema limit"));
    }
    Ok(text)
}

fn feature_array(value: &Value, key: u64, description: &str) -> io::Result<Vec<u64>> {
    let array = value
        .map_value(key)
        .and_then(Value::as_array)
        .ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, format!("missing {description}"))
        })?;
    let mut output = Vec::with_capacity(array.len());
    for item in array {
        let feature = item
            .as_u64()
            .ok_or_else(|| invalid("feature ID is not unsigned"))?;
        if output.last().is_some_and(|previous| *previous >= feature) {
            return Err(invalid("feature IDs are not strictly increasing"));
        }
        output.push(feature);
    }
    Ok(output)
}

fn text_array(value: &Value, key: u64, description: &str) -> io::Result<Vec<String>> {
    let array = value
        .map_value(key)
        .and_then(Value::as_array)
        .ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, format!("missing {description}"))
        })?;
    let mut output = Vec::with_capacity(array.len());
    for item in array {
        let text = item
            .as_text()
            .ok_or_else(|| invalid("profile name is not text"))?;
        if output
            .last()
            .is_some_and(|previous: &String| previous.as_str() >= text)
        {
            return Err(invalid("profile names are not strictly sorted"));
        }
        output.push(text.to_owned());
    }
    Ok(output)
}

fn truncate_utf8(value: &str, maximum: usize) -> &str {
    if value.len() <= maximum {
        return value;
    }
    let mut end = maximum;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

fn required_i64(value: &Value, key: u64, description: &str) -> io::Result<i64> {
    value
        .map_value(key)
        .and_then(Value::as_i64)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, format!("missing {description}")))
}

fn required_u32(value: &Value, key: u64, description: &str) -> io::Result<u32> {
    u32::try_from(required_u64(value, key, description)?)
        .map_err(|_| invalid("unsigned field exceeds u32"))
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn invalid_data(error: impl std::error::Error + Send + Sync + 'static) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error)
}

fn fixed_cells(cells: u32) -> i64 {
    i64::from(cells) << 32
}

fn key_u64(encoder: &mut Encoder, key: u64, value: u64) {
    encoder.u64(key);
    encoder.u64(value);
}

fn key_i64(encoder: &mut Encoder, key: u64, value: i64) {
    encoder.u64(key);
    encoder.i64(value);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hello_uses_control_envelope() {
        let value = cbor::decode(&hello(9, "secret")).unwrap();
        assert_eq!(value.map_value(0).and_then(Value::as_u64), Some(9));
        assert_eq!(
            value
                .map_value(3)
                .unwrap()
                .map_value(5)
                .and_then(Value::as_text),
            Some("vivi")
        );
    }

    #[test]
    fn fixed_cell_coordinates_use_signed_32_32() {
        let body = create_node(
            1,
            2,
            NodeConfig {
                node_id: 3,
                source_id: 4,
                context_id: 5,
                columns: 80,
                rows: 24,
                anchor_id: None,
            },
        );
        let envelope = cbor::decode(&body).unwrap();
        let payload = envelope.map_value(3).unwrap();
        assert_eq!(
            payload.map_value(6).and_then(Value::as_i64),
            Some(80_i64 << 32)
        );
    }

    #[test]
    fn clipped_scene_nodes_round_trip_without_changing_legacy_nodes() {
        let config = SceneNodeConfig {
            node_id: 3,
            source_id: 4,
            context_id: 5,
            x: -(1_i64 << 31),
            y: 1_i64 << 31,
            width: 10_i64 << 32,
            height: 6_i64 << 32,
            text_layer: TEXT_LAYER_BETWEEN_BACKGROUND_AND_GLYPH,
            z_index: -2,
            visible: true,
            anchor_id: Some(9),
            clip: Some(ClipRect {
                x: 1_i64 << 31,
                y: -(1_i64 << 31),
                width: 4_i64 << 32,
                height: 3_i64 << 32,
            }),
        };
        let body = create_scene_node(1, 2, &config);
        assert_eq!(
            body,
            create_scene_node(1, 2, &config),
            "CBOR must be deterministic"
        );
        let (envelope, parsed) = parse_scene_node(&body).unwrap();
        assert_eq!(envelope.transaction_id, Some(2));
        assert_eq!(parsed.node.node_id, config.node_id);
        assert_eq!(parsed.node.anchor_id, Some(9));
        assert_eq!(parsed.clip, config.clip);
        assert!(
            parse_create_node(&body).is_err(),
            "legacy parser must not drop clipping"
        );

        let legacy = create_node(
            2,
            3,
            NodeConfig {
                node_id: 4,
                source_id: 5,
                context_id: 6,
                columns: 2,
                rows: 2,
                anchor_id: None,
            },
        );
        assert!(parse_create_node(&legacy).is_ok());
        assert_eq!(parse_scene_node(&legacy).unwrap().1.clip, None);
    }

    #[test]
    fn clipped_scene_nodes_reject_invalid_geometry() {
        let base = SceneNodeConfig {
            node_id: 1,
            source_id: 2,
            context_id: 3,
            x: 0,
            y: 0,
            width: 1_i64 << 32,
            height: 1_i64 << 32,
            text_layer: TEXT_LAYER_BETWEEN_BACKGROUND_AND_GLYPH,
            z_index: 0,
            visible: true,
            anchor_id: None,
            clip: Some(ClipRect {
                x: 0,
                y: 0,
                width: 0,
                height: 1_i64 << 32,
            }),
        };
        assert!(parse_scene_node(&create_scene_node(1, 1, &base)).is_err());
        let overflow = SceneNodeConfig {
            clip: Some(ClipRect {
                x: i64::MAX,
                width: 1,
                height: 1,
                y: 0,
            }),
            ..base
        };
        assert!(parse_scene_node(&create_scene_node(1, 1, &overflow)).is_err());
    }

    #[test]
    fn configurable_hello_advertises_exact_identity_and_features() {
        let body = encode_hello(
            7,
            &HelloConfig {
                minimum_major: u64::from(VIVID_MAJOR),
                minimum_minor: u64::from(VIVID_MINOR),
                maximum_major: u64::from(VIVID_MAJOR),
                maximum_minor: u64::from(VIVID_MINOR),
                token: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                producer: "vvmux",
                producer_version: "0.1.0",
                required_features: &[FEATURE_NODE_CLIP_RECT_V1],
                optional_features: &[],
                maximum_record_body: 4096,
            },
        );
        let (_, hello) = parse_hello(&body).unwrap();
        assert_eq!(
            (
                hello.minimum_major,
                hello.minimum_minor,
                hello.maximum_major,
                hello.maximum_minor,
            ),
            (
                u64::from(VIVID_MAJOR),
                u64::from(VIVID_MINOR),
                u64::from(VIVID_MAJOR),
                u64::from(VIVID_MINOR),
            )
        );
        assert_eq!(hello.producer, "vvmux");
        assert_eq!(hello.required_features, [FEATURE_NODE_CLIP_RECT_V1]);
        assert!(hello.optional_features.is_empty());
        assert_eq!(hello.maximum_record_body, 4096);
    }

    #[test]
    fn server_parses_client_raster_and_node_messages() {
        let (_, raster) = parse_create_raster(&create_raster(7, 10, 640, 480)).unwrap();
        assert_eq!(
            (raster.source_id, raster.width, raster.height),
            (10, 640, 480)
        );

        let (_, node) = parse_create_node(&create_node(
            8,
            9,
            NodeConfig {
                node_id: 11,
                source_id: 10,
                context_id: 1,
                columns: 40,
                rows: 20,
                anchor_id: Some(7),
            },
        ))
        .unwrap();
        assert_eq!((node.node_id, node.source_id), (11, 10));
        assert_eq!(node.width, 40_i64 << 32);
        assert_eq!(node.anchor_id, Some(7));

        let (envelope, node_id) = parse_object_id(&delete_node(12, 9, 11), "node ID").unwrap();
        assert_eq!(envelope.transaction_id, Some(9));
        assert_eq!(node_id, 11);
    }

    #[test]
    fn anchor_events_round_trip() {
        assert_eq!(
            parse_anchor_event(&anchor_event(0x1020_3040)).unwrap(),
            0x1020_3040
        );
    }

    #[test]
    fn desktop_input_messages_round_trip_and_reject_bounds() {
        assert_eq!(
            parse_key_input(&key_input(0x04, true)).unwrap(),
            KeyInput {
                usage: 0x04,
                pressed: true,
            }
        );
        assert_eq!(
            parse_pointer_motion(&pointer_motion(7, 1919, 1079)).unwrap(),
            PointerMotion {
                source_id: 7,
                x: 1919,
                y: 1079,
            }
        );
        assert_eq!(
            parse_pointer_button(&pointer_button(7, 2, false)).unwrap(),
            PointerButton {
                source_id: 7,
                button: 2,
                pressed: false,
            }
        );
        assert_eq!(
            parse_pointer_axis(&pointer_axis(7, -120, 240)).unwrap(),
            PointerAxis {
                source_id: 7,
                horizontal_120: -120,
                vertical_120: 240,
            }
        );
        parse_input_reset(&input_reset()).unwrap();

        assert!(parse_key_input(&key_input(0x03, true)).is_err());
        assert!(parse_pointer_motion(&pointer_motion(0, 0, 0)).is_err());
        assert!(parse_pointer_button(&pointer_button(7, 5, true)).is_err());
        assert!(parse_pointer_axis(&pointer_axis(7, 12_001, 0)).is_err());
        assert!(parse_input_reset(&ok(1)).is_err());
    }

    #[test]
    fn desktop_input_feature_negotiates_and_advertises_its_profile() {
        let accepted = negotiate_features(
            &[FEATURE_SCENE_TRANSACTIONS],
            &[FEATURE_DESKTOP_INPUT_V1],
            |feature| {
                matches!(
                    feature,
                    FEATURE_SCENE_TRANSACTIONS | FEATURE_DESKTOP_INPUT_V1
                )
            },
        )
        .unwrap();
        assert_eq!(
            accepted,
            vec![FEATURE_SCENE_TRANSACTIONS, FEATURE_DESKTOP_INPUT_V1]
        );
        assert_eq!(
            negotiate_features(&[FEATURE_DESKTOP_INPUT_V1], &[], |_| false),
            Err(FEATURE_DESKTOP_INPUT_V1)
        );

        let display = DisplayChanged {
            display_generation: 1,
            viewport_width: 800,
            viewport_height: 600,
            grid_columns: 800,
            grid_rows: 600,
            cell_width: 1,
            cell_height: 1,
        };
        let body = welcome(7, 9, &[1; 16], 10, display, &accepted);
        let parsed = parse_welcome(&body).unwrap();
        assert!(
            parsed
                .accepted_profiles
                .iter()
                .any(|profile| profile == PROFILE_DESKTOP_INPUT)
        );
    }

    #[test]
    fn welcome_and_source_ready_require_vivid_1_0_fields() {
        let display = DisplayChanged {
            display_generation: 3,
            viewport_width: 800,
            viewport_height: 600,
            grid_columns: 80,
            grid_rows: 24,
            cell_width: 10,
            cell_height: 25,
        };
        let features = [
            FEATURE_RASTER_RGBA8,
            FEATURE_SCENE_TRANSACTIONS,
            FEATURE_GRID_CELL_NODES,
            FEATURE_CREDIT_FLOW_CONTROL,
            FEATURE_TEXT_ANCHORS_V2,
        ];
        let parsed = parse_welcome(&welcome(7, 9, &[1; 16], 10, display, &features)).unwrap();
        assert_eq!(
            (parsed.selected_major, parsed.selected_minor),
            (u64::from(VIVID_MAJOR), u64::from(VIVID_MINOR))
        );
        let unsupported = encode_welcome(
            7,
            &WelcomeConfig {
                session_id: 9,
                session_tag: &[1; 16],
                root_context_id: 10,
                capability_generation: 1,
                display,
                maximum_control_body: super::super::CONTROL_MAX_RECORD_BODY,
                accepted_profiles: &[],
                selected_major: 1,
                selected_minor: 1,
                accepted_features: &features,
            },
        );
        assert!(parse_welcome(&unsupported).is_err());
        assert_eq!(parsed.viewport_width, 800);
        assert_eq!(parsed.viewport_height, 600);
        assert_eq!(parsed.cell_width, 10);
        assert_eq!(parsed.cell_height, 25);
        assert_eq!(
            parsed.maximum_control_body,
            super::super::CONTROL_MAX_RECORD_BODY
        );
        assert!(
            parsed
                .accepted_features
                .binary_search(&FEATURE_TEXT_ANCHORS_V2)
                .is_ok()
        );
        assert!(
            !parsed
                .accepted_features
                .contains(&FEATURE_RETIRED_VIDEO_FFMPEG_PACKET_V0)
        );
        assert!(
            !parsed
                .accepted_features
                .contains(&FEATURE_RETIRED_TEXT_ANCHORS_V1)
        );

        let ready = parse_source_ready(&source_ready(
            8,
            11,
            &[2; 32],
            Credits {
                bytes: 4 << 20,
                packets: 32,
                fragments: 0,
            },
            1234,
        ))
        .unwrap();
        assert_eq!(ready.max_media_body, 1234);
        assert_eq!(ready.source_id, 11);
    }

    #[test]
    fn image_visibility_and_keyframe_messages_round_trip() {
        let image = ImageSourceConfig {
            source_id: 4,
            encoding: IMAGE_PNG,
            width: 320,
            height: 200,
            encoded_length: 99,
            sha256: Some([7; 32]),
        };
        let (_, parsed) = parse_create_image(&create_image(1, &image)).unwrap();
        assert_eq!(parsed.source_id, image.source_id);
        assert_eq!(parsed.sha256, image.sha256);

        let parsed = parse_visibility(&visibility(4, false, 3, 8)).unwrap();
        assert!(!parsed.visible);
        assert_eq!((parsed.reasons, parsed.display_generation), (3, 8));

        let parsed =
            parse_need_keyframe(&need_keyframe(4, 7, ERROR_DEVICE_LOST, Some(18))).unwrap();
        assert_eq!((parsed.source_id, parsed.minimum_epoch), (4, 7));
        assert_eq!(parsed.last_packet_id, Some(18));

        let parsed = parse_source_lost(&source_lost(4, ERROR_HASH_MISMATCH, "bad hash")).unwrap();
        assert_eq!(parsed.source_id, 4);
        assert_eq!(parsed.code, ERROR_HASH_MISMATCH);
        assert_eq!(parsed.diagnostic, "bad hash");
    }

    #[test]
    fn audio_config_round_trip_and_support_limits() {
        let config = AudioSourceConfig {
            source_id: 12,
            linked_video_source_id: Some(10),
            codec: "aac",
            packetization: "aac-raw-au-v1",
            // AudioSpecificConfig: AAC-LC, 48 kHz, stereo.
            extradata: &[0x11, 0x90],
            sample_rate: 48_000,
            channels: 2,
            channel_mask: 3,
            bitrate: 192_000,
            max_access_unit_bytes: 8_192,
            codec_string: Some("mp4a.40.2"),
        };
        let (envelope, parsed) = parse_create_audio(&create_audio(7, &config)).unwrap();
        assert_eq!(envelope.request_id, 7);
        assert_eq!(parsed.source_id, 12);
        assert_eq!(parsed.linked_video_source_id, Some(10));
        assert_eq!(parsed.codec, "aac");
        assert_eq!(parsed.extradata, [0x11, 0x90]);
        assert_eq!(parsed.codec_string.as_deref(), Some("mp4a.40.2"));
        assert!(audio_config_supported(&parsed));
    }

    #[test]
    fn video_config_round_trips_decoder_description() {
        let base = VideoSourceConfig {
            source_id: 3,
            codec: "h264",
            packetization: "h264-annexb-au-v1",
            extradata: &[0, 0, 0, 1, 0x67],
            width: 1280,
            height: 720,
            profile: 100,
            level: 31,
            bitrate: 2_000_000,
            color_primaries: 1,
            transfer: 1,
            matrix: 1,
            range: 1,
            sar_num: 1,
            sar_den: 1,
            max_access_unit_bytes: 1 << 20,
            codec_string: None,
            decoder_config: None,
        };

        // Without the feature the encoding stays byte-identical to a 21-key config.
        let (_, parsed) = parse_create_video(&create_video(11, &base)).unwrap();
        assert_eq!(parsed.codec_string, None);
        assert_eq!(parsed.decoder_config, None);

        let avcc = [1, 0x64, 0x00, 0x1f, 0xff, 0xe1, 0x00];
        let described = VideoSourceConfig {
            codec_string: Some("avc1.64001F"),
            decoder_config: Some(&avcc),
            ..base
        };
        let (_, parsed) = parse_create_video(&create_video(12, &described)).unwrap();
        assert_eq!(parsed.codec_string.as_deref(), Some("avc1.64001F"));
        assert_eq!(parsed.decoder_config.as_deref(), Some(avcc.as_slice()));

        // Family mismatch and malformed boxes are rejected at parse time.
        let mismatched = VideoSourceConfig {
            codec_string: Some("vp09.00.10.08"),
            decoder_config: None,
            ..base
        };
        assert!(parse_create_video(&create_video(13, &mismatched)).is_err());
        let wrong_box = VideoSourceConfig {
            codec_string: None,
            decoder_config: Some(&[0x81]),
            ..base
        };
        assert!(parse_create_video(&create_video(14, &wrong_box)).is_err());

        assert!(validate_video_codec_string("av1", "av01.0.05M.08").is_ok());
        assert!(validate_video_codec_string("hevc", "hvc1.1.6.L93.B0").is_ok());
        assert!(validate_video_codec_string("h264", "avc1 spaced").is_err());
        assert!(validate_video_decoder_config("av1", &[0x81, 0x00, 0x00, 0x00]).is_ok());
        assert!(validate_video_decoder_config("hevc", &[1; 22]).is_err());
        assert!(validate_video_decoder_config("vp9", &[1; 11]).is_err());
        assert!(validate_video_decoder_config("h264", &vec![1; MAX_DECODER_CONFIG + 1]).is_err());
    }

    #[test]
    fn negotiate_features_rejects_missing_required_and_sorts_accepted() {
        let supported = |feature: u64| feature <= 5 || feature == FEATURE_TEXT_ANCHORS_V2;
        assert_eq!(
            negotiate_features(&[1, 3], &[FEATURE_TEXT_ANCHORS_V2, 4, 99], supported),
            Ok(vec![1, 3, 4, FEATURE_TEXT_ANCHORS_V2])
        );
        assert_eq!(negotiate_features(&[1, 99], &[], supported), Err(99));
        assert_eq!(negotiate_features(&[], &[99], supported), Ok(Vec::new()));
    }

    #[test]
    fn hot_control_encoders_reuse_buffers_and_preserve_golden_bytes() {
        let mut output = Vec::with_capacity(128);
        let allocation = output.as_ptr();

        credit_into(&mut output, 1, 2, 3);
        assert_eq!(output, [0xa2, 0, 0, 3, 0xa3, 0, 1, 1, 2, 2, 3]);
        assert_eq!(output, credit(1, 2, 3));
        assert_eq!(output.as_ptr(), allocation);

        visibility_into(&mut output, 7, true, 2, 3);
        assert_eq!(output, [0xa2, 0, 0, 3, 0xa3, 0, 0xf5, 1, 2, 2, 3]);
        assert_eq!(output, visibility(7, true, 2, 3));
        assert_eq!(output.as_ptr(), allocation);

        ok_into(&mut output, 9);
        assert_eq!(output, [0xa2, 0, 9, 3, 0xa0]);
        assert_eq!(output, ok(9));
        assert_eq!(output.as_ptr(), allocation);

        ping_into(&mut output, 10);
        assert_eq!(output, [0xa2, 0, 10, 3, 0xa0]);
        assert_eq!(output, ping(10));
        pong_into(&mut output, 10);
        assert_eq!(output, [0xa2, 0, 10, 3, 0xa0]);
        assert_eq!(output, pong(10));
        assert_eq!(output.as_ptr(), allocation);
    }

    #[test]
    fn credit_ledger_checked_grant_and_consume() {
        let mut ledger = CreditLedger::new(Credits {
            bytes: 100,
            packets: 1,
            fragments: 0,
        });
        assert!(ledger.can_consume(100));
        assert!(!ledger.can_consume(101));
        ledger.consume(60).unwrap();
        // One packet credit paid: byte credit remains but the packet window is exhausted.
        assert!(!ledger.can_consume(1));
        assert!(ledger.consume(1).is_err());
        ledger
            .grant(Credits {
                bytes: 10,
                packets: 1,
                fragments: 2,
            })
            .unwrap();
        assert!(ledger.can_consume(50));
        assert!(ledger.can_consume_with_fragments(50, 2));
        ledger.consume_with_fragments(50, 2).unwrap();
        assert_eq!(ledger.fragments, 0);
        let mut saturated = CreditLedger::new(Credits {
            bytes: u64::MAX,
            packets: 1,
            fragments: 0,
        });
        assert!(
            saturated
                .grant(Credits {
                    bytes: 1,
                    packets: 0,
                    fragments: 0,
                })
                .is_err()
        );
        let mut atomic = CreditLedger::new(Credits {
            bytes: 1,
            packets: u64::MAX,
            fragments: 1,
        });
        assert!(
            atomic
                .grant(Credits {
                    bytes: 1,
                    packets: 1,
                    fragments: 1,
                })
                .is_err()
        );
        assert_eq!(
            (atomic.bytes, atomic.packets, atomic.fragments),
            (1, u64::MAX, 1)
        );
        ledger.mark_lost();
        assert!(ledger.is_lost());
        assert!(!ledger.can_consume(1));
        assert!(ledger.consume(1).is_err());
        assert!(ledger.grant(Credits::default()).is_err());
    }

    #[test]
    fn scene_rect_validation_rejects_degenerate_and_overflowing_rects() {
        assert!(validate_scene_rect(0, 0, 1 << 32, 1 << 32).is_ok());
        assert!(validate_scene_rect(-(1 << 32), -(1 << 32), 1, 1).is_ok());
        assert!(validate_scene_rect(0, 0, 0, 1).is_err());
        assert!(validate_scene_rect(0, 0, 1, -1).is_err());
        assert!(validate_scene_rect(i64::MAX, 0, 1, 1).is_err());
        assert!(validate_scene_rect(0, i64::MAX - 1, 1, 2).is_err());
    }

    #[test]
    fn scene_snapshot_validation_enforces_shared_structure() {
        let video = SceneValidationSource {
            key: SceneValidationKey {
                owner_id: 1,
                object_id: 10,
            },
            is_video: true,
            linked_video: None,
        };
        let audio = SceneValidationSource {
            key: SceneValidationKey {
                owner_id: 1,
                object_id: 11,
            },
            is_video: false,
            linked_video: Some(video.key),
        };
        let node = SceneValidationNode {
            owner_id: 1,
            node_id: 20,
            fragment_id: 0,
            source: video.key,
            x: 0,
            y: 0,
            width: 1 << 32,
            height: 1 << 32,
            clip: Some(ClipRect {
                x: 0,
                y: 0,
                width: 1 << 32,
                height: 1 << 32,
            }),
        };
        assert!(validate_scene_snapshot(&[video, audio], &[node]).is_ok());

        let foreign_audio = SceneValidationSource {
            linked_video: Some(SceneValidationKey {
                owner_id: 2,
                object_id: 10,
            }),
            ..audio
        };
        assert!(validate_scene_snapshot(&[video, foreign_audio], &[node]).is_err());
        let non_video_link = SceneValidationSource {
            linked_video: Some(audio.key),
            ..audio
        };
        assert!(validate_scene_snapshot(&[video, non_video_link], &[node]).is_err());
        assert!(validate_scene_snapshot(&[video], &[node, node]).is_err());

        let fragments = (0..=MAX_SCENE_FRAGMENTS_PER_NODE)
            .map(|fragment_id| SceneValidationNode {
                fragment_id: fragment_id as u64,
                ..node
            })
            .collect::<Vec<_>>();
        assert!(validate_scene_snapshot(&[video], &fragments).is_err());

        let nodes = (0..=MAX_SCENE_NODES)
            .map(|node_id| SceneValidationNode {
                node_id: node_id as u64,
                ..node
            })
            .collect::<Vec<_>>();
        assert!(validate_scene_snapshot(&[video], &nodes).is_err());

        let invalid_clip = SceneValidationNode {
            clip: Some(ClipRect {
                width: 0,
                ..node.clip.unwrap()
            }),
            ..node
        };
        assert!(validate_scene_snapshot(&[video], &[invalid_clip]).is_err());
    }

    #[test]
    fn flush_and_eos_parse_under_their_own_names() {
        let body = flush(9, 4, 7);
        let (envelope, source_id, epoch) = parse_flush(&body).unwrap();
        assert_eq!((envelope.request_id, source_id, epoch), (9, 4, 7));
        let body = eos(10, 4, 8);
        let (envelope, source_id, epoch) = parse_eos(&body).unwrap();
        assert_eq!((envelope.request_id, source_id, epoch), (10, 4, 8));
    }

    #[test]
    fn aac_audio_specific_config_validation() {
        // AAC-LC, 48 kHz, stereo.
        assert!(validate_aac_audio_specific_config(&[0x11, 0x90], 48_000, 2).is_ok());
        // HE-AAC convention: base configuration declares half the output rate.
        assert!(validate_aac_audio_specific_config(&[0x13, 0x10], 48_000, 2).is_ok());
        // Explicit 24-bit frequency (index 15).
        assert!(
            validate_aac_audio_specific_config(&[0x17, 0x80, 0x5D, 0xC0, 0x10], 48_000, 2).is_ok()
        );
        // Rate mismatch, channel mismatch, reserved index, truncation, null object type.
        assert!(validate_aac_audio_specific_config(&[0x12, 0x10], 48_000, 2).is_err());
        assert!(validate_aac_audio_specific_config(&[0x11, 0x88], 48_000, 2).is_err());
        assert!(validate_aac_audio_specific_config(&[0x16, 0x90], 48_000, 2).is_err());
        assert!(validate_aac_audio_specific_config(&[0x11], 48_000, 2).is_err());
        assert!(validate_aac_audio_specific_config(&[0x01, 0x90], 48_000, 2).is_err());
        // Program config element (channelConfiguration 0) accepts any declared count.
        assert!(validate_aac_audio_specific_config(&[0x11, 0x80], 48_000, 2).is_ok());
    }

    #[test]
    fn audio_probe_can_report_unsupported_codec_and_limits() {
        let unsupported = AudioSourceConfig {
            source_id: 0,
            linked_video_source_id: None,
            codec: "opus",
            packetization: "unsupported-audio-packetization",
            extradata: &[],
            sample_rate: 48_000,
            channels: 2,
            channel_mask: 3,
            bitrate: 128_000,
            max_access_unit_bytes: 4_096,
            codec_string: None,
        };
        let (_, parsed) = parse_create_audio(&probe_audio_config(8, &unsupported)).unwrap();
        assert!(!audio_config_supported(&parsed));

        let oversized = AudioSourceConfig {
            codec: "mp3",
            packetization: "mp3-frame-v1",
            max_access_unit_bytes: MAX_AUDIO_ACCESS_UNIT_BYTES + 1,
            ..unsupported
        };
        let (_, parsed) = parse_create_audio(&probe_audio_config(9, &oversized)).unwrap();
        assert!(!audio_config_supported(&parsed));
    }

    #[test]
    fn play_request_round_trip_and_policy_validation() {
        let request = PlayRequest {
            source_id: 19,
            start_pts_us: -25_000,
            minimum_buffer_us: 125_000,
            maximum_latency_us: 500_000,
            rate_32_32: 1_i64 << 32,
            late_policy: LATE_DROP_PRESENTATION,
            loop_count: 0,
            start_policy: START_AFTER_MINIMUM_BUFFER,
        };
        let (envelope, parsed) = parse_play(&play_request(41, &request)).unwrap();
        assert_eq!(envelope.request_id, 41);
        assert_eq!(parsed, request);

        assert!(
            PlayRequest {
                source_id: 0,
                ..request
            }
            .validate()
            .is_err()
        );
        assert!(
            PlayRequest {
                maximum_latency_us: 1,
                ..request
            }
            .validate()
            .is_err()
        );
        assert!(
            PlayRequest {
                loop_count: 1,
                ..request
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn rtt_buffer_formula_preserves_unsampled_requests_and_caps_samples() {
        assert_eq!(minimum_buffer_for_rtt(90_000, None), 90_000);
        assert_eq!(minimum_buffer_for_rtt(90_000, Some(10_000)), 90_000);
        assert_eq!(minimum_buffer_for_rtt(10_000, Some(50_000)), 125_000);
        assert_eq!(minimum_buffer_for_rtt(10_000, Some(u64::MAX)), 500_000);
    }

    #[test]
    fn canonical_portable_audio_initialization_is_validated() {
        let mut opus = b"OpusHead\x01\x02\x38\x01\x80\xbb\x00\x00\x00\x00\x00".to_vec();
        assert_eq!(opus.len(), 19);
        validate_opus_head(&opus, 48_000, 2).unwrap();
        opus[18] = 2;
        assert!(validate_opus_head(&opus, 48_000, 2).is_err());

        let mut identification = vec![0; 30];
        identification[..7].copy_from_slice(b"\x01vorbis");
        identification[11] = 2;
        identification[12..16].copy_from_slice(&48_000_u32.to_le_bytes());
        identification[28] = 0x86;
        identification[29] = 1;
        let mut comments = b"\x03vorbis".to_vec();
        comments.extend_from_slice(&0_u32.to_le_bytes());
        comments.extend_from_slice(&0_u32.to_le_bytes());
        comments.push(1);
        let setup = b"\x05vorbis-setup";
        let mut vorbis = vec![2, identification.len() as u8, comments.len() as u8];
        vorbis.extend_from_slice(&identification);
        vorbis.extend_from_slice(&comments);
        vorbis.extend_from_slice(setup);
        validate_vorbis_headers(&vorbis, 48_000, 2).unwrap();
        assert!(validate_vorbis_headers(&vorbis, 44_100, 2).is_err());

        let mut streaminfo = [0_u8; 34];
        streaminfo[0..2].copy_from_slice(&4096_u16.to_be_bytes());
        streaminfo[2..4].copy_from_slice(&4096_u16.to_be_bytes());
        let packed = (u64::from(48_000_u32) << 44)
            | (u64::from(2_u16 - 1) << 41)
            | (u64::from(16_u8 - 1) << 36);
        streaminfo[10..18].copy_from_slice(&packed.to_be_bytes());
        validate_flac_streaminfo(&streaminfo, 48_000, 2).unwrap();
        assert!(validate_flac_streaminfo(&streaminfo[..33], 48_000, 2).is_err());
    }
}
