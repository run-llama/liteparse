use super::*;
use crate::office::doc::styles::{Style, StyleProperties};

fn p(sprm: u16, operand: &[u8]) -> PropertyModifier {
    PropertyModifier {
        sprm,
        operand: operand.to_vec(),
    }
}
pub(super) fn sheet() -> Stylesheet {
    let styles = (0..12)
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
                    bytes: Vec::new(),
                }],
                raw_revision: Vec::new(),
            })
        })
        .collect();
    Stylesheet {
        schema_version: 1,
        status: "synthetic",
        std_base_bytes: 10,
        raw_header: vec![],
        styles,
        unresolved: [""; 3],
    }
}
fn hidden(s: &mut Stylesheet, index: usize, value: u8) {
    s.styles[index].as_mut().unwrap().properties[0].bytes = vec![0x3c, 0x08, value];
}
#[test]
fn direct_relative_toggle_uses_style_not_previous_direct_value() {
    let s = sheet();
    let state = resolve_hidden_property(&s, 0, Some(false), &[p(0x083c, &[1]), p(0x083c, &[0x81])])
        .unwrap();
    assert_eq!(state.hidden, Some(true));
}
#[test]
fn character_style_reset_and_plain_restore_paragraph_baseline() {
    let mut s = sheet();
    hidden(&mut s, 11, 1);
    let input = [p(0x4a30, &[11, 0]), p(0x083c, &[0x81])];
    assert_eq!(
        resolve_hidden_property(&s, 0, Some(false), &input)
            .unwrap()
            .hidden,
        Some(false)
    );
    assert_eq!(
        resolve_hidden_property(&s, 0, Some(false), &[p(0x4a30, &[11, 0]), p(0x2a33, &[0])])
            .unwrap()
            .hidden,
        Some(false)
    );
}
#[test]
fn unresolved_table_can_be_overridden_by_absolute_hidden_assignment() {
    let s = sheet();
    assert_eq!(
        resolve_hidden_property(&s, 0, None, &[p(0x083c, &[1])])
            .unwrap()
            .hidden,
        Some(true)
    );
    assert_eq!(
        resolve_hidden_property(&s, 0, None, &[p(0x083c, &[0x81])])
            .unwrap()
            .hidden,
        None
    );
}
#[test]
fn conflicting_inheritance_and_unknown_resets_are_explicit() {
    let mut s = sheet();
    hidden(&mut s, 0, 1);
    hidden(&mut s, 1, 0);
    s.styles[1].as_mut().unwrap().base_style = Some(0);
    assert_eq!(
        resolve_hidden_property(&s, 1, Some(false), &[])
            .unwrap()
            .hidden,
        None
    );
    assert_eq!(
        resolve_hidden_property(&s, 0, Some(false), &[p(0xca47, &[0])])
            .unwrap()
            .hidden,
        None
    );
}
#[test]
fn unique_inherited_hidden_property_and_cycles() {
    let mut s = sheet();
    hidden(&mut s, 0, 1);
    s.styles[1].as_mut().unwrap().base_style = Some(0);
    assert_eq!(
        resolve_hidden_property(&s, 1, Some(false), &[])
            .unwrap()
            .hidden,
        Some(true)
    );
    s.styles[0].as_mut().unwrap().base_style = Some(1);
    assert!(resolve_hidden_property(&s, 1, Some(false), &[]).is_err());
}

#[test]
fn character_style_absolute_off_overrides_hidden_paragraph() {
    let mut s = sheet();
    hidden(&mut s, 0, 1);
    hidden(&mut s, 11, 0);
    let state = resolve_hidden_property(&s, 0, Some(false), &[p(0x4a30, &[11, 0])]).unwrap();
    assert_eq!(state.hidden, Some(false));
}

