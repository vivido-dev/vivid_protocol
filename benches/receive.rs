//! Receive-path benchmark: what a presenter spends reading and parsing what a producer sends.
//!
//! Each scenario reports wall time per operation and throughput. Run it with
//! `cargo bench --bench receive`, optionally with a substring that selects scenarios, such as
//! `cargo bench --bench receive -- raster`. The send path is measured by `stage0`.

use std::hint::black_box;
use std::io::{self, Cursor};
use std::time::{Duration, Instant};

use vivid_protocol::{
    cbor,
    media::{self, RasterDeltaFrame, RasterDeltaOperation, RasterFrame, RasterSize, VideoPacket},
    messages::{Hello, HelloAuthentication},
    registry::{CORE_CONTROL, DESKTOP_SURFACE, LIVE_MEDIA},
    wire::{Connection, ConnectionKind, RecordHeader},
};

/// How long each scenario runs after warming up; long enough to average out scheduler noise.
const MEASUREMENT: Duration = Duration::from_millis(500);
/// Raster scenarios use a 1280x720 frame, a common desktop-share size.
const RASTER: RasterSize = RasterSize {
    width: 1280,
    height: 720,
};
/// A 64 KiB video access unit, typical of a 720p key frame.
const VIDEO_UNIT_BYTES: usize = 64 * 1024;
/// Records in one stream for the record-reading scenario.
const STREAM_RECORDS: usize = 1024;

fn main() -> io::Result<()> {
    let filter = std::env::args()
        .skip(1)
        .find(|argument| !argument.starts_with('-'));
    let selected = |name: &str| filter.as_deref().is_none_or(|filter| name.contains(filter));
    println!("{:<34} {:>12} {:>12}", "scenario", "ns/op", "MiB/s");

    let video = video_body();
    if selected("read_records") {
        // Leaked once so each iteration reads the same bytes in place instead of copying them.
        let stream: &'static [u8] = Vec::leak(record_stream(&video));
        report("read_records", stream.len(), || read_records(stream))?;
    }
    if selected("parse_video_packet") {
        report("parse_video_packet", video.len(), || {
            media::parse_video_packet(black_box(&video)).map(drop)
        })?;
    }

    let pixels = gradient(RASTER);
    let compressed = raster_body(&pixels, true)?;
    let raw = raster_body(&pixels, false)?;
    if selected("raster_raw_decode") {
        report("raster_raw_decode", raw.len(), || decode_raster(&raw))?;
    }
    if selected("raster_zstd_decode") {
        report("raster_zstd_decode", pixels.len(), || {
            decode_raster(&compressed)
        })?;
    }
    if selected("raster_delta_parse") {
        let delta = delta_body(&pixels)?;
        report("raster_delta_parse", delta.len(), || {
            media::parse_delta_raster_frame(black_box(&delta), RASTER, 16).map(drop)
        })?;
    }

    let hello = hello_body()?;
    if selected("cbor_decode_hello") {
        let envelope = hello.clone();
        report("cbor_decode_hello", envelope.len(), || {
            cbor::decode(black_box(&envelope)).map(drop)
        })?;
    }
    if selected("hello_decode") {
        report("hello_decode", hello.len(), || {
            Hello::decode(black_box(&hello)).map(drop)
        })?;
    }
    Ok(())
}

/// Runs `operation` repeatedly for [`MEASUREMENT`] and prints its time and throughput.
fn report<E: std::error::Error + Send + Sync + 'static>(
    name: &str,
    bytes: usize,
    mut operation: impl FnMut() -> Result<(), E>,
) -> io::Result<()> {
    let run = |operation: &mut dyn FnMut() -> Result<(), E>| -> io::Result<(u64, Duration)> {
        let started = Instant::now();
        let mut iterations = 0_u64;
        while started.elapsed() < MEASUREMENT {
            operation().map_err(io::Error::other)?;
            iterations += 1;
        }
        Ok((iterations, started.elapsed()))
    };
    run(&mut operation)?;
    let (iterations, elapsed) = run(&mut operation)?;
    #[expect(
        clippy::cast_precision_loss,
        reason = "benchmark reports convert counts to floating-point rates"
    )]
    let (nanoseconds, mebibytes) = (
        elapsed.as_nanos() as f64 / iterations as f64,
        bytes as f64 * iterations as f64 / elapsed.as_secs_f64() / (1024.0 * 1024.0),
    );
    println!("{name:<34} {nanoseconds:>12.0} {mebibytes:>12.1}");
    Ok(())
}

