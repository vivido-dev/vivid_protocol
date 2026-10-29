//! Binary layouts for media records and validation of codec initialization data.
//!
//! Media travels on track channels as fixed big-endian prefixes followed by payload bytes:
//!
//! - **Video and audio packets** carry a 48-byte prefix (epoch, packet ID, presentation and decode
//!   timestamps, duration, and video key-frame flags or audio trim counts) and one encoded access
//!   unit. [`video_packet_body`] and [`audio_packet_body`] build them;
//!   [`parse_video_packet`] and [`parse_audio_packet`] borrow them back out of a received body.
//! - **Raster frames** carry RGBA8 pixels, either a full frame or a delta of overwrite and copy
//!   operations against an earlier frame, optionally zstd-compressed. [`raster_frame_body`],
//!   [`raster_delta_frame_body`], [`parse_full_raster_frame`], and [`parse_delta_raster_frame`]
//!   handle them; [`decode_raster_pixels`] decompresses.
//!
//! The `*_body_len` functions compute body sizes with checked arithmetic and reject anything above
//! [`HARD_MAX_RECORD_BODY`], so a peer's declared dimensions cannot force an oversized allocation.
//!
//! The rest of the module validates what a track declares before any decoder sees it: portable
//! codec/packetization pairs, AAC, Opus, Vorbis, and FLAC headers, and H.264 parameter sets in
//! both avcC and Annex B form.
//!
//! zstd compression needs the `native` feature; wasm32 builds decode with a pure-Rust zstd
//! implementation instead.

use std::{borrow::Cow, fmt};

use crate::{CodecError, codec::Kind};

#[cfg(all(feature = "native", not(target_arch = "wasm32")))]
use std::io::Cursor;

#[cfg(any(feature = "native", target_arch = "wasm32"))]
use std::io::Read;

use crate::HARD_MAX_RECORD_BODY;

/// zstd level for compressed raster frames: the fastest standard level.
///
/// Producers compress every frame on the send path, where latency matters more than ratio.
/// Receivers decode any level, so changing it affects only bandwidth and producer CPU.
#[cfg(all(feature = "native", not(target_arch = "wasm32")))]
const RASTER_ZSTD_LEVEL: i32 = 1;

/// Video packet flag: decoding can begin with this access unit (bit 0).
pub const VIDEO_PACKET_KEY: u32 = 1 << 0;
/// Video packet flag: the access unit depends on earlier ones (bit 1).
pub const VIDEO_PACKET_DELTA: u32 = 1 << 1;
/// Raster frame flag: a full frame (bit 0).
pub const RASTER_FRAME_FULL: u32 = 1 << 0;
/// Raster frame flag: the pixel data is one zstd frame (bit 1).
pub const RASTER_FRAME_ZSTD: u32 = 1 << 1;
/// Raster frame flag: a delta against the preceding frame (bit 2).
pub const RASTER_FRAME_DELTA: u32 = 1 << 2;
/// Delta operation kind that overwrites a rectangle.
pub const RASTER_DELTA_OVERWRITE: u32 = 1;
/// Delta operation kind that copies a rectangle from the base frame.
pub const RASTER_DELTA_COPY: u32 = 2;
/// Most operations one delta frame may carry.
pub const RASTER_DELTA_OPERATION_LIMIT: u32 = 16;

const VIDEO_PACKET_PREFIX_SIZE: usize = 48;
const AUDIO_PACKET_PREFIX_SIZE: usize = 48;
const RASTER_FRAME_PREFIX_SIZE: usize = 48;
const RASTER_RECT_SIZE: usize = 24;
const RASTER_DELTA_OPERATION_SIZE: usize = 32;

/// A `VIDEO_PACKET` body split into its fields (media §9).
#[derive(Debug, Clone, Copy)]
pub struct ParsedVideoPacket<'a> {
    /// Media epoch; epochs never decrease.
    pub epoch: u32,
    /// [`VIDEO_PACKET_KEY`] or [`VIDEO_PACKET_DELTA`]; exactly one is set.
    pub flags: u32,
    /// Nonzero packet ID; IDs strictly increase for the track.
    pub packet_id: u64,
    /// Presentation time in signed microseconds.
    pub pts_us: i64,
    /// Decode time in signed microseconds.
    pub dts_us: i64,
    /// Duration in microseconds.
    pub duration_us: u64,
    /// Side data; always empty in Vivid 1.5.
    pub side_data: &'a [u8],
    /// One encoded access unit in the track's packetization.
    pub data: &'a [u8],
}

/// An `AUDIO_PACKET` body split into its fields (media §10).
#[derive(Debug, Clone, Copy)]
pub struct ParsedAudioPacket<'a> {
    /// Media epoch; epochs never decrease.
    pub epoch: u32,
    /// Nonzero packet ID; IDs strictly increase for the track.
    pub packet_id: u64,
    /// Presentation time in signed microseconds.
    pub pts_us: i64,
    /// Decode time in signed microseconds.
    pub dts_us: i64,
    /// Duration in microseconds.
    pub duration_us: u64,
    /// Decoded samples to drop from the start.
    pub trim_start_samples: u32,
    /// Decoded samples to drop from the end.
    pub trim_end_samples: u32,
    /// One encoded access unit; nonempty and at most 1 MiB.
    pub data: &'a [u8],
}

/// A full `RASTER_FRAME` body split into its fields (media §11.1).
#[derive(Debug, Clone, Copy)]
pub struct ParsedRasterFrame<'a> {
    /// Media epoch; epochs never decrease.
    pub epoch: u32,
    /// Nonzero frame ID; IDs strictly increase for the track.
    pub frame_id: u64,
    /// Presentation time in signed microseconds; metadata in live mode.
    pub pts_us: i64,
    /// Duration in microseconds.
    pub duration_us: u64,
    /// Frame width.
    pub width: u32,
    /// Frame height.
    pub height: u32,
    /// Whether the pixel data is one zstd frame.
    pub compressed: bool,
    /// Raw row-major RGBA8 pixels, or one zstd frame when `compressed`.
    pub pixels: &'a [u8],
}

/// Width and height of a raster, in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RasterSize {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
}

/// A full RGBA8 raster frame to encode with [`raster_frame_body`].
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct RasterFrame<'a> {
    /// The media epoch; a new epoch must begin with a full frame.
    pub epoch: u32,
    /// The nonzero frame ID, increasing within the track.
    pub frame_id: u64,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Four bytes per pixel, row-major, straight alpha.
    pub rgba: &'a [u8],
}

impl fmt::Debug for RasterFrame<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RasterFrame")
            .field("epoch", &self.epoch)
            .field("frame_id", &self.frame_id)
            .field("width", &self.width)
            .field("height", &self.height)
            .field("rgba_len", &self.rgba.len())
            .finish()
    }
}

/// A raster delta frame to encode with [`raster_delta_frame_body`].
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct RasterDeltaFrame<'a> {
    /// The media epoch, which must match the base frame's.
    pub epoch: u32,
    /// The nonzero frame ID of the frame this delta produces.
    pub frame_id: u64,
    /// The nonzero ID of the frame the operations patch.
    pub base_frame_id: u64,
    /// Presentation time in microseconds.
    pub pts_us: i64,
    /// Display duration in microseconds.
    pub duration_us: u64,
    /// Width of the base frame in pixels.
    pub width: u32,
    /// Height of the base frame in pixels.
    pub height: u32,
    /// The overwrite and copy operations, applied in order.
    pub operations: &'a [RasterDeltaOperation<'a>],
}

impl fmt::Debug for RasterDeltaFrame<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RasterDeltaFrame")
            .field("epoch", &self.epoch)
            .field("frame_id", &self.frame_id)
            .field("base_frame_id", &self.base_frame_id)
            .field("width", &self.width)
            .field("height", &self.height)
            .field("operations", &self.operations.len())
            .finish_non_exhaustive()
    }
}

/// One delta operation to encode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RasterDeltaOperation<'a> {
    /// Replaces a rectangle with new pixels (kind 1).
    Overwrite {
        /// Destination left edge.
        x: u32,
        /// Destination top edge.
        y: u32,
        /// Width of the rectangle.
        width: u32,
        /// Height of the rectangle.
        height: u32,
        /// Row-major RGBA8 pixels for the rectangle.
        rgba: &'a [u8],
    },
    /// Copies a rectangle of the base frame (kind 2).
    ///
    /// The copy behaves as if through a temporary, so overlap is exact.
    Copy {
        /// Destination left edge.
        destination_x: u32,
        /// Destination top edge.
        destination_y: u32,
        /// Width of the rectangle.
        width: u32,
        /// Height of the rectangle.
        height: u32,
        /// Source left edge in the base frame.
        source_x: u32,
        /// Source top edge in the base frame.
        source_y: u32,
    },
}

/// One decoded delta operation.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ParsedRasterDeltaOperation<'a> {
    /// Replaces a rectangle with new pixels (kind 1).
    Overwrite {
        /// Destination left edge.
        x: u32,
        /// Destination top edge.
        y: u32,
        /// Width of the rectangle.
        width: u32,
        /// Height of the rectangle.
        height: u32,
        /// Row-major RGBA8 pixels, decompressed when the frame was compressed.
        rgba: Cow<'a, [u8]>,
    },
    /// Copies a rectangle of the base frame (kind 2).
    ///
    /// The copy behaves as if through a temporary, so overlap is exact.
    Copy {
        /// Destination left edge.
        destination_x: u32,
        /// Destination top edge.
        destination_y: u32,
        /// Width of the rectangle.
        width: u32,
        /// Height of the rectangle.
        height: u32,
        /// Source left edge in the base frame.
        source_x: u32,
        /// Source top edge in the base frame.
        source_y: u32,
    },
}

/// A delta `RASTER_FRAME` body split into its fields (media §11.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedRasterDeltaFrame<'a> {
    /// Media epoch; epochs never decrease.
    pub epoch: u32,
    /// Nonzero frame ID; IDs strictly increase for the track.
    pub frame_id: u64,
    /// The immediately preceding accepted frame this delta applies to.
    pub base_frame_id: u64,
    /// Presentation time in signed microseconds; metadata in live mode.
    pub pts_us: i64,
    /// Duration in microseconds.
    pub duration_us: u64,
    /// Whether the pixel data is one zstd frame.
    pub compressed: bool,
    /// Operations, applied in order.
    pub operations: Vec<ParsedRasterDeltaOperation<'a>>,
}

/// Why a media dimension or length cannot become a record body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SizeError {
    /// Computing the size overflows.
    Overflow,
    /// The size exceeds its limit: `u32` for pixel data, [`HARD_MAX_RECORD_BODY`] for bodies.
    TooLarge,
    /// A dimension or length is zero.
    Empty,
}

