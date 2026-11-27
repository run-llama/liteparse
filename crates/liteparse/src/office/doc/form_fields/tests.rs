fn byte_budget() -> usize {
    super::MAX_BYTES
}
use super::*;
use crate::office::doc::{
    Story,
    field_inspection::StoryFields,
    fields::{CpRange, FieldTable},
    formatting::{CharacterSpan, PropertyModifier},
    formatting_inspection::PieceProperties,
    hidden_visibility::{HiddenProperty, HiddenSpan},
    visibility::{RevisionModifier, RevisionProperties},
};

fn string(out: &mut Vec<u8>, text: &str, terminated: bool) {
    let units: Vec<_> = text.encode_utf16().collect();
    out.extend((units.len() as u16).to_le_bytes());
    for u in units {
        out.extend(u.to_le_bytes());
    }
    if terminated {
        out.extend([0, 0]);
    }
}
fn record(typ: u16, state: u16, default: u16, items: &[&str]) -> Vec<u8> {
    let mut b = vec![255; 4];
    b.extend((typ | (state << 2) | if typ == 2 { 0x8000 } else { 0 }).to_le_bytes());
    b.extend(0u16.to_le_bytes());
    b.extend(if typ == 1 { 20u16 } else { 0 }.to_le_bytes());
    string(&mut b, "Name", true);
    if typ == 0 {
        string(&mut b, "Default", true);
    } else {
        b.extend(default.to_le_bytes());
    }
    for _ in 0..5 {
        string(&mut b, "", true);
    }
    if typ == 2 {
        b.extend([255, 255]);
        b.extend((items.len() as u16).to_le_bytes());
        b.extend([0, 0]);
        for text in items {
            string(&mut b, text, false);
        }
    }
    let mut out = ((b.len() + 68) as u32).to_le_bytes().to_vec();
    out.extend(68u16.to_le_bytes());
    out.resize(68, 0);
    out.extend(b);
    out
}
#[test]
fn current_state_is_separate_from_default_and_undefined() {
    for (state, default, checked) in [(1, 0, true), (0, 1, false), (25, 1, false)] {
        let (f, _) =
            decoder::decode(&record(1, state, default, &[]), 0, &mut byte_budget()).unwrap();
        assert_eq!(f.raw_current_state, state as u8);
        assert_eq!(f.default_state, Some(default));
        assert_eq!(f.effective_checked, Some(checked));
    }
    let (f, _) =
        decoder::decode(&record(2, 1, 0, &["Default", "😀"]), 0, &mut byte_budget()).unwrap();
    assert_eq!(f.selected_text.as_deref(), Some("😀"));
    assert_eq!(f.default_state, Some(0));
    let (f, _) =
        decoder::decode(&record(2, 25, 1, &["One", "Two"]), 0, &mut byte_budget()).unwrap();
    assert_eq!(f.selected_text, None);
    assert_eq!(f.selected_index, None);
    let (f, _) = decoder::decode(&record(0, 0, 0, &[]), 0, &mut byte_budget()).unwrap();
    assert_eq!(f.default_text.as_deref(), Some("Default"));
    assert_eq!(f.selected_text, None);
}
#[test]
fn rejects_malformed_records_and_enforces_aggregate_budget() {
    let b = record(2, 1, 0, &["One", "Two"]);
    for end in 0..b.len() {
        assert!(decoder::decode(&b[..end], 0, &mut byte_budget()).is_err());
    }
    for (offset, value) in [(4, 67), (68, 0), (72, 3), (74, 1), (78, 255)] {
        let mut invalid = b.clone();
        invalid[offset] = value;
        assert!(decoder::decode(&invalid, 0, &mut byte_budget()).is_err());
    }
    assert!(decoder::decode(&record(2, 3, 0, &["One", "Two"]), 0, &mut byte_budget()).is_err());
    assert!(decoder::decode(&record(1, 2, 0, &[]), 0, &mut byte_budget()).is_err());
    assert!(decoder::decode(&b, 0, &mut 1).is_err());
    let mut invalid = b;
    let n = invalid.len();
    invalid[n - 2..].copy_from_slice(&0xd800u16.to_le_bytes());
    assert!(decoder::decode(&invalid, 0, &mut byte_budget()).is_err());
}
struct Fixture {
    source: Inspection,
    fields: FieldInspection,
    formatting: FormattingInspection,
    revisions: RevisionEvidence,
    hidden: HiddenEvidence,
}
impl Fixture {
    fn new() -> Self {
        let text = "😀\u{13} FORMDROPDOWN \u{1}\u{15}!";
        let len = text.encode_utf16().count();
        let end = len - 1;
        Self {
            source: Inspection {
                schema_version: 1,
                status: "test",
                binary_version: 0xc1,
                trailing_container_bytes: 0,
                piece_count: 1,
                trailing_utf16_units: vec![],
                stories: vec![Story {
                    kind: "body",
                    start_cp: 0,
                    utf16_units: len,
                    text: text.into(),
                }],
            },
            fields: FieldInspection {
                schema_version: 1,
                status: "test",
                binary_version: 0xc1,
                unresolved: [""; 3],
                stories: vec![StoryFields {
                    status: "test",
                    kind: "body",
                    start_cp: 0,
                    utf16_units: len,
                    table_present: true,
                    error: None,
                    table: Some(FieldTable {
                        status: "test",
                        events: vec![],
                        ranges: vec![FieldRange {
                            source: CpRange { start: 2, end },
                            instruction: CpRange {
                                start: 3,
                                end: end - 1,
                            },
                            result: None,
                            parent_begin_cp: None,
                        }],
                        diagnostics: vec![],
                        topology_valid: true,
                        unindexed_marker_units: 0,
                    }),
                }],
            },
            formatting: FormattingInspection {
                schema_version: 1,
                status: "test",
                binary_version: 0xc1,
                authored_story_end_cp: len,
                trailing_nonstory_cp_range: None,
                raw_property_groups: vec![],
                unresolved: [""; 3],
                pieces: vec![PieceProperties {
                    start_cp: 0,
                    end_cp: len,
                    file_offset: 0,
                    compressed: false,
                    raw_flags: 0,
                    raw_prm: 0,
                }],
                character_spans: vec![CharacterSpan {
                    start_cp: 0,
                    end_cp: len,
                    properties: vec![PropertyModifier {
                        sprm: 0x0806,
                        operand: vec![1],
                    }],
                }],
            },
            revisions: RevisionEvidence {
                status: "test",
                basis: "test",
                hidden_text: "test",
                spans: vec![RevisionSpan {
                    start_cp: 0,
                    end_cp: len,
                    source_span_index: 0,
                    piece_index: 0,
                    properties: RevisionProperties {
                        special: Some(true),
                        ..Default::default()
                    },
                    evidence: vec![RevisionModifier {
                        source: "chpx",
                        sprm: 0x6a03,
                        operand: vec![0; 4],
                    }],
                    unresolved: vec![],
                }],
            },
            hidden: HiddenEvidence {
                status: "test",
                view: "test",
                spans: vec![HiddenSpan {
                    start_cp: 0,
                    end_cp: len,
                    source_span_index: 0,
                    piece_index: 0,
                    paragraph_end_cp: Some(len),
                    paragraph_table_depth: Some(0),
                    table_baseline_basis: "test",
                    property: HiddenProperty {
                        hidden: Some(false),
                        paragraph_style: Some(0),
                        character_style: 0,
                        unresolved: vec![],
                    },
                }],
            },
        }
    }
    fn inspect(&self) -> FormInspection {
        inspect_stream(
            Some(&record(2, 1, 0, &["Default", "Chosen"])),
            &self.source,
            &self.fields,
            &self.formatting,
            &self.revisions,
            &self.hidden,
        )
        .unwrap()
    }
}
#[test]
fn source_owned_dropdown_inserts_at_utf16_anchor_and_preserves_remainder() {
    let f = Fixture::new();
    let forms = f.inspect();
    assert!(forms.fields[0].eligible);
    let input = crate::office::doc::transcribe(&f.source);
    let out = apply_forms(&input, &forms).unwrap();
    assert_eq!(out.stories[0].text, "😀Chosen!");
    let s = out.stories[0]
        .segments
        .iter()
        .find(|s| s.kind == "generated_form_value")
        .unwrap();
    assert_eq!((s.start_cp, s.end_cp), (2, 2));
    let rest = out.stories[0]
        .segments
        .iter()
        .filter(|s| s.kind != "generated_form_value")
        .map(|s| &out.stories[0].text[s.output_start..s.output_end])
        .collect::<String>();
    assert_eq!(rest, input.stories[0].text);
    assert!(apply_forms(&out, &forms).is_err());
}
#[test]
fn deleted_hidden_nested_and_data_reset_fields_remain_unprojected() {
    for case in 0..6 {
        let mut f = Fixture::new();
        match case {
            0 => f.revisions.spans[0].properties.deleted = Some(true),
            1 => f.hidden.spans[0].property.hidden = Some(true),
            2 => f.hidden.spans[0].property.hidden = None,
            3 => f.fields.stories[0].table.as_mut().unwrap().ranges[0].parent_begin_cp = Some(0),
            4 => f.formatting.character_spans[0]
                .properties
                .push(PropertyModifier {
                    sprm: 0x0806,
                    operand: vec![0],
                }),
            5 => f.formatting.pieces[0].raw_prm = 0x47 << 1,
            _ => unreachable!(),
        }
        let forms = f.inspect();
        assert!(!forms.fields[0].eligible);
        assert!(forms.fields[0].diagnostic.is_some());
        assert_eq!(
            apply_forms(&crate::office::doc::transcribe(&f.source), &forms)
                .unwrap()
                .stories[0]
                .text,
            "😀!"
        );
    }
}

