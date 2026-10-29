//! Append-only Vivid 1.5 numeric and profile registries.
//!
//! This module mirrors `vivid-protocol-1.5-registry.toml`. Numeric assignments are wire
//! contracts: never reuse or renumber them within protocol version 1.5.
//!
//! Record types and `ERROR` codes are public through [`crate::messages`], next to the codecs
//! that use them; this module holds profiles, limits, and the remaining value sets.

use std::{collections::BTreeSet, fmt};

/// Required profile: framing, control, contexts, surfaces, scenes, leases, lanes, and errors.
pub const CORE_CONTROL: &str = "vivid-core-control-v1";
/// Presentation target: a terminal grid with text layers and authenticated anchors.
pub const TERMINAL_SURFACE: &str = "terminal-surface-v1";
/// Presentation target: desktop topology, coordinate mapping, and input-capable surfaces.
pub const DESKTOP_SURFACE: &str = "desktop-surface-v1";
/// Presentation target: a logical canvas with no terminal.
pub const CANVAS_SURFACE: &str = "canvas-surface-v1";
/// Immediate video, audio, raster, and encoded-image tracks.
pub const LIVE_MEDIA: &str = "live-media-v1";
/// Exact-PTS playback with pause, flush, end of stream, and drain.
pub const TIMED_MEDIA: &str = "timed-media-v1";
/// Authoritative presentation holds and synchronized audio and video starts.
pub const TIMED_MEDIA_SYNC: &str = "timed-media-sync-v1";
/// Track-scoped audio output gain.
pub const AUDIO_GAIN: &str = "audio-gain-v1";
/// Consent-gated microphone input, media §17.
pub const AUDIO_INPUT: &str = "audio-input-v1";
/// Epoch-checked desktop input binding with a renewal watchdog.
pub const DESKTOP_INPUT: &str = "desktop-input-v1";
/// User-initiated copy of regular files from the presenter to the producer.
pub const FILE_DROP: &str = "file-drop-v1";
/// Reports the committed absolute destination path in a successful `FILE_RESULT`.
pub const FILE_DROP_PATH: &str = "file-drop-path-v1";
/// Bounded status queries, change events, and waits.
pub const OBSERVABILITY: &str = "observability-v1";
/// WebTransport and WebSocket carrier constraints, selected by the web binding.
pub const WEB_CARRIER: &str = "web-carrier-v1";
/// Experimental multiplexed carrier name; never enabled on a standards-track endpoint.
pub const MULTIPLEXED_SESSION_CARRIER: &str = "multiplexed-session-carrier-v1";

/// Semantic profile for content with no target-specific semantics, such as terminal images.
pub const GENERIC_CONTENT: &str = "generic-content-v1";
/// Semantic profile for content measured in terminal cells.
pub const TERMINAL_CONTENT: &str = "terminal-content-v1";
/// Semantic profile for a desktop surface in desktop logical pixels.
pub const DESKTOP_CONTENT: &str = "desktop-content-v1";
/// Semantic profile for canvas content in canvas logical units.
pub const CANVAS_CONTENT: &str = "canvas-content-v1";

/// Vector overlay windows drawn over a terminal pane.
pub const TERMINAL_OVERLAY: &str = "terminal-overlay-v1";
/// Vector display-list tracks (`VECTOR_FRAME` and assets).
pub const VECTOR_SCENE: &str = "vector-scene-v1";
/// Keyboard and pointer input delivered to overlay windows.
pub const OVERLAY_INPUT: &str = "overlay-input-v1";
/// Host text measurement and platform editor positioning for overlays.
pub const OVERLAY_TEXT: &str = "overlay-text-v1";
/// Batched text measurement with retained layouts and vector command tag 10.
pub const OVERLAY_TEXT_LAYOUT: &str = "overlay-text-layout-v1";
/// Nondefault paragraph typography in styled overlay text.
pub const OVERLAY_TYPOGRAPHY: &str = "overlay-typography-v1";
/// The extra drawing commands a styled overlay surface needs.
pub const OVERLAY_PAINT: &str = "overlay-paint-v1";
/// Overlay cursors, hover transitions, click counting, and pressure.
pub const OVERLAY_POINTER: &str = "overlay-pointer-v1";
/// Lets a focused overlay place text on the clipboard after a user gesture.
pub const OVERLAY_CLIPBOARD: &str = "overlay-clipboard-v1";
/// Tells an overlay about its host, such as the default font and colors.
pub const OVERLAY_ENV: &str = "overlay-env-v1";
/// Lets an overlay describe itself to assistive technology.
pub const OVERLAY_A11Y: &str = "overlay-a11y-v1";

