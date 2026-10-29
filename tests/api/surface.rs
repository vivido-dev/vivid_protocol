//! Public-API tests for `vivid_protocol::surface`.
use vivid_protocol::surface::*;
use vivid_protocol::{
    cbor::Value,
    registry::DESKTOP_CONTENT,
    revision::{SurfaceGeneration, SurfaceRevision},
};

#[test]
fn a_surface_ready_body_carries_identity_revisions_policy_and_parameters() {
    let parameters = vec![(0, Value::Unsigned(9))];

    assert_eq!(
        SurfaceReady {
            context_id: 1,
            surface_id: 2,
            revision: SurfaceRevision::new(3),
            generation: SurfaceGeneration::new(4),
            policy: 5,
            profile_parameters: parameters.clone(),
        }
        .payload(),
        vec![
            (0, Value::Unsigned(1)),
            (1, Value::Unsigned(2)),
            (2, Value::Unsigned(3)),
            (3, Value::Unsigned(4)),
            (4, Value::Unsigned(5)),
            (5, Value::Map(parameters)),
        ]
    );
}

#[test]
fn a_surface_with_no_profile_parameters_still_carries_the_key() {
    // An absent key and an empty map are different on the wire, and a producer that treats a
    // missing key 5 as "no parameters" would be reading a different message than one that
    // rejects it.
    let payload = SurfaceReady {
        context_id: 1,
        surface_id: 1,
        revision: SurfaceRevision::new(1),
        generation: SurfaceGeneration::new(1),
        policy: 0,
        profile_parameters: Vec::new(),
    }
    .payload();

    assert_eq!(payload.last(), Some(&(5, Value::Map(Vec::new()))));
}

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
