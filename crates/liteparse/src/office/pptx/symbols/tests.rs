use super::*;

const ALL: [SymbolFont; 5] = [
    SymbolFont::Symbol,
    SymbolFont::Wingdings,
    SymbolFont::Wingdings2,
    SymbolFont::Wingdings3,
    SymbolFont::Webdings,
];

#[test]
fn recognizes_name_spellings() {
    for name in ["Wingdings 2", "Wingdings2", "WINGDINGS-2", "wingdings_2"] {
        assert_eq!(SymbolFont::from_name(name), Some(SymbolFont::Wingdings2));
    }
    assert_eq!(
        SymbolFont::from_name("Wingdings-Regular"),
        Some(SymbolFont::Wingdings)
    );
    assert_eq!(SymbolFont::from_name("SymbolMT"), Some(SymbolFont::Symbol));
    assert_eq!(
        SymbolFont::from_name("webdings"),
        Some(SymbolFont::Webdings)
    );
    assert_eq!(
        SymbolFont::from_name("Wingdings 3"),
        Some(SymbolFont::Wingdings3)
    );
    assert_eq!(SymbolFont::from_name("Arial"), None);
    assert_eq!(SymbolFont::from_name("Segoe UI Symbol"), None);
    assert_eq!(
        SymbolFont::from_name("Wingdings,Sans-Serif"),
        Some(SymbolFont::Wingdings)
    );
    assert_eq!(SymbolFont::from_name("Arial,Wingdings"), None);
    assert_eq!(map_symbol_char("Symbol,Sans-Serif", '\u{F0B7}'), Some('•'));
}

#[test]
fn every_code_maps_to_a_printable_character() {
    for font in ALL {
        for code in 0x20..=0xFFu32 {
            let ch = font.map(char::from_u32(code).unwrap());
            assert!(!ch.is_control(), "{font:?} {code:#X}");
            assert!(
                !('\u{E000}'..='\u{F8FF}').contains(&ch),
                "{font:?} {code:#X} maps to private use"
            );
        }
    }
}

#[test]
fn private_use_form_matches_byte_form() {
    for font in ALL {
        for code in 0x20..=0xFFu32 {
            let byte = font.map(char::from_u32(code).unwrap());
            let pua = font.map(char::from_u32(0xF000 + code).unwrap());
            assert_eq!(byte, pua, "{font:?} {code:#X}");
        }
    }
}

#[test]
fn maps_common_bullets() {
    let wd = |c: char| map_symbol_char("Wingdings", c);
    assert_eq!(wd('§'), Some('▪'));
    assert_eq!(wd('Ø'), Some('⮚'));
    assert_eq!(wd('\u{F0D8}'), Some('⮚'));
    assert_eq!(wd('ü'), Some('✓'));
    assert_eq!(wd('l'), Some('●'));
    assert_eq!(wd('n'), Some('◼'));
    assert_eq!(wd('q'), Some('❑'));
    assert_eq!(wd('v'), Some('❖'));
    assert_eq!(wd('\u{9F}'), Some('•'));
    assert_eq!(wd('J'), Some('☺'));
    assert_eq!(wd('!'), Some('🖉'));
    assert_eq!(map_symbol_char("Wingdings 2", '\u{97}'), Some('⦁'));
    assert_eq!(map_symbol_char("Wingdings 2", 'P'), Some('🗸'));
    assert_eq!(map_symbol_char("Wingdings 3", 'u'), Some('▶'));
    assert_eq!(map_symbol_char("Wingdings 3", '}'), Some('🞂'));
    assert_eq!(map_symbol_char("Webdings", 'a'), Some('✔'));
    assert_eq!(map_symbol_char("Webdings", '\u{F0FC}'), Some('🌍'));
}

#[test]
fn maps_symbol_font() {
    let sym = |c: char| map_symbol_char("Symbol", c);
    assert_eq!(sym('\u{F0B7}'), Some('•'));
    assert_eq!(sym('a'), Some('α'));
    assert_eq!(sym('D'), Some('Δ'));
    assert_eq!(sym('W'), Some('Ω'));
    assert_eq!(sym('m'), Some('μ'));
    assert_eq!(sym('-'), Some('−'));
    assert_eq!(sym('Þ'), Some('⇒'));
    assert_eq!(sym('¾'), Some('—'));
    assert_eq!(sym('æ'), Some('⎛'));
    assert_eq!(sym('2'), Some('2'));
}

#[test]
fn glyphless_codes_become_spaces() {
    assert_eq!(map_symbol_char("Wingdings 3", '\u{F0F5}'), Some(' '));
    assert_eq!(map_symbol_char("Symbol", '\u{80}'), Some(' '));
    assert_eq!(map_symbol_char("Wingdings", '\u{7F}'), Some(' '));
}

#[test]
fn unicode_text_passes_through() {
    assert_eq!(map_symbol_char("Wingdings", 'あ'), Some('あ'));
    assert_eq!(map_symbol_char("Symbol", '≥'), Some('≥'));
    assert_eq!(map_symbol_char("Wingdings", '\t'), Some('\t'));
    assert_eq!(map_symbol_char("Monotype Sorts", 'n'), Some('•'));
    assert_eq!(map_symbol_char("Arial", 'a'), None);
}

#[test]
fn symbol_glyphs_keep_the_real_font_widths() {
    // Wingdings 0xD8 (bullet arrow) and 0xA7 (small square), byte and private-use forms.
    assert_eq!(SymbolFont::Wingdings.advance_em('\u{D8}'), Some(0.794));
    assert_eq!(SymbolFont::Wingdings.advance_em('\u{F0A7}'), Some(0.458));
    assert_eq!(SymbolFont::Webdings.advance_em('a'), Some(1.0));
    assert_eq!(SymbolFont::Symbol.advance_em('a'), None);
    assert_eq!(SymbolFont::Wingdings.advance_em('\u{3042}'), None);
}
