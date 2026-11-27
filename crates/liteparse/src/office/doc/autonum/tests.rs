use super::*;
use crate::office::doc::{
    field_inspection::StoryFields,
    fields,
    formatting_inspection::PieceProperties,
    paragraph_formatting::ParagraphSpan,
    styles::{Style, StyleProperties},
    transcription::transcribe,
    visibility::{RevisionProperties, RevisionSpan},
};

struct Fixture {
    source: Inspection,
    fields: FieldInspection,
    paragraphs: ParagraphInspection,
    styles: Stylesheet,
    formatting: FormattingInspection,
    revisions: RevisionEvidence,
    lists: ListEmulationEvidence,
}
impl Fixture {
    fn new(text: &str) -> Self {
        let units: Vec<_> = text.encode_utf16().collect();
        let source = Inspection {
            schema_version: 1,
            status: "raw_stories_only",
            binary_version: 0xc1,
            trailing_container_bytes: 0,
            piece_count: 1,
            stories: vec![Story {
                kind: "body",
                start_cp: 0,
                utf16_units: units.len(),
                text: text.into(),
            }],
            trailing_utf16_units: vec![],
        };
        let events: Vec<_> = units
            .iter()
            .enumerate()
            .filter(|(_, u)| [0x13, 0x14, 0x15].contains(u))
            .collect();
        let mut plc = Vec::new();
        for (cp, _) in &events {
            plc.extend((*cp as u32).to_le_bytes());
        }
        plc.extend((units.len() as u32).to_le_bytes());
        for (_, u) in &events {
            plc.extend([**u as u8, 0]);
        }
        let table = fields::decode(&source.stories[0], &plc).unwrap();
        let fields = FieldInspection {
            schema_version: 1,
            status: "source_field_metadata_only",
            binary_version: 0xc1,
            stories: vec![StoryFields {
                status: "indexed_cf_spec_unresolved",
                kind: "body",
                start_cp: 0,
                utf16_units: units.len(),
                table_present: true,
                table: Some(table),
                error: None,
            }],
            unresolved: [""; 3],
        };
        let paragraphs = ParagraphInspection {
            schema_version: 1,
            status: "unresolved_paragraph_fragments",
            binary_version: 0xc1,
            excluded_nonstory_cp_range: None,
            spans: units
                .iter()
                .enumerate()
                .filter(|(_, u)| matches!(u, 7 | 13))
                .scan(0, |start, (cp, _)| {
                    let span = ParagraphSpan {
                        start_cp: *start,
                        end_cp: cp + 1,
                        style_index: Some(0),
                        raw_modifiers: vec![],
                    };
                    *start = cp + 1;
                    Some(span)
                })
                .collect(),
            unresolved: [""; 4],
        };
        let styles = Stylesheet {
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
        };
        let formatting = FormattingInspection {
            schema_version: 1,
            status: "unresolved_character_formatting",
            binary_version: 0xc1,
            authored_story_end_cp: units.len(),
            trailing_nonstory_cp_range: None,
            pieces: vec![PieceProperties {
                start_cp: 0,
                end_cp: units.len(),
                file_offset: 0,
                compressed: false,
                raw_flags: 0,
                raw_prm: 0,
            }],
            raw_property_groups: vec![],
            character_spans: vec![crate::office::doc::formatting::CharacterSpan {
                start_cp: 0,
                end_cp: units.len(),
                properties: vec![],
            }],
            unresolved: [""; 3],
        };
        let revisions = RevisionEvidence {
            status: "test",
            basis: "test",
            hidden_text: "unresolved",
            spans: units
                .iter()
                .enumerate()
                .map(|(cp, u)| RevisionSpan {
                    start_cp: cp,
                    end_cp: cp + 1,
                    source_span_index: cp,
                    piece_index: 0,
                    properties: RevisionProperties {
                        special: Some([0x13, 0x14, 0x15].contains(u)),
                        ..Default::default()
                    },
                    evidence: vec![],
                    unresolved: vec![],
                })
                .collect(),
        };
        Self {
            source,
            fields,
            paragraphs,
            styles,
            formatting,
            revisions,
            lists: decode_list_emulation(&[], &[]).unwrap(),
        }
    }
    fn context(&self) -> AutonumContext<'_> {
        AutonumContext {
            source: &self.source,
            fields: &self.fields,
            paragraphs: &self.paragraphs,
            styles: &self.styles,
            formatting: &self.formatting,
            revisions: &self.revisions,
            lists: &self.lists,
        }
    }
    fn run(&self) -> Transcription {
        let raw = transcribe(&self.source);
        let accepted =
            crate::office::doc::revision_projection::apply_revisions(&raw, &self.revisions)
                .unwrap();
        apply_autonum(&accepted, &self.context()).unwrap()
    }
}