#[test]
fn overlapping_outer_form_is_rejected_even_with_closing_order_field_index() {
    let mut f = Fixture::new();
    let end = f.fields.stories[0].table.as_ref().unwrap().ranges[0]
        .source
        .end;
    f.fields.stories[0].table.as_mut().unwrap().ranges.insert(
        0,
        FieldRange {
            source: CpRange { start: 5, end: 8 },
            instruction: CpRange { start: 6, end: 7 },
            result: None,
            parent_begin_cp: Some(2),
        },
    );
    let forms = f.inspect();
    assert_eq!(forms.fields.len(), 1);
    assert_eq!(forms.fields[0].field_end_cp, end);
    assert!(!forms.fields[0].eligible);
    assert!(
        forms.fields[0]
            .diagnostic
            .as_ref()
            .unwrap()
            .contains("nested")
    );
}
#[test]
fn existing_cached_output_is_preserved_and_undefined_has_no_default_insertion() {
    let f = Fixture::new();
    let mut forms = f.inspect();
    forms.fields[0].data.as_mut().unwrap().selected_text = None;
    let input = crate::office::doc::transcribe(&f.source);
    assert_eq!(apply_forms(&input, &forms).unwrap().stories[0].text, "😀!");
    forms.fields[0].data.as_mut().unwrap().selected_text = Some("New".into());
    let mut input = crate::office::doc::transcribe(&f.source);
    let s = input.stories[0]
        .segments
        .iter_mut()
        .find(|s| s.start_cp == 2)
        .unwrap();
    s.output_end += 1;
    assert!(apply_forms(&input, &forms).is_err());
}

