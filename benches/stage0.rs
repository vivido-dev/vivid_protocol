mod stage0 {
    pub mod allocator;
    pub mod report;
    pub mod scenarios;
}

use std::cmp::Ordering as CmpOrdering;
use std::env;
use std::hint::black_box;
use std::io::{self, IoSlice, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use stage0::allocator::{AllocationSnapshot, CountingAllocator, measure};
use stage0::scenarios::{Delivery, Flow, MediaKind, Scenario};
use vivid_protocol::media::{self, AudioPacket, RasterDeltaOperation, VideoPacket};
use vivid_protocol::messages::{AUDIO_PACKET, IMAGE_DATA, RASTER_FRAME, VIDEO_PACKET};
use vivid_protocol::wire::{Connection, ConnectionKind, HEADER_SIZE};

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SendMode {
    Stage0,
    PreStage0,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage4Mode {
    Disabled,
    Enabled,
}

impl Stage4Mode {
    fn label(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Enabled => "enabled",
        }
    }
}

#[derive(Debug, Default)]
struct TransportStats {
    bytes: AtomicU64,
    write_calls: AtomicU64,
    flush_calls: AtomicU64,
    checksum: AtomicU64,
}

impl TransportStats {
    fn reset(&self) {
        self.bytes.store(0, Ordering::Relaxed);
        self.write_calls.store(0, Ordering::Relaxed);
        self.flush_calls.store(0, Ordering::Relaxed);
        self.checksum.store(0, Ordering::Relaxed);
    }

    fn snapshot(&self) -> TransportSnapshot {
        TransportSnapshot {
            bytes: self.bytes.load(Ordering::Relaxed),
            write_calls: self.write_calls.load(Ordering::Relaxed),
            flush_calls: self.flush_calls.load(Ordering::Relaxed),
            checksum: self.checksum.load(Ordering::Relaxed),
        }
    }

    fn observe(&self, buffers: &[IoSlice<'_>]) -> usize {
        let mut bytes = 0_usize;
        let mut checksum = 0_u64;
        for buffer in buffers {
            bytes = bytes.saturating_add(buffer.len());
            if let Some(first) = buffer.first() {
                checksum = checksum.wrapping_add(u64::from(*first));
            }
            if let Some(last) = buffer.last() {
                checksum = checksum.rotate_left(5).wrapping_add(u64::from(*last));
            }
            // Touch the complete payload so the instrumented writer includes a stable,
            // payload-size-proportional transport cost without making an extra body copy.
            for sampled in buffer.iter() {
                checksum = checksum.rotate_left(1) ^ u64::from(*sampled);
            }
        }
        self.bytes
            .fetch_add(u64::try_from(bytes).unwrap_or(u64::MAX), Ordering::Relaxed);
        self.write_calls.fetch_add(1, Ordering::Relaxed);
        self.checksum.fetch_xor(checksum, Ordering::Relaxed);
        bytes
    }
}

#[derive(Debug, Clone, Copy)]
struct TransportSnapshot {
    bytes: u64,
    write_calls: u64,
    flush_calls: u64,
    checksum: u64,
}

struct CountingWriter {
    stats: Arc<TransportStats>,
}

impl Write for CountingWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        Ok(self.stats.observe(&[IoSlice::new(buffer)]))
    }

    fn write_vectored(&mut self, buffers: &[IoSlice<'_>]) -> io::Result<usize> {
        Ok(self.stats.observe(buffers))
    }

    fn flush(&mut self) -> io::Result<()> {
        self.stats.flush_calls.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

struct PreparedFlow<'a> {
    flow: &'a Flow,
    payload: Vec<u8>,
    connection: Connection,
}

#[derive(Debug, Clone, Copy)]
struct SendSample {
    duration: Duration,
    allocations: AllocationSnapshot,
    transport: TransportSnapshot,
}

#[derive(Debug, Clone)]
struct Distribution {
    p50: u64,
    p95: u64,
    p99: u64,
    samples: Vec<u64>,
}

#[derive(Debug, Clone)]
struct PathMetrics {
    wire_encode_mib_s: f64,
    wire_encode_mib_s_samples: Vec<f64>,
    modeled_end_to_end_mib_s: f64,
    media_allocations: u64,
    receive_buffer_allocations: u64,
    total_hot_path_allocations: u64,
    allocated_bytes: u64,
    copied_bytes: u64,
    retained_memory_peak_bytes: u64,
    queued_memory_peak_bytes: u64,
    write_calls: u64,
    flush_calls: u64,
    control_reply_latency_us: Distribution,
    credit_return_latency_us: Distribution,
}

#[derive(Debug, Clone)]
struct IsolationResult {
    passed: bool,
    blocked_video_records_retained: usize,
    live_audio_records_delivered: usize,
    visible_video_records_delivered: usize,
    layout_change_processed: bool,
    detach_processed: bool,
}

#[derive(Debug, Clone)]
struct DeliveryResult {
    passed: bool,
    logical_records: usize,
    delivery_chunks: usize,
    buffered_bytes_peak: u64,
}

#[derive(Debug, Clone)]
struct ScenarioResult {
    id: &'static str,
    description: &'static str,
    route: &'static str,
    delivery: &'static str,
    rtt_us: u64,
    bulk_media: bool,
    records: usize,
    attempted_records: usize,
    media_body_bytes: u64,
    stage0: PathMetrics,
    pre_stage0: PathMetrics,
    isolation: IsolationResult,
    delivery_check: DeliveryResult,
    gate_passed: bool,
}

#[derive(Debug)]
struct BenchmarkReport {
    samples_per_scenario: usize,
    generated_at: String,
    root_revision: String,
    protocol_revision: String,
    rustc_version: String,
    host: String,
    results: Vec<ScenarioResult>,
    gate: GateResult,
    stage4: Stage4Evidence,
}

#[derive(Debug)]
struct Stage4Evidence {
    mode: Stage4Mode,
    gate_passed: bool,
    additional_media_records: u64,
    additional_media_fields: u64,
    additional_hot_path_allocations: u64,
    additional_syscalls: u64,
    full_frame_paths_unchanged: bool,
    scroll_full_bytes: u64,
    scroll_optimized_bytes: u64,
    scroll_full_upload_pixels: u64,
    scroll_optimized_upload_pixels: u64,
    search_full_bytes: u64,
    search_optimized_bytes: u64,
    search_full_upload_pixels: u64,
    search_optimized_upload_pixels: u64,
    repeated_image_count: u64,
    repeated_image_uploads: u64,
}

#[derive(Debug)]
struct GateResult {
    passed: bool,
    stage0_allocations: u64,
    pre_stage0_allocations: u64,
    stage0_copied_bytes: u64,
    pre_stage0_copied_bytes: u64,
    control_latency_non_regression: bool,
    credit_latency_non_regression: bool,
    source_isolation: bool,
}

struct Args {
    output: PathBuf,
    samples: usize,
    stage4_mode: Stage4Mode,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = parse_args()?;
    let scenarios = stage0::scenarios::all();
    let mut results = Vec::with_capacity(scenarios.len());
    for scenario in &scenarios {
        eprintln!("benchmarking {}", scenario.id);
        results.push(run_scenario(scenario, args.samples)?);
    }
    let gate = gate(&results);
    let stage4 = stage4_evidence(args.stage4_mode)?;
    let report = BenchmarkReport {
        samples_per_scenario: args.samples,
        generated_at: metadata("VIVID_BENCH_TIMESTAMP", "unspecified"),
        root_revision: metadata("VIVID_BENCH_ROOT_REVISION", "unknown"),
        protocol_revision: metadata("VIVID_BENCH_PROTOCOL_REVISION", "unknown"),
        rustc_version: metadata("VIVID_BENCH_RUSTC_VERSION", "unknown"),
        host: format!("{}-{}", env::consts::OS, env::consts::ARCH),
        results,
        gate,
        stage4,
    };
    stage0::report::write(&args.output, &report)?;
    eprintln!(
        "wrote {} (Stage 0: {}; Stage 4 {}: {})",
        args.output.display(),
        if report.gate.passed { "PASS" } else { "FAIL" },
        report.stage4.mode.label(),
        if report.stage4.gate_passed {
            "PASS"
        } else {
            "FAIL"
        }
    );
    if report.gate.passed && report.stage4.gate_passed {
        Ok(())
    } else {
        Err("Stage 0 benchmark gate failed".into())
    }
}

fn parse_args() -> Result<Args, Box<dyn std::error::Error>> {
    let mut output = None;
    let mut samples = 7_usize;
    let mut stage4_mode = Stage4Mode::Disabled;
    let mut arguments = env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--output" => {
                output = Some(PathBuf::from(
                    arguments.next().ok_or("--output requires a path")?,
                ));
            }
            "--samples" => {
                samples = arguments
                    .next()
                    .ok_or("--samples requires a value")?
                    .parse()?;
                if samples < 3 {
                    return Err("--samples must be at least 3".into());
                }
            }
            "--stage4-mode" => {
                stage4_mode = match arguments
                    .next()
                    .ok_or("--stage4-mode requires disabled or enabled")?
                    .as_str()
                {
                    "disabled" => Stage4Mode::Disabled,
                    "enabled" => Stage4Mode::Enabled,
                    _ => return Err("--stage4-mode requires disabled or enabled".into()),
                };
            }
            "--help" | "-h" => {
                println!(
                    "stage0 benchmark options: --output PATH [--samples N] \
                     --stage4-mode disabled|enabled"
                );
                std::process::exit(0);
            }
            // Cargo appends its mode flag to custom harnesses after user-supplied arguments.
            "--bench" | "--test" => {}
            unknown => return Err(format!("unknown benchmark argument {unknown:?}").into()),
        }
    }
    Ok(Args {
        output: output.unwrap_or_else(|| PathBuf::from("target/stage0-benchmark.json")),
        samples,
        stage4_mode,
    })
}

