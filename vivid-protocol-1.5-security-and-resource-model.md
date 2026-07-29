# Vivid Protocol 1.5 Security and Resource Model

This file is a normative part of the
[Vivid Protocol 1.5 specification](vivid-protocol-1.5-spec.md).

## 1. Security model

Vivid authority is explicit and hop-local:

- a root secret authenticates the controller for one presentation target;
- a context narrows operation classes and reserves a finite resource contract;
- a session lease delegates one child logical session to one context;
- session handshake keys authenticate lane and track openings;
- a transparent relay changes none of these bytes; and
- a terminating gateway authenticates one side and originates an independent session on the
  other.

An endpoint string, session ID, session tag, context ID, lease ID, route ID, browser Origin,
channel generation, nonce, or causation ID is not authority.

Every remote transport carrying a root secret, activation secret, or resume proof MUST provide
server authentication, confidentiality, and integrity before Vivid authentication. The native
Unix/loopback profile relies on private local transport and peer checks. SSH and HTTPS bindings
provide the remote security layer.

## 2. Cryptographic primitives and encoding

Vivid 1.5 uses:

- a cryptographically secure random-number generator;
- SHA-256;
- HMAC-SHA256; and
- HKDF-SHA256 as defined by RFC 5869.

Byte concatenations below are exact. ASCII labels contain no terminating NUL. Integers are
fixed-width big-endian when a width is shown. Canonical CBOR values are encoded under the core
profile before hashing.

All authentication tags and secret verifiers are compared in constant time after exact-length
validation. Implementations MUST NOT log the compared bytes or a derived digest.

## 3. Root authentication and session key derivation

`VIVID_ROOT_SECRET` decodes to exactly 32 random bytes. It is not sent in `HELLO`.

### 3.1 Root authentication request

For root authentication, `HELLO` key 6 is:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Authentication kind: root (`0`) |
| 1 | bytes(32) | Client proof |

Let `hello_authless` be the canonical encoding of the complete `HELLO` payload with authentication
map key 1 omitted. Let `preface` be the exact 16 initiator-preface bytes. The proof is:

```text
HMAC-SHA256(
    root_secret,
    "VIVID-ROOT-HELLO-1" ||
    preface ||
    SHA-256(hello_authless)
)
```

The client nonce in `HELLO` key 5 is therefore authenticated. A presenter maintains a bounded,
short-lived replay filter for recently accepted root client nonces. Reuse is `AUTH_FAILED`.

### 3.2 Derived session secrets

For a new root session, `session_secret` is the root secret. For a newly activated lease it is the
activation secret. For a resumed lease it is the prior resume key.

Each binding supplies a 32-byte `carrier_binding_key`. Native, SSH, and WebSocket bindings use 32
zero bytes. A direct WebTransport binding that negotiated exporter binding uses the exporter value
defined by the web-binding specification. A byte-transparent relay cannot add exporter binding
because it is not an endpoint of the relayed Vivid authentication.

After choosing the 32-byte server nonce returned by `WELCOME`, derive:

```text
handshake_prk = HKDF-Extract(
    salt = client_nonce || server_nonce || carrier_binding_key,
    IKM  = session_secret
)

session_channel_key = HKDF-Expand(
    handshake_prk,
    "VIVID-SESSION-CHANNEL-1" || session_id_be64,
    32
)

session_resume_key = HKDF-Expand(
    handshake_prk,
    "VIVID-SESSION-RESUME-1" || session_id_be64 || resume_generation_be64,
    32
)

anchor_key = HKDF-Expand(
    handshake_prk,
    "VIVID-ANCHOR-KEY-3" || session_tag,
    32
)
```

A non-resumable root session derives `session_resume_key` only as transcript separation material
and immediately discards it.

`WELCOME` key 9 contains:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Authentication kind |
| 1 | bytes(32) | Server confirmation |
| 2 | uint | Lease state, or zero for root |
| 3 | uint | Activation-attempt status: fresh (`0`) or exact replay (`1`) |

