# Vivid Protocol 1.5 Core

This file is a normative part of the
[Vivid Protocol 1.5 specification](vivid-protocol-1.5-spec.md).

## 1. Scope and roles

The core profile name is `vivid-core-control-v1`. Every Vivid 1.5 session requires it.

A **presenter** authenticates producers, owns one presentation target, enforces authority and
resource contracts, maintains retained scene state, and renders surfaces.

A **producer** creates surfaces, tracks, and scene nodes and may receive presenter-originated input
when the negotiated profiles permit it.

A **controller** holds root authority and may create contexts and session leases. It may also be a
producer.

A **relay** forwards one authenticated session byte-for-byte. A **terminating gateway** accepts one
session and originates another. A **nested presenter** is a presenter on its inner side and a
producer on its outer side.

Every session has exactly one:

- control connection;
- authenticated principal and root context;
- presentation-target profile;
- target generation;
- scene revision; and
- finite effective resource contract.

Media bytes MUST NOT travel through a terminal PTY. Terminal text carries only ordinary terminal
bytes and the bounded authenticated marker defined by `terminal-surface-v1`.

## 2. Basic types, units, and identifiers

All fixed-width multibyte integers are big-endian.

Unless a narrower width is stated:

- `uint` is an integer in `0..=2^64-1`;
- `int` is an integer in `-2^63..=2^63-1`;
- fields ending in `_us` are integer microseconds;
- dimensions ending in `_px` are unsigned integer pixels; and
- fixed-point coordinates are signed 32.32 values stored in `i64`.

All arithmetic at a trust boundary MUST be checked. Overflow is never wrapping or saturating
unless a field explicitly defines saturation.

### 2.1 Complete object identities

Local numeric IDs are scoped. Their complete identities are:

| Object | Complete identity |
|---|---|
| Session | Presenter instance plus session ID |
| Context | Session ID, context ID |
| Surface | Session ID, context ID, surface ID |
| Track | Session ID, context ID, surface ID, track ID |
| Channel | Track identity, channel generation |
| Node | Session ID, owning context ID, node ID |
| Transaction | Session ID, owning context ID, transaction ID |
| Anchor | Session ID, owning context ID, anchor ID |
| Lease | Issuing session ID, owning context ID, lease ID |
| Input grant | Session ID, surface identity, producer input epoch, presenter grant generation |

A record header carries only the innermost local object ID. Every object operation also carries the
remaining owner tuple in its payload. The two representations of the local ID MUST match.

An implementation MUST store, compare, mutate, remove, retain, report, and tear down an object by
its complete identity. Two contexts or sessions MAY deliberately use the same local IDs.

IDs are nonzero unless a schema explicitly uses zero for session-level traffic. A local ID MUST NOT
be reused while the object, a reply about it, a wait, or a queryable tombstone can still be
observed.

### 2.2 Revisions and generations

The following checked `u64` counters never wrap:

| Counter | Scope | Advances on |
|---|---|---|
| `session_revision` | Session | Any applied session, context, lease, target, surface, track, scene, or input-state mutation |
| `context_revision` | Context | Authority, resource-contract, child, or lifecycle mutation |
| `surface_revision` | Surface | Any surface mutation, including descriptor, policy, active slot, or lifecycle |
| `surface_generation` | Surface | Coordinate mapping or actual input-injection target change |
| `track_revision` | Track | Channel, readiness, playback, activation, recovery, or lifecycle mutation |
| `scene_revision` | Session | Producer commit or automatic node-set change |
| `observation_sequence` | Session | Each non-actionable observation event admitted to the writer |

The presentation target has a separate `target_generation`. A target geometry or topology change
advances it. A stale scene commit fails before mutation.

The following domains are also distinct: capability generation, lease resume generation, channel
generation, input epoch, presenter grant generation, media epoch, packet/frame ID, record
sequence, presentation ID, and semantic content revision. A relay or gateway MUST NOT translate
one domain by copying a value from another domain.

Counter exhaustion closes the owning session with fatal `LIMIT_EXCEEDED`.

## 3. Native discovery and QoS lanes

A native presenter exposes:

| Variable | Meaning |
|---|---|
| `VIVID_ENDPOINT_CONTROL` | Required private reliable-stream endpoint |
| `VIVID_ENDPOINT_INTERACTIVE` | Optional endpoint for the interactive lane |
| `VIVID_ENDPOINT_REALTIME` | Optional endpoint for realtime track channels |
| `VIVID_ENDPOINT_BULK` | Optional endpoint for bulk track channels |
| `VIVID_ROOT_SECRET` | Per-target 256-bit root secret as exactly 64 hexadecimal characters |

