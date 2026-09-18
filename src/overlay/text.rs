//! Optional host text measurements and revision-bound platform editor placement.
use super::*;
use crate::vector::{Canvas, Command, Text};

#[path = "styled.rs"]
pub mod styled;

pub const MAX_MEASURE_TEXT_BYTES: usize = 4096;
pub const MAX_TEXT_GEOMETRY: usize = 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeasureText {
    pub address: WindowAddress,
    pub text: Text,
}
impl MeasureText {
    pub fn payload(&self) -> Result<PayloadMap, MessageError> {
        self.address.validate(self.address.surface_id)?;
        if self.text.text.len() > MAX_MEASURE_TEXT_BYTES {
            return Err(bad(3, "measurement text exceeds limit"));
        }
        let mut canvas = Canvas::new();
        canvas
            .push(Command::Text(self.text.clone()))
            .map_err(|e| bad(3, e.0))?;
        let bytes = canvas.encode().map_err(|e| bad(3, e.0))?;
        let value = crate::cbor::decode(&bytes).map_err(|_| bad(3, "invalid text"))?;
        let mut payload = self.address.payload();
        payload.push((3, value));
        Ok(payload)
    }
    pub fn decode(object: u64, value: &Value) -> Result<Self, MessageError> {
        let map = strict(value, &[0, 1, 2, 3])?;
        let address = WindowAddress::decode(object, &map)?;
        let value = map.required(3)?;
        if value.as_array().is_none_or(|v| v.len() != 1) {
            return Err(bad(3, "expected one text specification"));
        }
        if value
            .as_array()
            .and_then(|v| v[0].as_array())
            .and_then(|v| v.get(1))
            .and_then(Value::as_text)
            .is_none_or(|text| text.len() > MAX_MEASURE_TEXT_BYTES)
        {
            return Err(bad(3, "measurement text exceeds limit or is absent"));
        }
        let canvas = Canvas::from_value(value).map_err(|e| bad(3, e.0))?;
        let [Command::Text(text)] = canvas.commands() else {
            return Err(bad(3, "expected text specification"));
        };
        let result = Self {
            address,
            text: text.clone(),
        };
        result.payload()?;
        Ok(result)
    }
}

