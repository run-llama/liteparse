use super::*;
mod emboss;
mod grid_toggle;
mod inherited_bullet;
mod label_sources;
mod merged_cells;
mod nested_tables;
mod textbox;
use crate::office::doc::{
    field_inspection::StoryFields,
    formatting::CharacterSpan,
    formatting_inspection::PieceProperties,
    lists::{LevelOverride, ListDefinition, ListOverride},
    paragraph_formatting::ParagraphSpan,
    styles::{Style, StyleProperties},
    symbols::Font,
    transcription::transcribe,
    visibility::{RevisionProperties, RevisionSpan},
};

fn level(bullet: bool) -> ListLevel {
    ListLevel {
        start_at: 1,
        number_format: if bullet { 0x17 } else { 0 },
        flags: 0,
        placeholder_offsets: if bullet {
            [0; 9]
        } else {
            [1, 0, 0, 0, 0, 0, 0, 0, 0]
        },
        following_character: 0,
        saved_indent_twips: 0,
        restart_limit: 0,
        html_flags: 0,
        raw_papx: vec![],
        raw_chpx: if bullet {
            vec![0x4f, 0x4a, 0, 0, 0x51, 0x4a, 0, 0]
        } else {
            vec![]
        },
        number_text_utf16: if bullet { vec![0xf0b7] } else { vec![0, 46] },
    }
}
fn definitions() -> ListDefinitions {
    ListDefinitions {
        schema_version: 1,
        status: "source_list_definitions_only",
        lists: vec![
            ListDefinition {
                lsid: 7,
                template_code: 0,
                style_links: [0xfff; 9],
                flags: 1,
                html_flags: 0,
                levels: vec![level(false)],
            },
            ListDefinition {
                lsid: 8,
                template_code: 0,
                style_links: [0xfff; 9],
                flags: 1,
                html_flags: 0,
                levels: vec![level(true)],
            },
        ],
        overrides: [(7, 0), (8, 1), (7, 0)]
            .into_iter()
            .map(|(lsid, list_index)| ListOverride {
                lsid,
                list_index,
                field_type: 0,
                html_flags: 0,
                first_paragraph_cp: None,
                levels: vec![],
            })
            .collect(),
        appended_level_bytes: 0,
        unresolved: [""; 4],
    }
}
struct Fixture {
    source: Inspection,
    definitions: ListDefinitions,
    paragraphs: ParagraphInspection,
    styles: Stylesheet,
    formatting: FormattingInspection,
    revisions: RevisionEvidence,
    fonts: FontTable,
    font_defaults_preserved: Option<bool>,
    fields: FieldInspection,
}
impl Fixture {
    fn new(selections: &[u16]) -> Self {
        let text = "Item\r".repeat(selections.len());
        let len = text.len();
        let source = Inspection {
            schema_version: 1,
            status: "raw_stories_only",
            binary_version: 0xc1,
            trailing_container_bytes: 0,
            piece_count: 1,
            stories: vec![crate::office::doc::Story {
                kind: "body",
                start_cp: 0,
                utf16_units: len,
                text,
            }],
            trailing_utf16_units: vec![],
        };
        let fields = FieldInspection {
            schema_version: 1,
            status: "source_field_metadata_only",
            binary_version: 0xc1,
            stories: vec![StoryFields {
                status: "absent",
                kind: "body",
                start_cp: 0,
                utf16_units: len,
                table_present: false,
                table: Some(crate::office::doc::fields::decode(&source.stories[0], &[]).unwrap()),
                error: None,
            }],
            unresolved: [""; 3],
        };
        Self {
            source,
            definitions: definitions(),
            paragraphs: ParagraphInspection {
                schema_version: 1,
                status: "unresolved_paragraph_fragments",
                binary_version: 0xc1,
                excluded_nonstory_cp_range: None,
                spans: selections
                    .iter()
                    .enumerate()
                    .map(|(i, n)| ParagraphSpan {
                        start_cp: i * 5,
                        end_cp: (i + 1) * 5,
                        style_index: Some(0),
                        raw_modifiers: vec![0x0b, 0x46, *n as u8, (*n >> 8) as u8],
                    })
                    .collect(),
                unresolved: [""; 4],
            },
            styles: Stylesheet {
                schema_version: 1,
                status: "source_styles_only",
                std_base_bytes: 18,
                raw_header: vec![],
                styles: vec![Some(Style {
                    index: 0,
                    builtin_id: 0,
                    kind: 1,
                    base_style: None,
                    next_style: 0,
                    linked_style: None,
                    revision_marked: false,
                    name: "Normal".into(),
                    raw_flags: 0,
                    properties: vec![StyleProperties {
                        kind: "paragraph",
                        bytes: vec![0, 0],
                    }],
                    raw_revision: vec![],
                })],
                unresolved: [""; 3],
            },
            formatting: FormattingInspection {
                schema_version: 1,
                status: "unresolved_character_formatting",
                binary_version: 0xc1,
                authored_story_end_cp: len,
                trailing_nonstory_cp_range: None,
                pieces: vec![PieceProperties {
                    start_cp: 0,
                    end_cp: len,
                    file_offset: 0,
                    compressed: false,
                    raw_flags: 0,
                    raw_prm: 0,
                }],
                raw_property_groups: vec![],
                character_spans: vec![CharacterSpan {
                    start_cp: 0,
                    end_cp: len,
                    properties: vec![],
                }],
                unresolved: [""; 3],
            },
            revisions: RevisionEvidence {
                status: "test",
                basis: "test",
                hidden_text: "unresolved",
                spans: vec![RevisionSpan {
                    start_cp: 0,
                    end_cp: len,
                    source_span_index: 0,
                    piece_index: 0,
                    properties: RevisionProperties::default(),
                    evidence: vec![],
                    unresolved: vec![],
                }],
            },
            fonts: FontTable {
                status: "source_font_table",
                fonts: vec![Font {
                    index: 0,
                    name: "Symbol".into(),
                    alternate_name: None,
                    charset: 2,
                    family: 0,
                    weight: 400,
                    panose: vec![],
                    signature: vec![],
                }],
            },
            font_defaults_preserved: None,
            fields,
        }
    }
    fn context(&self) -> ListContext<'_> {
        ListContext {
            source: &self.source,
            definitions: &self.definitions,
            paragraphs: &self.paragraphs,
            styles: &self.styles,
            formatting: &self.formatting,
            revisions: &self.revisions,
            fonts: &self.fonts,
            font_defaults_preserved: self.font_defaults_preserved,
            fields: &self.fields,
        }
    }
    fn run(&self) -> Result<Transcription> {
        project_lists_with_font_source(&transcribe(&self.source), &self.context(), Some(false))
            .map(|r| r.transcription)
    }
}
fn partition(story: &TranscribedStory, end: usize) {
    let mut cp = 0;
    let mut output = 0;
    for s in &story.segments {
        assert_eq!(s.start_cp, cp);
        assert_eq!(s.output_start, output);
        if s.kind == "generated_list_label" {
            assert_eq!(s.end_cp, cp);
            assert!(s.output_end > output);
        } else {
            assert!(s.end_cp > cp);
            cp = s.end_cp;
        }
        output = s.output_end;
    }
    assert_eq!(cp, end);
    assert_eq!(output, story.text.len());
}

#[test]
fn interleaved_lists_and_distinct_overrides_share_source_list_sequence() {
    let f = Fixture::new(&[1, 2, 0, 3, 2, 1]);
    let out = f.run().unwrap();
    assert_eq!(
        out.stories[0].text,
        "1.\tItem\n•\tItem\nItem\n2.\tItem\n•\tItem\n3.\tItem\n"
    );
    partition(&out.stories[0], 30);
    let anchors: Vec<_> = out.stories[0]
        .segments
        .iter()
        .filter(|s| s.kind == "generated_list_label")
        .map(|s| s.start_cp)
        .collect();
    assert_eq!(anchors, [0, 5, 15, 20, 25]);
}

#[test]
fn unsupported_member_blocks_its_whole_sequence_but_preserves_other_lists() {
    let mut f = Fixture::new(&[1, 2, 3, 1]);
    f.definitions.overrides[2].levels.push(LevelOverride {
        level: 0,
        raw_flags: 16,
        raw_start_at: -1,
        start_override: Some(-1),
        format_override: None,
    });
    let out = f.run().unwrap();
    assert_eq!(out.stories[0].text, "Item\n•\tItem\nItem\nItem\n");
    assert!(
        out.stories[0]
            .diagnostics
            .iter()
            .any(|d| d.code == "list_sequence_unsupported")
    );
}

#[test]
fn skipped_paragraphs_and_negative_ilfo_preserve_numbering() {
    let mut f = Fixture::new(&[1, 1, 0xffff]);
    f.paragraphs.spans[1].raw_modifiers.extend([0x0a, 0x26, 12]);
    assert_eq!(
        f.run().unwrap().stories[0].text,
        "1.\tItem\nItem\n2.\tItem\n"
    );
}

#[test]
fn styles_apply_before_direct_selection_and_hidden_labels_are_blocked() {
    let mut f = Fixture::new(&[0, 1]);
    f.styles.styles[0].as_mut().unwrap().properties[0]
        .bytes
        .extend([0x0b, 0x46, 1, 0]);
    assert_eq!(f.run().unwrap().stories[0].text, "Item\n1.\tItem\n");
    f.styles.styles[0]
        .as_mut()
        .unwrap()
        .properties
        .push(StyleProperties {
            kind: "character",
            bytes: vec![0x3c, 0x08, 1],
        });
    assert_eq!(f.run().unwrap().stories[0].text, "Item\nItem\n");
}

#[test]
fn unknown_fonts_revisions_piece_properties_and_papx_remain_explicit() {
    let mut f = Fixture::new(&[1, 2]);
    f.fonts.fonts[0].name = "Unknown".into();
    assert_eq!(f.run().unwrap().stories[0].text, "1.\tItem\nItem\n");
    f.revisions.spans[0].properties.deleted = None;
    assert!(f.run().is_err());
    f.revisions.spans[0].properties.deleted = Some(false);
    f.formatting.pieces[0].raw_prm = 1;
    assert!(f.run().is_err());
    f.formatting.pieces[0].raw_prm = 0;
    f.paragraphs.spans[0]
        .raw_modifiers
        .extend([0x00, 0x46, 1, 0]);
    assert!(f.run().is_err());
}

#[test]
fn supplementary_characters_before_anchor_preserve_utf16_positions() {
    let f = Fixture::new(&[0, 1]);
    let mut input = transcribe(&f.source);
    input.stories[0].text = "😀\nItem\n".into();
    input.stories[0].segments = vec![
        SourceSegment {
            kind: "text",
            start_cp: 0,
            end_cp: 3,
            output_start: 0,
            output_end: 5,
        },
        SourceSegment {
            kind: "text",
            start_cp: 3,
            end_cp: 8,
            output_start: 5,
            output_end: 10,
        },
    ];
    let out = insert(
        &input.stories[0],
        &[Label {
            cp: 3,
            text: "1.\t".into(),
        }],
    )
    .unwrap();
    assert_eq!(out.text, "😀\n1.\tItem\n");
    partition(&out, 8);
}

#[test]
fn unsupported_templates_and_control_suffixes_are_not_invented() {
    let fonts = Fixture::new(&[1]).fonts;
    for mode in 0..3 {
        let mut lvl = level(false);
        match mode {
            0 => lvl.number_format = 3,
            1 => lvl.placeholder_offsets[0] = 2,
            _ => lvl.number_text_utf16.push(13),
        }
        assert!(label_text(&lvl, 1, &fonts, 0, LabelLanguage::default()).is_err());
    }
}

#[test]
fn inline_hidden_field_markers_do_not_hide_paragraph_list_labels() {
    let mut f = Fixture::new(&[1]);
    f.source.stories[0].text = "\u{13}X\u{15}A\r".into();
    let mut plc = Vec::new();
    for cp in [0u32, 2, 5] {
        plc.extend(cp.to_le_bytes());
    }
    plc.extend([0x13, 0, 0x15, 0]);
    f.fields.stories[0].table =
        Some(crate::office::doc::fields::decode(&f.source.stories[0], &plc).unwrap());
    f.revisions.spans[0].start_cp = 1;
    f.revisions.spans.insert(
        0,
        RevisionSpan {
            start_cp: 0,
            end_cp: 1,
            source_span_index: 0,
            piece_index: 0,
            properties: RevisionProperties {
                field_hidden: Some(true),
                special: Some(true),
                ..Default::default()
            },
            evidence: vec![],
            unresolved: vec![],
        },
    );
    let out = f.run().unwrap();
    assert_eq!(out.stories[0].text, "1.\tA\n");
    partition(&out.stories[0], 5);
    f.revisions.spans[1].properties.field_hidden = Some(true);
    assert_eq!(f.run().unwrap().stories[0].text, "A\n");
}

#[test]
fn bullet_uses_non_ascii_font_slot_and_rejects_unresolved_hint() {
    let mut f = Fixture::new(&[2]);
    f.fonts.fonts.push(Font {
        index: 1,
        name: "Unknown".into(),
        alternate_name: None,
        charset: 2,
        family: 0,
        weight: 400,
        panose: vec![],
        signature: vec![],
    });
    f.definitions.lists[1].levels[0].raw_chpx = vec![0x4f, 0x4a, 1, 0, 0x51, 0x4a, 0, 0];
    assert_eq!(f.run().unwrap().stories[0].text, "•\tItem\n");
    f.definitions.lists[1].levels[0].raw_chpx[6] = 1;
    assert_eq!(f.run().unwrap().stories[0].text, "Item\n");
    f.definitions.lists[1].levels[0].raw_chpx[6] = 0;
    f.definitions.lists[1].levels[0]
        .raw_chpx
        .extend([0x6f, 0x28, 1]);
    assert_eq!(f.run().unwrap().stories[0].text, "Item\n");
}

#[test]
fn output_expansion_and_templates_have_explicit_limits() {
    let mut f = Fixture::new(&vec![1; 20_000]);
    f.definitions.lists[0].levels[0].number_text_utf16 = std::iter::once(0)
        .chain(std::iter::repeat_n(46, 511))
        .collect();
    assert!(matches!(
        f.run(),
        Err(Error::Limit("list generated label bytes"))
    ));
    f.definitions.lists[0].levels[0].number_text_utf16.push(46);
    assert!(matches!(
        label_text(
            &f.definitions.lists[0].levels[0],
            1,
            &f.fonts,
            0,
            LabelLanguage::default()
        ),
        Err(Error::Unsupported("list long number template"))
    ));
}

#[test]
fn merged_text_segment_splits_at_utf16_anchor_without_losing_suffix() {
    let f = Fixture::new(&[0, 1]);
    let mut input = transcribe(&f.source).stories.remove(0);
    input.text = "😀\nItem\n".into();
    input.segments = vec![SourceSegment {
        kind: "text",
        start_cp: 0,
        end_cp: 8,
        output_start: 0,
        output_end: 10,
    }];
    let result = insert(
        &input,
        &[Label {
            cp: 3,
            text: "1.\t".into(),
        }],
    )
    .unwrap();
    assert_eq!(result.text, "😀\n1.\tItem\n");
    partition(&result, 8);
}

fn multilevel(levels: &[u8]) -> Fixture {
    let mut f = Fixture::new(&vec![1; levels.len()]);
    f.definitions.lists[0].flags = 0;
    f.definitions.lists[0].levels = (0..3)
        .map(|i| {
            let mut l = level(false);
            l.number_text_utf16 = vec![40, i, 41];
            l.placeholder_offsets[0] = 2;
            if i == 2 {
                l.number_format = 2;
            }
            l
        })
        .collect();
    for (p, i) in f.paragraphs.spans.iter_mut().zip(levels) {
        p.raw_modifiers.extend([0x0a, 0x26, *i]);
    }
    f
}
#[test]
fn roman_current_level_templates_and_more_significant_restarts() {
    let f = multilevel(&[2, 2, 1, 2, 2, 0, 2]);
    let out = f.run().unwrap();
    assert_eq!(
        out.stories[0].text,
        "(i)\tItem\n(ii)\tItem\n(1)\tItem\n(i)\tItem\n(ii)\tItem\n(1)\tItem\n(i)\tItem\n"
    );
    partition(&out.stories[0], 35);
}
#[test]
fn custom_restart_threshold_preserves_sequence_after_less_significant_parent() {
    let mut f = multilevel(&[2, 2, 1, 2, 0, 2]);
    f.definitions.lists[0].levels[2].flags |= 8;
    f.definitions.lists[0].levels[2].restart_limit = 1;
    let out = f.run().unwrap();
    assert_eq!(
        out.stories[0].text,
        "(i)\tItem\n(ii)\tItem\n(1)\tItem\n(iii)\tItem\n(1)\tItem\n(i)\tItem\n"
    );
}
#[test]
fn explicit_start_value_and_roman_bounds() {
    let mut f = multilevel(&[0, 0]);
    f.definitions.lists[0].levels[0].start_at = 3;
    assert_eq!(f.run().unwrap().stories[0].text, "(3)\tItem\n(4)\tItem\n");
    assert_eq!(roman(1994).unwrap(), "MCMXCIV");
    assert_eq!(roman(3999).unwrap(), "MMMCMXCIX");
    assert!(roman(0).is_err());
    assert!(roman(4000).is_err());
}
#[test]
fn ancestor_template_and_invalid_restart_are_explicit() {
    let mut f = multilevel(&[2, 2]);
    f.definitions.lists[0].levels[2].number_text_utf16 = vec![40, 0, 41];
    let out = f.run().unwrap();
    assert_eq!(out.stories[0].text, "Item\nItem\n");
    assert_eq!(
        out.stories[0].diagnostics[0].code,
        "list_ancestor_number_unresolved"
    );
    f.definitions.lists[0].levels[2].flags |= 8;
    f.definitions.lists[0].levels[2].restart_limit = 3;
    assert!(f.run().is_err());
}