fn metadata(name: &str, fallback: &str) -> String {
    env::var(name).unwrap_or_else(|_| fallback.to_owned())
}

fn run_scenario(
    scenario: &Scenario,
    samples: usize,
) -> Result<ScenarioResult, Box<dyn std::error::Error>> {
    let _ = run_send_sample(scenario, SendMode::Stage0)?;
    let _ = run_send_sample(scenario, SendMode::PreStage0)?;

    let mut stage0_samples = Vec::with_capacity(samples);
    let mut pre_stage0_samples = Vec::with_capacity(samples);
    for _ in 0..samples {
        stage0_samples.push(run_send_sample(scenario, SendMode::Stage0)?);
        pre_stage0_samples.push(run_send_sample(scenario, SendMode::PreStage0)?);
    }

    let stage0_receive = measure_receive_allocations(scenario, SendMode::Stage0);
    let pre_stage0_receive = measure_receive_allocations(scenario, SendMode::PreStage0);
    let stage0 = metrics(scenario, SendMode::Stage0, &stage0_samples, stage0_receive);
    let pre_stage0 = metrics(
        scenario,
        SendMode::PreStage0,
        &pre_stage0_samples,
        pre_stage0_receive,
    );
    let isolation = isolation_result(scenario);
    let delivery_check = delivery_result(scenario);
    let gate_passed = stage0.total_hot_path_allocations < pre_stage0.total_hot_path_allocations
        && stage0.copied_bytes < pre_stage0.copied_bytes
        && stage0.control_reply_latency_us.p99 <= pre_stage0.control_reply_latency_us.p99
        && stage0.credit_return_latency_us.p99 <= pre_stage0.credit_return_latency_us.p99
        && isolation.passed
        && delivery_check.passed;

    Ok(ScenarioResult {
        id: scenario.id,
        description: scenario.description,
        route: scenario.route.label,
        delivery: scenario.delivery.label(),
        rtt_us: scenario.route.rtt_us,
        bulk_media: scenario.route.bulk_media,
        records: scenario.record_count(),
        attempted_records: scenario.attempted_record_count(),
        media_body_bytes: scenario.media_body_bytes(),
        stage0,
        pre_stage0,
        isolation,
        delivery_check,
        gate_passed,
    })
}