#[test]
fn binary_marker_field_hidden_is_expected_and_piece_cfdata_override_is_ordered() {
    let mut f = Fixture::new();
    let end = f.fields.stories[0].table.as_ref().unwrap().ranges[0]
        .source
        .end;
    let len = f.source.stories[0].utf16_units;
    f.revisions.spans = [
        (0, end - 2, false),
        (end - 2, end - 1, true),
        (end - 1, len, false),
    ]
    .into_iter()
    .map(|(start_cp, end_cp, hidden)| RevisionSpan {
        start_cp,
        end_cp,
        source_span_index: 0,
        piece_index: 0,
        properties: RevisionProperties {
            special: Some(true),
            field_hidden: Some(hidden),
            ..Default::default()
        },
        evidence: vec![RevisionModifier {
            source: "chpx",
            sprm: 0x6a03,
            operand: vec![0; 4],
        }],
        unresolved: vec![],
    })
    .collect();
    f.formatting.character_spans[0].properties[0].operand = vec![0];
    f.formatting.pieces[0].raw_prm = 0x100 | (0x47 << 1);
    assert!(f.inspect().fields[0].eligible);
    f.formatting.pieces[0].raw_prm = 0x200 | (0x47 << 1);
    assert!(!f.inspect().fields[0].eligible);
}
#[test]
fn dropdown_count_limit_and_nil_ignored_bytes_are_distinct() {
    let mut b = record(2, 24, 0, &["Item"; 25]);
    b[12] = 99;
    assert_eq!(
        decoder::decode(&b, 0, &mut byte_budget())
            .unwrap()
            .0
            .items
            .len(),
        25
    );
    assert!(decoder::decode(&record(2, 24, 0, &["Item"; 26]), 0, &mut byte_budget()).is_err());
    let mut f = Fixture::new();
    f.source.stories[0].text = f.source.stories[0]
        .text
        .replace("FORMDROPDOWN ", "FORMDROPDOWNx");
    assert!(f.inspect().fields.is_empty());
}

