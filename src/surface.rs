//! Surfaces: what a producer presents and how it may change (core §8).
//!
//! A [`SurfaceDefinition`] is the `CREATE_SURFACE` schema: semantic profile, coordinate model,
//! logical size, [`SurfaceDescriptor`], capture policy, and profile parameters such as
//! [`DesktopSurfaceParameters`]. [`UpdateSurface`] decodes a change, and
//! [`SurfaceState::replace_mutable`] applies it, advancing the surface revision, and the surface
//! generation too when coordinates or input identity change.

use crate::{
    cbor::Value,
    geometry::{Rotation, SurfaceMapping},
    messages::{
        MessageError, PayloadMap, StrictMap, invalid_value, require_nonzero, validate_header_object,
    },
    registry::{CANVAS_CONTENT, DESKTOP_CONTENT, GENERIC_CONTENT, TERMINAL_CONTENT},
    revision::{SurfaceGeneration, SurfaceRevision},
    target::OutputDescriptor,
};

/// Policy bit 0: deny screenshots, canvas capture, and readback.
pub const POLICY_DENY_CAPTURE: u64 = 1 << 0;
/// Policy bit 1: deny descriptor, title, and semantic export beyond the role.
pub const POLICY_DENY_DESCRIPTOR_EXPORT: u64 = 1 << 1;
/// Policy bit 2: deny keeping a poster after disconnect.
pub const POLICY_DENY_POSTER_RETENTION: u64 = 1 << 2;
/// Policy bit 3: deny content-addressed image caching.
pub const POLICY_DENY_IMAGE_CACHE: u64 = 1 << 3;
/// Policy bit 4: reduce diagnostics to kind and lifecycle.
pub const POLICY_REDUCED_DIAGNOSTICS: u64 = 1 << 4;
/// Every assigned policy bit.
pub const POLICY_KNOWN_MASK: u64 = (1 << 5) - 1;

/// What a surface shows (descriptor key 0).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
#[non_exhaustive]
pub enum SurfaceRole {
    /// No stated role.
    Unspecified = 0,
    /// A document.
    Document = 1,
    /// A desktop.
    Desktop = 2,
    /// Timed media, such as a video.
    TimedMedia = 3,
    /// A figure or still image.
    Figure = 4,
    /// A terminal or text surface.
    TerminalText = 5,
    /// An application canvas.
    ApplicationCanvas = 6,
}

impl TryFrom<u64> for SurfaceRole {
    type Error = MessageError;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Unspecified),
            1 => Ok(Self::Document),
            2 => Ok(Self::Desktop),
            3 => Ok(Self::TimedMedia),
            4 => Ok(Self::Figure),
            5 => Ok(Self::TerminalText),
            6 => Ok(Self::ApplicationCanvas),
            _ => Err(invalid_value(
                "surface descriptor",
                0,
                "has an unknown role",
            )),
        }
    }
}

/// The units a surface's logical size and node geometry use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
#[non_exhaustive]
pub enum CoordinateModel {
    /// Desktop logical pixels.
    DesktopLogicalPixels = 1,
    /// Normalized coordinates.
    Normalized = 2,
    /// Canvas logical units.
    CanvasLogicalUnits = 3,
    /// Terminal content cells.
    TerminalContentCells = 4,
}

impl TryFrom<u64> for CoordinateModel {
    type Error = MessageError;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::DesktopLogicalPixels),
            2 => Ok(Self::Normalized),
            3 => Ok(Self::CanvasLogicalUnits),
            4 => Ok(Self::TerminalContentCells),
            _ => Err(invalid_value(
                "surface",
                3,
                "has an unknown coordinate model",
            )),
        }
    }
}

/// A surface's descriptor (core §8.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurfaceDescriptor {
    /// What the surface shows (key 0).
    pub role: SurfaceRole,
    /// Title, at most 256 UTF-8 bytes (key 1).
    pub title: String,
    /// Semantic content revision; it never decreases (key 2).
    pub semantic_content_revision: u64,
    /// Availability bits: extracted text, structure, links, outline, actions (key 3).
    pub semantic_availability: u64,
    /// Inert locator hint, at most 512 UTF-8 bytes; never opened or fetched (key 4).
    pub locator_hint: String,
}

