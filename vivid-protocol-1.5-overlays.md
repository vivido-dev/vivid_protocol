# Vivid 1.5 pane overlays

This optional extension does not change the 1.5 preface. A presenter MUST NOT accept an
overlay profile unless it implements its rendering, resource, and interaction requirements.
In particular, terminating gateways without these facilities MUST reject a required profile
and omit an optional profile. Converting a vector track to a raster or discarding input is
not an implementation of this extension.

## Profiles and ownership

`vector-scene-v1` requires `live-media-v1`. `terminal-overlay-v1` requires
`terminal-surface-v1` and `vector-scene-v1`. `overlay-input-v1` requires
`terminal-overlay-v1`. Interactive window helpers require all three profiles.

Window identities are existing surface identities. Assets and scene submissions belong to
the complete presenter/session/context/surface/track/channel-generation identity. Local
numeric IDs alone MUST NOT index resources shared by producers. A child window MUST have the
same authenticated session as its parent. A window is not an OS window or a terminal poster.

All geometry uses signed Q32.32 logical pixels, represented by CBOR integers, with absolute
value at most 1,000,000. Window geometry is relative to the owning pane's viewport; drawing
and hit geometry are window-local. Scrolling does not change these coordinates. Existing
grid and anchor geometry keep their existing meanings. Logical-to-physical scale is applied
only at presentation and pointer-coordinate conversion.

## Limits

WELCOME extension key 15 is a vector limits map when `vector-scene-v1` is accepted. Its
unsigned fields are:

| Key | Limit | Hard ceiling |
| --- | --- | --- |
| 0 | Encoded scene bytes, excluding frame header | 2 MiB |
| 1 | Commands per scene | 4096 |
| 2 | Segments per path | 4096 |
| 3 | Total path segments per scene | 65536 |
| 4 | Save depth and active clip depth | 32 each |
| 5 | Total UTF-8 text bytes per scene | 65536 |
| 6 | Stops per gradient | 64 |
| 7 | Bytes per decoded image | 16 MiB |
| 8 | Retained images per channel | 256 |
| 9 | Retained image bytes per channel | 64 MiB |
| 10 | Windows per session | 32 |
| 11 | Pending input events per session | 256 |

Values MUST be nonzero and no greater than their hard ceiling. The presenter may negotiate
smaller values. Existing control/media body limits, context resource limits, rate claims, and
cumulative channel credit also apply. Limits are validated before allocation or mutation.
At most one replacement scene may be pending per window. Replacing that pending submission
resolves the older submission as superseded; it is not silently reported as presented.

## Vector tracks and bodies

Track kind 5 and visual slot 5 are `vector-scene`. A vector track is immutable, downlink,
live, and bulk. Its kind configuration is a map containing key 0 logical width, key 1
logical height, and key 2 maximum encoded scene bytes. Width and height are positive integers
no greater than 16384. Resizing a window changes its viewport clip rather than rewriting a
track configuration. Track generations, readiness, activation and ordered EOS retain their
existing meanings.

`VECTOR_FRAME` (0x800d) contains a big-endian nonzero u32 media epoch, a big-endian nonzero u64
scene revision, then one deterministic CBOR display list. Revisions obey the track's
monotonic media-ID rules. The scene replaces the previous scene atomically. Drawing, hit
testing and the revision used by input events MUST become visible together. Decoding or
compilation failure MUST NOT publish a prefix of the list.

`VECTOR_ASSET` (0x800e) contains a big-endian nonzero u64 asset ID, u32 width, u32 height,
then exactly `width * height * 4` straight-alpha sRGB RGBA8 bytes. Dimensions and multiplication
are checked before allocating. An asset ID is immutable and cannot be redefined within a
channel generation. Images may be referenced by multiple scenes without retransmission.
Destroying or losing the channel releases its asset namespace. A scene retains references
to the images it uses. Transport processing returns credit when it releases record storage;
retained image storage remains charged separately and cannot be evaded by accumulating credit.

