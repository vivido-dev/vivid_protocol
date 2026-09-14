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

An overlay window's published scene revision also advances across track replacement and channel
generation changes. A replacement track MUST be primed with a revision greater than the window's
highest accepted revision (available through `QUERY_OVERLAY`). Accepted revisions advance across
all tracks belonging to a window, including tracks being primed. Activation MUST reject a stale
replacement before changing any slot binding. Re-activating the current binding is idempotent.

`VECTOR_ASSET` (0x800e) contains a big-endian nonzero u64 asset ID, u32 width, u32 height,
then exactly `width * height * 4` straight-alpha sRGB RGBA8 bytes. Dimensions and multiplication
are checked before allocating. An asset ID is immutable and cannot be redefined within a
channel generation. Upload IDs MUST strictly increase within a channel generation, including
after release; receivers need not retain an unbounded set of retired IDs. Images may be
referenced by multiple scenes without retransmission.
Destroying or losing the channel releases its asset namespace. A scene retains references
to the images it uses. Transport processing returns credit when it releases record storage;
retained image storage remains charged separately and cannot be evaded by accumulating credit.

`VECTOR_ASSET_RELEASE` (0x800f) contains exactly one big-endian nonzero u64 asset ID. It uses
the same authenticated bulk channel, object ID, ordered record sequence, cumulative credit,
and EOS ordering as uploads and frames. It removes the asset from future scene lookup without
advancing scene revisions or readiness. Unknown/already released IDs are invalid. Complete
pending, in-flight and displayed scenes retain their references; releasing the namespace or
retiring the track MUST NOT free their storage or stop charging their decoded resource budget.
The charge ends when the last reference is released. A later scene referencing a released ID
is rejected atomically, even if that display list was built before release.

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
(11), last presented scene revision (12), optional active binding
`[track_id, channel_generation, epoch, scene_revision]` (13), nonzero viewport revision (14),
and highest accepted scene revision (15). All identities/revisions are full-width unsigned
integers (epoch is u32); the binding inherits the reply's owner/context/surface/generation.
Absent binding means no active vector track. Accepted/active state does not imply presentation.
Target changes notify producers of authoritative
logical extent and scale; old target generations cannot authorize new placement mutations.

Successful `OVERLAY_ACTION` requests reply with an empty correlated `OK` using the addressed
surface object ID. Producers obtain the resulting window revision with `QUERY_OVERLAY` before
issuing another conditional mutation. Closing a window does not by itself destroy its semantic
surface; explicit surface destruction or owner cleanup releases the remaining surface resources.
A dismissed/closed overlay cannot be reopened on that surface; create a new surface/window to
avoid reusing its submission identities. Dismissal resolves unpresented submissions without
waiting for explicit surface destruction.

The extent is `[width, height]` in Q32.32; scale is `[numerator, denominator]`, both nonzero
u32 integers. A successful READY has keys 0–8 with key 3 set to the resulting nonzero window
revision. STATUS has keys 0–15 (parent key 7 and active binding key 13 are optional). Requests and replies reject
duplicate or unknown keys, invalid enum values, zero object identities/generations, and geometry
outside the scalar limits. A parent is resolved in the authenticated session, never a
producer-specified session. Self-parenting and cross-owner parenting are invalid.

## Input and lifecycle

`OVERLAY_SUBMISSION_OUTCOME` (0x7033) is an unsolicited interactive-lane envelope (request ID
zero, surface object ID). Keys are context (0), surface (1), surface generation (2), track (3),
channel generation (4), epoch (5), scene revision (6), and outcome (7: presented=0,
superseded=1). All identities, generations, epoch and revision are nonzero. A valid accepted
submission receives exactly one terminal outcome while the lane remains live. Presented means
the exact scene was included in a successfully composed host output frame, not that pixels
were scanned out by physical hardware. It atomically publishes matching hit state. A pending
scene replaced or closed before composition resolves as superseded; compilation/admission
errors use existing channel failure paths, and lane loss terminates unresolved receipt waits
with connection loss rather than inventing an outcome.

