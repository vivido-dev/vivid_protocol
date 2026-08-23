# Vivid 1.5 File Drop

**Status:** jointly normative Vivid Protocol 1.5 specification.
**Profile:** `file-drop-v1`.

## 1. Scope and invariants

`file-drop-v1` copies one regular file selected by a local operating-system drag gesture from the
presenter to a directory selected entirely by the producer. It is not a generic file-transfer or
filesystem-browsing facility. It does not define directories, symbolic links, MIME-only data,
move, drag-out, native application drag injection, metadata preservation, or local path
references.

The presenter MUST NOT create an offer without a current OS file-drop gesture and an effective
binding. A suggested name is inert. No record contains a source path, URI, machine identifier,
timestamp, mode, or source hash. A destination path appears in exactly one place: `FILE_RESULT`
key 5, only under `file-drop-path-v1` and only on a committed or already-committed result, as
defined in section 8. File bytes never use a terminal PTY.

One OS `DroppedFile` event creates at most one logical drop. Implementations MUST cap one binding
at 16 pending offers and four active transfers; the recommended limits are four and one. Source
handles, temporary files, results, and recovery state remain scoped by the complete authenticated
session/context/surface/drop/transfer tuple.

## 2. Binding

The producer requires operation-class bit 6. `SET_FILE_DROP_BINDING` is correlated and uses the
surface ID as record object ID; zero selects a target-wide binding. Its payload is:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Nonzero producer epoch, monotonically increasing for this binding identity |
| 1 | uint | Nonzero owning context |
| 2 | uint | Owned surface, or zero for a target-wide binding |
| 3 | uint | Surface generation, or zero for a target-wide binding |
| 4 | uint | Destination: disabled `0`, shell cwd `1`, desktop folder `2` |
| 5 | uint | Maximum file bytes; nonzero while enabled |
| 6 | uint | Maximum pending offers, `1..=16` |
| 7 | uint | Maximum active transfers, `1..=4` and no greater than key 6 |
| 8 | uint | Maximum receive record body |
| 9 | uint | Acceptance timeout in microseconds |
| 10 | uint | Transfer idle timeout in microseconds |

Timeouts are finite and within 1 through 300 seconds. Disable preserves keys 1 through 3 to name
the complete binding being removed and has keys 4 through 10 zero. Epoch ordering and replay are
scoped to that binding identity: a lower epoch is `BAD_STATE`; exact same-epoch bytes replay the
prior result; changed bytes at the same epoch are `BAD_MESSAGE`.

`FILE_DROP_BOUND` copies the request ID and returns keys 0 epoch, 1 presenter grant generation
(zero only when disabled), 2 context, 3 surface, 4 surface generation, 5 state (disabled `0`, enabled `1`, denied
`2`, standby `3`), 6 destination, 7 maximum file bytes, 8 pending offers, 9 active transfers,
10 record body, 11 acceptance timeout, 12 idle timeout, and 13 bounded reason. Every effective
limit is no greater than requested. The grant generation advances on each effective change.

An exact-surface binding wins hit testing over a target-wide binding. A presenter MAY retain a
bounded stack of target-wide bindings; only the top is enabled and the others are standby. A
superseded binding MAY become enabled again when the binding above it ends. Focus loss alone does
not revoke a grant or accepted transfer. Owner revocation, surface/target generation replacement,
or logical-session cleanup does.

## 3. Offer and acceptance

`FILE_DROP_OFFER` is an unsolicited request-ID-zero event whose object ID is the drop ID. Keys 0
through 5 are producer epoch, grant generation, context, surface, surface generation, and nonzero
drop ID. Key 6 is an inert UTF-8 suggested basename of at most 255 bytes. Key 7 is exact `u64`
file length, including zero for an empty file.

Before offering, the presenter opens the selected source without following a final symbolic link,
verifies that the opened object is regular, obtains the length from the handle, and retains that
same handle. It never reopens by path. Names containing separators, control characters, Windows
reserved punctuation or device basenames, trailing dots/spaces, or `.`/`..` are invalid; a
presenter may replace an invalid OS basename with `dropped-file`.

