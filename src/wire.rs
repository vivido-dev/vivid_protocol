use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::{DEFAULT_MAX_RECORD_BODY, FRAMING_MAJOR, FRAMING_MINOR, HARD_MAX_RECORD_BODY};

pub const PREFACE_SIZE: usize = 16;
pub const HEADER_SIZE: usize = 24;
const MAGIC: &[u8; 4] = b"VIVD";

pub const RECORD_OPTIONAL: u16 = 1 << 0;
pub const RECORD_KNOWN_FLAGS: u16 = RECORD_OPTIONAL;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
#[allow(dead_code)]
pub enum ConnectionKind {
    Control = 0,
    Video = 1,
    Raster = 2,
    Blob = 3,
    LocalBuffer = 4,
    Audio = 5,
}

impl TryFrom<u8> for ConnectionKind {
    type Error = io::Error;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Control),
            1 => Ok(Self::Video),
            2 => Ok(Self::Raster),
            3 => Ok(Self::Blob),
            4 => Ok(Self::LocalBuffer),
            5 => Ok(Self::Audio),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unknown Vivid connection kind {value}"),
            )),
        }
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Endpoint {
    Unix(PathBuf),
    Tcp(String),
}

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
            if address.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "empty TCP endpoint",
                ));
            }
            return Ok(Self::Tcp(address.to_owned()));
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

    fn connect(&self) -> io::Result<(Box<dyn Read + Send>, Box<dyn Write + Send>)> {
        match self {
            Self::Unix(path) => connect_unix(path),
            Self::Tcp(address) => {
                let stream = TcpStream::connect(address)?;
                stream.set_read_timeout(Some(Duration::from_secs(30)))?;
                stream.set_write_timeout(Some(Duration::from_secs(30)))?;
                stream.set_nodelay(true)?;
                let writer = stream.try_clone()?;
                Ok((Box::new(stream), Box::new(writer)))
            }
        }
    }
}

#[cfg(unix)]
fn connect_unix(path: &Path) -> io::Result<(Box<dyn Read + Send>, Box<dyn Write + Send>)> {
    use std::os::unix::net::UnixStream;

    let stream = UnixStream::connect(path)?;
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    stream.set_write_timeout(Some(Duration::from_secs(30)))?;
    let writer = stream.try_clone()?;
    Ok((Box::new(stream), Box::new(writer)))
}

#[cfg(not(unix))]
fn connect_unix(_path: &Path) -> io::Result<(Box<dyn Read + Send>, Box<dyn Write + Send>)> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Unix Vivid endpoints are not supported on this platform; named-pipe support is pending",
    ))
}

enum WriterIo {
    Live(Box<dyn Write + Send>),
    Trace(File),
    Sink(io::Sink),
}

struct WriterState {
    io: WriterIo,
    send_sequence: u64,
    send_body_limit: u32,
}

/// Cloneable, sequence-safe half of a Vivid connection.
#[derive(Clone)]
pub struct ConnectionWriter {
    state: Arc<Mutex<WriterState>>,
}

impl ConnectionWriter {
    pub fn write_record(
        &self,
        record_type: u16,
        flags: u16,
        object_id: u64,
        body: &[u8],
    ) -> io::Result<()> {
        if flags & !RECORD_KNOWN_FLAGS != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "record has nonzero reserved flags",
            ));
        }
        let body_length = u32::try_from(body.len())
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
        write_writer(&mut state.io, &header.encode())?;
        write_writer(&mut state.io, body)?;
        flush_writer(&mut state.io)
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
        flush_writer(&mut state.io)
    }
}

/// Blocking receive half of a live Vivid connection.
pub struct ConnectionReader {
    io: Box<dyn Read + Send>,
    receive_sequence: u64,
    receive_body_limit: u32,
}

impl ConnectionReader {
    pub fn set_receive_body_limit(&mut self, maximum: u32) -> io::Result<()> {
        validate_body_limit(maximum)?;
        self.receive_body_limit = maximum;
        Ok(())
    }

    pub fn read_record(&mut self) -> io::Result<Record> {
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
        let mut body = vec![0; header.body_length as usize];
        self.io.read_exact(&mut body)?;
        Ok(Record {
            record_type: header.record_type,
            flags: header.flags,
            object_id: header.object_id,
            sequence: header.sequence,
            body,
        })
    }
}

pub struct Connection {
    reader: Option<ConnectionReader>,
    writer: ConnectionWriter,
}

impl Connection {
    pub fn open(endpoint: &Endpoint, kind: ConnectionKind) -> io::Result<Self> {
        let (reader, writer) = endpoint.connect()?;
        Self::new(Some(reader), WriterIo::Live(writer), kind)
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

    fn new(
        reader: Option<Box<dyn Read + Send>>,
        io: WriterIo,
        kind: ConnectionKind,
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
            })),
        };
        writer.write_raw_preface(&encode_preface(kind, DEFAULT_MAX_RECORD_BODY))?;
        Ok(Self {
            reader: reader.map(|io| ConnectionReader {
                io,
                receive_sequence: 0,
                receive_body_limit: body_limit,
            }),
            writer,
        })
    }

    pub fn split(self) -> io::Result<(ConnectionReader, ConnectionWriter)> {
        let reader = self.reader.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::Unsupported,
                "trace connections do not have presenter replies",
            )
        })?;
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
    ) -> io::Result<()> {
        self.writer
            .write_record(record_type, flags, object_id, body)
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
}

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

fn write_writer(io: &mut WriterIo, bytes: &[u8]) -> io::Result<()> {
    match io {
        WriterIo::Live(stream) => stream.write_all(bytes),
        WriterIo::Trace(file) => file.write_all(bytes),
        WriterIo::Sink(sink) => sink.write_all(bytes),
    }
}

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
    let mut bytes = [0_u8; PREFACE_SIZE];
    bytes[0..4].copy_from_slice(MAGIC);
    bytes[4] = FRAMING_MAJOR;
    bytes[5] = FRAMING_MINOR;
    bytes[6] = kind as u8;
    bytes[7] = 0;
    bytes[8..12].copy_from_slice(&maximum.to_be_bytes());
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    struct SharedBytes(Arc<Mutex<Vec<u8>>>);

    impl Write for SharedBytes {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn preface_matches_protocol_layout() {
        let preface = encode_preface(ConnectionKind::Video, 0x0102_0304);
        assert_eq!(&preface[0..4], b"VIVD");
        assert_eq!(preface[4..8], [1, 0, 1, 0]);
        assert_eq!(preface[8..12], [1, 2, 3, 4]);
        assert_eq!(preface[12..16], [0; 4]);
        assert_eq!(
            Preface::decode(preface).unwrap().kind,
            ConnectionKind::Video
        );
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

    #[test]
    fn connection_rejects_reserved_record_flags() {
        let mut connection = Connection::sink(ConnectionKind::Control).unwrap();
        assert!(connection.write_record(1, 2, 0, &[]).is_err());
    }

    #[test]
    fn cloned_writers_serialize_complete_records_and_sequences() {
        let bytes = Arc::new(Mutex::new(Vec::new()));
        let writer = ConnectionWriter {
            state: Arc::new(Mutex::new(WriterState {
                io: WriterIo::Live(Box::new(SharedBytes(bytes.clone()))),
                send_sequence: 0,
                send_body_limit: 16,
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

    #[cfg(unix)]
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
    fn endpoint_parser_accepts_tcp() {
        assert_eq!(
            Endpoint::parse("tcp:127.0.0.1:12345").unwrap(),
            Endpoint::Tcp("127.0.0.1:12345".into())
        );
    }
}