fn table_fixture() -> Fixture {
    let mut f = Fixture::new(&[0, 1, 1]);
    f.source.stories[0].text = "abc\u{7}\u{7}Item\rItem\r".into();
    f.styles.styles.resize_with(12, || None);
    f.styles.styles[11] = Some(Style {
        index: 11,
        builtin_id: 105,
        kind: 3,
        base_style: None,
        next_style: 11,
        linked_style: None,
        revision_marked: false,
        name: "Table Normal".into(),
        raw_flags: 0,
        properties: vec![],
        raw_revision: vec![],
    });
    f.paragraphs.spans = [
        (0, 4, 0, false),
        (4, 5, 0, true),
        (5, 10, 1, false),
        (10, 15, 1, false),
    ]
    .into_iter()
    .map(|(start, end, ilfo, row)| {
        let mut raw = vec![0x0b, 0x46, ilfo, 0];
        if end <= 5 {
            raw.extend([
                0x16,
                0x24,
                1,
                0x49,
                0x66,
                1,
                0,
                0,
                0,
                0x17,
                0x24,
                u8::from(row),
            ]);
        }
        ParagraphSpan {
            start_cp: start,
            end_cp: end,
            style_index: Some(0),
            raw_modifiers: raw,
        }
    })
    .collect();
    f
}
#[test]
fn complete_nonlist_table_rows_allow_independent_outside_lists() {
    let f = table_fixture();
    let out = f.run().unwrap();
    let story = &out.stories[0];
    let labels: Vec<_> = story
        .segments
        .iter()
        .filter(|s| s.kind == "generated_list_label")
        .map(|s| (s.start_cp, &story.text[s.output_start..s.output_end]))
        .collect();
    assert_eq!(labels, [(5, "1.\t"), (10, "2.\t")]);
    partition(story, 15);
}
#[test]
fn table_list_member_blocks_shared_sequence_and_unknown_styles_are_explicit() {
    let mut f = table_fixture();
    f.paragraphs.spans[0].raw_modifiers[2] = 1;
    assert!(
        !f.run().unwrap().stories[0]
            .segments
            .iter()
            .any(|s| s.kind == "generated_list_label")
    );
    f.paragraphs.spans[0].raw_modifiers[2] = 0;
    f.paragraphs.spans[1]
        .raw_modifiers
        .extend([0x3a, 0x56, 12, 0]);
    assert!(f.run().is_err());
}
#[test]
fn incomplete_rows_and_unresolved_default_table_numbering_reject_generation() {
    let mut f = table_fixture();
    f.paragraphs.spans[1].raw_modifiers.pop();
    f.paragraphs.spans[1].raw_modifiers.push(0);
    assert!(f.run().is_err());
    let mut f = table_fixture();
    f.styles.styles[11]
        .as_mut()
        .unwrap()
        .properties
        .push(StyleProperties {
            kind: "paragraph",
            bytes: vec![11, 0, 0x0b, 0x46, 1, 0],
        });
    assert!(f.run().is_err());
}

#[test]
fn paragraph_layout_properties_preserve_numbering_and_source_text() {
    let mut f = Fixture::new(&[1, 1]);
    let layout = [
        0x31, 0x24, 1, 0x2a, 0x24, 0, 0x6d, 0x24, 1, 0x50, 0xc6, 8, 0, 0, 0, 0, 0, 0, 0, 0, 0x18,
        0x84, 0, 0, 0x19, 0x84, 0, 0, 0x1a, 0x84, 0, 0, 0x1b, 0x26, 0, 0x2b, 0x44, 0, 0, 0x2d,
        0x44, 0, 0, 0x55, 0x44, 0, 0, 0x58, 0x44, 0, 0,
    ];
    for p in &mut f.paragraphs.spans {
        p.raw_modifiers.extend(layout);
    }
    let out = f.run().unwrap();
    assert_eq!(out.stories[0].text, "1.\tItem\n2.\tItem\n");
    partition(&out.stories[0], 10);
}
#[test]
fn layout_support_preserves_revision_unknown_and_hidden_guards() {
    for op in [0xc645u16, 0xc66f, 0x4600, 0xc63e] {
        let mut f = Fixture::new(&[1]);
        f.paragraphs.spans[0].raw_modifiers.extend([0x31, 0x24, 1]);
        f.paragraphs.spans[0].raw_modifiers.extend(op.to_le_bytes());
        f.paragraphs.spans[0]
            .raw_modifiers
            .extend(if op >> 13 == 6 { vec![0] } else { vec![0, 0] });
        assert!(f.run().is_err());
    }
    let mut f = Fixture::new(&[1]);
    f.paragraphs.spans[0].raw_modifiers.extend([0x31, 0x24, 2]);
    assert!(f.run().is_err());
    let mut f = Fixture::new(&[1]);
    f.styles.styles[0]
        .as_mut()
        .unwrap()
        .properties
        .push(StyleProperties {
            kind: "character",
            bytes: vec![0x3c, 0x08, 1],
        });
    f.paragraphs.spans[0].raw_modifiers.extend([0x31, 0x24, 1]);
    assert_eq!(f.run().unwrap().stories[0].text, "Item\n");
}

#[test]
fn legal_number_override_is_explicit_until_its_formatting_is_resolved() {
    let mut f = multilevel(&[2, 2]);
    f.definitions.lists[0].levels[2].flags |= 4;
    let out = f.run().unwrap();
    assert_eq!(out.stories[0].text, "Item\nItem\n");
    assert_eq!(
        out.stories[0].diagnostics[0].code,
        "list_sequence_unsupported"
    );
}

#[test]
fn table_presentation_preserves_outside_sequence_and_source_ownership() {
    let mut f = table_fixture();
    let row = &mut f.paragraphs.spans[1].raw_modifiers;
    row.extend([0x02, 0x96, 108, 0, 0x0a, 0x74, 0, 0, 0xe0, 1]);
    row.extend([0x79, 0x74, 1, 2, 3, 4, 0x8a, 0x54, 1, 0]);
    row.extend([0x03, 0x34, 1, 0x04, 0x34, 0, 0x15, 0x36, 1, 0x66, 0x34, 1]);
    row.extend([0x18, 0xf6, 3, 0, 0]);
    row.extend([0x13, 0xd6, 48]);
    row.extend([0; 48]);
    row.extend([0x12, 0xd6, 10]);
    row.extend([0; 10]);
    row.extend([0x2c, 0xd6, 3, 0, 1, 1]);
    row.extend([0x2f, 0xd6, 11, 0, 1, 15]);
    row.extend([0; 8]);
    row.extend([0x32, 0xd6, 6, 0, 1, 15, 3, 108, 0]);
    row.extend([0x39, 0xd6, 3, 0, 1, 1, 0x42, 0xd6, 3, 0, 1, 1]);
    row.extend([0x70, 0xd6, 10]);
    row.extend([0; 10]);
    let out = f.run().unwrap();
    assert!(out.stories[0].text.ends_with("1.\tItem\n2.\tItem\n"));
    partition(&out.stories[0], 15);
    f.paragraphs.spans[0].raw_modifiers[2] = 1;
    assert!(
        !f.run().unwrap().stories[0]
            .segments
            .iter()
            .any(|s| s.kind == "generated_list_label")
    );
}

#[test]
fn table_options_keep_conditional_visibility_and_revision_guards() {
    for (kind, bytes) in [
        ("table", vec![0x6a, 0xd6, 0]),
        ("character", vec![0x3c, 0x08, 1]),
        ("paragraph", vec![11, 0, 0x0b, 0x46, 1, 0]),
    ] {
        let mut f = table_fixture();
        f.paragraphs.spans[1]
            .raw_modifiers
            .extend([0x0a, 0x74, 0, 0, 0xe0, 1]);
        f.styles.styles[11]
            .as_mut()
            .unwrap()
            .properties
            .push(StyleProperties { kind, bytes });
        assert!(f.run().is_err());
    }
    for bytes in [
        vec![0x68, 0x36, 1],
        vec![0x67, 0xd6, 0],
        vec![0x15, 0x36, 2],
        vec![0x02, 0x96, 255, 255],
        vec![0x8a, 0x54, 3, 0],
    ] {
        let mut f = table_fixture();
        f.paragraphs.spans[1].raw_modifiers.extend(bytes);
        assert!(f.run().is_err());
    }
    let mut f = Fixture::new(&[1]);
    f.paragraphs.spans[0]
        .raw_modifiers
        .extend([0x0a, 0x74, 0, 0, 0xe0, 1]);
    assert!(f.run().is_err());
}

#[test]
fn outside_numbering_continues_across_formatted_nonlist_table() {
    let mut f = table_fixture();
    f.source.stories[0].text = "Item\rabc\u{7}\u{7}Item\r".into();
    let cell = f.paragraphs.spans[0].raw_modifiers.clone();
    let mut row = f.paragraphs.spans[1].raw_modifiers.clone();
    row.extend([0x0a, 0x74, 0, 0, 0xa0, 4, 0x02, 0x96, 108, 0]);
    let list = f.paragraphs.spans[2].raw_modifiers.clone();
    f.paragraphs.spans = [
        (0, 5, list.clone()),
        (5, 9, cell),
        (9, 10, row),
        (10, 15, list),
    ]
    .into_iter()
    .map(|(start_cp, end_cp, raw_modifiers)| ParagraphSpan {
        start_cp,
        end_cp,
        style_index: Some(0),
        raw_modifiers,
    })
    .collect();
    let out = f.run().unwrap();
    let story = &out.stories[0];
    let labels: Vec<_> = story
        .segments
        .iter()
        .filter(|s| s.kind == "generated_list_label")
        .map(|s| (s.start_cp, &story.text[s.output_start..s.output_end]))
        .collect();
    assert_eq!(labels, [(0, "1.\t"), (10, "2.\t")]);
    partition(story, 15);
}

#[test]
fn legacy_tables_without_style_definitions_preserve_outside_numbering() {
    for version in [0x00c1, 0x00d9] {
        let mut f = table_fixture();
        f.source.binary_version = version;
        f.paragraphs.binary_version = version;
        f.formatting.binary_version = version;
        f.fields.binary_version = version;
        f.styles.styles[11] = None;
        f.fields.binary_version = 0x0101;
        assert!(f.run().is_err());
        f.fields.binary_version = version;
        let out = f.run().unwrap();
        assert!(out.stories[0].text.ends_with("1.\tItem\n2.\tItem\n"));
        partition(&out.stories[0], 15);
        f.styles.styles.truncate(1);
        assert_eq!(f.run().unwrap().stories[0].text, out.stories[0].text);
    }
}

#[test]
fn missing_table_style_guard_rejects_modern_and_explicit_styles() {
    let mut f = table_fixture();
    f.styles.styles[11] = None;
    for version in [0x0101, 0x010c, 0x0112] {
        f.source.binary_version = version;
        f.paragraphs.binary_version = version;
        f.formatting.binary_version = version;
        f.fields.binary_version = version;
        assert!(f.run().is_err());
    }
    let mut f = table_fixture();
    assert_eq!(f.source.binary_version, 0x00c1);
    let table_style = f.styles.styles[11].take();
    f.styles.styles.push(table_style);
    assert!(f.run().is_err());
    f.styles.styles.pop();
    f.paragraphs.spans[1]
        .raw_modifiers
        .extend([0x3a, 0x56, 11, 0]);
    assert!(f.run().is_err());
}

#[test]
fn legacy_malformed_default_slot_and_table_topology_remain_rejected() {
    let mut f = table_fixture();
    assert_eq!(f.source.binary_version, 0x00c1);
    f.styles.styles[11].as_mut().unwrap().kind = 1;
    assert!(f.run().is_err());
    f.styles.styles[11] = None;
    f.paragraphs.spans[1].raw_modifiers.pop();
    f.paragraphs.spans[1].raw_modifiers.push(0);
    assert!(f.run().is_err());
}

fn legacy_table_fixture() -> Fixture {
    let mut f = table_fixture();
    f.styles.styles[11] = None;
    for p in &mut f.paragraphs.spans[..2] {
        p.raw_modifiers.drain(7..13);
    }
    f.paragraphs.spans[1]
        .raw_modifiers
        .extend([0x08, 0xd6, 6, 0, 1, 0, 0, 100, 0]);
    f
}

#[test]
fn word97_implicit_depth_requires_matching_source_cell_definition() {
    let f = legacy_table_fixture();
    assert_eq!(f.source.binary_version, 0x00c1);
    let out = f.run().unwrap();
    assert!(out.stories[0].text.ends_with("1.\tItem\n2.\tItem\n"));
    partition(&out.stories[0], 15);
    let mut missing = legacy_table_fixture();
    let n = missing.paragraphs.spans[1].raw_modifiers.len();
    missing.paragraphs.spans[1].raw_modifiers.truncate(n - 9);
    assert!(missing.run().is_err());
    let mut explicit_zero = legacy_table_fixture();
    explicit_zero.paragraphs.spans[0]
        .raw_modifiers
        .extend([0x49, 0x66, 0, 0, 0, 0]);
    assert!(explicit_zero.run().is_err());
    let mut nested = legacy_table_fixture();
    nested.paragraphs.spans[0]
        .raw_modifiers
        .extend([0x4b, 0x24, 1]);
    assert!(nested.run().is_err());
    let mut modern = legacy_table_fixture();
    modern.source.binary_version = 0xd9;
    modern.fields.binary_version = 0xd9;
    modern.formatting.binary_version = 0xd9;
    modern.paragraphs.binary_version = 0xd9;
    assert!(modern.run().is_err());
}

#[test]
fn legacy_cell_definitions_validate_counts_edges_records_and_merges() {
    assert_eq!(table_columns(&[6, 0, 1, 0, 0, 100, 0]).unwrap(), (1, 0, 0));
    for data in [
        vec![6, 0, 0, 0, 0, 100, 0],
        vec![6, 0, 64, 0, 0, 100, 0],
        vec![6, 0, 1, 100, 0, 0, 0],
        vec![7, 0, 1, 0, 0, 100, 0, 0],
    ] {
        assert!(table_columns(&data).is_err());
    }
    for flags in [1u16, 2, 3, 0x20, 0x40, 0x60] {
        let mut data = vec![26, 0, 1, 0, 0, 100, 0];
        data.extend(flags.to_le_bytes());
        data.extend([0; 18]);
        if flags == 0x40 {
            assert!(table_columns(&data).is_err());
        } else {
            let (_, h, v) = table_columns(&data).unwrap();
            assert_ne!(h | v, 0);
        }
    }
    let mut f = legacy_table_fixture();
    let n = f.paragraphs.spans[1].raw_modifiers.len();
    f.paragraphs.spans[1].raw_modifiers.truncate(n - 9);
    f.paragraphs.spans[1]
        .raw_modifiers
        .extend([0x08, 0xd6, 8, 0, 2, 0, 0, 50, 0, 100, 0]);
    assert!(f.run().is_err());
}

#[test]
fn legacy_table_members_still_block_their_list_sequence() {
    let mut f = legacy_table_fixture();
    f.paragraphs.spans[0].raw_modifiers[2] = 1;
    assert!(
        !f.run().unwrap().stories[0]
            .segments
            .iter()
            .any(|s| s.kind == "generated_list_label")
    );
}

#[test]
fn legacy_optional_cell_records_and_layout_flags_follow_source_defaults() {
    fn operand(columns: u8, flags: &[u16]) -> Vec<u8> {
        let mut data = vec![0, 0, columns];
        for edge in 0..=u16::from(columns) {
            data.extend((edge * 100).to_le_bytes());
        }
        for flag in flags {
            data.extend(flag.to_le_bytes());
            data.extend([0; 18]);
        }
        let cb = u16::try_from(data.len() - 1).unwrap();
        data[..2].copy_from_slice(&cb.to_le_bytes());
        data
    }
    assert_eq!(table_columns(&operand(2, &[])).unwrap(), (2, 0, 0));
    assert_eq!(table_columns(&operand(2, &[0])).unwrap(), (2, 0, 0));
    assert_eq!(table_columns(&operand(1, &[0, 0xffff])).unwrap(), (1, 0, 0));
    for flags in [
        0x8000, 0x7000, 0x0200, 0x0400, 0x0600, 0x0080, 0x0100, 4, 12, 16, 20,
    ] {
        assert!(table_columns(&operand(1, &[flags])).is_ok());
    }
    for flags in [0x0800, 0x0180, 8, 24, 28] {
        assert!(table_columns(&operand(1, &[flags])).is_err());
    }
    let mut negative_width = operand(1, &[0]);
    negative_width[9..11].copy_from_slice(&(-1i16).to_le_bytes());
    assert!(table_columns(&negative_width).is_err());
}

