//! Typed overlay control and interactive-lane payloads.
//!
//! The authenticated connection supplies the owner; a payload cannot nominate another session.
//! These codecs validate wire shape and local bounds. The presenter must additionally validate
//! authority, current generations/revisions, and window eligibility before applying a request.

use crate::cbor::Value;
use crate::identity::{SessionIdentity, SurfaceIdentity};
use crate::messages::{MessageError, PayloadMap, StrictMap, invalid_value, validate_header_object};
use crate::vector::{Point, Rect, Scalar};

use super::{DismissReason, Event, WindowMode, WindowOptions, valid_event};

const SCHEMA: &str = "overlay";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowAddress {
    pub context_id: u64,
    pub surface_id: u64,
    pub generation: u64,
}

impl WindowAddress {
    pub fn identity(self, owner: SessionIdentity) -> Result<SurfaceIdentity, MessageError> {
        owner
            .context(self.context_id)
            .and_then(|c| c.surface(self.surface_id))
            .map_err(|_| bad(0, "window identity must be nonzero"))
    }

    pub fn validate(self, object: u64) -> Result<(), MessageError> {
        validate_header_object(object, self.surface_id)?;
        if self.context_id == 0 || self.surface_id == 0 || self.generation == 0 {
            return Err(bad(0, "identity and generation must be nonzero"));
        }
        Ok(())
    }

    fn payload(self) -> PayloadMap {
        vec![
            (0, u(self.context_id)),
            (1, u(self.surface_id)),
            (2, u(self.generation)),
        ]
    }

    fn decode(object: u64, map: &StrictMap<'_>) -> Result<Self, MessageError> {
        let address = Self {
            context_id: map.required_u64(0)?,
            surface_id: map.required_u64(1)?,
            generation: map.required_u64(2)?,
        };
        address.validate(object)?;
        Ok(address)
    }
}

/// Revision zero creates a window; a nonzero revision conditionally replaces its options.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetWindow {
    pub address: WindowAddress,
    pub expected_revision: u64,
    pub options: WindowOptions,
}

impl SetWindow {
    pub fn payload(&self, owner: SessionIdentity) -> Result<PayloadMap, MessageError> {
        self.address.validate(self.address.surface_id)?;
        self.options.validate().map_err(|e| bad(4, e.0))?;
        if self
            .options
            .parent
            .is_some_and(|p| p.context.session != owner)
        {
            return Err(bad(7, "parent belongs to another owner"));
        }
        if self.options.parent == Some(self.address.identity(owner)?) {
            return Err(bad(7, "window cannot parent itself"));
        }
        let mut values = self.address.payload();
        values.extend([
            (3, u(self.expected_revision)),
            (4, rectangle(self.options.bounds)),
            (
                5,
                u(match self.options.mode {
                    WindowMode::Floating => 0,
                    WindowMode::Popup => 1,
                    WindowMode::Modal => 2,
                }),
            ),
            (6, Value::Bool(self.options.visible)),
        ]);
        if let Some(parent) = self.options.parent {
            if parent.context.context_id == 0 || parent.surface_id == 0 {
                return Err(bad(7, "parent identity must be nonzero"));
            }
            values.push((
                7,
                Value::Array(vec![u(parent.context.context_id), u(parent.surface_id)]),
            ));
        }
        values.push((
            8,
            Value::Array(vec![
                scalar(self.options.min_width),
                scalar(self.options.min_height),
            ]),
        ));
        Ok(values)
    }

    pub fn decode(
        owner: SessionIdentity,
        object: u64,
        value: &Value,
    ) -> Result<Self, MessageError> {
        let map = strict(value, &[0, 1, 2, 3, 4, 5, 6, 7, 8])?;
        Self::from_map(owner, object, &map)
    }