impl std::fmt::Display for SizeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Overflow => "media size computation overflows",
            Self::TooLarge => "media size exceeds the record limit",
            Self::Empty => "media size is zero",
        })
    }
}

impl std::error::Error for SizeError {}

/// Returns the RGBA8 byte length of a `width` by `height` image.
///
/// # Errors
///
/// Returns [`SizeError::Empty`] when either dimension is zero, [`SizeError::Overflow`] when the
/// product overflows, and [`SizeError::TooLarge`] when it does not fit in `u32`.
pub fn rgba8_pixel_len(width: u32, height: u32) -> Result<u32, SizeError> {
    if width == 0 || height == 0 {
        return Err(SizeError::Empty);
    }
    u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or(SizeError::Overflow)
        .and_then(|bytes| u32::try_from(bytes).map_err(|_| SizeError::TooLarge))
}

/// Returns the body length of an uncompressed full-frame raster of `width` by `height`.
///
/// # Errors
///
/// Returns [`SizeError::Empty`] for a zero dimension, [`SizeError::Overflow`] when the length
/// overflows, and [`SizeError::TooLarge`] when it exceeds [`HARD_MAX_RECORD_BODY`].
pub fn rgba8_raw_frame_body_len(width: u32, height: u32) -> Result<u32, SizeError> {
    let bytes = u64::from(rgba8_pixel_len(width, height)?)
        .checked_add((RASTER_FRAME_PREFIX_SIZE + RASTER_RECT_SIZE) as u64)
        .ok_or(SizeError::Overflow)?;
    let bytes = u32::try_from(bytes).map_err(|_| SizeError::TooLarge)?;
    if bytes > HARD_MAX_RECORD_BODY {
        Err(SizeError::TooLarge)
    } else {
        Ok(bytes)
    }
}

/// Returns the largest video packet body for `max_access_unit_bytes` of payload.
///
/// # Errors
///
/// Returns [`SizeError::Empty`] when `max_access_unit_bytes` is zero, [`SizeError::Overflow`] when
/// adding the prefix overflows, and [`SizeError::TooLarge`] when the body exceeds
/// [`HARD_MAX_RECORD_BODY`].
pub fn video_body_len(max_access_unit_bytes: u32) -> Result<u32, SizeError> {
    if max_access_unit_bytes == 0 {
        return Err(SizeError::Empty);
    }
    let bytes = max_access_unit_bytes
        .checked_add(const { crate::const_u32(VIDEO_PACKET_PREFIX_SIZE) })
        .ok_or(SizeError::Overflow)?;
    if bytes > HARD_MAX_RECORD_BODY {
        Err(SizeError::TooLarge)
    } else {
        Ok(bytes)
    }
}

/// Returns the largest audio packet body for `max_access_unit_bytes` of payload.
///
/// # Errors
///
/// Returns [`SizeError::Empty`] when `max_access_unit_bytes` is zero, [`SizeError::Overflow`] when
/// adding the prefix overflows, and [`SizeError::TooLarge`] when the body exceeds
/// [`HARD_MAX_RECORD_BODY`].
pub fn audio_body_len(max_access_unit_bytes: u32) -> Result<u32, SizeError> {
    if max_access_unit_bytes == 0 {
        return Err(SizeError::Empty);
    }
    let bytes = max_access_unit_bytes
        .checked_add(const { crate::const_u32(AUDIO_PACKET_PREFIX_SIZE) })
        .ok_or(SizeError::Overflow)?;
    if bytes > HARD_MAX_RECORD_BODY {
        Err(SizeError::TooLarge)
    } else {
        Ok(bytes)
    }
}

/// Checks that media IDs strictly increase and epochs never decrease on one track.
#[derive(Debug, Clone, Copy, Default)]
pub struct MediaSequence {
    last_id: u64,
    last_epoch: u32,
}

impl MediaSequence {
    /// Records a received media ID and epoch, enforcing their ordering.
    ///
    /// # Errors
    ///
    /// Returns a malformed-input [`CodecError`] when `id` is zero or not greater than the last ID,
    /// or `epoch` is lower than the last epoch.
    pub fn accept(&mut self, id: u64, epoch: u32) -> Result<(), CodecError> {
        if id == 0 || id <= self.last_id {
            return Err(invalid("media ID is zero or not strictly increasing"));
        }
        if epoch < self.last_epoch {
            return Err(invalid("media epoch moved backward"));
        }
        self.last_id = id;
        self.last_epoch = epoch;
        Ok(())
    }

    /// Returns the epoch of the last accepted record.
    pub fn epoch(&self) -> u32 {
        self.last_epoch
    }
}

/// One video access unit to encode as a `VIDEO_PACKET` body (media §9).
///
/// Debug output reports the payload length, never the encoded media bytes.
#[derive(Clone, Copy)]
pub struct VideoPacket<'a> {
    /// Media epoch; epochs never decrease.
    pub epoch: u32,
    /// Nonzero packet ID; IDs strictly increase for the track.
    pub packet_id: u64,
    /// Presentation time in signed microseconds.
    pub pts_us: i64,
    /// Decode time in signed microseconds.
    pub dts_us: i64,
    /// Duration in microseconds.
    pub duration_us: u64,
    /// Whether decoding can begin with this access unit.
    pub key: bool,
    /// One encoded access unit in the track's packetization.
    pub data: &'a [u8],
}

/// One audio access unit to encode as an `AUDIO_PACKET` body (media §10).
///
/// Debug output reports the payload length, never the encoded media bytes.
#[derive(Clone, Copy)]
pub struct AudioPacket<'a> {
    /// Media epoch; epochs never decrease.
    pub epoch: u32,
    /// Nonzero packet ID; IDs strictly increase for the track.
    pub packet_id: u64,
    /// Presentation time in signed microseconds.
    pub pts_us: i64,
    /// Decode time in signed microseconds.
    pub dts_us: i64,
    /// Duration in microseconds.
    pub duration_us: u64,
    /// Decoded samples to drop from the start.
    pub trim_start_samples: u32,
    /// Decoded samples to drop from the end.
    pub trim_end_samples: u32,
    /// One encoded access unit; nonempty and at most 1 MiB.
    pub data: &'a [u8],
}

impl fmt::Debug for VideoPacket<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VideoPacket")
            .field("epoch", &self.epoch)
            .field("packet_id", &self.packet_id)
            .field("pts_us", &self.pts_us)
            .field("dts_us", &self.dts_us)
            .field("duration_us", &self.duration_us)
            .field("key", &self.key)
            .field("data_len", &self.data.len())
            .finish()
    }
}

impl fmt::Debug for AudioPacket<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AudioPacket")
            .field("epoch", &self.epoch)
            .field("packet_id", &self.packet_id)
            .field("pts_us", &self.pts_us)
            .field("dts_us", &self.dts_us)
            .field("duration_us", &self.duration_us)
            .field("trim_start_samples", &self.trim_start_samples)
            .field("trim_end_samples", &self.trim_end_samples)
            .field("data_len", &self.data.len())
            .finish()
    }
}

/// Builds the 48-byte audio packet prefix for `packet`.
///
/// # Errors
///
/// Returns an invalid-argument [`CodecError`] when the body would exceed [`HARD_MAX_RECORD_BODY`].
pub fn audio_packet_prefix(
    packet: &AudioPacket<'_>,
) -> Result<[u8; AUDIO_PACKET_PREFIX_SIZE], CodecError> {
    packet
        .data
        .len()
        .checked_add(AUDIO_PACKET_PREFIX_SIZE)
        .and_then(|length| u32::try_from(length).ok())
        .filter(|length| *length <= HARD_MAX_RECORD_BODY)
        .ok_or_else(|| CodecError::invalid_argument("audio packet too large"))?;
    let mut prefix = [0_u8; AUDIO_PACKET_PREFIX_SIZE];
    put_u32(&mut prefix, 0, packet.epoch);
    put_u32(&mut prefix, 4, 0);
    put_u64(&mut prefix, 8, packet.packet_id);
    put_i64(&mut prefix, 16, packet.pts_us);
    put_i64(&mut prefix, 24, packet.dts_us);
    put_u64(&mut prefix, 32, packet.duration_us);
    put_u32(&mut prefix, 40, packet.trim_start_samples);
    put_u32(&mut prefix, 44, packet.trim_end_samples);
    Ok(prefix)
}

/// Builds a complete audio packet body: prefix and access unit.
///
/// # Errors
///
/// Returns an invalid-argument [`CodecError`] when the body would exceed [`HARD_MAX_RECORD_BODY`].
pub fn audio_packet_body(packet: AudioPacket<'_>) -> Result<Vec<u8>, CodecError> {
    let prefix = audio_packet_prefix(&packet)?;
    let capacity = prefix.len() + packet.data.len();
    let mut body = Vec::with_capacity(capacity);
    body.extend_from_slice(&prefix);
    body.extend_from_slice(packet.data);
    Ok(body)
}

/// Builds the 48-byte video packet prefix for `packet`.
///
/// # Errors
///
/// Returns an invalid-argument [`CodecError`] when the body would exceed [`HARD_MAX_RECORD_BODY`].
pub fn video_packet_prefix(
    packet: &VideoPacket<'_>,
) -> Result<[u8; VIDEO_PACKET_PREFIX_SIZE], CodecError> {
    packet
        .data
        .len()
        .checked_add(VIDEO_PACKET_PREFIX_SIZE)
        .and_then(|length| u32::try_from(length).ok())
        .filter(|length| *length <= HARD_MAX_RECORD_BODY)
        .ok_or_else(|| CodecError::invalid_argument("video packet too large"))?;
    let mut prefix = [0_u8; VIDEO_PACKET_PREFIX_SIZE];
    put_u32(&mut prefix, 0, packet.epoch);
    put_u32(
        &mut prefix,
        4,
        if packet.key {
            VIDEO_PACKET_KEY
        } else {
            VIDEO_PACKET_DELTA
        },
    );
    put_u64(&mut prefix, 8, packet.packet_id);
    put_i64(&mut prefix, 16, packet.pts_us);
    put_i64(&mut prefix, 24, packet.dts_us);
    put_u64(&mut prefix, 32, packet.duration_us);
    put_u32(&mut prefix, 40, 0);
    put_u32(&mut prefix, 44, 0);
    Ok(prefix)
}

/// Builds a complete video packet body: prefix and access unit.
///
/// # Errors
///
/// Returns an invalid-argument [`CodecError`] when the body would exceed [`HARD_MAX_RECORD_BODY`].
pub fn video_packet_body(packet: VideoPacket<'_>) -> Result<Vec<u8>, CodecError> {
    let prefix = video_packet_prefix(&packet)?;
    let capacity = prefix.len() + packet.data.len();
    let mut body = Vec::with_capacity(capacity);
    body.extend_from_slice(&prefix);
    body.extend_from_slice(packet.data);
    Ok(body)
}