Uppercase and lowercase hexadecimal are accepted. Whitespace is not.

The endpoint forms are:

| Form | Meaning |
|---|---|
| `unix:/absolute/path` | Unix-domain stream socket |
| `/absolute/path` | Bare Unix-domain stream socket path |
| `tcp:127.0.0.1:<port>` | Native Windows loopback profile |

Other TCP forms are not part of the native 1.5 profile. Cross-host transport uses SSH or the web
binding and MUST provide confidentiality and integrity.

The logical lane classes are:

| Value | Name | Default use |
|---:|---|---|
| 0 | `control` | Session establishment, mutations, queries, waits |
| 1 | `interactive` | Input binding, input events, input reset and revocation |
| 2 | `realtime` | Audio and latency-critical live video |
| 3 | `bulk` | Ordinary video, raster, still image, export |

Missing endpoint variables select an endpoint value, not a shared byte stream:

```text
interactive -> control endpoint
realtime    -> bulk endpoint if present, otherwise control endpoint
bulk        -> control endpoint
```

Each control, interactive, and track channel remains a distinct transport connection even when
endpoint values are identical. Identical endpoint values are not retried as separate fallbacks.

A track declares its lane at creation. Selecting a lane does not grant priority or authority and
does not alter media semantics. A binding documents whether its physical transport preserves
loss-recovery independence between lanes. If it does not, it reports a degraded carrier; it MUST
NOT claim lane isolation that the transport does not provide.

Establishment may fall back to the selected fallback endpoint only before a `LANE_OPEN` or
`CHANNEL_OPEN` byte is sent. After opening may have begun, recovery uses the authenticated
generation state machine rather than replaying on another endpoint.

The root secret and every endpoint are local discovery values. They MUST NOT appear in Vivid
records, URLs, command arguments, terminal output, status, observations, traces, or logs.

### 3.1 Local peer rules

Unix endpoint directories and sockets MUST be private to the terminal or presenter owner. Where
peer credentials are available, the presenter MUST require the same effective user identity.

The Windows native profile:

- binds exact IPv4 `127.0.0.1` on an ephemeral nonzero port;
- rejects every non-loopback peer before protocol allocation;
- authenticates before producer-controlled allocation; and
- applies a bounded deadline through preface and initial open.

Host names, wildcard addresses, and IPv6 are not part of that profile.

### 3.2 SSH binding

SSH maps each remote private Unix socket connection to one local endpoint connection. The remote
sockets are owner-only. `VIVID_ROOT_SECRET` or a lease activation secret is delivered inside the
authenticated SSH session through a protected environment request, file descriptor, or standard
input channel, never a command argument.

An SSH binding MAY use separate lifecycle-bound SSH TCP connections for logical lanes. If several
lanes share one SSH TCP connection, the binding reports that they share retransmission ordering.
An implementation MUST benchmark this choice; it MUST NOT imply that separate channel sockets
remove head-of-line blocking when an underlying SSH connection reserializes them.

No Vivid record may be embedded in terminal text as fallback.

## 4. Connection and record framing

### 4.1 Initiator preface

The producer initiates every Vivid connection and writes one 16-byte preface:

| Offset | Size | Field |
|---:|---:|---|
| 0 | 4 | ASCII `VIVD` |
| 4 | 1 | Major version; `1` |
| 5 | 1 | Minor version; `5` |
| 6 | 1 | Connection kind |
| 7 | 1 | Flags; zero |
| 8 | 4 | Initiator transmit-body limit |
| 12 | 4 | Reserved; zero |

Connection kinds are `CONTROL` (`0`), `LANE` (`1`), and `TRACK` (`2`). The presenter does not send
a reciprocal preface.

The transmit-body limit is nonzero and no greater than 67,108,864 bytes. Reserved fields, unknown
connection kinds, and invalid limits are fatal framing errors.

The first record is:

- `HELLO` on `CONTROL`;
- `LANE_OPEN` on `LANE`; or
- `CHANNEL_OPEN` on `TRACK`.

No allocation other than bounded pre-authentication parsing occurs before that first record has
been validated and authenticated.

### 4.2 Record header

Every record has a 24-byte header:

| Offset | Size | Field |
|---:|---:|---|
| 0 | 4 | Body length |
| 4 | 2 | Record type |
| 6 | 2 | Record flags |
| 8 | 8 | Innermost local object ID |
| 16 | 8 | Directional connection sequence |

The body immediately follows. Sequence numbers are independent in each direction on each
connection, begin at one, increment by one, and never wrap. A gap, duplicate, reorder, or exhausted
sequence is fatal to that connection.

