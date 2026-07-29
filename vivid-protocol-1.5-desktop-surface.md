# Vivid Protocol 1.5 Desktop Surface and Input

This file is a normative part of the
[Vivid Protocol 1.5 specification](vivid-protocol-1.5-spec.md). It defines
`desktop-surface-v1` and `desktop-input-v1`.

## 1. Desktop presentation target

A desktop target uses logical desktop pixels and an explicit output topology. It does not
fabricate terminal grid metrics.

For `desktop-surface-v1`, `WELCOME` target descriptor key 5 contains:

| Key | Type | Meaning |
|---:|---|---|
| 0 | int | Virtual-desktop origin X in logical pixels |
| 1 | int | Virtual-desktop origin Y in logical pixels |
| 2 | uint | Virtual-desktop width in logical pixels |
| 3 | uint | Virtual-desktop height in logical pixels |
| 4 | array(map) | Output descriptors |
| 5 | bool | Topology is settled |
| 6 | uint | Topology revision |

Each output descriptor is:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Presenter-local output ID |
| 1 | int | Origin X in virtual logical pixels |
| 2 | int | Origin Y |
| 3 | uint | Logical width |
| 4 | uint | Logical height |
| 5 | uint | Scale numerator |
| 6 | uint | Scale denominator |
| 7 | uint | Clockwise rotation: `0`, `90`, `180`, `270` |
| 8 | bool | Primary output |

Output IDs are session-local and not device identities. No monitor serial number, user name,
desktop name, window title, or login-session identifier appears in the topology.

Output rectangles lie within the virtual-desktop rectangle after checked transform arithmetic.
The presenter may describe gaps. Exactly one output is primary when the list is nonempty.

`TARGET_CHANGED` carries the complete new descriptor, new target generation, and a reason mask:
virtual bounds (`0`), output added/removed (`1`), output geometry (`2`), scale/rotation (`3`),
presentation window (`4`), and target recreation (`5`).

The event is actionable and rate-limited to one update per compositor frame per source of truth.
The presenter may coalesce unsettled topology changes but must deliver the final settled truth.

## 2. Desktop content surfaces

A surface representing an input-capable desktop uses semantic profile `desktop-content-v1` and
coordinate model desktop logical pixels (`1`).

Its core logical width and height describe the canonical captured desktop coordinate rectangle.
Coordinates begin at `(0,0)` in the unrotated canonical surface. Core scale and rotation describe
presentation mapping; the producer's injection adapter maps canonical values to its OS target.

The profile-specific `CREATE_SURFACE`/`UPDATE_SURFACE` map is:

| Key | Type | Meaning |
|---:|---|---|
| 0 | int | Captured virtual-desktop origin X in producer logical pixels |
| 1 | int | Captured virtual-desktop origin Y |
| 2 | array(map) | Sanitized captured-output topology, using the target output schema |
| 3 | uint | Producer-owned desktop semantic generation |
| 4 | uint | Input capability mask |

Input capability bits are keyboard (`0`), pointer motion (`1`), pointer buttons (`2`), and pointer
axis (`3`). A bit says that the producer can inject that class for this surface; it does not grant
the presenter permission.

The producer-owned semantic generation advances when the actual OS session, desktop, security
boundary, injected seat, display mapping, or coordinate truth changes. It is not a user name or OS
identifier.

Any change to width, height, scale, rotation, captured origin/topology, semantic generation, or
input-capability mask advances protocol `surface_generation` and revokes input before the mutation
becomes active.

A codec, bitrate, packetization, colorimetry, or channel change does not change the surface
generation.

## 3. Desktop target node geometry

For a desktop presentation target, the core node geometry map is:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Coordinate space: target logical pixels (`1`) or normalized target (`2`) |
| 1 | int | X as signed 32.32 |
| 2 | int | Y as signed 32.32 |
| 3 | int | Positive width as signed 32.32 |
| 4 | int | Positive height as signed 32.32 |

In logical-pixel space, integer one is one target logical pixel. Origin is the target virtual
desktop origin represented as `(0,0)` in the scene. In normalized space, `0` is the target origin
and `1 << 32` is the complete current target width or height.

Normalized geometry changes its pixel projection when target generation changes. A transaction
still supplies the expected target generation so an application can decide whether to accept that
new projection.

The optional clip map uses the same coordinate space:

| Key | Type | Meaning |
|---:|---|---|
| 0 | int | X |
| 1 | int | Y |
| 2 | int | Positive width |
| 3 | int | Positive height |

The presenter applies fit, then clips the resulting media quad, then clips to target outputs.

## 4. Input ownership model

`desktop-input-v1` requires `desktop-surface-v1` and `live-media-v1`. All input-binding and input
event records travel on the authenticated interactive lane, never the control or bulk track
connection.

