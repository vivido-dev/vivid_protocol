# Migrating Vivid 1.1 Implementations to Vivid 1.5

**Status:** informative but detailed migration guide.
**Audience:** protocol, producer, presenter, relay, gateway, SDK, vvdesk, Vivi, Vivido, vvweb, and
vvmux implementers.

## 1. Executive summary

Vivid 1.5 is an intentional new protocol, not an additive 1.1 feature set. Do not place 1.5
semantics behind new 1.1 feature IDs. Do not teach a single session to switch versions.

The central migration is:

```text
Vivid 1.1
source = logical content + codec + transport + scene target + input target + policy

Vivid 1.5
surface = stable logical content + scene target + input target + policy
track   = immutable codec/raster/image/audio configuration
channel = authenticated transport generation for one track
```

Other system-level changes are:

- terminal metrics become one optional presentation-target profile;
- feature-bit combinations become coherent named profiles;
- root authentication uses a proof instead of sending the root secret;
- delegated capabilities become controller-secret session leases;
- unclean loss may suspend bounded logical state;
- single-use media tickets become authenticated channel opening with positive acceptance;
- incremental credits become cumulative channel-local limits;
- input events carry epochs/generations and are checked at final injection;
- media-specific endpoint selection becomes generic QoS lanes;
- gateway authentication rewriting is forbidden; and
- resource authority includes rates and decoder/GPU cost, not only object counts.

Portable encoded video/audio, full raster, raster-delta, and encoded-image bodies are retained
where this guide says they are retained. Reusing a body format does not make the surrounding record
or lifecycle compatible.

## 2. Deployment strategy

### 2.1 Freeze 1.1

Treat `vivid-protocol-1.1-spec.md` as frozen except for security errata that do not change the wire.
Do not reserve 1.1 feature IDs for 1.5 work.

### 2.2 Run isolated stacks

Recommended deployment:

1. Keep the existing 1.1 listener and parser unchanged.
2. Add a 1.5 listener, or dispatch only after reading the exact complete preface.
3. Build separate 1.1 and 1.5 session-state types.
4. Do not share a source/surface state enum, attachment/ticket type, credit counter, or input event
   type between versions.
5. Select WebTransport protocol or WebSocket subprotocol before Vivid parsing.
6. Roll back by disabling 1.5 admission/listeners, not by in-session downgrade.

The 24-byte record header and deterministic CBOR encoding look familiar. This makes accidental
cross-version dispatch more dangerous, not safe. Some numeric record values now have different
schemas or names.

### 2.3 Compatibility adapter limits

A terminating 1.1↔1.5 adapter may translate media and basic scene presentation, but it cannot
provide to a 1.5 peer:

- final-boundary input generation safety from a 1.1 input origin;
- retry-safe 1.5 channel opening over a 1.1 ticket hop;
- bounded 1.5 session resume through a 1.1 teardown;
- channel-local absolute flow through a 1.1 control-stream credit hop; or
- stable surface identity after a 1.1 source loss without synthesizing and owning that state.

Such an adapter reports its reduced guarantees and acts as a terminating gateway with independent
authority and accounting. It is never byte-transparent.

## 3. Object and identity migration

| 1.1 concept | 1.5 replacement | Why | Implementation action |
|---|---|---|---|
| Source ID | Surface ID plus track ID | A desktop/document is not a decoder instance | Create separate scoped ID types and maps |
| Source | Stable surface + one or more tracks | Codec/transport failure must not destroy semantics or input | Move descriptor, policy, scene reference, and input identity to surface |
| Source kind | Track kind | Media representation is replaceable | Put codec/raster/image/audio config in immutable track |
| Linked video/audio source IDs | Active video/audio slots on one surface | Link should survive video track replacement | Clock and activation operate on surface slot set |
| Media ticket | Session channel key + channel generation + nonce/tag | Ticket delivery/consumption was ambiguous | Implement `CHANNEL_OPEN`/`CHANNEL_ACCEPTED` and open-outcome cache |
| Attachment generation | Channel generation | Generation becomes authenticated and explicitly advanced | Reset flow/readiness on every advance |
| Source revision | Surface revision + surface generation + track revision | One counter mixed semantic and transport changes | Split mutations and update preconditions/queries |
| Source milestone mask | Generation-local track milestones | Sticky 1.1 milestones could describe old media | Reset bits on channel advance and report generation |
| Node source reference | Node surface reference | Scene placement should not change with codec | Retarget storage and scene validation |
| Pointer target source | Input binding surface tuple | Input must survive track replacement and reject semantic target changes | Carry surface generation and grant tuple |
| Source tombstone | Surface and track tombstones | Logical and media failures differ | Keep separate bounded metadata-only tombstones |

