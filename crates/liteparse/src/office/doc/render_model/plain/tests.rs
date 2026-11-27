use super::*;

#[test]
fn cell_controls_require_proven_terminal_positions() {
    let p = paragraph("A\u{7}");
    assert!(validate_partition(std::slice::from_ref(&p), [0, 2], 1).is_err());
    validate_partition_marks(std::slice::from_ref(&p), [0, 2], 1, &[1].into()).unwrap();
    assert!(validate_partition_marks(&[paragraph("\u{7}A\r")], [0, 3], 1, &[0].into()).is_err());
    assert!(validate_partition_marks(&[paragraph("A\r")], [0, 2], 1, &[1].into()).is_err());
}

#[test]
fn table_ledger_rejects_missing_duplicate_or_changed_source_owners() {
    use crate::office::doc::render_model::tables::{self, source::*};
    let cell = paragraph("A\u{7}");
    let mut mark = paragraph("\u{7}");
    mark.source_cp = [2, 3];
    mark.runs[0].source_cp = [2, 3];
    let mut outside = paragraph("B\r");
    outside.source_cp = [3, 5];
    outside.runs[0].source_cp = [3, 5];
    let paras = vec![cell, mark, outside];
    let layout = tables::decode_row(
        0xc1,
        &[
            crate::office::doc::formatting::PropertyModifier {
                sprm: 0x9602,
                operand: 108i16.to_le_bytes().to_vec(),
            },
            crate::office::doc::formatting::PropertyModifier {
                sprm: 0xd608,
                operand: vec![6, 0, 1, 0, 0, 0xe8, 3],
            },
        ],
    )
    .unwrap();
    let grid = tables::common_grid(std::slice::from_ref(&layout)).unwrap();
    let mut source = SourceTable {
        source_cp: [0, 3],
        grid,
        grouping_basis: "synthetic",
        rows: vec![SourceRow {
            table_membership: Vec::new(),
            depth: 1,
            source_cp: [0, 3],
            row_mark_paragraph: 1,
            row_mark_style: 0,
            row_mark_properties: paras[1].properties.clone(),
            layout,
            cells: vec![SourceCell {
                nested_tables: Vec::new(),
                source_cp: [0, 2],
                paragraphs: vec![CellParagraph {
                    table_membership: Vec::new(),
                    physical_index: 0,
                    source_cp: [0, 2],
                    source_style: 0,
                    properties: paras[0].properties.clone(),
                }],
            }],
        }],
    };
    assert_eq!(
        validate_ledger(std::slice::from_ref(&source), &paras, 0xc1).unwrap(),
        [1, 2].into()
    );
    source.rows[0].row_mark_paragraph = 0;
    assert!(validate_ledger(std::slice::from_ref(&source), &paras, 0xc1).is_err());
    source.rows[0].row_mark_paragraph = 1;
    source.rows[0].cells[0].paragraphs[0].physical_index = 2;
    assert!(validate_ledger(std::slice::from_ref(&source), &paras, 0xc1).is_err());
    source.rows[0].cells[0].paragraphs[0].physical_index = 0;
    source.grid.edges_twips[1] += 1;
    assert!(validate_ledger(std::slice::from_ref(&source), &paras, 0xc1).is_err());
}

#[test]
fn typographic_punctuation_preserves_source_and_font_route_guards() {
    let text = "A–—‘’“”•…※\u{2002}Z\r";
    let mut p = paragraph(text);
    let end = text.encode_utf16().count();
    validate_partition(std::slice::from_ref(&p), [0, end], 1).unwrap();
    assert_eq!(p.runs[0].text, text);
    p.runs[0].properties.hint = 1;
    validate_partition(std::slice::from_ref(&p), [0, end], 1).unwrap();
    for hint in [2, 3, 255] {
        p.runs[0].properties.hint = hint;
        assert!(validate_partition(std::slice::from_ref(&p), [0, end], 1).is_err());
    }
    p.runs[0].properties.hint = 0;
    p.runs[0].properties.bidi = true;
    assert!(validate_partition(std::slice::from_ref(&p), [0, end], 1).is_err());
    p.runs[0].properties.bidi = false;
    p.runs[0].properties.fonts[2] = 1;
    assert!(validate_partition(&[p], [0, end], 1).is_err());
}

