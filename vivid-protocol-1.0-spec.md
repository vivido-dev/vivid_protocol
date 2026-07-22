# Vivid Protocol 1.0 Specification

**Status:** normative Vivi/Vivido interoperable profile
**Vivid version:** 1.0
**Compatibility:** Vivid 1.0 only

## 1. Conventions and scope

The key words **MUST**, **MUST NOT**, **REQUIRED**, **SHALL**, **SHALL NOT**, **SHOULD**, **SHOULD NOT**, **RECOMMENDED**, **NOT RECOMMENDED**, **MAY**, and **OPTIONAL** are normative requirement levels.

Vivid is a terminal-attached media protocol. It keeps bulk media off the terminal PTY while allowing authenticated producers to create media sources, place those sources in a terminal-owned scene, and bind placements to semantic text positions.

Vivid 1.0 defines:

- endpoint discovery, local Unix-stream transport, and SSH forwarding;
- connection preface and record framing;
- deterministic control encoding and request correlation;
- session, source, scene, playback, credit, visibility, and anchor state machines;
- raw and zstd-compressed full-frame raster media;
- portable encoded-video access units;
- portable encoded-audio access units with linked A/V playback;
- encoded PNG/JPEG still images;
- authenticated text-anchor marker version 2;
- error isolation and aggregate quotas.

Assigned operations without a payload schema remain reserved for a negotiated future profile. Assignment alone does not require implementation support.

### 1.1 Roles

A **presenter** owns a terminal window, authenticates producers, decodes media, maintains scene state, and renders frames.

A **producer** creates sources and nodes and supplies media.

A **control connection** owns one producer session. A **media connection** carries media for exactly one source after ticket attachment.

Media bytes MUST NOT be transported through the terminal PTY. The PTY is used only for ordinary terminal data and the bounded text-anchor marker in Section 13.

### 1.2 Vivid version

The 16-byte connection preface identifies Vivid version `1.0`; connections write preface bytes major `1`, minor `0`.

The same Vivid version is offered by `HELLO` and selected by `WELCOME`.

A Vivid 1.0 producer MUST offer a range containing version 1.0. A conforming presenter selects only version 1.0 and rejects a range that does not contain it. `WELCOME` keys 13 through 15 are mandatory; their absence is `BAD_MESSAGE`, not a downgrade signal.

A producer MUST NOT use an optional feature unless the presenter advertises the corresponding accepted feature or profile.

### 1.3 Byte order, integer widths, and units

All fixed-width multibyte integers are big-endian.

Unless a narrower width is stated:

- `uint` means an integer in `0..=2^64-1`;
- `int` means an integer in `-2^63..=2^63-1`.

A value outside its declared width is `BAD_MESSAGE`.

Pixel dimensions are unsigned integer pixels. Timeline fields ending in `_us` are integer microseconds.

Scene geometry is signed 32.32 fixed point stored in an `i64`, where one terminal cell is `1 << 32`. Width and height MUST be positive. Implementations MUST use checked arithmetic for conversion, clipping, and composition; overflow is `BAD_MESSAGE` or `LIMIT_EXCEEDED`, not wraparound.

## 2. Discovery and transport

### 2.1 Environment discovery

A presenter supplies these variables to its child shell:

| Variable | Meaning |
|---|---|
| `VIVID_ENDPOINT` | Private reliable-stream endpoint |
| `VIVID_ENDPOINT_BULK` | Optional private endpoint for non-control media connections |
| `VIVID_TOKEN` | Per-window 256-bit capability encoded as exactly 64 hexadecimal characters |

Uppercase and lowercase hexadecimal are accepted. Whitespace is not accepted.

The endpoint forms are:

| Form | Meaning |
|---|---|
| `unix:/absolute/path` | Unix-domain stream socket |
| `/absolute/path` | Bare Unix-domain stream socket path |
| `tcp:host:port` | Trusted development TCP transport, or the exact Windows loopback local profile below |

The Unix-domain binding is REQUIRED for the Unix local profile. The endpoint directory and socket
MUST be private to the terminal user. A native Windows presenter MAY instead advertise exactly
`tcp:127.0.0.1:<port>` with an ephemeral nonzero port. It MUST bind only IPv4 `127.0.0.1`, reject
every peer address other than exact IPv4 loopback, authenticate a distinct 256-bit capability
before allocating presenter state, and apply a bounded deadline through the preface and initial
`HELLO` or `ATTACH_CHANNEL`. Host names, IPv6, wildcard addresses, and non-loopback addresses are
not part of this Windows local profile.

When `VIVID_ENDPOINT_BULK` is present, the producer uses `VIVID_ENDPOINT` for the control
connection and the bulk endpoint for video, raster, image, and audio connections. The two
endpoints address the same authenticated presenter session and use the same protocol framing.
Failure to establish the bulk transport MAY fall back to `VIVID_ENDPOINT` only before sending
`ATTACH_CHANNEL`. A ticket MUST NOT be retried after an attachment attempt because it may already
have been consumed.

Where the operating system exposes peer credentials for Unix-domain streams, the presenter MUST verify that the connecting peer has the same effective user identity as the presenter owner. Failure is `AUTH_FAILED` and closes the connection.

The TCP form provides no Vivid-level confidentiality, integrity, or network authentication. It MUST NOT be exposed to an untrusted network.

### 2.2 Capability handling

The capability token MUST NOT be placed in command-line arguments, diagnostic output, shell history, or logs. A producer SHOULD read it from the inherited environment, copy it into protected process memory, and remove it from any environment passed to unrelated child processes.

Token comparison MUST be constant-time after exact-length hexadecimal decoding.

### 2.3 SSH binding

Remote producers use an SSH remote stream-local forward. On Unix clients both ends are Unix
sockets; on Windows the local destination is the presenter's loopback TCP endpoint:

```text
remote private Unix socket -> SSH channel -> local presenter Unix socket
remote private Unix socket -> SSH channel -> local 127.0.0.1 TCP endpoint (Windows)
```

The remote socket MUST be owner-only. Each remote socket connection becomes an independent SSH channel.

The local capability token MUST be transferred inside the authenticated SSH session using an SSH environment request or a protected standard-input/file-descriptor channel. It MUST NOT be embedded in a remote command line.

The reference `vvssh` binding sets:

```text
ExitOnForwardFailure=yes
StreamLocalBindMask=0177
StreamLocalBindUnlink=yes
VIVID_REMOTE=1
VIVID_ANCHOR_TRANSPORT=conpty  # Windows presenter only
```

It accepts a TCP destination only when the host is exactly `127.0.0.1`, and exports
`VIVID_REMOTE=1` in the remote Linux login shell. A Windows `vvssh` also exports
`VIVID_ANCHOR_TRANSPORT=conpty`; the producer then uses the bounded ConPTY marker envelope and does
not synchronously wait for `ANCHOR_READY` before submitting its scene transaction.

As an opt-in transport policy, `vvssh` may establish a second lifecycle-bound SSH TCP connection
with a second private remote stream-local socket and export it as `VIVID_ENDPOINT_BULK`. That
helper MUST NOT reuse an OpenSSH control master, and its process and socket MUST be cleaned up with
the foreground session. This changes discovery and transport only; it does not define a second
control session or a new Vivid wire feature. The default SSH path remains one SSH connection.

No Vivid record may be embedded in terminal text as a fallback.

### 2.4 WebSocket binding

A browser presenter or producer cannot open Unix-domain or loopback-TCP streams. It reaches a
Vivid endpoint through a bridge that terminates WebSocket connections and relays bytes to the
local transport:

```text
browser <-WS(S)-> bridge <-Unix socket / loopback TCP-> peer
```

The binding changes discovery and transport only; it defines no new Vivid wire feature:

- Exactly one WebSocket connection maps to exactly one Vivid connection (the control connection,
  plus one per attached media channel). The bridge relays bytes verbatim in both directions,
  starting with the initiator's 16-byte preface. The non-initiating endpoint does not send a
  reciprocal preface; the WebSocket binding does not change the section 3.1 rule.
- Binary WebSocket messages carry the Vivid byte stream. Message boundaries are not significant;
  the receiver MUST reassemble the stream and honor record framing exactly as on a native stream.
  Text messages, fragment-level interpretation, and WebSocket-level compression contexts carrying
  protocol state are not part of the binding.
- When `VIVID_ENDPOINT_BULK` is present the bridge exposes it the same way; the bulk fallback rule
  in section 2.1 applies unchanged, including the prohibition on retrying a possibly consumed
  ticket.
