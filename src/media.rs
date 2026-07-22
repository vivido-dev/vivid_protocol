use std::io;

#[cfg(all(feature = "native", not(target_arch = "wasm32")))]
use std::io::Cursor;
#[cfg(any(feature = "native", target_arch = "wasm32"))]
use std::io::Read;

use crate::HARD_MAX_RECORD_BODY;

pub const VIDEO_PACKET_KEY: u32 = 1 << 0;
pub const VIDEO_PACKET_DELTA: u32 = 1 << 1;
pub const RASTER_FRAME_FULL: u32 = 1 << 0;
pub const RASTER_FRAME_ZSTD: u32 = 1 << 1;

const VIDEO_PACKET_PREFIX_SIZE: usize = 48;
const AUDIO_PACKET_PREFIX_SIZE: usize = 48;
const RASTER_FRAME_PREFIX_SIZE: usize = 48;
const RASTER_RECT_SIZE: usize = 24;

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

pub fn audio_packet_body(packet: AudioPacket<'_>) -> io::Result<Vec<u8>> {
    let capacity = AUDIO_PACKET_PREFIX_SIZE
        .checked_add(packet.data.len())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "audio packet too large"))?;
    let mut body = Vec::with_capacity(capacity);
    push_u32(&mut body, packet.epoch);
    push_u32(&mut body, 0);
    push_u64(&mut body, packet.packet_id);
    push_i64(&mut body, packet.pts_us);
    push_i64(&mut body, packet.dts_us);
    push_u64(&mut body, packet.duration_us);
    push_u32(&mut body, packet.trim_start_samples);
    push_u32(&mut body, packet.trim_end_samples);
    body.extend_from_slice(packet.data);
    Ok(body)
}

pub fn video_packet_body(packet: VideoPacket<'_>) -> io::Result<Vec<u8>> {
    let capacity = VIDEO_PACKET_PREFIX_SIZE
        .checked_add(packet.data.len())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "video packet too large"))?;
    let mut body = Vec::with_capacity(capacity);
    push_u32(&mut body, packet.epoch);
    push_u32(
        &mut body,
        if packet.key {
            VIDEO_PACKET_KEY
        } else {
            VIDEO_PACKET_DELTA
        },
    );
    push_u64(&mut body, packet.packet_id);
    push_i64(&mut body, packet.pts_us);
    push_i64(&mut body, packet.dts_us);
    push_u64(&mut body, packet.duration_us);
    push_u32(&mut body, 0); // side data length
    push_u32(&mut body, 0); // reserved
    body.extend_from_slice(packet.data);
    Ok(body)
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
    let data_length = u32::try_from(pixels.len()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "raster frame exceeds u32 length",
        )
    })?;

    let mut body = Vec::with_capacity(RASTER_FRAME_PREFIX_SIZE + RASTER_RECT_SIZE + pixels.len());
    push_u32(&mut body, epoch);
    push_u32(
        &mut body,
        RASTER_FRAME_FULL | if compress { RASTER_FRAME_ZSTD } else { 0 },
    );
    push_u64(&mut body, frame_id);
    push_u64(&mut body, 0); // no base frame
    push_i64(&mut body, 0); // PTS
    push_u64(&mut body, 0); // unknown duration
    push_u32(&mut body, 1); // one rectangle
    push_u32(&mut body, 0); // reserved

    push_u32(&mut body, 0); // x
    push_u32(&mut body, 0); // y
    push_u32(&mut body, width);
    push_u32(&mut body, height);
    push_u32(&mut body, 0); // data offset from rectangle-data area
    push_u32(&mut body, data_length);
    body.extend_from_slice(pixels);
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

pub fn decode_raster_pixels(frame: ParsedRasterFrame<'_>) -> io::Result<Vec<u8>> {
    let expected = rgba8_pixel_len(frame.width, frame.height)
        .map_err(|_| invalid("raster dimensions overflow"))? as usize;
    if !frame.compressed {
        return Ok(frame.pixels.to_vec());
    }
    if frame.pixels.len() < 4
        || u32::from_le_bytes(frame.pixels[..4].try_into().unwrap()) == 0x184d2a50
    {
        return Err(invalid("zstd skippable frames are forbidden"));
    }
    decode_zstd_pixels(frame.pixels, expected)
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

fn push_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_be_bytes());
}

fn push_u64(bytes: &mut Vec<u8>, value: u64) {
    bytes.extend_from_slice(&value.to_be_bytes());
}

fn push_i64(bytes: &mut Vec<u8>, value: i64) {
    bytes.extend_from_slice(&value.to_be_bytes());
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
