use super::*;
use crate::office::docx::testutil::docx;

fn fixture(
    body: &str,
) -> (
    crate::office::docx::extract::Opened,
    crate::office::pptx::fonts::FontSystem,
    Vec<OutPage>,
) {
    let opened =
        crate::office::docx::extract::open(docx(body, None, None), &Default::default()).unwrap();
    let fonts = crate::office::pptx::fonts::FontSystem::new(&[], true, &Default::default());
    let pages = crate::office::docx::extract::layout(&opened, &fonts).pages;
    (opened, fonts, pages)
}

fn output(
    index: &Index,
    page: &OutPage,
    number: u32,
    fonts: &crate::office::pptx::fonts::FontSystem,
) -> (Value, Value) {
    let scene =
        crate::office::docx::extract::reading_scene(&crate::office::docx::extract::scene_of(page));
    let mut text =
        crate::office::docx::extract::dedupe_text(crate::office::pptx::output::source_text_json(
            fonts,
            &scene,
            true,
            (0.0, 0.0),
            Some((page.info.w, page.info.h)),
        ));
    index.resolve_text(&mut text);
    let mut tree = super::super::source::structure(&mut text);
    index.decorate(&mut tree, number);
    (text, tree)
}

fn destinations(tree: &Value) -> Vec<Value> {
    let mut out = Vec::new();
    if let Some(destination) = tree.pointer("/annotations/0/destination") {
        out.push(destination.clone());
    }
    for child in tree["children"].as_array().into_iter().flatten() {
        out.extend(destinations(child));
    }
    out
}

