//! Authentication proofs and session-key derivation (security §3, §6, and §7).
//!
//! A root producer proves it holds the root secret with [`root_hello_proof`]; a lease holder
//! presents an activation secret that the presenter checks with [`verify_activation_secret`]. Both
//! sides then derive the session's [`ChannelKey`], [`ResumeKey`], and anchor key with
//! [`extract_handshake_prk`] and [`derive_session_keys`], and the presenter proves the same
//! derivation with [`welcome_confirmation`]. Every comparison is constant time, and every secret
//! type zeroizes on drop and redacts its `Debug` output.

use std::fmt;

use hmac::{Hmac, KeyInit, Mac};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use zeroize::{Zeroize, Zeroizing};

use crate::{
    anchor::AnchorKey,
    file_drop::FileTransferOpen,
    messages::{ChannelOpen, LaneClass, LaneOpen},
};

type HmacSha256 = Hmac<Sha256>;

/// Length of a root secret, activation secret, or derived key.
pub const SECRET_BYTES: usize = 32;
/// Length of a `HELLO` client nonce or `WELCOME` server nonce.
pub const NONCE_BYTES: usize = 32;
/// Length of an activation or resume attempt ID.
pub const ATTEMPT_ID_BYTES: usize = 16;
/// Length of a truncated open-record authentication tag.
pub const CHANNEL_TAG_BYTES: usize = 16;

/// A 32-byte secret that zeroizes on drop and never prints.
#[derive(Clone, Zeroize)]
#[zeroize(drop)]
pub struct Secret32([u8; SECRET_BYTES]);

impl Secret32 {
    /// Wraps secret bytes.
    pub fn new(bytes: [u8; SECRET_BYTES]) -> Self {
        Self(bytes)
    }

    /// Parses a secret from exactly 64 hexadecimal digits.
    ///
    /// # Errors
    ///
    /// Returns [`AuthError::InvalidSecretEncoding`] when `value` is not 64 ASCII hex digits.
    pub fn from_hex(value: &str) -> Result<Self, AuthError> {
        if value.len() != 64 || !value.is_ascii() {
            return Err(AuthError::InvalidSecretEncoding);
        }
        let mut bytes = Zeroizing::new([0_u8; SECRET_BYTES]);
        for (index, output) in bytes.iter_mut().enumerate() {
            let high =
                unhex(value.as_bytes()[index * 2]).ok_or(AuthError::InvalidSecretEncoding)?;
            let low =
                unhex(value.as_bytes()[index * 2 + 1]).ok_or(AuthError::InvalidSecretEncoding)?;
            *output = (high << 4) | low;
        }
        Ok(Self(*bytes))
    }

    /// Returns the secret bytes; keep them out of logs and arguments.
    pub fn expose(&self) -> &[u8; SECRET_BYTES] {
        &self.0
    }
}

impl fmt::Debug for Secret32 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Secret32([REDACTED])")
    }
}

/// Secret material that could not be parsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum AuthError {
    /// The secret is not 64 hexadecimal digits.
    InvalidSecretEncoding,
    /// An authentication value has the wrong length.
    InvalidLength,
}

impl fmt::Display for AuthError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSecretEncoding => {
                formatter.write_str("secret is not exactly 64 hexadecimal characters")
            }
            Self::InvalidLength => formatter.write_str("authentication value has invalid length"),
        }
    }
}

impl std::error::Error for AuthError {}

/// The handshake pseudorandom key that session keys and the `WELCOME` confirmation derive from.
#[derive(Clone, Zeroize)]
#[zeroize(drop)]
pub struct HandshakePrk([u8; 32]);

impl fmt::Debug for HandshakePrk {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("HandshakePrk([REDACTED])")
    }
}

/// The session key that authenticates `CHANNEL_OPEN`, `LANE_OPEN`, and `FILE_TRANSFER_OPEN`.
///
/// A distinct type from [`ResumeKey`], so one cannot be passed where the other is required.
/// `Debug` redacts it, and dropping it zeroizes it.
#[derive(Clone, Zeroize)]
#[zeroize(drop)]
pub struct ChannelKey([u8; 32]);

