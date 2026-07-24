# Vivid Protocol 1.1 Specification

**Status:** draft normative Vivi/Vivido interoperable profile. This document is expected to change
during implementation; it is not frozen.
**Vivid version:** 1.1
**Compatibility:** Vivid 1.1 only. Vivid 1.1 is a coordinated ecosystem cutover, not a negotiated
upgrade from Vivid 1.0.
**Derived from:** `vivid-protocol-1.0-spec.md`, plus the accepted items in
`docs/vivid-protocol-agent-improvements-v2.md`.

## 1. Conventions and scope

The key words **MUST**, **MUST NOT**, **REQUIRED**, **SHALL**, **SHALL NOT**, **SHOULD**, **SHOULD NOT**, **RECOMMENDED**, **NOT RECOMMENDED**, **MAY**, and **OPTIONAL** are normative requirement levels.

Vivid is a terminal-attached media protocol. It keeps bulk media off the terminal PTY while allowing authenticated producers to create media sources, place those sources in a terminal-owned scene, and bind placements to semantic text positions.

Vivid 1.1 defines everything Vivid 1.0 defined:

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

Vivid 1.1 adds:

- a forward-compatible negotiation encoding that survives authenticated relays;
- a normative control-plane execution model;
- structured, machine-parseable error detail;
- scene, source, and observation revisions with bounded status queries and one-shot waits;
- playback state transitions and precise source milestones;
- typed preconditions, idempotency, and causation identifiers;
- opaque delegated context capabilities;
- source descriptors and capture/export policy;
- damage-rectangle and copy-rectangle raster deltas;
- context-local encoded-image reuse;
- an EOS media-order barrier;
- credit-window advertisement;
- timestamped liveness probes for diagnostic clock alignment;
- a typed version-rejection result.

Assigned operations without a payload schema remain reserved for a negotiated future profile. Assignment alone does not require implementation support.

### 1.1 Roles

A **presenter** owns a terminal window, authenticates producers, decodes media, maintains scene state, and renders frames.

A **producer** creates sources and nodes and supplies media.

A **control connection** owns one producer session. A **media connection** carries media for exactly one source after ticket attachment.

Media bytes MUST NOT be transported through the terminal PTY. The PTY is used only for ordinary terminal data and the bounded text-anchor marker in Section 13.

### 1.2 Vivid version

The 16-byte connection preface identifies Vivid version `1.1`; connections write preface bytes major `1`, minor `1`.

The same Vivid version is offered by `HELLO` and selected by `WELCOME`.

A Vivid 1.1 producer MUST offer a range containing version 1.1. A conforming presenter selects only version 1.1 and rejects a range that does not contain it. `WELCOME` keys 13 through 15 are mandatory; their absence is `BAD_MESSAGE`, not a downgrade signal.

Vivid 1.1 is **not** reachable by negotiation from a Vivid 1.0 peer. A Vivid 1.0 receiver validates the preface before reading any record and rejects a nonzero minor version, so a 1.1 preface never reaches `HELLO` on a 1.0 peer and version-range negotiation cannot select 1.0 from a 1.1 connection. Implementations MUST NOT attempt a silent downgrade, MUST NOT emit a 1.0 preface followed by 1.1 semantics, and MUST NOT revive retired packet profiles. Section 3.6 defines the only permitted mixed-version behavior.

A producer MUST NOT use an optional feature unless the presenter advertises the corresponding accepted feature or profile.

### 1.3 Byte order, integer widths, and units

All fixed-width multibyte integers are big-endian.

Unless a narrower width is stated:

- `uint` means an integer in `0..=2^64-1`;
- `int` means an integer in `-2^63..=2^63-1`.

A value outside its declared width is `BAD_MESSAGE`.

Pixel dimensions are unsigned integer pixels. Timeline fields ending in `_us` are integer microseconds.

Scene geometry is signed 32.32 fixed point stored in an `i64`, where one terminal cell is `1 << 32`. Width and height MUST be positive. Implementations MUST use checked arithmetic for conversion, clipping, and composition; overflow is `BAD_MESSAGE` or `LIMIT_EXCEEDED`, not wraparound.

### 1.4 Revisions and identifier domains

Vivid 1.1 introduces three monotonic counters, each a checked `u64` scoped to one producer session:

| Counter | Scope | Advances on |
|---|---|---|
| `scene_revision` | Session | Every applied scene commit, and every automatic node-set change caused by source loss, anchor loss, context revocation, or policy teardown |
| `source_revision` | Source | Lifecycle, epoch, playback, policy, descriptor, attachment, and terminal-loss transitions |
| `observation_sequence` | Session | Every non-actionable observation event emitted to that session |

These counters MUST NOT wrap. Exhaustion closes the owning session with `LIMIT_EXCEEDED` and a fatal `ERROR`.

The following identifier domains are distinct and MUST NOT be conflated, compared, or forwarded across hops as if equivalent: display generation, capability generation, media packet and frame IDs, media record sequences, attachment generations, presentation IDs, semantic content revisions, and the three counters above. A bridge that re-originates a session maintains its own independent values in every domain.

### 1.5 Media hot-path invariant

Vivid 1.1 adds no field, prefix byte, or per-record parsing step to `VIDEO_PACKET`, `AUDIO_PACKET`, or the full-frame `RASTER_FRAME` layout. The 48-byte video prefix (Section 10.1), the 48-byte audio prefix (Section 10A), and the 72-byte full-frame raster header and rectangle descriptor (Section 11.1) are byte-identical to Vivid 1.0.

The raster delta form in Section 11.4 is a separate negotiated body form of the same record type, selected by a frame flag, and does not alter the full-frame layout.

A negotiated but unused Vivid 1.1 feature MUST cause no additional media record, no additional media-record field, no additional allocation, and no additional syscall on the media path.

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
have been consumed. Section 7.16 defines how a producer resolves an attachment whose outcome is
unknown without retrying a ticket.

Where the operating system exposes peer credentials for Unix-domain streams, the presenter MUST verify that the connecting peer has the same effective user identity as the presenter owner. Failure is `AUTH_FAILED` and closes the connection.

The TCP form provides no Vivid-level confidentiality, integrity, or network authentication. It MUST NOT be exposed to an untrusted network.

### 2.2 Capability handling

The capability token MUST NOT be placed in command-line arguments, diagnostic output, shell history, or logs. A producer SHOULD read it from the inherited environment, copy it into protected process memory, and remove it from any environment passed to unrelated child processes.

Token comparison MUST be constant-time after exact-length hexadecimal decoding.

A delegated context capability (Section 7.18) is subject to every rule in this section. It MUST NOT appear in command-line arguments, URLs, terminal output, logs, trace bodies, serialized session state, or any unsolicited event.

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

Because a remote host may be upgraded independently of the local presenter, an SSH binding is a
likely source of mixed Vivid versions. Section 3.6 applies unchanged across this transport.

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

A bridge that rewrites an authenticated `HELLO` or `WELCOME` is not a byte-transparent relay and is
subject to the preservation requirements in Section 4.4. A new connection kind is never covered
automatically by an existing bridge or gateway route; it requires explicit route and subprotocol
support.

### 2.5 Stream and write behavior

A Vivid transport MUST provide a reliable, ordered, full-duplex byte stream. Vivid framing supplies message boundaries.

Generic transport compression is not part of Vivid 1.1.

A record is a logical stream unit, not a requirement for one operating-system write. Producers SHOULD write large media bodies in bounded batches and SHOULD avoid building an unbounded socket write queue. A sender MAY buffer records and flush on a bounded batch boundary, but it MUST flush a correlated request, a correlated reply, `CREDIT`, `PONG`, and any record that completes a state transition without additional delay. Delaying a credit or a liveness reply to form a batch is a conformance failure. An SSH binding MAY use a separate SSH connection for bulk media when interactive-input latency is more important than connection reuse.

### 2.6 Terminal multiplexers

Text anchors require all of the following:

- the current terminal client's endpoint and token reach the producer;
- APC bytes are preserved without rewriting;
- the marker arrives at the same presenter that issued the session tag.

When tmux, screen, or another multiplexer cannot guarantee those properties, the producer MUST NOT emit Vivid anchor markers. It MAY continue with ordinary grid-cell nodes. Multiplexer passthrough is outside the Vivid 1.1 interoperable profile.

## 3. Connection framing

### 3.1 Initiator preface

The producer opens every Vivid connection and writes exactly one 16-byte preface before its first record.

| Offset | Size | Field |
|---:|---:|---|
| 0 | 4 | ASCII `VIVD` |
| 4 | 1 | Vivid major version; `1` |
| 5 | 1 | Vivid minor version; `1` |
| 6 | 1 | Connection kind |
| 7 | 1 | Flags; MUST be zero |
| 8 | 4 | Initiator transmit-body limit |
| 12 | 4 | Reserved; MUST be zero |

The presenter does not send a reciprocal preface.

The transmit-body limit is the largest record body the initiator will transmit on that connection. It MUST be nonzero and MUST NOT exceed 67,108,864 bytes. The receiver may impose a lower limit.

Connection kinds are:

| Value | Name | Vivid 1.1 use |
|---:|---|---|
| 0 | Control | REQUIRED |
| 1 | Video | Used by `CREATE_VIDEO` |
| 2 | Raster | Used by `CREATE_RASTER` |
| 3 | Blob | Used by `CREATE_IMAGE` when `ENCODED_IMAGE_V1` is negotiated |
| 4 | Local buffer | Reserved |
| 5 | Audio | Used by `CREATE_AUDIO` when `AUDIO_ACCESS_UNIT_V1` is negotiated |

Vivid 1.1 defines no new connection kind. Reserved preface fields or flags that are nonzero are framing errors.

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

The record sequence is the exact, gap-checked ordering identifier for a connection and is the identifier used by the EOS media-order barrier in Section 7.8. An implementation SHOULD make the assigned sequence available to the caller that wrote the record.

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

A status or query reply defined in Section 7.16 is additionally bounded to 65,536 bytes regardless of the negotiated control ceiling.

### 3.4 Record flags and unknown records

Record flag bit 0 (`0x0001`) is `OPTIONAL`. Bits 1 through 15 are reserved and MUST be zero.

An unknown record with `OPTIONAL` set is consumed and ignored without state mutation. An unknown required record on a valid control connection produces `UNSUPPORTED_FEATURE`. An unknown required record on a media connection closes that media connection and makes the source lost. Reserved record-flag bits are a framing error.

A sender MUST NOT mark a request `OPTIONAL` if correctness depends on receiving a reply.

### 3.5 Framing stability across versions

The record header layout in Section 3.2, the record-flag rules in Section 3.4, and the deterministic CBOR envelope in Section 4 are identical in Vivid 1.0 and Vivid 1.1. A receiver that has rejected a peer's preface version can therefore still emit exactly one well-formed record on that connection before closing it, as required by Section 3.6.

### 3.6 Version rejection

A receiver that rejects a preface solely because of its Vivid version SHOULD, before closing the connection, write exactly one session-level `ERROR` record with:

- error code `UNSUPPORTED_VERSION`;
- failed request ID zero;
- fatal `true`;
- detail keys 11 and 12 (Section 14.2) carrying the receiver's own supported major and minor version.

The receiver then closes the connection. It MUST NOT allocate producer-controlled session, source, scene, or ticket state, MUST NOT read a `HELLO`, MUST rate-limit these replies, and MUST NOT emit one for a malformed magic value, a malformed record header, a reserved nonzero preface field, or an invalid transmit-body limit. Those remain silent closes.

The connection is closed in every case. This is a diagnostic result, not a negotiation, and it does not create a version-downgrade path.

An initiator that receives such an `ERROR` MAY retry **once**, on a **new connection**, using a version the reply reported as supported, if and only if the initiator implements that version completely and the retry is explicitly enabled by its operator. The retry MUST be recorded in the initiator's diagnostics. Retrying is OPTIONAL, MUST default to disabled, MUST NOT reuse the rejected connection, MUST NOT reuse a media ticket, and MUST NOT be inferred from any other signal. An initiator that does not implement the reported version reports the typed failure to its caller.

## 4. Deterministic control encoding

Control and event bodies contain exactly one deterministic CBOR value with this envelope:

```text
{
  0: uint,             # request ID; zero for unsolicited events
  ? 1: uint,           # transaction ID
  ? 2: uint,           # expected display generation
  3: { * uint => any },# opcode-specific payload
  ? 4: { * uint => any },  # typed preconditions; ATOMIC_CONTROL_V1
  ? 5: bytes(16),      # idempotency key; ATOMIC_CONTROL_V1
  ? 6: bytes(16)       # causation ID
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

Duplicate, non-integer, or unsorted map keys are `BAD_MESSAGE`. Missing required keys are `BAD_MESSAGE`.

Unknown payload keys MAY be ignored unless a message schema states otherwise. Source-creation schemas, decoder-configuration schemas, and resource-allocation schemas are strict: an unknown key is `BAD_MESSAGE` unless a negotiated profile explicitly assigns and permits it. Schemas that explicitly allow ignorable future keys retain that behavior. Section 4.4 defines the opposite rule for the two negotiation schemas.

Envelope keys 4 and 5 MUST NOT be ignored. A receiver that has not accepted `ATOMIC_CONTROL_V1` MUST answer any record carrying envelope key 4 or 5 with `BAD_MESSAGE`. A receiver that has accepted it MUST evaluate them as specified in Section 4.2. Envelope key 6 is safely ignorable.

### 4.1 Requests, replies, and pipelining

A request that expects correlation uses a nonzero request ID. A producer MUST NOT reuse a request ID while any reply or error for the earlier request can still arrive.

Unsolicited presenter events use request ID zero. Replies copy the request ID of the request they answer.

A producer MAY pipeline requests without waiting for earlier replies when it already possesses all values needed to construct them. A presenter MUST apply state-mutating control records in receive order on a control connection. Replies MAY be emitted after independent work completes and therefore MAY be observed in a different order; producers correlate by request ID.

The following dependencies cannot be bypassed by pipelining:

- `HELLO` must be accepted before any session operation;
- a media ticket is unavailable until `SOURCE_READY`;
- an operation that requires a returned identifier or capability must wait for that value.

### 4.2 Preconditions, idempotency, and causation

These mechanisms require `ATOMIC_CONTROL_V1` and apply only to state-changing control requests. A probe, query, wait, or liveness record MUST NOT carry envelope key 4 or 5.

**Preconditions.** Envelope key 4 contains a bounded map of expected values:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Expected `scene_revision` |
| 1 | uint | Expected `source_revision` of the object being mutated |
| 2 | uint | Expected source epoch |
| 3 | uint | Expected source lifecycle state (Section 7.16) |
| 4 | uint | Expected anchor state: ready (`1`) or gone (`2`) |
| 5 | uint | Expected semantic content revision (Section 7.13) |

A precondition applies to the object identified by the record. A precondition key that is not meaningful for the target operation is `BAD_MESSAGE`, not a silently ignored value. Expected display generation keeps its own distinct envelope key 2 and is unchanged from Vivid 1.0.

Every supplied precondition is evaluated before any mutation. If any fails, the presenter applies **no** mutation and returns `ERROR` with code `PRECONDITION_FAILED`, detail key 6 naming the failed precondition kind, and the current sanitized values in detail keys 3, 4, and 5 as applicable.

**Idempotency.** Envelope key 5 contains 16 bytes generated by a cryptographically secure random source. It is valid only within the current authenticated session or delegated context and carries no authority.

A presenter maintains a bounded map from idempotency key to a hash of the complete request plus a non-secret outcome. Reusing a key with different request bytes is `BAD_MESSAGE`. A matching retry returns the previously computed non-secret result without applying the mutation a second time.

Source creation is special. A presenter MUST NOT cache or replay a media ticket as an idempotency result. When a replayed source-creation request matches an already-created source, the presenter answers `ERROR` with code `ALREADY_APPLIED`, detail key 9 set, and the source ID in the record object ID. The producer then resolves the source's real state, including whether its media channel is already attached, with `QUERY_SOURCE` (Section 7.16).

Idempotency state does not survive control-session loss.

**Causation.** Envelope key 6 contains 16 random bytes. It carries no authority and grants no capability. A presenter copies it into events that are a direct consequence of the request and into trace records, and nowhere else. It MUST NOT be derived from a token, capability, ticket, session tag, file path, or content hash. A bridge preserves a causation ID while recording its own identity translation.

### 4.3 Control-plane execution model

Both endpoints MUST service the control stream continuously and independently of media writes, credit availability, decoding, and presentation. In addition:

1. Parsing and request admission are ordered, and state mutations are applied in receive order.
2. An operation that cannot complete promptly MUST be registered as bounded pending state, and the control reader MUST yield.
3. While such an operation is outstanding, the endpoint MUST continue to answer `PING`, deliver and apply `CREDIT`, deliver actionable source events, satisfy unrelated waits, and emit independent replies.
4. `DRAIN`, capability probes, device opening, `PLAY` pre-roll, status queries, and source waits MUST NOT block the control reader or the session-state actor.
5. Pending operations, pending replies, and registered waits MUST be bounded in count; exceeding a bound returns `LIMIT_EXCEEDED` rather than growing the queue.

A conforming implementation can therefore be tested by issuing a long-running operation against one source and proving that `PING`, unrelated-source credits, and an unrelated `WAIT_SOURCE` all complete while it remains outstanding.

### 4.4 Negotiation-schema preservation

`HELLO` (Section 5.2) and `WELCOME` (Section 5.3) are negotiation schemas. Unlike the strict schemas in Section 4, they are **preserving**:

- a receiver MUST accept unknown payload keys in `HELLO` and `WELCOME` without error;
- an implementation that decodes and re-encodes either message MUST preserve every unknown key and value byte-for-byte in its output, in canonical key order, alongside the keys it does define;
- an implementation MUST NOT reorder, re-type, coalesce, truncate, or drop a preserved key;
- an implementation that recognizes a key but cannot honor its meaning MUST reject the message with `UNSUPPORTED_FEATURE` rather than silently dropping the key.

This rule binds authenticated relays specifically. A relay that substitutes the capability token in `HELLO` key 4, or replaces it with a non-secret placeholder in the peer-to-browser direction, MUST make that substitution its **only** mutation of the message. The same applies to any relay rewrite of `WELCOME`.

An implementation whose encoder emits a fixed-size map for either message does not conform. Conformance requires demonstrating that a `HELLO` carrying an unrecognized key arrives at the presenter byte-identical except for the substituted token, across every relay and gateway in the deployment.

## 5. Session establishment

### 5.1 Control ordering

The first control record MUST be session-level `HELLO` with object ID zero. The presenter validates framing, version range, required features, authentication material, and local peer identity before creating producer-controlled session resources.

Authentication failure returns `AUTH_FAILED` when safe to do so and ends establishment.

On success the presenter returns `WELCOME`. The session remains active until `GOODBYE`, control EOF, or a fatal control error. Control loss invalidates unused media tickets, closes attached media connections, cancels every registered wait, discards idempotency state, and applies the anchor/poster lifecycle in Section 13.

The execution requirements in Section 4.3 apply for the life of the session.

### 5.2 `HELLO`

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Minimum Vivid major version |
| 1 | uint | Minimum Vivid minor version |
| 2 | uint | Maximum Vivid major version |
| 3 | uint | Maximum Vivid minor version |
| 4 | text | Authentication material, exactly 64 hexadecimal characters |
| 5 | text | Producer name, at most 256 UTF-8 bytes |
| 6 | text | Producer version, at most 128 UTF-8 bytes |
| 7 | array(uint) | Required feature IDs, strictly increasing and unique |
| 8 | array(uint) | Optional feature IDs, strictly increasing and unique |
| 9 | uint | Maximum control-record body accepted from presenter |
| 10 | uint, optional | Authentication kind; window root token (`0`, default) or delegated context capability (`1`) |

Key 4 carries the 256-bit window capability token when key 10 is absent or zero, and a 256-bit delegated context capability (Section 7.18) when key 10 is one. Both are compared in constant time after exact-length hexadecimal decoding. Authentication kind `1` requires `DELEGATED_CONTEXT_V1`; a presenter that has not accepted that feature answers `UNSUPPORTED_FEATURE`.

The offered range MUST contain at least one Vivid version supported by the presenter. An unsupported required feature fails establishment with `UNSUPPORTED_FEATURE`. Unsupported optional features do not fail establishment.

`HELLO` is a preserving schema; see Section 4.4.

### 5.3 `WELCOME`

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Session ID |
| 1 | bytes(16) | Session tag |
| 2 | uint | Root context ID for this session |
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
| 16 | uint | Initial `scene_revision` |

Keys 13 through 16 are REQUIRED.

Key 2 is the private root context of the authenticated principal. For a session authenticated with a delegated capability it is that capability's bound context, and the session's authority is confined to that context's subtree.

Display values are authoritative for the returned display generation.

`WELCOME` is a preserving schema; see Section 4.4.

Recommended profile names are:

```text
raster-rgba8-full-v1
raster-zstd-full-v1
raster-delta-v1
image-png-jpeg-v1
image-cache-v1
video-access-unit-v1
audio-access-unit-v1
desktop-input-v1
text-anchor-cell-v2
visibility-source-v1
node-clip-rect-v1
observability-core-v1
atomic-control-v1
source-descriptor-v1
source-capture-policy-v1
delegated-context-v1
media-order-barrier-v1
clock-sampling-v1
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
| 17 | `DESKTOP_INPUT_V1` | Optional; terminal-independent keyboard and pointer input |
| 18 | `OBSERVABILITY_CORE_V1` | Optional; revisions, status queries, change events, waits |
| 19 | `ATOMIC_CONTROL_V1` | Optional; preconditions and idempotency |
| 20 | `SOURCE_DESCRIPTOR_V1` | Optional; source role, title, content revision, semantic availability |
| 21 | `DELEGATED_CONTEXT_V1` | Optional; contexts and opaque delegated capabilities |
| 22 | `SOURCE_CAPTURE_POLICY_V1` | Optional; capture and export policy bits |
| 23 | `RASTER_DELTA_V1` | Optional; damage and copy rectangles; requires `RASTER_RGBA8` |
| 24 | `IMAGE_CACHE_V1` | Optional; context-local encoded-image reuse; requires `ENCODED_IMAGE_V1` |
| 25 | `MEDIA_ORDER_BARRIER_V1` | Optional; EOS media-order barrier |
| 26 | `CLOCK_SAMPLING_V1` | Optional; timestamped liveness probes |

Feature ID 27 and above are unassigned in Vivid 1.1 and MUST NOT be negotiated. The portable Opus, Vorbis, and FLAC forms extend `AUDIO_ACCESS_UNIT_V1`; they do not allocate new feature IDs.

A feature that names a prerequisite MUST NOT be accepted unless the prerequisite is also accepted. A producer that requires such a feature MUST also list the prerequisite.

### 5.5 Capability generation and feature immutability

The set of accepted features and their wire syntax are fixed for the life of a session. `CAPS_CHANGED` MUST NOT remove an accepted feature, MUST NOT change the syntax of an accepted schema, and MUST NOT leave an already accepted schema in an indeterminate state.

`CAPS_CHANGED` reports a new capability generation, which changes when decoder, device, or policy availability for **future** source creation changes. It has these consequences and no others:

- probe results (`VIDEO_SUPPORT`, `AUDIO_SUPPORT`) obtained under an older generation are advisory only and SHOULD be re-obtained;
- a live source affected by the underlying change receives a typed `SOURCE_LOST` or device event through its own source-scoped path;
- already accepted feature syntax, existing sources, existing nodes, and outstanding credits are unaffected.

Context scope and quota changes use the context lifecycle in Section 7.18, never feature removal.

`CAPS_CHANGED` payload:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | New capability generation |
| 1 | uint | Change reason bit mask |

Reason bits: `0` decoder availability, `1` device availability, `2` presenter policy, `3` resource pressure.

## 6. Record-type registry

Numeric assignments are normative. Vivid 1.1 preserves every Vivid 1.0 assignment and meaning.

