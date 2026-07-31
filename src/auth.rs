//! Vivid 1.5 authentication proofs and session-key derivation.

use std::fmt;

use hmac::{Hmac, KeyInit, Mac};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use zeroize::{Zeroize, Zeroizing};

use crate::anchor::AnchorKey;

type HmacSha256 = Hmac<Sha256>;

pub const SECRET_BYTES: usize = 32;
pub const NONCE_BYTES: usize = 32;
pub const ATTEMPT_ID_BYTES: usize = 16;
pub const CHANNEL_TAG_BYTES: usize = 16;

#[derive(Clone, Zeroize)]
#[zeroize(drop)]
pub struct Secret32([u8; SECRET_BYTES]);

impl Secret32 {
    pub fn new(bytes: [u8; SECRET_BYTES]) -> Self {
        Self(bytes)
    }

    pub fn from_hex(value: &str) -> Result<Self, AuthError> {
        if value.len() != 64 || !value.is_ascii() {
            return Err(AuthError::InvalidSecretEncoding);
        }
        let mut bytes = [0_u8; SECRET_BYTES];
        for (index, output) in bytes.iter_mut().enumerate() {
            let high =
                unhex(value.as_bytes()[index * 2]).ok_or(AuthError::InvalidSecretEncoding)?;
            let low =
                unhex(value.as_bytes()[index * 2 + 1]).ok_or(AuthError::InvalidSecretEncoding)?;
            *output = (high << 4) | low;
        }
        Ok(Self(bytes))
    }

    pub fn expose(&self) -> &[u8; SECRET_BYTES] {
        &self.0
    }
}

impl fmt::Debug for Secret32 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Secret32([REDACTED])")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthError {
    InvalidSecretEncoding,
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

#[derive(Clone, Zeroize)]
#[zeroize(drop)]
pub struct HandshakePrk([u8; 32]);

impl fmt::Debug for HandshakePrk {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("HandshakePrk([REDACTED])")
    }
}

#[derive(Clone, Zeroize)]
#[zeroize(drop)]
pub struct SessionKeys {
    channel: [u8; 32],
    resume: [u8; 32],
}

impl fmt::Debug for SessionKeys {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SessionKeys([REDACTED])")
    }
}

impl SessionKeys {
    pub fn channel_key(&self) -> &[u8; 32] {
        &self.channel
    }

    pub fn resume_key(&self) -> &[u8; 32] {
        &self.resume
    }
}

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

pub fn activation_verifier(lease_id: u64, activation_secret: &Secret32) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"VIVID-LEASE-1");
    hash.update(lease_id.to_be_bytes());
    hash.update(activation_secret.expose());
    hash.finalize().into()
}

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

pub fn extract_handshake_prk(
    session_secret: &Secret32,
    client_nonce: &[u8; 32],
    server_nonce: &[u8; 32],
    carrier_binding_key: &[u8; 32],
) -> HandshakePrk {
    let mut salt = Zeroizing::new([0_u8; 96]);
    salt[..32].copy_from_slice(client_nonce);
    salt[32..64].copy_from_slice(server_nonce);
    salt[64..].copy_from_slice(carrier_binding_key);
    HandshakePrk(hmac_parts(&*salt, &[session_secret.expose()]))
}

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
    (SessionKeys { channel, resume }, AnchorKey::new(anchor))
}

pub fn welcome_confirmation(prk: &HandshakePrk, welcome_unconfirmed: &[u8]) -> [u8; 32] {
    hmac_parts(
        &prk.0,
        &[b"VIVID-WELCOME-1", &Sha256::digest(welcome_unconfirmed)],
    )
}

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

pub fn resume_hello_proof(
    prior_resume_key: &[u8; 32],
    preface: &[u8; 16],
    lease_id: u64,
    session_id: u64,
    resume_generation: u64,
    attempt_id: &[u8; 16],
    hello_authless: &[u8],
) -> [u8; 32] {
    hmac_parts(
        prior_resume_key,
        &[
            b"VIVID-RESUME-HELLO-1",
            preface,
            &lease_id.to_be_bytes(),
            &session_id.to_be_bytes(),
            &resume_generation.to_be_bytes(),
            attempt_id,
            &Sha256::digest(hello_authless),
        ],
    )
}

