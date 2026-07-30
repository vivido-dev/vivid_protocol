# Vivid Protocol 1.5 Media

This file is a normative part of the
[Vivid Protocol 1.5 specification](vivid-protocol-1.5-spec.md).

It defines `live-media-v1` and `timed-media-v1`.

## 1. Track model

A track is one immutable media configuration owned by one stable surface. A track's codec,
packetization, coded dimensions, pixel format, colorimetry, audio layout, mode, resource claims,
maximum record body, and lane never change.

Changing any immutable field creates a replacement track. Scene nodes and input remain bound to
the surface.

Track kinds are video (`1`), audio (`2`), raster (`3`), and encoded image (`4`). Surface slots are:

| Value | Slot | Legal kinds |
|---:|---|---|
| 1 | `primary-video` | Video |
| 2 | `audio` | Audio |
| 3 | `raster` | Raster |
| 4 | `poster` | Encoded image or raster |
| 5–31 | Reserved | None |
| 32 and above | Application-defined auxiliary slot | Kind declared by the track |

At most one track is active in a slot. A track may exist and be primed while inactive.

Track modes are live (`1`) and timed (`2`). Live mode requires `live-media-v1`. Timed mode requires
`timed-media-v1`.

### 1.1 Live mode

Live tracks:

- do not require `PLAY`;
- make the first decodable key unit or full frame eligible immediately;
- drop obsolete decoded output to remain within the configured target-latency policy;
- use the active surface audio slot as clock when it is present and healthy;
- retain only bounded latest-frame/poster state; and
- recover a channel with a new channel generation and key/full unit.

### 1.2 Timed mode

Timed tracks use exact PTS playback, pre-roll, pause, flush epochs, ordered EOS, and drain. The
active audio slot is the master clock for the surface playback group when present.

## 2. Track configuration and resource claims

`PROBE_TRACK_CONFIG` and `CREATE_TRACK` share the common schema. A probe uses track ID zero and
does not allocate or carry a descriptor/policy; creation uses a nonzero track ID.

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Owning context ID |
| 1 | uint | Owning surface ID |
| 2 | uint | Track ID; zero for probe |
| 3 | uint | Track kind |
| 4 | uint | Intended surface slot |
| 5 | uint | Mode: live (`1`) or timed (`2`) |
| 6 | uint | Lane: realtime (`2`) or bulk (`3`) |
| 7 | uint | Maximum media-record body, nonzero |
| 8 | uint | Maximum frame/access-unit rate in millihertz |
| 9 | uint | Maximum encoded bits per second |
| 10 | uint | Maximum media records per second |
| 11 | uint | Requested maximum in-flight body bytes |
| 12 | map | Kind-specific immutable configuration |
| 13 | uint | Target latency in microseconds; live mode |
| 14 | uint | Maximum latency in microseconds |
| 15 | uint | Requested retained-pixel charge |

This is a strict schema.

Keys 8 through 11 are contractual claims. Zero is invalid for a created streaming track. For a
still image, key 8 is 1, key 9 covers transfer size over the presenter's minimum accounting
interval, and key 10 is 1.

The presenter checks:

```text
coded_pixels = coded_width * coded_height

reserved_decoded_pixels_per_second =
    ceil(coded_pixels * maximum_rate_millihertz / 1000)
```

against the complete context ancestry. It also checks the requested bitrate, record rate, decoder
count, body size, in-flight bytes, audio layout, and retained pixels under the security resource
model. Checked arithmetic is mandatory.

`PROBE_TRACK_CONFIG` returns `TRACK_SUPPORT` with:

| Key | Type | Meaning |
|---:|---|---|
| 0 | bool | Exact configuration admissible now |
| 1 | text | Selected decoder/profile name for diagnostics |
| 2 | uint | Capability generation |
| 3 | map | Effective resource claims if created now |

Probes are authoritative only for that complete configuration and capability generation. They do
not reserve capacity.

## 3. Kind-specific configuration

### 3.1 Video

Video configuration map:

