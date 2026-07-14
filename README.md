# Vivid Protocol

Vivid Protocol provides the shared wire-format implementation for the Vivid terminal media
protocol. It is intended for terminal presenters, media producers, conformance tools, and protocol
tracers that need to exchange Vivid records without depending on a renderer or terminal emulator.

The crate currently provides:

- Vivid connection prefaces, record headers, ordered framing, endpoint parsing, and limits;
- deterministic CBOR encoding with strict bounded decoding;
- the Vivid 1.0 opcode, feature, error, and configuration registries;
- control-message encoders and parsers;
- full-frame RGBA raster and encoded-video packet layouts.

```toml
[dependencies]
vivid_protocol = "0.1"
```

```rust
use vivid_protocol::wire::{ConnectionKind, Preface, encode_preface};

let bytes = encode_preface(ConnectionKind::Control, 1024 * 1024);
let preface = Preface::decode(bytes)?;
assert_eq!(preface.kind, ConnectionKind::Control);
# Ok::<(), std::io::Error>(())
```

The public modules are organized by protocol plane:

- `wire` — connections, prefaces, headers, sequencing, and record framing;
- `cbor` — deterministic control-value encoding and bounded decoding;
- `messages` — numeric registries plus control-message schemas;
- `media` — raster-frame and video-packet binary layouts.

The crate follows the Vivid protocol version exposed by `PROTOCOL_MAJOR` and `PROTOCOL_MINOR`.
Before Vivid 1.0 stabilizes, `0.x` releases may make breaking API or wire-profile corrections.

## License

Licensed under Apache-2.0.