/// UTF-8 byte range and logical layout geometry. Zero advances (combining clusters) are valid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextGeometry {
    pub start: u32,
    pub end: u32,
    pub x: Scalar,
    pub y: Scalar,
    pub width: Scalar,
    pub height: Scalar,
    /// Baseline for a line; zero for a cluster.
    pub baseline: Scalar,
    pub rtl: bool,
}
impl TextGeometry {
    fn value(&self) -> Value {
        Value::Array(vec![
            u(self.start.into()),
            u(self.end.into()),
            scalar(self.x),
            scalar(self.y),
            scalar(self.width),
            scalar(self.height),
            scalar(self.baseline),
            Value::Bool(self.rtl),
        ])
    }
    fn decode(value: &Value) -> Result<Self, MessageError> {
        let v = array::<8>(value, 4)?;
        let result = Self {
            start: small(&v[0], 4)?,
            end: small(&v[1], 4)?,
            x: decode_scalar(&v[2], 4)?,
            y: decode_scalar(&v[3], 4)?,
            width: decode_scalar(&v[4], 4)?,
            height: decode_scalar(&v[5], 4)?,
            baseline: decode_scalar(&v[6], 4)?,
            rtl: boolean(&v[7], 4)?,
        };
        if result.start > result.end
            || result.end as usize > MAX_MEASURE_TEXT_BYTES
            || result.width < Scalar::ZERO
            || result.height < Scalar::ZERO
        {
            return Err(bad(4, "invalid text geometry"));
        }
        Ok(result)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextMeasurement {
    /// Original UTF-8 cutoff for an ellipsized layout; synthetic marker ranges are empty here.
    pub truncated_at: Option<u32>,
    pub width: Scalar,
    pub height: Scalar,
    pub lines: Vec<TextGeometry>,
    pub clusters: Vec<TextGeometry>,
}
impl TextMeasurement {
    pub fn payload(&self, address: WindowAddress) -> Result<PayloadMap, MessageError> {
        if self.lines.len() > MAX_TEXT_GEOMETRY || self.clusters.len() > MAX_TEXT_GEOMETRY {
            return Err(bad(4, "text geometry limit exceeded"));
        }
        address.validate(address.surface_id)?;
        let mut values = address.payload();
        values.extend([
            (
                3,
                Value::Array(vec![scalar(self.width), scalar(self.height)]),
            ),
            (
                4,
                Value::Array(self.lines.iter().map(TextGeometry::value).collect()),
            ),
            (
                5,
                Value::Array(self.clusters.iter().map(TextGeometry::value).collect()),
            ),
        ]);
        if let Some(cut) = self.truncated_at {
            values.push((6, u(cut.into())));
        }
        Self::decode(address, &Value::Map(values.clone()))?;
        Ok(values)
    }
    pub fn decode(address: WindowAddress, value: &Value) -> Result<Self, MessageError> {
        let map = strict(value, &[0, 1, 2, 3, 4, 5, 6])?;
        if WindowAddress::decode(address.surface_id, &map)? != address {
            return Err(bad(0, "measurement belongs to another window"));
        }
        let extent = array::<2>(map.required(3)?, 3)?;
        let width = decode_scalar(&extent[0], 3)?;
        let height = decode_scalar(&extent[1], 3)?;
        if width < Scalar::ZERO || height < Scalar::ZERO {
            return Err(bad(3, "negative text extent"));
        }
        let geometry = |key| -> Result<Vec<TextGeometry>, MessageError> {
            let values = map
                .required(key)?
                .as_array()
                .ok_or(bad(key, "expected geometry array"))?;
            if values.len() > MAX_TEXT_GEOMETRY {
                return Err(bad(key, "text geometry limit exceeded"));
            }
            values.iter().map(TextGeometry::decode).collect()
        };
        Ok(Self {
            truncated_at: map
                .optional(6)
                .map(|v| small(v, 6))
                .transpose()?
                .map(|cut| {
                    if cut as usize > MAX_MEASURE_TEXT_BYTES {
                        Err(bad(6, "invalid truncation offset"))
                    } else {
                        Ok(cut)
                    }
                })
                .transpose()?,
            width,
            height,
            lines: geometry(4)?,
            clusters: geometry(5)?,
        })
    }
    pub fn validate_text(&self, text: &str) -> Result<(), MessageError> {
        if self.truncated_at.is_some_and(|cut| {
            !text.is_char_boundary(cut as usize)
                || self.lines.iter().chain(&self.clusters).any(|g| g.end > cut)
        }) {
            return Err(bad(6, "invalid truncation geometry"));
        }
        if self.lines.iter().chain(&self.clusters).any(|g| {
            !text.is_char_boundary(g.start as usize) || !text.is_char_boundary(g.end as usize)
        }) {
            return Err(bad(4, "text range is not a UTF-8 boundary"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EditorGeometry {
    pub address: WindowAddress,
    pub scene_revision: u64,
    /// Window-local logical caret/exclusion rectangle, or None to clear it.
    pub caret: Option<Rect>,
}
impl EditorGeometry {
    pub fn payload(self) -> Result<PayloadMap, MessageError> {
        self.address.validate(self.address.surface_id)?;
        nonzero(self.scene_revision, 3)?;
        if let Some(caret) = self.caret {
            caret.validate().map_err(|e| bad(4, e.0))?;
        }
        let mut values = self.address.payload();
        values.extend([
            (3, u(self.scene_revision)),
            (4, self.caret.map(rectangle).unwrap_or(Value::Null)),
        ]);
        Ok(values)
    }
    pub fn decode(object: u64, value: &Value) -> Result<Self, MessageError> {
        let map = strict(value, &[0, 1, 2, 3, 4])?;
        Ok(Self {
            address: WindowAddress::decode(object, &map)?,
            scene_revision: nonzero(map.required_u64(3)?, 3)?,
            caret: match map.required(4)? {
                Value::Null => None,
                v => Some(decode_rectangle(v, 4)?),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request() -> MeasureText {
        MeasureText {
            address: WindowAddress {
                context_id: 1,
                surface_id: 2,
                generation: 3,
            },
            text: Text {
                text: "A😀日".into(),
                origin: Point::new(0., 0.).unwrap(),
                size: Scalar::new(16.).unwrap(),
                family: String::new(),
                weight: 400,
                italic: false,
                color: crate::vector::Color(0xffffffff),
                max_width: None,
            },
        }
    }
    #[test]
    fn measurement_and_editor_roundtrip_and_reject_malformed_input() {
        let request = request();
        let value = Value::Map(request.payload().unwrap());
        assert_eq!(MeasureText::decode(2, &value).unwrap(), request);
        assert!(MeasureText::decode(3, &value).is_err());
        let mut oversized = request.clone();
        oversized.text.text = "a".repeat(MAX_MEASURE_TEXT_BYTES + 1);
        assert!(oversized.payload().is_err());
        let mut duplicate = request.payload().unwrap();
        duplicate.push((3, Value::Null));
        assert!(MeasureText::decode(2, &Value::Map(duplicate)).is_err());
        for caret in [None, Some(Rect::new(1., 2., 1., 16.).unwrap())] {
            let editor = EditorGeometry {
                address: request.address,
                scene_revision: u64::MAX,
                caret,
            };
            assert_eq!(
                EditorGeometry::decode(2, &Value::Map(editor.payload().unwrap())).unwrap(),
                editor
            );
        }
        assert!(
            EditorGeometry {
                address: request.address,
                scene_revision: 0,
                caret: None
            }
            .payload()
            .is_err()
        );
    }
    #[test]
    fn measurements_reject_wrong_owner_geometry_limits_and_unicode_offsets() {
        let request = request();
        let geometry = TextGeometry {
            start: 1,
            end: 5,
            x: Scalar::ZERO,
            y: Scalar::ZERO,
            width: Scalar::ONE,
            height: Scalar::ONE,
            baseline: Scalar::ZERO,
            rtl: false,
        };
        let mut measured = TextMeasurement {
            truncated_at: None,
            width: Scalar::ONE,
            height: Scalar::ONE,
            lines: vec![],
            clusters: vec![geometry],
        };
        measured.validate_text(&request.text.text).unwrap();
        let value = Value::Map(measured.payload(request.address).unwrap());
        assert_eq!(
            TextMeasurement::decode(request.address, &value).unwrap(),
            measured
        );
        let wrong = WindowAddress {
            context_id: 8,
            ..request.address
        };
        assert!(TextMeasurement::decode(wrong, &value).is_err());
        measured.clusters[0].end = 2;
        assert!(measured.validate_text(&request.text.text).is_err());
        measured.clusters = vec![measured.clusters[0].clone(); MAX_TEXT_GEOMETRY + 1];
        assert!(measured.payload(request.address).is_err());
    }
}
