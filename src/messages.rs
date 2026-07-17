//! Normative Vivid 1.1 numeric registry and deterministic control schemas.

#![allow(dead_code)]

use std::io;

use super::cbor::{self, Encoder, Value};

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

pub const PROFILE_RASTER_RGBA8: &str = "raster-rgba8-full-v1";
pub const PROFILE_RASTER_ZSTD: &str = "raster-zstd-full-v1";
pub const PROFILE_IMAGE_PNG_JPEG: &str = "image-png-jpeg-v1";
pub const PROFILE_VIDEO_ACCESS_UNIT: &str = "video-access-unit-v1";
pub const PROFILE_TEXT_ANCHOR_V2: &str = "text-anchor-cell-v2";
pub const PROFILE_VISIBILITY: &str = "visibility-source-v1";
pub const PROFILE_AUDIO_ACCESS_UNIT: &str = "audio-access-unit-v1";

pub const MAX_AUDIO_EXTRADATA: usize = 64 * 1024;
pub const MAX_AUDIO_ACCESS_UNIT_BYTES: u32 = 1024 * 1024;

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

#[derive(Debug, Clone)]
pub struct Welcome {
    pub session_id: u64,
    pub session_tag: Vec<u8>,
    pub root_context_id: u64,
    pub display_generation: u64,
    pub grid_columns: u64,
    pub grid_rows: u64,
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
}

