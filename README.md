# Vivid Protocol

[![Crates.io](https://img.shields.io/crates/v/vivid_protocol.svg)](https://crates.io/crates/vivid_protocol)
[![Docs.rs](https://docs.rs/vivid_protocol/badge.svg)](https://docs.rs/vivid_protocol)
[![License](https://img.shields.io/crates/l/vivid_protocol.svg)](LICENSE)

**Secure, renderer-independent media scenes—from terminals to browser desktops.**

Vivid is an open wire protocol for moving images, raster frames, encoded video, audio, and input
between producers and presenters. The destination can be a GPU terminal, a browser canvas, a
streamed desktop, a multiplexer, or your own renderer.

Terminal integration is one Vivid deployment mode, not a requirement. A terminal presenter can
anchor media beside text; a terminal-free presenter can attach the same retained scene directly to
its display root.

## Why Vivid?

- **One media model, many surfaces.** Native and WebAssembly implementations share the same
  versioned wire contract.
- **Media stays out of text streams.** Control and media use authenticated side channels instead
  of escape-sequence payloads.
- **Built for real playback.** Retained scenes, exact-PTS playback, linked audio/video, flow
  control, visibility, recovery, and observability are part of the protocol.
- **Secure by design.** Private endpoints, transcript authentication, bounded records, and
  track-scoped failure are core expectations—not application-specific extras.
- **Renderer and transport independent.** Implement a producer, presenter, relay, multiplexer, or
  language binding without adopting a particular UI stack.

## See what it enables

| Experience | Vivid path |
| --- | --- |
| Terminal-free streamed desktop | Veston or Vvsway → vvbridge → vvweb browser canvas |
| Rich terminal media | Vivi → Vivido |
| Browser terminal media | Vivid producer → vvmux_server/web |
| Detachable and nested sessions | Vivid producer → vvmux → Vivido |
| Custom applications | Your producer → your presenter |

The [vvweb demo](../vvweb/demo/) is the clearest terminal-free example: a native desktop producer
streams H.264 video and linked Opus audio through an authenticated WebSocket bridge, while the
browser renders the Vivid root scene and returns physical input. No terminal emulator, shell, or
PTY is involved.

## Choose your starting point

**Building an application producer?** Start with
[`vivid_sdk`](https://github.com/vivido-dev/vivid_sdk). It provides the higher-level Rust and
Python client APIs.

**Building a presenter, relay, protocol tool, or language binding?** Use this crate for the shared
wire implementation:

```sh
cargo add vivid_protocol
```

For a WebAssembly target:

```sh
cargo add vivid_protocol --no-default-features
```

The crate provides deterministic bounded CBOR, framing, profile and numeric registries,
root/lease/resume authentication, finite resource accounting, stable surface and scene state,
immutable track and channel-generation state, final-gated desktop input, portable media record
layouts, and authenticated terminal anchors. Native builds also include metadata-only protocol
tracing. The crate contains no renderer and does not choose your application architecture.

API documentation is on [docs.rs](https://docs.rs/vivid_protocol).

## Protocol in 30 seconds

```text
producer ── control + lanes + per-track channels ──> presenter ──> any render surface
```

A producer creates stable surfaces, attaches immutable media tracks, and commits retained scene
updates. A presenter validates, buffers, schedules, and renders them. Track replacement does not
change surface, scene, or input identity. Terminal anchors are available when text-relative
placement is useful; they are absent from terminal-free root-scene deployments.

For record layouts, state machines, profile negotiation, security requirements, and
interoperability rules, read the normative multipart
**[Vivid Protocol 1.5 specification](vivid-protocol-1.5-spec.md)**. Implementers migrating from
the retired source/ticket/credit model should also read the
**[1.1 to 1.5 migration guide](vivid-protocol-1.1-to-1.5-migration.md)**.

## Compatibility

This crate implements Vivid Protocol 1.5 and requires Rust 1.87 or newer. Vivid 1.5 is not
wire-compatible with Vivid 1.1. Protocol support is negotiated by coherent named profiles; the
preface selects the exact wire version.

## Contributing

New producers, presenters, transports, language bindings, and interoperability tests are welcome.
If you are exploring a new Vivid surface, open an issue early—we would like to help make the
integration reusable.

## License

Apache-2.0. See [LICENSE](LICENSE).