impl SurfaceDescriptor {
    /// Encodes the descriptor as its CBOR map.
    ///
    /// # Errors
    ///
    /// Returns any error from [`SurfaceDescriptor::validate`].
    pub fn to_value(&self) -> Result<Value, MessageError> {
        self.validate()?;
        Ok(Value::Map(vec![
            (0, Value::Unsigned(self.role as u64)),
            (1, Value::Text(self.title.clone())),
            (2, Value::Unsigned(self.semantic_content_revision)),
            (3, Value::Unsigned(self.semantic_availability)),
            (4, Value::Text(self.locator_hint.clone())),
        ]))
    }

    /// Decodes a descriptor from its CBOR map.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError`] when `value` is not a strict descriptor map, a field is missing or
    /// has the wrong type, the role is unregistered, or the descriptor fails
    /// [`SurfaceDescriptor::validate`].
    pub fn from_value(value: &Value) -> Result<Self, MessageError> {
        let map = StrictMap::new("surface descriptor", value, &[0, 1, 2, 3, 4])?;
        let descriptor = Self {
            role: SurfaceRole::try_from(map.required_u64(0)?)?,
            title: map.required_text(1)?.to_owned(),
            semantic_content_revision: map.required_u64(2)?,
            semantic_availability: map.required_u64(3)?,
            locator_hint: map.required_text(4)?.to_owned(),
        };
        descriptor.validate()?;
        Ok(descriptor)
    }

    /// Checks the title and locator lengths and the availability bits.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError::InvalidValue`] when the title exceeds 256 UTF-8 bytes, the locator
    /// hint exceeds 512, or the availability mask has unknown bits.
    pub fn validate(&self) -> Result<(), MessageError> {
        if self.title.len() > 256 {
            return Err(invalid_value(
                "surface descriptor",
                1,
                "exceeds 256 UTF-8 bytes",
            ));
        }
        if self.semantic_availability & !0x1f != 0 {
            return Err(invalid_value(
                "surface descriptor",
                3,
                "has unknown availability bits",
            ));
        }
        if self.locator_hint.len() > 512 {
            return Err(invalid_value(
                "surface descriptor",
                4,
                "exceeds 512 UTF-8 bytes",
            ));
        }
        Ok(())
    }
}

/// The `CREATE_SURFACE` schema (core §8.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurfaceDefinition {
    /// Owning context (key 0).
    pub context_id: u64,
    /// Nonzero surface ID, unique within the context (key 1).
    pub surface_id: u64,
    /// Semantic profile, such as [`crate::registry::GENERIC_CONTENT`] (key 2).
    pub semantic_profile: String,
    /// Coordinate model; the semantic profile limits the choice (key 3).
    pub coordinate_model: CoordinateModel,
    /// Nonzero logical width (key 4).
    pub logical_width: u64,
    /// Nonzero logical height (key 5).
    pub logical_height: u64,
    /// Nonzero scale numerator (key 6).
    pub scale_numerator: u64,
    /// Nonzero scale denominator (key 7).
    pub scale_denominator: u64,
    /// Clockwise rotation: 0, 90, 180, or 270 (key 8).
    pub rotation: u16,
    /// Complete descriptor (key 9).
    pub descriptor: SurfaceDescriptor,
    /// Capture and export policy bits, such as [`POLICY_DENY_CAPTURE`] (key 10).
    pub policy: u64,
    /// Profile-specific parameters (key 11).
    pub profile_parameters: PayloadMap,
}

impl SurfaceDefinition {
    /// Checks the surface's IDs, geometry, rotation, policy, and profile coordinate model.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError::InvalidValue`] when the context or surface ID, logical width or
    /// height, or a scale term is zero, the rotation is not a right angle, the policy has unknown
    /// bits, or the coordinate model is illegal for the semantic profile.
    pub fn validate(&self) -> Result<(), MessageError> {
        require_nonzero("surface", 0, self.context_id)?;
        require_nonzero("surface", 1, self.surface_id)?;
        require_nonzero("surface", 4, self.logical_width)?;
        require_nonzero("surface", 5, self.logical_height)?;
        require_nonzero("surface", 6, self.scale_numerator)?;
        require_nonzero("surface", 7, self.scale_denominator)?;
        if !matches!(self.rotation, 0 | 90 | 180 | 270) {
            return Err(invalid_value(
                "surface",
                8,
                "rotation is not 0, 90, 180, or 270",
            ));
        }
        if self.policy & !POLICY_KNOWN_MASK != 0 {
            return Err(invalid_value("surface", 10, "has unknown policy bits"));
        }
        match (self.semantic_profile.as_str(), self.coordinate_model) {
            (GENERIC_CONTENT | DESKTOP_CONTENT, CoordinateModel::DesktopLogicalPixels)
            | (
                GENERIC_CONTENT | CANVAS_CONTENT,
                CoordinateModel::Normalized | CoordinateModel::CanvasLogicalUnits,
            )
            | (TERMINAL_CONTENT, CoordinateModel::TerminalContentCells) => {}
            _ => {
                return Err(invalid_value(
                    "surface",
                    3,
                    "is illegal for the semantic profile",
                ));
            }
        }
        self.descriptor.validate()
    }