pub struct NodeConfig {
    pub node_id: u64,
    pub source_id: u64,
    pub context_id: u64,
    pub columns: u32,
    pub rows: u32,
    pub anchor_id: Option<u64>,
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

#[derive(Debug, Clone)]
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

pub fn hello(request_id: u64, token: &str) -> Vec<u8> {
    envelope(request_id, None, None, |encoder| {
        encoder.map(10);
        key_u64(encoder, 0, 1);
        key_u64(encoder, 1, 1);
        key_u64(encoder, 2, 1);
        key_u64(encoder, 3, 1);
        encoder.u64(4);
        encoder.text(token);
        encoder.u64(5);
        encoder.text("vivi");
        encoder.u64(6);
        encoder.text(env!("CARGO_PKG_VERSION"));
        encoder.u64(7);
        encoder.array(5);
        encoder.u64(FEATURE_RASTER_RGBA8);
        encoder.u64(FEATURE_SCENE_TRANSACTIONS);
        encoder.u64(FEATURE_GRID_CELL_NODES);
        encoder.u64(FEATURE_CREDIT_FLOW_CONTROL);
        encoder.u64(FEATURE_TEXT_ANCHORS_V2);
        encoder.u64(8);
        encoder.array(7);
        encoder.u64(FEATURE_ENCODED_IMAGE_V1);
        encoder.u64(FEATURE_RASTER_ZSTD_V1);
        encoder.u64(FEATURE_RASTER_PREMULTIPLIED_ALPHA);
        encoder.u64(FEATURE_VISIBILITY_EVENTS_V1);
        encoder.u64(FEATURE_VIDEO_ACCESS_UNIT_V1);
        encoder.u64(FEATURE_VIDEO_CONTROL_V1);
        encoder.u64(FEATURE_AUDIO_ACCESS_UNIT_V1);
        key_u64(encoder, 9, u64::from(super::CONTROL_MAX_RECORD_BODY));
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
        encoder.map(21);
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
    })
}

pub fn probe_video_config(request_id: u64, config: &VideoSourceConfig<'_>) -> Vec<u8> {
    create_video(request_id, config)
}

pub fn create_audio(request_id: u64, config: &AudioSourceConfig<'_>) -> Vec<u8> {
    envelope(request_id, None, None, |encoder| {
        encoder.map(11);
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
    envelope(request_id, Some(transaction_id), None, |encoder| {
        encoder.map(if node.anchor_id.is_some() { 15 } else { 14 });
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
        key_i64(encoder, 4, x);
        key_i64(encoder, 5, y);
        key_i64(encoder, 6, fixed_cells(node.columns));
        key_i64(encoder, 7, fixed_cells(node.rows));
        key_u64(encoder, 8, FIT_CONTAIN);
        key_u64(encoder, 9, SAMPLING_LINEAR);
        key_u64(encoder, 10, TEXT_LAYER_BETWEEN_BACKGROUND_AND_GLYPH);
        key_i64(encoder, 11, 0);
        key_u64(encoder, 12, BLEND_SOURCE_OVER);
        encoder.u64(13);
        encoder.bool(true);
        if let Some(anchor_id) = node.anchor_id {
            key_u64(encoder, 14, anchor_id);
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

pub fn play(request_id: u64, source_id: u64, minimum_buffer_us: u64) -> Vec<u8> {
    envelope(request_id, None, None, |encoder| {
        encoder.map(8);
        key_u64(encoder, 0, source_id);
        key_i64(encoder, 1, 0);
        key_u64(encoder, 2, minimum_buffer_us);
        key_u64(encoder, 3, 500_000);
        key_i64(encoder, 4, 1_i64 << 32);
        key_u64(encoder, 5, LATE_DROP_PRESENTATION);
        key_u64(encoder, 6, 0);
        key_u64(encoder, 7, START_AFTER_MINIMUM_BUFFER);
    })
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
    envelope(request_id, None, None, |encoder| {
        encoder.map(16);
        key_u64(encoder, 0, session_id);
        encoder.u64(1);
        encoder.bytes(session_tag);
        key_u64(encoder, 2, root_context_id);
        key_u64(encoder, 3, 1); // capability generation
        key_u64(encoder, 4, display.display_generation);
        key_u64(encoder, 5, u64::from(display.viewport_width));
        key_u64(encoder, 6, u64::from(display.viewport_height));
        key_u64(encoder, 7, u64::from(display.grid_columns));
        key_u64(encoder, 8, u64::from(display.grid_rows));
        key_u64(encoder, 9, u64::from(display.cell_width));
        key_u64(encoder, 10, u64::from(display.cell_height));
        key_u64(encoder, 11, u64::from(super::CONTROL_MAX_RECORD_BODY));
        encoder.u64(12);
        encoder.array(7);
        encoder.text(PROFILE_AUDIO_ACCESS_UNIT);
        encoder.text(PROFILE_IMAGE_PNG_JPEG);
        encoder.text(PROFILE_RASTER_RGBA8);
        encoder.text(PROFILE_RASTER_ZSTD);
        encoder.text(PROFILE_TEXT_ANCHOR_V2);
        encoder.text(PROFILE_VIDEO_ACCESS_UNIT);
        encoder.text(PROFILE_VISIBILITY);
        key_u64(encoder, 13, 1);
        key_u64(encoder, 14, 1);
        encoder.u64(15);
        encoder.array(accepted_features.len());
        for feature in accepted_features {
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
    let _ = source_id;
    envelope(0, None, None, |encoder| {
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
    envelope(request_id, None, None, |encoder| encoder.map(0))
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
    envelope(0, None, None, |encoder| {
        encoder.map(3);
        key_u64(encoder, 0, bytes);
        key_u64(encoder, 1, packets);
        key_u64(encoder, 2, fragments);
    })
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
        return Err(invalid("HELLO protocol range is reversed"));
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
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20,
        ],
    )?;
    let profile = required_i64(payload, 6, "video profile")?;
    let level = required_i64(payload, 7, "video level")?;
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
    };
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
    reject_unknown_fields(payload, &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10])?;
    let channels = required_u32(payload, 6, "audio channel count")?;
    let linked = required_u64(payload, 1, "linked video source ID")?;
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
    };
    if config.linked_video_source_id == Some(config.source_id) {
        return Err(invalid("audio source cannot link to itself"));
    }
    if required_text(payload, 10, "audio timeline")? != "source-timebase-us" {
        return Err(invalid("unsupported audio configuration"));
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
        && valid_audio_packetization(&config.codec, &config.packetization)
}

pub fn valid_audio_packetization(codec: &str, packetization: &str) -> bool {
    match codec {
        "mp3" => packetization == "mp3-frame-v1",
        "aac" => packetization == "aac-raw-au-v1",
        "alac" => packetization == "alac-frame-v1",
        "pcm_u8" | "pcm_s16le" | "pcm_s24le" | "pcm_s32le" | "pcm_f32le" | "pcm_f64le"
        | "pcm_mulaw" | "pcm_alaw" => packetization == "pcm-packet-v1",
        _ => false,
    }
}

pub fn parse_create_node(body: &[u8]) -> io::Result<(ControlEnvelope, ParsedNodeConfig)> {
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
    Ok((envelope.clone(), parsed))
}

pub fn parse_anchor_event(body: &[u8]) -> io::Result<u64> {
    let (_, payload) = decode_envelope(body)?;
    required_u64(&payload, 0, "anchor ID")
}

pub fn parse_update_node(body: &[u8]) -> io::Result<(ControlEnvelope, ParsedNodeConfig)> {
    parse_create_node(body)
}

pub fn parse_object_id(body: &[u8], description: &str) -> io::Result<(ControlEnvelope, u64)> {
    let envelope = decode_control(body)?;
    let object_id = required_u64(&envelope.payload, 0, description)?;
    Ok((envelope, object_id))
}

pub fn parse_eos(body: &[u8]) -> io::Result<(ControlEnvelope, u64, u32)> {
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
        grid_columns: required_u64(&payload, 7, "grid columns")?,
        grid_rows: required_u64(&payload, 8, "grid rows")?,
        maximum_control_body: required_u32(&payload, 11, "maximum control body")?,
        accepted_profiles: text_array(&payload, 12, "accepted profiles")?,
        selected_major: required_u64(&payload, 13, "selected protocol major")?,
        selected_minor: required_u64(&payload, 14, "selected protocol minor")?,
        accepted_features: feature_array(&payload, 15, "accepted features")?,
    };
    if welcome.session_id == 0
        || welcome.root_context_id == 0
        || welcome.session_tag.len() != 16
        || welcome.maximum_control_body == 0
        || welcome.maximum_control_body > super::CONTROL_MAX_RECORD_BODY
        || (welcome.selected_major, welcome.selected_minor) != (1, 1)
    {
        return Err(invalid("WELCOME contains an invalid mandatory 1.1 field"));
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

fn envelope(
    request_id: u64,
    transaction_id: Option<u64>,
    expected_generation: Option<u64>,
    payload: impl FnOnce(&mut Encoder),
) -> Vec<u8> {
    let mut encoder = Encoder::new();
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
    encoder.into_vec()
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
    fn welcome_and_source_ready_require_protocol_1_1_fields() {
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
        assert_eq!((parsed.selected_major, parsed.selected_minor), (1, 1));
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
            extradata: &[0x12, 0x10],
            sample_rate: 48_000,
            channels: 2,
            channel_mask: 3,
            bitrate: 192_000,
            max_access_unit_bytes: 8_192,
        };
        let (envelope, parsed) = parse_create_audio(&create_audio(7, &config)).unwrap();
        assert_eq!(envelope.request_id, 7);
        assert_eq!(parsed.source_id, 12);
        assert_eq!(parsed.linked_video_source_id, Some(10));
        assert_eq!(parsed.codec, "aac");
        assert_eq!(parsed.extradata, [0x12, 0x10]);
        assert!(audio_config_supported(&parsed));
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
}