| Value | Name | Value | Name |
|---|---|---|---|
| `0x0001` | `HELLO` | `0x0002` | `WELCOME` |
| `0x0003` | `OK` | `0x0004` | `ERROR` |
| `0x0005` | `PING` | `0x0006` | `PONG` |
| `0x0007` | `GOODBYE` | `0x0008` | `DISPLAY_CHANGED` |
| `0x0009` | `CAPS_CHANGED` | `0x000a` | `SET_OBSERVATION` |
| `0x000b` | `QUERY_LIMITS` | `0x000c` | `LIMITS_STATUS` |
| `0x0100` | `PROBE_VIDEO_CONFIG` | `0x0101` | `VIDEO_SUPPORT` |
| `0x0102` | `CREATE_IMAGE` | `0x0103` | `CREATE_VIDEO` |
| `0x0104` | `CREATE_RASTER` | `0x0105` | `SOURCE_READY` |
| `0x0106` | `RECONFIGURE_SOURCE` | `0x0107` | `DESTROY_SOURCE` |
| `0x0108` | `SOURCE_LOST` | `0x0109` | `PROBE_AUDIO_CONFIG` |
| `0x010a` | `AUDIO_SUPPORT` | `0x010b` | `CREATE_AUDIO` |
| `0x010c` | `QUERY_SOURCE` | `0x010d` | `SOURCE_STATUS` |
| `0x010e` | `SOURCE_CHANGED` | `0x010f` | `WAIT_SOURCE` |
| `0x0110` | `WAIT_SATISFIED` | `0x0111` | `CANCEL_WAIT` |
| `0x0112` | `SET_SOURCE_POLICY` | `0x0113` | `UPDATE_SOURCE_DESCRIPTOR` |
| `0x0200` | `BEGIN_TXN` | `0x0201` | `CREATE_NODE` |
| `0x0202` | `UPDATE_NODE` | `0x0203` | `DELETE_NODE` |
| `0x0204` | `COMMIT_TXN` | `0x0205` | `ABORT_TXN` |
| `0x0206` | `PRESENTED` | `0x0207` | `ANCHOR_READY` |
| `0x0208` | `ANCHOR_GONE` | `0x0209` | `BARRIER_REACHED` |
| `0x020a` | `QUERY_SCENE` | `0x020b` | `SCENE_STATUS` |
| `0x020c` | `SCENE_CHANGED` | `0x020d` | `QUERY_ANCHOR` |
| `0x020e` | `ANCHOR_STATUS` |  |  |
| `0x0300` | `PLAY` | `0x0301` | `PAUSE` |
| `0x0302` | `STEP` | `0x0303` | `FLUSH` |
| `0x0304` | `DRAIN` | `0x0305` | `EOS` |
| `0x0306` | `PLAYBACK_STATE` | `0x0400` | `CREDIT` |
| `0x0401` | `FEEDBACK` | `0x0402` | `VISIBILITY` |
| `0x0403` | `QUALITY_HINT` | `0x0404` | `NEED_KEYFRAME` |
| `0x0405` | `NEED_FULL_FRAME` |  |  |
| `0x0500` | `BLOB_OFFER` | `0x0501` | `BLOB_HAVE` |
| `0x0502` | `BLOB_NEED` | `0x0503` | `BLOB_COMPLETE` |
| `0x0504` | `CACHE_EVICTED` | `0x0600` | `CREATE_CONTEXT` |
| `0x0601` | `DELEGATE_CONTEXT` | `0x0602` | `REVOKE_CONTEXT` |
| `0x0603` | `CONTEXT_CHANGED` | `0x0604` | `CONTEXT_READY` |
| `0x0605` | `CONTEXT_CAPABILITY` |  |  |
| `0x7000` | `KEY_INPUT` | `0x7001` | `POINTER_MOTION` |
| `0x7002` | `POINTER_BUTTON` | `0x7003` | `POINTER_AXIS` |
| `0x7004` | `INPUT_RESET` | `0x8000` | `ATTACH_CHANNEL` |
| `0x8001` | `VIDEO_PACKET` | `0x8002` | `VIDEO_FRAGMENT` |
| `0x8003` | `RASTER_FRAME` | `0x8004` | `BLOB_CHUNK` |
| `0x8005` | `BUFFER_SUBMIT` | `0x8006` | `IMAGE_DATA` |
| `0x8007` | `AUDIO_PACKET` |  |  |

Vivid 1.1 defines complete schemas for:

```text
HELLO WELCOME OK ERROR PING PONG GOODBYE DISPLAY_CHANGED CAPS_CHANGED
SET_OBSERVATION QUERY_LIMITS LIMITS_STATUS
PROBE_VIDEO_CONFIG VIDEO_SUPPORT PROBE_AUDIO_CONFIG AUDIO_SUPPORT
CREATE_IMAGE CREATE_VIDEO CREATE_AUDIO CREATE_RASTER
SOURCE_READY DESTROY_SOURCE SOURCE_LOST
QUERY_SOURCE SOURCE_STATUS SOURCE_CHANGED
WAIT_SOURCE WAIT_SATISFIED CANCEL_WAIT
SET_SOURCE_POLICY UPDATE_SOURCE_DESCRIPTOR
BEGIN_TXN CREATE_NODE UPDATE_NODE DELETE_NODE COMMIT_TXN ABORT_TXN PRESENTED
ANCHOR_READY ANCHOR_GONE QUERY_SCENE SCENE_STATUS SCENE_CHANGED
QUERY_ANCHOR ANCHOR_STATUS
PLAY PAUSE FLUSH DRAIN EOS PLAYBACK_STATE
CREDIT VISIBILITY NEED_KEYFRAME NEED_FULL_FRAME
CREATE_CONTEXT DELEGATE_CONTEXT REVOKE_CONTEXT CONTEXT_CHANGED
CONTEXT_READY CONTEXT_CAPABILITY
KEY_INPUT POINTER_MOTION POINTER_BUTTON POINTER_AXIS INPUT_RESET
ATTACH_CHANNEL VIDEO_PACKET AUDIO_PACKET RASTER_FRAME IMAGE_DATA
```

The following remain assigned and undefined in Vivid 1.1 and require a future negotiated profile:
`RECONFIGURE_SOURCE`, `BARRIER_REACHED`, `STEP`, `FEEDBACK`, `QUALITY_HINT`, `BLOB_OFFER`,
`BLOB_HAVE`, `BLOB_NEED`, `BLOB_COMPLETE`, `CACHE_EVICTED`, `VIDEO_FRAGMENT`, `BLOB_CHUNK`,
`BUFFER_SUBMIT`.

Extension ranges remain:

```text
0x7005-0x7fff  standards-track negotiated extensions
0x9000-0xbfff  experimental negotiated extensions
0xc000-0xffff  vendor-specific extensions, disabled unless explicitly negotiated
```

## 7. Control payload schemas

Unless stated otherwise, payload maps use envelope key 3 and contain only the keys below plus ignorable future keys. Unless a message defines another success reply, a successful correlated request returns `OK`.

### 7.1 Session and display

| Message | Payload |
|---|---|
| `OK` | Empty map |
| `GOODBYE` | Empty map; presenter replies `OK` and closes the session |
| `DISPLAY_CHANGED` | `0` generation, `1` viewport width, `2` viewport height, `3` grid columns, `4` grid rows, `5` cell width, `6` cell height, `7` settled |
| `ERROR` | `0` error code, `1` failed request ID, `2` detail map, `4` fatal boolean, `5` UTF-8 diagnostic |

`ERROR` payload key 2 is the bounded structured detail map defined in Section 14.2. Key 3 remains reserved and MUST NOT be emitted. Diagnostic text MUST NOT exceed 4,096 UTF-8 bytes and MUST NOT be parsed as protocol state; a producer that needs machine-readable failure information uses key 2.

**Liveness and clock sampling.** Either endpoint MAY send `PING`; the peer MUST promptly answer with `PONG` using the same request ID. Any valid inbound control record proves liveness. A reference implementation sends one probe after 15 seconds without inbound control activity and treats three consecutive unanswered idle probes as `TIMEOUT`. It estimates round-trip time only from clean `PING`/`PONG` samples and uses an EWMA with weight 1/8 for the new sample.

An endpoint SHOULD also sample round-trip time with occasional `PING` probes while the connection is active, at a bounded rate no faster than one outstanding probe per second, so that RTT-derived values such as the `PLAY` minimum buffer reflect the live transport rather than a static guess. Sampling probes MUST NOT change liveness accounting, and the clean-sample rule above still applies.

When `CLOCK_SAMPLING_V1` is accepted, the probe pair MAY carry timestamps:

| Message | Key | Type | Meaning |
|---|---:|---|---|
| `PING` | 0 | uint, optional | Sender transmit time, local monotonic microseconds |
| `PONG` | 0 | uint, conditional | Echoed sender transmit time; REQUIRED when the `PING` carried key 0 |
| `PONG` | 1 | uint, conditional | Responder receive time, local monotonic microseconds |
| `PONG` | 2 | uint, conditional | Responder transmit time, local monotonic microseconds |

Without the feature, both payloads are empty maps. A `PONG` MUST NOT carry keys 0 through 2 unless the corresponding `PING` carried key 0.

The sender combines its own receive time with keys 0 through 2 for the standard four-timestamp offset and delay calculation. Timestamps are process-local monotonic values in unrelated clock domains; they are not wall-clock times and MUST NOT be interpreted as such. A sample MUST be discarded when the round trip is not clean, when a local clock moves backward, or when the responder's processing delay exceeds an implementation bound.

Clock estimates are diagnostic only. They MUST NOT influence `PLAY`, credit accounting, drop decisions, epoch handling, or buffer sizing beyond the existing RTT rule in Section 7.8.

**Display changes.** `DISPLAY_CHANGED` is unsolicited and uses request ID zero. Display resize invalidates geometry assumptions, not source pixel dimensions. A producer normally updates node placement; it recreates a source only if it independently chooses a different source resolution.

A presenter MUST NOT emit more than one `DISPLAY_CHANGED` per source of truth per compositor frame, and SHOULD coalesce intermediate geometry during a continuous resize gesture into the smallest number of events that preserves the final state.

Payload key 7 is `settled`: `false` while a resize or reconfiguration gesture is still in progress, `true` for geometry the presenter considers final. A `DISPLAY_CHANGED` without key 7 is treated as `settled = true`, which preserves Vivid 1.0 producer behavior.

A producer that recreates dimension-fixed resources on a geometry change SHOULD act only on a settled event and SHOULD apply unsettled events to node placement alone. A producer MUST NOT assume that a settled event will arrive without a preceding unsettled event, or that any particular number of unsettled events precedes it.

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
| 23 | uint | OPTIONAL initial capture policy bit mask; `SOURCE_CAPTURE_POLICY_V1` |
| 24 | map | OPTIONAL source descriptor; `SOURCE_DESCRIPTOR_V1` |

Keys 14 through 20 are REQUIRED for `video-access-unit-v1`.

Keys 21 and 22 belong to `decoder-description-v1` (feature 16). A producer MUST NOT send them
unless the presenter accepted feature 16. The codec-string family MUST match key 1
(`avc1`/`avc3` for `h264`, `hvc1`/`hev1` for `hevc`, `vp09` for `vp9`, `av01` for `av1`), and the
decoder configuration MUST be the matching box body (avcC, hvcC, vpcC, or av1C) consistent with
the extradata and the stream. A presenter MAY ignore both keys; when it uses them it MUST
validate family and length first and MUST treat a mismatch as `BAD_MESSAGE`. The keys are
descriptive only: they change no packetization, and extradata (key 3) remains authoritative for
portable-profile initialization.

Keys 23 and 24 are defined in Sections 7.13 and 7.14. They MUST NOT appear in a `PROBE_VIDEO_CONFIG` record; a probe allocates no source and has no policy or descriptor. This schema is strict: an unknown key is `BAD_MESSAGE`.

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
| 2 | uint | Capability generation under which the answer was computed |

Key 2 is REQUIRED in Vivid 1.1. A producer SHOULD discard cached probe results when the capability generation changes (Section 5.5). Probes remain the authoritative answer for an exact configuration; Vivid 1.1 defines no coarse capability catalog.

Probes MAY be pipelined and answered independently, and MUST NOT block the control reader.

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
| 12 | uint | OPTIONAL initial capture policy bit mask; `SOURCE_CAPTURE_POLICY_V1` |
| 13 | map | OPTIONAL source descriptor; `SOURCE_DESCRIPTOR_V1` |

Key 11 belongs to `decoder-description-v1` (feature 16). A producer MUST NOT send it unless the
presenter accepted feature 16. The codec-string family MUST match key 2 (`mp4a.40.*` for `aac`,
`mp3` or `mp4a.6B` for `mp3`, the codec name itself for `opus`, `vorbis`, `flac`, and `alac`,
`ulaw` for `pcm_mulaw`, `alaw` for `pcm_alaw`, and a `pcm-*` string for other PCM codecs). A presenter MAY ignore the key; when it uses it, it MUST
validate family and length first. Extradata (key 4) remains authoritative for initialization.

Keys 12 and 13 MUST NOT appear in a `PROBE_AUDIO_CONFIG` record. This schema is strict.

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

`PROBE_AUDIO_CONFIG` returns `AUDIO_SUPPORT` with payload key 0 boolean exact-configuration support,
key 1 the codec/decoder name for operator diagnostics, and key 2 the capability generation.
Unsupported codecs, layouts, limits, or unavailable decoders return support false with the codec
name; they do not cause a protocol downgrade.

### 7.4 Raster configuration

`CREATE_RASTER` payload:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Nonzero source ID |
| 1 | uint | Width, 1 through 8,192 |
| 2 | uint | Height, 1 through 8,192 |
| 3 | uint | Pixel format; MUST be `RGBA8` (`1`) |
| 4 | uint | Alpha mode: straight (`1`) or premultiplied (`2`) |
| 5 | uint | Update mode: full-frame (`0`) or full-frame-and-delta (`1`) |
| 6 | uint | Rectangle limit |
| 7 | uint | Compression mode: raw only (`0`) or raw-or-zstd (`1`) |
| 8 | uint | Retention; MUST be none (`0`) |
| 9 | uint | OPTIONAL initial capture policy bit mask; `SOURCE_CAPTURE_POLICY_V1` |
| 10 | map | OPTIONAL source descriptor; `SOURCE_DESCRIPTOR_V1` |

