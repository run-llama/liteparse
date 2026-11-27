use super::*;

fn fixture() -> (Story, Story, Vec<u8>, Vec<u8>) {
    let body = Story {
        kind: "body",
        start_cp: 0,
        utf16_units: 3,
        text: "\u{2}x*".into(),
    };
    let note = Story {
        kind: "footnotes",
        start_cp: 100,
        utf16_units: 6,
        text: "\u{2}a\r*\r\r".into(),
    };
    let refs = [0u32, 2, u32::MAX]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .chain([42u16, 0].into_iter().flat_map(u16::to_le_bytes))
        .collect();
    let texts = [0u32, 3, 5, u32::MAX]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect();
    (body, note, refs, texts)
}

#[test]
fn links_preserve_source_positions_and_automatic_indicator() {
    let (body, note, refs, texts) = fixture();
    let links = decode_links(&body, &note, &refs, &texts, &mut 65_536).unwrap();
    assert_eq!(links.len(), 2);
    assert_eq!(
        (
            links[0].body_cp,
            links[0].note_start_cp,
            links[0].note_end_cp
        ),
        (0, 100, 103)
    );
    assert!(links[0].automatic);
    assert_eq!(links[0].raw_index, 42);
    assert!(!links[1].automatic);
    assert_eq!(
        (
            links[1].body_cp,
            links[1].note_start_cp,
            links[1].note_end_cp
        ),
        (2, 103, 105)
    );
}

#[test]
fn malformed_ranges_and_duplicate_references_are_rejected() {
    let (body, note, refs, texts) = fixture();
    let mut bad = refs.clone();
    bad[4..8].copy_from_slice(&0u32.to_le_bytes());
    assert!(decode_links(&body, &note, &bad, &texts, &mut 65_536).is_err());
    let mut bad = texts.clone();
    bad[4..8].copy_from_slice(&2u32.to_le_bytes());
    assert!(decode_links(&body, &note, &refs, &bad, &mut 65_536).is_err());
    assert!(decode_links(&body, &note, &refs, &texts[..12], &mut 65_536).is_err());
    let mut bad = refs.clone();
    bad[0..4].copy_from_slice(&1u32.to_le_bytes());
    assert!(decode_links(&body, &note, &bad, &texts, &mut 65_536).is_err());
    assert!(decode_links(&body, &note, &refs, &texts, &mut 1).is_err());
}