fn alphabet_fixture() -> Fixture {
    let mut f = Fixture::new(&[1, 1, 1, 1]);
    f.definitions.lists[0].levels[0].number_format = 3;
    f.definitions.lists[0].levels[0].start_at = 25;
    f.styles.styles[0]
        .as_mut()
        .unwrap()
        .properties
        .push(StyleProperties {
            kind: "character",
            bytes: vec![0x6d, 0x48, 9, 4],
        });
    f
}

#[test]
fn alphabetic_repeated_letters_have_bounded_non_excel_rollover() {
    for (value, expected) in [
        (1, "A"),
        (25, "Y"),
        (26, "Z"),
        (27, "AA"),
        (28, "BB"),
        (52, "ZZ"),
        (53, "AAA"),
    ] {
        assert_eq!(alphabetic(value, true).unwrap(), expected);
        assert_eq!(
            alphabetic(value, false).unwrap(),
            expected.to_ascii_lowercase()
        );
    }
    for value in [i32::MIN, -1, 0, 26 * 512 + 1, i32::MAX] {
        assert!(alphabetic(value, false).is_err());
    }
    assert_eq!(alphabetic(26 * 512, false).unwrap(), "z".repeat(512));
    let f = alphabet_fixture();
    let out = f.run().unwrap();
    assert_eq!(
        out.stories[0].text,
        "Y.\tItem\nZ.\tItem\nAA.\tItem\nBB.\tItem\n"
    );
    partition(&out.stories[0], 20);
}

#[test]
fn alphabet_language_overrides_follow_style_mark_and_level_order() {
    let mut f = alphabet_fixture();
    f.formatting.character_spans[0]
        .properties
        .push(PropertyModifier {
            sprm: 0x4873,
            operand: vec![6, 4],
        });
    assert_eq!(f.run().unwrap().stories[0].text, "Item\n".repeat(4));
    f.definitions.lists[0].levels[0]
        .raw_chpx
        .extend([0x73, 0x48, 9, 8]);
    assert!(f.run().unwrap().stories[0].text.starts_with("Y.\t"));
    f.definitions.lists[0].levels[0].number_format = 4;
    assert!(f.run().unwrap().stories[0].text.starts_with("y.\t"));
    f.definitions.lists[0].levels[0]
        .raw_chpx
        .extend([0x73, 0x48, 6, 4]);
    assert_eq!(
        f.run().unwrap().stories[0].diagnostics[0].code,
        "list_alphabet_language_unresolved"
    );
}

#[test]
fn unknown_alphabet_language_blocks_entire_sequence_and_preserves_guards() {
    let mut f = alphabet_fixture();
    f.styles.styles[0].as_mut().unwrap().properties.pop();
    assert_eq!(
        f.run().unwrap().stories[0].diagnostics[0].code,
        "list_alphabet_language_unresolved"
    );
    f.formatting.character_spans[0]
        .properties
        .push(PropertyModifier {
            sprm: 0x486d,
            operand: vec![9, 4],
        });
    assert!(f.run().unwrap().stories[0].text.starts_with("Y.\t"));
    f.formatting.character_spans[0]
        .properties
        .push(PropertyModifier {
            sprm: 0x286f,
            operand: vec![1],
        });
    assert_eq!(f.run().unwrap().stories[0].text, "Item\n".repeat(4));
    let mut f = alphabet_fixture();
    f.definitions.lists[0].levels[0]
        .raw_chpx
        .extend([0x73, 0x48, 9, 0xfc]);
    assert_eq!(f.run().unwrap().stories[0].text, "Item\n".repeat(4));
    let mut f = alphabet_fixture();
    f.definitions.lists[0].levels[0].flags |= 4;
    assert_eq!(f.run().unwrap().stories[0].text, "Item\n".repeat(4));
}

#[test]
fn assigned_english_language_variants_share_alphabet_and_reserved_ids_do_not() {
    for lid in [
        0x0009u16, 0x0409, 0x0809, 0x0c09, 0x1009, 0x1409, 0x1809, 0x1c09, 0x2009, 0x2409, 0x2809,
        0x2c09, 0x3009, 0x3409, 0x3c09, 0x4009, 0x4409, 0x4809, 0x4c09,
    ] {
        let mut f = alphabet_fixture();
        f.definitions.lists[0].levels[0]
            .raw_chpx
            .extend([0x73, 0x48]);
        f.definitions.lists[0].levels[0]
            .raw_chpx
            .extend(lid.to_le_bytes());
        assert!(f.run().unwrap().stories[0].text.starts_with("Y.\t"));
    }
    for lid in [
        0x3809, 0x5009, 0x5409, 0x5809, 0x5c09, 0x6009, 0x6409, 0xfc09, 0x1000, 0x0000,
    ] {
        assert!(!english_language(lid));
    }
}

#[test]
fn one_unresolved_language_member_blocks_the_shared_alphabetic_sequence() {
    let mut f = alphabet_fixture();
    f.formatting.character_spans[0].end_cp = 15;
    let mut last = f.formatting.character_spans[0].clone();
    last.start_cp = 15;
    last.end_cp = 20;
    last.properties.push(PropertyModifier {
        sprm: 0x4873,
        operand: vec![6, 4],
    });
    f.formatting.character_spans.push(last);
    assert_eq!(f.run().unwrap().stories[0].text, "Item\n".repeat(4));
}

#[test]
fn alphabet_language_is_inherited_then_overridden_by_child_paragraph_style() {
    let mut f = alphabet_fixture();
    let child = Style {
        index: 1,
        builtin_id: 0,
        kind: 1,
        base_style: Some(0),
        next_style: 0,
        linked_style: None,
        revision_marked: false,
        name: "Child".into(),
        raw_flags: 0,
        properties: vec![],
        raw_revision: vec![],
    };
    f.styles.styles.push(Some(child));
    for p in &mut f.paragraphs.spans {
        p.style_index = Some(1);
    }
    assert!(f.run().unwrap().stories[0].text.starts_with("Y.\t"));
    f.styles.styles[1]
        .as_mut()
        .unwrap()
        .properties
        .push(StyleProperties {
            kind: "character",
            bytes: vec![0x73, 0x48, 6, 4],
        });
    assert_eq!(f.run().unwrap().stories[0].text, "Item\n".repeat(4));
}

#[test]
fn table_border_colors_preserve_outside_labels_with_exact_cell_cardinality() {
    let mut f = legacy_table_fixture();
    for op in 0xd61au16..=0xd61d {
        f.paragraphs.spans[1].raw_modifiers.extend(op.to_le_bytes());
        f.paragraphs.spans[1]
            .raw_modifiers
            .extend([4, 0xff, 0xff, 0xff, 0xff]);
    }
    let out = f.run().unwrap();
    assert!(out.stories[0].text.ends_with("1.\tItem\n2.\tItem\n"));
    partition(&out.stories[0], 15);
    f.paragraphs.spans[1]
        .raw_modifiers
        .extend([0x1a, 0xd6, 8, 0, 0, 0, 0, 0, 0, 0, 0]);
    assert!(matches!(
        f.run(),
        Err(Error::Invalid("list border color cell count"))
    ));
}

#[test]
fn table_border_colors_reject_bad_framing_and_nonrow_scope() {
    for operand in [
        vec![0],
        vec![3, 0, 0, 0],
        vec![4, 0, 0, 0],
        vec![8, 0, 0, 0, 0],
    ] {
        let mut f = legacy_table_fixture();
        f.paragraphs.spans[1].raw_modifiers.extend([0x1a, 0xd6]);
        f.paragraphs.spans[1].raw_modifiers.extend(operand);
        assert!(f.run().is_err());
    }
    for index in [0, 2] {
        let mut f = legacy_table_fixture();
        f.paragraphs.spans[index]
            .raw_modifiers
            .extend([0x1a, 0xd6, 4, 0, 0, 0, 0]);
        assert!(f.run().is_err());
    }
}

