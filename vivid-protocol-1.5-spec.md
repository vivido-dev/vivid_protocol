# Vivid Protocol 1.5 Specification

**Status:** normative design specification for the Vivid 1.5 implementation cutover.
**Vivid version:** 1.5.
**Architecture:** stable surfaces, replaceable tracks, authenticated channel generations, and
bounded session leases.

The optional `audio-input-v1` profile in the media specification adds consent-gated microphone
uplink without changing the Vivid 1.5 preface or downlink defaults.

## 1. Status and normative language

The key words **MUST**, **MUST NOT**, **REQUIRED**, **SHALL**, **SHALL NOT**, **SHOULD**,
**SHOULD NOT**, **RECOMMENDED**, **NOT RECOMMENDED**, **MAY**, and **OPTIONAL** are normative
requirement levels.

This file is the entry point for a multipart specification. The following files are jointly
normative:

1. [Vivid 1.5 core](vivid-protocol-1.5-core.md) defines framing, deterministic control encoding,
   coherent-profile negotiation, the object model, scenes, observability, native transport lanes,
   errors, and base conformance.
2. [Vivid 1.5 security and resource model](vivid-protocol-1.5-security-and-resource-model.md)
   defines authentication, contexts, retry-safe session leases, suspension and resumption,
   resource contracts, and gateway trust boundaries.
3. [Vivid 1.5 media](vivid-protocol-1.5-media.md) defines immutable tracks, atomic track
   activation, authenticated channel opening, absolute flow control, live and timed playback,
   portable video/audio/raster/image bodies, and recovery.
4. [Vivid 1.5 terminal surface](vivid-protocol-1.5-terminal-surface.md) defines terminal target
   metrics, grid and anchor geometry, text layers, and authenticated text-anchor markers.
5. [Vivid 1.5 desktop surface](vivid-protocol-1.5-desktop-surface.md) defines desktop topology,
   coordinate mapping, surface generations, and generation-checked desktop input.
6. [Vivid 1.5 canvas surface](vivid-protocol-1.5-canvas-surface.md) defines a terminal-free logical
   canvas target.
7. [Vivid 1.5 web bindings](vivid-protocol-1.5-web-bindings.md) defines WebTransport and WebSocket
   carriers, explicit degraded modes, finite browser buffering, and terminating gateways.
8. [Vivid 1.5 file drop](vivid-protocol-1.5-file-drop.md) defines consent-gated regular-file
   copying from a presenter into a producer-selected directory, and the `file-drop-path-v1`
   sub-profile that discloses the committed destination path.

The machine-readable
[Vivid 1.5 registry](vivid-protocol-1.5-registry.toml) is normative for numeric assignments and
assignment status. A prose table and the registry disagreeing is a specification defect; until
corrected, the machine-readable registry controls numeric identity and the prose controls
semantics.

## 2. Architectural contract

Vivid 1.5 is a presentation and media protocol. A terminal is one presentation target profile,
not a core protocol assumption.

The protocol separates the following identities:

```text
authenticated session
└── context
    ├── surface                         stable semantic and input identity
    │   ├── descriptor and policy
    │   ├── scene placements
    │   ├── primary-video track         replaceable media configuration
    │   │   └── channel generation      current transport attachment
    │   ├── audio track
    │   │   └── channel generation
    │   ├── raster track
    │   └── poster track
    └── session lease                   delegated authority and recovery policy
```

A **surface** remains stable across codec, dimensions, packetization, colorimetry, device, and
transport changes. Its generation changes only when the surface coordinate mapping or actual
input-injection target changes.

A **track** is one immutable media configuration owned by one surface. A replacement track may be
created, primed, and atomically activated without changing scene placements, input identity,
descriptor, or capture policy.

A **channel generation** is one authenticated transport attachment for one track. Channel loss
does not destroy the surface. Reattachment uses a new generation and requires a new key unit or
full frame.

A **session lease** is delegated authority whose activation secret is created by the controller,
not returned by the presenter. Clean closure and authority revocation clean up immediately.
Unclean transport loss suspends bounded logical state for an explicitly limited grace period and
revokes input immediately.

## 3. Required invariants

Every conforming implementation MUST maintain these invariants:

1. **Stable identity:** a track or channel failure cannot delete, retarget, or transfer a surface
   or node except through the surface- and context-scoped lifecycle explicitly defined here.
2. **Complete ownership:** every surface, track, node, anchor, lease, wait, channel, and retained
   object is addressed and cleaned up by its complete owner tuple. A local numeric ID alone is
   never teardown identity.
3. **Input safety:** an input event reaches the final OS-injection operation only if its producer
   epoch, presenter grant generation, surface ID, and surface generation still equal the active
   binding and its watchdog has not expired.
4. **Lease safety:** at most one active logical child session exists for a one-use lease.
   Creation and activation retries never require the presenter to reproduce a secret-bearing
   reply.
5. **Channel safety:** at most one accepted transport exists for a track channel generation.
   An uncertain open can be retried without duplicating authority, media acceptance, or flow
   allowance.
6. **Resource safety:** active, reserved, and suspended state remains charged to finite context
   and presenter contracts. Continuous legal traffic cannot evade configured CPU, GPU, decoder,
   byte-rate, record-rate, or retained-memory limits.
7. **Lane isolation:** saturated bulk traffic cannot indefinitely delay input revocation, input
   reset, audio flow updates, authority revocation, or control liveness.
8. **Deterministic recovery:** transport loss either resumes within the negotiated grace period or
   performs owner-scoped cleanup at expiry. No half-owned state remains indefinitely.
9. **Gateway clarity:** a gateway is byte-transparent or it terminates one authenticated session
   and originates another. Authentication substitution is forbidden.
