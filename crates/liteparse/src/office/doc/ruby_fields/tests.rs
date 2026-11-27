use super::*;
use crate::office::doc::{
    Story,
    field_inspection::StoryFields,
    fields::{CpRange, FieldRange, FieldTable},
    hidden_visibility::{HiddenProperty, HiddenSpan},
    visibility::{RevisionProperties, RevisionSpan},
};

fn instruction(guide: &str, base: &str) -> String {
    format!(r#"EQ \* jc2 \* "Font:Example" \* hps12 \o\ad(\s\up 11({guide}),{base})"#)
}
struct Fixture {
    source: Inspection,
    fields: FieldInspection,
    revisions: RevisionEvidence,
    hidden: HiddenEvidence,
}
impl Fixture {
    fn new(codes: &[String]) -> Self {
        let mut text = "😀Prefix ".to_owned();
        let mut ranges = Vec::new();
        for code in codes {
            let start = 100 + text.encode_utf16().count();
            text.push('\u{13}');
            text.push_str(code);
            text.push('\u{15}');
            let end = 100 + text.encode_utf16().count();
            ranges.push(FieldRange {
                source: CpRange { start, end },
                instruction: CpRange {
                    start: start + 1,
                    end: end - 1,
                },
                result: None,
                parent_begin_cp: None,
            });
        }
        text.push_str(" suffix\r");
        let count = text.encode_utf16().count();
        let mut spans = Vec::new();
        for (index, c) in text.encode_utf16().enumerate() {
            spans.push(RevisionSpan {
                start_cp: 100 + index,
                end_cp: 101 + index,
                source_span_index: 0,
                piece_index: 0,
                properties: RevisionProperties {
                    special: Some(matches!(c, 0x13 | 0x15)),
                    ..Default::default()
                },
                evidence: Vec::new(),
                unresolved: Vec::new(),
            });
        }
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
    fn inspect(&self) -> RubyInspection {
        inspect_ruby_fields(&self.source, &self.fields, &self.revisions, &self.hidden).unwrap()
    }
}

#[test]
fn restores_base_and_retains_guide_with_utf16_source_and_utf8_output() {
    let f = Fixture::new(&[instruction("よみ😀", "字😀")]);
    let mut metadata = f.inspect();
    assert!(metadata.fields[0].eligible);
    let c = metadata.fields[0].components.as_ref().unwrap();
    assert_eq!(c.base_source_range[1] - c.base_source_range[0], 3);
    assert_eq!(c.guide_source_range[1] - c.guide_source_range[0], 4);
    let input = crate::office::doc::transcribe(&f.source);
    let output = apply_ruby_fields(&input, &f.source, &mut metadata).unwrap();
    assert_eq!(output.stories[0].text, "😀Prefix 字😀 suffix\n");
    assert!(!output.stories[0].text.contains("よみ"));
    let range = metadata.fields[0].projected_base_utf8_range.unwrap();
    assert_eq!(&output.stories[0].text[range[0]..range[1]], "字😀");
    assert!(apply_ruby_fields(&output, &f.source, &mut metadata).is_err());
}
#[test]
fn adjacent_fields_preserve_partition_and_annotations() {
    let f = Fixture::new(&[instruction("one", "A"), instruction("two", " B ")]);
    let mut metadata = f.inspect();
    let output = apply_ruby_fields(
        &crate::office::doc::transcribe(&f.source),
        &f.source,
        &mut metadata,
    )
    .unwrap();
    assert_eq!(output.stories[0].text, "😀Prefix A B  suffix\n");
    let mut cp = 100;
    for s in &output.stories[0].segments {
        assert_eq!(cp, s.start_cp);
        cp = s.end_cp;
    }
    assert_eq!(cp, 100 + f.source.stories[0].utf16_units);
    assert!(
        metadata
            .fields
            .iter()
            .all(|f| f.projected_base_utf8_range.is_some())
    );
}
#[test]
fn supports_documented_alignment_and_above_below_units() {
    for alignment in 0..6 {
        for (operator, position) in [("up", "above"), ("do", "below")] {
            let code = instruction(" guide ", " base ")
                .replace("jc2", &format!("jc{alignment}"))
                .replace("up 11", &format!("{operator}0"));
            let p = grammar::parse(&code).unwrap();
            assert_eq!(p.alignment, alignment);
            assert_eq!(p.position, position);
            assert_eq!(p.shift, 0);
            assert_eq!(p.half_points, 12);
            assert_eq!(&code[p.guide], " guide ");
            assert_eq!(&code[p.base], " base ");
        }
    }
}
#[test]
fn rejects_ambiguous_grammar_without_flattening_operands() {
    let good = instruction("guide", "base");
    for bad in [
        good.replace("jc2", "jc6"),
        good.replace("hps12", "hps0"),
        good.replace("up 11", "up9999999"),
        good.replace("guide", "a,b"),
        good.replace("guide", r"a\,b"),
        good.replace("base", "(base)"),
        good.replace("guide", "\u{13}REF x\u{15}"),
        good.replace("base", ""),
        good.replace("guide", "\n"),
        format!("{good} junk"),
        good.replace("hps12", r"hps12 \* hps14"),
        good.replace("Font:Example", r"Font:Ex\ample"),
        good.replace("guide", r"\s\up1(x)"),
        good.replace("up 11", "up-1"),
        good.replace("base", "\"base\""),
    ] {
        assert!(grammar::parse(&bad).is_err(), "{bad}");
    }
}
#[test]
fn every_truncated_instruction_is_rejected() {
    let good = instruction("guide", "base");
    for end in 0..good.len() {
        assert!(grammar::parse(&good[..end]).is_err());
    }
}
#[test]
fn source_revision_hidden_and_special_guards_remain_required() {
    for mode in 0..5 {
        let mut f = Fixture::new(&[instruction("guide", "base")]);
        match mode {
            0 => f.revisions.spans[10].properties.deleted = Some(true),
            1 => f.hidden.spans[0].property.hidden = Some(true),
            2 => f.hidden.spans[0].property.hidden = None,
            3 => f.revisions.spans[9].properties.special = None,
            _ => f.revisions.spans[10].unresolved.push("unknown".into()),
        }
        let mut m = f.inspect();
        assert!(!m.fields[0].eligible, "mode {mode}");
        assert!(m.fields[0].components.is_some());
        let input = crate::office::doc::transcribe(&f.source);
        let output = apply_ruby_fields(&input, &f.source, &mut m).unwrap();
        assert_eq!(output.stories[0].text, input.stories[0].text);
    }
}
#[test]
fn nested_and_cached_fields_are_not_projected() {
    for mode in 0..2 {
        let mut f = Fixture::new(&[instruction("guide", "base")]);
        let field = &mut f.fields.stories[0].table.as_mut().unwrap().ranges[0];
        if mode == 0 {
            field.parent_begin_cp = Some(1);
        } else {
            field.result = Some(CpRange {
                start: field.instruction.end,
                end: field.instruction.end,
            });
        }
        assert!(!f.inspect().fields[0].eligible);
    }
}
#[test]
fn preserves_existing_output_and_rejects_source_partition_faults() {
    let f = Fixture::new(&[instruction("guide", "base")]);
    for mode in 0..3 {
        let mut m = f.inspect();
        let mut input = crate::office::doc::transcribe(&f.source);
        if mode == 0 {
            input.stories[0].segments[1].start_cp += 1;
        } else if mode == 1 {
            input.stories[0].segments[1].end_cp += 1;
        } else {
            m.fields[0].components.as_mut().unwrap().base = "wrong".into();
        }
        assert!(apply_ruby_fields(&input, &f.source, &mut m).is_err());
        assert!(m.fields[0].projected_base_utf8_range.is_none());
    }
}
#[test]
fn literal_base_split_on_surrogate_is_rejected() {
    let f = Fixture::new(&[instruction("g", "😀")]);
    let mut m = f.inspect();
    let start = m.fields[0].components.as_ref().unwrap().base_source_range[0];
    let mut input = crate::office::doc::transcribe(&f.source);
    let at = input.stories[0]
        .segments
        .iter()
        .position(|s| s.start_cp <= start && start < s.end_cp)
        .unwrap();
    let s = &mut input.stories[0].segments[at];
    let extra = crate::office::doc::transcription::SourceSegment {
        kind: s.kind,
        start_cp: start + 1,
        end_cp: s.end_cp,
        output_start: s.output_start,
        output_end: s.output_end,
    };
    s.end_cp = start + 1;
    input.stories[0].segments.insert(at + 1, extra);
    assert!(apply_ruby_fields(&input, &f.source, &mut m).is_err());
}
#[test]
fn source_owned_existing_output_blocks_only_that_field() {
    let f = Fixture::new(&[instruction("g", "A"), instruction("g", "B")]);
    let mut m = f.inspect();
    let mut input = crate::office::doc::transcribe(&f.source);
    let cp = m.fields[0].components.as_ref().unwrap().base_source_range[0];
    let story = &mut input.stories[0];
    let i = story
        .segments
        .iter()
        .position(|s| s.start_cp <= cp && cp < s.end_cp)
        .unwrap();
    let at = story.segments[i].output_start;
    story.text.insert_str(at, "cached");
    story.segments[i].output_end += 6;
    for s in &mut story.segments[i + 1..] {
        s.output_start += 6;
        s.output_end += 6;
    }
    let output = apply_ruby_fields(&input, &f.source, &mut m).unwrap();
    assert_eq!(output.stories[0].text, "😀Prefix cachedB suffix\n");
    assert!(m.fields[0].projected_base_utf8_range.is_none());
    assert!(m.fields[0].diagnostic.is_some());
    assert!(m.fields[1].projected_base_utf8_range.is_some());
}
#[test]
fn instruction_budget_and_component_properties_are_explicit() {
    let f = Fixture::new(&[instruction(&"x".repeat(16_384), "base")]);
    let m = f.inspect();
    assert!(!m.fields[0].eligible);
    assert!(
        m.fields[0]
            .diagnostic
            .as_ref()
            .unwrap()
            .contains("instruction units")
    );
    let mut f = Fixture::new(&[instruction("guide", "base")]);
    let m = f.inspect();
    let cp = m.fields[0].components.as_ref().unwrap().guide_source_range[0];
    f.revisions.spans[cp - 100].properties.special = Some(true);
    assert!(!f.inspect().fields[0].eligible);
}
