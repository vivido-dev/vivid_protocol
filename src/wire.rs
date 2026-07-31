use std::io;

#[cfg(feature = "native")]
use std::fs::{self, File};
#[cfg(feature = "native")]
use std::io::{IoSlice, Read, Write};
#[cfg(feature = "native")]
use std::net::TcpStream;
#[cfg(all(feature = "native", unix))]
use std::os::unix::net::UnixStream;
#[cfg(feature = "native")]
use std::path::{Path, PathBuf};
#[cfg(feature = "native")]
use std::sync::{Arc, Mutex};
#[cfg(feature = "native")]
use std::time::Duration;

#[cfg(feature = "native")]
use super::DEFAULT_MAX_RECORD_BODY;
use super::{HARD_MAX_RECORD_BODY, VIVID_MAJOR, VIVID_MINOR};
#[cfg(feature = "native")]
use crate::messages::{ERROR, INPUT_RESET, INPUT_REVOKED, MAX_CHANNEL_DATA, PONG};

pub const PREFACE_SIZE: usize = 16;
pub const HEADER_SIZE: usize = 24;
const MAGIC: &[u8; 4] = b"VIVD";

pub const RECORD_OPTIONAL: u16 = 1 << 0;
pub const RECORD_KNOWN_FLAGS: u16 = RECORD_OPTIONAL;
#[cfg(feature = "native")]
const BATCH_RECORD_LIMIT: usize = 32;
#[cfg(feature = "native")]
const MAX_VECTORED_SLICES: usize = 16;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum FlushMode {
    #[default]
    Immediate,
    Batched,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
#[allow(dead_code)]
pub enum ConnectionKind {
    Control = 0,
    Lane = 1,
    Track = 2,
}

impl TryFrom<u8> for ConnectionKind {
    type Error = io::Error;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Control),
            1 => Ok(Self::Lane),
            2 => Ok(Self::Track),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unknown Vivid connection kind {value}"),
            )),
        }
    }
}

impl ConnectionKind {
    pub const fn required_first_record(self) -> u16 {
        match self {
            Self::Control => crate::messages::HELLO,
            Self::Lane => crate::messages::LANE_OPEN,
            Self::Track => crate::messages::CHANNEL_OPEN,
        }
    }

    pub fn validate_first_record(self, header: &RecordHeader) -> io::Result<()> {
        if header.sequence != 1 || header.record_type != self.required_first_record() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "first record does not match the Vivid connection kind",
            ));
        }
        if self == Self::Control && header.object_id != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "HELLO must use session-level object ID zero",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Preface {
    pub major: u8,
    pub minor: u8,
    pub kind: ConnectionKind,
    pub flags: u8,
    pub initiator_tx_body_limit: u32,
}

impl Preface {
    pub fn decode(bytes: [u8; PREFACE_SIZE]) -> io::Result<Self> {
        if &bytes[0..4] != MAGIC {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid Vivid magic",
            ));
        }
        if bytes[12..16] != [0; 4] {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Vivid preface reserved bytes are nonzero",
            ));
        }
        if bytes[7] != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Vivid preface reserved flags are nonzero",
            ));
        }
        let initiator_tx_body_limit = u32::from_be_bytes(bytes[8..12].try_into().unwrap());
        if initiator_tx_body_limit == 0 || initiator_tx_body_limit > HARD_MAX_RECORD_BODY {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid Vivid maximum record size",
            ));
        }
        Ok(Self {
            major: bytes[4],
            minor: bytes[5],
            kind: ConnectionKind::try_from(bytes[6])?,
            flags: bytes[7],
            initiator_tx_body_limit,
        })
    }

    /// Decode the structurally stable preface and classify only a well-formed version mismatch.
    pub fn classify(bytes: [u8; PREFACE_SIZE]) -> io::Result<PrefaceClassification> {
        let preface = Self::decode(bytes)?;
        if (preface.major, preface.minor) == (VIVID_MAJOR, VIVID_MINOR) {
            Ok(PrefaceClassification::Accepted(preface))
        } else {
            Ok(PrefaceClassification::UnsupportedVersion(preface))
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrefaceClassification {
    Accepted(Preface),
    UnsupportedVersion(Preface),
}

/// Build the one record a receiver may send for a structurally valid version mismatch.
pub fn unsupported_version_record() -> Vec<u8> {
    let body = crate::messages::unsupported_version_error();
    let header = RecordHeader {
        body_length: body.len() as u32,
        record_type: crate::messages::ERROR,
        flags: 0,
        object_id: 0,
        sequence: 1,
    };
    let mut record = Vec::with_capacity(HEADER_SIZE + body.len());
    record.extend_from_slice(&header.encode());
    record.extend_from_slice(&body);
    record
}

/// Validate an accepted preface, emitting one typed rejection only for a version mismatch.
#[cfg(feature = "native")]
pub fn accept_preface<W: Write + ?Sized>(
    bytes: [u8; PREFACE_SIZE],
    writer: &mut W,
) -> io::Result<Preface> {
    match Preface::classify(bytes)? {
        PrefaceClassification::Accepted(preface) => Ok(preface),
        PrefaceClassification::UnsupportedVersion(preface) => {
            writer.write_all(&unsupported_version_record())?;
            writer.flush()?;
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                format!(
                    "unsupported Vivid version {}.{}; local version is {}.{}",
                    preface.major, preface.minor, VIVID_MAJOR, VIVID_MINOR
                ),
            ))
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg(feature = "native")]
pub enum Endpoint {
    Unix(PathBuf),
    Tcp(String),
}

#[cfg(feature = "native")]
struct ConnectedIo {
    reader: ReaderIo,
    writer: Box<dyn Write + Send>,
}

#[cfg(feature = "native")]
enum ReaderIo {
    Tcp(TcpStream),
    #[cfg(unix)]
    Unix(UnixStream),
    Other(Box<dyn Read + Send>),
}

#[cfg(feature = "native")]
impl ReaderIo {
    fn clear_establishment_read_deadline(&self) -> io::Result<()> {
        match self {
            Self::Tcp(stream) => stream.set_read_timeout(None),
            #[cfg(unix)]
            Self::Unix(stream) => stream.set_read_timeout(None),
            Self::Other(_) => Ok(()),
        }
    }
}

#[cfg(feature = "native")]
impl Read for ReaderIo {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Tcp(stream) => stream.read(buffer),
            #[cfg(unix)]
            Self::Unix(stream) => stream.read(buffer),
            Self::Other(reader) => reader.read(buffer),
        }
    }
}

