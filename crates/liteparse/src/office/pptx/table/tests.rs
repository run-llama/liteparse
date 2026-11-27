use super::*;

#[test]
fn style_parts_apply_banding_and_header() {
    let flags = Flags {
        first_row: true,
        band_row: true,
        ..Default::default()
    };
    assert_eq!(
        style_parts(&flags, 0, 0, 4, 3),
        vec!["a:wholeTbl", "a:firstRow"]
    );
    assert_eq!(
        style_parts(&flags, 1, 0, 4, 3),
        vec!["a:wholeTbl", "a:band1H"]
    );
    assert_eq!(
        style_parts(&flags, 2, 1, 4, 3),
        vec!["a:wholeTbl", "a:band2H"]
    );
}

#[test]
fn builtin_table_styles_parse() {
    for id in [
        "{5940675A-B579-460E-94D1-54222C63F5DA}",
        "{2D5ABB26-0587-4C30-8999-92F81FD0307C}",
        "{5C22544A-7EE6-4342-B048-85BDC9FD1C3A}",
        "{F5AB1C69-6EDB-4FF4-983F-18BD219EF322}",
        "{E8034E78-7F5D-4C2E-B375-FC64B27BC917}",
    ] {
        let s = builtin_table_style(id).unwrap();
        assert!(s.child("a:wholeTbl").is_some(), "{id}");
    }
    assert_eq!(PRESETS.with(|p| p.len()), 74);
    assert!(builtin_table_style("{00000000-0000-0000-0000-000000000000}").is_none());
}
