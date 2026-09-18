//! Typed overlay control and interactive-lane payloads.
//!
//! The authenticated connection supplies the owner; a payload cannot nominate another session.
//! These codecs validate wire shape and local bounds. The presenter must additionally validate
//! authority, current generations/revisions, and window eligibility before applying a request.

use crate::cbor::Value;
use crate::identity::{SessionIdentity, SurfaceIdentity};
use crate::messages::{MessageError, PayloadMap, StrictMap, invalid_value, validate_header_object};
use crate::vector::{Point, Rect, Scalar};

use super::{
    AccessibleAction, DismissReason, Event, ScrollPhase, SemanticNode, SemanticRole, Semantics,
    Toggled, WindowMode, WindowOptions, valid_event,
};

const SCHEMA: &str = "overlay";

#[path = "text.rs"]
pub mod text;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
    pub active: Option<Submission>,
    pub viewport_revision: u64,
    pub accepted_revision: u64,
}
impl Status {
    fn validate_progress(&self) -> Result<(), MessageError> {
        if self.scene_revision > self.accepted_revision
            || self
                .active
                .is_some_and(|s| s.revision > self.accepted_revision)
        {
            return Err(bad(
                15,
                "accepted revision precedes active or presented content",
            ));
        }
        Ok(())
    }
    pub fn payload(&self, owner: SessionIdentity) -> Result<PayloadMap, MessageError> {
        self.validate_progress()?;
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
        if let Some(active) = self.active {
            active.validate()?;
            if active.address != self.window.address {
                return Err(bad(13, "active binding belongs to another window"));
            }
            fields.push((
                13,
                Value::Array(vec![
                    u(active.track_id),
                    u(active.channel_generation),
                    u(u64::from(active.epoch)),
                    u(active.revision),
                ]),
            ));
        }
        fields.extend([
            (14, u(nonzero(self.viewport_revision, 14)?)),
            (15, u(self.accepted_revision)),
        ]);
        Ok(fields)
    }
    pub fn decode(
        owner: SessionIdentity,
        object: u64,
        value: &Value,
    ) -> Result<Self, MessageError> {
        let map = strict(
            value,
            &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
        )?;
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
        let active = map
            .optional(13)
            .map(|value| {
                let fields = array::<4>(value, 13)?;
                let submission = Submission {
                    address: window.address,
                    track_id: unsigned(&fields[0], 13)?,
                    channel_generation: unsigned(&fields[1], 13)?,
                    epoch: small(&fields[2], 13)?,
                    revision: unsigned(&fields[3], 13)?,
                };
                submission.validate()?;
                Ok::<_, MessageError>(submission)
            })
            .transpose()?;
        let status = Self {
            window,
            viewport,
            focused: map.required_bool(11)?,
            scene_revision: map.required_u64(12)?,
            active,
            viewport_revision: nonzero(map.required_u64(14)?, 14)?,
            accepted_revision: map.required_u64(15)?,
        };
        status.validate_progress()?;
        Ok(status)
    }
}

