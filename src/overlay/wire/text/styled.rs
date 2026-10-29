//! Styled paragraphs and batched, optionally retained measurement (`overlay-text-layout-v1`).
//!
//! A [`StyledText`] paragraph concatenates [`TextRun`]s, each with its own [`TextStyle`], and may
//! carry paragraph [`Typography`] under `overlay-typography-v1`. [`MeasureBatch`] measures up to
//! [`MAX_TEXT_BATCH`] paragraphs atomically; when it asks to retain them, the host keeps each
//! layout so a scene can draw it without reshaping, until [`ReleaseLayouts`] frees it.
use super::{
    MAX_MEASURE_TEXT_BYTES, MAX_TEXT_GEOMETRY, MessageError, PayloadMap, Scalar, TextMeasurement,
    Value, WindowAddress, array, bad, boolean, decode_scalar, scalar, small, strict, u,
};
use crate::vector::Color;

/// Longest font family name, in bytes.
///
/// Family names are bounded everywhere they cross the wire, including in the environment snapshot a
/// host sends.
pub const MAX_FAMILY_BYTES: usize = 256;

/// Most paragraphs in one batch measurement.
pub const MAX_TEXT_BATCH: usize = 32;
/// Most runs in one paragraph, and in one batch in total.
pub const MAX_TEXT_RUNS: usize = 64;
/// Most retained text layouts one window may hold.
pub const MAX_RETAINED_LAYOUTS: usize = 128;

/// What happens to text that does not fit.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum TextOverflow {
    /// Clip it (`0`).
    #[default]
    Clip,
    /// End with an ellipsis at a grapheme boundary (`1`).
    Ellipsis,
}

/// Paragraph-wide typography controls (`overlay-typography-v1`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Typography {
    /// Clip or ellipsis; ellipsis needs a finite maximum width.
    pub overflow: TextOverflow,
    /// Extra space between letters, at most 1024 logical pixels.
    pub letter_spacing: Scalar,
    /// Extra space between words, at most 1024 logical pixels.
    pub word_spacing: Scalar,
    /// Absolute line height up to 4096 logical pixels, or `None` for font metrics.
    pub line_height: Option<Scalar>,
    /// Whether optional ligatures (`liga`, `clig`, `dlig`, `calt`) apply.
    pub ligatures: bool,
    /// Whether kerning (`kern`) applies.
    pub kerning: bool,
}
impl Default for Typography {
    fn default() -> Self {
        Self {
            overflow: TextOverflow::Clip,
            letter_spacing: Scalar::ZERO,
            word_spacing: Scalar::ZERO,
            line_height: None,
            ligatures: true,
            kerning: true,
        }
    }
}
impl Typography {
    /// Checks that spacing is within 0 through 1,024 and any line height within (0, 4,096].
    ///
    /// # Errors
    ///
    /// Returns [`MessageError::InvalidValue`] when letter or word spacing is negative or above
    /// 1,024, or the line height is not positive or above 4,096.
    pub fn validate(&self) -> Result<(), MessageError> {
        if [self.letter_spacing, self.word_spacing]
            .iter()
            .any(|s| s.get() < 0. || s.get() > 1024.)
            || self
                .line_height
                .is_some_and(|h| h.get() <= 0. || h.get() > 4096.)
        {
            return Err(bad(3, "invalid typographic spacing or line height"));
        }
        Ok(())
    }
    fn value(&self) -> Value {
        Value::Array(vec![
            u(u64::from(self.overflow != TextOverflow::Clip)),
            scalar(self.letter_spacing),
            scalar(self.word_spacing),
            self.line_height.map_or(Value::Null, scalar),
            Value::Bool(self.ligatures),
            Value::Bool(self.kerning),
        ])
    }
    fn decode(value: &Value) -> Result<Self, MessageError> {
        let v = array::<6>(value, 3)?;
        let result = Self {
            overflow: match v[0].as_u64() {
                Some(0) => TextOverflow::Clip,
                Some(1) => TextOverflow::Ellipsis,
                _ => return Err(bad(3, "invalid text overflow")),
            },
            letter_spacing: decode_scalar(&v[1], 3)?,
            word_spacing: decode_scalar(&v[2], 3)?,
            line_height: match &v[3] {
                Value::Null => None,
                v => Some(decode_scalar(v, 3)?),
            },
            ligatures: boolean(&v[4], 3)?,
            kerning: boolean(&v[5], 3)?,
        };
        result.validate()?;
        Ok(result)
    }
}

/// How a paragraph's lines align.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum TextAlignment {
    /// Aligned to the paragraph's starting edge (`0`).
    #[default]
    Start,
    /// Centered (`1`).
    Center,
    /// Aligned to the paragraph's ending edge (`2`).
    End,
    /// Stretched to the full width (`3`).
    Justify,
}

/// The style of one run of styled text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextStyle {
    /// Positive font size in logical pixels.
    pub size: Scalar,
    /// Font family of at most 256 bytes; empty selects the host default.
    pub family: String,
    /// Font weight, 1 through 1000.
    pub weight: u16,
    /// Uses an italic face.
    pub italic: bool,
    /// Straight-alpha sRGB color, `0xRRGGBBAA`.
    pub color: Color,
    /// Draws an underline from the shaped font's metrics.
    pub underline: bool,
    /// Draws a strikethrough from the shaped font's metrics.
    pub strikethrough: bool,
}
impl Default for TextStyle {
    fn default() -> Self {
        Self {
            size: Scalar::new(16.).expect("16 logical pixels is a valid scalar"),
            family: String::new(),
            weight: 400,
            italic: false,
            color: Color(0xff_ff_ff_ff),
            underline: false,
            strikethrough: false,
        }
    }
}
/// One styled run; runs concatenate into one shaping input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextRun {
    /// UTF-8 text.
    pub text: String,
    /// The run's style.
    pub style: TextStyle,
}