/// Builds the prefix of an uncompressed full-frame raster whose pixels follow separately.
///
/// # Errors
///
/// Returns an invalid-argument [`CodecError`] when a dimension is zero, `frame.rgba` is not four
/// bytes per pixel, or the body would exceed [`HARD_MAX_RECORD_BODY`].
pub fn raster_full_frame_prefix(
    frame: &RasterFrame<'_>,
) -> Result<[u8; RASTER_FRAME_PREFIX_SIZE + RASTER_RECT_SIZE], CodecError> {
    let &RasterFrame {
        epoch,
        frame_id,
        width,
        height,
        rgba,
    } = frame;
    let data_len = rgba.len();
    let expected_length = rgba8_pixel_len(width, height).map_err(|error| {
        CodecError::invalid_argument("invalid raster dimensions").with_source(error)
    })? as usize;
    if data_len != expected_length {
        return Err(CodecError::invalid_argument(format!(
            "RGBA data has {data_len} bytes, expected {expected_length}"
        )));
    }
    rgba8_raw_frame_body_len(width, height).map_err(|error| {
        CodecError::invalid_argument("invalid raster dimensions").with_source(error)
    })?;
    raster_full_frame_prefix_inner(epoch, frame_id, width, height, data_len, false)
}

fn raster_full_frame_prefix_inner(
    epoch: u32,
    frame_id: u64,
    width: u32,
    height: u32,
    data_len: usize,
    compressed: bool,
) -> Result<[u8; RASTER_FRAME_PREFIX_SIZE + RASTER_RECT_SIZE], CodecError> {
    if width == 0 || height == 0 {
        return Err(CodecError::invalid_argument("raster dimensions are empty"));
    }
    let data_len = u32::try_from(data_len)
        .map_err(|_| CodecError::invalid_argument("raster frame exceeds u32 length"))?;
    data_len
        .checked_add(const { crate::const_u32(RASTER_FRAME_PREFIX_SIZE + RASTER_RECT_SIZE) })
        .filter(|length| *length <= HARD_MAX_RECORD_BODY)
        .ok_or_else(|| CodecError::invalid_argument("raster frame too large"))?;

    let mut prefix = [0_u8; RASTER_FRAME_PREFIX_SIZE + RASTER_RECT_SIZE];
    put_u32(&mut prefix, 0, epoch);
    put_u32(
        &mut prefix,
        4,
        RASTER_FRAME_FULL | if compressed { RASTER_FRAME_ZSTD } else { 0 },
    );
    put_u64(&mut prefix, 8, frame_id);
    put_u64(&mut prefix, 16, 0);
    put_i64(&mut prefix, 24, 0);
    put_u64(&mut prefix, 32, 0);
    put_u32(&mut prefix, 40, 1);
    put_u32(&mut prefix, 44, 0);
    put_u32(&mut prefix, 48, 0);
    put_u32(&mut prefix, 52, 0);
    put_u32(&mut prefix, 56, width);
    put_u32(&mut prefix, 60, height);
    put_u32(&mut prefix, 64, 0);
    put_u32(&mut prefix, 68, data_len);
    Ok(prefix)
}

/// Builds an uncompressed full-frame raster body from `frame`.
///
/// # Errors
///
/// Returns an invalid-argument [`CodecError`] when a dimension is zero, `frame.rgba` is not
/// exactly four bytes per pixel, or the body would exceed [`HARD_MAX_RECORD_BODY`].
///
/// # Examples
///
/// ```
/// use vivid_protocol::media::{self, RasterFrame};
///
/// let rgba = [0x20, 0x40, 0x60, 0xff].repeat(2 * 2);
/// let body = media::raster_frame_body(&RasterFrame {
///     epoch: 1,
///     frame_id: 1,
///     width: 2,
///     height: 2,
///     rgba: &rgba,
/// })?;
///
/// let parsed = media::parse_full_raster_frame(&body)?;
/// assert_eq!((parsed.width, parsed.height), (2, 2));
/// assert_eq!(media::decode_raster_pixels(parsed)?, rgba);
/// # Ok::<(), vivid_protocol::CodecError>(())
/// ```
pub fn raster_frame_body(frame: &RasterFrame<'_>) -> Result<Vec<u8>, CodecError> {
    raster_frame_body_with_compression(frame, false)
}

/// Builds a full-frame raster body, zstd-compressing the pixels when `compress` is set.
///
/// # Errors
///
/// Returns an invalid-argument [`CodecError`] when a dimension is zero, `frame.rgba` is not exactly
/// four bytes per pixel, or the body would exceed [`HARD_MAX_RECORD_BODY`]; an unsupported
/// [`CodecError`] when compression is requested without the `native` feature or on wasm32; and a
/// malformed-input [`CodecError`] when compression fails.
pub fn raster_frame_body_with_compression(
    frame: &RasterFrame<'_>,
    compress: bool,
) -> Result<Vec<u8>, CodecError> {
    let &RasterFrame {
        epoch,
        frame_id,
        width,
        height,
        rgba,
    } = frame;
    let expected_length = rgba8_pixel_len(width, height).map_err(|error| {
        CodecError::invalid_argument("invalid raster dimensions").with_source(error)
    })? as usize;
    if rgba.len() != expected_length {
        return Err(CodecError::invalid_argument(format!(
            "RGBA data has {} bytes, expected {expected_length}",
            rgba.len()
        )));
    }
    #[cfg(all(feature = "native", not(target_arch = "wasm32")))]
    let compressed;
    let pixels = if compress {
        #[cfg(all(feature = "native", not(target_arch = "wasm32")))]
        {
            compressed = zstd::bulk::compress(rgba, RASTER_ZSTD_LEVEL).map_err(|error| {
                CodecError::malformed("zstd compression failed").with_source(error)
            })?;
            compressed.as_slice()
        }
        #[cfg(any(not(feature = "native"), target_arch = "wasm32"))]
        {
            return Err(CodecError::unsupported(
                "zstd raster compression requires the native feature",
            ));
        }
    } else {
        rgba
    };
    let prefix =
        raster_full_frame_prefix_inner(epoch, frame_id, width, height, pixels.len(), compress)?;
    let mut body = Vec::with_capacity(prefix.len() + pixels.len());
    body.extend_from_slice(&prefix);
    body.extend_from_slice(pixels);
    Ok(body)
}

/// Builds a raster delta body that patches frame `base_frame_id` with `operations`.
///
/// # Errors
///
/// Returns an invalid-argument [`CodecError`] when the operation limit is outside 1 through 16, a
/// frame or base ID is zero, a source dimension is zero, the operation count exceeds the limit, an
/// overwrite's pixels do not match its rectangle, or the body would exceed
/// [`HARD_MAX_RECORD_BODY`]; an unsupported [`CodecError`] when compression is requested without
/// the `native` feature or on wasm32.
pub fn raster_delta_frame_body(
    frame: &RasterDeltaFrame<'_>,
    effective_operation_limit: u32,
    compress: bool,
) -> Result<Vec<u8>, CodecError> {
    let &RasterDeltaFrame {
        epoch,
        frame_id,
        base_frame_id,
        pts_us,
        duration_us,
        width: source_width,
        height: source_height,
        operations,
    } = frame;
    validate_delta_operation_limit(effective_operation_limit, Kind::InvalidArgument)?;
    if frame_id == 0 || base_frame_id == 0 {
        return Err(CodecError::invalid_argument(
            "delta frame and base IDs must be nonzero",
        ));
    }
    if source_width == 0 || source_height == 0 {
        return Err(CodecError::invalid_argument(
            "delta source dimensions are empty",
        ));
    }
    let count = u32::try_from(operations.len())
        .map_err(|_| CodecError::invalid_argument("too many raster delta operations"))?;
    if count == 0 || count > effective_operation_limit {
        return Err(CodecError::invalid_argument(
            "raster delta operation count is outside the effective limit",
        ));
    }

    let descriptor_bytes = operations
        .len()
        .checked_mul(RASTER_DELTA_OPERATION_SIZE)
        .ok_or_else(|| CodecError::invalid_argument("delta body overflow"))?;
    let payload_start = RASTER_FRAME_PREFIX_SIZE
        .checked_add(descriptor_bytes)
        .ok_or_else(|| CodecError::invalid_argument("delta body overflow"))?;
    let mut descriptors = vec![0_u8; descriptor_bytes];
    let mut payloads: Vec<Cow<'_, [u8]>> = Vec::new();
    let mut payload_length = 0_usize;

    for (index, operation) in operations.iter().enumerate() {
        let descriptor = &mut descriptors
            [index * RASTER_DELTA_OPERATION_SIZE..(index + 1) * RASTER_DELTA_OPERATION_SIZE];
        match operation {
            RasterDeltaOperation::Overwrite {
                x,
                y,
                width,
                height,
                rgba,
            } => {
                validate_raster_rectangle(
                    *x,
                    *y,
                    *width,
                    *height,
                    source_width,
                    source_height,
                    Kind::InvalidArgument,
                )?;
                let expected = rgba8_pixel_len(*width, *height).map_err(|error| {
                    CodecError::invalid_argument("invalid raster dimensions").with_source(error)
                })? as usize;
                if rgba.len() != expected {
                    return Err(CodecError::invalid_argument(
                        "delta overwrite RGBA length does not match its rectangle",
                    ));
                }
                let payload = if compress {
                    #[cfg(all(feature = "native", not(target_arch = "wasm32")))]
                    {
                        Cow::Owned(zstd::bulk::compress(rgba, RASTER_ZSTD_LEVEL).map_err(
                            |error| {
                                CodecError::malformed("zstd compression failed").with_source(error)
                            },
                        )?)
                    }
                    #[cfg(any(not(feature = "native"), target_arch = "wasm32"))]
                    {
                        return Err(CodecError::unsupported(
                            "zstd raster compression requires the native feature",
                        ));
                    }
                } else {
                    Cow::Borrowed(*rgba)
                };
                let encoded_length = u32::try_from(payload.len())
                    .map_err(|_| CodecError::invalid_argument("delta overwrite is too large"))?;
                payload_length = payload_length
                    .checked_add(payload.len())
                    .ok_or_else(|| CodecError::invalid_argument("delta body overflow"))?;
                put_u32(descriptor, 0, RASTER_DELTA_OVERWRITE);
                put_u32(descriptor, 4, *x);
                put_u32(descriptor, 8, *y);
                put_u32(descriptor, 12, *width);
                put_u32(descriptor, 16, *height);
                put_u32(descriptor, 20, 0);
                put_u32(descriptor, 24, 0);
                put_u32(descriptor, 28, encoded_length);
                payloads.push(payload);
            }
            RasterDeltaOperation::Copy {
                destination_x,
                destination_y,
                width,
                height,
                source_x,
                source_y,
            } => {
                validate_raster_rectangle(
                    *destination_x,
                    *destination_y,
                    *width,
                    *height,
                    source_width,
                    source_height,
                    Kind::InvalidArgument,
                )?;
                validate_raster_rectangle(
                    *source_x,
                    *source_y,
                    *width,
                    *height,
                    source_width,
                    source_height,
                    Kind::InvalidArgument,
                )?;
                put_u32(descriptor, 0, RASTER_DELTA_COPY);
                put_u32(descriptor, 4, *destination_x);
                put_u32(descriptor, 8, *destination_y);
                put_u32(descriptor, 12, *width);
                put_u32(descriptor, 16, *height);
                put_u32(descriptor, 20, *source_x);
                put_u32(descriptor, 24, *source_y);
                put_u32(descriptor, 28, 0);
            }
        }
    }

    let body_length = payload_start
        .checked_add(payload_length)
        .filter(|length| *length <= HARD_MAX_RECORD_BODY as usize)
        .ok_or_else(|| CodecError::invalid_argument("delta frame too large"))?;
    let mut body = Vec::with_capacity(body_length);
    body.resize(RASTER_FRAME_PREFIX_SIZE, 0);
    put_u32(&mut body, 0, epoch);
    put_u32(
        &mut body,
        4,
        RASTER_FRAME_DELTA | if compress { RASTER_FRAME_ZSTD } else { 0 },
    );
    put_u64(&mut body, 8, frame_id);
    put_u64(&mut body, 16, base_frame_id);
    put_i64(&mut body, 24, pts_us);
    put_u64(&mut body, 32, duration_us);
    put_u32(&mut body, 40, count);
    put_u32(&mut body, 44, 0);
    body.extend_from_slice(&descriptors);
    for payload in payloads {
        body.extend_from_slice(&payload);
    }
    debug_assert_eq!(body.len(), body_length);
    Ok(body)
}

