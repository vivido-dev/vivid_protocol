use std::{borrow::Cow, io};

#[cfg(all(feature = "native", not(target_arch = "wasm32")))]
use std::io::Cursor;
#[cfg(any(feature = "native", target_arch = "wasm32"))]
use std::io::Read;

use crate::HARD_MAX_RECORD_BODY;

pub const VIDEO_PACKET_KEY: u32 = 1 << 0;
pub const VIDEO_PACKET_DELTA: u32 = 1 << 1;
pub const RASTER_FRAME_FULL: u32 = 1 << 0;
pub const RASTER_FRAME_ZSTD: u32 = 1 << 1;
pub const RASTER_FRAME_DELTA: u32 = 1 << 2;
pub const RASTER_DELTA_OVERWRITE: u32 = 1;
pub const RASTER_DELTA_COPY: u32 = 2;
pub const RASTER_DELTA_OPERATION_LIMIT: u32 = 16;

const VIDEO_PACKET_PREFIX_SIZE: usize = 48;
const AUDIO_PACKET_PREFIX_SIZE: usize = 48;
const RASTER_FRAME_PREFIX_SIZE: usize = 48;
const RASTER_RECT_SIZE: usize = 24;
const RASTER_DELTA_OPERATION_SIZE: usize = 32;

#[derive(Debug, Clone, Copy)]
pub struct ParsedVideoPacket<'a> {
    pub epoch: u32,
    pub flags: u32,
    pub packet_id: u64,
    pub pts_us: i64,
    pub dts_us: i64,
    pub duration_us: u64,
    pub side_data: &'a [u8],
    pub data: &'a [u8],
}

#[derive(Debug, Clone, Copy)]
pub struct ParsedAudioPacket<'a> {
    pub epoch: u32,
    pub packet_id: u64,
    pub pts_us: i64,
    pub dts_us: i64,
    pub duration_us: u64,
    pub trim_start_samples: u32,
    pub trim_end_samples: u32,
    pub data: &'a [u8],
}