Alpha mode 2 requires `RASTER_PREMULTIPLIED_ALPHA`. Compression mode 1 requires `RASTER_ZSTD_V1`.

Update mode 1 requires `RASTER_DELTA_V1`. With update mode 0, the rectangle limit MUST be `1` and the source accepts only full frames, exactly as in Vivid 1.0. With update mode 1, the rectangle limit is the maximum number of delta operations the producer will send in one frame and MUST be between `1` and `16`; the presenter MAY reduce it and reports the effective value in `SOURCE_READY`. A delta-capable source still accepts full frames at any time.

This schema is strict.

The presenter computes, with checked arithmetic:

```text
raw_frame_body = 72 + width * height * 4
```

It MUST reject source creation with `LIMIT_EXCEEDED` unless `raw_frame_body` fits the presenter's accepted media-body ceiling and resource budget. The axis limit alone does not make a source admissible. Delta admissibility is never used in place of this calculation: a delta-capable source MUST still be admissible as a full-frame source, because the presenter may demand a full frame at any time.

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
| 8 | bool | OPTIONAL cache-lookup request; `IMAGE_CACHE_V1` |
| 9 | uint | OPTIONAL initial capture policy bit mask; `SOURCE_CAPTURE_POLICY_V1` |
| 10 | map | OPTIONAL source descriptor; `SOURCE_DESCRIPTOR_V1` |

Key 8 requires `IMAGE_CACHE_V1` and requires key 5 to be present; a cache lookup without an exact content hash is `BAD_MESSAGE`. Section 12.2 defines the lookup.

The decoded pixel count and encoded byte length MUST fit presenter quotas before a ticket is issued. This schema is strict.

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
| 6 | uint | Steady-state rolling byte window |
| 7 | uint | Steady-state rolling packet window |
| 8 | uint | Initial `source_revision` |
| 9 | bool, optional | Media connection required; default `true` |
| 10 | uint, optional | Effective delta operation limit; raster sources with update mode 1 |

The `SOURCE_READY` object ID MUST equal the source ID.

Keys 6, 7, and 8 are REQUIRED in Vivid 1.1. Keys 1 through 5 are REQUIRED unless key 9 is present and `false`.

For every accepted source, key 2 MUST be at least the maximum body of one legal media record for that source, and key 3 MUST be at least one:

- raster: at least `raw_frame_body`;
- image: at least the declared encoded length;
- video: at least `48 + maximum encoded access-unit bytes`;
- audio: at least `48 + maximum encoded access-unit bytes`.

A presenter that cannot make that grant MUST reject source creation rather than create a source that cannot make progress.

Keys 6 and 7 advertise the steady-state rolling window the presenter intends to maintain for this source once it is streaming, in bytes and in records. They are a sizing hint for the producer's own queues, not a grant: a producer MUST still send only within credit actually received. Each MUST be at least the corresponding initial grant. Section 9 defines how the window is maintained.

Key 9 is `false` only for an `IMAGE_CACHE_V1` cache hit (Section 12.2). When it is `false`, no ticket is issued, the producer MUST NOT open a media connection for the source, and the source is immediately renderable.

`DESTROY_SOURCE` payload key 0 is the source ID. Success returns `OK`, closes its media channel, releases its resources, and removes placements using the source at the next compositor boundary.

`SOURCE_LOST` is unsolicited, uses request ID zero, and has:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Source ID |
| 1 | uint | Error code |
| 2 | text | Diagnostic, at most 4,096 UTF-8 bytes |
| 3 | uint | Final `source_revision` |
| 4 | map, optional | Structured detail map (Section 14.2) |

Its object ID MUST equal the source ID. A lost source accepts no further media. Its placements are removed at the next compositor boundary. The producer may create a replacement with a new source ID.

Source IDs are unique within a session and MUST NOT be reused while the source, any loss or destroy reply, or any queryable tombstone (Section 7.16) can still be observed.

**Source replacement.** A producer that needs different coded dimensions, a different codec, a different packetization, or a different colorimetry creates a new source rather than reconfiguring an existing one. The supported sequence is: create the replacement source, attach it and submit its first key or full frame, wait for the readiness milestone where required, atomically retarget existing nodes to the replacement in one scene transaction, then destroy the old source. `RECONFIGURE_SOURCE` remains assigned and undefined.

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

When `DELEGATED_CONTEXT_V1` is accepted, key 2 MAY name any context in the authenticated principal's subtree. A context ID outside that subtree is `NOT_FOUND`, never a disclosure of another principal's namespace.

Node operations require the transaction ID at envelope key 1. The record object ID MUST equal the node ID. `UPDATE_NODE` is a complete replacement. `DELETE_NODE` payload key 0 contains the node ID.

`COMMIT_TXN` contains transaction ID at envelope key 1, expected display generation at envelope key 2, an optional expected `scene_revision` at envelope key 4 key 0, and:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Presentation mode; next compositor frame (`0`) |
| 1 | bool | Acknowledgement requested; MUST be true in the Vivid 1.1 baseline |

All mutations apply atomically or not at all.

On success, when the new scene state becomes active at a compositor boundary, the presenter sends `PRESENTED` with the commit request ID and:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | `scene_revision` that became active |

`PRESENTED` means that the committed scene became active at the presenter's compositor boundary. For a browser presenter this means the compositor activation callback ran. `PRESENTED` does **not** assert that any source frame was decoded, displayed, or visible; Section 7.17 provides those milestones.

A stale display generation returns `STALE_DISPLAY_GENERATION` without applying mutations. A failed `scene_revision` precondition returns `PRECONDITION_FAILED` without applying mutations.

An automatic node-set change caused by source loss, anchor loss, context revocation, or policy teardown advances `scene_revision` and, when `OBSERVABILITY_CORE_V1` is accepted, emits `SCENE_CHANGED`. It MUST NOT fabricate a `PRESENTED` reply.

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

**Admission.** `PLAY` returns `OK` when the request has been validated and admitted, not when the clock starts. Waiting for pre-roll before replying would make `PLAY` an unbounded control operation and violate Section 4.3. A producer that needs the exact moment playback begins observes the `buffering` to `playing` transition through `PLAYBACK_STATE`, or registers `WAIT_SOURCE` with the playback-started condition (Section 7.17). When `OBSERVABILITY_CORE_V1` is not accepted, `OK` still means admitted, and no start milestone is available.

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

`FLUSH` is deliberately not subject to the media-order barrier below. Its purpose is to discard queued state; waiting to consume that state would make every seek slower. A producer waits for the `FLUSH` reply before writing the new epoch.

`EOS`:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Source ID |
| 1 | uint | Epoch |
| 2 | uint, conditional | Attachment generation; `MEDIA_ORDER_BARRIER_V1` |
| 3 | uint, conditional | Final media record sequence; `MEDIA_ORDER_BARRIER_V1` |

The object ID MUST match key 0. The epoch MUST NOT be older than the last accepted epoch. For video, queued decoder output may finish. For raster and image, the latest poster remains until source/node lifecycle removes it. Success returns `OK`.

Keys 2 and 3 require `MEDIA_ORDER_BARRIER_V1` and MUST either both be present or both be absent. When present they form the **media-order barrier**: the presenter admits the request immediately but applies EOS only after the record with exactly that sequence, on the media connection with exactly that attachment generation, has been validated and accepted into bounded source ownership.

Barrier rules:

- key 3 names a record sequence on the source's media connection, not a packet ID or frame ID; the record sequence is exact and gap-checked by framing (Section 3.2);
- key 2 MUST equal the current attachment generation of that source; a mismatch is `BAD_STATE` and applies no EOS;
- if the media connection fails before the named record is accepted, the source is lost or detached by the ordinary media-failure path and no EOS is applied;
- if the named record never arrives, the request completes with `TIMEOUT` after a bounded presenter-chosen interval and applies no EOS;
- a barrier is never forwarded verbatim across a hop; a bridge computes its own generation and sequence for its own outgoing connection.

EOS closes ingress. It is not an implicit `PAUSE`, and already-buffered media continues to play.

`DRAIN` requires `AUDIO_ACCESS_UNIT_V1` and has payload key 0 audio source ID. It returns `OK`
only after EOS has been observed, the decoder and resampler have flushed, and all queued device
samples have been consumed. Device loss returns `DEVICE_LOST`. `DRAIN` is a long operation and MUST be handled as bounded pending state under Section 4.3.

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

`NEED_FULL_FRAME` requires `RASTER_DELTA_V1`, is unsolicited, uses request ID zero, and identifies the source in both the record object ID and payload key 0:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Source ID |
| 1 | uint | Reason |

Reason values are:

| Value | Meaning |
|---:|---|
| 1 | Base frame unavailable |
| 2 | Accumulated damage budget exceeded |
| 3 | Renderer or resource reset |
| 4 | Policy or retention change |

After emitting `NEED_FULL_FRAME` the presenter MUST reject delta frames for that source with `BAD_STATE` and MAY resume ordinary latest-frame coalescing behavior until a full frame is accepted. The producer responds by sending a full frame. This recovery is source-scoped and does not lose the source.

### 7.9 Credits

`CREDIT` is unsolicited, uses request ID zero, and identifies the source in the record object ID:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Byte-credit increment |
| 1 | uint | Packet-credit increment |
| 2 | uint, optional | Fragment-credit increment; default zero |

Credit addition uses saturating unsigned arithmetic. See Section 9 for exact semantics.

Vivid 1.1 defines no batched credit record. A presenter MUST NOT delay a ready credit in order to combine it with another source's credit.

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

`VISIBILITY` is an actionable event and is never coalesced away or dropped by the observation mechanism in Section 7.15.

### 7.11 Anchor events

`ANCHOR_READY` and `ANCHOR_GONE` are unsolicited. Payload key 0 and the record object ID both identify the anchor. Anchor IDs are scoped to the authenticated session. Both are actionable events and are outside the Section 7.15 coalescing domain.

### 7.12 Desktop input

The messages in this section require `DESKTOP_INPUT_V1`. They are unsolicited, use request ID
zero, have no transaction or display generation, and receive no reply. A presenter MUST send
`INPUT_RESET` when its input focus is lost. A producer MUST release all held keys and buttons on
`INPUT_RESET`, control-session loss, delegated-capability revocation or expiry, or shutdown.

`KEY_INPUT` is session-level and uses object ID zero:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | USB HID keyboard-page usage, `0x04` through `0xe7` |
| 1 | bool | `true` for pressed, `false` for released |

The presenter sends physical transitions only and suppresses browser-generated key-repeat
transitions. The receiving desktop is responsible for key repeat and applies its configured
keyboard layout.

`POINTER_MOTION` identifies the target video source in both the record object ID and payload key
zero:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Nonzero target video source ID |
| 1 | uint | Absolute source-pixel x coordinate |
| 2 | uint | Absolute source-pixel y coordinate |

Coordinates MUST be within the target source dimensions. A presenter SHOULD coalesce motion to at
most one update per compositor frame.

`POINTER_BUTTON` identifies the target video source in both the record object ID and payload key
zero:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Nonzero target video source ID |
| 1 | uint | Button: `0` primary, `1` auxiliary, `2` secondary, `3` back, `4` forward |
| 2 | bool | `true` for pressed, `false` for released |

`POINTER_AXIS` identifies the target video source in both the record object ID and payload key zero:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Nonzero target video source ID |
| 1 | int | Horizontal wheel delta in 1/120 detent units, from `-12000` through `12000` |
| 2 | int | Vertical wheel delta in 1/120 detent units, from `-12000` through `12000` |

Positive horizontal values scroll left and positive vertical values scroll up. Zero on either
axis means no motion on that axis.

`INPUT_RESET` is session-level, uses object ID zero, and has an empty payload.

### 7.13 Source descriptor

The descriptor requires `SOURCE_DESCRIPTOR_V1`. It is a small, bounded, application-neutral
description of what a source *is*, so that an agent observing a presenter can identify a source and
detect when its content changed without the presenter carrying any semantic payload.

The descriptor map appears at source creation (Sections 7.2 through 7.5), in `SOURCE_STATUS`
(Section 7.16), and in `UPDATE_SOURCE_DESCRIPTOR`:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Role |
| 1 | text | Title, at most 256 UTF-8 bytes |
| 2 | uint | Semantic content revision |
| 3 | uint | Semantic availability bit mask |
| 4 | text | Opaque locator hint, at most 512 UTF-8 bytes |

Role values:

| Value | Meaning |
|---:|---|
| 0 | Unspecified |
| 1 | Document |
| 2 | Desktop |
| 3 | Timed media |
| 4 | Figure or still image |
| 5 | Terminal or text surface |

Semantic availability bits: `0` extracted text, `1` structure or accessibility tree, `2` links,
`3` outline or table of contents, `4` invocable actions.