pub(crate) mod record {
    //! Record type codes carried in every record header; public through [`crate::messages`].
    //!
    //! Retired codes stay assigned so they are never reused; [`is_retired`] names them.

    /// Creates or updates an overlay window's geometry and state.
    pub const SET_OVERLAY_WINDOW: u16 = 0x7020;
    /// Reply to `SET_OVERLAY_WINDOW` with the authoritative window fields.
    pub const OVERLAY_WINDOW_READY: u16 = 0x7021;
    /// Asks the host to act on a window, such as focusing or closing it.
    pub const OVERLAY_ACTION: u16 = 0x7022;
    /// Asks for an overlay window's current status.
    pub const QUERY_OVERLAY: u16 = 0x7023;
    /// Reply to `QUERY_OVERLAY` with the window's state.
    pub const OVERLAY_STATUS: u16 = 0x7024;
    /// Asks the host to measure one styled paragraph.
    pub const MEASURE_OVERLAY_TEXT: u16 = 0x7025;
    /// Reply to `MEASURE_OVERLAY_TEXT` with the paragraph's size.
    pub const OVERLAY_TEXT_MEASURED: u16 = 0x7026;
    /// Positions the platform text editor over an overlay's text field.
    pub const SET_OVERLAY_EDITOR: u16 = 0x7027;
    /// Asks the host to measure and retain several styled paragraphs.
    pub const MEASURE_OVERLAY_TEXT_BATCH: u16 = 0x7028;
    /// Reply to `MEASURE_OVERLAY_TEXT_BATCH` with one result per paragraph.
    pub const OVERLAY_TEXT_BATCH_MEASURED: u16 = 0x7029;
    /// Releases text layouts the host retained for a batch measurement.
    pub const RELEASE_OVERLAY_TEXT_LAYOUTS: u16 = 0x702a;
    /// One keyboard, pointer, or focus event delivered to an overlay window.
    pub const OVERLAY_INPUT_EVENT: u16 = 0x7030;
    /// Requests or releases input capture for a focused overlay window.
    pub const OVERLAY_INPUT_CAPTURE: u16 = 0x7031;
    /// Renews the bounded overlay input-lane lease.
    pub const OVERLAY_INPUT_RENEW: u16 = 0x7032;
    /// Vector track media: one display list for an overlay window.
    pub const VECTOR_FRAME: u16 = 0x800d;
    /// Vector track media: an image asset that display lists can reference.
    pub const VECTOR_ASSET: u16 = 0x800e;
    /// Vector track media: releases one retained asset.
    pub const VECTOR_ASSET_RELEASE: u16 = 0x800f;
    /// Event reporting whether a submitted vector frame was presented.
    pub const OVERLAY_SUBMISSION_OUTCOME: u16 = 0x7033;
    /// Event reporting a change to the pane viewport an overlay sits in.
    pub const OVERLAY_VIEWPORT_CHANGED: u16 = 0x7034;
    /// Places overlay text on the clipboard, `overlay-clipboard-v1`.
    pub const SET_OVERLAY_CLIPBOARD: u16 = 0x7035;
    /// Event describing the host environment, `overlay-env-v1`.
    pub const OVERLAY_ENV_CHANGED: u16 = 0x7036;
    /// Publishes an overlay's accessibility tree, `overlay-a11y-v1`.
    pub const SET_OVERLAY_SEMANTICS: u16 = 0x7037;
    /// The producer's first control record; it opens a session (core §6.1).
    pub const HELLO: u16 = 0x0001;
    /// The presenter's reply to `HELLO` that establishes the session (core §6.2).
    pub const WELCOME: u16 = 0x0002;
    /// Successful reply to a correlated request that has no typed reply.
    pub const OK: u16 = 0x0003;
    /// Failure reply carrying a registered error code and detail (core §11).
    pub const ERROR: u16 = 0x0004;
    /// Liveness probe carrying the sender's transmit time (core §6.3).
    pub const PING: u16 = 0x0005;
    /// Reply to `PING` echoing the sender's transmit time.
    pub const PONG: u16 = 0x0006;
    /// Requests a clean close of the logical session (core §6.3).
    pub const GOODBYE: u16 = 0x0007;
    /// Asks for the session's bounded reconciliation root (core §10).
    pub const QUERY_SESSION: u16 = 0x0008;
    /// Reply to `QUERY_SESSION`.
    pub const SESSION_STATUS: u16 = 0x0009;
    /// Opens an authenticated interactive lane (core §7).
    pub const LANE_OPEN: u16 = 0x000a;
    /// Reply to `LANE_OPEN` that accepts the lane.
    pub const LANE_ACCEPTED: u16 = 0x000b;
    /// Event carrying the new presentation-target descriptor and generation.
    pub const TARGET_CHANGED: u16 = 0x000c;
    /// Event reporting a new capability generation for future track probes.
    pub const CAPS_CHANGED: u16 = 0x000d;
    /// Subscribes to observation classes (core §10).
    pub const SET_OBSERVATION: u16 = 0x000e;
    /// Event reporting that queued observations were dropped.
    pub const OBSERVATION_GAP: u16 = 0x000f;