The host may hold one composition snapshot and one newer replacement for a window. A snapshot
reserved by an in-progress render MUST NOT be superseded until that render completes or is
abandoned. Failed/abandoned renders MUST NOT acknowledge presentation. A displayed scene and
its input state remain available while its replacement track is primed. Queues of pending
outcomes plus unresolved submissions are bounded to 256 per owner; overflow revokes the affected
owner's overlay lane and input state. An outcome does not retire previously queued input events.

`OVERLAY_VIEWPORT_CHANGED` (0x7034) is an unsolicited interactive-lane envelope with request and
object ID zero. Keys are nonzero viewport revision (0), logical `[width,height]` Q32.32 extent
(1), and positive u32 `[scale_numerator,scale_denominator]` (2). The host sends an initial
snapshot after lane authentication and another when authoritative extent or DPI changes.
Revisions strictly increase on changes; unchanged geometry does not advance them. Receivers
may coalesce queued viewport snapshots, but MUST preserve key/button transitions. Window
queries return the same authoritative viewport and revision. These records require the complete
overlay profile bundle and never pass through the terminal PTY.

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

## Host text and editor geometry

The optional `overlay-text-v1` profile requires `overlay-input-v1`. Presenters MUST omit it
unless host measurement and platform editor positioning are both implemented. Unsupported
terminating presenters/gateways MUST reject it when required. These records use authenticated
control envelopes with the window surface object ID; keys 0, 1, and 2 are context, surface,
and surface generation. Authority requires the context's surface/track/media operation and a
live overlay input lane. Replies retain request correlation and object identity.

`MEASURE_OVERLAY_TEXT` (0x7025) adds key 3 containing a one-element array holding precisely the
tag-7 text specification defined above. Other drawing commands are forbidden. Origin and color
have no effect on measurement; geometry is relative to the layout origin. Text is at most 4096
UTF-8 bytes; the existing font/style and control-body limits also apply. The host uses the same
font selection, fallback, shaping and wrapping as painting. Measurement MUST run independently
of the UI, control and interactive-lane loops. At most one measurement per authenticated owner
and 16 total may be processing; excess requests fail with LIMIT_EXCEEDED. Disconnect/revocation
MUST NOT remove a worker's budget charge before its work finishes.

`OVERLAY_TEXT_MEASURED` (0x7026) echoes keys 0-2. Key 3 is `[width,height]` in Q32.32. Keys 4 and 5
are line and visual-order cluster arrays, respectively, each bounded to 1024 entries. An entry is
`[utf8_start,utf8_end,x,y,width,height,baseline,rtl_boolean]`; end is exclusive. Ranges MUST lie on
UTF-8 boundaries in the requested string. Dimensions are nonnegative; combining clusters may
have zero advance. Baseline is layout-relative for lines and zero for clusters. A line's rtl flag
is false; direction is reported per cluster. Empty text has zero width and its host line height.
If the complete reply exceeds negotiated control limits, the request fails atomically. This is
a measurement snapshot, not a retained layout handle; callers must remeasure when font/style
inputs change. Custom fonts, styled runs and retained text-layout references are separate work.

`SET_OVERLAY_EDITOR` (0x7027) adds key 3, a nonzero presented scene revision, and key 4, a
window-local logical rectangle `[x,y,width,height]` or null to clear it. It replies with correlated
OK. Only the currently focused eligible window may set/clear its editor geometry, and its surface
generation and published scene revision MUST match. Accepted but unpresented revisions are not
sufficient. The rectangle already includes any application canvas transforms. The host clips it
to the owning window and pane, adds window placement, then applies current DPI for platform IME
caret/exclusion APIs. Geometry follows window movement and DPI updates without producer polling.
It does not take focus, alter the scene, or permit another producer to move the active editor.

The host MUST clear editor geometry on focus loss, hide, close, lane loss, revocation, disconnect,
or publication of a different scene revision. A new focused scene must publish new geometry.
When no eligible editor geometry remains, platform IME placement returns to the terminal cursor.
Native IME queries MUST use locally cached geometry rather than synchronously querying a producer.

## Batched styled text and retained layouts