Let `welcome_unconfirmed` be the canonical complete `WELCOME` payload with key 9 subkey 1 omitted.
The confirmation is:

```text
HMAC-SHA256(
    handshake_prk,
    "VIVID-WELCOME-1" || SHA-256(welcome_unconfirmed)
)
```

The producer validates it before using the session channel key. A mismatch is
`INTEGRITY_FAILED` and closes without application traffic.

Session channel, resume, and anchor keys are independent. None is derived from another derived key.
They are erased on final logical-session cleanup.

## 4. Contexts and resource contracts

A context is an authority and accounting subtree. The root context is created by the presenter
with finite policy-selected limits. A controller may carve child contexts from uncommitted parent
capacity.

### 4.1 Operation classes

| Bit | Authority |
|---:|---|
| 0 | Observe owned objects and sanitized status |
| 1 | Create surfaces and tracks and submit media |
| 2 | Create and mutate owned scene nodes |
| 3 | Create terminal anchors |
| 4 | Establish and receive desktop input for owned surfaces |
| 5 | Create child contexts and session leases |

A resource limit never grants an operation class. A class bit without sufficient resource
capacity does not make an operation admissible.

### 4.2 Resource-contract map

Every contract value is finite. Zero denies that resource. There is no wire value for unlimited.

| Key | Resource |
|---:|---|
| 0 | Maximum surfaces |
| 1 | Maximum tracks, aggregate |
| 2 | Maximum nodes |
| 3 | Maximum video tracks |
| 4 | Maximum audio tracks |
| 5 | Maximum raster tracks |
| 6 | Maximum encoded-image tracks |
| 7 | Maximum decoder instances |
| 8 | Maximum coded pixels per video/raster track |
| 9 | Maximum reserved decoded pixels per second |
| 10 | Maximum reserved encoded bits per second |
| 11 | Maximum reserved media records per second |
| 12 | Maximum audio sample rate per track |
| 13 | Maximum audio channels per track |
| 14 | Maximum aggregate in-flight media body bytes |
| 15 | Maximum concurrent track connections |
| 16 | Maximum retained decoded/poster pixels |
| 17 | Maximum media-record body |
| 18 | Maximum control-record body |
| 19 | Maximum pending correlated requests |
| 20 | Maximum registered waits |
| 21 | Maximum idempotency entries |
| 22 | Maximum child session leases |
| 23 | Maximum disconnect grace in microseconds |
| 24 | Maximum input events per second |
| 25 | Maximum observation-queue entries |
| 26 | Maximum encoded-image cache bytes |
| 27 | Maximum open scene transactions |
| 28 | Maximum child contexts |
| 29 | Maximum suspended child sessions |
| 30 | Maximum pending channel-open attempts |
| 31 | Maximum active terminal anchors |
| 32 | Maximum seen terminal anchor IDs |

The contract returned in `WELCOME` and every effective child contract contains all keys. Omission,
negative interpretation, or unknown key is `BAD_MESSAGE`.

### 4.3 `CREATE_CONTEXT`

`CREATE_CONTEXT` has:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Nonzero child context ID |
| 1 | uint | Parent context ID |
| 2 | uint | Requested operation-class mask |
| 3 | text | Diagnostic label, at most 64 UTF-8 bytes |
| 4 | uint | Lifetime from acceptance in microseconds; zero means parent lifetime |
| 5 | map | Requested resource contract |

The record object ID equals key 0. The caller requires class bit 5 in the parent.

The effective class mask is an intersection with the parent. The effective lifetime is no longer
than the parent's remaining lifetime. The effective resource contract is the component-wise
minimum of request, policy, and currently uncommitted parent capacity.

Contract capacity delegated to a live child is **reserved**, not merely checked. It is unavailable
for another child until the first child's final cleanup. This prevents sibling contracts whose
combined legal use exceeds the parent. A presenter may reserve less than requested but never more
than any requested value.

