//! Regression tests for wrapped outline (PDF bookmark) headings.

use super::blocks::{Block, PositionedBlock};
use super::classify_page_with_filters;
use super::test_helpers::{line, page};
use crate::config::ImageMode;
use crate::types::{OutlineTarget, StructNode};
use std::collections::HashSet;

fn outline(title: &str, level: u8, y: Option<f32>) -> OutlineTarget {
    OutlineTarget {
        level,
        title: title.into(),
        page_index: 0,
        y_pdf: y,
    }
}

fn classify(
    page: &crate::types::ParsedPage,
    heading_map: &[(f32, u8)],
    outline: &[OutlineTarget],
) -> Vec<PositionedBlock> {
    classify_page_with_filters(
        page,
        heading_map,
        &HashSet::new(),
        outline,
        ImageMode::Placeholder,
        &HashSet::new(),
    )
}

fn headings(blocks: &[PositionedBlock]) -> Vec<(u8, String)> {
    blocks
        .iter()
        .filter_map(|positioned| match &positioned.block {
            Block::Heading { level, text } => Some((*level, text.clone())),
            _ => None,
        })
        .collect()
}

#[test]
fn complete_two_and_three_line_bookmarks_emit_one_heading_with_union_bbox() {
    for (title, fragments, expected_height) in [
        (
            "Complete wrapped bookmark title",
            vec!["Complete wrapped", "bookmark title"],
            22.0,
        ),
        (
            "Another complete wrapped bookmark title",
            vec!["Another complete", "wrapped bookmark", "title"],
            34.0,
        ),
    ] {
        let lines = fragments
            .iter()
            .enumerate()
            .map(|(i, text)| line(text, 50.0, 100.0 + i as f32 * 12.0, 10.0, 20.0))
            .collect();
        let p = page(lines);
        // The font-size map deliberately disagrees: these lines would be H4
        // from size alone, while the verified bookmark is H1.
        let blocks = classify(&p, &[(20.0, 4)], &[outline(title, 1, Some(692.0))]);

        assert_eq!(headings(&blocks), vec![(1, title.to_string())]);
        let bbox = blocks[0]
            .bbox
            .as_ref()
            .expect("heading should have geometry");
        assert_eq!((bbox.y, bbox.height), (100.0, expected_height));
    }
}

#[test]
fn complete_bookmark_with_short_middle_fragment_joins_all_lines() {
    let p = page(vec![
        line("A guide to", 50.0, 100.0, 10.0, 20.0),
        line("a", 50.0, 112.0, 10.0, 20.0),
        line("new approach", 50.0, 124.0, 10.0, 20.0),
    ]);
    let blocks = classify(
        &p,
        &[(20.0, 2)],
        &[outline("A guide to a new approach", 1, Some(692.0))],
    );

    assert_eq!(
        headings(&blocks),
        vec![(1, "A guide to a new approach".into())]
    );
}

#[test]
fn dropped_decorative_divider_does_not_carry_bookmark_level_forward() {
    let p = page(vec![
        line("Safety", 50.0, 100.0, 10.0, 20.0),
        line("—", 50.0, 112.0, 10.0, 20.0),
        line("Operating procedures", 50.0, 124.0, 10.0, 20.0),
    ]);
    let blocks = classify(
        &p,
        &[(20.0, 2)],
        &[outline("Safety — Operating procedures", 1, Some(692.0))],
    );

    assert_eq!(
        headings(&blocks),
        vec![(1, "Safety".into()), (2, "Operating procedures".into()),]
    );
}

#[test]
fn incomplete_bookmark_prefix_does_not_merge_with_unrelated_following_text() {
    let p = page(vec![
        line("A complete", 50.0, 100.0, 10.0, 10.0),
        line(
            "Unrelated paragraph sentence. More prose follows.",
            50.0,
            112.0,
            10.0,
            10.0,
        ),
    ]);
    let blocks = classify(
        &p,
        &[],
        &[outline("A complete wrapped title", 1, Some(692.0))],
    );

    assert_eq!(headings(&blocks), vec![(1, "A complete".into())]);
    assert!(blocks.iter().any(|b| matches!(
        &b.block,
        Block::Paragraph { text, .. } if text.contains("Unrelated paragraph sentence")
    )));
}