/// Borrows the fields and access unit out of a received video packet body.
///
/// # Errors
///
/// Returns a malformed-input [`CodecError`] when `body` is shorter than its 48-byte prefix, the
/// reserved field or flags are invalid, or the side data does not fit in the body.
pub fn parse_video_packet(body: &[u8]) -> Result<ParsedVideoPacket<'_>, CodecError> {
    if body.len() < VIDEO_PACKET_PREFIX_SIZE {
        return Err(invalid("video packet is shorter than its 48-byte prefix"));
    }
    let side_data_length = read_u32(body, 40)? as usize;
    if read_u32(body, 44)? != 0 {
        return Err(invalid("video packet reserved field is nonzero"));
    }
    let data_offset = VIDEO_PACKET_PREFIX_SIZE
        .checked_add(side_data_length)
        .filter(|offset| *offset <= body.len())
        .ok_or_else(|| invalid("video packet side data exceeds its body"))?;
    let flags = read_u32(body, 4)?;
    if flags != VIDEO_PACKET_KEY && flags != VIDEO_PACKET_DELTA {
        return Err(invalid("video packet has invalid flags"));
    }
    Ok(ParsedVideoPacket {
        epoch: read_u32(body, 0)?,
        flags,
        packet_id: read_u64(body, 8)?,
        pts_us: read_i64(body, 16)?,
        dts_us: read_i64(body, 24)?,
        duration_us: read_u64(body, 32)?,
        side_data: &body[VIDEO_PACKET_PREFIX_SIZE..data_offset],
        data: &body[data_offset..],
    })
}

/// Borrows the fields and access unit out of a received audio packet body.
///
/// # Errors
///
/// Returns a malformed-input [`CodecError`] when `body` is shorter than its 48-byte prefix, carries
/// no access unit, or has nonzero reserved flags.
pub fn parse_audio_packet(body: &[u8]) -> Result<ParsedAudioPacket<'_>, CodecError> {
    if body.len() < AUDIO_PACKET_PREFIX_SIZE {
        return Err(invalid("audio packet is shorter than its 48-byte prefix"));
    }
    if body.len() == AUDIO_PACKET_PREFIX_SIZE {
        return Err(invalid("audio packet has an empty access unit"));
    }
    if read_u32(body, 4)? != 0 {
        return Err(invalid("audio packet reserved flags are nonzero"));
    }
    Ok(ParsedAudioPacket {
        epoch: read_u32(body, 0)?,
        packet_id: read_u64(body, 8)?,
        pts_us: read_i64(body, 16)?,
        dts_us: read_i64(body, 24)?,
        duration_us: read_u64(body, 32)?,
        trim_start_samples: read_u32(body, 40)?,
        trim_end_samples: read_u32(body, 44)?,
        data: &body[AUDIO_PACKET_PREFIX_SIZE..],
    })
}

/// Borrows a received full-frame raster body, checking its single rectangle.
///
/// # Errors
///
/// Returns a malformed-input [`CodecError`] when `body` is shorter than one rectangle, uses an
/// unsupported layout or flags, or its pixel data does not fit the body or match the declared
/// dimensions.
pub fn parse_full_raster_frame(body: &[u8]) -> Result<ParsedRasterFrame<'_>, CodecError> {
    let header_length = RASTER_FRAME_PREFIX_SIZE + RASTER_RECT_SIZE;
    if body.len() < header_length {
        return Err(invalid(
            "raster frame is shorter than one full-frame rectangle",
        ));
    }
    let flags = read_u32(body, 4)?;
    if flags & RASTER_FRAME_FULL == 0
        || flags & !(RASTER_FRAME_FULL | RASTER_FRAME_ZSTD) != 0
        || read_u64(body, 16)? != 0
        || read_u32(body, 40)? != 1
        || read_u32(body, 44)? != 0
    {
        return Err(invalid("unsupported raster frame layout"));
    }
    let x = read_u32(body, 48)?;
    let y = read_u32(body, 52)?;
    let width = read_u32(body, 56)?;
    let height = read_u32(body, 60)?;
    let data_offset = read_u32(body, 64)? as usize;
    let data_length = read_u32(body, 68)? as usize;
    if x != 0 || y != 0 || data_offset != 0 || width == 0 || height == 0 {
        return Err(invalid(
            "raster frame is not a complete origin-aligned frame",
        ));
    }
    let expected_length =
        rgba8_pixel_len(width, height).map_err(|_| invalid("raster dimensions overflow"))? as usize;
    let compressed = flags & RASTER_FRAME_ZSTD != 0;
    if (!compressed && data_length != expected_length)
        || header_length.checked_add(data_length) != Some(body.len())
    {
        return Err(invalid("raster RGBA byte length does not match dimensions"));
    }
    Ok(ParsedRasterFrame {
        epoch: read_u32(body, 0)?,
        frame_id: read_u64(body, 8)?,
        pts_us: read_i64(body, 24)?,
        duration_us: read_u64(body, 32)?,
        width,
        height,
        compressed,
        pixels: &body[header_length..],
    })
}

/// Borrows a received raster delta body that patches a frame of `source` size.
///
/// # Errors
///
/// Returns a malformed-input [`CodecError`] when the operation limit is outside 1 through 16, a
/// source dimension is zero, the header is short, has unsupported flags, a zero base frame ID, or
/// nonzero reserved fields, the operation count exceeds the limit, an operation has an unknown
/// kind, an empty or out-of-bounds rectangle, or a payload that does not fit, or the body has
/// trailing bytes.
pub fn parse_delta_raster_frame(
    body: &[u8],
    source: RasterSize,
    effective_operation_limit: u32,
) -> Result<ParsedRasterDeltaFrame<'_>, CodecError> {
    struct ValidatedOperation {
        kind: u32,
        destination_x: u32,
        destination_y: u32,
        width: u32,
        height: u32,
        source_x: u32,
        source_y: u32,
        payload_start: usize,
        payload_end: usize,
        expected_length: usize,
    }
    let RasterSize {
        width: source_width,
        height: source_height,
    } = source;

    validate_delta_operation_limit(effective_operation_limit, Kind::Malformed)?;
    if source_width == 0 || source_height == 0 {
        return Err(invalid("delta source dimensions are empty"));
    }
    if body.len() < RASTER_FRAME_PREFIX_SIZE {
        return Err(invalid("raster delta is shorter than its frame header"));
    }
    let flags = read_u32(body, 4)?;
    if flags != RASTER_FRAME_DELTA && flags != RASTER_FRAME_DELTA | RASTER_FRAME_ZSTD {
        return Err(invalid("unsupported raster delta flags"));
    }
    let base_frame_id = read_u64(body, 16)?;
    if base_frame_id == 0 {
        return Err(invalid("raster delta has a zero base frame ID"));
    }
    if read_u32(body, 44)? != 0 {
        return Err(invalid("raster delta reserved header field is nonzero"));
    }
    let count = read_u32(body, 40)?;
    if count == 0 || count > effective_operation_limit {
        return Err(invalid(
            "raster delta operation count is outside the effective limit",
        ));
    }
    let descriptor_bytes = usize::try_from(count)
        .ok()
        .and_then(|count| count.checked_mul(RASTER_DELTA_OPERATION_SIZE))
        .ok_or_else(|| invalid("raster delta operation array overflows"))?;
    let payload_start = RASTER_FRAME_PREFIX_SIZE
        .checked_add(descriptor_bytes)
        .filter(|offset| *offset <= body.len())
        .ok_or_else(|| invalid("raster delta operation array exceeds its body"))?;
    let compressed = flags & RASTER_FRAME_ZSTD != 0;

    let mut validated = Vec::with_capacity(count as usize);
    let mut payload_offset = payload_start;
    for index in 0..count as usize {
        let offset = RASTER_FRAME_PREFIX_SIZE + index * RASTER_DELTA_OPERATION_SIZE;
        let kind = read_u32(body, offset)?;
        let destination_x = read_u32(body, offset + 4)?;
        let destination_y = read_u32(body, offset + 8)?;
        let width = read_u32(body, offset + 12)?;
        let height = read_u32(body, offset + 16)?;
        let source_x = read_u32(body, offset + 20)?;
        let source_y = read_u32(body, offset + 24)?;
        let payload_length = read_u32(body, offset + 28)? as usize;
        validate_raster_rectangle(
            destination_x,
            destination_y,
            width,
            height,
            source_width,
            source_height,
            Kind::Malformed,
        )?;
        let expected_length = rgba8_pixel_len(width, height)
            .map_err(|_| invalid("raster delta rectangle dimensions overflow"))?
            as usize;
        match kind {
            RASTER_DELTA_OVERWRITE => {
                if source_x != 0 || source_y != 0 {
                    return Err(invalid(
                        "raster delta overwrite reserved source fields are nonzero",
                    ));
                }
                if !compressed && payload_length != expected_length {
                    return Err(invalid(
                        "raster delta overwrite length does not match its rectangle",
                    ));
                }
            }
            RASTER_DELTA_COPY => {
                if payload_length != 0 {
                    return Err(invalid("raster delta copy has a nonzero payload length"));
                }
                validate_raster_rectangle(
                    source_x,
                    source_y,
                    width,
                    height,
                    source_width,
                    source_height,
                    Kind::Malformed,
                )?;
            }
            _ => return Err(invalid("raster delta has an unknown operation kind")),
        }
        let payload_end = payload_offset
            .checked_add(payload_length)
            .filter(|end| *end <= body.len())
            .ok_or_else(|| invalid("raster delta overwrite payload exceeds its body"))?;
        validated.push(ValidatedOperation {
            kind,
            destination_x,
            destination_y,
            width,
            height,
            source_x,
            source_y,
            payload_start: payload_offset,
            payload_end,
            expected_length,
        });
        payload_offset = payload_end;
    }
    if payload_offset != body.len() {
        return Err(invalid("raster delta has trailing bytes"));
    }

    let mut operations = Vec::with_capacity(validated.len());
    for operation in validated {
        if operation.kind == RASTER_DELTA_OVERWRITE {
            let encoded = &body[operation.payload_start..operation.payload_end];
            let rgba = if compressed {
                validate_zstd_frame_magic(encoded)?;
                Cow::Owned(decode_zstd_pixels(encoded, operation.expected_length)?)
            } else {
                Cow::Borrowed(encoded)
            };
            operations.push(ParsedRasterDeltaOperation::Overwrite {
                x: operation.destination_x,
                y: operation.destination_y,
                width: operation.width,
                height: operation.height,
                rgba,
            });
        } else {
            operations.push(ParsedRasterDeltaOperation::Copy {
                destination_x: operation.destination_x,
                destination_y: operation.destination_y,
                width: operation.width,
                height: operation.height,
                source_x: operation.source_x,
                source_y: operation.source_y,
            });
        }
    }
    Ok(ParsedRasterDeltaFrame {
        epoch: read_u32(body, 0)?,
        frame_id: read_u64(body, 8)?,
        base_frame_id,
        pts_us: read_i64(body, 24)?,
        duration_us: read_u64(body, 32)?,
        compressed,
        operations,
    })
}

