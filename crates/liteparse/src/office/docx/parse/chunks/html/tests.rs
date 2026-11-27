use super::*;

fn element(root: &dom::Handle, tag: &str) -> Option<dom::Handle> {
    if root.tag() == tag {
        return Some(root.clone());
    }
    root.children
        .borrow()
        .iter()
        .find_map(|node| element(node, tag))
}

#[test]
fn html_tree_recovers_tables_entities_and_preserves_foreign_namespaces() {
    let (root, _) = dom::parse(
        "<table><tr><td>A&amp;B<td>C</table><svg><text>vector</text></svg>",
        100,
    )
    .unwrap();
    let table = element(&root, "table").unwrap();
    assert_eq!(table.children.borrow()[0].tag(), "tbody");
    assert_eq!(
        *element(&root, "td").unwrap().children.borrow()[0]
            .text
            .borrow(),
        "A&B"
    );
    assert_eq!(
        &*element(&root, "svg").unwrap().name.ns,
        "http://www.w3.org/2000/svg"
    );
}

#[test]
fn html_tree_limits_depth_nodes_and_final_token() {
    assert!(dom::parse(&"<div>".repeat(300), 1000).is_err());
    assert!(dom::parse(&"<p>x</p>".repeat(100), 20).is_err());
    assert!(dom::parse("<p>final unterminated text", 3).is_err());
    let long = format!("<p title='{}'>{}", "a".repeat(4096), "b".repeat(4096));
    let (root, count) = dom::parse(&long, 20).unwrap();
    assert!(count < 20);
    assert_eq!(
        element(&root, "p").unwrap().attr("title").unwrap().len(),
        4096
    );
}

#[test]
fn html_encoding_obeys_bom_transport_and_meta_without_lossy_decode() {
    assert_eq!(
        decode_html(b"<meta charset=utf-8>caf\xc3\xa9", None).unwrap(),
        "<meta charset=utf-8>caf\u{e9}"
    );
    assert!(decode_html(b"\xff", Some("utf-8")).is_err());
    assert!(decode_html(b"test", Some("unknown-encoding")).is_err());
    assert_eq!(
        decode_html(b"\xef\xbb\xbfplain", Some("windows-1252")).unwrap(),
        "plain"
    );
}

#[test]
fn css_cascade_preserves_important_specificity_and_inheritance() {
    let (root, _) = dom::parse(
        "<div id=x><p class=note style='color:green;font-weight:normal'>A</p></div>",
        100,
    )
    .unwrap();
    let mut cascade = style::Cascade::new(
        "p { color:red!important } #x .note { color:blue!important } .note { font-weight:bold }",
    );
    let parent = style::Computed(std::collections::BTreeMap::from([(
        "font-size".into(),
        "18pt".into(),
    )]));
    let result = cascade
        .compute(&element(&root, "p").unwrap(), &parent)
        .unwrap();
    assert_eq!(result.get("color"), Some("blue"));
    assert_eq!(result.get("font-weight"), Some("normal"));
    assert_eq!(result.get("font-size"), Some("18pt"));
}

#[test]
fn html_import_preserves_inline_spaces_and_structured_table_cells() {
    let bytes=super::super::tests::package("<w:altChunk r:id=\"chunk\"/>","text/html",b"<p>A <b>B</b><i> C</i></p><table><tr><td rowspan=2>MERGED</td><td>RIGHT1</td></tr><tr><td>RIGHT2</td></tr></table>",false);
    let opened = crate::office::docx::extract::open(bytes, &Default::default()).unwrap();
    assert_eq!(
        super::super::tests::text(&opened.sections[0].blocks),
        "A B C"
    );
    let Block::Table(table) = &opened.sections[0].blocks[1] else {
        panic!("missing table")
    };
    assert_eq!(table.rows.len(), 2);
    assert_eq!(table.rows[0].cells.len(), 2);
    assert_eq!(table.rows[1].cells.len(), 2);
}