Three states are distinct:

1. **Desired state** is the producer's latest `SET_INPUT_BINDING`.
2. **Presenter eligibility** is current focus, local consent, UI policy, and device capability.
3. **Effective state** is a presenter grant for the intersection of desired state and eligibility.

The initial desired and effective states are disabled.

Focus, consent, policy, transport, surface, topology, or watchdog loss revokes the effective grant.
It does not automatically reinstate an old grant when eligibility returns. The producer must issue
a strictly greater input epoch.

## 5. Input binding

### 5.1 `SET_INPUT_BINDING`

The producer sends:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Producer input epoch, nonzero and monotonically increasing |
| 1 | uint | Context ID, or zero to disable |
| 2 | uint | Surface ID, or zero to disable |
| 3 | uint | Expected surface generation, or zero to disable |
| 4 | uint | Requested event-class mask, or zero to disable |
| 5 | uint | Transition reason |
| 6 | uint | Requested watchdog timeout in microseconds |

The record object ID equals surface ID.

Reasons are ordinary policy (`0`), view-only (`1`), track replacement (`2`), OS session transition
(`3`), secure/non-injectable state (`4`), shutdown (`5`), initial enable (`6`), and recovery after
focus/policy (`7`).

Disable requires keys 1 through 4 zero. Enable requires:

- an owned active `desktop-content-v1` surface;
- exact current surface generation;
- context input class bit 4;
- nonzero requested classes within the surface capability mask;
- an active primary-video track whose current channel-generation milestone bit 5 proves first
  presentation for this surface generation; and
- a live interactive lane.

This milestone is part of `live-media-v1`; no undeclared observation-profile dependency exists.

A lower epoch, or the same epoch with different bytes, is `BAD_STATE`. An exact same-epoch retry
returns the current logical result. Epoch exhaustion requires a fresh logical session.

### 5.2 `INPUT_BOUND`

The presenter returns:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Producer input epoch |
| 1 | uint | Presenter grant generation |
| 2 | uint | Effective context ID, or zero |
| 3 | uint | Effective surface ID, or zero |
| 4 | uint | Surface generation, or zero |
| 5 | uint | Effective event-class mask |
| 6 | uint | State: disabled (`0`), enabled (`1`), denied (`2`) |
| 7 | uint | State reason |
| 8 | uint | Effective watchdog timeout in microseconds |

The presenter may narrow requested classes or deny. It never broadens them.

Grant generation is a checked session-wide `u64`. It advances before every effective grant change,
reset, revocation, focus transition, policy transition, lane replacement, suspension, or target
invalidation. It is never reused across resume.

An enabled `INPUT_BOUND` starts the producer's local watchdog. The timeout is between 250,000 and
5,000,000 microseconds and is capped by presenter and context policy. A recommended default is two
seconds.

## 6. Grant renewal and revocation

While a grant is enabled, the presenter sends `INPUT_LEASE_RENEW` no less often than half the
effective timeout:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Producer input epoch |
| 1 | uint | Presenter grant generation |
| 2 | uint | Context ID |
| 3 | uint | Surface ID |
| 4 | uint | Surface generation |
| 5 | uint | Renewal sequence, strictly increasing for the grant |
| 6 | uint | Watchdog timeout in microseconds |

A valid renewal sets the producer's local expiry to receipt monotonic time plus key 6. Ordinary
input events do not extend the watchdog.

Missing, stale, duplicate-with-different-bytes, or late renewal does not extend it. At expiry, the
producer atomically disables the grant, releases held keys and buttons at the injection boundary,
and accepts no more events for that tuple.

`INPUT_REVOKED` is actionable:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Revoked producer epoch |
| 1 | uint | New presenter grant generation |
| 2 | uint | Context ID |
| 3 | uint | Surface ID |
| 4 | uint | Surface generation |
| 5 | uint | Reason |

Reasons are focus loss (`1`), local policy/consent (`2`), surface unavailable (`3`), generation
change (`4`), authority loss (`5`), interactive-lane loss (`6`), suspension (`7`), watchdog (`8`),
queue/injector failure (`9`), and presenter shutdown (`10`).

The presenter:

1. stops admitting physical input for the old grant;
2. advances grant generation;
3. emits `INPUT_REVOKED` when the lane is writable;
4. clears its held-state model; and
5. does not grant again until a greater producer epoch.

Security does not depend on event delivery: connection loss and the producer watchdog take the
same release path.

`INPUT_RESET` has the same identity tuple and a reason. It also advances grant generation and
disables the effective binding. It is used when the presenter cannot prove a balanced transition
history. It is not a cheap "release but keep accepting old-generation input" operation.

## 7. Ordinary input events

