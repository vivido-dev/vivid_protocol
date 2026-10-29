//! Bounded, metadata-only diagnostics shared by Vivid components.
//!
//! Trace records deliberately cannot carry a control or media body. Callers submit only validated
//! transport metadata to a bounded queue; formatting and file or callback delivery happen on a
//! separate worker so diagnostics cannot delay the data plane.
//!
//! A [`TraceGuard`] owns that worker. Build one with [`TraceGuard::file`], which writes private
//! NDJSON, or [`TraceGuard::callback`], which hands each [`TraceRecord`] to your code and makes
//! tracing testable without touching the filesystem. Each guard names its component and hop with a
//! [`TraceSource`] and timestamps records with a [`TraceClock`], so tests can supply their own
//! time. [`TraceEmitter`] handles, cloned from the guard, submit [`TraceEvent`]s and never block.

use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::thread::{self, JoinHandle};
use std::time::Instant;

use crate::{messages, time::Monotonic};

/// Records the trace worker may have queued before new ones are counted as dropped.
///
/// Emitting never blocks the data plane, so a slow file or callback sheds records instead. 256
/// absorbs a burst of control traffic; a sustained overload shows up as a `dropped` record.
const TRACE_QUEUE_CAPACITY: usize = 256;
/// Schema version written into every NDJSON line; change it when a field changes meaning.
const TRACE_VERSION: u64 = 1;

/// The component that emits trace records, such as `"vivido"` or `"vvmux"`.
///
/// Names are 1 to 32 bytes of lowercase ASCII letters, digits, `_`, and `-`, so no name can break
/// the NDJSON encoding. Products name themselves; this crate does not keep a list of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TraceComponent(&'static str);

impl TraceComponent {
    /// This crate itself, which [`Connection::trace`](crate::wire::Connection::trace) uses.
    pub const PROTOCOL: Self = Self("vivid_protocol");

    /// Names a component, panicking when `name` is not a valid component name.
    ///
    /// In a `const` item the check happens at compile time.
    ///
    /// # Panics
    ///
    /// Panics when [`try_new`](Self::try_new) would return an error.
    pub const fn new(name: &'static str) -> Self {
        match Self::try_new(name) {
            Ok(component) => component,
            Err(_) => panic!("trace component names are 1 to 32 bytes of [a-z0-9_-]"),
        }
    }

    /// Names a component.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidTraceName`] when `name` is empty, longer than 32 bytes, or contains a byte
    /// other than a lowercase ASCII letter, a digit, `_`, or `-`.
    pub const fn try_new(name: &'static str) -> Result<Self, InvalidTraceName> {
        let bytes = name.as_bytes();
        if bytes.is_empty() || bytes.len() > 32 {
            return Err(InvalidTraceName(()));
        }
        let mut index = 0;
        while index < bytes.len() {
            if !matches!(bytes[index], b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-') {
                return Err(InvalidTraceName(()));
            }
            index += 1;
        }
        Ok(Self(name))
    }

    /// The component name as it appears in trace records.
    pub const fn name(self) -> &'static str {
        self.0
    }
}

impl fmt::Display for TraceComponent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}

/// A trace component name outside `[a-z0-9_-]{1,32}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidTraceName(());

impl fmt::Display for InvalidTraceName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "trace component names are 1 to 32 bytes of lowercase ASCII letters, digits, '_' or '-'",
        )
    }
}

impl std::error::Error for InvalidTraceName {}

/// Where on the path between producer and presenter a trace record was taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TraceHop {
    /// At a producer.
    Producer,
    /// At a terminating presenter.
    Presenter,
    /// On a gateway's inner session.
    Inner,
    /// On a gateway's outer session.
    Outer,
    /// At a byte-transparent relay.
    Relay,
    /// In a browser binding.
    Browser,
    /// Inside one process, with no network hop.
    Local,
}

impl TraceHop {
    const fn name(self) -> &'static str {
        match self {
            Self::Producer => "producer",
            Self::Presenter => "presenter",
            Self::Inner => "inner",
            Self::Outer => "outer",
            Self::Relay => "relay",
            Self::Browser => "browser",
            Self::Local => "local",
        }
    }
}