/// Returns a full frame's RGBA pixels, decompressing them when the frame is zstd-compressed.
///
/// # Errors
///
/// Returns a malformed-input [`CodecError`] when the dimensions overflow, the zstd stream is
/// invalid, uses skippable frames, a dictionary, or trailing frames, or decompresses to the wrong
/// size. Returns an unsupported [`CodecError`] for compressed frames without the `native` feature
/// outside wasm32.
pub fn decode_raster_pixels(frame: ParsedRasterFrame<'_>) -> Result<Vec<u8>, CodecError> {
    let expected = rgba8_pixel_len(frame.width, frame.height)
        .map_err(|_| invalid("raster dimensions overflow"))? as usize;
    if !frame.compressed {
        return Ok(frame.pixels.to_vec());
    }
    validate_zstd_frame_magic(frame.pixels)?;
    decode_zstd_pixels(frame.pixels, expected)
}

/// Magic number, little-endian, that opens every standard zstd frame (RFC 8878 §3.1.1).
///
/// Raster payloads must be exactly one standard frame. Legacy frames (zstd 0.1 through 0.7) use
/// other magic numbers and decoder paths no Vivid sender needs, so they are refused before any
/// decoder runs, even where the zstd library could still read them.
const ZSTD_FRAME_MAGIC: u32 = 0xfd2f_b528;

/// Skippable frames (RFC 8878 §3.1.2) use magic numbers `0x184D2A50` through `0x184D2A5F`.
const ZSTD_SKIPPABLE_MAGIC: u32 = 0x184d_2a50;
const ZSTD_SKIPPABLE_MAGIC_MASK: u32 = 0xffff_fff0;

fn validate_zstd_frame_magic(bytes: &[u8]) -> Result<(), CodecError> {
    let magic = (bytes.len() >= 4).then(|| u32::from_le_bytes(crate::array_at(bytes, 0)));
    match magic {
        Some(ZSTD_FRAME_MAGIC) => Ok(()),
        Some(magic) if magic & ZSTD_SKIPPABLE_MAGIC_MASK == ZSTD_SKIPPABLE_MAGIC => {
            Err(invalid("zstd skippable frames are forbidden"))
        }
        _ => Err(invalid(
            "zstd raster is not a standard zstd frame; legacy frames are forbidden",
        )),
    }
}

fn validate_delta_operation_limit(limit: u32, kind: Kind) -> Result<(), CodecError> {
    if !(1..=RASTER_DELTA_OPERATION_LIMIT).contains(&limit) {
        return Err(CodecError::new(
            kind,
            "raster delta operation limit is outside 1 through 16",
        ));
    }
    Ok(())
}

fn validate_raster_rectangle(
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    source_width: u32,
    source_height: u32,
    kind: Kind,
) -> Result<(), CodecError> {
    if width == 0
        || height == 0
        || x.checked_add(width)
            .is_none_or(|right| right > source_width)
        || y.checked_add(height)
            .is_none_or(|bottom| bottom > source_height)
    {
        return Err(CodecError::new(
            kind,
            "raster delta rectangle is empty, overflows, or exceeds the source",
        ));
    }
    Ok(())
}

#[cfg(all(feature = "native", not(target_arch = "wasm32")))]
fn decode_zstd_pixels(pixels: &[u8], expected: usize) -> Result<Vec<u8>, CodecError> {
    if zstd_safe::get_dict_id_from_frame(pixels).is_some()
        || zstd_safe::find_frame_compressed_size(pixels).ok() != Some(pixels.len())
    {
        return Err(invalid(
            "zstd dictionaries and trailing frames are forbidden",
        ));
    }
    let cursor = Cursor::new(pixels);
    let mut decoder = zstd::stream::read::Decoder::new(cursor)
        .map_err(|_| invalid("invalid zstd frame"))?
        .single_frame();
    let mut output = Vec::with_capacity(expected);
    decoder
        .by_ref()
        .take((expected + 1) as u64)
        .read_to_end(&mut output)
        .map_err(|error| invalid("invalid zstd frame").with_source(error))?;
    if output.len() != expected {
        return Err(invalid(
            "zstd raster has wrong output size or trailing data",
        ));
    }
    Ok(output)
}

#[cfg(target_arch = "wasm32")]
fn decode_zstd_pixels(pixels: &[u8], expected: usize) -> Result<Vec<u8>, CodecError> {
    let mut decoder = ruzstd::decoding::StreamingDecoder::new(pixels)
        .map_err(|_| invalid("invalid zstd frame"))?;
    let mut output = Vec::with_capacity(expected);
    decoder
        .by_ref()
        .take((expected + 1) as u64)
        .read_to_end(&mut output)
        .map_err(|error| invalid("invalid zstd frame").with_source(error))?;
    if output.len() != expected
        || !decoder.decoder.is_finished()
        || u64::try_from(pixels.len()) != Ok(decoder.decoder.bytes_read_from_source())
    {
        return Err(invalid(
            "zstd raster has wrong output size or trailing data",
        ));
    }
    Ok(output)
}

#[cfg(all(not(feature = "native"), not(target_arch = "wasm32")))]
fn decode_zstd_pixels(_pixels: &[u8], _expected: usize) -> Result<Vec<u8>, CodecError> {
    Err(CodecError::unsupported(
        "zstd raster decoding requires either native support or wasm32",
    ))
}

/// Returns whether `packetization` is the canonical video packetization for `codec`.
pub fn is_portable_packetization(codec: &str, packetization: &str) -> bool {
    matches!(
        (codec, packetization),
        ("h264", "h264-annexb-au-v1")
            | ("hevc", "hevc-annexb-au-v1")
            | ("vp9", "vp9-frame-v1")
            | ("av1", "av1-low-overhead-tu-v1")
    )
}

/// Packetization for Opus: one packet per record.
pub const AUDIO_PACKETIZATION_OPUS: &str = "opus-packet-v1";
/// Packetization for Vorbis: one packet per record.
pub const AUDIO_PACKETIZATION_VORBIS: &str = "vorbis-packet-v1";
/// Packetization for FLAC: one frame per record.
pub const AUDIO_PACKETIZATION_FLAC: &str = "flac-frame-v1";

/// Returns whether `packetization` is the canonical audio packetization for `codec`.
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

/// What an audio track declares about its codec, as [`validate_audio_initialization`] checks it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioInitialization<'a> {
    /// The portable codec name, such as `"opus"`.
    pub codec: &'a str,
    /// The packetization name, such as `"opus-packet-v1"`.
    pub packetization: &'a str,
    /// The codec's initialization data.
    ///
    /// That is an Opus head, Xiph-laced Vorbis headers, FLAC `STREAMINFO`, or an AAC
    /// `AudioSpecificConfig`.
    pub extradata: &'a [u8],
    /// The declared sample rate in hertz.
    pub sample_rate: u32,
    /// The declared channel count.
    pub channels: u16,
}

/// Largest audio initialization accepted, in bytes.
///
/// The bound keeps a track declaration from making the presenter hold or parse an arbitrarily large
/// blob before any decoder runs; 64 KiB leaves headroom for the portable codecs' headers.
const MAX_AUDIO_INITIALIZATION_BYTES: usize = 65_536;

/// Validates audio initialization data for a portable codec and packetization.
///
/// # Errors
///
/// Returns a malformed-input [`CodecError`] when the codec and packetization are not a portable
/// pair, `extradata` exceeds 64 KiB, or it fails the codec's own check: [`validate_opus_head`],
/// [`validate_vorbis_headers`], [`validate_flac_streaminfo`], or
/// [`validate_aac_audio_specific_config`].
pub fn validate_audio_initialization(
    initialization: AudioInitialization<'_>,
) -> Result<(), CodecError> {
    let AudioInitialization {
        codec,
        packetization,
        extradata,
        sample_rate,
        channels,
    } = initialization;
    if !valid_audio_packetization(codec, packetization)
        || extradata.len() > MAX_AUDIO_INITIALIZATION_BYTES
    {
        return Err(invalid("invalid portable audio initialization"));
    }
    match codec {
        "opus" => validate_opus_head(extradata, sample_rate, channels),
        "vorbis" => validate_vorbis_headers(extradata, sample_rate, channels),
        "flac" => validate_flac_streaminfo(extradata, sample_rate, channels),
        "aac" => validate_aac_audio_specific_config(extradata, sample_rate, channels),
        _ => Ok(()),
    }
}

const AAC_SAMPLE_RATES: [u32; 13] = [
    96_000, 88_200, 64_000, 48_000, 44_100, 32_000, 24_000, 22_050, 16_000, 12_000, 11_025, 8_000,
    7_350,
];

