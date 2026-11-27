use super::*;
use crate::office::doc::notes::{
    NoteInspection, NoteLabel, NoteLabels, NoteLink, NoteSection, Numbering,
};

fn sections() -> NoteSections {
    NoteSections {
        sections: vec![NoteSection {
            start_cp: 0,
            end_cp: 3,
            footnotes: Numbering {
                restart: 0,
                offset: 1,
                format: 0,
            },
            endnotes: Numbering {
                restart: 0,
                offset: 1,
                format: 2,
            },
            properties: vec![],
            unresolved: vec![],
        }],
        basis: "synthetic",
        legacy_numbering: None,
    }
}

fn fixture() -> (Inspection, NoteInspection, NoteLabels) {
    let source = Inspection {
        schema_version: 1,
        status: "decoded",
        binary_version: 0x112,
        trailing_container_bytes: 0,
        piece_count: 1,
        trailing_utf16_units: vec![],
        stories: vec![
            crate::office::doc::Story {
                kind: "body",
                start_cp: 0,
                utf16_units: 3,
                text: "A\u{2}\r".into(),
            },
            crate::office::doc::Story {
                kind: "footnotes",
                start_cp: 3,
                utf16_units: 6,
                text: "(\u{2})N\r\r".into(),
            },
        ],
    };
    let links = NoteInspection {
        status: "decoded",
        footnotes: vec![NoteLink {
            body_cp: 1,
            note_start_cp: 3,
            note_end_cp: 8,
            automatic: true,
            raw_index: 1,
        }],
        endnotes: vec![],
        unresolved: [""; 3],
    };
    let labels = NoteLabels {
        status: "resolved",
        labels: vec![NoteLabel {
            body_cp: 1,
            note_cp: Some(4),
            note_status: "resolved",
            section_index: Some(0),
            text: Some("12".into()),
            reason: "source_numbering",
        }],
    };
    (source, links, labels)
}

#[test]
fn retains_parentheses_and_separate_terminal_guard() {
    let (source, links, labels) = fixture();
    let n = prepare(&source, &links, &labels, NotePosition::PageBottom, "test")
        .unwrap()
        .unwrap();
    assert_eq!(n.source_text, "(\u{2})N\r\r");
    assert_eq!(n.terminal_guard, [8, 9]);
    assert_eq!(n.notes[0].source_cp, [3, 8]);
    assert_eq!(n.notes[0].marker_cp, Some(4));
    assert_eq!(n.notes[0].label, "12");
}

#[test]
fn rejects_unowned_tail_and_stale_label_marker() {
    let (mut source, links, mut labels) = fixture();
    labels.labels[0].note_cp = Some(3);
    assert!(prepare(&source, &links, &labels, NotePosition::PageBottom, "test").is_err());
    labels.labels[0].note_cp = Some(4);
    source.stories[1].text.push('\r');
    source.stories[1].utf16_units += 1;
    assert!(prepare(&source, &links, &labels, NotePosition::PageBottom, "test").is_err());
}

#[test]
fn modern_position_is_ordered_and_legacy_dop_is_version_scoped() {
    let mut s = sections();
    assert_eq!(
        position(0x112, &[], &s).unwrap().0,
        NotePosition::PageBottom
    );
    for value in [2, 1, 2] {
        s.sections[0]
            .properties
            .push(crate::office::doc::formatting::PropertyModifier {
                sprm: 0x303b,
                operand: vec![value],
            });
    }
    assert_eq!(
        position(0x112, &[32], &s).unwrap().0,
        NotePosition::BeneathText
    );
    assert_eq!(
        position(0xd9, &[32], &s).unwrap().0,
        NotePosition::PageBottom
    );
    assert_eq!(
        position(0xc1, &[0], &s).unwrap().0,
        NotePosition::SectionEnd
    );
    assert!(
        position(
            0x112,
            &[],
            &NoteSections {
                sections: vec![],
                ..sections()
            }
        )
        .is_err()
    );
    s.sections[0].properties[2].operand = vec![0];
    assert!(position(0x112, &[], &s).is_err());
    assert!(position(0xd9, &[96], &s).is_err());
    assert!(position(0x113, &[32], &s).is_err());
}

#[test]
fn incomplete_or_page_restarted_numbering_is_not_render_qualified() {
    let mut s = sections();
    s.sections[0].footnotes.restart = 2;
    assert!(position(0x112, &[], &s).is_err());
    let (source, links, mut labels) = fixture();
    labels.labels[0].note_cp = None;
    assert!(prepare(&source, &links, &labels, NotePosition::PageBottom, "test").is_err());
}

