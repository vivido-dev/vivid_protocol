//! Shared fixed-point geometry for target-profile scene placement and desktop coordinate mapping.
//!
//! Every presentation-target profile defines its own node-geometry map, but they share the same
//! signed 32.32 arithmetic, the same clip-map shape, the same fit rules, and — for the desktop and
//! canvas targets — the same coordinate-space discriminator. Native presenters, browser
//! presenters, and producers that use this module project geometry identically.
//!
//! All arithmetic at a trust boundary is checked. Nothing here saturates.

use crate::{
    cbor::Value,
    messages::{MessageError, PayloadMap, StrictMap, invalid_value},
    scene::Fit,
};

/// One unit of a signed 32.32 fixed-point coordinate.
pub const FIXED_ONE: i64 = 1 << 32;

/// Coordinate space of a target-profile geometry map.
///
/// Terminal geometry uses its own discriminator with different meanings; this type covers the
/// desktop and canvas targets, where integer one is one target logical pixel (`TargetLogical`) or
/// the complete current target extent (`NormalizedTarget`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
#[non_exhaustive]
pub enum CoordinateSpace {
    /// Target logical pixels (`1`).
    TargetLogical = 1,
    /// Coordinates normalized to the target extent (`2`).
    NormalizedTarget = 2,
}

impl TryFrom<u64> for CoordinateSpace {
    type Error = MessageError;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::TargetLogical),
            2 => Ok(Self::NormalizedTarget),
            _ => Err(invalid_value(
                "node geometry",
                0,
                "has an unknown coordinate space",
            )),
        }
    }
}

/// A rectangle in signed 32.32 fixed point. Width and height are strictly positive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FixedRect {
    /// Left edge, signed 32.32.
    pub x: i64,
    /// Top edge, signed 32.32.
    pub y: i64,
    /// Positive width, signed 32.32.
    pub width: i64,
    /// Positive height, signed 32.32.
    pub height: i64,
}

impl FixedRect {
    /// Creates a rectangle from signed 32.32 values.
    pub const fn new(x: i64, y: i64, width: i64, height: i64) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// The rectangle covering a whole normalized target.
    pub const fn unit() -> Self {
        Self::new(0, 0, FIXED_ONE, FIXED_ONE)
    }

    fn validate(&self, schema: &'static str) -> Result<(), MessageError> {
        if self.width <= 0 {
            return Err(invalid_value(schema, 3, "width must be positive"));
        }
        if self.height <= 0 {
            return Err(invalid_value(schema, 4, "height must be positive"));
        }
        // Origin plus extent must not overflow, so downstream transforms cannot wrap.
        self.x
            .checked_add(self.width)
            .ok_or_else(|| invalid_value(schema, 1, "origin plus width overflows"))?;
        self.y
            .checked_add(self.height)
            .ok_or_else(|| invalid_value(schema, 2, "origin plus height overflows"))?;
        Ok(())
    }
}

/// Desktop- and canvas-target node geometry: the core map at `CREATE_NODE` key 4.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NodeGeometry {
    /// Target logical pixels or normalized target coordinates (key 0).
    pub space: CoordinateSpace,
    /// The rectangle in that space (keys 1 through 4).
    pub rect: FixedRect,
}

impl NodeGeometry {
    /// A node covering the complete target, which is what a full-screen desktop uses.
    pub const fn full_target() -> Self {
        Self {
            space: CoordinateSpace::NormalizedTarget,
            rect: FixedRect::unit(),
        }
    }

    /// Decodes a scene node's geometry map.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError`] when `map` is not a strict geometry map, a field is missing or has
    /// the wrong type, the coordinate space is unregistered, the width or height is not positive,
    /// an edge overflows, or normalized geometry leaves the unit square.
    pub fn decode(map: &PayloadMap) -> Result<Self, MessageError> {
        let value = Value::Map(map.clone());
        let strict = StrictMap::new("node geometry", &value, &[0, 1, 2, 3, 4])?;
        let space = CoordinateSpace::try_from(strict.required_u64(0)?)?;
        let rect = FixedRect {
            x: required_i64(&strict, 1, "node geometry")?,
            y: required_i64(&strict, 2, "node geometry")?,
            width: required_i64(&strict, 3, "node geometry")?,
            height: required_i64(&strict, 4, "node geometry")?,
        };
        rect.validate("node geometry")?;
        if space == CoordinateSpace::NormalizedTarget {
            // Normalized geometry outside the unit square would project outside the target on
            // every generation, so reject it before it can become a scene node.
            let right = rect.x.saturating_add(rect.width);
            let bottom = rect.y.saturating_add(rect.height);
            if rect.x < 0 || rect.y < 0 || right > FIXED_ONE || bottom > FIXED_ONE {
                return Err(invalid_value(
                    "node geometry",
                    0,
                    "normalized geometry leaves the unit square",
                ));
            }
        }
        Ok(Self { space, rect })
    }