    /// Encodes the `CREATE_SURFACE` payload.
    ///
    /// # Errors
    ///
    /// Returns any error from [`SurfaceDefinition::validate`] or [`SurfaceDescriptor::validate`].
    pub fn create_payload(&self) -> Result<PayloadMap, MessageError> {
        self.validate()?;
        Ok(vec![
            (0, Value::Unsigned(self.context_id)),
            (1, Value::Unsigned(self.surface_id)),
            (2, Value::Text(self.semantic_profile.clone())),
            (3, Value::Unsigned(self.coordinate_model as u64)),
            (4, Value::Unsigned(self.logical_width)),
            (5, Value::Unsigned(self.logical_height)),
            (6, Value::Unsigned(self.scale_numerator)),
            (7, Value::Unsigned(self.scale_denominator)),
            (8, Value::Unsigned(u64::from(self.rotation))),
            (9, self.descriptor.to_value()?),
            (10, Value::Unsigned(self.policy)),
            (11, Value::Map(self.profile_parameters.clone())),
        ])
    }

    /// Decodes a `CREATE_SURFACE` payload.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError`] when `payload` is not a strict `CREATE_SURFACE` map, a field is
    /// missing or has the wrong type, the surface ID differs from `header_object_id`, the
    /// coordinate model is unregistered, the rotation does not fit in `u16`, the descriptor is
    /// invalid, or the definition fails [`SurfaceDefinition::validate`].
    pub fn decode_create(header_object_id: u64, payload: &Value) -> Result<Self, MessageError> {
        let map = StrictMap::new(
            "CREATE_SURFACE",
            payload,
            &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11],
        )?;
        let surface_id = map.required_u64(1)?;
        validate_header_object(header_object_id, surface_id)?;
        let definition = Self {
            context_id: map.required_u64(0)?,
            surface_id,
            semantic_profile: map.required_text(2)?.to_owned(),
            coordinate_model: CoordinateModel::try_from(map.required_u64(3)?)?,
            logical_width: map.required_u64(4)?,
            logical_height: map.required_u64(5)?,
            scale_numerator: map.required_u64(6)?,
            scale_denominator: map.required_u64(7)?,
            rotation: u16::try_from(map.required_u64(8)?)
                .map_err(|_| invalid_value("CREATE_SURFACE", 8, "does not fit in u16"))?,
            descriptor: SurfaceDescriptor::from_value(map.required(9)?)?,
            policy: map.required_u64(10)?,
            profile_parameters: map.required_map(11)?.to_vec(),
        };
        definition.validate()?;
        Ok(definition)
    }

    fn mapping_eq(&self, other: &Self) -> bool {
        self.logical_width == other.logical_width
            && self.logical_height == other.logical_height
            && self.scale_numerator == other.scale_numerator
            && self.scale_denominator == other.scale_denominator
            && self.rotation == other.rotation
            && self.profile_parameters == other.profile_parameters
    }
}

/// A presenter's state for one surface.
#[derive(Debug, Clone)]
pub struct SurfaceState {
    /// The current definition.
    pub definition: SurfaceDefinition,
    /// Surface revision; advances on each change.
    pub revision: SurfaceRevision,
    /// Surface generation; advances when coordinates or input identity change.
    pub generation: SurfaceGeneration,
}

/// The result of applying an `UPDATE_SURFACE`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SurfaceMutation {
    /// The new surface revision.
    pub revision: SurfaceRevision,
    /// The surface generation after the update.
    pub generation: SurfaceGeneration,
    /// Whether the generation changed, so input bound to the surface must be revoked.
    pub input_must_be_revoked: bool,
}

