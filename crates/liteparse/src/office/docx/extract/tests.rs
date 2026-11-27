use crate::office::docx::testutil::{docx, para};

#[test]
fn native_source_structure_keeps_link_occurrences_and_table_cells() {
    let body = r#"<w:p><w:r><w:t>A</w:t></w:r><w:hyperlink r:id="rhyperlink"><w:r><w:t>SAME</w:t></w:r></w:hyperlink><w:r><w:t> SAME </w:t></w:r><w:fldSimple w:instr='HYPERLINK "https://example.test/two"'><w:r><w:t>SAME</w:t></w:r></w:fldSimple></w:p><w:p><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText>HYPERLINK "https://example.test/three"</w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>SAME</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p><w:tbl><w:tblGrid><w:gridCol w:w="2000"/><w:gridCol w:w="2000"/></w:tblGrid><w:tr><w:tc><w:p><w:r><w:t>CELL</w:t></w:r></w:p><w:p><w:r><w:fldChar w:fldCharType="begin"/><w:instrText>HYPERLINK "https://example.test/cell"</w:instrText><w:fldChar w:fldCharType="separate"/><w:t>SECOND</w:t><w:fldChar w:fldCharType="end"/><w:t> TAIL</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>CELL</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#;
    let bytes = crate::office::docx::testutil::with_related_part(
        docx(body, None, None),
        "hyperlink",
        "https://example.test/one",
        "",
        true,
    );
    let dir = std::env::temp_dir().join(format!("docx-source-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let input = dir.join("source.docx");
    std::fs::write(&input, bytes).unwrap();
    let output = dir.join("pages.jsonl");
    super::run_read(
        r#"{"text":{"enabled":true},"structtree":{"enabled":true}}"#,
        input.to_str().unwrap(),
        dir.join("index.json").to_str().unwrap(),
        output.to_str().unwrap(),
    )
    .unwrap();
    let page: serde_json::Value =
        serde_json::from_str(std::fs::read_to_string(output).unwrap().trim()).unwrap();
    let raw: String = page["text"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|item| item["unicode"].as_str())
        .collect();
    assert!(raw.starts_with("ASAME SAME SAME"), "{raw:?}");
    let tree = &page["structtree"];
    assert_eq!(tree["nativeSource"], true);
    assert_eq!(tree["sourcePart"], "word/document.xml");
    let children = tree["children"].as_array().unwrap();
    let links: Vec<_> = children
        .iter()
        .filter(|child| child["type"] == "Link")
        .collect();
    assert_eq!(links.len(), 3);
    for (index, suffix) in ["one", "two", "three"].iter().enumerate() {
        assert_eq!(links[index]["annotations"][0]["subtype"], "Link");
        assert_eq!(
            links[index]["annotations"][0]["uri"],
            format!("https://example.test/{suffix}")
        );
        let ids = links[index]["markedContentIds"].as_array().unwrap();
        assert!(!ids.is_empty());
        for id in ids {
            assert_eq!(
                page["text"]["items"][id.as_u64().unwrap() as usize]["unicode"],
                "SAME"
            );
        }
    }
    let table = children
        .iter()
        .find(|child| child["type"] == "Table")
        .unwrap();
    assert_eq!(table["sourceTableId"], 0);
    assert_eq!(table["sourceTextOnly"], true);
    assert_eq!(
        table["children"][0]["sourceCellSpans"],
        serde_json::json!([1, 1])
    );
    let cells = table["children"][0]["children"].as_array().unwrap();
    assert_eq!(cells.len(), 2);
    assert_ne!(cells[0]["markedContentIds"], cells[1]["markedContentIds"]);
    let paragraphs = cells[0]["children"].as_array().unwrap();
    assert_eq!(paragraphs.len(), 2);
    assert_ne!(
        paragraphs[0]["sourceParagraphId"],
        paragraphs[1]["sourceParagraphId"]
    );
    assert_eq!(cells[0]["sourceCellCount"], 2);
    let link = &paragraphs[1]["children"][0];
    assert_eq!(link["annotations"][0]["uri"], "https://example.test/cell");
    let linked: String = link["markedContentIds"]
        .as_array()
        .unwrap()
        .iter()
        .map(|id| {
            page["text"]["items"][id.as_u64().unwrap() as usize]["unicode"]
                .as_str()
                .unwrap()
        })
        .collect();
    assert_eq!(linked, "SECOND");
    let paragraph_text: String = paragraphs[1]["markedContentIds"]
        .as_array()
        .unwrap()
        .iter()
        .map(|id| {
            page["text"]["items"][id.as_u64().unwrap() as usize]["unicode"]
                .as_str()
                .unwrap()
        })
        .collect();
    assert_eq!(paragraph_text, "SECOND TAIL");

    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn comment_metadata_is_opt_in_and_keeps_page_content_unchanged() {
    let dir = std::env::temp_dir().join(format!("docx-read-comments-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let input = dir.join("in.docx");
    let xml = format!(
        r#"<w:comments xmlns:w="{}"><w:comment w:id="3" w:author="Reviewer"><w:p><w:r><w:t>Review note</w:t></w:r></w:p></w:comment></w:comments>"#,
        crate::office::docx::testutil::W
    );
    std::fs::write(
        &input,
        crate::office::docx::testutil::with_comments(
            docx(&para("Body"), None, None),
            "comments.xml",
            &xml,
            false,
        ),
    )
    .unwrap();
    let mut pages = Vec::new();
    for enabled in [false, true] {
        let index = dir.join(format!("index-{enabled}.json"));
        let page_file = dir.join(format!("pages-{enabled}.jsonl"));
        super::run_read(
            &format!(r#"{{"text":{{"enabled":true}},"annotation":{{"enabled":{enabled}}}}}"#),
            input.to_str().unwrap(),
            index.to_str().unwrap(),
            page_file.to_str().unwrap(),
        )
        .unwrap();
        let idx: serde_json::Value =
            serde_json::from_slice(&std::fs::read(index).unwrap()).unwrap();
        assert_eq!(idx["totalPages"], 1);
        if enabled {
            assert_eq!(idx["document"]["comments"][0]["contents"], "Review note");
            assert_eq!(idx["document"]["comments"][0]["title"], "Reviewer");
        } else {
            assert!(idx.get("document").is_none());
        }
        let mut page: serde_json::Value =
            serde_json::from_str(std::fs::read_to_string(page_file).unwrap().trim()).unwrap();
        page.as_object_mut().unwrap().remove("annotations");
        pages.push(page);
    }
    assert_eq!(pages[0], pages[1]);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn writes_index_and_pages() {
    let dir = std::env::temp_dir().join(format!("docx-read-extract-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let input = dir.join("in.docx");
    let body = format!(
        r#"{}<w:p><w:r><w:br w:type="page"/></w:r></w:p>{}"#,
        para("a"),
        para("b")
    );
    std::fs::write(&input, docx(&body, None, None)).unwrap();
    let (index, pages) = (dir.join("index.json"), dir.join("pages.jsonl"));
    super::run_read(
        r#"{"text":{"enabled":true}}"#,
        input.to_str().unwrap(),
        index.to_str().unwrap(),
        pages.to_str().unwrap(),
    )
    .unwrap();
    let idx: serde_json::Value = serde_json::from_slice(&std::fs::read(&index).unwrap()).unwrap();
    assert_eq!(idx["totalPages"], 2);
    assert_eq!(idx["sourceFormat"], "docx");
    assert_eq!(std::fs::read_to_string(&pages).unwrap().lines().count(), 2);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn non_zip_input_is_a_document_error() {
    let dir = std::env::temp_dir().join(format!("docx-read-bad-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let input = dir.join("in.docx");
    std::fs::write(&input, b"<html><body>not a package</body></html>").unwrap();
    let (index, pages) = (dir.join("index.json"), dir.join("pages.jsonl"));
    super::run_read(
        "{}",
        input.to_str().unwrap(),
        index.to_str().unwrap(),
        pages.to_str().unwrap(),
    )
    .unwrap();
    let idx: serde_json::Value = serde_json::from_slice(&std::fs::read(&index).unwrap()).unwrap();
    assert_eq!(idx["error"], "OFFICE_FILE_IS_BROKEN");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn xml_depth_counts_element_nesting() {
    use super::xml_depth;
    assert_eq!(
        xml_depth(r#"<?xml version="1.0"?><a><b/><c><d x="1"></d></c></a>"#),
        3
    );
    assert_eq!(xml_depth("<a><!-- <b><c> --></a>"), 1);
    let deep = "<w:sdt>".repeat(5000) + &"</w:sdt>".repeat(5000);
    assert!(xml_depth(&deep) > super::MAX_XML_DEPTH);
}

fn missing_chart_drawing() -> &'static str {
    r#"<w:drawing><wp:inline xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"><wp:extent cx="2540000" cy="1524000"/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart r:id="rMissingChart"/></a:graphicData></a:graphic></wp:inline></w:drawing>"#
}

fn read_graphics_diagnostic(name: &str, bytes: &[u8]) -> (serde_json::Value, String) {
    let dir = std::env::temp_dir().join(format!(
        "docx-read-graphics-warning-{name}-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let input = dir.join("in.docx");
    let index = dir.join("index.json");
    let pages = dir.join("pages.jsonl");
    std::fs::write(&input, bytes).unwrap();
    super::run_read(
        r#"{"text":{"enabled":true}}"#,
        input.to_str().unwrap(),
        index.to_str().unwrap(),
        pages.to_str().unwrap(),
    )
    .unwrap();
    let result = (
        serde_json::from_slice(&std::fs::read(index).unwrap()).unwrap(),
        std::fs::read_to_string(pages).unwrap(),
    );
    std::fs::remove_dir_all(dir).ok();
    result
}

#[test]
fn missing_chart_warnings_reach_index_with_stable_source_context() {
    let drawing = missing_chart_drawing();
    let other = drawing.replace("rMissingChart", "rOtherChart");
    let body = format!(
        r#"<w:p><w:r><w:t>BODY</w:t>{drawing}{drawing}{other}</w:r></w:p><w:sectPr><w:headerReference w:type="default" r:id="rheader"/></w:sectPr>"#
    );
    let header = format!(
        r#"<w:hdr xmlns:w="{}" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><w:p><w:r>{drawing}</w:r></w:p></w:hdr>"#,
        crate::office::docx::testutil::W
    );
    let bytes = crate::office::docx::testutil::with_related_part(
        docx(&body, None, None),
        "header",
        "header1.xml",
        &header,
        false,
    );
    let alternate = crate::office::docx::testutil::with_related_part(
        docx(&body.replace("rOtherChart", "rDifferentChart"), None, None),
        "header",
        "header1.xml",
        &header,
        false,
    );
    let (alternate_index, alternate_pages) = read_graphics_diagnostic("alternate", &alternate);
    let mut prior = None;
    for _ in 0..2 {
        let (index, pages) = read_graphics_diagnostic("parts", &bytes);
        let warnings: Vec<_> = index["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|w| w["code"] == "CHART_MISSING")
            .cloned()
            .collect();
        assert_eq!(
            warnings,
            vec![
                serde_json::json!({"code":"CHART_MISSING","detail":"word/document.xml [r:id=rMissingChart]: Graphic"}),
                serde_json::json!({"code":"CHART_MISSING","detail":"word/document.xml [r:id=rOtherChart]: Graphic"}),
                serde_json::json!({"code":"CHART_MISSING","detail":"word/header1.xml [r:id=rMissingChart]: Graphic"})
            ]
        );
        assert!(warnings.iter().all(|w| w.get("page").is_none()));
        assert_eq!(index["totalPages"], 1);
        assert!(pages.contains("BODY"));
        assert_eq!(
            pages, alternate_pages,
            "diagnostic context does not alter page output"
        );
        assert_ne!(index["warnings"], alternate_index["warnings"]);
        if let Some(previous) = prior {
            assert_eq!(previous, pages);
        }
        prior = Some(pages);
    }
}

#[test]
fn nested_only_missing_chart_warning_follows_the_graphics_trigger() {
    let drawing = missing_chart_drawing();
    let body = format!(
        r#"<w:p><w:r><w:drawing><wp:inline xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"><wp:extent cx="3810000" cy="2540000"/><a:graphic><a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"><wps:wsp><wps:spPr><a:prstGeom prst="rect"/><a:noFill/></wps:spPr><wps:txbx><w:txbxContent><w:p><w:r><w:t>BOX</w:t>{drawing}</w:r></w:p></w:txbxContent></wps:txbx><wps:bodyPr/></wps:wsp></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>"#
    );
    let (index, pages) = read_graphics_diagnostic("nested", &docx(&body, None, None));
    let warnings: Vec<_> = index["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|w| w["code"] == "CHART_MISSING")
        .collect();
    assert_eq!(warnings.len(), 1);
    {
        assert_eq!(
            warnings[0]["detail"],
            "word/document.xml [r:id=rMissingChart]: Graphic"
        );
    }
    assert_eq!(index["totalPages"], 1);
    assert!(pages.contains("BOX"));
}

#[test]
fn offpage_text_diagnostic_matches_export_clip_without_disclosing_text() {
    use crate::office::pptx::scene::Item;
    let mut pages = crate::office::docx::testutil::pages(docx(&para("PRIVATE_MARKER"), None, None));
    let original = pages[0]
        .items
        .iter()
        .find_map(|i| match i {
            Item::Text(run) => Some(run.clone()),
            _ => None,
        })
        .unwrap();
    pages[0].items.clear();
    for (x, y) in [
        (612.0, 30.0),
        (-1000.0, 30.0),
        (30.0, 792.0),
        (30.0, -1000.0),
        (30.0, 1000.0),
    ] {
        let mut run = original.clone();
        run.transform = tiny_skia::Transform::from_translate(x, y);
        pages[0].items.push(Item::Text(run));
    }
    pages[0].info.w = 612.0;
    pages[0].info.h = 792.0;
    let warning = super::offpage_text_warning(&pages).unwrap();
    assert_eq!(warning.0, "TEXT_OUTSIDE_PAGE");
    // A baseline at the bottom still has ascent inside the physical page.
    assert!(warning.1.starts_with("4 nonempty"));
    assert!(!warning.1.contains("PRIVATE_MARKER"));
    let fonts = crate::office::pptx::fonts::FontSystem::new(&[], true, &Default::default());
    let exported = crate::office::pptx::output::text_json(
        &fonts,
        &super::scene_of(&pages[0]),
        false,
        (0.0, 0.0),
        Some((612.0, 792.0)),
    );
    assert_eq!(exported["items"].as_array().unwrap().len(), 1);
    assert_eq!(pages[0].items.len(), 5);
}

#[test]
fn offpage_text_diagnostic_excludes_generated_furniture_hidden_and_partial_runs() {
    use crate::office::pptx::scene::Item;
    let mut pages = crate::office::docx::testutil::pages(docx(&para("VISIBLE"), None, None));
    let original = pages[0]
        .items
        .iter()
        .find_map(|i| match i {
            Item::Text(run) => Some(run.clone()),
            _ => None,
        })
        .unwrap();
    pages[0].items.clear();
    for kind in 0..5 {
        let mut run = original.clone();
        run.transform = tiny_skia::Transform::from_translate(10000.0, 30.0);
        match kind {
            0 => {
                run.paragraph = 0;
                run.source = None;
            }
            1 => {
                run.paragraph = crate::office::docx::text::FOOTER_KEY;
                run.source = None;
            }
            2 => run.color.a = 0.0,
            3 => run.text = " \u{a0}\t".into(),
            _ => run.transform = tiny_skia::Transform::from_translate(-1.0, 30.0),
        }
        pages[0].items.push(Item::Text(run));
    }
    assert!(super::offpage_text_warning(&pages).is_none());
    let hidden = r#"<w:p><w:r><w:rPr><w:vanish/></w:rPr><w:t>HIDDEN</w:t></w:r></w:p>"#;
    let pages = crate::office::docx::testutil::pages(docx(hidden, None, None));
    assert!(
        pages
            .iter()
            .flat_map(|p| &p.items)
            .all(|i| !matches!(i, Item::Text(r) if r.text.contains("HIDDEN")))
    );
}

#[test]
fn offpage_text_diagnostic_reaches_index_for_source_textbox() {
    let dir = std::env::temp_dir().join(format!("docx-offpage-index-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let body = format!(
        r#"{}<w:p><w:r><w:pict xmlns:v="urn:schemas-microsoft-com:vml"><v:rect style="position:absolute;margin-left:1000pt;margin-top:0;width:100pt;height:50pt"><v:textbox><w:txbxContent><w:p><w:r><w:t>PRIVATE_OFFPAGE</w:t></w:r></w:p></w:txbxContent></v:textbox></v:rect></w:pict></w:r></w:p>"#,
        para("VISIBLE_BODY")
    );
    let input = dir.join("input.docx");
    std::fs::write(&input, docx(&body, None, None)).unwrap();
    let index = dir.join("index.json");
    let output = dir.join("pages.jsonl");
    super::run_read(
        r#"{"text":{"enabled":true}}"#,
        input.to_str().unwrap(),
        index.to_str().unwrap(),
        output.to_str().unwrap(),
    )
    .unwrap();
    let idx: serde_json::Value = serde_json::from_slice(&std::fs::read(index).unwrap()).unwrap();
    let warnings = idx["warnings"].as_array().unwrap();
    assert_eq!(
        warnings
            .iter()
            .filter(|w| w["code"] == "TEXT_OUTSIDE_PAGE")
            .count(),
        1
    );
    assert!(
        !serde_json::to_string(warnings)
            .unwrap()
            .contains("PRIVATE_OFFPAGE")
    );
    let output = std::fs::read_to_string(output).unwrap();
    assert!(output.contains("VISIBLE_BODY"));
    assert!(!output.contains("PRIVATE_OFFPAGE"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn offpage_text_diagnostic_keeps_rotated_outline_only_text() {
    use crate::office::pptx::scene::Item;
    let mut pages = crate::office::docx::testutil::pages(docx(&para("OUTLINE"), None, None));
    let mut run = pages[0]
        .items
        .iter()
        .find_map(|i| match i {
            Item::Text(run) => Some(run.clone()),
            _ => None,
        })
        .unwrap();
    let color = run.color;
    run.color.a = 0.0;
    run.outline = Some((color, 1.0));
    run.transform = tiny_skia::Transform::from_row(0.0, 1.0, -1.0, 0.0, 1000.0, 30.0);
    pages[0].items = vec![Item::Text(run)];
    assert!(
        super::offpage_text_warning(&pages)
            .unwrap()
            .1
            .starts_with("1 nonempty")
    );
}

#[test]
fn grow_autofit_setting_uses_explicit_boolean_value() {
    for (element, expected) in [
        ("", false),
        ("<w:growAutofit/>", true),
        (r#"<w:growAutofit w:val="0"/>"#, false),
        (r#"<w:growAutofit w:val="true"/>"#, true),
    ] {
        let xml = format!(
            r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:compat>{element}</w:compat></w:settings>"#
        );
        let node = crate::office::pptx::xml::parse_word(&xml).unwrap();
        assert_eq!(super::parse_settings(Some(&node)).grow_autofit, expected);
    }
}

#[test]
fn source_boundary_preserves_visually_joined_trailing_whitespace() {
    let body = "<w:p><w:hyperlink r:id=\"rhyperlink\"><w:r><w:t>LABEL</w:t></w:r></w:hyperlink><w:r><w:t xml:space=\"preserve\"> \u{a0} </w:t></w:r></w:p>";
    let body = format!(
        "{body}{}{}{}",
        r#"<w:p><w:hyperlink r:id="rhyperlink"><w:r><w:t>NBSP</w:t></w:r></w:hyperlink><w:r><w:t> </w:t></w:r></w:p>"#,
        r#"<w:p><w:r><w:rPr><w:u w:val="single"/></w:rPr><w:t xml:space="preserve"> </w:t></w:r><w:hyperlink r:id="rhyperlink"><w:r><w:rPr><w:u w:val="single"/></w:rPr><w:t>LEADING</w:t></w:r></w:hyperlink></w:p>"#,
        r#"<w:p><w:hyperlink r:id="rhyperlink"><w:r><w:rPr><w:u w:val="single"/></w:rPr><w:t>EDGE</w:t></w:r><w:r><w:t xml:space="preserve"> </w:t></w:r></w:hyperlink><w:r><w:t xml:space="preserve"> </w:t></w:r></w:p>"#
    );
    let bytes = crate::office::docx::testutil::with_related_part(
        docx(&body, None, None),
        "hyperlink",
        "https://example.test/trailing",
        "",
        true,
    );
    let dir = std::env::temp_dir().join(format!("docx-source-space-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let input = dir.join("source.docx");
    std::fs::write(&input, bytes).unwrap();
    let output = dir.join("pages.jsonl");
    super::run_read(
        r#"{"text":{"enabled":true},"structtree":{"enabled":true}}"#,
        input.to_str().unwrap(),
        dir.join("index.json").to_str().unwrap(),
        output.to_str().unwrap(),
    )
    .unwrap();
    let page: serde_json::Value =
        serde_json::from_str(std::fs::read_to_string(output).unwrap().trim()).unwrap();
    let items = page["text"]["items"].as_array().unwrap();
    let raw: String = items
        .iter()
        .filter_map(|item| item["unicode"].as_str())
        .collect();
    assert_eq!(raw, "LABEL \u{a0} NBSP\u{a0} LEADINGEDGE");
    let link = &page["structtree"]["children"][0];
    let linked: String = link["markedContentIds"]
        .as_array()
        .unwrap()
        .iter()
        .map(|id| {
            items[id.as_u64().unwrap() as usize]["unicode"]
                .as_str()
                .unwrap()
        })
        .collect();
    assert_eq!(linked, "LABEL");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn vertical_merge_continuation_export_uses_the_accepted_physical_row() {
    let paragraph = |text: &str| {
        format!(
            r#"<w:p><w:pPr><w:spacing w:after="0" w:line="240" w:lineRule="exact"/></w:pPr><w:r><w:t>{text}</w:t></w:r></w:p>"#
        )
    };
    let left: String = (0..36)
        .map(|i| paragraph(&format!("MERGE{i:02}")))
        .collect();
    let right: String = (0..36)
        .map(|i| paragraph(&format!("RIGHT{i:02}")))
        .collect();
    let body = format!(
        r#"{}<w:tbl><w:tblPr><w:tblW w:type="dxa" w:w="4800"/><w:tblLayout w:type="fixed"/></w:tblPr><w:tblGrid><w:gridCol w:w="2400"/><w:gridCol w:w="2400"/></w:tblGrid><w:tr><w:tc><w:tcPr><w:vMerge w:val="restart"/></w:tcPr>{left}</w:tc><w:tc>{}</w:tc></w:tr><w:tr><w:tc><w:tcPr><w:vMerge/></w:tcPr><w:p/></w:tc><w:tc>{right}</w:tc></w:tr></w:tbl><w:sectPr><w:pgSz w:w="6000" w:h="6000"/><w:pgMar w:top="600" w:bottom="600" w:left="600" w:right="600"/></w:sectPr>"#,
        paragraph("START"),
        paragraph("TOP")
    );
    let dir = std::env::temp_dir().join(format!("docx-merge-fragment-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let input = dir.join("source.docx");
    std::fs::write(&input, docx(&body, None, None)).unwrap();
    let output = dir.join("pages.jsonl");
    super::run_read(
        r#"{"text":{"enabled":true},"structtree":{"enabled":true}}"#,
        input.to_str().unwrap(),
        dir.join("index.json").to_str().unwrap(),
        output.to_str().unwrap(),
    )
    .unwrap();
    let pages: Vec<serde_json::Value> = std::fs::read_to_string(output)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(pages.len(), 2);
    let table = pages[1]["structtree"]["children"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["type"] == "Table")
        .unwrap();
    let rows = table["children"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["sourceRowIndex"], 1);
    let merged = &rows[0]["children"][0];
    assert_eq!(merged["sourceOwnerRowIndex"], 0);
    let items = pages[1]["text"]["items"].as_array().unwrap();
    let visible: String = merged["markedContentIds"]
        .as_array()
        .unwrap()
        .iter()
        .map(|id| {
            items[id.as_u64().unwrap() as usize]["unicode"]
                .as_str()
                .unwrap()
        })
        .collect();
    assert!(visible.ends_with("MERGE35"));
    assert!(!visible.contains("MERGE00"));
    let full: String = pages
        .iter()
        .flat_map(|page| page["text"]["items"].as_array().unwrap())
        .filter_map(|item| item["unicode"].as_str())
        .collect();
    for i in 0..36 {
        assert_eq!(full.matches(&format!("MERGE{i:02}")).count(), 1);
        assert_eq!(full.matches(&format!("RIGHT{i:02}")).count(), 1);
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn table_source_grid_offsets_survive_export_and_merge_transitions() {
    let cell = |properties: &str, text: &str| {
        format!("<w:tc><w:tcPr>{properties}</w:tcPr>{}</w:tc>", para(text))
    };
    let body = format!(
        r#"<w:tbl><w:tblPr><w:tblLayout w:type="fixed"/></w:tblPr><w:tblGrid>{}</w:tblGrid><w:tr><w:trPr><w:gridBefore w:val="1"/><w:gridAfter w:val="1"/></w:trPr>{}{}{}</w:tr><w:tr>{}{}{}</w:tr></w:tbl>"#,
        r#"<w:gridCol w:w="1000"/>"#.repeat(5),
        cell("", "A"),
        cell("", "B"),
        cell(r#"<w:vMerge w:val="restart"/>"#, "MERGED"),
        cell(r#"<w:gridSpan w:val="2"/>"#, "WIDE"),
        cell("", "LAST"),
        cell("<w:vMerge/>", "")
    );
    let dir = std::env::temp_dir().join(format!("docx-grid-offsets-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let input = dir.join("source.docx");
    std::fs::write(&input, docx(&body, None, None)).unwrap();
    let output = dir.join("pages.jsonl");
    super::run_read(
        r#"{"text":{"enabled":true},"structtree":{"enabled":true}}"#,
        input.to_str().unwrap(),
        dir.join("index.json").to_str().unwrap(),
        output.to_str().unwrap(),
    )
    .unwrap();
    let page: serde_json::Value =
        serde_json::from_str(std::fs::read_to_string(output).unwrap().trim()).unwrap();
    let table = &page["structtree"]["children"][0];
    let rows = table["children"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["sourceGridBefore"], 1);
    assert_eq!(rows[0]["sourceGridAfter"], 1);
    assert!(rows[1].get("sourceGridBefore").is_none());
    assert_eq!(rows[0]["sourceCellSpans"], serde_json::json!([1, 1, 1]));
    assert_eq!(rows[1]["sourceCellSpans"], serde_json::json!([2, 1, 1]));
    assert_eq!(rows[0]["sourceVerticalMerges"][0]["gridStart"], 3);
    let raw: String = page["text"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|item| item["unicode"].as_str())
        .collect();
    assert_eq!(raw, "ABMERGEDWIDELAST");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn large_grid_spans_preserve_equivalent_geometry_and_source_intervals() {
    let dir = std::env::temp_dir().join(format!("docx-large-spans-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut pages = Vec::new();
    for (name, grid, span) in [
        ("large", r#"<w:gridCol w:w="80"/>"#.repeat(76), 70),
        (
            "equivalent",
            r#"<w:gridCol w:w="5600"/><w:gridCol w:w="480"/>"#.to_string(),
            1,
        ),
        ("augmented", String::new(), 70),
    ] {
        let second_span = if span == 70 { 6 } else { 1 };
        let body = format!(
            r#"<w:tbl><w:tblPr><w:tblLayout w:type="fixed"/></w:tblPr><w:tblGrid>{grid}</w:tblGrid><w:tr><w:tc><w:tcPr><w:gridSpan w:val="{span}"/></w:tcPr>{}</w:tc><w:tc><w:tcPr><w:gridSpan w:val="{second_span}"/></w:tcPr>{}</w:tc></w:tr></w:tbl>"#,
            para("LEFTMARK"),
            para("RIGHTMARK")
        );
        let input = dir.join(format!("{name}.docx"));
        let output = dir.join(format!("{name}.jsonl"));
        std::fs::write(&input, docx(&body, None, None)).unwrap();
        super::run_read(
            r#"{"text":{"enabled":true},"structtree":{"enabled":true}}"#,
            input.to_str().unwrap(),
            dir.join(format!("{name}-index.json")).to_str().unwrap(),
            output.to_str().unwrap(),
        )
        .unwrap();
        let page: serde_json::Value =
            serde_json::from_str(std::fs::read_to_string(output).unwrap().trim()).unwrap();
        let raw: String = page["text"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|item| item["unicode"].as_str())
            .collect();
        assert_eq!(raw, "LEFTMARKRIGHTMARK");
        pages.push(page);
    }
    let right = |page: &serde_json::Value| page["text"]["items"][1]["coords"].clone();
    assert_eq!(right(&pages[0]), right(&pages[1]));
    assert_eq!(
        pages[0]["structtree"]["children"][0]["children"][0]["sourceCellSpans"],
        serde_json::json!([70, 6])
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn oversized_logical_grids_report_resource_error_without_partial_pages() {
    use crate::office::docx::testutil::{W, with_related_part};
    let table = |row: &str, span: &str| {
        format!(
            r#"<w:tbl><w:tr><w:trPr>{row}</w:trPr><w:tc><w:tcPr><w:gridSpan w:val="{span}"/></w:tcPr>{}</w:tc></w:tr></w:tbl>"#,
            para("PrivatePayload")
        )
    };
    let huge = table("", "184467440737095516160");
    let mut cases = vec![
        docx(&(para("Before") + &huge), None, None),
        docx(
            &table(r#"<w:gridBefore w:val="184467440737095516160"/>"#, "1"),
            None,
            None,
        ),
        docx(&table(r#"<w:gridAfter w:val="65536"/>"#, "1"), None, None),
        docx(
            &format!("<w:tbl><w:tr><w:tc>{huge}</w:tc></w:tr></w:tbl>"),
            None,
            None,
        ),
    ];
    let notes = format!(
        r#"<w:footnotes xmlns:w="{W}"><w:footnote w:id="0" w:type="continuationSeparator">{huge}</w:footnote></w:footnotes>"#
    );
    let settings = format!(
        r#"<w:settings xmlns:w="{W}"><w:footnotePr><w:footnote w:id="0"/></w:footnotePr></w:settings>"#
    );
    let bytes = with_related_part(
        docx(&para("Body"), None, None),
        "footnotes",
        "footnotes.xml",
        &notes,
        false,
    );
    cases.push(with_related_part(
        bytes,
        "settings",
        "settings.xml",
        &settings,
        false,
    ));
    let dir = std::env::temp_dir().join(format!("docx-grid-resource-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (case, bytes) in cases.into_iter().enumerate() {
        let input = dir.join(format!("{case}.docx"));
        let index = dir.join(format!("{case}-index.json"));
        let pages = dir.join(format!("{case}-pages.jsonl"));
        std::fs::write(&input, bytes).unwrap();
        super::run_read(
            "{}",
            input.to_str().unwrap(),
            index.to_str().unwrap(),
            pages.to_str().unwrap(),
        )
        .unwrap();
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(index).unwrap()).unwrap();
        assert_eq!(value["error"], "OFFICE_FILE_IS_BROKEN");
        assert!(
            value["errorDetail"]
                .as_str()
                .unwrap()
                .contains("table logical grid")
        );
        assert!(!value.to_string().contains("PrivatePayload"));
        assert_eq!(value["totalPages"], 0);
        assert!(std::fs::read(pages).unwrap().is_empty());
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn escaped_field_targets_keep_simple_and_complex_source_occurrences() {
    let instructions = [
        (
            r#"HYPERLINK "\\\\server\\share\\file.txt""#,
            r"\\server\share\file.txt",
        ),
        (r#"HYPERLINK "E:\\ReadMe.txt""#, r"E:\ReadMe.txt"),
        (
            r#"HYPERLINK "https://example.test/a\"b""#,
            "https://example.test/a\"b",
        ),
    ];
    let mut body = String::new();
    let mut expected = Vec::new();
    for (instruction, uri) in instructions {
        let xml = instruction
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('"', "&quot;");
        body.push_str(&format!(r#"<w:p><w:fldSimple w:instr="{xml}"><w:r><w:t>SAME</w:t></w:r></w:fldSimple><w:r><w:t> PLAIN </w:t><w:fldChar w:fldCharType="begin"/><w:instrText>{xml}</w:instrText><w:fldChar w:fldCharType="separate"/><w:t>SAME</w:t><w:fldChar w:fldCharType="end"/></w:r></w:p>"#));
        expected.extend([uri, uri]);
    }
    let dir = std::env::temp_dir().join(format!("docx-escaped-field-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let input = dir.join("source.docx");
    let output = dir.join("pages.jsonl");
    std::fs::write(&input, docx(&body, None, None)).unwrap();
    super::run_read(
        r#"{"text":{"enabled":true},"structtree":{"enabled":true}}"#,
        input.to_str().unwrap(),
        dir.join("index.json").to_str().unwrap(),
        output.to_str().unwrap(),
    )
    .unwrap();
    let page: serde_json::Value =
        serde_json::from_str(std::fs::read_to_string(output).unwrap().trim()).unwrap();
    let links: Vec<_> = page["structtree"]["children"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|child| child["type"] == "Link")
        .collect();
    assert_eq!(links.len(), expected.len());
    let mut ids = std::collections::HashSet::new();
    for (link, uri) in links.iter().zip(expected) {
        assert_eq!(link["annotations"][0]["uri"], uri);
        assert!(ids.insert(link["sourceLinkId"].as_str().unwrap()));
        let text: String = link["markedContentIds"]
            .as_array()
            .unwrap()
            .iter()
            .map(|id| {
                page["text"]["items"][id.as_u64().unwrap() as usize]["unicode"]
                    .as_str()
                    .unwrap()
            })
            .collect();
        assert_eq!(text, "SAME");
    }
    let raw: String = page["text"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["unicode"].as_str().unwrap())
        .collect();
    assert_eq!(raw, "SAME PLAIN SAME".repeat(3));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn condensed_source_fragments_keep_repeated_punctuation_within_one_painted_run() {
    use serde_json::json;
    let fragment = |x: f64, group: u64| {
        json!({
            "unicode": ", ", "coords": {"x": x, "y": 20.0, "w": 0.061, "h": 11.0},
            "nativePaintGroup": group
        })
    };
    let input = json!({"items": [
        fragment(158.301, 1), fragment(158.576, 1),
        fragment(158.301, 2), fragment(158.576, 2),
        {"unicode":" ","coords":{"x":158.301,"y":20.0},"nativePaintGroup":1},
        {"unicode":" ","coords":{"x":158.301,"y":20.0},"nativePaintGroup":1}
    ]});
    let output = super::dedupe_text(input);
    let items = output["items"].as_array().unwrap();
    assert_eq!(
        items
            .iter()
            .map(|item| item["unicode"].as_str().unwrap())
            .collect::<String>(),
        ", ,   "
    );
    assert_eq!(items.len(), 4);
    assert!(
        items
            .iter()
            .all(|item| item.get("nativePaintGroup").is_none())
    );
}

#[test]
fn offpage_text_diagnostic_includes_source_owned_headers_and_footers() {
    use crate::office::pptx::scene::Item;
    let footer = format!(
        r#"<w:ftr xmlns:w="{}"><w:tbl><w:tr><w:tc>{}</w:tc></w:tr></w:tbl></w:ftr>"#,
        crate::office::docx::testutil::W,
        para("SOURCE_FURNITURE")
    );
    let body = format!(
        r#"{}<w:sectPr><w:footerReference w:type="default" r:id="rfooter"/></w:sectPr>"#,
        para("BODY")
    );
    let mut pages =
        crate::office::docx::testutil::pages(crate::office::docx::testutil::with_related_part(
            docx(&body, None, None),
            "footer",
            "footer.xml",
            &footer,
            false,
        ));
    let original = pages[0]
        .items
        .iter()
        .find_map(|item| match item {
            Item::Text(run) if run.text == "SOURCE_FURNITURE" => Some(run.clone()),
            _ => None,
        })
        .unwrap();
    assert!(original.source.is_some());
    pages[0].items.clear();
    for key in [0, crate::office::docx::text::FOOTER_KEY] {
        let mut run = original.clone();
        run.paragraph = key;
        run.transform = tiny_skia::Transform::from_translate(10000.0, 30.0);
        pages[0].items.push(Item::Text(run));
    }
    let (_, detail) = super::offpage_text_warning(&pages).unwrap();
    assert!(detail.starts_with("2 nonempty source text runs"));
    assert!(!detail.contains("SOURCE_FURNITURE"));
}

/// Runs `run_read` on `bytes`; returns the page text, or the index error code and detail.
#[expect(
    clippy::unwrap_in_result,
    reason = "a test helper: setup failures should panic"
)]
fn read_input(bytes: Vec<u8>, name: &str) -> Result<String, (String, String)> {
    let dir = std::env::temp_dir().join(format!("docx-input-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (input, index, output) = (
        dir.join(name),
        dir.join("index.json"),
        dir.join("pages.jsonl"),
    );
    std::fs::write(&input, bytes).unwrap();
    super::run_read(
        r#"{"text":{"enabled":true}}"#,
        input.to_str().unwrap(),
        index.to_str().unwrap(),
        output.to_str().unwrap(),
    )
    .unwrap();
    let index: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(index).unwrap()).unwrap();
    if !index["error"].is_null() {
        return Err((index["error"].to_string(), index["errorDetail"].to_string()));
    }
    let pages = std::fs::read_to_string(output).unwrap();
    let page: serde_json::Value = serde_json::from_str(pages.lines().next().unwrap()).unwrap();
    Ok(page["text"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|item| item["unicode"].as_str())
        .collect())
}

#[test]
fn binary_word_input_is_converted_and_rendered() {
    let text = read_input(crate::office::doc::fixtures::fixture(), "input.doc").unwrap();
    assert!(text.contains("Body"), "{text:?}");
}

#[test]
fn encrypted_binary_word_is_protected_and_other_compound_files_stay_broken() {
    let mut encrypted = crate::office::doc::fixtures::fixture();
    crate::office::doc::fixtures::put16(&mut encrypted, 1536 + 10, 0x300);
    let (code, _) = read_input(encrypted, "encrypted.doc").unwrap_err();
    assert_eq!(code, r#""PDF_IS_PROTECTED""#);
    // Renaming the stream leaves a compound file that is not a Word document.
    let mut other = crate::office::doc::fixtures::fixture();
    other[1024 + 128..1024 + 130].copy_from_slice(&u16::from(b'X').to_le_bytes());
    let (code, detail) = read_input(other, "other.doc").unwrap_err();
    assert_eq!(code, r#""OFFICE_FILE_IS_BROKEN""#);
    assert!(detail.contains("not an OOXML package"), "{detail}");
}

/// A placeable WMF (200 x 100 units at 100 per inch) that draws "WMFTEXT" in Arial.
fn text_wmf() -> Vec<u8> {
    let rec = |f: u16, params: &[u8]| {
        let mut v = ((3 + params.len() / 2) as u32).to_le_bytes().to_vec();
        v.extend(f.to_le_bytes());
        v.extend(params);
        v
    };
    let mut v = 0x9AC6_CDD7u32.to_le_bytes().to_vec();
    v.extend(0u16.to_le_bytes());
    for x in [0i16, 0, 200, 100] {
        v.extend(x.to_le_bytes());
    }
    v.extend(100u16.to_le_bytes());
    v.extend([0; 6]);
    for w in [1u16, 9, 0x300, 0, 0, 4, 0, 0, 0] {
        v.extend(w.to_le_bytes());
    }
    // CREATEFONTINDIRECT: height -20, then zeros, weight 400 and the face name.
    let mut font = (-20i16).to_le_bytes().to_vec();
    font.extend([0; 6]);
    font.extend(400i16.to_le_bytes());
    font.extend([0; 8]);
    let mut face = b"Arial".to_vec();
    face.resize(32, 0);
    font.extend(face);
    v.extend(rec(0x02FB, &font));
    v.extend(rec(0x012D, &0u16.to_le_bytes()));
    // TEXTOUT: length, the string (padded to even), y, x.
    let mut text = 7i16.to_le_bytes().to_vec();
    text.extend(b"WMFTEXT\0");
    text.extend(40i16.to_le_bytes());
    text.extend(10i16.to_le_bytes());
    v.extend(rec(0x0521, &text));
    v.extend(rec(0, &[]));
    v
}

/// An EMF (100 x 50 device pixels) that draws "EMFTEXT" in Arial.
fn text_emf() -> Vec<u8> {
    let rec = |t: u32, params: &[i32]| {
        let mut v = t.to_le_bytes().to_vec();
        v.extend((8 + params.len() as u32 * 4).to_le_bytes());
        for p in params {
            v.extend(p.to_le_bytes());
        }
        v
    };
    // EXTCREATEFONTINDIRECTW: ih 1, height -10, face "Arial".
    let mut font = rec(82, &[1, -10, 0, 0, 0, 400, 0, 0, 0]);
    font.truncate(8 + 4 * 6 + 4 + 4);
    font.extend([0; 4]);
    let mut face = [0u8; 64];
    for (i, c) in "Arial".encode_utf16().enumerate() {
        face[i * 2..i * 2 + 2].copy_from_slice(&c.to_le_bytes());
    }
    font.extend(face);
    let size = font.len() as u32;
    font[4..8].copy_from_slice(&size.to_le_bytes());
    // EXTTEXTOUTW "EMFTEXT" at (10, 20), baseline-aligned.
    let mut text = rec(84, &[0, 0, 0, 0, 1, 0, 0, 10, 20, 7, 76, 0, 0, 0, 0, 0, 0]);
    text.extend("EMFTEXT\0".encode_utf16().flat_map(u16::to_le_bytes));
    let size = text.len() as u32;
    text[4..8].copy_from_slice(&size.to_le_bytes());
    let records = [font, rec(37, &[1]), rec(22, &[24]), text];
    let mut h = Vec::new();
    for x in [1i32, 88, 0, 0, 99, 49, 0, 0, 2500, 1250] {
        h.extend(x.to_le_bytes());
    }
    h.extend(b" EMF");
    for x in [0x10000, 0, records.len() as i32 + 2] {
        h.extend(x.to_le_bytes());
    }
    h.extend(8u16.to_le_bytes());
    h.extend(0u16.to_le_bytes());
    for x in [0i32, 0, 0, 1000, 1000, 250, 250] {
        h.extend(x.to_le_bytes());
    }
    for r in records {
        h.extend(r);
    }
    h.extend(rec(14, &[0, 0, 16]));
    let n = h.len() as i32;
    h[48..52].copy_from_slice(&n.to_le_bytes());
    h
}

/// A document holding `pictures` (part name, bytes) inline after the word "Body".
fn with_pictures(pictures: &[(&str, Vec<u8>)]) -> Vec<u8> {
    use std::io::{Read, Write};
    let drawing = |rid: &str| {
        format!(
            r#"<w:r><w:drawing><wp:inline xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture"><wp:extent cx="2540000" cy="1270000"/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic><pic:blipFill><a:blip r:embed="{rid}"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="2540000" cy="1270000"/></a:xfrm></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r>"#
        )
    };
    let runs: String = (0..pictures.len())
        .map(|i| drawing(&format!("rPic{i}")))
        .collect();
    let body = format!("<w:p><w:r><w:t>Body</w:t></w:r>{runs}</w:p>");
    let mut input = zip::ZipArchive::new(std::io::Cursor::new(docx(&body, None, None))).unwrap();
    let mut output = std::io::Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut output);
        for i in 0..input.len() {
            let mut f = input.by_index(i).unwrap();
            let name = f.name().unwrap().to_string();
            let mut data = Vec::new();
            f.read_to_end(&mut data).unwrap();
            if name == "word/_rels/document.xml.rels" {
                let rels: String = pictures
                    .iter()
                    .enumerate()
                    .map(|(i, (n, _))| format!(r#"<Relationship Id="rPic{i}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/{n}"/>"#))
                    .collect();
                data = String::from_utf8(data)
                    .unwrap()
                    .replace("</Relationships>", &format!("{rels}</Relationships>"))
                    .into_bytes();
            }
            z.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            z.write_all(&data).unwrap();
        }
        for (n, bytes) in pictures {
            z.start_file(
                format!("word/media/{n}"),
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
            z.write_all(bytes).unwrap();
        }
        z.finish().unwrap();
    }
    output.into_inner()
}

#[test]
fn emf_picture_text_is_extracted_and_wmf_text_is_not() {
    let fonts = crate::office::pptx::fonts::FontSystem::new(&[], true, &Default::default());
    if fonts.resolve("Arial", false, false).is_none()
        && fonts.resolve("Carlito", false, false).is_none()
    {
        return; // no fonts on this machine
    }
    // Word's PDF keeps the text of EMF diagrams and draws WMF text as outlines.
    let bytes = with_pictures(&[("d.emf", text_emf()), ("d.wmf", text_wmf())]);
    let text = read_input(bytes, "pictures.docx").unwrap();
    assert!(
        text.contains("Body") && text.contains("EMFTEXT"),
        "{text:?}"
    );
    assert!(!text.contains("WMFTEXT"), "{text:?}");
}
