//! Public-API tests for `vivid_protocol::trace`.
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;
use vivid_protocol::trace::*;
use vivid_protocol::{messages, time::Monotonic};

fn source(local_session_hint: [u8; 16]) -> TraceSource {
    TraceSource {
        component: TraceComponent::PROTOCOL,
        hop: TraceHop::Local,
        local_session_hint,
    }
}

fn event(direction: TraceDirection, record_type: u16, sequence: u64) -> TraceEvent {
    TraceEvent {
        direction,
        record_type,
        body_length: 0,
        connection_sequence: sequence,
        object_kind: TraceObjectKind::Session,
        object_id: None,
        request_id: None,
        causation_id: None,
        outcome: TraceOutcome::State,
    }
}

#[test]
fn component_names_are_checked() {
    assert_eq!(TraceComponent::new("vvmux").name(), "vvmux");
    for name in [
        "",
        "Vivido",
        "has space",
        "quote\"",
        "a-name-that-is-longer-than-32-bytes",
    ] {
        TraceComponent::try_new(name).unwrap_err();
    }
}

#[test]
fn trace_line_has_the_common_vocabulary_and_no_free_form_body() {
    let body = messages::Hello {
        producer_name: "private title".into(),
        producer_version: "1".into(),
        required_profiles: vec![
            vivid_protocol::registry::TERMINAL_SURFACE.into(),
            vivid_protocol::registry::CORE_CONTROL.into(),
        ],
        optional_profiles: vec![],
        maximum_control_body: vivid_protocol::CONTROL_MAX_RECORD_BODY,
        client_nonce: [1; 32],
        authentication: messages::HelloAuthentication::Root { proof: [2; 32] },
        target_profile: vivid_protocol::registry::TERMINAL_SURFACE.into(),
        extensions: vec![],
    }
    .encode(7)
    .unwrap();
    let records = Arc::new(Mutex::new(Vec::new()));
    let output = Arc::clone(&records);
    let guard = TraceGuard::callback(
        source([0x5a; 16]),
        TraceClock::new(|| Monotonic::from_micros(42)),
        move |record| output.lock().unwrap().push(record),
    )
    .unwrap();
    guard
        .emitter()
        .emit_control(event(TraceDirection::Send, messages::HELLO, 1), &body);
    drop(guard);
    let line = records.lock().unwrap()[0].ndjson_line();
    for required in [
        "\"version\":1",
        "\"monotonic_ts_us\":42",
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
    let output = Arc::clone(&records);
    let guard = TraceGuard::callback(source([1; 16]), TraceClock::process(), move |record| {
        std::thread::sleep(Duration::from_millis(1));
        output.lock().unwrap().push(record);
    })
    .unwrap();
    let emitter = guard.emitter();
    for sequence in 0..1024 {
        emitter.emit(event(TraceDirection::Local, 1, sequence));
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
    let callback_slot = Arc::clone(&guard_slot);
    let (done, completed) = std::sync::mpsc::sync_channel(1);
    let guard = TraceGuard::callback(source([2; 16]), TraceClock::process(), move |_| {
        drop(callback_slot.lock().unwrap().take());
        done.send(()).unwrap();
    })
    .unwrap();
    let emitter = guard.emitter();
    *guard_slot.lock().unwrap() = Some(guard);
    emitter.emit(event(TraceDirection::Local, 1, 1));
    completed.recv_timeout(Duration::from_secs(1)).unwrap();
}
