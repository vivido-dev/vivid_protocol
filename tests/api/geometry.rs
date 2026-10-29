//! Public-API tests for `vivid_protocol::geometry`.
use vivid_protocol::geometry::*;
use vivid_protocol::{cbor::Value, scene::Fit};

fn source(width: u32, height: u32, aspect_numerator: u32, aspect_denominator: u32) -> SourceSize {
    SourceSize {
        width,
        height,
        aspect_numerator,
        aspect_denominator,
    }
}

fn logical(x: i64, y: i64, width: i64, height: i64) -> NodeGeometry {
    NodeGeometry {
        space: CoordinateSpace::TargetLogical,
        rect: FixedRect::new(
            x * FIXED_ONE,
            y * FIXED_ONE,
            width * FIXED_ONE,
            height * FIXED_ONE,
        ),
    }
}

#[test]
fn geometry_round_trips_through_its_map() {
    for geometry in [logical(3, -4, 100, 50), NodeGeometry::full_target()] {
        let decoded = NodeGeometry::decode(&geometry.encode()).unwrap();
        assert_eq!(decoded, geometry);
    }
}

#[test]
fn normalized_projection_follows_the_target_extent() {
    let node = NodeGeometry::full_target();
    let small = node.project(TargetExtent::new(800, 600)).unwrap();
    assert_eq!(
        small,
        FixedRect::new(0, 0, 800 * FIXED_ONE, 600 * FIXED_ONE)
    );

    // The same node projects differently after a target generation change, which is exactly
    // why a scene commit carries the expected target generation.
    let large = node.project(TargetExtent::new(1920, 1080)).unwrap();
    assert_eq!(
        large,
        FixedRect::new(0, 0, 1920 * FIXED_ONE, 1080 * FIXED_ONE)
    );
    assert_ne!(small, large);
}

#[test]
fn logical_projection_ignores_the_target_extent() {
    let node = logical(10, 20, 30, 40);
    let first = node.project(TargetExtent::new(800, 600)).unwrap();
    let second = node.project(TargetExtent::new(1920, 1080)).unwrap();
    assert_eq!(first, second);
}

#[test]
fn normalized_geometry_outside_the_unit_square_is_rejected() {
    let mut node = NodeGeometry::full_target();
    node.rect.x = 1;
    NodeGeometry::decode(&node.encode()).unwrap_err();
}

#[test]
fn non_positive_extents_are_rejected() {
    let mut node = logical(0, 0, 4, 4);
    node.rect.width = 0;
    NodeGeometry::decode(&node.encode()).unwrap_err();
    node.rect.width = -FIXED_ONE;
    NodeGeometry::decode(&node.encode()).unwrap_err();
}

#[test]
fn geometry_rejects_an_origin_plus_extent_overflow() {
    let node = NodeGeometry {
        space: CoordinateSpace::TargetLogical,
        rect: FixedRect::new(i64::MAX - 1, 0, 8, 8),
    };
    NodeGeometry::decode(&node.encode()).unwrap_err();
}

#[test]
fn normalized_projection_rejects_an_overflowing_extent() {
    let node = NodeGeometry {
        space: CoordinateSpace::NormalizedTarget,
        rect: FixedRect::unit(),
    };
    // A target this wide cannot exist, but the transform must refuse rather than wrap.
    node.project(TargetExtent::new(u32::MAX, u32::MAX))
        .unwrap_err();
}

#[test]
fn unknown_coordinate_space_and_extra_keys_are_rejected() {
    let mut map = NodeGeometry::full_target().encode();
    map[0].1 = Value::Unsigned(9);
    NodeGeometry::decode(&map).unwrap_err();

    let mut extra = NodeGeometry::full_target().encode();
    extra.push((5, Value::Unsigned(0)));
    NodeGeometry::decode(&extra).unwrap_err();
}

