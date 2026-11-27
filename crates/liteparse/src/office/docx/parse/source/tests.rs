use super::*;

#[test]
fn native_table_fragment_eligibility_keeps_nontext_source_content() {
    for (body, expected) in [
        (
            "<w:tr><w:tc><w:p><w:r><w:t>TEXT</w:t></w:r></w:p></w:tc><w:tc><w:p/></w:tc></w:tr>",
            true,
        ),
        ("<w:tr><w:tc><w:p/></w:tc></w:tr>", false),
        (
            "<w:tr><w:tc><w:p><w:r><w:t>TEXT</w:t><w:drawing/></w:r></w:p></w:tc></w:tr>",
            false,
        ),
        ("<w:tr><w:tc><w:tbl/></w:tc></w:tr>", false),
    ] {
        let node = crate::office::pptx::xml::parse_word(&format!(
            r#"<w:tbl xmlns:w="{}">{body}</w:tbl>"#,
            crate::office::docx::testutil::W
        ))
        .unwrap();
        assert_eq!(text_only_table(&node), expected);
    }
}

fn merge_rows(body: &str) -> Vec<Vec<crate::office::pptx::scene::SourceVerticalMerge>> {
    let node = crate::office::pptx::xml::parse_word(&format!(
        r#"<w:tbl xmlns:w="{}">{body}</w:tbl>"#,
        crate::office::docx::testutil::W
    ))
    .unwrap();
    let mut rows = Vec::new();
    collect_named(&node.children, "w:tr", &mut rows);
    let grids = super::super::grid::Budget::document()
        .table(&node, &rows)
        .unwrap();
    vertical_merges(&rows, &grids)
}

#[test]
fn source_merges_follow_grid_intervals_across_cell_partition_changes() {
    let rows = merge_rows(
        r#"
      <w:tr><w:tc><w:tcPr><w:gridSpan w:val="2"/></w:tcPr></w:tc><w:tc><w:tcPr><w:vMerge w:val="restart"/></w:tcPr></w:tc></w:tr>
      <w:tr><w:tc/><w:tc/><w:tc><w:tcPr><w:vMerge/></w:tcPr></w:tc></w:tr>"#,
    );
    assert_eq!(rows[0][0].cell, 1);
    assert_eq!(rows[1][0].cell, 2);
    assert_eq!(
        (
            rows[1][0].grid_start,
            rows[1][0].span,
            rows[1][0].start_row,
            rows[1][0].end_row
        ),
        (2, 1, 0, 1)
    );
}

#[test]
fn source_merges_keep_parallel_chains_and_ordinary_empty_cells_distinct() {
    let row = |merge: &str| {
        format!(
            r#"<w:tr><w:trPr><w:gridBefore w:val="1"/></w:trPr><w:tc><w:tcPr><w:vMerge {merge}/></w:tcPr></w:tc><w:tc/><w:tc><w:tcPr><w:gridSpan w:val="2"/><w:vMerge {merge}/></w:tcPr></w:tc></w:tr>"#
        )
    };
    let rows = merge_rows(&(row(r#"w:val="restart""#) + &row("")));
    for row in rows {
        assert_eq!(row.len(), 2);
        assert_eq!((row[0].cell, row[0].grid_start, row[0].span), (0, 1, 1));
        assert_eq!((row[1].cell, row[1].grid_start, row[1].span), (2, 3, 2));
    }
}

#[test]
fn source_merges_reject_orphans_gaps_and_changed_spans() {
    let rows = merge_rows(
        r#"
      <w:tr><w:tc><w:tcPr><w:vMerge/></w:tcPr></w:tc></w:tr>
      <w:tr><w:tc><w:tcPr><w:vMerge w:val="restart"/></w:tcPr></w:tc></w:tr>
      <w:tr><w:tc/></w:tr>
      <w:tr><w:tc><w:tcPr><w:vMerge/></w:tcPr></w:tc></w:tr>
      <w:tr><w:tc><w:tcPr><w:vMerge w:val="restart"/></w:tcPr></w:tc></w:tr>
      <w:tr><w:tc><w:tcPr><w:gridSpan w:val="2"/><w:vMerge/></w:tcPr></w:tc></w:tr>"#,
    );
    assert!(rows.iter().all(Vec::is_empty));
}

#[test]
fn hyperlink_field_arguments_decode_escapes_without_becoming_switches() {
    for (instruction, expected) in [
        (
            r#"HYPERLINK"https://example.test/base"\l"section"\o"tip""#,
            Some("https://example.test/base#section"),
        ),
        (
            r#"HYPERLINK \o"""https://example.test/base"\t"""#,
            Some("https://example.test/base"),
        ),
        (r#"HYPERLINKX "https://example.test/base""#, None),
        (
            r#"HYPERLINK "\\\\server\\share\\file.txt""#,
            Some(r"\\server\share\file.txt"),
        ),
        (r#"HYPERLINK "E:\\ReadMe.txt""#, Some(r"E:\ReadMe.txt")),
        (
            r#"HYPERLINK "https://example.test/a\"b""#,
            Some("https://example.test/a\"b"),
        ),
        (
            r#"HYPERLINK "https://example.test/a\q""#,
            Some(r"https://example.test/a\q"),
        ),
        (
            r#"HYPERLINK "https://example.test/base" \l "section" \o "tip""#,
            Some("https://example.test/base#section"),
        ),
        (
            r#"HYPERLINK \o "" "https://example.test/base#existing" \t """#,
            Some("https://example.test/base#existing"),
        ),
        (r#"HYPERLINK "" \o "not a target" \l "bookmark""#, None),
        (r#"HYPERLINK \l "bookmark""#, None),
        (r#"HYPERLINK "\\l""#, Some(r"\l")),
    ] {
        assert_eq!(
            hyperlink_instruction(instruction).as_deref(),
            expected,
            "{instruction}"
        );
    }
}

#[test]
fn reference_field_navigation_requires_a_real_h_switch() {
    for kind in ["REF", "PAGEREF", "ref", "pageref"] {
        for (arguments, expected) in [
            (r#"target \h"#, Some("target")),
            (r#""target"\h"#, Some("target")),
            (r#"target \* MERGEFORMAT \h"#, Some("target")),
            (r#"target"#, None),
            (r#"target \hello"#, None),
            (r#"target \hfoo"#, None),
            (r#"target \unknown \h"#, None),
            (r#"target \h \d"#, None),
            (r#"target \h \*"#, None),
            (r#"target "\h""#, None),
            (r#"target \* \h"#, None),
            (r#"target \# "\h""#, None),
            (r#"target \d "\h""#, None),
            (r#""" \h"#, None),
        ] {
            let instruction = format!("{kind} {arguments}");
            let target = hyperlink_target(&instruction, "word/document.xml");
            let name = match &target {
                Some(SourceLinkTarget::Bookmark { part, name }) => {
                    assert_eq!(part, "word/document.xml");
                    Some(name.as_str())
                }
                None => None,
                _ => panic!("unexpected external target"),
            };
            assert_eq!(name, expected, "{instruction}");
        }
    }
}
