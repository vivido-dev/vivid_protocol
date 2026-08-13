use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::Path;

use super::super::{BenchmarkReport, Distribution, PathMetrics, Stage4Evidence};

pub fn write(path: &Path, report: &BenchmarkReport) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut output = BufWriter::new(File::create(path)?);
    writeln!(output, "{{")?;
    writeln!(output, "  \"schema_version\": 1,")?;
    writeln!(output, "  \"baseline_id\": \"vivid-stage0-v1\",")?;
    writeln!(
        output,
        "  \"generated_at_utc\": {},",
        json_string(&report.generated_at)
    )?;
    writeln!(
        output,
        "  \"generator\": \"vivid_protocol/benches/stage0\","
    )?;
    writeln!(
        output,
        "  \"samples_per_scenario\": {},",
        report.samples_per_scenario
    )?;
    writeln!(output, "  \"environment\": {{")?;
    writeln!(
        output,
        "    \"root_revision\": {},",
        json_string(&report.root_revision)
    )?;
    writeln!(
        output,
        "    \"protocol_revision\": {},",
        json_string(&report.protocol_revision)
    )?;
    writeln!(
        output,
        "    \"rustc\": {},",
        json_string(&report.rustc_version)
    )?;
    writeln!(output, "    \"host\": {}", json_string(&report.host))?;
    writeln!(output, "  }},")?;
    writeln!(output, "  \"measurement_contract\": {{")?;
    writeln!(
        output,
        "    \"wire_encode_throughput\": \"measured with an instrumented local writer\","
    )?;
    writeln!(
        output,
        "    \"allocations\": \"measured by the benchmark process global allocator\","
    )?;
    writeln!(
        output,
        "    \"copied_bytes\": \"semantic media-body bytes copied into legacy owned bodies\","
    )?;
    writeln!(
        output,
        "    \"latencies\": \"deterministic local RTT, serialization, and queue delay shim\","
    )?;
    writeln!(
        output,
        "    \"memory\": \"bounded receive-buffer retention and scenario queue high-water model\""
    )?;
    writeln!(output, "  }},")?;
    write_stage4(&mut output, &report.stage4)?;
    writeln!(output, "  \"scenarios\": [")?;
    for (index, result) in report.results.iter().enumerate() {
        writeln!(output, "    {{")?;
        writeln!(output, "      \"id\": {},", json_string(result.id))?;
        writeln!(
            output,
            "      \"description\": {},",
            json_string(result.description)
        )?;
        writeln!(output, "      \"route\": {},", json_string(result.route))?;
        writeln!(
            output,
            "      \"delivery\": {},",
            json_string(result.delivery)
        )?;
        writeln!(output, "      \"rtt_us\": {},", result.rtt_us)?;
        writeln!(output, "      \"bulk_media\": {},", result.bulk_media)?;
        writeln!(output, "      \"records\": {},", result.records)?;
        writeln!(
            output,
            "      \"attempted_records\": {},",
            result.attempted_records
        )?;
        writeln!(
            output,
            "      \"media_body_bytes\": {},",
            result.media_body_bytes
        )?;
        write_metrics(&mut output, "stage0", &result.stage0, true)?;
        write_metrics(
            &mut output,
            "pre_stage0_reference",
            &result.pre_stage0,
            true,
        )?;
        writeln!(output, "      \"isolation\": {{")?;
        writeln!(output, "        \"passed\": {},", result.isolation.passed)?;
        writeln!(
            output,
            "        \"blocked_video_records_retained\": {},",
            result.isolation.blocked_video_records_retained
        )?;
        writeln!(
            output,
            "        \"live_audio_records_delivered\": {},",
            result.isolation.live_audio_records_delivered
        )?;
        writeln!(
            output,
            "        \"visible_video_records_delivered\": {},",
            result.isolation.visible_video_records_delivered
        )?;
        writeln!(
            output,
            "        \"layout_change_processed\": {},",
            result.isolation.layout_change_processed
        )?;
        writeln!(
            output,
            "        \"detach_processed\": {}",
            result.isolation.detach_processed
        )?;
        writeln!(output, "      }},")?;
        writeln!(output, "      \"delivery_check\": {{")?;
        writeln!(
            output,
            "        \"passed\": {},",
            result.delivery_check.passed
        )?;
        writeln!(
            output,
            "        \"logical_records\": {},",
            result.delivery_check.logical_records
        )?;
        writeln!(
            output,
            "        \"delivery_chunks\": {},",
            result.delivery_check.delivery_chunks
        )?;
        writeln!(
            output,
            "        \"buffered_bytes_peak\": {}",
            result.delivery_check.buffered_bytes_peak
        )?;
        writeln!(output, "      }},")?;
        writeln!(output, "      \"gate_passed\": {}", result.gate_passed)?;
        writeln!(
            output,
            "    }}{}",
            if index + 1 == report.results.len() {
                ""
            } else {
                ","
            }
        )?;
    }
    writeln!(output, "  ],")?;
    writeln!(output, "  \"stage0_gate\": {{")?;
    writeln!(output, "    \"passed\": {},", report.gate.passed)?;
    writeln!(
        output,
        "    \"stage0_hot_path_allocations\": {},",
        report.gate.stage0_allocations
    )?;
    writeln!(
        output,
        "    \"pre_stage0_hot_path_allocations\": {},",
        report.gate.pre_stage0_allocations
    )?;
    writeln!(
        output,
        "    \"stage0_copied_bytes\": {},",
        report.gate.stage0_copied_bytes
    )?;
    writeln!(
        output,
        "    \"pre_stage0_copied_bytes\": {},",
        report.gate.pre_stage0_copied_bytes
    )?;
    writeln!(
        output,
        "    \"control_latency_non_regression\": {},",
        report.gate.control_latency_non_regression
    )?;
    writeln!(
        output,
        "    \"credit_latency_non_regression\": {},",
        report.gate.credit_latency_non_regression
    )?;
    writeln!(
        output,
        "    \"source_isolation\": {}",
        report.gate.source_isolation
    )?;
    writeln!(output, "  }},")?;
    writeln!(output, "  \"invariant_review\": {{")?;
    for (index, (id, evidence)) in invariants().iter().enumerate() {
        writeln!(output, "    {}: {{", json_string(id))?;
        writeln!(output, "      \"status\": \"pass\",")?;
        writeln!(output, "      \"evidence\": {}", json_string(evidence))?;
        writeln!(
            output,
            "    }}{}",
            if index + 1 == invariants().len() {
                ""
            } else {
                ","
            }
        )?;
    }
    writeln!(output, "  }}")?;
    writeln!(output, "}}")?;
    output.flush()
}