#[test]
fn checkbox_projection_normalizes_current_state_and_preserves_source_metadata() {
    let f = Fixture::new();
    let input = crate::office::doc::transcribe(&f.source);
    for (state, default, glyph) in [(0, 1, "☐"), (1, 0, "☒"), (25, 1, "☐")] {
        let mut forms = f.inspect();
        forms.fields[0].data = Some(
            decoder::decode(&record(1, state, default, &[]), 0, &mut byte_budget())
                .unwrap()
                .0,
        );
        forms.fields[0].field_type = "checkbox";
        let before = serde_json::to_value(&forms).unwrap();
        let out = apply_forms(&input, &forms).unwrap();
        assert_eq!(out.stories[0].text, format!("😀{glyph}!"));
        assert_eq!(serde_json::to_value(&forms).unwrap(), before);
        let segment = out.stories[0]
            .segments
            .iter()
            .find(|s| s.kind == "generated_form_value")
            .unwrap();
        assert_eq!((segment.start_cp, segment.end_cp), (2, 2));
        assert_eq!(segment.output_end - segment.output_start, 3);
        forms.fields[0].eligible = false;
        assert_eq!(
            apply_forms(&input, &forms).unwrap().stories[0].text,
            input.stories[0].text
        );
    }
}

#[test]
fn checkbox_projection_rejects_inconsistent_state_and_existing_output() {
    let f = Fixture::new();
    let mut forms = f.inspect();
    forms.fields[0].data = Some(
        decoder::decode(&record(1, 1, 0, &[]), 0, &mut byte_budget())
            .unwrap()
            .0,
    );
    forms.fields[0].field_type = "checkbox";
    let input = crate::office::doc::transcribe(&f.source);
    forms.fields[0].data.as_mut().unwrap().raw_current_state = 2;
    assert!(apply_forms(&input, &forms).is_err());
    forms.fields[0].data.as_mut().unwrap().raw_current_state = 0;
    assert!(apply_forms(&input, &forms).is_err());
    forms.fields[0].data.as_mut().unwrap().raw_current_state = 1;
    let mut cached = crate::office::doc::transcribe(&f.source);
    let segment = cached.stories[0]
        .segments
        .iter_mut()
        .find(|s| s.start_cp == 2)
        .unwrap();
    segment.output_end += 1;
    assert!(apply_forms(&cached, &forms).is_err());
}

#[test]
fn source_checkbox_visibility_and_deletion_guards_precede_normalized_glyphs() {
    for case in 0..5 {
        let mut f = Fixture::new();
        f.source.stories[0].text = f.source.stories[0]
            .text
            .replace("FORMDROPDOWN", "FORMCHECKBOX");
        match case {
            1 => f.revisions.spans[0].properties.deleted = Some(true),
            2 => f.hidden.spans[0].property.hidden = Some(true),
            3 => f.hidden.spans[0].property.hidden = None,
            4 => f.fields.stories[0].table.as_mut().unwrap().ranges[0].parent_begin_cp = Some(0),
            _ => (),
        }
        let forms = inspect_stream(
            Some(&record(1, 1, 0, &[])),
            &f.source,
            &f.fields,
            &f.formatting,
            &f.revisions,
            &f.hidden,
        )
        .unwrap();
        assert_eq!(forms.fields[0].eligible, case == 0);
        assert_eq!(forms.fields[0].data.as_ref().unwrap().raw_current_state, 1);
        let out = apply_forms(&crate::office::doc::transcribe(&f.source), &forms).unwrap();
        assert_eq!(out.stories[0].text, if case == 0 { "😀☒!" } else { "😀!" });
    }
}

