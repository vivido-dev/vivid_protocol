//! Append-only Vivid 1.5 numeric and profile registries.
//!
//! This module mirrors `vivid-protocol-1.5-registry.toml`. Numeric assignments are wire
//! contracts: never reuse or renumber them within protocol version 1.5.

use std::{collections::BTreeSet, fmt};

pub const CORE_CONTROL: &str = "vivid-core-control-v1";
pub const TERMINAL_SURFACE: &str = "terminal-surface-v1";
pub const DESKTOP_SURFACE: &str = "desktop-surface-v1";
pub const CANVAS_SURFACE: &str = "canvas-surface-v1";
pub const LIVE_MEDIA: &str = "live-media-v1";
pub const TIMED_MEDIA: &str = "timed-media-v1";
pub const DESKTOP_INPUT: &str = "desktop-input-v1";
pub const OBSERVABILITY: &str = "observability-v1";
pub const WEB_CARRIER: &str = "web-carrier-v1";
pub const MULTIPLEXED_SESSION_CARRIER: &str = "multiplexed-session-carrier-v1";

pub const GENERIC_CONTENT: &str = "generic-content-v1";
pub const TERMINAL_CONTENT: &str = "terminal-content-v1";
pub const DESKTOP_CONTENT: &str = "desktop-content-v1";
pub const CANVAS_CONTENT: &str = "canvas-content-v1";

pub mod record {
    pub const HELLO: u16 = 0x0001;
    pub const WELCOME: u16 = 0x0002;
    pub const OK: u16 = 0x0003;
    pub const ERROR: u16 = 0x0004;
    pub const PING: u16 = 0x0005;
    pub const PONG: u16 = 0x0006;
    pub const GOODBYE: u16 = 0x0007;
    pub const QUERY_SESSION: u16 = 0x0008;
    pub const SESSION_STATUS: u16 = 0x0009;
    pub const LANE_OPEN: u16 = 0x000a;
    pub const LANE_ACCEPTED: u16 = 0x000b;
    pub const TARGET_CHANGED: u16 = 0x000c;
    pub const CAPS_CHANGED: u16 = 0x000d;
    pub const SET_OBSERVATION: u16 = 0x000e;
    pub const OBSERVATION_GAP: u16 = 0x000f;

    pub const CREATE_SURFACE: u16 = 0x0100;
    pub const SURFACE_READY: u16 = 0x0101;
    pub const UPDATE_SURFACE: u16 = 0x0102;
    pub const DESTROY_SURFACE: u16 = 0x0103;
    pub const QUERY_SURFACE: u16 = 0x0104;
    pub const SURFACE_STATUS: u16 = 0x0105;
    pub const SURFACE_CHANGED: u16 = 0x0106;

    pub const PROBE_TRACK_CONFIG: u16 = 0x0120;
    pub const TRACK_SUPPORT: u16 = 0x0121;
    pub const CREATE_TRACK: u16 = 0x0122;
    pub const TRACK_READY: u16 = 0x0123;
    pub const DESTROY_TRACK: u16 = 0x0124;
    pub const TRACK_LOST: u16 = 0x0125;
    pub const ACTIVATE_TRACK: u16 = 0x0126;
    pub const TRACK_ACTIVATED: u16 = 0x0127;
    pub const ADVANCE_CHANNEL: u16 = 0x0128;
    pub const CHANNEL_ADVANCED: u16 = 0x0129;
    pub const QUERY_TRACK: u16 = 0x012a;
    pub const TRACK_STATUS: u16 = 0x012b;
    pub const WAIT_TRACK: u16 = 0x012c;
    pub const WAIT_SATISFIED: u16 = 0x012d;
    pub const CANCEL_WAIT: u16 = 0x012e;
    pub const TRACK_CHANGED: u16 = 0x012f;

    pub const BEGIN_TXN: u16 = 0x0200;
    pub const CREATE_NODE: u16 = 0x0201;
    pub const UPDATE_NODE: u16 = 0x0202;
    pub const DELETE_NODE: u16 = 0x0203;
    pub const COMMIT_TXN: u16 = 0x0204;
    pub const ABORT_TXN: u16 = 0x0205;
    pub const SCENE_PRESENTED: u16 = 0x0206;
    pub const QUERY_SCENE: u16 = 0x0207;
    pub const SCENE_STATUS: u16 = 0x0208;
    pub const SCENE_CHANGED: u16 = 0x0209;
    pub const ANCHOR_READY: u16 = 0x020a;
    pub const ANCHOR_GONE: u16 = 0x020b;
    pub const QUERY_ANCHOR: u16 = 0x020c;
    pub const ANCHOR_STATUS: u16 = 0x020d;

    pub const PLAY: u16 = 0x0300;
    pub const PAUSE: u16 = 0x0301;
    pub const FLUSH: u16 = 0x0303;
    pub const DRAIN: u16 = 0x0304;
    pub const PLAYBACK_STATE: u16 = 0x0306;