Key 2 is a producer-owned counter that advances when the source's semantic content changes. It is
independent of `source_revision`, frame IDs, and packet IDs, and it MUST NOT be derived from them.
A producer MUST advance it monotonically and MUST NOT reuse a value within a session.

Key 4 is an **inert** identifier for the producer-local surface that serves the source's semantics.
A presenter MUST treat it as opaque bytes. A presenter MUST NOT open it as a path, resolve it as a
name, connect to it, fetch it, parse it as structured data, or use it to make any protocol decision.
This restates the Section 15 requirement that a presenter never opens a producer-supplied pathname
or fetches a producer-supplied URL. A presenter MAY forward the exact bytes to its own owner-only
automation surface, which is responsible for deciding whether a caller may act on them.

`UPDATE_SOURCE_DESCRIPTOR` payload:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Source ID |
| 1 | map | Complete replacement descriptor |

The object ID MUST equal key 0. The update is a complete replacement. Success returns `OK` and
advances `source_revision`. The request MAY carry an expected content revision in envelope key 4
key 5 when `ATOMIC_CONTROL_V1` is accepted.

The descriptor is producer-asserted, untrusted data. A presenter MUST NOT render the title into the
terminal text plane, MUST bound every field before storage, and MUST treat title text as content
rather than as instruction. A source whose capture policy denies semantic export (Section 7.14)
reports only key 0 in `SOURCE_STATUS`.

Vivid 1.1 carries no document text, accessibility tree, region list, or semantic action on the wire.
Those remain producer-local; the descriptor exists so an agent can find them and know when to
re-read them.

### 7.14 Source capture and export policy

Capture policy requires `SOURCE_CAPTURE_POLICY_V1`. It is a bit mask supplied at source creation and
adjustable afterward:

| Bit | Meaning when set |
|---:|---|
| 0 | Deny ecosystem screenshot, canvas capture, and readback of this source |
| 1 | Deny semantic and title export, including the Section 7.13 descriptor beyond its role |
| 2 | Deny post-disconnect poster retention |
| 3 | Deny content-addressed caching (Section 12.2) |
| 4 | Reduce diagnostic detail for this source to kind and state |

`SET_SOURCE_POLICY` payload:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Source ID |
| 1 | uint | Requested policy bit mask |

The object ID MUST equal key 0. Success returns `OK`, advances `source_revision`, and reports the
effective mask in the next `SOURCE_STATUS`.

Rules:

- the effective policy is the union of every policy asserted for the source along its path;
- a presenter or intermediary MAY make policy stricter and MUST NOT make it less strict on its own;
- a request that clears a bit is explicit and MAY be denied with `BAD_STATE`;
- setting bit 2 or bit 3 MUST purge any retained poster or cache entry that the new policy
  disallows before the reply is sent;
- a nested presenter applies the strictest policy across the virtual source, its pane, and the outer
  presenter, and forwards the strictest value onward.

This is an ecosystem automation contract, not a content-protection mechanism. It constrains
cooperating Vivid implementations and their automation surfaces. It cannot and does not prevent
operating-system screen capture, external recording, or photography, and an implementation MUST NOT
describe it as doing so.

Terminal echo suppression and secure-input state are separate presenter-local signals: sensitive
terminal glyphs can exist with no Vivid source at all.

### 7.15 Observation configuration and change events

The records in this section require `OBSERVABILITY_CORE_V1`.

`SET_OBSERVATION` is session-level, uses object ID zero, and configures the one control session:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Event class bit mask |

Class bits: `0` source transitions (`SOURCE_CHANGED`), `1` scene changes (`SCENE_CHANGED`),
`2` playback transitions (`PLAYBACK_STATE`).

Success returns `OK`. The configuration replaces any previous configuration. A mask of zero disables
observation events. There are no subscription identifiers, no per-source filters, and no replay
queues: a producer owns all of its own sources, and recovery from lost detail is a query.

Observation events are unsolicited, use request ID zero, and are **non-actionable**: a presenter MAY
coalesce them latest-wins and MAY discard detail under writer-queue pressure. They MUST NOT be used
to convey anything a producer needs in order to make progress.

The following remain actionable, always-on, and outside this coalescing and loss domain:
`SOURCE_LOST`, `NEED_KEYFRAME`, `NEED_FULL_FRAME`, `CREDIT`, `VISIBILITY`, `ANCHOR_READY`,
`ANCHOR_GONE`, `DISPLAY_CHANGED`, `CAPS_CHANGED`, `CONTEXT_CHANGED`, `PING`, `PONG`, and
`INPUT_RESET`.

`SOURCE_CHANGED` identifies the source in the record object ID:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Source ID |
| 1 | uint | Current `source_revision` |
| 2 | uint | Changed-field bit mask |
| 3 | uint | `observation_sequence` of this event |
| 4 | uint, optional | First `observation_sequence` lost before this event |

Changed-field bits: `0` lifecycle, `1` epoch, `2` playback, `3` attachment, `4` visibility,
`5` capture policy, `6` descriptor, `7` milestones, `8` credit accounting.

`SCENE_CHANGED` is session-level and uses object ID zero:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Current `scene_revision` |
| 1 | uint | Change reason bit mask |
| 2 | uint | `observation_sequence` of this event |
| 3 | uint, optional | First `observation_sequence` lost before this event |

Reason bits: `0` producer commit, `1` source loss removed placements, `2` anchor gone,
`3` context revoked, `4` policy or resource teardown.

`PLAYBACK_STATE` identifies the source in the record object ID:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Source ID |
| 1 | uint | Playback state |
| 2 | int | Current clock PTS in microseconds |
| 3 | uint | Epoch |
| 4 | uint | Buffered-ahead duration in microseconds |
| 5 | uint | Underrun count |
| 6 | uint | Late-drop count |
| 7 | uint | EOS state: not received (`0`), accepted (`1`), applied (`2`) |
| 8 | uint | Current `source_revision` |
| 9 | uint | `observation_sequence` of this event |

Playback states: `0` idle, `1` buffering, `2` playing, `3` paused, `4` ended, `5` lost.

`PLAYBACK_STATE` is emitted on state transitions only. A presenter MUST NOT emit it on a clock tick,
per decoded frame, or per presented frame. Vivid 1.1 defines no periodic playback, presentation, or
statistics stream on the control connection.

**Gap semantics.** When a bounded writer queue causes observation detail to be discarded, the next
observation event of that class carries key 4 (or key 3 for `SCENE_CHANGED`) with the first
`observation_sequence` that was lost. The producer recovers current truth with a query from
Section 7.16. A reliable, unbroken control connection therefore has no replay obligation, while a
slow observer can never block control, credits, or media.

### 7.16 Status queries

The records in this section require `OBSERVABILITY_CORE_V1`. Every reply in this section is bounded
to 65,536 bytes regardless of the negotiated control ceiling (Section 3.3), and every query MUST be
answered from bounded state without blocking the control reader (Section 4.3).

No reply in this section contains media payload bytes, a capability token, a delegated capability, a
media ticket, an anchor authenticator or derived key, a file path, a command line, an endpoint
value, or an environment value.

**`QUERY_SOURCE`** — payload key 0 is the source ID; the object ID MUST match. The reply is
`SOURCE_STATUS`:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Source ID |
| 1 | uint | Current `source_revision` |
| 2 | uint | Kind: video (`1`), raster (`2`), image (`3`), audio (`4`) |
| 3 | uint | Lifecycle state |
| 4 | uint | Current epoch |
| 5 | uint | Attachment state |
| 6 | uint | Current attachment generation |
| 7 | uint | Last accepted media packet or frame ID |
| 8 | uint | Last accepted media record sequence |
| 9 | int | Last decoded PTS in microseconds |
| 10 | int | Last presented PTS in microseconds |
| 11 | uint | Last presentation ID |
| 12 | bool | Current visibility |
| 13 | uint | Effective capture policy bit mask |
| 14 | uint | Linked source ID, or zero |
| 15 | uint | Milestone bit mask |
| 16 | uint | Outstanding byte credit held by the producer |
| 17 | uint | Outstanding packet credit held by the producer |
| 18 | uint | Ingress queue depth bucket |
| 19 | map, optional | Source descriptor (Section 7.13) |
| 20 | map, optional | Playback state, keys 1 through 7 of `PLAYBACK_STATE` |
| 21 | uint, optional | Terminal loss error code; tombstones only |

Lifecycle states: `0` created, `1` attached, `2` active, `3` paused, `4` ended, `5` lost,
`6` tombstone.

Attachment states: `0` never attached, `1` attached, `2` closed.

Milestone bits, matching Section 7.17 conditions: `0` media channel attached, `1` first media record
accepted, `2` decoder initialized, `3` random-access video unit accepted, `4` first decoded output,
`5` first presentation while at least one eligible node was visible, `6` playback clock started
after pre-roll, `7` EOS accepted, `8` playback ended after buffered output was consumed, `9` source
lost. A milestone bit, once set, is never cleared.

Key 18 reports a coarse bucket (`0` empty, `1` low, `2` moderate, `3` high, `4` at capacity) rather
than an exact depth, unless the presenter is operating in a conformance mode that requires exact
values.

**Attachment resolution.** Keys 5 and 6 are the authoritative answer to "was my ticket consumed?"
A producer whose media connection failed during or immediately after `ATTACH_CHANNEL`, and which
therefore cannot retry the ticket (Section 2.1), resolves the outcome by querying its source: an
attachment state of `1` or `2` proves the ticket was consumed, and `0` proves it was not. This
resolution requires no new record, no ticket reuse, and no source recreation.

**Tombstones.** A terminally lost source retains a metadata-only tombstone so that a late query can
still explain what happened. A tombstone holds no media, decoder, device, node, ticket, or credit
resources. Tombstones are bounded by count and by age, and a source ID MUST NOT be reused while its
tombstone remains queryable. A tombstone reports lifecycle state `6` and the loss code in key 21.

**`QUERY_SCENE`** — session-level, object ID zero:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint, optional | Expected `scene_revision` |
| 1 | bytes, optional | Opaque continuation cursor, at most 64 bytes |
| 2 | uint, optional | Maximum node entries in the reply |

The reply is `SCENE_STATUS`:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | `scene_revision` the page was computed from |
| 1 | array(map) | Node entries, using the Section 7.7 node keys |
| 2 | bytes, optional | Continuation cursor; absent when the page is final |
| 3 | uint | Total node count at that revision |

A cursor is bound to exactly one `scene_revision`. If the scene changed since the cursor was issued,
the presenter returns `PRECONDITION_FAILED` with detail key 3 set to the current revision, and the
producer restarts the enumeration. If key 0 is supplied and does not match, the same error is
returned before any page is computed. A page reports only nodes the authenticated principal is
entitled to observe.

**`QUERY_ANCHOR`** — payload key 0 is the anchor ID; the object ID MUST match. The reply is
`ANCHOR_STATUS`:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Anchor ID |
| 1 | uint | State: ready (`1`), gone (`2`), unknown (`0`) |
| 2 | uint | Cell column |
| 3 | uint | Cell row |
| 4 | bool | Anchor cell intersects the current viewport |
| 5 | uint | Display generation used for the calculation |

Keys 2 through 4 are meaningful only when key 1 is `1`.

**`QUERY_LIMITS`** — session-level, object ID zero, empty payload. The reply is `LIMITS_STATUS` and
reports the values actually in force for this session and context, which may be lower than the
recommended reference limits in Section 15:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Maximum sources for this principal |
| 1 | uint | Maximum nodes for this principal |
| 2 | uint | Maximum open transactions |
| 3 | uint | Maximum active anchors |
| 4 | uint | Maximum control-record body accepted by the presenter |
| 5 | uint | Maximum media-record body accepted by the presenter |
| 6 | uint | Maximum concurrent registered waits |
| 7 | uint | Maximum pending correlated requests |
| 8 | uint | Default rolling media byte window |
| 9 | uint | Default rolling media packet window |
| 10 | uint | Retained decoded and poster pixel budget |
| 11 | uint | Current source count |
| 12 | uint | Current node count |
| 13 | uint | Current retained decoded and poster pixels |
| 14 | uint, optional | Encoded-image cache budget in bytes; `IMAGE_CACHE_V1` |

Vivid 1.1 defines no codec or profile catalog. `PROBE_VIDEO_CONFIG` and `PROBE_AUDIO_CONFIG` remain
the only authoritative answer for whether an exact configuration is supported, because profiles,
extradata, browser decoder support, device availability, and hardware acceleration cannot be
summarized safely by a table row.

### 7.17 Source waits

`WAIT_SOURCE` requires `OBSERVABILITY_CORE_V1`. It is a correlated request that completes when one
condition on one source becomes true, or when its timeout expires. The object ID MUST equal the
source ID.

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Source ID |
| 1 | uint | Condition kind |
| 2 | uint, conditional | Condition value |
| 3 | uint | Timeout in microseconds, nonzero and bounded by the presenter |

Condition kinds:

| Value | Meaning | Key 2 |
|---:|---|---|
| 1 | `source_revision` greater than a value | Required |
| 2 | First presentation while visible | Absent |
| 3 | Presented raster frame ID at or beyond a value | Required |
| 4 | Presented video PTS at or beyond a value | Required |
| 5 | Playback started | Absent |
| 6 | Playback ended after buffered output was consumed | Absent |
| 7 | Media channel attached | Absent |
| 8 | Media channel closed | Absent |
| 9 | Source lost | Absent |

A condition that is already true completes immediately. Success returns `WAIT_SATISFIED`:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Source ID |
| 1 | uint | `source_revision` when the condition was satisfied |
| 2 | uint | Condition kind |
| 3 | uint, optional | Observed value that satisfied the condition |

Failure returns `ERROR`:

| Code | Meaning |
|---|---|
| `TIMEOUT` | The timeout expired |
| `CANCELLED` | Cancelled by `CANCEL_WAIT`, session loss, or source destruction |
| `NOT_FOUND` | No such source |
| `NOT_VISIBLE` | The condition cannot be satisfied because the source has no eligible visible placement at this hop |
| `LIMIT_EXCEEDED` | Too many registered waits |

`NOT_VISIBLE` is the required answer when a presentation condition cannot be met because the
containing surface is not projected — for example a nested pane that is not currently composited
into the outer presenter. It is not a timeout and MUST NOT be reported as one.

A presenter MUST support at least 32 concurrently registered waits per session, MUST bound the
timeout it accepts, and MUST cancel every registered wait on control-session loss. A wait is pending
state, never a blocked reader or a blocked session actor.

`CANCEL_WAIT` payload key 0 is the request ID of the wait to cancel. Success returns `OK`; the
cancelled wait separately returns `CANCELLED`. Cancelling an unknown or already-completed wait
returns `OK`.

A presentation condition asserts what the presenter itself did. For a raster source, condition 3
names the producer's own frame ID. For a video source, condition 4 names a presentation PTS; an
originating packet ID is reported in key 3 only when the decoder can prove the mapping, because a
packet ID is not a frame ID and a decoder may emit zero, one, or several frames per input packet.

### 7.18 Contexts and delegated capabilities

The records in this section require `DELEGATED_CONTEXT_V1`.

A context is a named subtree of a session's resources with its own quotas, permitted operations,
lifetime, and cleanup. Vivid 1.0 exposed only the session root context; Vivid 1.1 allows a principal
to create child contexts and to mint an opaque capability that authenticates a *separate* session
confined to one of them.

**Authority model.** The inherited window token is the root authority. A scope requested in `HELLO`
is self-restraint, not least privilege: a holder of the root token can always reconnect and request
more. Real confinement therefore requires a distinct random capability that the presenter binds to a
context, a permitted-operation set, quotas, and an expiry.

`CREATE_CONTEXT` payload:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Nonzero context ID, unique within the session |
| 1 | uint | Parent context ID; MUST be the caller's root or an existing context in its subtree |
| 2 | uint | Permitted operation-class bit mask |
| 3 | text | Label for operator diagnostics, at most 64 UTF-8 bytes |
| 4 | uint | Expiry in microseconds from acceptance; zero means session lifetime |
| 5 | map | Requested quota map |

Operation-class bits:

| Bit | Meaning when set |
|---:|---|
| 0 | Observe objects owned by this context |
| 1 | Create sources and submit media |
| 2 | Create and mutate scene nodes owned by this context |
| 3 | Create anchors |
| 4 | Receive desktop input for owned sources |
| 5 | Administer child contexts |

Quota map keys: `0` maximum sources, `1` maximum nodes, `2` maximum retained decoded pixels,
`3` maximum aggregate media byte window, `4` maximum concurrent media connections.

The effective class mask and quotas are the intersection of the request with the parent's own
effective values. A request may only narrow. Success returns `CONTEXT_READY`:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Context ID |
| 1 | uint | Effective operation-class bit mask |
| 2 | map | Effective quota map |
| 3 | uint | Effective expiry in microseconds from acceptance; zero means session lifetime |

`DELEGATE_CONTEXT` payload key 0 is the context ID; the object ID MUST match. It requires the caller
to hold class bit 5 for that context's parent. Success returns `CONTEXT_CAPABILITY`:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Context ID |
| 1 | bytes(32) | Opaque delegated capability |

The capability is generated by a cryptographically secure random source and is unrelated to the
window token, the session tag, the context ID, and any content hash. The presenter stores only a
verifier permitting constant-time comparison, plus the binding to context, class mask, quotas, and
expiry.

`CONTEXT_CAPABILITY` is a **correlated reply and never an unsolicited event**, so a capability is
never left sitting in an observation or event queue. It is emitted exactly once per successful
`DELEGATE_CONTEXT` and is not re-derivable; a caller that loses it revokes the context and delegates
again. It MUST NOT appear in any query reply, status record, event, or trace.

Delivery to the intended holder is out of band: a protected file descriptor, owner-only IPC, or a
carefully scoped environment. Section 2.2 applies in full.

`REVOKE_CONTEXT` payload key 0 is the context ID; the object ID MUST match. Success returns `OK` and
synchronously:

1. invalidates every capability bound to that context and its descendants;
2. closes every session authenticated by those capabilities with `CONTEXT_REVOKED`;
3. destroys the sources, nodes, anchors, and unused tickets owned by that subtree;
4. releases all held desktop input for owned sources, as if `INPUT_RESET` had been delivered;
5. advances `scene_revision` and emits `SCENE_CHANGED` with reason bit 3 to remaining sessions.

`CONTEXT_CHANGED` is unsolicited, actionable, and identifies the context in the record object ID:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Context ID |
| 1 | uint | State: active (`1`), expired (`2`), revoked (`3`) |
| 2 | uint | Reason bit mask |

Reason bits: `0` explicit revocation, `1` expiry, `2` parent revoked, `3` quota reduction,
`4` presenter policy.

**Scope enforcement.** A session authenticated by a delegated capability observes and mutates only
objects within its bound context subtree. Every query, wait, and event is filtered to that subtree.
An object outside it is `NOT_FOUND`; a presenter MUST NOT reveal the existence, count, or identity
of another principal's objects, and MUST NOT allow one context to detect another's content through
cache-hit behavior (Section 12.2).

Window-wide observation across unrelated sessions is not a Vivid capability at any scope. It belongs
to the presenter's own owner-only automation surface, which already holds that authority.

**Nesting.** A nested presenter keeps its inner pane capabilities independent of any outer
credential. A foreground bridge MAY map a pane to an upstream context without transmitting the outer
root token or an outer delegated capability to the nested server or to the pane. Upstream authority
is the intersection of outer policy and pane policy, and pane teardown revokes the mapped upstream
context.

## 8. Media-channel binding

After `SOURCE_READY` with a media connection required, the producer opens a connection whose kind matches the source:

| Source | Connection kind |
|---|---|
| Video | `Video` (`1`) |
| Raster | `Raster` (`2`) |
| Encoded image | `Blob` (`3`) |
| Audio | `Audio` (`5`) |

The first record MUST be `ATTACH_CHANNEL`. Its body uses the deterministic CBOR envelope, request ID zero, and payload key 0 containing the 32-byte ticket. The record object ID is the source ID.

`ATTACH_CHANNEL` is not charged against media byte or packet credit.

A ticket is single-use and bound to session, source ID, and connection kind. In Vivid 1.1 a ticket does not expire by time; it remains valid until used, source destruction, context revocation, or session loss. Missing, reused, wrong-kind, or wrong-source tickets close the media connection.

No success acknowledgement is required. After writing a valid `ATTACH_CHANNEL`, the producer MAY immediately write the first media record on the same stream.

**Attachment generations.** Each source has an attachment generation, starting at zero when the source is created and incremented each time a media connection is successfully attached to it. The current value appears in `SOURCE_STATUS` key 6 and is the generation named by the EOS media-order barrier (Section 7.8).

A producer whose media connection fails during or immediately after `ATTACH_CHANNEL` MUST NOT retry the ticket, because the attachment may already have been consumed. When `OBSERVABILITY_CORE_V1` is accepted it resolves the outcome with `QUERY_SOURCE` (Section 7.16). Otherwise it destroys the source and creates a replacement.

Attachment generations are per hop. A nested presenter's inner generation and a bridge's outer generation are independent values in independent domains, and neither asserts anything about the other.

After attachment:

- a video channel accepts only `VIDEO_PACKET` for its source;
- a raster channel accepts only `RASTER_FRAME` for its source;
- an image channel accepts exactly one `IMAGE_DATA` for its source;
- an audio channel accepts only `AUDIO_PACKET` for its source.

Presenter-to-producer credits and events remain on the control connection.

## 9. Credit flow control

Credits represent bounded presenter capacity to accept additional media records. They are not decode acknowledgements, presentation acknowledgements, or transport writability.

Before sending a charged media record, the producer MUST hold:

- byte credit at least equal to the complete record-body length; and
- at least one packet credit.

It deducts those amounts before transmission. The 24-byte record header and `ATTACH_CHANNEL` are not charged.

A presenter returns byte and packet credits only when both of the following capacity is reusable:

1. the media body's ingress storage has been released or transferred into separately bounded storage; and
2. the corresponding bounded media-queue slot is available again.

For a decoder that retains the compressed packet buffer, credit is not returned until that ownership ends. For raster, credit may be returned after validated pixels have been copied or uploaded and the input body is released; it need not wait for presentation. For a raster delta (Section 11.4), credit is not returned until the delta has been completely composed into the retained framebuffer, because subsequent deltas depend on the resulting base.

A presenter MAY grant credit proactively, but MUST NOT grant capacity it cannot bound. It SHOULD maintain a rolling window using high and low watermarks rather than waiting for the producer to reach zero. Credit return MUST NOT be coalesced in a way that can strand the only packet or maximum-record-sized byte grant; any coalescing policy needs an independent timer and a low-watermark path.

The initial grant rules in Section 7.6 guarantee that every accepted source can send at least one maximum legal record. For remote high-throughput streams, a presenter SHOULD size the rolling window to at least one maximum record plus a reasonable estimate of the path bandwidth-delay product, subject to its memory budget.

`SOURCE_READY` keys 6 and 7 advertise that intended steady-state window. The advertisement is a sizing hint, not a grant: a producer sizes its own encoder queues, coalescing policy, and blocking behavior from it, but MUST send only within credit actually received. A presenter SHOULD converge toward its advertised window under sustained streaming and SHOULD report the current window and outstanding credit through `QUERY_LIMITS` and `SOURCE_STATUS`.

Credit-return latency is a first-order determinant of achievable throughput on high-RTT and browser paths. A presenter SHOULD instrument the interval between accepting a media record and returning its credit, and an implementation SHOULD treat regressions in that interval as performance defects rather than as invisible behavior. This is a diagnostic obligation; it adds no wire state.

A producer with insufficient credit waits, coalesces, drops an obsolete latest-frame raster update, or reports backpressure to its caller. It MUST NOT send past credit. Packet credit is a record-window limit: a sustainable remote window must cover the records expected during roughly one path RTT in addition to byte-credit sizing. Small PCM records SHOULD represent at least 20 ms of audio unless lower latency is explicitly required.

A credit violation closes the media connection and produces `SOURCE_LOST` with `FLOW_CONTROL`. It does not corrupt other sources or terminal text.

Backpressure is source-scoped. A slow, blocked, or malformed media source MUST NOT delay control service, credits for other sources, audio, desktop input, terminal rendering, or scene commits. An implementation MUST NOT introduce a shared media executor on which one source can block another.

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

This layout is byte-identical to Vivid 1.0 and is frozen by Section 1.5.

Packet flag bit 0 is `KEY`; bit 1 is `DELTA`. Exactly one is set. All other bits are zero.

Packet ID is nonzero and MUST be strictly greater than the previously accepted packet ID for the source, including across epoch changes. Exhaustion requires source replacement. Duration zero means unknown. The encoded access unit MUST be nonempty and MUST NOT exceed the source's declared maximum.

Side data is forbidden in `video-access-unit-v1`. Vivid 1.1 does not define typed side-data elements and does not add per-packet capture, encode, or trace timestamps. Diagnostic correlation uses packet IDs, record sequences, and out-of-band traces.

A packet ID is not a presented-frame identifier. A decoder may reorder, and may emit zero, one, or several frames for one input packet.

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

Packetization, codec, coded dimensions, and colorimetry remain fixed for the source lifetime. A producer requiring a different configuration creates a new source and follows the replacement sequence in Section 7.6.

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

This layout is byte-identical to Vivid 1.0 and is frozen by Section 1.5.

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

Vivid 1.1 defines no per-source gain, no variable playback rate, and no presenter-managed looping.
`STEP` remains assigned and undefined. A producer that needs replay seeks in its own media using
`FLUSH`, a new epoch, a key packet, and `PLAY`.