#[test]
fn blocked_checkbox_keeps_cached_text_and_does_not_block_safe_dropdown() {
    use crate::office::doc::transcription::{SourceSegment, TranscribedStory, Transcription};
    for cached in [false, true] {
        let f = Fixture::new();
        let mut forms = f.inspect();
        let mut checkbox = f.inspect().fields.remove(0);
        checkbox.field_start_cp = 1;
        checkbox.field_end_cp = 3;
        checkbox.field_type = "checkbox";
        checkbox.data = Some(
            decoder::decode(&record(1, 1, 0, &[]), 0, &mut byte_budget())
                .unwrap()
                .0,
        );
        forms.fields[0].field_start_cp = 3;
        forms.fields[0].field_end_cp = 5;
        forms.fields.insert(0, checkbox);
        let input = Transcription {
            schema_version: 1,
            status: "test",
            unresolved: vec![],
            stories: vec![TranscribedStory {
                kind: "body",
                text: if cached { "X" } else { "" }.into(),
                segments: vec![
                    SourceSegment {
                        kind: "field_instruction",
                        start_cp: 0,
                        end_cp: if cached { 1 } else { 2 },
                        output_start: 0,
                        output_end: 0,
                    },
                    SourceSegment {
                        kind: "text",
                        start_cp: if cached { 1 } else { 2 },
                        end_cp: 3,
                        output_start: 0,
                        output_end: usize::from(cached),
                    },
                    SourceSegment {
                        kind: "field_marker",
                        start_cp: 3,
                        end_cp: 5,
                        output_start: usize::from(cached),
                        output_end: usize::from(cached),
                    },
                ],
                diagnostics: vec![],
                fields_balanced: true,
                unresolved_control_units: 0,
            }],
        };
        let out = apply_forms(&input, &forms).unwrap();
        assert_eq!(
            out.stories[0].text,
            if cached { "XChosen" } else { "Chosen" }
        );
        assert_eq!(out.stories[0].diagnostics.len(), 1);
        assert_eq!(out.stories[0].diagnostics[0].cp, 1);
        assert_eq!(
            out.stories[0].diagnostics[0].code,
            "form_value_existing_output_or_ownership"
        );
        assert_eq!(
            out.stories[0]
                .segments
                .iter()
                .filter(|s| s.kind == "generated_form_value")
                .count(),
            1
        );
        assert!(
            out.unresolved
                .contains(&"form_value_existing_output_or_ownership")
        );
    }
}

#[test]
fn adjacent_field_markers_split_only_at_proven_empty_boundaries() {
    use crate::office::doc::transcription::{SourceSegment, TranscribedStory, Transcription};
    let f = Fixture::new();
    let mut forms = f.inspect();
    let mut checkbox = f.inspect().fields.remove(0);
    checkbox.field_start_cp = 1;
    checkbox.field_end_cp = 3;
    checkbox.field_type = "checkbox";
    checkbox.data = Some(
        decoder::decode(&record(1, 25, 1, &[]), 0, &mut byte_budget())
            .unwrap()
            .0,
    );
    forms.fields[0].field_start_cp = 3;
    forms.fields[0].field_end_cp = 5;
    forms.fields.insert(0, checkbox);
    let mut input = Transcription {
        schema_version: 1,
        status: "test",
        unresolved: vec![],
        stories: vec![TranscribedStory {
            kind: "body",
            text: String::new(),
            segments: vec![SourceSegment {
                kind: "field_marker",
                start_cp: 0,
                end_cp: 6,
                output_start: 0,
                output_end: 0,
            }],
            diagnostics: vec![],
            fields_balanced: true,
            unresolved_control_units: 0,
        }],
    };
    let out = apply_forms(&input, &forms).unwrap();
    assert_eq!(out.stories[0].text, "☐Chosen");
    assert!(out.stories[0].diagnostics.is_empty());
    let markers: Vec<_> = out.stories[0]
        .segments
        .iter()
        .filter(|s| s.kind == "field_marker")
        .collect();
    assert_eq!(
        markers
            .iter()
            .map(|s| (s.start_cp, s.end_cp))
            .collect::<Vec<_>>(),
        [(0, 1), (1, 3), (3, 5), (5, 6)]
    );
    assert!(markers.iter().all(|s| s.output_start == s.output_end));
    assert!(apply_forms(&out, &forms).is_err());
    input.stories[0].segments[0].kind = "field_instruction";
    let blocked = apply_forms(&input, &forms).unwrap();
    assert_eq!(blocked.stories[0].text, "");
    assert_eq!(blocked.stories[0].segments.len(), 1);
    assert_eq!(blocked.stories[0].diagnostics.len(), 2);
}