| Key | Type | Meaning |
|---:|---|---|
| 0 | text | Canonical codec name |
| 1 | text | Packetization identifier |
| 2 | bytes | Codec extradata, at most 65,536 bytes |
| 3 | uint | Coded width, `1..=8192` |
| 4 | uint | Coded height, `1..=8192` |
| 5 | int | Codec profile representable as `i32` |
| 6 | int | Codec level representable as `i32` |
| 7 | uint | Alpha mode; no alpha (`0`) |
| 8 | uint | Maximum reorder depth, at most 64 |
| 9 | text | Timeline; `source-timebase-us` |
| 10 | uint | Color primaries |
| 11 | uint | Transfer characteristic |
| 12 | uint | Matrix coefficients |
| 13 | uint | Signal range |
| 14 | uint | Sample-aspect-ratio numerator, nonzero |
| 15 | uint | Sample-aspect-ratio denominator, nonzero |
| 16 | uint | Maximum encoded access-unit bytes, nonzero |
| 17 | text, optional | RFC 6381 codec string, at most 64 printable ASCII bytes |
| 18 | bytes, optional | Matching decoder configuration body, at most 4,096 bytes |

Canonical pairs:

| Codec | Packetization |
|---|---|
| `h264` | `h264-annexb-au-v1` |
| `hevc` | `hevc-annexb-au-v1` |
| `vp9` | `vp9-frame-v1` |
| `av1` | `av1-low-overhead-tu-v1` |

Color primaries are BT.709 (`1`), BT.601 625-line (`2`), BT.601 525-line (`3`), and BT.2020 (`4`).
Transfer is BT.709 (`1`) or sRGB (`2`); PQ (`3`) and HLG (`4`) remain reserved. Matrix is
identity/RGB (`0`), BT.709 (`1`), BT.601 (`2`), or BT.2020 non-constant luminance (`3`). Signal
range is limited (`1`) or full (`2`).

The presenter never silently substitutes packetization or colorimetry.

### 3.2 Audio

Audio configuration map:

| Key | Type | Meaning |
|---:|---|---|
| 0 | text | Canonical codec name |
| 1 | text | Packetization identifier |
| 2 | bytes | Codec extradata, at most 65,536 bytes |
| 3 | uint | Sample rate, `8000..=192000` |
| 4 | uint | Channel count, `1..=8` |
| 5 | uint | Channel mask, or zero for default layout |
| 6 | uint | Maximum encoded access-unit bytes, `1..=1048576` |
| 7 | text | Timeline; `source-timebase-us` |
| 8 | text, optional | RFC 6381 codec string |

A nonzero channel mask contains exactly the declared channel count.

Canonical pairs:

| Codec | Packetization |
|---|---|
| `mp3` | `mp3-frame-v1` |
| `aac` | `aac-raw-au-v1` |
| `alac` | `alac-frame-v1` |
| `opus` | `opus-packet-v1` |
| `vorbis` | `vorbis-packet-v1` |
| `flac` | `flac-frame-v1` |
| `pcm_u8`, `pcm_s16le`, `pcm_s24le`, `pcm_s32le`, `pcm_f32le`, `pcm_f64le`, `pcm_mulaw`, `pcm_alaw` | `pcm-packet-v1` |

### 3.3 Raster

Raster configuration map:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Width, `1..=8192` |
| 1 | uint | Height, `1..=8192` |
| 2 | uint | Pixel format; RGBA8 (`1`) |
| 3 | uint | Alpha: straight (`1`) or premultiplied (`2`) |
| 4 | uint | Update mode: full (`0`) or full-and-delta (`1`) |
| 5 | uint | Maximum delta operations, `1..=16` |
| 6 | uint | Compression: raw (`0`) or raw-or-zstd (`1`) |
| 7 | uint | Color space; sRGB (`1`) |

For full-only mode, key 5 is one. A delta-capable track remains admissible for a raw full frame:

```text
raw_full_body = 72 + width * height * 4
```