    fn from_map(
        owner: SessionIdentity,
        object: u64,
        map: &StrictMap<'_>,
    ) -> Result<Self, MessageError> {
        let address = WindowAddress::decode(object, map)?;
        let parent = map
            .optional(7)
            .map(|value| {
                let fields = array::<2>(value, 7)?;
                owner
                    .context(unsigned(&fields[0], 7)?)
                    .and_then(|c| c.surface(fields[1].as_u64().unwrap_or(0)))
                    .map_err(|_| bad(7, "invalid parent identity"))
            })
            .transpose()?;
        let minimum = array::<2>(map.required(8)?, 8)?;
        let options = WindowOptions {
            bounds: decode_rectangle(map.required(4)?, 4)?,
            mode: match map.required_u64(5)? {
                0 => WindowMode::Floating,
                1 => WindowMode::Popup,
                2 => WindowMode::Modal,
                _ => return Err(bad(5, "unknown window mode")),
            },
            visible: map.required_bool(6)?,
            parent,
            min_width: decode_scalar(&minimum[0], 8)?,
            min_height: decode_scalar(&minimum[1], 8)?,
        };
        options.validate().map_err(|e| bad(4, e.0))?;
        if parent == Some(address.identity(owner)?) {
            return Err(bad(7, "window cannot parent itself"));
        }
        Ok(Self {
            address,
            expected_revision: map.required_u64(3)?,
            options,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowAction {
    Close,
    Focus,
    Raise,
    Lower,
    Center,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Action {
    pub address: WindowAddress,
    pub expected_revision: u64,
    pub action: WindowAction,
}
impl Action {
    pub fn payload(self) -> Result<PayloadMap, MessageError> {
        self.address.validate(self.address.surface_id)?;
        nonzero(self.expected_revision, 3)?;
        let mut fields = self.address.payload();
        fields.extend([
            (3, u(self.expected_revision)),
            (
                4,
                u(match self.action {
                    WindowAction::Close => 0,
                    WindowAction::Focus => 1,
                    WindowAction::Raise => 2,
                    WindowAction::Lower => 3,
                    WindowAction::Center => 4,
                }),
            ),
        ]);
        Ok(fields)
    }
    pub fn decode(object: u64, value: &Value) -> Result<Self, MessageError> {
        let map = strict(value, &[0, 1, 2, 3, 4])?;
        Ok(Self {
            address: WindowAddress::decode(object, &map)?,
            expected_revision: nonzero(map.required_u64(3)?, 3)?,
            action: match map.required_u64(4)? {
                0 => WindowAction::Close,
                1 => WindowAction::Focus,
                2 => WindowAction::Raise,
                3 => WindowAction::Lower,
                4 => WindowAction::Center,
                _ => return Err(bad(4, "unknown window action")),
            },
        })
    }
}

/// Authoritative viewport extent is independent of terminal cells and scrollback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Viewport {
    pub width: Scalar,
    pub height: Scalar,
    pub scale_numerator: u32,
    pub scale_denominator: u32,
}
impl Viewport {
    pub fn validate(self) -> Result<(), MessageError> {
        if self.width <= Scalar::ZERO
            || self.height <= Scalar::ZERO
            || self.scale_numerator == 0
            || self.scale_denominator == 0
        {
            return Err(bad(9, "viewport extent and scale must be positive"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    /// Key 3 is the current window revision, never zero in a successful reply.
    pub window: SetWindow,
    pub viewport: Viewport,
    pub focused: bool,
    pub scene_revision: u64,
}
impl Status {
    pub fn payload(&self, owner: SessionIdentity) -> Result<PayloadMap, MessageError> {
        nonzero(self.window.expected_revision, 3)?;
        self.viewport.validate()?;
        let mut fields = self.window.payload(owner)?;
        fields.extend([
            (
                9,
                Value::Array(vec![
                    scalar(self.viewport.width),
                    scalar(self.viewport.height),
                ]),
            ),
            (
                10,
                Value::Array(vec![
                    u(u64::from(self.viewport.scale_numerator)),
                    u(u64::from(self.viewport.scale_denominator)),
                ]),
            ),
            (11, Value::Bool(self.focused)),
            (12, u(self.scene_revision)),
        ]);
        Ok(fields)
    }
    pub fn decode(
        owner: SessionIdentity,
        object: u64,
        value: &Value,
    ) -> Result<Self, MessageError> {
        let map = strict(value, &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12])?;
        let window = SetWindow::from_map(owner, object, &map)?;
        nonzero(window.expected_revision, 3)?;
        let extent = array::<2>(map.required(9)?, 9)?;
        let scale = array::<2>(map.required(10)?, 10)?;
        let viewport = Viewport {
            width: decode_scalar(&extent[0], 9)?,
            height: decode_scalar(&extent[1], 9)?,
            scale_numerator: small(&scale[0], 10)?,
            scale_denominator: small(&scale[1], 10)?,
        };
        viewport.validate()?;
        Ok(Self {
            window,
            viewport,
            focused: map.required_bool(11)?,
            scene_revision: map.required_u64(12)?,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Query {
    pub context_id: u64,
    pub surface_id: u64,
}
impl Query {
    pub fn payload(self) -> Result<PayloadMap, MessageError> {
        nonzero(self.context_id, 0)?;
        nonzero(self.surface_id, 1)?;
        Ok(vec![(0, u(self.context_id)), (1, u(self.surface_id))])
    }
    pub fn decode(object: u64, value: &Value) -> Result<Self, MessageError> {
        let map = strict(value, &[0, 1])?;
        let query = Self {
            context_id: nonzero(map.required_u64(0)?, 0)?,
            surface_id: nonzero(map.required_u64(1)?, 1)?,
        };
        validate_header_object(object, query.surface_id)?;
        Ok(query)
    }
}

/// Scene revision is the revision whose hit/dispatch tree received this input, not window geometry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputEvent {
    pub address: WindowAddress,
    pub scene_revision: u64,
    pub event: Event,
}
impl From<super::WindowEvent> for InputEvent {
    fn from(event: super::WindowEvent) -> Self {
        Self {
            address: WindowAddress {
                context_id: event.window.context.context_id,
                surface_id: event.window.surface_id,
                generation: event.generation,
            },
            scene_revision: event.scene_revision,
            event: event.event,
        }
    }
}
impl InputEvent {
    pub fn payload(&self) -> Result<PayloadMap, MessageError> {
        self.address.validate(self.address.surface_id)?;
        if !valid_event(&self.event) {
            return Err(bad(5, "invalid or oversized input payload"));
        }
        let (kind, payload) = match &self.event {
            Event::Focus(focused) => (0, Value::Bool(*focused)),
            Event::Pointer {
                position,
                region,
                button,
                modifiers,
            } => (
                1,
                Value::Array(vec![
                    point(*position),
                    u(*region),
                    button.map_or(Value::Null, |(b, down)| {
                        Value::Array(vec![u(u64::from(b)), Value::Bool(down)])
                    }),
                    u(u64::from(*modifiers)),
                ]),
            ),
            Event::Wheel {
                position,
                dx,
                dy,
                modifiers,
            } => (
                2,
                Value::Array(vec![
                    point(*position),
                    scalar(*dx),
                    scalar(*dy),
                    u(u64::from(*modifiers)),
                ]),
            ),
            Event::Key {
                physical,
                down,
                repeat,
                modifiers,
            } => (
                3,
                Value::Array(vec![
                    u(u64::from(*physical)),
                    Value::Bool(*down),
                    Value::Bool(*repeat),
                    u(u64::from(*modifiers)),
                ]),
            ),
            Event::Text(text) => (4, Value::Text(text.clone())),
            Event::Ime { preedit, selection } => (
                5,
                Value::Array(vec![
                    Value::Text(preedit.clone()),
                    selection.map_or(Value::Null, |(start, end)| {
                        Value::Array(vec![u(u64::from(start)), u(u64::from(end))])
                    }),
                ]),
            ),
            Event::Geometry { bounds, settled } => (
                6,
                Value::Array(vec![rectangle(*bounds), Value::Bool(*settled)]),
            ),
            Event::Dismissed(reason) => (
                7,
                u(match reason {
                    DismissReason::Escape => 0,
                    DismissReason::OutsidePress => 1,
                    DismissReason::Closed => 2,
                    DismissReason::OwnerLost => 3,
                    DismissReason::ParentClosed => 4,
                }),
            ),
            Event::Cancel => (8, Value::Null),
        };
        let mut fields = self.address.payload();
        fields.extend([(3, u(self.scene_revision)), (4, u(kind)), (5, payload)]);
        Ok(fields)
    }
    pub fn decode(object: u64, value: &Value) -> Result<Self, MessageError> {
        let map = strict(value, &[0, 1, 2, 3, 4, 5])?;
        let address = WindowAddress::decode(object, &map)?;
        let payload = map.required(5)?;
        let event = match map.required_u64(4)? {
            0 => Event::Focus(boolean(payload, 5)?),
            1 => {
                let a = array::<4>(payload, 5)?;
                let button = if a[2] == Value::Null {
                    None
                } else {
                    let b = array::<2>(&a[2], 5)?;
                    Some((
                        u16::try_from(unsigned(&b[0], 5)?)
                            .map_err(|_| bad(5, "button exceeds u16"))?,
                        boolean(&b[1], 5)?,
                    ))
                };
                Event::Pointer {
                    position: decode_point(&a[0], 5)?,
                    region: unsigned(&a[1], 5)?,
                    button,
                    modifiers: small(&a[3], 5)?,
                }
            }
            2 => {
                let a = array::<4>(payload, 5)?;
                Event::Wheel {
                    position: decode_point(&a[0], 5)?,
                    dx: decode_scalar(&a[1], 5)?,
                    dy: decode_scalar(&a[2], 5)?,
                    modifiers: small(&a[3], 5)?,
                }
            }
            3 => {
                let a = array::<4>(payload, 5)?;
                Event::Key {
                    physical: small(&a[0], 5)?,
                    down: boolean(&a[1], 5)?,
                    repeat: boolean(&a[2], 5)?,
                    modifiers: small(&a[3], 5)?,
                }
            }
            4 => Event::Text(text(payload)?.to_owned()),
            5 => {
                let a = array::<2>(payload, 5)?;
                let preedit = text(&a[0])?;
                let selection = if a[1] == Value::Null {
                    None
                } else {
                    let s = array::<2>(&a[1], 5)?;
                    Some((small(&s[0], 5)?, small(&s[1], 5)?))
                };
                // Validate byte offsets before allocating the owned text.
                if selection.is_some_and(|(s, e)| {
                    s > e
                        || !preedit.is_char_boundary(s as usize)
                        || !preedit.is_char_boundary(e as usize)
                }) {
                    return Err(bad(5, "IME selection is not a UTF-8 range"));
                }
                Event::Ime {
                    preedit: preedit.to_owned(),
                    selection,
                }
            }
            6 => {
                let a = array::<2>(payload, 5)?;
                Event::Geometry {
                    bounds: decode_rectangle(&a[0], 5)?,
                    settled: boolean(&a[1], 5)?,
                }
            }
            7 => Event::Dismissed(match unsigned(payload, 5)? {
                0 => DismissReason::Escape,
                1 => DismissReason::OutsidePress,
                2 => DismissReason::Closed,
                3 => DismissReason::OwnerLost,
                4 => DismissReason::ParentClosed,
                _ => return Err(bad(5, "unknown dismissal reason")),
            }),
            8 if *payload == Value::Null => Event::Cancel,
            _ => return Err(bad(4, "unknown input event or invalid payload")),
        };
        Ok(Self {
            address,
            scene_revision: map.required_u64(3)?,
            event,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capture {
    pub address: WindowAddress,
    pub scene_revision: u64,
    pub capture: bool,
}
impl Capture {
    pub fn payload(self) -> Result<PayloadMap, MessageError> {
        self.address.validate(self.address.surface_id)?;
        nonzero(self.scene_revision, 3)?;
        let mut fields = self.address.payload();
        fields.extend([(3, u(self.scene_revision)), (4, Value::Bool(self.capture))]);
        Ok(fields)
    }
    pub fn decode(object: u64, value: &Value) -> Result<Self, MessageError> {
        let map = strict(value, &[0, 1, 2, 3, 4])?;
        Ok(Self {
            address: WindowAddress::decode(object, &map)?,
            scene_revision: nonzero(map.required_u64(3)?, 3)?,
            capture: map.required_bool(4)?,
        })
    }
}

/// Renew only the authenticated lane generation, never another producer's lane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Renew {
    pub lane_generation: u64,
    pub watchdog_us: u64,
}
impl Renew {
    pub fn payload(self) -> Result<PayloadMap, MessageError> {
        nonzero(self.lane_generation, 0)?;
        if !(crate::input::MIN_WATCHDOG_US..=crate::input::MAX_WATCHDOG_US)
            .contains(&self.watchdog_us)
        {
            return Err(bad(1, "watchdog is outside the supported range"));
        }
        Ok(vec![(0, u(self.lane_generation)), (1, u(self.watchdog_us))])
    }
    pub fn decode(object: u64, value: &Value) -> Result<Self, MessageError> {
        validate_header_object(object, 0)?;
        let map = strict(value, &[0, 1])?;
        let renew = Self {
            lane_generation: map.required_u64(0)?,
            watchdog_us: map.required_u64(1)?,
        };
        renew.payload()?;
        Ok(renew)
    }
}

fn bad(key: u64, reason: &'static str) -> MessageError {
    invalid_value(SCHEMA, key, reason)
}
fn u(value: u64) -> Value {
    Value::Unsigned(value)
}
fn nonzero(value: u64, key: u64) -> Result<u64, MessageError> {
    if value == 0 {
        Err(bad(key, "must be nonzero"))
    } else {
        Ok(value)
    }
}
fn strict<'a>(value: &'a Value, keys: &[u64]) -> Result<StrictMap<'a>, MessageError> {
    let map = StrictMap::new(SCHEMA, value, keys)?;
    if let Value::Map(entries) = value {
        if entries.windows(2).any(|pair| pair[0].0 >= pair[1].0) {
            return Err(bad(0, "keys must be unique and ordered"));
        }
    }
    Ok(map)
}
fn unsigned(value: &Value, key: u64) -> Result<u64, MessageError> {
    value.as_u64().ok_or(bad(key, "expected unsigned integer"))
}
fn small(value: &Value, key: u64) -> Result<u32, MessageError> {
    u32::try_from(unsigned(value, key)?).map_err(|_| bad(key, "integer exceeds u32"))
}
fn boolean(value: &Value, key: u64) -> Result<bool, MessageError> {
    value.as_bool().ok_or(bad(key, "expected boolean"))
}
fn array<const N: usize>(value: &Value, key: u64) -> Result<&[Value; N], MessageError> {
    value
        .as_array()
        .and_then(|a| a.try_into().ok())
        .ok_or(bad(key, "invalid array length or type"))
}
fn scalar(value: Scalar) -> Value {
    let raw = value.raw();
    if raw < 0 {
        Value::Negative(raw)
    } else {
        u(raw as u64)
    }
}
fn decode_scalar(value: &Value, key: u64) -> Result<Scalar, MessageError> {
    Scalar::from_raw(
        value
            .as_i64()
            .ok_or(bad(key, "expected signed Q32.32 integer"))?,
    )
    .map_err(|e| bad(key, e.0))
}
fn point(value: Point) -> Value {
    Value::Array(vec![scalar(value.x), scalar(value.y)])
}
fn decode_point(value: &Value, key: u64) -> Result<Point, MessageError> {
    let a = array::<2>(value, key)?;
    Ok(Point {
        x: decode_scalar(&a[0], key)?,
        y: decode_scalar(&a[1], key)?,
    })
}
fn rectangle(value: Rect) -> Value {
    Value::Array(vec![
        scalar(value.origin.x),
        scalar(value.origin.y),
        scalar(value.width),
        scalar(value.height),
    ])
}
fn decode_rectangle(value: &Value, key: u64) -> Result<Rect, MessageError> {
    let a = array::<4>(value, key)?;
    let rect = Rect {
        origin: Point {
            x: decode_scalar(&a[0], key)?,
            y: decode_scalar(&a[1], key)?,
        },
        width: decode_scalar(&a[2], key)?,
        height: decode_scalar(&a[3], key)?,
    };
    rect.validate().map_err(|e| bad(key, e.0))?;
    Ok(rect)
}
fn text(value: &Value) -> Result<&str, MessageError> {
    let text = value.as_text().ok_or(bad(5, "expected text"))?;
    if text.len() > super::MAX_EVENT_TEXT_BYTES {
        return Err(bad(5, "input text exceeds limit"));
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{cbor, identity::PresenterInstanceId};

    fn owner(id: u64) -> SessionIdentity {
        SessionIdentity::new(PresenterInstanceId([7; 16]), id).unwrap()
    }
    fn address() -> WindowAddress {
        WindowAddress {
            context_id: 1,
            surface_id: 2,
            generation: u64::MAX,
        }
    }
    fn request() -> SetWindow {
        SetWindow {
            address: address(),
            expected_revision: 0,
            options: WindowOptions::new(
                Rect::new(-5.25, 4.5, 200., 100.).unwrap(),
                WindowMode::Floating,
            ),
        }
    }
    fn encoded(fields: PayloadMap) -> Value {
        let bytes = cbor::encode(&Value::Map(fields)).unwrap();
        cbor::decode(&bytes).unwrap()
    }

    #[test]
    fn window_roundtrip_preserves_full_width_fields_and_authenticated_parent_owner() {
        let mut set = request();
        set.expected_revision = u64::MAX;
        set.options.parent = Some(owner(1).context(3).unwrap().surface(4).unwrap());
        let value = encoded(set.payload(owner(1)).unwrap());
        assert_eq!(SetWindow::decode(owner(1), 2, &value).unwrap(), set);
        let other = SetWindow::decode(owner(2), 2, &value).unwrap();
        assert_eq!(other.options.parent.unwrap().context.session, owner(2));
        assert!(set.payload(owner(2)).is_err());
        assert!(SetWindow::decode(owner(1), 3, &value).is_err());
        set.options.parent = Some(address().identity(owner(1)).unwrap());
        assert!(set.payload(owner(1)).is_err());
    }

    #[test]
    fn controls_reject_invalid_geometry_unknown_and_duplicate_fields() {
        let original = request().payload(owner(1)).unwrap();
        let mut duplicate = original.clone();
        duplicate.insert(1, duplicate[0].clone());
        assert!(SetWindow::decode(owner(1), 2, &Value::Map(duplicate)).is_err());
        let mut unknown = original.clone();
        unknown.push((90, Value::Null));
        assert!(SetWindow::decode(owner(1), 2, &Value::Map(unknown)).is_err());
        let mut bad_size = original;
        bad_size.iter_mut().find(|(k, _)| *k == 4).unwrap().1 =
            Value::Array(vec![u(0), u(0), u(0), u(1)]);
        assert!(SetWindow::decode(owner(1), 2, &Value::Map(bad_size)).is_err());
    }

    #[test]
    fn status_actions_capture_and_renew_roundtrip() {
        let mut window = request();
        window.expected_revision = 19;
        let status = Status {
            window,
            viewport: Viewport {
                width: Scalar::new(1920.).unwrap(),
                height: Scalar::new(1080.).unwrap(),
                scale_numerator: 3,
                scale_denominator: 2,
            },
            focused: true,
            scene_revision: u64::MAX,
        };
        assert_eq!(
            Status::decode(owner(1), 2, &encoded(status.payload(owner(1)).unwrap())).unwrap(),
            status
        );
        for action in [
            WindowAction::Close,
            WindowAction::Focus,
            WindowAction::Raise,
            WindowAction::Lower,
            WindowAction::Center,
        ] {
            let action = Action {
                address: address(),
                expected_revision: u64::MAX,
                action,
            };
            assert_eq!(
                Action::decode(2, &encoded(action.payload().unwrap())).unwrap(),
                action
            );
        }
        let capture = Capture {
            address: address(),
            scene_revision: u64::MAX,
            capture: true,
        };
        assert_eq!(
            Capture::decode(2, &encoded(capture.payload().unwrap())).unwrap(),
            capture
        );
        let renew = Renew {
            lane_generation: u64::MAX,
            watchdog_us: 1_000_000,
        };
        assert_eq!(
            Renew::decode(0, &encoded(renew.payload().unwrap())).unwrap(),
            renew
        );
        assert!(
            Renew {
                watchdog_us: u64::MAX,
                ..renew
            }
            .payload()
            .is_err()
        );
        assert!(Renew::decode(2, &encoded(renew.payload().unwrap())).is_err());
    }

    #[test]
    fn every_input_event_roundtrips_with_exact_scene_revision() {
        let position = Point::new(12.5, -2.25).unwrap();
        let events = [
            Event::Focus(true),
            Event::Pointer {
                position,
                region: u64::MAX,
                button: Some((u16::MAX, true)),
                modifiers: u32::MAX,
            },
            Event::Pointer {
                position,
                region: 0,
                button: None,
                modifiers: 0,
            },
            Event::Wheel {
                position,
                dx: Scalar::new(-2.).unwrap(),
                dy: Scalar::ONE,
                modifiers: 2,
            },
            Event::Key {
                physical: u32::MAX,
                down: true,
                repeat: true,
                modifiers: 3,
            },
            Event::Text("é🦀".into()),
            Event::Ime {
                preedit: "é🦀".into(),
                selection: Some((2, 6)),
            },
            Event::Ime {
                preedit: String::new(),
                selection: None,
            },
            Event::Geometry {
                bounds: request().options.bounds,
                settled: true,
            },
            Event::Dismissed(DismissReason::Escape),
            Event::Dismissed(DismissReason::OutsidePress),
            Event::Dismissed(DismissReason::Closed),
            Event::Dismissed(DismissReason::OwnerLost),
            Event::Dismissed(DismissReason::ParentClosed),
            Event::Cancel,
        ];
        for event in events {
            let input = InputEvent {
                address: address(),
                scene_revision: u64::MAX,
                event,
            };
            assert_eq!(
                InputEvent::decode(2, &encoded(input.payload().unwrap())).unwrap(),
                input
            );
        }
    }

    #[test]
    fn malformed_input_fails_before_owned_text_is_created() {
        let prefix = |kind, payload| {
            Value::Map(vec![
                (0, u(1)),
                (1, u(2)),
                (2, u(1)),
                (3, u(1)),
                (4, u(kind)),
                (5, payload),
            ])
        };
        assert!(
            InputEvent::decode(
                2,
                &prefix(
                    4,
                    Value::Text("x".repeat(super::super::MAX_EVENT_TEXT_BYTES + 1))
                )
            )
            .is_err()
        );
        for (start, end) in [(1, 2), (0, 1), (2, 0), (0, u64::MAX)] {
            let value = prefix(
                5,
                Value::Array(vec![
                    Value::Text("é".into()),
                    Value::Array(vec![u(start), u(end)]),
                ]),
            );
            assert!(InputEvent::decode(2, &value).is_err());
        }
        assert!(
            InputEvent::decode(
                2,
                &prefix(
                    3,
                    Value::Array(vec![
                        u(u64::MAX),
                        Value::Bool(true),
                        Value::Bool(false),
                        u(0)
                    ])
                )
            )
            .is_err()
        );
        assert!(InputEvent::decode(2, &prefix(8, Value::Bool(false))).is_err());
        assert!(InputEvent::decode(2, &prefix(90, Value::Null)).is_err());
    }
}