/// Validate ASC rate and channel metadata, not the codec-specific tools or full decoder support.
/// Configuration zero requires a complete General Audio program configuration element (PCE).
///
/// # Errors
///
/// Returns a malformed-input [`CodecError`] when `config` is truncated, has a null object type or
/// invalid sampling frequency, carries an invalid or unsupported channel configuration, or
/// disagrees with `sample_rate` or `channels`.
pub fn validate_aac_audio_specific_config(
    config: &[u8],
    sample_rate: u32,
    channels: u16,
) -> Result<(), CodecError> {
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
    // MPEG-4 Audio channelConfiguration, also listed in FFmpeg mpeg4audio.c.
    // These are layout identifiers, not channel counts.
    let configured = match channel_configuration {
        0 => reader
            .program_configuration_channels(audio_object_type, frequency)
            .ok_or_else(|| {
                invalid("AudioSpecificConfig PCE is invalid, truncated, or unsupported")
            })?,
        1..=6 => channel_configuration,
        7 | 12 | 14 => 8,
        11 => 7,
        13 => 24,
        _ => {
            return Err(invalid(
                "AudioSpecificConfig channel configuration is reserved",
            ));
        }
    };
    if configured != declared {
        return Err(invalid(
            "AudioSpecificConfig channel configuration does not match the declared channels",
        ));
    }
    Ok(())
}

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

    // Bounded parsing of the GA header and PCE (ISO/IEC 14496-3 tables 4.1 and 4.2).
    // Every loop count comes from a field of at most eight bits; no allocation is needed.
    fn program_configuration_channels(
        &mut self,
        mut object_type: u32,
        frequency: u32,
    ) -> Option<u32> {
        if matches!(object_type, 5 | 29) {
            self.read_sampling_frequency()?; // explicit SBR extension rate
            object_type = self.read_audio_object_type()?;
            if object_type == 22 {
                self.read_bits(4)?; // extensionChannelConfiguration
            }
        }
        if !matches!(object_type, 1..=4 | 6 | 7 | 17 | 19..=23) {
            return None;
        }
        self.read_bits(1)?; // frameLengthFlag
        if self.read_bits(1)? != 0 {
            self.read_bits(14)?; // coreCoderDelay
        }
        self.read_bits(1)?; // extensionFlag; tools after the PCE are decoder-specific
        // The PCE precedes layerNr and the remaining object-specific GA fields.
        self.read_bits(4)?; // element_instance_tag
        self.read_bits(2)?; // object_type
        let index = self.read_bits(4)?;
        if AAC_SAMPLE_RATES.get(index as usize).copied() != Some(frequency) {
            return None;
        }
        let front = self.read_bits(4)?;
        let side = self.read_bits(4)?;
        let back = self.read_bits(4)?;
        let lfe = self.read_bits(2)?;
        let associated = self.read_bits(3)?;
        let coupling = self.read_bits(4)?;
        for bits in [4, 4, 3] {
            if self.read_bits(1)? != 0 {
                self.read_bits(bits)?;
            }
        }
        let mut channels = lfe;
        for count in [front, side, back] {
            for _ in 0..count {
                channels = channels.checked_add(1 + self.read_bits(1)?)?;
                self.read_bits(4)?; // element tag
            }
        }
        for count in [lfe, associated] {
            for _ in 0..count {
                self.read_bits(4)?;
            }
        }
        for _ in 0..coupling {
            self.read_bits(5)?;
        }
        let padding = (8 - self.position % 8) % 8;
        self.read_bits(u32::try_from(padding).expect("byte-alignment padding is below 8"))?;
        let comment_bytes = self.read_bits(8)?;
        for _ in 0..comment_bytes {
            self.read_bits(8)?;
        }
        (channels != 0).then_some(channels)
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
            13 | 14 => None,
            index => AAC_SAMPLE_RATES.get(index as usize).copied(),
        }
    }
}

/// Validates an `OpusHead` against the declared sample rate and channel count.
///
/// # Errors
///
/// Returns a malformed-input [`CodecError`] when `sample_rate` is not 48 kHz, the header is short
/// or lacks its signature, the version or channel count is unsupported, or the channel mapping is
/// invalid for its family.
pub fn validate_opus_head(head: &[u8], sample_rate: u32, channels: u16) -> Result<(), CodecError> {
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

/// Validates Xiph-laced Vorbis headers against the declared sample rate and channel count.
///
/// # Errors
///
/// Returns a malformed-input [`CodecError`] when the headers are not three-header Xiph lacing, are
/// truncated or overflow, have invalid signatures, or disagree with `sample_rate` or `channels`.
pub fn validate_vorbis_headers(
    private: &[u8],
    sample_rate: u32,
    channels: u16,
) -> Result<(), CodecError> {
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
        || u32::from_le_bytes(crate::array_at(identification, 12)) != sample_rate
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

fn xiph_laced_length(bytes: &[u8], cursor: &mut usize) -> Result<usize, CodecError> {
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

/// Validates a raw 34-byte FLAC `STREAMINFO` against the declared sample rate and channels.
///
/// # Errors
///
/// Returns a malformed-input [`CodecError`] when `streaminfo` is not 34 bytes, the block sizes are
/// invalid, the sample rate is zero or differs from `sample_rate`, the channel count differs from
/// `channels`, or the bit depth is outside 4 through 32.
pub fn validate_flac_streaminfo(
    streaminfo: &[u8],
    sample_rate: u32,
    channels: u16,
) -> Result<(), CodecError> {
    if streaminfo.len() != 34 {
        return Err(invalid(
            "FLAC initialization is not a raw 34-byte STREAMINFO",
        ));
    }
    let minimum_block = u16::from_be_bytes(crate::array_at(streaminfo, 0));
    let maximum_block = u16::from_be_bytes(crate::array_at(streaminfo, 2));
    let packed = u64::from_be_bytes(crate::array_at(streaminfo, 10));
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

/// Checks that `data` is a plausible access unit for a portable codec and packetization.
///
/// # Errors
///
/// Returns a malformed-input [`CodecError`] when `data` is empty, the pair is not portable, an
/// H.264 or HEVC unit is not Annex B, or an AV1 unit sets the OBU forbidden bit.
pub fn validate_portable_packetization(
    codec: &str,
    packetization: &str,
    data: &[u8],
) -> Result<(), CodecError> {
    if data.is_empty() {
        return Err(invalid("portable access unit is empty"));
    }
    match (codec, packetization) {
        ("h264", "h264-annexb-au-v1") | ("hevc", "hevc-annexb-au-v1") => {
            if !(data.starts_with(&[0, 0, 1]) || data.starts_with(&[0, 0, 0, 1])) {
                return Err(invalid("H.264/HEVC access unit is not Annex B"));
            }
        }
        ("vp9", "vp9-frame-v1") => {}
        ("av1", "av1-low-overhead-tu-v1") => {
            if data[0] & 0x80 != 0 {
                return Err(invalid("AV1 OBU forbidden bit is set"));
            }
        }
        _ => return Err(invalid("unsupported portable codec/packetization pair")),
    }
    Ok(())
}

/// Determine random-access status from the portable codec syntax rather than container metadata.
///
/// # Errors
///
/// Returns a malformed-input [`CodecError`] when `data` is empty or malformed for `codec`, or
/// `codec` is not a portable codec.
pub fn access_unit_is_key(codec: &str, data: &[u8]) -> Result<bool, CodecError> {
    if data.is_empty() {
        return Err(invalid("portable access unit is empty"));
    }
    match codec {
        "h264" => Ok(annex_b_nal_headers(data)?.any(|header| header & 0x1f == 5)),
        "hevc" => {
            Ok(annex_b_nal_headers(data)?.any(|header| matches!((header >> 1) & 0x3f, 16..=21)))
        }
        "vp9" => {
            let mut bits = BitReader::new(data);
            if bits.read(2)? != 2 {
                return Err(invalid("VP9 frame marker is invalid"));
            }
            let profile = bits.read(1)? | (bits.read(1)? << 1);
            if profile == 3 && bits.read(1)? != 0 {
                return Err(invalid("VP9 reserved profile bit is set"));
            }
            if bits.read(1)? != 0 {
                return Ok(false);
            }
            Ok(bits.read(1)? == 0)
        }
        "av1" => av1_frame_is_key(data),
        _ => Err(invalid("unsupported portable codec")),
    }
}

fn annex_b_nal_headers(data: &[u8]) -> Result<impl Iterator<Item = u8> + '_, CodecError> {
    if !(data.starts_with(&[0, 0, 1]) || data.starts_with(&[0, 0, 0, 1])) {
        return Err(invalid("access unit is not Annex B"));
    }
    Ok((0..data.len()).filter_map(move |index| {
        let header = if data.get(index..index + 3) == Some(&[0, 0, 1]) {
            index + 3
        } else if data.get(index..index + 4) == Some(&[0, 0, 0, 1]) {
            index + 4
        } else {
            return None;
        };
        data.get(header).copied()
    }))
}

fn av1_frame_is_key(data: &[u8]) -> Result<bool, CodecError> {
    let mut offset = 0;
    while offset < data.len() {
        let header = *data
            .get(offset)
            .ok_or_else(|| invalid("truncated AV1 OBU"))?;
        offset += 1;
        if header & 0x81 != 0 || header & 0x02 == 0 {
            return Err(invalid("invalid AV1 low-overhead OBU header"));
        }
        let obu_type = (header >> 3) & 0x0f;
        if header & 0x04 != 0 {
            offset = offset
                .checked_add(1)
                .filter(|value| *value <= data.len())
                .ok_or_else(|| invalid("truncated AV1 OBU extension"))?;
        }
        let (size, length_bytes) = read_leb128(&data[offset..])?;
        offset += length_bytes;
        let end = offset
            .checked_add(size)
            .filter(|value| *value <= data.len())
            .ok_or_else(|| invalid("AV1 OBU exceeds access unit"))?;
        if matches!(obu_type, 3 | 6) {
            let mut bits = BitReader::new(&data[offset..end]);
            let show_existing_frame = bits.read(1)?;
            if show_existing_frame != 0 {
                return Ok(false);
            }
            return Ok(bits.read(2)? == 0);
        }
        offset = end;
    }
    Err(invalid("AV1 access unit has no frame header"))
}

fn read_leb128(data: &[u8]) -> Result<(usize, usize), CodecError> {
    let mut value = 0_u64;
    for (index, byte) in data.iter().copied().take(8).enumerate() {
        value |= u64::from(byte & 0x7f) << (index * 7);
        if byte & 0x80 == 0 {
            let value =
                usize::try_from(value).map_err(|_| invalid("AV1 OBU length does not fit usize"))?;
            return Ok((value, index + 1));
        }
    }
    Err(invalid("invalid AV1 OBU length"))
}

struct BitReader<'a> {
    data: &'a [u8],
    bit: usize,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, bit: 0 }
    }

    fn read(&mut self, count: usize) -> Result<u8, CodecError> {
        let mut value = 0_u8;
        for _ in 0..count {
            let byte = *self
                .data
                .get(self.bit / 8)
                .ok_or_else(|| invalid("truncated codec header"))?;
            value = (value << 1) | ((byte >> (7 - self.bit % 8)) & 1);
            self.bit += 1;
        }
        Ok(value)
    }
}

