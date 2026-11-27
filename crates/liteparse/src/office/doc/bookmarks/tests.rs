use super::*;
use crate::office::doc::tests::{fixture, put16, put32};

fn tables(names: &[&str], starts: &[u32], ends: &[u32], owners: &[u16]) -> [Vec<u8>; 3] {
    let count = names.len();
    let mut strings = vec![255, 255, count as u8, (count >> 8) as u8, 0, 0];
    for name in names {
        let units: Vec<_> = name.encode_utf16().collect();
        strings.extend_from_slice(&(units.len() as u16).to_le_bytes());
        for unit in units {
            strings.extend_from_slice(&unit.to_le_bytes());
        }
    }
    let mut first = vec![0; 4 + count * 8];
    let mut last = vec![0; 4 + count * 4];
    for i in 0..count {
        put32(&mut first, i * 4, starts[i]);
        put32(&mut last, i * 4, ends[i]);
        put16(&mut first, (count + 1) * 4 + i * 4, owners[i]);
    }
    // Terminal PLC values are retained independently of actual bookmark ends.
    put32(&mut first, count * 4, 11);
    put32(&mut last, count * 4, 11);
    [strings, first, last]
}

fn document(blocks: &[Vec<u8>; 3]) -> Vec<u8> {
    let mut data = fixture();
    let mut at = 128;
    for (index, block) in [21, 22, 23].into_iter().zip(blocks) {
        put32(&mut data, 1536 + 154 + index * 8, at as u32);
        put32(&mut data, 1536 + 158 + index * 8, block.len() as u32);
        data[5632 + at..5632 + at + block.len()].copy_from_slice(block);
        at += block.len();
    }
    data
}

#[test]
fn named_targets_join_end_indices_and_preserve_raw_story_controls() {
    let blocks = tables(
        &["_Outer", "Inner", "Foot"],
        &[0, 1, 5],
        &[3, 5, 10],
        &[1, 0, 2],
    );
    let data = document(&blocks);
    let before = crate::office::doc::inspect(&data).unwrap();
    let result = inspect_bookmarks(&data).unwrap();
    assert!(result.entries[0].hidden_name);
    assert_eq!(result.entries[0].raw_target_text.as_deref(), Some("Body\r"));
    assert_eq!(result.entries[1].raw_target_text.as_deref(), Some("od"));
    assert_eq!(result.entries[2].story, Some("footnotes"));
    assert_eq!(result.entries[2].raw_target_text.as_deref(), Some("Note\r"));
    assert_eq!(result.entries[0].source_cp, [0, 5]);
    assert_eq!(
        crate::office::doc::inspect(&data).unwrap().stories[0].text,
        before.stories[0].text
    );
}

#[test]
fn ambiguous_end_owners_and_incomplete_parallel_tables_are_rejected() {
    let mut blocks = tables(&["One", "Two"], &[0, 1], &[3, 5], &[0, 0]);
    assert_eq!(
        inspect_bookmarks(&document(&blocks)).unwrap_err(),
        Error::Invalid("bookmark end ownership")
    );
    put16(&mut blocks[1], 16, 1);
    blocks[2].pop();
    assert_eq!(
        inspect_bookmarks(&document(&blocks)).unwrap_err(),
        Error::Invalid("bookmark parallel table lengths")
    );
    let duplicate = tables(&["One", "One"], &[0, 1], &[3, 5], &[0, 1]);
    assert_eq!(
        inspect_bookmarks(&document(&duplicate)).unwrap_err(),
        Error::Invalid("duplicate bookmark name")
    );
}

#[test]
fn column_selection_does_not_export_unselected_table_text() {
    let mut blocks = tables(&["Column"], &[0], &[5], &[0]);
    put16(&mut blocks[1], 10, 0x8000);
    let result = inspect_bookmarks(&document(&blocks)).unwrap();
    assert!(result.entries[0].raw_target_text.is_none());
    assert!(
        result.entries[0]
            .unresolved
            .contains(&"column_selection_requires_table_projection")
    );
}

#[test]
fn out_of_story_ranges_stay_unresolved_without_losing_other_targets() {
    let blocks = tables(&["Cross", "Good"], &[3, 5], &[7, 10], &[0, 1]);
    let result = inspect_bookmarks(&document(&blocks)).unwrap();
    assert!(result.entries[0].raw_target_text.is_none());
    assert_eq!(result.entries[1].raw_target_text.as_deref(), Some("Note\r"));
}

#[test]
fn empty_targets_inside_surrogate_pairs_remain_unresolved() {
    let mut source = crate::office::doc::inspect(&fixture()).unwrap();
    source.stories[0].text = "A😀B\r".into();
    for (start, end, expected) in [(1, 3, Some("😀")), (2, 2, None), (3, 3, Some(""))] {
        let blocks = tables(&["Target"], &[start], &[end], &[0]);
        let result = decode(&blocks[0], &blocks[1], &blocks[2], &source).unwrap();
        assert_eq!(result.entries[0].raw_target_text.as_deref(), expected);
    }
}

#[test]
fn truncated_name_payloads_and_missing_tables_are_bounded_errors() {
    let blocks = tables(&["Target"], &[0], &[4], &[0]);
    let source = crate::office::doc::inspect(&fixture()).unwrap();
    for end in 0..blocks[0].len() {
        assert!(decode(&blocks[0][..end], &blocks[1], &blocks[2], &source).is_err());
    }
    assert!(inspect_bookmarks(&fixture()).unwrap().entries.is_empty());
}

#[test]
fn body_bookmarks_reach_the_converted_document() {
    // "_Toc1" covers the body's first word; "Foot" lies in the footnote story.
    let blocks = tables(&["_Toc1", "Foot"], &[0, 5], &[3, 10], &[0, 1]);
    let docx = crate::office::doc::docx_export::convert(&document(&blocks))
        .unwrap()
        .docx;
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(docx)).unwrap();
    let mut xml = String::new();
    std::io::Read::read_to_string(&mut zip.by_name("word/document.xml").unwrap(), &mut xml)
        .unwrap();
    assert!(xml.contains(r#"w:name="_Toc1""#), "{xml}");
    assert!(!xml.contains(r#"w:name="Foot""#));
}
