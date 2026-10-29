//! Public-API tests for `vivid_protocol::overlay::wire::text::styled`.
use vivid_protocol::cbor::Value;
use vivid_protocol::overlay::wire::WindowAddress;
use vivid_protocol::overlay::wire::text::styled::*;
use vivid_protocol::vector::Scalar;

use vivid_protocol::vector::{Canvas, Command, Point};

#[test]
fn typography_is_optional_bounded_and_roundtrips_without_changing_legacy_paragraphs() {
    let mut text = StyledText::new("hello", TextStyle::default());
    assert_eq!(text.value().unwrap().as_array().unwrap().len(), 5);
    text.max_width = Some(Scalar::new(100.).unwrap());
    text.typography = Typography {
        overflow: TextOverflow::Ellipsis,
        letter_spacing: Scalar::ONE,
        word_spacing: Scalar::new(3.).unwrap(),
        line_height: Some(Scalar::new(24.).unwrap()),
        ligatures: false,
        kerning: false,
    };
    assert_eq!(StyledText::decode(&text.value().unwrap()).unwrap(), text);
    text.max_width = None;
    text.value().unwrap_err();
    text.max_width = Some(Scalar::ONE);
    text.typography.letter_spacing = Scalar::new(-1.).unwrap();
    text.value().unwrap_err();
    text.typography.letter_spacing = Scalar::ZERO;
    text.typography.line_height = Some(Scalar::ZERO);
    text.value().unwrap_err();
}
#[test]
fn batches_roundtrip_and_enforce_aggregate_limits() {
    let address = WindowAddress {
        context_id: 1,
        surface_id: 2,
        generation: 3,
    };
    let text = StyledText::new(
        "A😀e\u{301}",
        TextStyle {
            size: Scalar::new(16.).unwrap(),
            underline: true,
            ..Default::default()
        },
    );
    let request = MeasureBatch {
        address,
        texts: vec![text.clone(), text.clone()],
        retain: true,
    };
    assert_eq!(
        MeasureBatch::decode(2, &Value::Map(request.payload().unwrap())).unwrap(),
        request
    );
    MeasureBatch {
        texts: vec![text.clone(); MAX_TEXT_BATCH + 1],
        ..request.clone()
    }
    .payload()
    .unwrap_err();
    let large = StyledText::new("x".repeat(2049), TextStyle::default());
    MeasureBatch {
        texts: vec![large.clone(), large],
        ..request.clone()
    }
    .payload()
    .unwrap_err();
    let mut invalid = text;
    invalid.max_lines = Some(0);
    invalid.value().unwrap_err();
    let release = ReleaseLayouts {
        address,
        ids: vec![u64::MAX, 1],
    };
    assert_eq!(
        ReleaseLayouts::decode(2, &Value::Map(release.payload().unwrap())).unwrap(),
        release
    );
    ReleaseLayouts {
        ids: vec![1, 1],
        ..release
    }
    .payload()
    .unwrap_err();
    let mut canvas = Canvas::new();
    canvas
        .push(Command::TextLayout {
            layout: u64::MAX,
            origin: Point::new(1., 2.).unwrap(),
        })
        .unwrap();
    assert_eq!(Canvas::decode(&canvas.encode().unwrap()).unwrap(), canvas);
}
