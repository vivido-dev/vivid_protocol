use std::fmt::{self, Display, Formatter};

const MAX_DEPTH: usize = 16;
const MAX_VALUE_LENGTH: usize = 16 * 1024 * 1024;
const MAX_CONTAINER_LENGTH: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
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

    pub fn into_vec(self) -> Vec<u8> {
        self.bytes
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
    let mut decoder = Decoder { bytes, offset: 0 };
    let value = decoder.value(0)?;
    if decoder.offset != bytes.len() {
        return Err(DecodeError("trailing bytes after CBOR value".into()));
    }
    Ok(value)
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
                let mut values = Vec::with_capacity(length);
                for _ in 0..length {
                    values.push(self.value(depth + 1)?);
                }
                Ok(Value::Array(values))
            }
            5 => {
                let length = self.container_length(additional, 2)?;
                let mut entries = Vec::with_capacity(length);
                let mut previous = None;
                for _ in 0..length {
                    let key = match self.value(depth + 1)? {
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
                    entries.push((key, self.value(depth + 1)?));
                }
                Ok(Value::Map(entries))
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
    fn rejects_container_lengths_before_large_allocation() {
        assert!(decode(&[0x99, 0x10, 0x01]).is_err());
        assert!(decode(&[0x99, 0x10, 0x00]).is_err());
        assert!(decode(&[0xb9, 0x10, 0x00]).is_err());
    }
}