#[test]
fn authored_references_preserve_literal_multichar_and_symbol_placeholder_source() {
    for mark in ["*", "(12)", "*)", "("] {
        let (mut source, mut links, mut labels) = fixture();
        source.stories[0].text = format!("A{mark}\r");
        source.stories[0].utf16_units = source.stories[0].text.encode_utf16().count();
        let start = source.stories[0].utf16_units;
        source.stories[1].start_cp = start;
        source.stories[1].text = format!("{mark} note\r\r");
        source.stories[1].utf16_units = source.stories[1].text.encode_utf16().count();
        links.footnotes[0].automatic = false;
        links.footnotes[0].raw_index = 0;
        links.footnotes[0].note_start_cp = start;
        links.footnotes[0].note_end_cp = start + source.stories[1].utf16_units - 1;
        labels.labels.clear();
        let plan = prepare(&source, &links, &labels, NotePosition::PageBottom, "test")
            .unwrap()
            .unwrap();
        assert_eq!(plan.source_text, source.stories[1].text);
        assert_eq!(plan.notes[0].body_cp, 1);
        assert_eq!(
            plan.notes[0].source_cp,
            [start, links.footnotes[0].note_end_cp]
        );
        assert_eq!(plan.notes[0].authored_reference, mark.encode_utf16().next());
        assert_eq!(plan.notes[0].marker_cp, None);
        assert!(plan.notes[0].label.is_empty());
        // A literal parenthesis can be a CSymbol placeholder; its glyph identity
        // remains a separate formatting requirement rather than a guessed label.
        links.footnotes[0].raw_index = 1;
        assert!(prepare(&source, &links, &labels, NotePosition::PageBottom, "test").is_err());
    }
}

#[test]
fn mixed_custom_and_automatic_references_consume_only_automatic_labels() {
    let (mut source, mut links, mut labels) = fixture();
    source.stories[0].text = "*\u{2}\r".into();
    source.stories[1].text = "*N\r\u{2}M\r\r".into();
    source.stories[1].utf16_units = 7;
    links.footnotes = vec![
        NoteLink {
            body_cp: 0,
            note_start_cp: 3,
            note_end_cp: 6,
            automatic: false,
            raw_index: 0,
        },
        NoteLink {
            body_cp: 1,
            note_start_cp: 6,
            note_end_cp: 9,
            automatic: true,
            raw_index: 1,
        },
    ];
    labels.labels[0].note_cp = Some(6);
    labels.labels[0].text = Some("1".into());
    let plan = prepare(&source, &links, &labels, NotePosition::PageBottom, "test")
        .unwrap()
        .unwrap();
    assert_eq!(plan.notes[0].authored_reference, Some(42));
    assert_eq!(plan.notes[1].label, "1");
    assert_eq!(plan.notes[1].marker_cp, Some(6));
    assert_eq!(plan.source_text, "*N\r\u{2}M\r\r");
    labels.labels[0].body_cp = 0;
    assert!(prepare(&source, &links, &labels, NotePosition::PageBottom, "test").is_err());
}

use crate::office::doc::Story;
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
fn sections_own_boundary_references_and_numbering_restarts() {
    let (source, links, mut sections, revisions) = projection_fixture();
    for (restart, expected) in [(0, ["6", "7"]), (1, ["6", "1"])] {
        sections.sections[1].footnotes.restart = restart;
        sections.sections[1].properties = vec![crate::office::doc::formatting::PropertyModifier {
            sprm: 0x303b,
            operand: vec![2],
        }];
        let labels =
            crate::office::doc::notes::resolve_note_labels(&source, &links, &sections, &revisions)
                .unwrap();
        let owned = section_plans(source.binary_version, &[], &sections, 4).unwrap();
        let plan = prepare_sections(&source, &links, &labels, owned)
            .unwrap()
            .unwrap();
        assert_eq!(
            plan.notes
                .iter()
                .map(|n| n.section_index)
                .collect::<Vec<_>>(),
            [0, 1]
        );
        assert_eq!(
            plan.notes
                .iter()
                .map(|n| n.label.as_str())
                .collect::<Vec<_>>(),
            expected
        );
        assert_eq!(plan.notes[1].body_cp, 2);
        assert_eq!(plan.position, None);
        assert_eq!(plan.sections[0].position, NotePosition::PageBottom);
        assert_eq!(plan.sections[1].position, NotePosition::BeneathText);
        assert_eq!(plan.sections[1].numbering.restart, restart);
    }
    sections.sections[1].start_cp = 3;
    assert!(section_plans(source.binary_version, &[], &sections, 4).is_err());
}

