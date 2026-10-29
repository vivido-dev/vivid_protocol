//! The error binary-layout codecs return.

use std::backtrace::{Backtrace, BacktraceStatus};
use std::borrow::Cow;
use std::error::Error;
use std::fmt;
use std::io;

/// Why a codec rejected its input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    /// Received bytes or values do not satisfy their layout.
    Malformed,
    /// A caller-supplied value cannot be encoded.
    InvalidArgument,
    /// The layout is valid but this build cannot process it.
    Unsupported,
}

/// A layout or value that does not satisfy its Vivid 1.5 definition.
///
/// Returned by the media, audio-input, file-data, anchor-marker, and connection-framing codecs.
/// Ask which side is at fault with [`is_malformed`](Self::is_malformed) (the input), or
/// [`is_invalid_argument`](Self::is_invalid_argument) (the caller's values), and whether this build
/// lacks support with [`is_unsupported`](Self::is_unsupported).
///
/// Converting into [`io::Error`] keeps the kind as [`io::ErrorKind::InvalidData`],
/// [`io::ErrorKind::InvalidInput`], or [`io::ErrorKind::Unsupported`], so `?` works in functions
/// that return `io::Result`.
///
/// A backtrace is captured when `RUST_BACKTRACE` or `RUST_LIB_BACKTRACE` enables one; otherwise
/// capturing costs a few instructions.
#[derive(Debug)]
pub struct CodecError {
    kind: Kind,
    message: Cow<'static, str>,
    source: Option<Box<dyn Error + Send + Sync + 'static>>,
    backtrace: Backtrace,
}

impl CodecError {
    pub(crate) fn new(kind: Kind, message: impl Into<Cow<'static, str>>) -> Self {
        Self {
            kind,
            message: message.into(),
            source: None,
            backtrace: Backtrace::capture(),
        }
    }

    pub(crate) fn malformed(message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(Kind::Malformed, message)
    }

    pub(crate) fn invalid_argument(message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(Kind::InvalidArgument, message)
    }

    #[cfg_attr(
        all(feature = "native", not(target_arch = "wasm32")),
        expect(
            dead_code,
            reason = "only builds without native zstd report unsupported compression"
        )
    )]
    pub(crate) fn unsupported(message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(Kind::Unsupported, message)
    }

    pub(crate) fn with_source(mut self, source: impl Into<Box<dyn Error + Send + Sync>>) -> Self {
        self.source = Some(source.into());
        self
    }

    /// Whether received bytes or values do not satisfy their layout.
    pub fn is_malformed(&self) -> bool {
        self.kind == Kind::Malformed
    }

    /// Whether a caller-supplied value cannot be encoded.
    pub fn is_invalid_argument(&self) -> bool {
        self.kind == Kind::InvalidArgument
    }

    /// Whether the input is valid but this build cannot process it.
    ///
    /// Compressed rasters report this without the `native` feature, for example.
    pub fn is_unsupported(&self) -> bool {
        self.kind == Kind::Unsupported
    }

    /// The summary of what was rejected, without the source or backtrace.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// The backtrace captured when the error was created; usually disabled.
    pub fn backtrace(&self) -> &Backtrace {
        &self.backtrace
    }
}

impl fmt::Display for CodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)?;
        if let Some(source) = &self.source {
            write!(formatter, ": {source}")?;
        }
        if self.backtrace.status() == BacktraceStatus::Captured {
            write!(formatter, "\n\nbacktrace:\n{}", self.backtrace)?;
        }
        Ok(())
    }
}

impl Error for CodecError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source
            .as_deref()
            .map(|source| source as &(dyn Error + 'static))
    }
}

impl From<CodecError> for io::Error {
    fn from(error: CodecError) -> Self {
        let kind = match error.kind {
            Kind::Malformed => io::ErrorKind::InvalidData,
            Kind::InvalidArgument => io::ErrorKind::InvalidInput,
            Kind::Unsupported => io::ErrorKind::Unsupported,
        };
        io::Error::new(kind, error)
    }
}
