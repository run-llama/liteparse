use super::*;

fn properties() -> Vec<(u16, u32)> {
    vec![
        (0x80, 0x10000),
        (0x181, 0x800000),
        (0x1bf, 0x100010),
        (0x1ff, 0x80000),
    ]
}

#[test]
fn explicit_fill_and_no_line_preserve_default_text_insets() {
    let style = resolve(&properties(), 0x10000).unwrap();
    assert_eq!(style.record_index, 0);
    assert_eq!(style.insets_emu, [91440, 45720, 91440, 45720]);
    assert_eq!(style.fill_rgb, Some(0x000080));
}

#[test]
fn text_identity_and_chain_position_must_match() {
    for id in [0, 1, 0x10001, 0x20000] {
        assert!(resolve(&properties(), id).is_err());
    }
}

#[test]
fn duplicate_unknown_color_reference_and_active_line_are_rejected() {
    let mut duplicate = properties();
    duplicate.insert(1, duplicate[0]);
    let mut unknown = properties();
    unknown.insert(1, (0x88, 1));
    let mut scheme = properties();
    scheme[1].1 |= 0x08000000;
    let mut line = properties();
    line[3].1 |= 8;
    for p in [duplicate, unknown, scheme, line] {
        assert!(resolve(&p, 0x10000).is_err());
    }
}

#[test]
fn explicit_insets_have_exact_signed_nonnegative_bounds() {
    let mut p = properties();
    p.insert(1, (0x81, 0));
    assert_eq!(resolve(&p, 0x10000).unwrap().insets_emu[0], 0);
    p[1].1 = 0x0132_f540;
    assert!(resolve(&p, 0x10000).is_ok());
    for value in [0x0132_f541, u32::MAX] {
        p[1].1 = value;
        assert!(resolve(&p, 0x10000).is_err());
    }
}

#[test]
fn absent_shape_properties_use_documented_fill_and_line_defaults() {
    let style = resolve(&[(0x80, 0x10000)], 0x10000).unwrap();
    assert_eq!(style.fill_rgb, Some(0xffffff));
    assert!(style.default_line);
    let explicit = resolve(&properties(), 0x10000).unwrap();
    assert!(!explicit.default_line);
}