Under the 64 MiB ceiling, the maximum raw pixel count is 16,777,198. A 4096×4096 raw frame does
not fit because of its 72-byte header.

### 3.4 Encoded image

Encoded-image configuration map:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Encoding: PNG (`1`) or JPEG (`2`) |
| 1 | uint | Decoded width, `1..=8192` |
| 2 | uint | Decoded height, `1..=8192` |
| 3 | uint | Exact encoded byte length |
| 4 | bytes(32), optional | SHA-256 of exact bytes |
| 5 | uint | Output color space; sRGB (`1`) |
| 6 | bool | Request context-local immutable cache lookup |

Cache lookup requires the hash. Only single-image PNG and JPEG are supported. Orientation metadata
is ignored; pixels arrive in intended orientation.

## 4. Track creation and lifecycle

Successful `CREATE_TRACK` returns `TRACK_READY`:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Context ID |
| 1 | uint | Surface ID |
| 2 | uint | Track ID |
| 3 | uint | Initial track revision |
| 4 | uint | Initial channel generation, always one |
| 5 | uint | Channel-open deadline from reply in microseconds |
| 6 | uint | Maximum media-record body |
| 7 | map | Effective resource claims |
| 8 | bool | Track connection required |
| 9 | uint | Effective delta-operation limit, raster only |

The record object ID equals track ID. A normal streaming track requires a connection. A permitted
encoded-image cache hit sets key 8 false, has no channel, is immediately ready, and still consumes
track, retained-pixel, and cache-reference accounting.

The open deadline is nonzero and at most 30 seconds. If no channel is accepted by the deadline, the
track remains queryable but unavailable; the producer may issue `ADVANCE_CHANNEL` to create a new
generation and deadline or destroy the track. The presenter may expire chronically unattached
tracks under the lease and context policy.

`DESTROY_TRACK` carries context, surface, and track IDs. It closes the channel, releases resources,
and removes the active slot only if it named this track. It does not destroy the surface or its
nodes.

`TRACK_LOST` is an actionable, uncorrelated control-connection event:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Context ID |
| 1 | uint | Surface ID |
| 2 | uint | Track ID |
| 3 | uint | Error code |
| 4 | uint | Final track revision |
| 5 | map | Structured detail |
| 6 | text | Bounded diagnostic |

A lost track accepts no channel or media. If active, its slot remains associated with a degraded
track until explicit replacement or destruction; a policy-permitted last poster may remain. Nodes
and input surface identity remain.

## 5. Authenticated channel opening

### 5.1 `CHANNEL_OPEN`

A track connection begins with `CHANNEL_OPEN`, record sequence one:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Session ID |
| 1 | uint | Context ID |
| 2 | uint | Surface ID |
| 3 | uint | Track ID |
| 4 | uint | Channel generation |
| 5 | uint | Track kind |
| 6 | uint | Lane |
| 7 | bytes(16) | Client channel nonce |
| 8 | bytes(16) | Authentication tag |

The record object ID equals track ID. The tag is:

```text
HMAC-SHA256(
    session_channel_key,
    "VIVID-CHANNEL-1" ||
    session_id_be64 ||
    context_id_be64 ||
    surface_id_be64 ||
    track_id_be64 ||
    channel_generation_be64 ||
    track_kind_be32 ||
    lane_be32 ||
    client_channel_nonce
)[0..16]
```

The kind and lane must exactly match the immutable track configuration. The presenter compares the
tag in constant time before channel allocation.

### 5.2 Positive acceptance

The presenter returns `CHANNEL_ACCEPTED` on the track connection before any media:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Context ID |
| 1 | uint | Surface ID |
| 2 | uint | Track ID |
| 3 | uint | Accepted channel generation |
| 4 | uint | Maximum cumulative media-body bytes initially allowed |
| 5 | uint | Maximum cumulative media-record count initially allowed |
| 6 | uint | Maximum media-record body |
| 7 | uint | Track revision |

The initial byte allowance is at least one maximum legal record and the record allowance is at
least one. The producer sends no media before validating this reply.

### 5.3 Retry and replacement

