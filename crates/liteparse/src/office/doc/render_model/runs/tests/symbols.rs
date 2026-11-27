use super::*;

#[test]
fn source_symbol_implicit_special_state_retains_raw_assignment_and_display() {
    let (styles, mut fonts) = fixture();
    fonts.fonts[0].name = "Wingdings 2".into();
    fonts.fonts[0].charset = 2;
    let evidence = crate::office::doc::symbols::SymbolSpan {
        start_cp: 4,
        end_cp: 5,
        font_index: 0,
        source_code: 0xf050,
        special: Some(true),
        unicode: Some('\u{1f5f8}'),
        basis: "published_symbol_encoding",
    };
    let symbol = super::super::symbols::from_evidence(&evidence, &fonts).unwrap();
    let direct = [p(0x6a09, &[0, 0, 0x50, 0xf0])];
    let value =
        super::super::symbols::resolve(&styles, &fonts, 0, &direct, &mut 0, &symbol).unwrap();
    assert_eq!(value.source_symbol.as_ref().unwrap().unicode, '\u{1f5f8}');
    assert!(value.applied.contains(&direct[0]));
    assert!(resolve(&styles, &fonts, 0, &direct).is_err());
    assert!(
        super::super::symbols::resolve(
            &styles,
            &fonts,
            0,
            &[p(0x6a09, &[0, 0, 0x51, 0xf0])],
            &mut 0,
            &symbol
        )
        .is_err()
    );
    let disabled = crate::office::doc::symbols::SymbolSpan {
        special: Some(false),
        ..evidence
    };
    assert!(super::super::symbols::from_evidence(&disabled, &fonts).is_err());
}