#[test]
fn html_computed_relative_fonts_and_rtl_alignment_are_resolved() {
    let (root, _) = dom::parse(
        "<div style='font-size:24pt'><p style='font-size:50%'>X</p></div>",
        100,
    )
    .unwrap();
    let mut cascade = style::Cascade::new("");
    let parent = cascade
        .compute(&element(&root, "div").unwrap(), &style::Computed::default())
        .unwrap();
    let child = cascade
        .compute(&element(&root, "p").unwrap(), &parent)
        .unwrap();
    assert_eq!(child.get("font-size"), Some("12pt"));
    let mut rtl = child;
    rtl.0.insert("direction".into(), "rtl".into());
    rtl.0.insert("text-align".into(), "start".into());
    let p = word::paragraph(&rtl, Vec::new());
    assert_eq!(
        p.path(&["w:pPr", "w:jc"]).unwrap().attr("val"),
        Some("right")
    );
}

#[test]
fn unsupported_css_syntax_and_values_are_reported() {
    let (root, _) =
        dom::parse("<p style='width:calc(100% - 2px); color:orange'>X</p>", 100).unwrap();
    let mut cascade = style::Cascade::new("p:has(b) { color:red }");
    cascade
        .compute(&element(&root, "p").unwrap(), &style::Computed::default())
        .unwrap();
    assert!(
        cascade
            .warnings
            .contains("unsupported stylesheet syntax or rule budget")
    );
    assert!(cascade.warnings.contains("unsupported CSS value for width"));
    assert!(cascade.warnings.contains("unsupported CSS value for color"));
}

#[test]
fn html_data_images_reuse_bytes_and_repeated_links_keep_occurrence_identity() {
    use base64::Engine;
    const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aP1sAAAAASUVORK5CYII=";
    let html = format!(
        "<p>A <img width=20 height=10 src='data:image/png;base64,{PNG}'> B <a href='https://example.test/one'>SAME</a><a href='https://example.test/two'>SAME</a></p>"
    );
    let bytes = super::super::tests::package(
        "<w:altChunk r:id=\"chunk\"/><w:altChunk r:id=\"chunk\"/>",
        "text/html",
        html.as_bytes(),
        false,
    );
    let opened = crate::office::docx::extract::open(bytes, &Default::default()).unwrap();
    let mut parts = Vec::new();
    let mut links = Vec::new();
    for block in &opened.sections[0].blocks {
        let Block::Para(paragraph) = block else {
            panic!("unexpected block")
        };
        for inline in &paragraph.inlines {
            match inline {
                Inline::Picture(pic, _) => {
                    assert_eq!((pic.w, pic.h), (15.0, 7.5));
                    parts.push(pic.part.clone().unwrap());
                }
                Inline::Text(text, style) if text == "SAME" => {
                    let link = style.source.as_ref().unwrap().link.as_ref().unwrap();
                    let crate::office::pptx::scene::SourceLinkTarget::ExternalUri(uri) =
                        &link.target
                    else {
                        panic!("external HTML link");
                    };
                    links.push((link.id.clone(), uri.clone()));
                }
                _ => {}
            }
        }
    }
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[0], parts[1]);
    assert_eq!(
        opened.pkg.read(&parts[0]).unwrap(),
        base64::engine::general_purpose::STANDARD
            .decode(PNG)
            .unwrap()
    );
    assert_eq!(
        links
            .iter()
            .map(|(_, uri)| uri.as_str())
            .collect::<Vec<_>>(),
        vec![
            "https://example.test/one",
            "https://example.test/two",
            "https://example.test/one",
            "https://example.test/two"
        ]
    );
    assert_eq!(
        links
            .iter()
            .map(|(id, _)| id)
            .collect::<std::collections::HashSet<_>>()
            .len(),
        4
    );
    assert_eq!(
        super::super::tests::text(&opened.sections[0].blocks),
        "A  B SAMESAMEA  B SAMESAME"
    );
}