- `VIVID_TOKEN` MUST NOT be delivered to the browser, embedded in a URL, or logged. The bridge
  holds the capability, injects it into `HELLO` on the browser's behalf or performs the `HELLO`
  itself, and applies the section 2.2 handling rules. Browser-to-bridge authentication is outside
  Vivid scope, but the bridge MUST authenticate the browser session, MUST validate the WebSocket
  origin, and MUST use TLS whenever the WebSocket crosses a host boundary.
- The bridge is the trust boundary: it enforces the same peer checks as a local presenter and
  MUST NOT expose the underlying endpoint to untrusted networks (section 2.1).

### 2.5 Stream and write behavior

A Vivid transport MUST provide a reliable, ordered, full-duplex byte stream. Vivid framing supplies message boundaries.

Generic transport compression is not part of Vivid 1.0.

A record is a logical stream unit, not a requirement for one operating-system write. Producers SHOULD write large media bodies in bounded batches and SHOULD avoid building an unbounded socket write queue. An SSH binding MAY use a separate SSH connection for bulk media when interactive-input latency is more important than connection reuse.

### 2.6 Terminal multiplexers

Text anchors require all of the following:

- the current terminal client’s endpoint and token reach the producer;
- APC bytes are preserved without rewriting;
- the marker arrives at the same presenter that issued the session tag.

When tmux, screen, or another multiplexer cannot guarantee those properties, the producer MUST NOT emit Vivid anchor markers. It MAY continue with ordinary grid-cell nodes. Multiplexer passthrough is outside the Vivid 1.0 interoperable profile.

## 3. Connection framing

### 3.1 Initiator preface

The producer opens every Vivid connection and writes exactly one 16-byte preface before its first record.

| Offset | Size | Field |
|---:|---:|---|
| 0 | 4 | ASCII `VIVD` |
| 4 | 1 | Vivid major version; `1` |
| 5 | 1 | Vivid minor version; `0` |
| 6 | 1 | Connection kind |
| 7 | 1 | Flags; MUST be zero |
| 8 | 4 | Initiator transmit-body limit |
| 12 | 4 | Reserved; MUST be zero |

The presenter does not send a reciprocal preface.

The transmit-body limit is the largest record body the initiator will transmit on that connection. It MUST be nonzero and MUST NOT exceed 67,108,864 bytes. The receiver may impose a lower limit.

Connection kinds are:

| Value | Name | Vivid 1.0 use |
|---:|---|---|
| 0 | Control | REQUIRED |
| 1 | Video | Used by `CREATE_VIDEO` |
| 2 | Raster | Used by `CREATE_RASTER` |
| 3 | Blob | Used by `CREATE_IMAGE` when `ENCODED_IMAGE_V1` is negotiated |
| 4 | Local buffer | Reserved |
| 5 | Audio | Used by `CREATE_AUDIO` when `AUDIO_ACCESS_UNIT_V1` is negotiated |

An unsupported Vivid version closes the connection. Reserved preface fields or flags that are nonzero are framing errors.

### 3.2 Record header

Every record has a 24-byte header followed by exactly `body_length` bytes.

| Offset | Size | Field |
|---:|---:|---|
| 0 | 4 | Body length |
| 4 | 2 | Record type |
| 6 | 2 | Record flags |
| 8 | 8 | Object ID |
| 16 | 8 | Sequence number |

Sequence numbers are independent in each direction on each connection. The first record in a direction has sequence `1`; every subsequent record increments by one. Gaps, duplicates, reordering, and exhaustion are fatal framing errors for that connection.

Object ID zero denotes session-level traffic. For source, node, transaction, and anchor operations, any object ID and payload ID required to match MUST match exactly.

A receiver MUST validate body length against all effective ceilings before allocating or dispatching the body.

### 3.3 Directional body ceilings

The effective body ceiling is directional.

For producer-to-presenter control records it is the minimum of:

- the control-connection preface transmit-body limit;
- `WELCOME` key 11;
- the 1 MiB control-profile ceiling;
- the 64 MiB hard ceiling.

For presenter-to-producer control records it is the minimum of:

- `HELLO` key 9;
- the 1 MiB control-profile ceiling;
- the 64 MiB hard ceiling.

For producer-to-presenter media records it is the minimum of:

- the media-connection preface transmit-body limit;
- `SOURCE_READY` key 5;
- available byte credit;
- the 64 MiB hard ceiling.

Record headers do not count toward body ceilings or byte credit.

### 3.4 Record flags and unknown records

Record flag bit 0 (`0x0001`) is `OPTIONAL`. Bits 1 through 15 are reserved and MUST be zero.

An unknown record with `OPTIONAL` set is consumed and ignored without state mutation. An unknown required record on a valid control connection produces `UNSUPPORTED_FEATURE`. An unknown required record on a media connection closes that media connection and makes the source lost. Reserved record-flag bits are a framing error.

A sender MUST NOT mark a request `OPTIONAL` if correctness depends on receiving a reply.

## 4. Deterministic control encoding

Control and event bodies contain exactly one deterministic CBOR value with this envelope:

```text
{
  0: uint,             # request ID; zero for unsolicited events
  ? 1: uint,           # transaction ID
  ? 2: uint,           # expected display generation
  3: { * uint => any } # opcode-specific payload
}
```

The constrained CBOR profile is:

- definite-length byte strings, text strings, arrays, and maps only;
- unsigned and signed integers in shortest form;
- map keys are unsigned integers in strictly increasing order;
- UTF-8 text;
- booleans and null;
- no tags, floating-point values, other simple values, or trailing bytes;
- maximum nesting depth 16;
- maximum byte-string or text-string length 16 MiB;
- maximum array or map length 4,096 entries.

The control-record ceiling normally imposes a lower effective value limit.

Duplicate, non-integer, or unsorted map keys are `BAD_MESSAGE`. Unknown payload keys MAY be ignored unless a message schema states otherwise. Missing required keys are `BAD_MESSAGE`.
Source-creation schemas, decoder-configuration schemas, and resource-allocation schemas are strict:
an unknown key is `BAD_MESSAGE` unless a negotiated profile explicitly assigns and permits it.
Schemas that explicitly allow ignorable future keys retain that behavior.

### 4.1 Requests, replies, and pipelining

A request that expects correlation uses a nonzero request ID. A producer MUST NOT reuse a request ID while any reply or error for the earlier request can still arrive.

Unsolicited presenter events use request ID zero. Replies copy the request ID of the request they answer.

A producer MAY pipeline requests without waiting for earlier replies when it already possesses all values needed to construct them. A presenter MUST apply state-mutating control records in receive order on a control connection. Replies MAY be emitted after independent work completes and therefore MAY be observed in a different order; producers correlate by request ID.

The following dependencies cannot be bypassed by pipelining:

- `HELLO` must be accepted before any session operation;
- a media ticket is unavailable until `SOURCE_READY`;
- an operation that requires a returned identifier or capability must wait for that value.

## 5. Session establishment

### 5.1 Control ordering

The first control record MUST be session-level `HELLO` with object ID zero. The presenter validates framing, version range, required features, capability token, and local peer identity before creating producer-controlled session resources.

Authentication failure returns `AUTH_FAILED` when safe to do so and ends establishment.

On success the presenter returns `WELCOME`. The session remains active until `GOODBYE`, control EOF, or a fatal control error. Control loss invalidates unused media tickets, closes attached media connections, and applies the anchor/poster lifecycle in Section 13.

Both endpoints MUST service the control stream continuously and independently of media writes,
credit availability, decoding, and presentation. In particular, a conforming implementation
handles credits, visibility, keyframe requests, source loss, display changes, correlated replies,
and `PING` without waiting for the next media operation.

### 5.2 `HELLO`

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Minimum Vivid major version |
| 1 | uint | Minimum Vivid minor version |
| 2 | uint | Maximum Vivid major version |
| 3 | uint | Maximum Vivid minor version |
| 4 | text | Capability token, exactly 64 hexadecimal characters |
| 5 | text | Producer name, at most 256 UTF-8 bytes |
| 6 | text | Producer version, at most 128 UTF-8 bytes |
| 7 | array(uint) | Required feature IDs, strictly increasing and unique |
| 8 | array(uint) | Optional feature IDs, strictly increasing and unique |
| 9 | uint | Maximum control-record body accepted from presenter |

The offered range MUST contain at least one Vivid version supported by the presenter. An unsupported required feature fails establishment with `UNSUPPORTED_FEATURE`. Unsupported optional features do not fail establishment.