The presenter stores a bounded outcome keyed by complete track identity, generation, nonce, and
open bytes.

- An exact duplicate while the accepted transport is still live receives `CHANNEL_BUSY`; the
  duplicate closes and does not gain flow authority.
- If the original transport is confirmed closed and accepted no media record, an exact retry may
  bind a replacement and returns the same initial logical allowance.
- Different bytes for the same generation are `BAD_MESSAGE`.
- Once any media record was accepted, a generation cannot attach again.
- Channel loss marks the track detached, discards decoder ingress, advances track revision, and
  retains only bounded poster/configuration state.

Actual reattachment requires `ADVANCE_CHANNEL` on control:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Context ID |
| 1 | uint | Surface ID |
| 2 | uint | Track ID |
| 3 | uint | Expected current channel generation |
| 4 | uint | New channel generation, exactly current plus one |
| 5 | uint | Reason |

It invalidates the old transport, resets generation-local flow totals and readiness milestones,
discards decoder state, and returns `CHANNEL_ADVANCED` with the new generation, open deadline, and
track revision. Video requires a key unit; raster requires a full frame; image requires the
complete image; audio starts with a valid independent access unit and its declared initialization.

No single-use ticket is minted or transported.

## 6. Absolute channel-local flow control

Flow-control state is scoped to one channel generation. The producer tracks:

```text
sent_body_bytes
sent_media_records
maximum_body_bytes
maximum_media_records
```

Before a media record:

```text
sent_body_bytes + body_length <= maximum_body_bytes
sent_media_records + 1        <= maximum_media_records
```

All additions are checked. The 24-byte record header, `CHANNEL_OPEN`, `CHANNEL_ACCEPTED`,
`MAX_CHANNEL_DATA`, recovery events, and `CHANNEL_EOS` are not charged.

The presenter raises limits with `MAX_CHANNEL_DATA` on the same track connection:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Context ID |
| 1 | uint | Surface ID |
| 2 | uint | Track ID |
| 3 | uint | Channel generation |
| 4 | uint | New maximum cumulative body bytes |
| 5 | uint | New maximum cumulative media records |

Each maximum is monotonically nondecreasing. Duplicate or reordered lower values are harmless and
ignored; a value that would exceed the context's finite aggregate capacity is never emitted.
Saturation is forbidden.

Flow updates are issued when ingress storage and queue capacity become reusable, or ownership
moves to separately bounded reserved storage. Raster-delta allowance is not returned until the
delta is fully applied to the retained base. Flow allowance is not a decode or presentation
acknowledgment.

A flow violation closes the track connection, detaches or loses only that track, and emits
`FLOW_CONTROL`. Other tracks and lanes remain live.

The presenter services reverse track traffic independently for every channel. One track may not
hold a shared executor, lock, or writer that prevents another track's flow or recovery.

## 7. Readiness and atomic activation

Generation-local milestone bits are:

| Bit | Milestone |
|---:|---|
| 0 | Channel accepted |
| 1 | First media record accepted |
| 2 | Decoder initialized |
| 3 | Random-access video unit or full raster/image accepted |
| 4 | First decoded/composed output ready |
| 5 | First presentation for the current surface generation while an eligible node is visible |
| 6 | Timed playback clock started |
| 7 | EOS accepted |
| 8 | Buffered playback ended |
| 9 | Channel detached |
| 10 | Track lost |

Bits 0 through 8 reset on channel-generation advance. Each status reports the generation to which
the bits belong. A producer MUST NOT use a sticky bit from an old generation as current readiness.

`ACTIVATE_TRACK` atomically updates one or more slots at a compositor boundary:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Context ID |
| 1 | uint | Surface ID |
| 2 | array(map) | Slot bindings |
| 3 | uint | Expected surface revision |

Each binding map contains slot (`0`), track ID or zero to clear (`1`), expected channel generation
(`2`), and required current-generation milestone bit (`3`). All nonzero tracks belong to the
surface and match the slot.