Success returns `CONTEXT_READY`:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Context ID |
| 1 | uint | Effective class mask |
| 2 | map | Effective resource contract |
| 3 | uint | Effective lifetime from acceptance |
| 4 | uint | Initial context revision |

`REVOKE_CONTEXT` contains the complete parent-authorized context identity. It synchronously:

1. invalidates descendant leases and resume proofs;
2. revokes active input;
3. closes child control, lane, and track transports;
4. destroys only surfaces, tracks, nodes, anchors, waits, and cache entries in that subtree;
5. releases that subtree's reservations;
6. advances affected context, scene, and session revisions; and
7. emits actionable `CONTEXT_CHANGED` to principals entitled to observe it.

Context states are active (`1`), expired (`2`), revoked (`3`), and closed (`4`). Reason bits are
explicit revocation (`0`), expiry (`1`), parent cleanup (`2`), presenter policy (`3`), and resource
enforcement (`4`).

`CONTEXT_CHANGED` is actionable and carries context ID, state, context revision, reason mask, and
parent context ID. It never reports another subtree's existence or resource use.

An object outside the caller's subtree is `NOT_FOUND`. Counts, IDs, cache hits, rate state, and
resource use of another subtree are never disclosed.

## 5. Resource admission and enforcement

### 5.1 Static reservations

Every track creation declares the worst-case claims required by the media specification:

- maximum coded pixels;
- maximum decoded pixels per second;
- maximum encoded bits per second;
- maximum records per second;
- decoder instances;
- maximum record body;
- maximum in-flight body bytes;
- audio sample rate and channels where applicable; and
- retained pixel charge.

The presenter validates both the per-track maxima and the sum of reservations in the complete
context ancestry before decoder, GPU, cache, channel, or large-buffer allocation.

Expected bitrate and frame rate are contractual maxima in Vivid 1.5, not descriptive hints. A
producer that cannot state a finite maximum requests a presenter-policy ceiling and uses the
effective value returned by `TRACK_READY`.

Reservations remain charged for the track lifetime, including detached-channel and suspended-lease
states, even when the presenter releases the physical decoder. This forbids a suspended child from
holding logical recovery authority while its parent delegates the same capacity elsewhere.

### 5.2 Dynamic bytes and rates

Absolute channel flow limits bound in-flight media bodies. They do not replace sustained-rate
contracts.

For each rate contract, a sender MUST pace traffic so the charged amount fits a token bucket with:

```text
refill_rate = effective units per second
capacity    = max(effective units per second, one maximum legal record's charge)
```

Buckets begin full. Time is monotonic. Charged units are:

- encoded bits: complete media body bytes multiplied by eight;
- media records: one per media record;
- decoded pixels: coded width times coded height for each accepted video access unit declared to
  contain a picture, or each accepted raster frame; and
- input events: one per ordinary key, motion, button, or axis event.

A presenter MAY enforce a stricter producer-declared per-track rate returned by `TRACK_READY`.
Exceeding a rate returns or emits `RATE_LIMITED`; repeated or material violation loses only the
track or input grant unless policy requires session revocation.

The token-bucket capacity permits a bounded burst but no continuous overuse. Saturating arithmetic
is forbidden. Counter overflow is fatal `LIMIT_EXCEEDED`.

### 5.3 Aggregate memory

The presenter separately accounts:

- bytes admitted under channel maxima but not yet released;
- decoder-owned packet buffers;
- decoded frames and GPU resources;
- raster composition bases;
- still-image cache entries;
- suspension posters;
- control and observation queues; and
- pending parser bodies.

Moving bytes between categories does not make them uncharged. A flow update is issued only after
the corresponding in-flight capacity is reusable or ownership has moved into another already
reserved bounded category.

Recommended root defaults are informative:

| Resource | Default |
|---|---:|
| Concurrent sessions | 16 |
| Concurrent connections | 64 |
| Surfaces | 64 |
| Tracks | 128 |
| Nodes | 256 |
| Active anchors | 256 |
| Seen anchor IDs | 4,096 per context |
| Contexts | 32 |
| Child leases | 32 |
| Suspended children | 8 |
| Decoder instances | 16 |
| Control body | 1 MiB |
| Status reply | 64 KiB |
| Media body hard limit | 64 MiB |
| Pending requests | 256 |
| Registered waits | 64 |
| Idempotency entries | 256 |
| Observation entries | 256 |
| Maximum disconnect grace | 30 seconds |

Deployments lower these values according to memory, GPU, browser, and platform constraints.

## 6. Retry-safe session leases

A controller creates the secret. The presenter receives only a verifier during lease creation, so
a lost reply can never lose the only copy of a credential.

### 6.1 Activation material

The controller generates:

```text
activation_secret = CSPRNG(32 bytes)

activation_verifier = SHA-256(
    "VIVID-LEASE-1" ||
    lease_id_be64 ||
    activation_secret
)
```

The controller delivers `activation_secret` to the intended child through protected owner-only IPC,
a protected file descriptor, or an authenticated encrypted channel. It never places it in an
argument, URL, log, terminal stream, browser storage available to unrelated origins, or status
record.

### 6.2 `CREATE_SESSION_LEASE`

The request is identified by the owning context and lease ID:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Owning context ID |
| 1 | uint | Nonzero lease ID, unique within context |
| 2 | bytes(32) | Activation verifier |
| 3 | uint | Activation timeout in microseconds |
| 4 | uint | Requested disconnect grace in microseconds |
| 5 | uint | Cleanup policy |
| 6 | array(text) | Permitted profiles, sorted and unique |
| 7 | map | Requested resource contract |
| 8 | bytes, optional | Client public key for deployment-specific proof of possession |

Cleanup policy is immediate on all disconnects (`0`) or suspend on unclean transport loss (`1`).
`GOODBYE`, explicit revoke, parent revoke, and authentication failure always clean up immediately.

The activation timeout is nonzero and at most 60 seconds. The effective disconnect grace is the
minimum of request, context key 23, parent lifetime, and presenter policy. Grace may be zero.
Permitted profiles form a closed set and include every prerequisite.

The child contract is reserved from the context exactly as for a child context. Success returns
`SESSION_LEASE_READY`:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Context ID |
| 1 | uint | Lease ID |
| 2 | uint | Lease state: issued (`1`) |
| 3 | uint | Effective activation timeout |
| 4 | uint | Effective disconnect grace |
| 5 | uint | Effective cleanup policy |
| 6 | array(text) | Effective permitted profiles |
| 7 | map | Effective reserved resource contract |
| 8 | uint | Lease revision |

There is no secret in this reply. Exact idempotency retry returns the same non-secret result.

### 6.3 Lease authentication request

For initial lease activation, `HELLO` key 6 is:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Authentication kind: lease activation (`1`) |
| 1 | uint | Issuing context ID |
| 2 | uint | Lease ID |
| 3 | bytes(32) | Activation secret |
| 4 | bytes(16) | Activation attempt ID |
| 5 | bytes, optional | Proof of possession for lease key 8 |

The presenter computes the verifier and compares it to the stored value in constant time. It
validates preface, complete `HELLO`, peer policy, profile closure, lease lifetime, and resource
availability before changing lease state.

### 6.4 Activation retry state machine

Lease states are:

```text
ISSUED
  -> RESERVED
  -> ACTIVE
  -> SUSPENDED
  -> CLOSED | REVOKED | EXPIRED
```

The transition rules are:

1. A valid first activation atomically changes `ISSUED` to `RESERVED`, binds the attempt ID and
   client nonce, creates the logical session ID and server nonce, and stores the non-secret
   `WELCOME` outcome.