Record flag bit 0 (`0x0001`) is `OPTIONAL`. Other bits are zero. An unknown optional record is
consumed and ignored without mutation. An unknown required control record receives
`UNSUPPORTED_PROFILE` when correlation is safe. An unknown required lane or track record closes
that connection.

The receiver validates the body length against all effective ceilings before body allocation or
dispatch.

### 4.3 Body ceilings

The producer-to-presenter control ceiling is the minimum of:

- the control preface limit;
- `WELCOME` key 7;
- the effective context control-body limit; and
- 1 MiB.

The presenter-to-producer control ceiling is the minimum of:

- `HELLO` key 4;
- the presenter's configured control ceiling; and
- 1 MiB.

Lane control bodies are limited to 64 KiB. Track control bodies are limited to 64 KiB. Media bodies
are limited by the track configuration, channel acceptance, effective resource contract, preface
limit, and 64 MiB hard ceiling.

Status and query replies are limited to 64 KiB and use pagination when the complete state is
larger.

### 4.4 Version rejection

A receiver rejecting only the preface version SHOULD write exactly one session-level `ERROR` with
`UNSUPPORTED_VERSION`, failed request ID zero, fatal true, and detail values containing its own
major and minor version, then close.

It MUST NOT read `HELLO`, allocate producer state, or emit this diagnostic for malformed magic,
reserved bits, an unknown connection kind, or an invalid body limit. Those are silent closes.

An initiator MAY retry once on a fresh connection only when it implements the reported version
fully and an operator explicitly enabled and logged retry. A track or lane open never retries as
another Vivid version.

## 5. Deterministic control encoding

Control, lane, and track-control records contain one deterministic CBOR value:

```text
{
  0: uint,                 # request ID; zero for unsolicited events
  ? 1: uint,               # transaction ID
  ? 2: uint,               # expected presentation-target generation
  3: { * uint => any },    # opcode payload
  ? 4: { * uint => any },  # typed preconditions
  ? 5: bytes(16),          # idempotency key
  ? 6: bytes(16)           # non-secret causation ID
}
```

The constrained CBOR profile permits:

- definite-length byte strings, text strings, arrays, and maps;
- unsigned and signed integers in shortest form;
- integer map keys in strictly increasing order;
- UTF-8 text, booleans, and null;
- maximum nesting depth 16;
- maximum byte/text length 16 MiB, further bounded by the record ceiling; and
- maximum array or map length 4,096.

It forbids tags, floating point, other simple values, indefinite lengths, duplicate or unsorted
keys, and trailing bytes.

Allocation and configuration schemas are strict. Unknown payload keys are `BAD_MESSAGE` unless a
schema explicitly permits them. `HELLO` and `WELCOME` are preserving schemas: a byte-transparent
relay preserves unknown canonical entries byte-for-byte. A terminating gateway does not forward
either negotiation message; it originates a new one.

### 5.1 Correlation and execution

Correlated requests use a nonzero request ID. IDs are unique while any result can arrive.
Unsolicited events use zero. Replies copy the request ID.

Control requests may be pipelined when their input dependencies are satisfied. The presenter
admits and applies mutations in receive order. Independently completing replies may arrive in
another order and are correlated by request ID.

Both endpoints continuously service control, lane, and reverse track-control traffic independently
of media writes, decoding, presentation, and unrelated flow allowance. A long operation becomes
bounded pending state; it never blocks the parser or session actor. `PING`, input revocation,
channel-local flow updates, track recovery, explicit authority revocation, and unrelated waits
continue while it is pending.

Pending requests, replies, waits, observations, and idempotency results are bounded by the resource
contract. Exceeding a bound returns `LIMIT_EXCEEDED`.

### 5.2 Preconditions and idempotency

Envelope key 4 contains:

| Key | Meaning |
|---:|---|
| 0 | Expected `scene_revision` |
| 1 | Expected `surface_revision` |
| 2 | Expected `surface_generation` |
| 3 | Expected `track_revision` |
| 4 | Expected media epoch |
| 5 | Expected context revision |
| 6 | Expected session revision |
| 7 | Expected semantic content revision |
| 8 | Expected channel generation |
| 9 | Expected lease resume generation |

A supplied precondition must be meaningful to the target. Every precondition is evaluated before
mutation. Failure applies no mutation and returns `PRECONDITION_FAILED` with current sanitized
values.

Envelope key 5 is a cryptographically random 16-byte idempotency key scoped to the authenticated
logical session. The presenter stores a bounded mapping from key to complete-request hash and
non-secret result. Reuse with different bytes is `BAD_MESSAGE`. Exact reuse returns the same
logical result without applying the mutation twice.