    /// Encodes the geometry as its node key 4 map.
    pub fn encode(&self) -> PayloadMap {
        vec![
            (0, Value::Unsigned(self.space as u64)),
            (1, Value::from(self.rect.x)),
            (2, Value::from(self.rect.y)),
            (3, Value::from(self.rect.width)),
            (4, Value::from(self.rect.height)),
        ]
    }

    /// Project into target logical pixels for the current target extent.
    ///
    /// Normalized geometry changes its pixel projection when the target generation changes, which
    /// is why a scene transaction carries the expected target generation.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError::InvalidValue`] when the normalized projection overflows or projects
    /// to an empty extent.
    pub fn project(&self, target: TargetExtent) -> Result<FixedRect, MessageError> {
        match self.space {
            CoordinateSpace::TargetLogical => Ok(self.rect),
            CoordinateSpace::NormalizedTarget => {
                let width = i64::from(target.width);
                let height = i64::from(target.height);
                Ok(FixedRect {
                    x: scale_normalized(self.rect.x, width)?,
                    y: scale_normalized(self.rect.y, height)?,
                    width: positive(scale_normalized(self.rect.width, width)?, 3)?,
                    height: positive(scale_normalized(self.rect.height, height)?, 4)?,
                })
            }
        }
    }
}

/// The current extent of a presentation target, in target logical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TargetExtent {
    /// Target width in logical pixels.
    pub width: u32,
    /// Target height in logical pixels.
    pub height: u32,
}

impl TargetExtent {
    /// Creates an extent.
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }
}

/// Decode the optional clip map, which has the same shape in every target profile.
///
/// # Errors
///
/// Returns [`MessageError`] when `map` is not a strict clip map, a field is missing or has the
/// wrong type, the width or height is not positive, or an edge overflows.
pub fn decode_clip(map: &PayloadMap) -> Result<FixedRect, MessageError> {
    let value = Value::Map(map.clone());
    let strict = StrictMap::new("node clip", &value, &[0, 1, 2, 3])?;
    let rect = FixedRect {
        x: required_i64(&strict, 0, "node clip")?,
        y: required_i64(&strict, 1, "node clip")?,
        width: required_i64(&strict, 2, "node clip")?,
        height: required_i64(&strict, 3, "node clip")?,
    };
    // The clip schema numbers x/y/width/height as 0..=3, so report against those keys.
    if rect.width <= 0 {
        return Err(invalid_value("node clip", 2, "width must be positive"));
    }
    if rect.height <= 0 {
        return Err(invalid_value("node clip", 3, "height must be positive"));
    }
    rect.x
        .checked_add(rect.width)
        .ok_or_else(|| invalid_value("node clip", 0, "origin plus width overflows"))?;
    rect.y
        .checked_add(rect.height)
        .ok_or_else(|| invalid_value("node clip", 1, "origin plus height overflows"))?;
    Ok(rect)
}

/// Encodes a node clip rectangle as its node key 11 map.
pub fn encode_clip(rect: FixedRect) -> PayloadMap {
    vec![
        (0, Value::from(rect.x)),
        (1, Value::from(rect.y)),
        (2, Value::from(rect.width)),
        (3, Value::from(rect.height)),
    ]
}

/// A media source's pixel size and sample aspect ratio, the input to [`fit_quad`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceSize {
    /// Width in source pixels.
    pub width: u32,
    /// Height in source pixels.
    pub height: u32,
    /// Sample aspect ratio numerator: how much wider than tall one pixel displays.
    pub aspect_numerator: u32,
    /// Sample aspect ratio denominator; `1` for square pixels.
    pub aspect_denominator: u32,
}