2. Concurrent attempts can perform only bounded pre-authentication work. At most one changes the
   state. Losers receive `AUTH_FAILED`.
3. An exact retry with the same attempt ID, client nonce, and `HELLO` bytes while `RESERVED`
   returns the same logical session ID, server nonce, and `WELCOME`.
4. Committing `WELCOME` to the transport changes `RESERVED` to `ACTIVE`.
5. If that transport is confirmed closed before any post-`HELLO` record was admitted, an exact
   retry may bind a replacement transport and receive the same `WELCOME`. The presenter first
   closes the old transport if it can still write.
6. After any post-`HELLO` record is admitted, the activation secret and attempt ID can no longer
   open a transport. Recovery uses the resume proof.
7. Reservation and activation retry state expire no later than the activation deadline. Expiry
   destroys the logical session and reserved contract.

This makes a lost `WELCOME` retryable without creating a second session and without a
server-generated secret-bearing result.

## 7. Suspension and bounded resumption

### 7.1 Clean close versus unclean loss

These events close immediately:

- valid `GOODBYE`;
- explicit `REVOKE_SESSION_LEASE`;
- parent context revocation or expiry;
- authentication or integrity failure;
- presenter shutdown that cannot retain protected state; and
- cleanup policy zero.

An unclean control EOF or transport failure under cleanup policy one:

1. atomically marks the lease and logical session `SUSPENDED`;
2. advances lease and session revisions;
3. revokes input and releases every held key/button immediately;
4. closes interactive and track transports;
5. discards media ingress, decoder queues, audio-device queues, and pending non-idempotent work;
6. retains surface/track metadata, scene nodes, policies, descriptors, active-slot mapping,
   idempotency results, and policy-permitted bounded posters;
7. marks every track channel detached and requiring a new generation and recovery unit;
8. starts the effective grace deadline; and
9. keeps the complete lease resource contract reserved and all retained bytes charged.

No control mutation is applied while suspended. Grace expiry performs final owner-scoped cleanup.
Input never survives or automatically resumes across suspension.

### 7.2 Resume request

For resumption, `HELLO` key 6 is:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Authentication kind: resume (`2`) |
| 1 | uint | Issuing context ID |
| 2 | uint | Lease ID |
| 3 | uint | Suspended session ID |
| 4 | uint | Current resume generation |
| 5 | bytes(16) | Resume attempt ID |
| 6 | bytes(32) | Resume proof |

Let `hello_authless` omit auth subkey 6. The proof is:

```text
HMAC-SHA256(
    prior_session_resume_key,
    "VIVID-RESUME-HELLO-1" ||
    preface ||
    lease_id_be64 ||
    session_id_be64 ||
    resume_generation_be64 ||
    resume_attempt_id ||
    SHA-256(hello_authless)
)
```

The lease must be suspended, unexpired, and at the named generation. The profile offer and target
profile must exactly equal the original session.

On acceptance:

- resume generation increments by one;
- the new `HELLO` client nonce and new `WELCOME` server nonce derive fresh channel, resume, and
  anchor keys using the prior resume key as `session_secret`;
- the old keys are erased after the new `WELCOME` confirmation is committed;
- lease state becomes active;
- `WELCOME` key 13 is resumed (`1`);
- `WELCOME` reports current revisions and input disabled.

An exact retry of a resume attempt has the same stored-outcome behavior as activation. Competing
resume attempts cannot both advance the generation.

### 7.3 Reconciliation

Resume does not replay media or arbitrary control records.

The producer:

1. treats every old lane and channel as closed;
2. treats input as revoked;
3. obtains `SESSION_STATUS` pages at one session revision;
4. compares context, surface, track, active-slot, and scene revisions;
5. uses idempotency results for requests with provable cached outcomes;
6. queries objects to resolve `UNKNOWN_OUTCOME`;
7. advances channel generation for each track it will continue;
8. sends a fresh key unit or full raster/image body as required; and
9. requests a fresh input epoch only after the active surface generation and presentation
   milestone are current.