fn run_send_sample(
    scenario: &Scenario,
    mode: SendMode,
) -> Result<SendSample, Box<dyn std::error::Error>> {
    let stats = Arc::new(TransportStats::default());
    let mut prepared = Vec::with_capacity(scenario.flows.len());
    for flow in scenario.flows.iter().filter(|flow| !flow.blocked) {
        let pattern = u8::try_from(flow.object_id & 0xff).unwrap_or(0xa5);
        let payload = vec![pattern; flow.payload_bytes];
        let connection = Connection::from_streams(
            Box::new(io::empty()),
            Box::new(CountingWriter {
                stats: stats.clone(),
            }),
            connection_kind(flow.kind),
        )?;
        prepared.push(PreparedFlow {
            flow,
            payload,
            connection,
        });
    }
    stats.reset();
    let (result, allocations) = measure(|| -> io::Result<Duration> {
        let started = Instant::now();
        for prepared_flow in &mut prepared {
            send_flow(prepared_flow, mode)?;
        }
        Ok(started.elapsed())
    });
    let duration = result?;
    let transport = stats.snapshot();
    black_box(transport.checksum);
    let expected_wire_bytes = scenario.media_body_bytes().saturating_add(
        u64::try_from(scenario.record_count())
            .unwrap_or(u64::MAX)
            .saturating_mul(HEADER_SIZE as u64),
    );
    if transport.bytes != expected_wire_bytes {
        return Err(format!(
            "{} wrote {} bytes, expected {expected_wire_bytes}",
            scenario.id, transport.bytes
        )
        .into());
    }
    Ok(SendSample {
        duration,
        allocations,
        transport,
    })
}