/// Full authenticated-channel submission identity; owner is supplied by the connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Submission {
    pub address: WindowAddress,
    pub track_id: u64,
    pub channel_generation: u64,
    pub epoch: u32,
    pub revision: u64,
}
impl Submission {
    pub fn validate(self) -> Result<(), MessageError> {
        self.address.validate(self.address.surface_id)?;
        nonzero(self.track_id, 3)?;
        nonzero(self.channel_generation, 4)?;
        nonzero(u64::from(self.epoch), 5)?;
        nonzero(self.revision, 6)?;
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresentationOutcome {
    Presented,
    Superseded,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubmissionOutcome {
    pub submission: Submission,
    pub outcome: PresentationOutcome,
}
impl SubmissionOutcome {
    pub fn payload(self) -> Result<PayloadMap, MessageError> {
        let s = self.submission;
        s.validate()?;
        let mut values = s.address.payload();
        values.extend([
            (3, u(s.track_id)),
            (4, u(s.channel_generation)),
            (5, u(u64::from(s.epoch))),
            (6, u(s.revision)),
            (
                7,
                u(match self.outcome {
                    PresentationOutcome::Presented => 0,
                    PresentationOutcome::Superseded => 1,
                }),
            ),
        ]);
        Ok(values)
    }
    pub fn decode(object: u64, value: &Value) -> Result<Self, MessageError> {
        let map = strict(value, &[0, 1, 2, 3, 4, 5, 6, 7])?;
        let submission = Submission {
            address: WindowAddress::decode(object, &map)?,
            track_id: map.required_u64(3)?,
            channel_generation: map.required_u64(4)?,
            epoch: small(map.required(5)?, 5)?,
            revision: map.required_u64(6)?,
        };
        submission.validate()?;
        Ok(Self {
            submission,
            outcome: match map.required_u64(7)? {
                0 => PresentationOutcome::Presented,
                1 => PresentationOutcome::Superseded,
                _ => return Err(bad(7, "unknown submission outcome")),
            },
        })
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewportChanged {
    pub revision: u64,
    pub viewport: Viewport,
}
impl ViewportChanged {
    pub fn payload(self) -> Result<PayloadMap, MessageError> {
        self.viewport.validate()?;
        nonzero(self.revision, 0)?;
        Ok(vec![
            (0, u(self.revision)),
            (
                1,
                Value::Array(vec![
                    scalar(self.viewport.width),
                    scalar(self.viewport.height),
                ]),
            ),
            (
                2,
                Value::Array(vec![
                    u(u64::from(self.viewport.scale_numerator)),
                    u(u64::from(self.viewport.scale_denominator)),
                ]),
            ),
        ])
    }
    pub fn decode(object: u64, value: &Value) -> Result<Self, MessageError> {
        validate_header_object(object, 0)?;
        let map = strict(value, &[0, 1, 2])?;
        let extent = array::<2>(map.required(1)?, 1)?;
        let scale = array::<2>(map.required(2)?, 2)?;
        let viewport = Viewport {
            width: decode_scalar(&extent[0], 1)?,
            height: decode_scalar(&extent[1], 1)?,
            scale_numerator: small(&scale[0], 2)?,
            scale_denominator: small(&scale[1], 2)?,
        };
        viewport.validate()?;
        Ok(Self {
            revision: nonzero(map.required_u64(0)?, 0)?,
            viewport,
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
                clicks,
                pressure,
            } => {
                let mut payload = vec![
                    point(*position),
                    u(*region),
                    button.map_or(Value::Null, |(b, down)| {
                        Value::Array(vec![u(u64::from(b)), Value::Bool(down)])
                    }),
                    u(u64::from(*modifiers)),
                ];
                // A plain motion or release stays four elements, so a host that has not adopted
                // the profile and a producer that has still agree on the common case.
                if *clicks != 0 || pressure.is_some() {
                    payload.push(u(u64::from(*clicks)));
                    payload.push(pressure.map_or(Value::Null, scalar));
                }
                (1, Value::Array(payload))
            }
            Event::Wheel {
                position,
                dx,
                dy,
                modifiers,
                precise,
                phase,
            } => (
                2,
                Value::Array(vec![
                    point(*position),
                    scalar(*dx),
                    scalar(*dy),
                    u(u64::from(*modifiers)),
                    Value::Bool(*precise),
                    u(match phase {
                        ScrollPhase::None => 0,
                        ScrollPhase::Began => 1,
                        ScrollPhase::Changed => 2,
                        ScrollPhase::Ended => 3,
                        ScrollPhase::Cancelled => 4,
                    }),
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
            Event::Accessibility { node, action } => {
                (10, Value::Array(vec![u(*node), u(action.index())]))
            }
            Event::Hover { region, entered } => {
                (9, Value::Array(vec![u(*region), Value::Bool(*entered)]))
            }
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
                // Four elements is plain motion or a release; six adds the click count and
                // pressure. Five is deliberately not a valid arity.
                let a: &[Value] = match payload.as_array() {
                    Some(a @ [_, _, _, _]) | Some(a @ [_, _, _, _, _, _]) => a,
                    _ => return Err(bad(5, "pointer payload has an unexpected arity")),
                };
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
                let (clicks, pressure) = match a {
                    [_, _, _, _, clicks, pressure] => (
                        u8::try_from(unsigned(clicks, 5)?)
                            .map_err(|_| bad(5, "click count exceeds u8"))?,
                        if *pressure == Value::Null {
                            None
                        } else {
                            Some(decode_scalar(pressure, 5)?)
                        },
                    ),
                    _ => (0, None),
                };
                Event::Pointer {
                    position: decode_point(&a[0], 5)?,
                    region: unsigned(&a[1], 5)?,
                    button,
                    modifiers: small(&a[3], 5)?,
                    clicks,
                    pressure,
                }
            }
            2 => {
                let a = array::<6>(payload, 5)?;
                Event::Wheel {
                    position: decode_point(&a[0], 5)?,
                    dx: decode_scalar(&a[1], 5)?,
                    dy: decode_scalar(&a[2], 5)?,
                    modifiers: small(&a[3], 5)?,
                    precise: boolean(&a[4], 5)?,
                    phase: match unsigned(&a[5], 5)? {
                        0 => ScrollPhase::None,
                        1 => ScrollPhase::Began,
                        2 => ScrollPhase::Changed,
                        3 => ScrollPhase::Ended,
                        4 => ScrollPhase::Cancelled,
                        _ => return Err(bad(5, "unknown scroll phase")),
                    },
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
            9 => {
                let a = array::<2>(payload, 5)?;
                Event::Hover {
                    region: unsigned(&a[0], 5)?,
                    entered: boolean(&a[1], 5)?,
                }
            }
            10 => {
                let a = array::<2>(payload, 5)?;
                let node = unsigned(&a[0], 5)?;
                if node == 0 {
                    return Err(bad(5, "an accessibility action requires a nonzero node"));
                }
                Event::Accessibility {
                    node,
                    action: AccessibleAction::from_index(unsigned(&a[1], 5)?)
                        .ok_or(bad(5, "unknown accessible action"))?,
                }
            }
            _ => return Err(bad(4, "unknown input event or invalid payload")),
        };
        // A receiver must not dispatch a reserved modifier bit, an unbounded button, or a key
        // outside the HID keyboard page, whatever a presenter claims.
        if !valid_event(&event) {
            return Err(bad(5, "invalid or oversized input payload"));
        }
        Ok(Self {
            address,
            scene_revision: map.required_u64(3)?,
            event,
        })
    }
}

/// A window's complete semantic tree for one published scene (`overlay-a11y-v1`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetSemantics {
    pub address: WindowAddress,
    pub semantics: Semantics,
}
impl SetSemantics {
    pub fn payload(&self) -> Result<PayloadMap, MessageError> {
        self.address.validate(self.address.surface_id)?;
        self.semantics
            .validate()
            .map_err(|_| bad(4, "invalid semantic tree"))?;
        let nodes = self
            .semantics
            .nodes
            .iter()
            .map(node_value)
            .collect::<Result<Vec<_>, MessageError>>()?;
        let mut fields = self.address.payload();
        fields.push((3, u(self.semantics.scene_revision)));
        fields.push((4, Value::Array(nodes)));
        Ok(fields)
    }
    pub fn decode(object: u64, value: &Value) -> Result<Self, MessageError> {
        let map = strict(value, &[0, 1, 2, 3, 4])?;
        let address = WindowAddress::decode(object, &map)?;
        let list = array_as_slice(map.required(4)?, 4)?;
        if list.is_empty() || list.len() > super::MAX_SEMANTIC_NODES {
            return Err(bad(4, "invalid semantic tree size"));
        }
        let mut nodes = Vec::with_capacity(list.len());
        for node in list {
            nodes.push(parse_node(node)?);
        }
        let semantics = Semantics {
            scene_revision: map.required_u64(3)?,
            nodes,
        };
        semantics
            .validate()
            .map_err(|_| bad(4, "invalid semantic tree"))?;
        Ok(Self { address, semantics })
    }
}

fn node_value(node: &SemanticNode) -> Result<Value, MessageError> {
    let mut fields = vec![
        (0, u(node.id)),
        (1, u(node.role.index())),
        (2, rectangle(node.bounds)),
    ];
    if !node.label.is_empty() {
        fields.push((3, Value::Text(node.label.clone())));
    }
    if let Some([value, minimum, maximum]) = node.numeric {
        fields.push((
            4,
            Value::Array(vec![scalar(value), scalar(minimum), scalar(maximum)]),
        ));
    }
    if let Some(level) = node.level {
        fields.push((5, u(u64::from(level))));
    }
    if let Some([position, size]) = node.set {
        fields.push((
            6,
            Value::Array(vec![u(u64::from(position)), u(u64::from(size))]),
        ));
    }
    if let Some(toggled) = node.toggled {
        fields.push((
            7,
            u(match toggled {
                Toggled::Off => 0,
                Toggled::On => 1,
                Toggled::Mixed => 2,
            }),
        ));
    }
    if node.disabled {
        fields.push((8, Value::Bool(true)));
    }
    if !node.actions.is_empty() {
        fields.push((
            9,
            Value::Array(node.actions.iter().map(|a| u(a.index())).collect()),
        ));
    }
    if !node.children.is_empty() {
        fields.push((
            10,
            Value::Array(node.children.iter().map(|c| u(u64::from(*c))).collect()),
        ));
    }
    Ok(Value::Map(fields))
}

fn parse_node(value: &Value) -> Result<SemanticNode, MessageError> {
    let map = strict(value, &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10])?;
    let id = map.required_u64(0)?;
    if id == 0 {
        return Err(bad(0, "semantic node ID must be nonzero"));
    }
    let role =
        SemanticRole::from_index(map.required_u64(1)?).ok_or(bad(1, "unknown semantic role"))?;
    let bounds = decode_rectangle(map.required(2)?, 2)?;
    let label = match map.optional(3) {
        None => String::new(),
        Some(Value::Text(text)) => text.clone(),
        Some(_) => return Err(bad(3, "semantic label must be text")),
    };
    let numeric = match map.optional(4) {
        None => None,
        Some(value) => {
            let a = array::<3>(value, 4)?;
            Some([
                decode_scalar(&a[0], 4)?,
                decode_scalar(&a[1], 4)?,
                decode_scalar(&a[2], 4)?,
            ])
        }
    };
    let level = match map.optional(5) {
        None => None,
        Some(value) => Some(
            u8::try_from(unsigned(value, 5)?).map_err(|_| bad(5, "heading level exceeds u8"))?,
        ),
    };
    let set = match map.optional(6) {
        None => None,
        Some(value) => {
            let a = array::<2>(value, 6)?;
            Some([
                u16::try_from(unsigned(&a[0], 6)?)
                    .map_err(|_| bad(6, "set position exceeds u16"))?,
                u16::try_from(unsigned(&a[1], 6)?).map_err(|_| bad(6, "set size exceeds u16"))?,
            ])
        }
    };
    let toggled = match map.optional(7) {
        None => None,
        Some(value) => Some(match unsigned(value, 7)? {
            0 => Toggled::Off,
            1 => Toggled::On,
            2 => Toggled::Mixed,
            _ => return Err(bad(7, "unknown toggled state")),
        }),
    };
    let disabled = match map.optional(8) {
        None => false,
        Some(value) => boolean(value, 8)?,
    };
    let actions = match map.optional(9) {
        None => Vec::new(),
        Some(value) => {
            let list = array_as_slice(value, 9)?;
            if list.len() > super::MAX_SEMANTIC_ACTIONS {
                return Err(bad(9, "semantic node lists too many actions"));
            }
            list.iter()
                .map(|value| {
                    AccessibleAction::from_index(unsigned(value, 9)?)
                        .ok_or(bad(9, "unknown accessible action"))
                })
                .collect::<Result<Vec<_>, _>>()?
        }
    };
    let children = match map.optional(10) {
        None => Vec::new(),
        Some(value) => {
            let list = array_as_slice(value, 10)?;
            list.iter()
                .map(|value| {
                    u32::try_from(unsigned(value, 10)?)
                        .map_err(|_| bad(10, "semantic child index exceeds u32"))
                })
                .collect::<Result<Vec<_>, _>>()?
        }
    };
    Ok(SemanticNode {
        id,
        role,
        bounds,
        label,
        numeric,
        level,
        set,
        toggled,
        disabled,
        actions,
        children,
    })
}

/// A request to place text on the user's clipboard (`overlay-clipboard-v1`).
///
/// Write-only by construction: there is no record that reads the clipboard back, so an overlay
/// can never observe what the user copied elsewhere. Paste still arrives as ordinary committed
/// text through the host's own paste policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Clipboard {
    pub address: WindowAddress,
    pub text: String,
}
impl Clipboard {
    pub fn payload(&self) -> Result<PayloadMap, MessageError> {
        self.address.validate(self.address.surface_id)?;
        if self.text.is_empty() || self.text.len() > super::MAX_CLIPBOARD_BYTES {
            return Err(bad(3, "clipboard text is empty or oversized"));
        }
        let mut fields = self.address.payload();
        fields.push((3, Value::Text(self.text.clone())));
        Ok(fields)
    }
    pub fn decode(object: u64, value: &Value) -> Result<Self, MessageError> {
        let map = strict(value, &[0, 1, 2, 3])?;
        let address = WindowAddress::decode(object, &map)?;
        let text = match map.required(3)? {
            Value::Text(text) => text.clone(),
            _ => return Err(bad(3, "clipboard text must be text")),
        };
        if text.is_empty() || text.len() > super::MAX_CLIPBOARD_BYTES {
            return Err(bad(3, "clipboard text is empty or oversized"));
        }
        Ok(Self { address, text })
    }
}

/// Who the host is: the defaults a toolkit needs so an overlay looks like the pane it sits in.
///
/// Delivered separately from the viewport because appearance and motion preference change
/// independently of geometry, and a producer that caches layout per viewport revision must not
/// relayout because the user switched their desktop theme.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Environment {
    /// The font a plain `Text` or an empty `TextStyle` family resolves to. Empty asks the host.
    pub font_family: String,
    /// The default size for that family, in logical pixels.
    pub font_size: Scalar,
    pub appearance: Appearance,
    /// Whether the user asked for reduced motion, or `None` when the host cannot tell. A host
    /// that has no such signal MUST report absence rather than assert a preference.
    pub reduced_motion: Option<bool>,
    /// How often the display refreshes, or `None` when the host cannot tell. A producer must
    /// still respect its own track's record ceiling, which may be lower.
    pub refresh_interval_us: Option<u64>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Appearance {
    #[default]
    Light,
    Dark,
}

impl Default for Environment {
    /// A valid environment with no host-specific detail: an empty family asks the host to choose,
    /// and the size is the one a plain `TextStyle` already defaults to. A default that failed
    /// validation would let a host revoke a lane before it ever learned its own font.
    fn default() -> Self {
        Self {
            font_family: String::new(),
            font_size: Scalar::new(16.).expect("16 logical pixels is a valid scalar"),
            appearance: Appearance::Light,
            reduced_motion: None,
            refresh_interval_us: None,
        }
    }
}

impl Environment {
    pub fn validate(&self) -> Result<(), MessageError> {
        if self.font_family.len() > text::styled::MAX_FAMILY_BYTES {
            return Err(bad(1, "environment font family exceeds its ceiling"));
        }
        if self.font_size <= Scalar::ZERO {
            return Err(bad(2, "environment font size must be positive"));
        }
        if self.refresh_interval_us == Some(0) {
            return Err(bad(5, "environment refresh interval must be nonzero"));
        }
        Ok(())
    }
}

/// `OVERLAY_ENV_CHANGED` (0x7036): an unsolicited interactive-lane envelope with request ID zero.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvironmentChanged {
    pub revision: u64,
    pub environment: Environment,
}
impl EnvironmentChanged {
    pub fn payload(self) -> Result<PayloadMap, MessageError> {
        self.environment.validate()?;
        nonzero(self.revision, 0)?;
        let env = &self.environment;
        Ok(vec![
            (0, u(self.revision)),
            (1, Value::Text(env.font_family.clone())),
            (2, scalar(env.font_size)),
            (
                3,
                u(match env.appearance {
                    Appearance::Light => 0,
                    Appearance::Dark => 1,
                }),
            ),
            (4, env.reduced_motion.map_or(Value::Null, Value::Bool)),
            (5, env.refresh_interval_us.map_or(Value::Null, u)),
        ])
    }
    pub fn decode(object: u64, value: &Value) -> Result<Self, MessageError> {
        validate_header_object(object, 0)?;
        let map = strict(value, &[0, 1, 2, 3, 4, 5])?;
        let font_family = match map.required(1)? {
            Value::Text(text) => text.clone(),
            _ => return Err(bad(1, "environment font family must be text")),
        };
        let appearance = match unsigned(map.required(3)?, 3)? {
            0 => Appearance::Light,
            1 => Appearance::Dark,
            _ => return Err(bad(3, "unknown appearance")),
        };
        let reduced_motion = match map.required(4)? {
            Value::Null => None,
            Value::Bool(value) => Some(*value),
            _ => return Err(bad(4, "reduced motion must be a boolean or null")),
        };
        let refresh_interval_us = match map.required(5)? {
            Value::Null => None,
            value => Some(unsigned(value, 5)?),
        };
        let environment = Environment {
            font_family,
            font_size: decode_scalar(map.required(2)?, 2)?,
            appearance,
            reduced_motion,
            refresh_interval_us,
        };
        environment.validate()?;
        Ok(Self {
            revision: map.required_u64(0)?,
            environment,
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
/// A variable-length array, for the two lists a node may carry.
fn array_as_slice(value: &Value, key: u64) -> Result<&[Value], MessageError> {
    value.as_array().ok_or(bad(key, "invalid array type"))
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
    use crate::overlay::{MAX_CLICKS, buttons, keys, modifiers};
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
            active: Some(Submission {
                address: address(),
                track_id: u64::MAX,
                channel_generation: u64::MAX,
                epoch: u32::MAX,
                revision: u64::MAX,
            }),
            viewport_revision: 1,
            accepted_revision: u64::MAX,
        };
        assert_eq!(
            Status::decode(owner(1), 2, &encoded(status.payload(owner(1)).unwrap())).unwrap(),
            status
        );
        let submission = status.active.unwrap();
        for outcome in [
            PresentationOutcome::Presented,
            PresentationOutcome::Superseded,
        ] {
            let record = SubmissionOutcome {
                submission,
                outcome,
            };
            let value = encoded(record.payload().unwrap());
            assert_eq!(SubmissionOutcome::decode(2, &value).unwrap(), record);
            assert!(SubmissionOutcome::decode(3, &value).is_err());
            let mut malformed = record.payload().unwrap();
            malformed.iter_mut().find(|(k, _)| *k == 7).unwrap().1 = u(2);
            assert!(SubmissionOutcome::decode(2, &Value::Map(malformed)).is_err());
        }
        let update = ViewportChanged {
            revision: u64::MAX,
            viewport: status.viewport,
        };
        assert_eq!(
            ViewportChanged::decode(0, &encoded(update.payload().unwrap())).unwrap(),
            update
        );
        assert!(ViewportChanged::decode(2, &encoded(update.payload().unwrap())).is_err());
        assert!(
            ViewportChanged {
                revision: 0,
                ..update
            }
            .payload()
            .is_err()
        );
        let release = crate::vector::AssetRelease { id: u64::MAX };
        assert_eq!(
            crate::vector::AssetRelease::decode(&release.encode().unwrap()).unwrap(),
            release
        );
        assert!(crate::vector::AssetRelease::decode(&[0; 8]).is_err());
        assert!(crate::vector::AssetRelease::decode(&[1; 9]).is_err());
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
    fn a_default_environment_is_one_a_host_could_already_publish() {
        // A host starts with defaults and learns its own font later; a default that failed
        // validation would revoke the lane before that happened.
        assert!(Environment::default().validate().is_ok());
    }

    #[test]
    fn semantic_trees_round_trip_with_their_optional_detail() {
        use crate::overlay::{AccessibleAction, SemanticNode, SemanticRole, Semantics, Toggled};
        let full = SetSemantics {
            address: address(),
            semantics: Semantics {
                scene_revision: 9,
                nodes: vec![
                    SemanticNode {
                        id: 1,
                        role: SemanticRole::Application,
                        bounds: Rect::new(0., 0., 200., 100.).unwrap(),
                        label: "Panel".to_owned(),
                        numeric: None,
                        level: None,
                        set: None,
                        toggled: None,
                        disabled: false,
                        actions: Vec::new(),
                        children: vec![1],
                    },
                    SemanticNode {
                        id: 7,
                        role: SemanticRole::SpinButton,
                        bounds: Rect::new(10., 10., 60., 20.).unwrap(),
                        label: "Count".to_owned(),
                        numeric: Some([
                            Scalar::new(5.).unwrap(),
                            Scalar::new(0.).unwrap(),
                            Scalar::new(10.).unwrap(),
                        ]),
                        level: None,
                        set: None,
                        toggled: None,
                        disabled: true,
                        actions: vec![AccessibleAction::Increment, AccessibleAction::Decrement],
                        children: Vec::new(),
                    },
                ],
            },
        };
        let bytes = encoded(full.clone().payload().unwrap());
        assert_eq!(SetSemantics::decode(2, &bytes).unwrap(), full);

        // A node that carries only what it needs stays that way: absent keys are absent.
        let minimal = SetSemantics {
            address: address(),
            semantics: Semantics {
                scene_revision: 1,
                nodes: vec![SemanticNode {
                    id: 1,
                    role: SemanticRole::Group,
                    bounds: Rect::new(0., 0., 1., 1.).unwrap(),
                    label: String::new(),
                    numeric: None,
                    level: None,
                    set: None,
                    toggled: None,
                    disabled: false,
                    actions: Vec::new(),
                    children: Vec::new(),
                }],
            },
        };
        let bytes = encoded(minimal.clone().payload().unwrap());
        let decoded = SetSemantics::decode(2, &bytes).unwrap();
        assert_eq!(decoded, minimal);
        assert!(decoded.semantics.nodes[0].toggled.is_none());
        assert!(decoded.semantics.nodes[0].set.is_none());

        // A toggled state survives, since a switch that reads as mixed is not merely off.
        let mut toggled = minimal.clone();
        toggled.semantics.nodes[0].toggled = Some(Toggled::Mixed);
        assert_eq!(
            SetSemantics::decode(2, &encoded(toggled.clone().payload().unwrap())).unwrap(),
            toggled
        );

        // The struct encoding is not a place to smuggle a zero identity.
        let mut zero = minimal;
        zero.semantics.nodes[0].id = 0;
        assert!(zero.payload().is_err());
    }

    #[test]
    fn an_accessibility_action_round_trips_and_refuses_a_zero_node() {
        let event = Event::Accessibility {
            node: u64::MAX,
            action: crate::overlay::AccessibleAction::Increment,
        };
        assert_eq!(
            InputEvent::decode(
                2,
                &encoded(
                    InputEvent {
                        address: address(),
                        scene_revision: 3,
                        event: event.clone(),
                    }
                    .payload()
                    .unwrap()
                )
            )
            .unwrap()
            .event,
            event
        );

        // A zero node is not an identity, so an action naming one cannot be dispatched.
        let zero = InputEvent {
            address: address(),
            scene_revision: 3,
            event: Event::Accessibility {
                node: 0,
                action: crate::overlay::AccessibleAction::Click,
            },
        };
        assert!(zero.payload().is_err());
    }

    #[test]
    fn environment_snapshots_round_trip_with_absent_preferences_intact() {
        let full = EnvironmentChanged {
            revision: 7,
            environment: Environment {
                font_family: "Iosevka Term".to_owned(),
                font_size: Scalar::new(13.5).unwrap(),
                appearance: Appearance::Dark,
                reduced_motion: Some(true),
                refresh_interval_us: Some(16_667),
            },
        };
        assert_eq!(
            EnvironmentChanged::decode(0, &encoded(full.clone().payload().unwrap())).unwrap(),
            full
        );

        // An unreadable preference stays absent across the wire rather than becoming "false".
        let unknown = EnvironmentChanged {
            revision: 1,
            environment: Environment {
                font_family: String::new(),
                font_size: Scalar::new(16.).unwrap(),
                appearance: Appearance::Light,
                reduced_motion: None,
                refresh_interval_us: None,
            },
        };
        let decoded =
            EnvironmentChanged::decode(0, &encoded(unknown.clone().payload().unwrap())).unwrap();
        assert_eq!(decoded.environment.reduced_motion, None);
        assert_eq!(decoded.environment.refresh_interval_us, None);
        assert_eq!(decoded, unknown);
    }

    #[test]
    fn environment_values_outside_their_bounds_are_refused() {
        let base = |environment: Environment| EnvironmentChanged {
            revision: 1,
            environment,
        };
        for environment in [
            Environment {
                font_family: "x".repeat(text::styled::MAX_FAMILY_BYTES + 1),
                ..sample_environment()
            },
            Environment {
                font_size: Scalar::ZERO,
                ..sample_environment()
            },
            Environment {
                font_size: Scalar::new(-1.).unwrap(),
                ..sample_environment()
            },
            Environment {
                refresh_interval_us: Some(0),
                ..sample_environment()
            },
        ] {
            assert!(base(environment).payload().is_err());
        }
        // A first-rate interval for a 60 Hz display is fine.
        assert!(base(sample_environment()).payload().is_ok());
    }

    fn sample_environment() -> Environment {
        Environment {
            font_family: String::new(),
            font_size: Scalar::new(14.).unwrap(),
            appearance: Appearance::Light,
            reduced_motion: None,
            refresh_interval_us: Some(16_667),
        }
    }

    #[test]
    fn an_unknown_appearance_is_refused_rather_than_guessed() {
        let fields: Vec<(u64, Value)> = EnvironmentChanged {
            revision: 1,
            environment: sample_environment(),
        }
        .payload()
        .unwrap()
        .into_iter()
        .map(|(key, value)| (key, if key == 3 { Value::Unsigned(9) } else { value }))
        .collect();
        assert!(
            EnvironmentChanged::decode(0, &Value::Map(fields)).is_err(),
            "an appearance outside the closed set must not decode"
        );
    }

    #[test]
    fn clipboard_writes_round_trip_and_refuse_empty_or_oversized_text() {
        let request = Clipboard {
            address: address(),
            text: "copied from an overlay".to_owned(),
        };
        let encoded = encoded(request.payload().unwrap());
        assert_eq!(Clipboard::decode(2, &encoded).unwrap(), request);

        for text in [
            String::new(),
            "x".repeat(super::super::MAX_CLIPBOARD_BYTES + 1),
        ] {
            let request = Clipboard {
                address: address(),
                text,
            };
            assert!(request.payload().is_err());
        }
    }

    #[test]
    fn every_input_event_roundtrips_with_exact_scene_revision() {
        let position = Point::new(12.5, -2.25).unwrap();
        let events = [
            Event::Focus(true),
            Event::Pointer {
                position,
                region: u64::MAX,
                button: Some((buttons::MAXIMUM, true)),
                modifiers: modifiers::KNOWN_MASK,
                clicks: MAX_CLICKS,
                pressure: Some(Scalar::ONE),
            },
            // Plain motion keeps the four-element form, so a host and producer agree on the
            // common case without either of them carrying the profile's extra fields.
            Event::Pointer {
                position,
                region: 0,
                button: None,
                modifiers: 0,
                clicks: 0,
                pressure: None,
            },
            Event::Pointer {
                position,
                region: 7,
                button: Some((buttons::PRIMARY, true)),
                modifiers: 0,
                clicks: 1,
                pressure: None,
            },
            Event::Hover {
                region: 7,
                entered: true,
            },
            Event::Hover {
                region: 0,
                entered: false,
            },
            Event::Wheel {
                position,
                dx: Scalar::new(-2.).unwrap(),
                dy: Scalar::ONE,
                modifiers: 2,
                precise: true,
                phase: ScrollPhase::Began,
            },
            Event::Wheel {
                position,
                dx: Scalar::ZERO,
                dy: Scalar::ONE,
                modifiers: 0,
                precise: false,
                phase: ScrollPhase::None,
            },
            Event::Key {
                physical: keys::LAST_USAGE,
                down: true,
                repeat: true,
                modifiers: 3,
            },
            Event::Key {
                physical: keys::UNMAPPED,
                down: false,
                repeat: false,
                modifiers: 0,
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
    fn reserved_modifiers_unbounded_buttons_and_foreign_keys_are_refused() {
        // These encodings are normative. A presenter that forwards a platform bitmask, an
        // unbounded device button, or a non-HID key code must not reach a producer's dispatch.
        let refused = [
            Event::Pointer {
                position: Point::new(1., 1.).unwrap(),
                region: 0,
                button: None,
                modifiers: modifiers::KNOWN_MASK | (1 << 6),
                clicks: 0,
                pressure: None,
            },
            Event::Pointer {
                position: Point::new(1., 1.).unwrap(),
                region: 0,
                button: Some((buttons::MAXIMUM + 1, true)),
                modifiers: 0,
                clicks: 0,
                pressure: None,
            },
            Event::Pointer {
                position: Point::new(1., 1.).unwrap(),
                region: 0,
                button: None,
                modifiers: 0,
                clicks: MAX_CLICKS + 1,
                pressure: None,
            },
            Event::Pointer {
                position: Point::new(1., 1.).unwrap(),
                region: 0,
                button: None,
                modifiers: 0,
                clicks: 0,
                pressure: Some(Scalar::new(1.5).unwrap()),
            },
            Event::Wheel {
                position: Point::new(1., 1.).unwrap(),
                dx: Scalar::ZERO,
                dy: Scalar::ONE,
                modifiers: u32::MAX,
                precise: false,
                phase: ScrollPhase::None,
            },
            Event::Key {
                physical: keys::LAST_USAGE + 1,
                down: true,
                repeat: false,
                modifiers: 0,
            },
            Event::Key {
                physical: keys::FIRST_USAGE - 1,
                down: true,
                repeat: false,
                modifiers: 0,
            },
        ];
        for event in refused {
            let input = InputEvent {
                address: address(),
                scene_revision: 1,
                event,
            };
            assert!(
                input.payload().is_err(),
                "{:?} must not encode",
                input.event
            );
        }

        // A hostile presenter cannot smuggle one past the decoder either.
        let hostile = Value::Map(vec![
            (0, u(1)),
            (1, u(2)),
            (2, u(1)),
            (3, u(1)),
            (4, u(3)),
            (
                5,
                Value::Array(vec![
                    u(u64::from(keys::FIRST_USAGE)),
                    Value::Bool(true),
                    Value::Bool(false),
                    u(u64::from(modifiers::KNOWN_MASK) + 64),
                ]),
            ),
        ]);
        assert!(InputEvent::decode(2, &hostile).is_err());
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
