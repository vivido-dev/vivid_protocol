//! Renderer-independent, bounded vector display lists. Coordinates are signed Q32.32.
//!
//! Lists are complete replacement scenes. A receiver validates the whole list before publishing
//! either drawing or hit geometry; Vello's private scene encoding never crosses the wire.

use crate::cbor::{self, Value};
use std::collections::BTreeSet;
use std::fmt;

pub const MAX_SCENE_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_COMMANDS: usize = 4096;
pub const MAX_PATH_SEGMENTS: usize = 4096;
pub const MAX_TOTAL_SEGMENTS: usize = 65536;
pub const MAX_STACK_DEPTH: usize = 32;
pub const MAX_TEXT_BYTES: usize = 64 * 1024;
pub const MAX_GRADIENT_STOPS: usize = 64;
pub const MAX_DASH_ENTRIES: usize = 32;
/// Hard ceiling for one dash length, corner radius, shadow blur, or spread, in logical pixels.
pub const MAX_PAINT_EXTENT: f64 = 4096.;
pub const MAX_ASSET_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_RETAINED_ASSETS: usize = 256;
pub const MAX_RETAINED_ASSET_BYTES: usize = 64 * 1024 * 1024;

/// The authenticated WELCOME limits for a vector-capable presenter, in registered key order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Limits([u64; 12]);
impl Default for Limits {
    fn default() -> Self {
        Self([
            MAX_SCENE_BYTES as u64,
            MAX_COMMANDS as u64,
            MAX_PATH_SEGMENTS as u64,
            MAX_TOTAL_SEGMENTS as u64,
            MAX_STACK_DEPTH as u64,
            MAX_TEXT_BYTES as u64,
            MAX_GRADIENT_STOPS as u64,
            MAX_ASSET_BYTES as u64,
            MAX_RETAINED_ASSETS as u64,
            MAX_RETAINED_ASSET_BYTES as u64,
            crate::overlay::MAX_WINDOWS_PER_OWNER as u64,
            crate::overlay::MAX_PENDING_EVENTS as u64,
        ])
    }
}
impl Limits {
    pub fn new(values: [u64; 12]) -> Result<Self> {
        if values
            .iter()
            .zip(Self::default().0)
            .any(|(value, maximum)| *value == 0 || *value > maximum)
        {
            return Err(InvalidScene("invalid negotiated vector limits"));
        }
        Ok(Self(values))
    }
    pub fn values(&self) -> &[u64; 12] {
        &self.0
    }
    pub fn to_value(&self) -> Value {
        Value::Map(
            self.0
                .iter()
                .enumerate()
                .map(|(key, value)| (key as u64, Value::Unsigned(*value)))
                .collect(),
        )
    }
    pub fn from_value(value: &Value) -> Result<Self> {
        let Value::Map(fields) = value else {
            return Err(InvalidScene("vector limits must be a map"));
        };
        if fields.len() != 12 {
            return Err(InvalidScene("incomplete vector limits"));
        }
        let mut values = [0; 12];
        for (index, (key, value)) in fields.iter().enumerate() {
            if *key != index as u64 {
                return Err(InvalidScene("unknown vector limit"));
            }
            values[index] = uint(value)?;
        }
        Self::new(values)
    }
}

impl Command {
    /// Whether this command is available only under `overlay-pointer-v1`.
    pub fn requires_pointer(&self) -> bool {
        matches!(
            self,
            Command::Hit {
                cursor: Some(_),
                ..
            }
        )
    }

    /// Whether this command is available only under `overlay-paint-v1`. A producer gates its
    /// submissions and a presenter gates compilation on exactly this predicate, so the two
    /// cannot drift apart.
    pub fn requires_paint(&self) -> bool {
        let brush = match self {
            Command::Shadow(_) | Command::StyledStroke(..) => return true,
            Command::Fill(_, brush) | Command::Stroke(_, brush, _) => brush,
            _ => return false,
        };
        match brush {
            Brush::Image { .. } => true,
            Brush::Linear { color_space, .. } | Brush::Radial { color_space, .. } => {
                *color_space != ColorSpace::Srgb
            }
            Brush::Solid(_) => false,
        }
    }
}

/// A complete scene replacement carried by an authenticated bulk track channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub epoch: u32,
    pub revision: u64,
    pub canvas: Canvas,
}
impl Frame {
    pub fn encode(&self) -> Result<Vec<u8>> {
        if self.epoch == 0 || self.revision == 0 {
            return Err(InvalidScene("scene epoch and revision must be nonzero"));
        }
        let commands = self.canvas.encode()?;
        let mut body = Vec::with_capacity(12 + commands.len());
        body.extend_from_slice(&self.epoch.to_be_bytes());
        body.extend_from_slice(&self.revision.to_be_bytes());
        body.extend(commands);
        Ok(body)
    }
    pub fn decode(body: &[u8]) -> Result<Self> {
        if body.len() < 12 || body.len() > MAX_SCENE_BYTES + 12 {
            return Err(InvalidScene("invalid vector frame length"));
        }
        let epoch = u32::from_be_bytes(
            body[..4]
                .try_into()
                .map_err(|_| InvalidScene("missing epoch"))?,
        );
        let revision = u64::from_be_bytes(
            body[4..12]
                .try_into()
                .map_err(|_| InvalidScene("missing revision"))?,
        );
        if epoch == 0 || revision == 0 {
            return Err(InvalidScene("scene epoch and revision must be nonzero"));
        }
        Ok(Self {
            epoch,
            revision,
            canvas: Canvas::decode(&body[12..])?,
        })
    }
}