#[test]
fn html_host_cell_ownership_and_row_group_span_boundaries_are_retained() {
    let html=b"<table><tbody><tr><td rowspan=0>A</td><td>B</td></tr><tr><td>C</td></tr></tbody><tbody><tr><td>D</td><td>E</td></tr></tbody></table>";
    let body = "<w:tbl><w:tblGrid><w:gridCol w:w=\"4000\"/></w:tblGrid><w:tr><w:tc><w:p><w:r><w:t>HOST</w:t></w:r></w:p><w:altChunk r:id=\"chunk\"/></w:tc></w:tr></w:tbl>";
    let opened = crate::office::docx::extract::open(
        super::super::tests::package(body, "text/html", html, false),
        &Default::default(),
    )
    .unwrap();
    let Block::Table(host) = &opened.sections[0].blocks[0] else {
        panic!("host table")
    };
    let Block::Table(imported) = &host.rows[0].cells[0].blocks[1] else {
        panic!("imported table")
    };
    assert_eq!(imported.rows.len(), 3);
    assert_eq!(imported.rows[2].cells.len(), 2);
    let Block::Para(paragraph) = &imported.rows[0].cells[0].blocks[0] else {
        panic!("paragraph")
    };
    let Inline::Text(_, style) = &paragraph.inlines[0] else {
        panic!("text")
    };
    let source = style.source.as_ref().unwrap();
    assert_eq!(source.part, "word/document.xml");
    assert_eq!(source.cells[0].table, 0);
    assert_eq!(source.cells.len(), 2);
}

#[test]
fn html_preformatted_projection_is_bounded_before_control_expansion() {
    let html = format!("<pre>{}</pre>", "x\n\t".repeat(30_000));
    let body = "<w:p><w:r><w:t>BEFORE</w:t></w:r></w:p><w:altChunk r:id=\"chunk\"/><w:p><w:r><w:t>AFTER</w:t></w:r></w:p>";
    let opened = crate::office::docx::extract::open(
        super::super::tests::package(body, "text/html", html.as_bytes(), false),
        &Default::default(),
    )
    .unwrap();
    assert_eq!(
        super::super::tests::text(&opened.sections[0].blocks),
        "BEFOREAFTER"
    );
    assert!(
        opened
            .import_warnings
            .iter()
            .any(|(code, _)| code == "ALTCHUNK_RESOURCE_LIMIT")
    );
}

#[test]
fn html_row_groups_inherit_style_and_positive_spans_create_bounded_implied_rows() {
    let html=b"<table><tbody style='font-size:24pt'><tr><td rowspan=3>A</td></tr></tbody><tbody><tr><td>B</td></tr></tbody></table>";
    let opened = crate::office::docx::extract::open(
        super::super::tests::package("<w:altChunk r:id=\"chunk\"/>", "text/html", html, false),
        &Default::default(),
    )
    .unwrap();
    let Block::Table(table) = &opened.sections[0].blocks[0] else {
        panic!("table")
    };
    assert_eq!(table.rows.len(), 4);
    let Block::Para(p) = &table.rows[0].cells[0].blocks[0] else {
        panic!("paragraph")
    };
    let Inline::Text(_, style) = &p.inlines[0] else {
        panic!("text")
    };
    assert_eq!(style.size, 24.0);
    assert_eq!(
        super::super::tests::text(&table.rows[3].cells[0].blocks),
        "B"
    );
}

#[test]
fn html_collapsible_spaces_cross_nested_inline_boundaries_once() {
    let html = b"<p>WHITE_A  <span>  WHITE_B </span> WHITE_C</p><pre>PRE_A  PRE_B</pre>";
    let opened = crate::office::docx::extract::open(
        super::super::tests::package("<w:altChunk r:id=\"chunk\"/>", "text/html", html, false),
        &Default::default(),
    )
    .unwrap();
    assert_eq!(
        super::super::tests::text(&opened.sections[0].blocks),
        "WHITE_A WHITE_B WHITE_CPRE_A  PRE_B"
    );
}

