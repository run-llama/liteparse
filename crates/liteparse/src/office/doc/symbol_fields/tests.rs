use super::*;
use crate::office::doc::{
    Story,
    field_inspection::StoryFields,
    fields::{CpRange, FieldRange, FieldTable},
    hidden_visibility::{HiddenProperty, HiddenSpan},
    visibility::{RevisionProperties, RevisionSpan},
};

struct Fixture {
    source: Inspection,
    fields: FieldInspection,
    revisions: RevisionEvidence,
    hidden: HiddenEvidence,
}
impl Fixture {
    fn new(codes: &[(&str, Option<&str>)]) -> Self {
        let mut text = "😀Prefix ".to_owned();
        let mut ranges = Vec::new();
        for (code, cache) in codes {
            let start = 100 + text.encode_utf16().count();
            text.push('\u{13}');
            text.push_str(code);
            let instruction_end = 100 + text.encode_utf16().count();
            let result = cache.map(|cache| {
                text.push('\u{14}');
                let start = 100 + text.encode_utf16().count();
                text.push_str(cache);
                CpRange {
                    start,
                    end: 100 + text.encode_utf16().count(),
                }
            });
            text.push('\u{15}');
            let end = 100 + text.encode_utf16().count();
            ranges.push(FieldRange {
                source: CpRange { start, end },
                instruction: CpRange {
                    start: start + 1,
                    end: instruction_end,
                },
                result,
                parent_begin_cp: None,
            });
        }
        text.push_str(" suffix\r");
        let count = text.encode_utf16().count();
        let spans = text
            .encode_utf16()
            .enumerate()
            .map(|(index, c)| RevisionSpan {
                start_cp: 100 + index,
                end_cp: 101 + index,
                source_span_index: 0,
                piece_index: 0,
                properties: RevisionProperties {
                    special: Some(matches!(c, 0x13..=0x15)),
                    ..Default::default()
                },
                evidence: Vec::new(),
                unresolved: Vec::new(),
            })
            .collect();
        Self {
            source: Inspection {
                schema_version: 1,
                status: "test",
                binary_version: 0x112,
                trailing_container_bytes: 0,
                piece_count: 1,
                stories: vec![Story {
                    kind: "body",
                    start_cp: 100,
                    utf16_units: count,
                    text,
                }],
                trailing_utf16_units: Vec::new(),
            },
            fields: FieldInspection {
                schema_version: 1,
                status: "test",
                binary_version: 0x112,
                unresolved: [""; 3],
                stories: vec![StoryFields {
                    status: "test",
                    kind: "body",
                    start_cp: 100,
                    utf16_units: count,
                    table_present: true,
                    error: None,
                    table: Some(FieldTable {
                        status: "test",
                        events: ranges
                            .iter()
                            .flat_map(|r| {
                                [
                                    crate::office::doc::fields::FieldEvent {
                                        cp: r.source.start,
                                        kind: FieldCharacter::Begin,
                                        metadata: 57,
                                    },
                                    crate::office::doc::fields::FieldEvent {
                                        cp: r.source.end - 1,
                                        kind: FieldCharacter::End,
                                        metadata: 0,
                                    },
                                ]
                            })
                            .collect(),
                        ranges,
                        diagnostics: Vec::new(),
                        topology_valid: true,
                        unindexed_marker_units: 0,
                    }),
                }],
            },
            revisions: RevisionEvidence {
                status: "test",
                basis: "test",
                hidden_text: "test",
                spans,
            },
            hidden: HiddenEvidence {
                status: "test",
                view: "accepted",
                spans: vec![HiddenSpan {
                    start_cp: 100,
                    end_cp: 100 + count,
                    source_span_index: 0,
                    piece_index: 0,
                    paragraph_end_cp: None,
                    paragraph_table_depth: Some(0),
                    table_baseline_basis: "test",
                    property: HiddenProperty {
                        hidden: Some(false),
                        paragraph_style: Some(0),
                        character_style: 10,
                        unresolved: Vec::new(),
                    },
                }],
            },
        }
    }
}

fn fonts() -> FontTable {
    FontTable {
        status: "test",
        fonts: vec![crate::office::doc::symbols::Font {
            index: 0,
            name: "Symbol".into(),
            alternate_name: None,
            charset: 2,
            family: 0,
            weight: 400,
            panose: vec![],
            signature: vec![],
        }],
    }
}
impl Fixture {
    fn inspect(&self) -> SymbolFields {
        inspect(
            &self.source,
            &self.fields,
            &fonts(),
            &self.revisions,
            &self.hidden,
            Some(false),
        )
        .unwrap()
    }
}