### 5.3 `WELCOME`

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Session ID |
| 1 | bytes(16) | Session tag |
| 2 | uint | Private root context ID |
| 3 | uint | Capability generation |
| 4 | uint | Display generation |
| 5 | uint | Viewport width in pixels |
| 6 | uint | Viewport height in pixels |
| 7 | uint | Grid columns |
| 8 | uint | Grid rows |
| 9 | uint | Cell width in pixels |
| 10 | uint | Cell height in pixels |
| 11 | uint | Maximum control-record body accepted by presenter |
| 12 | array(text) | Accepted profile names, sorted and unique |
| 13 | uint | Selected Vivid major version |
| 14 | uint | Selected Vivid minor version |
| 15 | array(uint) | Accepted feature IDs, sorted and unique |

Keys 13 through 15 are REQUIRED.

Display values are authoritative for the returned display generation.

Recommended profile names are:

```text
raster-rgba8-full-v1
raster-zstd-full-v1
image-png-jpeg-v1
video-access-unit-v1
audio-access-unit-v1
text-anchor-cell-v2
visibility-source-v1
node-clip-rect-v1
```

### 5.4 Feature registry

| ID | Name | Status |
|---:|---|---|
| 1 | `RASTER_RGBA8` | Baseline |
| 2 | Retired | MUST NOT be negotiated |
| 3 | `SCENE_TRANSACTIONS` | Baseline |
| 4 | `GRID_CELL_NODES` | Baseline |
| 5 | `CREDIT_FLOW_CONTROL` | Baseline |
| 6 | Retired | MUST NOT be negotiated |
| 7 | `ENCODED_IMAGE_V1` | Optional |
| 8 | `RASTER_ZSTD_V1` | Optional |
| 9 | `RASTER_PREMULTIPLIED_ALPHA` | Optional |
| 10 | `VISIBILITY_EVENTS_V1` | Optional |
| 11 | `VIDEO_ACCESS_UNIT_V1` | Optional |
| 12 | `VIDEO_CONTROL_V1` | Optional; `PAUSE`, `FLUSH`, `NEED_KEYFRAME` |
| 13 | `TEXT_ANCHORS_V2` | Required for authenticated anchors |
| 14 | `AUDIO_ACCESS_UNIT_V1` | Optional; encoded audio and `DRAIN` |
| 15 | `NODE_CLIP_RECT_V1` | Optional; exact cell-space node clipping |
| 16 | `DECODER_DESCRIPTION_V1` | Optional; decoder-ready codec description fields |

Feature IDs 17 and 18 are unassigned in Vivid 1.0 and MUST NOT be negotiated. The portable
Opus, Vorbis, and FLAC forms below extend `AUDIO_ACCESS_UNIT_V1`; they do not allocate new feature
IDs.

## 6. Record-type registry

Numeric assignments are normative.

| Value | Name | Value | Name |
|---|---|---|---|
| `0x0001` | `HELLO` | `0x0002` | `WELCOME` |
| `0x0003` | `OK` | `0x0004` | `ERROR` |
| `0x0005` | `PING` | `0x0006` | `PONG` |
| `0x0007` | `GOODBYE` | `0x0008` | `DISPLAY_CHANGED` |
| `0x0009` | `CAPS_CHANGED` | `0x0100` | `PROBE_VIDEO_CONFIG` |
| `0x0101` | `VIDEO_SUPPORT` | `0x0102` | `CREATE_IMAGE` |
| `0x0103` | `CREATE_VIDEO` | `0x0104` | `CREATE_RASTER` |
| `0x0105` | `SOURCE_READY` | `0x0106` | `RECONFIGURE_SOURCE` |
| `0x0107` | `DESTROY_SOURCE` | `0x0108` | `SOURCE_LOST` |
| `0x0109` | `PROBE_AUDIO_CONFIG` | `0x010a` | `AUDIO_SUPPORT` |
| `0x010b` | `CREATE_AUDIO` |  |  |
| `0x0200` | `BEGIN_TXN` | `0x0201` | `CREATE_NODE` |
| `0x0202` | `UPDATE_NODE` | `0x0203` | `DELETE_NODE` |
| `0x0204` | `COMMIT_TXN` | `0x0205` | `ABORT_TXN` |
| `0x0206` | `PRESENTED` | `0x0207` | `ANCHOR_READY` |
| `0x0208` | `ANCHOR_GONE` | `0x0209` | `BARRIER_REACHED` |
| `0x0300` | `PLAY` | `0x0301` | `PAUSE` |
| `0x0302` | `STEP` | `0x0303` | `FLUSH` |
| `0x0304` | `DRAIN` | `0x0305` | `EOS` |
| `0x0306` | `PLAYBACK_STATE` | `0x0400` | `CREDIT` |
| `0x0401` | `FEEDBACK` | `0x0402` | `VISIBILITY` |
| `0x0403` | `QUALITY_HINT` | `0x0404` | `NEED_KEYFRAME` |
| `0x0500` | `BLOB_OFFER` | `0x0501` | `BLOB_HAVE` |
| `0x0502` | `BLOB_NEED` | `0x0503` | `BLOB_COMPLETE` |
| `0x0504` | `CACHE_EVICTED` | `0x0600` | `CREATE_CONTEXT` |
| `0x0601` | `DELEGATE_CONTEXT` | `0x0602` | `REVOKE_CONTEXT` |
| `0x0603` | `CONTEXT_CHANGED` | `0x8000` | `ATTACH_CHANNEL` |
| `0x8001` | `VIDEO_PACKET` | `0x8002` | `VIDEO_FRAGMENT` |
| `0x8003` | `RASTER_FRAME` | `0x8004` | `BLOB_CHUNK` |
| `0x8005` | `BUFFER_SUBMIT` | `0x8006` | `IMAGE_DATA` |
| `0x8007` | `AUDIO_PACKET` |  |  |

Vivid 1.0 defines complete schemas for:

```text
HELLO WELCOME OK ERROR PING PONG GOODBYE DISPLAY_CHANGED
PROBE_VIDEO_CONFIG VIDEO_SUPPORT PROBE_AUDIO_CONFIG AUDIO_SUPPORT
CREATE_IMAGE CREATE_VIDEO CREATE_AUDIO CREATE_RASTER
SOURCE_READY DESTROY_SOURCE SOURCE_LOST
BEGIN_TXN CREATE_NODE UPDATE_NODE DELETE_NODE COMMIT_TXN ABORT_TXN PRESENTED
ANCHOR_READY ANCHOR_GONE
PLAY PAUSE FLUSH DRAIN EOS CREDIT VISIBILITY NEED_KEYFRAME
ATTACH_CHANNEL VIDEO_PACKET AUDIO_PACKET RASTER_FRAME IMAGE_DATA
```

All other assigned operations require a future negotiated profile.

Extension ranges remain:

```text
0x7000-0x7fff  standards-track negotiated extensions
0x9000-0xbfff  experimental negotiated extensions
0xc000-0xffff  vendor-specific extensions, disabled unless explicitly negotiated
```

## 7. Control payload schemas

Unless stated otherwise, payload maps use envelope key 3 and contain only the keys below plus ignorable future keys. Unless a message defines another success reply, a successful correlated request returns `OK`.

### 7.1 Session and display

| Message | Payload |
|---|---|
| `OK` | Empty map |
| `PING` | Empty map |
| `PONG` | Empty map; request ID matches `PING` |
| `GOODBYE` | Empty map; presenter replies `OK` and closes the session |
| `DISPLAY_CHANGED` | `0` generation, `1` viewport width, `2` viewport height, `3` grid columns, `4` grid rows, `5` cell width, `6` cell height |
| `ERROR` | `0` error code, `1` failed request ID, `4` fatal boolean, `5` UTF-8 diagnostic |

`ERROR` payload keys 2 and 3 are reserved and MUST NOT be emitted in Vivid 1.0. Diagnostic text MUST NOT exceed 4,096 UTF-8 bytes and MUST NOT be parsed as protocol state.

Either endpoint MAY send `PING`; the peer MUST promptly answer with `PONG` using the same request
ID. Any valid inbound control record proves liveness. A reference implementation sends one probe
after 15 seconds without inbound control activity and treats three consecutive unanswered idle
probes as `TIMEOUT`. It estimates round-trip time only from clean `PING`/`PONG` samples and uses an
EWMA with weight 1/8 for the new sample.

