use std::fmt::{self, Display, Formatter};
use zeroize::{Zeroize, Zeroizing};

const MAX_DEPTH: usize = 16;
const MAX_VALUE_LENGTH: usize = 16 * 1024 * 1024;
const MAX_CONTAINER_LENGTH: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq, Zeroize)]
pub enum Value {
    Unsigned(u64),
    Negative(i64),
    Bytes(Vec<u8>),
    Text(String),
    Array(Vec<Value>),
    Map(Vec<(u64, Value)>),
    Bool(bool),
    Null,
}

/// One unknown entry retained from a canonical CBOR map.
///
/// The value bytes borrow the original input so relays can forward an extension without parsing
/// it into a dynamic value tree or changing its canonical representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreservedEntry<'a> {
    pub key: u64,
    pub encoded_value: &'a [u8],
}

/// An owned preserved entry for message objects that outlive their decode buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreservedField {
    pub key: u64,
    pub encoded_value: Vec<u8>,
}

impl PreservedEntry<'_> {
    pub fn to_owned(self) -> PreservedField {
        PreservedField {
            key: self.key,
            encoded_value: self.encoded_value.to_vec(),
        }
    }
}

/// A canonical map split into parsed known fields and byte-exact unknown fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreservingMap<'a> {
    known: Vec<(u64, Value, &'a [u8])>,
    preserved: Vec<PreservedEntry<'a>>,
}

impl<'a> PreservingMap<'a> {
    pub fn known_value(&self, key: u64) -> Option<&Value> {
        self.known
            .iter()
            .find_map(|(entry_key, value, _)| (*entry_key == key).then_some(value))
    }

    pub fn encoded_known_value(&self, key: u64) -> Option<&'a [u8]> {
        self.known
            .iter()
            .find_map(|(entry_key, _, bytes)| (*entry_key == key).then_some(*bytes))
    }

    pub fn preserved(&self) -> &[PreservedEntry<'a>] {
        &self.preserved
    }

    pub fn preserved_owned(&self) -> Vec<PreservedField> {
        self.preserved
            .iter()
            .copied()
            .map(PreservedEntry::to_owned)
            .collect()
    }
}

impl Value {
    pub fn map_value(&self, key: u64) -> Option<&Value> {
        let Self::Map(entries) = self else {
            return None;
        };
        entries
            .iter()
            .find_map(|(entry_key, value)| (*entry_key == key).then_some(value))
    }

    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Self::Unsigned(value) => Some(*value),
            _ => None,
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Unsigned(value) => i64::try_from(*value).ok(),
            Self::Negative(value) => Some(*value),
            _ => None,
        }
    }

    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Self::Bytes(value) => Some(value),
            _ => None,
        }
    }

    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text(value) => Some(value),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(value) => Some(*value),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Self::Array(value) => Some(value),
            _ => None,
        }
    }
}

#[derive(Debug, Default)]
pub struct Encoder {
    bytes: Vec<u8>,
}