fn video_body() -> Vec<u8> {
    let data = vec![0x5a; VIDEO_UNIT_BYTES];
    media::video_packet_body(VideoPacket {
        epoch: 1,
        packet_id: 1,
        pts_us: 0,
        dts_us: 0,
        duration_us: 16_667,
        key: true,
        data: &data,
    })
    .expect("a 64 KiB video packet is within every record limit")
}

/// A track-connection byte stream of [`STREAM_RECORDS`] video records with consecutive sequences.
fn record_stream(body: &[u8]) -> Vec<u8> {
    let mut stream = Vec::with_capacity(STREAM_RECORDS * (body.len() + 24));
    for sequence in 1..=STREAM_RECORDS as u64 {
        let header = RecordHeader {
            body_length: u32::try_from(body.len()).expect("the video body fits a record"),
            record_type: vivid_protocol::messages::VIDEO_PACKET,
            flags: 0,
            object_id: 7,
            sequence,
        };
        stream.extend_from_slice(&header.encode());
        stream.extend_from_slice(body);
    }
    stream
}

fn read_records(stream: &'static [u8]) -> io::Result<()> {
    let connection =
        Connection::from_streams(Cursor::new(stream), io::sink(), ConnectionKind::Track)?;
    let (mut reader, _writer) = connection.split()?;
    let mut body = Vec::new();
    for _ in 0..STREAM_RECORDS {
        black_box(reader.read_record_into(&mut body)?);
    }
    Ok(())
}

fn gradient(size: RasterSize) -> Vec<u8> {
    let pixels = size.width as usize * size.height as usize;
    (0..pixels)
        .flat_map(|index| {
            let x = (index % size.width as usize).to_le_bytes()[0];
            let y = (index / size.width as usize).to_le_bytes()[0];
            [x, y, x ^ y, 0xff]
        })
        .collect()
}

fn raster_body(pixels: &[u8], compress: bool) -> Result<Vec<u8>, vivid_protocol::CodecError> {
    let frame = RasterFrame {
        epoch: 1,
        frame_id: 1,
        width: RASTER.width,
        height: RASTER.height,
        rgba: pixels,
    };
    media::raster_frame_body_with_compression(&frame, compress)
}

fn decode_raster(body: &[u8]) -> Result<(), vivid_protocol::CodecError> {
    let frame = media::parse_full_raster_frame(black_box(body))?;
    black_box(media::decode_raster_pixels(frame)?);
    Ok(())
}

/// A delta of sixteen 64x64 overwrites, the shape of scattered UI damage.
fn delta_body(pixels: &[u8]) -> Result<Vec<u8>, vivid_protocol::CodecError> {
    let tile = &pixels[..64 * 64 * 4];
    let operations: Vec<_> = (0..16)
        .map(|index| RasterDeltaOperation::Overwrite {
            x: index * 64,
            y: 0,
            width: 64,
            height: 64,
            rgba: tile,
        })
        .collect();
    let frame = RasterDeltaFrame {
        epoch: 1,
        frame_id: 2,
        base_frame_id: 1,
        pts_us: 0,
        duration_us: 16_667,
        width: RASTER.width,
        height: RASTER.height,
        operations: &operations,
    };
    media::raster_delta_frame_body(&frame, 16, false)
}

fn hello_body() -> Result<Vec<u8>, vivid_protocol::messages::MessageError> {
    Hello {
        producer_name: "receive benchmark".into(),
        producer_version: "1".into(),
        required_profiles: vec![
            DESKTOP_SURFACE.into(),
            LIVE_MEDIA.into(),
            CORE_CONTROL.into(),
        ],
        optional_profiles: vec![],
        maximum_control_body: vivid_protocol::CONTROL_MAX_RECORD_BODY,
        client_nonce: [1; 32],
        authentication: HelloAuthentication::Root { proof: [2; 32] },
        target_profile: DESKTOP_SURFACE.into(),
        extensions: vec![],
    }
    .encode(1)
}
