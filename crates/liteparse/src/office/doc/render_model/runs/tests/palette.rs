use super::*;

#[test]
fn palette_overrides_style_rgb_and_preserves_automatic_distinction() {
    let (mut styles, fonts) = fixture();
    styles.styles[0].as_mut().unwrap().properties[0]
        .bytes
        .extend(encoded(&[p(0x6870, &[10, 20, 30, 0])]));
    for (index, expected) in [
        (2, Some([0, 0, 255])),
        (6, Some([255, 0, 0])),
        (8, Some([255, 255, 255])),
        (0, None),
    ] {
        let direct = [p(0x2a42, &[index])];
        let result = resolve(&styles, &fonts, 0, &direct).unwrap();
        assert_eq!(result.color_rgb, expected);
        assert_eq!(result.applied.last(), direct.last());
    }
    let direct = [p(0x2a42, &[6]), p(0x6870, &[17, 18, 19, 0])];
    assert_eq!(
        resolve(&styles, &fonts, 0, &direct).unwrap().color_rgb,
        Some([17, 18, 19])
    );
}

#[test]
fn malformed_or_unqualified_palette_values_remain_rejected_before_reset() {
    let (styles, fonts) = fixture();
    for operand in [vec![], vec![2, 0], vec![17], vec![255], vec![13]] {
        assert!(resolve(&styles, &fonts, 0, &[p(0x2a42, &operand), p(0x2a42, &[0])]).is_err());
    }
}