Every event carries the binding tuple. The common keys are:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Producer input epoch |
| 1 | uint | Presenter grant generation |
| 2 | uint | Context ID |
| 3 | uint | Surface ID |
| 4 | uint | Surface generation |

The record object ID equals surface ID.

`KEY_INPUT` adds:

| Key | Type | Meaning |
|---:|---|---|
| 5 | uint | USB HID keyboard-page usage, `0x04..=0xe7` |
| 6 | bool | Pressed |

The presenter sends physical transitions and suppresses synthetic browser key repeat. The producer
applies its configured layout and repeat policy.

`POINTER_MOTION` adds:

| Key | Type | Meaning |
|---:|---|---|
| 5 | uint | Nonnegative 32.32 canonical surface X |
| 6 | uint | Nonnegative 32.32 canonical surface Y |

`POINTER_BUTTON` adds:

| Key | Type | Meaning |
|---:|---|---|
| 5 | uint | Button: primary (`0`), auxiliary (`1`), secondary (`2`), back (`3`), forward (`4`) |
| 6 | bool | Pressed |
| 7 | uint | Authoritative current pointer X in 32.32 surface units |
| 8 | uint | Authoritative current pointer Y |

`POINTER_AXIS` adds:

| Key | Type | Meaning |
|---:|---|---|
| 5 | int | Horizontal delta in 1/120-detent units, `-12000..=12000` |
| 6 | int | Vertical delta in 1/120-detent units, `-12000..=12000` |
| 7 | uint | Current pointer X |
| 8 | uint | Current pointer Y |

Coordinates are strictly inside the canonical logical surface:

```text
0 <= x < surface_width << 32
0 <= y < surface_height << 32
```

Rotation, captured origin, output scale, and OS coordinate conversion are applied by the producer
under the named surface generation.

Pointer motion may coalesce only within one exact input tuple and without crossing a button,
axis, revoke, reset, or renewal boundary. Keyboard and button transitions never coalesce.

## 8. Final injection-boundary requirement

Parser order alone is insufficient. The producer MUST implement a final injection gate at the
component that can actually invoke the OS input API.

Immediately before each OS operation, that gate verifies:

```text
event.producer_input_epoch        == active.producer_input_epoch
event.presenter_grant_generation  == active.presenter_grant_generation
event.context_id                  == active.context_id
event.surface_id                  == active.surface_id
event.surface_generation          == current.surface_generation
active.watchdog_deadline          > local_monotonic_now
event.class                       in active.effective_classes
```

The validation and dispatch to the current OS target are serialized with grant revocation and
surface-target changes. Concretely:

1. input work carries the complete tuple through every queue and IPC hop;
2. the final injector processes a generation through one ordered/cancellable gate;
3. revocation closes admission for that generation;
4. the injector completes or cancels already admitted old-generation operations before changing
   the OS target;
5. it releases all held keys and buttons on the old target;
6. only then may a new target or grant become effective.

If an OS API can block beyond the watchdog bound, the implementation isolates it in a cancellable
worker or terminates that worker on revocation. An implementation cannot claim input conformance
while allowing a delayed old-generation operation to execute against a new target.

Stale input is discarded and may increment a bounded diagnostic counter. It is not retried or
translated to the current tuple.

## 9. Overflow, suspension, and recovery

Input queues are finite and charged by event rate and queue capacity. On overflow or injector
uncertainty, the producer:

- disables the grant;
- releases held state;
- reports/reset state through its protected control path;
- discards queued events of that grant; and
- requires a greater producer epoch.

Unclean control loss, lease suspension, interactive-lane loss, context revocation, surface
generation change, or resume performs the same immediate disable and release. Resume always starts
with input disabled and a grant generation greater than any pre-suspension generation.

Replacing a video track under the same surface does not inherently revoke input. Input remains
safe because it is surface-bound. A producer may voluntarily request a higher input epoch during a
visual transition, but protocol identity does not force it.

Changing the actual captured/injected desktop, coordinate mapping, or semantic desktop generation
does advance `surface_generation` and therefore revokes input.

## 10. Desktop and input conformance

Conformance tests MUST:

- send an old event through every asynchronous queue and prove final injection rejects it after
  grant change;
- block or delay the OS-injection worker and prove a target switch cannot overtake it;
- lose focus, consent, policy, lane, control, authority, surface generation, and watchdog;
- prove none auto-restores the prior grant;
- renew just before and after expiry under reordered delivery;
- overflow every input queue and prove held state is released once;
- replace the active video track without changing surface/input identity;
- change the real desktop target and prove surface generation and grant both advance;
- reuse surface ID and input epochs under two contexts and prove no cross-injection;
- hold bulk flow at zero and prove reset, revoke, and watchdog still complete; and
- suspend/resume and prove all pre-suspension input is stale.
