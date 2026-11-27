use super::*;
use crate::office::doc::Story;

fn source(text: &str) -> Inspection {
    Inspection {
        schema_version: 1,
        status: "test",
        binary_version: 0x112,
        trailing_container_bytes: 0,
        piece_count: 1,
        stories: vec![Story {
            kind: "body",
            start_cp: 0,
            utf16_units: text.encode_utf16().count(),
            text: text.into(),
        }],
        trailing_utf16_units: Vec::new(),
    }
}

fn evidence(start: usize, end: usize, unicode: Option<char>) -> SymbolEvidence {
    SymbolEvidence {
        status: "test",
        spans: vec![SymbolSpan {
            start_cp: start,
            end_cp: end,
            font_index: 0,
            source_code: 0xf0b4,
            special: Some(true),
            unicode,
            basis: "test",
        }],
    }
}

#[test]
fn mappings_follow_published_code_points_and_reject_uncertain_slots() {
    let symbol = decode_font_table(&table("Symbol", None, 2)).unwrap();
    assert_eq!(map_symbol(&symbol.fonts[0], 0xf0b4).0, Some('×'));
    assert_eq!(map_symbol(&symbol.fonts[0], 0x61).0, Some('α'));
    assert_eq!(map_symbol(&symbol.fonts[0], 0xf0be).0, Some('\u{23af}'));
    assert_eq!(map_symbol(&symbol.fonts[0], 0xf096).0, None);
    let wing = decode_font_table(&table("Wingdings", None, 2)).unwrap();
    assert_eq!(map_symbol(&wing.fonts[0], 0xf0fc).0, Some('✓'));
    assert_eq!(map_symbol(&wing.fonts[0], 0xf06f).0, Some('\u{1f78f}'));
    let ordinary = decode_font_table(&table("Symbol", None, 0)).unwrap();
    assert_eq!(map_symbol(&ordinary.fonts[0], 0xf0b4).0, None);
    let unknown = decode_font_table(&table("Unknown Symbol", None, 2)).unwrap();
    assert_eq!(map_symbol(&unknown.fonts[0], 0xf0b4).0, None);
}

#[test]
fn full_tables_cover_published_source_ranges_without_fallback_glyphs() {
    let families = [
        ("Symbol", 156, 0x47, 'Γ'),
        ("Wingdings", 221, 0x6c, '\u{26ab}'),
        ("Wingdings 2", 216, 0x52, '\u{2611}'),
        ("Wingdings 3", 207, 0x21, '\u{2b60}'),
        ("Webdings", 222, 0x61, '\u{2714}'),
    ];
    for (name, count, sample, expected) in families {
        let fonts = decode_font_table(&table(name, None, 2)).unwrap();
        let font = &fonts.fonts[0];
        assert_eq!(
            (0x20..=0xff)
                .filter(|&code| map_symbol(font, code).0.is_some())
                .count(),
            count
        );
        assert_eq!(map_symbol(font, sample).0, Some(expected));
        assert_eq!(map_symbol(font, sample + 0xf000).0, Some(expected));
        assert_eq!(map_symbol(font, 0x7f).0, None);
        assert_eq!(map_symbol(font, 0x1f).0, None);
        assert_eq!(map_symbol(font, 0x100).0, None);
    }
    for (name, first, last) in [
        ("Wingdings", 0x21, 0xfe),
        ("Wingdings 2", 0x21, 0xf9),
        ("Wingdings 3", 0x21, 0xf0),
        ("Webdings", 0x21, 0xff),
    ] {
        let fonts = decode_font_table(&table(name, None, 2)).unwrap();
        assert!(map_symbol(&fonts.fonts[0], first).0.is_some());
        assert!(map_symbol(&fonts.fonts[0], last).0.is_some());
        assert!(map_symbol(&fonts.fonts[0], first - 1).0.is_none());
        assert!(map_symbol(&fonts.fonts[0], last + 1).0.is_none());
    }
}

#[test]
fn ambiguous_and_private_adobe_slots_remain_unresolved() {
    let fonts = decode_font_table(&table("Symbol", None, 2)).unwrap();
    for code in [0x20, 0x44, 0x57, 0x6d, 0xa4, 0x60, 0xe3, 0xff] {
        assert_eq!(map_symbol(&fonts.fonts[0], code).0, None);
    }
    assert_eq!(map_symbol(&fonts.fonts[0], 0xf2).0, Some('∫'));
}