`overlay-text-layout-v1` requires `overlay-text-v1`. It adds the following authenticated control
records and vector command tag 10; hosts MUST NOT accept them without this negotiated profile.
Terminating presenters without these services MUST decline the profile. The protocol preface is
unchanged. Window addressing, context authority, generation, and live input-lane checks are the
same as for single text measurement.

`MEASURE_OVERLAY_TEXT_BATCH` (0x7028) has address keys 0–2, key 3 an array of styled paragraphs,
and key 4 a boolean indicating whether to retain their measured glyph scenes. A paragraph is
`[runs,max_width,alignment,wrap,max_lines]`. Each run is
`[text,size,family,weight,italic,color,underline,strikethrough]`. Text/family are UTF-8 strings;
size and optional max_width are positive Q32.32 logical pixels, color is straight-alpha sRGB
0xRRGGBBAA, weight is 1–1000, and flags are booleans. Alignment is start=0, center=1, end=2,
justify=3, with start/end following paragraph direction. A null max_width is unconstrained.
The optional max_lines is null or 1–1024. `wrap=false` disables soft wrapping, preserving explicit
line breaks. Content exceeding max_width is clipped; max_lines truncates after complete lines,
without inserting an ellipsis. Glyphs and decorations are clipped to the resulting box.
When max_width is supplied, the measured width is that paragraph box width.

Runs concatenate into one shaping input; style boundaries MUST NOT restart script shaping or
font fallback. Geometry indexes refer to UTF-8 bytes in that concatenated input. Empty text
retains the first run's line height. Measurements include only the retained lines; clusters may
extend beyond the horizontal clip, allowing producers to reason about clipped editing content.
Host fonts and fallback are authoritative. Underline and strikethrough use the shaped font's
metrics. No custom fonts, host URL fetching, or implementation-specific font/layout data travel
over the wire.

Batches contain 1–32 paragraphs, each with 1–64 runs, and at most 64 runs and 4096 text bytes
in total. Font family strings are each at most 256 bytes. Existing control-body limits apply.
The single-measurement and batch services share one outstanding job per owner and 16 globally.
Shaping and glyph-scene compilation MUST run outside the UI/control/input loops.

`OVERLAY_TEXT_BATCH_MEASURED` (0x7029) echoes address keys 0–2. Key 3 is an ordered array of
`[layout_id,measurement]` pairs with one result per input paragraph. `measurement` is the complete
`OVERLAY_TEXT_MEASURED` payload map, including its repeated window address. Across the batch,
line plus cluster geometry is bounded to 1024 entries. IDs are zero for non-retaining requests;
otherwise they are nonzero u64 identities unique within the window generation and MUST NOT be
reused there, including after release. Results MUST preserve request order and be committed
atomically: any invalid input, shaping failure, resource limit, or oversized reply rejects the
whole batch without publishing any retained layout.

Vector command `[10,layout_id,[x,y]]` paints the retained glyph scene at a window-local logical
origin using the Canvas transform, clipping, and opacity. It MUST use the measured glyph positions,
fonts, colors, and decorations without reshaping. Font configuration changes do not alter an
existing layout. These resources belong to the complete authenticated owner/context/surface/
generation, and survive replacement of the vector track within that window. A track/channel may
reference only its own window's layout namespace; no numeric-ID lookup may cross ownership.

`RELEASE_OVERLAY_TEXT_LAYOUTS` (0x702a) has address keys 0–2 and key 3 an array of 1–32 distinct,
nonzero layout IDs. The host validates all IDs before removing any and replies with correlated OK.
Release removes future lookup entries; it MUST preserve scenes that already resolved their
references, including scenes compiling or awaiting composition. Because release uses control and
submission uses bulk transport, producers wanting guaranteed preservation wait for presented
before release. Later submissions referring to a released ID fail atomically.

At most 128 layouts per owner and 2048 globally may remain charged, including released layouts
still referenced by scenes. Pending batch work is separately bounded by the worker and batch
limits. Closing a window, context revocation, input-lane loss, and producer loss remove only that
owner's namespaces; charges remain until the last in-flight scene reference is released. A host
MUST NOT reclaim released-layout capacity early or evict live layouts to admit another owner.
