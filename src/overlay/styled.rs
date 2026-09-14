//! Portable styled paragraphs and atomic, optionally retained measurement batches.
use super::*;
use crate::vector::Color;

pub const MAX_TEXT_BATCH: usize = 32;
pub const MAX_TEXT_RUNS: usize = 64;
pub const MAX_RETAINED_LAYOUTS: usize = 128;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TextAlignment {
    #[default]
    Start,
    Center,
    End,
    Justify,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextStyle {
    pub size: Scalar,
    pub family: String,
    pub weight: u16,
    pub italic: bool,
    pub color: Color,
    pub underline: bool,
    pub strikethrough: bool,
}
impl Default for TextStyle {
    fn default() -> Self {
        Self {
            size: Scalar::new(16.).expect("16 logical pixels is a valid scalar"),
            family: String::new(),
            weight: 400,
            italic: false,
            color: Color(0xffffffff),
            underline: false,
            strikethrough: false,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextRun {
    pub text: String,
    pub style: TextStyle,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyledText {
    pub runs: Vec<TextRun>,
    pub max_width: Option<Scalar>,
    pub alignment: TextAlignment,
    pub wrap: bool,
    /// Clip after this many complete lines. None preserves all lines.
    pub max_lines: Option<u16>,
}
impl StyledText {
    pub fn new(text: impl Into<String>, style: TextStyle) -> Self {
        Self {
            runs: vec![TextRun {
                text: text.into(),
                style,
            }],
            max_width: None,
            alignment: TextAlignment::Start,
            wrap: true,
            max_lines: None,
        }
    }
    pub fn text(&self) -> String {
        self.runs.iter().map(|run| run.text.as_str()).collect()
    }
    pub fn validate(&self) -> Result<(), MessageError> {
        if self.runs.is_empty()
            || self.runs.len() > MAX_TEXT_RUNS
            || self.max_width.is_some_and(|w| w <= Scalar::ZERO)
            || self
                .max_lines
                .is_some_and(|n| n == 0 || usize::from(n) > MAX_TEXT_GEOMETRY)
        {
            return Err(bad(3, "invalid styled paragraph limits"));
        }
        let mut bytes = 0usize;
        for run in &self.runs {
            bytes = bytes
                .checked_add(run.text.len())
                .ok_or(bad(3, "text size overflow"))?;
            if bytes > MAX_MEASURE_TEXT_BYTES
                || run.style.size <= Scalar::ZERO
                || run.style.family.len() > 256
                || !(1..=1000).contains(&run.style.weight)
            {
                return Err(bad(3, "invalid or oversized styled text"));
            }
        }
        Ok(())
    }
    pub fn value(&self) -> Result<Value, MessageError> {
        self.validate()?;
        let runs = self
            .runs
            .iter()
            .map(|r| {
                Value::Array(vec![
                    Value::Text(r.text.clone()),
                    scalar(r.style.size),
                    Value::Text(r.style.family.clone()),
                    u(r.style.weight.into()),
                    Value::Bool(r.style.italic),
                    u(r.style.color.0.into()),
                    Value::Bool(r.style.underline),
                    Value::Bool(r.style.strikethrough),
                ])
            })
            .collect();
        Ok(Value::Array(vec![
            Value::Array(runs),
            self.max_width.map(scalar).unwrap_or(Value::Null),
            u(match self.alignment {
                TextAlignment::Start => 0,
                TextAlignment::Center => 1,
                TextAlignment::End => 2,
                TextAlignment::Justify => 3,
            }),
            Value::Bool(self.wrap),
            self.max_lines.map(|n| u(n.into())).unwrap_or(Value::Null),
        ]))
    }
    pub fn decode(value: &Value) -> Result<Self, MessageError> {
        let v = array::<5>(value, 3)?;
        let runs = v[0].as_array().ok_or(bad(3, "expected text runs"))?;
        if runs.is_empty() || runs.len() > MAX_TEXT_RUNS {
            return Err(bad(3, "text run limit"));
        }
        // Check aggregate string storage before cloning untrusted strings.
        let mut bytes = 0usize;
        for value in runs {
            let run = array::<8>(value, 3)?;
            let text = run[0].as_text().ok_or(bad(3, "expected run text"))?;
            let family = run[2].as_text().ok_or(bad(3, "expected font family"))?;
            bytes = bytes
                .checked_add(text.len())
                .ok_or(bad(3, "text size overflow"))?;
            if bytes > MAX_MEASURE_TEXT_BYTES || family.len() > 256 {
                return Err(bad(3, "text size limit"));
            }
        }
        let runs = runs
            .iter()
            .map(|value| {
                let r = array::<8>(value, 3)?;
                Ok(TextRun {
                    text: r[0].as_text().ok_or(bad(3, "expected run text"))?.into(),
                    style: TextStyle {
                        size: decode_scalar(&r[1], 3)?,
                        family: r[2].as_text().ok_or(bad(3, "expected font family"))?.into(),
                        weight: u16::try_from(small(&r[3], 3)?)
                            .map_err(|_| bad(3, "invalid font weight"))?,
                        italic: boolean(&r[4], 3)?,
                        color: Color(small(&r[5], 3)?),
                        underline: boolean(&r[6], 3)?,
                        strikethrough: boolean(&r[7], 3)?,
                    },
                })
            })
            .collect::<Result<Vec<_>, MessageError>>()?;
        let result = Self {
            runs,
            max_width: match &v[1] {
                Value::Null => None,
                v => Some(decode_scalar(v, 3)?),
            },
            alignment: match v[2].as_u64() {
                Some(0) => TextAlignment::Start,
                Some(1) => TextAlignment::Center,
                Some(2) => TextAlignment::End,
                Some(3) => TextAlignment::Justify,
                _ => return Err(bad(3, "invalid alignment")),
            },
            wrap: boolean(&v[3], 3)?,
            max_lines: match &v[4] {
                Value::Null => None,
                v => Some(u16::try_from(small(v, 3)?).map_err(|_| bad(3, "invalid line limit"))?),
            },
        };
        result.validate()?;
        Ok(result)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeasureBatch {
    pub address: WindowAddress,
    pub texts: Vec<StyledText>,
    pub retain: bool,
}
impl MeasureBatch {
    pub fn payload(&self) -> Result<PayloadMap, MessageError> {
        self.address.validate(self.address.surface_id)?;
        if self.texts.is_empty() || self.texts.len() > MAX_TEXT_BATCH {
            return Err(bad(3, "text batch count limit"));
        }
        let mut bytes = 0usize;
        let mut runs = 0usize;
        for text in &self.texts {
            text.validate()?;
            runs += text.runs.len();
            bytes += text.runs.iter().map(|r| r.text.len()).sum::<usize>();
        }
        if bytes > MAX_MEASURE_TEXT_BYTES || runs > MAX_TEXT_RUNS {
            return Err(bad(3, "aggregate batch limit"));
        }
        let mut payload = self.address.payload();
        payload.push((
            3,
            Value::Array(
                self.texts
                    .iter()
                    .map(StyledText::value)
                    .collect::<Result<_, _>>()?,
            ),
        ));
        payload.push((4, Value::Bool(self.retain)));
        Ok(payload)
    }
    pub fn decode(object: u64, value: &Value) -> Result<Self, MessageError> {
        let map = strict(value, &[0, 1, 2, 3, 4])?;
        let values = map
            .required(3)?
            .as_array()
            .ok_or(bad(3, "expected batch"))?;
        if values.is_empty() || values.len() > MAX_TEXT_BATCH {
            return Err(bad(3, "text batch count limit"));
        }
        let result = Self {
            address: WindowAddress::decode(object, &map)?,
            texts: values
                .iter()
                .map(StyledText::decode)
                .collect::<Result<_, _>>()?,
            retain: boolean(map.required(4)?, 4)?,
        };
        result.payload()?;
        Ok(result)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchMeasured {
    pub address: WindowAddress,
    pub layouts: Vec<(u64, TextMeasurement)>,
}
impl BatchMeasured {
    pub fn payload(&self) -> Result<PayloadMap, MessageError> {
        if self.layouts.is_empty() || self.layouts.len() > MAX_TEXT_BATCH {
            return Err(bad(3, "batch result limit"));
        }
        let mut payload = self.address.payload();
        let mut geometry = 0usize;
        let values = self
            .layouts
            .iter()
            .map(|(id, m)| {
                geometry += m.lines.len() + m.clusters.len();
                Ok(Value::Array(vec![
                    u(*id),
                    Value::Map(m.payload(self.address)?),
                ]))
            })
            .collect::<Result<_, MessageError>>()?;
        if geometry > MAX_TEXT_GEOMETRY {
            return Err(bad(3, "batch geometry limit"));
        }
        payload.push((3, Value::Array(values)));
        Ok(payload)
    }
    pub fn decode(address: WindowAddress, value: &Value) -> Result<Self, MessageError> {
        let map = strict(value, &[0, 1, 2, 3])?;
        if WindowAddress::decode(address.surface_id, &map)? != address {
            return Err(bad(0, "wrong layout window"));
        }
        let values = map
            .required(3)?
            .as_array()
            .ok_or(bad(3, "expected batch results"))?;
        if values.is_empty() || values.len() > MAX_TEXT_BATCH {
            return Err(bad(3, "batch result limit"));
        }
        let layouts = values
            .iter()
            .map(|v| {
                let a = array::<2>(v, 3)?;
                Ok((
                    a[0].as_u64().ok_or(bad(3, "invalid layout ID"))?,
                    TextMeasurement::decode(address, &a[1])?,
                ))
            })
            .collect::<Result<_, MessageError>>()?;
        let result = Self { address, layouts };
        result.payload()?;
        Ok(result)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseLayouts {
    pub address: WindowAddress,
    pub ids: Vec<u64>,
}
impl ReleaseLayouts {
    pub fn payload(&self) -> Result<PayloadMap, MessageError> {
        self.address.validate(self.address.surface_id)?;
        let mut ids = std::collections::BTreeSet::new();
        if self.ids.is_empty()
            || self.ids.len() > MAX_TEXT_BATCH
            || self.ids.iter().any(|id| *id == 0 || !ids.insert(*id))
        {
            return Err(bad(3, "invalid layout release IDs"));
        }
        let mut payload = self.address.payload();
        payload.push((3, Value::Array(self.ids.iter().copied().map(u).collect())));
        Ok(payload)
    }
    pub fn decode(object: u64, value: &Value) -> Result<Self, MessageError> {
        let map = strict(value, &[0, 1, 2, 3])?;
        let values = map
            .required(3)?
            .as_array()
            .ok_or(bad(3, "expected layout IDs"))?;
        if values.len() > MAX_TEXT_BATCH {
            return Err(bad(3, "release count limit"));
        }
        let result = Self {
            address: WindowAddress::decode(object, &map)?,
            ids: values
                .iter()
                .map(|v| v.as_u64().ok_or(bad(3, "invalid layout ID")))
                .collect::<Result<_, _>>()?,
        };
        result.payload()?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn batches_roundtrip_and_enforce_aggregate_limits() {
        let address = WindowAddress {
            context_id: 1,
            surface_id: 2,
            generation: 3,
        };
        let text = StyledText::new(
            "A😀e\u{301}",
            TextStyle {
                size: Scalar::new(16.).unwrap(),
                underline: true,
                ..Default::default()
            },
        );
        let request = MeasureBatch {
            address,
            texts: vec![text.clone(), text.clone()],
            retain: true,
        };
        assert_eq!(
            MeasureBatch::decode(2, &Value::Map(request.payload().unwrap())).unwrap(),
            request
        );
        assert!(
            MeasureBatch {
                texts: vec![text.clone(); MAX_TEXT_BATCH + 1],
                ..request.clone()
            }
            .payload()
            .is_err()
        );
        let large = StyledText::new("x".repeat(2049), TextStyle::default());
        assert!(
            MeasureBatch {
                texts: vec![large.clone(), large],
                ..request.clone()
            }
            .payload()
            .is_err()
        );
        let mut invalid = text;
        invalid.max_lines = Some(0);
        assert!(invalid.value().is_err());
        let release = ReleaseLayouts {
            address,
            ids: vec![u64::MAX, 1],
        };
        assert_eq!(
            ReleaseLayouts::decode(2, &Value::Map(release.payload().unwrap())).unwrap(),
            release
        );
        assert!(
            ReleaseLayouts {
                ids: vec![1, 1],
                ..release
            }
            .payload()
            .is_err()
        );
        let mut canvas = Canvas::new();
        canvas
            .push(Command::TextLayout {
                layout: u64::MAX,
                origin: Point::new(1., 2.).unwrap(),
            })
            .unwrap();
        assert_eq!(Canvas::decode(&canvas.encode().unwrap()).unwrap(), canvas);
    }
}