#[test]
fn modern_explicit_depth_one_definitions_preserve_outside_labels() {
    for version in [0x00d9, 0x0101, 0x010c, 0x0112] {
        let fixture = || {
            let mut f = table_fixture();
            f.source.binary_version = version;
            f.fields.binary_version = version;
            f.formatting.binary_version = version;
            f.paragraphs.binary_version = version;
            f.paragraphs.spans[1]
                .raw_modifiers
                .extend([0x08, 0xd6, 6, 0, 1, 0, 0, 100, 0]);
            f
        };
        let f = fixture();
        let out = f.run().unwrap();
        assert!(out.stories[0].text.ends_with("1.\tItem\n2.\tItem\n"));
        partition(&out.stories[0], 15);
        for depth in [0u32, 2] {
            let mut bad = fixture();
            bad.paragraphs.spans[1].raw_modifiers.extend([0x49, 0x66]);
            bad.paragraphs.spans[1]
                .raw_modifiers
                .extend(depth.to_le_bytes());
            assert!(bad.run().is_err());
        }
        let mut no_depth = fixture();
        no_depth.paragraphs.spans[1].raw_modifiers.drain(7..13);
        assert!(no_depth.run().is_err());
        let mut style = fixture();
        style.paragraphs.spans[1]
            .raw_modifiers
            .extend([0x3a, 0x56, 12, 0]);
        assert!(style.run().is_err());
        let mut malformed = fixture();
        malformed.paragraphs.spans[1].raw_modifiers.pop();
        assert!(malformed.run().is_err());
        let mut merged = fixture();
        let b = &mut merged.paragraphs.spans[1].raw_modifiers;
        let start = b.len() - 9;
        b[start + 2] = 26;
        b.extend([1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        let merged_out = merged.run().unwrap();
        assert!(
            merged_out
                .unresolved
                .contains(&"merged_cell_visible_ownership_unresolved")
        );
        assert_eq!(merged_out.stories[0].text, out.stories[0].text);
    }
}

#[test]
fn table_position_operands_validate_bounds_and_ignored_padding() {
    for op in [0x940eu16, 0x940f] {
        for value in [-31_679i16, -20, -16, -12, -8, -4, 0, 1, 31_681] {
            let mut data = op.to_le_bytes().to_vec();
            data.extend(value.to_le_bytes());
            let mut selection = Selection::default();
            apply_paragraph(&mut selection, &data).unwrap();
            assert!(selection.table_placement);
        }
        for value in [i16::MIN, -31_680, 31_682, i16::MAX] {
            let mut data = op.to_le_bytes().to_vec();
            data.extend(value.to_le_bytes());
            assert!(apply_paragraph(&mut Selection::default(), &data).is_err());
        }
    }
    for op in [0x9410u16, 0x9411, 0x941e, 0x941f] {
        for value in [0u16, 31_680, 31_681, u16::MAX] {
            let mut data = op.to_le_bytes().to_vec();
            data.extend(value.to_le_bytes());
            assert_eq!(
                apply_paragraph(&mut Selection::default(), &data).is_ok(),
                value <= 31_680
            );
        }
    }
    for anchor in 0u8..=255 {
        assert!(apply_paragraph(&mut Selection::default(), &[0x0d, 0x36, anchor]).is_ok());
    }
    for data in [vec![0x0d, 0x36], vec![0x0e, 0x94, 0], vec![0x10, 0x94, 0]] {
        assert!(apply_paragraph(&mut Selection::default(), &data).is_err());
    }
}

#[test]
fn table_position_preserves_membership_order_and_explicit_layout_limit() {
    let placement = [
        0x0d, 0x36, 0x9f, 0x0e, 0x94, 0xfc, 0xff, 0x0f, 0x94, 0, 0, 0x10, 0x94, 0, 0,
    ];
    for before in [true, false] {
        let mut f = table_fixture();
        let row = &mut f.paragraphs.spans[1].raw_modifiers;
        if before {
            row.splice(0..0, placement);
        } else {
            row.extend(placement);
        }
        let out = f.run().unwrap();
        assert!(out.stories[0].text.ends_with("1.\tItem\n2.\tItem\n"));
        assert!(
            out.unresolved
                .contains(&"table_visual_placement_unresolved")
        );
        assert!(
            out.stories[0]
                .diagnostics
                .iter()
                .any(|d| d.code == "table_visual_placement_unresolved")
        );
        partition(&out.stories[0], 15);
        f.paragraphs.spans[1].raw_modifiers.extend([0x16, 0x24, 0]);
        assert!(f.run().is_err());
    }
    let mut outside = Fixture::new(&[1]);
    outside.paragraphs.spans[0].raw_modifiers.extend(placement);
    assert!(outside.run().is_err());
    let mut member = table_fixture();
    member.paragraphs.spans[0].raw_modifiers.extend(placement);
    member.paragraphs.spans[0]
        .raw_modifiers
        .extend([0x0b, 0x46, 1, 0]);
    assert!(
        !member.run().unwrap().stories[0]
            .segments
            .iter()
            .any(|s| s.kind == "generated_list_label")
    );
}

#[test]
fn ordered_insertions_and_vertical_merges_validate_physical_indices() {
    let mut s = Selection::default();
    apply_paragraph(&mut s, &[0x21, 0x76, 0, 2, 100, 0, 0x2b, 0xd6, 2, 1, 3]).unwrap();
    assert_eq!((s.table_columns, s.vertical_merges), (Some(2), 2));
    apply_paragraph(&mut s, &[0x21, 0x76, 1, 1, 100, 0]).unwrap();
    assert_eq!((s.table_columns, s.vertical_merges), (Some(3), 4));
    apply_paragraph(&mut s, &[0x2b, 0xd6, 2, 2, 0]).unwrap();
    assert_eq!(s.vertical_merges, 0);
    apply_paragraph(&mut s, &[0x2b, 0xd6, 2, 0, 1]).unwrap();
    apply_paragraph(&mut s, &[0x08, 0xd6, 6, 0, 1, 0, 0, 100, 0]).unwrap();
    assert_eq!((s.table_columns, s.vertical_merges), (Some(1), 0));
    for bytes in [
        vec![0x2b, 0xd6, 2, 0, 1],
        vec![0x21, 0x76, 1, 1, 0, 0],
        vec![0x21, 0x76, 0, 0, 0, 0],
        vec![0x21, 0x76, 0, 64, 0, 0],
        vec![0x21, 0x76, 0, 1, 255, 255],
        vec![0x21, 0x76, 0, 1, 0, 0, 0x2b, 0xd6, 2, 1, 3],
        vec![0x21, 0x76, 0, 1, 0, 0, 0x2b, 0xd6, 2, 0, 2],
        vec![0x21, 0x76, 0, 1, 0, 0, 0x2b, 0xd6, 1, 0],
        vec![0x21, 0x76, 0, 1, 0, 0, 0x2b, 0xd6, 2, 0],
    ] {
        assert!(apply_paragraph(&mut Selection::default(), &bytes).is_err());
    }
    let mut full = Selection::default();
    apply_paragraph(&mut full, &[0x21, 0x76, 0, 63, 0, 0, 0x2b, 0xd6, 2, 62, 1]).unwrap();
    assert_eq!(full.vertical_merges, 1u64 << 62);
    assert!(apply_paragraph(&mut full, &[0x21, 0x76, 63, 1, 0, 0]).is_err());
}

#[test]
fn merged_table_preserves_unrelated_lists_without_resolving_visible_cells() {
    let mut f = table_fixture();
    f.paragraphs.spans[1]
        .raw_modifiers
        .extend([0x21, 0x76, 0, 1, 100, 0, 0x2b, 0xd6, 2, 0, 3]);
    let out = f.run().unwrap();
    assert!(out.stories[0].text.ends_with("1.\tItem\n2.\tItem\n"));
    assert!(
        out.unresolved
            .contains(&"merged_cell_visible_ownership_unresolved")
    );
    partition(&out.stories[0], 15);
    f.paragraphs.spans[0].raw_modifiers[2] = 1;
    assert!(
        !f.run().unwrap().stories[0]
            .segments
            .iter()
            .any(|s| s.kind == "generated_list_label")
    );
    f.paragraphs.spans[0].raw_modifiers[2] = 0;
    f.paragraphs.spans[1]
        .raw_modifiers
        .extend([0x21, 0x76, 1, 1, 100, 0]);
    assert!(f.run().is_err());
}

#[test]
fn table_shading_validates_colors_patterns_and_framing() {
    let base = [0x60, 0xd6, 10, 0, 0, 0, 0xff, 0xff, 0xff, 0xff, 0, 0, 0];
    for pattern in [0, 0x19, 0x23, 0x3d, 0xffff] {
        let mut data = base;
        data[11..].copy_from_slice(&u16::to_le_bytes(pattern));
        let mut s = Selection::default();
        apply_paragraph(&mut s, &data).unwrap();
        assert!(s.table_appearance);
    }
    for pattern in [0x1a, 0x22, 0x3e, 0xfffe] {
        let mut data = base;
        data[11..].copy_from_slice(&u16::to_le_bytes(pattern));
        assert!(apply_paragraph(&mut Selection::default(), &data).is_err());
    }
    for index in [6, 10] {
        let mut data = base;
        data[index] = 1;
        assert!(apply_paragraph(&mut Selection::default(), &data).is_err());
    }
    for end in 2..base.len() {
        assert!(apply_paragraph(&mut Selection::default(), &base[..end]).is_err());
    }
    for length in [9, 11] {
        let mut data = base.to_vec();
        data[2] = length;
        data.resize(usize::from(length) + 3, 0);
        assert!(apply_paragraph(&mut Selection::default(), &data).is_err());
    }
}

#[test]
fn default_cell_spacing_validates_all_units_bounds_and_scope_bytes() {
    let base = [0x33, 0xd6, 6, 0, 1, 0x0f, 3, 0, 0];
    for unit in [0, 3, 0x13] {
        for width in [0, 1, 15_840, 15_841, 0xffff] {
            let mut data = base;
            data[6] = unit;
            data[7..].copy_from_slice(&u16::to_le_bytes(width));
            assert_eq!(
                apply_paragraph(&mut Selection::default(), &data).is_ok(),
                width <= 15_840 && (unit != 0 || width == 0)
            );
        }
    }
    for (index, value) in [(2, 5), (2, 7), (3, 1), (4, 2), (5, 7), (6, 1), (6, 0x23)] {
        let mut data = base;
        data[index] = value;
        assert!(apply_paragraph(&mut Selection::default(), &data).is_err());
    }
    for end in 2..base.len() {
        assert!(apply_paragraph(&mut Selection::default(), &base[..end]).is_err());
    }
}

#[test]
fn table_shading_and_spacing_preserve_outside_lists_and_semantic_guards() {
    let modifiers = [
        0x60, 0xd6, 10, 0, 0, 0, 0xff, 0xff, 0xff, 0xff, 0, 0, 0, 0x33, 0xd6, 6, 0, 1, 0x0f, 3, 15,
        0,
    ];
    for before in [true, false] {
        let mut f = table_fixture();
        let row = &mut f.paragraphs.spans[1].raw_modifiers;
        if before {
            row.splice(0..0, modifiers);
        } else {
            row.extend(modifiers);
        }
        let out = f.run().unwrap();
        assert!(out.stories[0].text.ends_with("1.\tItem\n2.\tItem\n"));
        assert!(
            out.unresolved
                .contains(&"table_visual_appearance_unresolved")
        );
        partition(&out.stories[0], 15);
        f.paragraphs.spans[0]
            .raw_modifiers
            .extend([0x0b, 0x46, 1, 0]);
        assert!(
            !f.run().unwrap().stories[0]
                .segments
                .iter()
                .any(|s| s.kind == "generated_list_label")
        );
        f.paragraphs.spans[1].raw_modifiers.extend([0x16, 0x24, 0]);
        assert!(f.run().is_err());
    }
    let mut outside = Fixture::new(&[1]);
    outside.paragraphs.spans[0].raw_modifiers.extend(modifiers);
    assert!(outside.run().is_err());
}

#[test]
fn list_mark_requires_its_own_physical_paragraph_endpoint() {
    let mut f = Fixture::new(&[1, 1]);
    f.paragraphs.spans[0].end_cp = f.paragraphs.spans[1].end_cp;
    f.paragraphs.spans.remove(1);
    assert!(matches!(
        f.run(),
        Err(Error::Invalid("list paragraph coverage"))
    ));
}

#[test]
fn canonical_paragraphs_keep_interior_controls_without_false_list_anchors() {
    let mut f = Fixture::new(&[1, 1, 1]);
    f.paragraphs.status = "reference_expanded_endpoint_selected_paragraphs";
    f.paragraphs.spans[0].end_cp = f.paragraphs.spans[1].end_cp;
    f.paragraphs.spans.remove(1);
    let out = f.run().unwrap();
    let anchors: Vec<_> = out.stories[0]
        .segments
        .iter()
        .filter(|s| s.kind == "generated_list_label")
        .map(|s| s.start_cp)
        .collect();
    assert_eq!(anchors, vec![0, 10]);
    assert_eq!(out.stories[0].text, "1.\tItem\nItem\n2.\tItem\n");
}

#[test]
fn canonical_section_end_cannot_move_next_label_before_its_source_paragraph() {
    let mut f = Fixture::new(&[1, 1]);
    f.paragraphs.status = "reference_expanded_endpoint_selected_paragraphs";
    f.source.stories[0].text.replace_range(4..5, "\u{c}");
    let out = f.run().unwrap();
    assert_eq!(out.stories[0].text, "1.\tItem\u{c}2.\tItem\n");
    let anchors: Vec<_> = out.stories[0]
        .segments
        .iter()
        .filter(|s| s.kind == "generated_list_label")
        .map(|s| s.start_cp)
        .collect();
    assert_eq!(anchors, vec![0, 5]);
    f.paragraphs.spans[0].end_cp = 10;
    f.paragraphs.spans.remove(1);
    let out = f.run().unwrap();
    let anchors: Vec<_> = out.stories[0]
        .segments
        .iter()
        .filter(|s| s.kind == "generated_list_label")
        .map(|s| s.start_cp)
        .collect();
    assert_eq!(anchors, vec![0]);
    assert!(out.stories[0].text.contains('\u{c}'));
}

#[test]
fn nonlist_section_endpoint_preserves_counter_and_unsupported_terminals_block() {
    let mut f = Fixture::new(&[1, 1, 1]);
    f.paragraphs.status = "reference_expanded_endpoint_selected_paragraphs";
    f.source.stories[0].text.replace_range(9..10, "\u{c}");
    f.paragraphs.spans[1].raw_modifiers.clear();
    let out = f.run().unwrap();
    assert_eq!(out.stories[0].text, "1.\tItem\nItem\u{c}2.\tItem\n");
    f.source.stories[0].text.replace_range(9..10, "\u{e}");
    assert!(matches!(
        f.run(),
        Err(Error::Unsupported("list physical paragraph terminal"))
    ));
    let mut f = table_fixture();
    f.paragraphs.status = "reference_expanded_endpoint_selected_paragraphs";
    let cp = f.paragraphs.spans[0].end_cp - 1;
    f.source.stories[0].text.replace_range(cp..cp + 1, "\u{c}");
    assert!(matches!(
        f.run(),
        Err(Error::Unsupported("list section endpoint inside table"))
    ));
}

#[test]
fn exact_section_only_paragraph_keeps_boundary_without_advancing_visible_counter() {
    let mut f = Fixture::new(&[1, 1]);
    f.paragraphs.status = "reference_expanded_endpoint_selected_paragraphs";
    f.source.stories[0].text = "\u{c}Item\rabc\r".into();
    f.paragraphs.spans[0].end_cp = 1;
    f.paragraphs.spans[1].start_cp = 1;
    f.paragraphs.spans[1].end_cp = 6;
    f.paragraphs.spans.push(ParagraphSpan {
        start_cp: 6,
        end_cp: 10,
        style_index: Some(0),
        raw_modifiers: vec![0x0b, 0x46, 1, 0],
    });
    let out = f.run().unwrap();
    assert_eq!(out.stories[0].text, "\u{c}1.\tItem\n2.\tabc\n");
    assert!(
        out.stories[0]
            .diagnostics
            .iter()
            .any(|d| d.code == "section_only_list_visibility_unresolved" && d.cp == 0)
    );
    partition(&out.stories[0], 10);
    f.source.stories[0].text = " \u{c}Item\rxy\r".into();
    f.paragraphs.spans[0].end_cp = 2;
    f.paragraphs.spans[1].start_cp = 2;
    f.paragraphs.spans[1].end_cp = 7;
    f.paragraphs.spans[2].start_cp = 7;
    let out = f.run().unwrap();
    assert_eq!(out.stories[0].text, "1.\t \u{c}2.\tItem\n3.\txy\n");
}

fn historical_number(was_numbered: bool, value: u32) -> Vec<u8> {
    let mut data = vec![0x45, 0xc6, 128];
    let mut raw = vec![0; 128];
    raw[0] = u8::from(was_numbered);
    raw[8] = 1;
    raw[28..32].copy_from_slice(&value.to_le_bytes());
    raw[64] = 2;
    raw[68] = 46;
    data.extend(raw);
    data
}

#[test]
fn accepted_numbering_uses_current_lists_and_preserves_historical_snapshots() {
    let mut f = Fixture::new(&[1, 0, 1]);
    for (index, old) in [27, 4, 99].into_iter().enumerate() {
        f.paragraphs.spans[index]
            .raw_modifiers
            .extend(historical_number(true, old));
    }
    let report = project_lists(&transcribe(&f.source), &f.context()).unwrap();
    assert_eq!(
        report.transcription.stories[0].text,
        "1.\tItem\nItem\n2.\tItem\n"
    );
    partition(&report.transcription.stories[0], 15);
    assert!(
        report
            .transcription
            .unresolved
            .contains(&"historical_numbering_view_unresolved")
    );
    assert_eq!(report.numbering_revision_evidence.len(), 3);
    for (index, evidence) in report.numbering_revision_evidence.iter().enumerate() {
        assert_eq!(evidence.paragraph_mark_cp, index * 5 + 4);
        assert_eq!(
            evidence.snapshot.as_ref().unwrap().number_values[0],
            [27, 4, 99][index]
        );
        assert_eq!(evidence.historical_view, "unresolved");
    }
}

#[test]
fn numbering_insertion_flag_does_not_create_list_membership_or_override_later_state() {
    let mut f = Fixture::new(&[0, 1]);
    for span in &mut f.paragraphs.spans {
        span.raw_modifiers.extend(historical_number(false, 999));
        span.raw_modifiers.extend([0x43, 0x24, 1]);
    }
    assert_eq!(f.run().unwrap().stories[0].text, "Item\n1.\tItem\n");
    f.paragraphs.spans[1]
        .raw_modifiers
        .extend(historical_number(true, 3));
    assert!(matches!(
        f.run(),
        Err(Error::Unsupported("inconsistent numbering revision flags"))
    ));
    f.paragraphs.spans[1].raw_modifiers.extend([0x43, 0x24, 0]);
    assert_eq!(f.run().unwrap().stories[0].text, "Item\n1.\tItem\n");
    f.revisions.spans[0].properties.deleted = Some(true);
    assert!(f.run().is_err());
}

#[test]
fn numbering_revision_properties_are_forbidden_in_paragraph_and_table_styles() {
    for bytes in [historical_number(true, 1), vec![0x43, 0x24, 0]] {
        let mut paragraph = Fixture::new(&[1]);
        paragraph.styles.styles[0].as_mut().unwrap().properties[0]
            .bytes
            .extend(bytes.clone());
        assert!(matches!(
            paragraph.run(),
            Err(Error::Unsupported("numbering revision property in style"))
        ));
        let mut table = table_fixture();
        table.styles.styles[11]
            .as_mut()
            .unwrap()
            .properties
            .push(StyleProperties {
                kind: "table",
                bytes,
            });
        assert!(matches!(
            table.run(),
            Err(Error::Unsupported("numbering revision property in style"))
        ));
    }
}

#[test]
fn malformed_numbering_history_is_not_discarded_to_generate_labels() {
    for (index, value) in [(2, 127), (3, 2), (11, 3), (67, 32)] {
        let mut f = Fixture::new(&[1]);
        let mut raw = historical_number(true, 1);
        raw[index] = value;
        f.paragraphs.spans[0].raw_modifiers.extend(raw);
        assert!(f.run().is_err());
    }
}

#[test]
fn numbering_revision_evidence_has_independent_record_and_byte_limits() {
    let mut selection = Selection::default();
    apply_paragraph(&mut selection, &historical_number(true, 1)).unwrap();
    let mut records = Vec::new();
    let mut bytes = 0;
    assert!(matches!(
        retain_numbering_revision(&mut records, &mut bytes, 4, &selection),
        Err(Error::Limit("numbering revision evidence bytes"))
    ));
    assert!(records.is_empty());
    let mut bytes = MAX_REVISION_BYTES;
    retain_numbering_revision(&mut records, &mut bytes, 4, &selection).unwrap();
    records.resize(MAX_REVISION_RECORDS, records[0].clone());
    assert!(matches!(
        retain_numbering_revision(&mut records, &mut bytes, 9, &selection),
        Err(Error::Limit("numbering revision evidence records"))
    ));
}

fn property_mark(op: u16, flag: u8) -> Vec<u8> {
    let mut bytes = op.to_le_bytes().to_vec();
    bytes.extend([7, flag, 0xfe, 0xff, 1, 2, 3, 4]);
    bytes
}

#[test]
fn property_revision_metadata_keeps_current_selection_and_ordered_reset() {
    let mut f = Fixture::new(&[1, 0, 1]);
    for p in &mut f.paragraphs.spans {
        p.raw_modifiers.extend(property_mark(0xc66f, 1));
    }
    f.paragraphs.spans[0]
        .raw_modifiers
        .extend(property_mark(0xc66f, 0));
    let report = project_lists(&transcribe(&f.source), &f.context()).unwrap();
    assert_eq!(
        report.transcription.stories[0].text,
        "1.\tItem\nItem\n2.\tItem\n"
    );
    partition(&report.transcription.stories[0], 15);
    assert_eq!(report.property_revision_evidence.len(), 3);
    for (i, record) in report.property_revision_evidence.iter().enumerate() {
        assert_eq!(record.paragraph_mark_cp, i * 5 + 4);
        let mark = record.paragraph.as_ref().unwrap();
        assert_eq!(mark.revision_present, i != 0);
        assert_eq!(mark.author_index, -2);
        assert_eq!(mark.timestamp_raw, 0x04030201);
        assert_eq!(
            record.current_display_status,
            "not_determined_by_revision_metadata"
        );
        assert_eq!(record.historical_view, "unresolved");
    }
    assert!(
        report
            .transcription
            .unresolved
            .contains(&"historical_property_view_unresolved")
    );
    f.revisions.spans[0].properties.deleted = Some(true);
    assert!(f.run().is_err());
}

#[test]
fn property_revision_styles_and_old_property_walls_remain_blocked() {
    for op in [0xc66f, 0xd667] {
        for flag in [0, 1] {
            let mut f = Fixture::new(&[1]);
            f.styles.styles[0].as_mut().unwrap().properties[0]
                .bytes
                .extend(property_mark(op, flag));
            assert!(matches!(
                f.run(),
                Err(Error::Unsupported("property revision in style"))
            ));
            let mut f = table_fixture();
            f.styles.styles[11]
                .as_mut()
                .unwrap()
                .properties
                .push(StyleProperties {
                    kind: "table",
                    bytes: property_mark(op, flag),
                });
            assert!(matches!(
                f.run(),
                Err(Error::Unsupported("property revision in style"))
            ));
        }
    }
    for wall in [0x2664u16, 0x3668] {
        for flag in [0, 1] {
            for before in [false, true] {
                let mut f = Fixture::new(&[1]);
                let mut bytes = wall.to_le_bytes().to_vec();
                bytes.push(flag);
                let mark = property_mark(0xc66f, 1);
                if before {
                    bytes.extend(mark);
                } else {
                    let mut first = mark;
                    first.extend(bytes);
                    bytes = first;
                }
                f.paragraphs.spans[0].raw_modifiers.extend(bytes);
                assert!(matches!(
                    f.run(),
                    Err(Error::Unsupported("list property revision history wall"))
                ));
            }
        }
    }
}

#[test]
fn table_property_revision_requires_table_context_and_does_not_change_labels() {
    let mut f = table_fixture();
    let expected = f.run().unwrap().stories[0].text.clone();
    f.paragraphs.spans[0]
        .raw_modifiers
        .extend(property_mark(0xd667, 1));
    let report = project_lists(&transcribe(&f.source), &f.context()).unwrap();
    assert_eq!(report.transcription.stories[0].text, expected);
    assert!(
        report.property_revision_evidence[0]
            .table_row
            .as_ref()
            .unwrap()
            .revision_present
    );
    let mut f = Fixture::new(&[1]);
    f.paragraphs.spans[0]
        .raw_modifiers
        .extend(property_mark(0xd667, 0));
    assert!(matches!(
        f.run(),
        Err(Error::Unsupported("table property revision outside table"))
    ));
}

#[test]
fn malformed_property_revision_and_evidence_limits_are_explicit() {
    for op in [0xc66f, 0xd667] {
        for (index, value) in [(2, 6), (3, 2)] {
            let mut f = Fixture::new(&[1]);
            let mut data = property_mark(op, 1);
            data[index] = value;
            f.paragraphs.spans[0].raw_modifiers.extend(data);
            assert!(f.run().is_err());
        }
    }
    let mut selection = Selection::default();
    apply_paragraph(&mut selection, &property_mark(0xc66f, 1)).unwrap();
    let mut records = Vec::new();
    let mut bytes = 0;
    assert!(matches!(
        retain_property_revision(&mut records, &mut bytes, 4, &selection),
        Err(Error::Limit("property revision evidence bytes"))
    ));
    assert!(records.is_empty());
    let mut bytes = MAX_REVISION_BYTES;
    retain_property_revision(&mut records, &mut bytes, 4, &selection).unwrap();
    records.resize(MAX_REVISION_RECORDS, records[0].clone());
    assert!(matches!(
        retain_property_revision(&mut records, &mut bytes, 9, &selection),
        Err(Error::Limit("property revision evidence records"))
    ));
}

#[test]
fn paragraph_direction_resolves_in_source_order_before_list_projection() {
    let mut f = Fixture::new(&[1, 1]);
    f.paragraphs.spans[0].raw_modifiers.extend([0x41, 0x24, 0]);
    f.paragraphs.spans[1]
        .raw_modifiers
        .extend([0x41, 0x24, 1, 0x41, 0x24, 0]);
    assert_eq!(f.run().unwrap().stories[0].text, "1.\tItem\n2.\tItem\n");
    f.paragraphs.spans[1].raw_modifiers.extend([0x41, 0x24, 1]);
    assert_eq!(f.run().unwrap().stories[0].text, "Item\nItem\n");
    let mut f = Fixture::new(&[0, 1]);
    f.paragraphs.spans[0].raw_modifiers.extend([0x41, 0x24, 1]);
    assert_eq!(f.run().unwrap().stories[0].text, "Item\n1.\tItem\n");
}

#[test]
fn inherited_paragraph_direction_can_be_explicitly_reset_by_current_properties() {
    let mut f = Fixture::new(&[1, 1]);
    f.styles.styles[0].as_mut().unwrap().properties[0]
        .bytes
        .extend([0x41, 0x24, 1]);
    for p in &mut f.paragraphs.spans {
        p.raw_modifiers.extend([0x41, 0x24, 0]);
    }
    assert_eq!(f.run().unwrap().stories[0].text, "1.\tItem\n2.\tItem\n");
    f.paragraphs.spans[1].raw_modifiers.truncate(4);
    assert_eq!(f.run().unwrap().stories[0].text, "Item\nItem\n");
    f.styles.styles[0].as_mut().unwrap().properties[0]
        .bytes
        .extend([0x41, 0x24, 0]);
    assert_eq!(f.run().unwrap().stories[0].text, "1.\tItem\n2.\tItem\n");
}

#[test]
fn invalid_and_truncated_direction_operands_fail_before_later_reset() {
    for bytes in [vec![0x41, 0x24], vec![0x41, 0x24, 2, 0x41, 0x24, 0]] {
        let mut f = Fixture::new(&[1]);
        f.paragraphs.spans[0].raw_modifiers.extend(bytes);
        assert!(f.run().is_err());
    }
}

#[test]
fn table_style_and_list_level_direction_remain_guarded() {
    for flag in [0, 1] {
        let mut f = table_fixture();
        f.styles.styles[11]
            .as_mut()
            .unwrap()
            .properties
            .push(StyleProperties {
                kind: "paragraph",
                bytes: vec![11, 0, 0x41, 0x24, flag],
            });
        assert_eq!(f.run().is_ok(), flag == 0);
        let mut f = Fixture::new(&[1]);
        f.definitions.lists[0].levels[0]
            .raw_papx
            .extend([0x41, 0x24, flag]);
        let out = f.run().unwrap();
        assert_eq!(
            out.stories[0].text,
            if flag == 0 { "1.\tItem\n" } else { "Item\n" }
        );
        if flag == 1 {
            assert!(
                out.stories[0]
                    .diagnostics
                    .iter()
                    .any(|d| d.code == "list_active_right_to_left_unresolved")
            );
        }
    }
}

#[test]
fn child_style_direction_overrides_parent_before_direct_paragraph_properties() {
    let mut f = Fixture::new(&[1]);
    f.styles.styles[0].as_mut().unwrap().properties[0]
        .bytes
        .extend([0x41, 0x24, 1]);
    f.styles.styles.push(Some(Style {
        index: 1,
        builtin_id: 0,
        kind: 1,
        base_style: Some(0),
        next_style: 0,
        linked_style: None,
        revision_marked: false,
        name: "Child".into(),
        raw_flags: 0,
        properties: vec![StyleProperties {
            kind: "paragraph",
            bytes: vec![1, 0, 0x41, 0x24, 0],
        }],
        raw_revision: vec![],
    }));
    f.paragraphs.spans[0].style_index = Some(1);
    assert_eq!(f.run().unwrap().stories[0].text, "1.\tItem\n");
    f.paragraphs.spans[0].raw_modifiers.extend([0x41, 0x24, 1]);
    assert_eq!(f.run().unwrap().stories[0].text, "Item\n");
}

#[test]
fn rtl_nonlist_paragraph_keeps_logical_text_and_unrelated_numbering() {
    let mut f = Fixture::new(&[0, 1]);
    f.source.stories[0].text = "(אב)\rItem\r".into();
    f.paragraphs.spans[0].raw_modifiers.extend([0x41, 0x24, 1]);
    let report = project_lists(&transcribe(&f.source), &f.context()).unwrap();
    assert_eq!(report.transcription.stories[0].text, "(אב)\n1.\tItem\n");
    partition(&report.transcription.stories[0], 10);
    assert_eq!(report.paragraph_direction_evidence.len(), 1);
    let e = &report.paragraph_direction_evidence[0];
    assert_eq!((e.start_cp, e.paragraph_mark_cp), (0, 4));
    assert_eq!(e.text_order, "source_logical_order");
    assert_eq!(e.visual_order_and_shaping, "unresolved");
    assert_eq!(e.list_override_index, None);
    assert!(
        report
            .transcription
            .unresolved
            .contains(&"rtl_visual_order_and_shaping_unresolved")
    );
}

#[test]
fn rtl_list_member_blocks_all_overrides_of_same_lsid_but_not_other_sequences() {
    let mut f = Fixture::new(&[1, 0, 1, 2, 3, 2]);
    for index in [1, 2] {
        f.paragraphs.spans[index]
            .raw_modifiers
            .extend([0x41, 0x24, 1]);
    }
    let report = project_lists(&transcribe(&f.source), &f.context()).unwrap();
    assert_eq!(
        report.transcription.stories[0].text,
        "Item\nItem\nItem\n•\tItem\nItem\n•\tItem\n"
    );
    partition(&report.transcription.stories[0], 30);
    let e = &report.paragraph_direction_evidence[1];
    assert_eq!((e.start_cp, e.paragraph_mark_cp), (10, 14));
    assert_eq!(e.list_override_index, Some(1));
    assert_eq!(e.list_level, Some(0));
    assert_eq!(
        e.list_label_status,
        "unresolved_right_to_left_label_properties"
    );
    assert!(
        report
            .transcription
            .unresolved
            .contains(&"rtl_list_label_properties_unresolved")
    );
    assert!(
        report.transcription.stories[0]
            .diagnostics
            .iter()
            .any(|d| d.code == "list_active_right_to_left_unresolved" && d.cp == 10)
    );
    f.revisions.spans[0].properties.deleted = Some(true);
    assert!(f.run().is_err());
}

#[test]
fn rtl_direction_evidence_uses_canonical_endpoints_not_interior_controls() {
    let mut f = Fixture::new(&[0, 0, 1]);
    f.paragraphs.status = "reference_expanded_endpoint_selected_paragraphs";
    f.paragraphs.spans[0].end_cp = 10;
    f.paragraphs.spans.remove(1);
    f.paragraphs.spans[0].raw_modifiers.extend([0x41, 0x24, 1]);
    let report = project_lists(&transcribe(&f.source), &f.context()).unwrap();
    assert_eq!(report.paragraph_direction_evidence.len(), 1);
    let e = &report.paragraph_direction_evidence[0];
    assert_eq!((e.start_cp, e.paragraph_mark_cp), (0, 9));
    assert_eq!(
        report.transcription.stories[0].text,
        "Item\nItem\n1.\tItem\n"
    );
}

#[test]
fn direction_evidence_has_record_and_shared_byte_limits() {
    let selection = Selection {
        right_to_left: true,
        ..Default::default()
    };
    let mut records = Vec::new();
    let mut budget = 0;
    assert!(matches!(
        retain_direction(&mut records, &mut budget, 0, 4, &selection),
        Err(Error::Limit("paragraph direction evidence bytes"))
    ));
    assert!(records.is_empty());
    let mut budget = MAX_REVISION_BYTES;
    retain_direction(&mut records, &mut budget, 0, 4, &selection).unwrap();
    records.resize(MAX_REVISION_RECORDS, records[0].clone());
    assert!(matches!(
        retain_direction(&mut records, &mut budget, 5, 9, &selection),
        Err(Error::Limit("paragraph direction evidence records"))
    ));
}

fn rtl_fixture(format: u8) -> Fixture {
    let mut f = Fixture::new(&[1, 1]);
    for p in &mut f.paragraphs.spans {
        p.raw_modifiers.extend([0x41, 0x24, 1]);
    }
    f.fonts.fonts[0].charset = 0;
    f.fonts.fonts[0].name = "Unicode font".into();
    f.styles.raw_header = vec![0; 20];
    f.definitions.lists[0].levels[0].number_format = format;
    f.definitions.lists[0].levels[0].raw_chpx = vec![0x5a, 0x08, 1, 0x5e, 0x4a, 0, 0];
    f
}

#[test]
fn rtl_decimal_labels_preserve_logical_punctuation_and_source_ownership() {
    let mut f = rtl_fixture(0);
    f.definitions.lists[0].levels[0].number_text_utf16 = vec![40, 0, 41];
    f.definitions.lists[0].levels[0].placeholder_offsets[0] = 2;
    let report = project_lists(&transcribe(&f.source), &f.context()).unwrap();
    assert_eq!(
        report.transcription.stories[0].text,
        "(1)\tItem\n(2)\tItem\n"
    );
    partition(&report.transcription.stories[0], 10);
    for record in report.paragraph_direction_evidence {
        assert_eq!(
            record.list_label_status,
            "logical_label_generated_visual_shaping_unresolved"
        );
        let properties = record.label_properties.unwrap();
        assert_eq!(properties.font_index, 0);
        assert!(properties.character_right_to_left);
        assert_eq!(properties.language_id, None);
    }
}

#[test]
fn rtl_arabic_alpha_uses_spec_alphabet_repeat_and_nonjoiner() {
    assert_eq!(rtl::arabic_alpha(1).unwrap(), "أ\u{200c}");
    assert_eq!(rtl::arabic_alpha(27).unwrap(), "و\u{200c}");
    assert_eq!(rtl::arabic_alpha(28).unwrap(), "ي\u{200c}");
    assert_eq!(rtl::arabic_alpha(29).unwrap(), "أأ\u{200c}");
    assert_eq!(rtl::arabic_alpha(30).unwrap(), "بب\u{200c}");
    assert!(rtl::arabic_alpha(0).is_err());
    assert!(rtl::arabic_alpha(28 * 512 + 1).is_err());
    let mut f = rtl_fixture(46);
    f.definitions.lists[0].levels[0].start_at = 28;
    let out = f.run().unwrap();
    assert_eq!(out.stories[0].text, "ي\u{200c}.\tItem\nأأ\u{200c}.\tItem\n");
    partition(&out.stories[0], 10);
}

#[test]
fn rtl_mark_and_level_toggles_use_style_baseline() {
    let mut f = rtl_fixture(0);
    f.styles.styles[0]
        .as_mut()
        .unwrap()
        .properties
        .push(StyleProperties {
            kind: "character",
            bytes: vec![0x5a, 0x08, 1, 0x5c, 0x08, 1, 0x5f, 0x48, 1, 4],
        });
    f.formatting.character_spans[0].properties =
        decode_character_modifiers(&[0x5c, 0x08, 129]).unwrap();
    f.definitions.lists[0].levels[0].raw_chpx =
        vec![0x5a, 0x08, 128, 0x5c, 0x08, 128, 0x5d, 0x08, 129];
    let report = project_lists(&transcribe(&f.source), &f.context()).unwrap();
    assert_eq!(report.transcription.stories[0].text, "1.\tItem\n2.\tItem\n");
    let properties = report.paragraph_direction_evidence[0]
        .label_properties
        .as_ref()
        .unwrap();
    assert!(properties.complex_bold && properties.complex_italic);
    assert_eq!(properties.language_id, Some(0x0401));
}

#[test]
fn rtl_bad_font_toggle_hidden_property_and_budget_remain_blocked() {
    for raw in [
        vec![0x5a, 0x08, 2],
        vec![0x5a, 0x08, 1, 0x5e, 0x4a, 255, 255],
        vec![0x5a, 0x08, 1, 0x3c, 0x08, 1],
    ] {
        let mut f = rtl_fixture(0);
        f.definitions.lists[0].levels[0].raw_chpx = raw;
        assert_eq!(f.run().unwrap().stories[0].text, "Item\nItem\n");
    }
    let mut f = rtl_fixture(0);
    f.fonts.fonts[0].charset = 2;
    assert_eq!(f.run().unwrap().stories[0].text, "Item\nItem\n");
    let mut budget = 0;
    assert!(
        rtl::resolve(
            &f.styles,
            0,
            &[],
            &f.definitions.lists[0].levels[0],
            &f.fonts,
            &mut budget
        )
        .is_err()
    );
}

#[test]
fn rtl_language_defaults_hint_and_level_font_override() {
    let mut f = rtl_fixture(0);
    f.styles.styles[0]
        .as_mut()
        .unwrap()
        .properties
        .push(StyleProperties {
            kind: "character",
            bytes: vec![0x73, 0x48, 9, 4, 0x5f, 0x48, 1, 4, 0x5e, 0x4a, 99, 0],
        });
    f.definitions.lists[0].levels[0].raw_chpx = vec![0x6f, 0x28, 2, 0x5e, 0x4a, 0, 0];
    let report = project_lists(&transcribe(&f.source), &f.context()).unwrap();
    let properties = report.paragraph_direction_evidence[0]
        .label_properties
        .as_ref()
        .unwrap();
    assert_eq!(properties.font_index, 0);
    assert_eq!(properties.language_id, Some(0x0401));
    assert!(properties.complex_script_hint && properties.character_right_to_left);
    for hint in [1, 3, 255] {
        f.definitions.lists[0].levels[0].raw_chpx[2] = hint;
        assert_eq!(f.run().unwrap().stories[0].text, "Item\nItem\n");
    }
}

#[test]
fn rtl_unknown_direction_inherited_duplicates_and_character_styles_stay_unresolved() {
    let mut f = rtl_fixture(0);
    f.definitions.lists[0].levels[0].raw_chpx = vec![];
    assert_eq!(f.run().unwrap().stories[0].text, "Item\nItem\n");
    f.definitions.lists[0].levels[0].raw_chpx = vec![0x5a, 0x08, 1];
    f.styles.styles[0]
        .as_mut()
        .unwrap()
        .properties
        .push(StyleProperties {
            kind: "character",
            bytes: vec![0x5a, 0x08, 0, 0x5a, 0x08, 1],
        });
    assert_eq!(f.run().unwrap().stories[0].text, "Item\nItem\n");
    let mut f = rtl_fixture(0);
    f.formatting.character_spans[0].properties =
        decode_character_modifiers(&[0x30, 0x4a, 10, 0]).unwrap();
    assert_eq!(f.run().unwrap().stories[0].text, "Item\nItem\n");
}

#[test]
fn rtl_unsupported_member_still_blocks_whole_shared_sequence() {
    let mut f = rtl_fixture(0);
    f.formatting.character_spans = vec![
        CharacterSpan {
            start_cp: 0,
            end_cp: 5,
            properties: vec![],
        },
        CharacterSpan {
            start_cp: 5,
            end_cp: 10,
            properties: decode_character_modifiers(&[0x3c, 0x08, 1]).unwrap(),
        },
    ];
    let report = project_lists(&transcribe(&f.source), &f.context()).unwrap();
    assert_eq!(report.transcription.stories[0].text, "Item\nItem\n");
    assert!(
        report
            .paragraph_direction_evidence
            .iter()
            .all(|e| e.label_properties.is_none())
    );
}

#[test]
fn rtl_level_membership_and_ltr_override_do_not_bypass_physical_guards() {
    for raw in [
        vec![0x16, 0x24, 1],
        vec![0x49, 0x66, 1, 0, 0, 0],
        vec![0x41, 0x24, 0],
    ] {
        let mut f = rtl_fixture(0);
        f.definitions.lists[0].levels[0].raw_papx = raw;
        assert_eq!(f.run().unwrap().stories[0].text, "Item\nItem\n");
    }
}

#[test]
fn rtl_repeated_direct_notstyle_uses_style_baseline_each_time() {
    let mut f = rtl_fixture(0);
    f.styles.styles[0]
        .as_mut()
        .unwrap()
        .properties
        .push(StyleProperties {
            kind: "character",
            bytes: vec![0x5a, 0x08, 1],
        });
    f.definitions.lists[0].levels[0].raw_chpx = vec![];
    f.formatting.character_spans[0].properties =
        decode_character_modifiers(&[0x5a, 0x08, 129, 0x5a, 0x08, 129]).unwrap();
    let report = project_lists(&transcribe(&f.source), &f.context()).unwrap();
    assert_eq!(report.transcription.stories[0].text, "1.\tItem\n2.\tItem\n");
    assert!(
        !report.paragraph_direction_evidence[0]
            .label_properties
            .as_ref()
            .unwrap()
            .character_right_to_left
    );
}

#[test]
fn rtl_inherited_language_aliases_cannot_evade_duplicate_guard() {
    for (older, newer) in [(0x486du16, 0x4873u16), (0x486e, 0x4874)] {
        let mut f = rtl_fixture(0);
        let mut parent = older.to_le_bytes().to_vec();
        parent.extend([9, 4]);
        f.styles.styles[0]
            .as_mut()
            .unwrap()
            .properties
            .push(StyleProperties {
                kind: "character",
                bytes: parent,
            });
        let mut child = newer.to_le_bytes().to_vec();
        child.extend([1, 4]);
        f.styles.styles.push(Some(Style {
            index: 1,
            builtin_id: 0xffe,
            kind: 1,
            base_style: Some(0),
            next_style: 1,
            linked_style: None,
            revision_marked: false,
            name: "Derived".into(),
            raw_flags: 0,
            raw_revision: vec![],
            properties: vec![
                StyleProperties {
                    kind: "paragraph",
                    bytes: vec![1, 0],
                },
                StyleProperties {
                    kind: "character",
                    bytes: child,
                },
            ],
        }));
        for p in &mut f.paragraphs.spans {
            p.style_index = Some(1);
        }
        assert_eq!(f.run().unwrap().stories[0].text, "Item\nItem\n");
    }
}

#[test]
fn rtl_arabic_number_font_does_not_mask_symbol_punctuation_font() {
    let mut f = rtl_fixture(46);
    f.fonts.fonts.push(Font {
        index: 1,
        name: "Other Unicode font".into(),
        alternate_name: None,
        charset: 0,
        family: 0,
        weight: 400,
        panose: vec![],
        signature: vec![],
    });
    f.definitions.lists[0].levels[0].raw_chpx = vec![0x5a, 0x08, 0, 0x5e, 0x4a, 1, 0];
    let report = project_lists(&transcribe(&f.source), &f.context()).unwrap();
    assert_eq!(
        report.transcription.stories[0].text,
        "أ\u{200c}.\tItem\nب\u{200c}.\tItem\n"
    );
    let props = report.paragraph_direction_evidence[0]
        .label_properties
        .as_ref()
        .unwrap();
    assert_eq!(props.font_index, 1);
    assert_eq!(props.ascii_literal_font_index, Some(0));
    f.fonts.fonts[0].charset = 2;
    assert_eq!(f.run().unwrap().stories[0].text, "Item\nItem\n");
    f.definitions.lists[0].levels[0].raw_chpx[2] = 1;
    assert!(f.run().unwrap().stories[0].text.starts_with("أ\u{200c}."));
    f.fonts.fonts[0].charset = 0;
    f.definitions.lists[0].levels[0].number_text_utf16[1] = 0x2014;
    f.definitions.lists[0].levels[0].raw_chpx[2] = 0;
    assert_eq!(f.run().unwrap().stories[0].text, "Item\nItem\n");
    f.definitions.lists[0].levels[0].raw_chpx[2] = 1;
    assert!(f.run().unwrap().stories[0].text.starts_with("أ\u{200c}—"));
}

#[test]
fn table_direction_slots_combine_with_or_and_preserve_outside_labels() {
    for (old, new) in [(0, 0), (0, 1), (1, 0), (1, 1)] {
        let mut f = table_fixture();
        f.paragraphs.spans[1]
            .raw_modifiers
            .extend([0x0b, 0x56, old, 0, 0x64, 0x56, new, 0]);
        let report = project_lists(&transcribe(&f.source), &f.context()).unwrap();
        assert_eq!(
            report.transcription.stories[0].text,
            "abc\t\t1.\tItem\n2.\tItem\n"
        );
        partition(&report.transcription.stories[0], 15);
        let e = &report.table_direction_evidence[0];
        assert_eq!((e.paragraph_start_cp, e.row_mark_cp), (4, 4));
        assert_eq!(e.tf_bidi, Some(old != 0));
        assert_eq!(e.tf_bidi90, Some(new != 0));
        assert_eq!(e.right_to_left, old != 0 || new != 0);
        f.paragraphs.spans[1]
            .raw_modifiers
            .extend([0x0b, 0x56, 0, 0]);
        let report = project_lists(&transcribe(&f.source), &f.context()).unwrap();
        assert_eq!(report.table_direction_evidence[0].right_to_left, new != 0);
    }
}

#[test]
fn table_direction_retains_row_membership_visibility_and_bool_guards() {
    for raw in [
        vec![0x0b, 0x56, 2, 0],
        vec![0x64, 0x56, 0, 1],
        vec![0x0b, 0x56, 1],
    ] {
        let mut f = table_fixture();
        f.paragraphs.spans[1].raw_modifiers.extend(raw);
        assert!(f.run().is_err());
    }
    for index in [0, 2] {
        let mut f = table_fixture();
        f.paragraphs.spans[index]
            .raw_modifiers
            .extend([0x0b, 0x56, 1, 0]);
        assert!(f.run().is_err());
    }
    let mut f = table_fixture();
    f.paragraphs.spans[1]
        .raw_modifiers
        .extend([0x0b, 0x56, 1, 0]);
    f.paragraphs.spans[0].raw_modifiers[2] = 1;
    assert!(
        !f.run().unwrap().stories[0]
            .segments
            .iter()
            .any(|s| s.kind == "generated_list_label")
    );
    f.paragraphs.spans[0].raw_modifiers[2] = 0;
    f.revisions.spans[0].properties.deleted = Some(true);
    assert!(f.run().is_err());
}

#[test]
fn table_direction_default_style_and_direct_reset_remain_distinct() {
    let mut f = table_fixture();
    f.styles.styles[11]
        .as_mut()
        .unwrap()
        .properties
        .push(StyleProperties {
            kind: "table",
            bytes: vec![0x0b, 0x56, 1, 0, 0x64, 0x56, 1, 0],
        });
    f.paragraphs.spans[1]
        .raw_modifiers
        .extend([0x0b, 0x56, 0, 0]);
    let report = project_lists(&transcribe(&f.source), &f.context()).unwrap();
    assert_eq!(report.table_direction_evidence.len(), 1);
    assert_eq!(report.table_direction_evidence[0].tf_bidi, Some(false));
    assert_eq!(report.table_direction_evidence[0].tf_bidi90, Some(true));
    assert!(report.table_direction_evidence[0].right_to_left);
}

#[test]
fn identical_language_aliases_and_assigned_french_defaults_resolve_toggle_baseline() {
    for lid in [
        0x000cu16, 0x040c, 0x080c, 0x0c0c, 0x100c, 0x140c, 0x180c, 0x1c0c, 0x200c, 0x240c, 0x280c,
        0x2c0c, 0x300c, 0x340c, 0x380c, 0x3c0c,
    ] {
        let mut f = rtl_fixture(46);
        let mut properties = vec![0x6d, 0x48];
        properties.extend(lid.to_le_bytes());
        properties.extend([0x73, 0x48]);
        properties.extend(lid.to_le_bytes());
        f.styles.styles[0]
            .as_mut()
            .unwrap()
            .properties
            .push(StyleProperties {
                kind: "character",
                bytes: properties,
            });
        f.definitions.lists[0].levels[0].raw_chpx = vec![0x5a, 0x08, 129];
        assert!(f.run().unwrap().stories[0].text.starts_with("أ\u{200c}."));
    }
    let mut f = rtl_fixture(46);
    f.styles.styles[0]
        .as_mut()
        .unwrap()
        .properties
        .push(StyleProperties {
            kind: "character",
            bytes: vec![0x73, 0x48, 0x0c, 0xfc],
        });
    f.definitions.lists[0].levels[0].raw_chpx = vec![0x5a, 0x08, 129];
    assert_eq!(f.run().unwrap().stories[0].text, "Item\nItem\n");
}

fn border_style(index: usize, parent: usize) -> Style {
    let mut borders = vec![0x3a, 0x56, index as u8, (index >> 8) as u8, 0x13, 0xd6, 48];
    for _ in 0..6 {
        borders.extend([0, 0, 0, 255, 4, 1, 0, 0]);
    }
    Style {
        index,
        builtin_id: 0xffe,
        kind: 3,
        base_style: Some(parent),
        next_style: index,
        linked_style: None,
        revision_marked: false,
        name: "Custom borders".into(),
        raw_flags: 0,
        raw_revision: vec![],
        properties: vec![
            StyleProperties {
                kind: "table",
                bytes: borders,
            },
            StyleProperties {
                kind: "paragraph",
                bytes: (index as u16).to_le_bytes().to_vec(),
            },
            StyleProperties {
                kind: "character",
                bytes: vec![],
            },
        ],
    }
}

#[test]
fn derived_border_style_preserves_outside_labels_and_table_sequence_guard() {
    let mut f = table_fixture();
    f.styles.styles.push(Some(border_style(12, 11)));
    f.styles.styles.push(Some(border_style(13, 12)));
    f.paragraphs.spans[1]
        .raw_modifiers
        .extend([0x3a, 0x56, 13, 0]);
    assert_eq!(
        f.run().unwrap().stories[0].text,
        "abc\t\t1.\tItem\n2.\tItem\n"
    );
    f.paragraphs.spans[0].raw_modifiers[2] = 1;
    assert!(
        !f.run().unwrap().stories[0]
            .segments
            .iter()
            .any(|s| s.kind == "generated_list_label")
    );
}

#[test]
fn derived_border_style_rejects_history_cycles_identity_and_semantic_groups() {
    for case in 0..10 {
        let mut f = table_fixture();
        let mut s = border_style(12, 11);
        match case {
            0 => s.base_style = Some(12),
            1 => s.base_style = None,
            2 => s.revision_marked = true,
            3 => s.raw_revision.push(0),
            4 => s.properties[0].bytes[2] = 11,
            5 => s.properties[1].bytes[0] = 11,
            6 => s.properties[1].bytes.extend([0x0b, 0x46, 1, 0]),
            7 => s.properties[2].bytes.extend([0x3c, 0x08, 1]),
            8 => s.properties.push(StyleProperties {
                kind: "conditional",
                bytes: vec![],
            }),
            9 => s.properties[0].bytes.extend([0x08, 0xd6, 1, 0]),
            _ => unreachable!(),
        }
        f.styles.styles.push(Some(s));
        f.paragraphs.spans[1]
            .raw_modifiers
            .extend([0x3a, 0x56, 12, 0]);
        assert!(f.run().is_err(), "case {case}");
    }
}

#[test]
fn derived_border_style_validates_borders_and_bounds_cache_work() {
    for (offset, value) in [(6, 47), (10, 1), (12, 2), (12, 4), (12, 64)] {
        let mut f = table_fixture();
        let mut s = border_style(12, 11);
        s.properties[0].bytes[offset] = value;
        f.styles.styles.push(Some(s));
        let mut cache = BTreeSet::new();
        assert!(table_style::validate(12, &f.styles, &mut cache, &mut 4096).is_err());
        assert!(cache.is_empty());
    }
    let mut f = table_fixture();
    f.styles.styles.push(Some(border_style(12, 11)));
    let mut cache = BTreeSet::new();
    assert!(table_style::validate(12, &f.styles, &mut cache, &mut 1).is_err());
    let mut remaining = 4096;
    table_style::validate(12, &f.styles, &mut cache, &mut remaining).unwrap();
    let first = remaining;
    table_style::validate(12, &f.styles, &mut cache, &mut remaining).unwrap();
    assert_eq!(remaining, first);
}

#[test]
fn rtl_level_paragraph_indents_and_ordered_direction_use_paragraph_decoder() {
    for raw in [
        vec![0x0f, 0x84, 0xd0, 2, 0x11, 0x84, 0x98, 0xfe],
        vec![0x15, 0xc6, 5, 0, 1, 0xd0, 2, 6],
        vec![0x41, 0x24, 1],
        vec![0x41, 0x24, 0, 0x41, 0x24, 1],
    ] {
        let mut f = rtl_fixture(0);
        f.definitions.lists[0].levels[0].raw_papx = raw;
        assert_eq!(f.run().unwrap().stories[0].text, "1.\tItem\n2.\tItem\n");
    }
    for raw in [vec![0x41, 0x24, 1, 0x41, 0x24, 0], vec![0x41, 0x24, 2]] {
        let mut f = rtl_fixture(0);
        f.definitions.lists[0].levels[0].raw_papx = raw;
        assert_eq!(f.run().unwrap().stories[0].text, "Item\nItem\n");
    }
}

mod inherited;

mod same_style;

fn bullet_format_override() -> LevelOverride {
    LevelOverride {
        level: 0,
        raw_flags: 0x20,
        raw_start_at: i32::MIN,
        start_override: None,
        format_override: Some(level(true)),
    }
}
#[test]
fn simple_bullet_format_override_replaces_complete_base_level() {
    let mut f = Fixture::new(&[1, 1]);
    f.definitions.lists[0].levels[0].number_text_utf16 = vec![42];
    f.definitions.lists[0].levels[0].raw_chpx = vec![0x3c, 0x08, 1];
    let mut over = bullet_format_override();
    over.format_override.as_mut().unwrap().following_character = 2;
    f.definitions.overrides[0].levels.push(over);
    let out = f.run().unwrap();
    assert_eq!(out.stories[0].text, "•Item\n•Item\n");
    partition(&out.stories[0], 10);
}
#[test]
fn formatting_override_ignored_scalar_and_unused_flags_do_not_change_bullets() {
    let mut f = Fixture::new(&[1, 1, 1]);
    let mut over = bullet_format_override();
    over.raw_flags = 0xffff_ffe0;
    f.definitions.overrides[0].levels.push(over);
    assert_eq!(
        f.run().unwrap().stories[0].text,
        "•\tItem\n•\tItem\n•\tItem\n"
    );
}
#[test]
fn bullet_override_ignores_number_sequence_fields() {
    let mut f = Fixture::new(&[1, 1]);
    let mut over = bullet_format_override();
    let level = over.format_override.as_mut().unwrap();
    level.start_at = i32::MIN;
    level.flags |= 8;
    level.restart_limit = u8::MAX;
    f.definitions.overrides[0].levels.push(over);
    assert_eq!(f.run().unwrap().stories[0].text, "•\tItem\n•\tItem\n");
}
#[test]
fn mixed_numbered_family_blocks_override_in_both_orders() {
    for selections in [&[1, 3, 2][..], &[3, 1, 2][..]] {
        let mut f = Fixture::new(selections);
        f.definitions.overrides[0]
            .levels
            .push(bullet_format_override());
        let out = f.run().unwrap();
        assert_eq!(out.stories[0].text, "Item\nItem\n•\tItem\n");
        assert!(
            out.stories[0]
                .diagnostics
                .iter()
                .any(|d| d.code == "list_mixed_bullet_override_sequence_unresolved")
        );
    }
}
#[test]
fn interleaved_bullet_overrides_share_no_numeric_counter() {
    let mut f = Fixture::new(&[1, 3, 1, 2, 3]);
    f.definitions.overrides[0]
        .levels
        .push(bullet_format_override());
    f.definitions.overrides[2]
        .levels
        .push(bullet_format_override());
    let out = f.run().unwrap();
    assert_eq!(out.stories[0].text, "•\tItem\n".repeat(5));
    partition(&out.stories[0], 25);
}
#[test]
fn unsupported_start_format_and_duplicate_override_remain_guarded() {
    for mode in 0..5 {
        let mut f = Fixture::new(&[1, 2]);
        let mut over = bullet_format_override();
        match mode {
            0 => over.raw_flags = 0x30,
            1 => over.format_override = None,
            2 => over.level = 1,
            3 => over.format_override.as_mut().unwrap().number_format = 0,
            _ => over.start_override = Some(1),
        };
        f.definitions.overrides[0].levels.push(over);
        let out = f.run().unwrap();
        assert_eq!(out.stories[0].text, "Item\n•\tItem\n");
    }
    let mut f = Fixture::new(&[1, 2]);
    f.definitions.overrides[0].levels = vec![bullet_format_override(), bullet_format_override()];
    assert_eq!(f.run().unwrap().stories[0].text, "Item\n•\tItem\n");
}
#[test]
fn override_visibility_and_font_guards_preserve_unrelated_lists() {
    for bytes in [
        vec![0x3c, 0x08, 1],
        vec![0x4f, 0x4a, 2, 0, 0x51, 0x4a, 2, 0],
        vec![0x6f, 0x28, 1],
    ] {
        let mut f = Fixture::new(&[1, 2]);
        let mut over = bullet_format_override();
        over.format_override.as_mut().unwrap().raw_chpx = bytes;
        f.definitions.overrides[0].levels.push(over);
        assert_eq!(f.run().unwrap().stories[0].text, "Item\n•\tItem\n");
    }
}
#[test]
fn overriding_level_paragraph_numbering_properties_remain_unsupported() {
    let mut f = Fixture::new(&[1, 2]);
    let mut over = bullet_format_override();
    over.format_override.as_mut().unwrap().raw_papx = vec![0x0b, 0x46, 1, 0];
    f.definitions.overrides[0].levels.push(over);
    assert_eq!(f.run().unwrap().stories[0].text, "Item\n•\tItem\n");
}
#[test]
fn embedded_or_unchecked_fonts_block_new_symbol_overrides() {
    let mut f = Fixture::new(&[1, 2]);
    f.definitions.overrides[0]
        .levels
        .push(bullet_format_override());
    for embedded in [None, Some(true)] {
        let out =
            project_lists_with_font_source(&transcribe(&f.source), &f.context(), embedded).unwrap();
        assert_eq!(out.transcription.stories[0].text, "Item\n•\tItem\n");
    }
}
#[test]
fn unicode_literal_override_keeps_character_semantics_with_embedded_fonts() {
    let mut f = Fixture::new(&[1]);
    let mut over = bullet_format_override();
    over.format_override.as_mut().unwrap().number_text_utf16 = vec![45];
    f.definitions.overrides[0].levels.push(over);
    f.fonts.fonts[0].charset = 0;
    f.fonts.fonts[0].name = "Synthetic Unicode".into();
    assert_eq!(
        project_lists_with_font_source(&transcribe(&f.source), &f.context(), Some(true))
            .unwrap()
            .transcription
            .stories[0]
            .text,
        "-\tItem\n"
    );
}
#[test]
fn newly_enabled_plain_family_members_require_symbol_font_source() {
    for selections in [&[1, 3, 2][..], &[3, 1, 2][..]] {
        for embedded in [None, Some(true)] {
            let mut f = Fixture::new(selections);
            f.definitions.lists[0].levels[0] = level(true);
            f.fonts.fonts.push(Font {
                index: 1,
                name: "Synthetic Unicode".into(),
                alternate_name: None,
                charset: 0,
                family: 0,
                weight: 400,
                panose: vec![],
                signature: vec![],
            });
            let mut over = bullet_format_override();
            let literal = over.format_override.as_mut().unwrap();
            literal.number_text_utf16 = vec![45];
            literal.raw_chpx = vec![0x4f, 0x4a, 1, 0, 0x51, 0x4a, 1, 0];
            f.definitions.overrides[0].levels.push(over);
            let out =
                project_lists_with_font_source(&transcribe(&f.source), &f.context(), embedded)
                    .unwrap();
            assert_eq!(out.transcription.stories[0].text, "Item\nItem\n•\tItem\n");
            f.definitions.lists[0].levels[0].number_text_utf16 = vec![45];
            f.definitions.lists[0].levels[0].raw_chpx = vec![0x4f, 0x4a, 1, 0, 0x51, 0x4a, 1, 0];
            let out =
                project_lists_with_font_source(&transcribe(&f.source), &f.context(), embedded)
                    .unwrap();
            assert_eq!(
                out.transcription.stories[0].text,
                "-\tItem\n-\tItem\n•\tItem\n"
            );
        }
    }
}
#[test]
fn empty_physical_paragraph_retains_source_owned_override_bullet() {
    let mut f = Fixture::new(&[1]);
    f.source.stories[0].text = "\r".into();
    f.source.stories[0].utf16_units = 1;
    f.paragraphs.status = "reference_expanded_endpoint_selected_paragraphs";
    f.paragraphs.spans[0].end_cp = 1;
    f.formatting.authored_story_end_cp = 1;
    f.formatting.pieces[0].end_cp = 1;
    f.formatting.character_spans[0].end_cp = 1;
    f.revisions.spans[0].end_cp = 1;
    f.fields.stories[0].utf16_units = 1;
    f.fields.stories[0].table =
        Some(crate::office::doc::fields::decode(&f.source.stories[0], &[]).unwrap());
    f.definitions.overrides[0]
        .levels
        .push(bullet_format_override());
    let out = f.run().unwrap();
    assert_eq!(out.stories[0].text, "•\t\n");
    assert_eq!(out.stories[0].segments[0].start_cp, 0);
    assert_eq!(out.stories[0].segments[0].end_cp, 0);
    partition(&out.stories[0], 1);
}

fn start_override(index: u8, value: i32) -> LevelOverride {
    LevelOverride {
        level: index,
        raw_flags: u32::from(index) | 0x10,
        raw_start_at: value,
        start_override: Some(value),
        format_override: None,
    }
}

#[test]
fn numeric_start_override_and_switchback_preserve_shared_family_sequence() {
    let mut f = Fixture::new(&[1, 1, 3, 1, 3]);
    f.definitions.overrides[2].levels.push(start_override(0, 4));
    let out = f.run().unwrap();
    assert_eq!(
        out.stories[0].text,
        "1.\tItem\n2.\tItem\n4.\tItem\n5.\tItem\n6.\tItem\n"
    );
    partition(&out.stories[0], 25);
}

#[test]
fn complete_numeric_format_override_ignores_undefined_scalar_start() {
    let mut f = Fixture::new(&[1, 1, 3, 3]);
    let mut custom = level(false);
    custom.start_at = 3;
    custom.number_format = 1;
    custom.number_text_utf16 = vec![40, 0, 41];
    custom.placeholder_offsets[0] = 2;
    f.definitions.overrides[2].levels.push(LevelOverride {
        level: 0,
        raw_flags: 0x30,
        raw_start_at: -987,
        start_override: None,
        format_override: Some(custom),
    });
    assert_eq!(
        f.run().unwrap().stories[0].text,
        "1.\tItem\n2.\tItem\n(III)\tItem\n(IV)\tItem\n"
    );
}

#[test]
fn formatting_only_numeric_override_preserves_base_start_without_new_reset() {
    let mut f = Fixture::new(&[1, 3, 1]);
    let mut custom = level(false);
    custom.start_at = 8;
    f.definitions.overrides[2].levels.push(LevelOverride {
        level: 0,
        raw_flags: 0x20,
        raw_start_at: -1,
        start_override: None,
        format_override: Some(custom),
    });
    assert_eq!(
        f.run().unwrap().stories[0].text,
        "1.\tItem\n2.\tItem\n3.\tItem\n"
    );
}

#[test]
fn numeric_override_lower_restart_and_level_specific_first_use() {
    let mut f = multilevel(&[0, 1, 1, 0, 1]);
    f.definitions.overrides[0].levels = vec![start_override(0, 3), start_override(1, 7)];
    assert_eq!(
        f.run().unwrap().stories[0].text,
        "(3)\tItem\n(7)\tItem\n(8)\tItem\n(4)\tItem\n(1)\tItem\n"
    );
}

#[test]
fn malformed_numeric_overrides_and_hidden_family_members_stay_guarded() {
    for value in [-1, 32768] {
        let mut f = Fixture::new(&[1, 2]);
        f.definitions.overrides[0]
            .levels
            .push(start_override(0, value));
        assert_eq!(f.run().unwrap().stories[0].text, "Item\n•\tItem\n");
    }
    let mut f = Fixture::new(&[1, 3, 2]);
    f.definitions.overrides[0].levels.push(start_override(0, 0));
    f.revisions.spans[0].properties.field_hidden = Some(true);
    assert_eq!(f.run().unwrap().stories[0].text, "Item\nItem\nItem\n");
}

#[test]
fn formatting_only_first_use_and_switchback_keep_nonunit_base_start() {
    let mut f = Fixture::new(&[3, 1, 3]);
    f.definitions.lists[0].levels[0].start_at = 7;
    let mut custom = level(false);
    custom.start_at = 1;
    custom.number_format = 1;
    f.definitions.overrides[2].levels.push(LevelOverride {
        level: 0,
        raw_flags: 0x20,
        raw_start_at: -1,
        start_override: None,
        format_override: Some(custom),
    });
    assert_eq!(
        f.run().unwrap().stories[0].text,
        "VII.\tItem\n8.\tItem\nIX.\tItem\n"
    );
}

#[test]
fn formatting_only_lower_restart_uses_base_counter_initial() {
    let mut f = multilevel(&[1, 1, 0, 1]);
    f.definitions.lists[0].levels[1].start_at = 7;
    let mut custom = multilevel(&[1])
        .definitions
        .lists
        .remove(0)
        .levels
        .remove(1);
    custom.start_at = 2;
    f.definitions.overrides[0].levels.push(LevelOverride {
        level: 1,
        raw_flags: 0x21,
        raw_start_at: -1,
        start_override: None,
        format_override: Some(custom),
    });
    assert_eq!(
        f.run().unwrap().stories[0].text,
        "(7)\tItem\n(8)\tItem\n(1)\tItem\n(7)\tItem\n"
    );
}

#[test]
fn inactive_picture_bullet_properties_recover_ordinary_numbered_labels() {
    let mut f = Fixture::new(&[1, 1]);
    let properties = [0x87, 0x68, 0, 0, 0, 0, 0x88, 0x48, 0, 0];
    f.definitions.lists[0].levels[0].raw_chpx = properties.to_vec();
    f.formatting.character_spans[0].properties = decode_character_modifiers(&properties).unwrap();
    let out = f.run().unwrap();
    assert_eq!(out.stories[0].text, "1.\tItem\n2.\tItem\n");
    partition(&out.stories[0], 10);
}

#[test]
fn picture_bullet_state_is_ordered_and_ignores_only_defined_non_enable_bits() {
    for flags in [0u16, 2, 4, 0xfffe] {
        let mut properties = vec![0x88, 0x48, 1, 0, 0x88, 0x48];
        properties.extend(flags.to_le_bytes());
        assert!(visible_list_modifiers(
            &decode_character_modifiers(&properties).unwrap()
        ));
        properties.extend([0x88, 0x48, 1, 0]);
        assert!(!visible_list_modifiers(
            &decode_character_modifiers(&properties).unwrap()
        ));
    }
    for flags in [1u16, 3, 0xffff] {
        assert!(!visible_list_modifiers(&[PropertyModifier {
            sprm: 0x4888,
            operand: flags.to_le_bytes().to_vec(),
        }]));
    }
}

#[test]
fn picture_bullet_state_rejects_malformed_operands_and_negative_source_positions() {
    for (sprm, operand) in [
        (0x6887, vec![0, 0, 0]),
        (0x6887, vec![0, 0, 0, 0, 0]),
        (0x6887, (-1i32).to_le_bytes().to_vec()),
        (0x4888, vec![0]),
        (0x4888, vec![0, 0, 0]),
    ] {
        assert!(!visible_list_modifiers(&[PropertyModifier {
            sprm,
            operand
        }]));
    }
    assert!(visible_list_modifiers(&[PropertyModifier {
        sprm: 0x6887,
        operand: i32::MAX.to_le_bytes().to_vec(),
    }]));
}

#[test]
fn active_picture_layer_and_hidden_members_still_block_the_whole_list_family() {
    for active_mark in [false, true] {
        let mut f = Fixture::new(&[1, 3]);
        f.definitions.lists[0].levels[0].raw_chpx = vec![0x88, 0x48, u8::from(!active_mark), 0];
        f.formatting.character_spans[0].properties = vec![PropertyModifier {
            sprm: 0x4888,
            operand: vec![u8::from(active_mark), 0],
        }];
        assert_eq!(f.run().unwrap().stories[0].text, "Item\nItem\n");
    }
    let mut f = Fixture::new(&[1, 3]);
    f.definitions.lists[0].levels[0].raw_chpx = vec![0x88, 0x48, 0, 0];
    f.revisions.spans[0].properties.field_hidden = Some(true);
    assert_eq!(f.run().unwrap().stories[0].text, "Item\nItem\n");
}

#[test]
fn picture_bullet_properties_are_still_forbidden_in_style_groups() {
    let mut f = Fixture::new(&[1]);
    f.styles.styles[0]
        .as_mut()
        .unwrap()
        .properties
        .push(StyleProperties {
            kind: "character",
            bytes: vec![0x88, 0x48, 0, 0],
        });
    assert_eq!(f.run().unwrap().stories[0].text, "Item\n");
}

fn table_list_proof(f: &Fixture) -> crate::office::doc::hidden_visibility::TableHiddenEvidence {
    use crate::office::doc::structure::{
        ParagraphEndpoint, ParagraphOwnership, StoryStructure, StructurePlan,
    };
    let units: Vec<_> = f.source.stories[0].text.encode_utf16().collect();
    let structure = StructurePlan {
        status: "synthetic",
        unresolved: [""; 3],
        stories: vec![StoryStructure {
            kind: "body",
            paragraphs: f
                .paragraphs
                .spans
                .iter()
                .map(|p| ParagraphOwnership {
                    source_range: [p.start_cp, p.end_cp],
                    projected_bytes: Some([p.start_cp, p.end_cp]),
                    terminal_unit: Some(units[p.end_cp - 1]),
                    endpoint: Some(ParagraphEndpoint {
                        end_cp: p.end_cp,
                        piece_index: 0,
                        fkp_offset: 0,
                        fkp_run: 0,
                        end_fc: p.end_cp,
                    }),
                    terminal_projection: "synthetic",
                    unresolved: vec![],
                })
                .collect(),
        }],
    };
    crate::office::doc::hidden_visibility::resolve_hidden_evidence_with_tables(
        &f.source,
        &f.formatting,
        &f.paragraphs,
        &f.styles,
        &structure,
        &f.revisions,
    )
    .unwrap()
    .1
}

#[test]
fn qualified_cell_list_continues_into_outside_paragraphs() {
    let mut f = table_fixture();
    f.paragraphs.status = "reference_expanded_endpoint_selected_paragraphs";
    f.paragraphs.spans[0].raw_modifiers[2] = 1;
    f.paragraphs.spans[1]
        .raw_modifiers
        .extend([0x21, 0x76, 0, 1, 10, 0]);
    let proof = table_list_proof(&f);
    assert!(proof.permits_list_paragraph([0, 4]));
    assert!(!proof.permits_list_paragraph([4, 5]));
    assert!(!proof.permits_list_paragraph([1, 4]));
    let input = transcribe(&f.source);
    let out = project_lists_with_sources(&input, &f.context(), None, Some(&proof))
        .unwrap()
        .transcription;
    let story = &out.stories[0];
    let labels: Vec<_> = story
        .segments
        .iter()
        .filter(|s| s.kind == "generated_list_label")
        .map(|s| (s.start_cp, &story.text[s.output_start..s.output_end]))
        .collect();
    assert_eq!(labels, [(0, "1.\t"), (5, "2.\t"), (10, "3.\t")]);
    partition(story, 15);
}

#[test]
fn table_lists_count_source_paragraphs_across_cells_and_outside() {
    let mut f = table_fixture();
    f.paragraphs.status = "reference_expanded_endpoint_selected_paragraphs";
    f.source.stories[0].text = "A\u{7}B\u{7}\u{7}Item\rItem\r".into();
    f.paragraphs.spans[0].end_cp = 2;
    f.paragraphs.spans[0].raw_modifiers[2] = 1;
    let second = ParagraphSpan {
        start_cp: 2,
        end_cp: 4,
        style_index: Some(0),
        raw_modifiers: f.paragraphs.spans[0].raw_modifiers.clone(),
    };
    f.paragraphs.spans.insert(1, second);
    f.paragraphs.spans[2]
        .raw_modifiers
        .extend([0x21, 0x76, 0, 2, 10, 0]);
    let proof = table_list_proof(&f);
    assert!(proof.permits_list_paragraph([0, 2]));
    assert!(proof.permits_list_paragraph([2, 4]));
    assert!(!proof.permits_list_paragraph([0, 4]));
    let out = project_lists_with_sources(&transcribe(&f.source), &f.context(), None, Some(&proof))
        .unwrap()
        .transcription;
    let story = &out.stories[0];
    let labels: Vec<_> = story
        .segments
        .iter()
        .filter(|s| s.kind == "generated_list_label")
        .map(|s| (s.start_cp, &story.text[s.output_start..s.output_end]))
        .collect();
    assert_eq!(
        labels,
        [(0, "1.\t"), (2, "2.\t"), (5, "3.\t"), (10, "4.\t")]
    );
    partition(story, 15);
}

#[test]
fn outside_list_continues_through_cells_and_honors_source_restart() {
    for restarted in [false, true] {
        let mut f = table_fixture();
        f.paragraphs.status = "reference_expanded_endpoint_selected_paragraphs";
        f.source.stories[0].text = "Item\rA\u{7}B\u{7}\u{7}Item\r".into();
        let inside = vec![0x0b, 0x46, 1, 0, 0x16, 0x24, 1, 0x49, 0x66, 1, 0, 0, 0];
        f.paragraphs.spans = [
            (0, 5, false),
            (5, 7, true),
            (7, 9, true),
            (9, 10, true),
            (10, 15, false),
        ]
        .into_iter()
        .map(|(start, end, table)| {
            let mut raw = if table {
                inside.clone()
            } else {
                vec![0x0b, 0x46, 1, 0]
            };
            if start == 9 {
                raw[2] = 0;
                raw.extend([0x17, 0x24, 1, 0x21, 0x76, 0, 2, 10, 0]);
            }
            ParagraphSpan {
                start_cp: start,
                end_cp: end,
                style_index: Some(0),
                raw_modifiers: raw,
            }
        })
        .collect();
        if restarted {
            f.definitions.overrides[1].lsid = f.definitions.overrides[0].lsid;
            f.definitions.overrides[1].list_index = 0;
            f.definitions.overrides[1].levels = vec![start_override(0, 7)];
            f.paragraphs.spans[1].raw_modifiers[2] = 2;
        }
        let proof = table_list_proof(&f);
        let out = project_lists_with_sources(
            &transcribe(&f.source),
            &f.context(),
            Some(false),
            Some(&proof),
        )
        .unwrap()
        .transcription;
        let story = &out.stories[0];
        let labels: Vec<_> = story
            .segments
            .iter()
            .filter(|s| s.kind == "generated_list_label")
            .map(|s| (s.start_cp, &story.text[s.output_start..s.output_end]))
            .collect();
        let expected = if restarted {
            [(0, "1.\t"), (5, "7.\t"), (7, "8.\t"), (10, "9.\t")]
        } else {
            [(0, "1.\t"), (5, "2.\t"), (7, "3.\t"), (10, "4.\t")]
        };
        assert_eq!(labels, expected);
        partition(story, 15);
    }
}

#[test]
fn numbered_row_mark_keeps_entire_family_unresolved() {
    let mut f = table_fixture();
    f.paragraphs.status = "reference_expanded_endpoint_selected_paragraphs";
    f.paragraphs.spans[0].raw_modifiers[2] = 1;
    f.paragraphs.spans[1].raw_modifiers[2] = 1;
    f.paragraphs.spans[1]
        .raw_modifiers
        .extend([0x21, 0x76, 0, 1, 10, 0]);
    let proof = table_list_proof(&f);
    assert!(proof.permits_list_paragraph([0, 4]));
    assert!(!proof.permits_list_paragraph([4, 5]));
    let out = project_lists_with_sources(&transcribe(&f.source), &f.context(), None, Some(&proof))
        .unwrap()
        .transcription;
    assert!(
        !out.stories[0]
            .segments
            .iter()
            .any(|s| s.kind == "generated_list_label")
    );
    assert!(
        out.stories[0]
            .diagnostics
            .iter()
            .any(|d| d.cp == 4 && d.code == "list_sequence_unsupported")
    );
}

#[test]
fn complex_script_weight_toggles_preserve_logical_list_labels() {
    for sprm in [0x085cu16, 0x085d] {
        for value in [0, 1, 0x80, 0x81] {
            for layer in 0..3 {
                let mut f = Fixture::new(&[1, 1]);
                let bytes = [sprm.to_le_bytes().as_slice(), &[value]].concat();
                match layer {
                    0 => f.styles.styles[0]
                        .as_mut()
                        .unwrap()
                        .properties
                        .push(StyleProperties {
                            kind: "character",
                            bytes,
                        }),
                    1 => {
                        f.formatting.character_spans[0].properties =
                            decode_character_modifiers(&bytes).unwrap()
                    }
                    _ => f.definitions.lists[0].levels[0].raw_chpx = bytes,
                }
                let out = f.run().unwrap();
                assert_eq!(out.stories[0].text, "1.\tItem\n2.\tItem\n");
                partition(&out.stories[0], 10);
            }
        }
    }
}

#[test]
fn complex_script_weight_requires_exact_toggle_operands() {
    for sprm in [0x085c, 0x085d] {
        for operand in [
            vec![],
            vec![2],
            vec![0x7f],
            vec![0x82],
            vec![0xff],
            vec![0, 1],
        ] {
            assert!(!visible_modifier(&PropertyModifier {
                sprm,
                operand: operand.clone()
            }));
            let mut f = Fixture::new(&[1, 1]);
            f.formatting.character_spans[0]
                .properties
                .push(PropertyModifier { sprm, operand });
            assert!(
                !f.run().unwrap().stories[0]
                    .segments
                    .iter()
                    .any(|s| s.kind == "generated_list_label")
            );
        }
    }
}

#[test]
fn complex_script_weight_does_not_admit_direction_or_visibility_properties() {
    for unsafe_sprm in [0x0800, 0x0801, 0x0802, 0x083c, 0x085a, 0x0882] {
        let mut f = Fixture::new(&[1, 1]);
        f.formatting.character_spans[0].properties = vec![
            PropertyModifier {
                sprm: 0x085c,
                operand: vec![0x81],
            },
            PropertyModifier {
                sprm: unsafe_sprm,
                operand: vec![1],
            },
            PropertyModifier {
                sprm: 0x085d,
                operand: vec![0],
            },
        ];
        assert!(
            !f.run().unwrap().stories[0]
                .segments
                .iter()
                .any(|s| s.kind == "generated_list_label")
        );
    }
}

#[test]
fn inherited_complex_script_weight_and_direct_resets_preserve_bullet_identity() {
    let mut f = Fixture::new(&[2, 2]);
    f.styles.styles[0]
        .as_mut()
        .unwrap()
        .properties
        .push(StyleProperties {
            kind: "character",
            bytes: vec![0x5c, 0x08, 1, 0x5d, 0x08, 0x81],
        });
    f.styles.styles.push(Some(Style {
        index: 1,
        builtin_id: 0,
        kind: 1,
        base_style: Some(0),
        next_style: 1,
        linked_style: None,
        revision_marked: false,
        name: "Synthetic child".into(),
        raw_flags: 0,
        raw_revision: vec![],
        properties: vec![
            StyleProperties {
                kind: "paragraph",
                bytes: vec![1, 0],
            },
            StyleProperties {
                kind: "character",
                bytes: vec![0x5c, 0x08, 0x81, 0x5d, 0x08, 0x80],
            },
        ],
    }));
    for p in &mut f.paragraphs.spans {
        p.style_index = Some(1);
    }
    f.formatting.character_spans[0].properties =
        decode_character_modifiers(&[0x5c, 0x08, 1, 0x5c, 0x08, 0, 0x5d, 0x08, 0x81]).unwrap();
    f.definitions.lists[1].levels[0]
        .raw_chpx
        .extend([0x5c, 0x08, 0x80, 0x5d, 0x08, 0]);
    let out = f.run().unwrap();
    assert_eq!(out.stories[0].text, "•\tItem\n•\tItem\n");
    partition(&out.stories[0], 10);
    f.fonts.fonts[0].name = "Unresolved synthetic font".into();
    assert!(
        !f.run().unwrap().stories[0]
            .segments
            .iter()
            .any(|s| s.kind == "generated_list_label")
    );
}

#[test]
fn malformed_complex_script_weight_blocks_only_its_list_family() {
    let mut f = Fixture::new(&[1, 2, 1]);
    f.definitions.lists[0].levels[0].raw_chpx = vec![0x5c, 0x08, 2];
    let out = f.run().unwrap();
    let labels: Vec<_> = out.stories[0]
        .segments
        .iter()
        .filter(|s| s.kind == "generated_list_label")
        .map(|s| s.start_cp)
        .collect();
    assert_eq!(labels, [5]);
    partition(&out.stories[0], 15);
}

#[test]
fn proofing_exclusion_toggles_preserve_logical_list_labels() {
    for sprm in [0x0875u16] {
        for value in [0, 1, 0x80, 0x81] {
            for layer in 0..3 {
                let mut f = Fixture::new(&[1, 1]);
                let bytes = [sprm.to_le_bytes().as_slice(), &[value]].concat();
                match layer {
                    0 => f.styles.styles[0]
                        .as_mut()
                        .unwrap()
                        .properties
                        .push(StyleProperties {
                            kind: "character",
                            bytes,
                        }),
                    1 => {
                        f.formatting.character_spans[0].properties =
                            decode_character_modifiers(&bytes).unwrap()
                    }
                    _ => f.definitions.lists[0].levels[0].raw_chpx = bytes,
                }
                let out = f.run().unwrap();
                assert_eq!(out.stories[0].text, "1.\tItem\n2.\tItem\n");
                partition(&out.stories[0], 10);
            }
        }
    }
}

#[test]
fn proofing_exclusion_requires_exact_toggle_operands() {
    for sprm in [0x0875] {
        for operand in [
            vec![],
            vec![2],
            vec![0x7f],
            vec![0x82],
            vec![0xff],
            vec![0, 1],
        ] {
            assert!(!visible_modifier(&PropertyModifier {
                sprm,
                operand: operand.clone()
            }));
            let mut f = Fixture::new(&[1, 1]);
            f.formatting.character_spans[0]
                .properties
                .push(PropertyModifier { sprm, operand });
            assert!(
                !f.run().unwrap().stories[0]
                    .segments
                    .iter()
                    .any(|s| s.kind == "generated_list_label")
            );
        }
    }
}

#[test]
fn proofing_exclusion_does_not_admit_direction_or_visibility_properties() {
    for unsafe_sprm in [0x0800, 0x0801, 0x0802, 0x083c, 0x085a, 0x0882, 0x2a33] {
        let mut f = Fixture::new(&[1, 1]);
        f.formatting.character_spans[0].properties = vec![
            PropertyModifier {
                sprm: 0x0875,
                operand: vec![0x81],
            },
            PropertyModifier {
                sprm: unsafe_sprm,
                operand: vec![1],
            },
            PropertyModifier {
                sprm: 0x0875,
                operand: vec![0],
            },
        ];
        assert!(
            !f.run().unwrap().stories[0]
                .segments
                .iter()
                .any(|s| s.kind == "generated_list_label")
        );
    }
}

#[test]
fn inherited_proofing_exclusion_and_direct_resets_preserve_bullet_identity() {
    let mut f = Fixture::new(&[2, 2]);
    f.styles.styles[0]
        .as_mut()
        .unwrap()
        .properties
        .push(StyleProperties {
            kind: "character",
            bytes: vec![0x75, 0x08, 1, 0x75, 0x08, 0x81],
        });
    f.styles.styles.push(Some(Style {
        index: 1,
        builtin_id: 0,
        kind: 1,
        base_style: Some(0),
        next_style: 1,
        linked_style: None,
        revision_marked: false,
        name: "Synthetic child".into(),
        raw_flags: 0,
        raw_revision: vec![],
        properties: vec![
            StyleProperties {
                kind: "paragraph",
                bytes: vec![1, 0],
            },
            StyleProperties {
                kind: "character",
                bytes: vec![0x75, 0x08, 0x81, 0x75, 0x08, 0x80],
            },
        ],
    }));
    for p in &mut f.paragraphs.spans {
        p.style_index = Some(1);
    }
    f.formatting.character_spans[0].properties =
        decode_character_modifiers(&[0x75, 0x08, 1, 0x75, 0x08, 0, 0x75, 0x08, 0x81]).unwrap();
    f.definitions.lists[1].levels[0]
        .raw_chpx
        .extend([0x75, 0x08, 0x80, 0x75, 0x08, 0]);
    let out = f.run().unwrap();
    assert_eq!(out.stories[0].text, "•\tItem\n•\tItem\n");
    partition(&out.stories[0], 10);
    f.fonts.fonts[0].name = "Unresolved synthetic font".into();
    assert!(
        !f.run().unwrap().stories[0]
            .segments
            .iter()
            .any(|s| s.kind == "generated_list_label")
    );
}

#[test]
fn malformed_proofing_exclusion_blocks_only_its_list_family() {
    let mut f = Fixture::new(&[1, 2, 1]);
    f.definitions.lists[0].levels[0].raw_chpx = vec![0x75, 0x08, 2];
    let out = f.run().unwrap();
    let labels: Vec<_> = out.stories[0]
        .segments
        .iter()
        .filter(|s| s.kind == "generated_list_label")
        .map(|s| s.start_cp)
        .collect();
    assert_eq!(labels, [5]);
    partition(&out.stories[0], 15);
}

#[test]
fn decimal_zero_boundaries_preserve_zero_and_large_values() {
    let mut lvl = level(false);
    lvl.number_format = 0x16;
    let f = Fixture::new(&[1]);
    for (value, expected) in [
        (0, "0.\t"),
        (1, "01.\t"),
        (9, "09.\t"),
        (10, "10.\t"),
        (99, "99.\t"),
        (100, "100.\t"),
        (i32::MAX, "2147483647.\t"),
    ] {
        for legal in [false, true] {
            lvl.flags = if legal { 4 } else { 0 };
            assert_eq!(
                label_text(&lvl, value, &f.fonts, 0, LabelLanguage::default()).unwrap(),
                expected
            );
        }
    }
    assert!(label_text(&lvl, -1, &f.fonts, 0, LabelLanguage::default()).is_err());
}

mod highlight;

#[test]
fn absent_ancestor_does_not_erase_qualified_current_level_numbers() {
    let mut f = multilevel(&[1, 2, 2, 1, 2]);
    let parent = &mut f.definitions.lists[0].levels[1];
    parent.number_text_utf16 = vec![0, 46, 1];
    parent.placeholder_offsets = [1, 3, 0, 0, 0, 0, 0, 0, 0];
    let out = f.run().unwrap();
    assert_eq!(
        out.stories[0].text,
        "Item\n(i)\tItem\n(ii)\tItem\nItem\n(i)\tItem\n"
    );
    assert_eq!(
        out.stories[0]
            .diagnostics
            .iter()
            .filter(|d| d.code == "list_ancestor_number_unresolved")
            .count(),
        2
    );
}

#[test]
fn unsupported_counter_member_still_blocks_the_entire_family() {
    let mut f = multilevel(&[1, 2, 2]);
    let parent = &mut f.definitions.lists[0].levels[1];
    parent.number_text_utf16 = vec![0, 46, 1];
    parent.placeholder_offsets = [1, 3, 0, 0, 0, 0, 0, 0, 0];
    parent.number_format = 0x7f;
    assert_eq!(f.run().unwrap().stories[0].text, "Item\nItem\nItem\n");
}

#[test]
fn literal_unicode_bullets_retain_source_scalars_with_inference_diagnostics() {
    for code in [0x2022, 0x25cf] {
        let mut f = Fixture::new(&[2, 2]);
        f.fonts.fonts[0].charset = 0;
        let level = &mut f.definitions.lists[1].levels[0];
        level.number_text_utf16 = vec![code];
        level.raw_chpx = vec![0x4f, 0x4a, 0, 0, 0x51, 0x4a, 0, 0];
        let out = f.run().unwrap();
        let ch = char::from_u32(code as u32).unwrap();
        assert_eq!(out.stories[0].text, format!("{ch}\tItem\n{ch}\tItem\n"));
        assert_eq!(
            out.stories[0]
                .diagnostics
                .iter()
                .filter(|d| d.code == "list_literal_unicode_bullet_inferred")
                .map(|d| d.cp)
                .collect::<Vec<_>>(),
            vec![0, 5]
        );
    }
}

#[test]
fn literal_unicode_bullets_retain_font_hint_and_scalar_guards() {
    let mut f = Fixture::new(&[2]);
    f.fonts.fonts[0].charset = 0;
    let original = vec![0x4f, 0x4a, 0, 0, 0x51, 0x4a, 0, 0];
    f.definitions.lists[1].levels[0].raw_chpx = original.clone();
    for code in [0x25aa, 0x25a1, 0xf022, 0xd800] {
        f.definitions.lists[1].levels[0].number_text_utf16 = vec![code];
        assert_eq!(f.run().unwrap().stories[0].text, "Item\n");
    }
    f.definitions.lists[1].levels[0].number_text_utf16 = vec![0x2022];
    for raw in [
        vec![0x51, 0x4a, 0, 0],
        vec![0x4f, 0x4a, 1, 0, 0x51, 0x4a, 0, 0],
        [original.clone(), vec![0x6f, 0x28, 1]].concat(),
    ] {
        f.definitions.lists[1].levels[0].raw_chpx = raw;
        assert_eq!(f.run().unwrap().stories[0].text, "Item\n");
    }
    f.definitions.lists[1].levels[0].raw_chpx = original;
    f.fonts.fonts[0].charset = 77;
    assert_eq!(f.run().unwrap().stories[0].text, "Item\n");
}

#[test]
fn literal_unicode_bullet_diagnostic_budget_keeps_unannotated_labels_unresolved() {
    let mut f = Fixture::new(&vec![2; 257]);
    f.fonts.fonts[0].charset = 0;
    let level = &mut f.definitions.lists[1].levels[0];
    level.number_text_utf16 = vec![0x2022];
    level.raw_chpx = vec![0x4f, 0x4a, 0, 0, 0x51, 0x4a, 0, 0];
    let out = f.run().unwrap();
    let story = &out.stories[0];
    let anchors: Vec<_> = story
        .segments
        .iter()
        .filter(|s| s.kind == "generated_list_label")
        .map(|s| s.start_cp)
        .collect();
    let diagnostics: Vec<_> = story
        .diagnostics
        .iter()
        .filter(|d| d.code == "list_literal_unicode_bullet_inferred")
        .map(|d| d.cp)
        .collect();
    assert_eq!(anchors.len(), 256);
    assert_eq!(anchors, diagnostics);
    assert_eq!(story.text, format!("{}Item\n", "•\tItem\n".repeat(256)));
}

mod international;

mod international_next;
