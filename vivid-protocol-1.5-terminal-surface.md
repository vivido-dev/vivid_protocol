# Vivid Protocol 1.5 Terminal Surface

This file is a normative part of the
[Vivid Protocol 1.5 specification](vivid-protocol-1.5-spec.md) and defines
`terminal-surface-v1`.

## 1. Scope

`terminal-surface-v1` makes terminal concepts a presentation-target profile. Grid metrics, text
layers, cell geometry, scrollback, and anchors are absent from core sessions that select a desktop
or canvas target.

A terminal target owns:

- a pixel viewport;
- grid columns and rows;
- cell width and height in pixels;
- a text plane with defined composition layers;
- a target generation; and
- a bounded authenticated anchor namespace for each authorized context.

Content surfaces remain stable protocol surfaces. A terminal scene node places a surface relative
to the terminal target; the surface itself is not a terminal cell or decoder.

## 2. Target descriptor and changes

For this profile, `WELCOME` key 5 contains:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Viewport width in pixels |
| 1 | uint | Viewport height in pixels |
| 2 | uint | Grid columns |
| 3 | uint | Grid rows |
| 4 | uint | Cell width in pixels |
| 5 | uint | Cell height in pixels |
| 6 | bool | Geometry is settled |
| 7 | uint | Anchor marker version; `3` |
| 8 | uint | Maximum active anchors in the authenticated context |

Dimensions and grid metrics are positive. Their products and conversions use checked arithmetic.

`TARGET_CHANGED` is actionable and carries the same keys plus:

| Key | Type | Meaning |
|---:|---|---|
| 9 | uint | New target generation |
| 10 | uint | Reason mask |

Reason bits are viewport size (`0`), grid size (`1`), cell metrics (`2`), scale/output change (`3`),
and target recreation (`4`).

The presenter emits at most one target change per compositor frame for a source of truth and
coalesces intermediate resize geometry. `settled = false` means a gesture or reconfiguration is
ongoing; `true` means the presenter regards it as final.

A stale scene commit receives the current `TARGET_CHANGED` followed by
`STALE_TARGET_GENERATION`; no scene mutation applies.

## 3. Terminal node geometry

For `CREATE_NODE` and `UPDATE_NODE`, geometry key 4 is:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Coordinate space: grid cell (`1`) or anchor cell (`2`) |
| 1 | int | X in signed 32.32 cells |
| 2 | int | Y in signed 32.32 cells |
| 3 | int | Width in positive signed 32.32 cells |
| 4 | int | Height in positive signed 32.32 cells |
| 5 | uint | Text layer |
| 6 | uint, conditional | Anchor-owning context ID |
| 7 | uint, conditional | Anchor ID |

Anchor keys 6 and 7 are required exactly when coordinate space is anchor cell.

Text layers are behind terminal background (`0`), between background and glyphs (`1`), and above
glyphs (`2`). A presenter MAY reject layer 0 or 2 by policy; it MUST NOT silently substitute
another layer. Z index from the core node schema orders nodes within a text layer.

The optional node clip map is:

| Key | Type | Meaning |
|---:|---|---|
| 0 | int | Clip X in 32.32 cells |
| 1 | int | Clip Y in 32.32 cells |
| 2 | int | Positive clip width |
| 3 | int | Positive clip height |

The clip follows the node coordinate space and anchor transform. It clips the fitted media quad
without refitting. The final draw region is the intersection of media quad, clip, and viewport.

For grid coordinates, origin `(0,0)` is the upper-left cell of the current viewport. Positive x is
right and positive y is down.

For anchor coordinates, origin `(0,0)` is the upper-left of the anchor's current semantic cell.
When an anchor scrolls, its nodes follow it. When it leaves the retained terminal model, the
presenter removes those nodes through the anchor lifecycle.

## 4. Authenticated anchor marker version 3

### 4.1 APC form

The Unix terminal marker is:

```text
ESC _ VIVID;3;A;<session-tag>;<context-id>;<anchor-id>;<auth> ESC \
```

Equivalent escaped form:

```text
\x1b_VIVID;3;A;<session-tag>;<context-id>;<anchor-id>;<auth>\x1b\\
```

Fields are:

- `<session-tag>`: the 16-byte session tag as exactly 22 unpadded base64url characters;
- `<context-id>`: exactly 16 hexadecimal digits encoding a nonzero `u64`;
- `<anchor-id>`: exactly 16 hexadecimal digits encoding a nonzero `u64`; and
- `<auth>`: exactly 22 unpadded base64url characters encoding 16 bytes.

Hexadecimal is case-insensitive. A canonical producer emits lowercase. The complete marker is
ASCII, has zero display width, does not move the cursor, and is at most 192 bytes.

### 4.2 ConPTY form

When the binding explicitly selects ConPTY scanning, the authenticated payload uses:

```text
VIVID;3;A;<session-tag>;<context-id>;<anchor-id>;<auth>;VIVID-END
```

The suffix is framing and is not authenticated. The complete envelope is at most 192 bytes.
Scanners are fragmentation-safe and preserve malformed or oversized candidates byte-for-byte.
Accepting both APC and ConPTY forms is a deployment migration behavior; a session selects one
emission form.

### 4.3 Authenticator

Let `context_id_be` and `anchor_id_be` be eight-byte big-endian values. Using the session
`anchor_key` from the security specification:

```text
auth_full = HMAC-SHA256(
    anchor_key,
    "VIVID-ANCHOR-3" ||
    session_tag ||
    context_id_be ||
    anchor_id_be
)

auth = auth_full[0..16]
```

The presenter compares `auth` in constant time.

The context must be within the authenticated principal's subtree and must grant anchor class bit 3.
Failure consumes the marker as invalid terminal content according to presenter policy but creates
no anchor and reveals no other context.

The session tag and IDs are not secret. They do not permit forgery without the session-derived
anchor key.

## 5. Anchor identity, replay, and lifecycle

The complete anchor identity is session, context, and anchor ID. A producer uses a CSPRNG for
anchor IDs and never reuses an ID in that context during the logical session, including across
suspension and resume.

The presenter maintains a bounded seen-ID set per context. A duplicate marker does not create,
move, or recreate an anchor.

A valid new marker:

1. verifies target session, context authority, ID freshness, and authenticator;
2. charges active-anchor and seen-ID resources;
3. creates the anchor at the current semantic terminal position; and
4. emits actionable `ANCHOR_READY` with context ID, anchor ID, cell column, cell row, and target
   generation.

When the semantic position is erased or evicted:

1. the presenter removes the anchor;
2. removes only nodes referencing that complete anchor identity;
3. advances scene and session revisions;
4. emits actionable `ANCHOR_GONE`; and
5. emits a coalescible `SCENE_CHANGED` observation when enabled.

Clearing the text plane invalidates all anchors for that target without touching unanchored nodes
or surfaces.

`QUERY_ANCHOR` carries context and anchor IDs. `ANCHOR_STATUS` reports unknown (`0`), ready (`1`),
gone (`2`), current cell, viewport intersection, and target generation. No query exposes another
context's anchor.

## 6. Suspension and post-disconnect poster behavior

Terminal text and anchors belong to the presenter target, not to a child decoder. During an
eligible lease suspension:

- the presenter's terminal continues normally;
- anchor identities may remain only while their semantic positions remain;
- active node relationships and policy-permitted bounded posters may remain;
- decoder, media queue, and input state do not remain; and
- all retained anchor/node/poster resources remain charged to the suspended lease.

If a retained anchor disappears during suspension, its nodes are removed normally. Resume status
reports the resulting scene revision; it never recreates an anchor.

After a clean `GOODBYE`, a terminal presenter MAY preserve the last rendered visual for an
authenticated anchored node as a bounded target-native poster. Before doing so it MUST:

- reject retention when the effective surface policy denies post-disconnect posters;
- copy only the selected visual output, placement, clip, composition, capture policy, and anchor
  relationship needed to reproduce the last presentation;
- release the logical session, authority, contexts, surface and track state, channels, decoder
  state, media queues, waits, and all other protocol resources; and
- enforce a finite target-wide retained-pixel limit.

The retained poster is not protocol state: it is absent from session, surface, track, scene, and
anchor queries and receives no protocol events or presentation acknowledgements. It follows its
complete authenticated anchor through scroll and reflow and is erased when that anchor is cleared,
evicted, or leaves its terminal screen. Grid-positioned nodes are never retained after disconnect.

An unclean root-session control loss retains no poster. Lease suspension uses the resumable rules
above rather than this clean-closure materialization.

## 7. Multiplexers and nested terminals

Anchors require:

- the correct endpoint and authority reaching the producer;
- marker bytes preserved exactly;
- the marker reaching the presenter that issued the session tag; and
- one unambiguous target terminal parser.

If tmux, screen, or another intermediary cannot prove these conditions, the producer MUST NOT emit
markers. It may use grid nodes.

A terminating nested presenter verifies inner markers and independently creates any outer anchor.
It never forwards an inner marker, tag, context ID, anchor ID, or authenticator as outer authority.

## 8. Terminal-profile conformance

A conforming terminal presenter:

1. makes grid fields conditional on this profile rather than core `WELCOME`;
2. validates fixed-point geometry and clips with checked arithmetic;
3. parses markers with a bounded fragmentation-safe scanner;
4. compares authenticators in constant time;
5. scopes anchor replay and cleanup by complete context identity;
6. keeps malformed media out of the terminal parser;
7. advances target generation for authoritative grid/viewport changes; and
8. proves that anchor loss for one of two contexts reusing the same anchor and node IDs leaves the
   other unchanged.