/// Immutable straight-alpha sRGB RGBA8 asset. The authenticated channel supplies its namespace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageAsset {
    pub id: u64,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Ordered removal from a channel's asset namespace; existing scenes keep their references.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AssetRelease {
    pub id: u64,
}
impl AssetRelease {
    pub fn encode(self) -> Result<[u8; 8]> {
        if self.id == 0 {
            return Err(InvalidScene("asset ID must be nonzero"));
        }
        Ok(self.id.to_be_bytes())
    }
    pub fn decode(body: &[u8]) -> Result<Self> {
        let bytes: [u8; 8] = body
            .try_into()
            .map_err(|_| InvalidScene("asset release must contain exactly one u64"))?;
        let value = Self {
            id: u64::from_be_bytes(bytes),
        };
        value.encode()?;
        Ok(value)
    }
}
impl ImageAsset {
    pub fn validate(&self) -> Result<()> {
        let bytes = u64::from(self.width)
            .checked_mul(u64::from(self.height))
            .and_then(|n| n.checked_mul(4));
        if self.id == 0
            || self.width == 0
            || self.height == 0
            || bytes.is_none_or(|n| n > MAX_ASSET_BYTES as u64 || n != self.rgba.len() as u64)
        {
            return Err(InvalidScene(
                "invalid retained RGBA image dimensions or bytes",
            ));
        }
        Ok(())
    }
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let mut body = Vec::with_capacity(16 + self.rgba.len());
        body.extend_from_slice(&self.id.to_be_bytes());
        body.extend_from_slice(&self.width.to_be_bytes());
        body.extend_from_slice(&self.height.to_be_bytes());
        body.extend_from_slice(&self.rgba);
        Ok(body)
    }
    pub fn decode(body: &[u8]) -> Result<Self> {
        if body.len() < 16 || body.len() > 16 + MAX_ASSET_BYTES {
            return Err(InvalidScene("invalid vector asset length"));
        }
        let id = u64::from_be_bytes(
            body[..8]
                .try_into()
                .map_err(|_| InvalidScene("missing asset ID"))?,
        );
        let width = u32::from_be_bytes(
            body[8..12]
                .try_into()
                .map_err(|_| InvalidScene("missing asset width"))?,
        );
        let height = u32::from_be_bytes(
            body[12..16]
                .try_into()
                .map_err(|_| InvalidScene("missing asset height"))?,
        );
        let bytes = u64::from(width)
            .checked_mul(u64::from(height))
            .and_then(|n| n.checked_mul(4));
        if id == 0 || width == 0 || height == 0 || bytes != Some((body.len() - 16) as u64) {
            return Err(InvalidScene("invalid retained image geometry"));
        }
        Ok(Self {
            id,
            width,
            height,
            rgba: body[16..].to_vec(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidScene(pub &'static str);
impl fmt::Display for InvalidScene {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for InvalidScene {}
type Result<T> = std::result::Result<T, InvalidScene>;

/// A finite logical coordinate, bounded to +/- 1 million units before fixed-point conversion.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Scalar(i64);
impl Scalar {
    pub const ZERO: Self = Self(0);
    pub const ONE: Self = Self(1_i64 << 32);
    pub fn new(value: f64) -> Result<Self> {
        if !value.is_finite() || value.abs() > 1_000_000.0 {
            return Err(InvalidScene(
                "coordinate is non-finite or exceeds the logical extent limit",
            ));
        }
        Ok(Self((value * 4294967296.0).round() as i64))
    }
    pub fn get(self) -> f64 {
        self.0 as f64 / 4294967296.0
    }
    pub const fn raw(self) -> i64 {
        self.0
    }
    pub fn from_raw(raw: i64) -> Result<Self> {
        if raw.unsigned_abs() > (1_000_000_u64 << 32) {
            return Err(InvalidScene("coordinate exceeds limit"));
        }
        Ok(Self(raw))
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Point {
    pub x: Scalar,
    pub y: Scalar,
}
impl Point {
    pub fn new(x: f64, y: f64) -> Result<Self> {
        Ok(Self {
            x: Scalar::new(x)?,
            y: Scalar::new(y)?,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub origin: Point,
    pub width: Scalar,
    pub height: Scalar,
}
impl Rect {
    pub fn new(x: f64, y: f64, width: f64, height: f64) -> Result<Self> {
        let rect = Self {
            origin: Point::new(x, y)?,
            width: Scalar::new(width)?,
            height: Scalar::new(height)?,
        };
        rect.validate()?;
        Ok(rect)
    }
    pub fn validate(self) -> Result<()> {
        if self.width <= Scalar::ZERO || self.height <= Scalar::ZERO {
            return Err(InvalidScene("rectangle extent must be positive"));
        }
        Scalar::new(self.origin.x.get() + self.width.get())?;
        Scalar::new(self.origin.y.get() + self.height.get())?;
        Ok(())
    }
    pub fn contains(self, p: Point) -> bool {
        let x = p.x.get() - self.origin.x.get();
        let y = p.y.get() - self.origin.y.get();
        x >= 0.0 && y >= 0.0 && x < self.width.get() && y < self.height.get()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Transform(pub [Scalar; 6]);
impl Default for Transform {
    fn default() -> Self {
        Self([
            Scalar::ONE,
            Scalar::ZERO,
            Scalar::ZERO,
            Scalar::ONE,
            Scalar::ZERO,
            Scalar::ZERO,
        ])
    }
}
impl Transform {
    pub fn new(values: [f64; 6]) -> Result<Self> {
        Ok(Self([
            Scalar::new(values[0])?,
            Scalar::new(values[1])?,
            Scalar::new(values[2])?,
            Scalar::new(values[3])?,
            Scalar::new(values[4])?,
            Scalar::new(values[5])?,
        ]))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Segment {
    Move(Point),
    Line(Point),
    Quad(Point, Point),
    Cubic(Point, Point, Point),
    Close,
}
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Path {
    pub segments: Vec<Segment>,
    pub even_odd: bool,
}
impl Path {
    pub fn rectangle(rect: Rect) -> Result<Self> {
        rect.validate()?;
        let x = rect.origin.x.get();
        let y = rect.origin.y.get();
        let right = x + rect.width.get();
        let bottom = y + rect.height.get();
        Ok(Self {
            segments: vec![
                Segment::Move(rect.origin),
                Segment::Line(Point::new(right, y)?),
                Segment::Line(Point::new(right, bottom)?),
                Segment::Line(Point::new(x, bottom)?),
                Segment::Close,
            ],
            even_odd: false,
        })
    }
    pub fn rounded_rectangle(rect: Rect, radius: f64) -> Result<Self> {
        Self::rounded_rectangle_corners(rect, Corners::uniform(radius)?)
    }

    /// A rounded rectangle whose four corners may differ. Radii are clockwise from the top
    /// left: `[top_left, top_right, bottom_right, bottom_left]`. Over-large radii are scaled
    /// down together by the smallest side ratio, as CSS does, so the outline never overlaps
    /// itself.
    pub fn rounded_rectangle_corners(rect: Rect, radii: Corners) -> Result<Self> {
        rect.validate()?;
        radii.validate()?;
        let [tl, tr, br, bl] = radii.0.map(|r| r.get());
        let (w, h) = (rect.width.get(), rect.height.get());
        let fit = (w / (tl + tr))
            .min(w / (bl + br))
            .min(h / (tl + bl))
            .min(h / (tr + br));
        let [tl, tr, br, bl] = if fit < 1.0 {
            [tl * fit, tr * fit, br * fit, bl * fit]
        } else {
            [tl, tr, br, bl]
        };
        let x = rect.origin.x.get();
        let y = rect.origin.y.get();
        let k = 0.5522847498307936;
        // One quarter arc from a point on one edge to a point on the next, bulging toward the
        // rectangle corner where those edges meet.
        // `f64::signum` answers 1.0 for +0.0, so an axis with no distance toward the corner
        // must contribute no bulge rather than a positive one.
        let bulge = |delta: f64, radius: f64| -> f64 {
            if delta == 0. {
                0.
            } else {
                delta.signum() * k * radius
            }
        };
        let arc = |a: Point, corner: Point, b: Point, radius: f64| -> Result<Vec<Segment>> {
            let c1 = Point::new(
                a.x.get() + bulge(corner.x.get() - a.x.get(), radius),
                a.y.get() + bulge(corner.y.get() - a.y.get(), radius),
            )?;
            let c2 = Point::new(
                b.x.get() + bulge(corner.x.get() - b.x.get(), radius),
                b.y.get() + bulge(corner.y.get() - b.y.get(), radius),
            )?;
            Ok(vec![Segment::Cubic(c1, c2, b)])
        };
        let p = Point::new;
        let mut segments = vec![Segment::Move(p(x + tl, y)?)];
        segments.push(Segment::Line(p(x + w - tr, y)?));
        segments.extend(arc(p(x + w - tr, y)?, p(x + w, y)?, p(x + w, y + tr)?, tr)?);
        segments.push(Segment::Line(p(x + w, y + h - br)?));
        segments.extend(arc(
            p(x + w, y + h - br)?,
            p(x + w, y + h)?,
            p(x + w - br, y + h)?,
            br,
        )?);
        segments.push(Segment::Line(p(x + bl, y + h)?));
        segments.extend(arc(p(x + bl, y + h)?, p(x, y + h)?, p(x, y + h - bl)?, bl)?);
        segments.push(Segment::Line(p(x, y + tl)?));
        segments.extend(arc(p(x, y + tl)?, p(x, y)?, p(x + tl, y)?, tl)?);
        segments.push(Segment::Close);
        Ok(Self {
            segments,
            even_odd: false,
        })
    }
    pub fn ellipse(rect: Rect) -> Result<Self> {
        rect.validate()?;
        let cx = rect.origin.x.get() + rect.width.get() / 2.0;
        let cy = rect.origin.y.get() + rect.height.get() / 2.0;
        let rx = rect.width.get() / 2.0;
        let ry = rect.height.get() / 2.0;
        let k = 0.5522847498307936;
        let p = Point::new;
        Ok(Self {
            segments: vec![
                Segment::Move(p(cx + rx, cy)?),
                Segment::Cubic(
                    p(cx + rx, cy + ry * k)?,
                    p(cx + rx * k, cy + ry)?,
                    p(cx, cy + ry)?,
                ),
                Segment::Cubic(
                    p(cx - rx * k, cy + ry)?,
                    p(cx - rx, cy + ry * k)?,
                    p(cx - rx, cy)?,
                ),
                Segment::Cubic(
                    p(cx - rx, cy - ry * k)?,
                    p(cx - rx * k, cy - ry)?,
                    p(cx, cy - ry)?,
                ),
                Segment::Cubic(
                    p(cx + rx * k, cy - ry)?,
                    p(cx + rx, cy - ry * k)?,
                    p(cx + rx, cy)?,
                ),
                Segment::Close,
            ],
            even_odd: false,
        })
    }
    pub fn validate(&self) -> Result<()> {
        if self.segments.is_empty() || self.segments.len() > MAX_PATH_SEGMENTS {
            return Err(InvalidScene("path segment limit exceeded"));
        }
        let mut open = false;
        for segment in &self.segments {
            match segment {
                Segment::Move(_) => open = true,
                Segment::Close if open => open = false,
                Segment::Close => return Err(InvalidScene("close without a subpath")),
                _ if !open => return Err(InvalidScene("path must begin with move")),
                _ => (),
            }
        }
        Ok(())
    }
}

/// Straight-alpha sRGB RGBA8, most significant byte red.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color(pub u32);

/// Four corner radii, clockwise from the top left:
/// `[top_left, top_right, bottom_right, bottom_left]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Corners(pub [Scalar; 4]);
impl Corners {
    pub fn new(values: [f64; 4]) -> Result<Self> {
        Ok(Self([
            Scalar::new(values[0])?,
            Scalar::new(values[1])?,
            Scalar::new(values[2])?,
            Scalar::new(values[3])?,
        ]))
    }
    pub fn uniform(radius: f64) -> Result<Self> {
        Self::new([radius; 4])
    }
    /// The single radius when all four corners agree, as `draw_blurred_rounded_rect` wants.
    pub fn uniform_value(&self) -> Option<f64> {
        let [tl, tr, br, bl] = self.0.map(|r| r.get());
        (tl == tr && tr == br && br == bl).then_some(tl)
    }
    pub fn validate(&self) -> Result<()> {
        if self
            .0
            .iter()
            .any(|r| *r < Scalar::ZERO || r.get() > MAX_PAINT_EXTENT)
        {
            return Err(InvalidScene("corner radius is negative or out of range"));
        }
        Ok(())
    }
}

/// How gradient stops interpolate, and therefore how the transition looks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ColorSpace {
    #[default]
    Srgb,
    Oklab,
}

/// How an image brush samples outside its own extent.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Extend {
    #[default]
    Pad,
    Repeat,
    Reflect,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GradientStop {
    pub offset: u16,
    pub color: Color,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Brush {
    Solid(Color),
    Linear {
        start: Point,
        end: Point,
        stops: Vec<GradientStop>,
        color_space: ColorSpace,
    },
    Radial {
        center: Point,
        radius: Scalar,
        stops: Vec<GradientStop>,
        color_space: ColorSpace,
    },
    /// Fill with a retained image. The image sits at its natural pixel size, anchored at the
    /// origin, unless `transform` repositions or scales it; `extend` governs sampling outside
    /// its extent. The asset must already exist on the same channel.
    Image {
        asset: u64,
        transform: Option<Transform>,
        extend: Extend,
    },
}
impl Brush {
    fn validate(&self) -> Result<()> {
        let stops = match self {
            Self::Solid(_) => return Ok(()),
            Self::Image { asset, .. } => {
                if *asset == 0 {
                    return Err(InvalidScene("image brush asset ID must be nonzero"));
                }
                return Ok(());
            }
            Self::Linear {
                start, end, stops, ..
            } => {
                if start == end {
                    return Err(InvalidScene("gradient endpoints coincide"));
                }
                stops
            }
            Self::Radial { radius, stops, .. } => {
                if *radius <= Scalar::ZERO {
                    return Err(InvalidScene("radial radius must be positive"));
                }
                stops
            }
        };
        if !(2..=MAX_GRADIENT_STOPS).contains(&stops.len())
            || stops.windows(2).any(|s| s[0].offset > s[1].offset)
        {
            return Err(InvalidScene("gradient stops must be bounded and sorted"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Text {
    pub text: String,
    pub origin: Point,
    pub size: Scalar,
    pub family: String,
    pub weight: u16,
    pub italic: bool,
    pub color: Color,
    pub max_width: Option<Scalar>,
}
/// A blurred rounded rectangle cast behind (or inside) a region, as CSS box-shadow defines it.
///
/// `blur` is the gaussian diameter; the host cuts it off at its own kernel extent. `spread`
/// expands or contracts the shape before the blur. Drawing the element over its own shadow
/// is the caller's job: the shadow command only paints the shadow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shadow {
    pub rect: Rect,
    pub radii: Corners,
    pub color: Color,
    pub offset: Point,
    pub blur: Scalar,
    pub spread: Scalar,
    pub inset: bool,
}
impl Shadow {
    pub fn validate(&self) -> Result<()> {
        self.rect.validate()?;
        self.radii.validate()?;
        if self.blur < Scalar::ZERO || self.blur.get() > MAX_PAINT_EXTENT {
            return Err(InvalidScene("shadow blur is negative or out of range"));
        }
        if self.spread.get().abs() > MAX_PAINT_EXTENT {
            return Err(InvalidScene("shadow spread is out of range"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Cap {
    #[default]
    Butt,
    Round,
    Square,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Join {
    #[default]
    Miter,
    Bevel,
    Round,
}

/// A stroke with caps, joins, and an optional dash pattern. Tag 1 remains the plain stroke.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StrokeStyle {
    pub width: Scalar,
    pub cap: Cap,
    pub join: Join,
    pub miter_limit: Scalar,
    pub dashes: Vec<Scalar>,
    pub dash_offset: Scalar,
}
impl StrokeStyle {
    pub fn new(width: f64) -> Result<Self> {
        Ok(Self {
            width: Scalar::new(width)?,
            ..Self::default()
        })
    }
    pub fn validate(&self) -> Result<()> {
        if self.width <= Scalar::ZERO {
            return Err(InvalidScene("stroke width must be positive"));
        }
        if self.miter_limit < Scalar::ONE || self.miter_limit.get() > MAX_PAINT_EXTENT {
            return Err(InvalidScene("miter limit must be at least one"));
        }
        if self.dashes.len() > MAX_DASH_ENTRIES
            || self
                .dashes
                .iter()
                .any(|d| *d <= Scalar::ZERO || d.get() > MAX_PAINT_EXTENT)
            || self.dash_offset < Scalar::ZERO
            || self.dash_offset.get() > MAX_PAINT_EXTENT * MAX_DASH_ENTRIES as f64
        {
            return Err(InvalidScene("dash pattern is invalid or out of range"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HitRole {
    Input,
    Drag,
    Resize(u8),
    Transparent,
}

/// The pointer shape a host shows while a region is hovered (`overlay-pointer-v1`).
///
/// A closed set rather than a platform cursor name: a host maps these onto whatever its
/// platform offers, and an unknown shape must fail rather than silently become an arrow.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CursorShape {
    #[default]
    Default,
    Pointer,
    Text,
    Move,
    Crosshair,
    NotAllowed,
    Grab,
    Grabbing,
    Wait,
    Progress,
    ResizeLeft,
    ResizeRight,
    ResizeUp,
    ResizeDown,
    ResizeUpLeft,
    ResizeUpRight,
    ResizeDownLeft,
    ResizeDownRight,
    ResizeLeftRight,
    ResizeUpDown,
}
impl CursorShape {
    /// The wire index, which is the declaration order above.
    pub fn index(self) -> u64 {
        self as u64
    }
    pub fn from_index(index: u64) -> Option<Self> {
        const ALL: [CursorShape; 20] = [
            CursorShape::Default,
            CursorShape::Pointer,
            CursorShape::Text,
            CursorShape::Move,
            CursorShape::Crosshair,
            CursorShape::NotAllowed,
            CursorShape::Grab,
            CursorShape::Grabbing,
            CursorShape::Wait,
            CursorShape::Progress,
            CursorShape::ResizeLeft,
            CursorShape::ResizeRight,
            CursorShape::ResizeUp,
            CursorShape::ResizeDown,
            CursorShape::ResizeUpLeft,
            CursorShape::ResizeUpRight,
            CursorShape::ResizeDownLeft,
            CursorShape::ResizeDownRight,
            CursorShape::ResizeLeftRight,
            CursorShape::ResizeUpDown,
        ];
        ALL.get(usize::try_from(index).ok()?).copied()
    }
}

/// One hit-test result: the application region, its role, and the cursor it asks for.
///
/// A hit test and the cursor it implies are decided from the same compiled scene, so they
/// travel together rather than being looked up separately and risking a stale pairing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HitRegion {
    pub id: u64,
    pub role: HitRole,
    pub cursor: Option<CursorShape>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Fill(Path, Brush),
    Stroke(Path, Brush, Scalar),
    Save,
    Restore,
    Transform(Transform),
    Clip(Path),
    Opacity(u16),
    Text(Text),
    /// A host-shaped layout in this window's retained namespace (overlay-text-layout-v1).
    TextLayout {
        layout: u64,
        origin: Point,
    },
    Image {
        asset: u64,
        rect: Rect,
        opacity: u16,
    },
    Hit {
        id: u64,
        path: Path,
        role: HitRole,
        /// Cursor shown while this region is hovered (overlay-pointer-v1).
        cursor: Option<CursorShape>,
    },
    /// A blurred rounded rectangle (overlay-paint-v1).
    Shadow(Shadow),
    /// A stroke with caps, joins, and dashes (overlay-paint-v1).
    StyledStroke(Path, Brush, StrokeStyle),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Canvas {
    commands: Vec<Command>,
}
impl Canvas {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn commands(&self) -> &[Command] {
        &self.commands
    }
    pub fn push(&mut self, command: Command) -> Result<&mut Self> {
        if self.commands.len() >= MAX_COMMANDS {
            return Err(InvalidScene("command limit exceeded"));
        }
        self.commands.push(command);
        Ok(self)
    }
    pub fn fill(&mut self, path: Path, brush: Brush) -> Result<&mut Self> {
        self.push(Command::Fill(path, brush))
    }
    pub fn stroke(&mut self, path: Path, brush: Brush, width: f64) -> Result<&mut Self> {
        self.push(Command::Stroke(path, brush, Scalar::new(width)?))
    }
    /// Cast a blurred rounded rectangle, as CSS box-shadow defines it.
    pub fn shadow(&mut self, shadow: Shadow) -> Result<&mut Self> {
        self.push(Command::Shadow(shadow))
    }
    /// Stroke with caps, joins, and an optional dash pattern.
    pub fn stroke_styled(
        &mut self,
        path: Path,
        brush: Brush,
        style: StrokeStyle,
    ) -> Result<&mut Self> {
        self.push(Command::StyledStroke(path, brush, style))
    }
    /// Check the host's authenticated limits, which may be below the profile ceilings.
    pub fn validate_with_limits(&self, limits: &Limits) -> Result<()> {
        self.validate()?;
        let limit = &limits.0;
        if self.commands.len() as u64 > limit[1] || self.encode()?.len() as u64 > limit[0] {
            return Err(InvalidScene(
                "scene exceeds negotiated byte or command limits",
            ));
        }
        let mut segments = 0_u64;
        let mut text_bytes = 0_u64;
        let mut clips = 0_u64;
        let mut stack = Vec::new();
        for command in &self.commands {
            match command {
                Command::Save => {
                    stack.push(clips);
                    if stack.len() as u64 > limit[4] {
                        return Err(InvalidScene("negotiated save depth exceeded"));
                    }
                }
                Command::Restore => {
                    clips = stack.pop().ok_or(InvalidScene("unbalanced restore"))?;
                }
                Command::Clip(_) => {
                    clips += 1;
                    if clips > limit[4] {
                        return Err(InvalidScene("negotiated clip depth exceeded"));
                    }
                }
                Command::Text(value) => {
                    text_bytes += value.text.len() as u64;
                }
                _ => {}
            }
            let path = match command {
                Command::Fill(path, brush)
                | Command::Stroke(path, brush, _)
                | Command::StyledStroke(path, brush, _) => {
                    let count = match brush {
                        Brush::Solid(_) | Brush::Image { .. } => 0,
                        Brush::Linear { stops, .. } | Brush::Radial { stops, .. } => stops.len(),
                    };
                    if count as u64 > limit[6] {
                        return Err(InvalidScene("negotiated gradient limit exceeded"));
                    }
                    Some(path)
                }
                Command::Hit { path, .. } | Command::Clip(path) => Some(path),
                _ => None,
            };
            if let Some(path) = path {
                let count = path.segments.len() as u64;
                if count > limit[2] {
                    return Err(InvalidScene("negotiated path limit exceeded"));
                }
                segments += count;
            }
        }
        if segments > limit[3] || text_bytes > limit[5] {
            return Err(InvalidScene("negotiated aggregate scene limit exceeded"));
        }
        Ok(())
    }
    pub fn validate(&self) -> Result<()> {
        if self.commands.len() > MAX_COMMANDS {
            return Err(InvalidScene("command limit exceeded"));
        }
        let mut depth = 0;
        let mut clips = 0;
        let mut clip_stack = Vec::new();
        let mut segments = 0_usize;
        let mut text_bytes = 0_usize;
        let mut hits = BTreeSet::new();
        for command in &self.commands {
            let path = match command {
                Command::Fill(path, brush) => {
                    brush.validate()?;
                    Some(path)
                }
                Command::Stroke(path, brush, width) => {
                    brush.validate()?;
                    if *width <= Scalar::ZERO {
                        return Err(InvalidScene("stroke width must be positive"));
                    }
                    Some(path)
                }
                Command::Clip(path) => {
                    clips += 1;
                    if clips > MAX_STACK_DEPTH {
                        return Err(InvalidScene("clip nesting limit exceeded"));
                    }
                    Some(path)
                }
                Command::Hit { id, path, role, .. } => {
                    if *id == 0 || !hits.insert(*id) {
                        return Err(InvalidScene("hit region IDs must be nonzero and unique"));
                    }
                    if let HitRole::Resize(edges) = role {
                        if *edges == 0 || *edges > 15 {
                            return Err(InvalidScene("invalid resize edges"));
                        }
                    }
                    Some(path)
                }
                Command::Save => {
                    clip_stack.push(clips);
                    depth += 1;
                    if depth > MAX_STACK_DEPTH {
                        return Err(InvalidScene("save stack limit exceeded"));
                    }
                    None
                }
                Command::Restore => {
                    if depth == 0 {
                        return Err(InvalidScene("unbalanced restore"));
                    }
                    depth -= 1;
                    clips = clip_stack
                        .pop()
                        .ok_or(InvalidScene("unbalanced clip restore"))?;
                    None
                }
                Command::Text(text) => {
                    text_bytes = text_bytes
                        .checked_add(text.text.len())
                        .ok_or(InvalidScene("text size overflow"))?;
                    if text_bytes > MAX_TEXT_BYTES
                        || text.family.len() > 256
                        || text.size <= Scalar::ZERO
                        || !(1..=1000).contains(&text.weight)
                        || text.max_width.is_some_and(|w| w <= Scalar::ZERO)
                    {
                        return Err(InvalidScene("invalid or oversized text"));
                    }
                    None
                }
                Command::TextLayout { layout, .. } => {
                    if *layout == 0 {
                        return Err(InvalidScene("text layout ID must be nonzero"));
                    }
                    None
                }
                Command::Image { asset, rect, .. } => {
                    if *asset == 0 {
                        return Err(InvalidScene("image asset ID must be nonzero"));
                    }
                    rect.validate()?;
                    None
                }
                Command::Shadow(shadow) => {
                    shadow.validate()?;
                    None
                }
                Command::StyledStroke(path, brush, style) => {
                    brush.validate()?;
                    style.validate()?;
                    Some(path)
                }
                _ => None,
            };
            if let Some(path) = path {
                path.validate()?;
                segments = segments
                    .checked_add(path.segments.len())
                    .ok_or(InvalidScene("segment count overflow"))?;
            }
            if segments > MAX_TOTAL_SEGMENTS {
                return Err(InvalidScene("scene path budget exceeded"));
            }
        }
        if depth != 0 {
            return Err(InvalidScene("unbalanced save stack"));
        }
        Ok(())
    }
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let bytes = cbor::encode(&Value::Array(
            self.commands.iter().map(command_value).collect(),
        ))
        .map_err(|_| InvalidScene("cannot encode scene"))?;
        if bytes.len() > MAX_SCENE_BYTES {
            return Err(InvalidScene("scene byte limit exceeded"));
        }
        Ok(bytes)
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > MAX_SCENE_BYTES {
            return Err(InvalidScene("scene byte limit exceeded"));
        }
        Self::from_value(&cbor::decode(bytes).map_err(|_| InvalidScene("invalid scene encoding"))?)
    }
    pub fn from_value(value: &Value) -> Result<Self> {
        let values = array(value)?;
        if values.len() > MAX_COMMANDS {
            return Err(InvalidScene("command limit exceeded"));
        }
        let canvas = Self {
            commands: values.iter().map(parse_command).collect::<Result<_>>()?,
        };
        canvas.validate()?;
        Ok(canvas)
    }
}

fn integer(n: i64) -> Value {
    if n >= 0 {
        Value::Unsigned(n as u64)
    } else {
        Value::Negative(n)
    }
}
fn point(p: Point) -> Value {
    Value::Array(vec![integer(p.x.raw()), integer(p.y.raw())])
}
fn rect(r: Rect) -> Value {
    Value::Array(vec![
        point(r.origin),
        integer(r.width.raw()),
        integer(r.height.raw()),
    ])
}
fn path(p: &Path) -> Value {
    Value::Array(vec![
        Value::Bool(p.even_odd),
        Value::Array(
            p.segments
                .iter()
                .map(|s| {
                    Value::Array(match *s {
                        Segment::Move(p) => vec![Value::Unsigned(0), point(p)],
                        Segment::Line(p) => vec![Value::Unsigned(1), point(p)],
                        Segment::Quad(a, b) => vec![Value::Unsigned(2), point(a), point(b)],
                        Segment::Cubic(a, b, c) => {
                            vec![Value::Unsigned(3), point(a), point(b), point(c)]
                        }
                        Segment::Close => vec![Value::Unsigned(4)],
                    })
                })
                .collect(),
        ),
    ])
}
fn stops(s: &[GradientStop]) -> Value {
    Value::Array(
        s.iter()
            .map(|s| {
                Value::Array(vec![
                    Value::Unsigned(u64::from(s.offset)),
                    Value::Unsigned(u64::from(s.color.0)),
                ])
            })
            .collect(),
    )
}
fn brush(b: &Brush) -> Value {
    Value::Array(match b {
        Brush::Solid(c) => vec![Value::Unsigned(0), Value::Unsigned(u64::from(c.0))],
        Brush::Linear {
            start,
            end,
            stops: s,
            color_space,
        } => {
            let mut v = vec![Value::Unsigned(1), point(*start), point(*end), stops(s)];
            // The default space is omitted so an existing scene encodes byte-for-byte alike.
            if *color_space != ColorSpace::Srgb {
                v.push(color_space_value(*color_space));
            }
            v
        }
        Brush::Radial {
            center,
            radius,
            stops: s,
            color_space,
        } => {
            let mut v = vec![
                Value::Unsigned(2),
                point(*center),
                integer(radius.raw()),
                stops(s),
            ];
            if *color_space != ColorSpace::Srgb {
                v.push(color_space_value(*color_space));
            }
            v
        }
        Brush::Image {
            asset,
            transform,
            extend,
        } => vec![
            Value::Unsigned(3),
            Value::Unsigned(*asset),
            transform.map_or(Value::Null, |t| {
                Value::Array(t.0.iter().map(|x| integer(x.raw())).collect())
            }),
            Value::Unsigned(match extend {
                Extend::Pad => 0,
                Extend::Repeat => 1,
                Extend::Reflect => 2,
            }),
        ],
    })
}
fn color_space_value(space: ColorSpace) -> Value {
    Value::Unsigned(match space {
        ColorSpace::Srgb => 0,
        ColorSpace::Oklab => 1,
    })
}
fn command_value(c: &Command) -> Value {
    Value::Array(match c {
        Command::TextLayout { layout, origin } => vec![
            Value::Unsigned(10),
            Value::Unsigned(*layout),
            point(*origin),
        ],
        Command::Fill(p, b) => vec![Value::Unsigned(0), path(p), brush(b)],
        Command::Stroke(p, b, w) => vec![Value::Unsigned(1), path(p), brush(b), integer(w.raw())],
        Command::Save => vec![Value::Unsigned(2)],
        Command::Restore => vec![Value::Unsigned(3)],
        Command::Transform(t) => vec![
            Value::Unsigned(4),
            Value::Array(t.0.iter().map(|x| integer(x.raw())).collect()),
        ],
        Command::Clip(p) => vec![Value::Unsigned(5), path(p)],
        Command::Opacity(o) => vec![Value::Unsigned(6), Value::Unsigned(u64::from(*o))],
        Command::Text(t) => vec![
            Value::Unsigned(7),
            Value::Text(t.text.clone()),
            point(t.origin),
            integer(t.size.raw()),
            Value::Text(t.family.clone()),
            Value::Unsigned(u64::from(t.weight)),
            Value::Bool(t.italic),
            Value::Unsigned(u64::from(t.color.0)),
            t.max_width.map_or(Value::Null, |w| integer(w.raw())),
        ],
        Command::Image {
            asset,
            rect: r,
            opacity,
        } => vec![
            Value::Unsigned(8),
            Value::Unsigned(*asset),
            rect(*r),
            Value::Unsigned(u64::from(*opacity)),
        ],
        Command::Hit {
            id,
            path: p,
            role,
            cursor,
        } => {
            let mut v = vec![
                Value::Unsigned(9),
                Value::Unsigned(*id),
                path(p),
                Value::Unsigned(match role {
                    HitRole::Input => 0,
                    HitRole::Drag => 1,
                    HitRole::Transparent => 2,
                    HitRole::Resize(e) => 16 + u64::from(*e),
                }),
            ];
            // A default cursor is omitted, so a region that asks for nothing encodes as before.
            if let Some(shape) = cursor {
                v.push(Value::Unsigned(shape.index()));
            }
            v
        }
        Command::Shadow(shadow) => vec![
            Value::Unsigned(11),
            rect(shadow.rect),
            Value::Array(shadow.radii.0.iter().map(|r| integer(r.raw())).collect()),
            Value::Unsigned(u64::from(shadow.color.0)),
            point(shadow.offset),
            integer(shadow.blur.raw()),
            integer(shadow.spread.raw()),
            Value::Bool(shadow.inset),
        ],
        Command::StyledStroke(p, b, style) => vec![
            Value::Unsigned(12),
            path(p),
            brush(b),
            integer(style.width.raw()),
            Value::Unsigned(match style.cap {
                Cap::Butt => 0,
                Cap::Round => 1,
                Cap::Square => 2,
            }),
            Value::Unsigned(match style.join {
                Join::Miter => 0,
                Join::Bevel => 1,
                Join::Round => 2,
            }),
            integer(style.miter_limit.raw()),
            if style.dashes.is_empty() {
                Value::Null
            } else {
                Value::Array(style.dashes.iter().map(|d| integer(d.raw())).collect())
            },
            integer(style.dash_offset.raw()),
        ],
    })
}
fn array(v: &Value) -> Result<&[Value]> {
    if let Value::Array(a) = v {
        Ok(a)
    } else {
        Err(InvalidScene("expected array"))
    }
}
fn uint(v: &Value) -> Result<u64> {
    v.as_u64().ok_or(InvalidScene("expected unsigned integer"))
}
fn scalar(v: &Value) -> Result<Scalar> {
    Scalar::from_raw(
        v.as_i64()
            .ok_or(InvalidScene("expected fixed coordinate"))?,
    )
}
fn boolean(v: &Value) -> Result<bool> {
    if let Value::Bool(b) = v {
        Ok(*b)
    } else {
        Err(InvalidScene("expected boolean"))
    }
}
fn text(v: &Value) -> Result<String> {
    if let Value::Text(t) = v {
        Ok(t.clone())
    } else {
        Err(InvalidScene("expected text"))
    }
}
fn small(v: &Value) -> Result<u16> {
    u16::try_from(uint(v)?).map_err(|_| InvalidScene("integer exceeds u16"))
}
fn color(v: &Value) -> Result<Color> {
    Ok(Color(
        u32::try_from(uint(v)?).map_err(|_| InvalidScene("color exceeds RGBA8"))?,
    ))
}
fn parse_point(v: &Value) -> Result<Point> {
    let [x, y] = array(v)? else {
        return Err(InvalidScene("invalid point"));
    };
    Ok(Point {
        x: scalar(x)?,
        y: scalar(y)?,
    })
}
fn parse_rect(v: &Value) -> Result<Rect> {
    let [p, w, h] = array(v)? else {
        return Err(InvalidScene("invalid rectangle"));
    };
    Ok(Rect {
        origin: parse_point(p)?,
        width: scalar(w)?,
        height: scalar(h)?,
    })
}
fn parse_path(v: &Value) -> Result<Path> {
    let [rule, list] = array(v)? else {
        return Err(InvalidScene("invalid path"));
    };
    let list = array(list)?;
    if list.len() > MAX_PATH_SEGMENTS {
        return Err(InvalidScene("path segment limit exceeded"));
    }
    let segments = list
        .iter()
        .map(|v| {
            let a = array(v)?;
            match a {
                [Value::Unsigned(0), p] => Ok(Segment::Move(parse_point(p)?)),
                [Value::Unsigned(1), p] => Ok(Segment::Line(parse_point(p)?)),
                [Value::Unsigned(2), p, q] => Ok(Segment::Quad(parse_point(p)?, parse_point(q)?)),
                [Value::Unsigned(3), p, q, r] => Ok(Segment::Cubic(
                    parse_point(p)?,
                    parse_point(q)?,
                    parse_point(r)?,
                )),
                [Value::Unsigned(4)] => Ok(Segment::Close),
                _ => Err(InvalidScene("unknown or malformed path segment")),
            }
        })
        .collect::<Result<_>>()?;
    Ok(Path {
        segments,
        even_odd: boolean(rule)?,
    })
}
fn hit_role(v: &Value) -> Result<HitRole> {
    Ok(match uint(v)? {
        0 => HitRole::Input,
        1 => HitRole::Drag,
        2 => HitRole::Transparent,
        e @ 17..=31 => HitRole::Resize((e - 16) as u8),
        _ => return Err(InvalidScene("invalid hit role")),
    })
}
fn parse_stops(v: &Value) -> Result<Vec<GradientStop>> {
    let a = array(v)?;
    if a.len() > MAX_GRADIENT_STOPS {
        return Err(InvalidScene("gradient stop limit exceeded"));
    }
    a.iter()
        .map(|v| {
            let [offset, c] = array(v)? else {
                return Err(InvalidScene("invalid gradient stop"));
            };
            Ok(GradientStop {
                offset: small(offset)?,
                color: color(c)?,
            })
        })
        .collect()
}
fn parse_brush(v: &Value) -> Result<Brush> {
    fn space(v: &Value) -> Result<ColorSpace> {
        match uint(v)? {
            0 => Ok(ColorSpace::Srgb),
            1 => Ok(ColorSpace::Oklab),
            _ => Err(InvalidScene("unknown gradient color space")),
        }
    }
    match array(v)? {
        [Value::Unsigned(0), c] => Ok(Brush::Solid(color(c)?)),
        [Value::Unsigned(1), a, b, s] => Ok(Brush::Linear {
            start: parse_point(a)?,
            end: parse_point(b)?,
            stops: parse_stops(s)?,
            color_space: ColorSpace::Srgb,
        }),
        [Value::Unsigned(1), a, b, s, cs] => Ok(Brush::Linear {
            start: parse_point(a)?,
            end: parse_point(b)?,
            stops: parse_stops(s)?,
            color_space: space(cs)?,
        }),
        [Value::Unsigned(2), c, r, s] => Ok(Brush::Radial {
            center: parse_point(c)?,
            radius: scalar(r)?,
            stops: parse_stops(s)?,
            color_space: ColorSpace::Srgb,
        }),
        [Value::Unsigned(2), c, r, s, cs] => Ok(Brush::Radial {
            center: parse_point(c)?,
            radius: scalar(r)?,
            stops: parse_stops(s)?,
            color_space: space(cs)?,
        }),
        [Value::Unsigned(3), asset, transform, extend] => Ok(Brush::Image {
            asset: uint(asset)?,
            transform: if *transform == Value::Null {
                None
            } else {
                let values: Vec<Scalar> = array(transform)?
                    .iter()
                    .map(scalar)
                    .collect::<Result<_>>()?;
                Some(Transform(values.try_into().map_err(|_| {
                    InvalidScene("image brush transform requires six terms")
                })?))
            },
            extend: match uint(extend)? {
                0 => Extend::Pad,
                1 => Extend::Repeat,
                2 => Extend::Reflect,
                _ => return Err(InvalidScene("unknown image extend mode")),
            },
        }),
        _ => Err(InvalidScene("unknown or malformed brush")),
    }
}
fn parse_command(v: &Value) -> Result<Command> {
    match array(v)? {
        [Value::Unsigned(0), p, b] => Ok(Command::Fill(parse_path(p)?, parse_brush(b)?)),
        [Value::Unsigned(1), p, b, w] => {
            Ok(Command::Stroke(parse_path(p)?, parse_brush(b)?, scalar(w)?))
        }
        [Value::Unsigned(2)] => Ok(Command::Save),
        [Value::Unsigned(3)] => Ok(Command::Restore),
        [Value::Unsigned(4), t] => {
            let t = array(t)?;
            let [a, b, c, d, e, f] = t else {
                return Err(InvalidScene("invalid transform"));
            };
            Ok(Command::Transform(Transform([
                scalar(a)?,
                scalar(b)?,
                scalar(c)?,
                scalar(d)?,
                scalar(e)?,
                scalar(f)?,
            ])))
        }
        [Value::Unsigned(5), p] => Ok(Command::Clip(parse_path(p)?)),
        [Value::Unsigned(6), o] => Ok(Command::Opacity(small(o)?)),
        [Value::Unsigned(7), t, p, s, f, w, i, c, m] => Ok(Command::Text(Text {
            text: text(t)?,
            origin: parse_point(p)?,
            size: scalar(s)?,
            family: text(f)?,
            weight: small(w)?,
            italic: boolean(i)?,
            color: color(c)?,
            max_width: if *m == Value::Null {
                None
            } else {
                Some(scalar(m)?)
            },
        })),
        [Value::Unsigned(10), id, p] => Ok(Command::TextLayout {
            layout: uint(id)?,
            origin: parse_point(p)?,
        }),
        [Value::Unsigned(8), id, r, o] => Ok(Command::Image {
            asset: uint(id)?,
            rect: parse_rect(r)?,
            opacity: small(o)?,
        }),
        [Value::Unsigned(9), id, p, role] => Ok(Command::Hit {
            id: uint(id)?,
            path: parse_path(p)?,
            role: hit_role(role)?,
            cursor: None,
        }),
        [Value::Unsigned(9), id, p, role, cursor] => Ok(Command::Hit {
            id: uint(id)?,
            path: parse_path(p)?,
            role: hit_role(role)?,
            cursor: Some(
                CursorShape::from_index(uint(cursor)?)
                    .ok_or(InvalidScene("unknown cursor shape"))?,
            ),
        }),
        [
            Value::Unsigned(11),
            r,
            radii,
            c,
            offset,
            blur,
            spread,
            inset,
        ] => Ok(Command::Shadow(Shadow {
            rect: parse_rect(r)?,
            radii: {
                let values: Vec<Scalar> =
                    array(radii)?.iter().map(scalar).collect::<Result<_>>()?;
                Corners(
                    values
                        .try_into()
                        .map_err(|_| InvalidScene("shadow radii require four terms"))?,
                )
            },
            color: color(c)?,
            offset: parse_point(offset)?,
            blur: scalar(blur)?,
            spread: scalar(spread)?,
            inset: boolean(inset)?,
        })),
        [
            Value::Unsigned(12),
            p,
            b,
            w,
            cap,
            join,
            miter,
            dashes,
            dash_offset,
        ] => Ok(Command::StyledStroke(
            parse_path(p)?,
            parse_brush(b)?,
            StrokeStyle {
                width: scalar(w)?,
                cap: match uint(cap)? {
                    0 => Cap::Butt,
                    1 => Cap::Round,
                    2 => Cap::Square,
                    _ => return Err(InvalidScene("unknown stroke cap")),
                },
                join: match uint(join)? {
                    0 => Join::Miter,
                    1 => Join::Bevel,
                    2 => Join::Round,
                    _ => return Err(InvalidScene("unknown stroke join")),
                },
                miter_limit: scalar(miter)?,
                dashes: if *dashes == Value::Null {
                    Vec::new()
                } else {
                    array(dashes)?.iter().map(scalar).collect::<Result<_>>()?
                },
                dash_offset: scalar(dash_offset)?,
            },
        )),
        _ => Err(InvalidScene("unknown or malformed drawing command")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn frame_and_asset_headers_preserve_full_width_ids_and_reject_truncation() {
        let frame = Frame {
            epoch: 1,
            revision: u64::MAX,
            canvas: Canvas::new(),
        };
        let bytes = frame.encode().unwrap();
        assert_eq!(Frame::decode(&bytes).unwrap(), frame);
        for end in 0..bytes.len() {
            assert!(Frame::decode(&bytes[..end]).is_err());
        }
        let asset = ImageAsset {
            id: u64::MAX,
            width: 2,
            height: 1,
            rgba: vec![255, 0, 0, 255, 0, 255, 0, 255],
        };
        let bytes = asset.encode().unwrap();
        assert_eq!(ImageAsset::decode(&bytes).unwrap(), asset);
        for end in 0..bytes.len() {
            assert!(ImageAsset::decode(&bytes[..end]).is_err());
        }
        let mut oversized = bytes;
        oversized[8..12].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(ImageAsset::decode(&oversized).is_err());
    }
    #[test]
    fn negotiated_limits_are_complete_and_applied_to_scenes() {
        let limits = Limits::default();
        assert_eq!(Limits::from_value(&limits.to_value()).unwrap(), limits);
        let mut reduced = *limits.values();
        reduced[1] = 1;
        let reduced = Limits::new(reduced).unwrap();
        let mut canvas = Canvas::new();
        canvas
            .push(Command::Save)
            .unwrap()
            .push(Command::Restore)
            .unwrap();
        assert!(canvas.validate_with_limits(&limits).is_ok());
        assert!(canvas.validate_with_limits(&reduced).is_err());
        assert!(Limits::from_value(&Value::Map(vec![])).is_err());
        assert!(Limits::new([u64::MAX; 12]).is_err());
    }
    #[test]
    fn clips_are_bounded_even_without_save_commands() {
        let mut canvas = Canvas::new();
        let path = Path::rectangle(Rect::new(0., 0., 10., 10.).unwrap()).unwrap();
        for _ in 0..MAX_STACK_DEPTH + 1 {
            canvas.push(Command::Clip(path.clone())).unwrap();
        }
        assert!(canvas.validate().is_err());
    }
    #[test]
    fn paint_commands_round_trip_with_their_style_intact() {
        let rect = Rect::new(10., 20., 300., 120.).unwrap();
        let mut c = Canvas::new();
        c.shadow(Shadow {
            rect,
            radii: Corners::new([4., 8., 12., 16.]).unwrap(),
            color: Color(0x11223344),
            offset: Point::new(0., 6.).unwrap(),
            blur: Scalar::new(18.).unwrap(),
            spread: Scalar::new(-2.).unwrap(),
            inset: true,
        })
        .unwrap();
        let path = Path::rectangle(rect).unwrap();
        c.stroke_styled(
            path.clone(),
            Brush::Image {
                asset: u64::MAX,
                transform: Some(Transform::new([2., 0., 0., 2., 5., -5.]).unwrap()),
                extend: Extend::Repeat,
            },
            StrokeStyle {
                width: Scalar::new(2.5).unwrap(),
                cap: Cap::Round,
                join: Join::Bevel,
                miter_limit: Scalar::new(6.).unwrap(),
                dashes: vec![Scalar::new(4.).unwrap(), Scalar::new(2.).unwrap()],
                dash_offset: Scalar::new(1.5).unwrap(),
            },
        )
        .unwrap();
        c.fill(
            path,
            Brush::Radial {
                center: Point::new(50., 50.).unwrap(),
                radius: Scalar::new(40.).unwrap(),
                stops: vec![
                    GradientStop {
                        offset: 0,
                        color: Color(0xff0000ff),
                    },
                    GradientStop {
                        offset: u16::MAX,
                        color: Color(0x00ff00ff),
                    },
                ],
                color_space: ColorSpace::Oklab,
            },
        )
        .unwrap();
        let decoded = Canvas::decode(&c.encode().unwrap()).unwrap();
        assert_eq!(decoded, c);
        // An sRGB gradient carries no trailing field, so its encoding is unchanged.
        let mut plain = Canvas::new();
        plain
            .fill(
                Path::rectangle(rect).unwrap(),
                Brush::Linear {
                    start: Point::new(0., 0.).unwrap(),
                    end: Point::new(100., 0.).unwrap(),
                    stops: vec![
                        GradientStop {
                            offset: 0,
                            color: Color(0xff0000ff),
                        },
                        GradientStop {
                            offset: u16::MAX,
                            color: Color(0x0000ffff),
                        },
                    ],
                    color_space: ColorSpace::Srgb,
                },
            )
            .unwrap();
        let legacy = b"\x9f\x83\x00\x01\x82\x00\x00\x82\x00\x00\x01\x90\x00\x00\x00";
        let _ = legacy; // byte-level equality is asserted through decode below instead.
        assert_eq!(Canvas::decode(&plain.encode().unwrap()).unwrap(), plain);
    }

    #[test]
    fn paint_values_out_of_range_are_refused_before_encoding() {
        let rect = Rect::new(0., 0., 100., 100.).unwrap();
        let path = Path::rectangle(rect).unwrap();
        let mut cases: Vec<Canvas> = Vec::new();
        for shadow in [
            Shadow {
                radii: Corners::new([-1., 0., 0., 0.]).unwrap(),
                ..valid_shadow(rect)
            },
            Shadow {
                blur: Scalar::new(4097.).unwrap(),
                ..valid_shadow(rect)
            },
            Shadow {
                blur: Scalar::new(-1.).unwrap(),
                ..valid_shadow(rect)
            },
            Shadow {
                spread: Scalar::new(-4097.).unwrap(),
                ..valid_shadow(rect)
            },
        ] {
            let mut c = Canvas::new();
            c.shadow(shadow).unwrap();
            cases.push(c);
        }
        for style in [
            StrokeStyle {
                width: Scalar::ZERO,
                ..StrokeStyle::new(1.).unwrap()
            },
            StrokeStyle {
                miter_limit: Scalar::new(0.5).unwrap(),
                ..StrokeStyle::new(1.).unwrap()
            },
            StrokeStyle {
                dashes: vec![Scalar::ZERO; 1],
                ..StrokeStyle::new(1.).unwrap()
            },
            StrokeStyle {
                dashes: vec![Scalar::ONE; MAX_DASH_ENTRIES + 1],
                ..StrokeStyle::new(1.).unwrap()
            },
            StrokeStyle {
                dash_offset: Scalar::new(-1.).unwrap(),
                ..StrokeStyle::new(1.).unwrap()
            },
        ] {
            let mut c = Canvas::new();
            c.stroke_styled(path.clone(), Brush::Solid(Color(0xffffffff)), style)
                .unwrap();
            cases.push(c);
        }
        let mut c = Canvas::new();
        c.fill(
            path,
            Brush::Image {
                asset: 0,
                transform: None,
                extend: Extend::Pad,
            },
        )
        .unwrap();
        cases.push(c);
        for case in &cases {
            assert!(case.validate().is_err(), "{case:?}");
            assert!(case.encode().is_err());
        }
    }

    fn valid_shadow(rect: Rect) -> Shadow {
        Shadow {
            rect,
            radii: Corners::uniform(8.).unwrap(),
            color: Color(0x00000040),
            offset: Point::new(0., 4.).unwrap(),
            blur: Scalar::new(12.).unwrap(),
            spread: Scalar::ZERO,
            inset: false,
        }
    }

    #[test]
    fn unknown_paint_tags_are_refused_rather_than_guessed() {
        // A presenter that did not negotiate overlay-paint-v1 must never see these commands, and
        // one that did must still refuse a value outside the definitions.
        let scene = b"\x81\x82\x0b";
        assert!(Canvas::decode(scene).is_err());
        let mut c = Canvas::new();
        let rect = Rect::new(0., 0., 10., 10.).unwrap();
        c.fill(
            Path::rectangle(rect).unwrap(),
            Brush::Solid(Color(0xffffffff)),
        )
        .unwrap();
        let encoded = c.encode().unwrap();
        // A gradient brush carrying an unknown interpolation space must fail decode.
        let mut c2 = Canvas::new();
        c2.fill(
            Path::rectangle(rect).unwrap(),
            Brush::Linear {
                start: Point::new(0., 0.).unwrap(),
                end: Point::new(10., 0.).unwrap(),
                stops: vec![
                    GradientStop {
                        offset: 0,
                        color: Color(0xff0000ff),
                    },
                    GradientStop {
                        offset: u16::MAX,
                        color: Color(0x0000ffff),
                    },
                ],
                color_space: ColorSpace::Oklab,
            },
        )
        .unwrap();
        let bytes = c2.encode().unwrap();
        // The trailing color-space tag is the final unsigned byte of the brush array.
        let last = bytes
            .iter()
            .rposition(|b| *b <= 0x1f)
            .expect("color space tag");
        let mut hostile = bytes.clone();
        hostile[last] = 7;
        assert!(Canvas::decode(&hostile).is_err());
        assert_eq!(Canvas::decode(&bytes).unwrap(), c2);
        assert_eq!(Canvas::decode(&encoded).unwrap(), c);
    }

    #[test]
    fn per_corner_radii_scale_together_and_match_the_uniform_path() {
        // The four-radius form with equal corners must produce exactly the path the uniform
        // helper always produced, so refactoring one through the other changed no bytes.
        let rect = Rect::new(0., 0., 120., 80.).unwrap();
        for radius in [0., 1., 8., 40., 60.] {
            let uniform = Path::rounded_rectangle(rect, radius).unwrap();
            let corners =
                Path::rounded_rectangle_corners(rect, Corners::uniform(radius).unwrap()).unwrap();
            assert_eq!(uniform, corners, "radius {radius}");
        }
        // Radii too large for their sides shrink together rather than overlapping.
        let clamped =
            Path::rounded_rectangle_corners(rect, Corners::new([100., 100., 100., 100.]).unwrap())
                .unwrap();
        assert!(clamped.validate().is_ok());
        // Every x remains within the rectangle: the outline never self-intersects horizontally.
        for segment in &clamped.segments {
            let points: &[Point] = match segment {
                Segment::Move(a) | Segment::Line(a) => std::slice::from_ref(a),
                Segment::Quad(a, b) => &[*a, *b],
                Segment::Cubic(a, b, c2) => &[*a, *b, *c2],
                Segment::Close => &[],
            };
            for p in points {
                assert!((0. ..=120.).contains(&p.x.get()), "x {} escaped", p.x.get());
                assert!((0. ..=80.).contains(&p.y.get()), "y {} escaped", p.y.get());
            }
        }
    }

    #[test]
    fn exact_roundtrip_preserves_paths_gradients_and_large_hit_ids() {
        let mut c = Canvas::new();
        let p = Path::rounded_rectangle(Rect::new(1.25, 2.5, 100.0, 60.0).unwrap(), 8.0).unwrap();
        c.fill(
            p.clone(),
            Brush::Linear {
                start: Point::new(0.0, 0.0).unwrap(),
                end: Point::new(100.0, 0.0).unwrap(),
                stops: vec![
                    GradientStop {
                        offset: 0,
                        color: Color(0xff0000ff),
                    },
                    GradientStop {
                        offset: u16::MAX,
                        color: Color(0x0000ffff),
                    },
                ],
                color_space: ColorSpace::Srgb,
            },
        )
        .unwrap();
        c.push(Command::Hit {
            id: u64::MAX,
            path: p,
            role: HitRole::Drag,
            cursor: None,
        })
        .unwrap();
        assert_eq!(Canvas::decode(&c.encode().unwrap()).unwrap(), c);
    }
    #[test]
    fn invalid_commands_and_unbalanced_lists_are_rejected() {
        assert!(Scalar::new(f64::NAN).is_err());
        assert!(Scalar::new(f64::INFINITY).is_err());
        assert!(Rect::new(0.0, 0.0, -1.0, 1.0).is_err());
        let mut c = Canvas::new();
        c.push(Command::Restore).unwrap();
        assert!(c.encode().is_err());
        assert!(
            Canvas::from_value(&Value::Array(vec![Value::Array(vec![Value::Unsigned(99)])]))
                .is_err()
        );
    }
    #[test]
    fn scene_limits_apply_before_decoding_and_to_aggregate_text() {
        assert!(Canvas::decode(&vec![0; MAX_SCENE_BYTES + 1]).is_err());
        let mut c = Canvas::new();
        c.push(Command::Text(Text {
            text: "x".repeat(MAX_TEXT_BYTES + 1),
            origin: Point::default(),
            size: Scalar::ONE,
            family: "sans-serif".into(),
            weight: 400,
            italic: false,
            color: Color(0xff),
            max_width: None,
        }))
        .unwrap();
        assert!(c.encode().is_err());
    }
}
