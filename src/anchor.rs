//! Authenticated terminal anchor markers, version 3 (terminal §4 and §5).
//!
//! A producer writes an anchor marker into its terminal output so the presenter learns which cell a
//! scene node is attached to. [`encode_marker`] builds the APC form, and [`encode_conpty_marker`]
//! the printable form that survives `ConPTY`. [`parse_marker`] and [`parse_conpty_marker`] read
//! them back, and [`verify_marker`] checks the [`authenticator`] with the session's [`AnchorKey`].

use std::fmt;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use subtle::ConstantTimeEq;
use zeroize::Zeroize;

use crate::{
    CodecError,
    cbor::Value,
    messages::{MessageError, PayloadMap, StrictMap},
};

pub mod conpty;

const AUTH_LABEL: &[u8] = b"VIVID-ANCHOR-3";
/// Most bytes one anchor marker may span in the terminal stream.
pub const MAX_MARKER_BYTES: usize = 192;

/// The session's anchor key; it zeroizes on drop and never prints.
#[derive(Clone, Zeroize)]
#[zeroize(drop)]
pub struct AnchorKey([u8; 32]);

impl AnchorKey {
    /// Wraps key bytes.
    pub fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Returns the key bytes; keep them out of logs.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for AnchorKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AnchorKey([REDACTED])")
    }
}

/// A parsed terminal anchor marker (terminal §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnchorMarker {
    /// The session's non-secret tag.
    pub session_tag: [u8; 16],
    /// The context that created the anchor.
    pub context_id: u64,
    /// The anchor.
    pub anchor_id: u64,
    /// Truncated HMAC that proves the marker came from the session.
    pub authenticator: [u8; 16],
}

/// Computes a marker's authenticator (terminal §4.3).
pub fn authenticator(
    key: &AnchorKey,
    session_tag: &[u8; 16],
    context_id: u64,
    anchor_id: u64,
) -> [u8; 16] {
    let full = crate::auth::hmac_parts(
        &key.0,
        &[
            AUTH_LABEL,
            session_tag,
            &context_id.to_be_bytes(),
            &anchor_id.to_be_bytes(),
        ],
    );
    crate::array_at(&full, 0)
}

/// Encodes the APC marker that announces anchor `anchor_id` in `context_id`.
///
/// # Errors
///
/// Returns an invalid-argument [`CodecError`] when `context_id` or `anchor_id` is zero, or the
/// marker would exceed [`MAX_MARKER_BYTES`].
pub fn encode_marker(
    key: &AnchorKey,
    session_tag: &[u8; 16],
    context_id: u64,
    anchor_id: u64,
) -> Result<String, CodecError> {
    if context_id == 0 || anchor_id == 0 {
        return Err(CodecError::invalid_argument(
            "context and anchor IDs must be nonzero",
        ));
    }
    let tag = URL_SAFE_NO_PAD.encode(session_tag);
    let auth = URL_SAFE_NO_PAD.encode(authenticator(key, session_tag, context_id, anchor_id));
    let marker = format!("\x1b_VIVID;3;A;{tag};{context_id:016x};{anchor_id:016x};{auth}\x1b\\");
    if marker.len() > MAX_MARKER_BYTES {
        return Err(CodecError::invalid_argument("anchor marker is oversized"));
    }
    Ok(marker)
}

/// Encodes the anchor marker in the form that survives `ConPTY`'s line wrapping.
///
/// # Errors
///
/// Returns an invalid-argument [`CodecError`] when [`encode_marker`] fails or the `ConPTY` form
/// would exceed [`MAX_MARKER_BYTES`].
pub fn encode_conpty_marker(
    key: &AnchorKey,
    session_tag: &[u8; 16],
    context_id: u64,
    anchor_id: u64,
) -> Result<String, CodecError> {
    let apc = encode_marker(key, session_tag, context_id, anchor_id)?;
    let body = &apc[2..apc.len() - 2];
    let marker = format!("{body};VIVID-END");
    if marker.len() > MAX_MARKER_BYTES {
        return Err(CodecError::invalid_argument("anchor marker is oversized"));
    }
    Ok(marker)
}