An endpoint SHOULD also sample round-trip time with occasional `PING` probes while the connection
is active, at a bounded rate no faster than one outstanding probe per second, so that RTT-derived
values such as the `PLAY` minimum buffer reflect the live transport rather than a static guess.
Sampling probes MUST NOT change liveness accounting, and the clean-sample rule above still
applies.

`DISPLAY_CHANGED` is unsolicited and uses request ID zero. Display resize invalidates geometry assumptions, not source pixel dimensions. A producer normally updates node placement; it recreates a source only if it independently chooses a different source resolution.

A commit against stale display geometry receives the current `DISPLAY_CHANGED` followed by `STALE_DISPLAY_GENERATION`; no mutation is applied.

### 7.2 Video configuration

`PROBE_VIDEO_CONFIG` and `CREATE_VIDEO` use the same payload. A probe uses source ID zero; creation uses a nonzero source ID.

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Source ID |
| 1 | text | Canonical codec name |
| 2 | text | Packetization identifier |
| 3 | bytes | Codec extradata |
| 4 | uint | Coded width, 1 through 8,192 |
| 5 | uint | Coded height, 1 through 8,192 |
| 6 | int | Codec profile, representable as `i32` |
| 7 | int | Codec level, representable as `i32` |
| 8 | uint | Alpha mode; `0` means no alpha |
| 9 | uint | Latency mode; `0` low-latency baseline |
| 10 | uint | Retention mode; `0` no encoded-packet retention |
| 11 | uint | Expected bitrate in bits per second; zero unknown |
| 12 | uint | Maximum reorder depth, at most 64 |
| 13 | text | Timeline; MUST be `source-timebase-us` |
| 14 | uint | Color primaries |
| 15 | uint | Transfer characteristic |
| 16 | uint | Matrix coefficients |
| 17 | uint | Signal range |
| 18 | uint | Sample-aspect-ratio numerator, nonzero |
| 19 | uint | Sample-aspect-ratio denominator, nonzero |
| 20 | uint | Maximum encoded access-unit bytes, nonzero |
| 21 | text | OPTIONAL RFC 6381 codec string, at most 64 printable-ASCII bytes |
| 22 | bytes | OPTIONAL ISO-BMFF decoder configuration body, at most 4,096 bytes |

Keys 14 through 20 are REQUIRED for `video-access-unit-v1`.

Keys 21 and 22 belong to `decoder-description-v1` (feature 16). A producer MUST NOT send them
unless the presenter accepted feature 16. The codec-string family MUST match key 1
(`avc1`/`avc3` for `h264`, `hvc1`/`hev1` for `hevc`, `vp09` for `vp9`, `av01` for `av1`), and the
decoder configuration MUST be the matching box body (avcC, hvcC, vpcC, or av1C) consistent with
the extradata and the stream. A presenter MAY ignore both keys; when it uses them it MUST
validate family and length first and MUST treat a mismatch as `BAD_MESSAGE`. The keys are
descriptive only: they change no packetization, and extradata (key 3) remains authoritative for
portable-profile initialization.

Canonical codec and packetization pairs for the portable profile are:

| Codec key 1 | Packetization key 2 |
|---|---|
| `h264` | `h264-annexb-au-v1` |
| `hevc` | `hevc-annexb-au-v1` |
| `vp9` | `vp9-frame-v1` |
| `av1` | `av1-low-overhead-tu-v1` |

Color primaries:

| Value | Meaning |
|---:|---|
| 0 | Unspecified; unsupported in this profile |
| 1 | BT.709 |
| 2 | BT.601 625-line family |
| 3 | BT.601 525-line family |
| 4 | BT.2020 |

Transfer characteristics:

| Value | Meaning |
|---:|---|
| 0 | Unspecified; unsupported in this profile |
| 1 | BT.709 |
| 2 | sRGB |
| 3 | PQ; reserved unless separately advertised |
| 4 | HLG; reserved unless separately advertised |

Matrix coefficients:

| Value | Meaning |
|---:|---|
| 0 | Identity/RGB |
| 1 | BT.709 |
| 2 | BT.601 |
| 3 | BT.2020 non-constant luminance |

Signal range:

| Value | Meaning |
|---:|---|
| 1 | Limited range |
| 2 | Full range |

A presenter MUST return unsupported rather than silently substituting colorimetry or packetization.

`PROBE_VIDEO_CONFIG` is session-level and returns `VIDEO_SUPPORT`:

| Key | Type | Meaning |
|---:|---|---|
| 0 | bool | Exact configuration supported |
| 1 | text | Selected decoder/profile name; operator information only |

Probes MAY be pipelined and answered independently.

### 7.3 Audio configuration

`PROBE_AUDIO_CONFIG` and `CREATE_AUDIO` use the same payload. A probe uses source ID zero and no
link; creation uses a nonzero source ID. A linked video source must already exist in the same
session.

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Source ID |
| 1 | uint | Linked video source ID, or zero for standalone audio |
| 2 | text | Canonical codec name, at most 64 UTF-8 bytes |
| 3 | text | Packetization identifier, at most 64 UTF-8 bytes |
| 4 | bytes | Codec extradata, at most 65,536 bytes |
| 5 | uint | Sample rate, 8,000 through 192,000 Hz |
| 6 | uint | Channel count, 1 through 8 |
| 7 | uint | Native channel mask, or zero for the default layout |
| 8 | uint | Expected bitrate in bits per second; zero unknown |
| 9 | uint | Maximum encoded access-unit bytes, 1 through 1,048,576 |
| 10 | text | Timeline; MUST be `source-timebase-us` |
| 11 | text | OPTIONAL RFC 6381 codec string, at most 64 printable-ASCII bytes |

Key 11 belongs to `decoder-description-v1` (feature 16). A producer MUST NOT send it unless the
presenter accepted feature 16. The codec-string family MUST match key 2 (`mp4a.40.*` for `aac`,
`mp3` or `mp4a.6B` for `mp3`, the codec name itself for `opus`, `vorbis`, `flac`, and `alac`,
`ulaw` for `pcm_mulaw`, `alaw` for `pcm_alaw`, and a `pcm-*` string for other PCM codecs). A presenter MAY ignore the key; when it uses it, it MUST
validate family and length first. Extradata (key 4) remains authoritative for initialization.

Nonzero channel masks MUST contain exactly the declared number of channels. Canonical codec and
packetization pairs are:

| Codec key 2 | Packetization key 3 |
|---|---|
| `mp3` | `mp3-frame-v1` |
| `aac` | `aac-raw-au-v1` |
| `alac` | `alac-frame-v1` |
| `opus` | `opus-packet-v1` |
| `vorbis` | `vorbis-packet-v1` |
| `flac` | `flac-frame-v1` |
| `pcm_u8`, `pcm_s16le`, `pcm_s24le`, `pcm_s32le`, `pcm_f32le`, `pcm_f64le`, `pcm_mulaw`, `pcm_alaw` | `pcm-packet-v1` |

`PROBE_AUDIO_CONFIG` returns `AUDIO_SUPPORT` with payload key 0 boolean exact-configuration support
and key 1 the codec/decoder name for operator diagnostics. Unsupported codecs, layouts, limits, or
unavailable decoders return support false with the codec name; they do not cause a protocol
downgrade.

### 7.4 Raster configuration

`CREATE_RASTER` payload:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Nonzero source ID |
| 1 | uint | Width, 1 through 8,192 |
| 2 | uint | Height, 1 through 8,192 |
| 3 | uint | Pixel format; MUST be `RGBA8` (`1`) |
| 4 | uint | Alpha mode: straight (`1`) or premultiplied (`2`) |
| 5 | uint | Update mode; full-frame (`0`) |
| 6 | uint | Rectangle limit; MUST be `1` |
| 7 | uint | Compression mode: raw only (`0`) or raw-or-zstd (`1`) |
| 8 | uint | Retention; MUST be none (`0`) |

Alpha mode 2 requires `RASTER_PREMULTIPLIED_ALPHA`. Compression mode 1 requires `RASTER_ZSTD_V1`.

The presenter computes, with checked arithmetic:

```text
raw_frame_body = 72 + width * height * 4
```

It MUST reject source creation with `LIMIT_EXCEEDED` unless `raw_frame_body` fits the presenter’s accepted media-body ceiling and resource budget. The axis limit alone does not make a source admissible.

Under the 64 MiB hard body ceiling, the maximum raw RGBA8 pixel count is 16,777,198. A 4096×4096 raw frame is 72 bytes too large; a 4095×4095 frame fits.

Raster samples are full-range sRGB RGBA8.