#[test]
fn html_rows_use_minimum_height_and_complete_percentage_cells_seed_grid() {
    let html=b"<table style='width:50%'><tr style='height:20pt'><td style='width:25%'>A</td><td style='width:25%'>B</td><td style='width:50%'>C<br>D<br>E</td></tr></table>";
    let opened = crate::office::docx::extract::open(
        super::super::tests::package("<w:altChunk r:id=\"chunk\"/>", "text/html", html, false),
        &Default::default(),
    )
    .unwrap();
    let Block::Table(table) = &opened.sections[0].blocks[0] else {
        panic!("table")
    };
    assert_eq!(table.rows[0].height, 20.0);
    assert!(matches!(table.rows[0].height_rule, HeightRule::AtLeast));
    assert_eq!(table.grid[0], table.grid[1]);
    assert_eq!(table.grid[2], table.grid[0] * 2.0);
    assert_eq!(
        super::super::tests::text(&table.rows[0].cells[2].blocks),
        "CDE"
    );
}

#[test]
fn html_conflicting_percentage_rows_preserve_text_and_diagnose_negotiation() {
    let html=b"<table style='width:50%'><tr><td style='width:25%'>A</td><td style='width:75%'>B</td></tr><tr><td style='width:50%'>C</td><td style='width:50%'>D</td></tr></table>";
    let opened = crate::office::docx::extract::open(
        super::super::tests::package("<w:altChunk r:id=\"chunk\"/>", "text/html", html, false),
        &Default::default(),
    )
    .unwrap();
    assert!(
        opened
            .import_warnings
            .iter()
            .any(|(code, detail)| code == "ALTCHUNK_HTML_UNSUPPORTED"
                && detail.contains("conflicting percentage"))
    );
    let Block::Table(table) = &opened.sections[0].blocks[0] else {
        panic!("table")
    };
    let raw = table
        .rows
        .iter()
        .flat_map(|r| &r.cells)
        .map(|c| super::super::tests::text(&c.blocks))
        .collect::<String>();
    assert_eq!(raw, "ABCD");
}

#[test]
fn materialized_host_tables_require_complete_text_only_imports() {
    for (mime, html, ordinary, eligible) in [
        ("text/html", "<p>FIRST</p><p>SECOND</p>", "", true),
        ("text/plain", "FIRST\nSECOND", "", true),
        ("text/html", "<p>FIRST</p>", "HOST", true),
        ("text/rtf", "unsupported", "HOST", false),
        (
            "text/html",
            "<p>FIRST</p><canvas>FALLBACK</canvas>",
            "HOST",
            false,
        ),
        (
            "text/html",
            "<table><tr><td>NESTED</td></tr></table>",
            "HOST",
            false,
        ),
        ("text/html", "<p> </p>", "", false),
    ] {
        let body = format!(
            "<w:tbl><w:tblGrid><w:gridCol w:w=\"4000\"/></w:tblGrid><w:tr><w:tc><w:p><w:r><w:t>{ordinary}</w:t></w:r></w:p><w:altChunk r:id=\"chunk\"/></w:tc></w:tr><w:tr><w:tc><w:p><w:r><w:t>TAIL</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"
        );
        let opened = crate::office::docx::extract::open(
            super::super::tests::package(&body, mime, html.as_bytes(), false),
            &Default::default(),
        )
        .unwrap();
        let Block::Table(table) = &opened.sections[0].blocks[0] else {
            panic!("table")
        };
        let Block::Para(tail) = &table.rows[1].cells[0].blocks[0] else {
            panic!("tail")
        };
        let Inline::Text(_, style) = &tail.inlines[0] else {
            panic!("tail text")
        };
        let source = style.source.as_ref().unwrap();
        assert_eq!(source.cells[0].text_only, eligible, "{mime}: {html}");
        assert_eq!(
            (
                source.cells[0].table,
                source.cells[0].row,
                source.cells[0].cell
            ),
            (0, 1, 0)
        );
    }
}