#[test]
fn standardized_arrow_extender_preserves_its_distinct_character() {
    let fonts = decode_font_table(&table("Symbol", None, 2)).unwrap();
    assert_eq!(
        map_symbol(&fonts.fonts[0], 0xbe),
        (Some('\u{23af}'), "unicode_standardized_arrow_extender")
    );
    assert_eq!(map_symbol(&fonts.fonts[0], 0xf0be).0, Some('\u{23af}'));
    assert_ne!(map_symbol(&fonts.fonts[0], 0xbe).0, Some('—'));
    let source = source("((");
    let projected = crate::office::doc::transcription::transcribe(&source);
    let result = apply_symbols(&source, &projected, &evidence(0, 2, Some('\u{23af}'))).unwrap();
    assert_eq!(result.stories[0].text, "\u{23af}\u{23af}");
    assert_eq!(
        (
            result.stories[0].segments[0].start_cp,
            result.stories[0].segments[0].end_cp
        ),
        (0, 2)
    );
}

#[test]
fn projection_replaces_only_source_markers_and_counts_utf16_vs_utf8() {
    let source = source("😀(x(");
    let projected = crate::office::doc::transcription::transcribe(&source);
    let result = apply_symbols(&source, &projected, &evidence(2, 3, Some('\u{1f78f}'))).unwrap();
    assert_eq!(result.stories[0].text, "😀\u{1f78f}x(");
    let s = result.stories[0]
        .segments
        .iter()
        .find(|s| s.kind == "symbol")
        .unwrap();
    assert_eq!(
        (s.start_cp, s.end_cp, s.output_start, s.output_end),
        (2, 3, 4, 8)
    );
}

#[test]
fn unresolved_symbol_retains_source_and_deleted_symbol_stays_deleted() {
    let source = source("(");
    let mut projected = crate::office::doc::transcription::transcribe(&source);
    let result = apply_symbols(&source, &projected, &evidence(0, 1, None)).unwrap();
    assert_eq!(result.stories[0].text, "(");
    assert_eq!(result.stories[0].segments[0].kind, "unresolved_symbol");
    projected.stories[0].text.clear();
    projected.stories[0].segments[0].kind = "deleted_revision";
    projected.stories[0].segments[0].output_end = 0;
    let result = apply_symbols(&source, &projected, &evidence(0, 1, Some('×'))).unwrap();
    assert!(result.stories[0].text.is_empty());
    assert_eq!(result.stories[0].segments[0].kind, "deleted_revision");
}

#[test]
fn generated_content_and_source_mismatch_are_retained() {
    let source = source("x(");
    let mut projected = crate::office::doc::transcription::transcribe(&source);
    let result = apply_symbols(&source, &projected, &evidence(0, 1, Some('×'))).unwrap();
    assert_eq!(result.stories[0].text, "x(");
    assert_eq!(
        result.stories[0].diagnostics[0].code,
        "symbol_source_marker_unresolved"
    );
    projected.stories[0].text = "12345".into();
    projected.stories[0].segments[0].output_end = 5;
    projected.stories[0].segments[0].kind = "generated_number";
    let result = apply_symbols(&source, &projected, &evidence(1, 2, Some('×'))).unwrap();
    assert_eq!(result.stories[0].text, "12345");
    assert_eq!(result.stories[0].segments[0].kind, "generated_number");
}

#[test]
fn resolver_requires_valid_font_and_final_special_state() {
    use crate::office::doc::visibility::{RevisionModifier, RevisionProperties, RevisionSpan};
    let fonts = decode_font_table(&table("Symbol", None, 2)).unwrap();
    let mut revision = RevisionEvidence {
        status: "test",
        basis: "test",
        hidden_text: "test",
        spans: vec![RevisionSpan {
            start_cp: 0,
            end_cp: 1,
            source_span_index: 0,
            piece_index: 0,
            properties: RevisionProperties {
                special: Some(true),
                ..RevisionProperties::default()
            },
            evidence: vec![RevisionModifier {
                source: "chpx",
                sprm: 0x6a09,
                operand: vec![0, 0, 0xb4, 0xf0],
            }],
            unresolved: Vec::new(),
        }],
    };
    assert_eq!(
        resolve_symbol_evidence(&fonts, &revision).unwrap().spans[0].unicode,
        Some('×')
    );
    revision.spans[0].properties.special = Some(false);
    assert_eq!(
        resolve_symbol_evidence(&fonts, &revision).unwrap().spans[0].unicode,
        None
    );
    revision.spans[0].properties.special = Some(true);
    revision.spans[0].evidence[0].operand[0] = 1;
    assert_eq!(
        resolve_symbol_evidence(&fonts, &revision).unwrap().spans[0].basis,
        "font_index_unavailable"
    );
    revision.spans[0].evidence[0].operand.pop();
    assert!(resolve_symbol_evidence(&fonts, &revision).is_err());
}