### 7.5 Encoded-image configuration

`CREATE_IMAGE` requires `ENCODED_IMAGE_V1` and uses:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Nonzero source ID |
| 1 | uint | Encoding: PNG (`1`) or JPEG (`2`) |
| 2 | uint | Decoded width, 1 through 8,192 |
| 3 | uint | Decoded height, 1 through 8,192 |
| 4 | uint | Exact encoded byte length, nonzero |
| 5 | bytes(32), optional | SHA-256 of the encoded bytes |
| 6 | uint | Output color space; MUST be sRGB (`1`) |
| 7 | uint | Retention mode; MUST be decoded-source retention (`1`) |

The decoded pixel count and encoded byte length MUST fit presenter quotas before a ticket is issued.

Only single-image PNG and single-image JPEG are in this profile. Animated PNG, multi-picture formats, embedded executable content, and format switching are not supported. Orientation metadata is ignored; the producer MUST encode pixels in the intended display orientation.

### 7.6 Source creation and loss

Successful `CREATE_VIDEO`, `CREATE_AUDIO`, `CREATE_RASTER`, or `CREATE_IMAGE` returns `SOURCE_READY`:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Source ID |
| 1 | bytes(32) | Single-use media-channel ticket |
| 2 | uint | Initial byte credits |
| 3 | uint | Initial packet credits |
| 4 | uint | Initial fragment credits; zero in this profile |
| 5 | uint | Maximum media-record body accepted for this source |

The `SOURCE_READY` object ID MUST equal the source ID.

For every accepted source, key 2 MUST be at least the maximum body of one legal media record for that source, and key 3 MUST be at least one:

- raster: at least `raw_frame_body`;
- image: at least the declared encoded length;
- video: at least `48 + maximum encoded access-unit bytes`.
- audio: at least `48 + maximum encoded access-unit bytes`.

A presenter that cannot make that grant MUST reject source creation rather than create a source that cannot make progress.

`DESTROY_SOURCE` payload key 0 is the source ID. Success returns `OK`, closes its media channel, releases its resources, and removes placements using the source at the next compositor boundary.

`SOURCE_LOST` is unsolicited, uses request ID zero, and has:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Source ID |
| 1 | uint | Error code |
| 2 | text | Diagnostic, at most 4,096 UTF-8 bytes |

Its object ID MUST equal the source ID. A lost source accepts no further media. Its placements are removed at the next compositor boundary. The producer may create a replacement with a new source ID.

Source IDs are unique within a session and MUST NOT be reused while the source or any loss/destroy reply can still be observed.

### 7.7 Scene transactions

`BEGIN_TXN` contains the transaction ID at envelope key 1 and payload key 0; the values MUST match and be nonzero. Success returns `OK`.

A presenter MUST support at least one open transaction per session. It MAY impose a lower-than-global concurrency limit and reject an additional `BEGIN_TXN` with `LIMIT_EXCEEDED`. Transaction IDs MUST be unique while open or while replies remain outstanding.

`CREATE_NODE` and `UPDATE_NODE` use a complete node representation:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Nonzero node ID |
| 1 | uint | Nonzero source ID |
| 2 | uint | Context ID; baseline requires the session root context |
| 3 | uint | Coordinate space: grid cell (`1`) or anchor cell (`3`) |
| 4 | int | X, signed 32.32 cells |
| 5 | int | Y, signed 32.32 cells |
| 6 | int | Width, positive signed 32.32 cells |
| 7 | int | Height, positive signed 32.32 cells |
| 8 | uint | Fit mode; contain (`2`) |
| 9 | uint | Sampling; linear (`1`) |
| 10 | uint | Text layer; between background and glyph (`1`) |
| 11 | int | Z index within the text layer |
| 12 | uint | Blend mode; source-over (`0`) |
| 13 | bool, optional | Visibility; default true |
| 14 | uint, conditional | Anchor ID, required only for anchor-cell coordinates |
| 15 | int, conditional | Clip X, signed 32.32 cells |
| 16 | int, conditional | Clip Y, signed 32.32 cells |
| 17 | int, conditional | Clip width, positive signed 32.32 cells |
| 18 | int, conditional | Clip height, positive signed 32.32 cells |

Keys 15 through 18 require `NODE_CLIP_RECT_V1` and MUST either all be present or all be absent.
The clip uses the node's coordinate space and follows the same anchor transform. Width and height
MUST be positive, and each origin-plus-extent calculation MUST be checked for signed overflow. The
presenter clips the fitted media quad without rescaling it, including its letterbox area; clipping
therefore adjusts geometry and texture coordinates rather than performing another fit. The final
draw region is the intersection of the media quad, node clip, and display viewport. An empty
intersection draws nothing. A presenter MUST reject clip keys when the feature was not negotiated.

Node operations require the transaction ID at envelope key 1. The record object ID MUST equal the node ID. `UPDATE_NODE` is a complete replacement. `DELETE_NODE` payload key 0 contains the node ID.

`COMMIT_TXN` contains transaction ID at envelope key 1, expected display generation at envelope key 2, and:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Presentation mode; next compositor frame (`0`) |
| 1 | bool | Acknowledgement requested; MUST be true in the Vivid 1.0 baseline |

All mutations apply atomically or not at all.

On success, when the new scene state becomes active at a compositor boundary, the presenter sends `PRESENTED` with an empty payload and the commit request ID. `PRESENTED` does **not** assert that any source frame was decoded, displayed, or visible.

A stale display generation returns `STALE_DISPLAY_GENERATION` without applying mutations.

`ABORT_TXN` repeats the transaction ID at envelope key 1 and payload key 0. Success returns `OK` and discards queued mutations.

### 7.8 Playback and recovery

`PLAY` applies to video and audio sources:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Source ID |
| 1 | int | Start PTS in microseconds |
| 2 | uint | Minimum buffer in microseconds |
| 3 | uint | Maximum latency; baseline default 500,000 |
| 4 | int | Signed 32.32 playback rate; baseline requires 1.0 |
| 5 | uint | Late policy; drop presentation (`1`) |
| 6 | uint | Loop count; baseline requires zero |
| 7 | uint | Start policy; after minimum buffer (`1`) |

The object ID MUST match key 0.

The presenter validates and retains every field before changing playback state. It starts the source
clock when the requested minimum buffered horizon is available or EOS proves that no more pre-roll
will arrive. Decoded audio and video before `start_pts` are discarded. Media time `start_pts` maps
exactly to the clock start. For linked sources, played audio-device frames are the master clock;
the presenter inserts initial silence or trims leading samples to align PTS. On audio underrun it
emits silence while the clock continues. Frames that become late are dropped according to the late
policy.

If a linked audio source delivers no access units within an implementation-defined bound after
`PLAY` (a bound of at least two seconds is recommended), a presenter MAY temporarily present the
linked video on its own video timeline. It resumes audio-clocked presentation once audio arrives.
Producers MUST NOT rely on this fallback: a producer that abandons a linked audio source MUST
destroy that source.

After obtaining a clean RTT sample, a producer or bridge MAY conservatively raise the requested
minimum buffer for a remote hop to:

```text
minimum_buffer_us = min(500000, max(requested_us, 2 * rtt_us + 25000))
```

Without a clean RTT sample it preserves the requested value. This is startup sizing, not
telemetry-driven adaptation, and it does not alter `maximum_latency` or other `PLAY` fields.

`PAUSE` requires `VIDEO_CONTROL_V1` and has payload key 0 source ID. It freezes the source clock, holds the latest frame, and stops consuming audio samples. Incoming packets MAY continue to buffer within available credit. A control applied to a linked video also applies to its linked audio. Success returns `OK`.

`FLUSH` requires `VIDEO_CONTROL_V1`:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Source ID |
| 1 | uint | New epoch, representable as `u32` and greater than the current epoch |

`FLUSH` discards queued packets and decoder state, enters paused state, and requires the next accepted packet to be a key packet in the new epoch. Success returns `OK`. A producer seeking in its own media uses `FLUSH`, sends a key packet for the new epoch, and then sends `PLAY` with the desired start PTS.

`EOS` payload key 0 is source ID and key 1 is epoch. The object ID MUST match. The epoch MUST NOT be older than the last accepted epoch. For video, queued decoder output may finish. For raster and image, the latest poster remains until source/node lifecycle removes it. Success returns `OK`.

