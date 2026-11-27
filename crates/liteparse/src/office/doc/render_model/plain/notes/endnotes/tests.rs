use super::*;
use crate::office::doc::notes::{
    NoteInspection, NoteLabel, NoteLabels, NoteLink, NoteSection, NoteSections, Numbering,
};
fn fixture() -> (Inspection, NoteInspection, NoteLabels, NoteSections) {
    let source = Inspection {
        schema_version: 1,
        status: "fixture",
        binary_version: 0x10c,
        trailing_container_bytes: 0,
        piece_count: 1,
        trailing_utf16_units: vec![],
        stories: vec![
            crate::office::doc::Story {
                kind: "body",
                start_cp: 0,
                utf16_units: 4,
                text: "\u{2}x*\r".into(),
            },
            crate::office::doc::Story {
                kind: "headers",
                start_cp: 4,
                utf16_units: 2,
                text: "\r\r".into(),
            },
            crate::office::doc::Story {
                kind: "endnotes",
                start_cp: 6,
                utf16_units: 6,
                text: "\u{2}a\r*\r\r".into(),
            },
        ],
    };
    let links = NoteInspection {
        status: "fixture",
        footnotes: vec![],
        endnotes: vec![
            NoteLink {
                body_cp: 0,
                note_start_cp: 6,
                note_end_cp: 9,
                automatic: true,
                raw_index: 42,
            },
            NoteLink {
                body_cp: 2,
                note_start_cp: 9,
                note_end_cp: 11,
                automatic: false,
                raw_index: 0,
            },
        ],
        unresolved: ["a", "b", "c"],
    };
    let labels = NoteLabels {
        status: "fixture",
        labels: vec![NoteLabel {
            body_cp: 0,
            note_cp: Some(6),
            note_status: "source_special_note_marker",
            section_index: Some(0),
            text: Some("iv".into()),
            reason: "source_number_and_body_reference",
        }],
    };
    let numbering = Numbering {
        restart: 0,
        offset: 4,
        format: 2,
    };
    let sections = NoteSections {
        basis: "fixture",
        legacy_numbering: None,
        sections: vec![NoteSection {
            start_cp: 0,
            end_cp: 4,
            footnotes: numbering,
            endnotes: numbering,
            properties: vec![],
            unresolved: vec![],
        }],
    };
    (source, links, labels, sections)
}
#[test]
fn endnotes_preserve_global_offsets_custom_reference_and_raw_index() {
    let (source, links, labels, sections) = fixture();
    let p = prepare(&source, &links, &labels, &sections)
        .unwrap()
        .unwrap();
    assert_eq!(p.kind, NoteKind::Endnote);
    assert_eq!(p.source_cp, [6, 12]);
    assert_eq!(p.terminal_guard, [11, 12]);
    assert_eq!(p.notes[0].label, "iv");
    assert_eq!(p.notes[0].raw_reference_index, 42);
    assert_eq!(p.notes[1].authored_reference, Some(42));
    assert!(p.notes[1].label.is_empty());
}
#[test]
fn duplicate_reference_and_missing_tail_cannot_hide_note_content() {
    let (source, mut links, labels, sections) = fixture();
    links.endnotes[1].body_cp = 0;
    assert!(prepare(&source, &links, &labels, &sections).is_err());
    let (source, mut links, labels, sections) = fixture();
    links.endnotes.pop();
    assert!(prepare(&source, &links, &labels, &sections).is_err());
}
#[test]
fn unrelated_or_ambiguous_mark_cannot_acquire_generated_label() {
    let (source, links, mut labels, sections) = fixture();
    labels.labels[0].note_cp = Some(7);
    assert!(prepare(&source, &links, &labels, &sections).is_err());
    let (mut source, links, labels, sections) = fixture();
    source.stories[2].text = "\u{2}\u{2}\r*\r\r".into();
    assert!(prepare(&source, &links, &labels, &sections).is_err());
}
#[test]
fn unresolved_numbering_and_multiple_sections_stay_explicit() {
    let (source, links, mut labels, sections) = fixture();
    labels.labels[0].text = None;
    assert!(prepare(&source, &links, &labels, &sections).is_err());
    let (source, links, labels, mut sections) = fixture();
    sections.sections[0].end_cp = 3;
    assert!(prepare(&source, &links, &labels, &sections).is_err());
}