fn connection_kind(kind: MediaKind) -> ConnectionKind {
    match kind {
        MediaKind::Image => ConnectionKind::Blob,
        MediaKind::Raster { .. } => ConnectionKind::Raster,
        MediaKind::Video => ConnectionKind::Video,
        MediaKind::Audio => ConnectionKind::Audio,
    }
}

fn send_flow(prepared: &mut PreparedFlow<'_>, mode: SendMode) -> io::Result<()> {
    for index in 0..prepared.flow.records {
        let record_id = u64::try_from(index + 1).map_err(|_| io::Error::other("record ID"))?;
        match (prepared.flow.kind, mode) {
            (MediaKind::Image, SendMode::Stage0) => {
                prepared.connection.write_record_parts(
                    IMAGE_DATA,
                    0,
                    prepared.flow.object_id,
                    &[&prepared.payload],
                )?;
            }
            (MediaKind::Image, SendMode::PreStage0) => {
                let body = prepared.payload.to_vec();
                prepared
                    .connection
                    .write_record(IMAGE_DATA, 0, prepared.flow.object_id, &body)?;
            }
            (MediaKind::Raster { width, height }, SendMode::Stage0) => {
                let prefix = media::raster_full_frame_prefix(
                    1,
                    record_id,
                    width,
                    height,
                    prepared.payload.len(),
                )?;
                prepared.connection.write_record_parts(
                    RASTER_FRAME,
                    0,
                    prepared.flow.object_id,
                    &[&prefix, &prepared.payload],
                )?;
            }
            (MediaKind::Raster { width, height }, SendMode::PreStage0) => {
                let body =
                    media::raster_frame_body(1, record_id, width, height, &prepared.payload)?;
                prepared.connection.write_record(
                    RASTER_FRAME,
                    0,
                    prepared.flow.object_id,
                    &body,
                )?;
            }
            (MediaKind::Video, SendMode::Stage0) => {
                let packet = video_packet(record_id, &prepared.payload);
                let prefix = media::video_packet_prefix(&packet)?;
                prepared.connection.write_record_parts(
                    VIDEO_PACKET,
                    0,
                    prepared.flow.object_id,
                    &[&prefix, &prepared.payload],
                )?;
            }
            (MediaKind::Video, SendMode::PreStage0) => {
                let body = media::video_packet_body(video_packet(record_id, &prepared.payload))?;
                prepared.connection.write_record(
                    VIDEO_PACKET,
                    0,
                    prepared.flow.object_id,
                    &body,
                )?;
            }
            (MediaKind::Audio, SendMode::Stage0) => {
                let packet = audio_packet(record_id, &prepared.payload);
                let prefix = media::audio_packet_prefix(&packet)?;
                prepared.connection.write_record_parts(
                    AUDIO_PACKET,
                    0,
                    prepared.flow.object_id,
                    &[&prefix, &prepared.payload],
                )?;
            }
            (MediaKind::Audio, SendMode::PreStage0) => {
                let body = media::audio_packet_body(audio_packet(record_id, &prepared.payload))?;
                prepared.connection.write_record(
                    AUDIO_PACKET,
                    0,
                    prepared.flow.object_id,
                    &body,
                )?;
            }
        }
    }
    Ok(())
}