Complete identity is now mandatory. If a 1.1 implementation keyed cleanup by local source/node ID,
fix that design before adding 1.5. Every 1.5 object key includes session and context ancestry.

## 4. Section-by-section changes

### 4.1 Vivid 1.1 §1: conventions and scope

**Changed:** Vivid is no longer defined as terminal-attached media. Core does not require a terminal
window, grid, cell size, text plane, or anchor parser.

**Why:** desktop and browser presenters otherwise invent terminal coordinate truth and mix CSS,
logical, physical, cell, and source pixels.

**Apply:**

- make target behavior a trait/profile;
- move grid, cell, text-layer, and anchor state into `terminal-surface-v1`;
- implement desktop topology and logical pixels under `desktop-surface-v1`;
- implement logical viewport mapping under `canvas-surface-v1`;
- keep target generation common, but profile-specific metrics out of core structures.

### 4.2 Vivid 1.1 §1.1: roles

**Changed:** a media connection no longer "carries media for exactly one source after ticket
attachment." A track connection carries one authenticated channel generation for one track.

**Why:** the logical surface must outlive that connection.

**Apply:** ensure track connection teardown detaches the track and leaves surface/node/input
identity untouched.

### 4.3 Vivid 1.1 §1.2 and §3.1: version

**Changed:**

- preface minor byte is `5`;
- `HELLO` has no minimum/maximum version;
- connection kinds are control (`0`), lane (`1`), and track (`2`);
- old media-type connection kinds are not selected in 1.5.

**Why:** an incompatible preface already selects the parser. A second range created false
negotiation expectations.

**Apply:** dispatch parser after exact preface; remove version range from 1.5 negotiation; derive
track kind and lane from authenticated `CHANNEL_OPEN`.

The typed version rejection remains diagnostic and uses a fresh-connection, explicit-opt-in retry.

### 4.4 Vivid 1.1 §1.3: units and geometry

**Changed:** signed 32.32 remains available, but "one unit is one terminal cell" is not core.
Units are profile-specific:

- terminal cells;
- desktop logical pixels;
- canvas logical units; or
- normalized target/surface units.

**Why:** there must be one declared coordinate truth rather than a fake grid projection.

**Apply:** carry a coordinate-space discriminator with geometry and centralize checked transforms.

### 4.5 Vivid 1.1 §1.4: revisions

**Changed:** source revision is split into session, context, surface revision, surface generation,
track revision, scene revision, and observation sequence.

**Why:** semantic/input target changes and media/channel changes require different invalidation.

**Apply:**

- use typed newtypes, not raw `u64`, for every counter;
- advance surface generation only for coordinate/injection target truth;
- do not advance surface generation for codec or channel changes;
- reset track milestones on channel generation;
- never forward a hop's revisions as another hop's values.

### 4.6 Vivid 1.1 §1.5: hot-path invariant

**Retained:** the 48-byte video prefix, 48-byte audio prefix, 72-byte full raster header/descriptor,
and delta operation format remain byte-identical.

**Changed:** record ownership, channel opening, flow control, packet-ID lifetime, and recovery are
1.5 track/channel semantics.

**Apply:** reuse tested media body codecs behind a new versioned record/session API. Golden vectors
should prove body identity while separate tests prove surrounding 1.5 semantics.

### 4.7 Vivid 1.1 §2: discovery and transport

**Changed:**