impl ChannelKey {
    /// Wraps channel-key bytes received from a session's key derivation.
    pub fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// The key bytes, for a relay that must pass the key on.
    pub fn expose(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for ChannelKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ChannelKey([REDACTED])")
    }
}

/// The key a later `HELLO` proves it holds to resume this session's lease.
///
/// `Debug` redacts it, and dropping it zeroizes it.
#[derive(Clone, Zeroize)]
#[zeroize(drop)]
pub struct ResumeKey([u8; 32]);

impl ResumeKey {
    /// Wraps resume-key bytes, such as ones a presenter stored with a suspended lease.
    pub fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// The key bytes, for storing with a suspended lease.
    pub fn expose(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for ResumeKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ResumeKey([REDACTED])")
    }
}

/// The two keys a session derives from its handshake.
#[derive(Clone)]
pub struct SessionKeys {
    channel: ChannelKey,
    resume: ResumeKey,
}

impl fmt::Debug for SessionKeys {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SessionKeys([REDACTED])")
    }
}

impl SessionKeys {
    /// The key that authenticates this session's channel, lane, and file-transfer openings.
    pub fn channel_key(&self) -> &ChannelKey {
        &self.channel
    }

    /// The key a resume of this session's lease must prove.
    pub fn resume_key(&self) -> &ResumeKey {
        &self.resume
    }
}

/// The client and server nonces of one handshake, named so they cannot be swapped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HandshakeNonces {
    /// The nonce from `HELLO` key 6.
    pub client: [u8; NONCE_BYTES],
    /// The nonce from `WELCOME` key 8.
    pub server: [u8; NONCE_BYTES],
}

/// Computes the root `HELLO` proof (security §3.1).
///
/// The proof covers `preface` and the `HELLO` payload without its proof.
pub fn root_hello_proof(
    root_secret: &Secret32,
    preface: &[u8; 16],
    hello_authless: &[u8],
) -> [u8; 32] {
    hmac_parts(
        root_secret.expose(),
        &[
            b"VIVID-ROOT-HELLO-1",
            preface,
            &Sha256::digest(hello_authless),
        ],
    )
}

/// Checks a root `HELLO` proof in constant time.
pub fn verify_root_hello_proof(
    root_secret: &Secret32,
    preface: &[u8; 16],
    hello_authless: &[u8],
    proof: &[u8],
) -> bool {
    if proof.len() != 32 {
        return false;
    }
    root_hello_proof(root_secret, preface, hello_authless)
        .ct_eq(proof)
        .into()
}

/// Computes the lease activation verifier the controller stores instead of the secret (security
/// §6.1).
pub fn activation_verifier(lease_id: u64, activation_secret: &Secret32) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"VIVID-LEASE-1");
    hash.update(lease_id.to_be_bytes());
    hash.update(activation_secret.expose());
    hash.finalize().into()
}

/// Checks an activation secret against its stored verifier in constant time.
pub fn verify_activation_secret(
    lease_id: u64,
    activation_secret: &Secret32,
    verifier: &[u8],
) -> bool {
    if verifier.len() != 32 {
        return false;
    }
    activation_verifier(lease_id, activation_secret)
        .ct_eq(verifier)
        .into()
}

/// Extracts the handshake key that session keys derive from (security §3.2).
///
/// It mixes the session secret, both nonces, and the carrier binding key.
pub fn extract_handshake_prk(
    session_secret: &Secret32,
    nonces: &HandshakeNonces,
    carrier_binding_key: &[u8; 32],
) -> HandshakePrk {
    let mut salt = Zeroizing::new([0_u8; 96]);
    salt[..32].copy_from_slice(&nonces.client);
    salt[32..64].copy_from_slice(&nonces.server);
    salt[64..].copy_from_slice(carrier_binding_key);
    HandshakePrk(hmac_parts(&*salt, &[session_secret.expose()]))
}

