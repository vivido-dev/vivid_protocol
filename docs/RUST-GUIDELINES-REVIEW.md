# Rust guidelines review

This review checks the crate against Microsoft's Pragmatic Rust Guidelines for libraries (reviewed
2026-10-07). Commits `56ab835` and `56f7462` fixed the findings that fit inside the crate without a
redesign:

- identities that can only be built through their nonzero checks;
- `CHANNEL_OPEN` and `FILE_TRANSFER_OPEN` tags signed and verified from the message's named fields;
- `Debug` on every public type, redacted where it could expose keys, tags, proofs, or payloads;
- record types re-exported by name instead of by glob;
- `NativeDiscovery::from_lookup`, so discovery no longer requires the process environment;
- the guideline lint table in `Cargo.toml`.

A follow-up raised `rust-version` to 1.95 to match `vivid_sdk` and `vivido`. Nearly every
consumer already needed 1.95 through the SDK.

The open findings follow, most important first. Guideline IDs such as `M-INIT-CASCADED` name the
rule each one comes from, and each `#[expect]` reason in the source points back here.

## Errors (M-ERRORS-CANONICAL-STRUCTS)

Version 1.5.12 replaced the string-message errors:

- Binary-layout codecs (`media`, `audio_input`, `file_drop` file data, `anchor` markers, and
  `wire` preface and first-record checks) return `CodecError`: a struct with a private kind,
  `is_malformed`, `is_invalid_argument`, and `is_unsupported`, an optional source, and a
  backtrace. It converts into `io::Error` with the kinds callers saw before.
- Revision and generation counters return `revision::CounterError`.
- `timed` decoders return `MessageError`, like every other control-message codec.
- `MessageError` chains its CBOR and resource-contract causes through `source()`, and error
  enums are `#[non_exhaustive]`. `SizeError` renders sentences.

`map_err(|_| ...)` stays where a codec replaces an integer-conversion or slice-length error with
a message naming the field; the dropped error says only "out of range", so `Cargo.toml` keeps
`clippy::map_err_ignore` allowed with that reason.

## Documentation (M-CANONICAL-DOCS)

Every fallible public function now documents its errors, and every function that can panic says
when, so `clippy::missing_errors_doc` and `clippy::missing_panics_doc` are enforced. Each module
has `//!` documentation, and the size and CBOR constants explain how they were chosen.

`missing_docs` still reports 1,825 undocumented public items. Most are self-describing wire
fields (722), registry and flag constants (408), and enum variants (343). The rest are 122
methods, 165 types, 56 functions, eight associated constants, and one type alias. Documenting them would let the
crate enable `missing_docs`; the wire fields can usually point to the specification table that
defines them.

To reproduce the count:

```sh
cargo clippy --lib -- -A clippy::all -W missing_docs
```

## Long positional parameter lists (M-INIT-CASCADED)

Sixteen public functions take five or more parameters, often several integers in a row that the
compiler cannot tell apart:

- `media::raster_delta_frame_body` (10)
- `trace::TraceEmitter::emit` (9) and `emit_control` (7)
- `lease::LeaseMachine::begin_resume` (8) and `begin_activation` (7)
- `auth::resume_hello_proof` (7)
- `geometry::fit_quad`, `media::raster_frame_body_with_compression`, and
  `surface::surface_ready_payload` (6 each)

Five of them carry `#[expect(clippy::too_many_arguments)]`, whose reasons point here. The auth tag
functions were the worst case and now take the `ChannelOpen` or `FileTransferOpen` message
instead. The same approach fits here: pass the decoded message or a small named-field struct.

## Smaller items

- **The trace clock is not mockable (M-MOCKABLE-SYSCALLS).** `TraceGuard::callback` is an
  I/O-free sink, but timestamps come from `Instant::now()` and `Connection::trace` takes only a
  path. It is diagnostics only, so this matters less than discovery did.
- **The record re-export list is manual.** `messages` re-exports the `registry::record` constants
  by name, so a new record type must be added in both places. A missing one fails to compile only
  where someone first uses `messages::NEW_RECORD`.

## Deliberate deviations

- **`must_use_candidate` is allowed.** `Result`-returning functions are already `#[must_use]`, and
  marking every pure accessor adds noise. Builder-style methods that return `Self` are marked.
- **`too_many_lines` is allowed.** Codec functions walk spec field tables in order, and splitting
  them would scatter that order.
- **Descriptive trait-parameter names.** `clippy.toml` permits `formatter`, `buffer`, and similar
  names in `fmt`, `io`, and `GlobalAlloc` implementations, instead of std's `f` and `buf`.
- **Many public modules (M-BALANCED-MODULES).** The crate root holds only version and size
  constants, and the 31 public modules mirror the specification's sections.