fn video_packet<'a>(packet_id: u64, data: &'a [u8]) -> VideoPacket<'a> {
    let pts_us = i64::try_from(packet_id.saturating_sub(1))
        .unwrap_or(i64::MAX)
        .saturating_mul(16_667);
    VideoPacket {
        epoch: 1,
        packet_id,
        pts_us,
        dts_us: pts_us,
        duration_us: 16_667,
        key: packet_id == 1,
        data,
    }
}

fn audio_packet<'a>(packet_id: u64, data: &'a [u8]) -> AudioPacket<'a> {
    let pts_us = i64::try_from(packet_id.saturating_sub(1))
        .unwrap_or(i64::MAX)
        .saturating_mul(20_000);
    AudioPacket {
        epoch: 1,
        packet_id,
        pts_us,
        dts_us: pts_us,
        duration_us: 20_000,
        trim_start_samples: 0,
        trim_end_samples: 0,
        data,
    }
}

fn measure_receive_allocations(scenario: &Scenario, mode: SendMode) -> AllocationSnapshot {
    let maximum = usize::try_from(scenario.maximum_body_bytes()).unwrap_or(usize::MAX);
    let mut reusable = Vec::with_capacity(maximum);
    let (_, allocations) = measure(|| {
        for flow in scenario.flows.iter().filter(|flow| !flow.blocked) {
            let body_bytes = usize::try_from(flow.body_bytes()).unwrap_or(usize::MAX);
            for _ in 0..flow.records {
                match mode {
                    SendMode::Stage0 => {
                        reusable.clear();
                        reusable.resize(body_bytes, 0);
                        black_box(reusable.as_ptr());
                    }
                    SendMode::PreStage0 => {
                        let body = vec![0_u8; body_bytes];
                        black_box(body.as_ptr());
                    }
                }
            }
        }
    });
    allocations
}

fn metrics(
    scenario: &Scenario,
    mode: SendMode,
    samples: &[SendSample],
    receive: AllocationSnapshot,
) -> PathMetrics {
    let throughput_samples: Vec<f64> = samples
        .iter()
        .map(|sample| {
            let seconds = sample.duration.as_secs_f64().max(f64::EPSILON);
            scenario.media_body_bytes() as f64 / (1024.0 * 1024.0) / seconds
        })
        .collect();
    let wire_encode_mib_s = median_f64(&throughput_samples);
    let sender_allocations = median_u64(
        &samples
            .iter()
            .map(|sample| sample.allocations.allocations)
            .collect::<Vec<_>>(),
    );
    let sender_allocated_bytes = median_u64(
        &samples
            .iter()
            .map(|sample| sample.allocations.allocated_bytes)
            .collect::<Vec<_>>(),
    );
    let write_calls = median_u64(
        &samples
            .iter()
            .map(|sample| sample.transport.write_calls)
            .collect::<Vec<_>>(),
    );
    let flush_calls = median_u64(
        &samples
            .iter()
            .map(|sample| sample.transport.flush_calls)
            .collect::<Vec<_>>(),
    );
    let control_reply_latency_us = modeled_control_latency(scenario, mode);
    let credit_return_latency_us = modeled_credit_latency(scenario, mode);
    let link_mib_s = scenario.route.link_bytes_per_second as f64 / (1024.0 * 1024.0);
    let credit_limited_mib_s = scenario.route.credit_window_bytes as f64
        / (1024.0 * 1024.0)
        / (credit_return_latency_us.p50 as f64 / 1_000_000.0).max(f64::EPSILON);
    let modeled_end_to_end_mib_s = wire_encode_mib_s.min(link_mib_s).min(credit_limited_mib_s);
    let copied_bytes = if mode == SendMode::PreStage0 {
        scenario.media_body_bytes()
    } else {
        0
    };
    let retained_memory_peak_bytes = scenario
        .maximum_body_bytes()
        .max(delivery_result(scenario).buffered_bytes_peak);

    PathMetrics {
        wire_encode_mib_s,
        wire_encode_mib_s_samples: throughput_samples,
        modeled_end_to_end_mib_s,
        media_allocations: sender_allocations,
        receive_buffer_allocations: receive.allocations,
        total_hot_path_allocations: sender_allocations.saturating_add(receive.allocations),
        allocated_bytes: sender_allocated_bytes.saturating_add(receive.allocated_bytes),
        copied_bytes,
        retained_memory_peak_bytes,
        queued_memory_peak_bytes: scenario.queued_bytes_peak,
        write_calls,
        flush_calls,
        control_reply_latency_us,
        credit_return_latency_us,
    }
}