#[test]
fn marker_split_rejects_source_gaps_overlaps_and_nonempty_marker_output() {
    use crate::office::doc::transcription::SourceSegment;
    let f = Fixture::new();
    let forms = f.inspect();
    for delta in [0usize, 2] {
        let mut input = crate::office::doc::transcribe(&f.source);
        let index = input.stories[0]
            .segments
            .iter()
            .position(|s| s.start_cp == 2)
            .unwrap();
        input.stories[0].segments[index].start_cp = delta + 1;
        assert!(apply_forms(&input, &forms).is_err());
    }
    let boundaries = [1usize, 2].into_iter().collect();
    let input = [SourceSegment {
        kind: "field_marker",
        start_cp: 0,
        end_cp: 3,
        output_start: 0,
        output_end: 1,
    }];
    let out = projection::split_markers(&input, &boundaries, 0).unwrap();
    assert_eq!(out.len(), 1);
    assert_eq!((out[0].start_cp, out[0].end_cp), (0, 3));
}

fn localized_fixture() -> Fixture {
    let mut f = Fixture::new();
    // Match the original fixture's UTF-16 extent so every property range stays exact.
    f.source.stories[0].text = "😀\u{13}FORMCASECOCHER\u{1}\u{15}!".into();
    let table = f.fields.stories[0].table.as_mut().unwrap();
    table.events = vec![
        crate::office::doc::fields::FieldEvent {
            cp: 2,
            kind: FieldCharacter::Begin,
            metadata: 71,
        },
        crate::office::doc::fields::FieldEvent {
            cp: table.ranges[0].source.end - 1,
            kind: FieldCharacter::End,
            metadata: 0x24,
        },
    ];
    f
}
fn localized_inspect(f: &Fixture, data: &[u8]) -> FormInspection {
    inspect_stream(
        Some(data),
        &f.source,
        &f.fields,
        &f.formatting,
        &f.revisions,
        &f.hidden,
    )
    .unwrap()
}
#[test]
fn localized_checkbox_identity_preserves_flags_and_uses_owned_current_state() {
    for flags in [0, 4, 0x10, 0x20, 0x24, 0x34] {
        let mut f = localized_fixture();
        f.fields.stories[0].table.as_mut().unwrap().events[1].metadata = flags;
        let forms = localized_inspect(&f, &record(1, 1, 0, &[]));
        let item = &forms.fields[0];
        assert!(item.eligible, "{:?}", item.diagnostic);
        let identity = item.identity.as_ref().unwrap();
        assert!(identity.qualified);
        assert_eq!(identity.end_flags, Some(flags));
        assert_eq!(identity.cached_type, Some(71));
        assert_eq!(
            String::from_utf16(&identity.raw_instruction_utf16).unwrap(),
            "FORMCASECOCHER\u{1}"
        );
        assert_eq!(
            apply_forms(&crate::office::doc::transcribe(&f.source), &forms)
                .unwrap()
                .stories[0]
                .text,
            "😀☒!"
        );
    }
}
#[test]
fn localized_identity_rejects_stale_type_edited_result_and_payload_mismatch() {
    for case in 0..8 {
        let mut f = localized_fixture();
        let mut data = record(1, 0, 1, &[]);
        let table = f.fields.stories[0].table.as_mut().unwrap();
        match case {
            0 => table.events[0].metadata = 70,
            1 => table.events.clear(),
            2 => table.events[1].metadata |= 8,
            3 => table.events[1].metadata |= 2,
            4 => table.events[1].metadata |= 1,
            5 => table.events[1].metadata |= 0x80,
            6 => table.ranges[0].result = Some(CpRange { start: 16, end: 16 }),
            7 => data = record(0, 0, 0, &[]),
            _ => unreachable!(),
        }
        let forms = localized_inspect(&f, &data);
        assert!(!forms.fields[0].eligible);
        assert!(!forms.fields[0].identity.as_ref().unwrap().qualified);
        assert!(forms.fields[0].diagnostic.is_some());
        assert_eq!(
            apply_forms(&crate::office::doc::transcribe(&f.source), &forms)
                .unwrap()
                .stories[0]
                .text,
            "😀!"
        );
    }
}
#[test]
fn localized_alias_is_exact_and_does_not_dispatch_by_cached_type_alone() {
    for token in [
        "FORMCASECOCHERX",
        "FORMCASECOCHE",
        "OTHER",
        "FORMCASECOCHER\u{2}",
    ] {
        assert!(field_kind(&token.encode_utf16().collect::<Vec<_>>()).is_none());
    }
    assert_eq!(
        field_kind(&" formcasecocher \u{1}".encode_utf16().collect::<Vec<_>>()),
        Some(("checkbox", "FORMCASECOCHER"))
    );
    let mut f = localized_fixture();
    f.source.stories[0].text = f.source.stories[0]
        .text
        .replace("FORMCASECOCHER", "UNKNOWNCODEXXX");
    assert!(
        localized_inspect(&f, &record(1, 1, 0, &[]))
            .fields
            .is_empty()
    );
}
#[test]
fn localized_binary_state_retains_hidden_deleted_nested_and_marker_guards() {
    for case in 0..5 {
        let mut f = localized_fixture();
        match case {
            0 => f.revisions.spans[0].properties.deleted = Some(true),
            1 => f.hidden.spans[0].property.hidden = Some(true),
            2 => f.hidden.spans[0].property.hidden = None,
            3 => f.fields.stories[0].table.as_mut().unwrap().ranges[0].parent_begin_cp = Some(0),
            4 => f.formatting.character_spans[0].properties[0].operand = vec![0],
            _ => unreachable!(),
        }
        let forms = localized_inspect(&f, &record(1, 1, 0, &[]));
        assert!(!forms.fields[0].eligible);
        assert_eq!(
            apply_forms(&crate::office::doc::transcribe(&f.source), &forms)
                .unwrap()
                .stories[0]
                .text,
            "😀!"
        );
    }
}

