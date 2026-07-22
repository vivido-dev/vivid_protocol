//! Authenticated Vivid 1.0 text-anchor marker codec.

use std::fmt;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;
use zeroize::{Zeroize, Zeroizing};

const KEY_LABEL: &[u8] = b"VIVID-ANCHOR-KEY-V2";
const AUTH_LABEL: &[u8] = b"VIVID-ANCHOR-V2";

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone, Zeroize)]
#[zeroize(drop)]
pub struct AnchorKey([u8; 32]);

impl fmt::Debug for AnchorKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AnchorKey([REDACTED])")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnchorMarker {
    pub session_tag: [u8; 16],
    pub anchor_id: u64,
    pub authenticator: [u8; 16],
}

pub fn decode_token(token: &str) -> Result<Zeroizing<[u8; 32]>, &'static str> {
    if token.len() != 64 || !token.is_ascii() {
        return Err("Vivid token is not exactly 64 hexadecimal characters");
    }
    let mut output = Zeroizing::new([0_u8; 32]);
    for (index, byte) in output.iter_mut().enumerate() {
        let high = unhex(token.as_bytes()[index * 2]).ok_or("Vivid token is not hexadecimal")?;
        let low = unhex(token.as_bytes()[index * 2 + 1]).ok_or("Vivid token is not hexadecimal")?;
        *byte = (high << 4) | low;
    }
    Ok(output)
}

pub fn derive_key(token: &[u8; 32], session_tag: &[u8; 16]) -> AnchorKey {
    let mut mac = HmacSha256::new_from_slice(token).expect("HMAC accepts every key length");
    mac.update(KEY_LABEL);
    mac.update(session_tag);
    AnchorKey(mac.finalize().into_bytes().into())
}

pub fn authenticator(key: &AnchorKey, session_tag: &[u8; 16], anchor_id: u64) -> [u8; 16] {
    let mut mac = HmacSha256::new_from_slice(&key.0).expect("HMAC accepts every key length");
    mac.update(AUTH_LABEL);
    mac.update(session_tag);
    mac.update(&anchor_id.to_be_bytes());
    let full = mac.finalize().into_bytes();
    full[..16].try_into().unwrap()
}

pub fn encode_marker(
    key: &AnchorKey,
    session_tag: &[u8; 16],
    anchor_id: u64,
) -> Result<String, &'static str> {
    if anchor_id == 0 {
        return Err("anchor ID is zero");
    }
    let tag = URL_SAFE_NO_PAD.encode(session_tag);
    let auth = URL_SAFE_NO_PAD.encode(authenticator(key, session_tag, anchor_id));
    Ok(format!(
        "\x1b_VIVID;2;A;{tag};{anchor_id:016x};{auth}\x1b\\"
    ))
}

pub fn parse_marker(marker: &str) -> Result<AnchorMarker, &'static str> {
    if marker.len() > 124 || !marker.is_ascii() {
        return Err("anchor marker is oversized or non-ASCII");
    }
    let mut fields = marker.split(';');
    if fields.next() != Some("VIVID") || fields.next() != Some("2") || fields.next() != Some("A") {
        return Err("not a Vivid anchor-v2 marker");
    }
    let tag = fields.next().ok_or("missing session tag")?;
    let id = fields.next().ok_or("missing anchor ID")?;
    let auth = fields.next().ok_or("missing anchor authenticator")?;
    if fields.next().is_some() || tag.len() != 22 || id.len() != 16 || auth.len() != 22 {
        return Err("invalid anchor marker field length");
    }
    let tag = URL_SAFE_NO_PAD
        .decode(tag)
        .map_err(|_| "invalid session tag")?;
    let auth = URL_SAFE_NO_PAD
        .decode(auth)
        .map_err(|_| "invalid anchor authenticator")?;
    let anchor_id = u64::from_str_radix(id, 16).map_err(|_| "invalid anchor ID")?;
    if anchor_id == 0 {
        return Err("anchor ID is zero");
    }
    Ok(AnchorMarker {
        session_tag: tag.try_into().map_err(|_| "invalid session tag length")?,
        anchor_id,
        authenticator: auth
            .try_into()
            .map_err(|_| "invalid authenticator length")?,
    })
}

pub fn verify_marker(key: &AnchorKey, marker: &AnchorMarker) -> bool {
    authenticator(key, &marker.session_tag, marker.anchor_id)
        .ct_eq(&marker.authenticator)
        .into()
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
    fn marker_v2_round_trip_and_forgery_rejection() {
        let token = [0x42; 32];
        let tag = [0x24; 16];
        let key = derive_key(&token, &tag);
        let encoded = encode_marker(&key, &tag, 7).unwrap();
        assert_eq!(
            encoded,
            "\x1b_VIVID;2;A;JCQkJCQkJCQkJCQkJCQkJA;0000000000000007;cvLT9ZYk2egoi5bsgX0PhA\x1b\\"
        );
        assert!(encoded.len() <= 128);
        let marker = parse_marker(&encoded[2..encoded.len() - 2]).unwrap();
        assert!(verify_marker(&key, &marker));
        let mut forged = marker;
        forged.authenticator[0] ^= 1;
        assert!(!verify_marker(&key, &forged));
    }

    #[test]
    fn token_requires_exact_hex() {
        assert!(decode_token(&"ab".repeat(32)).is_ok());
        assert!(decode_token("abcd").is_err());
        assert!(decode_token(&"zz".repeat(32)).is_err());
    }
}