#[test]
fn default_body_labels_have_independent_values_and_real_source_ownership() {
    let f = Fixture::new("😀\u{13} AUTONUM \u{15} One\rPlain\r\u{13} AUTONUM \u{15} Two\r");
    let result = f.run();
    let story = &result.stories[0];
    assert_eq!(story.text, "😀1. One\nPlain\n2. Two\n");
    let labels: Vec<_> = story
        .segments
        .iter()
        .filter(|s| s.kind == "generated_number")
        .collect();
    assert_eq!(labels.len(), 2);
    assert_eq!((labels[0].start_cp, labels[0].end_cp), (2, 13));
    assert_eq!(
        &story.text[labels[0].output_start..labels[0].output_end],
        "1."
    );
    validate_projection(story, &f.source.stories[0]).unwrap();
}

#[test]
fn pagination_only_modifier_keeps_sequence() {
    let mut f = Fixture::new("\u{13} AUTONUM \u{15} One\r\u{13} AUTONUM \u{15} Two\r");
    f.paragraphs.spans[0].raw_modifiers = vec![6, 0x24, 1];
    assert_eq!(f.run().stories[0].text, "1. One\n2. Two\n");
}

#[test]
fn unsupported_field_forms_block_the_entire_sequence() {
    for form in [
        " AUTONUM \\s :",
        " AUTONUM \\* ROMAN",
        "AUTONUM\u{14}7",
        "IF X \u{13}AUTONUM\u{15}",
    ] {
        let f = Fixture::new(&format!(
            "\u{13}AUTONUM\u{15} One\r\u{13}{form}\u{15} Two\r"
        ));
        let result = f.run();
        assert!(
            !result.stories[0]
                .segments
                .iter()
                .any(|s| s.kind == "generated_number")
        );
        assert!(
            result.stories[0]
                .diagnostics
                .iter()
                .any(|d| d.code == "autonum_field_form_unsupported")
        );
    }
}

#[test]
fn headings_numbering_properties_and_multiple_fields_are_explicitly_unsupported() {
    for mode in 0..4 {
        let mut f = Fixture::new("\u{13}AUTONUM\u{15} One\r\u{13}AUTONUM\u{15} Two\r");
        match mode {
            0 => f.paragraphs.spans[0].style_index = Some(1),
            1 => f.paragraphs.spans[0].raw_modifiers = vec![0x0a, 0x26, 1],
            2 => f.formatting.pieces[0].raw_prm = 1,
            _ => f.lists.has_autonum_emulation = true,
        }
        assert!(
            !f.run().stories[0]
                .segments
                .iter()
                .any(|s| s.kind == "generated_number")
        );
    }
    let f = Fixture::new("\u{13}AUTONUM\u{15} \u{13}AUTONUM\u{15}\r");
    assert!(
        f.run().stories[0]
            .diagnostics
            .iter()
            .any(|d| d.code == "autonum_paragraph_unsupported")
    );
}

#[test]
fn deleted_unknown_or_unproven_markers_cannot_generate_numbers() {
    for mode in 0..4 {
        let mut f = Fixture::new("\u{13}AUTONUM\u{15} One\r");
        match mode {
            0 => f.revisions.spans[0].properties.deleted = Some(true),
            1 => f.revisions.spans[1].properties.deleted = None,
            2 => f.revisions.spans[0].properties.special = Some(false),
            _ => f.revisions.spans[1].properties.field_hidden = Some(true),
        }
        assert!(
            !f.run().stories[0]
                .segments
                .iter()
                .any(|s| s.kind == "generated_number")
        );
    }
}

#[test]
fn stale_source_or_visible_projection_is_not_overwritten() {
    let mut f = Fixture::new("\u{13}AUTONUM\u{15} One\r");
    f.fields.stories[0].utf16_units += 1;
    assert!(apply_autonum(&transcribe(&f.source), &f.context()).is_err());
    f.fields.stories[0].utf16_units -= 1;
    let mut input = transcribe(&f.source);
    input.stories[0].segments[0].kind = "deleted_revision";
    assert!(
        apply_autonum(&input, &f.context()).unwrap().stories[0]
            .diagnostics
            .iter()
            .any(|d| d.code == "autonum_projection_unresolved")
    );
}