#[test]
fn bookmark_resolution_distinguishes_missing_duplicate_unplaced_and_empty_target() {
    let links = ["empty", "missing", "duplicate", "unplaced"]
        .map(|name| {
            format!(r#"<w:hyperlink w:anchor="{name}"><w:r><w:t>SAME</w:t></w:r></w:hyperlink>"#)
        })
        .join("");
    let body = format!(
        r#"<w:p>{links}</w:p>
      <w:p><w:pPr><w:pageBreakBefore/></w:pPr><w:bookmarkStart w:id="1" w:name="empty"/><w:bookmarkEnd w:id="1"/></w:p>
      <w:p><w:bookmarkStart w:id="2" w:name="duplicate"/><w:bookmarkEnd w:id="2"/></w:p>
      <w:p><w:bookmarkStart w:id="2" w:name="duplicate"/><w:bookmarkEnd w:id="2"/></w:p>
"#
    );
    let (mut opened, fonts, pages) = fixture(&body);
    let mut unplaced = (*opened.bookmarks[0]).clone();
    unplaced.id.push_str("-unplaced");
    unplaced.name = "unplaced".into();
    opened.bookmarks.push(Arc::new(unplaced));
    assert_eq!(pages.len(), 2);
    let index = Index::new(&opened.bookmarks, &pages);
    let (text, tree) = output(&index, &pages[0], 0, &fonts);
    let targets = destinations(&tree);
    assert_eq!(
        targets
            .iter()
            .map(|target| target["status"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["resolved", "missing", "ambiguous", "unplaced"]
    );
    assert_eq!(targets[0]["pageIndex"], 1);
    for (i, item) in text["items"].as_array().unwrap().iter().enumerate() {
        assert_eq!(item["mcid"], i);
    }
    let (target_text, target_tree) = output(&index, &pages[1], 1, &fonts);
    assert!(target_text["items"].as_array().unwrap().is_empty());
    let marker = &target_tree["destinations"][0];
    assert_eq!(marker["id"], targets[0]["id"]);
    assert_eq!(marker["canonical"], true);
    assert_eq!(marker["textItemBoundary"], 0);
    assert!(marker.get("mcid").is_none());
}

#[test]
fn bookmark_repeated_instances_keep_first_full_document_occurrence_when_page_is_selected_alone() {
    let (opened, fonts, mut pages) = fixture(
        r#"<w:p><w:bookmarkStart w:id="1" w:name="repeat"/><w:r><w:t>TARGET</w:t></w:r><w:bookmarkEnd w:id="1"/></w:p>"#,
    );
    pages.push(OutPage {
        info: pages[0].info.clone(),
        items: pages[0].items.clone(),
    });
    let index = Index::new(&opened.bookmarks, &pages);
    let (_, tree) = output(&index, &pages[1], 1, &fonts);
    assert_eq!(tree["destinations"][0]["canonical"], false);
    assert_eq!(tree["destinations"][0]["referenced"], false);
    assert_eq!(tree["destinations"][0]["pageIndex"], 1);
    let mut target =
        json!({"kind":"bookmark","sourcePart":"word/header1.xml","bookmarkName":"repeat"});
    index.resolve(&mut target);
    assert_eq!(target["pageIndex"], 0);
    assert_eq!(target["targetPart"], "word/document.xml");
    assert_eq!(target["sourcePart"], "word/header1.xml");
}

#[test]
fn bookmark_dedupe_retains_distinct_source_boundaries_and_final_mcid() {
    let (opened, fonts, mut pages) = fixture(
        r#"<w:p><w:bookmarkStart w:id="1" w:name="first"/><w:r><w:t>TARGET</w:t></w:r><w:bookmarkEnd w:id="1"/></w:p>"#,
    );
    let mut copy = pages[0].items.clone();
    let marker = copy
        .iter_mut()
        .find_map(|item| match item {
            Item::Text(run) => run
                .bookmarks
                .as_mut()
                .and_then(|bookmarks| bookmarks.boundaries.first_mut()),
            _ => None,
        })
        .unwrap();
    let mut second = (*marker.source).clone();
    second.id.push_str("-second");
    second.name = "second".into();
    marker.source = Arc::new(second);
    let mut definitions = opened.bookmarks;
    definitions.push(marker.source.clone());
    pages[0].items.extend(copy);
    let index = Index::new(&definitions, &pages);
    let (text, tree) = output(&index, &pages[0], 0, &fonts);
    assert_eq!(text["items"].as_array().unwrap().len(), 1);
    assert_eq!(tree["destinations"].as_array().unwrap().len(), 2);
    for marker in tree["destinations"].as_array().unwrap() {
        assert_eq!(marker["mcid"], 0);
        assert_eq!(marker["utf16Offset"], 0);
    }
}

#[test]
fn bookmark_block_boundaries_preserve_split_table_paint_and_container_ownership() {
    let marker = |name: &str| {
        format!(r#"<w:bookmarkStart w:id="1" w:name="{name}"/><w:bookmarkEnd w:id="1"/>"#)
    };
    let before = "<w:p><w:r><w:t>BEFORE</w:t></w:r></w:p>";
    let lines = (0..35)
        .map(|i| format!("<w:p><w:r><w:t>ROW{i:02}</w:t></w:r></w:p>"))
        .collect::<String>();
    let table = format!(
        r#"<w:tbl><w:tblPr><w:tblLayout w:type="fixed"/></w:tblPr><w:tblGrid><w:gridCol w:w="4000"/></w:tblGrid><w:tr><w:tc>{lines}</w:tc></w:tr></w:tbl>"#
    );
    let section = r#"<w:sectPr><w:pgSz w:w="6000" w:h="4000"/><w:pgMar w:top="400" w:bottom="400" w:left="400" w:right="400"/></w:sectPr>"#;
    let base = format!("{before}{table}{section}");
    let marked = format!(
        "{}{before}{}{table}{}{section}",
        marker("start"),
        marker("table-start"),
        marker("table-end")
    );
    let (_, _, baseline) = fixture(&base);
    let (opened, fonts, pages) = fixture(&marked);
    assert!(pages.len() > 1);
    assert_eq!(pages.len(), baseline.len());
    let index = Index::new(&opened.bookmarks, &pages);
    let mut targets = Vec::new();
    for (number, (page, old)) in pages.iter().zip(&baseline).enumerate() {
        let paint = |page: &OutPage| {
            let mut items = page.items.clone();
            items.retain(|item| !matches!(item, Item::Bookmark(_)));
            for item in &mut items {
                if let Item::Text(run) = item {
                    run.bookmarks = None;
                }
            }
            format!("{items:?}")
        };
        assert_eq!(paint(page), paint(old));
        let (_, tree) = output(&index, page, number as u32, &fonts);
        targets.extend(
            tree["destinations"]
                .as_array()
                .into_iter()
                .flatten()
                .cloned(),
        );
    }
    assert_eq!(targets.len(), 3);
    for target in &targets {
        assert!(target.get("sourceParagraphId").is_none());
        assert!(target["cells"].as_array().unwrap().is_empty());
    }
    let start = targets
        .iter()
        .find(|t| t["sourceBookmarkName"] == "table-start")
        .unwrap();
    let end = targets
        .iter()
        .find(|t| t["sourceBookmarkName"] == "table-end")
        .unwrap();
    assert_eq!(start["pageIndex"], 0);
    assert_eq!(end["pageIndex"], pages.len() - 1);
    assert_eq!(start["textItemBoundary"], 1);
    assert!(end["textItemBoundary"].as_u64().unwrap() > 0);
}

#[test]
fn bookmark_links_from_header_and_footnote_resolve_in_document_namespace() {
    use crate::office::docx::testutil::{W, with_related_part};
    let body = r#"<w:p><w:bookmarkStart w:id="1" w:name="body-target"/><w:r><w:t>BODY</w:t><w:footnoteReference w:id="2"/></w:r><w:bookmarkEnd w:id="1"/></w:p><w:sectPr><w:headerReference w:type="default" r:id="rheader"/></w:sectPr>"#;
    let header = format!(
        r#"<w:hdr xmlns:w="{W}"><w:p><w:hyperlink w:anchor="body-target"><w:r><w:t>HEADER_LINK</w:t></w:r></w:hyperlink></w:p></w:hdr>"#
    );
    let footnotes = format!(
        r#"<w:footnotes xmlns:w="{W}"><w:footnote w:id="2"><w:p><w:hyperlink w:anchor="body-target"><w:r><w:t>NOTE_LINK</w:t></w:r></w:hyperlink></w:p></w:footnote></w:footnotes>"#
    );
    let bytes = with_related_part(
        with_related_part(
            docx(body, None, None),
            "header",
            "header1.xml",
            &header,
            false,
        ),
        "footnotes",
        "footnotes.xml",
        &footnotes,
        false,
    );
    let opened = crate::office::docx::extract::open(bytes, &Default::default()).unwrap();
    let fonts = crate::office::pptx::fonts::FontSystem::new(&[], true, &Default::default());
    let pages = crate::office::docx::extract::layout(&opened, &fonts).pages;
    let index = Index::new(&opened.bookmarks, &pages);
    let (_, tree) = output(&index, &pages[0], 0, &fonts);
    let targets = destinations(&tree);
    assert_eq!(targets.len(), 2);
    let parts = targets
        .iter()
        .map(|target| target["sourcePart"].as_str().unwrap())
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(
        parts,
        ["word/header1.xml", "word/footnotes.xml"]
            .into_iter()
            .collect()
    );
    for target in targets {
        assert_eq!(target["status"], "resolved");
        assert_eq!(target["targetPart"], "word/document.xml");
        assert_eq!(target["pageIndex"], 0);
    }
}

#[test]
fn bookmark_modern_failed_external_relationship_does_not_become_internal() {
    let body = r#"<w:p><w:bookmarkStart w:id="1" w:name="target"/><w:r><w:t>TARGET</w:t></w:r><w:bookmarkEnd w:id="1"/></w:p><w:p><w:hyperlink r:id="missing" w:anchor="target"><w:r><w:t>PLAIN</w:t></w:r></w:hyperlink><w:hyperlink w:anchor="target"><w:r><w:t>LINK</w:t></w:r></w:hyperlink></w:p>"#;
    let (opened, fonts, pages) = fixture(body);
    let index = Index::new(&opened.bookmarks, &pages);
    let (text, tree) = output(&index, &pages[0], 0, &fonts);
    assert_eq!(destinations(&tree).len(), 1);
    let plain = text["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["unicode"] == "PLAIN")
        .unwrap();
    assert!(plain.pointer("/nativeSource/link").is_none());
}

#[test]
fn bookmark_empty_note_target_keeps_authoritative_note_reading_boundary() {
    use crate::office::docx::testutil::{W, with_related_part};
    let body = r#"<w:p><w:r><w:t>BODY</w:t><w:footnoteReference w:id="2"/></w:r></w:p>"#;
    let notes = format!(
        r#"<w:footnotes xmlns:w="{W}"><w:footnote w:id="2"><w:p><w:r><w:t>NOTE_BEFORE</w:t></w:r></w:p><w:p><w:bookmarkStart w:id="3" w:name="note-target"/><w:bookmarkEnd w:id="3"/></w:p><w:p><w:r><w:t>NOTE_AFTER</w:t></w:r></w:p></w:footnote></w:footnotes>"#
    );
    let opened = crate::office::docx::extract::open(
        with_related_part(
            docx(body, None, None),
            "footnotes",
            "footnotes.xml",
            &notes,
            false,
        ),
        &Default::default(),
    )
    .unwrap();
    let fonts = crate::office::pptx::fonts::FontSystem::new(&[], true, &Default::default());
    let pages = crate::office::docx::extract::layout(&opened, &fonts).pages;
    let index = Index::new(&opened.bookmarks, &pages);
    let (text, tree) = output(&index, &pages[0], 0, &fonts);
    let items = text["items"].as_array().unwrap();
    let before = items
        .iter()
        .position(|item| item["unicode"] == "NOTE_BEFORE")
        .unwrap();
    let after = items
        .iter()
        .position(|item| item["unicode"] == "NOTE_AFTER")
        .unwrap();
    let target = &tree["destinations"][0];
    assert_eq!(target["sourcePart"], "word/footnotes.xml");
    let boundary = target["textItemBoundary"].as_u64().unwrap() as usize;
    assert!(before < boundary && boundary <= after);
}

#[test]
fn bookmark_repeated_header_reuses_one_source_identity() {
    use crate::office::docx::testutil::{W, with_related_part};
    let body = r#"<w:p><w:r><w:t>PAGE_ONE</w:t></w:r></w:p><w:p><w:pPr><w:pageBreakBefore/></w:pPr><w:r><w:t>PAGE_TWO</w:t></w:r></w:p><w:sectPr><w:headerReference w:type="default" r:id="rheader"/></w:sectPr>"#;
    let header = format!(
        r#"<w:hdr xmlns:w="{W}"><w:p><w:bookmarkStart w:id="8" w:name="header-target"/><w:r><w:t>HEADER</w:t></w:r><w:bookmarkEnd w:id="8"/></w:p></w:hdr>"#
    );
    let opened = crate::office::docx::extract::open(
        with_related_part(
            docx(body, None, None),
            "header",
            "header1.xml",
            &header,
            false,
        ),
        &Default::default(),
    )
    .unwrap();
    assert_eq!(opened.bookmarks.len(), 1);
    let fonts = crate::office::pptx::fonts::FontSystem::new(&[], true, &Default::default());
    let pages = crate::office::docx::extract::layout(&opened, &fonts).pages;
    assert_eq!(pages.len(), 2);
    let index = Index::new(&opened.bookmarks, &pages);
    let (_, first) = output(&index, &pages[0], 0, &fonts);
    let (_, second) = output(&index, &pages[1], 1, &fonts);
    assert_eq!(
        first["destinations"][0]["id"],
        second["destinations"][0]["id"]
    );
    assert_eq!(first["destinations"][0]["canonical"], true);
    assert_eq!(second["destinations"][0]["canonical"], false);
}

#[test]
fn bookmark_simple_complex_and_outer_toc_links_preserve_cached_labels() {
    let body = r#"<w:p><w:fldSimple w:instr="HYPERLINK \l &quot;target&quot;"><w:r><w:t>SIMPLE</w:t></w:r></w:fldSimple><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText>HYPERLINK \l "target"</w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>COMPLEX</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r><w:hyperlink w:anchor="target"><w:r><w:t>TOC</w:t></w:r><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText>PAGEREF target \h</w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>99</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r></w:hyperlink></w:p><w:p><w:pPr><w:pageBreakBefore/></w:pPr><w:bookmarkStart w:id="1" w:name="target"/><w:bookmarkEnd w:id="1"/></w:p>"#;
    let (opened, fonts, pages) = fixture(body);
    let index = Index::new(&opened.bookmarks, &pages);
    let (text, tree) = output(&index, &pages[0], 0, &fonts);
    let targets = destinations(&tree);
    assert_eq!(targets.len(), 3);
    for target in targets {
        assert_eq!(target["pageIndex"], 1);
        assert_eq!(target["status"], "resolved");
    }
    let items = text["items"].as_array().unwrap();
    let toc = items
        .iter()
        .find(|item| item["unicode"].as_str().unwrap().starts_with("TOC"))
        .unwrap();
    let owner = toc.pointer("/nativeSource/link/id").unwrap();
    let owned = items
        .iter()
        .filter(|item| item.pointer("/nativeSource/link/id") == Some(owner))
        .collect::<Vec<_>>();
    assert_eq!(
        owned
            .iter()
            .map(|item| item["unicode"].as_str().unwrap())
            .collect::<String>(),
        "TOC99"
    );
    for item in owned {
        assert!(item.pointer("/nativeSource/link/destination").is_some());
    }
}

#[test]
fn bookmark_referenced_flag_uses_resolved_links_across_full_document() {
    let body = r#"<w:p><w:bookmarkStart w:id="1" w:name="used"/><w:r><w:t>TARGET</w:t></w:r><w:bookmarkEnd w:id="1"/></w:p>
    <w:p><w:bookmarkStart w:id="2" w:name="unused"/><w:bookmarkEnd w:id="2"/></w:p>
    <w:p><w:bookmarkStart w:id="3" w:name="duplicate"/><w:bookmarkEnd w:id="3"/><w:bookmarkStart w:id="4" w:name="duplicate"/><w:bookmarkEnd w:id="4"/></w:p>
    <w:p><w:pPr><w:pageBreakBefore/></w:pPr><w:hyperlink w:anchor="used"><w:r><w:t>JUMP</w:t></w:r></w:hyperlink><w:hyperlink w:anchor="duplicate"><w:r><w:t>AMBIGUOUS</w:t></w:r></w:hyperlink></w:p>"#;
    let (opened, fonts, pages) = fixture(body);
    let index = Index::new(&opened.bookmarks, &pages);
    let (_, tree) = output(&index, &pages[0], 0, &fonts);
    let markers = tree["destinations"].as_array().unwrap();
    assert_eq!(markers.len(), 4);
    for marker in markers {
        assert_eq!(marker["referenced"], marker["sourceBookmarkName"] == "used");
    }
    let (_, links_only) = output(&index, &pages[1], 1, &fonts);
    assert!(
        links_only["destinations"]
            .as_array()
            .is_none_or(|markers| markers.is_empty())
    );
    assert_eq!(destinations(&links_only)[0]["pageIndex"], 0);
}

#[test]
fn bookmark_inside_owned_combining_cluster_keeps_export_boundary() {
    let properties = r#"<w:rPr><w:rFonts w:ascii="Arial"/><w:kern w:val="2"/></w:rPr>"#;
    let body = format!(
        r#"<w:p><w:hyperlink w:anchor="inside"><w:r>{properties}<w:t>e</w:t></w:r></w:hyperlink><w:bookmarkStart w:id="1" w:name="inside"/><w:bookmarkEnd w:id="1"/><w:r>{properties}<w:t>́</w:t></w:r></w:p>"#
    );
    let (opened, fonts, pages) = fixture(&body);
    let index = Index::new(&opened.bookmarks, &pages);
    let (text, tree) = output(&index, &pages[0], 0, &fonts);
    assert_eq!(tree["destinations"].as_array().unwrap().len(), 1);
    let marker = &tree["destinations"][0];
    assert_eq!(marker["referenced"], true);
    assert_eq!(marker["utf16Offset"], 0);
    let item = &text["items"][marker["mcid"].as_u64().unwrap() as usize];
    assert_eq!(item["unicode"], "́");
    let targets = destinations(&tree);
    assert_eq!(targets[0]["status"], "resolved");
}

#[test]
fn reference_fields_link_cached_results_without_changing_text_or_geometry() {
    let simple = |instruction: &str, label: &str| {
        format!(
            r#"<w:fldSimple w:instr="{instruction}"><w:r><w:t>{label}</w:t></w:r></w:fldSimple>"#
        )
    };
    let complex = |instruction: &str, label: &str| {
        format!(
            r#"<w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText>{instruction}</w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>{label}</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r>"#
        )
    };
    let cases = [
        (
            simple(r"REF target \h", "SIMPLE"),
            "SIMPLE",
            Some("resolved"),
        ),
        (
            complex(r"PAGEREF target \h", "999"),
            "999",
            Some("resolved"),
        ),
        (
            simple(r"REF missing \h", "MISSING"),
            "MISSING",
            Some("missing"),
        ),
        (complex(r"REF target", "PLAIN"), "PLAIN", None),
        (
            simple(r"PAGEREF target \* MERGEFORMAT", "FORMAT"),
            "FORMAT",
            None,
        ),
        (complex(r#"REF target "\h""#, "QUOTED"), "QUOTED", None),
        (complex(r"REF target \* \h", "ARGUMENT"), "ARGUMENT", None),
    ];
    let target = r#"<w:p><w:bookmarkStart w:id="1" w:name="target"/><w:r><w:t>TARGET</w:t></w:r><w:bookmarkEnd w:id="1"/></w:p>"#;
    let body = target.to_owned()
        + &cases
            .iter()
            .map(|(xml, _, _)| format!("<w:p>{xml}</w:p>"))
            .collect::<String>();
    let plain = target.to_owned()
        + &cases
            .iter()
            .map(|(_, label, _)| format!("<w:p><w:r><w:t>{label}</w:t></w:r></w:p>"))
            .collect::<String>();
    let (opened, fonts, pages) = fixture(&body);
    let (baseline, _, old_pages) = fixture(&plain);
    assert_eq!(pages.len(), old_pages.len());
    let index = Index::new(&opened.bookmarks, &pages);
    let old_index = Index::new(&baseline.bookmarks, &old_pages);
    let (text, tree) = output(&index, &pages[0], 0, &fonts);
    let (old_text, _) = output(&old_index, &old_pages[0], 0, &fonts);
    let strip = |mut text: Value| {
        for item in text["items"].as_array_mut().unwrap() {
            for key in ["nativeSource", "nativeBookmarks", "mcid"] {
                item.as_object_mut().unwrap().remove(key);
            }
        }
        text
    };
    assert_eq!(strip(text), strip(old_text));
    let targets = destinations(&tree);
    assert_eq!(
        targets
            .iter()
            .map(|target| target["status"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["resolved", "resolved", "missing"]
    );
}

#[test]
fn reference_fields_preserve_nested_result_ownership_and_ignore_instruction_results() {
    let complex = |instruction: &str, result: &str| {
        format!(
            r#"<w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText>{instruction}</w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r>{result}<w:r><w:fldChar w:fldCharType="end"/></w:r>"#
        )
    };
    let text = |label: &str| format!("<w:r><w:t>{label}</w:t></w:r>");
    let simple =
        r#"<w:fldSimple w:instr="REF target"><w:r><w:t>SIMPLE_INNER</w:t></w:r></w:fldSimple>"#;
    let nested = complex(
        r"REF target \h",
        &complex("REF target", &text("COMPLEX_INNER")),
    );
    let modern = format!(r#"<w:hyperlink w:anchor="target">{simple}</w:hyperlink>"#);
    let instruction = format!(
        r#"<w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText>REF target</w:instrText></w:r>{}<w:r><w:instrText> \h</w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r>{}<w:r><w:fldChar w:fldCharType="end"/></w:r>"#,
        complex("REF target", &text("target")),
        text("DYNAMIC")
    );
    let simple_instruction = instruction
        .replace(&complex("REF target", &text("target")), simple)
        .replace("DYNAMIC", "SIMPLE_DYNAMIC");
    let body = format!(
        r#"<w:p><w:bookmarkStart w:id="1" w:name="target"/><w:bookmarkEnd w:id="1"/></w:p><w:p>{nested}</w:p><w:p>{modern}</w:p><w:p>{instruction}</w:p><w:p>{simple_instruction}</w:p>"#
    );
    let (opened, fonts, pages) = fixture(&body);
    let index = Index::new(&opened.bookmarks, &pages);
    let (text, tree) = output(&index, &pages[0], 0, &fonts);
    assert_eq!(destinations(&tree).len(), 2);
    let items = text["items"].as_array().unwrap();
    assert_eq!(
        items
            .iter()
            .map(|item| item["unicode"].as_str().unwrap())
            .collect::<String>(),
        "COMPLEX_INNERSIMPLE_INNERDYNAMICSIMPLE_DYNAMIC"
    );
    for label in ["COMPLEX_INNER", "SIMPLE_INNER"] {
        let item = items.iter().find(|item| item["unicode"] == label).unwrap();
        assert_eq!(
            item.pointer("/nativeSource/link/destination/status")
                .and_then(Value::as_str),
            Some("resolved")
        );
    }
    let dynamic = items
        .iter()
        .find(|item| item["unicode"] == "DYNAMIC")
        .unwrap();
    assert!(dynamic.pointer("/nativeSource/link").is_none());
}

#[test]
fn reference_fields_choose_the_innermost_result_link_scope() {
    let complex = |name: &str, result: &str| {
        format!(
            r#"<w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText>REF {name} \h</w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r>{result}<w:r><w:fldChar w:fldCharType="end"/></w:r>"#
        )
    };
    let simple = |name: &str, result: &str| {
        format!(r#"<w:fldSimple w:instr="REF {name} \h">{result}</w:fldSimple>"#)
    };
    let text = |label: &str| format!("<w:r><w:t>{label}</w:t></w:r>");
    let cases = [
        simple(
            "outer",
            &(text("BEFORE") + &complex("inner", &text("INNER_COMPLEX")) + &text("AFTER")),
        ),
        complex("outer", &simple("inner", &text("INNER_SIMPLE"))),
        complex("outer", &complex("inner", &text("INNER_NESTED"))),
    ];
    let body = format!(
        r#"<w:p><w:bookmarkStart w:id="1" w:name="outer"/><w:bookmarkEnd w:id="1"/><w:bookmarkStart w:id="2" w:name="inner"/><w:bookmarkEnd w:id="2"/></w:p>{}"#,
        cases
            .iter()
            .map(|case| format!("<w:p>{case}</w:p>"))
            .collect::<String>()
    );
    let (opened, fonts, pages) = fixture(&body);
    let index = Index::new(&opened.bookmarks, &pages);
    let (text, _) = output(&index, &pages[0], 0, &fonts);
    for item in text["items"].as_array().unwrap() {
        let label = item["unicode"].as_str().unwrap();
        let expected = if label.starts_with("INNER") {
            "inner"
        } else {
            "outer"
        };
        assert_eq!(
            item.pointer("/nativeSource/link/destination/bookmarkName")
                .and_then(Value::as_str),
            Some(expected),
            "{label}"
        );
    }
}

#[test]
fn reference_fields_keep_explicit_hyperlink_targets_for_nested_cached_results() {
    let field = |kind: &str, result: &str| {
        format!(
            r#"<w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText>{kind}</w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r>{result}<w:r><w:fldChar w:fldCharType="end"/></w:r>"#
        )
    };
    let text = |label: &str| format!("<w:r><w:t>{label}</w:t></w:r>");
    let modern = format!(
        r#"<w:hyperlink w:anchor="outer">{}</w:hyperlink>"#,
        field(r"REF inner \h", &text("MODERN"))
    );
    let external = field(
        r#"HYPERLINK "https://example.test/document#outer""#,
        &field(r"PAGEREF inner \h", &text("EXTERNAL")),
    );
    let simple = format!(
        r#"<w:fldSimple w:instr="HYPERLINK \l &quot;outer&quot;">{}</w:fldSimple>"#,
        field(r"REF inner \h", &text("SIMPLE"))
    );
    let body = format!(
        r#"<w:p><w:bookmarkStart w:id="1" w:name="outer"/><w:bookmarkEnd w:id="1"/><w:bookmarkStart w:id="2" w:name="inner"/><w:bookmarkEnd w:id="2"/></w:p><w:p>{modern}</w:p><w:p>{external}</w:p><w:p>{simple}</w:p>"#
    );
    let (opened, fonts, pages) = fixture(&body);
    let index = Index::new(&opened.bookmarks, &pages);
    let (text, _) = output(&index, &pages[0], 0, &fonts);
    for item in text["items"].as_array().unwrap() {
        if item["unicode"] == "EXTERNAL" {
            assert_eq!(
                item.pointer("/nativeSource/link/uri")
                    .and_then(Value::as_str),
                Some("https://example.test/document#outer")
            );
        } else {
            assert_eq!(
                item.pointer("/nativeSource/link/destination/bookmarkName")
                    .and_then(Value::as_str),
                Some("outer")
            );
        }
    }
}

#[test]
fn reference_fields_preserve_condensed_separators_and_dedupe_overlay_copies() {
    let run = |label: &str| {
        format!(
            r#"<w:r><w:rPr><w:w w:val="1"/></w:rPr><w:t xml:space="preserve">{label}</w:t></w:r>"#
        )
    };
    let reference = |label: &str| {
        format!(
            r#"<w:fldSimple w:instr="REF target \h">{}</w:fldSimple>"#,
            run(label)
        )
    };
    let content = [
        reference("A"),
        run(", "),
        reference("B"),
        run(", "),
        reference("C"),
        run(", "),
        reference("D"),
    ]
    .concat();
    let body = format!(
        r#"<w:p><w:bookmarkStart w:id="1" w:name="target"/><w:bookmarkEnd w:id="1"/></w:p><w:p>{content}</w:p>"#
    );
    let (opened, fonts, mut pages) = fixture(&body);
    let index = Index::new(&opened.bookmarks, &pages);
    let shared = crate::office::pptx::output::text_json(
        &fonts,
        &crate::office::docx::extract::scene_of(&pages[0]),
        true,
        (0.0, 0.0),
        None,
    );
    assert!(
        shared["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item.get("nativePaintGroup").is_none())
    );
    let (text, _) = output(&index, &pages[0], 0, &fonts);
    let raw = |text: &Value| {
        text["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["unicode"].as_str().unwrap())
            .collect::<String>()
    };
    assert_eq!(raw(&text), "A, B, C, D");
    let commas = text["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["unicode"] == ", ")
        .collect::<Vec<_>>();
    assert_eq!(commas.len(), 3);
    let buckets = commas
        .iter()
        .map(|item| (item["coords"]["x"].as_f64().unwrap() * 2.0).round() as i64)
        .collect::<std::collections::HashSet<_>>();
    assert!(
        buckets.len() < commas.len(),
        "fixture must exercise coincident dedupe buckets"
    );
    assert!(
        text["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item.get("nativePaintGroup").is_none())
    );
    let copy = pages[0].items.clone();
    pages[0].items.extend(copy);
    let (overlaid, _) = output(&index, &pages[0], 0, &fonts);
    assert_eq!(raw(&overlaid), "A, B, C, D");
}
