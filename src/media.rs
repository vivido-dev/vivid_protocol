use std::io;

pub const VIDEO_PACKET_KEY: u32 = 1 << 0;
pub const VIDEO_PACKET_DELTA: u32 = 1 << 1;
pub const RASTER_FRAME_FULL: u32 = 1 << 0;

const VIDEO_PACKET_PREFIX_SIZE: usize = 48;
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
pub struct ParsedRasterFrame<'a> {
    pub epoch: u32,
    pub frame_id: u64,
    pub pts_us: i64,
    pub duration_us: u64,
    pub width: u32,
    pub height: u32,
    pub rgba: &'a [u8],
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
    let expected_length = usize::try_from(width)
        .ok()
        .and_then(|width| {
            usize::try_from(height)
                .ok()
                .and_then(|height| width.checked_mul(height))
        })
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "raster dimensions overflow"))?;
    if rgba.len() != expected_length {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "RGBA data has {} bytes, expected {expected_length}",
                rgba.len()
            ),
        ));
    }
    let data_length = u32::try_from(rgba.len()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "raster frame exceeds u32 length",
        )
    })?;

    let mut body = Vec::with_capacity(RASTER_FRAME_PREFIX_SIZE + RASTER_RECT_SIZE + rgba.len());
    push_u32(&mut body, epoch);
    push_u32(&mut body, RASTER_FRAME_FULL);
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
    body.extend_from_slice(rgba);
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

pub fn parse_full_raster_frame(body: &[u8]) -> io::Result<ParsedRasterFrame<'_>> {
    let header_length = RASTER_FRAME_PREFIX_SIZE + RASTER_RECT_SIZE;
    if body.len() < header_length {
        return Err(invalid(
            "raster frame is shorter than one full-frame rectangle",
        ));
    }
    if read_u32(body, 4)? != RASTER_FRAME_FULL
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
    let expected_length = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| invalid("raster dimensions overflow"))?;
    if data_length != expected_length || body.len() != header_length + data_length {
        return Err(invalid("raster RGBA byte length does not match dimensions"));
    }
    Ok(ParsedRasterFrame {
        epoch: read_u32(body, 0)?,
        frame_id: read_u64(body, 8)?,
        pts_us: read_i64(body, 24)?,
        duration_us: read_u64(body, 32)?,
        width,
        height,
        rgba: &body[header_length..],
    })
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
    fn full_raster_contains_one_rectangle() {
        let body = raster_frame_body(1, 2, 2, 1, &[0; 8]).unwrap();
        assert_eq!(body.len(), 48 + 24 + 8);
        assert_eq!(u32::from_be_bytes(body[40..44].try_into().unwrap()), 1);
        assert_eq!(&body[72..], &[0; 8]);
        let parsed = parse_full_raster_frame(&body).unwrap();
        assert_eq!((parsed.width, parsed.height), (2, 1));
        assert_eq!(parsed.rgba, &[0; 8]);
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
        raster[4..8].copy_from_slice(&(RASTER_FRAME_FULL | 2).to_be_bytes());
        assert!(parse_full_raster_frame(&raster).is_err());
    }
}