#[test]
fn malformed_operands_and_work_limit_are_errors() {
    let s = sheet();
    assert!(resolve_hidden_property(&s, 0, Some(false), &[p(0x083c, &[2])]).is_err());
    assert!(resolve_hidden_property(&s, 0, Some(false), &[p(0x4a30, &[11])]).is_err());
    assert!(resolve_hidden_property(&s, 0, Some(false), &[p(0x2a33, &[1])]).is_err());
    let mut budget = 2;
    assert!(charge(&mut budget, 3).is_err());
}

pub(super) fn contexts() -> (
    crate::office::doc::Inspection,
    crate::office::doc::formatting_inspection::FormattingInspection,
    crate::office::doc::paragraph_formatting::ParagraphInspection,
) {
    use crate::office::doc::formatting::CharacterSpan;
    use crate::office::doc::formatting_inspection::{FormattingInspection, PieceProperties};
    use crate::office::doc::paragraph_formatting::{ParagraphInspection, ParagraphSpan};
    let source = crate::office::doc::Inspection {
        schema_version: 1,
        status: "synthetic",
        binary_version: 0x112,
        trailing_container_bytes: 0,
        piece_count: 1,
        trailing_utf16_units: vec![],
        stories: vec![crate::office::doc::Story {
            kind: "body",
            start_cp: 0,
            utf16_units: 4,
            text: "A\rB\r".into(),
        }],
    };
    let formatting = FormattingInspection {
        schema_version: 1,
        status: "synthetic",
        binary_version: 0x112,
        authored_story_end_cp: 4,
        trailing_nonstory_cp_range: None,
        pieces: vec![PieceProperties {
            start_cp: 0,
            end_cp: 4,
            file_offset: 0,
            compressed: true,
            raw_flags: 0,
            raw_prm: 0,
        }],
        raw_property_groups: vec![],
        character_spans: vec![CharacterSpan {
            start_cp: 0,
            end_cp: 4,
            properties: vec![],
        }],
        unresolved: [""; 3],
    };
    let paragraphs = ParagraphInspection {
        schema_version: 1,
        status: "synthetic",
        binary_version: 0x112,
        excluded_nonstory_cp_range: None,
        spans: vec![
            ParagraphSpan {
                start_cp: 0,
                end_cp: 2,
                style_index: Some(0),
                raw_modifiers: vec![],
            },
            ParagraphSpan {
                start_cp: 2,
                end_cp: 4,
                style_index: Some(1),
                raw_modifiers: vec![],
            },
        ],
        unresolved: [""; 4],
    };
    (source, formatting, paragraphs)
}

#[test]
fn paragraph_end_style_splits_a_shared_character_run() {
    let (source, formatting, paragraphs) = contexts();
    let mut s = sheet();
    hidden(&mut s, 1, 1);
    let result = resolve_hidden_evidence(&source, &formatting, &paragraphs, &s).unwrap();
    assert_eq!(result.spans.len(), 2);
    assert_eq!((result.spans[0].start_cp, result.spans[0].end_cp), (0, 2));
    assert_eq!(result.spans[0].property.hidden, Some(false));
    assert_eq!(result.spans[1].property.hidden, Some(true));
    assert_eq!(result.spans[1].paragraph_end_cp, Some(3));
}

#[test]
fn piece_absolute_override_filters_known_mixed_property_groups() {
    let (source, mut formatting, paragraphs) = contexts();
    formatting.pieces[0].raw_prm = 1;
    formatting.raw_property_groups = vec![vec![0x3c, 0x08, 1]];
    let s = sheet();
    let result = resolve_hidden_evidence(&source, &formatting, &paragraphs, &s).unwrap();
    assert!(
        result
            .spans
            .iter()
            .all(|span| span.property.hidden == Some(true))
    );
    formatting.raw_property_groups[0].extend([0x03, 0x24, 0]);
    let result = resolve_hidden_evidence(&source, &formatting, &paragraphs, &s).unwrap();
    assert!(
        result
            .spans
            .iter()
            .all(|span| span.property.hidden == Some(true))
    );
}