#[test]
fn contain_letterboxes_and_cover_fills() {
    let destination = FixedRect::new(0, 0, 400 * FIXED_ONE, 400 * FIXED_ONE);
    let contain = fit_quad(destination, source(200, 100, 1, 1), Fit::Contain).unwrap();
    assert_eq!(contain.width, 400 * FIXED_ONE);
    assert_eq!(contain.height, 200 * FIXED_ONE);
    assert_eq!(contain.y, 100 * FIXED_ONE, "letterboxed and centred");

    let cover = fit_quad(destination, source(200, 100, 1, 1), Fit::Cover).unwrap();
    assert_eq!(cover.width, 800 * FIXED_ONE);
    assert_eq!(cover.height, 400 * FIXED_ONE);
    assert_eq!(cover.x, -200 * FIXED_ONE, "overflows equally on both sides");
}

#[test]
fn fill_and_none_do_not_preserve_or_ignore_aspect() {
    let destination = FixedRect::new(0, 0, 400 * FIXED_ONE, 400 * FIXED_ONE);
    let fill = fit_quad(destination, source(200, 100, 1, 1), Fit::Fill).unwrap();
    assert_eq!(fill, destination);

    let none = fit_quad(destination, source(200, 100, 1, 1), Fit::None).unwrap();
    assert_eq!(none.width, 200 * FIXED_ONE);
    assert_eq!(none.height, 100 * FIXED_ONE);
}

#[test]
fn sample_aspect_ratio_widens_the_source() {
    let destination = FixedRect::new(0, 0, 400 * FIXED_ONE, 400 * FIXED_ONE);
    let square = fit_quad(destination, source(100, 100, 1, 1), Fit::None).unwrap();
    let wide = fit_quad(destination, source(100, 100, 2, 1), Fit::None).unwrap();
    assert_eq!(wide.width, square.width * 2);
    assert_eq!(wide.height, square.height);
}

#[test]
fn degenerate_fit_inputs_are_refused() {
    let destination = FixedRect::new(0, 0, 400 * FIXED_ONE, 400 * FIXED_ONE);
    assert!(fit_quad(destination, source(0, 100, 1, 1), Fit::Contain).is_none());
    assert!(fit_quad(destination, source(100, 0, 1, 1), Fit::Contain).is_none());
    assert!(fit_quad(destination, source(100, 100, 0, 1), Fit::Contain).is_none());
    assert!(fit_quad(destination, source(100, 100, 1, 0), Fit::Contain).is_none());
}

#[test]
fn fit_compares_scales_as_rationals_not_as_tuples() {
    // Regression: `(numerator, denominator)` tuples order lexicographically, which picks the
    // larger denominator rather than the larger ratio. Cover must choose 400/100, not 400/200.
    let destination = FixedRect::new(0, 0, 400 * FIXED_ONE, 400 * FIXED_ONE);
    let cover = fit_quad(destination, source(200, 100, 1, 1), Fit::Cover).unwrap();
    let contain = fit_quad(destination, source(200, 100, 1, 1), Fit::Contain).unwrap();
    assert!(
        cover.width > contain.width,
        "cover scales up more than contain"
    );
    assert_eq!(cover.width / contain.width, 2);
}

#[test]
fn fit_does_not_overflow_at_realistic_display_sizes() {
    // Regression: fixed-point extents are around 2^41, so `value * numerator` in i64 overflows
    // long before the inputs are unreasonable. 8K into 8K must simply work.
    let destination = FixedRect::new(0, 0, 7680 * FIXED_ONE, 4320 * FIXED_ONE);
    for fit in [Fit::Contain, Fit::Cover, Fit::Fill, Fit::None] {
        let quad = fit_quad(destination, source(7680, 4320, 1, 1), fit).unwrap();
        assert!(
            quad.width > 0 && quad.height > 0,
            "{fit:?} produced an empty quad"
        );
    }
    let scaled = fit_quad(destination, source(1920, 1080, 1, 1), Fit::Contain).unwrap();
    assert_eq!(scaled.width, 7680 * FIXED_ONE);
    assert_eq!(scaled.height, 4320 * FIXED_ONE);
}

#[test]
fn intersection_clips_and_reports_empty() {
    let quad = FixedRect::new(0, 0, 100, 100);
    let clip = FixedRect::new(50, 50, 100, 100);
    assert_eq!(intersect(quad, clip), Some(FixedRect::new(50, 50, 50, 50)));
    assert_eq!(intersect(quad, FixedRect::new(200, 200, 10, 10)), None);
}