#[test]
fn note_free_sections_retain_ownership_and_unknown_modes_stay_guarded() {
    let (mut source, mut links, mut sections, revisions) = projection_fixture();
    source.stories[0].text = "A\u{c}\u{2}\r".into();
    source.stories[1].text = "\u{2}\r\r".into();
    source.stories[1].utf16_units = 3;
    source.stories[2].start_cp = 7;
    links.footnotes.remove(0);
    links.footnotes[0].note_start_cp = 4;
    links.footnotes[0].note_end_cp = 6;
    let labels =
        crate::office::doc::notes::resolve_note_labels(&source, &links, &sections, &revisions)
            .unwrap();
    let plan = prepare_sections(
        &source,
        &links,
        &labels,
        section_plans(source.binary_version, &[], &sections, 4).unwrap(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(plan.sections.len(), 2);
    assert_eq!(plan.notes[0].section_index, 1);
    assert_eq!(plan.notes[0].label, "6");
    sections.sections[0]
        .unresolved
        .push("unknown section property");
    assert!(section_plans(source.binary_version, &[], &sections, 4).is_err());
    sections.sections[0].unresolved.clear();
    sections.sections[1].footnotes.restart = 2;
    assert!(section_plans(source.binary_version, &[], &sections, 4).is_err());
}

#[test]
fn coexisting_endnotes_retain_ranges_and_labels_do_not_enter_footnote_sequence() {
    let (mut source, mut links, mut labels) = fixture();
    source.stories[0].text = "*\u{2}\r".into();
    source.stories.push(Story {
        kind: "endnotes",
        start_cp: 9,
        utf16_units: 3,
        text: "*\r\r".into(),
    });
    links.endnotes.push(NoteLink {
        body_cp: 0,
        note_start_cp: 9,
        note_end_cp: 11,
        automatic: false,
        raw_index: 0,
    });
    let plan = prepare(&source, &links, &labels, NotePosition::PageBottom, "test")
        .unwrap()
        .unwrap();
    assert_eq!(plan.notes[0].label, "12");
    assert_eq!(plan.coexisting_endnote_source, Some([9, 12]));
    assert_eq!(plan.coexisting_endnotes[0].body_cp, 0);
    assert!(!plan.coexisting_endnotes[0].automatic);
    source.stories[0].text = "\u{2}\u{2}\r".into();
    source.stories[2].text = "\u{2}\r\r".into();
    links.endnotes[0].automatic = true;
    links.endnotes[0].raw_index = 7;
    labels.labels.push(NoteLabel {
        body_cp: 0,
        note_cp: Some(9),
        note_status: "resolved",
        section_index: Some(0),
        text: Some("1".into()),
        reason: "test",
    });
    let plan = prepare(&source, &links, &labels, NotePosition::PageBottom, "test")
        .unwrap()
        .unwrap();
    assert_eq!(plan.notes[0].label, "12");
    assert_eq!(plan.coexisting_endnotes[0].raw_index, 7);
    labels.labels[1].body_cp = 1;
    assert!(prepare(&source, &links, &labels, NotePosition::PageBottom, "test").is_err());
    labels.labels[1].body_cp = 0;
    links.endnotes[0].body_cp = 1;
    assert!(prepare(&source, &links, &labels, NotePosition::PageBottom, "test").is_err());
    links.endnotes[0].body_cp = 0;
    links.endnotes[0].note_end_cp = 99;
    assert!(prepare(&source, &links, &labels, NotePosition::PageBottom, "test").is_err());
}

#[test]
fn absent_automatic_marker_preserves_authored_prefix_without_matching_digits() {
    for text in ["2abc\r\r", "word\r\r"] {
        let (mut source, links, mut labels) = fixture();
        source.stories[1].text = text.into();
        labels.labels[0].note_cp = None;
        labels.labels[0].note_status = "note_label_marker_absent";
        let plan = prepare(&source, &links, &labels, NotePosition::PageBottom, "test")
            .unwrap()
            .unwrap();
        assert_eq!(plan.source_text, text);
        assert_eq!(plan.notes[0].marker_cp, None);
        assert_eq!(
            plan.notes[0].marker_status,
            FootnoteMarkerStatus::AutomaticAbsent
        );
        assert_eq!(plan.notes[0].authored_reference, None);
        assert_eq!(plan.notes[0].label, "12");
        assert_eq!(plan.notes[0].raw_reference_index, 1);
        for status in [
            "multiple_note_label_markers",
            "note_label_marker_properties_unresolved",
        ] {
            labels.labels[0].note_status = status;
            assert!(prepare(&source, &links, &labels, NotePosition::PageBottom, "test").is_err());
        }
        labels.labels[0].note_status = "note_label_marker_absent";
        source.stories[1].text = "\u{2}abc\r\r".into();
        assert!(prepare(&source, &links, &labels, NotePosition::PageBottom, "test").is_err());
    }
}