#[test]
fn unmatched_paragraph_boundary_and_bad_piece_order_are_explicit() {
    let (source, mut formatting, mut paragraphs) = contexts();
    paragraphs.spans.remove(0);
    paragraphs.spans[0].start_cp = 0;
    let result = resolve_hidden_evidence(&source, &formatting, &paragraphs, &sheet()).unwrap();
    assert_eq!(result.spans[0].paragraph_end_cp, None);
    assert_eq!(result.spans[0].property.hidden, None);
    formatting.pieces[0].start_cp = 1;
    assert!(resolve_hidden_evidence(&source, &formatting, &paragraphs, &sheet()).is_err());
}

fn evidence(values: &[Option<bool>]) -> HiddenEvidence {
    HiddenEvidence {
        status: "synthetic",
        view: "hidden_text_display_off",
        spans: values
            .iter()
            .enumerate()
            .map(|(cp, value)| HiddenSpan {
                start_cp: cp,
                end_cp: cp + 1,
                source_span_index: cp,
                piece_index: 0,
                paragraph_end_cp: None,
                paragraph_table_depth: None,
                table_baseline_basis: "synthetic",
                property: HiddenProperty {
                    hidden: *value,
                    paragraph_style: None,
                    character_style: 10,
                    unresolved: vec![],
                },
            })
            .collect(),
    }
}
fn projected(
    text: &str,
    segments: Vec<crate::office::doc::SourceSegment>,
) -> crate::office::doc::Transcription {
    crate::office::doc::Transcription {
        schema_version: 1,
        status: "synthetic",
        unresolved: vec![],
        stories: vec![crate::office::doc::TranscribedStory {
            kind: "body",
            text: text.into(),
            segments,
            diagnostics: vec![],
            fields_balanced: true,
            unresolved_control_units: 0,
        }],
    }
}
fn segment(
    kind: &'static str,
    start_cp: usize,
    end_cp: usize,
    output_start: usize,
    output_end: usize,
) -> crate::office::doc::SourceSegment {
    crate::office::doc::SourceSegment {
        kind,
        start_cp,
        end_cp,
        output_start,
        output_end,
    }
}
#[test]
fn visible_projection_retains_every_source_cp_and_accepted_input() {
    let input = projected("ABC", vec![segment("text", 0, 3, 0, 3)]);
    let output = apply_hidden(&input, &evidence(&[Some(false), Some(true), None])).unwrap();
    assert_eq!(input.stories[0].text, "ABC");
    assert_eq!(output.stories[0].text, "AC");
    let spans = &output.stories[0].segments;
    assert_eq!(
        spans
            .iter()
            .map(|s| (s.start_cp, s.end_cp))
            .collect::<Vec<_>>(),
        [(0, 1), (1, 2), (2, 3)]
    );
    assert_eq!(spans[1].kind, "hidden_text");
    assert_eq!(spans[1].output_start, spans[1].output_end);
}
#[test]
fn generated_and_zero_width_labels_keep_their_mapping_when_ambiguous() {
    let input = projected(
        "1.7i",
        vec![
            segment("generated_list_label", 0, 0, 0, 2),
            segment("generated_number", 0, 3, 2, 3),
            segment("generated_note_label", 3, 4, 3, 4),
        ],
    );
    let output = apply_hidden(
        &input,
        &evidence(&[Some(true), Some(false), Some(true), Some(true)]),
    )
    .unwrap();
    assert_eq!(output.stories[0].text, "1.7");
    let spans = &output.stories[0].segments;
    assert_eq!((spans[0].start_cp, spans[0].end_cp), (0, 0));
    assert_eq!((spans[1].start_cp, spans[1].end_cp), (0, 3));
    assert_eq!(spans[2].kind, "hidden_text");
}
#[test]
fn surrogate_boundary_and_existing_deletion_are_preserved() {
    let input = projected(
        "😀",
        vec![
            segment("text", 0, 2, 0, 4),
            segment("deleted_revision", 2, 3, 4, 4),
        ],
    );
    let output = apply_hidden(&input, &evidence(&[Some(true), Some(false), Some(true)])).unwrap();
    assert_eq!(output.stories[0].text, "😀");
    assert_eq!(output.stories[0].segments[1].kind, "deleted_revision");
    let output = apply_hidden(&input, &evidence(&[Some(true), Some(true), Some(true)])).unwrap();
    assert_eq!(output.stories[0].text, "");
    assert_eq!(output.stories[0].segments[0].end_cp, 2);
}
#[test]
fn projection_rejects_reversed_source_and_stale_output_offsets() {
    let input = projected("A", vec![segment("text", 2, 1, 0, 1)]);
    assert!(apply_hidden(&input, &evidence(&[Some(true)])).is_err());
    let input = projected("A", vec![segment("text", 0, 1, 0, 2)]);
    assert!(apply_hidden(&input, &evidence(&[Some(true)])).is_err());
}