/// One styled paragraph for batch measurement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyledText {
    /// Paragraph typography; nondefault values need `overlay-typography-v1`.
    pub typography: Typography,
    /// One to 64 runs.
    pub runs: Vec<TextRun>,
    /// Positive paragraph width, or `None` for unconstrained.
    pub max_width: Option<Scalar>,
    /// Line alignment.
    pub alignment: TextAlignment,
    /// Whether lines soft-wrap; explicit line breaks always apply.
    pub wrap: bool,
    /// Clip after this many complete lines. None preserves all lines.
    pub max_lines: Option<u16>,
}
impl StyledText {
    /// Creates a single-run paragraph that wraps, starts aligned, and has no width limit.
    pub fn new(text: impl Into<String>, style: TextStyle) -> Self {
        Self {
            typography: Typography::default(),
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
    /// Returns the runs' text concatenated, the string geometry offsets index into.
    pub fn text(&self) -> String {
        self.runs.iter().map(|run| run.text.as_str()).collect()
    }
    /// Checks the paragraph's typography, runs, and layout limits.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError::InvalidValue`] when the typography is invalid, ellipsis is requested
    /// without a maximum width, there are no runs or too many, the maximum width or line count is
    /// out of range, the text exceeds the byte ceiling, or a run has a non-positive size, an
    /// over-long font family, or a weight outside 1 through 1,000.
    pub fn validate(&self) -> Result<(), MessageError> {
        self.typography.validate()?;
        if self.typography.overflow == TextOverflow::Ellipsis && self.max_width.is_none() {
            return Err(bad(3, "ellipsis requires a maximum width"));
        }
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
    /// Encodes the paragraph as its CBOR array.
    ///
    /// # Errors
    ///
    /// Returns any error from [`StyledText::validate`].
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
        let mut values = vec![
            Value::Array(runs),
            self.max_width.map_or(Value::Null, scalar),
            u(match self.alignment {
                TextAlignment::Start => 0,
                TextAlignment::Center => 1,
                TextAlignment::End => 2,
                TextAlignment::Justify => 3,
            }),
            Value::Bool(self.wrap),
            self.max_lines.map_or(Value::Null, |n| u(n.into())),
        ];
        if self.typography != Typography::default() {
            values.push(self.typography.value());
        }
        Ok(Value::Array(values))
    }
    /// Decodes a paragraph from its CBOR array.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError`] when `value` is not a five- or six-element paragraph array, a run
    /// is malformed, the text exceeds the byte ceiling, or the result fails
    /// [`StyledText::validate`].
    pub fn decode(value: &Value) -> Result<Self, MessageError> {
        let v = value
            .as_array()
            .filter(|v| v.len() == 5 || v.len() == 6)
            .ok_or(bad(3, "invalid paragraph shape"))?;
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
            typography: v
                .get(5)
                .map(Typography::decode)
                .transpose()?
                .unwrap_or_default(),
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

/// A `MEASURE_OVERLAY_TEXT_BATCH` request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeasureBatch {
    /// The window (keys 0 through 2).
    pub address: WindowAddress,
    /// One to 32 paragraphs (key 3).
    pub texts: Vec<StyledText>,
    /// Whether the host retains each measured layout for drawing (key 4).
    pub retain: bool,
}
impl MeasureBatch {
    /// Encodes the `MEASURE_OVERLAY_TEXT_BATCH` payload.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError`] when the window address is invalid, the batch is empty or over the
    /// batch limit, a paragraph fails [`StyledText::validate`], or the batch's aggregate text
    /// exceeds its ceiling.
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
    /// Decodes a `MEASURE_OVERLAY_TEXT_BATCH` payload.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError`] when `value` is not a strict map, the batch is not an array or is
    /// over the batch limit, the window address fails against `object`, a paragraph is malformed,
    /// or the batch fails [`MeasureBatch::payload`].
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

/// The `OVERLAY_TEXT_BATCH_MEASURED` reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchMeasured {
    /// The window (keys 0 through 2).
    pub address: WindowAddress,
    /// One `(layout ID, measurement)` per paragraph, in request order; IDs are zero unless retained
    /// (key 3).
    pub layouts: Vec<(u64, TextMeasurement)>,
}
impl BatchMeasured {
    /// Encodes the `OVERLAY_TEXT_BATCH_MEASURED` payload.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError`] when the result list is empty or over the batch limit, a
    /// measurement is invalid, or the combined geometry exceeds its ceiling.
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
    /// Decodes an `OVERLAY_TEXT_BATCH_MEASURED` payload for the window at `address`.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError`] when `value` is not a strict map, names another window, the results
    /// are not an array of layout ID and measurement pairs, the list is empty or over the batch
    /// limit, or the batch fails [`BatchMeasured::payload`].
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

/// A `RELEASE_OVERLAY_TEXT_LAYOUTS` request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseLayouts {
    /// The window (keys 0 through 2).
    pub address: WindowAddress,
    /// One to 32 distinct, nonzero layout IDs (key 3).
    pub ids: Vec<u64>,
}
impl ReleaseLayouts {
    /// Encodes the `RELEASE_OVERLAY_TEXT_LAYOUTS` payload.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError`] when the window address is invalid, or the ID list is empty, over
    /// the batch limit, or holds a zero or duplicate ID.
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
    /// Decodes a `RELEASE_OVERLAY_TEXT_LAYOUTS` payload.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError`] when `value` is not a strict map, the IDs are not an array of
    /// unsigned integers, the list exceeds the release limit, the window address fails against
    /// `object`, or the list fails [`ReleaseLayouts::payload`].
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