| 1.1 | 1.5 |
|---|---|
| `VIVID_ENDPOINT` | `VIVID_ENDPOINT_CONTROL` |
| `VIVID_ENDPOINT_BULK` | Retained name, now generic bulk lane |
| Proposed `VIVID_ENDPOINT_AUDIO` | Not adopted |
| None | `VIVID_ENDPOINT_INTERACTIVE` |
| None | `VIVID_ENDPOINT_REALTIME` |
| `VIVID_TOKEN` | `VIVID_ROOT_SECRET` |

Missing lane endpoints fall back by endpoint value while connections stay distinct.

**Why:** endpoint taxonomy must describe QoS, not media kinds. Audio is realtime by default, but
latency-critical video may also be realtime.

**Apply:**

- update environment parsing and sanitization;
- map each track's declared lane to endpoint selection;
- keep fallback before open bytes only;
- document when SSH/proxy layers reserialize distinct connections;
- never claim independent loss recovery without transport evidence.

Local peer checks and "no media through PTY" remain.

### 4.8 Vivid 1.1 §2.4: web transport

**Changed:**

- WebTransport explicitly negotiates protocol `vivid-1.5`;
- mode verifies HTTP/3-capable versus reliable-only behavior;
- pooling is disabled;
- carrier admission is a binding-local header value;
- a bidirectional stream maps to one Vivid connection from preface byte zero;
- WebSocket uses lane-specific subprotocols, no compression, 64 KiB chunks, lower record ceilings,
  and finite reassembly;
- gateways are transparent or terminating.

**Why:** "WebTransport exists" did not prove QUIC stream independence, and classic WebSocket has no
browser receive backpressure equivalent to streams. Authentication rewriting was a confused trust
boundary.

**Apply:** implement the web binding as its own adapter with strict browser budgets. Delete any
zero-token placeholder or "only mutate HELLO key 4" logic.

### 4.9 Vivid 1.1 §3: record framing

**Retained:** 16-byte initiator preface shape, 24-byte record header, directional sequence rules,
optional-record flag, and 64 MiB hard ceiling.

**Changed:** preface version/kinds; first records; lane-specific ceilings; media body limits now
come from track/channel/resource contracts.

**Apply:** keep low-level header codec where cleanly version-independent, but dispatch legal record
sets and object schemas by version and connection kind.

### 4.10 Vivid 1.1 §4: deterministic CBOR

**Retained:** constrained deterministic CBOR and envelope key positions.

**Changed:**

- envelope key 2 means expected presentation-target generation, not terminal display generation;
- precondition keys name surface/track/context/channel/lease domains;
- idempotency state may survive bounded lease suspension;
- `UNKNOWN_OUTCOME` explicitly handles an unprovable pre-loss mutation;
- a terminating gateway originates negotiation instead of preserving it.

**Why:** resume cannot claim arbitrary replay safety, and target/media/authority revisions must not
be conflated.

**Apply:** version the precondition enum and idempotency store. Retain results only while charged
to the logical lease.

### 4.11 Vivid 1.1 §5: establishment and negotiation

**Changed `HELLO`:**

- removes version keys;
- replaces feature ID arrays with required/optional profile-name arrays;
- adds selected target profile and client nonce;
- replaces plaintext token/auth-kind fields with an authentication map and proof.

**Changed `WELCOME`:**

- removes mandatory grid/viewport fields from core;
- adds target profile descriptor;
- adds server nonce and handshake confirmation;
- reports complete effective resource contract;
- reports new/resumed state and lease resume generation.

**Why:** coherent profiles close undeclared-prerequisite gaps, and transcript-bound proof avoids
sending the root secret.

**Apply:**

- implement exact profile prerequisite closure;
- reject partial profile behavior;
- validate server confirmation before application records;
- make target descriptor a profile-specific variant;
- keep unknown-key preservation only in truly transparent relays.

The 1.1 feature registry does not extend into 1.5. Profile names are governed by the 1.5 TOML
registry.

### 4.12 Vivid 1.1 §6: record registry