#[test]
fn list_headers_distinguish_absence_customization_and_invalid_data() {
    let mut lst = vec![0; 30];
    lst[0] = 1;
    lst[2] = 7;
    let mut lfo = vec![0; 20];
    lfo[0] = 1;
    lfo[4] = 7;
    assert!(
        !decode_list_emulation(&lst, &lfo)
            .unwrap()
            .has_autonum_emulation
    );
    lst[28] = 4;
    lfo[17] = 0xfe;
    assert!(
        decode_list_emulation(&lst, &lfo)
            .unwrap()
            .has_autonum_emulation
    );
    lfo[17] = 0;
    assert!(decode_list_emulation(&lst, &lfo).is_err());
    assert!(decode_list_emulation(&lst[..29], &[]).is_err());
    assert!(decode_list_emulation(&[], &[0xff; 4]).is_err());
}

#[test]
fn metadata_order_and_bounds_are_checked_before_generation() {
    let mut f = Fixture::new("\u{13}AUTONUM\u{15} One\r");
    f.revisions.spans.swap(0, 1);
    assert!(apply_autonum(&transcribe(&f.source), &f.context()).is_err());
    f.revisions.spans.swap(0, 1);
    f.fields.stories[0].table.as_mut().unwrap().ranges[0]
        .instruction
        .end = usize::MAX;
    assert!(apply_autonum(&transcribe(&f.source), &f.context()).is_err());
    let mut f = Fixture::new("\u{13}AUTONUM\u{15} One\r");
    f.source.stories[0].utf16_units = MAX_UNITS + 1;
    assert!(plan(&f.source.stories[0], &f.context()).is_err());
}

#[test]
fn larger_sequence_preserves_partition_and_decimal_carry() {
    let text = "\u{13}AUTONUM\u{15} Item\r".repeat(2_000);
    let f = Fixture::new(&text);
    let result = f.run();
    let story = &result.stories[0];
    assert!(story.text.starts_with("1. Item\n2. Item\n"));
    assert!(story.text.contains("999. Item\n1000. Item\n"));
    assert!(story.text.ends_with("2000. Item\n"));
    validate_projection(story, &f.source.stories[0]).unwrap();
}

#[test]
fn zero_output_segments_can_span_neighboring_field_boundaries() {
    let f = Fixture::new("\u{13}AUTONUM\u{15}\u{13}OTHER\u{15}\r");
    let result = f.run();
    assert_eq!(result.stories[0].text, "1.\n");
    validate_projection(&result.stories[0], &f.source.stories[0]).unwrap();
}

#[test]
fn hidden_styles_character_overrides_and_unknown_properties_block_sequence() {
    for sprm in [0x083c, 0x4a30, 0xca47, 0x2a33, 0x0866, 0xffff] {
        let mut f = Fixture::new("\u{13}AUTONUM\u{15} One\r");
        f.formatting.character_spans[0]
            .properties
            .push(PropertyModifier {
                sprm,
                operand: vec![1],
            });
        let result = f.run();
        assert!(
            !result.stories[0]
                .segments
                .iter()
                .any(|s| s.kind == "generated_number")
        );
        assert!(
            result.stories[0]
                .diagnostics
                .iter()
                .any(|d| d.code == "autonum_hidden_character_properties_unresolved")
        );
    }
    let mut f = Fixture::new("\u{13}AUTONUM\u{15} One\r");
    f.styles.styles[0]
        .as_mut()
        .unwrap()
        .properties
        .push(StyleProperties {
            kind: "character",
            bytes: vec![0x3c, 0x08, 1],
        });
    assert!(
        f.run().stories[0]
            .diagnostics
            .iter()
            .any(|d| d.code == "autonum_hidden_style_unresolved")
    );
}

#[test]
fn interior_control_does_not_borrow_later_paragraph_endpoint_properties() {
    let mut f = Fixture::new("\u{13}AUTONUM\u{15} One\rTail\r");
    let end = f.paragraphs.spans.last().unwrap().end_cp;
    f.paragraphs.spans.truncate(1);
    f.paragraphs.spans[0].end_cp = end;
    let output = f.run();
    assert!(
        !output.stories[0]
            .segments
            .iter()
            .any(|s| s.kind == "generated_number")
    );
}
