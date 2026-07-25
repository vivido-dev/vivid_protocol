//! Bounded, metadata-only diagnostics shared by Vivid components.
//!
//! Trace records deliberately cannot carry a control or media body. Callers submit only validated
//! transport metadata to a bounded queue; formatting and file or callback delivery happen on a
//! separate worker so diagnostics cannot delay the data plane.

use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::thread::{self, JoinHandle};
use std::time::Instant;

use crate::messages;

const TRACE_QUEUE_CAPACITY: usize = 256;
const TRACE_VERSION: u64 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceComponent {
    Protocol,
    Sdk,
    Vivido,
    Vvmux,
    Vvbridge,
    Vivi,
    Vvrd,
    Veston,
    Vvsway,
}

impl TraceComponent {
    const fn name(self) -> &'static str {
        match self {
            Self::Protocol => "vivid_protocol",
            Self::Sdk => "vivid_sdk",
            Self::Vivido => "vivido",
            Self::Vvmux => "vvmux",
            Self::Vvbridge => "vvbridge",
            Self::Vivi => "vivi",
            Self::Vvrd => "vvrd",
            Self::Veston => "veston",
            Self::Vvsway => "vvsway",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceHop {
    Producer,
    Presenter,
    Inner,
    Outer,
    Relay,
    Browser,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceDirection {
    Send,
    Receive,
    Map,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceObjectKind {
    Session,
    Source,
    Node,
    Context,
    Anchor,
    Connection,
    Trace,
}

impl TraceObjectKind {
    const fn name(self) -> &'static str {
        match self {
            Self::Session => "session",
            Self::Source => "source",
            Self::Node => "node",
            Self::Context => "context",
            Self::Anchor => "anchor",
            Self::Connection => "connection",
            Self::Trace => "trace",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceOutcome {
    Ok,
    Error,
    Dropped,
    Restricted,
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
    pub monotonic_ts_us: u64,
    pub component: TraceComponent,
    pub hop: TraceHop,
    pub direction: TraceDirection,
    pub record_type: u16,
    pub body_length: u64,
    pub connection_sequence: u64,
    pub object_kind: TraceObjectKind,
    pub object_id: Option<u64>,
    pub request_id: Option<u64>,
    pub causation_id: Option<[u8; messages::CAUSATION_ID_BYTES]>,
    pub local_session_hint: [u8; 16],
    pub outcome: TraceOutcome,
}

impl TraceRecord {
    /// Deterministic newline-delimited JSON. No caller-provided string can enter this encoding.
    pub fn ndjson_line(&self) -> String {
        let object_id = optional_u64(self.object_id);
        let request_id = optional_u64(self.request_id);
        let causation_id = self
            .causation_id
            .map(|value| format!("\"{}\"", hex(&value)))
            .unwrap_or_else(|| "null".into());
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

#[derive(Debug)]
struct TraceContext {
    start: Instant,
    component: TraceComponent,
    hop: TraceHop,
    local_session_hint: [u8; 16],
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

impl TraceEmitter {
    #[allow(clippy::too_many_arguments)]
    pub fn emit(
        &self,
        direction: TraceDirection,
        record_type: u16,
        body_length: u64,
        connection_sequence: u64,
        object_kind: TraceObjectKind,
        object_id: Option<u64>,
        request_id: Option<u64>,
        causation_id: Option<[u8; messages::CAUSATION_ID_BYTES]>,
        outcome: TraceOutcome,
    ) {
        let record = TraceRecord {
            monotonic_ts_us: u64::try_from(self.context.start.elapsed().as_micros())
                .unwrap_or(u64::MAX),
            component: self.context.component,
            hop: self.context.hop,
            direction,
            record_type,
            body_length,
            connection_sequence,
            object_kind,
            object_id,
            request_id,
            causation_id,
            local_session_hint: self.context.local_session_hint,
            outcome,
        };
        match self.sender.try_send(TraceMessage::Record(record)) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                let _ = self
                    .dropped
                    .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                        Some(value.saturating_add(1))
                    });
            }
            Err(TrySendError::Disconnected(_)) => {}
        }
    }

    /// Emit metadata for a control body after extracting only its public request and causation IDs.
    #[allow(clippy::too_many_arguments)]
    pub fn emit_control(
        &self,
        direction: TraceDirection,
        record_type: u16,
        body: &[u8],
        connection_sequence: u64,
        object_kind: TraceObjectKind,
        object_id: Option<u64>,
        outcome: TraceOutcome,
    ) {
        let (request_id, causation_id) = control_correlation(body);
        self.emit(
            direction,
            record_type,
            u64::try_from(body.len()).unwrap_or(u64::MAX),
            connection_sequence,
            object_kind,
            object_id,
            request_id,
            causation_id,
            outcome,
        );
    }

    /// Emit a policy-restricted source state without its hop-local source ID.
    pub fn emit_restricted_source(
        &self,
        direction: TraceDirection,
        record_type: u16,
        body_length: u64,
        connection_sequence: u64,
    ) {
        self.emit(
            direction,
            record_type,
            body_length,
            connection_sequence,
            TraceObjectKind::Source,
            None,
            None,
            None,
            TraceOutcome::Restricted,
        );
    }
}

/// Owns the bounded trace worker. Dropping the guard drains queued records and joins the worker.
pub struct TraceGuard {
    emitter: TraceEmitter,
    join: Option<JoinHandle<()>>,
    callback_shutdown: Arc<AtomicBool>,
}

impl TraceGuard {
    pub fn callback(
        component: TraceComponent,
        hop: TraceHop,
        local_session_hint: [u8; 16],
        mut callback: impl FnMut(TraceRecord) + Send + 'static,
    ) -> io::Result<Self> {
        let (sender, receiver) = mpsc::sync_channel(TRACE_QUEUE_CAPACITY);
        let dropped = Arc::new(AtomicU64::new(0));
        let callback_shutdown = Arc::new(AtomicBool::new(false));
        let context = Arc::new(TraceContext {
            start: Instant::now(),
            component,
            hop,
            local_session_hint,
        });
        let worker_context = context.clone();
        let worker_dropped = dropped.clone();
        let worker_callback_shutdown = callback_shutdown.clone();
        let join = thread::Builder::new()
            .name("vivid-trace".into())
            .spawn(move || {
                while let Ok(message) = receiver.recv() {
                    let TraceMessage::Record(record) = message else {
                        break;
                    };
                    let lost = worker_dropped.swap(0, Ordering::Relaxed);
                    if lost != 0 {
                        callback(TraceRecord {
                            monotonic_ts_us: u64::try_from(
                                worker_context.start.elapsed().as_micros(),
                            )
                            .unwrap_or(u64::MAX),
                            component: worker_context.component,
                            hop: worker_context.hop,
                            direction: TraceDirection::Local,
                            record_type: 0,
                            body_length: 0,
                            connection_sequence: 0,
                            object_kind: TraceObjectKind::Trace,
                            object_id: None,
                            request_id: None,
                            causation_id: None,
                            local_session_hint: worker_context.local_session_hint,
                            outcome: TraceOutcome::Dropped,
                        });
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

    pub fn file(
        path: &Path,
        component: TraceComponent,
        hop: TraceHop,
        local_session_hint: [u8; 16],
    ) -> io::Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut writer = BufWriter::new(File::create(path)?);
        Self::callback(component, hop, local_session_hint, move |record| {
            let _ = writer.write_all(record.ndjson_line().as_bytes());
        })
    }

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

pub fn control_correlation(
    body: &[u8],
) -> (Option<u64>, Option<[u8; messages::CAUSATION_ID_BYTES]>) {
    messages::decode_control(body)
        .map(|envelope| {
            (
                (envelope.request_id != 0).then_some(envelope.request_id),
                envelope.causation_id,
            )
        })
        .unwrap_or((None, None))
}

pub fn object_kind(record_type: u16, object_id: u64) -> TraceObjectKind {
    if object_id == 0 {
        return TraceObjectKind::Session;
    }
    match record_type {
        messages::CREATE_NODE | messages::UPDATE_NODE | messages::DELETE_NODE => {
            TraceObjectKind::Node
        }
        messages::CREATE_CONTEXT
        | messages::REVOKE_CONTEXT
        | messages::CONTEXT_CAPABILITY
        | messages::CONTEXT_CHANGED => TraceObjectKind::Context,
        messages::ANCHOR_READY | messages::ANCHOR_GONE | messages::ANCHOR_STATUS => {
            TraceObjectKind::Anchor
        }
        _ => TraceObjectKind::Source,
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

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::time::Duration;

    use super::*;

    #[test]
    fn trace_line_has_the_common_vocabulary_and_no_free_form_body() {
        let body = messages::encode_hello(
            7,
            &messages::HelloConfig {
                minimum_major: 1,
                minimum_minor: 1,
                maximum_major: 1,
                maximum_minor: 1,
                token: "secret-token",
                producer: "private title",
                producer_version: "1",
                required_features: &[],
                optional_features: &[],
                maximum_record_body: crate::CONTROL_MAX_RECORD_BODY,
                authentication_kind: messages::AUTHENTICATION_WINDOW_ROOT,
                preserved_fields: &[],
            },
        );
        let records = Arc::new(Mutex::new(Vec::new()));
        let output = records.clone();
        let guard = TraceGuard::callback(
            TraceComponent::Protocol,
            TraceHop::Local,
            [0x5a; 16],
            move |record| output.lock().unwrap().push(record),
        )
        .unwrap();
        guard.emitter().emit_control(
            TraceDirection::Send,
            messages::HELLO,
            &body,
            1,
            TraceObjectKind::Session,
            None,
            TraceOutcome::Ok,
        );
        drop(guard);
        let line = records.lock().unwrap()[0].ndjson_line();
        for required in [
            "\"version\":1",
            "\"monotonic_ts_us\"",
            "\"clock_domain\":\"process_monotonic\"",
            "\"component\":\"vivid_protocol\"",
            "\"direction\":\"send\"",
            "\"request_id\":7",
            "\"local_session_hint\"",
        ] {
            assert!(line.contains(required), "{required}");
        }
        for forbidden in [
            "secret-token",
            "private title",
            "endpoint",
            "cookie",
            "descriptor",
            "command",
        ] {
            assert!(!line.contains(forbidden), "{forbidden}");
        }
    }

    #[test]
    fn full_queue_is_nonblocking_and_reports_loss() {
        let records = Arc::new(Mutex::new(Vec::new()));
        let output = records.clone();
        let guard = TraceGuard::callback(
            TraceComponent::Protocol,
            TraceHop::Local,
            [1; 16],
            move |record| {
                std::thread::sleep(Duration::from_millis(1));
                output.lock().unwrap().push(record);
            },
        )
        .unwrap();
        let emitter = guard.emitter();
        for sequence in 0..1024 {
            emitter.emit(
                TraceDirection::Local,
                1,
                0,
                sequence,
                TraceObjectKind::Session,
                None,
                None,
                None,
                TraceOutcome::State,
            );
        }
        drop(guard);
        assert!(
            records
                .lock()
                .unwrap()
                .iter()
                .any(|record| record.outcome == TraceOutcome::Dropped)
        );
    }

    #[test]
    fn callback_can_disable_its_own_trace_without_deadlocking() {
        let guard_slot = Arc::new(Mutex::new(None::<TraceGuard>));
        let callback_slot = guard_slot.clone();
        let (done, completed) = std::sync::mpsc::sync_channel(1);
        let guard = TraceGuard::callback(
            TraceComponent::Protocol,
            TraceHop::Local,
            [2; 16],
            move |_| {
                drop(callback_slot.lock().unwrap().take());
                done.send(()).unwrap();
            },
        )
        .unwrap();
        let emitter = guard.emitter();
        *guard_slot.lock().unwrap() = Some(guard);
        emitter.emit(
            TraceDirection::Local,
            1,
            0,
            1,
            TraceObjectKind::Session,
            None,
            None,
            None,
            TraceOutcome::State,
        );
        completed.recv_timeout(Duration::from_secs(1)).unwrap();
    }
}
