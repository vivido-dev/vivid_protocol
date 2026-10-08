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
rule each one comes from. `Cargo.toml` opts out of the lints for the first three until they are
done, and each `#[expect]` reason in the source points back here.

## Errors are `io::Error` with string messages (M-ERRORS-CANONICAL-STRUCTS)

Pure codecs report malformed input as `io::Error` with `InvalidData` and a formatted message:
about 117 signatures in `media`, `file_drop`, `timed`, `revision`, `audio_input`, and `wire`.
Callers can only tell causes apart by matching strings.

`io::Error` is right for `wire`, which does real I/O. The codecs should return typed errors
instead. Every public error type now implements `Display` and `Error`; `CompletionError` and
`LeaseTransitionError` were the last two. The remaining gaps all break callers, so they wait for
the next semver-major release:

- **Typed codec errors.** Replacing `io::Result` changes every signature that `vivid_sdk`,
  `vivid_gateway`, and `vivido` call.
- **`#[non_exhaustive]`.** `MessageError`, `AuthError`, `ResourceError`, `ProfileError`,
  `SizeError`, `CompletionError`, and `LeaseTransitionError` lack it. Adding it breaks any
  exhaustive `match`, so adding a variant later breaks callers too.
- **`SizeError`'s `Display` prints the variant name** (`TooLarge`). `vivi` reports oversized
  images through `io::Error::other(SizeError)` and its tests match on that text, which
  `tests/errors.rs` pins. Change both together.
- **`MessageError::Cbor` holds a `String`**, not the `cbor::DecodeError`, so `source()` cannot
  chain to it.

About 128 `map_err(|_| ...)` calls deliberately replace an integer-conversion error with a
labelled protocol error, so `clippy::map_err_ignore` is allowed until this redesign. The change
reaches every signature that `vivid_sdk`, `vivid_gateway`, and `vivido` call.

## Documentation (M-CANONICAL-DOCS, M-MODULE-DOCS, M-DOCUMENTED-MAGIC)

- 347 public functions that return `Result` have no `# Errors` section, and 13 that can panic have
  no `# Panics` section. `clippy::missing_errors_doc` and `clippy::missing_panics_doc` are allowed
  until they do.
- `missing_docs` reports 2,144 undocumented public items, counting fields and variants.
- `cbor.rs`, `media.rs`, and `wire.rs` have no `//!` module documentation.
- In `lib.rs`, `VIVID_MINOR` and `CONTROL_MAX_RECORD_BODY` have no doc comment.
  `DEFAULT_MAX_RECORD_BODY` and `HARD_MAX_RECORD_BODY` are both 64 MiB, and nothing explains why
  the crate keeps two.
- The `cbor.rs` decoder limits (`MAX_DEPTH`, `MAX_VALUE_LENGTH`, `MAX_CONTAINER_LENGTH`) don't say
  how they were chosen.

To reproduce the counts:

```sh
cargo clippy --lib -- -W clippy::missing_errors_doc -W clippy::missing_panics_doc -W missing_docs
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

- **Trace I/O is not mockable (M-MOCKABLE-SYSCALLS).** `trace.rs` opens files and reads
  `Instant::now()` directly. It is diagnostics only, so this matters less than discovery did.
- **`Endpoint::parse` echoes its input.** The unsupported-scheme error includes the rejected value
  (`wire.rs`), while `NativeDiscovery`'s `Debug` redacts endpoints. Drop the value from the message
  if endpoints are sensitive.
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
