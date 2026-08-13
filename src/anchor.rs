//! Authenticated terminal anchor marker version 3.

use std::fmt;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;
use zeroize::Zeroize;

const AUTH_LABEL: &[u8] = b"VIVID-ANCHOR-3";
pub const MAX_MARKER_BYTES: usize = 192;

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone, Zeroize)]
#[zeroize(drop)]
pub struct AnchorKey([u8; 32]);

impl AnchorKey {
    pub fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for AnchorKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AnchorKey([REDACTED])")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnchorMarker {
    pub session_tag: [u8; 16],
    pub context_id: u64,
    pub anchor_id: u64,
    pub authenticator: [u8; 16],
}

pub fn authenticator(
    key: &AnchorKey,
    session_tag: &[u8; 16],
    context_id: u64,
    anchor_id: u64,
) -> [u8; 16] {
    let mut mac = HmacSha256::new_from_slice(&key.0).expect("HMAC accepts every key length");
    mac.update(AUTH_LABEL);
    mac.update(session_tag);
    mac.update(&context_id.to_be_bytes());
    mac.update(&anchor_id.to_be_bytes());
    let full = mac.finalize().into_bytes();
    full[..16].try_into().expect("fixed-size authenticator")
}

pub fn encode_marker(
    key: &AnchorKey,
    session_tag: &[u8; 16],
    context_id: u64,
    anchor_id: u64,
) -> Result<String, &'static str> {
    if context_id == 0 || anchor_id == 0 {
        return Err("context and anchor IDs must be nonzero");
    }
    let tag = URL_SAFE_NO_PAD.encode(session_tag);
    let auth = URL_SAFE_NO_PAD.encode(authenticator(key, session_tag, context_id, anchor_id));
    let marker = format!("\x1b_VIVID;3;A;{tag};{context_id:016x};{anchor_id:016x};{auth}\x1b\\");
    if marker.len() > MAX_MARKER_BYTES {
        return Err("anchor marker is oversized");
    }
    Ok(marker)
}

pub fn encode_conpty_marker(
    key: &AnchorKey,
    session_tag: &[u8; 16],
    context_id: u64,
    anchor_id: u64,
) -> Result<String, &'static str> {
    let apc = encode_marker(key, session_tag, context_id, anchor_id)?;
    let body = &apc[2..apc.len() - 2];
    let marker = format!("{body};VIVID-END");
    if marker.len() > MAX_MARKER_BYTES {
        return Err("anchor marker is oversized");
    }
    Ok(marker)
}

/// Parse the marker body between APC introducer `ESC _` and terminator `ESC \`.
pub fn parse_marker(marker: &str) -> Result<AnchorMarker, &'static str> {
    if marker.len() > MAX_MARKER_BYTES - 4 || !marker.is_ascii() {
        return Err("anchor marker is oversized or non-ASCII");
    }
    let mut fields = marker.split(';');
    if fields.next() != Some("VIVID") || fields.next() != Some("3") || fields.next() != Some("A") {
        return Err("not a Vivid anchor-v3 marker");
    }
    let tag = fields.next().ok_or("missing session tag")?;
    let context_id = fields.next().ok_or("missing context ID")?;
    let anchor_id = fields.next().ok_or("missing anchor ID")?;
    let auth = fields.next().ok_or("missing anchor authenticator")?;
    if fields.next().is_some()
        || tag.len() != 22
        || context_id.len() != 16
        || anchor_id.len() != 16
        || auth.len() != 22
    {
        return Err("invalid anchor marker field length");
    }
    let tag = URL_SAFE_NO_PAD
        .decode(tag)
        .map_err(|_| "invalid session tag")?;
    let auth = URL_SAFE_NO_PAD
        .decode(auth)
        .map_err(|_| "invalid anchor authenticator")?;
    let context_id = u64::from_str_radix(context_id, 16).map_err(|_| "invalid context ID")?;
    let anchor_id = u64::from_str_radix(anchor_id, 16).map_err(|_| "invalid anchor ID")?;
    if context_id == 0 || anchor_id == 0 {
        return Err("context and anchor IDs must be nonzero");
    }
    Ok(AnchorMarker {
        session_tag: tag.try_into().map_err(|_| "invalid session tag length")?,
        context_id,
        anchor_id,
        authenticator: auth
            .try_into()
            .map_err(|_| "invalid authenticator length")?,
    })
}

pub fn parse_conpty_marker(marker: &str) -> Result<AnchorMarker, &'static str> {
    if marker.len() > MAX_MARKER_BYTES || !marker.ends_with(";VIVID-END") {
        return Err("not a bounded Vivid ConPTY anchor-v3 marker");
    }
    parse_marker(&marker[..marker.len() - ";VIVID-END".len()])
}

pub fn verify_marker(key: &AnchorKey, marker: &AnchorMarker) -> bool {
    authenticator(
        key,
        &marker.session_tag,
        marker.context_id,
        marker.anchor_id,
    )
    .ct_eq(&marker.authenticator)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marker_v3_round_trip_and_owner_binding() {
        let key = AnchorKey::new([0x42; 32]);
        let tag = [0x24; 16];
        let encoded = encode_marker(&key, &tag, 3, 7).unwrap();
        assert!(encoded.len() <= MAX_MARKER_BYTES);
        let marker = parse_marker(&encoded[2..encoded.len() - 2]).unwrap();
        assert!(verify_marker(&key, &marker));
        let conpty = encode_conpty_marker(&key, &tag, 3, 7).unwrap();
        assert_eq!(parse_conpty_marker(&conpty).unwrap(), marker);

        let mut wrong_owner = marker;
        wrong_owner.context_id = 4;
        assert!(!verify_marker(&key, &wrong_owner));
    }
}
