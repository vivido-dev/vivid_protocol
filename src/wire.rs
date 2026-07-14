use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::{DEFAULT_MAX_RECORD_BODY, HARD_MAX_RECORD_BODY, PROTOCOL_MAJOR, PROTOCOL_MINOR};

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
    pub maximum_record_body: u32,
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
        let maximum_record_body = u32::from_be_bytes(bytes[8..12].try_into().unwrap());
        if maximum_record_body == 0 || maximum_record_body > HARD_MAX_RECORD_BODY {
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
            maximum_record_body,
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
            if path.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "empty Unix endpoint",
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
        Ok(Self::Unix(PathBuf::from(value)))
    }

    fn connect(&self) -> io::Result<Box<dyn ReadWrite>> {
        match self {
            Self::Unix(path) => connect_unix(path),
            Self::Tcp(address) => {
                let stream = TcpStream::connect(address)?;
                stream.set_read_timeout(Some(Duration::from_secs(30)))?;
                stream.set_write_timeout(Some(Duration::from_secs(30)))?;
                stream.set_nodelay(true)?;
                Ok(Box::new(stream))
            }
        }
    }
}

trait ReadWrite: Read + Write {}
impl<T: Read + Write> ReadWrite for T {}

#[cfg(unix)]
fn connect_unix(path: &Path) -> io::Result<Box<dyn ReadWrite>> {
    use std::os::unix::net::UnixStream;

    let stream = UnixStream::connect(path)?;
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    stream.set_write_timeout(Some(Duration::from_secs(30)))?;
    Ok(Box::new(stream))
}

#[cfg(not(unix))]
fn connect_unix(_path: &Path) -> io::Result<Box<dyn ReadWrite>> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Unix Vivid endpoints are not supported on this platform; named-pipe support is pending",
    ))
}

enum ConnectionIo {
    Live(Box<dyn ReadWrite>),
    Trace(File),
    Sink(io::Sink),
}

pub struct Connection {
    io: ConnectionIo,
    send_sequence: u64,
    receive_sequence: u64,
    max_record_body: u32,
}

impl Connection {
    pub fn open(endpoint: &Endpoint, kind: ConnectionKind) -> io::Result<Self> {
        let stream = endpoint.connect()?;
        Self::new(ConnectionIo::Live(stream), kind)
    }

    pub fn trace(path: &Path, kind: ConnectionKind) -> io::Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let file = File::create(path)?;
        Self::new(ConnectionIo::Trace(file), kind)
    }

    pub fn sink(kind: ConnectionKind) -> io::Result<Self> {
        Self::new(ConnectionIo::Sink(io::sink()), kind)
    }

    fn new(mut io: ConnectionIo, kind: ConnectionKind) -> io::Result<Self> {
        write_io(&mut io, &encode_preface(kind, DEFAULT_MAX_RECORD_BODY))?;
        flush_io(&mut io)?;
        Ok(Self {
            io,
            send_sequence: 0,
            receive_sequence: 0,
            max_record_body: DEFAULT_MAX_RECORD_BODY,
        })
    }

    pub fn write_record(
        &mut self,
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
        if body_length > self.max_record_body || body_length > HARD_MAX_RECORD_BODY {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("record body of {body_length} bytes exceeds negotiated maximum"),
            ));
        }

        self.send_sequence = self.send_sequence.checked_add(1).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "record sequence exhausted")
        })?;
        let header = RecordHeader {
            body_length,
            record_type,
            flags,
            object_id,
            sequence: self.send_sequence,
        };
        write_io(&mut self.io, &header.encode())?;
        write_io(&mut self.io, body)?;
        flush_io(&mut self.io)
    }

    pub fn read_record(&mut self) -> io::Result<Record> {
        let ConnectionIo::Live(stream) = &mut self.io else {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "trace connections do not have presenter replies",
            ));
        };

        let mut header = [0_u8; HEADER_SIZE];
        stream.read_exact(&mut header)?;
        let header = RecordHeader::decode(header);
        if header.flags & !RECORD_KNOWN_FLAGS != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "presenter record has nonzero reserved flags",
            ));
        }
        if header.body_length > self.max_record_body || header.body_length > HARD_MAX_RECORD_BODY {
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
        stream.read_exact(&mut body)?;
        Ok(Record {
            record_type: header.record_type,
            flags: header.flags,
            object_id: header.object_id,
            sequence: header.sequence,
            body,
        })
    }
}

fn write_io(io: &mut ConnectionIo, bytes: &[u8]) -> io::Result<()> {
    match io {
        ConnectionIo::Live(stream) => stream.write_all(bytes),
        ConnectionIo::Trace(file) => file.write_all(bytes),
        ConnectionIo::Sink(sink) => sink.write_all(bytes),
    }
}

fn flush_io(io: &mut ConnectionIo) -> io::Result<()> {
    match io {
        ConnectionIo::Live(stream) => stream.flush(),
        ConnectionIo::Trace(file) => file.flush(),
        ConnectionIo::Sink(sink) => sink.flush(),
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
    bytes[4] = PROTOCOL_MAJOR;
    bytes[5] = PROTOCOL_MINOR;
    bytes[6] = kind as u8;
    bytes[7] = 0;
    bytes[8..12].copy_from_slice(&maximum.to_be_bytes());
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