/// Which way a traced record moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TraceDirection {
    /// The component sent it.
    Send,
    /// The component received it.
    Receive,
    /// A gateway mapped it from one session to another.
    Map,
    /// A local event with no record on the wire.
    Local,
}

impl TraceDirection {
    const fn name(self) -> &'static str {
        match self {
            Self::Send => "send",
            Self::Receive => "receive",
            Self::Map => "map",
            Self::Local => "local",
        }
    }
}

/// The kind of object a traced record names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TraceObjectKind {
    /// The session itself; object ID zero.
    Session,
    /// A surface.
    Surface,
    /// A track.
    Track,
    /// A scene node.
    Node,
    /// A scene transaction.
    Transaction,
    /// A context.
    Context,
    /// A session lease.
    Lease,
    /// A terminal anchor.
    Anchor,
    /// A connection.
    Connection,
    /// The trace stream itself, such as a drop report.
    Trace,
}

impl TraceObjectKind {
    const fn name(self) -> &'static str {
        match self {
            Self::Session => "session",
            Self::Surface => "surface",
            Self::Track => "track",
            Self::Node => "node",
            Self::Transaction => "transaction",
            Self::Context => "context",
            Self::Lease => "lease",
            Self::Anchor => "anchor",
            Self::Connection => "connection",
            Self::Trace => "trace",
        }
    }
}

/// How handling of a traced record ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TraceOutcome {
    /// It succeeded.
    Ok,
    /// It failed.
    Error,
    /// The trace queue overflowed and records were lost.
    Dropped,
    /// Policy withheld the object's identity from the trace.
    Restricted,
    /// A state transition rather than a record result.
    State,
}

impl TraceOutcome {
    const fn name(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Error => "error",
            Self::Dropped => "dropped",
            Self::Restricted => "restricted",
            Self::State => "state",
        }
    }
}

/// One Vivid trace-format-v1 record.
///
/// Every textual value is a closed registry value and all byte arrays are random correlation
/// values rendered as hexadecimal. There is intentionally no free-form diagnostic or body field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TraceRecord {
    /// Microseconds on the guard's [`TraceClock`].
    pub monotonic_ts_us: u64,
    /// The component that emitted the record.
    pub component: TraceComponent,
    /// Where on the path it was taken.
    pub hop: TraceHop,
    /// Which way the record moved.
    pub direction: TraceDirection,
    /// The traced record's type.
    pub record_type: u16,
    /// The traced record's body length; the body itself is never traced.
    pub body_length: u64,
    /// The traced record's connection sequence.
    pub connection_sequence: u64,
    /// The kind of object the record names.
    pub object_kind: TraceObjectKind,
    /// The object's local ID, unless policy withheld it.
    pub object_id: Option<u64>,
    /// The envelope's request ID, when nonzero.
    pub request_id: Option<u64>,
    /// The envelope's causation ID, when present.
    pub causation_id: Option<[u8; messages::CAUSATION_ID_BYTES]>,
    /// Hop-local session hint; never a session tag or key.
    pub local_session_hint: [u8; 16],
    /// How handling ended.
    pub outcome: TraceOutcome,
}

impl TraceRecord {
    /// Deterministic newline-delimited JSON. No caller-provided string can enter this encoding.
    pub fn ndjson_line(&self) -> String {
        let object_id = optional_u64(self.object_id);
        let request_id = optional_u64(self.request_id);
        let causation_id = self
            .causation_id
            .map_or_else(|| "null".into(), |value| format!("\"{}\"", hex(&value)));
        format!(
            concat!(
                "{{\"version\":{},\"monotonic_ts_us\":{},",
                "\"clock_domain\":\"process_monotonic\",\"component\":\"{}\",",
                "\"hop\":\"{}\",\"direction\":\"{}\",\"record_type\":{},",
                "\"body_length\":{},\"connection_sequence\":{},",
                "\"object_kind\":\"{}\",\"object_id\":{},\"request_id\":{},",
                "\"causation_id\":{},\"local_session_hint\":\"{}\",",
                "\"outcome\":\"{}\"}}\n"
            ),
            TRACE_VERSION,
            self.monotonic_ts_us,
            self.component.name(),
            self.hop.name(),
            self.direction.name(),
            self.record_type,
            self.body_length,
            self.connection_sequence,
            self.object_kind.name(),
            object_id,
            request_id,
            causation_id,
            hex(&self.local_session_hint),
            self.outcome.name(),
        )
    }
}

