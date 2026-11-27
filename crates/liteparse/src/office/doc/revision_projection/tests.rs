use super::*;
use crate::office::doc::{Inspection, Story, transcribe, visibility::RevisionProperties};

fn source(text: &str) -> Transcription {
    transcribe(&Inspection {
        schema_version: 1,
        status: "raw_stories_only",
        binary_version: 0x112,
        trailing_container_bytes: 0,
        piece_count: 1,
        trailing_utf16_units: Vec::new(),
        stories: vec![Story {
            kind: "body",
            start_cp: 0,
            utf16_units: text.encode_utf16().count(),
            text: text.into(),
        }],
    })
}
fn evidence(ranges: &[(usize, usize, Option<bool>)]) -> RevisionEvidence {
    RevisionEvidence {
        status: "revision_evidence_only",
        basis: "synthetic",
        hidden_text: "unresolved_style_dependent",
        spans: ranges
            .iter()
            .enumerate()
            .map(|(i, (a, b, deleted))| RevisionSpan {
                start_cp: *a,
                end_cp: *b,
                source_span_index: i,
                piece_index: 0,
                properties: RevisionProperties {
                    deleted: *deleted,
                    ..RevisionProperties::default()
                },
                evidence: Vec::new(),
                unresolved: Vec::new(),
            })
            .collect(),
    }
}
fn partition(story: &TranscribedStory, end: usize) {
    let (mut cp, mut byte) = (0, 0);
    for s in &story.segments {
        assert_eq!(s.start_cp, cp);
        assert_eq!(s.output_start, byte);
        assert!(s.end_cp > cp);
        assert!(story.text.get(s.output_start..s.output_end).is_some());
        cp = s.end_cp;
        byte = s.output_end;
    }
    assert_eq!((cp, byte), (end, story.text.len()));
}

#[test]
fn removes_known_deletions_and_keeps_zero_output_source_ranges() {
    let input = source("AoldB");
    let result = apply_revisions(
        &input,
        &evidence(&[(0, 1, Some(false)), (1, 4, Some(true)), (4, 5, Some(false))]),
    )
    .unwrap();
    let story = &result.stories[0];
    assert_eq!(story.text, "AB");
    assert_eq!(story.segments[1].kind, "deleted_revision");
    assert_eq!(story.segments[1].output_start, story.segments[1].output_end);
    assert_eq!(input.stories[0].text, "AoldB");
    partition(story, 5);
}

#[test]
fn deletes_whole_field_while_preserving_instruction_and_marker_provenance() {
    let input = source("A\u{13}C\u{14}xy\u{15}Z");
    let result = apply_revisions(
        &input,
        &evidence(&[(0, 1, Some(false)), (1, 7, Some(true)), (7, 8, Some(false))]),
    )
    .unwrap();
    assert_eq!(result.stories[0].text, "AZ");
    assert!(
        result.stories[0]
            .diagnostics
            .iter()
            .any(|d| d.code == "field_revision_structure_unresolved")
    );
    partition(&result.stories[0], 8);
}

#[test]
fn independent_field_instruction_and_result_deletions_preserve_result_order() {
    let input = source("A\u{13}C\u{14}xy\u{15}Z");
    let result = apply_revisions(
        &input,
        &evidence(&[
            (0, 2, Some(false)),
            (2, 3, Some(true)),
            (3, 4, Some(false)),
            (4, 5, Some(true)),
            (5, 8, Some(false)),
        ]),
    )
    .unwrap();
    assert_eq!(result.stories[0].text, "AyZ");
    partition(&result.stories[0], 8);
}

#[test]
fn deletion_across_paragraph_table_and_object_controls_rebuilds_offsets() {
    let input = source("A\rB\tC\u{7}D\u{1}E");
    let result = apply_revisions(
        &input,
        &evidence(&[(0, 1, Some(false)), (1, 8, Some(true)), (8, 9, Some(false))]),
    )
    .unwrap();
    assert_eq!(result.stories[0].text, "AE");
    assert_eq!(result.stories[0].unresolved_control_units, 0);
    partition(&result.stories[0], 9);
}

#[test]
fn utf8_offsets_follow_utf16_source_positions() {
    let input = source("Lé😀R");
    let result = apply_revisions(
        &input,
        &evidence(&[(0, 1, Some(false)), (1, 2, Some(true)), (2, 5, Some(false))]),
    )
    .unwrap();
    assert_eq!(result.stories[0].text, "L😀R");
    partition(&result.stories[0], 5);
    let result = apply_revisions(
        &input,
        &evidence(&[(0, 2, Some(false)), (2, 4, Some(true)), (4, 5, Some(false))]),
    )
    .unwrap();
    assert_eq!(result.stories[0].text, "LéR");
    partition(&result.stories[0], 5);
}

#[test]
fn ambiguous_surrogate_deletion_retains_entire_character() {
    let input = source("😀");
    let result = apply_revisions(
        &input,
        &evidence(&[(0, 1, Some(true)), (1, 2, Some(false))]),
    )
    .unwrap();
    assert_eq!(result.stories[0].text, "😀");
    assert!(
        result.stories[0]
            .diagnostics
            .iter()
            .any(|d| d.code == "revision_splits_unicode_scalar")
    );
    partition(&result.stories[0], 2);
}

#[test]
fn unknown_visibility_retains_text_and_reports_uncertainty() {
    let input = source("keep");
    let result = apply_revisions(&input, &evidence(&[(0, 4, None)])).unwrap();
    assert_eq!(result.stories[0].text, "keep");
    assert!(
        result.stories[0]
            .diagnostics
            .iter()
            .any(|d| d.code == "revision_visibility_unresolved")
    );
    assert_eq!(result.status, "partial_transcription");
    partition(&result.stories[0], 4);
}

#[test]
fn malformed_field_projection_retains_existing_diagnostics() {
    let input = source("A\u{13}CODE");
    let result = apply_revisions(
        &input,
        &evidence(&[(0, 2, Some(false)), (2, 3, Some(true)), (3, 6, Some(false))]),
    )
    .unwrap();
    assert_eq!(result.stories[0].text, "A\u{fffc}ODE");
    assert!(!result.stories[0].fields_balanced);
    assert_eq!(result.stories[0].unresolved_control_units, 1);
    assert!(
        result.stories[0]
            .diagnostics
            .iter()
            .any(|d| d.code == "unclosed_field")
    );
    partition(&result.stories[0], 6);
}

#[test]
fn rejects_ambiguous_mapping_and_missing_revision_coverage() {
    let mut input = source("AB");
    assert!(apply_revisions(&input, &evidence(&[(0, 1, Some(false))])).is_err());
    input.stories[0].segments[0].end_cp = 3;
    assert!(apply_revisions(&input, &evidence(&[(0, 3, Some(false))])).is_err());
}

#[test]
fn unprojected_limit_tail_keeps_character_mapping() {
    let mut input = source("é😀\r");
    input.stories[0].text = "é😀\r".into();
    input.stories[0].segments = vec![SourceSegment {
        kind: "unprojected_tail",
        start_cp: 0,
        end_cp: 4,
        output_start: 0,
        output_end: 7,
    }];
    let result = apply_revisions(
        &input,
        &evidence(&[(0, 1, Some(false)), (1, 3, Some(true)), (3, 4, Some(false))]),
    )
    .unwrap();
    assert_eq!(result.stories[0].text, "é\r");
    partition(&result.stories[0], 4);
}