#[test]
fn separate_bookmarks_of_the_same_style_remain_separate_headings() {
    let p = page(vec![
        line("First independent heading", 50.0, 100.0, 10.0, 14.0),
        line("Body paragraph between sections.", 50.0, 130.0, 10.0, 10.0),
        line("Second independent heading", 50.0, 180.0, 10.0, 14.0),
    ]);
    let blocks = classify(
        &p,
        &[(14.0, 2)],
        &[
            outline("First independent heading", 1, Some(692.0)),
            outline("Second independent heading", 1, Some(612.0)),
        ],
    );

    assert_eq!(
        headings(&blocks),
        vec![
            (1, "First independent heading".into()),
            (1, "Second independent heading".into()),
        ]
    );
}

#[test]
fn bookmark_shorter_than_visible_line_keeps_the_full_visible_heading() {
    let visible = "Bookmark title followed by extra words";
    let p = page(vec![line(visible, 50.0, 100.0, 10.0, 14.0)]);
    let blocks = classify(&p, &[], &[outline("Bookmark title", 1, Some(692.0))]);

    assert_eq!(headings(&blocks), vec![(1, visible.into())]);
}

#[test]
fn tagged_heading_level_wins_over_bookmark_level() {
    let mut tagged = line("Tagged heading", 50.0, 100.0, 10.0, 14.0);
    tagged.mcid = Some(7);
    let mut p = page(vec![tagged]);
    p.struct_nodes = vec![StructNode {
        role: "H3".into(),
        mcids: vec![7],
        bbox: None,
        alt_text: None,
    }];
    let blocks = classify(&p, &[], &[outline("Tagged heading", 1, Some(692.0))]);

    assert_eq!(headings(&blocks), vec![(3, "Tagged heading".into())]);
}

#[test]
fn separate_bookmark_on_continuation_line_stops_wrapped_title_merge() {
    let p = page(vec![
        line("The verified heading", 50.0, 100.0, 10.0, 14.0),
        line("continuation", 50.0, 112.0, 10.0, 14.0),
    ]);
    let blocks = classify(
        &p,
        &[],
        &[
            outline("The verified heading continuation", 1, Some(692.0)),
            outline("continuation", 2, Some(680.0)),
        ],
    );

    assert_eq!(
        headings(&blocks),
        vec![
            (1, "The verified heading".into()),
            (2, "continuation".into()),
        ]
    );
}

#[test]
fn overlapping_bookmark_on_middle_fragment_stops_wrapped_title_merge() {
    let p = page(vec![
        line("A guide to", 50.0, 100.0, 10.0, 20.0),
        line("a", 50.0, 112.0, 10.0, 20.0),
        line("new approach", 50.0, 124.0, 10.0, 20.0),
    ]);
    let blocks = classify(
        &p,
        &[(20.0, 2)],
        &[
            outline("A guide to a new approach", 1, Some(692.0)),
            outline("a new", 2, Some(680.0)),
        ],
    );

    assert_eq!(
        headings(&blocks),
        vec![(1, "A guide to a".into()), (2, "new approach".into()),]
    );
}

#[test]
fn tagged_continuation_line_stops_wrapped_title_merge() {
    let mut continuation = line("continuation", 50.0, 112.0, 10.0, 14.0);
    continuation.mcid = Some(8);
    let p = page(vec![
        line("The verified heading", 50.0, 100.0, 10.0, 14.0),
        continuation,
    ]);
    let mut p = p;
    p.struct_nodes = vec![StructNode {
        role: "H3".into(),
        mcids: vec![8],
        bbox: None,
        alt_text: None,
    }];
    let blocks = classify(
        &p,
        &[],
        &[outline("The verified heading continuation", 1, Some(692.0))],
    );

    assert_eq!(
        headings(&blocks),
        vec![
            (1, "The verified heading".into()),
            (3, "continuation".into()),
        ]
    );
}

#[test]
fn printed_toc_entries_are_not_promoted_by_matching_bookmarks() {
    let entries = [
        "Introduction 1",
        "Background and Scope 4",
        "Methods and Materials 8",
        "Results and Discussion 12",
    ];
    let p = page(
        entries
            .iter()
            .enumerate()
            .map(|(i, text)| line(text, 50.0, 100.0 + i as f32 * 20.0, 10.0, 10.0))
            .collect(),
    );
    let bookmarks = [outline("Introduction", 1, None)];
    let blocks = classify(&p, &[], &bookmarks);

    assert!(
        headings(&blocks).is_empty(),
        "TOC entries became headings: {blocks:?}"
    );
    let paragraph_text = blocks
        .iter()
        .filter_map(|b| match &b.block {
            Block::Paragraph { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    for entry in entries {
        assert!(
            paragraph_text.contains(entry),
            "TOC entry {entry:?} was lost: {paragraph_text:?}"
        );
    }
}