#[test]
fn majority_is_ignored_for_an_explicit_nondefault_character_style() {
    let mut s = sheet();
    hidden(&mut s, 11, 1);
    let result =
        resolve_hidden_property(&s, 0, Some(false), &[p(0x4a30, &[11, 0]), p(0xca47, &[0])])
            .unwrap();
    assert_eq!(result.hidden, Some(true));
}

#[test]
fn adjacent_generated_labels_preserve_individual_source_ownership() {
    let input = projected(
        "ii",
        vec![
            segment("generated_note_label", 0, 1, 0, 1),
            segment("generated_note_label", 1, 2, 1, 2),
        ],
    );
    let output = apply_hidden(&input, &evidence(&[Some(false), Some(false)])).unwrap();
    assert_eq!(output.stories[0].segments.len(), 2);
    assert_eq!(output.stories[0].text, "ii");
}

#[test]
fn projection_caps_story_count_and_source_work_without_large_allocations() {
    let mut input = projected("", vec![]);
    for _ in 0..8 {
        input.stories.push(projected("", vec![]).stories.remove(0));
    }
    assert!(apply_hidden(&input, &evidence(&[])).is_err());
    let input = projected(
        "A",
        vec![segment("generated_number", 0, 16 * 1024 * 1024 + 1, 0, 1)],
    );
    assert!(apply_hidden(&input, &evidence(&[Some(true)])).is_err());
}

#[test]
fn unused_table_style_does_not_change_direct_paragraph_membership() {
    let mut styles = sheet();
    styles.styles[11].as_mut().unwrap().kind = 3;
    let mut cache = std::collections::BTreeMap::new();
    let mut work = 8 * 1024 * 1024;
    let paragraph = paragraph::resolve(0, &[], 0, &[], 13, &styles, &mut cache, &mut work).unwrap();
    assert!(paragraph.non_table);
    assert_eq!(paragraph.table_depth, Some(0));
    let state = resolve_hidden_property(
        &styles,
        paragraph.style.unwrap(),
        Some(false),
        &[p(0x083c, &[0x81])],
    )
    .unwrap();
    assert_eq!(state.hidden, Some(true));
}

#[test]
fn paragraph_end_piece_and_ordered_depth_determine_table_context() {
    let styles = sheet();
    let run = |data: &[u8], prm, groups: &[Vec<u8>], mark| {
        paragraph::resolve(
            0,
            data,
            prm,
            groups,
            mark,
            &styles,
            &mut std::collections::BTreeMap::new(),
            &mut (8 * 1024 * 1024),
        )
        .unwrap()
    };
    let in_table = [0x16, 0x24, 1, 0x49, 0x66, 1, 0, 0, 0];
    assert_eq!(run(&in_table, 0, &[], 13).table_depth, Some(1));
    assert!(!run(&in_table, 0, &[], 13).non_table);
    let clear = vec![0x16, 0x24, 0, 0x4a, 0x66, 0xff, 0xff, 0xff, 0xff];
    assert!(run(&in_table, 1, &[clear], 13).non_table);
    assert!(!run(&[], (1 << 8) | (0x18 << 1), &[], 13).non_table);
    assert!(!run(&[], 0, &[], 7).non_table);
    assert!(!run(&[0x4b, 0x24, 1], 0, &[], 13).non_table);
    assert!(!run(&[0x17, 0x24, 1], 0, &[], 13).non_table);
}