/// Derives the session's channel, resume, and anchor keys from the handshake key.
pub fn derive_session_keys(
    prk: &HandshakePrk,
    session_id: u64,
    resume_generation: u64,
    session_tag: &[u8; 16],
) -> (SessionKeys, AnchorKey) {
    let channel = hkdf_expand(
        &prk.0,
        &[b"VIVID-SESSION-CHANNEL-1", &session_id.to_be_bytes()],
    );
    let resume = hkdf_expand(
        &prk.0,
        &[
            b"VIVID-SESSION-RESUME-1",
            &session_id.to_be_bytes(),
            &resume_generation.to_be_bytes(),
        ],
    );
    let anchor = hkdf_expand(&prk.0, &[b"VIVID-ANCHOR-KEY-3", session_tag]);
    (
        SessionKeys {
            channel: ChannelKey::new(channel),
            resume: ResumeKey::new(resume),
        },
        AnchorKey::new(anchor),
    )
}

/// Computes the `WELCOME` confirmation over the payload without its confirmation.
pub fn welcome_confirmation(prk: &HandshakePrk, welcome_unconfirmed: &[u8]) -> [u8; 32] {
    hmac_parts(
        &prk.0,
        &[b"VIVID-WELCOME-1", &Sha256::digest(welcome_unconfirmed)],
    )
}

/// Checks a `WELCOME` confirmation in constant time.
pub fn verify_welcome_confirmation(
    prk: &HandshakePrk,
    welcome_unconfirmed: &[u8],
    confirmation: &[u8],
) -> bool {
    if confirmation.len() != 32 {
        return false;
    }
    welcome_confirmation(prk, welcome_unconfirmed)
        .ct_eq(confirmation)
        .into()
}

/// The lease and session a resume `HELLO` names, which its proof binds.
pub(crate) struct ResumeBinding {
    pub(crate) lease_id: u64,
    pub(crate) session_id: u64,
    pub(crate) resume_generation: u64,
    pub(crate) attempt_id: [u8; ATTEMPT_ID_BYTES],
}

/// Computes the resume proof over the preface, `binding`, and the proof-free `HELLO`.
pub(crate) fn resume_hello_proof(
    prior_resume_key: &ResumeKey,
    preface: &[u8; 16],
    binding: &ResumeBinding,
    hello_authless: &[u8],
) -> [u8; 32] {
    hmac_parts(
        prior_resume_key.expose(),
        &[
            b"VIVID-RESUME-HELLO-1",
            preface,
            &binding.lease_id.to_be_bytes(),
            &binding.session_id.to_be_bytes(),
            &binding.resume_generation.to_be_bytes(),
            &binding.attempt_id,
            &Sha256::digest(hello_authless),
        ],
    )
}

/// Computes the `LANE_OPEN` tag over every binding field of `open` except the tag itself.
pub(crate) fn lane_tag(session_channel_key: &ChannelKey, open: &LaneOpen) -> [u8; 16] {
    truncate_tag(hmac_parts(
        session_channel_key.expose(),
        &[
            b"VIVID-LANE-1",
            &open.session_id.to_be_bytes(),
            &(LaneClass::Interactive as u32).to_be_bytes(),
            &open.lane_generation.to_be_bytes(),
            &open.client_nonce,
        ],
    ))
}

/// Computes the `CHANNEL_OPEN` tag over every binding field of `open` except the tag itself.
pub(crate) fn channel_tag(session_channel_key: &ChannelKey, open: &ChannelOpen) -> [u8; 16] {
    truncate_tag(hmac_parts(
        session_channel_key.expose(),
        &[
            b"VIVID-CHANNEL-1",
            &open.session_id.to_be_bytes(),
            &open.context_id.to_be_bytes(),
            &open.surface_id.to_be_bytes(),
            &open.track_id.to_be_bytes(),
            &open.channel_generation.to_be_bytes(),
            &(open.track_kind as u32).to_be_bytes(),
            &(open.lane as u32).to_be_bytes(),
            &open.client_nonce,
        ],
    ))
}