The presenter never claims that retained poster pixels prove a live decoder, current OS desktop
generation, or valid input grant.

## 8. Lease mutation and events

`REVOKE_SESSION_LEASE` carries context and lease IDs. Success is synchronous logical cleanup and
`OK`.

`SESSION_LEASE_CHANGED` is actionable and carries:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Context ID |
| 1 | uint | Lease ID |
| 2 | uint | State |
| 3 | uint | Lease revision |
| 4 | uint | Resume generation |
| 5 | uint | Reason mask |
| 6 | uint | Remaining grace in microseconds, meaningful only when suspended |

Reason bits are activation (`0`), clean close (`1`), unclean loss (`2`), resumed (`3`), explicit
revoke (`4`), parent cleanup (`5`), activation expiry (`6`), grace expiry (`7`), policy (`8`), and
resource violation (`9`).

Events never include an activation verifier, secret, transcript proof, channel key, resume key,
anchor key, or their hashes.

## 9. Gateway and relay requirements

A transparent gateway:

- forwards the exact Vivid byte stream, including `HELLO` and `WELCOME`;
- does not possess authority to rewrite authentication;
- does not merge principals or resource accounting; and
- either provides an end-to-end transport to the authenticated presenter or does not call itself
  transparent.

A terminating gateway:

- authenticates its inbound principal under its binding;
- enforces an inbound finite contract;
- originates a new Vivid session using separately provisioned authority;
- allocates independent session/context/surface/track/node IDs and revisions;
- derives independent channel, resume, and anchor keys;
- maps causation only through non-secret internal route IDs;
- intersects profiles, resource contracts, and capture policy; and
- performs owner-scoped cleanup on both sides.

A root-token-shaped zero placeholder, token substitution, "transparent except for authentication,"
or reuse of an inbound activation secret on the outbound hop is forbidden.

A broker may mint an out-of-band browser admission value only for the browser-to-gateway binding.
It is not a Vivid root secret, context, lease, or channel credential. The gateway terminates it and
originates a normal Vivid session.

## 10. Secret handling and operational requirements

Root, activation, resume, session-channel, and anchor secrets:

- reside only in protected process memory or an OS credential facility;
- are removed from environments passed to unrelated children;
- are never command arguments, URLs, cookies available to unrelated paths, diagnostics, traces,
  crash annotations, metrics labels, or serialization;
- are erased promptly after their lifetime;
- are not combined into a loggable correlation value; and
- are not hashed for logging.

Session and object IDs may be logged under deployment privacy policy, but implementations SHOULD
avoid combining browser identity, route identity, session ID, and track ID into a globally
correlatable value.

Rate-limit invalid prefaces, authentication attempts, lease activations, resume attempts, lane
opens, and channel opens before expensive cryptography or allocation. Rate limiting never changes
the constant-time comparison rule for a fully parsed candidate.

## 11. Required security and isolation tests

Conformance suites include:

- race at least two valid activations; exactly one logical session becomes active;
- drop `SESSION_LEASE_READY`; exact retry returns the same non-secret outcome;
- drop `WELCOME`; exact activation retry receives the same session ID and server nonce;
- attempt activation reuse after application traffic; it fails;
- race two resume attempts; exactly one advances generation;
- suspend, resume, revoke, and expire one of two owners reusing every local ID;
- prove retained suspended state remains charged and cannot be oversubscribed by a sibling;
- exceed each count, pixel, byte, bit-rate, record-rate, decoder, audio, input, and connection
  bound before unbounded allocation or work;
- scan logs, errors, status, environment inheritance, argv, URLs, traces, and crash text for secret
  sentinels;
- prove a transparent gateway changes no byte;
- prove a terminating gateway shares no authority or identifier domain; and
- fuzz canonical transcript hashing, malformed maps, wrong lengths, and cross-kind proof replay.