#[test]
fn punctuation_support_keeps_layout_controls_and_soft_hyphens_guarded() {
    for c in [
        '\u{ad}', '\u{2003}', '\u{200b}', '\u{200e}', '\u{2011}', '\u{2028}', '\u{2029}',
        '\u{202a}', '\u{202f}', '\u{2060}', '\u{2066}',
    ] {
        let text = format!("A{c}B\r");
        assert!(validate_partition(&[paragraph(&text)], [0, 4], 1).is_err());
        assert!(!supported_character(c as u32));
    }
}

#[test]
fn no_proofing_language_is_retained_only_for_plain_paragraph_mark() {
    let mut p = paragraph("\r");
    p.runs[0].properties.language = 0x0400;
    validate_partition(std::slice::from_ref(&p), [0, 1], 1).unwrap();
    assert_eq!(p.runs[0].properties.language, 0x0400);
    for text in ["a\r", " \r", "\t\r"] {
        let mut other = paragraph(text);
        other.runs[0].properties.language = 0x0400;
        assert!(validate_partition(&[other], [0, text.len()], 1).is_err());
    }
    p.runs[0].properties.bidi = true;
    assert!(validate_partition(std::slice::from_ref(&p), [0, 1], 1).is_err());
    p.runs[0].properties.bidi = false;
    p.runs[0].properties.hint = 2;
    assert!(validate_partition(&[p], [0, 1], 1).is_err());
}

#[test]
fn paragraph_controls_require_exact_physical_ownership() {
    let units: Vec<u16> = "a\r\rb\r".encode_utf16().collect();
    assert!(paragraph_range(&units, 0, 2));
    assert!(paragraph_range(&units, 2, 3));
    assert!(paragraph_range(&units, 3, 5));
    for (start, end) in [(0, 1), (0, 3), (0, 5), (3, 3), (4, 3), (3, 6)] {
        assert!(!paragraph_range(&units, start, end));
    }
}

#[test]
fn footnote_story_requires_owned_relationship_tables() {
    assert_eq!(
        resolve(&crate::office::doc::tests::fixture()).unwrap_err(),
        Error::Invalid("missing note relationship tables")
    );
}

#[test]
fn source_field_marker_is_rejected_before_projection_can_hide_it() {
    let mut data = crate::office::doc::tests::fixture();
    crate::office::doc::tests::put32(&mut data[1536..5632], 80, 0);
    crate::office::doc::tests::put32(&mut data[1536..5632], 76, 10);
    data[1536 + 2048] = 0x13;
    assert_eq!(
        resolve(&data).unwrap_err(),
        Error::Unsupported("render cached field topology")
    );
}

pub(super) fn paragraph(text: &str) -> PlainParagraph {
    PlainParagraph {
        cached_fields: Vec::new(),
        source_cp: [0, text.encode_utf16().count()],
        source_style: 0,
        properties: paragraphs::ParagraphProperties {
            justification: 0,
            font_alignment: None,
            left_twips: 0,
            right_twips: 0,
            first_twips: 0,
            before_twips: 0,
            after_twips: 0,
            keep_next: false,
            keep_lines: false,
            page_break_before: false,
            widow_control: true,
            outline_level: 9,
            line_multiple_240ths: 240,
            line_spacing: None,
            tabs: vec![],
            applied: vec![],
        },
        runs: vec![PlainRun {
            source_cp: [0, text.encode_utf16().count()],
            text: text.into(),
            properties: runs::RunProperties {
                source_symbol: None,
                fonts: [0; 3],
                size_half_points: 20,
                bold: false,
                italic: false,
                small_caps: false,
                all_caps: false,
                underline: false,
                hidden: false,
                language: 0x409,
                kerning_threshold_half_points: 0,
                color_rgb: None,
                shading_rgb: None,
                highlight_rgb: None,
                scale_percent: 100,
                character_spacing_twips: 0,
                complex: Default::default(),
                hint: 0,
                bidi: false,
                vertical_alignment: 0,
                complex_property_evidence: vec![],
                applied: vec![],
            },
        }],
    }
}

