# Vivid Protocol

Vivid is a secure, terminal-attached media protocol for displaying images and video and playing
audio inside a terminal. It keeps bulk media off the terminal PTY, so ordinary terminal text stays
separate from media transport.

The protocol has two roles: a producer creates media sources and supplies their data, while a
presenter owns the terminal window, authenticates producers, decodes media, manages placement, and
renders or plays the result. A private endpoint and per-window capability token protect each
session. Control connections handle capability negotiation, scene state, playback, flow control,
visibility, and recovery; source-specific media connections carry raster, image, video, or audio
data. The PTY carries only normal terminal output and a bounded authenticated text-anchor marker
that can bind media placement to a semantic terminal position. Local transports and SSH forwarding
allow the same model to work for both local and remote producers.

`vivid_protocol` is the shared, renderer-independent Rust wire implementation used by Vivi,
Vivido, conformance tools, and protocol tracers.

The crate provides:

- directional connection limits, ordered record framing, endpoint parsing, and the 64 MiB ceiling;
- deterministic, bounded CBOR and typed control-message schemas;
- raw/zstd RGBA raster, straight or premultiplied alpha, and PNG/JPEG image bodies;
- portable H.264/HEVC/VP9/AV1 video and MP3/AAC/ALAC/PCM audio access units, including media
  sequence and trim metadata validation;
- authenticated text anchors using base64url and HMAC-SHA256.

```sh
cargo add vivid_protocol
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
- `messages` — the Vivid registry and control schemas;
- `media` — raster, image, portable-video, and portable-audio binary contracts;
- `anchor` — token decoding, session-key derivation, and text-anchor authentication.

## License

Licensed under Apache-2.0.