10. **Bounded parsing:** lengths, counts, rates, geometry, generations, revisions, hashes,
    cumulative offsets, and decoded sizes are validated with checked arithmetic before allocation
    or mutation.

These invariants apply across nested presenters and relays. A hop that re-originates a session
creates independent IDs, revisions, grants, keys, channel generations, and flow-control offsets.

## 4. Coherent profiles

Vivid 1.5 negotiates named, versioned profiles rather than independent feature bits. A profile is
an indivisible syntax and behavior contract. A presenter MUST NOT accept a profile unless it
implements the entire profile and every declared prerequisite.

| Profile | Status | Prerequisites | Purpose |
|---|---|---|---|
| `vivid-core-control-v1` | Required | None | Framing, control, contexts, surfaces, scenes, leases, lanes, errors |
| `terminal-surface-v1` | Optional | Core | Terminal grid, text layers, authenticated anchors |
| `desktop-surface-v1` | Optional | Core | Desktop topology, coordinate mapping, input-capable surfaces |
| `canvas-surface-v1` | Optional | Core | Logical terminal-free canvas target |
| `live-media-v1` | Optional | Core and one surface profile | Immediate/live video, audio, raster, and image tracks |
| `timed-media-v1` | Optional | `live-media-v1` | Exact-PTS playback, pause, flush, EOS, and drain |
| `audio-gain-v1` | Optional | `timed-media-v1` | Track-scoped audio output gain |
| `desktop-input-v1` | Optional | `desktop-surface-v1`, `live-media-v1` | Epoch-checked input binding and watchdog |
| `file-drop-v1` | Optional | Core | User-gesture regular-file copy from presenter to producer |
| `file-drop-path-v1` | Optional | `file-drop-v1` | Committed absolute destination path on a successful `FILE_RESULT` |
| `observability-v1` | Optional | Core | Bounded status, change events, and waits |
| `web-carrier-v1` | Binding-selected | Core | WebTransport and WebSocket carrier constraints |
| `multiplexed-session-carrier-v1` | Experimental | Core | Non-normative carrier research only |

Exactly one of `terminal-surface-v1`, `desktop-surface-v1`, or `canvas-surface-v1` is selected as
the session's presentation-target profile. A producer can still create content surfaces whose
semantic role differs from the target; this selection defines target metrics and scene geometry,
not the content's media kind.

Unknown optional profiles are ignored. An unknown required profile rejects establishment with
`UNSUPPORTED_PROFILE`. Profile syntax is immutable for a session, including across suspension and
resume.

`multiplexed-session-carrier-v1` is not part of Vivid 1.5 conformance. It reserves a name for
experimentation with a multiplexed scheduler and compact framing. It MUST NOT be enabled on a
standards-track Vivid 1.5 endpoint and MUST NOT reuse standards-track record assignments.

## 5. Version boundary

Every Vivid 1.5 connection writes the exact version-1.5 preface defined by the core specification.
`HELLO` contains no minimum or maximum Vivid version. The preface is the version selection.

A 1.5 endpoint MAY coexist with an endpoint of another Vivid version on separate listeners or may
inspect only the complete preface before dispatching to isolated parsers. It MUST NOT:

- negotiate 1.5 from another version's preface;
- emit another version's preface with 1.5 semantics;
- carry more than one version in one logical session;
- reuse another version's token, media ticket, delegated capability, source object, revision, or
  attachment generation as 1.5 authority or identity; or
- downgrade an established session.

A version rejection is diagnostic only. A retry, if explicitly enabled, uses a fresh connection
and a fully independent implementation of the reported version.

## 6. Conformance and proof obligations

An implementation conforms only to the profiles it advertises. All implementations conforming to
Vivid 1.5 MUST implement `vivid-core-control-v1`.

Before a presenter advertises Vivid 1.5, its composed lease, suspension, channel-generation,
input-binding, and revocation state machines MUST be subjected to exhaustive finite-state
exploration or a stronger model-checking method. The model and implementation tests MUST cover:

- simultaneous activation attempts;
- lost `WELCOME` and lost `CHANNEL_ACCEPTED`;
- old and new transports alive concurrently;
- clean close, unclean loss, resume, explicit revoke, parent revoke, and grace expiry;
- input events queued before and after every grant transition;
- channel loss during media parsing and during a cumulative flow update;
- resource charging throughout issued, reserved, active, and suspended lease states; and
- at least two owners deliberately reusing the same local surface, track, node, channel, and lease
  numbers.

Scenario tests alone are insufficient for the composed authority state machine. Wire-number
allocation status is controlled by the registry; experimental designs remain in experimental
ranges until their state models and parser fuzzing satisfy the same obligations.

## 7. Deliberately excluded from the initial baseline

The following are not standards-track Vivid 1.5 behavior:

- one protocol-level multiplexed carrier across all native transports;
- arbitrary replay of control requests or media after resume;
- more than a bounded metadata/poster suspension state;
- input motion datagrams;
- media-type-specific endpoint variables;
- gateway authentication rewriting;
- in-place mutation of an immutable track's codec configuration;
- implicit input restoration after focus, policy, transport, or target loss;
- unbounded capability catalogs, observation streams, or browser receive queues;
- generic file transfer, filesystem browsing, clipboard, secure-attention sequence, credential
  transport, or login approval; `file-drop-v1` is only the bounded user-gesture copy defined by
  its normative part, and `file-drop-path-v1` adds only the committed destination path on a
  successful result; and
- treating capture-policy bits as operating-system content protection.

The experimental multiplexed carrier may be developed after the baseline object, authority, and
input models are implemented and verified. Its adoption would require a separately versioned
profile and cannot silently change the baseline carrier.