#[cfg(feature = "native")]
impl Endpoint {
    pub fn parse(value: &str) -> io::Result<Self> {
        if let Some(path) = value.strip_prefix("unix:") {
            if path.is_empty() || !Path::new(path).is_absolute() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Unix endpoint is empty or not absolute",
                ));
            }
            return Ok(Self::Unix(PathBuf::from(path)));
        }
        if let Some(address) = value.strip_prefix("tcp:") {
            let parsed = address.parse::<std::net::SocketAddrV4>().map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "native TCP endpoint is not an IPv4 socket address",
                )
            })?;
            if *parsed.ip() != std::net::Ipv4Addr::LOCALHOST || parsed.port() == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "native TCP endpoint must use exact 127.0.0.1 and a nonzero port",
                ));
            }
            return Ok(Self::Tcp(parsed.to_string()));
        }
        if value.contains("://") {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("unsupported Vivid endpoint scheme in {value:?}"),
            ));
        }
        let path = PathBuf::from(value);
        if !path.is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Unix endpoint is not absolute",
            ));
        }
        Ok(Self::Unix(path))
    }

    fn connect(&self) -> io::Result<ConnectedIo> {
        match self {
            Self::Unix(path) => connect_unix(path),
            Self::Tcp(address) => {
                let stream = TcpStream::connect(address)?;
                stream.set_read_timeout(Some(Duration::from_secs(30)))?;
                stream.set_write_timeout(Some(Duration::from_secs(30)))?;
                stream.set_nodelay(true)?;
                let writer = stream.try_clone()?;
                Ok(ConnectedIo {
                    reader: ReaderIo::Tcp(stream),
                    writer: Box::new(writer),
                })
            }
        }
    }
}

#[cfg(all(feature = "native", unix))]
fn connect_unix(path: &Path) -> io::Result<ConnectedIo> {
    let stream = UnixStream::connect(path)?;
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    stream.set_write_timeout(Some(Duration::from_secs(30)))?;
    let writer = stream.try_clone()?;
    Ok(ConnectedIo {
        reader: ReaderIo::Unix(stream),
        writer: Box::new(writer),
    })
}

#[cfg(all(feature = "native", not(unix)))]
fn connect_unix(_path: &Path) -> io::Result<ConnectedIo> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Unix Vivid endpoints are not supported on this platform; named-pipe support is pending",
    ))
}

#[cfg(feature = "native")]
enum WriterIo {
    Live(Box<dyn Write + Send>),
    Trace(File),
    Sink(io::Sink),
}

#[cfg(feature = "native")]
struct WriterState {
    io: WriterIo,
    send_sequence: u64,
    send_body_limit: u32,
    flush_mode: FlushMode,
    unflushed_records: usize,
}

/// Cloneable, sequence-safe half of a Vivid connection.
#[derive(Clone)]
#[cfg(feature = "native")]
pub struct ConnectionWriter {
    state: Arc<Mutex<WriterState>>,
}

#[cfg(feature = "native")]
impl ConnectionWriter {
    pub fn write_record(
        &self,
        record_type: u16,
        flags: u16,
        object_id: u64,
        body: &[u8],
    ) -> io::Result<u64> {
        self.write_record_parts(record_type, flags, object_id, &[body])
    }

    pub fn write_record_parts(
        &self,
        record_type: u16,
        flags: u16,
        object_id: u64,
        parts: &[&[u8]],
    ) -> io::Result<u64> {
        self.write_record_parts_inner(record_type, flags, object_id, parts, false)
    }

    pub fn write_record_checkpoint(
        &self,
        record_type: u16,
        flags: u16,
        object_id: u64,
        body: &[u8],
    ) -> io::Result<u64> {
        self.write_record_parts_inner(record_type, flags, object_id, &[body], true)
    }

    pub fn write_record_parts_checkpoint(
        &self,
        record_type: u16,
        flags: u16,
        object_id: u64,
        parts: &[&[u8]],
    ) -> io::Result<u64> {
        self.write_record_parts_inner(record_type, flags, object_id, parts, true)
    }