The common live-media replacement requirement is milestone 4: decoded/composed output ready.
Activation applies all bindings or none. Success is `TRACK_ACTIVATED` after the compositor
boundary:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Context ID |
| 1 | uint | Surface ID |
| 2 | array(map) | Effective slot-to-track bindings |
| 3 | uint | New surface revision |
| 4 | uint | Presentation ID |

Activation does not change `surface_generation` because semantic coordinate and input identity did
not change. Old tracks remain valid but inactive until destroyed.

## 8. Track status, events, and waits

`QUERY_TRACK` carries the complete track identity. `TRACK_STATUS` returns:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Context ID |
| 1 | uint | Surface ID |
| 2 | uint | Track ID |
| 3 | uint | Track revision |
| 4 | uint | Kind |
| 5 | uint | Mode |
| 6 | uint | Lifecycle: created (`0`), attached (`1`), ready (`2`), active (`3`), detached (`4`), ended (`5`), lost (`6`), tombstone (`7`) |
| 7 | uint | Current channel generation |
| 8 | uint | Attachment state: never (`0`), active (`1`), closed (`2`) |
| 9 | uint | Current-generation milestone mask |
| 10 | uint | Current media epoch |
| 11 | uint | Last accepted packet/frame ID |
| 12 | uint | Last accepted media record sequence |
| 13 | int | Last decoded PTS |
| 14 | int | Last presented PTS |
| 15 | uint | Last presentation ID |
| 16 | uint | Cumulative accepted body bytes |
| 17 | uint | Cumulative accepted media records |
| 18 | uint | Current maximum body bytes |
| 19 | uint | Current maximum media records |
| 20 | uint | Ingress depth bucket |
| 21 | map, optional | Playback state |
| 22 | uint, optional | Terminal loss code |

Keys 10 through 17 are presenter-accepted progress, not producer-submission acknowledgments.
Control and track connections are independently ordered, including when they are carried through
SSH forwarding, WebTransport streams, WebSocket substreams, or distinct native connections.
Consequently, a `TRACK_STATUS` response can legitimately lag media records that the producer has
already written. A producer MUST preserve its own track-wide increasing media ID and epoch state,
MUST NOT move that submitted state backward to the status snapshot, and MUST NOT infer loss merely
because accepted progress lags submitted progress. It may merge accepted IDs or epochs that are
ahead, such as after authenticated resume and reconciliation.

Key 12 is the exact wire record sequence of the last accepted media record on the current track
connection. It is not a media-record count. `CHANNEL_EOS` remains the ordered acceptance barrier
because it travels on that same connection.

`TRACK_CHANGED` is a coalescible observation and names the current track revision and changed-field
mask. It never carries flow authority or recovery requirements.

`WAIT_TRACK` is a bounded correlated request:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Context ID |
| 1 | uint | Surface ID |
| 2 | uint | Track ID |
| 3 | uint | Condition |
| 4 | uint, optional | Condition value |
| 5 | uint | Timeout in microseconds |
| 6 | uint | Required channel generation |

The timeout is nonzero and at most 30 seconds. Longer application waits use successive correlated
requests and treat each `TIMEOUT` result as an opportunity to re-evaluate current track state.

Conditions are track revision greater than value (`1`), current-generation milestone set (`2`),
presented raster frame at least value (`3`), presented video PTS at least value (`4`), playback
started (`5`), playback ended (`6`), channel accepted (`7`), channel closed (`8`), and track lost
(`9`).

Generation mismatch returns `STALE_CHANNEL_GENERATION`. A presentation condition that cannot be
satisfied because no node is eligible at this hop returns `NOT_VISIBLE`, not timeout.

Success returns `WAIT_SATISFIED` with the complete track identity, track revision, channel
generation, condition, and optional observed value. Timeout returns `TIMEOUT`; cancellation
returns `CANCELLED`; destruction returns `NOT_FOUND`.

`TRACK_CHANGED` carries the complete track identity, current track revision, current channel
generation, changed-field mask, observation sequence, and optional first-lost sequence. Changed
bits are lifecycle (`0`), channel (`1`), readiness (`2`), activation (`3`), playback (`4`), flow
accounting (`5`), and recovery (`6`).