/// Reads the `N`-byte field at `offset`, rejecting a body too short to hold it.
fn read_field<const N: usize>(bytes: &[u8], offset: usize) -> Result<[u8; N], CodecError> {
    offset
        .checked_add(N)
        .filter(|end| *end <= bytes.len())
        .map(|_| crate::array_at(bytes, offset))
        .ok_or_else(|| invalid("truncated media field"))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, CodecError> {
    read_field(bytes, offset).map(u32::from_be_bytes)
}

fn read_u64(bytes: &[u8], offset: usize) -> Result<u64, CodecError> {
    read_field(bytes, offset).map(u64::from_be_bytes)
}

fn read_i64(bytes: &[u8], offset: usize) -> Result<i64, CodecError> {
    read_field(bytes, offset).map(i64::from_be_bytes)
}

fn invalid(message: &'static str) -> CodecError {
    CodecError::malformed(message)
}

fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
}

fn put_u64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_be_bytes());
}

fn put_i64(bytes: &mut [u8], offset: usize, value: i64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_be_bytes());
}

// ---------------------------------------------------------------------------------------------
// H.264 Annex-B normalization and decoder description
//
// `h264-annexb-au-v1` requires Annex-B access units and forbids AVCC length prefixes, but real
// encoders emit both: Windows Media Foundation produces AVCC. Normalization therefore belongs
// beside `access_unit_is_key`, which is what every producer cross-checks the result against.
//
// Pure byte transforms over buffers a caller obtained from an encoder. Nothing here touches
// FFmpeg, which is what makes it exhaustively testable.
// ---------------------------------------------------------------------------------------------

/// Largest decoder configuration accepted from an encoder.
const MAX_H264_EXTRADATA: usize = 4096;

fn annexb_invalid(message: &str) -> CodecError {
    CodecError::malformed(format!("H.264: {message}"))
}

/// The immutable H.264 parameters a Vivid `VideoConfiguration` needs.
///
/// Every field is derived from the opened encoder's extradata, never from CLI input: the track's
/// coded dimensions, profile, and level are immutable once negotiated, so a claim that disagrees
/// with the bitstream is a defect the presenter cannot recover from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct H264DecoderDescription {
    /// H.264 `profile_idc`.
    pub profile: i32,
    /// H.264 `level_idc`.
    pub level: i32,
    /// RFC 6381 codec string, such as `avc1.64001f`.
    pub codec_string: String,
    /// Annex-B parameter sets, for prepending to key access units.
    pub extradata: Vec<u8>,
    /// avcC form, for the track's `decoder_configuration`.
    pub decoder_config: Vec<u8>,
}

/// Derive the decoder description from an encoder's extradata, in either avcC or Annex-B form.
///
/// # Errors
///
/// Returns a malformed-input [`CodecError`] when `extradata` is neither a valid avcC configuration
/// nor Annex B parameter sets with an SPS and a PPS, or the configuration is oversized.
pub fn h264_decoder_description(extradata: &[u8]) -> Result<H264DecoderDescription, CodecError> {
    let avcc = if extradata.first() == Some(&1) {
        if extradata.len() < 7 || extradata.len() > MAX_H264_EXTRADATA {
            return Err(annexb_invalid("avcC decoder configuration is invalid"));
        }
        extradata.to_vec()
    } else {
        avcc_from_annexb(extradata)?
    };
    let profile = i32::from(avcc[1]);
    let level = i32::from(avcc[3]);
    let portable_extradata = annexb_from_avcc(&avcc)?;
    Ok(H264DecoderDescription {
        profile,
        level,
        codec_string: format!("avc1.{:02X}{:02X}{:02X}", avcc[1], avcc[2], avcc[3]),
        extradata: portable_extradata,
        decoder_config: avcc,
    })
}

/// Return `unit` as Annex-B, converting from AVCC length-prefixed form when necessary.
///
/// A buffer that is neither well-formed Annex-B nor well-formed AVCC is rejected rather than
/// passed through: sending a mangled access unit produces a silent green picture at the
/// presenter, while an error here names the encoder that produced it on the very first frame.
///
/// # Errors
///
/// Returns a malformed-input [`CodecError`] when `unit` is empty or is neither Annex B nor
/// well-formed length-prefixed NAL units.
pub fn ensure_annexb(unit: &[u8]) -> Result<Cow<'_, [u8]>, CodecError> {
    if unit.is_empty() {
        return Err(annexb_invalid("access unit is empty"));
    }
    if starts_with_start_code(unit) {
        return Ok(Cow::Borrowed(unit));
    }
    Ok(Cow::Owned(annexb_from_length_prefixed(unit)?))
}

fn starts_with_start_code(unit: &[u8]) -> bool {
    unit.starts_with(&[0, 0, 0, 1]) || unit.starts_with(&[0, 0, 1])
}

/// Rewrite a 4-byte-length-prefixed access unit as Annex-B.
///
/// The buffer must be consumed exactly: a trailing remainder means the guess that this was AVCC
/// was wrong, and continuing would emit a truncated access unit.
fn annexb_from_length_prefixed(unit: &[u8]) -> Result<Vec<u8>, CodecError> {
    let mut annexb = Vec::with_capacity(unit.len());
    let mut cursor = 0_usize;
    while cursor < unit.len() {
        let length_end = cursor
            .checked_add(4)
            .filter(|end| *end <= unit.len())
            .ok_or_else(|| annexb_invalid("access unit is neither Annex-B nor AVCC"))?;
        let length = u32::from_be_bytes(crate::array_at(unit, cursor));
        let length = usize::try_from(length)
            .map_err(|_| annexb_invalid("AVCC access unit length is out of range"))?;
        if length == 0 {
            return Err(annexb_invalid(
                "AVCC access unit contains an empty NAL unit",
            ));
        }
        cursor = length_end;
        let end = cursor
            .checked_add(length)
            .filter(|end| *end <= unit.len())
            .ok_or_else(|| annexb_invalid("AVCC access unit is truncated"))?;
        annexb.extend_from_slice(&[0, 0, 0, 1]);
        annexb.extend_from_slice(&unit[cursor..end]);
        cursor = end;
    }
    if annexb.is_empty() {
        return Err(annexb_invalid("access unit contains no NAL units"));
    }
    Ok(annexb)
}

/// Prepend Annex-B parameter sets to a key access unit.
///
/// Encoders that do not repeat SPS/PPS on every IDR need this: the track's immutable extradata
/// stays empty (matching what real Vivido and vvmux presenters already accept from `vvland`), so
/// the parameter sets have to ride the bitstream instead.
pub fn with_parameter_sets(parameter_sets: &[u8], unit: &[u8]) -> Vec<u8> {
    let mut prefixed = Vec::with_capacity(parameter_sets.len().saturating_add(unit.len()));
    prefixed.extend_from_slice(parameter_sets);
    prefixed.extend_from_slice(unit);
    prefixed
}

/// Convert an avcC decoder configuration into Annex-B parameter sets.
///
/// # Errors
///
/// Returns a malformed-input [`CodecError`] when `avcc` is short, has the wrong version, or lacks a
/// valid SPS or PPS.
pub fn annexb_from_avcc(avcc: &[u8]) -> Result<Vec<u8>, CodecError> {
    if avcc.len() < 7 || avcc[0] != 1 {
        return Err(annexb_invalid("avcC decoder configuration is invalid"));
    }
    let mut cursor = 6;
    let sps_count = usize::from(avcc[5] & 0x1f);
    if sps_count == 0 {
        return Err(annexb_invalid("avcC decoder configuration has no SPS"));
    }
    let mut parameter_sets = Vec::with_capacity(sps_count.saturating_add(1));
    for _ in 0..sps_count {
        let sps = take_avcc_nal(avcc, &mut cursor)?;
        if sps.first().is_none_or(|header| header & 0x1f != 7) {
            return Err(annexb_invalid(
                "avcC decoder configuration has an invalid SPS",
            ));
        }
        parameter_sets.push(sps);
    }
    let pps_count = avcc
        .get(cursor)
        .copied()
        .map(usize::from)
        .ok_or_else(|| annexb_invalid("avcC decoder configuration has no PPS count"))?;
    cursor += 1;
    if pps_count == 0 {
        return Err(annexb_invalid("avcC decoder configuration has no PPS"));
    }
    for _ in 0..pps_count {
        let pps = take_avcc_nal(avcc, &mut cursor)?;
        if pps.first().is_none_or(|header| header & 0x1f != 8) {
            return Err(annexb_invalid(
                "avcC decoder configuration has an invalid PPS",
            ));
        }
        parameter_sets.push(pps);
    }
    let capacity = parameter_sets
        .iter()
        .try_fold(0_usize, |total, parameter_set| {
            total.checked_add(4)?.checked_add(parameter_set.len())
        })
        .ok_or_else(|| annexb_invalid("Annex-B decoder initialization is oversized"))?;
    let mut annexb = Vec::with_capacity(capacity);
    for parameter_set in parameter_sets {
        annexb.extend_from_slice(&[0, 0, 0, 1]);
        annexb.extend_from_slice(parameter_set);
    }
    Ok(annexb)
}

fn take_avcc_nal<'a>(avcc: &'a [u8], cursor: &mut usize) -> Result<&'a [u8], CodecError> {
    let length_end = cursor
        .checked_add(2)
        .filter(|end| *end <= avcc.len())
        .ok_or_else(|| annexb_invalid("avcC parameter-set length is truncated"))?;
    let length = usize::from(u16::from_be_bytes(crate::array_at(avcc, *cursor)));
    *cursor = length_end;
    let end = cursor
        .checked_add(length)
        .filter(|end| *end <= avcc.len())
        .ok_or_else(|| annexb_invalid("avcC parameter set is truncated"))?;
    let nal = &avcc[*cursor..end];
    *cursor = end;
    if nal.is_empty() {
        return Err(annexb_invalid("avcC parameter set is empty"));
    }
    Ok(nal)
}