#[test]
fn localized_versionless_checkbox_uses_same_typed_identity() {
    let f = localized_fixture();
    let mut data = vec![0; 68];
    data.extend([
        101, 0, 0, 0, 20, 0, 0, 0, 4, b'N', b'a', b'm', b'e', 0, 1, 0,
    ]);
    data.extend([0; 10]);
    let n = data.len() as u32;
    data[..4].copy_from_slice(&n.to_le_bytes());
    data[4..6].copy_from_slice(&68u16.to_le_bytes());
    let forms = localized_inspect(&f, &data);
    let field = &forms.fields[0];
    assert!(field.eligible);
    assert!(field.identity.as_ref().unwrap().qualified);
    assert_eq!(field.data.as_ref().unwrap().raw_current_state, 25);
    assert_eq!(field.data.as_ref().unwrap().default_state, Some(1));
    assert!(field.data.as_ref().unwrap().legacy_encoding.is_some());
    assert_eq!(
        apply_forms(&crate::office::doc::transcribe(&f.source), &forms)
            .unwrap()
            .stories[0]
            .text,
        "😀☐!"
    );
}

#[test]
fn oversized_english_and_localized_instructions_retain_fatal_resource_limit() {
    for keyword in ["FORMCHECKBOX", "FORMCASECOCHER"] {
        let mut f = localized_fixture();
        let text = format!("😀\u{13}{keyword}{}\u{1}\u{15}!", " ".repeat(1025));
        let len = text.encode_utf16().count();
        f.source.stories[0].text = text;
        f.source.stories[0].utf16_units = len;
        let story = &mut f.fields.stories[0];
        story.utf16_units = len;
        let table = story.table.as_mut().unwrap();
        table.ranges[0].source.end = len - 1;
        table.ranges[0].instruction.end = len - 2;
        table.events[1].cp = len - 2;
        assert!(matches!(
            inspect_stream(
                Some(&record(1, 1, 0, &[])),
                &f.source,
                &f.fields,
                &f.formatting,
                &f.revisions,
                &f.hidden
            ),
            Err(Error::Limit("form instruction units"))
        ));
    }
}