/// Who emits a guard's records: the component, its hop, and a per-session correlation hint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TraceSource {
    /// The emitting component.
    pub component: TraceComponent,
    /// Where the component sits on the path between producer and presenter.
    pub hop: TraceHop,
    /// A random value that correlates one session's records without naming the session.
    pub local_session_hint: [u8; 16],
}

/// The clock that timestamps a guard's records.
///
/// Every timestamp is a [`Monotonic`] value from one origin, as everywhere else in this crate.
/// [`TraceClock::process`] reads the process's monotonic clock; [`TraceClock::new`] takes any
/// source, such as a test's fake clock. NDJSON files label every timestamp with the
/// `process_monotonic` clock domain, so a custom clock should count from a process-local origin.
#[derive(Clone)]
pub struct TraceClock(Arc<dyn Fn() -> Monotonic + Send + Sync>);

impl TraceClock {
    /// Timestamps records with `now`, which must not go backwards.
    pub fn new(now: impl Fn() -> Monotonic + Send + Sync + 'static) -> Self {
        Self(Arc::new(now))
    }

    /// Timestamps records with microseconds of the process monotonic clock since this call.
    pub fn process() -> Self {
        let origin = Instant::now();
        Self::new(move || {
            Monotonic::from_micros(u64::try_from(origin.elapsed().as_micros()).unwrap_or(u64::MAX))
        })
    }

    fn now(&self) -> Monotonic {
        (self.0)()
    }
}

impl fmt::Debug for TraceClock {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("TraceClock").finish_non_exhaustive()
    }
}

/// The metadata of one record a component sent, received, mapped, or noted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TraceEvent {
    /// Whether the record was sent, received, mapped across a hop, or is local state.
    pub direction: TraceDirection,
    /// The record type from the record header.
    pub record_type: u16,
    /// The record body length in bytes; the body itself is never traced.
    pub body_length: u64,
    /// The record's sequence number on its connection.
    pub connection_sequence: u64,
    /// What kind of object the record addresses.
    pub object_kind: TraceObjectKind,
    /// The addressed object, when tracing it is permitted.
    pub object_id: Option<u64>,
    /// The control request ID, for correlated control records.
    pub request_id: Option<u64>,
    /// The causation ID a control envelope carries, if any.
    pub causation_id: Option<[u8; messages::CAUSATION_ID_BYTES]>,
    /// How handling the record turned out.
    pub outcome: TraceOutcome,
}

struct TraceContext {
    clock: TraceClock,
    source: TraceSource,
}

impl TraceContext {
    fn record(&self, event: TraceEvent) -> TraceRecord {
        TraceRecord {
            monotonic_ts_us: self.clock.now().as_micros(),
            component: self.source.component,
            hop: self.source.hop,
            direction: event.direction,
            record_type: event.record_type,
            body_length: event.body_length,
            connection_sequence: event.connection_sequence,
            object_kind: event.object_kind,
            object_id: event.object_id,
            request_id: event.request_id,
            causation_id: event.causation_id,
            local_session_hint: self.source.local_session_hint,
            outcome: event.outcome,
        }
    }
}

enum TraceMessage {
    Record(TraceRecord),
    Shutdown,
}

/// Cloneable, nonblocking trace submission handle.
#[derive(Clone)]
pub struct TraceEmitter {
    context: Arc<TraceContext>,
    sender: SyncSender<TraceMessage>,
    dropped: Arc<AtomicU64>,
}

