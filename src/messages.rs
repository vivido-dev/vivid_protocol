//! Normative Vivid 1.0 numeric registry and deterministic control schemas.

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

pub const FEATURE_RASTER_RGBA8: u64 = 1;
pub const FEATURE_VIDEO_FFMPEG_PACKET_V0: u64 = 2;
pub const FEATURE_SCENE_TRANSACTIONS: u64 = 3;
pub const FEATURE_GRID_CELL_NODES: u64 = 4;
pub const FEATURE_CREDIT_FLOW_CONTROL: u64 = 5;
pub const FEATURE_TEXT_ANCHORS: u64 = 6;

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
pub const RASTER_FULL_FRAME: u64 = 0;
pub const COMPRESSION_NONE: u64 = 0;
pub const RETENTION_NONE: u64 = 0;
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
    pub required_features: Vec<u64>,
    pub maximum_record_body: u32,
}

#[derive(Debug, Clone)]
pub struct RasterSourceConfig {
    pub source_id: u64,
    pub width: u32,
    pub height: u32,
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
        key_u64(encoder, 1, 0);
        key_u64(encoder, 2, 1);
        key_u64(encoder, 3, 0);
        encoder.u64(4);
        encoder.text(token);
        encoder.u64(5);
        encoder.text("vivi");
        encoder.u64(6);
        encoder.text(env!("CARGO_PKG_VERSION"));
        encoder.u64(7);
        encoder.array(1);
        encoder.u64(FEATURE_TEXT_ANCHORS);
        encoder.u64(8);
        encoder.array(0);
        key_u64(encoder, 9, u64::from(super::CONTROL_MAX_RECORD_BODY));
    })
}

pub fn create_raster(request_id: u64, source_id: u64, width: u32, height: u32) -> Vec<u8> {
    envelope(request_id, None, None, |encoder| {
        encoder.map(9);
        key_u64(encoder, 0, source_id);
        key_u64(encoder, 1, u64::from(width));
        key_u64(encoder, 2, u64::from(height));
        key_u64(encoder, 3, PIXEL_FORMAT_RGBA8);
        key_u64(encoder, 4, ALPHA_STRAIGHT);
        key_u64(encoder, 5, RASTER_FULL_FRAME);
        key_u64(encoder, 6, 1);
        key_u64(encoder, 7, COMPRESSION_NONE);
        key_u64(encoder, 8, RETENTION_NONE);
    })
}

pub fn create_video(request_id: u64, config: &VideoSourceConfig<'_>) -> Vec<u8> {
    envelope(request_id, None, None, |encoder| {
        encoder.map(14);
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
    })
}

pub fn probe_video_config(request_id: u64, config: &VideoSourceConfig<'_>) -> Vec<u8> {
    create_video(request_id, config)
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
        key_i64(encoder, 4, 0);
        key_i64(encoder, 5, 0);
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
) -> Vec<u8> {
    envelope(request_id, None, None, |encoder| {
        encoder.map(13);
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
        encoder.array(3);
        encoder.text("raster-rgba8");
        encoder.text("video-ffmpeg-packet-v0");
        encoder.text("text-anchor-cell-v1");
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

pub fn source_ready(request_id: u64, source_id: u64, ticket: &[u8], credits: Credits) -> Vec<u8> {
    envelope(request_id, None, None, |encoder| {
        encoder.map(5);
        key_u64(encoder, 0, source_id);
        encoder.u64(1);
        encoder.bytes(ticket);
        key_u64(encoder, 2, credits.bytes);
        key_u64(encoder, 3, credits.packets);
        key_u64(encoder, 4, credits.fragments);
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
        encoder.text(diagnostic);
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
        payload: value
            .map_value(3)
            .cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing control payload"))?,
    })
}

pub fn parse_hello(body: &[u8]) -> io::Result<(u64, Hello)> {
    let envelope = decode_control(body)?;
    let payload = &envelope.payload;
    let maximum_record_body = u32::try_from(required_u64(payload, 9, "maximum record body")?)
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "maximum record body exceeds u32",
            )
        })?;
    Ok((
        envelope.request_id,
        Hello {
            minimum_major: required_u64(payload, 0, "minimum major version")?,
            minimum_minor: required_u64(payload, 1, "minimum minor version")?,
            maximum_major: required_u64(payload, 2, "maximum major version")?,
            maximum_minor: required_u64(payload, 3, "maximum minor version")?,
            token: required_text(payload, 4, "authentication token")?.to_owned(),
            producer: required_text(payload, 5, "producer name")?.to_owned(),
            required_features: payload
                .map_value(7)
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidData, "missing required features")
                })?
                .iter()
                .map(|value| {
                    value
                        .as_u64()
                        .ok_or_else(|| invalid("feature ID is not unsigned"))
                })
                .collect::<io::Result<Vec<_>>>()?,
            maximum_record_body,
        },
    ))
}