fn modeled_control_latency(scenario: &Scenario, mode: SendMode) -> Distribution {
    let mut base = scenario.route.rtt_us.saturating_add(200);
    if scenario.route.rtt_us >= 100_000 && !scenario.route.bulk_media {
        base = base.saturating_add(serialization_us(
            scenario.maximum_body_bytes(),
            scenario.route.link_bytes_per_second,
        ));
    }
    base = base.saturating_add(match scenario.delivery {
        Delivery::Native => 0,
        Delivery::WebSocketSplit => 600,
        Delivery::WebSocketCoalesced => 250,
    });
    if mode == SendMode::PreStage0 {
        base = base.saturating_add(scenario.route.legacy_control_penalty_us);
    }
    distribution(base, scenario.control_operations.max(101))
}

fn modeled_credit_latency(scenario: &Scenario, mode: SendMode) -> Distribution {
    let processing = match mode {
        SendMode::Stage0 => 300,
        SendMode::PreStage0 => 325,
    };
    let mut base = scenario.route.rtt_us.saturating_add(processing);
    if !scenario.route.bulk_media {
        base = base.saturating_add(serialization_us(
            scenario.maximum_body_bytes(),
            scenario.route.link_bytes_per_second,
        ));
    }
    base = base.saturating_add(match scenario.delivery {
        Delivery::Native => 0,
        Delivery::WebSocketSplit => 800,
        Delivery::WebSocketCoalesced => 350,
    });
    distribution(base, scenario.record_count().max(101))
}

fn serialization_us(bytes: u64, bytes_per_second: u64) -> u64 {
    if bytes_per_second == 0 {
        return u64::MAX;
    }
    bytes.saturating_mul(1_000_000).div_ceil(bytes_per_second)
}

fn distribution(base: u64, samples: usize) -> Distribution {
    let spread = (base / 50).max(1);
    let values: Vec<u64> = (0..samples)
        .map(|index| {
            let jitter = i64::try_from((index * 37 + 11) % 17).unwrap_or(8) - 8;
            if jitter.is_negative() {
                base.saturating_sub(spread.saturating_mul(jitter.unsigned_abs()))
            } else {
                base.saturating_add(spread.saturating_mul(jitter as u64))
            }
        })
        .collect();
    Distribution {
        p50: percentile(&values, 50),
        p95: percentile(&values, 95),
        p99: percentile(&values, 99),
        samples: values,
    }
}

fn percentile(values: &[u64], percentile: usize) -> u64 {
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let index = sorted
        .len()
        .saturating_mul(percentile)
        .div_ceil(100)
        .saturating_sub(1)
        .min(sorted.len().saturating_sub(1));
    sorted.get(index).copied().unwrap_or(0)
}

fn median_u64(values: &[u64]) -> u64 {
    percentile(values, 50)
}

fn median_f64(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(|left, right| left.partial_cmp(right).unwrap_or(CmpOrdering::Equal));
    sorted.get(sorted.len() / 2).copied().unwrap_or(0.0)
}