    /// Creates a surface (core §8.2).
    pub const CREATE_SURFACE: u16 = 0x0100;
    /// Reply to `CREATE_SURFACE` or `UPDATE_SURFACE` with the authoritative surface.
    pub const SURFACE_READY: u16 = 0x0101;
    /// Replaces a surface's mutable definition (core §8.3).
    pub const UPDATE_SURFACE: u16 = 0x0102;
    /// Destroys a surface and everything it owns (core §8.3).
    pub const DESTROY_SURFACE: u16 = 0x0103;
    /// Asks for a surface's status (core §8.4).
    pub const QUERY_SURFACE: u16 = 0x0104;
    /// Reply to `QUERY_SURFACE`.
    pub const SURFACE_STATUS: u16 = 0x0105;
    /// Event reporting a surface change.
    pub const SURFACE_CHANGED: u16 = 0x0106;

    /// Asks whether a track configuration is supported, without creating it.
    pub const PROBE_TRACK_CONFIG: u16 = 0x0120;
    /// Reply to `PROBE_TRACK_CONFIG`.
    pub const TRACK_SUPPORT: u16 = 0x0121;
    /// Creates a media track on a surface (media §4).
    pub const CREATE_TRACK: u16 = 0x0122;
    /// Reply to `CREATE_TRACK`.
    pub const TRACK_READY: u16 = 0x0123;
    /// Destroys a media track.
    pub const DESTROY_TRACK: u16 = 0x0124;
    /// Event reporting that the presenter lost a track.
    pub const TRACK_LOST: u16 = 0x0125;
    /// Atomically binds ready tracks to a surface's slots (media §7).
    pub const ACTIVATE_TRACK: u16 = 0x0126;
    /// Reply to `ACTIVATE_TRACK`.
    pub const TRACK_ACTIVATED: u16 = 0x0127;
    /// Moves a track to its next channel generation.
    pub const ADVANCE_CHANNEL: u16 = 0x0128;
    /// Reply to `ADVANCE_CHANNEL`.
    pub const CHANNEL_ADVANCED: u16 = 0x0129;
    /// Asks for a track's status (media §8).
    pub const QUERY_TRACK: u16 = 0x012a;
    /// Reply to `QUERY_TRACK`.
    pub const TRACK_STATUS: u16 = 0x012b;
    /// Registers a bounded wait for a track condition (media §8).
    pub const WAIT_TRACK: u16 = 0x012c;
    /// Reply to `WAIT_TRACK` once its condition holds.
    pub const WAIT_SATISFIED: u16 = 0x012d;
    /// Cancels a registered wait by its request ID.
    pub const CANCEL_WAIT: u16 = 0x012e;
    /// Event reporting a track change.
    pub const TRACK_CHANGED: u16 = 0x012f;