The display list is an array of command arrays. A point is `[x, y]`; a rectangle is
`[point, width, height]`; a transform is `[a, b, c, d, e, f]` using the usual affine mapping.
Colors are unsigned RGBA8 with red in the most significant byte. Opacity is an unsigned
integer from 0 to 65535. These integers are not IEEE floating-point values.

A path is `[even_odd_boolean, segments]`. Segment arrays are `[0, point]` (move),
`[1, point]` (line), `[2, control, end]` (quadratic), `[3, control1, control2, end]` (cubic),
or `[4]` (close). A contour starts with move; drawing segments require an open contour.

A brush is `[0, color]`, `[1, start, end, stops]`, or `[2, center, radius, stops]` for solid,
linear, or radial paint. Stops are `[offset, color]`, with nondecreasing unsigned 0–65535
offsets. Gradients have at least two stops. Linear endpoints differ; radial radius is positive.

| Tag | Remaining command fields |
| --- | --- |
| 0 | path, brush: fill |
| 1 | path, brush, positive width: stroke |
| 2 | none: save transform, opacity and clip state |
| 3 | none: restore saved state |
| 4 | affine transform: concatenate |
| 5 | path: intersect current clip |
| 6 | opacity: set drawing opacity |
| 7 | text, origin, size, family, weight, italic, color, optional maximum width |
| 8 | asset ID, rectangle, opacity |
| 9 | nonzero unique application region ID, path, role |

Text is UTF-8. Font family is at most 256 bytes, weight is 1–1000, and size and optional
maximum width are positive. Empty family selects the host default. The host shapes text
and performs script/font fallback. Custom font bytes are not supported. Saves and restores
must balance; clips installed outside a save remain active to the end of the scene.
Singular or unbounded accumulated transforms and transformed geometry MUST be rejected.

Hit roles are 0 ordinary input, 1 host drag, 2 transparent, or 16 plus a nonzero resize-edge
mask (left=1, top=2, right=4, bottom=8). Later regions take precedence. Regions use the
same transform and active clips as drawing. If no regions are supplied the window rectangle
is the default hit area; otherwise unmatched areas are input-transparent.

## Window control

Window control records use normal authenticated control envelopes and surface object IDs.
The context field and surface field MUST agree with the authenticated owner and object ID.
Control replies preserve correlation; no control body contains a display list or image.

`SET_OVERLAY_WINDOW` (0x7020) carries context (0), surface (1), surface generation (2),
expected window revision (3), rectangle (4), mode (5: floating=0, popup=1, modal=2), visibility
(6), optional parent `[context, surface]` (7), and minimum `[width,height]` (8). Revision zero
creates a window for an existing surface. Nonzero revisions update it. Parent and mode are
immutable. Successful mutations advance a checked window revision and reply with
`OVERLAY_WINDOW_READY` (0x7021), containing the authoritative window fields.

`OVERLAY_ACTION` (0x7022) carries context (0), surface (1), generation (2), expected revision
(3), and action (4: close=0, focus=1, raise=2, lower=3, center=4). Close is explicit and
releases input state. Stacking requests cannot bypass an active modal. `QUERY_OVERLAY`
(0x7023) carries context (0) and surface (1); `OVERLAY_STATUS` (0x7024) returns the window
fields plus viewport logical extent (9), positive scale numerator/denominator (10), focus
(11), and last published scene revision (12). Target changes notify producers of authoritative
logical extent and scale; old target generations cannot authorize new placement mutations.

The extent is `[width, height]` in Q32.32; scale is `[numerator, denominator]`, both nonzero
u32 integers. A successful READY has keys 0–8 with key 3 set to the resulting nonzero window
revision. STATUS has keys 0–12 (parent key 7 remains optional). Requests and replies reject
duplicate or unknown keys, invalid enum values, zero object identities/generations, and geometry
outside the scalar limits. A parent is resolved in the authenticated session, never a
producer-specified session. Self-parenting and cross-owner parenting are invalid.

## Input and lifecycle