Idempotency state remains charged and is retained during a lease suspension. It is discarded on
logical-session closure. A request accepted before unclean loss but lacking a cached, provable
outcome is reported as `UNKNOWN_OUTCOME` after resume; the producer reconciles through revisions
and queries.

Secret material is never an idempotency result. Lease creation contains only a controller-created
verifier. Channel acceptance contains no reusable bearer credential.

Envelope key 6 is a random, non-secret causation ID. It is not derived from authority, identity,
content, a path, or an endpoint. A re-originating gateway may preserve it while recording its own
non-secret route mapping.

## 6. Session establishment

### 6.1 `HELLO`

The first control record is session-level `HELLO`, object ID zero:

| Key | Type | Meaning |
|---:|---|---|
| 0 | text | Producer name, at most 256 UTF-8 bytes |
| 1 | text | Producer version, at most 128 UTF-8 bytes |
| 2 | array(text) | Required profile names, sorted and unique |
| 3 | array(text) | Optional profile names, sorted and unique |
| 4 | uint | Maximum control-record body accepted from presenter |
| 5 | bytes(32) | Client nonce |
| 6 | map | Authentication request |
| 7 | text | Requested presentation-target profile |

`vivid-core-control-v1` and key 7's profile MUST be required. Required-profile failure returns
`UNSUPPORTED_PROFILE`. Optional profiles are intersected with presenter support. The presenter
closes instead of partially accepting a profile whose prerequisite is absent.

The preface already selected version 1.5; version range keys are forbidden.

Authentication map schemas and transcript proofs are defined by the security specification.
Authentication, local peer identity, profile coherence, and resource admission are validated
before producer-controlled allocation.

### 6.2 `WELCOME`

Success returns:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Nonzero session ID |
| 1 | bytes(16) | Non-secret session tag |
| 2 | uint | Root context ID visible to this principal |
| 3 | uint | Presentation-target generation |
| 4 | text | Selected presentation-target profile |
| 5 | map | Target-profile descriptor |
| 6 | array(text) | Accepted profiles, sorted and unique |
| 7 | uint | Maximum control-record body accepted by presenter |
| 8 | bytes(32) | Server nonce |
| 9 | map | Authentication confirmation and lease state |
| 10 | uint | Initial or resumed `session_revision` |
| 11 | uint | Current `scene_revision` |
| 12 | map | Effective resource contract |
| 13 | uint | Establishment state: new (`0`) or resumed (`1`) |
| 14 | uint | Lease resume generation; zero for a non-resumable root session |

The producer validates the authentication confirmation before sending any other record. A failure
closes without application traffic.

On resume, accepted profiles and target profile are exactly those of the suspended logical
session. A changed offer is `BAD_STATE`. `WELCOME` reports current revisions; the producer
reconciles with bounded queries before assuming the fate of a non-idempotent request.

The session tag is an identifier, never authority. It may be logged subject to privacy policy but
MUST NOT be used as an authentication proof, channel key, resume proof, or capability derivation
input by itself.

### 6.3 Session lifetime and liveness

`GOODBYE` is a correlated empty request. The presenter replies `OK`, closes the logical session,
invalidates its lanes and channels, and applies owner-scoped cleanup. A resumable lease does not
suspend after clean `GOODBYE`.

Owner-scoped cleanup removes all addressable protocol objects, authority, waits, decoder state,
media queues, and resource reservations. For a terminal target, this does not require erasing a
bounded target-native poster already attached to authenticated terminal text. Such a poster is no
longer a surface, track, scene node, or queryable session object and follows the terminal profile's
post-disconnect rules. This exception never applies after unclean root-session loss.

Either endpoint may send `PING`; the peer promptly returns `PONG` with the same request ID.
Optional timestamp keys follow the Vivid 1.1 four-timestamp pattern:

| Message | Key | Meaning |
|---|---:|---|
| `PING` | 0 | Sender monotonic transmit time in microseconds |
| `PONG` | 0 | Echoed sender transmit time |
| `PONG` | 1 | Responder monotonic receive time |
| `PONG` | 2 | Responder monotonic transmit time |

Timestamps are in unrelated process-local monotonic domains and are diagnostic. They MUST NOT
modify media PTS, flow allowance, input epochs, or authority deadlines.

Any valid inbound control record proves control liveness. Input has an independent, shorter
watchdog in the desktop surface specification. Track flow remains channel-local.

`CAPS_CHANGED` reports a new capability generation and a reason mask for future track probes. It
does not change accepted profile syntax or mutate live tracks.