`DRAIN` requires `AUDIO_ACCESS_UNIT_V1` and has payload key 0 audio source ID. It returns `OK`
only after EOS has been observed, the decoder and resampler have flushed, and all queued device
samples have been consumed. Device loss returns `DEVICE_LOST`.

`NEED_KEYFRAME` requires `VIDEO_CONTROL_V1`, is unsolicited, and uses:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Source ID |
| 1 | uint | Minimum acceptable epoch, representable as `u32` |
| 2 | uint | Reason |
| 3 | uint, optional | Last accepted packet ID |

Reason values are:

| Value | Meaning |
|---:|---|
| 1 | Initial random-access packet required |
| 2 | Decoder error or lost decoder state |
| 3 | Invalid discontinuity/epoch transition |
| 4 | Device or renderer reset |

The presenter discards unusable delta packets and waits for a key packet at the stated epoch or a greater epoch. This recovery is source-scoped.

### 7.9 Credits

`CREDIT` is unsolicited, uses request ID zero, and identifies the source in the record object ID:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Byte-credit increment |
| 1 | uint | Packet-credit increment |
| 2 | uint, optional | Fragment-credit increment; default zero |

Credit addition uses saturating unsigned arithmetic. See Section 9 for exact semantics.

### 7.10 Visibility

`VISIBILITY` requires `VISIBILITY_EVENTS_V1`. It is unsolicited, source-level, and advisory:

| Key | Type | Meaning |
|---:|---|---|
| 0 | bool | At least one committed visible node for the source intersects the current viewport and the presenter window is eligible to render |
| 1 | uint | Reason bit mask |
| 2 | uint | Display generation used for the calculation |

Reason bits are:

| Bit | Meaning when set |
|---:|---|
| 0 | No committed visible placement intersects the viewport |
| 1 | Presenter window is hidden, minimized, or not renderable |
| 2 | Presenter is applying resource-pressure throttling |

The object ID identifies the source. A presenter SHOULD emit an event when aggregate state changes and MUST rate-limit events to at most one state update per source per compositor frame.

A producer MAY pause, reduce frame rate, or reduce bitrate while false. It MUST treat the event as a hint: events can be delayed, and absence of an event does not prove visibility.

### 7.11 Anchor events

`ANCHOR_READY` and `ANCHOR_GONE` are unsolicited. Payload key 0 and the record object ID both identify the anchor. Anchor IDs are scoped to the authenticated session.

## 8. Media-channel binding

After `SOURCE_READY`, the producer opens a connection whose kind matches the source:

| Source | Connection kind |
|---|---|
| Video | `Video` (`1`) |
| Raster | `Raster` (`2`) |
| Encoded image | `Blob` (`3`) |
| Audio | `Audio` (`5`) |

The first record MUST be `ATTACH_CHANNEL`. Its body uses the deterministic CBOR envelope, request ID zero, and payload key 0 containing the 32-byte ticket. The record object ID is the source ID.

`ATTACH_CHANNEL` is not charged against media byte or packet credit.

A ticket is single-use and bound to session, source ID, and connection kind. In Vivid 1.0 a ticket does not expire by time; it remains valid until used, source destruction, or session loss. Missing, reused, wrong-kind, or wrong-source tickets close the media connection.

No success acknowledgement is required. After writing a valid `ATTACH_CHANNEL`, the producer MAY immediately write the first media record on the same stream.

After attachment:

- a video channel accepts only `VIDEO_PACKET` for its source;
- a raster channel accepts only `RASTER_FRAME` for its source;
- an image channel accepts exactly one `IMAGE_DATA` for its source.
- an audio channel accepts only `AUDIO_PACKET` for its source.

Presenter-to-producer credits and events remain on the control connection.

## 9. Credit flow control

Credits represent bounded presenter capacity to accept additional media records. They are not decode acknowledgements, presentation acknowledgements, or transport writability.

Before sending a charged media record, the producer MUST hold:

- byte credit at least equal to the complete record-body length; and
- at least one packet credit.

It deducts those amounts before transmission. The 24-byte record header and `ATTACH_CHANNEL` are not charged.

A presenter returns byte and packet credits only when both of the following capacity is reusable:

1. the media body’s ingress storage has been released or transferred into separately bounded storage; and
2. the corresponding bounded media-queue slot is available again.

For a decoder that retains the compressed packet buffer, credit is not returned until that ownership ends. For raster, credit may be returned after validated pixels have been copied/uploaded and the input body is released; it need not wait for presentation.

A presenter MAY grant credit proactively, but MUST NOT grant capacity it cannot bound. It SHOULD maintain a rolling window using high and low watermarks rather than waiting for the producer to reach zero. Credit return MUST NOT be coalesced in a way that can strand the only packet or maximum-record-sized byte grant; any coalescing policy needs an independent timer and a low-watermark path.

The initial grant rules in Section 7.6 guarantee that every accepted source can send at least one maximum legal record. For remote high-throughput streams, a presenter SHOULD size the rolling window to at least one maximum record plus a reasonable estimate of the path bandwidth-delay product, subject to its memory budget.

A producer with insufficient credit waits, coalesces, drops an obsolete latest-frame raster update, or reports backpressure to its caller. It MUST NOT send past credit. Packet credit is a record-window limit: a sustainable remote window must cover the records expected during roughly one path RTT in addition to byte-credit sizing. Small PCM records SHOULD represent at least 20 ms of audio unless lower latency is explicitly required.

A credit violation closes the media connection and produces `SOURCE_LOST` with `FLOW_CONTROL`. It does not corrupt other sources or terminal text.

Fragment credits remain zero; `VIDEO_FRAGMENT` is not defined by this profile.

## 10. Portable encoded-video profile

### 10.1 Packet body

A `VIDEO_PACKET` body contains a 48-byte prefix followed by encoded access-unit bytes:

| Offset | Size | Field |
|---:|---:|---|
| 0 | 4 | Epoch |
| 4 | 4 | Packet flags |
| 8 | 8 | Packet ID |
| 16 | 8 | PTS in microseconds, signed |
| 24 | 8 | DTS in microseconds, signed |
| 32 | 8 | Duration in microseconds |
| 40 | 4 | Side-data length; MUST be zero |
| 44 | 4 | Reserved; MUST be zero |
| 48 | variable | Encoded access unit |

Packet flag bit 0 is `KEY`; bit 1 is `DELTA`. Exactly one is set. All other bits are zero.

Packet ID is nonzero and MUST be strictly greater than the previously accepted packet ID for the source, including across epoch changes. Exhaustion requires source replacement. Duration zero means unknown. The encoded access unit MUST be nonempty and MUST NOT exceed the source’s declared maximum.

Side data is forbidden in `video-access-unit-v1`. A future negotiated profile may define typed side-data elements.

### 10.2 Canonical packetizations

#### H.264: `h264-annexb-au-v1`

Each body contains exactly one H.264 access unit in Annex B byte-stream form. Every NAL unit is preceded by a three- or four-byte start code. AVCC length-prefixed NAL units are not permitted.

Extradata is empty or a concatenation of Annex B parameter-set NAL units sufficient to initialize the decoder. If extradata is empty, the first key access unit contains the required parameter sets.

A `KEY` packet is a random-access access unit from which decoding can begin with the supplied parameter sets.

#### HEVC: `hevc-annexb-au-v1`

Each body contains exactly one HEVC access unit in Annex B byte-stream form. Length-prefixed NAL units are not permitted.

Extradata is empty or a concatenation of Annex B VPS, SPS, and PPS NAL units sufficient to initialize the decoder. If empty, the first key access unit contains the required parameter sets.

A `KEY` packet is an independently decodable random-access access unit with the supplied parameter sets.

#### VP9: `vp9-frame-v1`

Each body contains exactly one complete VP9 compressed frame without container framing. Extradata is empty. `KEY` denotes a VP9 key frame.

#### AV1: `av1-low-overhead-tu-v1`

Each body contains exactly one AV1 temporal unit in low-overhead OBU form without container framing. Extradata is empty or one sequence-header OBU. `KEY` denotes a random-access temporal unit that can initialize decoding with the supplied sequence header.

### 10.3 Decode order, timestamps, and epochs

Packets are sent in decoder input order. PTS and DTS are integer microseconds in `source-timebase-us`.

Epochs are monotonically nondecreasing. The first accepted packet for a source is a key packet. The first packet of a greater epoch is a key packet. A packet from an older epoch is dropped and reports `STALE_EPOCH`; a greater epoch beginning with a delta packet causes `NEED_KEYFRAME`.

Packetization, codec, coded dimensions, and colorimetry remain fixed for the source lifetime. A producer requiring a different configuration creates a new source.

