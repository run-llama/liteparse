use super::*;
use crate::office::doc::SourceSegment;

fn projected() -> TranscribedStory {
    TranscribedStory {
        kind: "textboxes",
        text: "1.A\n2.B\n".into(),
        segments: vec![
            segment(10, 10, 0, 2),
            segment(10, 12, 2, 4),
            segment(12, 12, 4, 6),
            segment(12, 14, 6, 8),
        ],
        diagnostics: vec![],
        fields_balanced: true,
        unresolved_control_units: 0,
    }
}
fn segment(a: usize, b: usize, c: usize, d: usize) -> SourceSegment {
    SourceSegment {
        kind: "text",
        start_cp: a,
        end_cp: b,
        output_start: c,
        output_end: d,
    }
}

#[test]
fn generated_labels_belong_to_their_start_range() {
    let story = projected();
    validate_projection(&story, 10, 14).unwrap();
    assert_eq!(projection_range(&story, 10, 12), Some([0, 4]));
    assert_eq!(projection_range(&story, 12, 14), Some([4, 8]));
    assert_eq!(projection_range(&story, 11, 14), Some([3, 8]));
    assert_eq!(projection_range(&story, 10, 13), Some([0, 7]));
}

#[test]
fn inconsistent_output_partitions_fail() {
    let mut story = projected();
    story.segments[1].output_start = 1;
    assert!(validate_projection(&story, 10, 14).is_err());
    let mut story = projected();
    story.segments[1].end_cp = 11;
    assert!(validate_projection(&story, 10, 14).is_err());
    assert!(validate_projection(&projected(), 10, 15).is_err());
}

#[test]
fn paragraph_relative_overlay_is_not_inline_order() {
    let shape = ShapeAnchor {
        shape_id: 1,
        source_cp: 20,
        rectangle_twips: [1, 2, 3, 4],
        raw_flags: 116,
    };
    let p = placement(&shape);
    assert_eq!(
        (p.horizontal_origin, p.vertical_origin, p.wrapping),
        (2, 2, 3)
    );
    assert_eq!(p.wrapping_side, None);
    assert_eq!(p.behind_text, Some(false));
    assert_eq!(p.reading_order, "unresolved");
}

#[test]
fn plan_keeps_ambiguous_ranges_and_accounts_for_spare_units() {
    use crate::office::doc::textbox_anchors::{TextboxPart, TextboxRange, TextboxRecord};
    use crate::office::doc::visibility::{RevisionProperties, RevisionSpan};
    let source = Inspection {
        schema_version: 1,
        status: "raw",
        binary_version: 0x101,
        trailing_container_bytes: 0,
        piece_count: 1,
        trailing_utf16_units: vec![],
        stories: vec![
            crate::office::doc::Story {
                kind: "body",
                start_cp: 0,
                utf16_units: 2,
                text: "\u{8}\r".into(),
            },
            crate::office::doc::Story {
                kind: "textboxes",
                start_cp: 10,
                utf16_units: 4,
                text: "A\rB\r".into(),
            },
        ],
    };
    let transcription = Transcription {
        schema_version: 1,
        status: "partial",
        unresolved: vec![],
        stories: vec![projected()],
    };
    let record = |index, start| TextboxRecord {
        record_index: index,
        raw_cp_range: [0, 0],
        reusable: false,
        source: Some(TextboxRange {
            start_cp: start,
            end_cp: start + 2,
        }),
        shape_id: Some(1),
        chain_shape_count: Some(1),
        anchor_index: Some(0),
        anchor_status: "candidate",
    };
    let mut anchors = TextboxInspection {
        status: "metadata",
        unresolved: [""; 5],
        parts: vec![TextboxPart {
            text_story: "textboxes",
            anchor_story: "body",
            shapes: vec![ShapeAnchor {
                shape_id: 1,
                source_cp: 0,
                rectangle_twips: [0; 4],
                raw_flags: 116,
            }],
            records: vec![record(0, 10), record(1, 12)],
        }],
    };
    let mut revisions = RevisionEvidence {
        status: "evidence",
        basis: "source",
        hidden_text: "unresolved",
        spans: vec![RevisionSpan {
            start_cp: 0,
            end_cp: 2,
            source_span_index: 0,
            piece_index: 0,
            properties: RevisionProperties {
                special: Some(true),
                ..Default::default()
            },
            evidence: vec![],
            unresolved: vec![],
        }],
    };
    let plan = plan_flow(&source, &transcription, &anchors, &revisions).unwrap();
    assert_eq!(plan.parts[0].active_ranges.len(), 2);
    assert_eq!(plan.parts[0].source_units_outside_active_ranges, 0);
    assert!(plan.parts[0].source_anchor_sequence.is_empty());
    anchors.parts[0].records[1].reusable = true;
    let plan = plan_flow(&source, &transcription, &anchors, &revisions).unwrap();
    assert_eq!(plan.parts[0].source_anchor_sequence, vec![0]);
    assert_eq!(plan.parts[0].reusable_records, vec![1]);
    assert_eq!(plan.parts[0].source_units_outside_active_ranges, 2);
    revisions.spans[0].properties.deleted = Some(true);
    assert!(
        plan_flow(&source, &transcription, &anchors, &revisions)
            .unwrap()
            .parts[0]
            .source_anchor_sequence
            .is_empty()
    );
}