#[test]
fn public_model_partition_rejects_bad_font_and_source_ranges() {
    let mut p = paragraph("ab\r");
    assert!(validate_partition(&[p], [0, 3], 1).is_ok());
    p = paragraph("ab\r");
    p.runs[0].properties.fonts[2] = 1;
    assert!(validate_partition(&[p], [0, 3], 1).is_err());
    p = paragraph("ab\r");
    p.runs[0].source_cp = [1, 3];
    assert!(validate_partition(&[p], [0, 3], 1).is_err());
    p = paragraph("ab\r");
    p.source_cp[1] = 2;
    assert!(validate_partition(&[p], [0, 3], 1).is_err());
    p = paragraph("ab\r");
    p.runs[0].source_cp = [3, 0];
    assert!(validate_partition(&[p], [0, 3], 1).is_err());
    assert!(validate_partition(&[paragraph("a\rb\r")], [0, 4], 1).is_err());
    assert!(validate_partition(&[paragraph("abc")], [0, 3], 1).is_err());
    assert!(validate_partition(&[paragraph("😀\r")], [0, 3], 1).is_err());
}

#[test]
fn split_run_owns_paragraph_mark_and_empty_paragraph() {
    let mut p = paragraph("ab\r");
    let properties = p.runs[0].properties.clone();
    p.runs = vec![
        PlainRun {
            source_cp: [0, 2],
            text: "ab".into(),
            properties: properties.clone(),
        },
        PlainRun {
            source_cp: [2, 3],
            text: "\r".into(),
            properties,
        },
    ];
    assert!(validate_partition(&[p], [0, 3], 1).is_ok());
    assert!(validate_partition(&[paragraph("\r")], [0, 1], 1).is_ok());
}

#[test]
fn justified_paragraphs_keep_source_controls_and_reject_unknown_alignment() {
    let mut blank = paragraph("\r");
    blank.properties.justification = 3;
    validate_partition(&[blank], [0, 1], 1).unwrap();
    for text in [" \r", "A\r", "\t\r"] {
        let mut p = paragraph(text);
        p.properties.justification = 3;
        validate_partition(
            std::slice::from_ref(&p),
            [0, text.encode_utf16().count()],
            1,
        )
        .unwrap();
        p.properties.justification = 4;
        assert!(validate_partition(&[p], [0, text.encode_utf16().count()], 1).is_err());
    }
}

#[test]
fn complex_route_needs_selected_font_size_and_qualified_script() {
    let mut p = paragraph(" \r");
    p.runs[0].properties.hint = 2;
    p.runs[0].properties.bidi = true;
    assert!(validate_partition(std::slice::from_ref(&p), [0, 2], 1).is_err());
    p.runs[0].properties.complex.font = Some(0);
    p.runs[0].properties.complex.size_half_points = Some(36);
    validate_partition(std::slice::from_ref(&p), [0, 2], 1).unwrap();
    for text in ["A\r", "ب\r"] {
        p.runs[0].text = text.into();
        validate_partition(std::slice::from_ref(&p), [0, 2], 1).unwrap();
    }
    for text in ["中\r", "א\r", "\u{202e}\r", "\u{200c}\r"] {
        p.runs[0].text = text.into();
        assert!(validate_partition(std::slice::from_ref(&p), [0, 2], 1).is_err());
    }
    p.runs[0].text = "ب\r".into();
    p.runs[0].properties.hint = 0;
    p.runs[0].properties.bidi = false;
    assert!(validate_partition(&[p], [0, 2], 1).is_err());
}

