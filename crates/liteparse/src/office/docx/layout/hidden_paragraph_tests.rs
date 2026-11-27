use super::*;

fn paragraph(mark_hidden: bool, run_hidden: bool, text: &str) -> String {
    let hidden = |on| if on { "<w:vanish/>" } else { "" };
    format!(
        r#"<w:p><w:pPr><w:spacing w:before="400" w:after="600"/><w:pageBreakBefore/><w:pBdr><w:bottom w:val="single" w:sz="8"/></w:pBdr><w:shd w:fill="F2F2F2"/><w:rPr>{}</w:rPr></w:pPr><w:bookmarkStart w:id="1" w:name="hidden-position"/><w:r><w:rPr>{}</w:rPr><w:t xml:space="preserve">{text}</w:t></w:r><w:bookmarkEnd w:id="1"/></w:p>"#,
        hidden(mark_hidden),
        hidden(run_hidden),
    )
}

#[test]
fn hidden_paragraph_retains_bookmarks_without_flow_or_decoration() {
    let units = units(&paragraph(true, true, "hidden"));
    let Unit::Para(p) = &units[0] else {
        panic!("paragraph");
    };
    assert_eq!(p.before + p.after + p.pad_top + p.pad_bottom, 0.0);
    assert!(p.decor.is_none());
    assert!(!p.page_break_before);
    assert_eq!(p.lines.iter().map(|l| l.height).sum::<f64>(), 0.0);
    assert_eq!(
        p.lines
            .iter()
            .flat_map(|l| &l.items)
            .filter(|i| matches!(i, Item::Bookmark(_)))
            .count(),
        1
    );
}

#[test]
fn hidden_paragraph_at_a_full_page_end_keeps_one_page() {
    let body = format!(
        r#"<w:p><w:pPr><w:spacing w:line="2000" w:lineRule="exact"/></w:pPr><w:r><w:t>visible</w:t></w:r></w:p>{}<w:sectPr><w:pgSz w:w="12240" w:h="4880"/><w:pgMar w:top="1440" w:bottom="1440" w:left="1440" w:right="1440"/></w:sectPr>"#,
        paragraph(true, true, "hidden")
    );
    let pages = crate::office::docx::testutil::pages(docx(&body, None, None));
    assert_eq!(pages.len(), 1);
}

#[test]
fn hidden_paragraph_preserves_visible_marks_content_and_cell_marks() {
    for (mark_hidden, run_hidden, text, in_cell) in [
        (false, true, "hidden", false),
        (true, false, "visible", false),
        (true, false, " ", false),
        (true, true, "hidden", true),
    ] {
        let units = units_context(&paragraph(mark_hidden, run_hidden, text), in_cell, false);
        let Unit::Para(p) = &units[0] else {
            panic!("paragraph");
        };
        assert!(p.lines.iter().map(|l| l.height).sum::<f64>() > 0.0);
        assert!(p.decor.is_some());
    }
}

#[test]
fn hidden_paragraph_keeps_following_text_and_collapsed_spacing_in_place() {
    let visible = |text| {
        format!(
            r#"<w:p><w:pPr><w:spacing w:before="400" w:after="600"/></w:pPr><w:r><w:t>{text}</w:t></w:r></w:p>"#
        )
    };
    let before = visible("before");
    let after = visible("after");
    let baseline =
        crate::office::docx::testutil::pages(docx(&format!("{before}{after}"), None, None));
    let changed = crate::office::docx::testutil::pages(docx(
        &format!("{before}{}{after}", paragraph(true, true, "hidden")),
        None,
        None,
    ));
    assert_eq!(changed.len(), baseline.len());
    let text_positions = |pages: &[crate::office::docx::paginate::OutPage]| {
        pages
            .iter()
            .flat_map(|p| &p.items)
            .filter_map(|item| match item {
                Item::Text(run) => Some((run.text.clone(), run.transform.ty)),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(text_positions(&changed), text_positions(&baseline));
    assert!(
        !changed
            .iter()
            .flat_map(|p| &p.items)
            .any(|item| matches!(item, Item::Path(_)))
    );
}