/// Parse the marker body between APC introducer `ESC _` and terminator `ESC \`.
///
/// # Errors
///
/// Returns a malformed-input [`CodecError`] when `marker` is oversized or non-ASCII, is not a Vivid
/// anchor-v3 marker, lacks a field, or has a field of the wrong length or encoding.
pub fn parse_marker(marker: &str) -> Result<AnchorMarker, CodecError> {
    if marker.len() > MAX_MARKER_BYTES - 4 || !marker.is_ascii() {
        return Err(malformed("anchor marker is oversized or non-ASCII"));
    }
    let mut fields = marker.split(';');
    if fields.next() != Some("VIVID") || fields.next() != Some("3") || fields.next() != Some("A") {
        return Err(malformed("not a Vivid anchor-v3 marker"));
    }
    let tag = fields
        .next()
        .ok_or_else(|| malformed("missing session tag"))?;
    let context_id = fields
        .next()
        .ok_or_else(|| malformed("missing context ID"))?;
    let anchor_id = fields
        .next()
        .ok_or_else(|| malformed("missing anchor ID"))?;
    let auth = fields
        .next()
        .ok_or_else(|| malformed("missing anchor authenticator"))?;
    if fields.next().is_some()
        || tag.len() != 22
        || context_id.len() != 16
        || !context_id.bytes().all(|byte| byte.is_ascii_hexdigit())
        || anchor_id.len() != 16
        || !anchor_id.bytes().all(|byte| byte.is_ascii_hexdigit())
        || auth.len() != 22
    {
        return Err(malformed("invalid anchor marker field length"));
    }
    let tag = URL_SAFE_NO_PAD
        .decode(tag)
        .map_err(|_| malformed("invalid session tag"))?;
    let auth = URL_SAFE_NO_PAD
        .decode(auth)
        .map_err(|_| malformed("invalid anchor authenticator"))?;
    let context_id =
        u64::from_str_radix(context_id, 16).map_err(|_| malformed("invalid context ID"))?;
    let anchor_id =
        u64::from_str_radix(anchor_id, 16).map_err(|_| malformed("invalid anchor ID"))?;
    if context_id == 0 || anchor_id == 0 {
        return Err(malformed("context and anchor IDs must be nonzero"));
    }
    Ok(AnchorMarker {
        session_tag: tag
            .try_into()
            .map_err(|_| malformed("invalid session tag length"))?,
        context_id,
        anchor_id,
        authenticator: auth
            .try_into()
            .map_err(|_| malformed("invalid authenticator length"))?,
    })
}

/// Parses an anchor marker recovered from `ConPTY` output, which ends in `;VIVID-END`.
///
/// # Errors
///
/// Returns a malformed-input [`CodecError`] when `marker` exceeds [`MAX_MARKER_BYTES`], lacks the
/// `;VIVID-END` suffix, or fails [`parse_marker`].
pub fn parse_conpty_marker(marker: &str) -> Result<AnchorMarker, CodecError> {
    if marker.len() > MAX_MARKER_BYTES || !marker.ends_with(";VIVID-END") {
        return Err(malformed("not a bounded Vivid ConPTY anchor-v3 marker"));
    }
    parse_marker(&marker[..marker.len() - ";VIVID-END".len()])
}

/// Checks a marker's authenticator in constant time.
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

fn malformed(message: &'static str) -> CodecError {
    CodecError::malformed(message)
}

/// The `QUERY_ANCHOR` payload: ask where an anchor currently is (terminal surface §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueryAnchor {
    /// The context that owns the anchor.
    pub context_id: u64,
    /// The anchor, local to its context.
    pub anchor_id: u64,
}

impl QueryAnchor {
    /// Encodes the payload map.
    pub fn payload(&self) -> PayloadMap {
        vec![
            (0, Value::Unsigned(self.context_id)),
            (1, Value::Unsigned(self.anchor_id)),
        ]
    }

    /// Decodes a `QUERY_ANCHOR` payload.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError`] when `value` is not a strict `QUERY_ANCHOR` map or a field is
    /// missing or not an unsigned integer.
    pub fn decode(value: &Value) -> Result<Self, MessageError> {
        let map = StrictMap::new("QUERY_ANCHOR", value, &[0, 1])?;
        Ok(Self {
            context_id: map.required_u64(0)?,
            anchor_id: map.required_u64(1)?,
        })
    }
}