fn table(name: &str, alternate: Option<&str>, charset: u8) -> Vec<u8> {
    let mut record = vec![0; 39];
    record[1..3].copy_from_slice(&400u16.to_le_bytes());
    record[3] = charset;
    if alternate.is_some() {
        record[4] = (name.encode_utf16().count() + 1) as u8;
    }
    for part in std::iter::once(name).chain(alternate) {
        for unit in part.encode_utf16().chain(std::iter::once(0)) {
            record.extend(unit.to_le_bytes());
        }
    }
    let mut result = vec![1, 0, 0, 0, record.len() as u8];
    result.extend(record);
    result
}

#[test]
fn font_identity_and_alternate_are_source_data() {
    let decoded = decode_font_table(&table("Symbol", Some("Alternate"), 2)).unwrap();
    assert_eq!(decoded.fonts[0].name, "Symbol");
    assert_eq!(
        decoded.fonts[0].alternate_name.as_deref(),
        Some("Alternate")
    );
    assert_eq!(decoded.fonts[0].charset, 2);
    assert_eq!(decoded.fonts[0].weight, 400);
}

#[test]
fn truncated_and_malformed_font_records_are_rejected() {
    let valid = table("Example", None, 0);
    for n in 0..valid.len() {
        assert!(decode_font_table(&valid[..n]).is_err());
    }
    let mut bad = valid.clone();
    bad[9] = 1;
    assert!(decode_font_table(&bad).is_err());
    let mut bad = valid.clone();
    bad[8] = 3;
    assert!(decode_font_table(&bad).is_err());
    let mut bad = valid.clone();
    bad.extend([0]);
    assert!(decode_font_table(&bad).is_err());
    let mut bad = valid;
    bad[2] = 1;
    assert!(decode_font_table(&bad).is_err());
}

#[test]
fn font_name_requires_valid_unicode_and_terminator() {
    let mut bad = table("A", None, 0);
    bad[44..46].copy_from_slice(&0xd800u16.to_le_bytes());
    assert!(decode_font_table(&bad).is_err());
    let mut bad = table("A", None, 0);
    *bad.last_mut().unwrap() = 1;
    assert!(decode_font_table(&bad).is_err());
}

#[test]
fn mac_script_charsets_are_accepted() {
    // Word for Mac writes Mac script charsets: 78 is Mac Japanese.
    let decoded = decode_font_table(&table("ＭＳ 明朝", None, 78)).unwrap();
    assert_eq!(decoded.fonts[0].charset, 78);
    assert!(decode_font_table(&table("Example", None, 90)).is_err());
}

#[test]
fn lenient_decoding_keeps_a_font_with_an_unknown_charset() {
    let data = table("Times New Roman", None, 122);
    assert!(decode_font_table(&data).is_err());
    let decoded = decode_font_table_lenient(&data).unwrap();
    assert_eq!(decoded.fonts[0].name, "Times New Roman");
    assert_eq!(decoded.fonts[0].charset, 0);
}

#[test]
fn lenient_decoding_keeps_a_record_with_an_empty_primary_name() {
    // Primary name empty, alternate "Times New Roman" (seen in Word 97 files).
    let mut data = table("Times New Roman", None, 0);
    let record_start = 5;
    let name_at = record_start + 39;
    let mut units: Vec<u8> = vec![0, 0];
    units.extend(
        "Times New Roman"
            .encode_utf16()
            .chain([0])
            .flat_map(u16::to_le_bytes),
    );
    data.truncate(name_at);
    data.extend(&units);
    data[4] = (data.len() - record_start) as u8;
    data[record_start + 4] = 1;
    assert!(decode_font_table(&data).is_err());
    let decoded = decode_font_table_lenient(&data).unwrap();
    assert_eq!(decoded.fonts[0].name, "Times New Roman");
}
