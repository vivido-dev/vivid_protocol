//! Public-API tests for `vivid_protocol::overlay::wire::text`.
use vivid_protocol::cbor::Value;
use vivid_protocol::overlay::wire::WindowAddress;
use vivid_protocol::overlay::wire::text::*;
use vivid_protocol::vector::{Rect, Scalar, Text};

use vivid_protocol::vector::Point;

fn request() -> MeasureText {
    MeasureText {
        address: WindowAddress {
            context_id: 1,
            surface_id: 2,
            generation: 3,
        },
        text: Text {
            text: "A😀日".into(),
            origin: Point::new(0., 0.).unwrap(),
            size: Scalar::new(16.).unwrap(),
            family: String::new(),
            weight: 400,
            italic: false,
            color: vivid_protocol::vector::Color(0xff_ff_ff_ff),
            max_width: None,
        },
    }
}
#[test]
fn measurement_and_editor_roundtrip_and_reject_malformed_input() {
    let request = request();
    let value = Value::Map(request.payload().unwrap());
    assert_eq!(MeasureText::decode(2, &value).unwrap(), request);
    MeasureText::decode(3, &value).unwrap_err();
    let mut oversized = request.clone();
    oversized.text.text = "a".repeat(MAX_MEASURE_TEXT_BYTES + 1);
    oversized.payload().unwrap_err();
    let mut duplicate = request.payload().unwrap();
    duplicate.push((3, Value::Null));
    MeasureText::decode(2, &Value::Map(duplicate)).unwrap_err();
    for caret in [None, Some(Rect::new(1., 2., 1., 16.).unwrap())] {
        let editor = EditorGeometry {
            address: request.address,
            scene_revision: u64::MAX,
            caret,
        };
        assert_eq!(
            EditorGeometry::decode(2, &Value::Map(editor.payload().unwrap())).unwrap(),
            editor
        );
    }
    EditorGeometry {
        address: request.address,
        scene_revision: 0,
        caret: None,
    }
    .payload()
    .unwrap_err();
}
#[test]
fn measurements_reject_wrong_owner_geometry_limits_and_unicode_offsets() {
    let request = request();
    let geometry = TextGeometry {
        start: 1,
        end: 5,
        x: Scalar::ZERO,
        y: Scalar::ZERO,
        width: Scalar::ONE,
        height: Scalar::ONE,
        baseline: Scalar::ZERO,
        rtl: false,
    };
    let mut measured = TextMeasurement {
        truncated_at: None,
        width: Scalar::ONE,
        height: Scalar::ONE,
        lines: vec![],
        clusters: vec![geometry],
    };
    measured.validate_text(&request.text.text).unwrap();
    let value = Value::Map(measured.payload(request.address).unwrap());
    assert_eq!(
        TextMeasurement::decode(request.address, &value).unwrap(),
        measured
    );
    let wrong = WindowAddress {
        context_id: 8,
        ..request.address
    };
    TextMeasurement::decode(wrong, &value).unwrap_err();
    measured.clusters[0].end = 2;
    assert!(measured.validate_text(&request.text.text).is_err());
    measured.clusters = vec![measured.clusters[0].clone(); MAX_TEXT_GEOMETRY + 1];
    measured.payload(request.address).unwrap_err();
}