impl SurfaceState {
    /// Starts tracking a newly created surface at revision and generation one.
    ///
    /// # Errors
    ///
    /// Returns any error from [`SurfaceDefinition::validate`].
    pub fn new(definition: SurfaceDefinition) -> Result<Self, MessageError> {
        definition.validate()?;
        Ok(Self {
            definition,
            revision: SurfaceRevision::ONE,
            generation: SurfaceGeneration::ONE,
        })
    }

    /// Replaces the mutable surface fields when the expected revision and generation are current.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError::InvalidValue`] when `expected_revision` or `expected_generation` is
    /// stale, the replacement changes immutable identity or profile state or fails
    /// [`SurfaceDefinition::validate`], or the revision or generation is exhausted.
    pub fn replace_mutable(
        &mut self,
        expected_revision: SurfaceRevision,
        expected_generation: SurfaceGeneration,
        mut replacement: SurfaceDefinition,
    ) -> Result<SurfaceMutation, MessageError> {
        if expected_revision != self.revision {
            return Err(invalid_value(
                "UPDATE_SURFACE",
                2,
                "does not match the current surface revision",
            ));
        }
        if expected_generation != self.generation {
            return Err(invalid_value(
                "UPDATE_SURFACE",
                3,
                "does not match the current surface generation",
            ));
        }
        if replacement.context_id != self.definition.context_id
            || replacement.surface_id != self.definition.surface_id
            || replacement.semantic_profile != self.definition.semantic_profile
            || replacement.coordinate_model != self.definition.coordinate_model
        {
            return Err(invalid_value(
                "UPDATE_SURFACE",
                0,
                "attempts to change immutable identity or profile state",
            ));
        }
        replacement.validate()?;
        let generation_changed = !self.definition.mapping_eq(&replacement);
        replacement.policy |= self.definition.policy;
        let revision = self
            .revision
            .advance()
            .map_err(|_| invalid_value("UPDATE_SURFACE", 2, "exhausted"))?;
        let generation = if generation_changed {
            self.generation
                .advance()
                .map_err(|_| invalid_value("UPDATE_SURFACE", 3, "exhausted"))?
        } else {
            self.generation
        };
        self.definition = replacement;
        self.revision = revision;
        self.generation = generation;
        Ok(SurfaceMutation {
            revision,
            generation,
            input_must_be_revoked: generation_changed,
        })
    }
}

/// Input classes a producer can inject for a `desktop-content-v1` surface, desktop §2 key 4.
///
/// A bit says the producer *can* inject that class. It does not grant the presenter permission —
/// that is the input binding's job.
pub mod input_capability {
    /// The desktop surface accepts keyboard input.
    pub const KEYBOARD: u64 = 1 << 0;
    /// The desktop surface accepts pointer motion.
    pub const POINTER_MOTION: u64 = 1 << 1;
    /// The desktop surface accepts pointer buttons.
    pub const POINTER_BUTTON: u64 = 1 << 2;
    /// The desktop surface accepts scrolling.
    pub const POINTER_AXIS: u64 = 1 << 3;
    /// Every assigned input capability bit.
    pub const KNOWN_MASK: u64 = (1 << 4) - 1;
}

/// Profile-specific parameters of a `desktop-content-v1` surface, desktop §2 keys 0 through 4.
///
/// Without this type every producer hand-encodes the map and every presenter hand-decodes it,
/// which is how the sanitization rule gets quietly dropped: desktop §1 forbids monitor serials,
/// user names, desktop names, window titles, and login-session identifiers anywhere in a topology.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopSurfaceParameters {
    /// Captured virtual-desktop origin in producer logical pixels.
    pub captured_origin_x: i32,
    /// Captured virtual-desktop origin Y in producer logical pixels.
    pub captured_origin_y: i32,
    /// Sanitized captured-output topology, using the target output schema.
    pub topology: Vec<OutputDescriptor>,
    /// Producer-owned desktop semantic generation. It advances when the actual OS session,
    /// desktop, security boundary, injected seat, display mapping, or coordinate truth changes —
    /// never as a user name or OS identifier.
    pub semantic_generation: u64,
    /// Input capability bits, such as [`input_capability::KEYBOARD`].
    pub input_capabilities: u64,
}