#[derive(Debug, Clone, Copy)]
pub struct ParsedRasterFrame<'a> {
    pub epoch: u32,
    pub frame_id: u64,
    pub pts_us: i64,
    pub duration_us: u64,
    pub width: u32,
    pub height: u32,
    pub compressed: bool,
    pub pixels: &'a [u8],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RasterDeltaOperation<'a> {
    Overwrite {
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        rgba: &'a [u8],
    },
    Copy {
        destination_x: u32,
        destination_y: u32,
        width: u32,
        height: u32,
        source_x: u32,
        source_y: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedRasterDeltaOperation<'a> {
    Overwrite {
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        rgba: Cow<'a, [u8]>,
    },
    Copy {
        destination_x: u32,
        destination_y: u32,
        width: u32,
        height: u32,
        source_x: u32,
        source_y: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedRasterDeltaFrame<'a> {
    pub epoch: u32,
    pub frame_id: u64,
    pub base_frame_id: u64,
    pub pts_us: i64,
    pub duration_us: u64,
    pub compressed: bool,
    pub operations: Vec<ParsedRasterDeltaOperation<'a>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SizeError {
    Overflow,
    TooLarge,
    Empty,
}

impl std::fmt::Display for SizeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for SizeError {}

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

pub fn video_body_len(max_access_unit_bytes: u32) -> Result<u32, SizeError> {
    if max_access_unit_bytes == 0 {
        return Err(SizeError::Empty);
    }
    let bytes = max_access_unit_bytes
        .checked_add(VIDEO_PACKET_PREFIX_SIZE as u32)
        .ok_or(SizeError::Overflow)?;
    if bytes > HARD_MAX_RECORD_BODY {
        Err(SizeError::TooLarge)
    } else {
        Ok(bytes)
    }
}

pub fn audio_body_len(max_access_unit_bytes: u32) -> Result<u32, SizeError> {
    if max_access_unit_bytes == 0 {
        return Err(SizeError::Empty);
    }
    let bytes = max_access_unit_bytes
        .checked_add(AUDIO_PACKET_PREFIX_SIZE as u32)
        .ok_or(SizeError::Overflow)?;
    if bytes > HARD_MAX_RECORD_BODY {
        Err(SizeError::TooLarge)
    } else {
        Ok(bytes)
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct MediaSequence {
    last_id: u64,
    last_epoch: u32,
}

impl MediaSequence {
    pub fn accept(&mut self, id: u64, epoch: u32) -> io::Result<()> {
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

    pub fn epoch(&self) -> u32 {
        self.last_epoch
    }
}

pub struct VideoPacket<'a> {
    pub epoch: u32,
    pub packet_id: u64,
    pub pts_us: i64,
    pub dts_us: i64,
    pub duration_us: u64,
    pub key: bool,
    pub data: &'a [u8],
}

pub struct AudioPacket<'a> {
    pub epoch: u32,
    pub packet_id: u64,
    pub pts_us: i64,
    pub dts_us: i64,
    pub duration_us: u64,
    pub trim_start_samples: u32,
    pub trim_end_samples: u32,
    pub data: &'a [u8],
}

pub fn audio_packet_prefix(packet: &AudioPacket<'_>) -> io::Result<[u8; AUDIO_PACKET_PREFIX_SIZE]> {
    packet
        .data
        .len()
        .checked_add(AUDIO_PACKET_PREFIX_SIZE)
        .and_then(|length| u32::try_from(length).ok())
        .filter(|length| *length <= HARD_MAX_RECORD_BODY)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "audio packet too large"))?;
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

pub fn audio_packet_body(packet: AudioPacket<'_>) -> io::Result<Vec<u8>> {
    let prefix = audio_packet_prefix(&packet)?;
    let capacity = prefix.len() + packet.data.len();
    let mut body = Vec::with_capacity(capacity);
    body.extend_from_slice(&prefix);
    body.extend_from_slice(packet.data);
    Ok(body)
}

pub fn video_packet_prefix(packet: &VideoPacket<'_>) -> io::Result<[u8; VIDEO_PACKET_PREFIX_SIZE]> {
    packet
        .data
        .len()
        .checked_add(VIDEO_PACKET_PREFIX_SIZE)
        .and_then(|length| u32::try_from(length).ok())
        .filter(|length| *length <= HARD_MAX_RECORD_BODY)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "video packet too large"))?;
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

pub fn video_packet_body(packet: VideoPacket<'_>) -> io::Result<Vec<u8>> {
    let prefix = video_packet_prefix(&packet)?;
    let capacity = prefix.len() + packet.data.len();
    let mut body = Vec::with_capacity(capacity);
    body.extend_from_slice(&prefix);
    body.extend_from_slice(packet.data);
    Ok(body)
}

pub fn raster_full_frame_prefix(
    epoch: u32,
    frame_id: u64,
    width: u32,
    height: u32,
    data_len: usize,
) -> io::Result<[u8; RASTER_FRAME_PREFIX_SIZE + RASTER_RECT_SIZE]> {
    let expected_length = rgba8_pixel_len(width, height)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?
        as usize;
    if data_len != expected_length {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("RGBA data has {data_len} bytes, expected {expected_length}"),
        ));
    }
    rgba8_raw_frame_body_len(width, height)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    raster_full_frame_prefix_inner(epoch, frame_id, width, height, data_len, false)
}

fn raster_full_frame_prefix_inner(
    epoch: u32,
    frame_id: u64,
    width: u32,
    height: u32,
    data_len: usize,
    compressed: bool,
) -> io::Result<[u8; RASTER_FRAME_PREFIX_SIZE + RASTER_RECT_SIZE]> {
    if width == 0 || height == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "raster dimensions are empty",
        ));
    }
    let data_len = u32::try_from(data_len).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "raster frame exceeds u32 length",
        )
    })?;
    data_len
        .checked_add((RASTER_FRAME_PREFIX_SIZE + RASTER_RECT_SIZE) as u32)
        .filter(|length| *length <= HARD_MAX_RECORD_BODY)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "raster frame too large"))?;

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

pub fn raster_frame_body(
    epoch: u32,
    frame_id: u64,
    width: u32,
    height: u32,
    rgba: &[u8],
) -> io::Result<Vec<u8>> {
    raster_frame_body_with_compression(epoch, frame_id, width, height, rgba, false)
}

