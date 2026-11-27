use super::*;
use crate::office::doc::{
    SourceSegment, Story, TranscribedStory,
    formatting::{CharacterSpan, PropertyModifier},
    formatting_inspection::PieceProperties,
    hidden_visibility::{HiddenProperty, HiddenSpan},
    styles::{Style, StyleProperties},
    visibility::{RevisionProperties, RevisionSpan},
};
fn p(sprm: u16, value: u8) -> PropertyModifier {
    PropertyModifier {
        sprm,
        operand: vec![value],
    }
}
fn sheet() -> Stylesheet {
    Stylesheet {
        schema_version: 1,
        status: "fixture",
        std_base_bytes: 10,
        raw_header: vec![],
        unresolved: [""; 3],
        styles: (0..12)
            .map(|index| {
                Some(Style {
                    index,
                    builtin_id: 0,
                    kind: if index >= 10 { 2 } else { 1 },
                    base_style: None,
                    next_style: index,
                    linked_style: None,
                    revision_marked: false,
                    name: "Synthetic".into(),
                    raw_flags: 0,
                    properties: vec![StyleProperties {
                        kind: "character",
                        bytes: vec![],
                    }],
                    raw_revision: vec![],
                })
            })
            .collect(),
    }
}
fn toggle(
    s: &Stylesheet,
    d: &[PropertyModifier],
    target: u16,
) -> hidden_visibility::HiddenProperty {
    hidden_visibility::resolve_capitalization_toggle(
        s,
        0,
        Some(false),
        d,
        &mut { MAX_WORK },
        target,
    )
    .unwrap()
}
#[test]
fn caps_and_small_caps_are_independent_relative_to_style_not_prior_direct() {
    let s = sheet();
    let d = [p(0x083b, 1), p(0x083b, 129), p(0x083a, 128)];
    assert_eq!(toggle(&s, &d, 0x083b).hidden, Some(true));
    assert_eq!(toggle(&s, &d, 0x083a).hidden, Some(false));
}
#[test]
fn character_style_plain_and_direct_reset_preserve_order() {
    let mut s = sheet();
    s.styles[11].as_mut().unwrap().properties[0].bytes = vec![0x3a, 8, 1];
    let style = PropertyModifier {
        sprm: 0x4a30,
        operand: vec![11, 0],
    };
    assert_eq!(
        toggle(&s, &[style.clone(), p(0x083a, 129)], 0x083a).hidden,
        Some(false)
    );
    assert_eq!(
        toggle(&s, &[style.clone(), p(0x2a33, 0)], 0x083a).hidden,
        Some(false)
    );
    assert_eq!(
        toggle(&s, &[p(0x2a33, 0), style], 0x083a).hidden,
        Some(true)
    );
}
#[test]
fn inherited_single_toggle_resolves_but_conflicting_history_stays_unknown() {
    let mut s = sheet();
    s.styles[0].as_mut().unwrap().base_style = Some(1);
    s.styles[1].as_mut().unwrap().properties[0].bytes = vec![0x3b, 8, 1];
    assert_eq!(toggle(&s, &[], 0x083b).hidden, Some(true));
    s.styles[0].as_mut().unwrap().properties[0].bytes = vec![0x3b, 8, 0];
    assert_eq!(toggle(&s, &[], 0x083b).hidden, None);
}
#[test]
fn majority_permutation_wall_and_malformed_toggle_do_not_establish_caps() {
    for op in [0xca47, 0xca31, 0x2a83] {
        assert_eq!(toggle(&sheet(), &[p(op, 0)], 0x083b).hidden, None, "{op:x}");
    }
    assert!(
        hidden_visibility::resolve_capitalization_toggle(
            &sheet(),
            0,
            Some(false),
            &[p(0x083a, 2)],
            &mut { MAX_WORK },
            0x083a
        )
        .is_err()
    );
}
#[test]
fn majority_is_ignored_for_nondefault_character_style() {
    let d = [
        PropertyModifier {
            sprm: 0x4a30,
            operand: vec![11, 0],
        },
        p(0x083b, 1),
        p(0xca47, 0),
    ];
    assert_eq!(toggle(&sheet(), &d, 0x083b).hidden, Some(true));
}
struct Fixture {
    raw: Inspection,
    text: Transcription,
    format: FormattingInspection,
    styles: Stylesheet,
    hidden: HiddenEvidence,
    revisions: RevisionEvidence,
}
impl Fixture {
    fn new() -> Self {
        let text = "Attention. 😀";
        let n = text.encode_utf16().count();
        Self {
            raw: Inspection {
                schema_version: 1,
                status: "fixture",
                binary_version: 0x112,
                trailing_container_bytes: 0,
                piece_count: 1,
                trailing_utf16_units: vec![],
                stories: vec![Story {
                    kind: "body",
                    start_cp: 0,
                    utf16_units: n,
                    text: text.into(),
                }],
            },
            text: Transcription {
                schema_version: 1,
                status: "fixture",
                unresolved: vec![],
                stories: vec![TranscribedStory {
                    kind: "body",
                    text: text.into(),
                    segments: vec![SourceSegment {
                        kind: "text",
                        start_cp: 0,
                        end_cp: n,
                        output_start: 0,
                        output_end: text.len(),
                    }],
                    diagnostics: vec![],
                    fields_balanced: true,
                    unresolved_control_units: 0,
                }],
            },
            format: FormattingInspection {
                schema_version: 1,
                status: "fixture",
                binary_version: 0x112,
                pieces: vec![PieceProperties {
                    start_cp: 0,
                    end_cp: n,
                    file_offset: 0,
                    compressed: false,
                    raw_flags: 0,
                    raw_prm: 0,
                }],
                authored_story_end_cp: n,
                trailing_nonstory_cp_range: None,
                raw_property_groups: vec![],
                character_spans: vec![CharacterSpan {
                    start_cp: 0,
                    end_cp: n,
                    properties: vec![p(0x083a, 129)],
                }],
                unresolved: [""; 3],
            },
            styles: sheet(),
            hidden: HiddenEvidence {
                status: "fixture",
                view: "fixture",
                spans: vec![HiddenSpan {
                    start_cp: 0,
                    end_cp: n,
                    source_span_index: 0,
                    piece_index: 0,
                    paragraph_end_cp: Some(n - 1),
                    paragraph_table_depth: Some(0),
                    table_baseline_basis: "source_paragraph_depth_zero",
                    property: HiddenProperty {
                        hidden: Some(false),
                        paragraph_style: Some(0),
                        character_style: 10,
                        unresolved: vec![],
                    },
                }],
            },
            revisions: RevisionEvidence {
                status: "fixture",
                basis: "fixture",
                hidden_text: "fixture",
                spans: vec![RevisionSpan {
                    start_cp: 0,
                    end_cp: n,
                    source_span_index: 0,
                    piece_index: 0,
                    properties: RevisionProperties::default(),
                    evidence: vec![],
                    unresolved: vec![],
                }],
            },
        }
    }
    fn inspect(&self) -> Result<Capitalization> {
        self.inspect_with_table(None)
    }
    fn inspect_with_table(
        &self,
        table_properties: Option<&hidden_visibility::TableHiddenEvidence>,
    ) -> Result<Capitalization> {
        inspect(
            &self.text,
            &Context {
                source: &self.raw,
                formatting: &self.format,
                styles: &self.styles,
                hidden: &self.hidden,
                revisions: &self.revisions,
                table_properties,
            },
        )
    }
}
#[test]
fn actual_context_retains_logical_spelling_and_records_small_caps() {
    let f = Fixture::new();
    let before = serde_json::to_value(&f.text).unwrap();
    let out = f.inspect().unwrap();
    assert_eq!(before, serde_json::to_value(&f.text).unwrap());
    assert_eq!(out.spans[0].small_caps, Some(true));
    assert_eq!(out.spans[0].all_caps, Some(false));
    assert_eq!(
        out.spans[0].output[0].output_range,
        Some([0, f.text.stories[0].text.len()])
    );
    assert_eq!(f.text.stories[0].text, "Attention. 😀");
}
#[test]
fn piece_property_follows_chpx_and_preserves_raw_operand() {
    let mut f = Fixture::new();
    f.format.pieces[0].raw_prm = 0x5a << 1;
    let out = f.inspect().unwrap();
    assert_eq!(out.spans[0].small_caps, Some(false));
    assert_eq!(f.format.character_spans[0].properties[0].operand, [129]);
}
#[test]
fn hidden_deleted_and_field_instructions_keep_distinct_source_ownership() {
    let mut f = Fixture::new();
    f.hidden.spans[0].property.hidden = Some(true);
    f.revisions.spans[0].properties.deleted = Some(true);
    f.text.stories[0].text.clear();
    f.text.stories[0].segments[0].output_end = 0;
    f.text.stories[0].segments[0].kind = "field_instruction";
    let out = f.inspect().unwrap();
    let s = &out.spans[0];
    assert_eq!(s.hidden, Some(true));
    assert_eq!(s.deleted, Some(true));
    assert_eq!(s.output[0].basis, "suppressed_source");
    assert_eq!(s.output[0].output_range, Some([0, 0]));
}
#[test]
fn generated_anchors_do_not_inherit_source_caps_and_partition_gaps_fail() {
    let mut f = Fixture::new();
    f.text.stories[0].text.insert_str(0, "1. ");
    f.text.stories[0].segments[0].output_start += 3;
    f.text.stories[0].segments[0].output_end += 3;
    f.text.stories[0].segments.insert(
        0,
        SourceSegment {
            kind: "generated_list_label",
            start_cp: 0,
            end_cp: 0,
            output_start: 0,
            output_end: 3,
        },
    );
    assert_eq!(f.inspect().unwrap().generated_output_ranges.len(), 1);
    f.text.stories[0].segments[1].start_cp = 1;
    assert!(f.inspect().is_err());
}
#[test]
fn utf16_output_mapping_respects_surrogates_and_transformed_segments() {
    let s = SourceSegment {
        kind: "text",
        start_cp: 0,
        end_cp: 4,
        output_start: 0,
        output_end: 6,
    };
    let offsets = scalar_offsets("a😀b", "a😀b", &s, &[0, 1, 2, 3, 4], &mut { MAX_WORK }).unwrap();
    assert_eq!(output_range("a😀b", &s, 1, 2, offsets.as_ref(), 1).0, None);
    assert_eq!(
        output_range("a😀b", &s, 2, 3, offsets.as_ref(), 2).1,
        "surrogate_boundary_unresolved"
    );
    let good = scalar_offsets("a😀b", "a😀b", &s, &[0, 1, 3, 4], &mut { MAX_WORK }).unwrap();
    assert_eq!(
        output_range("a😀b", &s, 1, 3, good.as_ref(), 1).0,
        Some([1, 5])
    );
    let transformed = SourceSegment {
        kind: "generated_number",
        ..s
    };
    assert!(
        scalar_offsets("a😀b", "a😀b", &transformed, &[0, 1, 3, 4], &mut {
            MAX_WORK
        })
        .unwrap()
        .is_none()
    );
}
#[test]
fn table_caps_and_revision_unknowns_are_explicit_and_work_is_bounded() {
    let mut f = Fixture::new();
    f.styles.styles[1].as_mut().unwrap().kind = 3;
    f.hidden.spans[0].paragraph_table_depth = Some(1);
    f.revisions.spans[0].unresolved.push("unknown".into());
    let out = f.inspect().unwrap();
    assert_eq!(out.spans[0].small_caps, None);
    assert!(!out.spans[0].visibility_resolved);
    assert!(
        hidden_visibility::resolve_capitalization_toggle(
            &sheet(),
            0,
            Some(false),
            &[p(0x083b, 1)],
            &mut 0,
            0x083b
        )
        .is_err()
    );
}