/// Build an avcC decoder configuration from Annex-B parameter sets.
///
/// # Errors
///
/// Returns a malformed-input [`CodecError`] when `extradata` lacks an SPS or a PPS, or the
/// configuration would be oversized.
pub fn avcc_from_annexb(extradata: &[u8]) -> Result<Vec<u8>, CodecError> {
    let nals = annexb_nals(extradata);
    let sps = nals
        .iter()
        .copied()
        .find(|nal| nal.first().is_some_and(|header| header & 0x1f == 7))
        .filter(|sps| sps.len() >= 4)
        .ok_or_else(|| annexb_invalid("extradata has no SPS"))?;
    let pps = nals
        .iter()
        .copied()
        .find(|nal| nal.first().is_some_and(|header| header & 0x1f == 8))
        .ok_or_else(|| annexb_invalid("extradata has no PPS"))?;
    let sps_length = u16::try_from(sps.len()).map_err(|_| annexb_invalid("SPS is oversized"))?;
    let pps_length = u16::try_from(pps.len()).map_err(|_| annexb_invalid("PPS is oversized"))?;
    let capacity = 11_usize
        .checked_add(sps.len())
        .and_then(|length| length.checked_add(pps.len()))
        .filter(|length| *length <= MAX_H264_EXTRADATA)
        .ok_or_else(|| annexb_invalid("decoder configuration is oversized"))?;
    let mut avcc = Vec::with_capacity(capacity);
    avcc.extend_from_slice(&[1, sps[1], sps[2], sps[3], 0xff, 0xe1]);
    avcc.extend_from_slice(&sps_length.to_be_bytes());
    avcc.extend_from_slice(sps);
    avcc.push(1);
    avcc.extend_from_slice(&pps_length.to_be_bytes());
    avcc.extend_from_slice(pps);
    Ok(avcc)
}

/// Split an Annex-B buffer into NAL units, dropping trailing zero padding from each.
pub fn annexb_nals(mut data: &[u8]) -> Vec<&[u8]> {
    let mut nals = Vec::new();
    while let Some((start, prefix)) = find_start_code(data) {
        data = &data[start + prefix..];
        let end = find_start_code(data).map_or(data.len(), |(index, _)| index);
        let mut nal = &data[..end];
        while nal.last() == Some(&0) {
            nal = &nal[..nal.len() - 1];
        }
        if !nal.is_empty() {
            nals.push(nal);
        }
        data = &data[end..];
    }
    nals
}

fn find_start_code(data: &[u8]) -> Option<(usize, usize)> {
    (0..data.len().saturating_sub(2)).find_map(|index| {
        if data[index..].starts_with(&[0, 0, 0, 1]) {
            Some((index, 4))
        } else if data[index..].starts_with(&[0, 0, 1]) {
            Some((index, 3))
        } else {
            None
        }
    })
}

#[cfg(test)]
mod h264_annexb_tests {
    use super::*;

    /// A minimal but structurally valid SPS: NAL header 0x67, then profile/compat/level.
    fn sps() -> Vec<u8> {
        vec![0x67, 0x64, 0x00, 0x29, 0xac, 0x1b]
    }

    fn pps() -> Vec<u8> {
        vec![0x68, 0xee, 0x3c, 0xb0]
    }

    fn annexb_extradata() -> Vec<u8> {
        let mut data = vec![0, 0, 0, 1];
        data.extend_from_slice(&sps());
        data.extend_from_slice(&[0, 0, 0, 1]);
        data.extend_from_slice(&pps());
        data
    }

    fn avcc_extradata() -> Vec<u8> {
        avcc_from_annexb(&annexb_extradata()).unwrap()
    }

    #[test]
    fn derives_description_from_either_extradata_form() {
        let from_annexb = h264_decoder_description(&annexb_extradata()).unwrap();
        let from_avcc = h264_decoder_description(&avcc_extradata()).unwrap();
        assert_eq!(from_annexb, from_avcc);
        // High profile (0x64 = 100), level 4.1 (0x29 = 41).
        assert_eq!(from_annexb.profile, 100);
        assert_eq!(from_annexb.level, 41);
        assert_eq!(from_annexb.codec_string, "avc1.640029");
    }

    #[test]
    fn avcc_and_annexb_round_trip() {
        let avcc = avcc_extradata();
        let annexb = annexb_from_avcc(&avcc).unwrap();
        assert_eq!(annexb, annexb_extradata());
        assert_eq!(avcc_from_annexb(&annexb).unwrap(), avcc);
    }

    #[test]
    fn accepts_three_byte_start_codes() {
        let mut data = vec![0, 0, 1];
        data.extend_from_slice(&sps());
        data.extend_from_slice(&[0, 0, 1]);
        data.extend_from_slice(&pps());
        let description = h264_decoder_description(&data).unwrap();
        assert_eq!(description.codec_string, "avc1.640029");
    }

    #[test]
    fn strips_trailing_zero_padding_from_nals() {
        let mut data = vec![0, 0, 0, 1];
        data.extend_from_slice(&sps());
        data.extend_from_slice(&[0, 0]);
        data.extend_from_slice(&[0, 0, 0, 1]);
        data.extend_from_slice(&pps());
        data.extend_from_slice(&[0]);
        let nals = annexb_nals(&data);
        assert_eq!(nals.len(), 2);
        assert_eq!(nals[0], sps().as_slice());
        assert_eq!(nals[1], pps().as_slice());
    }

    #[test]
    fn carries_multiple_parameter_sets() {
        // Two SPS then one PPS, hand-built because avcc_from_annexb only emits one of each.
        let mut avcc = vec![1, 0x64, 0x00, 0x29, 0xff, 0xe2];
        for _ in 0..2 {
            avcc.extend_from_slice(&u16::try_from(sps().len()).unwrap().to_be_bytes());
            avcc.extend_from_slice(&sps());
        }
        avcc.push(1);
        avcc.extend_from_slice(&u16::try_from(pps().len()).unwrap().to_be_bytes());
        avcc.extend_from_slice(&pps());
        let annexb = annexb_from_avcc(&avcc).unwrap();
        assert_eq!(annexb_nals(&annexb).len(), 3);
    }

    #[test]
    #[expect(
        clippy::similar_names,
        reason = "SPS/PPS variants are the subject of the test"
    )]
    fn rejects_malformed_avcc() {
        let avcc = avcc_extradata();
        // Zero SPS count.
        let mut no_sps = avcc.clone();
        no_sps[5] = 0xe0;
        annexb_from_avcc(&no_sps).unwrap_err();
        // Zero PPS count: the PPS count byte follows the single SPS.
        let mut no_pps = avcc.clone();
        let pps_count_index = 6 + 2 + sps().len();
        no_pps[pps_count_index] = 0;
        annexb_from_avcc(&no_pps).unwrap_err();
        // Truncated parameter-set body.
        annexb_from_avcc(&avcc[..avcc.len() - 2]).unwrap_err();
        // Wrong configuration version.
        let mut wrong_version = avcc.clone();
        wrong_version[0] = 2;
        annexb_from_avcc(&wrong_version).unwrap_err();
        // Too short to carry a header at all.
        annexb_from_avcc(&[1, 0x64, 0, 0x29, 0xff]).unwrap_err();
    }

    #[test]
    #[expect(
        clippy::similar_names,
        reason = "SPS/PPS variants are the subject of the test"
    )]
    fn rejects_extradata_missing_a_parameter_set() {
        let mut only_sps = vec![0, 0, 0, 1];
        only_sps.extend_from_slice(&sps());
        avcc_from_annexb(&only_sps).unwrap_err();
        let mut only_pps = vec![0, 0, 0, 1];
        only_pps.extend_from_slice(&pps());
        avcc_from_annexb(&only_pps).unwrap_err();
        avcc_from_annexb(&[]).unwrap_err();
    }

    #[test]
    fn rejects_oversized_extradata() {
        let oversized = vec![1_u8; MAX_H264_EXTRADATA + 1];
        h264_decoder_description(&oversized).unwrap_err();
    }

    #[test]
    fn ensure_annexb_passes_annexb_through_without_copying() {
        let unit = annexb_extradata();
        let normalized = ensure_annexb(&unit).unwrap();
        assert!(matches!(normalized, Cow::Borrowed(_)));
        assert_eq!(normalized.as_ref(), unit.as_slice());

        let three_byte = {
            let mut data = vec![0, 0, 1];
            data.extend_from_slice(&sps());
            data
        };
        assert!(matches!(
            ensure_annexb(&three_byte).unwrap(),
            Cow::Borrowed(_)
        ));
    }

    #[test]
    fn ensure_annexb_converts_length_prefixed_units() {
        let mut avcc_unit = Vec::new();
        for nal in [sps(), pps()] {
            avcc_unit.extend_from_slice(&u32::try_from(nal.len()).unwrap().to_be_bytes());
            avcc_unit.extend_from_slice(&nal);
        }
        let normalized = ensure_annexb(&avcc_unit).unwrap();
        assert!(matches!(normalized, Cow::Owned(_)));
        assert_eq!(normalized.as_ref(), annexb_extradata().as_slice());
    }

    #[test]
    fn ensure_annexb_rejects_rather_than_mangles() {
        // Empty.
        ensure_annexb(&[]).unwrap_err();
        // A length prefix that overruns the buffer.
        ensure_annexb(&[0x00, 0x00, 0x10, 0x00, 0x67, 0x64]).unwrap_err();
        // A zero-length NAL unit.
        ensure_annexb(&[0x00, 0x00, 0x00, 0x00]).unwrap_err();
        // A trailing remainder too short to be another length prefix.
        let mut trailing = Vec::new();
        trailing.extend_from_slice(&u32::try_from(sps().len()).unwrap().to_be_bytes());
        trailing.extend_from_slice(&sps());
        trailing.extend_from_slice(&[0x00, 0x02]);
        ensure_annexb(&trailing).unwrap_err();
    }

    #[test]
    fn with_parameter_sets_prefixes_without_reordering() {
        let parameter_sets = annexb_extradata();
        let mut unit = vec![0, 0, 0, 1];
        unit.extend_from_slice(&[0x65, 0x88, 0x84]);
        let prefixed = with_parameter_sets(&parameter_sets, &unit);
        assert!(prefixed.starts_with(&parameter_sets));
        assert!(prefixed.ends_with(&unit));
        assert_eq!(annexb_nals(&prefixed).len(), 3);
    }

    #[test]
    fn a_normalized_key_unit_is_still_recognized_as_key() {
        // The guard every producer relies on: whatever normalization produced must remain
        // classifiable, or a wrong guess becomes a silent green picture instead of an error.
        let mut unit = annexb_extradata();
        unit.extend_from_slice(&[0, 0, 0, 1, 0x65, 0x88, 0x84]);
        assert!(access_unit_is_key("h264", &unit).unwrap());

        let delta = [0, 0, 0, 1, 0x41, 0x9a, 0x00];
        assert!(!access_unit_is_key("h264", &delta).unwrap());
    }
}