    /// Opens a scene transaction (core §9.2).
    pub const BEGIN_TXN: u16 = 0x0200;
    /// Adds a scene node inside a transaction.
    pub const CREATE_NODE: u16 = 0x0201;
    /// Replaces a scene node inside a transaction.
    pub const UPDATE_NODE: u16 = 0x0202;
    /// Removes a scene node inside a transaction.
    pub const DELETE_NODE: u16 = 0x0203;
    /// Validates and applies a scene transaction.
    pub const COMMIT_TXN: u16 = 0x0204;
    /// Discards a scene transaction.
    pub const ABORT_TXN: u16 = 0x0205;
    /// Reply to `COMMIT_TXN` after the scene is activated.
    pub const SCENE_PRESENTED: u16 = 0x0206;
    /// Asks for a context's scene (core §9.3).
    pub const QUERY_SCENE: u16 = 0x0207;
    /// Reply to `QUERY_SCENE`.
    pub const SCENE_STATUS: u16 = 0x0208;
    /// Event reporting a scene change.
    pub const SCENE_CHANGED: u16 = 0x0209;
    /// Event reporting a verified terminal anchor (terminal §5).
    pub const ANCHOR_READY: u16 = 0x020a;
    /// Event reporting that a terminal anchor no longer exists.
    pub const ANCHOR_GONE: u16 = 0x020b;
    /// Asks for a terminal anchor's status.
    pub const QUERY_ANCHOR: u16 = 0x020c;
    /// Reply to `QUERY_ANCHOR`.
    pub const ANCHOR_STATUS: u16 = 0x020d;

    /// Starts or resumes timed playback (media §15).
    pub const PLAY: u16 = 0x0300;
    /// Pauses timed playback.
    pub const PAUSE: u16 = 0x0301;
    /// Discards a timed track's queued media up to a new epoch.
    pub const FLUSH: u16 = 0x0303;
    /// Completes once a timed track has presented everything queued.
    pub const DRAIN: u16 = 0x0304;
    /// Event reporting a timed track's playback state.
    pub const PLAYBACK_STATE: u16 = 0x0306;
    /// Sets a track's audio output gain (media §15.1).
    pub const SET_AUDIO_GAIN: u16 = 0x0307;
    /// Event reporting a presentation hold (media §15.2).
    pub const PLAYBACK_HOLD: u16 = 0x0308;

    /// Creates a child context with a resource contract (security §4.3).
    pub const CREATE_CONTEXT: u16 = 0x0600;
    /// Reply to `CREATE_CONTEXT`.
    pub const CONTEXT_READY: u16 = 0x0601;
    /// Revokes a child context and everything it owns.
    pub const REVOKE_CONTEXT: u16 = 0x0602;
    /// Event reporting a context change.
    pub const CONTEXT_CHANGED: u16 = 0x0603;
    /// Creates a session lease for another producer (security §6.2).
    pub const CREATE_SESSION_LEASE: u16 = 0x0610;
    /// Reply to `CREATE_SESSION_LEASE`.
    pub const SESSION_LEASE_READY: u16 = 0x0611;
    /// Revokes a session lease.
    pub const REVOKE_SESSION_LEASE: u16 = 0x0612;
    /// Event reporting a session-lease change.
    pub const SESSION_LEASE_CHANGED: u16 = 0x0613;

    /// Requests a desktop input grant for a surface (desktop §5.1).
    pub const SET_INPUT_BINDING: u16 = 0x0700;
    /// Reply to `SET_INPUT_BINDING` (desktop §5.2).
    pub const INPUT_BOUND: u16 = 0x0701;
    /// Event reporting that an input grant ended.
    pub const INPUT_REVOKED: u16 = 0x0702;
    /// Presenter renewal of an enabled input grant (desktop §6).
    pub const INPUT_LEASE_RENEW: u16 = 0x0703;
    /// Releases all held input and advances the grant generation.
    pub const INPUT_RESET: u16 = 0x0704;
    /// One keyboard event on the interactive lane (desktop §7).
    pub const KEY_INPUT: u16 = 0x0710;
    /// One pointer motion event on the interactive lane.
    pub const POINTER_MOTION: u16 = 0x0711;
    /// One pointer button event on the interactive lane.
    pub const POINTER_BUTTON: u16 = 0x0712;
    /// One scroll or axis event on the interactive lane.
    pub const POINTER_AXIS: u16 = 0x0713;