    fn write_record_parts_inner(
        &self,
        record_type: u16,
        flags: u16,
        object_id: u64,
        parts: &[&[u8]],
        checkpoint: bool,
    ) -> io::Result<u64> {
        if flags & !RECORD_KNOWN_FLAGS != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "record has nonzero reserved flags",
            ));
        }
        let body_length = parts.iter().try_fold(0_usize, |length, part| {
            length.checked_add(part.len()).ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "record body length overflows")
            })
        })?;
        let body_length = u32::try_from(body_length)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "record body exceeds u32"))?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| io::Error::other("Vivid connection writer lock is poisoned"))?;
        if body_length > state.send_body_limit || body_length > HARD_MAX_RECORD_BODY {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("record body of {body_length} bytes exceeds negotiated maximum"),
            ));
        }
        state.send_sequence = state.send_sequence.checked_add(1).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "record sequence exhausted")
        })?;
        let header = RecordHeader {
            body_length,
            record_type,
            flags,
            object_id,
            sequence: state.send_sequence,
        };
        write_writer_parts(&mut state.io, &header.encode(), parts)?;

        state.unflushed_records = state.unflushed_records.saturating_add(1);
        let must_flush = state.flush_mode == FlushMode::Immediate
            || checkpoint
            || matches!(
                record_type,
                MAX_CHANNEL_DATA | INPUT_REVOKED | INPUT_RESET | PONG | ERROR
            )
            || has_correlated_control_envelope(record_type, parts)
            || state.unflushed_records >= BATCH_RECORD_LIMIT;
        if must_flush {
            flush_writer(&mut state.io)?;
            state.unflushed_records = 0;
        }
        Ok(header.sequence)
    }

    pub fn set_flush_mode(&self, mode: FlushMode) -> io::Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| io::Error::other("Vivid connection writer lock is poisoned"))?;
        if mode == FlushMode::Immediate && state.unflushed_records != 0 {
            flush_writer(&mut state.io)?;
            state.unflushed_records = 0;
        }
        state.flush_mode = mode;
        Ok(())
    }

    pub fn flush(&self) -> io::Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| io::Error::other("Vivid connection writer lock is poisoned"))?;
        flush_writer(&mut state.io)?;
        state.unflushed_records = 0;
        Ok(())
    }

    pub fn set_send_body_limit(&self, maximum: u32) -> io::Result<()> {
        validate_body_limit(maximum)?;
        self.state
            .lock()
            .map_err(|_| io::Error::other("Vivid connection writer lock is poisoned"))?
            .send_body_limit = maximum;
        Ok(())
    }

    fn write_raw_preface(&self, preface: &[u8; PREFACE_SIZE]) -> io::Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| io::Error::other("Vivid connection writer lock is poisoned"))?;
        write_writer(&mut state.io, preface)?;
        flush_writer(&mut state.io)?;
        state.unflushed_records = 0;
        Ok(())
    }
}

/// Blocking receive half of a live Vivid connection.
#[cfg(feature = "native")]
pub struct ConnectionReader {
    io: ReaderIo,
    receive_sequence: u64,
    receive_body_limit: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BorrowedRecord<'a> {
    pub record_type: u16,
    pub flags: u16,
    pub object_id: u64,
    pub sequence: u64,
    pub body: &'a [u8],
}

#[cfg(feature = "native")]
impl ConnectionReader {
    fn clear_establishment_read_deadline(&mut self) -> io::Result<()> {
        self.io.clear_establishment_read_deadline()
    }

    pub fn set_receive_body_limit(&mut self, maximum: u32) -> io::Result<()> {
        validate_body_limit(maximum)?;
        self.receive_body_limit = maximum;
        Ok(())
    }

    pub fn read_record(&mut self) -> io::Result<Record> {
        let mut body = Vec::new();
        let header = self.read_record_body_into(&mut body)?;
        Ok(Record {
            record_type: header.record_type,
            flags: header.flags,
            object_id: header.object_id,
            sequence: header.sequence,
            body,
        })
    }

    pub fn read_record_into<'a>(
        &mut self,
        body: &'a mut Vec<u8>,
    ) -> io::Result<BorrowedRecord<'a>> {
        let header = self.read_record_body_into(body)?;
        Ok(BorrowedRecord {
            record_type: header.record_type,
            flags: header.flags,
            object_id: header.object_id,
            sequence: header.sequence,
            body,
        })
    }

    fn read_record_body_into(&mut self, body: &mut Vec<u8>) -> io::Result<RecordHeader> {
        let mut header = [0_u8; HEADER_SIZE];
        self.io.read_exact(&mut header)?;
        let header = RecordHeader::decode(header);
        if header.flags & !RECORD_KNOWN_FLAGS != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "presenter record has nonzero reserved flags",
            ));
        }
        if header.body_length > self.receive_body_limit || header.body_length > HARD_MAX_RECORD_BODY
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "presenter record exceeds configured maximum",
            ));
        }
        let expected_sequence = self.receive_sequence.checked_add(1).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "presenter sequence exhausted")
        })?;
        if header.sequence != expected_sequence {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "presenter sequence is {}, expected {expected_sequence}",
                    header.sequence
                ),
            ));
        }
        self.receive_sequence = header.sequence;
        body.clear();
        body.resize(header.body_length as usize, 0);
        self.io.read_exact(body.as_mut_slice())?;
        Ok(header)
    }
}

#[cfg(feature = "native")]
pub struct Connection {
    reader: Option<ConnectionReader>,
    writer: ConnectionWriter,
}

#[cfg(feature = "native")]
impl Connection {
    pub fn open(endpoint: &Endpoint, kind: ConnectionKind) -> io::Result<Self> {
        Self::open_version(endpoint, kind, VIVID_MAJOR, VIVID_MINOR)
    }

    /// Open a fresh connection using a version the caller has explicitly selected.
    pub fn open_version(
        endpoint: &Endpoint,
        kind: ConnectionKind,
        major: u8,
        minor: u8,
    ) -> io::Result<Self> {
        let ConnectedIo { reader, writer } = endpoint.connect()?;
        Self::new_version(Some(reader), WriterIo::Live(writer), kind, major, minor)
    }

