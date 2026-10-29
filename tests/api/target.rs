//! Public-API tests for `vivid_protocol::target`.
use vivid_protocol::target::*;
use vivid_protocol::{
    cbor::Value,
    geometry::{Rotation, TargetExtent},
    revision::TargetGeneration,
};

fn output(id: u64, origin_x: i32, width: u32, primary: bool) -> OutputDescriptor {
    OutputDescriptor {
        output_id: id,
        origin_x,
        origin_y: 0,
        width,
        height: 1080,
        scale_numerator: 1,
        scale_denominator: 1,
        rotation: Rotation::None,
        primary,
    }
}

fn single() -> DesktopTarget {
    DesktopTarget {
        origin_x: 0,
        origin_y: 0,
        width: 1920,
        height: 1080,
        outputs: vec![output(1, 0, 1920, true)],
        settled: true,
        topology_revision: 1,
    }
}

fn dual() -> DesktopTarget {
    DesktopTarget {
        width: 3840,
        outputs: vec![output(1, 0, 1920, true), output(2, 1920, 1920, false)],
        ..single()
    }
}

#[test]
fn descriptor_round_trips() {
    for target in [single(), dual()] {
        assert_eq!(DesktopTarget::decode(&target.encode()).unwrap(), target);
    }
}

#[test]
fn a_descriptor_carries_no_device_identity() {
    // Desktop §1 forbids serials, user names, desktop names, window titles, and login-session
    // identifiers. The encoding is all integers and one boolean, so there is nowhere to put
    // them — assert that structurally rather than by scanning strings.
    for (_, value) in dual().encode() {
        let leaves = match value {
            Value::Array(entries) => entries,
            other => vec![other],
        };
        for leaf in leaves {
            match leaf {
                Value::Map(entries) => assert!(
                    entries
                        .iter()
                        .all(|(_, item)| !matches!(item, Value::Text(_) | Value::Bytes(_))),
                    "an output descriptor carried free-form data"
                ),
                Value::Text(_) | Value::Bytes(_) => panic!("target descriptor carried text"),
                _ => {}
            }
        }
    }
}

#[test]
fn outputs_must_lie_inside_the_virtual_rectangle() {
    let mut target = single();
    target.outputs[0].width = 2560;
    assert!(target.validate().is_err());
}

#[test]
fn exactly_one_output_is_primary() {
    let mut none_primary = dual();
    none_primary.outputs[0].primary = false;
    assert!(none_primary.validate().is_err());

    let mut both_primary = dual();
    both_primary.outputs[1].primary = true;
    assert!(both_primary.validate().is_err());
}

#[test]
fn output_ids_are_unique_and_extents_are_nonzero() {
    let mut duplicate = dual();
    duplicate.outputs[1].output_id = 1;
    assert!(duplicate.validate().is_err());

    let mut empty = single();
    empty.outputs[0].width = 0;
    DesktopTarget::decode(&empty.encode()).unwrap_err();

    let mut zero_scale = single();
    zero_scale.outputs[0].scale_denominator = 0;
    DesktopTarget::decode(&zero_scale.encode()).unwrap_err();
}

#[test]
fn an_empty_topology_is_allowed() {
    let headless = DesktopTarget {
        outputs: Vec::new(),
        ..single()
    };
    headless.validate().unwrap();
}

#[test]
fn settling_repeats_the_generation_exactly() {
    let unsettled = DesktopTarget {
        settled: false,
        ..single()
    };
    let mut state = DesktopTargetState::new(unsettled).unwrap();
    assert_eq!(state.generation(), TargetGeneration::ONE);

    // Core §2.2: the final unsettled-to-settled transition of an otherwise byte-identical
    // descriptor repeats the current generation rather than advancing it.
    let transition = state.offer(single()).unwrap();
    assert_eq!(
        transition,
        TargetTransition::Settled {
            generation: TargetGeneration::ONE
        }
    );
    assert_eq!(state.generation(), TargetGeneration::ONE);
}

#[test]
fn settling_together_with_a_real_change_still_advances() {
    let unsettled = DesktopTarget {
        settled: false,
        ..single()
    };
    let mut state = DesktopTargetState::new(unsettled).unwrap();
    let grown = DesktopTarget {
        settled: true,
        ..dual()
    };
    let transition = state.offer(grown).unwrap();
    match transition {
        TargetTransition::Advanced { generation, reason } => {
            assert_eq!(generation.get(), 2);
            assert!(reason & reason::VIRTUAL_BOUNDS != 0);
            assert!(reason & reason::OUTPUT_SET != 0);
        }
        other => panic!("expected an advance, got {other:?}"),
    }
}

#[test]
fn an_identical_descriptor_owes_nothing() {
    let mut state = DesktopTargetState::new(single()).unwrap();
    assert_eq!(state.offer(single()).unwrap(), TargetTransition::Unchanged);
    assert_eq!(state.generation(), TargetGeneration::ONE);
}

#[test]
fn scale_and_rotation_changes_are_reported_separately_from_geometry() {
    let mut state = DesktopTargetState::new(single()).unwrap();
    let scaled = DesktopTarget {
        outputs: vec![OutputDescriptor {
            scale_numerator: 3,
            scale_denominator: 2,
            ..output(1, 0, 1920, true)
        }],
        ..single()
    };
    match state.offer(scaled).unwrap() {
        TargetTransition::Advanced { reason, .. } => {
            assert_eq!(
                reason & reason::SCALE_OR_ROTATION,
                reason::SCALE_OR_ROTATION
            );
            assert_eq!(reason & reason::OUTPUT_GEOMETRY, 0);
            assert_eq!(reason & reason::VIRTUAL_BOUNDS, 0);
        }
        other => panic!("expected an advance, got {other:?}"),
    }
}

#[test]
fn a_rotated_output_reports_scale_or_rotation() {
    let mut state = DesktopTargetState::new(single()).unwrap();
    let rotated = DesktopTarget {
        outputs: vec![OutputDescriptor {
            rotation: Rotation::Ninety,
            ..output(1, 0, 1920, true)
        }],
        ..single()
    };
    match state.offer(rotated).unwrap() {
        TargetTransition::Advanced { reason, .. } => {
            assert!(reason & reason::SCALE_OR_ROTATION != 0);
        }
        other => panic!("expected an advance, got {other:?}"),
    }
}

#[test]
fn an_invalid_offer_leaves_the_state_untouched() {
    let mut state = DesktopTargetState::new(single()).unwrap();
    let mut broken = dual();
    broken.outputs[1].primary = true;
    state.offer(broken).unwrap_err();
    assert_eq!(state.current(), &single());
    assert_eq!(state.generation(), TargetGeneration::ONE);
}

#[test]
fn the_extent_feeds_normalized_projection() {
    let state = DesktopTargetState::new(dual()).unwrap();
    assert_eq!(state.current().extent(), TargetExtent::new(3840, 1080));
    assert_eq!(state.current().primary().unwrap().output_id, 1);
}

#[test]
fn decoding_rejects_unknown_keys() {
    let mut map = single().encode();
    map.push((7, Value::Unsigned(0)));
    DesktopTarget::decode(&map).unwrap_err();
}
