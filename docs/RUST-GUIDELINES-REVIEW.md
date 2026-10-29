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

The sections below record how the remaining findings were resolved and which deviations are
deliberate. Guideline IDs such as `M-INIT-CASCADED` name the rule each one comes from.

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

Every public item is documented, and `Cargo.toml` enables `missing_docs`. Wire fields name the
CBOR key or byte offset their specification table assigns, and constants name their bit or code.
Every fallible public function documents its errors, and every function that can panic says when,
so `clippy::missing_errors_doc` and `clippy::missing_panics_doc` are enforced.

The crate root has a module map and a producer and presenter quick start that runs as a doctest.
`Connection`, `Hello`, `cbor`, `media::raster_frame_body`, and `NativeDiscovery` have examples,
and each module's `//!` documentation names its main types and specification sections.

## Long positional parameter lists (M-INIT-CASCADED)

Public functions that took five or more positional parameters now take a message or a small
named-field struct, so integers cannot be swapped silently:

- media builders and validators take `RasterFrame`, `RasterDeltaFrame`, `RasterSize`, and `AudioInitialization`;
- `trace::TraceEmitter::emit` takes a `TraceEvent`;
- `lease::LeaseMachine::begin_activation` and `begin_resume` take an `EstablishmentAttempt`;
- `geometry::fit_quad` takes a `SourceSize`, and `surface::SurfaceReady` encodes its own payload;
- `track::TrackState::admit_media` takes `ReceivedMedia`, and `ChannelOpenState::open` a
  `ChannelOpenAttempt`;
- the resume proof is `Hello::authenticate_resume` and `Hello::verify_resume`, and the lane tag is
  `LaneOpen::sign` and `LaneOpen::verify`.

Keys are typed too: `ChannelKey` and `ResumeKey` cannot be passed for each other, and
`HandshakeNonces` names the client and server nonce. No `#[expect(clippy::too_many_arguments)]`
remains. `PathBuilder::cubic_to` and `quad_to`, `vector::Rect::new`, and `geometry::FixedRect::new`
keep the coordinate lists every graphics API uses.

## Smaller items

- **The trace clock is mockable (M-MOCKABLE-SYSCALLS).** Trace guards take a `TraceClock`, and
  `TraceClock::new` accepts any time source, so tests need neither the filesystem nor real time.
- **Record and error codes have one public path (M-SINGLE-ITEM-PATH).** The registry's `record`
  and `error` modules are crate-private, and `messages` re-exports them by name.
  `tests/registry.rs` fails when a live record or error code is missing from `messages`.

## Deliberate deviations

- **`must_use_candidate` is allowed.** `Result`-returning functions are already `#[must_use]`, and
  marking every pure accessor adds noise. Builder-style methods that return `Self` are marked.
- **`too_many_lines` is allowed.** Codec functions walk spec field tables in order, and splitting
  them would scatter that order.
- **Descriptive trait-parameter names.** `clippy.toml` permits `formatter`, `buffer`, and similar
  names in `fmt`, `io`, and `GlobalAlloc` implementations, instead of std's `f` and `buf`.
- **Many public modules (M-BALANCED-MODULES).** The crate root holds only version and size
  constants, and the 31 public modules mirror the specification's sections.