    /// Start an initiator-side Vivid connection over an already authenticated transport.
    ///
    /// The caller owns transport authentication and peer routing. This constructor still emits
    /// the connection preface and preserves all normal record limits and sequencing, making it
    /// suitable for bindings such as an authenticated WebSocket relay.
    pub fn from_streams(
        reader: Box<dyn Read + Send>,
        writer: Box<dyn Write + Send>,
        kind: ConnectionKind,
    ) -> io::Result<Self> {
        Self::new(Some(ReaderIo::Other(reader)), WriterIo::Live(writer), kind)
    }

    pub fn trace(path: &Path, kind: ConnectionKind) -> io::Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        Self::new(None, WriterIo::Trace(File::create(path)?), kind)
    }

    pub fn sink(kind: ConnectionKind) -> io::Result<Self> {
        Self::new(None, WriterIo::Sink(io::sink()), kind)
    }

    fn new(reader: Option<ReaderIo>, io: WriterIo, kind: ConnectionKind) -> io::Result<Self> {
        Self::new_version(reader, io, kind, VIVID_MAJOR, VIVID_MINOR)
    }

    fn new_version(
        reader: Option<ReaderIo>,
        io: WriterIo,
        kind: ConnectionKind,
        major: u8,
        minor: u8,
    ) -> io::Result<Self> {
        let body_limit = if kind == ConnectionKind::Control {
            super::CONTROL_MAX_RECORD_BODY
        } else {
            DEFAULT_MAX_RECORD_BODY
        };
        let writer = ConnectionWriter {
            state: Arc::new(Mutex::new(WriterState {
                io,
                send_sequence: 0,
                send_body_limit: body_limit,
                flush_mode: FlushMode::Immediate,
                unflushed_records: 0,
            })),
        };
        writer.write_raw_preface(&encode_preface_version(kind, body_limit, major, minor))?;
        Ok(Self {
            reader: reader.map(|io| ConnectionReader {
                io,
                receive_sequence: 0,
                receive_body_limit: body_limit,
            }),
            writer,
        })
    }

    /// Commit a positively established connection to its long-lived reader.
    ///
    /// Native endpoints use a bounded read deadline while waiting for the first positive
    /// handshake response. Once the caller has validated that response, `split` removes that
    /// transport deadline: ordinary protocol idleness is not a framing error or loss signal.
    pub fn split(self) -> io::Result<(ConnectionReader, ConnectionWriter)> {
        let mut reader = self.reader.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::Unsupported,
                "trace connections do not have presenter replies",
            )
        })?;
        reader.clear_establishment_read_deadline()?;
        Ok((reader, self.writer))
    }

    pub fn writer(&self) -> ConnectionWriter {
        self.writer.clone()
    }

    pub fn write_record(
        &mut self,
        record_type: u16,
        flags: u16,
        object_id: u64,
        body: &[u8],
    ) -> io::Result<u64> {
        self.writer
            .write_record(record_type, flags, object_id, body)
    }

    pub fn write_record_parts(
        &mut self,
        record_type: u16,
        flags: u16,
        object_id: u64,
        parts: &[&[u8]],
    ) -> io::Result<u64> {
        self.writer
            .write_record_parts(record_type, flags, object_id, parts)
    }

    pub fn write_record_checkpoint(
        &mut self,
        record_type: u16,
        flags: u16,
        object_id: u64,
        body: &[u8],
    ) -> io::Result<u64> {
        self.writer
            .write_record_checkpoint(record_type, flags, object_id, body)
    }

    pub fn write_record_parts_checkpoint(
        &mut self,
        record_type: u16,
        flags: u16,
        object_id: u64,
        parts: &[&[u8]],
    ) -> io::Result<u64> {
        self.writer
            .write_record_parts_checkpoint(record_type, flags, object_id, parts)
    }

    pub fn set_flush_mode(&mut self, mode: FlushMode) -> io::Result<()> {
        self.writer.set_flush_mode(mode)
    }

    pub fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }

    pub fn set_send_body_limit(&mut self, maximum: u32) -> io::Result<()> {
        self.writer.set_send_body_limit(maximum)
    }

    pub fn set_receive_body_limit(&mut self, maximum: u32) -> io::Result<()> {
        self.reader
            .as_mut()
            .ok_or_else(|| io::Error::new(io::ErrorKind::Unsupported, "connection has no reader"))?
            .set_receive_body_limit(maximum)
    }

    pub fn read_record(&mut self) -> io::Result<Record> {
        self.reader
            .as_mut()
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::Unsupported,
                    "trace connections do not have presenter replies",
                )
            })?
            .read_record()
    }

    pub fn read_record_into<'a>(
        &mut self,
        body: &'a mut Vec<u8>,
    ) -> io::Result<BorrowedRecord<'a>> {
        self.reader
            .as_mut()
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::Unsupported,
                    "trace connections do not have presenter replies",
                )
            })?
            .read_record_into(body)
    }
}

