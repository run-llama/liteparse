use super::*;
use crate::office::doc::{
    Story,
    visibility::{RevisionProperties, RevisionSpan},
};

fn source(body: &[u16], comments: &[u16]) -> Inspection {
    let mut cp = 0;
    let stories = [
        "body",
        "footnotes",
        "headers",
        "reserved_macro",
        "comments",
        "endnotes",
        "textboxes",
        "header_textboxes",
    ]
    .into_iter()
    .enumerate()
    .map(|(i, kind)| {
        let raw = if i == 0 {
            body
        } else if i == 4 {
            comments
        } else {
            &[]
        };
        let story = Story {
            kind,
            start_cp: cp,
            utf16_units: raw.len(),
            text: String::from_utf16(raw).unwrap(),
        };
        cp += raw.len();
        story
    })
    .collect();
    Inspection {
        schema_version: 1,
        status: "raw_stories_only",
        binary_version: 0x101,
        trailing_container_bytes: 0,
        piece_count: 1,
        stories,
        trailing_utf16_units: Vec::new(),
    }
}
fn cp(values: &[u32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}
fn blocks(references: &[u32], text: &[u32], tags: &[u32]) -> Vec<Vec<u8>> {
    let mut refs = cp(references);
    refs.extend(u32::MAX.to_le_bytes());
    for tag in tags {
        let mut record = vec![0; 30];
        record[..2].copy_from_slice(&1u16.to_le_bytes());
        record[2..4].copy_from_slice(&88u16.to_le_bytes());
        record[26..].copy_from_slice(&tag.to_le_bytes());
        refs.extend(record);
    }
    vec![
        refs,
        cp(text),
        vec![1, 0, 65, 0],
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
    ]
}
fn inspect(
    source: &Inspection,
    blocks: &[Vec<u8>],
    rev: Option<&RevisionEvidence>,
) -> Result<CommentInspection> {
    validate_source(
        source,
        source.binary_version,
        &std::array::from_fn(|i| source.stories[i].utf16_units),
    )?;
    decode(
        source,
        rev,
        &blocks.iter().map(Vec::as_slice).collect::<Vec<_>>(),
    )
}
fn revisions(end: usize, deleted: bool) -> RevisionEvidence {
    RevisionEvidence {
        status: "test",
        basis: "test",
        hidden_text: "unknown",
        spans: vec![RevisionSpan {
            start_cp: 0,
            end_cp: end,
            source_span_index: 0,
            piece_index: 0,
            properties: RevisionProperties {
                special: Some(true),
                deleted: Some(deleted),
                ..RevisionProperties::default()
            },
            evidence: Vec::new(),
            unresolved: Vec::new(),
        }],
    }
}
fn bookmarks_fixture(
    blocks: &mut [Vec<u8>],
    tags: &[u32],
    starts: &[u32],
    ends: &[u32],
    indices: &[u16],
) {
    blocks[3] = [
        u16::MAX.to_le_bytes(),
        (tags.len() as u16).to_le_bytes(),
        10u16.to_le_bytes(),
    ]
    .concat();
    for tag in tags {
        blocks[3].extend([0, 0, 0, 1]);
        blocks[3].extend(tag.to_le_bytes());
        blocks[3].extend(u32::MAX.to_le_bytes());
    }
    blocks[4] = cp(starts);
    blocks[4].extend(u32::MAX.to_le_bytes());
    for index in indices {
        blocks[4].extend(index.to_le_bytes());
        blocks[4].extend([0, 0]);
    }
    blocks[5] = cp(ends);
    blocks[5].extend(u32::MAX.to_le_bytes());
}
#[test]
fn point_comment_keeps_text_markers_terminal_and_deleted_reference() {
    let s = source(&[65, 5, 13], &[5, 90, 13, 13]);
    let b = blocks(&[1], &[0, 3, u32::MAX], &[u32::MAX]);
    let r = revisions(7, true);
    let out = inspect(&s, &b, Some(&r)).unwrap();
    let c = &out.comments[0];
    assert_eq!(c.anchor_status, "point_at_reference");
    assert_eq!(
        c.anchor,
        Some(SourceRange {
            start_cp: 1,
            end_cp: 1
        })
    );
    assert_eq!(
        c.source,
        SourceRange {
            start_cp: 3,
            end_cp: 6
        }
    );
    assert_eq!(c.body_reference.properties.unwrap().deleted, Some(true));
    assert_eq!(
        out.terminal_source,
        Some(SourceRange {
            start_cp: 6,
            end_cp: 7
        })
    );
    assert_eq!(out.ignored_text_cp, Some(u32::MAX));
    assert_eq!(out.authors[0].text.as_deref(), Some("A"));
    assert!(c.unresolved.is_empty());
}
#[test]
fn tags_join_reordered_ends_and_preserve_overlapping_ranges() {
    let s = source(&[65, 66, 5, 67, 5, 13], &[5, 13, 5, 13, 13]);
    let mut b = blocks(&[2, 4], &[0, 2, 4, u32::MAX], &[9, 7]);
    bookmarks_fixture(&mut b, &[7, 9], &[0, 1], &[2, 4], &[1, 0]);
    let out = inspect(&s, &b, None).unwrap();
    assert_eq!(out.comments[0].bookmark_index, Some(1));
    assert_eq!(
        out.comments[0].anchor,
        Some(SourceRange {
            start_cp: 1,
            end_cp: 2
        })
    );
    assert_eq!(
        out.comments[1].anchor,
        Some(SourceRange {
            start_cp: 0,
            end_cp: 4
        })
    );
    assert!(out.comments[0].unresolved.contains(&"body_comment_marker"));
}
#[test]
fn missing_duplicate_tags_and_duplicate_end_indices_are_explicit() {
    let s = source(&[65, 5, 66, 5, 13], &[5, 13, 5, 13, 13]);
    let mut b = blocks(&[1, 3], &[0, 2, 4, u32::MAX], &[7, 8]);
    bookmarks_fixture(&mut b, &[7, 7], &[0, 2], &[1, 3], &[0, 0]);
    let out = inspect(&s, &b, None).unwrap();
    assert!(out.comments.iter().all(|c| c.anchor.is_none()));
    assert!(out.comments.iter().all(|c| {
        c.unresolved
            .contains(&"comment_bookmark_tag_missing_or_ambiguous")
    }));
    assert!(
        out.bookmarks
            .iter()
            .all(|b| b.unresolved.contains(&"ambiguous_bookmark_end_index"))
    );
}
#[test]
fn noncanonical_initial_marker_keeps_source_ownership_with_diagnostic() {
    let s = source(&[5, 13], &[19, 65, 21, 13, 13]);
    let b = blocks(&[0], &[0, 4, 0], &[u32::MAX]);
    let out = inspect(&s, &b, Some(&revisions(7, false))).unwrap();
    assert_eq!(
        out.comments[0].source,
        SourceRange {
            start_cp: 2,
            end_cp: 6
        }
    );
    assert!(
        out.comments[0]
            .unresolved
            .contains(&"comment_initial_marker")
    );
}
#[test]
fn empty_and_sentinel_only_comment_stories_do_not_create_comments() {
    let s = source(&[13], &[]);
    let mut b = vec![Vec::new(); 7];
    b[4] = cp(&[u32::MAX]);
    b[5] = cp(&[u32::MAX]);
    assert!(inspect(&s, &b, None).unwrap().comments.is_empty());
    let s = source(&[13], &[13]);
    let b = blocks(&[], &[0, u32::MAX], &[]);
    let out = inspect(&s, &b, None).unwrap();
    assert!(out.comments.is_empty());
    assert!(out.terminal_source.is_some());
}
#[test]
fn text_and_reference_coverage_and_counts_are_checked() {
    let s = source(&[5, 13], &[5, 13, 13]);
    let good = blocks(&[0], &[0, 2, u32::MAX], &[u32::MAX]);
    for index in [0, 1] {
        let mut b = good.clone();
        b[index].pop();
        assert!(inspect(&s, &b, None).is_err());
    }
    let mut b = good.clone();
    b[1] = cp(&[1, 2, u32::MAX]);
    assert!(inspect(&s, &b, None).is_err());
    let mut b = good;
    b[0][..4].copy_from_slice(&2u32.to_le_bytes());
    assert!(inspect(&s, &b, None).is_err());
}
#[test]
fn source_identity_and_revision_span_order_are_validated() {
    let mut s = source(&[5, 13], &[5, 13, 13]);
    let lengths = std::array::from_fn(|i| s.stories[i].utf16_units);
    s.stories[4].start_cp += 1;
    assert!(validate_source(&s, 0x101, &lengths).is_err());
    let mut r = revisions(10, false);
    r.spans[0].end_cp = 11;
    assert!(validate_revisions(Some(&r), 10).is_err());
    r.spans[0].start_cp = 11;
    assert!(validate_revisions(Some(&r), 20).is_err());
}
#[test]
fn bookmark_surrogate_cuts_column_flags_and_author_indices_stay_uncertain() {
    let s = source(&[0xd83d, 0xde00, 5, 13], &[5, 13, 13]);
    let mut b = blocks(&[2], &[0, 2, u32::MAX], &[7]);
    bookmarks_fixture(&mut b, &[7], &[1], &[2], &[0]);
    b[4][11] = 0x80;
    b[0][8 + 20] = 9;
    let out = inspect(&s, &b, None).unwrap();
    assert!(
        out.bookmarks[0]
            .unresolved
            .contains(&"bookmark_surrogate_boundary")
    );
    assert!(
        out.bookmarks[0]
            .unresolved
            .contains(&"annotation_bookmark_column_flag")
    );
    assert!(out.comments[0].unresolved.contains(&"comment_author_index"));
    assert_eq!(out.comments[0].anchor_status, "unresolved_bookmark");
}
fn extra(rows: &[(u32, i32, u32)]) -> Vec<u8> {
    rows.iter()
        .flat_map(|(depth, parent, flags)| {
            let mut b = vec![0; 6];
            b.extend(depth.to_le_bytes());
            b.extend(parent.to_le_bytes());
            b.extend(flags.to_le_bytes());
            b
        })
        .collect()
}
#[test]
fn extended_preorder_tree_preserves_ink_and_signed_parent_evidence() {
    let out = extended(
        &extra(&[(0, 0, 0), (1, -1, 2), (2, -1, 0), (1, -3, 0)]),
        4,
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(out[3].as_ref().unwrap().parent_index, Some(0));
    assert!(out[1].as_ref().unwrap().ink);
    assert!(
        out.iter()
            .all(|x| x.as_ref().unwrap().unresolved.is_empty())
    );
}
#[test]
fn extended_cycles_depth_jumps_and_size_mismatches_remain_unresolved() {
    for rows in [
        vec![(1, 0, 0)],
        vec![(0, 0, 0), (2, -1, 0)],
        vec![(1, 1, 0), (1, -1, 0)],
    ] {
        let out = extended(&extra(&rows), rows.len(), &mut Vec::new()).unwrap();
        assert!(
            out.iter()
                .any(|x| !x.as_ref().unwrap().unresolved.is_empty())
        );
    }
    let mut diag = Vec::new();
    let out = extended(&[0; 17], 1, &mut diag).unwrap();
    assert!(out[0].is_none());
    assert_eq!(diag, vec!["comment_extended_count_mismatch"]);
}
#[test]
fn author_and_comment_limits_precede_large_output_allocation() {
    assert!(authors(&[56, 0]).is_err());
    let s = source(&[13], &[]);
    let mut b = vec![Vec::new(); 7];
    b[0] = vec![0; 4 + 34 * (MAX_COMMENTS + 1)];
    assert!(matches!(
        inspect(&s, &b, None),
        Err(Error::Limit("comment count"))
    ));
}

#[test]
fn complete_compound_fixture_maps_comments_across_supported_versions() {
    use crate::office::doc::tests::{put16, put32};
    for version in [0xc1, 0xd9, 0x101, 0x10c, 0x112] {
        let mut data = crate::office::doc::tests::fixture();
        let b = blocks(&[0], &[0, 3, u32::MAX], &[u32::MAX]);
        let word = &mut data[1536..5632];
        put16(word, 156 + 183 * 8, version);
        put32(word, 92, 4);
        word[2048] = 5;
        word[2058..2062].copy_from_slice(&[5, 90, 13, 13]);
        let mut offset = 100;
        for (i, block) in [4, 5, 36, 37, 42, 43, 112].into_iter().zip(&b) {
            put32(word, 154 + i * 8, offset);
            put32(word, 158 + i * 8, block.len() as u32);
            offset += block.len() as u32;
        }
        let table = &mut data[5632..9728];
        put32(table, 9, 14);
        let mut offset = 100;
        for block in &b {
            table[offset..offset + block.len()].copy_from_slice(block);
            offset += block.len();
        }
        let source = crate::office::doc::inspect(&data).unwrap();
        let out = inspect_comments(&data, &source, None).unwrap();
        assert_eq!(out.binary_version, version);
        assert_eq!(out.comments.len(), 1);
        assert_eq!(
            out.comments[0].source,
            SourceRange {
                start_cp: 10,
                end_cp: 13
            }
        );
        assert_eq!(out.comments[0].body_reference.source_cp, 0);
    }
}

#[test]
fn ignored_bookmark_terminal_cps_preserve_raw_values_and_uncertainty() {
    let s = source(&[65, 5, 13], &[5, 13, 13]);
    let mut b = blocks(&[1], &[0, 2, u32::MAX], &[7]);
    bookmarks_fixture(&mut b, &[7], &[0], &[1], &[0]);
    b[2].extend([1, 0, 65, 0]);
    for terminal in [4, 5, u32::MAX] {
        b[4][4..8].copy_from_slice(&terminal.to_le_bytes());
        b[5][4..8].copy_from_slice(&terminal.to_le_bytes());
        let out = inspect(&s, &b, None).unwrap();
        assert_eq!(out.ignored_bookmark_start_cp, Some(terminal));
        assert_eq!(out.ignored_bookmark_end_cp, Some(terminal));
        assert_eq!(
            out.bookmarks[0].source,
            Some(SourceRange {
                start_cp: 0,
                end_cp: 1
            })
        );
        assert!(
            out.unresolved
                .contains(&"annotation_bookmark_terminal_cp_conformance")
        );
        assert_eq!(out.diagnostics, ["duplicate_comment_author"]);
    }
}

#[test]
fn mismatched_bookmark_tables_and_nonmonotone_plcs_are_rejected() {
    let s = source(&[65, 5, 66, 5, 13], &[5, 13, 5, 13, 13]);
    let mut b = blocks(&[1, 3], &[0, 2, 4, u32::MAX], &[7, 8]);
    bookmarks_fixture(&mut b, &[7, 8], &[0, 2], &[1, 3], &[0, 1]);
    b[4][..4].copy_from_slice(&3u32.to_le_bytes());
    assert!(inspect(&s, &b, None).is_err());
    b[4][..4].copy_from_slice(&0u32.to_le_bytes());
    b[3].pop();
    assert!(inspect(&s, &b, None).is_err());
}

#[test]
fn ignored_extended_bits_do_not_change_parent_or_ink_semantics() {
    let mut raw = extra(&[(0, 0, 0), (1, -1, 3)]);
    raw[18 + 4] = 1;
    let out = extended(&raw, 2, &mut Vec::new()).unwrap();
    let child = out[1].as_ref().unwrap();
    assert_eq!(child.parent_index, Some(0));
    assert!(child.ink);
    assert_eq!(child.unresolved, vec!["comment_extended_reserved_bits"]);
}

#[test]
fn pre_word2002_extension_bytes_do_not_fabricate_thread_metadata() {
    let mut s = source(&[5, 13], &[5, 13, 13]);
    s.binary_version = 0xd9;
    let mut b = blocks(&[0], &[0, 2, u32::MAX], &[u32::MAX]);
    b[6] = extra(&[(0, 0, 2)]);
    let out = inspect(&s, &b, None).unwrap();
    assert!(out.comments[0].extended.is_none());
    assert!(
        out.diagnostics
            .contains(&"comment_extended_version_unsupported")
    );
    assert_eq!(out.comments[0].anchor_status, "point_at_reference");
}