#[test]
fn hidden_scope_keeps_only_exact_empty_non_table_paragraph() {
    let mut p = paragraph("\r");
    p.runs[0].properties.hidden = true;
    assert!(validate_partition(std::slice::from_ref(&p), [0, 1], 1).is_ok());
    assert!(
        validate_partition_marks(&[p], [0, 1], 1, &std::collections::BTreeSet::from([0])).is_err()
    );
    for text in ["a\r", " \r", "\t\r", "\u{7}"] {
        let mut p = paragraph(text);
        p.runs[0].properties.hidden = true;
        assert!(validate_partition(&[p], [0, text.len()], 1).is_err());
    }
}

#[test]
fn table_line_grid_compatibility_requires_supported_source_grid() {
    assert!(!unsupported_table_compatibility(
        "do_not_adjust_line_height_in_table",
        0
    ));
    for grid in [1, 3, u16::MAX] {
        assert!(unsupported_table_compatibility(
            "do_not_adjust_line_height_in_table",
            grid
        ));
    }
    for name in [
        "same_style_spacing_in_table",
        "do_not_autofit_constrained_tables",
        "autofit_like_word11",
        "do_not_vertically_align_cell_with_shapes",
        "do_not_break_constrained_forced_tables",
        "word11_kerning_pairs",
    ] {
        assert!(unsupported_table_compatibility(name, 0));
    }
}

#[test]
fn manual_line_breaks_keep_paragraph_and_cell_source_partition() {
    for text in ["A\u{b}B\r", "\u{b}\u{b}\r", "A\u{b}\r", "A\u{e}B\r"] {
        let p = paragraph(text);
        validate_partition(std::slice::from_ref(&p), [0, text.len()], 1).unwrap();
        assert_eq!(p.runs[0].text, text);
    }
    let p = paragraph("A\u{b}B\u{7}");
    validate_partition_marks(&[p], [0, 4], 1, &[3].into()).unwrap();
    let text = "A\u{e}B\r";
    assert!(validate_partition(&[paragraph(text)], [0, text.len() - 1], 1).is_err());
    let mut p = paragraph("A\u{b}B\r");
    p.runs[0].properties.hidden = true;
    assert!(validate_partition(&[p], [0, 4], 1).is_err());
}

#[test]
fn manual_break_charset_qualification_uses_other_font() {
    assert!(qualify_character_charset('\u{b}', [2, 0, 0], 0).is_ok());
    assert!(qualify_character_charset('\u{e}', [2, 0, 0], 0).is_ok());
    assert!(qualify_character_charset('\u{e}', [0, 0, 2], 0).is_err());
    assert!(qualify_character_charset('\u{b}', [0, 0, 2], 0).is_err());
    assert!(qualify_character_charset('A', [2, 0, 0], 0).is_err());
    assert!(qualify_character_charset('A', [0, 0, 2], 0).is_ok());
    assert!(qualify_character_charset('\r', [0, 0, 2], 0).is_ok());
}

#[test]
fn manual_page_control_uses_other_font_charset_and_retains_paragraph_end() {
    qualify_character_charset('\u{c}', [2, 0, 0], 0).unwrap();
    assert!(qualify_character_charset('\u{c}', [0, 0, 2], 0).is_err());
    let p = paragraph("A\u{c}B\r");
    validate_partition(std::slice::from_ref(&p), [0, 4], 1).unwrap();
    assert!(validate_partition(&[paragraph("A\u{c}")], [0, 2], 1).is_err());
}

#[test]
fn ordinary_symbols_preserve_scalars_and_selected_font_ownership() {
    let text = "A‚„†‡‰‹›€™√≤≥─■□○●Z\r";
    let p = paragraph(text);
    validate_partition(
        std::slice::from_ref(&p),
        [0, text.encode_utf16().count()],
        1,
    )
    .unwrap();
    assert_eq!(p.runs[0].text, text);
    for c in text.chars().filter(|c| !c.is_ascii()) {
        assert!(qualify_character_charset(c, [2, 0, 0], 0).is_ok());
        assert!(qualify_character_charset(c, [0, 0, 2], 0).is_err());
    }
    let mut invalid = p;
    invalid.runs[0].source_cp[1] -= 1;
    assert!(validate_partition(&[invalid], [0, text.encode_utf16().count()], 1).is_err());
}

