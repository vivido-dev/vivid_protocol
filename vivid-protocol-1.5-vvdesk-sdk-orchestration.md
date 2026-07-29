# Vivid 1.5 vvdesk SDK Orchestration

**Status:** informative implementation guide. The normative contracts are in the Vivid 1.5
specification suite.

## 1. Goal

The vvdesk SDK should expose one stable desktop surface while handling replaceable media tracks,
channel generations, input epochs, and lease recovery. It must not recreate surface, scene, policy,
and input identity merely because a codec or transport changed.

The SDK keeps protocol IDs, revisions, generations, errors, and resource claims visible to callers.
It may automate order; it must not hide state required for recovery.

## 2. Establishment

Controller flow:

1. Create a child context with desktop/media/scene/input classes and a finite resource contract.
2. Generate a 32-byte activation secret locally.
3. Create a one-use session lease with its verifier, permitted profiles, activation timeout,
   disconnect grace, and cleanup policy.
4. Deliver the activation secret to the worker through protected IPC.
5. Retain the lease ID and controller cleanup handle.

Worker flow:

1. Connect with the exact 1.5 preface.
2. Require `vivid-core-control-v1`, `desktop-surface-v1`, `live-media-v1`, and
   `desktop-input-v1` for an input-enabled session.
3. Authenticate the lease and validate `WELCOME` confirmation.
4. Open and authenticate the interactive lane.
5. Record the effective resource contract before starting capture or encoders.

View-only sessions omit the input profile and interactive lane if no interactive control is
needed.

## 3. Initial desktop presentation

1. Create one `desktop-content-v1` surface containing sanitized topology, semantic generation,
   descriptor, and capture policy.
2. Commit one scene node referencing that surface.
3. Probe and create the primary-video track and optional audio track with worst-case rate/resource
   claims.
4. Open both track channels concurrently and wait for `CHANNEL_ACCEPTED`.
5. Submit an initial video key unit and audio pre-roll, observing absolute flow maxima.
6. Wait for current-generation decoded-output milestone 4.
7. Atomically activate the video and audio slots in one `ACTIVATE_TRACK`.
8. Wait for video milestone 5: first presentation for the current surface generation.
9. Request a new input epoch only after milestone 5.

The scene node exists before media is ready because it references the stable surface. The
presenter may display a policy-permitted poster or empty surface until activation.

## 4. Codec, bitrate, or resolution replacement

When the logical desktop coordinate mapping is unchanged:

1. Keep the surface, node, descriptor, policy, surface generation, and input binding.
2. Create a replacement immutable video track.
3. Open its channel and send a key unit.
4. Wait for current-generation decoded-output readiness.
5. Activate the replacement in the primary-video slot.
6. Observe first presentation if product telemetry needs it.
7. Destroy the old track.

Do not disable/re-enable input solely because the codec track changed. Pointer mapping remains the
surface mapping.

If the encoded resolution change also changes the actual logical desktop coordinate mapping,
follow the surface-transition flow instead.

## 5. Desktop, OS-session, or coordinate transition

This changes surface generation:

1. Disable desired input with a greater producer input epoch.
2. Wait for effective disabled state or locally enforce the same release barrier after lane loss.
3. Ensure the injection backend has completed/cancelled old-generation work and released held
   state.
4. Update the stable surface with the new semantic generation, topology, dimensions, and
   capability mask.
5. Create and prime replacement tracks.
6. Atomically activate the new slot set.
7. Wait for first presentation under the new surface generation.
8. Enable input with another greater producer epoch and exact new surface generation.

The surface ID remains stable when product semantics say this is the same vvdesk desktop. If policy
considers the new OS session a different authority object, destroy the old surface after creating
and presenting a distinct one; do not silently transfer input.

## 6. Track-channel loss

1. Mark only the affected track detached.
2. Continue control, input, surface, scene, and unrelated tracks.
3. Query track status if the local outcome is uncertain.
4. Advance to exactly the next channel generation.
5. Open and authenticate a new channel.
6. Send a key unit, full raster, complete image, or fresh audio access unit as required.
7. Reestablish live readiness; the existing active slot and policy-permitted poster remain.

Do not create a replacement surface. Do not replay an old open or flow maximum into the new
generation.

## 7. Session suspension and resume

On unclean control loss:

- release input immediately;
- stop capture writes;
- consider every lane and track channel closed;
- retain local object/revision state for the negotiated grace only; and
- attempt resume with the derived resume key, lease ID, session ID, and current generation.

After resumed `WELCOME`:

1. Treat input as disabled.
2. Page through `SESSION_STATUS` at one revision.
3. Reconcile surface, scene, track, slot, and idempotency outcomes.
4. Resolve unknown non-idempotent outcomes with object queries.
5. Open a fresh interactive-lane generation.
6. Advance/reopen selected track channels and send recovery units.
7. Wait for current-generation media readiness and presentation.
8. request a new input epoch.

If grace expires or resume is rejected, discard the logical session and ask the controller to
create a new lease. Do not treat activation material as a resume credential.

## 8. Ordered shutdown

For a clean timed session:

1. Disable input and complete the injection release barrier.
2. Send `CHANNEL_EOS` on video and audio channels after their final media records.
3. Wait for buffered playback completion when required.
4. Drain audio.
5. Destroy inactive and active tracks.
6. Destroy the surface, or retain only if application policy explicitly requires a presenter
   poster until `GOODBYE`.
7. Send `GOODBYE`.

For live desktop shutdown, EOS/drain is optional unless the application wants buffered completion;
input release is not optional.

Cancellation reuses this same owner-scoped path with bounded timeouts. Timeout escalation closes
only this lease-owned subtree.

## 9. API shape

Recommended SDK objects:

- `SessionLeaseBuilder`: controller-created secret/verifier, profile closure, contract, grace.
- `DesktopSurface`: stable ID, revision, generation, descriptor, policy, topology.
- `TrackBuilder`: immutable configuration and explicit resource claims.
- `TrackChannel`: generation, authenticated open, cumulative flow maxima, recovery.
- `SurfaceSlots`: atomic activation set with readiness preconditions.
- `InputBindingGuard`: producer epoch, grant generation, watchdog, final-injector barrier.
- `DesktopSession`: orchestration and cancellation without hiding constituent state.

Deterministic fakes should model lost replies, exact retries, partial transport loss, delayed
injection, cumulative flow updates, revision races, and two-owner local-ID reuse.

## 10. Performance gates

Before enabling 1.5 vvdesk by default, measure:

- direct/native, SSH, WebTransport, and WebSocket paths;
- key-unit-to-first-presentation latency;
- channel-open and reattach latency;
- audio underrun and input-reset latency under saturated bulk video;
- allocations and copies within web reassembly limits;
- input event overhead from generation fields;
- suspension memory and grace cleanup; and
- throughput at exact resource and flow-window limits.

No negotiated-but-unused profile should add media records or per-media allocations. Input event
growth is intentional and measured; it buys final-boundary stale-event rejection.