impl fmt::Debug for TraceEmitter {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TraceEmitter")
            .field("component", &self.context.source.component)
            .field("hop", &self.context.source.hop)
            .field("dropped", &self.dropped.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

impl TraceEmitter {
    /// Queues `event`, or counts it as dropped when the worker is behind; never blocks.
    pub fn emit(&self, event: TraceEvent) {
        let record = self.context.record(event);
        match self.sender.try_send(TraceMessage::Record(record)) {
            Ok(()) | Err(TrySendError::Disconnected(_)) => {}
            Err(TrySendError::Full(_)) => {
                let _ = self
                    .dropped
                    .try_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                        Some(value.saturating_add(1))
                    });
            }
        }
    }

    /// Emits `event` for the control record `body`.
    ///
    /// The length, request ID, and causation ID come from the body itself.
    ///
    /// Only those public envelope fields are read; nothing else in `body` reaches the trace.
    pub fn emit_control(&self, event: TraceEvent, body: &[u8]) {
        let (request_id, causation_id) = control_correlation(body);
        self.emit(TraceEvent {
            body_length: u64::try_from(body.len()).unwrap_or(u64::MAX),
            request_id,
            causation_id,
            ..event
        });
    }

    /// Emit a policy-restricted track state without its hop-local track ID.
    pub fn emit_restricted_track(
        &self,
        direction: TraceDirection,
        record_type: u16,
        body_length: u64,
        connection_sequence: u64,
    ) {
        self.emit(TraceEvent {
            direction,
            record_type,
            body_length,
            connection_sequence,
            object_kind: TraceObjectKind::Track,
            object_id: None,
            request_id: None,
            causation_id: None,
            outcome: TraceOutcome::Restricted,
        });
    }
}

/// Owns the bounded trace worker. Dropping the guard drains queued records and joins the worker.
pub struct TraceGuard {
    emitter: TraceEmitter,
    join: Option<JoinHandle<()>>,
    callback_shutdown: Arc<AtomicBool>,
}

impl fmt::Debug for TraceGuard {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TraceGuard")
            .field("emitter", &self.emitter)
            .field("worker_running", &self.join.is_some())
            .finish_non_exhaustive()
    }
}

impl TraceGuard {
    /// Starts a trace worker that hands records to `callback`.
    ///
    /// The worker runs on a background thread, names records with `source`, and timestamps them
    /// with `clock`.
    ///
    /// When the queue overflows, the worker reports the loss as a [`TraceOutcome::Dropped`]
    /// record before the next one.
    ///
    /// # Errors
    ///
    /// Returns any error from spawning the worker thread.
    pub fn callback(
        source: TraceSource,
        clock: TraceClock,
        mut callback: impl FnMut(TraceRecord) + Send + 'static,
    ) -> io::Result<Self> {
        let (sender, receiver) = mpsc::sync_channel(TRACE_QUEUE_CAPACITY);
        let dropped = Arc::new(AtomicU64::new(0));
        let callback_shutdown = Arc::new(AtomicBool::new(false));
        let context = Arc::new(TraceContext { clock, source });
        let worker_context = Arc::clone(&context);
        let worker_dropped = Arc::clone(&dropped);
        let worker_callback_shutdown = Arc::clone(&callback_shutdown);
        let join = thread::Builder::new()
            .name("vivid-trace".into())
            .spawn(move || {
                while let Ok(message) = receiver.recv() {
                    let TraceMessage::Record(record) = message else {
                        break;
                    };
                    let lost = worker_dropped.swap(0, Ordering::Relaxed);
                    if lost != 0 {
                        callback(worker_context.record(TraceEvent {
                            direction: TraceDirection::Local,
                            record_type: 0,
                            body_length: 0,
                            connection_sequence: 0,
                            object_kind: TraceObjectKind::Trace,
                            object_id: None,
                            request_id: None,
                            causation_id: None,
                            outcome: TraceOutcome::Dropped,
                        }));
                    }
                    callback(record);
                    if worker_callback_shutdown.load(Ordering::Acquire) {
                        break;
                    }
                }
            })?;
        Ok(Self {
            emitter: TraceEmitter {
                context,
                sender,
                dropped,
            },
            join: Some(join),
            callback_shutdown,
        })
    }