    /// Makes a surface a file-drop target (file drop §2).
    pub const SET_FILE_DROP_BINDING: u16 = 0x7000;
    /// Reply to `SET_FILE_DROP_BINDING`.
    pub const FILE_DROP_BOUND: u16 = 0x7001;
    /// Event offering files the user dropped (file drop §3).
    pub const FILE_DROP_OFFER: u16 = 0x7002;
    /// Accepts a file-drop offer.
    pub const ACCEPT_FILE_DROP: u16 = 0x7003;
    /// Reply to `ACCEPT_FILE_DROP`.
    pub const FILE_DROP_ACCEPTED: u16 = 0x7004;
    /// Cancels a file drop.
    pub const CANCEL_FILE_DROP: u16 = 0x7005;
    /// Event or reply reporting a cancelled file drop.
    pub const FILE_DROP_CANCELLED: u16 = 0x7006;
    /// Moves a file transfer to its next generation (file drop §6).
    pub const ADVANCE_FILE_TRANSFER: u16 = 0x7007;
    /// Reply to `ADVANCE_FILE_TRANSFER`.
    pub const FILE_TRANSFER_ADVANCED: u16 = 0x7008;
    /// Asks for a file drop's status.
    pub const QUERY_FILE_DROP: u16 = 0x7009;
    /// Reply to `QUERY_FILE_DROP`.
    pub const FILE_DROP_STATUS: u16 = 0x700a;
    /// Opens an authenticated file-transfer connection (file drop §4).
    pub const FILE_TRANSFER_OPEN: u16 = 0x7010;
    /// Reply to `FILE_TRANSFER_OPEN`.
    pub const FILE_TRANSFER_ACCEPTED: u16 = 0x7011;
    /// One chunk of file bytes on a file-transfer connection (file drop §5).
    pub const FILE_DATA: u16 = 0x7012;
    /// Raises a file transfer's absolute byte and record allowance.
    pub const MAX_FILE_DATA: u16 = 0x7013;
    /// Marks the end of a file's bytes and carries its hash.
    pub const FILE_FINISH: u16 = 0x7014;
    /// The producer's final outcome for one transferred file.
    pub const FILE_RESULT: u16 = 0x7015;
    /// Aborts a file transfer; either side may send it.
    pub const FILE_TRANSFER_ABORT: u16 = 0x7016;

    /// Opens an authenticated track connection (media §5.1).
    pub const CHANNEL_OPEN: u16 = 0x8000;
    /// One coded video access unit (media §9).
    pub const VIDEO_PACKET: u16 = 0x8001;
    pub const VIDEO_FRAGMENT: u16 = 0x8002;
    /// One raster frame, full or delta (media §11).
    pub const RASTER_FRAME: u16 = 0x8003;
    pub const BLOB_CHUNK: u16 = 0x8004;
    pub const BUFFER_SUBMIT: u16 = 0x8005;
    /// One encoded image (media §12).
    pub const IMAGE_DATA: u16 = 0x8006;
    /// One coded audio packet (media §10).
    pub const AUDIO_PACKET: u16 = 0x8007;
    /// Reply to `CHANNEL_OPEN` that accepts the channel (media §5.2).
    pub const CHANNEL_ACCEPTED: u16 = 0x8008;
    /// Raises a channel's absolute byte and record allowance (media §6).
    pub const MAX_CHANNEL_DATA: u16 = 0x8009;
    /// Ordered end of stream for a channel generation (media §14).
    pub const CHANNEL_EOS: u16 = 0x800a;
    /// Presenter request for a random-access unit (media §13).
    pub const NEED_KEYFRAME: u16 = 0x800b;
    /// Presenter request for a full raster frame (media §13).
    pub const NEED_FULL_FRAME: u16 = 0x800c;

    pub const fn is_retired(value: u16) -> bool {
        matches!(value, VIDEO_FRAGMENT | BLOB_CHUNK | BUFFER_SUBMIT)
    }

    pub const fn is_experimental(value: u16) -> bool {
        value >= 0x9000 && value <= 0xbfff
    }

    pub const fn is_vendor(value: u16) -> bool {
        value >= 0xc000
    }
}

pub(crate) mod error {
    //! Error codes carried by `ERROR` replies; public through [`crate::messages`].