## 9. Video packet format

A `VIDEO_PACKET` body is unchanged from the portable 1.1 body:

| Offset | Size | Field |
|---:|---:|---|
| 0 | 4 | Media epoch |
| 4 | 4 | Flags |
| 8 | 8 | Packet ID |
| 16 | 8 | PTS in microseconds, signed |
| 24 | 8 | DTS in microseconds, signed |
| 32 | 8 | Duration in microseconds |
| 40 | 4 | Side-data length; zero |
| 44 | 4 | Reserved; zero |
| 48 | variable | Encoded access unit |

Flag bit 0 is `KEY`; bit 1 is `DELTA`; exactly one is set. Packet IDs are nonzero and strictly
increase for the track across media epochs and channel generations. Exhaustion requires a new
track.

Epochs are monotonically nondecreasing. The first packet of a track or recovered channel is a key
packet. The first packet of a greater epoch is a key packet. An older epoch is rejected
`STALE_EPOCH`; a greater epoch beginning with delta emits `NEED_KEYFRAME`.

Packets are in decoder-input order. PTS and DTS use `source-timebase-us`. One packet may decode to
zero, one, or several frames.

Canonical packetizations:

- H.264 contains exactly one Annex B access unit with three- or four-byte start codes.
  Extradata is empty or Annex B parameter-set NAL units. AVCC lengths are forbidden.
- HEVC contains exactly one Annex B access unit. Extradata is empty or Annex B VPS/SPS/PPS.
- VP9 contains one complete compressed frame without container framing; extradata is empty.
- AV1 contains one temporal unit in low-overhead OBU form; extradata is empty or one sequence
  header OBU.

`KEY` means decoding can begin with the declared extradata and that access unit.

## 10. Audio packet format

An `AUDIO_PACKET` body is unchanged from the portable 1.1 body:

| Offset | Size | Field |
|---:|---:|---|
| 0 | 4 | Media epoch |
| 4 | 4 | Reserved; zero |
| 8 | 8 | Packet ID |
| 16 | 8 | PTS in microseconds, signed |
| 24 | 8 | DTS in microseconds, signed |
| 32 | 8 | Duration in microseconds |
| 40 | 4 | Leading trim samples |
| 44 | 4 | Trailing trim samples |
| 48 | variable | Encoded access unit |

Packet IDs strictly increase across epochs and channel generations. The body is nonempty and at
most the declared maximum and 1 MiB.

- MP3 contains one MPEG audio frame.
- AAC contains one raw AAC access unit; extradata is one AudioSpecificConfig, not ADTS.
- ALAC contains one ALAC frame.
- PCM contains integral interleaved samples in the named representation.
- Opus contains one packet. Extradata is canonical `OpusHead`; decode rate is 48 kHz, mapping
  families 0 and 1 are supported, and pre-skip is applied once.
- Vorbis contains one packet. Extradata contains the three initialization headers in Xiph lacing
  form: leading byte 2, laced identification/comment lengths, then identification, comment, setup.
- FLAC contains one frame. Extradata is exactly the 34-byte raw STREAMINFO payload, excluding
  `fLaC` and metadata-block header.

All internal boundaries, versions, sample rate, channel count, mapping, and resulting decoder
configuration are validated before decoder allocation.

Audio decode and resampling use a finite device buffer no longer than two seconds. Timed playback
underrun emits silence while the master clock continues. Audio failure does not destroy the
surface or active video track.

## 11. Raster packet format

### 11.1 Full frame

A full `RASTER_FRAME` body has a 48-byte frame header, one 24-byte descriptor, and pixels:

| Offset | Size | Field |
|---:|---:|---|
| 0 | 4 | Epoch |
| 4 | 4 | Flags |
| 8 | 8 | Frame ID |
| 16 | 8 | Base frame ID; zero |
| 24 | 8 | PTS, signed microseconds |
| 32 | 8 | Duration |
| 40 | 4 | Operation count; one |
| 44 | 4 | Reserved; zero |
| 48 | 4 | X; zero |
| 52 | 4 | Y; zero |
| 56 | 4 | Width |
| 60 | 4 | Height |
| 64 | 4 | Data offset; zero |
| 68 | 4 | Data length |
| 72 | variable | Pixels |

Flags are full (`bit 0`), zstd (`bit 1`), and delta (`bit 2`). Exactly one of full and delta is
set. Frame IDs strictly increase across epochs and channel generations. The first recovered frame
is full.

Raw data length is `width * height * 4`. Pixels are row-major RGBA8. Premultiplied samples have
zero color when alpha is zero and no normalized color component greater than alpha.

Zstd data is exactly one zstd frame with no dictionary or skippable frame. It consumes the entire
payload and expands to the already validated exact raw length. The presenter never trusts a size
declared by the compressed stream.

### 11.2 Delta

A delta names the immediately preceding accepted frame as nonzero base ID. It contains `1..=limit`
32-byte operations followed by overwrite payloads:

| Operation offset | Size | Field |
|---:|---:|---|
| 0 | 4 | Kind: overwrite (`1`) or copy (`2`) |
| 4 | 4 | Destination X |
| 8 | 4 | Destination Y |
| 12 | 4 | Width |
| 16 | 4 | Height |
| 20 | 4 | Source X; zero for overwrite |
| 24 | 4 | Source Y; zero for overwrite |
| 28 | 4 | Payload length; zero for copy |

All bounds, payload lengths, decompression, and final body length are validated before mutation.
Operations apply in order. Copy behaves as if the source rectangle were first copied to a
temporary, so overlap is exact.

The presenter rejects a missing base and emits `NEED_FULL_FRAME`. It maintains a finite
accumulated-damage budget. Once exceeded, it rejects deltas until a full frame. An unapplied delta
cannot be coalesced away because later deltas depend on it.

A nested presenter terminates the incoming delta chain, composes a full retained raster, and
independently chooses full or delta on its outgoing hop.

Raster is immediate in live mode. PTS is metadata. Accepted full frames may coalesce latest-wins;
deltas first compose into the base.

## 12. Encoded-image transfer and caching

After channel acceptance, an encoded-image track sends exactly one `IMAGE_DATA` body containing the
declared bytes.

The presenter validates length and optional SHA-256 before decode, uses an allowlisted decoder,
bounds decoded allocation independently of encoded length, requires exact declared dimensions,
rejects animated/multi-picture content, ignores orientation metadata, and produces sRGB pixels.

PNG alpha is straight. JPEG is opaque. External metadata resources are never fetched.

A context-local immutable cache key is:

```text
(context identity, encoding, exact SHA-256, exact length,
 decoded width, decoded height, output color space, decode profile)
```

One context cannot detect another's hit. A no-cache surface policy forbids lookup and seeding.
Active tracks own decoded references independently of eviction. Context cleanup purges its entries.

## 13. Recovery records

`NEED_KEYFRAME` travels presenter-to-producer on the affected track channel:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Context ID |
| 1 | uint | Surface ID |
| 2 | uint | Track ID |
| 3 | uint | Channel generation |
| 4 | uint | Minimum acceptable epoch |
| 5 | uint | Reason |
| 6 | uint, optional | Last accepted packet ID |

Reasons are initial/reconnect (`1`), decoder reset (`2`), invalid epoch (`3`), renderer reset (`4`),
and relay packet loss with decoder state otherwise intact (`5`). Reason 5 permits a key packet in
the current epoch; other reset reasons may require a greater epoch as reported.

`NEED_FULL_FRAME` has the complete track identity, generation, and reason: no base (`1`), damage
budget (`2`), renderer reset (`3`), policy/retention change (`4`), or reconnect (`5`).

These records are actionable, uncoalesced, and track-scoped. If the track transport is already
gone, current recovery requirement appears in `TRACK_STATUS`.

## 14. Ordered EOS