/// Computes the `FILE_TRANSFER_OPEN` tag over every binding field of `open` except the tag.
pub(crate) fn file_transfer_tag(
    session_channel_key: &ChannelKey,
    open: &FileTransferOpen,
) -> [u8; 16] {
    truncate_tag(hmac_parts(
        session_channel_key.expose(),
        &[
            b"VIVID-FILE-TRANSFER-1",
            &open.session_id.to_be_bytes(),
            &open.context_id.to_be_bytes(),
            &open.surface_id.to_be_bytes(),
            &open.producer_epoch.get().to_be_bytes(),
            &open.grant_generation.get().to_be_bytes(),
            &open.surface_generation.get().to_be_bytes(),
            &open.drop_id.to_be_bytes(),
            &open.transfer_id.to_be_bytes(),
            &open.transfer_generation.get().to_be_bytes(),
            &open.resume_offset.to_be_bytes(),
            &open.maximum_record_body.to_be_bytes(),
            &open.maximum_body_bytes.to_be_bytes(),
            &open.maximum_records.to_be_bytes(),
            &open.client_nonce,
        ],
    ))
}

/// Constant-time comparison of a 32-byte transcript proof.
///
/// Security §2: every authentication tag and secret verifier is compared in constant time after an
/// exact-length check, and neither the bytes nor a digest of them is logged.
pub fn verify_proof(expected: &[u8; 32], supplied: &[u8]) -> bool {
    supplied.len() == 32 && expected.ct_eq(supplied).into()
}

/// Checks a 16-byte authentication tag in constant time; any other length fails.
pub fn verify_tag(expected: &[u8; 16], supplied: &[u8]) -> bool {
    supplied.len() == 16 && expected.ct_eq(supplied).into()
}

fn truncate_tag(value: [u8; 32]) -> [u8; 16] {
    value[..16].try_into().expect("fixed-size tag")
}

fn hkdf_expand(prk: &[u8; 32], info: &[&[u8]]) -> [u8; 32] {
    let mut mac = HmacSha256::new_from_slice(prk).expect("HMAC accepts every key length");
    for part in info {
        mac.update(part);
    }
    mac.update(&[1]);
    mac.finalize().into_bytes().into()
}

pub(crate) fn hmac_parts(key: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts every key length");
    for part in parts {
        mac.update(part);
    }
    mac.finalize().into_bytes().into()
}

