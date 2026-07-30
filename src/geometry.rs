//! Shared fixed-point geometry for target-profile scene placement and desktop coordinate mapping.
//!
//! Every presentation-target profile defines its own node-geometry map, but they share the same
//! signed 32.32 arithmetic, the same clip-map shape, the same fit rules, and — for the desktop and
//! canvas targets — the same coordinate-space discriminator. That shared part lives here so that
//! Vivido, a browser presenter compiled to WebAssembly, and every producer project identically.
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
pub enum CoordinateSpace {
    TargetLogical = 1,
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
    pub x: i64,
    pub y: i64,
    pub width: i64,
    pub height: i64,
}

impl FixedRect {
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
    pub space: CoordinateSpace,
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

    pub fn encode(&self) -> PayloadMap {
        vec![
            (0, Value::Unsigned(self.space as u64)),
            (1, signed(self.rect.x)),
            (2, signed(self.rect.y)),
            (3, signed(self.rect.width)),
            (4, signed(self.rect.height)),
        ]
    }

    /// Project into target logical pixels for the current target extent.
    ///
    /// Normalized geometry changes its pixel projection when the target generation changes, which
    /// is why a scene transaction carries the expected target generation.
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
    pub width: u32,
    pub height: u32,
}

impl TargetExtent {
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }
}

/// Decode the optional clip map, which has the same shape in every target profile.
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

pub fn encode_clip(rect: FixedRect) -> PayloadMap {
    vec![
        (0, signed(rect.x)),
        (1, signed(rect.y)),
        (2, signed(rect.width)),
        (3, signed(rect.height)),
    ]
}

/// Fit a media quad of `source_width` by `source_height` into `destination`.
///
/// The sample aspect ratio widens or narrows the source before fitting, so non-square pixels are
/// presented correctly. The result is the quad the presenter draws before clipping.
pub fn fit_quad(
    destination: FixedRect,
    source_width: u32,
    source_height: u32,
    aspect_numerator: u32,
    aspect_denominator: u32,
    fit: Fit,
) -> Option<FixedRect> {
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
    None = 0,
    Ninety = 90,
    OneEighty = 180,
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
    pub captured_origin_y: i32,
    /// Rotation of the captured target relative to the canonical surface.
    pub rotation: Rotation,
}

