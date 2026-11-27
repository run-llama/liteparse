use super::*;
use crate::office::pptx::xml::parse;

fn scheme() -> ColorScheme {
    let mut s = ColorScheme {
        map: ColorScheme::default_map(),
        ..Default::default()
    };
    s.slots
        .insert("accent1".into(), Rgba::from_hex("4472C4").unwrap());
    s.slots.insert("lt1".into(), Rgba::WHITE);
    s.slots.insert("dk1".into(), Rgba::BLACK);
    s
}

fn hex(xml: &str) -> String {
    let wrapped = format!(
        r#"<a:solidFill xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">{xml}</a:solidFill>"#
    );
    let node = parse(&wrapped).unwrap();
    resolve(&node, &scheme(), None).unwrap().to_argb_hex()
}

#[test]
fn lighter_40_matches_powerpoint_palette() {
    // PowerPoint's "Accent 1, Lighter 40%" swatch for 4472C4 is 8FAADC.
    let got = hex(
        r#"<a:schemeClr val="accent1"><a:lumMod val="60000"/><a:lumOff val="40000"/></a:schemeClr>"#,
    );
    assert_eq!(got, "FF8FAADC");
}

#[test]
fn darker_25_matches_powerpoint_palette() {
    // "Accent 1, Darker 25%" is 2F5597.
    let got = hex(r#"<a:schemeClr val="accent1"><a:lumMod val="75000"/></a:schemeClr>"#);
    assert_eq!(got, "FF2F5597");
}

#[test]
fn maps_bg_and_tx_through_clr_map() {
    assert_eq!(hex(r#"<a:schemeClr val="bg1"/>"#), "FFFFFFFF");
    assert_eq!(
        hex(r#"<a:schemeClr val="tx1"><a:alpha val="50000"/></a:schemeClr>"#),
        "80000000"
    );
}