#[test]
fn materialized_tables_keep_selected_branches_and_original_table_ids() {
    let row = "<w:tr><w:tc><w:altChunk r:id=\"chunk\"/><w:altChunk r:id=\"chunk\"/></w:tc></w:tr>";
    let inactive = r#"<mc:AlternateContent xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:unknown="urn:unsupported"><mc:Choice Requires="unknown"><w:altChunk r:id="missing"/></mc:Choice><mc:Fallback><w:p><w:r><w:t>FALLBACK</w:t></w:r></w:p></mc:Fallback></mc:AlternateContent>"#;
    let body = format!(
        "<w:tbl>{row}</w:tbl><w:tbl><w:tr><w:tc>{inactive}</w:tc></w:tr></w:tbl><w:tbl>{row}</w:tbl>"
    );
    let opened = crate::office::docx::extract::open(
        super::super::tests::package(&body, "text/html", b"<p>IMPORT</p>", false),
        &Default::default(),
    )
    .unwrap();
    assert!(opened.import_warnings.is_empty());
    for (id, block) in opened.sections[0].blocks.iter().enumerate() {
        let Block::Table(table) = block else {
            panic!("table")
        };
        assert_eq!(
            super::super::tests::text(&table.rows[0].cells[0].blocks),
            if id == 1 { "FALLBACK" } else { "IMPORTIMPORT" }
        );
        for block in &table.rows[0].cells[0].blocks {
            let Block::Para(p) = block else {
                panic!("paragraph")
            };
            for inline in &p.inlines {
                if let Inline::Text(_, style) = inline {
                    let source = style.source.as_ref().unwrap();
                    assert_eq!(source.cells[0].table, id);
                    assert!(source.cells[0].text_only);
                }
            }
        }
    }
}

#[test]
fn margin_shorthand_preserves_declaration_order_and_priority() {
    for (css, expected) in [
        ("margin:1pt", ["1pt", "1pt", "1pt", "1pt"]),
        ("margin:1pt 2pt", ["1pt", "2pt", "1pt", "2pt"]),
        ("margin:1pt 2pt 3pt", ["1pt", "2pt", "3pt", "2pt"]),
        ("margin:1pt 2pt 3pt 4pt", ["1pt", "2pt", "3pt", "4pt"]),
        (
            "margin-left:9pt;margin:1pt 2pt",
            ["1pt", "2pt", "1pt", "2pt"],
        ),
        (
            "margin:1pt 2pt;margin-left:9pt",
            ["1pt", "2pt", "1pt", "9pt"],
        ),
        (
            "margin-left:9pt!important;margin:1pt 2pt",
            ["1pt", "2pt", "1pt", "9pt"],
        ),
        (
            "margin:1pt!important;margin-left:9pt",
            ["1pt", "1pt", "1pt", "1pt"],
        ),
    ] {
        let (root, _) = dom::parse(&format!("<p style='{css}'>X</p>"), 100).unwrap();
        let mut cascade = style::Cascade::new("");
        let result = cascade
            .compute(&element(&root, "p").unwrap(), &style::Computed::default())
            .unwrap();
        assert!(cascade.warnings.is_empty(), "{css}");
        for (side, value) in ["top", "right", "bottom", "left"].into_iter().zip(expected) {
            assert_eq!(result.get(&format!("margin-{side}")), Some(value), "{css}");
        }
    }
    let (root, _) = dom::parse("<p style='margin:2pt'>X</p>", 100).unwrap();
    let mut cascade = style::Cascade::new("p { margin-left:9pt!important; margin:1pt }");
    let result = cascade
        .compute(&element(&root, "p").unwrap(), &style::Computed::default())
        .unwrap();
    assert_eq!(result.get("margin-left"), Some("9pt"));
    assert_eq!(result.get("margin-top"), Some("2pt"));
}