`ACCEPT_FILE_DROP` is correlated and repeats offer keys 0 through 5. Keys 6 through 10 are a
nonzero transfer ID, initial generation exactly one, receive record-body limit, initial maximum
cumulative record-body bytes, and initial maximum record count. The two maxima may both be zero;
otherwise they admit at least one maximum-size legal record. `FILE_DROP_ACCEPTED` replies with
drop ID, transfer ID, generation, and a finite file-transfer-open timeout.

`CANCEL_FILE_DROP` is correlated, repeats offer keys 0 through 5, and adds reason key 6. Success
returns `OK`. `FILE_DROP_CANCELLED` is the request-ID-zero presenter event with the same payload.
Cancellation closes the source handle and removes uncommitted receiver state; it never deletes an
already committed destination.

## 4. Authenticated file-transfer connection

The producer opens connection kind `FILE_TRANSFER` through the bulk endpoint. The first record is
`FILE_TRANSFER_OPEN`, sequence one. It is a raw deterministic CBOR map:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Session ID |
| 1 | uint | Context ID |
| 2 | uint | Surface ID, possibly zero |
| 3 | uint | Drop ID |
| 4 | uint | Transfer ID |
| 5 | uint | Transfer generation |
| 6 | uint | Receiver committed resume offset |
| 7 | uint | Receiver maximum record body |
| 8 | uint | Initial absolute cumulative body-byte maximum |
| 9 | uint | Initial absolute cumulative record maximum |
| 10 | bytes(16) | Client nonce |
| 11 | bytes(16) | Authentication tag |
| 12 | uint | Producer binding epoch |
| 13 | uint | Presenter grant generation |
| 14 | uint | Surface generation, or zero for a target-wide binding |

The tag is the first 16 bytes of:

```text
HMAC-SHA256(
  session_channel_key,
  "VIVID-FILE-TRANSFER-1" || session_id_be64 || context_id_be64 ||
  surface_id_be64 || producer_epoch_be64 || grant_generation_be64 ||
  surface_generation_be64 || drop_id_be64 || transfer_id_be64 ||
  generation_be64 || resume_offset_be64 || maximum_record_body_be32 ||
  maximum_body_bytes_be64 || maximum_records_be64 || client_nonce
)
```

The presenter compares it in constant time before allocation and confirms the exact live accepted
tuple. `FILE_TRANSFER_ACCEPTED` returns transfer ID, generation, and resume offset before any file
data. Authentication failure silently closes or returns the existing typed authentication error
without logging the tag or its digest.

## 5. Data, flow, finish, and result

`FILE_DATA` travels presenter to producer. Its body has a 16-byte prefix followed by opaque bytes:

| Offset | Size | Field |
|---:|---:|---|
| 0 | 8 | Exact file offset, big endian |
| 8 | 4 | Nonzero payload length, big endian |
| 12 | 4 | Reserved zero |

The prefix length must equal the remaining body. Offsets are exactly sequential from the accepted
resume offset. A generation never accepts an overlapping, skipped, empty, or over-length body.
The logical offset cannot exceed the offered length.

Absolute flow maxima count the complete `FILE_DATA` record body and one record. `MAX_FILE_DATA`
travels producer to presenter and contains transfer ID, generation, new cumulative body-byte
maximum, and new cumulative record maximum. Lower reordered values are harmless. Overflow and
traffic beyond either maximum are `FLOW_CONTROL`. Flow counters reset for a new generation, but
the logical file length and sustained policy accounting do not.

After sending exactly the offered length, the presenter sends `FILE_FINISH` containing transfer
ID, generation, final length, and SHA-256 computed over the bytes read from the retained handle.
The producer compares its independently computed length and hash before commit.

`FILE_RESULT` travels producer to presenter and contains transfer ID, generation, result
(committed `0`, rejected `1`, cancelled `2`, hash mismatch `3`, I/O error `4`, already committed
`5`), committed length, and final basename. Only successful results carry a nonempty basename.
Under `file-drop-path-v1` a committed or already-committed result MAY additionally carry key 5,
the UTF-8 absolute path of the committed file on the producer's host, as defined in section 8.
Every other result omits it.
`FILE_TRANSFER_ABORT` can travel either way and contains transfer ID, generation, bounded reason,
and final offset. Diagnostics never contain a path or digest.