## 7. Authenticated lanes

An interactive lane uses a `LANE` connection. `LANE_OPEN`, sequence one, contains:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Session ID |
| 1 | uint | Lane class; `interactive` (`1`) |
| 2 | uint | Lane generation, beginning at one |
| 3 | bytes(16) | Client nonce |
| 4 | bytes(16) | Authentication tag |

The tag is:

```text
HMAC-SHA256(
    session_channel_key,
    "VIVID-LANE-1" ||
    session_id_be64 ||
    lane_class_be32 ||
    lane_generation_be64 ||
    client_nonce
)[0..16]
```

The presenter compares it in constant time and replies on that connection:

| Record | Payload |
|---|---|
| `LANE_ACCEPTED` | `0` session ID, `1` lane class, `2` lane generation, `3` maximum body |

Only one interactive transport may be active for a generation. An exact duplicate open while the
old transport is alive receives `CHANNEL_BUSY` and closes the duplicate. After confirmed transport
loss, an exact open with the same generation and nonce may receive the same logical acceptance if
no input event was accepted on the old transport. Otherwise the producer obtains a new lane
generation through a control-session reconciliation; input remains revoked until a fresh binding.

The interactive lane accepts only records defined by `desktop-input-v1`, `PING`, `PONG`, and
`ERROR`. Loss of the lane revokes input and releases held state but does not by itself destroy the
control session, surfaces, or tracks.

## 8. Surfaces

A surface is the stable semantic, policy, and input identity for presented content. It is not a
decoder or channel.

### 8.1 Common descriptor and policy

A surface descriptor is:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Role |
| 1 | text | Title, at most 256 UTF-8 bytes |
| 2 | uint | Monotonic semantic content revision |
| 3 | uint | Semantic availability mask |
| 4 | text | Opaque locator hint, at most 512 UTF-8 bytes |

Roles are unspecified (`0`), document (`1`), desktop (`2`), timed media (`3`), figure/still
image (`4`), terminal/text surface (`5`), and application canvas (`6`).

Availability bits are extracted text (`0`), structure/accessibility (`1`), links (`2`), outline
(`3`), and invocable actions (`4`). The locator is inert. A presenter never opens, resolves,
connects to, fetches, or parses it.

Capture/export policy bits are:

| Bit | Meaning |
|---:|---|
| 0 | Deny ecosystem screenshot, canvas capture, and readback |
| 1 | Deny descriptor/title/semantic export beyond role |
| 2 | Deny post-disconnect poster retention |
| 3 | Deny content-addressed image caching |
| 4 | Reduce diagnostics to kind and lifecycle |

Effective policy is the union of policy asserted along every hop and by presenter policy. A hop
may make policy stricter but never silently less strict. This constrains cooperating Vivid
components; it is not operating-system content protection.

Registered surface semantic profiles are:

| Profile | Coordinate models | Profile-specific parameters |
|---|---|---|
| `generic-content-v1` | Desktop logical pixels, normalized, or canvas logical units | Empty map |
| `terminal-content-v1` | Terminal content cells | Empty map |
| `desktop-content-v1` | Desktop logical pixels | Desktop surface specification |
| `canvas-content-v1` | Canvas logical units or normalized | Canvas surface specification |

`generic-content-v1` is the normal choice for terminal images/video and other content that has no
desktop-injection or terminal-semantic identity. A semantic profile does not select the session's
presentation-target profile.

### 8.2 `CREATE_SURFACE`

The record object ID equals key 1:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Owning context ID |
| 1 | uint | Nonzero surface ID, unique within context |
| 2 | text | Surface semantic profile |
| 3 | uint | Coordinate model |
| 4 | uint | Logical width, nonzero |
| 5 | uint | Logical height, nonzero |
| 6 | uint | Scale numerator, nonzero |
| 7 | uint | Scale denominator, nonzero |
| 8 | uint | Clockwise rotation: `0`, `90`, `180`, or `270` |
| 9 | map | Complete descriptor |
| 10 | uint | Initial capture/export policy |
| 11 | map | Profile-specific parameters |

The coordinate model is desktop logical pixels (`1`), normalized 32.32 unit square (`2`), canvas
logical units (`3`), or terminal content cells (`4`). The selected surface semantic profile
defines legal models and profile-specific parameters.

Dimensions, scale, rotation, claimed retained pixels, and profile parameters are validated against
the context resource contract before allocation.

Success returns `SURFACE_READY`:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Context ID |
| 1 | uint | Surface ID |
| 2 | uint | Initial `surface_revision` |
| 3 | uint | Initial `surface_generation`, always one |
| 4 | uint | Effective policy |
| 5 | map | Effective profile-specific parameters |