`CHANNEL_EOS` is sent on the track channel after the last media record:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Context ID |
| 1 | uint | Surface ID |
| 2 | uint | Track ID |
| 3 | uint | Channel generation |
| 4 | uint | Media epoch |
| 5 | uint | Last media record sequence |

Key 5 equals the immediately preceding media record's exact track-connection sequence, or zero
when no media record was sent. Because EOS shares the ordered channel, no cross-connection
media-order barrier exists.

EOS closes ingress for this track generation after every prior record was validated and accepted.
It does not pause and does not cascade to another slot. Buffered timed output continues. A channel
failure before EOS acceptance leaves EOS unapplied and recovery is explicit.

## 15. Timed playback

This section requires `timed-media-v1`.

`PLAY` carries complete track identity plus:

| Key | Type | Meaning |
|---:|---|---|
| 3 | int | Exact start PTS |
| 4 | uint | Minimum buffer |
| 5 | uint | Maximum latency |
| 6 | int | Signed 32.32 playback rate; baseline 1.0 |
| 7 | uint | Late policy; drop presentation (`1`) |
| 8 | uint | Loop count; zero |
| 9 | uint | Start policy; after minimum buffer (`1`) |
| 10 | uint | Required channel generation |

The presenter validates and admits the request without blocking control. `OK` means admitted, not
started. The clock begins when the minimum buffered horizon exists or accepted EOS proves no more
pre-roll will arrive. Media before `start_pts` is discarded. `start_pts` maps exactly to clock
start.

Active audio is master clock. Initial silence or leading trim aligns PTS. Audio underrun emits
silence while the clock continues. Late video frames drop under policy.

`PAUSE` freezes the surface playback group, holds latest video, and stops consuming audio.

`FLUSH` carries a new `u32` epoch greater than current. It discards queued decoder state, pauses,
and requires a new key unit. It deliberately does not wait for old media order; the producer waits
for its reply before writing the new epoch.

`DRAIN` for audio completes only after EOS, decoder/resampler flush, and consumption of queued
device samples. It is bounded pending state and does not block control.

`PLAYBACK_STATE` is a transition-only observation:

| Key | Meaning |
|---:|---|
| 0–2 | Complete context, surface, track identity |
| 3 | State: idle (`0`), buffering (`1`), playing (`2`), paused (`3`), ended (`4`), lost (`5`) |
| 4 | Current clock PTS |
| 5 | Epoch |
| 6 | Buffered-ahead microseconds |
| 7 | Underrun count |
| 8 | Late-drop count |
| 9 | EOS: not received (`0`), accepted (`1`), applied (`2`) |
| 10 | Track revision |

Already-buffered media plays to completion after EOS. EOS is never an implicit pause.

## 16. Media conformance

A producer:

1. treats track configuration as immutable and replaces a track rather than reconfiguring it;
2. waits for `CHANNEL_ACCEPTED` before media;
3. obeys absolute byte and record maxima and sustained-rate claims;
4. uses explicit channel-generation advance for reattachment;
5. primes a replacement to its required current-generation milestone before atomic activation;
6. sends a key/full recovery unit after channel generation change;
7. sends EOS on the affected track channel; and
8. never changes scene or input identity solely because a track changes.

A presenter:

1. admits worst-case resource claims before decoder or large allocation;
2. authenticates a complete track identity and generation on channel open;
3. permits at most one live transport per generation;
4. sends flow updates on the corresponding channel as cumulative maxima;
5. resets current-generation milestones on reattach;
6. keeps track failure from deleting the surface or nodes;
7. atomically activates a slot set at a compositor boundary; and
8. validates portable initialization and decoded sizes before use.

Regression suites cover lost channel acceptance, duplicate opens, half-open old transports,
generation advance, zero and duplicate flow updates, maximum counters near overflow, channel loss
during parsing, two owners reusing every local ID, replacement without input changes, live A/V
clocking, timed pre-roll, pause, flush, EOS, drain, keyframe recovery, and full-frame recovery.
