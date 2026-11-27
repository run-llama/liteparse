use super::*;

#[test]
fn mt_extra_shared_slots_have_unicode_meanings() {
    for (slot, expected) in [
        (0x4B, '…'),
        (0x4C, '⋯'),
        (0x4D, '⋮'),
        (0x4E, '⋰'),
        (0x4F, '⋱'),
        (0x68, 'ℏ'),
    ] {
        assert_eq!(mt_extra_to_unicode(slot), Some(expected));
        assert_eq!(mt_extra_to_unicode(slot | 0xF000), Some(expected));
    }
    for slot in [0x31, 0x34, 0x72, 0x7B, 0xFF] {
        assert_eq!(mt_extra_to_unicode(slot), None);
        assert_eq!(readable_char("MT Extra", false, slot), Some('\u{FFFD}'));
    }
}

#[test]
fn mt_extra_default_and_symbol_charsets_preserve_slot_meaning() {
    for charset in [1, 2] {
        let decoded = decode_ansi(&[0x4C, 0x4D], charset);
        let text: String = decoded
            .into_iter()
            .filter_map(|c| readable_char("MT Extra", false, c))
            .collect();
        assert_eq!(text, "⋯⋮");
    }
    assert_eq!(readable_char("mt extra", false, 0xF04C), Some('⋯'));
    assert_eq!(readable_char("Times New Roman", false, 0x4C), Some('L'));
    assert_eq!(readable_char("Symbol", true, 0x4C), Some('Λ'));
}

/// Format12 maps one source slot to glyph7; no actual font file is needed.
fn cmap(platform: u16, encoding: u16, slot: u32) -> Vec<u8> {
    let mut bytes = Vec::new();
    for v in [0u16, 1, platform, encoding] {
        bytes.extend(v.to_be_bytes());
    }
    bytes.extend(12u32.to_be_bytes());
    for v in [12u16, 0] {
        bytes.extend(v.to_be_bytes());
    }
    for v in [28u32, 0, 1, slot, slot, 7] {
        bytes.extend(v.to_be_bytes());
    }
    bytes
}

#[test]
fn exact_legacy_cmaps_use_source_slots_without_unicode_substitution() {
    for (platform, encoding, slot) in [(3, 0, 0xF04C), (3, 1, 0x4C), (0, 4, 0xF04C)] {
        let bytes = cmap(platform, encoding, slot);
        let table = ttf_parser::cmap::Table::parse(&bytes).unwrap();
        assert_eq!(legacy_cmap_glyph(table, 0x4C), Some(7));
        assert_eq!(legacy_cmap_glyph(table, 0xF04C), Some(7));
        assert_eq!(legacy_cmap_glyph(table, 0x4D), None);
    }
    let bytes = cmap(3, 1, 0x22EF);
    let table = ttf_parser::cmap::Table::parse(&bytes).unwrap();
    assert_eq!(legacy_cmap_glyph(table, 0x4C), None);
}

#[test]
fn wide_mt_extra_text_preserves_real_unicode_outside_legacy_slots() {
    for ch in ['⋯', '⋮', 'Ā', '𝛼'] {
        assert!(!legacy_slot(ch as u32));
        assert_eq!(readable_char("MT Extra", false, ch as u32), Some(ch));
    }
    assert!(legacy_slot(0xF031));
    assert_eq!(readable_char("MT Extra", false, 0xF031), Some('\u{FFFD}'));
}