impl DesktopSurfaceParameters {
    /// Encodes the parameters as the surface's profile-parameter map.
    pub fn encode(&self) -> PayloadMap {
        vec![
            (0, Value::from(i64::from(self.captured_origin_x))),
            (1, Value::from(i64::from(self.captured_origin_y))),
            (
                2,
                Value::Array(self.topology.iter().map(OutputDescriptor::encode).collect()),
            ),
            (3, Value::Unsigned(self.semantic_generation)),
            (4, Value::Unsigned(self.input_capabilities)),
        ]
    }

    /// Decodes desktop-surface profile parameters.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError`] when `map` is not a strict parameter map, a field is missing or has
    /// the wrong type, an origin does not fit in `i32`, the topology is not an array of valid
    /// outputs, or the parameters fail [`DesktopSurfaceParameters::validate`].
    pub fn decode(map: &PayloadMap) -> Result<Self, MessageError> {
        let value = Value::Map(map.clone());
        let strict = StrictMap::new("desktop surface parameters", &value, &[0, 1, 2, 3, 4])?;
        let topology = match strict.required(2)? {
            Value::Array(entries) => entries
                .iter()
                .map(OutputDescriptor::decode)
                .collect::<Result<Vec<_>, _>>()?,
            _ => {
                return Err(invalid_value(
                    "desktop surface parameters",
                    2,
                    "topology must be an array",
                ));
            }
        };
        let parameters = Self {
            captured_origin_x: required_i32(&strict, 0)?,
            captured_origin_y: required_i32(&strict, 1)?,
            topology,
            semantic_generation: strict.required_u64(3)?,
            input_capabilities: strict.required_u64(4)?,
        };
        parameters.validate()?;
        Ok(parameters)
    }

    /// Checks the semantic generation, input capabilities, and output topology.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError::InvalidValue`] when the semantic generation is zero, the input
    /// capability mask has unassigned bits, or output IDs repeat within the topology.
    pub fn validate(&self) -> Result<(), MessageError> {
        if self.semantic_generation == 0 {
            return Err(invalid_value(
                "desktop surface parameters",
                3,
                "semantic generation must be nonzero",
            ));
        }
        if self.input_capabilities & !input_capability::KNOWN_MASK != 0 {
            return Err(invalid_value(
                "desktop surface parameters",
                4,
                "input capability mask has unassigned bits",
            ));
        }
        for (index, output) in self.topology.iter().enumerate() {
            if self.topology[..index]
                .iter()
                .any(|earlier| earlier.output_id == output.output_id)
            {
                return Err(invalid_value(
                    "desktop surface parameters",
                    2,
                    "output IDs must be unique within a topology",
                ));
            }
        }
        Ok(())
    }

    /// The canonical-surface to OS-coordinate mapping this surface implies.
    ///
    /// Rotation belongs to the surface's core fields rather than these parameters, so the caller
    /// supplies the surface's logical extent and rotation alongside.
    pub fn mapping(
        &self,
        logical_width: u32,
        logical_height: u32,
        rotation: Rotation,
    ) -> SurfaceMapping {
        SurfaceMapping {
            logical_width,
            logical_height,
            captured_origin_x: self.captured_origin_x,
            captured_origin_y: self.captured_origin_y,
            rotation,
        }
    }
}

fn required_i32(map: &StrictMap<'_>, key: u64) -> Result<i32, MessageError> {
    map.required(key)?
        .as_i64()
        .and_then(|value| i32::try_from(value).ok())
        .ok_or_else(|| {
            invalid_value(
                "desktop surface parameters",
                key,
                "must fit in a signed 32-bit integer",
            )
        })
}

/// The `UPDATE_SURFACE` payload: a complete replacement of a surface's mutable state (core §8.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateSurface {
    /// The owning context.
    pub context_id: u64,
    /// The surface to update.
    pub surface_id: u64,
    /// The surface revision the update expects.
    pub expected_revision: SurfaceRevision,
    /// The surface generation the update expects.
    pub expected_generation: SurfaceGeneration,
    /// The new logical width.
    pub logical_width: u64,
    /// The new logical height.
    pub logical_height: u64,
    /// The new scale numerator.
    pub scale_numerator: u64,
    /// The new scale denominator.
    pub scale_denominator: u64,
    /// The new rotation in degrees.
    pub rotation: u16,
    /// The complete new descriptor.
    pub descriptor: SurfaceDescriptor,
    /// The requested policy bits.
    pub policy: u64,
    /// The complete new profile-specific parameters.
    pub profile_parameters: PayloadMap,
}