fn unhex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        messages::TrackKind,
        revision::{
            FileDropEpoch, FileDropGrantGeneration, FileTransferGeneration, SurfaceGeneration,
        },
    };

    fn nonces() -> HandshakeNonces {
        HandshakeNonces {
            client: [1; 32],
            server: [2; 32],
        }
    }

    fn lane_open() -> LaneOpen {
        LaneOpen {
            session_id: 4,
            lane_generation: 1,
            client_nonce: [8; 16],
            authentication_tag: [0; 16],
        }
    }

    #[test]
    fn swapped_nonces_derive_a_different_key() {
        let secret = Secret32::new([7; 32]);
        let swapped = HandshakeNonces {
            client: [2; 32],
            server: [1; 32],
        };
        assert_ne!(
            extract_handshake_prk(&secret, &nonces(), &[0; 32]).0,
            extract_handshake_prk(&secret, &swapped, &[0; 32]).0
        );
    }

    #[test]
    fn secret_hex_is_exact_and_redacted() {
        let secret = Secret32::from_hex(&"a5".repeat(32)).unwrap();
        assert_eq!(secret.expose(), &[0xa5; 32]);
        assert_eq!(format!("{secret:?}"), "Secret32([REDACTED])");
        Secret32::from_hex("a5").unwrap_err();
    }

    #[test]
    fn independent_keys_and_tags_are_deterministic() {
        let secret = Secret32::new([7; 32]);
        let prk = extract_handshake_prk(&secret, &nonces(), &[0; 32]);
        let (keys, anchor) = derive_session_keys(&prk, 4, 2, &[3; 16]);
        assert_ne!(keys.channel_key().expose(), keys.resume_key().expose());
        let first = lane_tag(keys.channel_key(), &lane_open());
        assert!(verify_tag(&first, &first));
        assert!(!verify_tag(&first, &[0; 15]));
        assert_eq!(anchor.as_bytes().len(), 32);
    }

    #[test]
    fn file_transfer_tag_scopes_every_binding_generation() {
        let key = ChannelKey::new([0x5a; 32]);
        let tag = |producer_epoch, grant_generation, surface_generation| {
            let mut open = FileTransferOpen {
                session_id: 1,
                context_id: 2,
                surface_id: 3,
                producer_epoch: FileDropEpoch::new(producer_epoch),
                grant_generation: FileDropGrantGeneration::new(grant_generation),
                surface_generation: SurfaceGeneration::new(surface_generation),
                drop_id: 4,
                transfer_id: 5,
                transfer_generation: FileTransferGeneration::new(1),
                resume_offset: 0,
                maximum_record_body: 4096,
                maximum_body_bytes: 8192,
                maximum_records: 2,
                client_nonce: [6; 16],
                authentication_tag: [0; 16],
            };
            open.sign(&key);
            assert!(open.verify(&key));
            assert!(!open.verify(&ChannelKey::new([0x5b; 32])));
            open.authentication_tag
        };
        let baseline = tag(7, 8, 9);
        assert_ne!(baseline, tag(10, 8, 9));
        assert_ne!(baseline, tag(7, 10, 9));
        assert_ne!(baseline, tag(7, 8, 10));
    }

    #[test]
    fn authentication_and_kdf_golden_vectors() {
        let secret = Secret32::new([7; 32]);
        let mut preface = [0_u8; 16];
        preface[..4].copy_from_slice(b"VIVD");
        preface[4..8].copy_from_slice(&[1, 5, 0, 0]);
        preface[8..12].copy_from_slice(&1_048_576_u32.to_be_bytes());
        assert_eq!(
            root_hello_proof(&secret, &preface, &[0xa1, 0x00, 0x01]),
            from_hex("7bad412eba08c80136f03c23e0577e43d95ef2cf39dd23de3b096b65610058d4")
        );

        let prk = extract_handshake_prk(&secret, &nonces(), &[0; 32]);
        assert_eq!(
            prk.0,
            from_hex("7440a4fbf72d53425155c0be10afb4050fcac9c51bae7ec6282d630ffdcab15a")
        );
        let (keys, anchor) = derive_session_keys(&prk, 4, 2, &[3; 16]);
        assert_eq!(
            *keys.channel_key().expose(),
            from_hex("6c829e3fd4658ed622ece7818a4214e177bd947d14051d471fbbf79b5acc400e")
        );
        assert_eq!(
            *keys.resume_key().expose(),
            from_hex("2a51136c44be287c07155ae9d59ffdc0834f1c4bd2abd2679a3542638c3ae689")
        );
        assert_eq!(
            *anchor.as_bytes(),
            from_hex("5b1f60115bb02492e29df73b046bd921e38527d7fa92b6f1c4ca7c8ec58625c4")
        );
        let mut lane = lane_open();
        lane.sign(keys.channel_key());
        assert_eq!(
            lane.authentication_tag,
            from_hex("af23a5f078c30ace874f32f79f097a0b")
        );
        assert!(lane.verify(keys.channel_key()));
        lane.lane_generation = 2;
        assert!(!lane.verify(keys.channel_key()));
        let mut open = ChannelOpen {
            session_id: 4,
            context_id: 5,
            surface_id: 6,
            track_id: 7,
            channel_generation: 1,
            track_kind: TrackKind::Video,
            lane: LaneClass::Realtime,
            client_nonce: [9; 16],
            authentication_tag: [0; 16],
        };
        open.sign(keys.channel_key());
        assert_eq!(
            open.authentication_tag,
            from_hex("076d75b4ec9c6d04a3c3ac29bbd513ef")
        );
        assert!(open.verify(keys.channel_key()));
        open.track_id = 8;
        assert!(!open.verify(keys.channel_key()));
    }

    fn from_hex<const N: usize>(value: &str) -> [u8; N] {
        assert_eq!(value.len(), N * 2);
        let mut output = [0; N];
        for (index, byte) in output.iter_mut().enumerate() {
            *byte = (unhex(value.as_bytes()[index * 2]).unwrap() << 4)
                | unhex(value.as_bytes()[index * 2 + 1]).unwrap();
        }
        output
    }
}
