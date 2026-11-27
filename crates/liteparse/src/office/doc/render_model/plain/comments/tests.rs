use super::*;
use crate::office::doc::{
    comments::{Author, Comment, MarkerEvidence},
    headers::SourceRange,
    visibility::RevisionProperties,
};
fn fixture() -> (Inspection, CommentInspection) {
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
                text: "a\u{5}b\r".into(),
            },
            crate::office::doc::Story {
                kind: "comments",
                start_cp: 4,
                utf16_units: 5,
                text: "\u{5}xy\r\r".into(),
            },
        ],
    };
    let marker = |cp| MarkerEvidence {
        source_cp: cp,
        source_unit: 5,
        formatting_unresolved: false,
        properties: Some(RevisionProperties {
            special: Some(true),
            ..Default::default()
        }),
    };
    let ownership = CommentInspection {
        status: "fixture",
        binary_version: 0x10c,
        source: SourceRange {
            start_cp: 4,
            end_cp: 9,
        },
        authors: vec![Author {
            utf16: vec![65],
            text: Some("A".into()),
        }],
        comments: vec![Comment {
            index: 0,
            body_reference: marker(1),
            source: SourceRange {
                start_cp: 4,
                end_cp: 8,
            },
            initial_marker: marker(4),
            author_index: 0,
            initials_utf16: vec![65],
            initials: Some("A".into()),
            raw_initials_length: 1,
            raw_unused: [0; 2],
            tag: 7,
            anchor_status: "bookmark_range",
            anchor: Some(SourceRange {
                start_cp: 0,
                end_cp: 1,
            }),
            bookmark_index: None,
            extended: None,
            unresolved: vec![],
        }],
        bookmarks: vec![],
        terminal_source: Some(SourceRange {
            start_cp: 8,
            end_cp: 9,
        }),
        ignored_reference_cp: None,
        ignored_text_cp: None,
        ignored_bookmark_start_cp: None,
        ignored_bookmark_end_cp: None,
        diagnostics: vec![],
        unresolved: ["a", "b", "c", "d"],
    };
    (source, ownership)
}
#[test]
fn appendix_binds_generated_identifier_to_raw_marker_and_comment_text() {
    let (s, o) = fixture();
    let (plan, ownership) = prepare(&s, o).unwrap();
    assert_eq!(plan.notes[0].label, "[C1]");
    assert_eq!(plan.notes[0].source_cp, [4, 8]);
    assert_eq!(plan.source_text, "\u{5}xy\r\r");
    let mut c = CommentDocument { plan, ownership };
    c.validate_ownership().unwrap();
    c.plan.notes[0].body_cp = 2;
    assert!(c.validate_ownership().is_err());
}
#[test]
fn deleted_or_ordinary_marker_cannot_reveal_a_comment() {
    let (s, mut o) = fixture();
    o.comments[0]
        .body_reference
        .properties
        .as_mut()
        .unwrap()
        .deleted = Some(true);
    assert!(prepare(&s, o).is_err());
    let (mut s, o) = fixture();
    s.stories[0].text = "axb\r".into();
    assert!(prepare(&s, o).is_err());
}
#[test]
fn partition_gaps_and_unowned_tail_cannot_drop_comment_text() {
    let (s, mut o) = fixture();
    o.comments[0].source.end_cp = 7;
    assert!(prepare(&s, o).is_err());
    let (mut s, o) = fixture();
    s.stories[1].text = "\u{5}xy\rz".into();
    assert!(prepare(&s, o).is_err());
}

#[test]
fn empty_comment_storage_retains_its_terminal_cp_without_a_generated_comment() {
    let (mut source, mut ownership) = fixture();
    source.stories[1].text = "\r".into();
    source.stories[1].utf16_units = 1;
    ownership.comments.clear();
    ownership.source.end_cp = 5;
    ownership.terminal_source = Some(SourceRange {
        start_cp: 4,
        end_cp: 5,
    });
    let (plan, ownership) = prepare(&source, ownership).unwrap();
    assert!(plan.notes.is_empty());
    assert_eq!(plan.source_cp, [4, 5]);
    assert_eq!(plan.terminal_guard, [4, 5]);
    assert_eq!(plan.source_text, "\r");
    let mut bad_source = source;
    bad_source.stories[1].text = "x".into();
    assert!(prepare(&bad_source, ownership).is_err());
}
