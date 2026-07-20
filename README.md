# Vivid Protocol

Vivid is a secure, terminal-attached media protocol for displaying images and video and playing
audio inside a terminal. It keeps bulk media off the terminal PTY, so ordinary terminal text stays
separate from media transport.

The protocol has two roles: a producer creates media sources and supplies their data, while a
presenter owns the terminal window, authenticates producers, decodes media, manages placement, and
renders or plays the result. A private endpoint and per-window capability token protect each
session. Control connections handle capability negotiation, scene state, complete playback
requests, flow control, visibility, keepalive, and recovery; source-specific media connections
carry raster, image, video, or audio data. The PTY carries only normal terminal output and a
bounded authenticated text-anchor marker that can bind media placement to a semantic terminal
position. Local transports and SSH forwarding allow the same model to work for both local and
remote producers. An optional `VIVID_ENDPOINT_BULK` selects another private endpoint for
non-control connections without changing the wire protocol.

`vivid_protocol` is the shared, renderer-independent Rust wire implementation used by Vivi,
Vivido, conformance tools, and protocol tracers.

The crate provides:

- directional connection limits, ordered record framing, endpoint parsing, split
  `ConnectionReader`/cloneable `ConnectionWriter` handles, and the 64 MiB ceiling;
- deterministic, bounded CBOR, typed control-message schemas, complete `PlayRequest` parsing, and
  conservative RTT-based initial-buffer calculation;
- raw/zstd RGBA raster, straight or premultiplied alpha, and PNG/JPEG image bodies;
- portable H.264/HEVC/VP9/AV1 video and MP3/AAC/ALAC/PCM/Opus/Vorbis/FLAC audio access units,
  including media sequence and trim metadata validation;
- canonical OpusHead, Xiph-laced Vorbis-header, and raw FLAC STREAMINFO validators;
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

- `wire` — prefaces, records, split reader/writer handles, directional limits, sequencing, and
  transports;
- `cbor` — deterministic encoding and strict bounded decoding;
- `messages` — the Vivid registry, control schemas, `PlayRequest`, keepalive types, and canonical
  audio initialization validation;
- `media` — raster, image, portable-video, and portable-audio binary contracts;
- `anchor` — token decoding, session-key derivation, and text-anchor authentication.

## Python image demo

[`examples/vivid_image.py`](examples/vivid_image.py) is a self-contained producer that displays a
PNG or JPEG directly through Vivid Protocol. It uses only the Python standard library and shows the
complete handshake, authenticated text anchor, encoded-image source, scene transaction, media
channel, and credit flow.

Run it from a shell inside Vivido:

```sh
python3 examples/vivid_image.py path/to/image.png
python3 examples/vivid_image.py --scale 0.5 path/to/image.jpg
```

The demo can run directly in Vivido, through vvmux, or through `vvssh`. Generic terminal
multiplexers that do not preserve authenticated Vivid anchors are not supported.

## Compatibility

Vivid Protocol 1.1 deliberately retains the framing-1.0 `VIVD` preface. The Opus, Vorbis, and FLAC
packetizations extend the existing audio feature rather than allocating new feature IDs, so an
older presenter rejects unsupported configurations through the normal `CREATE_AUDIO` error path.
The crate declares Rust 1.85 compatibility.

`PLAY` carries start PTS, minimum buffer, maximum latency, 32.32 rate, late policy, loop count, and
start policy. `PING`/`PONG` are bidirectional correlated session records. Playback telemetry,
derived media tickets, audio batching, and alternate packet framing are not part of Vivid 1.1.

## License

Licensed under Apache-2.0.