#[test]
fn paragraph_style_inheritance_cannot_smuggle_table_properties() {
    let mut styles = sheet();
    styles.styles[1].as_mut().unwrap().base_style = Some(0);
    styles.styles[0]
        .as_mut()
        .unwrap()
        .properties
        .push(StyleProperties {
            kind: "paragraph",
            bytes: vec![0, 0, 0x16, 0x24, 1],
        });
    let result = paragraph::resolve(
        1,
        &[],
        0,
        &[],
        13,
        &styles,
        &mut std::collections::BTreeMap::new(),
        &mut (8 * 1024 * 1024),
    )
    .unwrap();
    assert!(!result.non_table);
    assert_eq!(result.style, None);
}

#[test]
fn paragraph_style_piece_changes_are_applied_after_papx() {
    let styles = sheet();
    let result = paragraph::resolve(
        0,
        &[0, 0x46, 1, 0],
        (2 << 8) | (4 << 1),
        &[],
        13,
        &styles,
        &mut std::collections::BTreeMap::new(),
        &mut (8 * 1024 * 1024),
    )
    .unwrap();
    assert_eq!(result.style, Some(3));
    assert!(result.non_table);
    let groups = vec![vec![0x3c, 8, 1, 0, 0x46, 2, 0]];
    let result = paragraph::resolve(
        0,
        &[],
        1,
        &groups,
        13,
        &styles,
        &mut std::collections::BTreeMap::new(),
        &mut (8 * 1024 * 1024),
    )
    .unwrap();
    assert_eq!(result.style, Some(2));
    assert_eq!(
        paragraph::piece(1, &groups, 2, &mut (8 * 1024 * 1024)).unwrap(),
        vec![p(0x083c, &[1])]
    );
}

#[test]
fn invalid_depth_and_exhausted_context_work_are_rejected() {
    let styles = sheet();
    for data in [
        vec![0x4a, 0x66, 0xff, 0xff, 0xff, 0xff],
        vec![0x16, 0x24, 2],
    ] {
        assert!(
            paragraph::resolve(
                0,
                &data,
                0,
                &[],
                13,
                &styles,
                &mut std::collections::BTreeMap::new(),
                &mut (8 * 1024 * 1024)
            )
            .is_err()
        );
    }
    assert!(
        paragraph::resolve(
            0,
            &[],
            0,
            &[],
            13,
            &styles,
            &mut std::collections::BTreeMap::new(),
            &mut 0
        )
        .is_err()
    );
}

#[test]
fn canonical_endpoint_owns_interior_control_and_unknown_terminal_stays_local() {
    let (mut source, formatting, mut paragraphs) = contexts();
    let mut s = sheet();
    hidden(&mut s, 1, 1);
    paragraphs.status = "reference_expanded_endpoint_selected_paragraphs";
    paragraphs.spans[0].end_cp = 4;
    paragraphs.spans[0].style_index = Some(1);
    paragraphs.spans.truncate(1);
    let result = resolve_hidden_evidence(&source, &formatting, &paragraphs, &s).unwrap();
    assert!(result.spans.iter().all(|p| p.property.hidden == Some(true)));
    assert!(result.spans.iter().all(|p| p.paragraph_end_cp == Some(3)));
    let (_, _, mut paragraphs) = contexts();
    paragraphs.status = "reference_expanded_endpoint_selected_paragraphs";
    source.stories[0].text = "A\u{e}B\r".into();
    let result = resolve_hidden_evidence(&source, &formatting, &paragraphs, &s).unwrap();
    assert_eq!(result.spans[0].property.hidden, None);
    assert_eq!(result.spans[1].property.hidden, Some(true));
    assert_eq!(result.spans[1].start_cp, 2);
}