/// Fits a media quad of `source` into `destination`, returning `None` for an empty source.
///
/// The sample aspect ratio widens or narrows the source before fitting, so non-square pixels are
/// presented correctly. The result is the quad the presenter draws before clipping.
pub fn fit_quad(destination: FixedRect, source: SourceSize, fit: Fit) -> Option<FixedRect> {
    let SourceSize {
        width: source_width,
        height: source_height,
        aspect_numerator,
        aspect_denominator,
    } = source;
    if source_width == 0 || source_height == 0 || aspect_numerator == 0 || aspect_denominator == 0 {
        return None;
    }
    // Display width in fixed point, widened by the sample aspect ratio.
    let display_width = i64::from(source_width)
        .checked_mul(i64::from(aspect_numerator))?
        .checked_mul(FIXED_ONE)?
        .checked_div(i64::from(aspect_denominator))?;
    let display_height = i64::from(source_height).checked_mul(FIXED_ONE)?;

    let (width, height) = match fit {
        Fit::Fill => (destination.width, destination.height),
        Fit::None => (display_width, display_height),
        Fit::Contain | Fit::Cover => {
            // The scale is a rational, and a rational is not ordered by its (numerator,
            // denominator) tuple. Compare cross-products instead, in a width that cannot overflow.
            let by_width = Ratio::new(destination.width, display_width)?;
            let by_height = Ratio::new(destination.height, display_height)?;
            let selected = match fit {
                Fit::Contain => by_width.min(by_height),
                _ => by_width.max(by_height),
            };
            (
                selected.apply(display_width)?,
                selected.apply(display_height)?,
            )
        }
    };
    if width <= 0 || height <= 0 {
        return None;
    }
    // Centre the quad inside the destination.
    let x = destination
        .x
        .checked_add(destination.width.checked_sub(width)? / 2)?;
    let y = destination
        .y
        .checked_add(destination.height.checked_sub(height)? / 2)?;
    Some(FixedRect::new(x, y, width, height))
}

/// Intersect a quad with a clip rectangle, returning `None` when nothing remains visible.
pub fn intersect(first: FixedRect, second: FixedRect) -> Option<FixedRect> {
    let left = first.x.max(second.x);
    let top = first.y.max(second.y);
    let right = first
        .x
        .checked_add(first.width)?
        .min(second.x.checked_add(second.width)?);
    let bottom = first
        .y
        .checked_add(first.height)?
        .min(second.y.checked_add(second.height)?);
    if left >= right || top >= bottom {
        return None;
    }
    Some(FixedRect::new(left, top, right - left, bottom - top))
}

/// Clockwise rotation of a presentation or capture target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum Rotation {
    /// No rotation.
    None = 0,
    /// 90 degrees clockwise.
    Ninety = 90,
    /// 180 degrees.
    OneEighty = 180,
    /// 270 degrees clockwise.
    TwoSeventy = 270,
}

impl TryFrom<u64> for Rotation {
    type Error = MessageError;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::None),
            90 => Ok(Self::Ninety),
            180 => Ok(Self::OneEighty),
            270 => Ok(Self::TwoSeventy),
            _ => Err(invalid_value(
                "rotation",
                0,
                "must be 0, 90, 180, or 270 degrees",
            )),
        }
    }
}

/// The mapping between a canonical desktop surface and the producer's OS coordinate space.
///
/// Desktop §2 puts the canonical surface at origin `(0,0)` unrotated, and desktop §7 requires the
/// producer to apply rotation, captured origin, and output scale under the named surface
/// generation. This type is that transform, with every step checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SurfaceMapping {
    /// Canonical surface width in logical units, nonzero.
    pub logical_width: u32,
    /// Canonical surface height in logical units, nonzero.
    pub logical_height: u32,
    /// Captured virtual-desktop origin in producer logical pixels.
    pub captured_origin_x: i32,
    /// Captured virtual-desktop origin Y in producer logical pixels.
    pub captured_origin_y: i32,
    /// Rotation of the captured target relative to the canonical surface.
    pub rotation: Rotation,
}

impl SurfaceMapping {
    /// Reject a coordinate outside the canonical surface.
    ///
    /// Desktop §7 requires `0 <= x < width << 32` strictly, so the right and bottom edges are
    /// outside the surface and an event naming them is discarded rather than clamped.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError::InvalidValue`] when a logical dimension overflows 32.32 fixed point
    /// or the point lies outside the surface.
    pub fn validate_point(&self, x: u64, y: u64) -> Result<(), MessageError> {
        let width = u64::from(self.logical_width)
            .checked_shl(32)
            .ok_or_else(|| invalid_value("surface mapping", 0, "width overflows fixed point"))?;
        let height = u64::from(self.logical_height)
            .checked_shl(32)
            .ok_or_else(|| invalid_value("surface mapping", 1, "height overflows fixed point"))?;
        if x >= width {
            return Err(invalid_value(
                "pointer event",
                5,
                "x is outside the canonical surface",
            ));
        }
        if y >= height {
            return Err(invalid_value(
                "pointer event",
                6,
                "y is outside the canonical surface",
            ));
        }
        Ok(())
    }