**Changed:** use `vivid-protocol-1.5-registry.toml`. Assignment states are `reserved`,
`experimental`, `standards-track`, and `retired`.

Some important collisions by version:

| Value | Vivid 1.1 | Vivid 1.5 |
|---:|---|---|
| `0x0001` | `HELLO` with version range/token | `HELLO` with profiles/nonce/auth proof |
| `0x0002` | Terminal-shaped `WELCOME` | Target-profile/auth-confirming `WELCOME` |
| `0x0102` | `CREATE_IMAGE` | `UPDATE_SURFACE` |
| `0x0103` | `CREATE_VIDEO` | `DESTROY_SURFACE` |
| `0x0104` | `CREATE_RASTER` | `QUERY_SURFACE` |
| `0x0206` | `PRESENTED` | `SCENE_PRESENTED` with target generation |
| `0x0300` | Source `PLAY` | Track `PLAY` |
| `0x0601` | `DELEGATE_CONTEXT` | `CONTEXT_READY` |
| `0x7000`–`0x7004` | Input records | Not 1.5 input records |
| `0x8000` | Ticket `ATTACH_CHANNEL` | Authenticated `CHANNEL_OPEN` |
| `0x8001` | `VIDEO_PACKET` | `VIDEO_PACKET`, body retained |
| `0x8003` | `RASTER_FRAME` | `RASTER_FRAME`, body retained |
| `0x8006` | `IMAGE_DATA` | `IMAGE_DATA`, body retained |
| `0x8007` | `AUDIO_PACKET` | `AUDIO_PACKET`, body retained |

**Apply:** never dispatch an opcode without a versioned connection/session type.

### 4.13 Vivid 1.1 §7.1: session/display

**Changed:** `DISPLAY_CHANGED` becomes target-profile `TARGET_CHANGED`. Timestamped `PING`/`PONG`
remains diagnostic. Liveness remains full-duplex, but input has a shorter independent watchdog.

**Why:** a control TCP timeout is too slow to guarantee release of remote held input.

**Apply:** route target changes through profile code and input release through the interactive
watchdog path.

### 4.14 Vivid 1.1 §§7.2–7.5: media configuration

**Changed:** video, audio, raster, and image configurations become kind-specific maps inside
`PROBE_TRACK_CONFIG`/`CREATE_TRACK`. They add contractual maximum frame/access-unit rate, bitrate,
record rate, in-flight bytes, lane, mode, and latency.

**Retained:** codec names, packetizations, portable extradata, colorimetry, raster formats, and
image constraints.

**Why:** one legal track could otherwise consume unbounded sustained decoder/GPU/renderer work.

**Apply:** encoder settings must stay within declared maxima. Presenter admission reserves
worst-case sums before decoder allocation.

### 4.15 Vivid 1.1 §7.6: source creation/loss

**Changed:**

```text
CREATE_* -> SOURCE_READY(ticket, incremental credit)
```

becomes:

```text
CREATE_SURFACE -> SURFACE_READY
CREATE_TRACK   -> TRACK_READY(channel generation, deadline, effective claims)
CHANNEL_OPEN  -> CHANNEL_ACCEPTED(absolute maxima)
```

`SOURCE_LOST` becomes `TRACK_LOST` for decoder/media failure. Surface destruction is explicit.

**Why:** media failure should not blank/delete the logical desktop or its scene placement.

**Apply:** keep degraded active slots and bounded posters; create/prime/activate replacement tracks
without changing surface identity.

### 4.16 Vivid 1.1 §7.7: scene transactions

**Changed:** nodes reference a complete surface identity. Geometry comes from the selected target
profile. Activation reply is `SCENE_PRESENTED`.

**Retained:** atomic transaction behavior, expected target generation, next-compositor activation,
and distinction between scene activation and media presentation.

**Why:** source retargeting during codec replacement was unnecessary scene churn.

**Apply:** remove track/source ID from node storage. Track switching uses `ACTIVATE_TRACK`, not a
scene transaction.

### 4.17 Vivid 1.1 §7.8: playback and recovery