    pub const AUTH_FAILED: u64 = 1;
    pub const UNSUPPORTED_VERSION: u64 = 2;
    pub const UNSUPPORTED_PROFILE: u64 = 3;
    pub const UNSUPPORTED_CONFIG: u64 = 4;
    pub const BAD_MESSAGE: u64 = 5;
    pub const BAD_STATE: u64 = 6;
    pub const DUPLICATE_ID: u64 = 7;
    pub const NOT_FOUND: u64 = 8;
    pub const LIMIT_EXCEEDED: u64 = 9;
    pub const NO_MEMORY: u64 = 10;
    pub const FLOW_CONTROL: u64 = 11;
    pub const HASH_MISMATCH: u64 = 12;
    pub const NEED_KEYFRAME: u64 = 13;
    pub const STALE_EPOCH: u64 = 14;
    pub const STALE_TARGET_GENERATION: u64 = 15;
    pub const ANCHOR_INVALIDATED: u64 = 16;
    pub const AUTHORITY_REVOKED: u64 = 17;
    pub const DECODER: u64 = 18;
    pub const DEVICE_LOST: u64 = 19;
    pub const TIMEOUT: u64 = 20;
    pub const PRECONDITION_FAILED: u64 = 21;
    pub const ALREADY_APPLIED: u64 = 22;
    pub const NOT_VISIBLE: u64 = 23;
    pub const CANCELLED: u64 = 24;
    pub const UNKNOWN_OUTCOME: u64 = 25;
    pub const CHANNEL_BUSY: u64 = 26;
    pub const STALE_CHANNEL_GENERATION: u64 = 27;
    pub const LEASE_SUSPENDED: u64 = 28;
    pub const RATE_LIMITED: u64 = 29;
    pub const INTEGRITY_FAILED: u64 = 30;
    pub const fn is_registered(code: u64) -> bool {
        matches!(
            code,
            AUTH_FAILED
                | UNSUPPORTED_VERSION
                | UNSUPPORTED_PROFILE
                | UNSUPPORTED_CONFIG
                | BAD_MESSAGE
                | BAD_STATE
                | DUPLICATE_ID
                | NOT_FOUND
                | LIMIT_EXCEEDED
                | NO_MEMORY
                | FLOW_CONTROL
                | HASH_MISMATCH
                | NEED_KEYFRAME
                | STALE_EPOCH
                | STALE_TARGET_GENERATION
                | ANCHOR_INVALIDATED
                | AUTHORITY_REVOKED
                | DECODER
                | DEVICE_LOST
                | TIMEOUT
                | PRECONDITION_FAILED
                | ALREADY_APPLIED
                | NOT_VISIBLE
                | CANCELLED
                | UNKNOWN_OUTCOME
                | CHANNEL_BUSY
                | STALE_CHANNEL_GENERATION
                | LEASE_SUSPENDED
                | RATE_LIMITED
                | INTEGRITY_FAILED
        )
    }
}

/// Standards-track `ADVANCE_CHANNEL` reason values.
pub mod channel_advance_reason {
    /// Producer-originated timeline discontinuity, such as a seek.
    pub const TIMELINE_DISCONTINUITY: u64 = 1;
    /// Decoder or transport recovery that does not imply a surface-wide timeline change.
    pub const RECOVERY: u64 = 3;
}

pub mod limit {
    //! Limit identifiers that `ERROR` detail key 0 uses to name the bound a request exceeded.