#[test]
fn boundary_split_rejects_surrogate_halves_and_generated_replacements() {
    let mut story = projected();
    story.text = "😀x".into();
    story.segments = vec![segment(10, 13, 0, 5)];
    assert_eq!(projection_range(&story, 10, 11), None);
    assert_eq!(projection_range(&story, 10, 12), Some([0, 4]));
    story.segments[0].kind = "field_display";
    assert_eq!(projection_range(&story, 10, 12), None);
    assert_eq!(projection_range(&story, 10, 13), Some([0, 5]));
}

#[test]
fn merged_control_run_indexes_many_boundaries_in_one_pass() {
    let count = 131_072;
    let mut story = projected();
    story.text = "\n".repeat(count);
    story.segments = vec![SourceSegment {
        kind: "paragraph",
        start_cp: 0,
        end_cp: count,
        output_start: 0,
        output_end: count,
    }];
    let requests = (0..=count).step_by(2).collect();
    let boundaries = projection_boundaries(&story, &requests);
    assert_eq!(boundaries.len(), 65_537);
    assert!(boundaries.iter().all(|(cp, byte)| cp == byte));
}

#[test]
fn hidden_projection_shifts_bytes_without_changing_textbox_source_ranges() {
    use crate::office::doc::hidden_visibility::{
        HiddenEvidence, HiddenProperty, HiddenSpan, apply_hidden,
    };
    use crate::office::doc::textbox_anchors::{TextboxPart, TextboxRange, TextboxRecord};
    let source = Inspection {
        schema_version: 1,
        status: "raw",
        binary_version: 0x101,
        trailing_container_bytes: 0,
        piece_count: 1,
        trailing_utf16_units: vec![],
        stories: vec![
            crate::office::doc::Story {
                kind: "body",
                start_cp: 0,
                utf16_units: 0,
                text: String::new(),
            },
            crate::office::doc::Story {
                kind: "textboxes",
                start_cp: 10,
                utf16_units: 6,
                text: "hideA\r".into(),
            },
        ],
    };
    let input = Transcription {
        schema_version: 1,
        status: "partial",
        unresolved: vec![],
        stories: vec![TranscribedStory {
            kind: "textboxes",
            text: "hideA\n".into(),
            segments: vec![segment(10, 16, 0, 6)],
            diagnostics: vec![],
            fields_balanced: true,
            unresolved_control_units: 0,
        }],
    };
    let span = |start_cp, end_cp, hidden| HiddenSpan {
        start_cp,
        end_cp,
        source_span_index: 0,
        piece_index: 0,
        paragraph_end_cp: None,
        paragraph_table_depth: None,
        table_baseline_basis: "synthetic",
        property: HiddenProperty {
            hidden: Some(hidden),
            paragraph_style: None,
            character_style: 0,
            unresolved: vec![],
        },
    };
    let evidence = HiddenEvidence {
        status: "source",
        view: "hidden excluded",
        spans: vec![span(10, 14, true), span(14, 16, false)],
    };
    let anchors = TextboxInspection {
        status: "source",
        unresolved: [""; 5],
        parts: vec![TextboxPart {
            text_story: "textboxes",
            anchor_story: "body",
            shapes: vec![],
            records: vec![TextboxRecord {
                record_index: 0,
                raw_cp_range: [4, 6],
                reusable: false,
                source: Some(TextboxRange {
                    start_cp: 14,
                    end_cp: 16,
                }),
                shape_id: None,
                chain_shape_count: Some(1),
                anchor_index: None,
                anchor_status: "unresolved",
            }],
        }],
    };
    let revisions = RevisionEvidence {
        status: "source",
        basis: "source",
        hidden_text: "unresolved",
        spans: vec![],
    };
    let before = plan_flow(&source, &input, &anchors, &revisions).unwrap();
    let visible = apply_hidden(&input, &evidence).unwrap();
    assert_eq!(visible.stories[0].text, "A\n");
    let after = plan_flow(&source, &visible, &anchors, &revisions).unwrap();
    assert_eq!(
        before.parts[0].active_ranges[0].projected_bytes,
        Some([4, 6])
    );
    assert_eq!(
        after.parts[0].active_ranges[0].projected_bytes,
        Some([0, 2])
    );
    assert_eq!(after.parts[0].active_ranges[0].source_range, [14, 16]);
}