pub fn raster_frame_body_with_compression(
    epoch: u32,
    frame_id: u64,
    width: u32,
    height: u32,
    rgba: &[u8],
    compress: bool,
) -> io::Result<Vec<u8>> {
    let expected_length = rgba8_pixel_len(width, height)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?
        as usize;
    if rgba.len() != expected_length {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "RGBA data has {} bytes, expected {expected_length}",
                rgba.len()
            ),
        ));
    }
    #[cfg(all(feature = "native", not(target_arch = "wasm32")))]
    let compressed;
    let pixels = if compress {
        #[cfg(all(feature = "native", not(target_arch = "wasm32")))]
        {
            compressed = zstd::bulk::compress(rgba, 1)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
            compressed.as_slice()
        }
        #[cfg(any(not(feature = "native"), target_arch = "wasm32"))]
        {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
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

#[allow(clippy::too_many_arguments)]
pub fn raster_delta_frame_body(
    epoch: u32,
    frame_id: u64,
    base_frame_id: u64,
    pts_us: i64,
    duration_us: u64,
    source_width: u32,
    source_height: u32,
    effective_operation_limit: u32,
    operations: &[RasterDeltaOperation<'_>],
    compress: bool,
) -> io::Result<Vec<u8>> {
    validate_delta_operation_limit(effective_operation_limit, io::ErrorKind::InvalidInput)?;
    if frame_id == 0 || base_frame_id == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "delta frame and base IDs must be nonzero",
        ));
    }
    if source_width == 0 || source_height == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "delta source dimensions are empty",
        ));
    }
    let count = u32::try_from(operations.len()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "too many raster delta operations",
        )
    })?;
    if count == 0 || count > effective_operation_limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "raster delta operation count is outside the effective limit",
        ));
    }

    let descriptor_bytes = operations
        .len()
        .checked_mul(RASTER_DELTA_OPERATION_SIZE)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "delta body overflow"))?;
    let payload_start = RASTER_FRAME_PREFIX_SIZE
        .checked_add(descriptor_bytes)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "delta body overflow"))?;
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
                    io::ErrorKind::InvalidInput,
                )?;
                let expected = rgba8_pixel_len(*width, *height)
                    .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?
                    as usize;
                if rgba.len() != expected {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "delta overwrite RGBA length does not match its rectangle",
                    ));
                }
                let payload =
                    if compress {
                        #[cfg(all(feature = "native", not(target_arch = "wasm32")))]
                        {
                            Cow::Owned(zstd::bulk::compress(rgba, 1).map_err(|error| {
                                io::Error::new(io::ErrorKind::InvalidData, error)
                            })?)
                        }
                        #[cfg(any(not(feature = "native"), target_arch = "wasm32"))]
                        {
                            return Err(io::Error::new(
                                io::ErrorKind::Unsupported,
                                "zstd raster compression requires the native feature",
                            ));
                        }
                    } else {
                        Cow::Borrowed(*rgba)
                    };
                let encoded_length = u32::try_from(payload.len()).map_err(|_| {
                    io::Error::new(io::ErrorKind::InvalidInput, "delta overwrite is too large")
                })?;
                payload_length = payload_length.checked_add(payload.len()).ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidInput, "delta body overflow")
                })?;
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
                    io::ErrorKind::InvalidInput,
                )?;
                validate_raster_rectangle(
                    *source_x,
                    *source_y,
                    *width,
                    *height,
                    source_width,
                    source_height,
                    io::ErrorKind::InvalidInput,
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
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "delta frame too large"))?;
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

pub fn parse_video_packet(body: &[u8]) -> io::Result<ParsedVideoPacket<'_>> {
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

pub fn parse_audio_packet(body: &[u8]) -> io::Result<ParsedAudioPacket<'_>> {
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

pub fn parse_full_raster_frame(body: &[u8]) -> io::Result<ParsedRasterFrame<'_>> {
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
    if (!compressed && data_length != expected_length) || body.len() != header_length + data_length
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

pub fn parse_delta_raster_frame(
    body: &[u8],
    source_width: u32,
    source_height: u32,
    effective_operation_limit: u32,
) -> io::Result<ParsedRasterDeltaFrame<'_>> {
    validate_delta_operation_limit(effective_operation_limit, io::ErrorKind::InvalidData)?;
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
            io::ErrorKind::InvalidData,
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
                    io::ErrorKind::InvalidData,
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
                if is_zstd_skippable_frame(encoded) {
                    return Err(invalid("zstd skippable frames are forbidden"));
                }
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

pub fn decode_raster_pixels(frame: ParsedRasterFrame<'_>) -> io::Result<Vec<u8>> {
    let expected = rgba8_pixel_len(frame.width, frame.height)
        .map_err(|_| invalid("raster dimensions overflow"))? as usize;
    if !frame.compressed {
        return Ok(frame.pixels.to_vec());
    }
    if is_zstd_skippable_frame(frame.pixels) {
        return Err(invalid("zstd skippable frames are forbidden"));
    }
    decode_zstd_pixels(frame.pixels, expected)
}

fn is_zstd_skippable_frame(bytes: &[u8]) -> bool {
    bytes.len() < 4
        || u32::from_le_bytes(bytes[..4].try_into().unwrap()) & 0xffff_fff0 == 0x184d_2a50
}