## 10A. Portable encoded-audio profile

An `AUDIO_PACKET` body contains a 48-byte prefix followed by one encoded access unit:

| Offset | Size | Field |
|---:|---:|---|
| 0 | 4 | Epoch |
| 4 | 4 | Reserved; MUST be zero |
| 8 | 8 | Packet ID |
| 16 | 8 | PTS in microseconds, signed |
| 24 | 8 | DTS in microseconds, signed |
| 32 | 8 | Duration in microseconds |
| 40 | 4 | Leading trim samples |
| 44 | 4 | Trailing trim samples |
| 48 | variable | Encoded access unit |

Packet IDs are nonzero and strictly increase for the source, including across epochs. Epochs are
monotonically nondecreasing. The encoded access unit MUST be nonempty, no larger than the source's
declared maximum, and no larger than 1 MiB. Trim counts are in the configured source sample rate
and are normalized from container/codec delay metadata such as FFmpeg `AV_PKT_DATA_SKIP_SAMPLES`.

MP3 bodies contain one complete MPEG audio frame. AAC bodies contain one raw AAC access unit, with
decoder configuration in extradata rather than ADTS framing. AAC extradata is one
AudioSpecificConfig; its audio object type MUST NOT be null, its sampling frequency MUST equal the
declared sample rate or exactly half of it (the HE-AAC signaling convention), and a nonzero
channel configuration MUST match the declared channel count. ALAC bodies contain one ALAC frame.
PCM bodies contain an integral number of interleaved samples in the byte order and representation
named by the codec.

An `opus-packet-v1` body contains one complete Opus packet. Extradata is one complete canonical
`OpusHead`; the decode sample rate is 48 kHz, mapping families 0 and 1 are supported, and its
pre-skip is applied exactly once. Packet leading trim MUST NOT apply the same pre-skip again.

A `vorbis-packet-v1` body contains one complete Vorbis audio packet. Extradata contains the three
Vorbis initialization headers in Xiph lacing form: the leading byte is 2, followed by the laced
identification-header and comment-header lengths, then the identification, comment, and setup
headers. Container-specific alternate packing is not permitted on the wire.

A `flac-frame-v1` body contains one complete FLAC frame. Extradata is exactly the raw 34-byte
STREAMINFO payload; it excludes both the `fLaC` marker and the four-byte metadata-block header.

Before source allocation, the presenter validates canonical signatures and lengths, codec
versions, sample rate, channel count, supported Opus mapping, and all internal header boundaries.
The decoder's resulting configuration MUST match the declared source configuration.

The presenter decodes and resamples into a bounded two-second device buffer. Once playback has
started, an underrun produces silence rather than blocking the device callback. Pause does not
consume samples. A device or decoder failure emits `SOURCE_LOST`; linked video remains alive and
continues silently, while a standalone audio producer treats the loss as playback failure.

## 11. Full-frame raster profile

### 11.1 Frame body

A `RASTER_FRAME` body has a 48-byte frame header, one 24-byte rectangle descriptor, and one pixel payload.

Frame header:

| Offset | Size | Field |
|---:|---:|---|
| 0 | 4 | Epoch |
| 4 | 4 | Flags |
| 8 | 8 | Frame ID |
| 16 | 8 | Base frame ID; MUST be zero |
| 24 | 8 | PTS in microseconds, signed |
| 32 | 8 | Duration in microseconds |
| 40 | 4 | Rectangle count; MUST be one |
| 44 | 4 | Reserved; MUST be zero |

Frame flag bit 0 is `FULL` and MUST be set. Bit 1 is `ZSTD`. Other bits are zero.

Rectangle descriptor:

| Offset | Size | Field |
|---:|---:|---|
| 48 | 4 | X; zero |
| 52 | 4 | Y; zero |
| 56 | 4 | Width |
| 60 | 4 | Height |
| 64 | 4 | Data offset from rectangle-data area; zero |
| 68 | 4 | Data length |
| 72 | variable | Raw or zstd-compressed RGBA8 pixels |

Width and height MUST exactly match the source. Frame ID is nonzero and MUST be strictly greater than the previously accepted frame ID for the source, including across epoch changes. Exhaustion requires source replacement. Epochs are monotonically nondecreasing. A stale epoch is dropped with `STALE_EPOCH` and does not mutate the displayed latest frame.

The body ends immediately after `data_length` bytes.

### 11.2 Raw pixels

When `ZSTD` is clear:

```text
data_length = width * height * 4
```

The multiplication is checked for overflow. Pixels are row-major RGBA8.

Straight-alpha sources store independent sRGB color components and alpha.

Premultiplied-alpha sources store color components already multiplied by alpha. Producers MUST emit zero color components when alpha is zero and MUST NOT emit a color component greater than alpha in normalized integer representation.

### 11.3 Zstd pixels

`ZSTD` may be set only when the source was created with compression mode 1 and `RASTER_ZSTD_V1` was accepted.

The payload contains exactly one zstd frame, with no dictionary and no skippable frames. Decompression MUST produce exactly `width * height * 4` bytes and consume the entire payload. Short output, excess output, trailing frames, dictionary references, or decoder errors lose the source with `BAD_MESSAGE` or `DECODER`.

The presenter MUST bound decompression output by the already validated raw pixel length and MUST NOT trust a size declared inside the compressed stream.

A producer SHOULD send raw pixels when zstd does not reduce the body size. Source admissibility and initial credit are based on the raw body, so compression is never required for liveness.

### 11.4 Timing and coalescing

Raster is immediate, latest-frame media. An accepted frame becomes eligible at the next compositor opportunity. PTS and duration are metadata only and do not schedule presentation in Vivid 1.0. `PLAY`, `PAUSE`, and `FLUSH` are invalid for raster sources.

If multiple raster frames become ready before presentation, the presenter MAY discard intermediate frames and present only the newest valid frame. Credit return still follows storage/queue capacity, not whether the frame was presented.

## 12. Encoded still-image profile

`ENCODED_IMAGE_V1` carries one encoded image over a blob-kind media connection.

After valid attachment, the producer sends exactly one `IMAGE_DATA` record whose object ID is the source ID. The body is exactly the encoded bytes declared by `CREATE_IMAGE`; it has no additional prefix.

The presenter MUST:

1. verify body length and optional SHA-256 before decode;
2. enforce the declared encoded-length and decoded-pixel quotas;
3. use an allowlisted PNG or JPEG decoder;
4. reject an encoded image whose decoded dimensions differ from the declaration;
5. reject animated/multi-picture content;
6. produce an sRGB source image.

PNG alpha is interpreted as straight alpha. JPEG is opaque. Orientation metadata is ignored.

The source becomes renderable after successful decode. It behaves as a retained still source and requires no `PLAY`. Decode failure emits `SOURCE_LOST` and removes placements using that source.

The presenter MUST NOT fetch external resources referenced by metadata and MUST bound decoder allocations independently of encoded byte length.

## 13. Text anchors

### 13.1 Marker version 2

Vivid 1.0 authenticated anchors use this APC marker:

```text
ESC _ VIVID;2;A;<session-tag>;<anchor-id>;<auth> ESC \
```

Equivalent escaped form:

```text
\x1b_VIVID;2;A;<session-tag>;<anchor-id>;<auth>\x1b\\
```

Fields are:

- `<session-tag>`: the 16-byte `WELCOME` session tag as exactly 22 unpadded base64url characters;
- `<anchor-id>`: exactly 16 lowercase or uppercase hexadecimal digits encoding a nonzero `u64`;
- `<auth>`: exactly 22 unpadded base64url characters encoding a 16-byte authenticator.

The complete marker is ASCII, has zero display width, does not move the cursor, and MUST NOT exceed 128 bytes.

On a ConPTY path selected by `VIVID_ANCHOR_TRANSPORT=conpty`, the identical authenticated marker
payload is carried in this established scanner envelope instead of APC:

```text
VIVID;2;A;<session-tag>;<anchor-id>;<auth>;VIVID-END
```

The complete ConPTY envelope is ASCII and MUST NOT exceed 128 bytes. It has the same zero-width
semantic effect as APC: the presenter consumes the entire envelope without rendering cells or
moving the cursor, then parses and verifies the exact bytes from `VIVID` through `<auth>` using the
rules below. The `;VIVID-END` suffix is transport framing only and is not authenticated payload or
a Vivid record. Scanners MUST be fragment-safe, preserve malformed or oversized candidates
byte-for-byte, and MAY accept both ConPTY and APC envelopes during migration. Unix local presenters
remain APC-only.