#[test]
fn default_character_style_applies_until_plain_or_explicit_style_reset() {
    let mut s = sheet();
    s.styles[10].as_mut().unwrap().properties[0].bytes = vec![0x3b, 8, 1];
    assert_eq!(toggle(&s, &[], 0x083b).hidden, Some(true));
    assert_eq!(toggle(&s, &[p(0x083b, 129)], 0x083b).hidden, Some(false));
    assert_eq!(toggle(&s, &[p(0x2a33, 0)], 0x083b).hidden, Some(false));
    let other = PropertyModifier {
        sprm: 0x4a30,
        operand: vec![11, 0],
    };
    assert_eq!(toggle(&s, &[other], 0x083b).hidden, Some(false));
    s.styles[10] = None;
    assert_eq!(toggle(&s, &[], 0x083b).hidden, None);
}
#[test]
fn shorter_raw_story_cannot_reuse_longer_formatting_evidence() {
    let mut f = Fixture::new();
    f.raw.stories[0].utf16_units = 0;
    f.raw.stories[0].text.clear();
    f.text.stories[0].text.clear();
    f.text.stories[0].segments.clear();
    assert!(f.inspect().is_err());
}

#[test]
fn coalesced_control_offsets_require_exact_source_scalar_transforms() {
    for (kind, source, text) in [
        ("paragraph", "\r\r", "\n\n"),
        ("tab", "\t\t", "\t\t"),
        ("cell_or_row_mark", "\u{7}\u{7}", "\t\t"),
        ("line_break", "\u{b}\u{b}", "\n\n"),
        ("column_break", "\u{e}\u{e}", "\n\n"),
        ("page_or_section_break", "\u{c}\u{c}", "\u{c}\u{c}"),
        ("nonbreaking_hyphen", "\u{1e}\u{1e}", "\u{2011}\u{2011}"),
        ("optional_hyphen", "\u{1f}\u{1f}", "\u{ad}\u{ad}"),
        ("unresolved_control", "\u{8}\u{1}", "\u{fffc}\u{fffc}"),
    ] {
        let seg = SourceSegment {
            kind,
            start_cp: 10,
            end_cp: 12,
            output_start: 7,
            output_end: 7 + text.len(),
        };
        let map = scalar_offsets(source, text, &seg, &[10, 11, 12], &mut { MAX_WORK }).unwrap();
        let first = output_range(text, &seg, 10, 11, map.as_ref(), 0);
        assert_eq!(
            first.0,
            Some([7, 7 + text.chars().next().unwrap().len_utf8()])
        );
        assert_eq!(first.1, "source_verified_scalar_projection");
        assert!(
            scalar_offsets("aa", text, &seg, &[10, 11, 12], &mut { MAX_WORK })
                .unwrap()
                .is_none()
        );
    }
}
#[test]
fn literal_field_display_requires_source_equality_with_multibyte_offsets() {
    let seg = SourceSegment {
        kind: "field_display_argument",
        start_cp: 0,
        end_cp: 4,
        output_start: 5,
        output_end: 12,
    };
    let map = scalar_offsets("é😀x", "é😀x", &seg, &[0, 1, 3, 4], &mut { MAX_WORK }).unwrap();
    assert_eq!(
        output_range("é😀x", &seg, 1, 3, map.as_ref(), 1).0,
        Some([7, 11])
    );
    assert!(
        scalar_offsets("é😀x", "é😀y", &seg, &[0, 1, 3, 4], &mut { MAX_WORK })
            .unwrap()
            .is_none()
    );
}
#[test]
fn same_length_mixed_control_and_literal_forgeries_remain_unresolved() {
    let seg = SourceSegment {
        kind: "paragraph",
        start_cp: 0,
        end_cp: 2,
        output_start: 0,
        output_end: 2,
    };
    assert!(
        scalar_offsets("\r\t", "\n\n", &seg, &[0, 1, 2], &mut { MAX_WORK })
            .unwrap()
            .is_none()
    );
    let literal = SourceSegment {
        kind: "text",
        ..seg
    };
    assert!(
        scalar_offsets("ab", "ac", &literal, &[0, 1, 2], &mut { MAX_WORK })
            .unwrap()
            .is_none()
    );
}
#[test]
fn many_boundaries_in_one_control_segment_use_linear_work() {
    let n = 5000;
    let source = "\r".repeat(n);
    let text = "\n".repeat(n);
    let seg = SourceSegment {
        kind: "paragraph",
        start_cp: 0,
        end_cp: n,
        output_start: 0,
        output_end: n,
    };
    let boundaries: Vec<_> = (0..=n).collect();
    let mut work = 128 * 1024;
    let offsets = scalar_offsets(&source, &text, &seg, &boundaries, &mut work).unwrap();
    for i in 0..n {
        assert_eq!(
            output_range(&text, &seg, i, i + 1, offsets.as_ref(), i).0,
            Some([i, i + 1])
        );
    }
    assert!(work > 0);
}

#[test]
fn declared_source_units_bound_the_forward_walk_before_processing() {
    let mut f = Fixture::new();
    f.raw.stories[0].utf16_units = MAX_SOURCE_UNITS + 1;
    assert_eq!(
        f.inspect().unwrap_err(),
        Error::Limit("capitalization source units")
    );
}