fn validate_delta_operation_limit(limit: u32, kind: io::ErrorKind) -> io::Result<()> {
    if !(1..=RASTER_DELTA_OPERATION_LIMIT).contains(&limit) {
        return Err(io::Error::new(
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
    kind: io::ErrorKind,
) -> io::Result<()> {
    if width == 0
        || height == 0
        || x.checked_add(width)
            .is_none_or(|right| right > source_width)
        || y.checked_add(height)
            .is_none_or(|bottom| bottom > source_height)
    {
        return Err(io::Error::new(
            kind,
            "raster delta rectangle is empty, overflows, or exceeds the source",
        ));
    }
    Ok(())
}

#[cfg(all(feature = "native", not(target_arch = "wasm32")))]
fn decode_zstd_pixels(pixels: &[u8], expected: usize) -> io::Result<Vec<u8>> {
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
        .read_to_end(&mut output)?;
    let cursor = decoder.finish();
    if output.len() != expected || cursor.get_ref().position() as usize != pixels.len() {
        return Err(invalid(
            "zstd raster has wrong output size or trailing data",
        ));
    }
    Ok(output)
}

#[cfg(target_arch = "wasm32")]
fn decode_zstd_pixels(pixels: &[u8], expected: usize) -> io::Result<Vec<u8>> {
    let mut decoder = ruzstd::decoding::StreamingDecoder::new(pixels)
        .map_err(|_| invalid("invalid zstd frame"))?;
    let mut output = Vec::with_capacity(expected);
    decoder
        .by_ref()
        .take((expected + 1) as u64)
        .read_to_end(&mut output)?;
    if output.len() != expected
        || !decoder.decoder.is_finished()
        || decoder.decoder.bytes_read_from_source() as usize != pixels.len()
    {
        return Err(invalid(
            "zstd raster has wrong output size or trailing data",
        ));
    }
    Ok(output)
}

#[cfg(all(not(feature = "native"), not(target_arch = "wasm32")))]
fn decode_zstd_pixels(_pixels: &[u8], _expected: usize) -> io::Result<Vec<u8>> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "zstd raster decoding requires either native support or wasm32",
    ))
}

pub fn is_portable_packetization(codec: &str, packetization: &str) -> bool {
    matches!(
        (codec, packetization),
        ("h264", "h264-annexb-au-v1")
            | ("hevc", "hevc-annexb-au-v1")
            | ("vp9", "vp9-frame-v1")
            | ("av1", "av1-low-overhead-tu-v1")
    )
}

pub fn validate_portable_packetization(
    codec: &str,
    packetization: &str,
    data: &[u8],
) -> io::Result<()> {
    if data.is_empty() {
        return Err(invalid("portable access unit is empty"));
    }
    if !is_portable_packetization(codec, packetization) {
        return Err(invalid("unsupported portable codec/packetization pair"));
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
        _ => unreachable!("pair was checked above"),
    }
    Ok(())
}