**Changed:**

- desktop/live media does not issue `PLAY`;
- timed media retains exact-PTS `PLAY`, `PAUSE`, `FLUSH`, and `DRAIN` against tracks;
- EOS moves onto the ordered track channel as `CHANNEL_EOS`;
- no cross-connection EOS barrier fields are needed;
- key/full recovery events travel on the track channel when live;
- channel reattach resets decoder/readiness state under a new generation.

**Why:** live desktop capture should not rebuild a timed playback state machine, and same-channel
EOS has inherent media order.

**Apply:** split live and timed track implementations. Preserve linked audio master-clock behavior
at the surface slot group.

### 4.18 Vivid 1.1 §7.9 and §9: credit

**Changed:** remove incremental, saturating `CREDIT` on the control connection. Use cumulative
`MAX_CHANNEL_DATA` on each bidirectional track channel:

```text
sent_body_bytes <= maximum_cumulative_body_bytes
sent_records    <= maximum_cumulative_records
```

**Why:** duplicate updates become harmless, overflow becomes detectable, and a blocked control
writer cannot strand media flow.

**Apply:**

- store sent and maximum totals as checked generation-local counters;
- issue flow updates on the affected track writer;
- reset totals only on explicit channel generation advance;
- size aggregate windows under the context in-flight byte contract.

Do not translate a 1.1 increment to a 1.5 maximum without maintaining a terminating hop's own
bounded accounting.

### 4.19 Vivid 1.1 §7.10: visibility

**Changed:** visibility is primarily a stable surface/placement property, with track presentation
milestones reporting current media. It is no longer tied solely to a transient source.

**Apply:** compute node/target visibility from the surface. Track status may report whether its
active slot produced presentation, but track replacement does not fabricate a surface visibility
transition.

### 4.20 Vivid 1.1 §7.12: desktop input

**Changed completely:**

- input starts disabled;
- binding names context, surface, and exact surface generation;
- producer input epoch and presenter grant generation are on every event;
- pointer events carry surface generation and canonical coordinates;
- presenter grants have a short renewal watchdog;
- focus/policy loss requires a fresh producer epoch, never automatic resume;
- final OS injection validates the tuple and is serialized with target change;
- input travels on an interactive lane.

**Why:** ordered parser bytes could not cancel work already dequeued or crossing IPC. A delayed old
key could reach a new OS target.

**Apply:**

- replace 1.1 input structs/opcodes with 1.5 types;
- carry the tuple through every queue and IPC message;
- implement a final cancellable injection gate;
- make revocation, watchdog expiry, overflow, suspension, and target change share one release path;
- add per-event overhead to performance baselines intentionally.

### 4.21 Vivid 1.1 §§7.13–7.14: descriptor and policy

**Changed:** descriptor and capture/export policy move from source to surface.

**Why:** semantic identity and policy should survive codec/channel replacement.

**Retained:** bounded untrusted text, inert locator, strictest-policy union, cache/poster purge, and
the statement that capture policy is not OS content protection.

**Apply:** migrate storage and query fields; remove duplicates from track configuration.

### 4.22 Vivid 1.1 §§7.15–7.17: observability and waits

**Changed:**

- one optional coherent `observability-v1` profile;
- session reconciliation pages summarize context/surface/track/scene revisions;
- track status separates generation-local milestones;
- input correctness does not depend on observability;
- resume explicitly uses queries and known idempotency outcomes.

**Why:** the proposed 1.1 input-state feature had an undeclared wait/milestone dependency and sticky
old milestones.

**Apply:** update subscriptions and queries to typed surface/track identities. Include channel
generation in every readiness wait.

### 4.23 Vivid 1.1 §7.18: delegated contexts

**Changed:**

- contexts use full resource contracts and capacity reservation;
- `DELEGATE_CONTEXT` and secret-bearing `CONTEXT_CAPABILITY` are removed;
- controller generates activation secret;
- `CREATE_SESSION_LEASE` sends only verifier;
- exact activation retries return the same non-secret logical outcome;
- lease state distinguishes clean close, unclean suspension, resume, revoke, and expiry.