fn separator(
    s: &Stylesheet,
    base: Option<bool>,
    direct: &[PropertyModifier],
) -> Result<Option<bool>> {
    let mut budget = MAX_PROPERTY_WORK;
    Ok(resolve_toggle_with_budget(s, 0, base, direct, &mut budget, 0x0818)?.hidden)
}

#[test]
fn separator_toggles_reset_and_majority_follow_their_own_property_semantics() {
    let mut s = sheet();
    assert_eq!(separator(&s, Some(false), &[]).unwrap(), Some(false));
    assert_eq!(
        separator(&s, None, &[p(0x0818, &[0])]).unwrap(),
        Some(false)
    );
    assert_eq!(separator(&s, None, &[p(0x0818, &[0x80])]).unwrap(), None);
    let direct = [p(0x0818, &[1]), p(0x0818, &[0x81])];
    assert_eq!(separator(&s, Some(false), &direct).unwrap(), Some(true));
    s.styles[11].as_mut().unwrap().properties[0].bytes = vec![0x18, 0x08, 1];
    assert_eq!(
        separator(&s, Some(false), &[p(0x4a30, &[11, 0])]).unwrap(),
        Some(true)
    );
    assert_eq!(
        separator(&s, Some(false), &[p(0x4a30, &[11, 0]), p(0x2a33, &[0])]).unwrap(),
        Some(false)
    );
    let direct = [p(0x0818, &[1]), p(0xca47, &[0])];
    assert_eq!(separator(&s, Some(false), &direct).unwrap(), Some(true));
    assert!(separator(&s, Some(false), &[p(0x0818, &[2])]).is_err());
    assert_eq!(
        separator(&s, Some(false), &[p(0xca31, &[0])]).unwrap(),
        None
    );
}

#[test]
fn separator_style_inheritance_keeps_ambiguous_repeated_toggles_unknown() {
    let mut s = sheet();
    s.styles[0].as_mut().unwrap().base_style = Some(1);
    s.styles[1].as_mut().unwrap().properties[0].bytes = vec![0x18, 0x08, 1];
    assert_eq!(separator(&s, Some(false), &[]).unwrap(), Some(true));
    s.styles[0].as_mut().unwrap().properties[0].bytes = vec![0x18, 0x08, 0];
    assert_eq!(separator(&s, Some(false), &[]).unwrap(), None);
}

fn line_break_context() -> (
    crate::office::doc::Inspection,
    crate::office::doc::formatting_inspection::FormattingInspection,
    crate::office::doc::paragraph_formatting::ParagraphInspection,
) {
    let (mut source, formatting, mut paragraphs) = contexts();
    source.stories[0].text = "A\u{b}B\r".into();
    paragraphs.status = "reference_expanded_endpoint_selected_paragraphs";
    paragraphs.spans.truncate(1);
    paragraphs.spans[0].end_cp = 4;
    (source, formatting, paragraphs)
}

#[test]
fn ordinary_line_break_preserves_paragraph_context_but_separator_does_not() {
    let (source, mut formatting, paragraphs) = line_break_context();
    let s = sheet();
    let ordinary = resolve_hidden_evidence(&source, &formatting, &paragraphs, &s).unwrap();
    assert_eq!(ordinary.spans[0].property.paragraph_style, Some(0));
    assert_eq!(ordinary.spans[0].property.hidden, Some(false));
    formatting.character_spans[0].properties = vec![p(0x0818, &[1])];
    let separator = resolve_hidden_evidence(&source, &formatting, &paragraphs, &s).unwrap();
    assert_eq!(separator.spans[0].property.paragraph_style, None);
    assert_eq!(separator.spans[0].property.hidden, None);
}