#[test]
fn legacy_ethiopic_break_candidates_stay_outside_plain_text_partition() {
    for c in 0x1361..=0x1368 {
        let text = format!("A{}B\r", char::from_u32(c).unwrap());
        assert!(validate_partition(&[paragraph(&text)], [0, 4], 1).is_err());
    }
    validate_partition(&[paragraph("A†B\r")], [0, 4], 1).unwrap();
}

#[test]
fn inactive_ethiopic_setting_preserves_unrelated_compatibility_rejections() {
    let mut s = settings::DocumentSettings {
        binary_version: 0x112,
        dop_schema: "fixture",
        raw_dop: vec![],
        recorded_install_language: 0x409,
        override_normal_font_and_language: false,
        override_section_geometry: false,
        default_tab_twips: 720,
        mirror_margins: false,
        gutter_at_top: false,
        different_even_and_odd_headers: false,
        automatic_hyphenation: false,
        hyphenate_capitals: false,
        hyphenation_zone_twips: 0,
        consecutive_hyphen_limit: 0,
        update_styles_from_template: false,
        saved_word97_compatibility: false,
        saved_show_revision_markup: false,
        saved_print_revision_markup: false,
        two_pages_per_sheet: false,
        feature_restrictions: None,
        feature_restrictions_dop_byte: None,
        issues: vec![],
        compatibility: vec![settings::CompatibilityOption {
            name: "ethiopic_amharic_breaking_rules",
            enabled: true,
            dop_byte: 512,
            bit: 13,
        }],
    };
    for name in [
        "ethiopic_amharic_breaking_rules",
        "do_not_break_wrapped_tables",
        "do_not_snap_to_grid_in_cell",
        "do_not_allow_field_end_selection",
        "do_not_wrap_punctuation",
        "do_not_use_asian_break_rules",
        "do_not_use_html_auto_spacing",
    ] {
        s.compatibility[0].name = name;
        qualify_settings(&s).unwrap();
    }
    assert!(s.compatibility[0].enabled);
    s.compatibility[0].name = "wordperfect_justification";
    qualify_settings(&s).unwrap();
    let unapplied = unmodeled_compatibility_options(&s);
    assert_eq!(unapplied.len(), 1);
    assert_eq!(unapplied[0], &s.compatibility[0]);
    s.compatibility[0].enabled = false;
    qualify_settings(&s).unwrap();
    assert!(unmodeled_compatibility_options(&s).is_empty());
    s.compatibility[0].name = "unknown_source_option";
    assert!(qualify_settings(&s).is_err());
    s.compatibility[0].name = "wordperfect_justification";
    s.issues.push("malformed source settings");
    assert!(qualify_settings(&s).is_err());
    s.automatic_hyphenation = true;
    assert!(qualify_settings(&s).is_err());
}

#[test]
fn no_language_digits_require_complete_cached_result_ownership() {
    let field = "\u{13} PAGE \u{14}23\u{15}";
    let raw = format!("{field}\r");
    let mut p = paragraph(&raw);
    p.cached_fields
        .push(cached_fields::CachedHyperlink::from_source(field, 0, 33, 0x80).unwrap());
    let result = p.cached_fields[0].result_cp();
    let properties = p.runs[0].properties.clone();
    p.runs = [
        (0, result[0]),
        (result[0], result[1]),
        (result[1], raw.len()),
    ]
    .into_iter()
    .map(|(start, end)| PlainRun {
        source_cp: [start, end],
        text: raw[start..end].into(),
        properties: properties.clone(),
    })
    .collect();
    p.runs[1].properties.language = 0x0400;
    validate_partition(std::slice::from_ref(&p), [0, raw.len()], 1).unwrap();
    let mut missing = p.clone();
    missing.cached_fields.clear();
    assert!(validate_partition(&[missing], [0, raw.len()], 1).is_err());
    p.runs[1].text = "ab".into();
    assert!(validate_partition(&[p], [0, raw.len()], 1).is_err());
}