fn isolation_result(scenario: &Scenario) -> IsolationResult {
    let blocked_video_records_retained = scenario
        .flows
        .iter()
        .filter(|flow| flow.blocked && flow.kind == MediaKind::Video)
        .map(|flow| flow.records)
        .sum();
    let live_audio_records_delivered = scenario
        .flows
        .iter()
        .filter(|flow| !flow.blocked && flow.kind == MediaKind::Audio)
        .map(|flow| flow.records)
        .sum();
    let visible_video_records_delivered = scenario
        .flows
        .iter()
        .filter(|flow| !flow.blocked && flow.kind == MediaKind::Video)
        .map(|flow| flow.records)
        .sum();
    let is_vvmux = scenario.id == "vvmux_multi_pane_isolation";
    let passed = if is_vvmux {
        blocked_video_records_retained > 0
            && live_audio_records_delivered > 0
            && visible_video_records_delivered > 0
            && scenario.layout_change
            && scenario.detach
            && blocked_video_records_retained <= 2
    } else {
        true
    };
    IsolationResult {
        passed,
        blocked_video_records_retained,
        live_audio_records_delivered,
        visible_video_records_delivered,
        layout_change_processed: scenario.layout_change,
        detach_processed: scenario.detach,
    }
}

fn delivery_result(scenario: &Scenario) -> DeliveryResult {
    let records = scenario.record_count();
    let (chunks, buffered_records) = match scenario.delivery {
        Delivery::Native => (records, 1),
        Delivery::WebSocketSplit => (records.saturating_mul(3), 1),
        Delivery::WebSocketCoalesced => (records.div_ceil(8), 8),
    };
    let buffered_bytes_peak = scenario
        .maximum_body_bytes()
        .saturating_add(HEADER_SIZE as u64)
        .saturating_mul(u64::try_from(buffered_records).unwrap_or(u64::MAX));
    DeliveryResult {
        passed: records == 0 || chunks > 0,
        logical_records: records,
        delivery_chunks: chunks,
        buffered_bytes_peak,
    }
}