### 13.2 Authenticator derivation

Let:

- `token` be the 32 raw bytes decoded from `VIVID_TOKEN`;
- `tag` be the 16 raw session-tag bytes;
- `anchor_id_be` be the 8-byte big-endian anchor ID;
- `HMAC-SHA256(K, M)` have its conventional meaning.

Derive:

```text
anchor_key = HMAC-SHA256(
    token,
    ASCII("VIVID-ANCHOR-KEY-V2") || tag
)

auth_full = HMAC-SHA256(
    anchor_key,
    ASCII("VIVID-ANCHOR-V2") || tag || anchor_id_be
)

auth = first 16 bytes of auth_full
```

`<auth>` is the unpadded base64url encoding of `auth`.

The presenter compares the authenticator in constant time. It MAY discard the original token after deriving the per-session anchor key.

The session tag is an identifier, not a secret. Terminal recordings may contain it; disclosure does not permit forging a new version-2 marker without the token-derived key.

### 13.3 Anchor IDs and replay

A producer MUST generate anchor IDs using a cryptographically secure random source. IDs are scoped to one control session and MUST never be reused during that session, even after an anchor is gone.

A presenter tracks seen anchor IDs for the session. A duplicate or replayed marker MUST NOT create, move, or recreate an anchor. It is ignored and MAY be logged subject to rate limiting.

A valid new marker creates the anchor at the current semantic text position and emits `ANCHOR_READY`.

### 13.4 Lifecycle

An anchor follows its semantic text position through scrolling and scrollback. When that position is erased or evicted, the presenter removes attached nodes and emits `ANCHOR_GONE`.

Clearing the terminal text plane removes all anchors and anchored nodes.

After producer disconnect, an anchored latest frame MAY remain as a poster. The presenter MUST release decoder state, compressed packet queues, and other non-poster resources. Poster memory remains bounded by the aggregate resource budget and is reclaimed when the anchor is removed or under a documented resource-pressure policy.

Unanchored nodes are tied to control-session lifetime.

`BARRIER_REACHED` remains assigned but undefined.

## 14. Errors and failure isolation

Core error codes are:

| Value | Name | Value | Name |
|---:|---|---:|---|
| 1 | `AUTH_FAILED` | 11 | `FLOW_CONTROL` |
| 2 | `UNSUPPORTED_VERSION` | 12 | `HASH_MISMATCH` |
| 3 | `UNSUPPORTED_FEATURE` | 13 | `NEED_KEYFRAME` |
| 4 | `UNSUPPORTED_CONFIG` | 14 | `STALE_EPOCH` |
| 5 | `BAD_MESSAGE` | 15 | `STALE_DISPLAY_GENERATION` |
| 6 | `BAD_STATE` | 16 | `ANCHOR_INVALIDATED` |
| 7 | `DUPLICATE_ID` | 17 | `CONTEXT_REVOKED` |
| 8 | `NOT_FOUND` | 18 | `DECODER` |
| 9 | `LIMIT_EXCEEDED` | 19 | `DEVICE_LOST` |
| 10 | `NO_MEMORY` | 20 | `TIMEOUT` |

`ANCHOR_GONE` is a legacy symbolic alias for error code 16; new code uses `ANCHOR_INVALIDATED`. The event record remains named `ANCHOR_GONE`.

Malformed prefaces, malformed record headers, reserved framing bits, invalid sequence numbers, and bodies over an effective ceiling close the affected connection.

A malformed control body produces `BAD_MESSAGE` when request correlation remains safe. A failed transaction never partially mutates the scene.

Media framing, flow-control, decode, hash, decompression, and epoch failures are scoped to the affected source where possible and emit `SOURCE_LOST` or `NEED_KEYFRAME`. They MUST NOT corrupt terminal text parsing or unrelated sources.

## 15. Security and resource requirements

A conforming presenter MUST:

- authenticate `HELLO` and verify local peer identity before allocating producer-controlled source/scene resources;
- compare capabilities and anchor authenticators without data-dependent early exit;
- keep local and SSH-forwarded Unix sockets private to the owning user;
- reject ID collisions in session, source, node, transaction, and anchor namespaces;
- validate all lengths, integer widths, dimensions, fixed-point geometry, tickets, epochs, hashes, decoder outputs, and credit before use;
- bound sessions, connections, sources, nodes, transactions, anchors, seen anchor IDs, encoded bytes, decoded pixels, posters, compressed output, CBOR nesting, and record bodies;
- use allowlisted image/video decoders and source-scoped decoder failure boundaries;
- never open a producer-supplied pathname or fetch a producer-supplied URL;
- prevent media failure from entering the terminal control-sequence parser.

Resource budgets are aggregate per presenter window unless explicitly stated otherwise. They are not multiplied independently by the maximum session count.

Recommended reference limits are:

| Resource | Limit or policy |
|---|---:|
| Concurrent sessions | 16 |
| Concurrent connections | 64 |
| Sources, aggregate | 64 |
| Nodes, aggregate | 256 |
| Active anchors, aggregate | 256 |
| Seen anchor IDs | 4,096 per session |
| Source width or height | 8,192 pixels, plus body/pixel constraints |
| Retained decoded/poster pixels | At most `8192 * 8192 * 2`, further reduced by configured memory budget |
| Control record body | 1 MiB |
| Hard record body | 64 MiB |
| Default rolling media byte window | 4 MiB, raised to at least one source-maximum record |
| Default media packet credits | 32, never below one for an accepted source |

A presenter SHOULD scale decoded/poster budgets to available system and GPU memory and SHOULD reject source creation before memory pressure becomes uncontrolled.

## 16. Conformance

### 16.1 Vivid 1.0 producer conformance

A producer conforms when it:

1. emits version 1.0 in the connection preface and selects Vivid 1.0;
2. uses deterministic CBOR and valid request correlation;
3. authenticates with `HELLO` and honors all `WELCOME` limits and selected features;
4. pipelines only operations whose input dependencies are satisfied;
5. uses one single-use ticket per media channel;
6. never exceeds source body limits or credits;
7. emits raster, video, image, or audio bodies matching the negotiated profile;
8. uses atomic scene transactions and the selected anchor marker version;
9. services control continuously, including source-scoped loss, visibility, keyframe recovery, and
   bidirectional liveness for features it requested.

### 16.2 Vivid 1.0 presenter conformance

A presenter conforms when it:

1. validates framing and directional limits before body allocation;
2. authenticates token and local peer identity before producer-controlled allocation;
3. implements control, raw full-frame RGBA8 raster, scene transactions, credit flow control, and authenticated text-anchor v2 behavior;
4. grants enough initial source credit for one maximum legal record;
5. implements encoded image, zstd raster, premultiplied alpha, visibility, portable video, and video controls only when it advertises them;
6. rejects sessions that cannot select Vivid 1.0;
7. isolates source/media failures from the control session and terminal parser;
8. keeps aggregate quotas and replay state bounded;
9. services control continuously and answers valid session-level `PING` requests promptly.

### 16.3 Reference-code mapping

The shared Rust crate should continue to mirror the specification:

| Specification area | Reference source |
|---|---|
| Version, limits, feature/profile constants | `lib.rs` |
| Endpoint parsing, preface, directional body limits, headers, flags, sequences | `wire.rs` |
| Deterministic CBOR and numeric/size bounds | `cbor.rs` |
| Opcodes, features, errors, control schemas, colorimetry, audio initialization | `messages.rs` |
| Video/audio/raster/image binary bodies and zstd validation | `media.rs` |
| Anchor HMAC and marker codec | new `anchor.rs` or equivalent |

Protocol changes MUST update this specification, shared codecs/constants, golden vectors, and conformance tests in the same change.

## 17. Deliberately deferred work

The following are not part of Vivid 1.0:

- damage-rectangle raster deltas and base-frame recovery;
- RGB8, indexed/palette, HDR, or 10-bit raster formats;
- content-addressed blob caching;
- detailed frame callbacks, presentation timestamps, drop telemetry, and adaptive-quality feedback;
- generic media fragmentation;
- local shared-memory, `memfd`, or DMA-buffer transport;
- source reconfiguration in place;
- exhaustive codec/profile capability catalogs;
- session resumption;
- terminal-multiplexer passthrough;
- source transcoding.

These require separate state machines or platform contracts and should be specified as negotiated extensions after the 1.0 correctness, interoperability, and security changes have stable conformance coverage.