### 8.3 Surface mutation and destruction

`UPDATE_SURFACE` is a complete replacement of mutable surface state:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Context ID |
| 1 | uint | Surface ID |
| 2 | uint | Expected `surface_revision` |
| 3 | uint | Expected `surface_generation` |
| 4 | uint | Logical width |
| 5 | uint | Logical height |
| 6 | uint | Scale numerator |
| 7 | uint | Scale denominator |
| 8 | uint | Rotation |
| 9 | map | Complete descriptor |
| 10 | uint | Requested policy |
| 11 | map | Complete profile-specific parameters |

The semantic profile and coordinate-model kind are immutable. A change to width, height, scale,
rotation, topology, or actual injection target advances `surface_generation`. Any accepted update
advances `surface_revision` and `session_revision`.

Generation-changing updates:

1. revoke the current input grant before applying the new mapping;
2. retain scene nodes and active track slots;
3. require a new input binding against the new generation; and
4. emit `SURFACE_CHANGED` when observations are enabled.

Changing track dimensions or codec does not require a surface update if the surface coordinate
mapping is unchanged.

`DESTROY_SURFACE` contains context and surface IDs. It atomically revokes input, closes and destroys
owned tracks, removes nodes referencing the surface, releases retained resources, advances the
scene and session revisions, and leaves unrelated contexts unchanged.

### 8.4 Surface status

`QUERY_SURFACE` contains context and surface IDs. `SURFACE_STATUS` returns:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Context ID |
| 1 | uint | Surface ID |
| 2 | uint | Surface revision |
| 3 | uint | Surface generation |
| 4 | text | Semantic profile |
| 5 | uint | Coordinate model |
| 6 | uint | Logical width |
| 7 | uint | Logical height |
| 8 | uint | Scale numerator |
| 9 | uint | Scale denominator |
| 10 | uint | Rotation |
| 11 | map | Sanitized descriptor |
| 12 | uint | Effective policy |
| 13 | map | Active slot to track-ID map |
| 14 | uint | Lifecycle: active (`1`), suspended (`2`), tombstone (`3`) |
| 15 | map | Profile-specific status |

A tombstone is metadata-only and bounded by count and age. It contains no channel, decoder, input,
media queue, or scene-node resource.

## 9. Retained scenes

Nodes reference surfaces, never tracks. A scene transaction applies atomically at a compositor
boundary.

### 9.1 Node representation

`CREATE_NODE` and `UPDATE_NODE` use:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Owning context ID |
| 1 | uint | Nonzero node ID within that context |
| 2 | uint | Referenced surface context ID |
| 3 | uint | Referenced surface ID |
| 4 | map | Target-profile geometry |
| 5 | uint | Fit: fill (`1`), contain (`2`), cover (`3`), none (`4`) |
| 6 | uint | Sampling: nearest (`0`), linear (`1`) |
| 7 | int | Z index |
| 8 | uint | Blend: source-over (`0`) |
| 9 | bool | Visible |
| 10 | uint | Opacity, `0..=65535` |
| 11 | map, optional | Target-profile clip |

The target-profile documents define geometry and clipping. All origin-plus-extent and transform
calculations use checked arithmetic. `UPDATE_NODE` is a complete replacement.

The caller must hold scene authority for the node context and observation authority for the
referenced surface. An out-of-authority identity returns `NOT_FOUND`, never disclosure.

### 9.2 Transactions

`BEGIN_TXN` carries the owning context and a nonzero transaction ID in both envelope key 1 and
payload. `CREATE_NODE`, `UPDATE_NODE`, and `DELETE_NODE` carry that transaction ID.

`COMMIT_TXN` carries:

- the transaction ID at envelope key 1;
- expected target generation at envelope key 2;
- optional expected scene revision at precondition key 0; and
- payload key 0 presentation mode, currently next compositor boundary (`0`).

All validation precedes mutation. Success is `SCENE_PRESENTED` after activation:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Activated scene revision |
| 1 | uint | Target generation |

`SCENE_PRESENTED` says only that scene state became active. It does not assert that any track
decoded or displayed media. Track milestones provide that proof.

Stale target generation returns `STALE_TARGET_GENERATION`; failed scene revision returns
`PRECONDITION_FAILED`. Neither applies a partial transaction.

`ABORT_TXN` discards the transaction. A context or surface teardown removes only nodes owned by or
referencing the affected identity, advances `scene_revision`, and never fabricates a
`SCENE_PRESENTED` reply.

### 9.3 Scene query