#[test]
fn east_asian_characters_qualify_the_far_east_source_face() {
    for c in ['漢', 'あ', 'カ', '한', '\u{20000}'] {
        assert!(qualify_character_charset(c, [2, 0, 2], 0).is_ok());
        assert!(qualify_character_charset(c, [0, 2, 0], 0).is_err());
    }
    assert!(qualify_character_charset('é', [0, 0, 2], 1).is_ok());
    assert!(qualify_character_charset('é', [0, 2, 0], 1).is_err());
}

#[test]
fn supplementary_source_range_keeps_following_control_at_utf16_position() {
    let units: Vec<_> = "\u{20000}\u{8}\r".encode_utf16().collect();
    qualify_source_range(&units, 100, &[], &[102].into(), &Default::default()).unwrap();
    assert!(qualify_source_range(&units, 100, &[], &[101].into(), &Default::default()).is_err());
    assert!(
        qualify_source_range(&units[1..], 101, &[], &[102].into(), &Default::default()).is_err()
    );
    assert!(
        qualify_source_range(&[1], 100, &[], &Default::default(), &Default::default()).is_err()
    );
}

#[test]
fn object_cell_grid_guard_keeps_ordinary_body_objects_outside_table_ranges() {
    let mut p = paragraph("A\u{1}\r");
    p.source_cp = [3, 6];
    p.runs[0].source_cp = p.source_cp;
    assert!(qualify_grid_cell_objects(vec![[0, 3]], std::slice::from_ref(&p)).is_ok());
    assert!(qualify_grid_cell_objects(vec![[3, 6]], std::slice::from_ref(&p)).is_err());
    p.runs[0].text = "A\u{8}\r".into();
    assert!(qualify_grid_cell_objects(vec![[3, 6]], std::slice::from_ref(&p)).is_err());
    p.runs[0].text = "AB\r".into();
    assert!(qualify_grid_cell_objects(vec![[3, 6]], &[p]).is_ok());
}

#[test]
fn greek_cyrillic_text_keeps_font_and_control_qualification() {
    let text = "Άλφα ΐΰ Ω ЖЃя΄\r";
    let mut p = paragraph(text);
    let end = text.encode_utf16().count();
    validate_partition(std::slice::from_ref(&p), [0, end], 1).unwrap();
    for c in text.chars().filter(|c| !c.is_ascii()) {
        assert_eq!(crate::office::doc::render_model::runs::font_slot(c, 0), 2);
        assert_eq!(crate::office::doc::render_model::runs::font_slot(c, 1), 1);
        assert!(qualify_character_charset(c, [0, 0, 2], 0).is_err());
        assert!(qualify_character_charset(c, [0, 2, 0], 1).is_err());
        qualify_character_charset(c, [2, 2, 161], 0).unwrap();
        qualify_character_charset(c, [2, 2, 204], 0).unwrap();
    }
    p.runs[0].properties.bidi = true;
    assert!(validate_partition(&[p], [0, end], 1).is_err());
    for c in [0x0387, 0x03a2, 0x0483, 0x0301, 0x200e, 0x202e] {
        assert!(!supported_character(c));
    }
}

#[test]
fn optional_hyphen_retains_raw_control_and_other_font_qualification() {
    let mut p = paragraph("AB\u{1f}AB\r");
    validate_partition(std::slice::from_ref(&p), [0, 6], 1).unwrap();
    assert_eq!(p.runs[0].text, "AB\u{1f}AB\r");
    qualify_character_charset('\u{1f}', [2, 0, 0], 0).unwrap();
    assert!(qualify_character_charset('\u{1f}', [0, 0, 2], 0).is_err());
    p.runs[0].properties.hidden = true;
    assert!(validate_partition(&[p], [0, 6], 1).is_err());
}
