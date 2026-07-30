//! Stable surface schemas and generation-safe mutation helpers.

use crate::{
    cbor::Value,
    messages::{
        MessageError, PayloadMap, StrictMap, invalid_value, require_nonzero, validate_header_object,
    },
    registry::{CANVAS_CONTENT, DESKTOP_CONTENT, GENERIC_CONTENT, TERMINAL_CONTENT},
    revision::{SurfaceGeneration, SurfaceRevision},
};

pub const POLICY_DENY_CAPTURE: u64 = 1 << 0;
pub const POLICY_DENY_DESCRIPTOR_EXPORT: u64 = 1 << 1;
pub const POLICY_DENY_POSTER_RETENTION: u64 = 1 << 2;
pub const POLICY_DENY_IMAGE_CACHE: u64 = 1 << 3;
pub const POLICY_REDUCED_DIAGNOSTICS: u64 = 1 << 4;
pub const POLICY_KNOWN_MASK: u64 = (1 << 5) - 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
pub enum SurfaceRole {
    Unspecified = 0,
    Document = 1,
    Desktop = 2,
    TimedMedia = 3,
    Figure = 4,
    TerminalText = 5,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
pub enum CoordinateModel {
    DesktopLogicalPixels = 1,
    Normalized = 2,
    CanvasLogicalUnits = 3,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurfaceDescriptor {
    pub role: SurfaceRole,
    pub title: String,
    pub semantic_content_revision: u64,
    pub semantic_availability: u64,
    pub locator_hint: String,
}

impl SurfaceDescriptor {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurfaceDefinition {
    pub context_id: u64,
    pub surface_id: u64,
    pub semantic_profile: String,
    pub coordinate_model: CoordinateModel,
    pub logical_width: u64,
    pub logical_height: u64,
    pub scale_numerator: u64,
    pub scale_denominator: u64,
    pub rotation: u16,
    pub descriptor: SurfaceDescriptor,
    pub policy: u64,
    pub profile_parameters: PayloadMap,
}

impl SurfaceDefinition {
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
            (GENERIC_CONTENT, CoordinateModel::DesktopLogicalPixels)
            | (GENERIC_CONTENT, CoordinateModel::Normalized)
            | (GENERIC_CONTENT, CoordinateModel::CanvasLogicalUnits)
            | (TERMINAL_CONTENT, CoordinateModel::TerminalContentCells)
            | (DESKTOP_CONTENT, CoordinateModel::DesktopLogicalPixels)
            | (CANVAS_CONTENT, CoordinateModel::CanvasLogicalUnits)
            | (CANVAS_CONTENT, CoordinateModel::Normalized) => {}
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

#[derive(Debug, Clone)]
pub struct SurfaceState {
    pub definition: SurfaceDefinition,
    pub revision: SurfaceRevision,
    pub generation: SurfaceGeneration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SurfaceMutation {
    pub revision: SurfaceRevision,
    pub generation: SurfaceGeneration,
    pub input_must_be_revoked: bool,
}

impl SurfaceState {
    pub fn new(definition: SurfaceDefinition) -> Result<Self, MessageError> {
        definition.validate()?;
        Ok(Self {
            definition,
            revision: SurfaceRevision::ONE,
            generation: SurfaceGeneration::ONE,
        })
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    fn surface(context_id: u64) -> SurfaceDefinition {
        SurfaceDefinition {
            context_id,
            surface_id: 7,
            semantic_profile: DESKTOP_CONTENT.into(),
            coordinate_model: CoordinateModel::DesktopLogicalPixels,
            logical_width: 1920,
            logical_height: 1080,
            scale_numerator: 1,
            scale_denominator: 1,
            rotation: 0,
            descriptor: SurfaceDescriptor {
                role: SurfaceRole::Desktop,
                title: "desktop".into(),
                semantic_content_revision: 1,
                semantic_availability: 0,
                locator_hint: String::new(),
            },
            policy: 0,
            profile_parameters: vec![],
        }
    }

    #[test]
    fn mapping_change_advances_generation_and_requires_input_revoke() {
        let mut state = SurfaceState::new(surface(1)).unwrap();
        let mut replacement = surface(1);
        replacement.logical_width = 1280;
        let mutation = state
            .replace_mutable(SurfaceRevision::ONE, SurfaceGeneration::ONE, replacement)
            .unwrap();
        assert_eq!(mutation.revision.get(), 2);
        assert_eq!(mutation.generation.get(), 2);
        assert!(mutation.input_must_be_revoked);
    }

    #[test]
    fn same_local_id_in_different_contexts_is_not_same_surface() {
        let first = surface(1);
        let second = surface(2);
        assert_ne!(first.context_id, second.context_id);
        assert_eq!(first.surface_id, second.surface_id);
    }
}