`QUERY_SCENE` carries optional expected scene revision, opaque continuation cursor up to 64 bytes,
and maximum entry count. `SCENE_STATUS` returns the revision, bounded node entries, optional next
cursor, and total visible-to-principal node count.

A cursor is bound to one session, principal, and scene revision. Mutation invalidates it.

## 10. Observability

This section requires `observability-v1`.

`SET_OBSERVATION` replaces the session observation mask:

| Bit | Class |
|---:|---|
| 0 | Surface changes |
| 1 | Track changes |
| 2 | Scene changes |
| 3 | Playback transitions |
| 4 | Lease and context changes visible to the principal |

Observation events use request ID zero, carry `observation_sequence`, and are non-actionable. They
may be coalesced latest-wins and dropped under bounded writer pressure. `OBSERVATION_GAP` identifies
the first lost sequence and affected classes. Current truth is recovered by query.

The following are actionable and never enter the coalesced observation queue:

- `TARGET_CHANGED`;
- `TRACK_LOST`;
- `NEED_KEYFRAME` and `NEED_FULL_FRAME`;
- `MAX_CHANNEL_DATA`;
- `INPUT_REVOKED`, `INPUT_RESET`, and input watchdog renewal;
- `CONTEXT_CHANGED` and `SESSION_LEASE_CHANGED`;
- `PING`, `PONG`, and fatal `ERROR`.

`SURFACE_CHANGED` carries context ID, surface ID, current surface revision, current surface
generation, changed-field mask, observation sequence, and optional first-lost sequence. Changed
bits are lifecycle (`0`), geometry/generation (`1`), descriptor (`2`), policy (`3`), active slots
(`4`), visibility (`5`), and suspension (`6`).

`SCENE_CHANGED` carries current scene revision, reason mask, observation sequence, and optional
first-lost sequence. Reasons are producer commit (`0`), surface teardown (`1`), anchor loss (`2`),
context/lease cleanup (`3`), and presenter policy/resource cleanup (`4`).

`CAPS_CHANGED` carries the new capability generation and a reason mask: decoder availability
(`0`), device availability (`1`), presenter policy (`2`), and resource pressure (`3`). It does not
alter accepted profiles or existing object syntax.

`QUERY_SESSION` provides a bounded reconciliation root. `SESSION_STATUS` includes:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Session revision |
| 1 | uint | Scene revision |
| 2 | uint | Target generation |
| 3 | uint | Establishment state: active (`1`), suspended (`2`), closing (`3`) |
| 4 | uint | Lease resume generation, or zero |
| 5 | bool | Input is disabled; always true immediately after resume |
| 6 | array(map) | Bounded context/surface/track revision summaries |
| 7 | bytes, optional | Continuation cursor |
| 8 | map | Current aggregate resource usage |

The query accepts a cursor and maximum entries. A cursor is bound to one session revision. A
revision change returns `PRECONDITION_FAILED` and the caller restarts.

Waits are one-shot, bounded pending operations. Core defines their lifecycle; media defines
`WAIT_TRACK` conditions. A wait never blocks the control reader. `CANCEL_WAIT` succeeds even if the
wait is already complete.

## 11. Error encoding

`ERROR` contains:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Registered error code |
| 1 | uint | Failed request ID |
| 2 | map | Structured detail |
| 3 | bool | Fatal |
| 4 | text | Human diagnostic, at most 4,096 UTF-8 bytes |

The diagnostic is untrusted content and is never protocol state. Structured detail is at most
4,096 encoded bytes:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Limit identifier |
| 1 | uint | Current value |
| 2 | uint | Maximum/effective value |
| 3 | uint | Current session revision |
| 4 | uint | Current scene revision |
| 5 | uint | Current surface revision |
| 6 | uint | Current surface generation |
| 7 | uint | Current track revision |
| 8 | uint | Current media epoch |
| 9 | uint | Failed precondition key |
| 10 | bool | Retryable |
| 11 | uint | Suggested retry delay in microseconds |
| 12 | uint | Idempotent result: fresh (`0`), replayed (`1`), already applied (`2`) |
| 13 | uint | Offending payload key |
| 14 | uint | Supported Vivid major |
| 15 | uint | Supported Vivid minor |
| 16 | uint | Current channel generation |
| 17 | uint | Current lease resume generation |
| 18 | uint | Unknown-outcome reconciliation class |

No error detail contains authority material, an authentication tag, channel key, content hash,
endpoint, path, command argument, environment value, media bytes, or input contents.

The numeric error and limit registries are in
`vivid-protocol-1.5-registry.toml`. Registered errors have these semantics:

| Name | Meaning |
|---|---|
| `AUTH_FAILED` | Authentication or proof failed |
| `UNSUPPORTED_VERSION` | Preface version unsupported |
| `UNSUPPORTED_PROFILE` | Required profile or complete profile behavior unavailable |
| `UNSUPPORTED_CONFIG` | Exact surface/track/device configuration unsupported |
| `BAD_MESSAGE` | Structurally or semantically invalid message |
| `BAD_STATE` | Valid message invalid in current state |
| `DUPLICATE_ID` | Complete scoped identity already live |
| `NOT_FOUND` | Object absent or outside caller authority |
| `LIMIT_EXCEEDED` | Static count, size, or allocation limit exceeded |
| `NO_MEMORY` | Bounded allocation failed after admission |
| `FLOW_CONTROL` | Absolute byte/record allowance exceeded |
| `HASH_MISMATCH` | Declared and computed image hash differ |
| `NEED_KEYFRAME` | Random-access unit required |
| `STALE_EPOCH` | Media epoch older than accepted epoch |
| `STALE_TARGET_GENERATION` | Scene target generation mismatch |
| `ANCHOR_INVALIDATED` | Terminal anchor no longer exists |
| `AUTHORITY_REVOKED` | Context or lease authority ended |
| `DECODER` | Decoder failed within a track boundary |
| `DEVICE_LOST` | Required render or audio device lost |
| `TIMEOUT` | Bounded deadline elapsed |
| `PRECONDITION_FAILED` | Typed expected value did not match |
| `ALREADY_APPLIED` | Mutation already applied but no replayable secret/result exists |
| `NOT_VISIBLE` | Presentation condition cannot be satisfied at this hop |
| `CANCELLED` | Pending work cancelled |
| `UNKNOWN_OUTCOME` | Resume cannot prove a non-idempotent pre-loss outcome |
| `CHANNEL_BUSY` | Another transport is active for the generation |
| `STALE_CHANNEL_GENERATION` | Open or media record targets an old generation |
| `LEASE_SUSPENDED` | Mutation is unavailable while logical session is suspended |
| `RATE_LIMITED` | Contractual sustained rate exceeded |
| `INTEGRITY_FAILED` | Authenticated channel or transcript integrity failed |

## 12. Failure isolation

Malformed prefaces, headers, flags, sequences, or over-ceiling bodies close only the affected
connection unless the control connection is the session's authority connection.

Malformed correlated control bodies return `BAD_MESSAGE` when correlation remains safe. Scene and
surface failures apply no partial mutation.

A track framing, flow, decode, decompression, hash, epoch, or channel-authentication failure is
track-scoped where possible. It may detach or lose that track, but it MUST NOT:

- remove the owning surface;
- remove nodes referencing the surface;
- change the surface generation;
- transfer input to another surface;
- corrupt terminal parsing; or
- block unrelated control, input, audio, tracks, or visibility.

An unclean control loss follows the session-lease policy. A root session or zero-grace lease closes
immediately. A positive-grace lease suspends only the complete lease-owned subtree and charges it
until resume or cleanup.

## 13. Core conformance

A Vivid 1.5 producer:

1. emits the 1.5 preface and no version range in `HELLO`;
2. negotiates coherent profiles and uses only accepted complete profiles;
3. uses deterministic CBOR, complete owner tuples, request correlation, and checked counters;
4. validates `WELCOME` authentication before application traffic;
5. never treats a track or channel as surface identity;
6. never retries a possibly active channel on another endpoint outside the channel-generation
   state machine;
7. services control, interactive, and reverse track traffic continuously; and
8. reconciles revisions after resume rather than replaying arbitrary operations.

A Vivid 1.5 presenter:

1. validates prefaces, lengths, peer identity, authentication, profiles, and admission before
   producer-controlled allocation;
2. enforces complete scoped identity for every mutation and cleanup;
3. applies mutations in receive order and long work as bounded pending state;
4. enforces finite aggregate and context resource contracts;
5. keeps lane and track failure isolated;
6. maintains exact revision and generation semantics;
7. preserves unknown negotiation entries only when acting as a byte-transparent relay; and
8. advertises no profile until its complete state machine and failure cleanup are implemented.

A relay or nested presenter:

1. is explicitly transparent or terminating at each hop;
2. maintains independent session, object, revision, generation, flow, and key domains;
3. intersects resource and capture policy at each hop;
4. does not equate an inner acceptance with outer decode or presentation;
5. reports `NOT_VISIBLE` when an outer projection makes a presentation condition impossible; and
6. proves with two-owner ID-reuse tests that one hop's teardown leaves unrelated state unchanged.