#[test]
fn line_break_uses_its_own_piece_override_and_retains_unknown_table_context() {
    let (source, mut formatting, paragraphs) = line_break_context();
    let mut end_piece = contexts().1.pieces.remove(0);
    formatting.pieces[0].end_cp = 2;
    formatting.pieces[0].raw_prm = 1;
    end_piece.start_cp = 2;
    formatting.pieces.push(end_piece);
    let mut end_span = formatting.character_spans[0].clone();
    formatting.character_spans[0].end_cp = 2;
    end_span.start_cp = 2;
    formatting.character_spans.push(end_span);
    formatting.raw_property_groups = vec![vec![0x18, 0x08, 1]];
    let s = sheet();
    assert!(
        resolve_hidden_evidence(&source, &formatting, &paragraphs, &s)
            .unwrap()
            .spans
            .iter()
            .all(|p| p.property.paragraph_style.is_none())
    );
    formatting.raw_property_groups[0] = vec![0x18, 0x08, 0];
    assert!(
        resolve_hidden_evidence(&source, &formatting, &paragraphs, &s)
            .unwrap()
            .spans
            .iter()
            .all(|p| p.property.paragraph_style == Some(0))
    );
    let mut s = s;
    s.styles[11].as_mut().unwrap().kind = 3;
    let mut paragraphs = paragraphs;
    paragraphs.spans[0].raw_modifiers = vec![0x16, 0x24, 1, 0x49, 0x66, 1, 0, 0, 0];
    formatting.pieces[0].raw_prm = 0;
    assert!(
        resolve_hidden_evidence(&source, &formatting, &paragraphs, &s)
            .unwrap()
            .spans
            .iter()
            .all(|p| p.property.paragraph_style.is_none())
    );
}

#[test]
fn malformed_line_break_character_and_piece_coverage_cannot_establish_context() {
    let (source, formatting, paragraphs) = line_break_context();
    let mut gap = formatting;
    gap.character_spans[0].start_cp = 1;
    assert!(resolve_hidden_evidence(&source, &gap, &paragraphs, &sheet()).is_err());
    let (_, mut overlap, _) = line_break_context();
    overlap
        .character_spans
        .push(overlap.character_spans[0].clone());
    assert!(resolve_hidden_evidence(&source, &overlap, &paragraphs, &sheet()).is_err());
    let (_, mut piece_gap, _) = line_break_context();
    piece_gap.pieces[0].start_cp = 1;
    assert!(resolve_hidden_evidence(&source, &piece_gap, &paragraphs, &sheet()).is_err());
    let (_, mut missing_group, _) = line_break_context();
    missing_group.pieces[0].raw_prm = 1;
    assert!(resolve_hidden_evidence(&source, &missing_group, &paragraphs, &sheet()).is_err());
}

#[test]
fn numbering_history_preserves_direct_membership_but_not_style_origin() {
    let mut styles = sheet();
    let mut raw = vec![0x45, 0xc6, 128];
    raw.resize(131, 0);
    let read = |styles: &Stylesheet, bytes: &[u8]| {
        paragraph::resolve(
            0,
            bytes,
            0,
            &[],
            13,
            styles,
            &mut Default::default(),
            &mut (8 * 1024 * 1024),
        )
    };
    let resolved = read(&styles, &raw).unwrap();
    assert_eq!(resolved.style, Some(0));
    assert!(resolved.non_table);
    assert!(read(&styles, &raw[..130]).is_err());
    let mut bad = raw.clone();
    bad[3] = 2;
    assert!(read(&styles, &bad).is_err());
    let mut style_bytes = vec![0, 0];
    style_bytes.extend_from_slice(&raw);
    styles.styles[0]
        .as_mut()
        .unwrap()
        .properties
        .push(StyleProperties {
            kind: "paragraph",
            bytes: style_bytes,
        });
    let resolved = read(&styles, &[]).unwrap();
    assert_eq!(resolved.style, None);
    assert!(!resolved.non_table);
}

