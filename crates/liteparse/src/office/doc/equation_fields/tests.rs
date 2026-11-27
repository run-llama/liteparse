use super::*;
use crate::office::doc::{
    Story,
    field_inspection::StoryFields,
    fields::{CpRange, FieldRange, FieldTable},
    hidden_visibility::{HiddenProperty, HiddenSpan},
    visibility::{RevisionProperties, RevisionSpan},
};

const LITERAL: &str = " EQ \\o\\ad(visible,    )";
const RUBY: &str = r#"EQ \* jc2 \* "Font:Example" \* hps12 \o\ad(\s\up11(guide),base)"#;
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
                        events: Vec::new(),
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
    fn equations(&self) -> EquationInspection {
        inspect_equations(&self.source, &self.fields, &self.revisions, &self.hidden).unwrap()
    }
    fn ruby(&self) -> RubyInspection {
        crate::office::doc::ruby_fields::inspect_ruby_fields(
            &self.source,
            &self.fields,
            &self.revisions,
            &self.hidden,
        )
        .unwrap()
    }
}
#[test]
fn empty_cached_literal_retains_separator_and_exact_unicode_ownership() {
    let code = LITERAL.replace("visible", "字😀");
    let f = Fixture::new(&[(&code, Some(""))]);
    let mut e = f.equations();
    assert!(e.fields[0].eligible);
    let input = crate::office::doc::transcribe(&f.source);
    assert_eq!(input.stories[0].text, "😀Prefix  suffix\n");
    let output = apply_fields(&input, &f.source, None, Some(&mut e)).unwrap();
    assert_eq!(output.stories[0].text, "😀Prefix 字😀 suffix\n");
    let range = e.fields[0].literal_source_range.unwrap();
    assert_eq!(range[1] - range[0], 3);
    let separator = e.fields[0].separator_cp.unwrap();
    let marker = output.stories[0]
        .segments
        .iter()
        .find(|s| s.start_cp <= separator && separator < s.end_cp)
        .unwrap();
    assert_eq!(marker.kind, "field_marker");
    assert_eq!(marker.output_start, marker.output_end);
    assert!(apply_fields(&output, &f.source, None, Some(&mut e)).is_err());
}
#[test]
fn empty_cached_ruby_and_eq_share_final_offsets_in_both_orders() {
    for codes in [
        vec![(LITERAL, Some("")), (RUBY, Some(""))],
        vec![(RUBY, Some("")), (LITERAL, Some(""))],
    ] {
        let f = Fixture::new(&codes);
        let mut r = f.ruby();
        let mut e = f.equations();
        assert_eq!(r.fields.len(), 1);
        assert!(r.fields[0].eligible);
        assert_eq!(e.fields.len(), 1);
        assert!(e.fields[0].eligible);
        let output = apply_fields(
            &crate::office::doc::transcribe(&f.source),
            &f.source,
            Some(&mut r),
            Some(&mut e),
        )
        .unwrap();
        let rb = r.fields[0].projected_base_utf8_range.unwrap();
        let eq = e.fields[0].projected_literal_utf8_range.unwrap();
        assert_eq!(&output.stories[0].text[rb[0]..rb[1]], "base");
        assert_eq!(&output.stories[0].text[eq[0]..eq[1]], "visible");
        assert!(!output.stories[0].text.contains("guide"));
    }
}
#[test]
fn nonempty_cache_is_preserved_for_both_grammars() {
    let f = Fixture::new(&[(LITERAL, Some("cached")), (RUBY, Some("cached ruby"))]);
    let mut r = f.ruby();
    let mut e = f.equations();
    assert!(e.fields.is_empty());
    assert!(!r.fields[0].eligible);
    let input = crate::office::doc::transcribe(&f.source);
    let output = apply_fields(&input, &f.source, Some(&mut r), Some(&mut e)).unwrap();
    assert_eq!(
        output.stories[0].text,
        "😀Prefix cachedcached ruby suffix\n"
    );
}
#[test]
fn source_separator_properties_and_all_visibility_are_required() {
    for code in [LITERAL, RUBY] {
        for mode in 0..7 {
            let mut f = Fixture::new(&[(code, Some(""))]);
            let sep = f.fields.stories[0].table.as_ref().unwrap().ranges[0]
                .instruction
                .end;
            match mode {
                0 => f.revisions.spans[sep - 100].properties.special = Some(false),
                1 => f.revisions.spans[sep - 100].properties.special = None,
                2 => f.revisions.spans[sep - 100].properties.field_hidden = Some(true),
                3 => f.revisions.spans[sep - 100].properties.deleted = Some(true),
                4 => f.hidden.spans[0].property.hidden = Some(true),
                5 => f.revisions.spans[sep - 100].properties.field_hidden = None,
                _ => f.hidden.spans[0].property.hidden = None,
            }
            if code == RUBY {
                assert!(!f.ruby().fields[0].eligible);
            } else {
                assert!(!f.equations().fields[0].eligible);
            }
        }
    }
}
#[test]
fn malformed_empty_result_topology_and_nested_fields_stay_guarded() {
    for code in [LITERAL, RUBY] {
        for mode in 0..4 {
            let mut f = Fixture::new(&[(code, Some(""))]);
            let field = &mut f.fields.stories[0].table.as_mut().unwrap().ranges[0];
            match mode {
                0 => {
                    let cache = field.result.as_mut().unwrap();
                    cache.start -= 1;
                    cache.end -= 1;
                }
                1 => field.source.end += 1,
                2 => field.parent_begin_cp = Some(1),
                _ => field.source.start += 1,
            }
            if code == RUBY {
                assert!(!f.ruby().fields[0].eligible);
            } else {
                assert!(!f.equations().fields[0].eligible);
            }
        }
    }
}
#[test]
fn unindexed_or_invalid_topology_cannot_recover_empty_result() {
    let mut f = Fixture::new(&[(LITERAL, Some(""))]);
    f.fields.stories[0].table.as_mut().unwrap().topology_valid = false;
    let mut e = f.equations();
    assert!(e.fields.is_empty());
    let input = crate::office::doc::transcribe(&f.source);
    let output = apply_fields(&input, &f.source, None, Some(&mut e)).unwrap();
    assert_eq!(input.stories[0].text, output.stories[0].text);
}
#[test]
fn unsupported_nested_composite_keeps_other_safe_literals() {
    let f = Fixture::new(&[(r"EQ \o\ac(a,\f(b,c))", Some("")), (LITERAL, Some(""))]);
    let mut e = f.equations();
    assert!(!e.fields[0].eligible);
    assert!(e.fields[1].eligible);
    let output = apply_fields(
        &crate::office::doc::transcribe(&f.source),
        &f.source,
        None,
        Some(&mut e),
    )
    .unwrap();
    assert_eq!(output.stories[0].text, "😀Prefix visible suffix\n");
}

mod composite;