#[test]
fn zero_margin_stylesheet_preserves_imported_host_table_eligibility() {
    let body = "<w:tbl><w:tr><w:tc><w:altChunk r:id=\"chunk\"/></w:tc></w:tr></w:tbl>";
    let html = b"<style>p { margin:0 }</style><p>CELL_FIRST</p><p>CELL_SECOND</p>";
    let opened = crate::office::docx::extract::open(
        super::super::tests::package(body, "text/html", html, false),
        &Default::default(),
    )
    .unwrap();
    assert!(opened.import_warnings.is_empty());
    let Block::Table(table) = &opened.sections[0].blocks[0] else {
        panic!("table")
    };
    assert_eq!(
        super::super::tests::text(&table.rows[0].cells[0].blocks),
        "CELL_FIRSTCELL_SECOND"
    );
    for block in &table.rows[0].cells[0].blocks {
        let Block::Para(p) = block else {
            panic!("paragraph")
        };
        let Inline::Text(_, style) = &p.inlines[0] else {
            panic!("text")
        };
        assert!(style.source.as_ref().unwrap().cells[0].text_only);
    }
}

#[test]
fn descendant_selector_search_has_a_shared_work_budget() {
    let html = format!(
        "{}<p>VISIBLE</p>{}",
        "<div>".repeat(24),
        "</div>".repeat(24)
    );
    let (root, _) = dom::parse(&html, 1000).unwrap();
    let css = format!(".absent {}p {{ color:red }}", "div ".repeat(12));
    let mut cascade = style::Cascade::new(&css);
    let paragraph = element(&root, "p").unwrap();
    assert_eq!(
        cascade
            .compute(&paragraph, &style::Computed::default())
            .unwrap_err(),
        "HTML selector work budget exceeded"
    );
    assert_eq!(
        cascade
            .compute(&paragraph, &style::Computed::default())
            .unwrap_err(),
        "HTML selector work budget exceeded"
    );
}

#[test]
fn exhausted_selector_import_is_discarded_before_following_import() {
    let body = "<w:p><w:r><w:t>BEFORE</w:t></w:r></w:p><w:altChunk r:id=\"chunk\"/><w:altChunk r:id=\"chunk\"/><w:p><w:r><w:t>AFTER</w:t></w:r></w:p>";
    let html = format!(
        "<style>.absent {}p {{color:red}}</style><p>EARLY</p>{}<p>LATE</p>{}",
        "div ".repeat(15),
        "<div>".repeat(30),
        "</div>".repeat(30)
    );
    let opened = crate::office::docx::extract::open(
        super::super::tests::package(body, "text/html", html.as_bytes(), false),
        &Default::default(),
    )
    .unwrap();
    assert_eq!(
        super::super::tests::text(&opened.sections[0].blocks),
        "BEFOREAFTER"
    );
    assert!(
        opened
            .import_warnings
            .iter()
            .any(|(code, why)| code == "ALTCHUNK_RESOURCE_LIMIT" && why.contains("selector work"))
    );
}

#[test]
fn supported_simple_deep_and_adjacent_selectors_retain_cascade() {
    let html = format!(
        "{}<p class='first'>A</p><p class='second'>B</p>{}",
        "<div>".repeat(50),
        "</div>".repeat(50)
    );
    let (root, _) = dom::parse(&html, 1000).unwrap();
    let mut cascade = style::Cascade::new(
        "div p {color:red} .first + .second {color:blue} div > p {font-weight:bold}",
    );
    let first = element(&root, "p").unwrap();
    let parent = first.parent.borrow().upgrade().unwrap();
    let second = parent
        .children
        .borrow()
        .iter()
        .find(|n| n.attr("class").as_deref() == Some("second"))
        .unwrap()
        .clone();
    assert_eq!(
        cascade
            .compute(&first, &style::Computed::default())
            .unwrap()
            .get("color"),
        Some("red")
    );
    let result = cascade
        .compute(&second, &style::Computed::default())
        .unwrap();
    assert_eq!(result.get("color"), Some("blue"));
    assert_eq!(result.get("font-weight"), Some("bold"));
    assert!(cascade.warnings.is_empty());
}