    /// Bound on concurrent sessions.
    pub const CONCURRENT_SESSIONS: u64 = 1;
    /// Bound on concurrent connections.
    pub const CONCURRENT_CONNECTIONS: u64 = 2;
    /// Bound on child contexts.
    pub const CONTEXTS: u64 = 3;
    /// Bound on child session leases.
    pub const SESSION_LEASES: u64 = 4;
    /// Bound on surfaces.
    pub const SURFACES: u64 = 5;
    /// Bound on aggregate tracks.
    pub const TRACKS: u64 = 6;
    /// Bound on scene nodes.
    pub const NODES: u64 = 7;
    /// Bound on decoder instances.
    pub const DECODER_INSTANCES: u64 = 8;
    /// Bound on coded pixels per video or raster track.
    pub const CODED_PIXELS_PER_TRACK: u64 = 9;
    /// Bound on reserved decoded pixels per second.
    pub const DECODED_PIXELS_PER_SECOND: u64 = 10;
    /// Bound on reserved encoded bits per second.
    pub const ENCODED_BITS_PER_SECOND: u64 = 11;
    /// Bound on reserved media records per second.
    pub const MEDIA_RECORDS_PER_SECOND: u64 = 12;
    /// Bound on audio sample rate per track.
    pub const AUDIO_SAMPLE_RATE: u64 = 13;
    /// Bound on audio channels per track.
    pub const AUDIO_CHANNELS_PER_TRACK: u64 = 14;
    /// Bound on aggregate in-flight media body bytes.
    pub const INFLIGHT_MEDIA_BYTES: u64 = 15;
    /// Bound on retained decoded and poster pixels.
    pub const RETAINED_PIXELS: u64 = 16;
    /// Bound on media record body bytes.
    pub const MEDIA_RECORD_BODY: u64 = 17;
    /// Bound on control record body bytes.
    pub const CONTROL_RECORD_BODY: u64 = 18;
    /// Bound on pending correlated requests.
    pub const PENDING_REQUESTS: u64 = 19;
    /// Bound on registered waits.
    pub const REGISTERED_WAITS: u64 = 20;
    /// Bound on idempotency entries.
    pub const IDEMPOTENCY_ENTRIES: u64 = 21;
    /// Bound on suspended child sessions.
    pub const SUSPENDED_SESSIONS: u64 = 22;
    /// Bound on disconnect grace in microseconds.
    pub const DISCONNECT_GRACE: u64 = 23;
    /// Bound on input events per second.
    pub const INPUT_EVENTS_PER_SECOND: u64 = 24;
    /// Bound on observation-queue entries.
    pub const OBSERVATION_QUEUE: u64 = 25;
    /// Bound on encoded-image cache bytes.
    pub const IMAGE_CACHE_BYTES: u64 = 26;
    /// Bound on pending channel-open attempts.
    pub const CHANNEL_OPEN_ATTEMPTS: u64 = 27;
    /// Bound on open scene transactions.
    pub const OPEN_SCENE_TRANSACTIONS: u64 = 28;
    /// Bound on active terminal anchors.
    pub const ACTIVE_ANCHORS: u64 = 29;
    /// Bound on seen terminal anchor IDs.
    pub const SEEN_ANCHOR_IDS: u64 = 30;
}

/// A profile list that [`validate_profile_set`] rejects.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProfileError {
    /// The named profile is not registered.
    Unknown(String),
    /// A profile is present without one of its prerequisites.
    MissingPrerequisite {
        /// The profile that needs the prerequisite.
        profile: String,
        /// The prerequisite that is absent.
        prerequisite: &'static str,
    },
    /// The list is not sorted, or it repeats a profile.
    NotSortedUnique,
}

impl fmt::Display for ProfileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown(profile) => write!(formatter, "unknown Vivid profile {profile:?}"),
            Self::MissingPrerequisite {
                profile,
                prerequisite,
            } => write!(
                formatter,
                "profile {profile:?} requires profile {prerequisite:?}"
            ),
            Self::NotSortedUnique => formatter.write_str("profiles are not sorted and unique"),
        }
    }
}

impl std::error::Error for ProfileError {}

#[expect(
    clippy::match_same_arms,
    reason = "one arm per profile mirrors the registry prerequisite table"
)]
/// Returns the profiles that `profile` requires, or `None` when it is not registered.
pub fn prerequisites(profile: &str) -> Option<&'static [&'static str]> {
    match profile {
        CORE_CONTROL => Some(&[]),
        TERMINAL_OVERLAY => Some(&[TERMINAL_SURFACE, VECTOR_SCENE]),
        VECTOR_SCENE => Some(&[LIVE_MEDIA]),
        OVERLAY_INPUT => Some(&[TERMINAL_OVERLAY]),
        OVERLAY_TEXT => Some(&[OVERLAY_INPUT]),
        OVERLAY_TEXT_LAYOUT => Some(&[OVERLAY_TEXT]),
        OVERLAY_TYPOGRAPHY => Some(&[OVERLAY_TEXT_LAYOUT]),
        OVERLAY_PAINT => Some(&[TERMINAL_OVERLAY]),
        OVERLAY_POINTER => Some(&[TERMINAL_OVERLAY]),
        OVERLAY_CLIPBOARD => Some(&[OVERLAY_INPUT]),
        OVERLAY_ENV => Some(&[OVERLAY_INPUT]),
        OVERLAY_A11Y => Some(&[TERMINAL_OVERLAY]),
        TERMINAL_SURFACE
        | DESKTOP_SURFACE
        | CANVAS_SURFACE
        | LIVE_MEDIA
        | OBSERVABILITY
        | WEB_CARRIER
        | FILE_DROP
        | MULTIPLEXED_SESSION_CARRIER => Some(&[CORE_CONTROL]),
        FILE_DROP_PATH => Some(&[FILE_DROP]),
        TIMED_MEDIA | AUDIO_INPUT => Some(&[LIVE_MEDIA]),
        AUDIO_GAIN | TIMED_MEDIA_SYNC => Some(&[TIMED_MEDIA]),
        DESKTOP_INPUT => Some(&[DESKTOP_SURFACE, LIVE_MEDIA]),
        _ => None,
    }
}