/// Determine random-access status from the portable codec syntax rather than container metadata.
pub fn access_unit_is_key(codec: &str, data: &[u8]) -> io::Result<bool> {
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

fn annex_b_nal_headers(data: &[u8]) -> io::Result<impl Iterator<Item = u8> + '_> {
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

fn av1_frame_is_key(data: &[u8]) -> io::Result<bool> {
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

fn read_leb128(data: &[u8]) -> io::Result<(usize, usize)> {
    let mut value = 0_usize;
    for (index, byte) in data.iter().copied().take(8).enumerate() {
        value |= usize::from(byte & 0x7f) << (index * 7);
        if byte & 0x80 == 0 {
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

    fn read(&mut self, count: usize) -> io::Result<u8> {
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

fn read_u32(bytes: &[u8], offset: usize) -> io::Result<u32> {
    bytes
        .get(offset..offset + 4)
        .map(|value| u32::from_be_bytes(value.try_into().unwrap()))
        .ok_or_else(|| invalid("truncated media field"))
}

fn read_u64(bytes: &[u8], offset: usize) -> io::Result<u64> {
    bytes
        .get(offset..offset + 8)
        .map(|value| u64::from_be_bytes(value.try_into().unwrap()))
        .ok_or_else(|| invalid("truncated media field"))
}

fn read_i64(bytes: &[u8], offset: usize) -> io::Result<i64> {
    bytes
        .get(offset..offset + 8)
        .map(|value| i64::from_be_bytes(value.try_into().unwrap()))
        .ok_or_else(|| invalid("truncated media field"))
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_packet_uses_documented_48_byte_prefix() {
        let body = video_packet_body(VideoPacket {
            epoch: 1,
            packet_id: 9,
            pts_us: 10,
            dts_us: 8,
            duration_us: 16_667,
            key: true,
            data: &[1, 2, 3],
        })
        .unwrap();
        assert_eq!(body.len(), 51);
        assert_eq!(u32::from_be_bytes(body[0..4].try_into().unwrap()), 1);
        assert_eq!(
            u32::from_be_bytes(body[4..8].try_into().unwrap()),
            VIDEO_PACKET_KEY
        );
        assert_eq!(&body[48..], &[1, 2, 3]);
    }

    #[test]
    fn vivid_1_0_media_prefixes_are_frozen_golden_vectors() {
        let video = video_packet_prefix(&VideoPacket {
            epoch: 0x0102_0304,
            packet_id: 0x0506_0708_090a_0b0c,
            pts_us: -2,
            dts_us: 0x1112_1314_1516_1718,
            duration_us: 0x2122_2324_2526_2728,
            key: true,
            data: &[1],
        })
        .unwrap();
        assert_eq!(
            video,
            [
                0x01, 0x02, 0x03, 0x04, 0x00, 0x00, 0x00, 0x01, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a,
                0x0b, 0x0c, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xfe, 0x11, 0x12, 0x13, 0x14,
                0x15, 0x16, 0x17, 0x18, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x28, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            ]
        );

        let audio = audio_packet_prefix(&AudioPacket {
            epoch: 0x3132_3334,
            packet_id: 0x3536_3738_393a_3b3c,
            pts_us: -3,
            dts_us: 0x4142_4344_4546_4748,
            duration_us: 0x5152_5354_5556_5758,
            trim_start_samples: 0x6162_6364,
            trim_end_samples: 0x7172_7374,
            data: &[1],
        })
        .unwrap();
        assert_eq!(
            audio,
            [
                0x31, 0x32, 0x33, 0x34, 0x00, 0x00, 0x00, 0x00, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3a,
                0x3b, 0x3c, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xfd, 0x41, 0x42, 0x43, 0x44,
                0x45, 0x46, 0x47, 0x48, 0x51, 0x52, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x61, 0x62,
                0x63, 0x64, 0x71, 0x72, 0x73, 0x74,
            ]
        );

        let raster =
            raster_full_frame_prefix(0x0102_0304, 0x0506_0708_090a_0b0c, 3, 2, 24).unwrap();
        assert_eq!(
            raster,
            [
                0x01, 0x02, 0x03, 0x04, // epoch
                0x00, 0x00, 0x00, 0x01, // FULL
                0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, // frame ID
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // base frame ID
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // PTS
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // duration
                0x00, 0x00, 0x00, 0x01, // rectangle count
                0x00, 0x00, 0x00, 0x00, // reserved
                0x00, 0x00, 0x00, 0x00, // x
                0x00, 0x00, 0x00, 0x00, // y
                0x00, 0x00, 0x00, 0x03, // width
                0x00, 0x00, 0x00, 0x02, // height
                0x00, 0x00, 0x00, 0x00, // data offset
                0x00, 0x00, 0x00, 0x18, // data length
            ]
        );
    }

    #[test]
    fn borrowed_prefixes_match_owned_media_bodies() {
        let video = VideoPacket {
            epoch: 3,
            packet_id: 11,
            pts_us: 20,
            dts_us: 18,
            duration_us: 16_667,
            key: false,
            data: &[1, 2, 3, 4],
        };
        let video_prefix = video_packet_prefix(&video).unwrap();
        let video_body = video_packet_body(VideoPacket {
            epoch: video.epoch,
            packet_id: video.packet_id,
            pts_us: video.pts_us,
            dts_us: video.dts_us,
            duration_us: video.duration_us,
            key: video.key,
            data: video.data,
        })
        .unwrap();
        assert_eq!([video_prefix.as_slice(), video.data].concat(), video_body);

        let audio = AudioPacket {
            epoch: 4,
            packet_id: 12,
            pts_us: 30,
            dts_us: 28,
            duration_us: 20_000,
            trim_start_samples: 32,
            trim_end_samples: 16,
            data: &[5, 6, 7],
        };
        let audio_prefix = audio_packet_prefix(&audio).unwrap();
        let audio_body = audio_packet_body(AudioPacket {
            epoch: audio.epoch,
            packet_id: audio.packet_id,
            pts_us: audio.pts_us,
            dts_us: audio.dts_us,
            duration_us: audio.duration_us,
            trim_start_samples: audio.trim_start_samples,
            trim_end_samples: audio.trim_end_samples,
            data: audio.data,
        })
        .unwrap();
        assert_eq!([audio_prefix.as_slice(), audio.data].concat(), audio_body);

        let pixels = [8; 24];
        let raster_prefix = raster_full_frame_prefix(5, 13, 3, 2, pixels.len()).unwrap();
        let raster_body = raster_frame_body(5, 13, 3, 2, &pixels).unwrap();
        assert_eq!(
            [raster_prefix.as_slice(), pixels.as_slice()].concat(),
            raster_body
        );
        assert!(raster_full_frame_prefix(5, 13, 3, 2, pixels.len() - 1).is_err());
    }

    #[test]
    fn audio_packet_round_trip_uses_documented_prefix() {
        let body = audio_packet_body(AudioPacket {
            epoch: 2,
            packet_id: 7,
            pts_us: 12,
            dts_us: 10,
            duration_us: 20_000,
            trim_start_samples: 2112,
            trim_end_samples: 17,
            data: &[4, 5, 6],
        })
        .unwrap();
        let parsed = parse_audio_packet(&body).unwrap();
        assert_eq!(parsed.epoch, 2);
        assert_eq!(parsed.packet_id, 7);
        assert_eq!(parsed.trim_start_samples, 2112);
        assert_eq!(parsed.trim_end_samples, 17);
        assert_eq!(parsed.data, &[4, 5, 6]);
        assert_eq!(audio_body_len(3).unwrap(), 51);
    }

    #[test]
    fn audio_packets_reject_malformed_headers_and_reused_ids() {
        assert!(parse_audio_packet(&[0; 47]).is_err());
        assert!(parse_audio_packet(&[0; 48]).is_err());
        let mut body = audio_packet_body(AudioPacket {
            epoch: 1,
            packet_id: 1,
            pts_us: 0,
            dts_us: 0,
            duration_us: 1_000,
            trim_start_samples: 0,
            trim_end_samples: 0,
            data: &[1],
        })
        .unwrap();
        body[7] = 1;
        assert!(parse_audio_packet(&body).is_err());

        let mut sequence = MediaSequence::default();
        assert!(sequence.accept(1, 1).is_ok());
        assert!(sequence.accept(1, 1).is_err());
        assert!(sequence.accept(2, 0).is_err());
    }

    #[test]
    fn full_raster_contains_one_rectangle() {
        let body = raster_frame_body(1, 2, 2, 1, &[0; 8]).unwrap();
        assert_eq!(body.len(), 48 + 24 + 8);
        assert_eq!(u32::from_be_bytes(body[40..44].try_into().unwrap()), 1);
        assert_eq!(&body[72..], &[0; 8]);
        let parsed = parse_full_raster_frame(&body).unwrap();
        assert_eq!((parsed.width, parsed.height), (2, 1));
        assert_eq!(parsed.pixels, &[0; 8]);
    }

    fn mixed_delta_body(compress: bool) -> Vec<u8> {
        raster_delta_frame_body(
            3,
            9,
            8,
            -10,
            16_667,
            4,
            3,
            4,
            &[
                RasterDeltaOperation::Copy {
                    destination_x: 0,
                    destination_y: 1,
                    width: 4,
                    height: 2,
                    source_x: 0,
                    source_y: 0,
                },
                RasterDeltaOperation::Overwrite {
                    x: 0,
                    y: 0,
                    width: 4,
                    height: 1,
                    rgba: &[7; 16],
                },
            ],
            compress,
        )
        .unwrap()
    }

    #[test]
    fn raster_delta_round_trip_preserves_order_without_payload_offsets() {
        let body = mixed_delta_body(false);
        assert_eq!(body.len(), 48 + 2 * 32 + 16);
        assert_eq!(
            u32::from_be_bytes(body[4..8].try_into().unwrap()),
            RASTER_FRAME_DELTA
        );
        assert_eq!(u64::from_be_bytes(body[16..24].try_into().unwrap()), 8);
        assert_eq!(u32::from_be_bytes(body[40..44].try_into().unwrap()), 2);
        assert_eq!(
            u32::from_be_bytes(body[48 + 28..48 + 32].try_into().unwrap()),
            0
        );
        assert_eq!(
            u32::from_be_bytes(body[80 + 28..80 + 32].try_into().unwrap()),
            16
        );
        assert_eq!(&body[112..], &[7; 16]);

        let parsed = parse_delta_raster_frame(&body, 4, 3, 4).unwrap();
        assert_eq!(
            (parsed.epoch, parsed.frame_id, parsed.base_frame_id),
            (3, 9, 8)
        );
        assert_eq!((parsed.pts_us, parsed.duration_us), (-10, 16_667));
        assert!(!parsed.compressed);
        assert_eq!(
            parsed.operations[0],
            ParsedRasterDeltaOperation::Copy {
                destination_x: 0,
                destination_y: 1,
                width: 4,
                height: 2,
                source_x: 0,
                source_y: 0,
            }
        );
        assert_eq!(
            parsed.operations[1],
            ParsedRasterDeltaOperation::Overwrite {
                x: 0,
                y: 0,
                width: 4,
                height: 1,
                rgba: Cow::Borrowed(&[7; 16]),
            }
        );
    }

    #[test]
    #[cfg(feature = "native")]
    fn raster_delta_zstd_validates_and_decodes_each_overwrite_frame() {
        let body = mixed_delta_body(true);
        let parsed = parse_delta_raster_frame(&body, 4, 3, 2).unwrap();
        assert!(parsed.compressed);
        match &parsed.operations[1] {
            ParsedRasterDeltaOperation::Overwrite { rgba, .. } => {
                assert_eq!(rgba.as_ref(), &[7; 16]);
                assert!(matches!(rgba, Cow::Owned(_)));
            }
            _ => panic!("second operation should be an overwrite"),
        }

        let mut concatenated = body;
        let second = zstd::bulk::compress(&[7; 16], 1).unwrap();
        concatenated.extend_from_slice(&second);
        let length = u32::from_be_bytes(concatenated[108..112].try_into().unwrap())
            + u32::try_from(second.len()).unwrap();
        concatenated[108..112].copy_from_slice(&length.to_be_bytes());
        assert!(parse_delta_raster_frame(&concatenated, 4, 3, 2).is_err());
    }

    #[test]
    fn raster_delta_rejects_invalid_header_and_operation_counts() {
        let baseline = mixed_delta_body(false);
        let mut malformed = baseline.clone();
        malformed[4..8].copy_from_slice(&(RASTER_FRAME_FULL | RASTER_FRAME_DELTA).to_be_bytes());
        assert!(parse_delta_raster_frame(&malformed, 4, 3, 4).is_err());

        let mut malformed = baseline.clone();
        malformed[16..24].copy_from_slice(&0_u64.to_be_bytes());
        assert!(parse_delta_raster_frame(&malformed, 4, 3, 4).is_err());

        let mut malformed = baseline.clone();
        malformed[44..48].copy_from_slice(&1_u32.to_be_bytes());
        assert!(parse_delta_raster_frame(&malformed, 4, 3, 4).is_err());

        let mut malformed = baseline.clone();
        malformed[40..44].copy_from_slice(&0_u32.to_be_bytes());
        assert!(parse_delta_raster_frame(&malformed, 4, 3, 4).is_err());
        assert!(parse_delta_raster_frame(&baseline, 4, 3, 1).is_err());
        assert!(parse_delta_raster_frame(&baseline, 4, 3, 17).is_err());
        assert!(parse_delta_raster_frame(&baseline[..79], 4, 3, 4).is_err());
    }

    #[test]
    fn raster_delta_rejects_every_malformed_rectangle_form() {
        let baseline = mixed_delta_body(false);
        for (offset, value) in [
            (48, 3_u32),    // unknown copy operation kind
            (60, 0),        // zero destination width
            (56, u32::MAX), // overflowing destination Y
            (52, u32::MAX), // overflowing destination X
            (68, u32::MAX), // overflowing copy source X
            (72, u32::MAX), // overflowing copy source Y
        ] {
            let mut malformed = baseline.clone();
            malformed[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
            assert!(
                parse_delta_raster_frame(&malformed, 4, 3, 4).is_err(),
                "field at offset {offset} unexpectedly passed"
            );
        }

        let mut overwrite_reserved = baseline.clone();
        overwrite_reserved[100..104].copy_from_slice(&1_u32.to_be_bytes());
        assert!(parse_delta_raster_frame(&overwrite_reserved, 4, 3, 4).is_err());

        let mut copy_payload = baseline.clone();
        copy_payload[76..80].copy_from_slice(&1_u32.to_be_bytes());
        assert!(parse_delta_raster_frame(&copy_payload, 4, 3, 4).is_err());
    }

    #[test]
    fn raster_delta_rejects_payload_length_mismatch_and_trailing_bytes() {
        let baseline = mixed_delta_body(false);
        for length in [15_u32, 17, u32::MAX] {
            let mut malformed = baseline.clone();
            malformed[108..112].copy_from_slice(&length.to_be_bytes());
            assert!(parse_delta_raster_frame(&malformed, 4, 3, 4).is_err());
        }
        let mut trailing = baseline;
        trailing.push(0);
        assert!(parse_delta_raster_frame(&trailing, 4, 3, 4).is_err());
    }

    #[test]
    fn raster_delta_builder_rejects_invalid_inputs_before_encoding() {
        let overwrite = RasterDeltaOperation::Overwrite {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
            rgba: &[0; 4],
        };
        for (frame, base, width, height, limit) in [
            (0, 1, 1, 1, 1),
            (1, 0, 1, 1, 1),
            (1, 1, 0, 1, 1),
            (1, 1, 1, 0, 1),
            (1, 1, 1, 1, 0),
            (1, 1, 1, 1, 17),
        ] {
            assert!(
                raster_delta_frame_body(
                    0,
                    frame,
                    base,
                    0,
                    0,
                    width,
                    height,
                    limit,
                    &[overwrite],
                    false,
                )
                .is_err()
            );
        }
        assert!(raster_delta_frame_body(0, 1, 1, 0, 0, 1, 1, 1, &[], false).is_err());
        let wrong_length = RasterDeltaOperation::Overwrite {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
            rgba: &[0; 3],
        };
        assert!(raster_delta_frame_body(0, 1, 1, 0, 0, 1, 1, 1, &[wrong_length], false).is_err());
    }

    #[test]
    fn raster_body_limits_are_exact() {
        assert!(rgba8_raw_frame_body_len(4095, 4095).is_ok());
        assert_eq!(
            rgba8_raw_frame_body_len(4096, 4096),
            Err(SizeError::TooLarge)
        );
        assert!(rgba8_raw_frame_body_len(8192, 1).is_ok());
    }

    #[test]
    #[cfg(feature = "native")]
    fn zstd_raster_round_trip() {
        let pixels = vec![7; 64];
        let body = raster_frame_body_with_compression(1, 1, 4, 4, &pixels, true).unwrap();
        let parsed = parse_full_raster_frame(&body).unwrap();
        assert!(parsed.compressed);
        assert_eq!(decode_raster_pixels(parsed).unwrap(), pixels);
    }

    #[test]
    #[cfg(feature = "native")]
    fn zstd_rejects_concatenated_frames() {
        let pixels = vec![7; 64];
        let mut body = raster_frame_body_with_compression(1, 1, 4, 4, &pixels, true).unwrap();
        body.extend_from_slice(&zstd::bulk::compress(&pixels, 1).unwrap());
        let compressed_length = u32::try_from(body.len() - 72).unwrap();
        body[68..72].copy_from_slice(&compressed_length.to_be_bytes());
        let parsed = parse_full_raster_frame(&body).unwrap();
        assert!(decode_raster_pixels(parsed).is_err());
    }

    #[test]
    fn rejects_ambiguous_or_reserved_media_flags() {
        let mut packet = video_packet_body(VideoPacket {
            epoch: 1,
            packet_id: 1,
            pts_us: 0,
            dts_us: 0,
            duration_us: 0,
            key: true,
            data: &[1],
        })
        .unwrap();
        packet[4..8].copy_from_slice(&(VIDEO_PACKET_KEY | VIDEO_PACKET_DELTA).to_be_bytes());
        assert!(parse_video_packet(&packet).is_err());

        let mut raster = raster_frame_body(1, 1, 1, 1, &[0; 4]).unwrap();
        raster[4..8].copy_from_slice(&(RASTER_FRAME_FULL | 4).to_be_bytes());
        assert!(parse_full_raster_frame(&raster).is_err());
    }

    #[test]
    fn portable_key_status_comes_from_codec_syntax() {
        assert!(access_unit_is_key("h264", &[0, 0, 0, 1, 0x65]).unwrap());
        assert!(!access_unit_is_key("h264", &[0, 0, 1, 0x41]).unwrap());
        assert!(access_unit_is_key("hevc", &[0, 0, 1, 19 << 1]).unwrap());
        assert!(access_unit_is_key("vp9", &[0x80]).unwrap());
        assert!(!access_unit_is_key("vp9", &[0x84]).unwrap());
        assert!(access_unit_is_key("av1", &[0x32, 0x01, 0x00]).unwrap());
        assert!(!access_unit_is_key("av1", &[0x32, 0x01, 0x20]).unwrap());
    }
}