    /// Map a canonical 32.32 surface point to an OS logical pixel coordinate.
    ///
    /// The point is validated first, then rotated into the captured target's orientation, then
    /// offset by the captured origin. Truncation toward zero is deliberate: a pointer at
    /// `x = 1919.75` belongs to pixel 1919.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError::InvalidValue`] when the point fails
    /// [`SurfaceMapping::validate_point`], rotation overflows, or the result leaves the `i32` OS
    /// coordinate space.
    pub fn to_os_logical(&self, x: u64, y: u64) -> Result<(i32, i32), MessageError> {
        self.validate_point(x, y)?;
        let (rotated_x, rotated_y) = self.rotate(x, y)?;
        let os_x = i64::from(self.captured_origin_x)
            .checked_add(rotated_x)
            .and_then(|value| i32::try_from(value).ok())
            .ok_or_else(|| {
                invalid_value("surface mapping", 2, "x leaves the OS coordinate space")
            })?;
        let os_y = i64::from(self.captured_origin_y)
            .checked_add(rotated_y)
            .and_then(|value| i32::try_from(value).ok())
            .ok_or_else(|| {
                invalid_value("surface mapping", 3, "y leaves the OS coordinate space")
            })?;
        Ok((os_x, os_y))
    }

    /// Rotate a canonical point into the captured target's orientation, in whole logical pixels.
    fn rotate(&self, x: u64, y: u64) -> Result<(i64, i64), MessageError> {
        let point_x = i64::try_from(x >> 32)
            .map_err(|_| invalid_value("surface mapping", 0, "x overflows"))?;
        let point_y = i64::try_from(y >> 32)
            .map_err(|_| invalid_value("surface mapping", 1, "y overflows"))?;
        let width = i64::from(self.logical_width);
        let height = i64::from(self.logical_height);
        // The last row and column are `width - 1` and `height - 1`; `validate_point` already
        // guarantees the input is inside, so these subtractions cannot go negative.
        Ok(match self.rotation {
            Rotation::None => (point_x, point_y),
            Rotation::Ninety => (height - 1 - point_y, point_x),
            Rotation::OneEighty => (width - 1 - point_x, height - 1 - point_y),
            Rotation::TwoSeventy => (point_y, width - 1 - point_x),
        })
    }

    /// The extent of the captured target after rotation, in logical pixels.
    pub fn rotated_extent(&self) -> (u32, u32) {
        match self.rotation {
            Rotation::None | Rotation::OneEighty => (self.logical_width, self.logical_height),
            Rotation::Ninety | Rotation::TwoSeventy => (self.logical_height, self.logical_width),
        }
    }
}

/// Convert a whole logical pixel to signed 32.32.
pub fn from_pixels(pixels: i64) -> Option<i64> {
    pixels.checked_mul(FIXED_ONE)
}

/// Truncate a signed 32.32 value toward zero into whole logical pixels.
pub fn to_pixels(fixed: i64) -> i64 {
    fixed / FIXED_ONE
}

fn scale_normalized(value: i64, extent: i64) -> Result<i64, MessageError> {
    value
        .checked_mul(extent)
        .ok_or_else(|| invalid_value("node geometry", 0, "normalized projection overflows"))
}

fn positive(value: i64, key: u64) -> Result<i64, MessageError> {
    if value <= 0 {
        return Err(invalid_value(
            "node geometry",
            key,
            "projects to an empty extent",
        ));
    }
    Ok(value)
}

/// A non-negative rational scale, compared and applied without overflowing.
///
/// Fixed-point extents reach roughly 2^41 for ordinary displays, so a naive
/// `value * numerator` in `i64` overflows well before the inputs are unreasonable. Every product
/// here widens to `i128` first and narrows back with a checked conversion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Ratio {
    numerator: i64,
    denominator: i64,
}

impl Ratio {
    fn new(numerator: i64, denominator: i64) -> Option<Self> {
        (denominator > 0 && numerator >= 0).then_some(Self {
            numerator,
            denominator,
        })
    }

    fn is_greater_than(self, other: Self) -> bool {
        let left = i128::from(self.numerator) * i128::from(other.denominator);
        let right = i128::from(other.numerator) * i128::from(self.denominator);
        left > right
    }

    fn min(self, other: Self) -> Self {
        if self.is_greater_than(other) {
            other
        } else {
            self
        }
    }

    fn max(self, other: Self) -> Self {
        if self.is_greater_than(other) {
            self
        } else {
            other
        }
    }

    fn apply(self, value: i64) -> Option<i64> {
        let product = i128::from(value).checked_mul(i128::from(self.numerator))?;
        i64::try_from(product.checked_div(i128::from(self.denominator))?).ok()
    }
}

fn required_i64(map: &StrictMap<'_>, key: u64, schema: &'static str) -> Result<i64, MessageError> {
    map.required(key)?
        .as_i64()
        .ok_or_else(|| invalid_value(schema, key, "must be an integer"))
}