/// Checks that `profiles` are sorted, unique, known, and closed under their prerequisites.
///
/// # Errors
///
/// Returns [`ProfileError::NotSortedUnique`] when the list is unsorted or repeats a profile,
/// [`ProfileError::Unknown`] for an unregistered profile, and [`ProfileError::MissingPrerequisite`]
/// when a profile's prerequisite is absent.
pub fn validate_profile_set<'a>(
    profiles: impl IntoIterator<Item = &'a str>,
) -> Result<(), ProfileError> {
    let profiles: Vec<_> = profiles.into_iter().collect();
    if profiles.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(ProfileError::NotSortedUnique);
    }
    let set: BTreeSet<_> = profiles.iter().copied().collect();
    for profile in profiles {
        let required =
            prerequisites(profile).ok_or_else(|| ProfileError::Unknown(profile.to_owned()))?;
        for prerequisite in required {
            if !set.contains(prerequisite) {
                return Err(ProfileError::MissingPrerequisite {
                    profile: profile.to_owned(),
                    prerequisite,
                });
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_prerequisites_are_closed() {
        validate_profile_set([DESKTOP_INPUT, DESKTOP_SURFACE, LIVE_MEDIA, CORE_CONTROL]).unwrap();
        assert!(matches!(
            validate_profile_set([DESKTOP_INPUT, CORE_CONTROL]),
            Err(ProfileError::MissingPrerequisite { .. })
        ));
    }

    #[test]
    fn the_file_drop_path_profile_requires_file_drop() {
        validate_profile_set([FILE_DROP_PATH, FILE_DROP, CORE_CONTROL]).unwrap();
        assert!(matches!(
            validate_profile_set([FILE_DROP_PATH, CORE_CONTROL]),
            Err(ProfileError::MissingPrerequisite { .. })
        ));
    }

    #[test]
    fn the_overlay_clipboard_profile_requires_overlay_input() {
        // Reading a clipboard is a 1.5 non-goal with one narrow carve-out; the carve-out still
        // depends on the input lane, so a set that omits it cannot be negotiated.
        // The set must be sorted and unique, exactly as a negotiated list would be.
        let mut profiles = vec![
            OVERLAY_CLIPBOARD,
            OVERLAY_INPUT,
            TERMINAL_OVERLAY,
            TERMINAL_SURFACE,
            VECTOR_SCENE,
            LIVE_MEDIA,
            CORE_CONTROL,
        ];
        profiles.sort_unstable();
        profiles.dedup();
        validate_profile_set(profiles).unwrap();
        assert!(matches!(
            validate_profile_set([OVERLAY_CLIPBOARD, TERMINAL_OVERLAY]),
            Err(ProfileError::MissingPrerequisite { .. })
        ));
    }

    #[test]
    fn the_overlay_environment_profile_requires_overlay_input() {
        assert!(matches!(
            validate_profile_set([OVERLAY_ENV, TERMINAL_OVERLAY, TERMINAL_SURFACE]),
            Err(ProfileError::MissingPrerequisite { .. })
        ));
    }

    #[test]
    fn retired_assignments_remain_reserved() {
        assert!(record::is_retired(record::VIDEO_FRAGMENT));
        assert!(record::is_retired(record::BLOB_CHUNK));
        assert!(record::is_retired(record::BUFFER_SUBMIT));
    }
}