The receiver creates a random `0600` temporary regular file relative to an already-open destination
directory, never follows a link, and commits with an atomic no-replace operation. A collision is
resolved with `name (1).ext`, `name (2).ext`, and so forth. Existing entries are never overwritten.
The temporary file is removed after any failed verification or terminal failure.

## 6. Recovery and status

`ADVANCE_FILE_TRANSFER` is correlated with object ID equal to transfer ID. It carries context,
surface, drop, transfer, expected generation, exactly-next generation, committed offset, and new
initial body/record credit. `FILE_TRANSFER_ADVANCED` confirms transfer ID, generation, offset, and
open timeout. Once advanced, records on older generations are rejected. A committed transfer
returns its cached terminal result and never creates a second file.

`QUERY_FILE_DROP` is correlated and contains the drop ID. `FILE_DROP_STATUS` returns drop ID,
state (offered `1`, accepted `2`, transferring `3`, committed `4`, cancelled `5`, failed `6`),
transfer ID, generation, committed offset, optional result, and final basename. Status never
returns a directory or absolute path. This holds under `file-drop-path-v1` as well: the committed
path travels only on the authenticated file-transfer connection that produced it, and the replayed
already-committed result carries the identical path.

After sending `FILE_RESULT`, the receiver queries status on the control connection. If the result
was lost, it advances to the next generation at the full committed offset, accepts the presenter's
identical finish, and sends `already committed`; it never creates or renames a second file.

Terminal outcomes remain in a bounded cache for no more than 60 seconds and no longer than the
logical session or negotiated suspension grace. A connection loss is recoverable while control
and the logical drop remain live. Logical-session cleanup closes source handles and deletes only
that owner's uncommitted state.

## 7. Scheduling, consent, gateways, and conformance

File-transfer connections use the bulk scheduler. Zero credit or slow disk I/O MUST NOT block
control, input revocation/reset, audio flow, rendering, or unrelated tracks.

Consent is presenter policy, not producer authority. A presenter exposes a trusted indication
that a drop will copy bytes to a remote shell or desktop and MUST NOT render producer text as a
trusted destination identity. A local deployment may require first-use consent for each complete
logical binding. When no effective binding exists, terminal filename-paste behavior is outside
this profile and remains unchanged.

A byte-transparent carrier may relay kind 3 only when its admission layer permits that kind. A
terminating gateway accepts an outer drop and independently re-originates an inner drop with new
IDs, generations, HMAC, credit, and consent. It never forwards authentication tags, and it never
introduces or forwards path references: a terminating gateway MUST NOT negotiate
`file-drop-path-v1` on either session, because an inner destination path names a filesystem the
outer presenter does not have. A component that does not implement this behavior omits `file-drop-v1` and
rejects kind 3.

Conformance tests cover canonical decoding, checked lengths and offsets before allocation,
malformed prefixes, zero credit, hash mismatch, collision/symlink races, lost replies, generation
advance, result replay, cancellation, timeout, and two owners reusing every local numeric ID.

## 8. `file-drop-path-v1`

Prerequisite `file-drop-v1`. It adds one optional key to `FILE_RESULT` and no records, bindings,
authority, operation-class bits, or registry assignments.

When negotiated, a producer whose destination is a real filesystem directory MAY add key 5 to
`FILE_RESULT`: the UTF-8 absolute path of the committed file on the producer's host. It is present
only with result committed (`0`) or already committed (`5`), and a replayed already-committed
result carries the byte-identical path. It is absent from every other result, from
`FILE_DROP_STATUS`, and from every diagnostic and log.

Key 5 is at most 4096 bytes, begins with `/`, contains no control character, contains no `..`
component, and its final `/`-separated component is byte-identical to key 4. A decoder that finds
key 5 without all of those properties rejects the record rather than repairing it. A producer that
has not negotiated the profile MUST omit key 5, and a presenter that has not negotiated it rejects
key 5 as an unknown key — which is what makes the profile safe across version skew in both
directions.

Key 5 is producer-supplied data, never a presenter-trusted identity. A presenter MUST revalidate
every property above before use and MUST NOT treat the value as evidence about the producer's
host. A presenter that types the path into a terminal types it as ordinary text for a binding the
presenter itself created, never as a command.