## 11. Raster profile

### 11.1 Full-frame body

A full `RASTER_FRAME` body has a 48-byte frame header, one 24-byte rectangle descriptor, and one pixel payload. This layout is byte-identical to Vivid 1.0 and is frozen by Section 1.5.

Frame header:

| Offset | Size | Field |
|---:|---:|---|
| 0 | 4 | Epoch |
| 4 | 4 | Flags |
| 8 | 8 | Frame ID |
| 16 | 8 | Base frame ID; MUST be zero for a full frame |
| 24 | 8 | PTS in microseconds, signed |
| 32 | 8 | Duration in microseconds |
| 40 | 4 | Rectangle or operation count; MUST be one for a full frame |
| 44 | 4 | Reserved; MUST be zero |

Frame flag bit 0 is `FULL`. Bit 1 is `ZSTD`. Bit 2 is `DELTA`. Bits 3 through 31 are zero.

Exactly one of `FULL` and `DELTA` MUST be set. A frame with `FULL` set uses this section. A frame with `DELTA` set uses Section 11.4 and requires `RASTER_DELTA_V1`.

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

Each compressed payload contains exactly one zstd frame, with no dictionary and no skippable frames. Decompression MUST produce exactly the expected byte count for its rectangle and consume the entire payload. Short output, excess output, trailing frames, dictionary references, or decoder errors lose the source with `BAD_MESSAGE` or `DECODER`.

The presenter MUST bound decompression output by the already validated raw pixel length and MUST NOT trust a size declared inside the compressed stream.

A producer SHOULD send raw pixels when zstd does not reduce the body size. Source admissibility and initial credit are based on the raw full-frame body, so compression is never required for liveness.

### 11.4 Damage and copy deltas

This section requires `RASTER_DELTA_V1` and a source created with update mode 1. Full frames remain legal for such a source at any time and retain the behavior in Sections 11.1 through 11.3 and 11.5 unchanged.

A delta frame sets `DELTA` and clears `FULL`. Its frame header is the Section 11.1 header with these differences:

- the base frame ID at offset 16 MUST be nonzero and MUST equal the frame ID of the immediately preceding **accepted** frame for the source;
- the count at offset 40 is the number of operations and MUST be between 1 and the effective limit reported in `SOURCE_READY` key 10.

The header is followed by exactly `count` 32-byte operation descriptors, then by the concatenated payloads of the overwrite operations in operation order:

| Offset | Size | Field |
|---:|---:|---|
| 0 | 4 | Operation kind: overwrite (`1`) or copy (`2`) |
| 4 | 4 | Destination X |
| 8 | 4 | Destination Y |
| 12 | 4 | Width |
| 16 | 4 | Height |
| 20 | 4 | Source X; MUST be zero for overwrite |
| 24 | 4 | Source Y; MUST be zero for overwrite |
| 28 | 4 | Payload length; MUST be zero for copy |

There is no per-operation data offset. Overwrite payloads are concatenated in operation order immediately after the operation array, and each is consumed using its own payload length. The body MUST end exactly after the final payload.

Validation, performed completely before any mutation:

- width and height MUST be nonzero;
- destination rectangle and, for a copy, source rectangle MUST lie entirely within the source dimensions, using checked arithmetic for every origin-plus-extent calculation;
- for an overwrite with `ZSTD` clear, payload length MUST equal `width * height * 4`;
- for an overwrite with `ZSTD` set, the payload MUST be exactly one zstd frame decompressing to exactly `width * height * 4` bytes under the Section 11.3 bounding rule;
- unknown operation kinds, nonzero reserved fields, and a body that does not end exactly at the final payload are `BAD_MESSAGE`.

Application semantics:

- operations are applied strictly in order to the presenter's retained composed framebuffer;
- a copy behaves as if the source region were first read into a temporary and then written to the destination, so overlapping copies are well defined and a scroll is exact;
- the accepted frame ID advances only after every operation has been applied;
- credit for the record is returned only after complete application (Section 9).

Presenter obligations:

- if the named base frame is not the current retained frame, the presenter MUST reject the delta with `BAD_STATE` and emit `NEED_FULL_FRAME` reason 1;
- a presenter MUST NOT discard an unapplied delta in the way it may discard an intermediate full frame, because subsequent deltas depend on the resulting base;
- a presenter MUST enforce a bounded accumulated-damage budget per source per interval. When a producer exceeds it, the presenter emits `NEED_FULL_FRAME` reason 2, rejects further deltas, and resumes ordinary latest-frame coalescing until a full frame is accepted;
- a presenter SHOULD upload only the union of the damaged regions to its renderer.

Producer obligations. A producer MUST send a full frame:

- as the first frame of a source and as the first frame after any `NEED_FULL_FRAME`;
- as the first frame of a new epoch;
- when the encoded delta would not be smaller than the full-frame representation;
- when accumulated damage since the last full frame exceeds an implementation-chosen fraction of the frame area.

A producer SHOULD express scrolling and panning as copy operations plus overwrite operations for the newly exposed region, rather than as a full frame.

Nesting. A nested presenter terminates delta chains at its own boundary: it validates and composes into its retained latest raster, then independently chooses a full or delta encoding for its outgoing hop. It MUST NOT forward a delta whose base frame identity belongs to another hop.

### 11.5 Timing and coalescing

Raster is immediate, latest-frame media. An accepted frame becomes eligible at the next compositor opportunity. PTS and duration are metadata only and do not schedule presentation in Vivid 1.1. `PLAY`, `PAUSE`, and `FLUSH` are invalid for raster sources.

If multiple raster **full** frames become ready before presentation, the presenter MAY discard intermediate frames and present only the newest valid frame. Delta frames are exempt from this coalescing until they have been composed, as required by Section 11.4. Credit return still follows storage and queue capacity, not whether the frame was presented.

## 12. Encoded still-image profile

### 12.1 Transfer

`ENCODED_IMAGE_V1` carries one encoded image over a blob-kind media connection.

After valid attachment, the producer sends exactly one `IMAGE_DATA` record whose object ID is the source ID. The body is exactly the encoded bytes declared by `CREATE_IMAGE`; it has no additional prefix.

The presenter MUST:

1. verify body length and optional SHA-256 before decode;
2. enforce the declared encoded-length and decoded-pixel quotas;
3. use an allowlisted PNG or JPEG decoder;
4. reject an encoded image whose decoded dimensions differ from the declaration;
5. reject animated or multi-picture content;
6. produce an sRGB source image.

PNG alpha is interpreted as straight alpha. JPEG is opaque. Orientation metadata is ignored.

The source becomes renderable after successful decode. It behaves as a retained still source and requires no `PLAY`. Decode failure emits `SOURCE_LOST` and removes placements using that source.

The presenter MUST NOT fetch external resources referenced by metadata and MUST bound decoder allocations independently of encoded byte length.

### 12.2 Context-local image reuse

This section requires `IMAGE_CACHE_V1`.

A presenter MAY maintain an immutable cache of decoded images keyed by the tuple:

```text
(delegated context, encoding, exact encoded SHA-256, exact encoded length,
 decoded width, decoded height, output color space, decode profile)
```

When `CREATE_IMAGE` sets key 8 and the presenter holds a matching entry, it MAY answer
`SOURCE_READY` with key 9 set to `false` and keys 1 through 5 absent. The source is immediately
renderable, no ticket is issued, and the producer MUST NOT open a media connection for it.

On a miss, the presenter answers an ordinary `SOURCE_READY` with a ticket, and the producer performs
the Section 12.1 transfer. After successful validation and decode the presenter MAY seed the cache
with that entry. A miss is the normal case and requires no event.

Rules:

- the cache key is scoped to a **delegated context**, not merely to a session, so one principal
  cannot detect another principal's content through hit behavior;
- an entry is seeded only from bytes this presenter validated itself;
- a source whose capture policy sets the no-cache bit (Section 7.14) MUST NOT be served from the
  cache and MUST NOT seed it;
- an active source holds a reference to its decoded resource independently of cache retention, so
  eviction never disturbs a live source;
- only unreferenced entries are eviction candidates, and eviction is silent: `CACHE_EVICTED` remains
  assigned and undefined;
- the cache is bounded by the budget reported in `LIMITS_STATUS` key 14;
- context revocation purges every entry scoped to that context and its descendants.

A cache hit asserts nothing beyond this presenter and this context. In a nested deployment each hop
maintains its own independent cache, and a hit at one hop implies nothing about another.

## 13. Text anchors

### 13.1 Marker version 2

Vivid 1.1 authenticated anchors use the same APC marker as Vivid 1.0:

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

The session tag is an identifier, not a secret. Terminal recordings may contain it; disclosure does not permit forging a new version-2 marker without the token-derived key. Because the session tag is not secret, it MUST NOT be used as authentication material, as a resumption proof, or as an input to any capability derivation.

A session authenticated by a delegated capability derives its anchor key from that capability using the same construction, substituting the 32 raw capability bytes for `token`. Anchors created under a delegated capability are destroyed when that context is revoked or expires.

### 13.3 Anchor IDs and replay

A producer MUST generate anchor IDs using a cryptographically secure random source. IDs are scoped to one control session and MUST never be reused during that session, even after an anchor is gone.

A presenter tracks seen anchor IDs for the session. A duplicate or replayed marker MUST NOT create, move, or recreate an anchor. It is ignored and MAY be logged subject to rate limiting.

A valid new marker creates the anchor at the current semantic text position and emits `ANCHOR_READY`.

### 13.4 Lifecycle

An anchor follows its semantic text position through scrolling and scrollback. When that position is erased or evicted, the presenter removes attached nodes and emits `ANCHOR_GONE`, advances `scene_revision`, and emits `SCENE_CHANGED` with reason bit 2 when observation is enabled.

Clearing the terminal text plane removes all anchors and anchored nodes.

After producer disconnect, an anchored latest frame MAY remain as a poster unless the source's capture policy denies post-disconnect retention (Section 7.14). The presenter MUST release decoder state, compressed packet queues, and other non-poster resources. Poster memory remains bounded by the aggregate resource budget and is reclaimed when the anchor is removed or under a documented resource-pressure policy.

Unanchored nodes are tied to control-session lifetime.

`BARRIER_REACHED` remains assigned and undefined.

## 14. Errors and failure isolation

### 14.1 Error codes

| Value | Name | Value | Name |
|---:|---|---:|---|
| 1 | `AUTH_FAILED` | 13 | `NEED_KEYFRAME` |
| 2 | `UNSUPPORTED_VERSION` | 14 | `STALE_EPOCH` |
| 3 | `UNSUPPORTED_FEATURE` | 15 | `STALE_DISPLAY_GENERATION` |
| 4 | `UNSUPPORTED_CONFIG` | 16 | `ANCHOR_INVALIDATED` |
| 5 | `BAD_MESSAGE` | 17 | `CONTEXT_REVOKED` |
| 6 | `BAD_STATE` | 18 | `DECODER` |
| 7 | `DUPLICATE_ID` | 19 | `DEVICE_LOST` |
| 8 | `NOT_FOUND` | 20 | `TIMEOUT` |
| 9 | `LIMIT_EXCEEDED` | 21 | `PRECONDITION_FAILED` |
| 10 | `NO_MEMORY` | 22 | `ALREADY_APPLIED` |
| 11 | `FLOW_CONTROL` | 23 | `NOT_VISIBLE` |
| 12 | `HASH_MISMATCH` | 24 | `CANCELLED` |

Codes 1 through 20 are unchanged from Vivid 1.0. `ANCHOR_GONE` remains a legacy symbolic alias for error code 16; new code uses `ANCHOR_INVALIDATED`. The event record remains named `ANCHOR_GONE`.

Error code 25 and above are unassigned.

### 14.2 Structured error detail

`ERROR` payload key 2 is a bounded map with numeric, registry-defined keys. It MUST NOT exceed 4,096 bytes encoded, and every value is bounded.

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Limit identifier (Section 14.3) |
| 1 | uint | Current value of that limit or counter |
| 2 | uint | Maximum permitted value |
| 3 | uint | Current `scene_revision` |
| 4 | uint | Current `source_revision` |
| 5 | uint | Current source epoch |
| 6 | uint | Failed precondition kind; the envelope key-4 map key that failed |
| 7 | bool | Retryable |
| 8 | uint | Suggested retry delay in microseconds |
| 9 | uint | Idempotent-outcome status: fresh (`0`), replayed (`1`), already applied (`2`) |
| 10 | uint | Offending payload key number |
| 11 | uint | Responder's supported Vivid major version |
| 12 | uint | Responder's supported Vivid minor version |

Every key is optional; a sender includes only what is meaningful for the failure. Keys 11 and 12 are used by Section 3.6.

The detail map MUST NOT contain free-form text, a file path, a command line, an environment value, a token, a capability, a ticket, an authenticator, a hash of producer content, or any media byte.