impl SurfaceMapping {
    /// Reject a coordinate outside the canonical surface.
    ///
    /// Desktop §7 requires `0 <= x < width << 32` strictly, so the right and bottom edges are
    /// outside the surface and an event naming them is discarded rather than clamped.
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

fn signed(value: i64) -> Value {
    if value >= 0 {
        Value::Unsigned(value as u64)
    } else {
        Value::Negative(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn logical(x: i64, y: i64, width: i64, height: i64) -> NodeGeometry {
        NodeGeometry {
            space: CoordinateSpace::TargetLogical,
            rect: FixedRect::new(
                x * FIXED_ONE,
                y * FIXED_ONE,
                width * FIXED_ONE,
                height * FIXED_ONE,
            ),
        }
    }

    #[test]
    fn geometry_round_trips_through_its_map() {
        for geometry in [logical(3, -4, 100, 50), NodeGeometry::full_target()] {
            let decoded = NodeGeometry::decode(&geometry.encode()).unwrap();
            assert_eq!(decoded, geometry);
        }
    }

    #[test]
    fn normalized_projection_follows_the_target_extent() {
        let node = NodeGeometry::full_target();
        let small = node.project(TargetExtent::new(800, 600)).unwrap();
        assert_eq!(
            small,
            FixedRect::new(0, 0, 800 * FIXED_ONE, 600 * FIXED_ONE)
        );

        // The same node projects differently after a target generation change, which is exactly
        // why a scene commit carries the expected target generation.
        let large = node.project(TargetExtent::new(1920, 1080)).unwrap();
        assert_eq!(
            large,
            FixedRect::new(0, 0, 1920 * FIXED_ONE, 1080 * FIXED_ONE)
        );
        assert_ne!(small, large);
    }

    #[test]
    fn logical_projection_ignores_the_target_extent() {
        let node = logical(10, 20, 30, 40);
        let first = node.project(TargetExtent::new(800, 600)).unwrap();
        let second = node.project(TargetExtent::new(1920, 1080)).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn normalized_geometry_outside_the_unit_square_is_rejected() {
        let mut node = NodeGeometry::full_target();
        node.rect.x = 1;
        assert!(NodeGeometry::decode(&node.encode()).is_err());
    }

    #[test]
    fn non_positive_extents_are_rejected() {
        let mut node = logical(0, 0, 4, 4);
        node.rect.width = 0;
        assert!(NodeGeometry::decode(&node.encode()).is_err());
        node.rect.width = -FIXED_ONE;
        assert!(NodeGeometry::decode(&node.encode()).is_err());
    }

    #[test]
    fn geometry_rejects_an_origin_plus_extent_overflow() {
        let node = NodeGeometry {
            space: CoordinateSpace::TargetLogical,
            rect: FixedRect::new(i64::MAX - 1, 0, 8, 8),
        };
        assert!(NodeGeometry::decode(&node.encode()).is_err());
    }

    #[test]
    fn normalized_projection_rejects_an_overflowing_extent() {
        let node = NodeGeometry {
            space: CoordinateSpace::NormalizedTarget,
            rect: FixedRect::unit(),
        };
        // A target this wide cannot exist, but the transform must refuse rather than wrap.
        assert!(node.project(TargetExtent::new(u32::MAX, u32::MAX)).is_err());
    }

    #[test]
    fn unknown_coordinate_space_and_extra_keys_are_rejected() {
        let mut map = NodeGeometry::full_target().encode();
        map[0].1 = Value::Unsigned(9);
        assert!(NodeGeometry::decode(&map).is_err());

        let mut extra = NodeGeometry::full_target().encode();
        extra.push((5, Value::Unsigned(0)));
        assert!(NodeGeometry::decode(&extra).is_err());
    }

    #[test]
    fn contain_letterboxes_and_cover_fills() {
        let destination = FixedRect::new(0, 0, 400 * FIXED_ONE, 400 * FIXED_ONE);
        let contain = fit_quad(destination, 200, 100, 1, 1, Fit::Contain).unwrap();
        assert_eq!(contain.width, 400 * FIXED_ONE);
        assert_eq!(contain.height, 200 * FIXED_ONE);
        assert_eq!(contain.y, 100 * FIXED_ONE, "letterboxed and centred");

        let cover = fit_quad(destination, 200, 100, 1, 1, Fit::Cover).unwrap();
        assert_eq!(cover.width, 800 * FIXED_ONE);
        assert_eq!(cover.height, 400 * FIXED_ONE);
        assert_eq!(cover.x, -200 * FIXED_ONE, "overflows equally on both sides");
    }

    #[test]
    fn fill_and_none_do_not_preserve_or_ignore_aspect() {
        let destination = FixedRect::new(0, 0, 400 * FIXED_ONE, 400 * FIXED_ONE);
        let fill = fit_quad(destination, 200, 100, 1, 1, Fit::Fill).unwrap();
        assert_eq!(fill, destination);

        let none = fit_quad(destination, 200, 100, 1, 1, Fit::None).unwrap();
        assert_eq!(none.width, 200 * FIXED_ONE);
        assert_eq!(none.height, 100 * FIXED_ONE);
    }

    #[test]
    fn sample_aspect_ratio_widens_the_source() {
        let destination = FixedRect::new(0, 0, 400 * FIXED_ONE, 400 * FIXED_ONE);
        let square = fit_quad(destination, 100, 100, 1, 1, Fit::None).unwrap();
        let wide = fit_quad(destination, 100, 100, 2, 1, Fit::None).unwrap();
        assert_eq!(wide.width, square.width * 2);
        assert_eq!(wide.height, square.height);
    }

    #[test]
    fn degenerate_fit_inputs_are_refused() {
        let destination = FixedRect::new(0, 0, 400 * FIXED_ONE, 400 * FIXED_ONE);
        assert!(fit_quad(destination, 0, 100, 1, 1, Fit::Contain).is_none());
        assert!(fit_quad(destination, 100, 0, 1, 1, Fit::Contain).is_none());
        assert!(fit_quad(destination, 100, 100, 0, 1, Fit::Contain).is_none());
        assert!(fit_quad(destination, 100, 100, 1, 0, Fit::Contain).is_none());
    }

    #[test]
    fn fit_compares_scales_as_rationals_not_as_tuples() {
        // Regression: `(numerator, denominator)` tuples order lexicographically, which picks the
        // larger denominator rather than the larger ratio. Cover must choose 400/100, not 400/200.
        let destination = FixedRect::new(0, 0, 400 * FIXED_ONE, 400 * FIXED_ONE);
        let cover = fit_quad(destination, 200, 100, 1, 1, Fit::Cover).unwrap();
        let contain = fit_quad(destination, 200, 100, 1, 1, Fit::Contain).unwrap();
        assert!(
            cover.width > contain.width,
            "cover scales up more than contain"
        );
        assert_eq!(cover.width / contain.width, 2);
    }

    #[test]
    fn fit_does_not_overflow_at_realistic_display_sizes() {
        // Regression: fixed-point extents are around 2^41, so `value * numerator` in i64 overflows
        // long before the inputs are unreasonable. 8K into 8K must simply work.
        let destination = FixedRect::new(0, 0, 7680 * FIXED_ONE, 4320 * FIXED_ONE);
        for fit in [Fit::Contain, Fit::Cover, Fit::Fill, Fit::None] {
            let quad = fit_quad(destination, 7680, 4320, 1, 1, fit).unwrap();
            assert!(
                quad.width > 0 && quad.height > 0,
                "{fit:?} produced an empty quad"
            );
        }
        let scaled = fit_quad(destination, 1920, 1080, 1, 1, Fit::Contain).unwrap();
        assert_eq!(scaled.width, 7680 * FIXED_ONE);
        assert_eq!(scaled.height, 4320 * FIXED_ONE);
    }

    #[test]
    fn intersection_clips_and_reports_empty() {
        let quad = FixedRect::new(0, 0, 100, 100);
        let clip = FixedRect::new(50, 50, 100, 100);
        assert_eq!(intersect(quad, clip), Some(FixedRect::new(50, 50, 50, 50)));
        assert_eq!(intersect(quad, FixedRect::new(200, 200, 10, 10)), None);
    }

    fn mapping(rotation: Rotation) -> SurfaceMapping {
        SurfaceMapping {
            logical_width: 1920,
            logical_height: 1080,
            captured_origin_x: 0,
            captured_origin_y: 0,
            rotation,
        }
    }

    #[test]
    fn a_point_on_the_far_edge_is_outside_the_surface() {
        let mapping = mapping(Rotation::None);
        assert!(mapping.validate_point(1919 << 32, 1079 << 32).is_ok());
        // Desktop §7 is strict: `x < width << 32`, so the width itself is out.
        assert!(mapping.validate_point(1920 << 32, 0).is_err());
        assert!(mapping.validate_point(0, 1080 << 32).is_err());
    }

    #[test]
    fn every_rotation_maps_the_corners_correctly() {
        let top_left = (0_u64, 0_u64);
        let bottom_right = (1919_u64 << 32, 1079_u64 << 32);

        assert_eq!(
            mapping(Rotation::None)
                .to_os_logical(top_left.0, top_left.1)
                .unwrap(),
            (0, 0)
        );
        assert_eq!(
            mapping(Rotation::None)
                .to_os_logical(bottom_right.0, bottom_right.1)
                .unwrap(),
            (1919, 1079)
        );

        // 90 degrees clockwise sends the canonical top-left to the captured top-right.
        assert_eq!(
            mapping(Rotation::Ninety)
                .to_os_logical(top_left.0, top_left.1)
                .unwrap(),
            (1079, 0)
        );
        assert_eq!(
            mapping(Rotation::OneEighty)
                .to_os_logical(top_left.0, top_left.1)
                .unwrap(),
            (1919, 1079)
        );
        assert_eq!(
            mapping(Rotation::TwoSeventy)
                .to_os_logical(top_left.0, top_left.1)
                .unwrap(),
            (0, 1919)
        );
    }

    #[test]
    fn rotation_swaps_the_extent_only_on_the_quarter_turns() {
        assert_eq!(mapping(Rotation::None).rotated_extent(), (1920, 1080));
        assert_eq!(mapping(Rotation::OneEighty).rotated_extent(), (1920, 1080));
        assert_eq!(mapping(Rotation::Ninety).rotated_extent(), (1080, 1920));
        assert_eq!(mapping(Rotation::TwoSeventy).rotated_extent(), (1080, 1920));
    }

    #[test]
    fn the_captured_origin_offsets_the_result() {
        let mapping = SurfaceMapping {
            captured_origin_x: -1920,
            captured_origin_y: 200,
            ..mapping(Rotation::None)
        };
        assert_eq!(mapping.to_os_logical(0, 0).unwrap(), (-1920, 200));
    }

    #[test]
    fn a_captured_origin_that_leaves_the_os_space_is_refused() {
        let mapping = SurfaceMapping {
            captured_origin_x: i32::MAX,
            ..mapping(Rotation::None)
        };
        assert!(mapping.to_os_logical(1919 << 32, 0).is_err());
    }

    #[test]
    fn sub_pixel_coordinates_truncate_toward_the_containing_pixel() {
        let mapping = mapping(Rotation::None);
        let three_quarters = (1919_u64 << 32) | (3 << 30);
        assert_eq!(mapping.to_os_logical(three_quarters, 0).unwrap().0, 1919);
    }

    #[test]
    fn pixel_conversion_round_trips_and_refuses_overflow() {
        assert_eq!(to_pixels(from_pixels(42).unwrap()), 42);
        assert_eq!(to_pixels(from_pixels(-42).unwrap()), -42);
        assert!(from_pixels(i64::MAX).is_none());
    }

    #[test]
    fn rotation_rejects_an_unregistered_angle() {
        assert!(Rotation::try_from(45).is_err());
        assert_eq!(Rotation::try_from(270).unwrap(), Rotation::TwoSeventy);
    }

    #[test]
    fn clip_round_trips_and_rejects_empty_extents() {
        let clip = FixedRect::new(-5, -6, 20, 30);
        assert_eq!(decode_clip(&encode_clip(clip)).unwrap(), clip);
        assert!(decode_clip(&encode_clip(FixedRect::new(0, 0, 0, 4))).is_err());
        assert!(decode_clip(&encode_clip(FixedRect::new(0, 0, 4, -1))).is_err());
    }
}