    pub const CREATE_CONTEXT: u16 = 0x0600;
    pub const CONTEXT_READY: u16 = 0x0601;
    pub const REVOKE_CONTEXT: u16 = 0x0602;
    pub const CONTEXT_CHANGED: u16 = 0x0603;
    pub const CREATE_SESSION_LEASE: u16 = 0x0610;
    pub const SESSION_LEASE_READY: u16 = 0x0611;
    pub const REVOKE_SESSION_LEASE: u16 = 0x0612;
    pub const SESSION_LEASE_CHANGED: u16 = 0x0613;

    pub const SET_INPUT_BINDING: u16 = 0x0700;
    pub const INPUT_BOUND: u16 = 0x0701;
    pub const INPUT_REVOKED: u16 = 0x0702;
    pub const INPUT_LEASE_RENEW: u16 = 0x0703;
    pub const INPUT_RESET: u16 = 0x0704;
    pub const KEY_INPUT: u16 = 0x0710;
    pub const POINTER_MOTION: u16 = 0x0711;
    pub const POINTER_BUTTON: u16 = 0x0712;
    pub const POINTER_AXIS: u16 = 0x0713;

    pub const CHANNEL_OPEN: u16 = 0x8000;
    pub const VIDEO_PACKET: u16 = 0x8001;
    pub const VIDEO_FRAGMENT: u16 = 0x8002;
    pub const RASTER_FRAME: u16 = 0x8003;
    pub const BLOB_CHUNK: u16 = 0x8004;
    pub const BUFFER_SUBMIT: u16 = 0x8005;
    pub const IMAGE_DATA: u16 = 0x8006;
    pub const AUDIO_PACKET: u16 = 0x8007;
    pub const CHANNEL_ACCEPTED: u16 = 0x8008;
    pub const MAX_CHANNEL_DATA: u16 = 0x8009;
    pub const CHANNEL_EOS: u16 = 0x800a;
    pub const NEED_KEYFRAME: u16 = 0x800b;
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

pub mod error {
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
}

pub mod limit {
    pub const CONCURRENT_SESSIONS: u64 = 1;
    pub const CONCURRENT_CONNECTIONS: u64 = 2;
    pub const CONTEXTS: u64 = 3;
    pub const SESSION_LEASES: u64 = 4;
    pub const SURFACES: u64 = 5;
    pub const TRACKS: u64 = 6;
    pub const NODES: u64 = 7;
    pub const DECODER_INSTANCES: u64 = 8;
    pub const CODED_PIXELS_PER_TRACK: u64 = 9;
    pub const DECODED_PIXELS_PER_SECOND: u64 = 10;
    pub const ENCODED_BITS_PER_SECOND: u64 = 11;
    pub const MEDIA_RECORDS_PER_SECOND: u64 = 12;
    pub const AUDIO_SAMPLE_RATE: u64 = 13;
    pub const AUDIO_CHANNELS_PER_TRACK: u64 = 14;
    pub const INFLIGHT_MEDIA_BYTES: u64 = 15;
    pub const RETAINED_PIXELS: u64 = 16;
    pub const MEDIA_RECORD_BODY: u64 = 17;
    pub const CONTROL_RECORD_BODY: u64 = 18;
    pub const PENDING_REQUESTS: u64 = 19;
    pub const REGISTERED_WAITS: u64 = 20;
    pub const IDEMPOTENCY_ENTRIES: u64 = 21;
    pub const SUSPENDED_SESSIONS: u64 = 22;
    pub const DISCONNECT_GRACE: u64 = 23;
    pub const INPUT_EVENTS_PER_SECOND: u64 = 24;
    pub const OBSERVATION_QUEUE: u64 = 25;
    pub const IMAGE_CACHE_BYTES: u64 = 26;
    pub const CHANNEL_OPEN_ATTEMPTS: u64 = 27;
    pub const OPEN_SCENE_TRANSACTIONS: u64 = 28;
    pub const ACTIVE_ANCHORS: u64 = 29;
    pub const SEEN_ANCHOR_IDS: u64 = 30;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileError {
    Unknown(String),
    MissingPrerequisite {
        profile: String,
        prerequisite: &'static str,
    },
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

pub fn prerequisites(profile: &str) -> Option<&'static [&'static str]> {
    match profile {
        CORE_CONTROL => Some(&[]),
        TERMINAL_SURFACE
        | DESKTOP_SURFACE
        | CANVAS_SURFACE
        | LIVE_MEDIA
        | OBSERVABILITY
        | WEB_CARRIER
        | MULTIPLEXED_SESSION_CARRIER => Some(&[CORE_CONTROL]),
        TIMED_MEDIA => Some(&[LIVE_MEDIA]),
        DESKTOP_INPUT => Some(&[DESKTOP_SURFACE, LIVE_MEDIA]),
        _ => None,
    }
}

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
    fn retired_assignments_remain_reserved() {
        assert!(record::is_retired(record::VIDEO_FRAGMENT));
        assert!(record::is_retired(record::BLOB_CHUNK));
        assert!(record::is_retired(record::BUFFER_SUBMIT));
    }
}