pub fn lane_tag(
    session_channel_key: &[u8; 32],
    session_id: u64,
    lane_class: u32,
    lane_generation: u64,
    client_nonce: &[u8; 16],
) -> [u8; 16] {
    truncate_tag(hmac_parts(
        session_channel_key,
        &[
            b"VIVID-LANE-1",
            &session_id.to_be_bytes(),
            &lane_class.to_be_bytes(),
            &lane_generation.to_be_bytes(),
            client_nonce,
        ],
    ))
}

#[allow(clippy::too_many_arguments)]
pub fn channel_tag(
    session_channel_key: &[u8; 32],
    session_id: u64,
    context_id: u64,
    surface_id: u64,
    track_id: u64,
    channel_generation: u64,
    track_kind: u32,
    lane: u32,
    client_nonce: &[u8; 16],
) -> [u8; 16] {
    truncate_tag(hmac_parts(
        session_channel_key,
        &[
            b"VIVID-CHANNEL-1",
            &session_id.to_be_bytes(),
            &context_id.to_be_bytes(),
            &surface_id.to_be_bytes(),
            &track_id.to_be_bytes(),
            &channel_generation.to_be_bytes(),
            &track_kind.to_be_bytes(),
            &lane.to_be_bytes(),
            client_nonce,
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

fn hmac_parts(key: &[u8], parts: &[&[u8]]) -> [u8; 32] {
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

    #[test]
    fn secret_hex_is_exact_and_redacted() {
        let secret = Secret32::from_hex(&"a5".repeat(32)).unwrap();
        assert_eq!(secret.expose(), &[0xa5; 32]);
        assert_eq!(format!("{secret:?}"), "Secret32([REDACTED])");
        assert!(Secret32::from_hex("a5").is_err());
    }

    #[test]
    fn independent_keys_and_tags_are_deterministic() {
        let secret = Secret32::new([7; 32]);
        let prk = extract_handshake_prk(&secret, &[1; 32], &[2; 32], &[0; 32]);
        let (keys, anchor) = derive_session_keys(&prk, 4, 2, &[3; 16]);
        assert_ne!(keys.channel_key(), keys.resume_key());
        let first = lane_tag(keys.channel_key(), 4, 1, 1, &[8; 16]);
        assert!(verify_tag(&first, &first));
        assert!(!verify_tag(&first, &[0; 15]));
        assert_eq!(anchor.as_bytes().len(), 32);
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

        let prk = extract_handshake_prk(&secret, &[1; 32], &[2; 32], &[0; 32]);
        assert_eq!(
            prk.0,
            from_hex("7440a4fbf72d53425155c0be10afb4050fcac9c51bae7ec6282d630ffdcab15a")
        );
        let (keys, anchor) = derive_session_keys(&prk, 4, 2, &[3; 16]);
        assert_eq!(
            *keys.channel_key(),
            from_hex("6c829e3fd4658ed622ece7818a4214e177bd947d14051d471fbbf79b5acc400e")
        );
        assert_eq!(
            *keys.resume_key(),
            from_hex("2a51136c44be287c07155ae9d59ffdc0834f1c4bd2abd2679a3542638c3ae689")
        );
        assert_eq!(
            *anchor.as_bytes(),
            from_hex("5b1f60115bb02492e29df73b046bd921e38527d7fa92b6f1c4ca7c8ec58625c4")
        );
        assert_eq!(
            lane_tag(keys.channel_key(), 4, 1, 1, &[8; 16]),
            from_hex("af23a5f078c30ace874f32f79f097a0b")
        );
        assert_eq!(
            channel_tag(keys.channel_key(), 4, 5, 6, 7, 1, 1, 2, &[9; 16]),
            from_hex("076d75b4ec9c6d04a3c3ac29bbd513ef")
        );
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