#[cfg(feature = "native")]
fn validate_body_limit(maximum: u32) -> io::Result<()> {
    if maximum == 0 || maximum > HARD_MAX_RECORD_BODY {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid record-body limit",
        ))
    } else {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DirectionalLimits {
    pub peer_limit: u32,
    pub profile_limit: u32,
    pub hard_limit: u32,
}

impl DirectionalLimits {
    pub fn effective(self) -> u32 {
        self.peer_limit.min(self.profile_limit).min(self.hard_limit)
    }
}

#[cfg(feature = "native")]
fn write_writer(io: &mut WriterIo, bytes: &[u8]) -> io::Result<()> {
    match io {
        WriterIo::Live(stream) => stream.write_all(bytes),
        WriterIo::Trace(file) => file.write_all(bytes),
        WriterIo::Sink(sink) => sink.write_all(bytes),
    }
}

#[cfg(feature = "native")]
fn write_writer_parts(io: &mut WriterIo, header: &[u8], parts: &[&[u8]]) -> io::Result<()> {
    let mut part_index = 0_usize;
    let mut part_offset = 0_usize;
    while part_index <= parts.len() {
        while part_index <= parts.len() {
            let part = if part_index == 0 {
                header
            } else {
                parts[part_index - 1]
            };
            if part_offset < part.len() {
                break;
            }
            part_index += 1;
            part_offset = 0;
        }
        if part_index > parts.len() {
            return Ok(());
        }

        let mut slices = [IoSlice::new(&[]); MAX_VECTORED_SLICES];
        let mut slice_count = 0_usize;
        let mut index = part_index;
        while index <= parts.len() && slice_count < slices.len() {
            let part = if index == 0 { header } else { parts[index - 1] };
            let offset = if index == part_index { part_offset } else { 0 };
            if offset < part.len() {
                slices[slice_count] = IoSlice::new(&part[offset..]);
                slice_count += 1;
            }
            index += 1;
        }

        let written = loop {
            let result = match io {
                WriterIo::Live(stream) => stream.write_vectored(&slices[..slice_count]),
                WriterIo::Trace(file) => file.write_vectored(&slices[..slice_count]),
                WriterIo::Sink(sink) => sink.write_vectored(&slices[..slice_count]),
            };
            match result {
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                result => break result?,
            }
        };
        if written == 0 {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "failed to write Vivid record",
            ));
        }

        let mut remaining = written;
        while remaining != 0 {
            let part = if part_index == 0 {
                header
            } else {
                parts[part_index - 1]
            };
            let available = part.len() - part_offset;
            if remaining < available {
                part_offset += remaining;
                remaining = 0;
            } else {
                remaining -= available;
                part_index += 1;
                part_offset = 0;
                while part_index <= parts.len() {
                    let part = if part_index == 0 {
                        header
                    } else {
                        parts[part_index - 1]
                    };
                    if !part.is_empty() {
                        break;
                    }
                    part_index += 1;
                }
            }
        }
    }
    Ok(())
}

#[cfg(feature = "native")]
fn has_correlated_control_envelope(record_type: u16, parts: &[&[u8]]) -> bool {
    if record_type >= 0x8000 {
        return false;
    }
    let mut bytes = parts.iter().flat_map(|part| part.iter().copied());
    let Some(map) = bytes.next() else {
        return false;
    };
    if map >> 5 != 5 || bytes.next() != Some(0) {
        return false;
    }
    let Some(initial) = bytes.next() else {
        return false;
    };
    if initial >> 5 != 0 {
        return false;
    }
    let additional = initial & 0x1f;
    let request_id = match additional {
        value @ 0..=23 => u64::from(value),
        24 => bytes.next().map(u64::from).unwrap_or(0),
        25 => read_control_uint(&mut bytes, 2).unwrap_or(0),
        26 => read_control_uint(&mut bytes, 4).unwrap_or(0),
        27 => read_control_uint(&mut bytes, 8).unwrap_or(0),
        _ => 0,
    };
    request_id != 0
}

#[cfg(feature = "native")]
fn read_control_uint(bytes: &mut impl Iterator<Item = u8>, length: usize) -> Option<u64> {
    (0..length).try_fold(0_u64, |value, _| {
        bytes.next().map(|byte| (value << 8) | u64::from(byte))
    })
}

#[cfg(feature = "native")]
fn flush_writer(io: &mut WriterIo) -> io::Result<()> {
    match io {
        WriterIo::Live(stream) => stream.flush(),
        WriterIo::Trace(file) => file.flush(),
        WriterIo::Sink(sink) => sink.flush(),
    }
}

pub struct Record {
    pub record_type: u16,
    pub flags: u16,
    pub object_id: u64,
    pub sequence: u64,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordHeader {
    pub body_length: u32,
    pub record_type: u16,
    pub flags: u16,
    pub object_id: u64,
    pub sequence: u64,
}

impl RecordHeader {
    pub fn encode(self) -> [u8; HEADER_SIZE] {
        let mut bytes = [0_u8; HEADER_SIZE];
        bytes[0..4].copy_from_slice(&self.body_length.to_be_bytes());
        bytes[4..6].copy_from_slice(&self.record_type.to_be_bytes());
        bytes[6..8].copy_from_slice(&self.flags.to_be_bytes());
        bytes[8..16].copy_from_slice(&self.object_id.to_be_bytes());
        bytes[16..24].copy_from_slice(&self.sequence.to_be_bytes());
        bytes
    }