impl Encoder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn into_vec(mut self) -> Vec<u8> {
        std::mem::take(&mut self.bytes)
    }

    pub(crate) fn from_vec(bytes: Vec<u8>) -> Self {
        Self { bytes }
    }

    pub(crate) fn clear(&mut self) {
        self.bytes.zeroize();
    }

    pub fn map(&mut self, length: usize) {
        self.major_length(5, length as u64);
    }

    pub fn array(&mut self, length: usize) {
        self.major_length(4, length as u64);
    }

    pub fn u64(&mut self, value: u64) {
        self.major_length(0, value);
    }

    pub fn i64(&mut self, value: i64) {
        if value >= 0 {
            self.u64(value as u64);
        } else {
            self.major_length(1, (-1_i128 - i128::from(value)) as u64);
        }
    }

    pub fn bytes(&mut self, value: &[u8]) {
        self.major_length(2, value.len() as u64);
        self.bytes.extend_from_slice(value);
    }

    pub fn text(&mut self, value: &str) {
        self.major_length(3, value.len() as u64);
        self.bytes.extend_from_slice(value.as_bytes());
    }

    pub fn bool(&mut self, value: bool) {
        self.bytes.push(if value { 0xf5 } else { 0xf4 });
    }

    pub fn null(&mut self) {
        self.bytes.push(0xf6);
    }

    fn major_length(&mut self, major: u8, value: u64) {
        let prefix = major << 5;
        if value <= 23 {
            self.bytes.push(prefix | value as u8);
        } else if value <= u8::MAX as u64 {
            self.bytes.push(prefix | 24);
            self.bytes.push(value as u8);
        } else if value <= u16::MAX as u64 {
            self.bytes.push(prefix | 25);
            self.bytes.extend_from_slice(&(value as u16).to_be_bytes());
        } else if value <= u32::MAX as u64 {
            self.bytes.push(prefix | 26);
            self.bytes.extend_from_slice(&(value as u32).to_be_bytes());
        } else {
            self.bytes.push(prefix | 27);
            self.bytes.extend_from_slice(&value.to_be_bytes());
        }
    }

    pub(crate) fn canonical_value(&mut self, value: &[u8]) {
        self.bytes.extend_from_slice(value);
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        self.bytes.zeroize();
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodeError(String);

impl Display for EncodeError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for EncodeError {}

/// Encode a value using the canonical Vivid CBOR subset.
pub fn encode(value: &Value) -> Result<Vec<u8>, EncodeError> {
    let mut output = Vec::new();
    encode_into(&mut output, value)?;
    Ok(output)
}

/// Encode a value into a reusable caller-owned buffer.
pub fn encode_into(output: &mut Vec<u8>, value: &Value) -> Result<(), EncodeError> {
    let mut encoder = Encoder::from_vec(std::mem::take(output));
    encoder.clear();
    let result = encode_value(&mut encoder, value, 0);
    if result.is_err() {
        encoder.bytes.zeroize();
    }
    *output = encoder.into_vec();
    result
}

/// Merge parsed known entries with byte-exact preserved entries in canonical key order.
pub fn encode_preserving_map(
    known: &[(u64, Value)],
    preserved: &[PreservedField],
) -> Result<Vec<u8>, EncodeError> {
    validate_container_length(
        known
            .len()
            .checked_add(preserved.len())
            .ok_or_else(|| EncodeError("CBOR map length overflows".into()))?,
    )?;
    if known.windows(2).any(|pair| pair[0].0 >= pair[1].0)
        || preserved.windows(2).any(|pair| pair[0].key >= pair[1].key)
    {
        return Err(EncodeError("CBOR map keys are not strictly sorted".into()));
    }
    for entry in preserved {
        let _value = Zeroizing::new(
            decode_at_depth(&entry.encoded_value, 1)
                .map_err(|error| EncodeError(format!("invalid preserved CBOR value: {error}")))?,
        );
    }

    let mut encoder = Encoder::new();
    encoder.map(known.len() + preserved.len());
    let mut known_index = 0;
    let mut preserved_index = 0;
    while known_index < known.len() || preserved_index < preserved.len() {
        let take_known = match (known.get(known_index), preserved.get(preserved_index)) {
            (Some((known_key, _)), Some(preserved)) if *known_key == preserved.key => {
                return Err(EncodeError(
                    "known and preserved CBOR map keys collide".into(),
                ));
            }
            (Some((known_key, _)), Some(preserved)) => *known_key < preserved.key,
            (Some(_), None) => true,
            (None, Some(_)) => false,
            (None, None) => break,
        };
        if take_known {
            let (key, value) = &known[known_index];
            encoder.u64(*key);
            encode_value(&mut encoder, value, 1)?;
            known_index += 1;
        } else {
            let entry = &preserved[preserved_index];
            encoder.u64(entry.key);
            encoder.canonical_value(&entry.encoded_value);
            preserved_index += 1;
        }
    }
    Ok(encoder.into_vec())
}

fn encode_value(encoder: &mut Encoder, value: &Value, depth: usize) -> Result<(), EncodeError> {
    if depth > MAX_DEPTH {
        return Err(EncodeError("CBOR nesting exceeds 16 levels".into()));
    }
    match value {
        Value::Unsigned(value) => encoder.u64(*value),
        Value::Negative(value) => encoder.i64(*value),
        Value::Bytes(value) => {
            validate_value_length(value.len())?;
            encoder.bytes(value);
        }
        Value::Text(value) => {
            validate_value_length(value.len())?;
            encoder.text(value);
        }
        Value::Array(values) => {
            validate_container_length(values.len())?;
            encoder.array(values.len());
            for value in values {
                encode_value(encoder, value, depth + 1)?;
            }
        }
        Value::Map(entries) => {
            validate_container_length(entries.len())?;
            if entries.windows(2).any(|pair| pair[0].0 >= pair[1].0) {
                return Err(EncodeError("CBOR map keys are not strictly sorted".into()));
            }
            encoder.map(entries.len());
            for (key, value) in entries {
                encoder.u64(*key);
                encode_value(encoder, value, depth + 1)?;
            }
        }
        Value::Bool(value) => encoder.bool(*value),
        Value::Null => encoder.null(),
    }
    Ok(())
}

fn validate_value_length(length: usize) -> Result<(), EncodeError> {
    if length > MAX_VALUE_LENGTH {
        Err(EncodeError(
            "CBOR value exceeds configured length limit".into(),
        ))
    } else {
        Ok(())
    }
}

fn validate_container_length(length: usize) -> Result<(), EncodeError> {
    if length > MAX_CONTAINER_LENGTH {
        Err(EncodeError("CBOR container exceeds 4096 items".into()))
    } else {
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodeError(String);

impl Display for DecodeError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for DecodeError {}

pub fn decode(bytes: &[u8]) -> Result<Value, DecodeError> {
    decode_at_depth(bytes, 0)
}

fn decode_at_depth(bytes: &[u8], depth: usize) -> Result<Value, DecodeError> {
    let mut decoder = Decoder { bytes, offset: 0 };
    let mut value = Zeroizing::new(decoder.value(depth)?);
    if decoder.offset != bytes.len() {
        return Err(DecodeError("trailing bytes after CBOR value".into()));
    }
    Ok(std::mem::replace(&mut *value, Value::Null))
}

/// Decode a canonical numeric-keyed map while retaining unknown values as borrowed byte slices.
pub fn decode_preserving_map<'a>(
    bytes: &'a [u8],
    known_keys: &[u64],
) -> Result<PreservingMap<'a>, DecodeError> {
    if known_keys.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(DecodeError(
            "known CBOR map keys are not strictly sorted".into(),
        ));
    }
    let mut decoder = Decoder { bytes, offset: 0 };
    let initial = decoder.byte()?;
    if initial >> 5 != 5 {
        return Err(DecodeError("CBOR value is not a map".into()));
    }
    let length = decoder.container_length(initial & 0x1f, 2)?;
    let mut known = Vec::with_capacity(length.min(known_keys.len()));
    let mut preserved = Vec::with_capacity(length.saturating_sub(known_keys.len()));
    let mut previous = None;
    for _ in 0..length {
        let key = match decoder.value(1)? {
            Value::Unsigned(key) => key,
            _ => {
                return Err(DecodeError(
                    "CBOR map key is not an unsigned integer".into(),
                ));
            }
        };
        if previous.is_some_and(|previous| previous >= key) {
            return Err(DecodeError("CBOR map keys are not strictly sorted".into()));
        }
        previous = Some(key);
        let start = decoder.offset;
        let value = decoder.value(1)?;
        let encoded_value = &bytes[start..decoder.offset];
        if known_keys.binary_search(&key).is_ok() {
            known.push((key, value, encoded_value));
        } else {
            preserved.push(PreservedEntry { key, encoded_value });
        }
    }
    if decoder.offset != bytes.len() {
        return Err(DecodeError("trailing bytes after CBOR value".into()));
    }
    Ok(PreservingMap { known, preserved })
}