pub fn parse_create_raster(body: &[u8]) -> io::Result<(ControlEnvelope, RasterSourceConfig)> {
    let envelope = decode_control(body)?;
    let payload = &envelope.payload;
    let config = RasterSourceConfig {
        source_id: required_u64(payload, 0, "source ID")?,
        width: required_u32(payload, 1, "raster width")?,
        height: required_u32(payload, 2, "raster height")?,
    };
    if config.source_id == 0 {
        return Err(invalid("raster source ID is zero"));
    }
    if required_u64(payload, 3, "pixel format")? != PIXEL_FORMAT_RGBA8
        || required_u64(payload, 4, "alpha mode")? != ALPHA_STRAIGHT
        || required_u64(payload, 5, "raster mode")? != RASTER_FULL_FRAME
        || required_u64(payload, 6, "rectangle limit")? != 1
        || required_u64(payload, 7, "compression")? != COMPRESSION_NONE
        || required_u64(payload, 8, "retention")? != RETENTION_NONE
    {
        return Err(invalid("unsupported raster configuration"));
    }
    if config.width == 0 || config.height == 0 || config.width > 8192 || config.height > 8192 {
        return Err(invalid("raster dimensions are outside Vivid v1 limits"));
    }
    Ok((envelope, config))
}

pub fn parse_create_video(body: &[u8]) -> io::Result<(ControlEnvelope, ParsedVideoSourceConfig)> {
    let envelope = decode_control(body)?;
    let payload = &envelope.payload;
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
    };
    if config.width == 0 || config.height == 0 || config.width > 8192 || config.height > 8192 {
        return Err(invalid("video dimensions are outside Vivid v1 limits"));
    }
    if required_u64(payload, 8, "alpha mode")? != 0
        || required_u64(payload, 9, "latency mode")? != 0
        || required_u64(payload, 10, "retention mode")? != RETENTION_NONE
        || required_u64(payload, 12, "reorder depth")? > 64
        || required_text(payload, 13, "timeline name")? != "source-timebase-us"
    {
        return Err(invalid("unsupported video configuration"));
    }
    Ok((envelope, config))
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
    Ok(Welcome {
        session_id: required_u64(&payload, 0, "session ID")?,
        session_tag: required_bytes(&payload, 1, "session tag")?.to_vec(),
        root_context_id: required_u64(&payload, 2, "root context ID")?,
        display_generation: required_u64(&payload, 4, "display generation")?,
        grid_columns: required_u64(&payload, 7, "grid columns")?,
        grid_rows: required_u64(&payload, 8, "grid rows")?,
    })
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
    Ok(SourceReady {
        source_id: required_u64(&payload, 0, "source ID")?,
        media_ticket: required_bytes(&payload, 1, "media ticket")?.to_vec(),
        byte_credits: required_u64(&payload, 2, "byte credits")?,
        packet_credits: required_u64(&payload, 3, "packet credits")?,
        fragment_credits: payload.map_value(4).and_then(Value::as_u64).unwrap_or(0),
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
    let (_, payload) = decode_envelope(body)?;
    let code = required_u64(&payload, 0, "error code")?;
    let request_id = required_u64(&payload, 1, "failed request ID")?;
    let diagnostic = payload
        .map_value(5)
        .and_then(Value::as_text)
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
}