#[test]
fn generated_symbols_preserve_text_and_adjacent_field_source_partition() {
    let f = Fixture::new(&[
        (r#"SYMBOL 211 \f "Symbol""#, None),
        (r#"SYMBOL 227 \f "Symbol""#, None),
    ]);
    let fields = f.inspect();
    assert_eq!(fields.fields.len(), 2);
    assert!(fields.fields.iter().all(|f| f.eligible));
    let original = crate::office::doc::transcribe(&f.source);
    let output = apply_symbol_fields(&original, &f.source, &fields).unwrap();
    assert_eq!(output.stories[0].text, "😀Prefix ©© suffix\n");
    let s = &output.stories[0];
    let mut cp = 100;
    let mut byte = 0;
    let mut without = String::new();
    for seg in &s.segments {
        assert_eq!(seg.start_cp, cp);
        assert_eq!(seg.output_start, byte);
        cp = seg.end_cp;
        byte = seg.output_end;
        if seg.kind != "generated_symbol_field" {
            without.push_str(&s.text[seg.output_start..seg.output_end]);
        } else {
            assert_eq!(seg.start_cp, seg.end_cp);
        }
    }
    assert_eq!(without, original.stories[0].text);
    assert_eq!(cp, 100 + f.source.stories[0].utf16_units);
    assert_eq!(byte, s.text.len());
    assert!(apply_symbol_fields(&output, &f.source, &fields).is_err());
}

#[test]
fn cached_nested_hidden_revised_and_stale_symbol_fields_do_not_generate() {
    let code = r#"SYMBOL 211 \f "Symbol""#;
    for case in 0..7 {
        let mut f = Fixture::new(&[(code, if case == 0 { Some("©") } else { None })]);
        let table = f.fields.stories[0].table.as_mut().unwrap();
        match case {
            0 => {}
            1 => table.ranges[0].parent_begin_cp = Some(100),
            2 => f.hidden.spans[0].property.hidden = Some(true),
            3 => f.revisions.spans[10].properties.deleted = Some(true),
            4 => table.events[0].metadata = 1,
            5 => table.events[1].metadata = 1,
            6 => f.revisions.spans[10].properties.special = Some(true),
            _ => unreachable!(),
        }
        assert!(!f.inspect().fields[0].eligible, "case {case}");
    }
}

#[test]
fn source_font_and_embedding_guards_preserve_ambiguity() {
    let f = Fixture::new(&[(r#"SYMBOL 211 \f "Symbol""#, None)]);
    for case in 0..5 {
        let mut fonts = fonts();
        let mut embedded = Some(false);
        match case {
            0 => embedded = Some(true),
            1 => embedded = None,
            2 => fonts.fonts[0].charset = 0,
            3 => fonts.fonts[0].alternate_name = Some("Custom".into()),
            4 => fonts.fonts.clear(),
            _ => unreachable!(),
        }
        assert!(
            !inspect(
                &f.source,
                &f.fields,
                &fonts,
                &f.revisions,
                &f.hidden,
                embedded
            )
            .unwrap()
            .fields[0]
                .eligible
        );
    }
}

#[test]
fn projection_rechecks_story_membership_and_source_instruction() {
    let mut f = Fixture::new(&[(r#"SYMBOL 211 \f "Symbol""#, None)]);
    let fields = f.inspect();
    let input = crate::office::doc::transcribe(&f.source);
    let mut missing = crate::office::doc::transcribe(&f.source);
    missing.stories.clear();
    assert!(apply_symbol_fields(&missing, &f.source, &fields).is_err());
    let mut duplicate = crate::office::doc::transcribe(&f.source);
    duplicate
        .stories
        .push(crate::office::doc::transcribe(&f.source).stories.remove(0));
    assert!(apply_symbol_fields(&duplicate, &f.source, &fields).is_err());
    f.source.stories[0].text = f.source.stories[0].text.replace("211", "227");
    assert!(apply_symbol_fields(&input, &f.source, &fields).is_err());
}

#[test]
fn invalid_source_endpoint_returns_error_without_panicking() {
    let mut f = Fixture::new(&[(r#"SYMBOL 211 \f "Symbol""#, None)]);
    f.fields.stories[0].table.as_mut().unwrap().ranges[0]
        .source
        .end = 0;
    assert!(!f.inspect().fields[0].eligible);
}
