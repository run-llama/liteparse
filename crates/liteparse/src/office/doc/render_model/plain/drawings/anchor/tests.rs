use super::*;

#[test]
fn signed_paragraph_relative_overlay_keeps_source_coordinates() {
    let result = overlay([0, -540, 8820, 1050], 0x74).unwrap();
    assert_eq!(result.rectangle_twips, [0, -540, 8820, 1050]);
    assert_eq!(result.horizontal_origin, HorizontalOrigin::Column);
    assert_eq!(result.vertical_origin, VerticalOrigin::Paragraph);
    assert!(!result.behind_text);
    assert!(!result.locked);
}

#[test]
fn ignored_header_and_wrap_side_bits_do_not_change_overlay_geometry() {
    let a = overlay([0, 0, 20, 40], 0x60).unwrap();
    let b = overlay([0, 0, 20, 40], 0x1e61).unwrap();
    assert_eq!(a, b);
}

#[test]
fn invalid_origins_reserved_simple_flag_and_nonoverlay_remain_guarded() {
    for flags in [0x66, 0x78, 0x2060, 0x20, 0x40, 0xa0] {
        assert!(overlay([0, 0, 20, 40], flags).is_err(), "{flags:x}");
    }
}

#[test]
fn rectangle_dimensions_use_checked_wide_arithmetic() {
    for rectangle in [[0, 0, 0, 1], [0, 1, 1, 0], [i32::MIN, 0, i32::MAX, 1]] {
        assert!(overlay(rectangle, 0x60).is_err());
    }
    let result = overlay([-20, -40, 0, 0], 0xc060).unwrap();
    assert!(result.behind_text && result.locked);
}

#[test]
fn square_side_and_ignored_behind_flag_preserve_origin_and_lock() {
    for (bits, side) in [
        (0, WrapSide::Both),
        (1, WrapSide::Left),
        (2, WrapSide::Right),
        (3, WrapSide::Largest),
    ] {
        let flags = 0xc054 | (bits << 9);
        let (anchor, actual_side) = square([-10, 20, 90, 120], flags).unwrap();
        assert_eq!(actual_side, side);
        assert_eq!(anchor.horizontal_origin, HorizontalOrigin::Column);
        assert_eq!(anchor.vertical_origin, VerticalOrigin::Paragraph);
        assert!(!anchor.behind_text);
        assert!(anchor.locked);
        assert_eq!(anchor.rectangle_twips, [-10, 20, 90, 120]);
        assert!(overlay([-10, 20, 90, 120], flags).is_err());
    }
    for flags in [0x854, 0x2054, 0x74, 0x56, 0x58] {
        assert!(square([0, 0, 20, 40], flags).is_err());
    }
}

#[test]
fn top_bottom_ignores_all_side_bits_and_behind_but_keeps_coordinate_guards() {
    let expected = top_bottom([-10, 20, 90, 120], 0x34).unwrap();
    for side in 0..16 {
        let actual = top_bottom([-10, 20, 90, 120], 0x4034 | (side << 9)).unwrap();
        assert_eq!(actual, expected);
    }
    for flags in [0x36, 0x38, 0x2034, 0x54, 0x74, 0x94] {
        assert!(top_bottom([0, 0, 20, 40], flags).is_err());
    }
    assert!(top_bottom([0, 0, 0, 40], 0x34).is_err());
}
