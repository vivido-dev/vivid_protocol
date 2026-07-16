# Vivid Protocol

`vivid_protocol` is the shared wire implementation for the Vivid 1.1 terminal-media protocol. It
is used by Vivi, Vivido, conformance tools, and protocol tracers without depending on a renderer.

The protocol selected by `HELLO`/`WELCOME` is 1.1 only. The 16-byte connection preface deliberately
remains framing version 1.0, as required by the 1.1 specification.

The crate provides:

- directional connection limits, ordered record framing, endpoint parsing, and the 64 MiB ceiling;
- deterministic, bounded CBOR and typed 1.1 control-message schemas;
- raw/zstd RGBA raster, straight or premultiplied alpha, and PNG/JPEG image bodies;
- portable H.264/HEVC/VP9/AV1 access-unit validation and media sequence checking;
- authenticated marker-v2 anchors using base64url and HMAC-SHA256.

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

Public modules:

- `wire` — prefaces, records, directional limits, sequencing, and transports;
- `cbor` — deterministic encoding and strict bounded decoding;
- `messages` — the Vivid 1.1 registry and control schemas;
- `media` — raster, image, and portable-video binary contracts;
- `anchor` — token decoding, session-key derivation, and marker-v2 authentication.

## License

Licensed under Apache-2.0.
