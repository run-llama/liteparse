use super::*;

#[test]
fn selected_table_text_precedes_paragraph_character_and_direct_properties() {
    let (mut styles, fonts) = fixture();
    styles.styles[11] = Some(style(11, 2, None, vec![p(0x0836, &[0])]));
    let table = [
        p(0x0835, &[1]),
        p(0x0836, &[1]),
        p(0x4a4f, &[0, 0]),
        p(0x4873, &[0x11, 4]),
        p(0x6870, &[10, 20, 30, 0]),
    ];
    let inherited = resolve_table_text(&styles, &fonts, 1, &[], &table, &mut 0).unwrap();
    // Paragraph style 1 toggles inherited bold and selects font 1; its base
    // supplies English. The other selected table properties remain effective.
    assert!(!inherited.bold);
    assert!(inherited.italic);
    assert_eq!(inherited.fonts[0], 1);
    assert_eq!(inherited.language, 0x409);
    assert_eq!(inherited.color_rgb, Some([10, 20, 30]));
    let direct = [
        p(0x4a30, &[11, 0]),
        p(0x0835, &[1]),
        p(0x6870, &[1, 2, 3, 0]),
    ];
    let effective = resolve_table_text(&styles, &fonts, 1, &direct, &table, &mut 0).unwrap();
    assert!(effective.bold);
    assert!(!effective.italic);
    assert_eq!(effective.color_rgb, Some([1, 2, 3]));
    assert_eq!(effective.fonts[0], 1);
}