**Why:** a server-generated one-time secret reply was unrecoverable when lost, and teardown on any
EOF confused authority with availability.

**Apply:**

- generate activation material in controller SDK;
- persist only verifier before activation;
- implement issued/reserved/active/suspended/terminal states atomically;
- retain contract and bounded metadata/posters during grace;
- disable input and discard media/decoder queues immediately on suspension;
- derive resume keys from the authenticated handshake.

### 4.24 Vivid 1.1 §8: media attachment

**Changed:** no ticket, no unaffirmed attach, no query asking whether a ticket was consumed.

**Why:** tickets did not expire by time, source allocation could remain unattached, and uncertain
consumption forced a control recovery.

**Apply:** authenticate complete track/generation/nonce with HMAC; send positive
`CHANNEL_ACCEPTED`; cache bounded open outcomes; use explicit `ADVANCE_CHANNEL` for reconnect.

### 4.25 Vivid 1.1 §§10–12: portable media

**Retained exactly:**

- video 48-byte prefix and H.264/HEVC/VP9/AV1 packetizations;
- audio 48-byte prefix and portable MP3/AAC/ALAC/Opus/Vorbis/FLAC/PCM initialization;
- raster full-frame header/descriptor;
- raster overwrite/copy delta layout and semantics;
- exact encoded PNG/JPEG image body.

**Changed:**

- packet/frame IDs belong to a track across channel generations;
- recovered channel requires key/full input;
- body admission obeys absolute channel flow and sustained rate contracts;
- raster/image retained state belongs to track/surface slots;
- image cache is context-scoped under the 1.5 authority model.

**Apply:** retain binary codec modules and golden vectors, replace source lookup and credit APIs.

### 4.26 Vivid 1.1 §13: terminal anchors

**Changed:** anchors exist only in `terminal-surface-v1`. Marker version is 3 and includes context
ID in identity and HMAC. The key derives from the 1.5 handshake.

**Why:** anchors must support context-local ID reuse and cannot derive from a plaintext token or
old delegated capability.

**Apply:** version the scanner; never accept a v2 marker as v3; maintain replay state by complete
context identity. During dual-stack operation, each parser recognizes only its session's marker
version.

### 4.27 Vivid 1.1 §14: errors

**Changed:**

- `UNSUPPORTED_FEATURE` becomes `UNSUPPORTED_PROFILE` at code 3;
- source/display detail keys become surface/track/target keys;
- new errors include `UNKNOWN_OUTCOME`, `CHANNEL_BUSY`, `STALE_CHANNEL_GENERATION`,
  `LEASE_SUSPENDED`, `RATE_LIMITED`, and `INTEGRITY_FAILED`;
- the limit registry describes surfaces, tracks, decoder/rates, lanes, leases, suspension, and
  input.

**Apply:** use the machine registry. Human diagnostics remain non-normative and secret-free.

### 4.28 Vivid 1.1 §15: security and resources

**Changed:** root secret is a transcript proof key. Resource contracts include:

- object counts;
- per-track coded pixels;
- aggregate decoded pixels/second;
- encoded bits/second;
- records/second;
- decoder instances;
- audio rate/channels;
- in-flight bytes;
- retained pixels;
- input event rate;
- pending/observation/wait/idempotency state; and
- lease grace/suspended sessions.

Child contracts reserve capacity from parents. Suspension remains charged.

**Why:** object counts and credits did not constrain continuous CPU/GPU/decoder cost.

**Apply:** add admission reservations and token-bucket enforcement. Moving memory between queues,
decoders, and posters must not make it unaccounted.

### 4.29 Vivid 1.1 §16: conformance

**Changed:** conformance is per coherent profile and additionally requires model exploration of the
composed lease/channel/input/revocation system.

**Apply:** add finite-state model or stronger checker before advertising 1.5. Include two-owner ID
reuse in every lifecycle and recovery regression.

### 4.30 Vivid 1.1 §17: formerly deferred work

Addressed in 1.5:

| 1.1 deferred item | 1.5 result |
|---|---|
| Media reattachment | Explicit channel-generation advance and recovery |
| Session resumption | Bounded lease suspension/resume; no arbitrary replay |
| Source reconfiguration | Track replacement under stable surface; track remains immutable |
| Batched/incremental credit limitations | Duplicate-safe cumulative channel maxima |

Still deferred: arbitrary control/media replay, generic media fragmentation, shared memory/DMA
transport, quality hints without a stable metric, variable playback rate beyond baseline,
presenter looping/step, semantic payload channels, and unreliable input motion.

## 5. Control-record migration map

| Vivid 1.1 operation | Vivid 1.5 operation |
|---|---|
| `HELLO`/`WELCOME` | Same names, incompatible profile/auth/target schemas |
| `DISPLAY_CHANGED` | `TARGET_CHANGED` with profile descriptor |
| `PROBE_VIDEO_CONFIG`, `PROBE_AUDIO_CONFIG` | `PROBE_TRACK_CONFIG` |
| `VIDEO_SUPPORT`, `AUDIO_SUPPORT` | `TRACK_SUPPORT` |
| `CREATE_VIDEO/AUDIO/RASTER/IMAGE` | `CREATE_SURFACE` once, then `CREATE_TRACK` |
| `SOURCE_READY` | `SURFACE_READY`, `TRACK_READY`, then `CHANNEL_ACCEPTED` |
| `DESTROY_SOURCE` | `DESTROY_TRACK` or `DESTROY_SURFACE` depending intent |
| `SOURCE_LOST` | `TRACK_LOST`; surface remains |
| `QUERY_SOURCE`/`SOURCE_STATUS` | `QUERY_SURFACE`/`SURFACE_STATUS` and `QUERY_TRACK`/`TRACK_STATUS` |
| `WAIT_SOURCE` | `WAIT_TRACK` with required channel generation |
| `SET_SOURCE_POLICY` | `UPDATE_SURFACE` policy |
| `UPDATE_SOURCE_DESCRIPTOR` | `UPDATE_SURFACE` descriptor |
| Node with source ID | Node with surface context + surface ID |
| Scene retarget for codec change | `ACTIVATE_TRACK` slot replacement |
| `PLAY/PAUSE/FLUSH/DRAIN` | Track operations under `timed-media-v1` |
| Control `EOS` plus barrier | Track-channel `CHANNEL_EOS` |
| Control `CREDIT` increment | Track-channel `MAX_CHANNEL_DATA` cumulative maximum |
| Source `VISIBILITY` | Surface placement visibility plus track presentation milestone |
| `DELEGATE_CONTEXT` | `CREATE_SESSION_LEASE` with controller verifier |
| `CONTEXT_CAPABILITY` | No equivalent secret-bearing reply |
| `ATTACH_CHANNEL` | Authenticated `CHANNEL_OPEN` |
| No attach reply | `CHANNEL_ACCEPTED` required |
| 1.1 input records | New interactive-lane binding/event records with full tuple |
| Anchor marker v2 | Context-scoped marker v3 |

## 6. Component migration

### 6.1 `vivid_protocol`

- Add a versioned 1.5 module rather than mutating 1.1 types in place.
- Parse `vivid-protocol-1.5-registry.toml` in a collision/status audit.
- Reuse only body codecs whose exact layout is retained.
- Add typed identity tuples, revisions, generations, profile closure, auth transcripts, resource
  contracts, channel-open tags, cumulative flow, input tuples, and v3 anchors.
- Maintain separate golden vectors for 1.1 envelopes and 1.5 envelopes.

### 6.2 SDK

- Replace source-centric desktop helper with surface/track/channel objects.
- Generate lease activation secret before creation.
- Add atomic slot activation and generation-aware waits.
- Add final-injector input guard and watchdog.
- Add lane selection and web-mode reporting.
- Implement resume reconciliation, not request replay.
- Follow the separate vvdesk orchestration guide.

### 6.3 Vivido presenter