A producer MAY parse key 2. A producer MUST NOT parse the human diagnostic in key 5.

### 14.3 Limit identifiers

| Value | Limit |
|---:|---|
| 1 | Concurrent sessions |
| 2 | Concurrent connections |
| 3 | Sources |
| 4 | Nodes |
| 5 | Open transactions |
| 6 | Active anchors |
| 7 | Seen anchor IDs |
| 8 | Control-record body |
| 9 | Media-record body |
| 10 | Source width or height |
| 11 | Decoded or poster pixels |
| 12 | Media byte credit window |
| 13 | Media packet credit window |
| 14 | Pending correlated requests |
| 15 | Registered waits |
| 16 | Idempotency map entries |
| 17 | Contexts |
| 18 | Delta operations per frame |
| 19 | Accumulated raster damage budget |
| 20 | Encoded-image cache budget |
| 21 | Status reply size |
| 22 | Observation queue depth |

Limit identifier 23 and above are unassigned.

### 14.4 Failure isolation

Malformed prefaces, malformed record headers, reserved framing bits, invalid sequence numbers, and bodies over an effective ceiling close the affected connection. Section 3.6 defines the one permitted diagnostic record on a version-rejected connection.

A malformed control body produces `BAD_MESSAGE` when request correlation remains safe. A failed transaction never partially mutates the scene. A failed precondition never mutates anything.

Media framing, flow-control, decode, hash, decompression, delta-validation, and epoch failures are scoped to the affected source where possible and emit `SOURCE_LOST`, `NEED_KEYFRAME`, or `NEED_FULL_FRAME`. They MUST NOT corrupt terminal text parsing or unrelated sources.

Loss of an observation event, a status reply, or a wait MUST NOT affect media delivery, credits, or scene state.

## 15. Security and resource requirements

A conforming presenter MUST:

- authenticate `HELLO` and verify local peer identity before allocating producer-controlled source/scene resources;
- compare capability tokens, delegated capabilities, and anchor authenticators without data-dependent early exit;
- keep local and SSH-forwarded Unix sockets private to the owning user;
- reject ID collisions in session, source, node, transaction, anchor, and context namespaces;
- validate all lengths, integer widths, dimensions, fixed-point geometry, rectangles, tickets, epochs, hashes, decoder outputs, revisions, cursors, and credit before use;
- bound sessions, connections, sources, nodes, transactions, anchors, contexts, seen anchor IDs, encoded bytes, decoded pixels, posters, compressed output, cache entries, registered waits, idempotency entries, pending operations, observation queues, tombstones, CBOR nesting, and record bodies;
- use allowlisted image/video decoders and source-scoped decoder failure boundaries;
- never open a producer-supplied pathname, fetch a producer-supplied URL, or dereference a source-descriptor locator hint;
- never emit a delegated capability in an event, query reply, status record, or trace;
- never let one context observe, enumerate, or cache-probe another context's objects;
- prevent media failure from entering the terminal control-sequence parser.

Producer-supplied text — producer name, producer version, source title, context label — is untrusted content. It MUST be bounded, MUST NOT be written into the terminal text plane, and MUST NOT be interpreted as instruction by any component that forwards it to an automated consumer.

Resource budgets are aggregate per presenter window unless explicitly stated otherwise. They are not multiplied independently by the maximum session count, and a delegated context's quotas are carved out of its parent's, never added to them.

Recommended reference limits are:

| Resource | Limit or policy |
|---|---:|
| Concurrent sessions | 16 |
| Concurrent connections | 64 |
| Sources, aggregate | 64 |
| Nodes, aggregate | 256 |
| Active anchors, aggregate | 256 |
| Seen anchor IDs | 4,096 per session |
| Contexts, per session | 32 |
| Registered waits | 32 per session |
| Pending idempotency entries | 256 per session |
| Source tombstones | 32 per session, retained at most 60 seconds |
| Source width or height | 8,192 pixels, plus body/pixel constraints |
| Retained decoded/poster pixels | At most `8192 * 8192 * 2`, further reduced by configured memory budget |
| Control record body | 1 MiB |
| Status or query reply body | 64 KiB |
| Hard record body | 64 MiB |
| Delta operations per frame | 16 |
| Default rolling media byte window | 4 MiB, raised to at least one source-maximum record |
| Default media packet credits | 32, never below one for an accepted source |

A presenter SHOULD scale decoded/poster budgets to available system and GPU memory and SHOULD reject source creation before memory pressure becomes uncontrolled.

## 16. Conformance

### 16.1 Vivid 1.1 producer conformance

A producer conforms when it:

1. emits version 1.1 in the connection preface and selects Vivid 1.1;
2. uses deterministic CBOR and valid request correlation;
3. authenticates with `HELLO` and honors all `WELCOME` limits and selected features;
4. pipelines only operations whose input dependencies are satisfied;
5. uses one single-use ticket per media channel and never retries a ticket after an attachment attempt;
6. never exceeds source body limits or credits;
7. emits raster, video, image, or audio bodies matching the negotiated profile, including the full-frame obligations in Section 11.4 when deltas are used;
8. uses atomic scene transactions and the selected anchor marker version;
9. services control continuously, including source-scoped loss, visibility, keyframe and full-frame recovery, and bidirectional liveness for features it requested;
10. treats `PLAY`'s `OK` as admission rather than as playback start;
11. treats a version rejection as typed failure and retries, if at all, only on a new connection under explicit operator opt-in.

### 16.2 Vivid 1.1 presenter conformance

A presenter conforms when it:

1. validates framing and directional limits before body allocation;
2. authenticates token or delegated capability and local peer identity before producer-controlled allocation;
3. implements control, raw full-frame RGBA8 raster, scene transactions, credit flow control, and authenticated text-anchor v2 behavior;
4. grants enough initial source credit for one maximum legal record and advertises a steady-state window;
5. implements encoded image, zstd raster, premultiplied alpha, visibility, portable video, video controls, deltas, caching, observability, atomic control, contexts, policy, barriers, and clock sampling only when it advertises them;
6. rejects sessions that cannot select Vivid 1.1, and emits the typed version rejection in Section 3.6;
7. isolates source/media failures from the control session and terminal parser;
8. keeps aggregate quotas, replay state, waits, tombstones, and observation queues bounded;
9. services control continuously under Section 4.3, provably answering `PING`, returning unrelated credits, and satisfying unrelated waits while a long operation is outstanding;
10. never emits a delegated capability except as the direct reply to `DELEGATE_CONTEXT`;
11. preserves unknown `HELLO` and `WELCOME` keys under Section 4.4.

### 16.3 Relay and nested-presenter conformance

An implementation that relays or re-originates Vivid conforms when it:

1. preserves unknown `HELLO` and `WELCOME` keys byte-for-byte, substituting only authentication material (Section 4.4);
2. maintains independent values in every identifier domain listed in Section 1.4 and never forwards one hop's value as another's;
3. never presents an inner acceptance as an outer decode, paint, or presentation;
4. answers `NOT_VISIBLE` rather than timing out when an outer presentation condition cannot be met;
5. terminates raster delta chains at its own boundary and re-encodes independently;
6. computes its own EOS barrier generation and sequence rather than forwarding a received pair;
7. applies the strictest capture policy across all hops;
8. never transmits an outer credential to an inner server or pane;
9. requires explicit route support before carrying any connection kind it does not already implement.

### 16.4 Reference-code mapping

The shared Rust crate should continue to mirror the specification:

| Specification area | Reference source |
|---|---|
| Version, limits, feature/profile constants | `lib.rs` |
| Endpoint parsing, preface, directional body limits, headers, flags, sequences, version rejection | `wire.rs` |
| Deterministic CBOR, numeric/size bounds, preserving negotiation codec | `cbor.rs` |
| Opcodes, features, errors, control schemas, colorimetry, audio initialization, observation and context schemas | `messages.rs` |
| Video/audio/raster/image binary bodies, delta validation, zstd validation | `media.rs` |
| Anchor HMAC and marker codec | `anchor.rs` |

Protocol changes MUST update this specification, shared codecs/constants, golden vectors, and conformance tests in the same change.

Golden vectors MUST assert that the 48-byte video prefix, the 48-byte audio prefix, and the 72-byte full-frame raster header and rectangle descriptor are bit-identical to their Vivid 1.0 encodings, as required by Section 1.5.

## 17. Deliberately deferred work

The following are not part of Vivid 1.1:

- a dedicated semantic side channel carrying document text, accessibility trees, or semantic actions;
- media reattachment after media-connection loss;
- session resumption;
- per-source gain, variable playback rate, presenter-managed looping, and frame stepping;
- periodic presentation, playback, or statistics streams on the control connection;
- adaptive quality hints;
- batched credit records;
- coarse codec/profile capability catalogs;
- per-packet timing or side data;
- generic media fragmentation;
- local shared-memory, `memfd`, or DMA-buffer transport;
- source reconfiguration in place;
- content-addressed caching beyond the context-local encoded-image reuse in Section 12.2;
- RGB8, indexed/palette, HDR, or 10-bit raster formats;
- terminal-multiplexer PTY passthrough;
- source transcoding.

Each requires a separate state machine, platform contract, or measurement that Vivid 1.1 does not have. Several are addressed indirectly: attachment ambiguity is resolved by Section 7.16 rather than by reattachment; semantic discovery is served by the Section 7.13 descriptor rather than by a semantic channel; and diagnostic correlation is served by record sequences, causation IDs, and Section 7.1 clock sampling rather than by per-packet side data.

## 18. Changes from Vivid 1.0

This section is informative.

### 18.1 Compatibility

Vivid 1.1 is a coordinated cutover. A 1.0 peer rejects a 1.1 preface before reading `HELLO`, so the versions do not interoperate on one connection. Section 3.6 makes that rejection diagnosable and permits an explicitly enabled, logged, single retry on a fresh connection.

### 18.2 Unchanged

Transport and discovery, the record header, record flags, the CBOR profile, scene node geometry and clipping, anchors and their derivation, the portable video profile, the portable audio profile, the full-frame raster layout, the still-image transfer, desktop input, and every Vivid 1.0 error code and record-type assignment are unchanged. Section 1.5 freezes all three media prefixes.

### 18.3 Added

| Area | Addition |
|---|---|
| §1.4, §1.5 | Revision domains; media hot-path freeze |
| §3.6 | Typed version rejection and opt-in fresh-connection retry |
| §4 envelope | Preconditions (key 4), idempotency (key 5), causation (key 6) |
| §4.3 | Normative control-plane execution model |
| §4.4 | Preserving negotiation schemas and the relay preservation rule |
| §5.2, §5.3 | `HELLO` key 10 authentication kind; `WELCOME` key 16 initial scene revision |
| §5.4 | Features 18 through 26 |
| §5.5 | Capability generation semantics; feature immutability |
| §7.1 | `ERROR` detail map; `DISPLAY_CHANGED` rate limit and `settled`; timestamped probes |
| §7.4 | Raster update mode 1 and operation limit |
| §7.5 | `CREATE_IMAGE` cache-lookup flag |
| §7.6 | `SOURCE_READY` window advertisement, initial revision, cache-hit form; source replacement sequence |
| §7.8 | `PLAY` admission semantics; EOS media-order barrier; `NEED_FULL_FRAME` |
| §7.13, §7.14 | Source descriptor; capture and export policy |
| §7.15–§7.17 | Observation configuration, change events, status queries, source waits |
| §7.18 | Contexts and opaque delegated capabilities |
| §8 | Attachment generations and attachment resolution |
| §9 | Steady-state window advertisement; credit-return latency obligation |
| §11.4 | Damage and copy raster deltas |
| §12.2 | Context-local encoded-image reuse |
| §14 | Error codes 21 through 24; detail-map and limit-identifier registries |
| §16.3 | Relay and nested-presenter conformance |

### 18.4 Renumbered

Vivid 1.0 §11.4 "Timing and coalescing" is Vivid 1.1 §11.5. Vivid 1.0 §12 gains subsections 12.1 and 12.2. Vivid 1.0 §14 through §17 keep their numbers.

### 18.5 Behavioral changes to existing messages

| Message | Change |
|---|---|
| `PRESENTED` | Now carries the activated `scene_revision` in payload key 0; its meaning is otherwise unchanged |
| `PLAY` | `OK` is defined as admission, not playback start |
| `EOS` | May carry a media-order barrier |
| `DISPLAY_CHANGED` | Rate-limited; may carry `settled` |
| `SOURCE_LOST` | Carries a final `source_revision` and an optional detail map |
| `VIDEO_SUPPORT`, `AUDIO_SUPPORT` | Carry the capability generation that produced the answer |
| `CAPS_CHANGED` | Defined schema; may no longer remove an accepted feature |
| `ERROR` | Payload key 2 is now the structured detail map |
| `CREDIT` | Unchanged, with an explicit prohibition on delaying a credit to batch it |