#[test]
fn numbering_insert_history_preserves_current_style_and_table_membership() {
    let styles = sheet();
    for flag in [0, 1] {
        let input = [0x00, 0x46, 1, 0, 0x43, 0x24, flag];
        let state = paragraph::resolve(
            0,
            &input,
            0,
            &[],
            13,
            &styles,
            &mut Default::default(),
            &mut 1000000,
        )
        .unwrap();
        assert_eq!(state.style, Some(1));
        assert!(state.non_table);
        let table = [0x16, 0x24, 1, 0x49, 0x66, 1, 0, 0, 0, 0x43, 0x24, flag];
        let state = paragraph::resolve(
            0,
            &table,
            0,
            &[],
            7,
            &styles,
            &mut Default::default(),
            &mut 1000000,
        )
        .unwrap();
        assert_eq!(state.table_depth, Some(1));
        assert!(!state.non_table);
    }
    for bytes in [&[0x43, 0x24, 2][..], &[0x43, 0x24][..]] {
        assert!(
            paragraph::resolve(
                0,
                bytes,
                0,
                &[],
                13,
                &styles,
                &mut Default::default(),
                &mut 1000000
            )
            .is_err()
        );
    }
    let state = paragraph::resolve(
        99,
        &[0x43, 0x24, 1],
        0,
        &[],
        13,
        &styles,
        &mut Default::default(),
        &mut 1000000,
    )
    .unwrap();
    assert_eq!(state.style, None);
    assert!(!state.non_table);
    let mut selected = styles;
    hidden(&mut selected, 1, 1);
    let state = resolve_hidden_property(&selected, 1, Some(true), &[]).unwrap();
    assert_eq!(state.hidden, Some(true));
}

#[test]
fn character_borders_preserve_hidden_style_reset_and_relative_toggles() {
    let mut s = sheet();
    hidden(&mut s, 11, 1);
    s.styles[11].as_mut().unwrap().properties[0].bytes.extend([
        0x65, 0x68, 255, 25, 16, 255, 0x72, 0xca, 8, 1, 2, 3, 0, 1, 27, 255, 255,
    ]);
    let border = p(0xca72, &[8, 1, 2, 3, 255, 1, 27, 255, 255]);
    let selected = [p(0x4a30, &[11, 0]), border.clone()];
    assert_eq!(
        resolve_hidden_property(&s, 0, Some(false), &selected)
            .unwrap()
            .hidden,
        Some(true)
    );
    let toggled = [selected.to_vec(), vec![p(0x083c, &[0x81])]].concat();
    assert_eq!(
        resolve_hidden_property(&s, 0, Some(false), &toggled)
            .unwrap()
            .hidden,
        Some(false)
    );
    let reset = [selected.to_vec(), vec![p(0x2a33, &[0]), border]].concat();
    assert_eq!(
        resolve_hidden_property(&s, 0, Some(false), &reset)
            .unwrap()
            .hidden,
        Some(false)
    );
}

#[test]
fn malformed_character_borders_do_not_create_visible_or_capitalized_text() {
    for property in [
        p(0x6865, &[1, 26, 0, 0]),
        p(0x6865, &[1, 1, 17, 0]),
        p(0x6865, &[255; 4]),
        p(0xca72, &[7, 0, 0, 0, 0, 1, 1, 0, 0]),
        p(0xca72, &[8, 0, 0, 0, 1, 1, 1, 0, 0]),
        p(0xca72, &[8, 0, 0, 0, 0, 1, 64, 0, 0]),
    ] {
        let s = sheet();
        let direct = [property];
        assert_eq!(
            resolve_hidden_property(&s, 0, Some(false), &direct)
                .unwrap()
                .hidden,
            None
        );
        let mut budget = MAX_PROPERTY_WORK;
        assert_eq!(
            resolve_capitalization_toggle(&s, 0, Some(false), &direct, &mut budget, 0x083b)
                .unwrap()
                .hidden,
            None
        );
    }
}

mod paragraph_bidi;