fn mapping(rotation: Rotation) -> SurfaceMapping {
    SurfaceMapping {
        logical_width: 1920,
        logical_height: 1080,
        captured_origin_x: 0,
        captured_origin_y: 0,
        rotation,
    }
}

#[test]
fn a_point_on_the_far_edge_is_outside_the_surface() {
    let mapping = mapping(Rotation::None);
    mapping.validate_point(1919 << 32, 1079 << 32).unwrap();
    // Desktop §7 is strict: `x < width << 32`, so the width itself is out.
    assert!(mapping.validate_point(1920 << 32, 0).is_err());
    assert!(mapping.validate_point(0, 1080 << 32).is_err());
}

#[test]
fn every_rotation_maps_the_corners_correctly() {
    let top_left = (0_u64, 0_u64);
    let bottom_right = (1919_u64 << 32, 1079_u64 << 32);

    assert_eq!(
        mapping(Rotation::None)
            .to_os_logical(top_left.0, top_left.1)
            .unwrap(),
        (0, 0)
    );
    assert_eq!(
        mapping(Rotation::None)
            .to_os_logical(bottom_right.0, bottom_right.1)
            .unwrap(),
        (1919, 1079)
    );

    // 90 degrees clockwise sends the canonical top-left to the captured top-right.
    assert_eq!(
        mapping(Rotation::Ninety)
            .to_os_logical(top_left.0, top_left.1)
            .unwrap(),
        (1079, 0)
    );
    assert_eq!(
        mapping(Rotation::OneEighty)
            .to_os_logical(top_left.0, top_left.1)
            .unwrap(),
        (1919, 1079)
    );
    assert_eq!(
        mapping(Rotation::TwoSeventy)
            .to_os_logical(top_left.0, top_left.1)
            .unwrap(),
        (0, 1919)
    );
}

#[test]
fn rotation_swaps_the_extent_only_on_the_quarter_turns() {
    assert_eq!(mapping(Rotation::None).rotated_extent(), (1920, 1080));
    assert_eq!(mapping(Rotation::OneEighty).rotated_extent(), (1920, 1080));
    assert_eq!(mapping(Rotation::Ninety).rotated_extent(), (1080, 1920));
    assert_eq!(mapping(Rotation::TwoSeventy).rotated_extent(), (1080, 1920));
}

#[test]
fn the_captured_origin_offsets_the_result() {
    let mapping = SurfaceMapping {
        captured_origin_x: -1920,
        captured_origin_y: 200,
        ..mapping(Rotation::None)
    };
    assert_eq!(mapping.to_os_logical(0, 0).unwrap(), (-1920, 200));
}

#[test]
fn a_captured_origin_that_leaves_the_os_space_is_refused() {
    let mapping = SurfaceMapping {
        captured_origin_x: i32::MAX,
        ..mapping(Rotation::None)
    };
    mapping.to_os_logical(1919 << 32, 0).unwrap_err();
}

#[test]
fn sub_pixel_coordinates_truncate_toward_the_containing_pixel() {
    let mapping = mapping(Rotation::None);
    let three_quarters = (1919_u64 << 32) | (3 << 30);
    assert_eq!(mapping.to_os_logical(three_quarters, 0).unwrap().0, 1919);
}

#[test]
fn pixel_conversion_round_trips_and_refuses_overflow() {
    assert_eq!(to_pixels(from_pixels(42).unwrap()), 42);
    assert_eq!(to_pixels(from_pixels(-42).unwrap()), -42);
    assert!(from_pixels(i64::MAX).is_none());
}

#[test]
fn rotation_rejects_an_unregistered_angle() {
    Rotation::try_from(45).unwrap_err();
    assert_eq!(Rotation::try_from(270).unwrap(), Rotation::TwoSeventy);
}

#[test]
fn clip_round_trips_and_rejects_empty_extents() {
    let clip = FixedRect::new(-5, -6, 20, 30);
    assert_eq!(decode_clip(&encode_clip(clip)).unwrap(), clip);
    decode_clip(&encode_clip(FixedRect::new(0, 0, 0, 4))).unwrap_err();
    decode_clip(&encode_clip(FixedRect::new(0, 0, 4, -1))).unwrap_err();
}
