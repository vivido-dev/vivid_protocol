//! Public-API tests for `vivid_protocol::surface`.
use vivid_protocol::surface::*;
use vivid_protocol::{cbor::Value, geometry::Rotation, target::OutputDescriptor};

fn output(id: u64) -> OutputDescriptor {
    OutputDescriptor {
        output_id: id,
        origin_x: 0,
        origin_y: 0,
        width: 1920,
        height: 1080,
        scale_numerator: 1,
        scale_denominator: 1,
        rotation: Rotation::None,
        primary: id == 1,
    }
}

fn parameters() -> DesktopSurfaceParameters {
    DesktopSurfaceParameters {
        captured_origin_x: -1920,
        captured_origin_y: 0,
        topology: vec![output(1), output(2)],
        semantic_generation: 3,
        input_capabilities: input_capability::KEYBOARD | input_capability::POINTER_MOTION,
    }
}

#[test]
fn parameters_round_trip() {
    assert_eq!(
        DesktopSurfaceParameters::decode(&parameters().encode()).unwrap(),
        parameters()
    );
}

#[test]
fn a_zero_semantic_generation_is_rejected() {
    let mut broken = parameters();
    broken.semantic_generation = 0;
    assert!(broken.validate().is_err());
}

#[test]
fn unassigned_capability_bits_are_rejected() {
    let mut broken = parameters();
    broken.input_capabilities = 1 << 8;
    assert!(broken.validate().is_err());
}

#[test]
fn duplicate_output_ids_are_rejected() {
    let mut broken = parameters();
    broken.topology[1].output_id = 1;
    assert!(broken.validate().is_err());
}

#[test]
fn an_empty_topology_is_allowed_for_a_headless_producer() {
    let headless = DesktopSurfaceParameters {
        topology: Vec::new(),
        ..parameters()
    };
    headless.validate().unwrap();
}

#[test]
fn the_mapping_carries_the_captured_origin() {
    let mapping = parameters().mapping(1920, 1080, Rotation::None);
    assert_eq!(mapping.to_os_logical(0, 0).unwrap(), (-1920, 0));
    assert_eq!(mapping.logical_width, 1920);
}

#[test]
fn unknown_keys_are_rejected() {
    let mut map = parameters().encode();
    map.push((5, Value::Unsigned(0)));
    DesktopSurfaceParameters::decode(&map).unwrap_err();
}