- Add target-profile abstraction; keep terminal fields in terminal implementation.
- Build independent 1.5 session actor and maps.
- Add context capacity reservations and rate accounting.
- Add lease reservation/suspension/resume and secret erasure.
- Decouple surface/node lifetime from decoder/track lifetime.
- Add per-track bidirectional writer for flow/recovery.
- Keep posters bounded and policy-aware during suspension.

### 6.4 Vivi and desktop producers

- Create stable surface before tracks.
- State worst-case encoder rate/bitrate/body claims.
- Observe channel acceptance before encode output.
- Keep packet IDs increasing across channel recovery.
- Use key/full frame after generation advance.
- Treat active slot change separately from scene change.

### 6.5 vvweb and `vivido.js`

- Implement desktop or canvas target without fake grid fields.
- Enforce exact web record/reassembly ceilings.
- Run input watchdog outside/stable against render stalls.
- Use current WebTransport protocol/options/mode checks and WebSocket subprotocols.
- Never store Vivid secrets persistently or log admissions.

### 6.6 vvbridge

- Choose transparent or terminating route.
- Delete root-token/zero-placeholder rewriting.
- Bind web admission to validated Origin, browser generation, route, limits, and short activation.
- Map exactly one web stream/socket to one Vivid connection.
- Enforce stream/socket/reassembly/aggregate budgets before routing.
- In terminating mode, translate complete identities and independently account both hops.

### 6.7 vvmux and nested presenters

- Expose inner target profile explicitly.
- Map inner stable surface to an outer stable surface.
- Re-origin each track, channel generation, flow maximum, and recovery request.
- Intersect resource contracts and capture policy.
- Re-origin input binding/grants per hop and validate at the final injector.
- Never copy a credential, revision, milestone, or generation across hops.

## 7. Recommended implementation order

1. Freeze and baseline 1.1 direct, SSH, browser, and nested paths.
2. Add registry audit and isolated 1.5 framing/CBOR/profile negotiation.
3. Implement authentication transcript, context resource contracts, and lease model.
4. Implement stable surface/scene state without media.
5. Implement tracks, channel opening, cumulative flow, and retained media body codecs.
6. Implement live-media activation and channel recovery.
7. Implement desktop/canvas targets.
8. Implement input binding, final injector, and watchdog.
9. Implement bounded suspension/resume and reconciliation.
10. Implement terminal profile and marker v3.
11. Implement WebTransport/WebSocket binding and gateway modes.
12. Add terminating 1.1↔1.5 adapter only if product migration needs it; never make it the 1.5
    semantic reference.

Vvdesk input-enabled sessions should be the first required-1.5 deployment. Ordinary terminal
producers can remain on frozen 1.1 until their surface/track migration is complete.

## 8. Verification checklist

For each implementation:

- exact 1.1 and 1.5 prefaces select isolated parsers;
- no 1.1 feature or opcode table is used as 1.5 authority;
- root secret never appears on wire;
- lost lease creation and `WELCOME` are retry-safe;
- only one activation/resume/channel transport wins each race;
- suspended contracts remain reserved and expire deterministically;
- source loss equivalent loses a track but keeps surface/node/input identity;
- channel open loss and reattach never duplicate flow;
- cumulative maxima tolerate duplicates and reject overflow;
- stale input fails immediately before OS dispatch;
- target switch cannot overtake old injection work;
- input releases on every lane/control/authority/watchdog failure;
- bulk saturation cannot starve interactive or realtime actions;
- web queues close at finite limits;
- all portable body golden vectors remain exact;
- marker v2 and v3 never cross-authenticate; and
- two owners reuse local context, surface, track, node, anchor, lease, and channel numbers while
  one owner's complete teardown leaves the other semantically unchanged.

For each changed Rust project, run from that project:

```sh
cargo fmt --all --check
cargo test --all-targets
cargo clippy --all-targets -- -D warnings
```

Use `--workspace` for Vivido and vvmux changes affecting workspace members. Rerun socket tests
outside a sandbox when socket creation is denied; a skipped path is not integration evidence.