    /// Starts a trace worker that appends records to a private NDJSON file.
    ///
    /// The worker creates a new file at `path`, names records with `source`, and timestamps them
    /// with `clock`.
    ///
    /// The file is created with `create_new`, so an existing file or symlink is never followed or
    /// truncated; on Unix it gets mode `0600`. Write errors drop the affected lines.
    ///
    /// # Errors
    ///
    /// Returns [`io::ErrorKind::AlreadyExists`] when `path` exists, and any error creating its
    /// parent directory, the file, or the worker thread.
    pub fn file(path: &Path, source: TraceSource, clock: TraceClock) -> io::Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        // Never follow or truncate an existing trace path, including a symlink.
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600)
        };
        let mut writer = BufWriter::new(options.open(path)?);
        Self::callback(source, clock, move |record| {
            let _ = writer.write_all(record.ndjson_line().as_bytes());
        })
    }

    /// A new submission handle for this guard's worker.
    pub fn emitter(&self) -> TraceEmitter {
        self.emitter.clone()
    }
}

impl Drop for TraceGuard {
    fn drop(&mut self) {
        if self
            .join
            .as_ref()
            .is_some_and(|join| join.thread().id() == thread::current().id())
        {
            self.callback_shutdown.store(true, Ordering::Release);
            self.join.take();
            return;
        }
        let _ = self.emitter.sender.send(TraceMessage::Shutdown);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

/// Returns the request and causation IDs of a control `body`.
///
/// Both are `None` when the body does not decode.
pub fn control_correlation(
    body: &[u8],
) -> (Option<u64>, Option<[u8; messages::CAUSATION_ID_BYTES]>) {
    messages::decode_control(body).map_or((None, None), |envelope| {
        let envelope = zeroize::Zeroizing::new(envelope);
        (
            (envelope.request_id != 0).then_some(envelope.request_id),
            envelope.causation_id,
        )
    })
}

/// Returns the kind of object a record of `record_type` names with `object_id`.
pub fn object_kind(record_type: u16, object_id: u64) -> TraceObjectKind {
    if object_id == 0 {
        return TraceObjectKind::Session;
    }
    match record_type {
        messages::CREATE_NODE | messages::UPDATE_NODE | messages::DELETE_NODE => {
            TraceObjectKind::Node
        }
        messages::BEGIN_TXN | messages::COMMIT_TXN | messages::ABORT_TXN => {
            TraceObjectKind::Transaction
        }
        messages::CREATE_CONTEXT | messages::REVOKE_CONTEXT | messages::CONTEXT_CHANGED => {
            TraceObjectKind::Context
        }
        messages::CREATE_SESSION_LEASE
        | messages::SESSION_LEASE_READY
        | messages::REVOKE_SESSION_LEASE
        | messages::SESSION_LEASE_CHANGED => TraceObjectKind::Lease,
        messages::ANCHOR_READY | messages::ANCHOR_GONE | messages::ANCHOR_STATUS => {
            TraceObjectKind::Anchor
        }
        messages::CREATE_SURFACE
        | messages::UPDATE_SURFACE
        | messages::DESTROY_SURFACE
        | messages::QUERY_SURFACE
        | messages::SURFACE_STATUS
        | messages::SURFACE_CHANGED
        | messages::SET_INPUT_BINDING
        | messages::INPUT_BOUND
        | messages::INPUT_REVOKED
        | messages::INPUT_LEASE_RENEW
        | messages::INPUT_RESET
        | messages::KEY_INPUT
        | messages::POINTER_MOTION
        | messages::POINTER_BUTTON
        | messages::POINTER_AXIS => TraceObjectKind::Surface,
        _ => TraceObjectKind::Track,
    }
}

fn optional_u64(value: Option<u64>) -> String {
    value.map_or_else(|| "null".into(), |value| value.to_string())
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(DIGITS[usize::from(byte >> 4)]));
        encoded.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    encoded
}