#[test]
fn missing_tables_require_an_empty_note_story() {
    let (body, mut note, _, _) = fixture();
    assert!(decode_links(&body, &note, &[], &[], &mut 65_536).is_err());
    note.text.clear();
    note.utf16_units = 0;
    assert!(
        decode_links(&body, &note, &[], &[], &mut 65_536)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn empty_reference_plc_can_preserve_only_ignored_terminal_cp() {
    let (body, mut note, _, _) = fixture();
    note.text.clear();
    note.utf16_units = 0;
    assert!(
        decode_links(&body, &note, &u32::MAX.to_le_bytes(), &[], &mut 65_536)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn source_cp_overflow_is_rejected() {
    let (body, mut note, refs, texts) = fixture();
    note.start_cp = usize::MAX;
    assert!(decode_links(&body, &note, &refs, &texts, &mut 65_536).is_err());
}

fn projection_fixture() -> (
    Inspection,
    NoteInspection,
    NoteSections,
    crate::office::doc::visibility::RevisionEvidence,
) {
    use crate::office::doc::visibility::*;
    let source = Inspection {
        schema_version: 1,
        status: "source",
        binary_version: 0x10c,
        trailing_container_bytes: 0,
        piece_count: 1,
        stories: vec![
            Story {
                kind: "body",
                start_cp: 0,
                utf16_units: 4,
                text: "\u{2}\u{c}\u{2}\r".into(),
            },
            Story {
                kind: "footnotes",
                start_cp: 4,
                utf16_units: 5,
                text: "\u{2}\r\u{2}\r\r".into(),
            },
            Story {
                kind: "endnotes",
                start_cp: 9,
                utf16_units: 0,
                text: String::new(),
            },
        ],
        trailing_utf16_units: vec![],
    };
    let notes = NoteInspection {
        status: "source",
        footnotes: vec![
            NoteLink {
                body_cp: 0,
                note_start_cp: 4,
                note_end_cp: 6,
                automatic: true,
                raw_index: 77,
            },
            NoteLink {
                body_cp: 2,
                note_start_cp: 6,
                note_end_cp: 8,
                automatic: true,
                raw_index: 88,
            },
        ],
        endnotes: vec![],
        unresolved: ["a", "b", "c"],
    };
    let sections = NoteSections {
        basis: "source",
        legacy_numbering: None,
        sections: [(0, 2), (2, 4)]
            .into_iter()
            .map(|(start_cp, end_cp)| NoteSection {
                start_cp,
                end_cp,
                footnotes: Numbering {
                    restart: 0,
                    offset: 6,
                    format: 0,
                },
                endnotes: Numbering {
                    restart: 0,
                    offset: 1,
                    format: 2,
                },
                properties: vec![],
                unresolved: vec![],
            })
            .collect(),
    };
    let revisions = RevisionEvidence {
        status: "source",
        basis: "source",
        hidden_text: "unresolved",
        spans: vec![RevisionSpan {
            start_cp: 0,
            end_cp: 9,
            source_span_index: 0,
            piece_index: 0,
            properties: RevisionProperties {
                deleted: Some(false),
                inserted: Some(false),
                field_hidden: Some(false),
                special: Some(true),
            },
            evidence: vec![],
            unresolved: vec![],
        }],
    };
    (source, notes, sections, revisions)
}
#[test]
fn continuous_offset_and_section_restart_use_source_ordinals() {
    let (source, notes, mut sections, revisions) = projection_fixture();
    let labels = resolve_note_labels(&source, &notes, &sections, &revisions).unwrap();
    assert_eq!(
        labels
            .labels
            .iter()
            .map(|l| l.text.as_deref())
            .collect::<Vec<_>>(),
        [Some("6"), Some("7")]
    );
    sections.sections[1].footnotes.restart = 1;
    let labels = resolve_note_labels(&source, &notes, &sections, &revisions).unwrap();
    assert_eq!(labels.labels[1].text.as_deref(), Some("1"));
    sections.sections[1].footnotes.format = 2;
    assert_eq!(
        resolve_note_labels(&source, &notes, &sections, &revisions)
            .unwrap()
            .labels[1]
            .text
            .as_deref(),
        Some("i")
    );
}
#[test]
fn unsupported_page_restart_and_revision_sequences_stay_unknown() {
    let (source, notes, mut sections, mut revisions) = projection_fixture();
    sections.sections[0].footnotes.restart = 2;
    assert!(
        resolve_note_labels(&source, &notes, &sections, &revisions)
            .unwrap()
            .labels[0]
            .text
            .is_none()
    );
    revisions.spans[0].properties.deleted = None;
    assert!(
        resolve_note_labels(&source, &notes, &sections, &revisions)
            .unwrap()
            .labels
            .iter()
            .all(|l| l.text.is_none())
    );
}
#[test]
fn projected_labels_own_both_source_markers() {
    let (source, notes, sections, revisions) = projection_fixture();
    let labels = resolve_note_labels(&source, &notes, &sections, &revisions).unwrap();
    let input = crate::office::doc::transcribe(&source);
    let output = apply_notes(&input, &labels).unwrap();
    assert_eq!(output.stories[0].text, "6\u{c}7\n");
    assert_eq!(output.stories[1].text, "6\n7\n\n");
    assert_eq!(output.stories[0].unresolved_control_units, 0);
    assert_eq!(output.stories[0].segments[0].start_cp, 0);
    assert_eq!(output.stories[0].segments[0].end_cp, 1);
}
#[test]
fn stale_markers_and_reversed_ranges_fail_without_mutating_input() {
    let (source, notes, sections, revisions) = projection_fixture();
    let mut labels = resolve_note_labels(&source, &notes, &sections, &revisions).unwrap();
    let input = crate::office::doc::transcribe(&source);
    labels.labels[0].body_cp = 1;
    assert!(apply_notes(&input, &labels).is_err());
    assert!(input.stories[0].text.starts_with('\u{fffc}'));
    let mut input = input;
    input.stories[0].segments[0].end_cp = 0;
    input.stories[0].segments[0].start_cp = 1;
    assert!(apply_notes(&input, &labels).is_err());
}
#[test]
fn projection_preserves_generated_and_deleted_segments_and_surrogates() {
    use crate::office::doc::{SourceSegment, TranscribedStory, Transcription};
    let pieces = [
        ("generated_number", 0, 5, "12."),
        ("symbol", 5, 6, "😀"),
        ("deleted_revision", 6, 8, ""),
        ("unresolved_control", 8, 9, "\u{fffc}"),
        ("text", 9, 11, "𝄞"),
        ("unresolved_control", 11, 12, "\u{fffc}"),
    ];
    let mut text = String::new();
    let mut segments = vec![];
    for (kind, start_cp, end_cp, value) in pieces {
        let output_start = text.len();
        text.push_str(value);
        segments.push(SourceSegment {
            kind,
            start_cp,
            end_cp,
            output_start,
            output_end: text.len(),
        });
    }
    let input = Transcription {
        schema_version: 1,
        status: "partial",
        unresolved: vec![],
        stories: vec![TranscribedStory {
            kind: "body",
            text,
            segments,
            diagnostics: vec![],
            fields_balanced: true,
            unresolved_control_units: 2,
        }],
    };
    let labels = NoteLabels {
        status: "known",
        labels: vec![NoteLabel {
            body_cp: 8,
            note_cp: Some(11),
            note_status: "source_special_note_marker",
            section_index: Some(0),
            text: Some("14".into()),
            reason: "source",
        }],
    };
    let output = apply_notes(&input, &labels).unwrap();
    assert_eq!(output.stories[0].text, "12.😀14𝄞14");
    for i in 0..3 {
        assert_eq!(
            output.stories[0].segments[i].end_cp,
            input.stories[0].segments[i].end_cp
        );
    }
    assert_eq!(
        output.stories[0].segments[2].output_start,
        output.stories[0].segments[2].output_end
    );
}

#[test]
fn legacy_section_restart_reuses_document_start_value() {
    let (source, notes, mut sections, revisions) = projection_fixture();
    for section in &mut sections.sections {
        section.footnotes.restart = 1;
        section.footnotes.offset = 6;
    }
    sections.legacy_numbering = Some(LegacyNumbering {
        raw_footnote_start_restart: 25,
        raw_endnote_start_restart: 4,
        footnote_format: 0,
        endnote_format: 2,
    });
    let labels = resolve_note_labels(&source, &notes, &sections, &revisions).unwrap();
    assert_eq!(
        labels
            .labels
            .iter()
            .map(|l| l.text.as_deref())
            .collect::<Vec<_>>(),
        [Some("6"), Some("6")]
    );
    sections.legacy_numbering = None;
    let modern = resolve_note_labels(&source, &notes, &sections, &revisions).unwrap();
    assert_eq!(
        modern
            .labels
            .iter()
            .map(|l| l.text.as_deref())
            .collect::<Vec<_>>(),
        [Some("1"), Some("1")]
    );
}

fn replace_first_note(
    source: &mut Inspection,
    notes: &mut NoteInspection,
    revisions: &mut crate::office::doc::visibility::RevisionEvidence,
    first: &str,
) {
    let first_len = first.encode_utf16().count();
    source.stories[1].text = format!("{first}\u{2}\r\r");
    source.stories[1].utf16_units = first_len + 3;
    notes.footnotes[0].note_end_cp = 4 + first_len;
    notes.footnotes[1].note_start_cp = 4 + first_len;
    notes.footnotes[1].note_end_cp = 6 + first_len;
    source.stories[2].start_cp = 7 + first_len;
    revisions.spans[0].end_cp = 7 + first_len;
}
#[test]
fn note_marker_inside_literal_prefix_retains_parentheses() {
    let (mut source, mut notes, sections, mut revisions) = projection_fixture();
    replace_first_note(&mut source, &mut notes, &mut revisions, "(\u{2})\r");
    let labels = resolve_note_labels(&source, &notes, &sections, &revisions).unwrap();
    assert_eq!(labels.labels[0].note_cp, Some(5));
    let output = apply_notes(&crate::office::doc::transcribe(&source), &labels).unwrap();
    assert_eq!(output.stories[1].text, "(6)\n7\n\n");
    assert_eq!(
        output.stories[1]
            .segments
            .iter()
            .find(|s| s.kind == "note_label")
            .unwrap()
            .start_cp,
        5
    );
}
#[test]
fn absent_or_multiple_note_marks_remain_explicitly_unresolved() {
    let (mut source, mut notes, sections, mut revisions) = projection_fixture();
    replace_first_note(&mut source, &mut notes, &mut revisions, "\u{2}\u{2}\r");
    let labels = resolve_note_labels(&source, &notes, &sections, &revisions).unwrap();
    assert_eq!(labels.labels[0].note_status, "multiple_note_label_markers");
    assert!(labels.labels[0].note_cp.is_none());
    assert_eq!(labels.labels[0].text.as_deref(), Some("6"));
    replace_first_note(&mut source, &mut notes, &mut revisions, "literal\r");
    let labels = resolve_note_labels(&source, &notes, &sections, &revisions).unwrap();
    assert_eq!(labels.labels[0].note_status, "note_label_marker_absent");
    assert!(labels.labels[0].note_cp.is_none());
    assert_eq!(labels.labels[0].text.as_deref(), Some("6"));
    let output = apply_notes(&crate::office::doc::transcribe(&source), &labels).unwrap();
    assert_eq!(output.stories[0].text, "6\u{c}7\n");
    assert_eq!(output.stories[1].text, "literal\n7\n\n");
    assert!(
        output
            .unresolved
            .contains(&"unsupported_note_numbering_or_markers")
    );
}
#[test]
fn candidate_note_mark_requires_known_special_and_revision_properties() {
    use crate::office::doc::visibility::RevisionSpan;
    let (source, notes, sections, mut revisions) = projection_fixture();
    let properties = revisions.spans[0].properties;
    revisions.spans[0].end_cp = 4;
    revisions.spans.push(RevisionSpan {
        start_cp: 4,
        end_cp: 9,
        source_span_index: 1,
        piece_index: 0,
        properties,
        evidence: vec![],
        unresolved: vec![],
    });
    revisions.spans[1].properties.special = Some(false);
    assert_eq!(
        resolve_note_labels(&source, &notes, &sections, &revisions)
            .unwrap()
            .labels[0]
            .note_status,
        "note_label_marker_absent"
    );
    revisions.spans[1].properties.special = Some(true);
    revisions.spans[1].properties.deleted = None;
    assert!(
        resolve_note_labels(&source, &notes, &sections, &revisions)
            .unwrap()
            .labels[0]
            .note_cp
            .is_none()
    );
    revisions.spans[1].properties.deleted = Some(true);
    assert!(
        resolve_note_labels(&source, &notes, &sections, &revisions)
            .unwrap()
            .labels[0]
            .note_cp
            .is_none()
    );
}
#[test]
fn chicago_symbols_use_documented_cycle_and_bounded_repetitions() {
    let (source, notes, mut sections, revisions) = projection_fixture();
    sections.sections[0].footnotes.format = 9;
    for (number, expected) in [
        (1, "*"),
        (2, "†"),
        (3, "‡"),
        (4, "§"),
        (5, "**"),
        (6, "††"),
        (7, "‡‡"),
        (8, "§§"),
        (9, "***"),
    ] {
        sections.sections[0].footnotes.offset = number;
        assert_eq!(
            resolve_note_labels(&source, &notes, &sections, &revisions)
                .unwrap()
                .labels[0]
                .text
                .as_deref(),
            Some(expected)
        );
    }
    sections.sections[0].footnotes.offset = 116;
    let labels = resolve_note_labels(&source, &notes, &sections, &revisions).unwrap();
    assert_eq!(labels.labels[0].text, Some("§".repeat(29)));
    assert!(apply_notes(&crate::office::doc::transcribe(&source), &labels).is_ok());
    for number in [0, 117] {
        sections.sections[0].footnotes.offset = number;
        assert!(
            resolve_note_labels(&source, &notes, &sections, &revisions)
                .unwrap()
                .labels[0]
                .text
                .is_none()
        );
    }
}

#[test]
fn direct_hidden_property_retains_source_labels_with_visibility_unresolved() {
    use crate::office::doc::formatting::{CharacterSpan, PropertyModifier};
    use crate::office::doc::formatting_inspection::{FormattingInspection, PieceProperties};
    let (source, notes, sections, _) = projection_fixture();
    let formatting = FormattingInspection {
        schema_version: 1,
        status: "unresolved_character_formatting",
        binary_version: source.binary_version,
        authored_story_end_cp: 9,
        trailing_nonstory_cp_range: None,
        pieces: vec![PieceProperties {
            start_cp: 0,
            end_cp: 9,
            file_offset: 100,
            compressed: true,
            raw_flags: 0,
            raw_prm: 0,
        }],
        raw_property_groups: vec![],
        character_spans: vec![CharacterSpan {
            start_cp: 0,
            end_cp: 9,
            properties: vec![
                PropertyModifier {
                    sprm: 0x0855,
                    operand: vec![1],
                },
                PropertyModifier {
                    sprm: 0x083c,
                    operand: vec![1],
                },
            ],
        }],
        unresolved: ["styles", "piece semantics", "visibility"],
    };
    let revisions = crate::office::doc::visibility::resolve_revision_evidence(&formatting).unwrap();
    assert_eq!(revisions.hidden_text, "unresolved_style_dependent");
    let labels = resolve_note_labels(&source, &notes, &sections, &revisions).unwrap();
    let input = crate::office::doc::revision_projection::apply_revisions(
        &crate::office::doc::transcribe(&source),
        &revisions,
    )
    .unwrap();
    let output = apply_notes(&input, &labels).unwrap();
    assert_eq!(output.stories[0].text, "6\u{c}7\n");
    assert!(
        output
            .unresolved
            .contains(&"hidden_text_and_unresolved_revision_properties")
    );
}

#[test]
fn ordinary_u2_does_not_hide_a_unique_special_note_mark() {
    use crate::office::doc::visibility::RevisionSpan;
    for ordinary_first in [false, true] {
        let (mut source, mut notes, sections, mut revisions) = projection_fixture();
        replace_first_note(&mut source, &mut notes, &mut revisions, "\u{2}\u{2}\r");
        let end = revisions.spans[0].end_cp;
        let properties = revisions.spans[0].properties;
        revisions.spans = [(0, 4), (4, 5), (5, 6), (6, end)]
            .into_iter()
            .enumerate()
            .map(|(i, (start_cp, end_cp))| RevisionSpan {
                start_cp,
                end_cp,
                source_span_index: i,
                piece_index: 0,
                properties,
                evidence: vec![],
                unresolved: vec![],
            })
            .collect();
        let ordinary = if ordinary_first { 1 } else { 2 };
        revisions.spans[ordinary].properties.special = Some(false);
        let labels = resolve_note_labels(&source, &notes, &sections, &revisions).unwrap();
        assert_eq!(
            labels.labels[0].note_cp,
            Some(if ordinary_first { 5 } else { 4 })
        );
        let output = apply_notes(&crate::office::doc::transcribe(&source), &labels).unwrap();
        assert_eq!(
            output.stories[1].text,
            if ordinary_first {
                "\u{fffc}6\n7\n\n"
            } else {
                "6\u{fffc}\n7\n\n"
            }
        );
        for state in 0..5 {
            revisions.spans[ordinary].properties = properties;
            revisions.spans[ordinary].properties.special = Some(false);
            revisions.spans[ordinary].unresolved.clear();
            match state {
                0 => revisions.spans[ordinary].properties.special = None,
                1 => revisions.spans[ordinary].properties.deleted = Some(true),
                2 => revisions.spans[ordinary].properties.inserted = Some(true),
                3 => revisions.spans[ordinary].properties.field_hidden = Some(true),
                _ => revisions.spans[ordinary]
                    .unresolved
                    .push("unknown source property".into()),
            }
            let labels = resolve_note_labels(&source, &notes, &sections, &revisions).unwrap();
            assert_eq!(labels.labels[0].note_status, "multiple_note_label_markers");
            assert_eq!(labels.labels[0].note_cp, None);
        }
    }
}