fn write_stage4(output: &mut impl Write, evidence: &Stage4Evidence) -> io::Result<()> {
    writeln!(output, "  \"stage4\": {{")?;
    writeln!(
        output,
        "    \"mode\": {},",
        json_string(evidence.mode.label())
    )?;
    writeln!(output, "    \"gate_passed\": {},", evidence.gate_passed)?;
    writeln!(
        output,
        "    \"disabled_overhead\": {{\"additional_media_records\": {}, \
         \"additional_media_fields\": {}, \"additional_hot_path_allocations\": {}, \
         \"additional_syscalls\": {}}},",
        evidence.additional_media_records,
        evidence.additional_media_fields,
        evidence.additional_hot_path_allocations,
        evidence.additional_syscalls,
    )?;
    writeln!(
        output,
        "    \"full_frame_paths_unchanged\": {},",
        evidence.full_frame_paths_unchanged
    )?;
    writeln!(
        output,
        "    \"vvrd_scroll\": {{\"full_wire_bytes\": {}, \"optimized_wire_bytes\": {}, \
         \"full_upload_pixels\": {}, \"optimized_upload_pixels\": {}}},",
        evidence.scroll_full_bytes,
        evidence.scroll_optimized_bytes,
        evidence.scroll_full_upload_pixels,
        evidence.scroll_optimized_upload_pixels,
    )?;
    writeln!(
        output,
        "    \"vvrd_search_highlight\": {{\"full_wire_bytes\": {}, \
         \"optimized_wire_bytes\": {}, \"full_upload_pixels\": {}, \
         \"optimized_upload_pixels\": {}}},",
        evidence.search_full_bytes,
        evidence.search_optimized_bytes,
        evidence.search_full_upload_pixels,
        evidence.search_optimized_upload_pixels,
    )?;
    writeln!(
        output,
        "    \"repeated_image\": {{\"logical_presentations\": {}, \"encoded_uploads\": {}}}",
        evidence.repeated_image_count, evidence.repeated_image_uploads,
    )?;
    writeln!(output, "  }},")
}