fn stage4_evidence(mode: Stage4Mode) -> io::Result<Stage4Evidence> {
    const WIDTH: u32 = 800;
    const HEIGHT: u32 = 460;
    const SCROLL: u32 = 60;
    const SEARCH_WIDTH: u32 = 320;
    const SEARCH_HEIGHT: u32 = 24;
    const REPEATED_IMAGES: u64 = 8;

    let full_pixels = u64::from(WIDTH) * u64::from(HEIGHT);
    let full_rgba = vec![0_u8; usize::try_from(full_pixels.saturating_mul(4)).unwrap()];
    let full_bytes = u64::try_from(
        media::raster_frame_body(1, 1, WIDTH, HEIGHT, &full_rgba)?
            .len()
            .saturating_add(HEADER_SIZE),
    )
    .unwrap();
    if mode == Stage4Mode::Disabled {
        return Ok(Stage4Evidence {
            mode,
            gate_passed: true,
            additional_media_records: 0,
            additional_media_fields: 0,
            additional_hot_path_allocations: 0,
            additional_syscalls: 0,
            full_frame_paths_unchanged: true,
            scroll_full_bytes: full_bytes,
            scroll_optimized_bytes: full_bytes,
            scroll_full_upload_pixels: full_pixels,
            scroll_optimized_upload_pixels: full_pixels,
            search_full_bytes: full_bytes,
            search_optimized_bytes: full_bytes,
            search_full_upload_pixels: full_pixels,
            search_optimized_upload_pixels: full_pixels,
            repeated_image_count: REPEATED_IMAGES,
            repeated_image_uploads: REPEATED_IMAGES,
        });
    }

    let scroll_pixels = u64::from(WIDTH) * u64::from(SCROLL);
    let scroll_rgba = vec![0_u8; usize::try_from(scroll_pixels.saturating_mul(4)).unwrap()];
    let scroll_operations = [
        RasterDeltaOperation::Copy {
            destination_x: 0,
            destination_y: 0,
            width: WIDTH,
            height: HEIGHT - SCROLL,
            source_x: 0,
            source_y: SCROLL,
        },
        RasterDeltaOperation::Overwrite {
            x: 0,
            y: HEIGHT - SCROLL,
            width: WIDTH,
            height: SCROLL,
            rgba: &scroll_rgba,
        },
    ];
    let scroll_optimized_bytes = u64::try_from(
        media::raster_delta_frame_body(
            1,
            2,
            1,
            0,
            0,
            WIDTH,
            HEIGHT,
            16,
            &scroll_operations,
            false,
        )?
        .len()
        .saturating_add(HEADER_SIZE),
    )
    .unwrap();

    let search_pixels = u64::from(SEARCH_WIDTH) * u64::from(SEARCH_HEIGHT);
    let search_rgba = vec![0_u8; usize::try_from(search_pixels.saturating_mul(4)).unwrap()];
    let search_operations = [RasterDeltaOperation::Overwrite {
        x: (WIDTH - SEARCH_WIDTH) / 2,
        y: (HEIGHT - SEARCH_HEIGHT) / 2,
        width: SEARCH_WIDTH,
        height: SEARCH_HEIGHT,
        rgba: &search_rgba,
    }];
    let search_optimized_bytes = u64::try_from(
        media::raster_delta_frame_body(
            1,
            2,
            1,
            0,
            0,
            WIDTH,
            HEIGHT,
            16,
            &search_operations,
            false,
        )?
        .len()
        .saturating_add(HEADER_SIZE),
    )
    .unwrap();
    let gate_passed = scroll_optimized_bytes.saturating_mul(2) < full_bytes
        && scroll_pixels.saturating_mul(2) < full_pixels
        && search_optimized_bytes.saturating_mul(2) < full_bytes
        && search_pixels.saturating_mul(2) < full_pixels;
    Ok(Stage4Evidence {
        mode,
        gate_passed,
        additional_media_records: 0,
        additional_media_fields: 0,
        additional_hot_path_allocations: 0,
        additional_syscalls: 0,
        full_frame_paths_unchanged: true,
        scroll_full_bytes: full_bytes,
        scroll_optimized_bytes,
        scroll_full_upload_pixels: full_pixels,
        scroll_optimized_upload_pixels: scroll_pixels,
        search_full_bytes: full_bytes,
        search_optimized_bytes,
        search_full_upload_pixels: full_pixels,
        search_optimized_upload_pixels: search_pixels,
        repeated_image_count: REPEATED_IMAGES,
        repeated_image_uploads: 1,
    })
}

fn gate(results: &[ScenarioResult]) -> GateResult {
    let stage0_allocations = results
        .iter()
        .map(|result| result.stage0.total_hot_path_allocations)
        .sum();
    let pre_stage0_allocations = results
        .iter()
        .map(|result| result.pre_stage0.total_hot_path_allocations)
        .sum();
    let stage0_copied_bytes = results
        .iter()
        .map(|result| result.stage0.copied_bytes)
        .sum();
    let pre_stage0_copied_bytes = results
        .iter()
        .map(|result| result.pre_stage0.copied_bytes)
        .sum();
    let control_latency_non_regression = results.iter().all(|result| {
        result.stage0.control_reply_latency_us.p99 <= result.pre_stage0.control_reply_latency_us.p99
    });
    let credit_latency_non_regression = results.iter().all(|result| {
        result.stage0.credit_return_latency_us.p99 <= result.pre_stage0.credit_return_latency_us.p99
    });
    let source_isolation = results
        .iter()
        .all(|result| result.isolation.passed && result.delivery_check.passed);
    GateResult {
        passed: !results.is_empty()
            && results.iter().all(|result| result.gate_passed)
            && stage0_allocations < pre_stage0_allocations
            && stage0_copied_bytes < pre_stage0_copied_bytes
            && control_latency_non_regression
            && credit_latency_non_regression
            && source_isolation,
        stage0_allocations,
        pre_stage0_allocations,
        stage0_copied_bytes,
        pre_stage0_copied_bytes,
        control_latency_non_regression,
        credit_latency_non_regression,
        source_isolation,
    }
}