struct Decoder<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl Decoder<'_> {
    fn value(&mut self, depth: usize) -> Result<Value, DecodeError> {
        if depth > MAX_DEPTH {
            return Err(DecodeError("CBOR nesting exceeds 16 levels".into()));
        }
        let initial = self.byte()?;
        let major = initial >> 5;
        let additional = initial & 0x1f;

        match major {
            0 => Ok(Value::Unsigned(self.argument(additional)?)),
            1 => {
                let argument = self.argument(additional)?;
                let value = -1_i128 - i128::from(argument);
                let value = i64::try_from(value)
                    .map_err(|_| DecodeError("negative CBOR integer exceeds i64".into()))?;
                Ok(Value::Negative(value))
            }
            2 => {
                let length = self.length(additional)?;
                Ok(Value::Bytes(self.take(length)?.to_vec()))
            }
            3 => {
                let length = self.length(additional)?;
                let text = std::str::from_utf8(self.take(length)?)
                    .map_err(|_| DecodeError("CBOR text is not UTF-8".into()))?;
                Ok(Value::Text(text.to_owned()))
            }
            4 => {
                let length = self.container_length(additional, 1)?;
                let mut values = Zeroizing::new(Vec::with_capacity(length));
                for _ in 0..length {
                    values.push(self.value(depth + 1)?);
                }
                Ok(Value::Array(std::mem::take(&mut *values)))
            }
            5 => {
                let length = self.container_length(additional, 2)?;
                let mut entries = Zeroizing::new(Vec::with_capacity(length));
                let mut previous = None;
                for _ in 0..length {
                    let key_value = Zeroizing::new(self.value(depth + 1)?);
                    let key = match &*key_value {
                        Value::Unsigned(key) => *key,
                        _ => {
                            return Err(DecodeError(
                                "CBOR map key is not an unsigned integer".into(),
                            ));
                        }
                    };
                    if previous.is_some_and(|previous| previous >= key) {
                        return Err(DecodeError("CBOR map keys are not strictly sorted".into()));
                    }
                    previous = Some(key);
                    entries.push((key, self.value(depth + 1)?));
                }
                Ok(Value::Map(std::mem::take(&mut *entries)))
            }
            7 => match additional {
                20 => Ok(Value::Bool(false)),
                21 => Ok(Value::Bool(true)),
                22 => Ok(Value::Null),
                _ => Err(DecodeError(
                    "unsupported CBOR simple or floating-point value".into(),
                )),
            },
            _ => Err(DecodeError(
                "CBOR tags and indefinite values are not supported".into(),
            )),
        }
    }

    fn length(&mut self, additional: u8) -> Result<usize, DecodeError> {
        let value = self.argument(additional)?;
        let length = usize::try_from(value)
            .map_err(|_| DecodeError("CBOR length does not fit usize".into()))?;
        if length > MAX_VALUE_LENGTH {
            return Err(DecodeError(
                "CBOR value exceeds configured length limit".into(),
            ));
        }
        Ok(length)
    }

    fn container_length(
        &mut self,
        additional: u8,
        minimum_bytes_per_item: usize,
    ) -> Result<usize, DecodeError> {
        let length = self.length(additional)?;
        if length > MAX_CONTAINER_LENGTH {
            return Err(DecodeError("CBOR container exceeds 4096 items".into()));
        }
        let minimum_bytes = length
            .checked_mul(minimum_bytes_per_item)
            .ok_or_else(|| DecodeError("CBOR container size overflows".into()))?;
        if minimum_bytes > self.bytes.len().saturating_sub(self.offset) {
            return Err(DecodeError(
                "CBOR container length exceeds its input".into(),
            ));
        }
        Ok(length)
    }

    fn argument(&mut self, additional: u8) -> Result<u64, DecodeError> {
        match additional {
            value @ 0..=23 => Ok(u64::from(value)),
            24 => {
                let value = u64::from(self.byte()?);
                if value < 24 {
                    return Err(DecodeError("non-shortest CBOR integer encoding".into()));
                }
                Ok(value)
            }
            25 => {
                let value = u64::from(u16::from_be_bytes(self.take(2)?.try_into().unwrap()));
                if value <= u8::MAX as u64 {
                    return Err(DecodeError("non-shortest CBOR integer encoding".into()));
                }
                Ok(value)
            }
            26 => {
                let value = u64::from(u32::from_be_bytes(self.take(4)?.try_into().unwrap()));
                if value <= u16::MAX as u64 {
                    return Err(DecodeError("non-shortest CBOR integer encoding".into()));
                }
                Ok(value)
            }
            27 => {
                let value = u64::from_be_bytes(self.take(8)?.try_into().unwrap());
                if value <= u32::MAX as u64 {
                    return Err(DecodeError("non-shortest CBOR integer encoding".into()));
                }
                Ok(value)
            }
            _ => Err(DecodeError(
                "indefinite-length CBOR is not supported".into(),
            )),
        }
    }

    fn byte(&mut self) -> Result<u8, DecodeError> {
        let byte = self
            .bytes
            .get(self.offset)
            .copied()
            .ok_or_else(|| DecodeError("unexpected end of CBOR input".into()))?;
        self.offset += 1;
        Ok(byte)
    }

    fn take(&mut self, length: usize) -> Result<&[u8], DecodeError> {
        let end = self
            .offset
            .checked_add(length)
            .filter(|end| *end <= self.bytes.len())
            .ok_or_else(|| DecodeError("unexpected end of CBOR input".into()))?;
        let value = &self.bytes[self.offset..end];
        self.offset = end;
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_map_round_trip() {
        let mut encoder = Encoder::new();
        encoder.map(3);
        encoder.u64(0);
        encoder.u64(42);
        encoder.u64(1);
        encoder.text("vivi");
        encoder.u64(2);
        encoder.bytes(&[1, 2, 3]);
        let bytes = encoder.into_vec();

        assert_eq!(bytes, b"\xa3\x00\x18\x2a\x01\x64vivi\x02\x43\x01\x02\x03");
        assert_eq!(
            decode(&bytes).unwrap().map_value(1).unwrap().as_text(),
            Some("vivi")
        );
    }

    #[test]
    fn rejects_unsorted_map_keys() {
        assert!(decode(&[0xa2, 0x01, 0x00, 0x00, 0x00]).is_err());
    }

    #[test]
    fn negative_integer_round_trip() {
        let mut encoder = Encoder::new();
        encoder.i64(i64::MIN);
        assert_eq!(decode(&encoder.into_vec()), Ok(Value::Negative(i64::MIN)));
    }

    #[test]
    fn generic_encoder_is_canonical_and_round_trips_null() {
        let value = Value::Map(vec![
            (0, Value::Unsigned(42)),
            (1, Value::Array(vec![Value::Bool(true), Value::Null])),
        ]);
        let bytes = encode(&value).unwrap();
        assert_eq!(bytes, [0xa2, 0x00, 0x18, 0x2a, 0x01, 0x82, 0xf5, 0xf6]);
        assert_eq!(decode(&bytes), Ok(value));
    }

    #[test]
    fn generic_encoder_rejects_unsorted_maps() {
        let value = Value::Map(vec![(1, Value::Unsigned(0)), (0, Value::Unsigned(0))]);
        assert!(encode(&value).is_err());
    }

    #[test]
    fn generic_encoder_reuses_caller_buffer() {
        let value = Value::Map(vec![(0, Value::Unsigned(42))]);
        let mut output = Vec::with_capacity(32);
        let allocation = output.as_ptr();
        encode_into(&mut output, &value).unwrap();
        assert_eq!(output, [0xa1, 0, 0x18, 42]);
        assert_eq!(output.as_ptr(), allocation);

        encode_into(&mut output, &Value::Bool(true)).unwrap();
        assert_eq!(output, [0xf5]);
        assert_eq!(output.as_ptr(), allocation);
    }

    #[test]
    fn rejects_container_lengths_before_large_allocation() {
        assert!(decode(&[0x99, 0x10, 0x01]).is_err());
        assert!(decode(&[0x99, 0x10, 0x00]).is_err());
        assert!(decode(&[0xb9, 0x10, 0x00]).is_err());
    }

    #[test]
    fn preserving_map_round_trips_interleaved_unknown_values_exactly() {
        let bytes = [
            0xa5, 0x00, 0x01, 0x02, 0x82, 0xf5, 0xf6, 0x04, 0x64, b'v', b'i', b'v', b'i', 0x07,
            0xa1, 0x00, 0x18, 0x2a, 0x09, 0x19, 0x10, 0x00,
        ];
        let decoded = decode_preserving_map(&bytes, &[0, 4, 9]).unwrap();
        assert_eq!(
            decoded.known_value(4).and_then(Value::as_text),
            Some("vivi")
        );
        assert_eq!(
            decoded
                .preserved()
                .iter()
                .map(|entry| (entry.key, entry.encoded_value))
                .collect::<Vec<_>>(),
            vec![(2, &bytes[4..7] as &[u8]), (7, &bytes[14..18] as &[u8])]
        );
        let known = vec![
            (0, decoded.known_value(0).unwrap().clone()),
            (4, decoded.known_value(4).unwrap().clone()),
            (9, decoded.known_value(9).unwrap().clone()),
        ];
        assert_eq!(
            encode_preserving_map(&known, &decoded.preserved_owned()).unwrap(),
            bytes
        );
    }

    #[test]
    fn preserving_map_rejects_malformed_unknown_value() {
        assert!(decode_preserving_map(&[0xa2, 0x00, 0x01, 0x02, 0x82, 0xf5], &[0]).is_err());
    }

    #[test]
    fn preserving_map_rejects_known_unknown_collisions_and_combined_limit() {
        let preserved = vec![PreservedField {
            key: 1,
            encoded_value: vec![0],
        }];
        assert!(encode_preserving_map(&[(1, Value::Unsigned(1))], &preserved).is_err());

        let too_many = (0..=MAX_CONTAINER_LENGTH)
            .map(|key| PreservedField {
                key: key as u64,
                encoded_value: vec![0],
            })
            .collect::<Vec<_>>();
        assert!(encode_preserving_map(&[], &too_many).is_err());
    }
}