Input uses an independently authenticated interactive lane. It MUST remain responsive when
bulk decoding or rendering is saturated. `OVERLAY_INPUT_EVENT` (0x7030) contains context (0),
surface (1), surface generation (2), scene revision (3), event type (4), and typed payload (5).
Pointer coordinates are window-local logical pixels. Events include pointer motion/buttons,
wheel, physical keys, committed text, IME preedit/selection, focus, geometry (including final
settled geometry), dismissal, and cancellation. Connection loss is additionally reported by
the producer binding when its transport closes.

Key 3 identifies the **published scene revision**, independently of the window geometry revision.
Revision zero is permitted for lifecycle notifications before the first scene is published.
Pointer targeting MUST use the atomically published drawing/hit state. Coalescing MUST NOT cross
a scene-revision boundary; dispatch for older queued events remains associated with its old scene.

| Event type | Key 5 payload |
| --- | --- |
| 0 focus | boolean |
| 1 pointer | `[point, region_id, button_or_null, modifiers]` |
| 2 wheel | `[point, delta_x, delta_y, modifiers]` |
| 3 key | `[physical_key, down, repeat, modifiers]` |
| 4 committed text | UTF-8 text |
| 5 IME | `[preedit_text, selection_or_null]` |
| 6 geometry | `[rectangle, settled_boolean]` |
| 7 dismissed | reason: Escape=0, outside press=1, explicit close=2, owner loss=3, parent close=4 |
| 8 cancel | null |

Points, rectangles, and wheel deltas use the drawing codec's Q32.32 geometry. A pointer button is
`[u16_button, down_boolean]` or null for motion. Region IDs are u64 (zero denotes the default
rectangular hit region); physical keys and modifier masks are u32. An IME selection is
`[start_byte, end_byte]` with ordered u32 UTF-8 byte offsets at character boundaries inside the
preedit string, or null. Language/platform adapters convert their native offset conventions.
Committed text and preedit text are limited to 4096 UTF-8 bytes each. Unknown event types, extra
array fields, invalid booleans, and narrowing overflow MUST be rejected before dispatch.

`OVERLAY_INPUT_CAPTURE` (0x7031) requests or releases capture for a currently eligible focused
window. `OVERLAY_INPUT_RENEW` (0x7032) renews the bounded input-lane lease. Host shortcuts are
handled before application input. No overlay can intercept another pane's input.

CAPTURE uses context (0), surface (1), surface generation (2), nonzero scene revision (3), and
capture boolean (4). The header object ID is the surface. False releases capture; true requires
the currently eligible focused window and its current scene revision. The presenter uses its
last observed pointer position, not a producer-supplied position, to establish capture.
RENEW uses lane generation (0) and watchdog duration in microseconds (1), with header object zero.
The generation MUST match the authenticated lane and the watchdog MUST be in 250,000–5,000,000
microseconds. CAPTURE and RENEW are correlated requests replied to with OK or ERROR. Lease expiry
uses presenter monotonic time; delayed renewals cannot reactivate a revoked lane generation.

Floating windows receive input in their hit regions and keys while focused. An outside press
dismisses the foremost popup and consumes both the press and matching release. Escape dismisses
popups and modals by default. The top modal blocks ordinary terminal input in its pane; only
that modal and its descendant popups are eligible. Another session cannot steal its focus.
Closing a window restores the previous eligible focus target, including the terminal.

Host drag/resize regions update geometry without rebuilding content. Motion notifications may
be coalesced, but final geometry and key/button transitions MUST NOT be dropped. Queue overflow,
lease expiry, revocation, hide, close, or lane/session loss clears captures, focus and held-input
state. Overflow revokes the affected session's overlay input and modal blockade. Cleanup never
revokes another owner's windows even when their local IDs match. Child windows become
ineligible when their parent is hidden, and close when their parent closes.

## Composition

The pane is composed in this order: layer-0 media, cell backgrounds, layer-1 media, terminal
glyphs/cursor/selection, layer-2 media and viewport overlays, then trusted host chrome. Every
application overlay is clipped to its pane. Overlay animation MUST NOT require reshaping
unchanged terminal text or uploading unchanged assets. Compilation and text shaping happen
off the UI event loop; moving a cached scene changes its placement transform.