impl UpdateSurface {
    /// Encodes the payload map.
    ///
    /// # Errors
    ///
    /// Returns any error from [`SurfaceDescriptor::to_value`].
    pub fn payload(&self) -> Result<PayloadMap, MessageError> {
        Ok(vec![
            (0, Value::Unsigned(self.context_id)),
            (1, Value::Unsigned(self.surface_id)),
            (2, Value::Unsigned(self.expected_revision.get())),
            (3, Value::Unsigned(self.expected_generation.get())),
            (4, Value::Unsigned(self.logical_width)),
            (5, Value::Unsigned(self.logical_height)),
            (6, Value::Unsigned(self.scale_numerator)),
            (7, Value::Unsigned(self.scale_denominator)),
            (8, Value::Unsigned(u64::from(self.rotation))),
            (9, self.descriptor.to_value()?),
            (10, Value::Unsigned(self.policy)),
            (11, Value::Map(self.profile_parameters.clone())),
        ])
    }

    /// Decodes an `UPDATE_SURFACE` payload.
    ///
    /// # Errors
    ///
    /// Returns [`MessageError`] when `value` is not a strict `UPDATE_SURFACE` map, a field is
    /// missing or has the wrong type, the rotation does not fit in `u16`, or the descriptor is
    /// invalid.
    pub fn decode(value: &Value) -> Result<Self, MessageError> {
        let map = StrictMap::new(
            "UPDATE_SURFACE",
            value,
            &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11],
        )?;
        Ok(Self {
            context_id: map.required_u64(0)?,
            surface_id: map.required_u64(1)?,
            expected_revision: SurfaceRevision::new(map.required_u64(2)?),
            expected_generation: SurfaceGeneration::new(map.required_u64(3)?),
            logical_width: map.required_u64(4)?,
            logical_height: map.required_u64(5)?,
            scale_numerator: map.required_u64(6)?,
            scale_denominator: map.required_u64(7)?,
            rotation: u16::try_from(map.required_u64(8)?)
                .map_err(|_| invalid_value("UPDATE_SURFACE", 8, "does not fit in u16"))?,
            descriptor: SurfaceDescriptor::from_value(map.required(9)?)?,
            policy: map.required_u64(10)?,
            profile_parameters: map.required_map(11)?.to_vec(),
        })
    }

    /// The definition this update would leave, keeping `current`'s immutable identity, profile,
    /// and coordinate model.
    pub fn replacement(&self, current: &SurfaceDefinition) -> SurfaceDefinition {
        SurfaceDefinition {
            context_id: self.context_id,
            surface_id: self.surface_id,
            semantic_profile: current.semantic_profile.clone(),
            coordinate_model: current.coordinate_model,
            logical_width: self.logical_width,
            logical_height: self.logical_height,
            scale_numerator: self.scale_numerator,
            scale_denominator: self.scale_denominator,
            rotation: self.rotation,
            descriptor: self.descriptor.clone(),
            policy: self.policy,
            profile_parameters: self.profile_parameters.clone(),
        }
    }
}

/// The `SURFACE_READY` body: a surface exists with this policy and these profile parameters.
///
/// [`payload`](Self::payload) always carries key 5, even when the parameter map is empty: a
/// missing key and an empty map are different messages on the wire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurfaceReady {
    /// The owning context.
    pub context_id: u64,
    /// The surface, local to its context.
    pub surface_id: u64,
    /// The surface revision after creation.
    pub revision: SurfaceRevision,
    /// The surface generation, which input bindings name.
    pub generation: SurfaceGeneration,
    /// The effective `POLICY_*` bits.
    pub policy: u64,
    /// The profile-specific parameters, such as a desktop surface's canonical size.
    pub profile_parameters: PayloadMap,
}

impl SurfaceReady {
    /// Encodes the body as a payload map.
    pub fn payload(self) -> PayloadMap {
        vec![
            (0, Value::Unsigned(self.context_id)),
            (1, Value::Unsigned(self.surface_id)),
            (2, Value::Unsigned(self.revision.get())),
            (3, Value::Unsigned(self.generation.get())),
            (4, Value::Unsigned(self.policy)),
            (5, Value::Map(self.profile_parameters)),
        ]
    }
}