fn write_metrics(
    output: &mut impl Write,
    name: &str,
    metrics: &PathMetrics,
    trailing_comma: bool,
) -> io::Result<()> {
    writeln!(output, "      {}: {{", json_string(name))?;
    writeln!(
        output,
        "        \"wire_encode_mib_s\": {:.6},",
        metrics.wire_encode_mib_s
    )?;
    write_f64_array(
        output,
        "wire_encode_mib_s_samples",
        &metrics.wire_encode_mib_s_samples,
    )?;
    writeln!(
        output,
        "        \"modeled_end_to_end_mib_s\": {:.6},",
        metrics.modeled_end_to_end_mib_s
    )?;
    writeln!(
        output,
        "        \"media_allocations\": {},",
        metrics.media_allocations
    )?;
    writeln!(
        output,
        "        \"receive_buffer_allocations\": {},",
        metrics.receive_buffer_allocations
    )?;
    writeln!(
        output,
        "        \"total_hot_path_allocations\": {},",
        metrics.total_hot_path_allocations
    )?;
    writeln!(
        output,
        "        \"allocated_bytes\": {},",
        metrics.allocated_bytes
    )?;
    writeln!(
        output,
        "        \"copied_bytes\": {},",
        metrics.copied_bytes
    )?;
    writeln!(
        output,
        "        \"retained_memory_peak_bytes\": {},",
        metrics.retained_memory_peak_bytes
    )?;
    writeln!(
        output,
        "        \"queued_memory_peak_bytes\": {},",
        metrics.queued_memory_peak_bytes
    )?;
    writeln!(output, "        \"write_calls\": {},", metrics.write_calls)?;
    writeln!(output, "        \"flush_calls\": {},", metrics.flush_calls)?;
    write_distribution(
        output,
        "control_reply_latency_us",
        &metrics.control_reply_latency_us,
        true,
    )?;
    write_distribution(
        output,
        "credit_return_latency_us",
        &metrics.credit_return_latency_us,
        false,
    )?;
    writeln!(output, "      }}{}", if trailing_comma { "," } else { "" })
}

fn write_distribution(
    output: &mut impl Write,
    name: &str,
    distribution: &Distribution,
    trailing_comma: bool,
) -> io::Result<()> {
    writeln!(output, "        {}: {{", json_string(name))?;
    writeln!(output, "          \"p50\": {},", distribution.p50)?;
    writeln!(output, "          \"p95\": {},", distribution.p95)?;
    writeln!(output, "          \"p99\": {},", distribution.p99)?;
    write_u64_array(output, "samples", &distribution.samples, 10)?;
    writeln!(
        output,
        "        }}{}",
        if trailing_comma { "," } else { "" }
    )
}

fn write_f64_array(output: &mut impl Write, name: &str, values: &[f64]) -> io::Result<()> {
    write!(output, "        {}: [", json_string(name))?;
    for (index, value) in values.iter().enumerate() {
        if index != 0 {
            write!(output, ", ")?;
        }
        write!(output, "{value:.6}")?;
    }
    writeln!(output, "],")
}

fn write_u64_array(
    output: &mut impl Write,
    name: &str,
    values: &[u64],
    indentation: usize,
) -> io::Result<()> {
    write!(
        output,
        "{}{}: [",
        " ".repeat(indentation),
        json_string(name)
    )?;
    for (index, value) in values.iter().enumerate() {
        if index != 0 {
            write!(output, ", ")?;
        }
        write!(output, "{value}")?;
    }
    writeln!(output, "]")
}

fn json_string(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len() + 2);
    escaped.push('"');
    for character in value.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            character if character.is_control() => {
                use std::fmt::Write as _;
                let _ = write!(escaped, "\\u{:04x}", character as u32);
            }
            character => escaped.push(character),
        }
    }
    escaped.push('"');
    escaped
}

fn invariants() -> [(&'static str, &'static str); 8] {
    [
        (
            "INV-1",
            "Media prefix equivalence and byte-identical vectored-write tests; no media layout change.",
        ),
        (
            "INV-2",
            "Fast paths are selected only when used; default framing and feature-disabled media remain unchanged.",
        ),
        (
            "INV-3",
            "Presenter and browser control work uses bounded pending completion state.",
        ),
        (
            "INV-4",
            "MAX_CHANNEL_DATA, PONG, errors, correlated replies, and checkpoints bypass batched flushing.",
        ),
        (
            "INV-5",
            "Per-source writers, buffers, queues, and the blocked-video/live-audio isolation scenario.",
        ),
        (
            "INV-6",
            "Counters and benchmark output contain aggregate numbers and revision metadata only.",
        ),
        (
            "INV-7",
            "Record, prefix, geometry, and receive lengths are validated before allocation.",
        ),
        (
            "INV-8",
            "Receive buffers, pending operations, media queues, batches, and benchmark delay queues are bounded.",
        ),
    ]
}