    pub fn decode(bytes: [u8; HEADER_SIZE]) -> Self {
        Self {
            body_length: u32::from_be_bytes(bytes[0..4].try_into().unwrap()),
            record_type: u16::from_be_bytes(bytes[4..6].try_into().unwrap()),
            flags: u16::from_be_bytes(bytes[6..8].try_into().unwrap()),
            object_id: u64::from_be_bytes(bytes[8..16].try_into().unwrap()),
            sequence: u64::from_be_bytes(bytes[16..24].try_into().unwrap()),
        }
    }
}

pub fn encode_preface(kind: ConnectionKind, maximum: u32) -> [u8; PREFACE_SIZE] {
    encode_preface_version(kind, maximum, VIVID_MAJOR, VIVID_MINOR)
}

pub fn encode_preface_version(
    kind: ConnectionKind,
    maximum: u32,
    major: u8,
    minor: u8,
) -> [u8; PREFACE_SIZE] {
    let mut bytes = [0_u8; PREFACE_SIZE];
    bytes[0..4].copy_from_slice(MAGIC);
    bytes[4] = major;
    bytes[5] = minor;
    bytes[6] = kind as u8;
    bytes[7] = 0;
    bytes[8..12].copy_from_slice(&maximum.to_be_bytes());
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "native")]
    #[derive(Clone)]
    struct SharedBytes(Arc<Mutex<Vec<u8>>>);

    #[cfg(feature = "native")]
    impl Write for SharedBytes {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[cfg(feature = "native")]
    #[derive(Default)]
    struct WriteStats {
        bytes: Vec<u8>,
        flushes: usize,
    }

    #[cfg(feature = "native")]
    #[derive(Clone)]
    struct ShortWriter {
        stats: Arc<Mutex<WriteStats>>,
        maximum: usize,
    }

    #[cfg(feature = "native")]
    impl Write for ShortWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            let length = bytes.len().min(self.maximum);
            self.stats
                .lock()
                .unwrap()
                .bytes
                .extend_from_slice(&bytes[..length]);
            Ok(length)
        }

        fn write_vectored(&mut self, slices: &[IoSlice<'_>]) -> io::Result<usize> {
            let mut remaining = self.maximum;
            let mut stats = self.stats.lock().unwrap();
            let before = stats.bytes.len();
            for slice in slices {
                let length = slice.len().min(remaining);
                stats.bytes.extend_from_slice(&slice[..length]);
                remaining -= length;
                if remaining == 0 {
                    break;
                }
            }
            Ok(stats.bytes.len() - before)
        }

        fn flush(&mut self) -> io::Result<()> {
            self.stats.lock().unwrap().flushes += 1;
            Ok(())
        }
    }

    #[cfg(feature = "native")]
    fn test_writer(writer: impl Write + Send + 'static, flush_mode: FlushMode) -> ConnectionWriter {
        ConnectionWriter {
            state: Arc::new(Mutex::new(WriterState {
                io: WriterIo::Live(Box::new(writer)),
                send_sequence: 0,
                send_body_limit: 1024,
                flush_mode,
                unflushed_records: 0,
            })),
        }
    }

    #[test]
    fn preface_matches_vivid_layout() {
        let preface = encode_preface(ConnectionKind::Track, 0x0102_0304);
        assert_eq!(&preface[0..4], b"VIVD");
        assert_eq!(preface[4..8], [1, 5, 2, 0]);
        assert_eq!(preface[8..12], [1, 2, 3, 4]);
        assert_eq!(preface[12..16], [0; 4]);
        assert_eq!(
            Preface::decode(preface).unwrap().kind,
            ConnectionKind::Track
        );
    }

    #[test]
    fn connection_kind_registry_is_collision_free_and_contiguous() {
        let kinds = [
            ConnectionKind::Control as u8,
            ConnectionKind::Lane as u8,
            ConnectionKind::Track as u8,
        ];
        assert_eq!(kinds, [0, 1, 2]);
        for (expected, value) in kinds.into_iter().enumerate() {
            assert_eq!(ConnectionKind::try_from(value).unwrap() as usize, expected);
        }
    }

    #[test]
    fn rejects_invalid_prefaces() {
        let mut preface = encode_preface(ConnectionKind::Control, 1024);
        preface[0] = b'X';
        assert!(Preface::decode(preface).is_err());

        let mut preface = encode_preface(ConnectionKind::Control, 1024);
        preface[15] = 1;
        assert!(Preface::decode(preface).is_err());

        let mut preface = encode_preface(ConnectionKind::Control, 1024);
        preface[7] = 1;
        assert!(Preface::decode(preface).is_err());
    }

    #[cfg(feature = "native")]
    #[test]
    fn version_mismatch_emits_one_typed_error_but_malformed_magic_is_silent() {
        let mut mismatched = encode_preface(ConnectionKind::Control, 4096);
        mismatched[5] = VIVID_MINOR.wrapping_add(1);
        let mut output = Vec::new();
        let error = accept_preface(mismatched, &mut output).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Unsupported);
        let header = RecordHeader::decode(output[..HEADER_SIZE].try_into().unwrap());
        assert_eq!(header.record_type, crate::messages::ERROR);
        assert_eq!(header.object_id, 0);
        assert_eq!(header.sequence, 1);
        assert_eq!(output.len(), HEADER_SIZE + header.body_length as usize);
        let reply = crate::messages::parse_error_reply(&output[HEADER_SIZE..]).unwrap();
        assert_eq!(reply.code, crate::messages::ERROR_UNSUPPORTED_VERSION);
        assert!(reply.fatal);
        assert_eq!(
            reply.detail.supported_version_tuple(),
            Some((u64::from(VIVID_MAJOR), u64::from(VIVID_MINOR)))
        );

        let mut malformed = encode_preface(ConnectionKind::Control, 4096);
        malformed[0] = b'X';
        let mut silent = Vec::new();
        assert!(accept_preface(malformed, &mut silent).is_err());
        assert!(silent.is_empty());
    }

    #[cfg(feature = "native")]
    fn assert_stream_version_rejection<S>(mut initiator: S, mut receiver: S)
    where
        S: Read + Write + Send + 'static,
    {
        let server = std::thread::spawn(move || {
            let mut preface = [0; PREFACE_SIZE];
            receiver.read_exact(&mut preface).unwrap();
            let error = accept_preface(preface, &mut receiver).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::Unsupported);
        });
        let mut preface = encode_preface(ConnectionKind::Control, 4096);
        preface[5] = VIVID_MINOR.wrapping_sub(1);
        initiator.write_all(&preface).unwrap();
        let mut received = Vec::new();
        initiator.read_to_end(&mut received).unwrap();
        server.join().unwrap();
        assert_eq!(received, unsupported_version_record());
    }

    #[cfg(all(feature = "native", unix))]
    #[test]
    fn unix_transport_emits_exactly_one_typed_version_rejection_then_closes() {
        let (initiator, receiver) = std::os::unix::net::UnixStream::pair().unwrap();
        assert_stream_version_rejection(initiator, receiver);
    }

    #[cfg(feature = "native")]
    #[test]
    fn loopback_tcp_and_ssh_forward_transport_reject_versions_identically() {
        for _transport in ["loopback TCP", "SSH-forwarded loopback TCP"] {
            let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
            let address = listener.local_addr().unwrap();
            let initiator = std::net::TcpStream::connect(address).unwrap();
            let (receiver, _) = listener.accept().unwrap();
            assert_stream_version_rejection(initiator, receiver);
        }
    }

    #[test]
    #[cfg(feature = "native")]
    fn connection_rejects_reserved_record_flags() {
        let mut connection = Connection::sink(ConnectionKind::Control).unwrap();
        assert!(connection.write_record(1, 2, 0, &[]).is_err());
    }

    #[test]
    #[cfg(feature = "native")]
    fn connection_from_streams_emits_the_normal_preface() {
        let bytes = Arc::new(Mutex::new(Vec::new()));
        let _connection = Connection::from_streams(
            Box::new(io::empty()),
            Box::new(SharedBytes(bytes.clone())),
            ConnectionKind::Track,
        )
        .unwrap();
        assert_eq!(
            bytes.lock().unwrap().as_slice(),
            encode_preface(ConnectionKind::Track, DEFAULT_MAX_RECORD_BODY)
        );
    }

    #[test]
    #[cfg(all(feature = "native", unix))]
    fn split_clears_the_unix_establishment_read_deadline() {
        let (stream, _peer) = UnixStream::pair().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(30)))
            .unwrap();
        let connection = Connection::new_version(
            Some(ReaderIo::Unix(stream)),
            WriterIo::Sink(io::sink()),
            ConnectionKind::Control,
            VIVID_MAJOR,
            VIVID_MINOR,
        )
        .unwrap();

        let (reader, _writer) = connection.split().unwrap();
        let ReaderIo::Unix(stream) = reader.io else {
            panic!("native Unix connection lost its concrete reader");
        };
        assert_eq!(stream.read_timeout().unwrap(), None);
    }

    #[test]
    #[cfg(all(feature = "native", not(unix)))]
    fn split_clears_the_tcp_establishment_read_deadline() {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let stream = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (_peer, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(30)))
            .unwrap();
        let connection = Connection::new_version(
            Some(ReaderIo::Tcp(stream)),
            WriterIo::Sink(io::sink()),
            ConnectionKind::Control,
            VIVID_MAJOR,
            VIVID_MINOR,
        )
        .unwrap();

        let (reader, _writer) = connection.split().unwrap();
        let ReaderIo::Tcp(stream) = reader.io else {
            panic!("native TCP connection lost its concrete reader");
        };
        assert_eq!(stream.read_timeout().unwrap(), None);
    }

    #[test]
    #[cfg(feature = "native")]
    fn cloned_writers_serialize_complete_records_and_sequences() {
        let bytes = Arc::new(Mutex::new(Vec::new()));
        let writer = ConnectionWriter {
            state: Arc::new(Mutex::new(WriterState {
                io: WriterIo::Live(Box::new(SharedBytes(bytes.clone()))),
                send_sequence: 0,
                send_body_limit: 16,
                flush_mode: FlushMode::Immediate,
                unflushed_records: 0,
            })),
        };
        let first = writer.clone();
        let second = writer.clone();
        let one = std::thread::spawn(move || first.write_record(0x10, 0, 1, &[0xaa]));
        let two = std::thread::spawn(move || second.write_record(0x20, 0, 2, &[0xbb]));
        one.join().unwrap().unwrap();
        two.join().unwrap().unwrap();

        let bytes = bytes.lock().unwrap();
        assert_eq!(bytes.len(), 2 * (HEADER_SIZE + 1));
        let first = RecordHeader::decode(bytes[..HEADER_SIZE].try_into().unwrap());
        let second_offset = HEADER_SIZE + usize::try_from(first.body_length).unwrap();
        let second = RecordHeader::decode(
            bytes[second_offset..second_offset + HEADER_SIZE]
                .try_into()
                .unwrap(),
        );
        assert_eq!((first.sequence, second.sequence), (1, 2));
        assert_eq!(first.body_length, 1);
        assert_eq!(second.body_length, 1);
        assert_eq!(
            bytes[HEADER_SIZE],
            if first.record_type == 0x10 {
                0xaa
            } else {
                0xbb
            }
        );
        assert_eq!(
            bytes[second_offset + HEADER_SIZE],
            if second.record_type == 0x10 {
                0xaa
            } else {
                0xbb
            }
        );
    }

    #[test]
    #[cfg(feature = "native")]
    fn vectored_records_match_contiguous_records_under_short_writes() {
        let vectored_stats = Arc::new(Mutex::new(WriteStats::default()));
        let vectored = test_writer(
            ShortWriter {
                stats: vectored_stats.clone(),
                maximum: 3,
            },
            FlushMode::Immediate,
        );
        assert_eq!(
            vectored
                .write_record_parts(0x8001, 0, 9, &[b"ab", b"", b"cdef"])
                .unwrap(),
            1
        );

        let contiguous_stats = Arc::new(Mutex::new(WriteStats::default()));
        let contiguous = test_writer(
            ShortWriter {
                stats: contiguous_stats.clone(),
                maximum: usize::MAX,
            },
            FlushMode::Immediate,
        );
        assert_eq!(contiguous.write_record(0x8001, 0, 9, b"abcdef").unwrap(), 1);
        assert_eq!(
            vectored_stats.lock().unwrap().bytes,
            contiguous_stats.lock().unwrap().bytes
        );
    }

    #[test]
    #[cfg(feature = "native")]
    fn returned_sequences_match_wire_headers_and_restart_per_writer() {
        let stats = Arc::new(Mutex::new(WriteStats::default()));
        let writer = test_writer(
            ShortWriter {
                stats: stats.clone(),
                maximum: usize::MAX,
            },
            FlushMode::Immediate,
        );
        assert_eq!(writer.write_record(1, 0, 0, b"a").unwrap(), 1);
        assert_eq!(writer.write_record(1, 0, 0, b"bb").unwrap(), 2);
        assert_eq!(writer.write_record(1, 0, 0, b"ccc").unwrap(), 3);

        let bytes = &stats.lock().unwrap().bytes;
        let mut offset = 0;
        for expected in 1..=3 {
            let header =
                RecordHeader::decode(bytes[offset..offset + HEADER_SIZE].try_into().unwrap());
            assert_eq!(header.sequence, expected);
            offset += HEADER_SIZE + header.body_length as usize;
        }

        let other = test_writer(io::sink(), FlushMode::Immediate);
        assert_eq!(other.write_record(1, 0, 0, &[]).unwrap(), 1);
    }

    #[test]
    #[cfg(feature = "native")]
    fn batched_mode_flushes_flow_updates_correlated_records_and_bounded_batches() {
        let stats = Arc::new(Mutex::new(WriteStats::default()));
        let writer = test_writer(
            ShortWriter {
                stats: stats.clone(),
                maximum: usize::MAX,
            },
            FlushMode::Batched,
        );

        writer.write_record(0x8001, 0, 1, b"media").unwrap();
        assert_eq!(stats.lock().unwrap().flushes, 0);
        writer.write_record(MAX_CHANNEL_DATA, 0, 1, &[]).unwrap();
        assert_eq!(stats.lock().unwrap().flushes, 1);

        writer
            .write_record(crate::messages::OK, 0, 0, &crate::messages::ok(7))
            .unwrap();
        assert_eq!(stats.lock().unwrap().flushes, 2);

        for _ in 0..BATCH_RECORD_LIMIT {
            writer.write_record(0x8001, 0, 1, &[]).unwrap();
        }
        assert_eq!(stats.lock().unwrap().flushes, 3);
    }

    #[test]
    #[cfg(feature = "native")]
    fn read_record_into_reuses_capacity_and_rejects_oversize_before_resize() {
        let first = RecordHeader {
            body_length: 32,
            record_type: 1,
            flags: 0,
            object_id: 2,
            sequence: 1,
        };
        let second = RecordHeader {
            sequence: 2,
            ..first
        };
        let mut input = Vec::new();
        input.extend_from_slice(&first.encode());
        input.extend_from_slice(&[1; 32]);
        input.extend_from_slice(&second.encode());
        input.extend_from_slice(&[2; 32]);
        let mut reader = ConnectionReader {
            io: ReaderIo::Other(Box::new(io::Cursor::new(input))),
            receive_sequence: 0,
            receive_body_limit: 32,
        };
        let mut body = Vec::new();
        assert_eq!(reader.read_record_into(&mut body).unwrap().body, &[1; 32]);
        let capacity = body.capacity();
        assert!(capacity >= 32);
        assert_eq!(reader.read_record_into(&mut body).unwrap().body, &[2; 32]);
        assert_eq!(body.capacity(), capacity);

        let oversized = RecordHeader {
            body_length: 33,
            record_type: 1,
            flags: 0,
            object_id: 0,
            sequence: 1,
        };
        let mut reader = ConnectionReader {
            io: ReaderIo::Other(Box::new(io::Cursor::new(oversized.encode()))),
            receive_sequence: 0,
            receive_body_limit: 32,
        };
        let mut untouched = vec![9; 8];
        let capacity = untouched.capacity();
        assert!(reader.read_record_into(&mut untouched).is_err());
        assert_eq!(untouched, vec![9; 8]);
        assert_eq!(untouched.capacity(), capacity);
    }

    #[test]
    fn record_header_round_trip() {
        let header = RecordHeader {
            body_length: 17,
            record_type: 0x8001,
            flags: 2,
            object_id: 42,
            sequence: 9,
        };
        assert_eq!(RecordHeader::decode(header.encode()), header);
    }

    #[cfg(all(feature = "native", unix))]
    #[test]
    fn endpoint_parser_accepts_explicit_and_bare_unix_paths() {
        assert_eq!(
            Endpoint::parse("unix:/tmp/vivid.sock").unwrap(),
            Endpoint::Unix(PathBuf::from("/tmp/vivid.sock"))
        );
        assert_eq!(
            Endpoint::parse("/tmp/vivid.sock").unwrap(),
            Endpoint::Unix(PathBuf::from("/tmp/vivid.sock"))
        );
    }

    #[test]
    #[cfg(feature = "native")]
    fn endpoint_parser_accepts_tcp() {
        assert_eq!(
            Endpoint::parse("tcp:127.0.0.1:12345").unwrap(),
            Endpoint::Tcp("127.0.0.1:12345".into())
        );
    }
}
