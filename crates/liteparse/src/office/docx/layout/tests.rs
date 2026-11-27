use super::*;
use crate::office::docx::testutil::docx;

fn units(body: &str) -> Vec<Unit> {
    units_context(body, false, false)
}

fn units_context(body: &str, in_cell: bool, in_header: bool) -> Vec<Unit> {
    units_story(body, in_cell, in_header, false)
}

fn units_story(body: &str, in_cell: bool, in_header: bool, footer: bool) -> Vec<Unit> {
    units_package(docx(body, None, None), in_cell, in_header, footer)
}

fn units_package(bytes: Vec<u8>, in_cell: bool, in_header: bool, footer: bool) -> Vec<Unit> {
    let opened = crate::office::docx::extract::open(bytes, &Default::default()).unwrap();
    let fonts = crate::office::pptx::fonts::FontSystem::new(&[], true, &Default::default());
    let env = TextEnv {
        fonts: &fonts,
        default_tab: 36.0,
        fields: Default::default(),
        grid: None,
        char_grid: Default::default(),
        expand_shift_return: false,
        mark_in_last_line: true,
        html_spacing: true,
        grow_autofit: false,
        in_cell: std::cell::Cell::new(in_cell),
        in_header: std::cell::Cell::new(in_header),
        grid_in_table: false,
        no_leading: false,
        compat_mode: 15,
    };
    let flow = Flow::new(&env, 15);
    let flow = if footer {
        flow.with_footer_frames()
    } else {
        flow
    };
    flow.units(&opened.sections[0].blocks, 468.0, in_cell)
}

fn table(rows: &str) -> String {
    format!(
        r#"<w:tbl><w:tblPr><w:tblBorders><w:top w:val="double" w:sz="16"/><w:bottom w:val="single" w:sz="32"/><w:insideH w:val="single" w:sz="64"/></w:tblBorders><w:tblCellMar><w:top w:w="0" w:type="dxa"/><w:bottom w:w="0" w:type="dxa"/></w:tblCellMar></w:tblPr><w:tblGrid><w:gridCol w:w="3000"/></w:tblGrid>{rows}</w:tbl>"#
    )
}

fn row(height: usize, extra: &str, content: &str) -> String {
    format!(
        r#"<w:tr><w:trPr><w:trHeight w:val="{height}" w:hRule="exact"/>{extra}</w:trPr><w:tc>{content}</w:tc></w:tr>"#
    )
}

fn rows(units: &[Unit]) -> Vec<&RowUnit> {
    units
        .iter()
        .filter_map(|u| match u {
            Unit::Row(r) => Some(r),
            _ => None,
        })
        .collect()
}

#[test]
fn exact_row_height_adds_largest_bottom_padding_once() {
    let cell = |bottom| {
        format!(
            r#"<w:tc><w:tcPr><w:tcMar><w:top w:w="200" w:type="dxa"/><w:bottom w:w="{bottom}" w:type="dxa"/></w:tcMar></w:tcPr><w:p/></w:tc>"#
        )
    };
    let body = format!(
        r#"<w:tbl><w:tblGrid><w:gridCol w:w="3000"/><w:gridCol w:w="3000"/></w:tblGrid><w:tr><w:trPr><w:trHeight w:val="1200" w:hRule="exact"/></w:trPr>{}{}</w:tr></w:tbl>"#,
        cell(40),
        cell(140)
    );
    let us = units(&body);
    let rs = rows(&us);
    assert_eq!(rs[0].height, 67.0);
    assert_eq!(stack(&us).1, 67.0);
    // An "at least" height leaves out the padding: 60 pt plus the largest top
    // (10 pt) and bottom (7 pt) padding (Word for Mac probe).
    let minimum = units(&body.replace("w:hRule=\"exact\"", "w:hRule=\"atLeast\""));
    assert_eq!(rows(&minimum)[0].height, 77.0);
}

#[test]
fn exact_row_padding_uses_resolved_cell_margin_exceptions() {
    let body = |bottom| {
        format!(
            r#"<w:tbl><w:tblPr><w:tblCellMar><w:bottom w:w="140" w:type="dxa"/></w:tblCellMar></w:tblPr><w:tblGrid><w:gridCol w:w="3000"/></w:tblGrid><w:tr><w:trPr><w:trHeight w:val="1200" w:hRule="exact"/></w:trPr><w:tc><w:tcPr><w:tcMar>{bottom}</w:tcMar></w:tcPr><w:p/></w:tc></w:tr></w:tbl>"#
        )
    };
    let inherited = units(&body(r#"<w:bottom w:w="0" w:type="nil"/>"#));
    let cleared = units(&body(r#"<w:bottom w:w="0" w:type="dxa"/>"#));
    assert_eq!(rows(&inherited)[0].height, 67.0);
    assert_eq!(rows(&cleared)[0].height, 60.0);
}

#[test]
fn outer_table_bands_leave_row_centerline_heights_unchanged() {
    let us = units(&table(&(row(400, "", "<w:p/>") + &row(600, "", "<w:p/>"))));
    let rs = rows(&us);
    assert_eq!((rs[0].height, rs[1].height), (20.0, 30.0));
    assert_eq!((rs[0].outer_before, rs[0].outer_after), (0.0, 0.0));
    assert_eq!((rs[1].outer_before, rs[1].outer_after), (0.0, 0.0));
    let (slices, height) = stack(&us);
    assert_eq!(height, 50.0);
    assert_eq!(slices[1].y, 20.0);
}

#[test]
fn natural_outer_edges_follow_first_and_final_split_pieces() {
    let para = r#"<w:p><w:pPr><w:spacing w:before="0" w:after="0" w:line="200" w:lineRule="exact"/><w:widowControl w:val="0"/></w:pPr><w:r><w:t>line</w:t></w:r></w:p>"#;
    let us = units(&table(&row(1000, "", &para.repeat(4))));
    let r = rows(&us)[0];
    let (top, rest) = split_row(r, 25.0, false).expect("two lines fit");
    assert!(top.flow_height() <= 25.0);
    assert_eq!(top.outer_before, r.outer_before);
    assert_eq!(top.outer_after, 0.0);
    assert_eq!(rest.outer_before, 0.0);
    assert_eq!(rest.outer_after, r.outer_after);
    assert_eq!(top.cells[0].slices.len(), 2);
    assert_eq!(rest.cells[0].slices.len(), 2);
}

#[test]
fn repeated_headers_retain_only_the_natural_leading_band() {
    let us = units(&table(
        &(row(400, "<w:tblHeader/>", "<w:p/>") + &row(600, "", "<w:p/>")),
    ));
    let rs = rows(&us);
    let repeated = &rs[1].headers[0];
    assert_eq!(repeated.outer_before, rs[0].outer_before);
    assert_eq!(repeated.outer_after, 0.0);
    assert_eq!(repeated.height, 20.0);
}

#[test]
fn nested_table_slice_keeps_its_painted_outer_extent() {
    let inner = table(&row(400, "", "<w:p/>"));
    let outer = format!(
        r#"<w:tbl><w:tblGrid><w:gridCol w:w="6000"/></w:tblGrid><w:tr><w:tc>{inner}<w:p/></w:tc></w:tr></w:tbl>"#
    );
    let us = units(&outer);
    let nested = &rows(&us)[0].cells[0].slices[0];
    assert_eq!(nested.h, 20.0);
}

#[test]
fn trailing_outer_edge_does_not_create_an_empty_final_piece() {
    let us = units(&table(&row(1000, "", "<w:p/>")));
    let r = rows(&us)[0];
    assert!(r.flow_height() < 54.0);
}

fn merge_table(spec: &[(usize, usize, &str)]) -> Vec<Unit> {
    let mut body = String::from(
        r#"<w:tbl><w:tblPr><w:tblBorders><w:top w:val="single" w:sz="4"/><w:bottom w:val="single" w:sz="4"/><w:insideH w:val="single" w:sz="4"/></w:tblBorders></w:tblPr><w:tblGrid><w:gridCol w:w="1000"/><w:gridCol w:w="1000"/><w:gridCol w:w="1000"/><w:gridCol w:w="1000"/></w:tblGrid>"#,
    );
    for &(before, span, merge) in spec {
        body.push_str(&format!(r#"<w:tr><w:trPr><w:gridBefore w:val="{before}"/><w:trHeight w:val="400" w:hRule="exact"/></w:trPr><w:tc><w:tcPr><w:gridSpan w:val="{span}"/>{merge}</w:tcPr><w:p/></w:tc><w:tc><w:p/></w:tc></w:tr>"#));
    }
    body.push_str("</w:tbl>");
    units(&body)
}

#[test]
fn merge_interior_rules_disappear_only_with_adjacent_successor() {
    let us = merge_table(&[
        (1, 2, r#"<w:vMerge w:val="restart"/>"#),
        (1, 2, "<w:vMerge/>"),
        (1, 2, "<w:vMerge/>"),
    ]);
    let rs = rows(&us);
    assert_eq!(
        rs.iter().map(|r| r.height).collect::<Vec<_>>(),
        vec![20.0; 3]
    );
    assert_eq!(
        rs.iter()
            .map(|r| r.cells[0].merge_continues)
            .collect::<Vec<_>>(),
        vec![true, true, false]
    );
    for r in &rs {
        assert!(r.cells[0].borders[2].is_some());
        assert!(!r.cells[1].merge_continues);
    }
    // Empty, unshaded cells draw only single horizontal border strips.
    assert_eq!(
        rs.iter()
            .map(|r| row_items(r, 0.0, 0.0).len())
            .collect::<Vec<_>>(),
        vec![3, 2, 3]
    );
    assert_eq!(row_items_fragment(rs[0], 0.0, 0.0, false).len(), 4);
    assert_eq!(row_items_fragment(rs[1], 0.0, 0.0, false).len(), 3);
}

#[test]
fn malformed_or_restarted_merges_keep_their_bottom_perimeter() {
    let restart = r#"<w:vMerge w:val="restart"/>"#;
    for spec in [
        vec![(1, 1, "<w:vMerge/>"), (1, 1, "<w:vMerge/>")],
        vec![(1, 1, restart), (1, 2, "<w:vMerge/>")],
        vec![(1, 1, restart), (0, 1, "<w:vMerge/>")],
        vec![(1, 1, restart), (1, 1, restart)],
    ] {
        let us = merge_table(&spec);
        assert!(rows(&us).iter().all(|r| !r.cells[0].merge_continues));
    }
}

#[test]
fn merged_explicit_edges_are_retained_for_fragment_painting() {
    let restart = r#"<w:vMerge w:val="restart"/><w:tcBorders><w:bottom w:val="single" w:sz="16"/></w:tcBorders>"#;
    let continuation = r#"<w:vMerge/><w:tcBorders><w:bottom w:val="nil"/></w:tcBorders>"#;
    let us = merge_table(&[(0, 1, restart), (0, 1, continuation)]);
    let rs = rows(&us);
    assert_eq!(rs[0].cells[0].borders[2].as_ref().unwrap().width, 2.0);
    assert!(rs[1].cells[0].borders[2].is_none());
    assert_eq!(row_items(rs[0], 0.0, 0.0).len(), 3);
    assert_eq!(row_items_fragment(rs[0], 0.0, 0.0, false).len(), 4);
}

#[test]
fn explicit_page_frames_group_and_leave_text_relative_paragraphs_in_flow() {
    let body = r#"<w:p><w:pPr><w:framePr w:w="2400" w:y="3600" w:vAnchor="page" w:wrap="notBeside"/></w:pPr><w:r><w:t>FIRST</w:t></w:r></w:p>
<w:p><w:pPr><w:framePr w:w="2400" w:y="3600" w:vAnchor="page" w:wrap="notBeside"/></w:pPr><w:r><w:t>SECOND</w:t></w:r></w:p>
<w:p><w:pPr><w:framePr w:yAlign="center" w:wrap="notBeside"/></w:pPr><w:r><w:t>FLOW</w:t></w:r></w:p>"#;
    let laid = units(body);
    assert_eq!(laid.len(), 1);
    let Unit::Para(last) = laid.last().unwrap() else {
        panic!("paragraph expected")
    };
    let anchors: Vec<_> = last.lines.iter().flat_map(|l| &l.anchors).collect();
    assert_eq!(anchors.len(), 1);
    if let Some(frame) = anchors.first() {
        assert_eq!(frame.y, Ok(180.0));
        assert_eq!(frame.pic.w, 120.0);
        assert_eq!(frame.v_rel, crate::office::docx::model::VRel::Page);
        assert!(frame.text_items.len() >= 2);
    }
}

#[test]
fn scoped_page_frames_require_explicit_numeric_page_offset() {
    for attrs in [
        r#"w:vAnchor="text" w:y="400""#,
        r#"w:vAnchor="margin" w:y="400""#,
        r#"w:vAnchor="page""#,
        r#"w:vAnchor="page" w:yAlign="center""#,
        r#"w:vAnchor="page" w:y="400" w:dropCap="drop""#,
    ] {
        let body =
            format!(r#"<w:p><w:pPr><w:framePr {attrs}/></w:pPr><w:r><w:t>FLOW</w:t></w:r></w:p>"#);
        let laid = units(&body);
        assert_eq!(laid.len(), 1);
        let Unit::Para(p) = &laid[0] else {
            panic!("paragraph expected")
        };
        assert!(p.lines.iter().all(|l| l.anchors.is_empty()));
    }
}

#[test]
fn page_frames_float_in_headers_and_stay_in_cell_flow() {
    let body = r#"<w:p><w:pPr><w:framePr w:w="2000" w:y="2000" w:vAnchor="page"/></w:pPr><w:r><w:t>FRAME</w:t></w:r></w:p><w:p><w:r><w:t>FOLLOWING</w:t></w:r></w:p>"#;
    let laid = units_context(body, true, false);
    assert_eq!(laid.len(), 2);
    for unit in &laid {
        let Unit::Para(p) = unit else {
            panic!("paragraph expected")
        };
        assert!(p.lines.iter().all(|l| l.anchors.is_empty()));
    }
    // In a header the frame floats from the following paragraph.
    let laid = units_context(body, false, true);
    assert_eq!(laid.len(), 1);
    let Unit::Para(p) = &laid[0] else {
        panic!("paragraph expected")
    };
    assert_eq!(p.lines[0].anchors.len(), 1);
}

#[test]
fn nested_frame_drawing_retains_distinct_paragraph_and_line_origins() {
    let body = r#"<w:p><w:pPr><w:framePr w:w="4000" w:vAnchor="page" w:y="4000"/></w:pPr><w:r><w:t>FIRST</w:t><w:br/><w:drawing><wp:anchor xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"><wp:extent cx="914400" cy="914400"/></wp:anchor></w:drawing><w:t>SECOND</w:t></w:r></w:p><w:p/>"#;
    let laid = units(body);
    {
        let Unit::Para(p) = &laid[0] else {
            panic!("paragraph expected")
        };
        let frame = &p.lines[0].anchors[0];
        assert_eq!(frame.frame_children.len(), 1);
        let child = &frame.frame_children[0];
        assert!(
            child.line_y > child.paragraph_y,
            "drawing anchored after the line break"
        );
    }
}

#[test]
fn frame_child_origins_include_paragraph_spacing_and_border_padding() {
    let body = r#"<w:p><w:r><w:t>FIRST</w:t><w:br/><w:drawing><wp:anchor xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"><wp:extent cx="914400" cy="914400"/></wp:anchor></w:drawing></w:r></w:p>"#;
    let mut laid = units(body);
    let Unit::Para(p) = &mut laid[0] else {
        panic!("paragraph expected")
    };
    p.before = 12.0;
    p.pad_top = 3.0;
    let first_height = p.lines[0].height;
    let anchors = frame_child_anchors(&laid);
    assert_eq!(anchors.len(), 1);
    assert_eq!(anchors[0].paragraph_y, 0.0);
    assert_eq!(anchors[0].line_y, 15.0 + first_height);
}

#[test]
fn table_page_break_is_scoped_to_leading_body_cell_paragraph() {
    let p = r#"<w:p><w:pPr><w:pageBreakBefore/></w:pPr><w:r><w:t>break</w:t></w:r></w:p>"#;
    let body = table(&row(400, "", p));
    assert!(rows(&units(&body))[0].page_break_before);
    assert!(!rows(&units_context(&body, true, false))[0].page_break_before);
    assert!(!rows(&units_context(&body, false, true))[0].page_break_before);
    let later = table(&row(400, "", &format!("<w:p/>{p}")));
    assert!(!rows(&units(&later))[0].page_break_before);
    let other_cell = table(&format!("<w:tr><w:tc><w:p/></w:tc><w:tc>{p}</w:tc></w:tr>"));
    assert!(!rows(&units(&other_cell))[0].page_break_before);
    let floating = body.replace(
        "<w:tblPr>",
        r#"<w:tblPr><w:tblpPr w:vertAnchor="page" w:horzAnchor="margin" w:tblpY="0" w:tblpX="0"/>"#,
    );
    assert!(!rows(&units(&floating))[0].page_break_before);
    let nested = table(&row(400, "", &format!("{body}<w:p/>")));
    assert!(!rows(&units(&nested))[0].page_break_before);
    let continued = body.replace("<w:tc>", r#"<w:tc><w:tcPr><w:vMerge/></w:tcPr>"#);
    assert!(!rows(&units(&continued))[0].page_break_before);
    let disabled = body.replace("<w:pageBreakBefore/>", r#"<w:pageBreakBefore w:val="0"/>"#);
    assert!(!rows(&units(&disabled))[0].page_break_before);
}

#[test]
fn top_merge_flow_retains_note_and_anchor_placement_owners() {
    let anchor = r#"<w:drawing><wp:anchor xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"><wp:positionH relativeFrom="column"><wp:posOffset>0</wp:posOffset></wp:positionH><wp:positionV relativeFrom="paragraph"><wp:posOffset>0</wp:posOffset></wp:positionV><wp:extent cx="127000" cy="127000"/><wp:wrapNone/></wp:anchor></w:drawing>"#;
    for (extra, eligible) in [
        ("", true),
        (r#"<w:footnoteReference w:id="1"/>"#, false),
        (anchor, false),
    ] {
        let body = table(&format!(
            r#"<w:tr><w:tc><w:tcPr><w:vMerge w:val="restart"/></w:tcPr><w:p><w:r><w:t>merge text</w:t>{extra}</w:r></w:p></w:tc></w:tr><w:tr><w:tc><w:tcPr><w:vMerge/></w:tcPr><w:p/></w:tc></w:tr>"#
        ));
        let us = units(&body);
        let rs: Vec<_> = us
            .iter()
            .filter_map(|u| match u {
                Unit::Row(r) => Some(r),
                _ => None,
            })
            .collect();
        assert_eq!(rs[1].cells[0].moved.is_some(), eligible);
        if !eligible {
            assert!(!rs[0].cells[0].slices.is_empty());
            if extra == anchor {
                assert!(rs[0].cells[0].slices.iter().any(|s| !s.anchors.is_empty()));
            } else {
                assert_eq!(rs[0].notes(), vec![1]);
            }
        }
    }
}

#[test]
fn top_merge_flow_keeps_nested_slice_content_local() {
    let body = table(
        r#"<w:tr><w:tc><w:tcPr><w:vMerge w:val="restart"/></w:tcPr><w:p><w:r><w:t>nested owner</w:t></w:r></w:p></w:tc></w:tr><w:tr><w:tc><w:tcPr><w:vMerge/></w:tcPr><w:p/></w:tc></w:tr>"#,
    );
    for (in_cell, in_header) in [(true, false), (false, true)] {
        let us = units_context(&body, in_cell, in_header);
        let rs: Vec<_> = us
            .iter()
            .filter_map(|u| match u {
                Unit::Row(r) => Some(r),
                _ => None,
            })
            .collect();
        assert!(rs[1].cells[0].moved.is_none());
        assert!(!rs[0].cells[0].slices.is_empty());
        assert_eq!(
            row_items(rs[0], 0.0, 0.0)
                .iter()
                .filter(|i| matches!(i,Item::Text(r)if r.text=="nested owner"))
                .count(),
            1
        );
    }
}

#[test]
fn merge_flow_header_scope_uses_actual_leading_prefix() {
    let merged = |header: &str, merge: &str| {
        format!(
            r#"<w:tr><w:trPr>{header}</w:trPr><w:tc><w:tcPr>{merge}</w:tcPr><w:p><w:r><w:t>owner</w:t></w:r></w:p></w:tc></w:tr>"#
        )
    };
    let start = r#"<w:vMerge w:val="restart"/>"#;
    let cases = [
        format!(
            "{}{}{}",
            merged("<w:tblHeader/>", start),
            merged("", "<w:vMerge/>"),
            merged("", "")
        ),
        format!(
            "{}{}{}",
            merged("", ""),
            merged("<w:tblHeader/>", start),
            merged("<w:tblHeader/>", "<w:vMerge/>")
        ),
    ];
    for (case, body) in cases.iter().enumerate() {
        let us = units(&table(body));
        let rs: Vec<_> = us
            .iter()
            .filter_map(|u| match u {
                Unit::Row(r) => Some(r),
                _ => None,
            })
            .collect();
        if case == 0 {
            assert!(rs[0].repeating_header);
            assert!(
                rs[1].cells[0].moved.is_none(),
                "crossing header/body retains original top owner"
            );
        } else {
            assert!(rs.iter().all(|r| !r.repeating_header));
        }
    }
}

#[test]
fn relocated_cell_content_keeps_perimeter_strokes_above_its_raster() {
    let body = r#"<w:tbl><w:tblGrid><w:gridCol w:w="1600"/></w:tblGrid><w:tr><w:trPr><w:trHeight w:val="800"/></w:trPr><w:tc><w:tcPr><w:shd w:fill="FFFFFF"/><w:tcBorders><w:top w:val="single" w:sz="64" w:color="FF0000"/></w:tcBorders></w:tcPr><w:p/></w:tc></w:tr></w:tbl>"#;
    let laid = units(body);
    let Unit::Row(row) = &laid[0] else {
        panic!("row")
    };
    let (mut content, borders) = row_items_fragment_layers(row, 20.0, 20.0, false);
    content.extend(solid(
        rect(40.0, 18.0, 10.0, 20.0),
        Rgba::from_hex("008080").unwrap(),
    ));
    content.extend(borders);
    let scene = crate::office::pptx::scene::Scene {
        width: 120.0,
        height: 100.0,
        items: content,
        ..Default::default()
    };
    let cfg = crate::office::pptx::config::Config::default();
    let opened = crate::office::docx::extract::open(docx(body, None, None), &cfg).unwrap();
    let doc = crate::office::pptx::document::Document::from_package(opened.pkg, 1524000, 1270000);
    let fonts = crate::office::pptx::fonts::FontSystem::new(&[], true, &Default::default());
    let images = crate::office::pptx::render::ImageCache::new(cfg.limits.max_image_pixels, &fonts);
    let target = crate::office::pptx::render::RenderTarget {
        region: crate::office::pptx::scene::Rect {
            x: 0.0,
            y: 0.0,
            w: 120.0,
            h: 100.0,
        },
        width: 120,
        height: 100,
    };
    let pm = crate::office::pptx::render::render(&scene, &target, &doc, &fonts, &images).unwrap();
    let edge = pm.pixel(45, 20).unwrap();
    assert!(
        edge.red() > 245 && edge.green() < 10 && edge.blue() < 10,
        "the red border covers overlapping continuation content"
    );
    let interior = pm.pixel(45, 30).unwrap();
    assert!(
        interior.red() < 10
            && interior.green() > 110
            && interior.green() < 145
            && interior.blue() > 110
            && interior.blue() < 145,
        "the continuation remains visible inside the white cell"
    );
}

#[test]
fn mixed_vertical_padding_sizes_auto_and_minimum_rows_once() {
    let fixture = |rule: &str, padding: usize, align: &str| {
        let cell = |n: usize, margin: usize| {
            let text = "<w:t>marker</w:t>";
            let content = (0..n).map(|_| format!(r#"<w:p><w:pPr><w:spacing w:before="0" w:after="0" w:line="240" w:lineRule="exact"/></w:pPr><w:r><w:rPr><w:sz w:val="20"/></w:rPr>{text}</w:r></w:p>"#)).collect::<String>();
            format!(
                r#"<w:tc><w:tcPr><w:tcMar><w:top w:w="{margin}" w:type="dxa"/><w:bottom w:w="{margin}" w:type="dxa"/></w:tcMar><w:vAlign w:val="{align}"/></w:tcPr>{content}</w:tc>"#
            )
        };
        format!(
            r#"<w:tbl><w:tblGrid><w:gridCol w:w="3000"/><w:gridCol w:w="3000"/></w:tblGrid><w:tr><w:trPr>{rule}</w:trPr>{}{}</w:tr></w:tbl>"#,
            cell(3, 0),
            cell(1, padding)
        )
    };
    for rule in ["", r#"<w:trHeight w:val="400" w:hRule="atLeast"/>"#] {
        for padding in [0, 29, 58] {
            for align in ["top", "center", "bottom"] {
                let us = units(&fixture(rule, padding, align));
                let r = rows(&us)[0];
                let margin = padding as f64 / 20.0;
                assert!((r.height - (36.0 + 2.0 * margin)).abs() < 0.001);
                for c in &r.cells {
                    assert_eq!((c.margins[0], c.margins[2]), (margin, margin));
                }
                assert!((stack(&us).1 - r.height).abs() < 0.001);
                assert_eq!(r.cells[0].slices.len(), 3);
                assert_eq!(r.cells[1].slices.len(), 1);
            }
        }
    }
    // A minimum exceeding the content takes the padding on top (Word for Mac:
    // 60 pt with 58 tw top and bottom padding is 65.8 pt).
    let minimum = units(&fixture(
        r#"<w:trHeight w:val="1200" w:hRule="atLeast"/>"#,
        58,
        "top",
    ));
    assert!((rows(&minimum)[0].height - 65.8).abs() < 0.001);
    // Exact-row clipping and its separate bottom-padding rule remain unchanged.
    let exact = units(&fixture(
        r#"<w:trHeight w:val="400" w:hRule="exact"/>"#,
        58,
        "top",
    ));
    let r = rows(&exact)[0];
    assert_eq!(r.height, 22.9);
    assert_eq!(r.cells[0].margins[0], 0.0);
    assert_eq!(r.cells[1].margins[0], 2.9);
}

#[test]
fn mixed_vertical_padding_survives_row_fragmentation() {
    let para = r#"<w:p><w:pPr><w:spacing w:line="240" w:lineRule="exact" w:before="0" w:after="0"/></w:pPr><w:r><w:rPr><w:sz w:val="20"/></w:rPr><w:t>line</w:t></w:r></w:p>"#;
    let body = format!(
        r#"<w:tbl><w:tblGrid><w:gridCol w:w="3000"/><w:gridCol w:w="3000"/></w:tblGrid><w:tr><w:tc>{}</w:tc><w:tc><w:tcPr><w:tcMar><w:top w:w="58" w:type="dxa"/><w:bottom w:w="58" w:type="dxa"/></w:tcMar></w:tcPr>{para}</w:tc></w:tr></w:tbl>"#,
        para.repeat(3)
    );
    let us = units(&body);
    let r = rows(&us)[0];
    let (first, rest) = split_row(r, 30.0, false).expect("two padded lines fit");
    assert_eq!(first.cells[0].slices.len(), 2);
    assert_eq!(rest.cells[0].slices.len(), 1);
    assert!((first.height - 29.8).abs() < 0.001);
    assert!((rest.height - 17.8).abs() < 0.001);
    for row in [&first, &rest] {
        for c in &row.cells {
            assert_eq!((c.margins[0], c.margins[2]), (2.9, 2.9));
        }
    }
}

#[test]
fn mixed_vertical_padding_keeps_merge_start_owner_margins() {
    let para = r#"<w:p><w:pPr><w:spacing w:line="240" w:lineRule="exact" w:before="0" w:after="0"/></w:pPr><w:r><w:t>line</w:t></w:r></w:p>"#;
    let body = format!(
        r#"<w:tbl><w:tblGrid><w:gridCol w:w="3000"/><w:gridCol w:w="3000"/></w:tblGrid><w:tr><w:tc><w:tcPr><w:vMerge w:val="restart"/><w:vAlign w:val="center"/></w:tcPr>{}</w:tc><w:tc><w:tcPr><w:tcMar><w:top w:w="58" w:type="dxa"/><w:bottom w:w="58" w:type="dxa"/></w:tcMar></w:tcPr>{para}</w:tc></w:tr><w:tr><w:tc><w:tcPr><w:vMerge/></w:tcPr><w:p/></w:tc><w:tc>{para}</w:tc></w:tr></w:tbl>"#,
        para.repeat(3)
    );
    let us = units(&body);
    let rs = rows(&us);
    let owner = &rs[1].cells[0];
    assert_eq!((owner.margins[0], owner.margins[2]), (2.9, 2.9));
    assert_eq!(owner.merged_from, 1);
    assert_eq!(owner.moved.as_ref().unwrap().0.len(), 3);
    assert!((rs.iter().map(|r| r.height).sum::<f64>() - 41.8).abs() < 0.001);
}

#[test]
fn vertical_merge_uses_start_owner_padding_for_rotated_text() {
    let para = r#"<w:p><w:pPr><w:spacing w:line="240" w:lineRule="exact" w:before="0" w:after="0"/></w:pPr><w:r><w:t>line</w:t></w:r></w:p>"#;
    let body = format!(
        r#"<w:tbl><w:tblGrid><w:gridCol w:w="3000"/><w:gridCol w:w="3000"/></w:tblGrid><w:tr><w:tc><w:tcPr><w:vMerge w:val="restart"/><w:textDirection w:val="tbRl"/><w:vAlign w:val="top"/></w:tcPr>{}</w:tc><w:tc><w:tcPr><w:tcMar><w:top w:w="58" w:type="dxa"/><w:bottom w:w="58" w:type="dxa"/></w:tcMar></w:tcPr>{para}</w:tc></w:tr><w:tr><w:tc><w:tcPr><w:vMerge/></w:tcPr><w:p/></w:tc><w:tc>{para}</w:tc></w:tr></w:tbl>"#,
        para.repeat(3)
    );
    let us = units(&body);
    let rs = rows(&us);
    let owner = &rs[1].cells[0];
    assert_eq!((owner.margins[0], owner.margins[2]), (2.9, 2.9));
    assert_eq!(owner.merged_from, 1);
    assert!(!owner.slices[0].items.is_empty());
    assert_eq!(owner.slices.len(), 1);
}

#[test]
fn body_text_frames_float_from_the_following_paragraph() {
    let frame = r#"<w:p><w:pPr><w:framePr w:w="800" w:hAnchor="page" w:vAnchor="text" w:x="360" w:y="40"/></w:pPr><w:r><w:t>FRAME</w:t></w:r></w:p>"#;
    let laid = units(&format!(
        "{frame}<w:p><w:r><w:t>FOLLOWING</w:t></w:r></w:p>"
    ));
    assert_eq!(laid.len(), 1);
    let Unit::Para(p) = &laid[0] else {
        panic!("anchor paragraph")
    };
    let a = &p.lines[0].anchors[0];
    assert_eq!(
        (a.h_rel, a.x.clone(), a.y.clone()),
        (crate::office::docx::model::HRel::Page, Ok(18.0), Ok(2.0))
    );
    // With no paragraph to anchor to, the frame stays in flow.
    assert_eq!(units(frame).len(), 1);
    // An aligned frame floats at the top of the following paragraph.
    let aligned = frame.replace(r#"w:y="40""#, r#"w:yAlign="top""#);
    let laid = units(&format!(
        "{aligned}<w:p><w:r><w:t>FOLLOWING</w:t></w:r></w:p>"
    ));
    assert_eq!(laid.len(), 1);
}

#[test]
fn a_margin_positioned_anchor_in_a_cell_counts_from_the_cell_text() {
    // Word for Mac probe: 43 pt from the margin inside a cell lands 43 pt from the
    // cell's text, like a column-relative position.
    let para = r#"<w:p><w:r><w:drawing><wp:anchor xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" layoutInCell="1"><wp:positionH relativeFrom="margin"><wp:posOffset>548005</wp:posOffset></wp:positionH><wp:positionV relativeFrom="paragraph"><wp:posOffset>0</wp:posOffset></wp:positionV><wp:extent cx="914400" cy="914400"/><wp:wrapSquare wrapText="bothSides"/></wp:anchor></w:drawing><w:t>RIGHT</w:t></w:r></w:p>"#;
    let laid = units(&table(&row(0, "", para)));
    let anchors: Vec<_> = rows(&laid)[0]
        .cells
        .iter()
        .flat_map(|c| c.slices.iter())
        .flat_map(|s| s.anchors.iter())
        .collect();
    assert_eq!(anchors[0].0.h_rel, crate::office::docx::model::HRel::Column);
    // A page-relative offset counts from the cell's edge: less the 5.4 pt left padding.
    let page = para.replace(r#"relativeFrom="margin""#, r#"relativeFrom="page""#);
    let laid = units(&table(&row(0, "", &page)));
    let a = &rows(&laid)[0].cells[0].slices[0].anchors[0].0;
    assert_eq!(a.h_rel, crate::office::docx::model::HRel::Column);
    assert!(
        (a.x.clone().unwrap() - (548005.0 / 12700.0 - 5.4)).abs() < 0.01,
        "{:?}",
        a.x
    );
    // Outside a table it stays margin-relative.
    let Unit::Para(p) = &units(para)[0] else {
        panic!("paragraph")
    };
    assert_eq!(
        p.lines[0].anchors[0].h_rel,
        crate::office::docx::model::HRel::Margin
    );
}

#[test]
fn header_text_frames_side_by_side_float_from_the_paragraph_after_them() {
    // A title frame and a page-number frame, then an empty header paragraph.
    let frame = |align: &str, text: &str| {
        format!(
            r#"<w:p><w:pPr><w:framePr w:wrap="around" w:hAnchor="margin" w:vAnchor="text" w:xAlign="{align}" w:y="1"/></w:pPr><w:r><w:t>{text}</w:t></w:r></w:p>"#
        )
    };
    let source = format!("{}{}<w:p/>", frame("right", "TITLE"), frame("center", "3"));
    let header = units_story(&source, false, true, false);
    // Both frames float from the empty paragraph: one unit, two anchors.
    assert_eq!(header.len(), 1);
    let Unit::Para(p) = &header[0] else {
        panic!("anchor paragraph")
    };
    assert_eq!(p.lines[0].anchors.len(), 2);
}

#[test]
fn footer_text_frames_preserve_nested_anchors_and_require_following_anchor() {
    let frame = r#"<w:p><w:pPr><w:framePr w:w="4000" w:vAnchor="text" w:y="-120"/></w:pPr><w:r><w:t>FIRST</w:t><w:br/><w:drawing><wp:anchor xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"><wp:extent cx="914400" cy="914400"/></wp:anchor></w:drawing><w:t>SECOND</w:t></w:r></w:p>"#;
    let source = format!("{frame}<w:p/>");
    let footer = units_story(&source, false, true, true);
    assert_eq!(footer.len(), 1);
    let Unit::Para(p) = &footer[0] else {
        panic!("anchor paragraph")
    };
    let a = &p.lines[0].anchors[0];
    assert_eq!(a.y, Ok(-6.0));
    assert_eq!(a.frame_children.len(), 1);
    assert!(a.frame_children[0].line_y > a.frame_children[0].paragraph_y);
    assert!(
        a.text_items
            .iter()
            .any(|i| matches!(i, Item::Text(t) if t.text.contains("SECOND")))
    );
    // Header frames anchored to the text float from the next paragraph too.
    let header = units_story(&source, false, true, false);
    assert_eq!(header.len(), 1);
    let unanchored = units_story(frame, false, true, true);
    assert_eq!(unanchored.len(), 1);
    let Unit::Para(p) = &unanchored[0] else {
        panic!("flow paragraph")
    };
    assert!(
        p.lines
            .iter()
            .flat_map(|l| &l.items)
            .any(|i| matches!(i, Item::Text(t) if t.text.contains("FIRST")))
    );
}

#[test]
fn inherited_frame_is_not_an_ordinary_footer_anchor() {
    let source = r#"<w:p><w:pPr><w:framePr w:vAnchor="text" w:y="1"/></w:pPr></w:p><w:p/>"#;
    let mut opened =
        crate::office::docx::extract::open(docx(source, None, None), &Default::default()).unwrap();
    let blocks = &mut opened.sections[0].blocks;
    let inherited = footer_frame_of(&blocks[0]).unwrap().clone();
    assert!(ordinary_frame_anchor(&blocks[1]));
    let Block::Para(p) = &mut blocks[1] else {
        panic!("paragraph")
    };
    Rc::make_mut(&mut p.style).resolved_frame = Some(inherited);
    assert!(p.style.frame.is_none());
    assert!(!ordinary_frame_anchor(&blocks[1]));
    assert!(footer_frame_of(&blocks[1]).is_some());
}

#[test]
fn terminal_cell_paragraph_spacing_participates_in_row_split() {
    for empty in [false, true] {
        let first = r#"<w:p><w:pPr><w:spacing w:before="0" w:after="0" w:line="240" w:lineRule="exact"/></w:pPr><w:r><w:t>First</w:t></w:r></w:p>"#;
        let text = if empty { "" } else { "Last" };
        let last = format!(
            r#"<w:p><w:pPr><w:spacing w:before="0" w:after="600" w:line="240" w:lineRule="exact"/></w:pPr><w:r><w:t>{text}</w:t></w:r></w:p>"#
        );
        let body = format!(
            r#"<w:tbl><w:tblPr><w:tblCellMar><w:top w:w="0" w:type="dxa"/><w:bottom w:w="0" w:type="dxa"/></w:tblCellMar></w:tblPr><w:tblGrid><w:gridCol w:w="3000"/></w:tblGrid><w:tr><w:tc>{first}{first}{last}</w:tc></w:tr></w:tbl>"#
        );
        let us = units(&body);
        let r = rows(&us)[0];
        assert_eq!(r.height, 66.0);
        assert_eq!(r.cells[0].slices.last().unwrap().h, 12.0);
        let shared = stack(&units(&last));
        assert_eq!(shared.0.last().unwrap().h, 12.0);
        let (a, b) =
            split_row(r, 52.0, true).expect("trailing spacing travels with final paragraph");
        assert_eq!(a.cells[0].slices.len(), 2);
        assert_eq!(b.cells[0].slices.len(), 1);
        assert_eq!(a.height, 24.0);
        assert_eq!(b.height, 42.0);
        assert_eq!(a.height + b.height, r.height);
        assert_eq!(a.cells[0].terminal_after, 0.0);
        assert_eq!(b.cells[0].terminal_after, 30.0);
        assert!(split_row(&b, 41.0, false).is_none());
        let mut with_note = r.clone();
        with_note.cells[0].slices.last_mut().unwrap().notes.push(7);
        let (first, last) = split_row(&with_note, 52.0, true).unwrap();
        assert!(first.notes().is_empty());
        assert_eq!(last.notes(), vec![7]);
        let (one, two) = split_row(&first, 13.0, false).unwrap();
        assert!(one.notes().is_empty());
        assert!(two.notes().is_empty());
        assert_eq!(one.height + two.height + last.height, r.height);
        let minimum = units(
            &body
                .replace(
                    "<w:tr>",
                    "<w:tr><w:trPr><w:trHeight w:val=\"1320\" w:hRule=\"atLeast\"/></w:trPr>",
                )
                .replace("w:after=\"600\"", "w:after=\"0\""),
        );
        assert!(split_row(rows(&minimum)[0], 52.0, true).is_none());
    }
}

fn negative_frame_fixture(
    width: &str,
    indent: &str,
    align: &str,
    content: &str,
    extra: &str,
) -> String {
    format!(
        r#"<w:p><w:pPr><w:framePr w:wrap="around" w:vAnchor="text" w:hAnchor="margin" w:xAlign="left" w:y="1" {width}/><w:ind {indent}/><w:jc w:val="{align}"/>{extra}</w:pPr><w:r><w:rPr><w:sz w:val="24"/></w:rPr>{content}</w:r></w:p><w:p/>"#
    )
}

fn first_frame(units: &[Unit]) -> &crate::office::docx::model::Anchored {
    units
        .iter()
        .filter_map(|u| match u {
            Unit::Para(p) => Some(p),
            _ => None,
        })
        .flat_map(|p| &p.lines)
        .flat_map(|l| &l.anchors)
        .next()
        .unwrap()
}

#[test]
fn text_frame_rejects_only_fully_outside_lines_and_keeps_advance() {
    for width in ["", r#"w:w="2400""#] {
        for (indent, align) in [(r#"w:left="-475""#, "left"), (r#"w:right="-475""#, "right")] {
            for text in [
                r#"<w:t>ii</w:t><w:br/><w:t>AAAA</w:t>"#,
                r#"<w:t>AAAA</w:t><w:br/><w:t>ii</w:t>"#,
            ] {
                let body = negative_frame_fixture(width, indent, align, text, "");
                let actual = units_story(&body, false, true, true);
                let frame = first_frame(&actual);
                let painted: String = frame
                    .text_items
                    .iter()
                    .filter_map(|i| match i {
                        Item::Text(r) => Some(r.text.as_str()),
                        _ => None,
                    })
                    .collect();
                assert_eq!(painted, "AAAA");
                let control = units_story(
                    &negative_frame_fixture(width, "", align, text, ""),
                    false,
                    true,
                    true,
                );
                assert!((frame.pic.h - first_frame(&control).pic.h).abs() < 0.001);
                assert!(
                    frame
                        .text_items
                        .iter()
                        .any(|i| matches!(i, Item::Text(r) if r.width > 23.75))
                );
            }
        }
    }
}

#[test]
fn negative_indent_body_and_decorated_frame_text_are_preserved() {
    let body = units(r#"<w:p><w:pPr><w:ind w:left="-475"/></w:pPr><w:r><w:t>ii</w:t></w:r></w:p>"#);
    assert!(body.iter().any(|u| matches!(u, Unit::Para(p) if p.lines.iter().any(|l| l.items.iter().any(|i| matches!(i, Item::Text(r) if r.text == "ii"))))));
    let decorated = units_story(
        &negative_frame_fixture(
            r#"w:w="2400""#,
            r#"w:left="-475""#,
            "left",
            "<w:t>ii</w:t>",
            r#"<w:shd w:fill="FFFF00"/>"#,
        ),
        false,
        true,
        true,
    );
    assert!(
        first_frame(&decorated)
            .text_items
            .iter()
            .any(|i| matches!(i, Item::Text(r) if r.text == "ii"))
    );
}

#[test]
fn frame_line_bounds_preserve_mixed_fonts_overhang_and_uncertain_objects() {
    let fonts = crate::office::pptx::fonts::FontSystem::new(&[], true, &Default::default());
    let env = TextEnv {
        fonts: &fonts,
        default_tab: 36.0,
        fields: Default::default(),
        grid: None,
        char_grid: Default::default(),
        expand_shift_return: false,
        mark_in_last_line: true,
        html_spacing: true,
        grow_autofit: false,
        in_cell: Default::default(),
        in_header: Default::default(),
        grid_in_table: false,
        no_leading: false,
        compat_mode: 15,
    };
    let flow = Flow::new(&env, 15);
    let body = r#"<w:p><w:r><w:rPr><w:rFonts w:ascii="Arial" w:hAnsi="Arial"/><w:sz w:val="24"/></w:rPr><w:t>M</w:t></w:r><w:r><w:rPr><w:rFonts w:ascii="Times New Roman" w:hAnsi="Times New Roman"/><w:sz w:val="36"/></w:rPr><w:t>W</w:t></w:r></w:p>"#;
    let opened =
        crate::office::docx::extract::open(docx(body, None, None), &Default::default()).unwrap();
    let measured = flow.units(&opened.sections[0].blocks, 120.0, false);
    let Unit::Para(para) = &measured[0] else {
        panic!("paragraph expected")
    };
    let mut line = para.lines[0].clone();
    assert_eq!(
        line.items.len(),
        2,
        "distinct family/size runs should remain distinct"
    );
    for item in &mut line.items {
        let Item::Text(run) = item else {
            panic!("text expected")
        };
        run.transform.tx = -1000.0;
    }
    assert!(flow.frame_line_outside(&line, 120.0));
    let mut crossing = line.clone();
    let Item::Text(run) = &mut crossing.items[1] else {
        unreachable!()
    };
    run.transform.tx = 1.0;
    assert!(!flow.frame_line_outside(&crossing, 120.0));

    // Ink overhang keeps a line even when its recorded advance stays outside.
    let mut overhang = line.clone();
    overhang.items.truncate(1);
    let Item::Text(run) = &mut overhang.items[0] else {
        unreachable!()
    };
    let face = fonts.face(run.face).unwrap();
    let scale = run.size / face.metrics.units_per_em as f64;
    let glyph = run.glyphs[0];
    let right =
        glyph.x + fonts.outline(run.face, glyph.id).unwrap().bounds().right() as f64 * scale;
    assert!(right > 0.0);
    run.width = 0.0;
    run.transform.tx = (-right * 0.5) as f32;
    assert!(!flow.frame_line_outside(&overhang, 120.0));

    for invalid in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let mut unknown = line.clone();
        let Item::Text(run) = &mut unknown.items[1] else {
            unreachable!()
        };
        run.transform.tx = invalid as f32;
        assert!(!flow.frame_line_outside(&unknown, 120.0));
        let mut unknown = line.clone();
        let Item::Text(run) = &mut unknown.items[1] else {
            unreachable!()
        };
        run.glyphs[0].x = invalid;
        assert!(!flow.frame_line_outside(&unknown, 120.0));
    }

    let mut missing = line.clone();
    let Item::Text(run) = &mut missing.items[0] else {
        unreachable!()
    };
    run.glyphs.clear();
    assert!(!flow.frame_line_outside(&missing, 120.0));

    let mut missing_outline = line.clone();
    let Item::Text(run) = &mut missing_outline.items[0] else {
        unreachable!()
    };
    run.glyphs[0].id = u16::MAX;
    assert!(!flow.frame_line_outside(&missing_outline, 120.0));
    assert!(!flow.frame_line_outside(&line, 0.0));
    assert!(!flow.frame_line_outside(&line, f64::NAN));
    let mut on_edge = line.clone();
    for item in &mut on_edge.items {
        let Item::Text(run) = item else {
            unreachable!()
        };
        run.transform.tx = 120.0;
    }
    assert!(!flow.frame_line_outside(&on_edge, 120.0));

    let mut with_image = line.clone();
    with_image
        .items
        .push(Item::Image(crate::office::pptx::scene::ImageItem {
            image: crate::office::pptx::scene::ImageRef {
                part: "control.png".into(),
                effects: vec![],
            },
            transform: tiny_skia::Transform::identity(),
            crop: (0.0, 0.0, 0.0, 0.0),
            alpha: 1.0,
            clip: None,
            shape: None,
            alt_text: None,
        }));
    assert!(!flow.frame_line_outside(&with_image, 120.0));
    let mut clipped = line.clone();
    clipped.items.push(Item::PopClip);
    assert!(!flow.frame_line_outside(&clipped, 120.0));

    let mut units = vec![Unit::Para(para.clone())];
    let Unit::Para(p) = &mut units[0] else {
        unreachable!()
    };
    p.lines = vec![line];
    p.lines[0].notes.push(123);
    let before_height = p.lines[0].height;
    assert!(!flow.discard_outside_frame_lines(&mut units, 120.0));
    let Unit::Para(p) = &units[0] else {
        unreachable!()
    };
    assert_eq!(p.lines[0].items.len(), 2);
    assert_eq!(p.lines[0].height, before_height);
}

#[test]
fn culled_terminal_frame_line_preserves_spacing_and_cell_terminal_after() {
    let fonts = crate::office::pptx::fonts::FontSystem::new(&[], true, &Default::default());
    let env = TextEnv {
        fonts: &fonts,
        default_tab: 36.0,
        fields: Default::default(),
        grid: None,
        char_grid: Default::default(),
        expand_shift_return: false,
        mark_in_last_line: true,
        html_spacing: true,
        grow_autofit: false,
        in_cell: Default::default(),
        in_header: Default::default(),
        grid_in_table: false,
        no_leading: false,
        compat_mode: 15,
    };
    let flow = Flow::new(&env, 15);

    let body = r#"<w:p><w:pPr><w:ind w:left="-475"/><w:spacing w:before="60" w:after="380" w:line="240" w:lineRule="exact"/></w:pPr><w:r><w:rPr><w:sz w:val="24"/></w:rPr><w:t>AAAA</w:t><w:br/><w:t>ii</w:t></w:r></w:p>"#;
    let opened =
        crate::office::docx::extract::open(docx(body, None, None), &Default::default()).unwrap();
    let mut measured = flow.units(&opened.sections[0].blocks, 120.0, false);
    let Unit::Para(para) = &mut measured[0] else {
        panic!("paragraph expected")
    };
    assert_eq!(para.lines.len(), 2);
    para.pad_bottom = 7.0;
    para.before = 3.0;
    para.after = 19.0;
    let before = stack(&measured);
    assert!(flow.discard_outside_frame_lines(&mut measured, 120.0));
    let Unit::Para(para) = &measured[0] else {
        unreachable!()
    };
    assert!(!para.lines[0].items.is_empty());
    assert!(para.lines[1].items.is_empty());
    assert_eq!((para.before, para.pad_bottom, para.after), (3.0, 7.0, 19.0));
    let after = stack(&measured);
    assert_eq!(before.1, after.1);
    assert_eq!(after.1, 3.0 + 24.0 + 7.0 + 19.0);
    assert_eq!(before.0.len(), after.0.len());
    for (a, b) in before.0.iter().zip(&after.0) {
        assert_eq!((a.y, a.h), (b.y, b.h));
    }
    assert!(after.0.last().unwrap().items.is_empty());

    let mut table_units = units(
        r#"<w:tbl><w:tblPr><w:tblCellMar><w:top w:w="0"/><w:bottom w:w="0"/></w:tblCellMar></w:tblPr><w:tblGrid><w:gridCol w:w="3000"/></w:tblGrid><w:tr><w:tc><w:p><w:pPr><w:spacing w:before="0" w:after="600" w:line="240" w:lineRule="exact"/></w:pPr><w:r><w:t>ii</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#,
    );
    let Unit::Row(row) = &table_units[0] else {
        panic!("row expected")
    };
    assert_eq!(row.cells[0].terminal_after, 30.0);
    let initial_height = row.height;
    let initial_terminal = row.cells[0].terminal_after;
    let initial_slice_geometry: Vec<_> = row.cells[0].slices.iter().map(|s| (s.y, s.h)).collect();
    assert!(!flow.discard_outside_frame_lines(&mut table_units, 120.0));
    let Unit::Row(row) = &table_units[0] else {
        unreachable!()
    };
    assert_eq!(row.height, initial_height);
    assert_eq!(row.cells[0].terminal_after, initial_terminal);
    assert_eq!(
        row.cells[0]
            .slices
            .iter()
            .map(|s| (s.y, s.h))
            .collect::<Vec<_>>(),
        initial_slice_geometry
    );
}

#[test]
fn nested_row_split_keeps_late_anchor_and_reference_with_source_line() {
    let anchor = r#"<w:drawing><wp:anchor xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"><wp:positionH relativeFrom="column"><wp:posOffset>0</wp:posOffset></wp:positionH><wp:positionV relativeFrom="paragraph"><wp:posOffset>0</wp:posOffset></wp:positionV><wp:extent cx="127000" cy="127000"/><wp:wrapNone/></wp:anchor></w:drawing><w:footnoteReference w:id="7"/>"#;
    let paragraphs = (0..8).map(|i| format!(r#"<w:p><w:pPr><w:widowControl w:val="0"/><w:spacing w:before="0" w:after="0" w:line="240" w:lineRule="exact"/></w:pPr><w:r><w:t>NEST{i}</w:t>{}</w:r></w:p>"#, if i == 6 { anchor } else { "" })).collect::<String>();
    let nested = table(&format!("<w:tr><w:tc>{paragraphs}</w:tc></w:tr>"));
    let body = table(&format!("<w:tr><w:tc>{nested}<w:p/></w:tc></w:tr>"));
    let source = units(&body);
    let row = rows(&source)[0];
    assert_eq!(row_anchors(row, 0.0, 0.0).len(), 1);
    let (top, rest) = split_row(row, 50.0, false).unwrap();
    assert!(row_anchors(&top, 0.0, 0.0).is_empty());
    assert!(top.notes().is_empty());
    assert_eq!(row_anchors(&rest, 0.0, 0.0).len(), 1);
    assert_eq!(rest.notes(), vec![7]);
    let text = |r: &RowUnit| {
        row_items(r, 0.0, 0.0)
            .iter()
            .filter_map(|i| match i {
                Item::Text(t) => Some(t.text.clone()),
                _ => None,
            })
            .collect::<String>()
    };
    assert!(!text(&top).contains("NEST6"));
    assert!(text(&rest).contains("NEST6"));
}

#[test]
fn nested_row_provenance_preserves_edges_spacing_and_atomic_rows() {
    let paras = (0..6).map(|i| format!(r#"<w:p><w:pPr><w:widowControl w:val="0"/><w:spacing w:before="0" w:after="{}" w:line="240" w:lineRule="exact"/></w:pPr><w:r><w:t>PART{i}</w:t></w:r></w:p>"#, if i == 5 { 180 } else { 0 })).collect::<String>();
    let source = units(&table(&format!("<w:tr><w:tc>{paras}</w:tc></w:tr>")));
    let mut nested = rows(&source)[0].clone();
    nested.outer_before = 2.0;
    nested.outer_after = 3.0;
    nested.cells[0].margins[0] = 4.0;
    nested.cells[0].margins[2] = 6.0;
    nested.height += 10.0;
    let slice = nested_row_slice(Rc::new(nested.clone()), 7.0);
    assert_eq!(slice.h, nested.flow_height());
    let (first, last) = split_slice(&slice, 55.0, false, 200.0).unwrap();
    let a = first.nested_row.as_ref().unwrap();
    let b = last.nested_row.as_ref().unwrap();
    assert_eq!((a.outer_before, a.outer_after), (2.0, 0.0));
    assert_eq!((b.outer_before, b.outer_after), (0.0, 3.0));
    assert_eq!(a.cells[0].terminal_after, 0.0);
    assert_eq!(b.cells[0].terminal_after, 9.0);
    assert_eq!(last.y, first.y + first.h);
    assert_eq!(a.cells[0].slices.len() + b.cells[0].slices.len(), 6);
    for exact in [false, true] {
        let mut atomic = nested.clone();
        atomic.cant_split = !exact;
        atomic.exact_height = exact;
        assert!(split_slice(&nested_row_slice(Rc::new(atomic), 7.0), 55.0, false, 200.0).is_none());
    }
    let tall_line = units(
        r#"<w:p><w:pPr><w:widowControl w:val="0"/><w:spacing w:line="1200" w:lineRule="exact"/></w:pPr><w:r><w:t>ATOMIC LINE</w:t></w:r></w:p>"#,
    );
    let line = stack(&tall_line).0.remove(0);
    assert!(line.nested_row.is_none());
    assert!(split_slice(&line, 30.0, false, 200.0).is_none());
}

#[test]
fn stacked_nested_positioning_metadata_keeps_recursive_content_splittable() {
    let ps = (0..30).map(|i| format!(r#"<w:p><w:pPr><w:widowControl w:val="0"/><w:spacing w:before="0" w:after="0" w:line="240" w:lineRule="exact"/></w:pPr><w:r><w:t>LOCAL{i:03}</w:t></w:r></w:p>"#)).collect::<String>();
    let nested = table(&format!("<w:tr><w:tc>{ps}</w:tc></w:tr>")).replace(
        "<w:tblPr>",
        r#"<w:tblPr><w:tblpPr w:vertAnchor="text" w:tblpY="0"/>"#,
    );
    let source = units(&table(&format!("<w:tr><w:tc>{nested}<w:p/></w:tc></w:tr>")));
    let mut remaining = rows(&source)[0].clone();
    let mut text = String::new();
    for _ in 0..10 {
        let (top, rest) = if remaining.flow_height() <= 100.0 {
            (remaining.clone(), None)
        } else {
            let (a, b) = split_row(&remaining, 100.0, false).expect("stacked source must split");
            (a, Some(b))
        };
        for item in row_items(&top, 0.0, 0.0) {
            if let Item::Text(t) = item {
                text.push_str(&t.text);
            }
        }
        let Some(rest) = rest else {
            break;
        };
        remaining = rest;
    }
    for i in 0..30 {
        assert_eq!(text.matches(&format!("LOCAL{i:03}")).count(), 1);
    }
}

#[test]
fn occupied_slice_bottom_survives_both_row_fragments_without_moving_terminal_spacing() {
    let ps = (0..5)
        .map(|i| format!("<w:p><w:r><w:t>LINE{i}</w:t></w:r></w:p>"))
        .collect::<String>();
    let source = units(&format!(
        "<w:tbl><w:tblGrid><w:gridCol w:w=\"3000\"/></w:tblGrid><w:tr><w:tc>{ps}</w:tc></w:tr></w:tbl>"
    ));
    let mut row = rows(&source)[0].clone();
    row.height = 200.0;
    row.outer_before = 0.0;
    row.outer_after = 0.0;
    let cell = &mut row.cells[0];
    cell.margins = [0.0; 4];
    cell.terminal_after = 30.0;
    for (slice, (y, h)) in cell.slices.iter_mut().zip([
        (0.0, 80.0),
        (12.0, 12.0),
        (24.0, 12.0),
        (90.0, 100.0),
        (102.0, 12.0),
    ]) {
        slice.y = y;
        slice.h = h;
        slice.widow = None;
    }
    let (top, rest) = split_row(&row, 85.0, false).unwrap();
    assert_eq!(top.cells[0].slices.len(), 3);
    assert_eq!(top.cells[0].content_h, 80.0);
    assert_eq!(rest.cells[0].content_h, 100.0);
    assert_eq!(top.cells[0].terminal_after, 0.0);
    assert_eq!(rest.cells[0].terminal_after, 30.0);
}

fn growing_shape_body(height: f64, wrap: &str, fit: &str, count: usize) -> String {
    let text = (0..count)
        .map(|i| format!(r#"<w:p><w:pPr><w:spacing w:before="0" w:after="0" w:line="240" w:lineRule="exact"/></w:pPr><w:r><w:rPr><w:sz w:val="20"/></w:rPr><w:t>BOX{i:04}</w:t></w:r></w:p>"#))
        .collect::<String>();
    let emu = (height * 12700.0).round() as usize;
    format!(
        r#"<w:p><w:r><w:drawing><wp:anchor xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape" distT="0" distB="0" distL="0" distR="0"><wp:positionH relativeFrom="column"><wp:posOffset>0</wp:posOffset></wp:positionH><wp:positionV relativeFrom="paragraph"><wp:posOffset>0</wp:posOffset></wp:positionV><wp:extent cx="2286000" cy="{emu}"/><wp:wrapTopAndBottom/><a:graphic><a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"><wps:wsp><wps:spPr><a:prstGeom prst="rect"/><a:solidFill><a:srgbClr val="F3CFD0"/></a:solidFill></wps:spPr><wps:txbx><w:txbxContent>{text}</w:txbxContent></wps:txbx><wps:bodyPr {wrap} lIns="91440" rIns="91440" tIns="45720" bIns="45720">{fit}</wps:bodyPr></wps:wsp></a:graphicData></a:graphic></wp:anchor></w:drawing></w:r></w:p><w:p><w:r><w:t>FOLLOWING</w:t></w:r></w:p>"#
    )
}

fn shape_anchor(units: &[Unit]) -> &crate::office::docx::model::Anchored {
    let Unit::Para(p) = &units[0] else {
        panic!("shape paragraph")
    };
    &p.lines[0].anchors[0]
}

#[test]
fn wrapped_shape_autofit_grows_geometry_and_body_exclusion() {
    for wrap in [r#"wrap="square""#, r#"wrap="none""#, ""] {
        let body = growing_shape_body(60.0, wrap, "<a:spAutoFit/>", 20);
        let us = units(&body);
        let anchor = shape_anchor(&us);
        assert!(
            (anchor.pic.h - 247.2).abs() < 0.01,
            "height={}",
            anchor.pic.h
        );
        let spec = anchor.pic.shape.as_ref().unwrap();
        assert!((spec.texts[0].rect.h - 247.2).abs() < 0.01);
        let bottom = spec
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Path(path) => Some(path.path.bounds().bottom() as f64),
                _ => None,
            })
            .fold(0.0, f64::max);
        assert!((bottom - 247.2).abs() < 0.01, "fill bottom={bottom}");
        let pages = crate::office::docx::testutil::pages(docx(&body, None, None));
        let first = &pages[0];
        let following = first
            .items
            .iter()
            .find_map(|item| match item {
                Item::Text(run) if run.text == "FOLLOWING" => Some(run.transform.ty as f64),
                _ => None,
            })
            .unwrap();
        let last_box = first
            .items
            .iter()
            .find_map(|item| match item {
                Item::Text(run) if run.text == "BOX0019" => {
                    Some(run.transform.ty as f64 + run.size)
                }
                _ => None,
            })
            .unwrap();
        assert!(following >= last_box, "body={following}, box={last_box}");
    }
}

#[test]
fn shape_autofit_preserves_fixed_and_shrink_policy() {
    for (height, wrap, fit, expected) in [
        (60.0, r#"wrap="square""#, "<a:noAutofit/>", 60.0),
        (60.0, r#"wrap="none""#, "<a:noAutofit/>", 60.0),
        (300.0, r#"wrap="square""#, "<a:spAutoFit/>", 247.2),
        (246.7, r#"wrap="square""#, "<a:spAutoFit/>", 247.2),
    ] {
        let us = units(&growing_shape_body(height, wrap, fit, 20));
        assert!((shape_anchor(&us).pic.h - expected).abs() < 0.01);
    }
}

#[test]
fn wrapped_shape_autofit_preserves_source_across_page_and_cell_placement() {
    let section = r#"<w:sectPr><w:pgSz w:w="6000" w:h="8000"/><w:pgMar w:top="600" w:bottom="600" w:left="600" w:right="600"/></w:sectPr>"#;
    for (context, count, wrap) in [("retry", 20), ("oversized", 40), ("cell", 20), ("note", 20)]
        .into_iter()
        .flat_map(|(context, count)| {
            [r#"wrap="square""#, r#"wrap="none""#].map(|wrap| (context, count, wrap))
        })
    {
        let shape = growing_shape_body(60.0, wrap, "<a:spAutoFit/>", count);
        let body = match context {
            "retry" => format!(
                r#"<w:p><w:pPr><w:spacing w:line="3600" w:lineRule="exact"/></w:pPr><w:r><w:t>PREFIX</w:t></w:r></w:p>{shape}"#
            ),
            "cell" => format!(
                r#"<w:tbl><w:tblGrid><w:gridCol w:w="4800"/></w:tblGrid><w:tr><w:tc>{shape}</w:tc></w:tr></w:tbl>"#
            ),
            "note" => format!(
                r#"<w:p><w:r><w:t>REFERENCE</w:t><w:footnoteReference w:id="1"/></w:r></w:p>{shape}"#
            ),
            _ => shape,
        };
        let mut bytes = docx(&(body + section), None, None);
        if context == "note" {
            bytes = crate::office::docx::testutil::with_related_part(
                bytes,
                "footnotes",
                "footnotes.xml",
                &format!(
                    r#"<w:footnotes xmlns:w="{}"><w:footnote w:id="1"><w:p><w:r><w:t>NOTE</w:t></w:r></w:p></w:footnote></w:footnotes>"#,
                    crate::office::docx::testutil::W
                ),
                false,
            );
        }
        let pages = crate::office::docx::testutil::pages(bytes);
        assert!(pages.len() <= 4, "{context}: {} pages", pages.len());
        let text = pages
            .iter()
            .flat_map(|p| p.items.iter())
            .filter_map(|item| match item {
                Item::Text(run) => Some(run),
                _ => None,
            })
            .collect::<Vec<_>>();
        for i in 0..count {
            assert_eq!(
                text.iter()
                    .filter(|r| r.text == format!("BOX{i:04}"))
                    .count(),
                1,
                "{context}: BOX{i:04}"
            );
        }
        assert_eq!(
            text.iter().filter(|r| r.text == "FOLLOWING").count(),
            1,
            "{context}"
        );
        let first = text.iter().find(|r| r.text == "BOX0000").unwrap();
        assert!(first.transform.ty >= 30.0, "{context}: owner above page");
        if context == "retry" {
            assert!(
                !pages[0]
                    .items
                    .iter()
                    .any(|item| matches!(item, Item::Text(run) if run.text == "BOX0000"))
            );
        }
        if context == "note" {
            assert_eq!(text.iter().filter(|r| r.text == "NOTE").count(), 1);
        }
    }
}

#[test]
fn wrapped_shape_autofit_preserves_transformed_and_fitted_geometry() {
    let fonts = crate::office::pptx::fonts::FontSystem::new(&[], true, &Default::default());
    let env = TextEnv {
        fonts: &fonts,
        default_tab: 36.0,
        fields: Default::default(),
        grid: None,
        char_grid: Default::default(),
        expand_shift_return: false,
        mark_in_last_line: true,
        html_spacing: true,
        grow_autofit: false,
        in_cell: std::cell::Cell::new(false),
        in_header: std::cell::Cell::new(false),
        grid_in_table: false,
        no_leading: false,
        compat_mode: 15,
    };
    let flow = Flow::new(&env, 15);
    let us = units(&growing_shape_body(60.0, "", "<a:noAutofit/>", 20));
    let spec = shape_anchor(&us).pic.shape.as_ref().unwrap();
    for mode in ["fit", "vertical", "transformed", "partial", "multiple"] {
        let mut candidate = (**spec).clone();
        let tb = &mut candidate.texts[0];
        tb.grow = true;
        match mode {
            "fit" => tb.fit = true,
            "vertical" => tb.vert = Some("vert".into()),
            "transformed" => tb.transform = Some(tiny_skia::Transform::from_rotate(30.0)),
            "partial" => tb.rect.w -= 10.0,
            "multiple" => candidate.texts.push(candidate.texts[0].clone()),
            _ => unreachable!(),
        }
        assert!(flow.refit(&candidate, 180.0, 60.0).is_none(), "{mode}");
    }
}

#[test]
fn nowrap_autofit_measures_explicit_breaks_without_wrapping_long_lines() {
    for (content, expected_lines) in [
        ("SHORT<w:br/>SECOND<w:br/>THIRD".to_owned(), 3),
        ("LONG WORDS ".repeat(80), 1),
    ] {
        let mut body = growing_shape_body(12.0, r#"wrap="none""#, "<a:spAutoFit/>", 1);
        let text = content.replace("<w:br/>", "</w:t><w:br/><w:t>");
        body = body.replace("BOX0000", &text);
        let us = units(&body);
        let anchor = shape_anchor(&us);
        let expected_height = expected_lines as f64 * 12.0 + 7.2;
        assert!((anchor.pic.h - expected_height).abs() < 0.01);
        let lines = anchor
            .text_items
            .iter()
            .filter_map(|item| match item {
                Item::Text(run) => Some(run.transform.ty.to_bits()),
                _ => None,
            })
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(lines.len(), expected_lines);
        if expected_lines == 1 {
            assert_eq!(anchor.pic.w, 180.0);
            let width = anchor
                .text_items
                .iter()
                .filter_map(|item| match item {
                    Item::Text(run) => Some(run.width),
                    _ => None,
                })
                .fold(0.0, f64::max);
            assert!(width > 180.0, "explicit no-wrap retains the long line");
        }
    }
}

#[test]
fn continued_row_widow_adjustment_stops_at_zero() {
    let content = r#"<w:p><w:pPr><w:spacing w:line="280" w:lineRule="exact"/></w:pPr><w:r><w:t>FIRST</w:t><w:br/><w:t>SECOND</w:t></w:r></w:p>"#;
    let laid = units(&table(&format!("<w:tr><w:tc>{content}</w:tc></w:tr>")));
    let mut row = rows(&laid)[0].clone();
    assert_eq!(row.cells[0].slices.len(), 2);
    row.cells[0].slices[0].widow = Some((1, 3));
    row.cells[0].slices[1].widow = Some((2, 3));
    let room = row.cells[0].slices[0].y
        + row.cells[0].slices[0].h
        + row.cells[0].margins[0]
        + row.cells[0].margins[2];
    assert!(split_row(&row, room, false).is_none());
    let (first, rest) = split_row_first_line(&row, room, None, room).unwrap();
    assert_eq!(first.cells[0].slices.len(), 1);
    assert_eq!(rest.cells[0].slices.len(), 1);
}

#[test]
fn atomic_row_edges_preserve_authored_minimum_debt() {
    for minimum in [0, 290, 800] {
        let source = format!(
            r#"<w:tbl><w:tblPr><w:tblBorders><w:top w:val="single" w:sz="8"/><w:bottom w:val="single" w:sz="8"/></w:tblBorders><w:tblCellMar><w:top w:w="0" w:type="dxa"/><w:bottom w:w="0" w:type="dxa"/></w:tblCellMar></w:tblPr><w:tblGrid><w:gridCol w:w="3000"/></w:tblGrid><w:tr><w:trPr><w:trHeight w:val="{minimum}" w:hRule="atLeast"/></w:trPr><w:tc><w:p><w:pPr><w:spacing w:before="0" w:after="0" w:line="280" w:lineRule="exact"/></w:pPr><w:r><w:t>ATOM</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#
        );
        let laid = units(&source);
        let row = rows(&laid)[0];
        let (top, rest) = split_row_first_line(row, 12.0, None, 12.0).unwrap();
        assert_eq!(top.cells[0].slices.len(), 1);
        assert!(rest.cells[0].slices.is_empty());
        assert!((top.height + rest.height - row.height).abs() < 0.01);
        assert_eq!(rest.height > 0.01, minimum > 0);
    }
}

#[test]
fn percentage_body_grid_resolves_saved_width_in_both_directions() {
    for layout in ["fixed", "autofit"] {
        for (value, expected) in [("5000", 468.0), ("6250", 585.0), ("7500", 702.0)] {
            let source = format!(
                r#"<w:tbl><w:tblPr><w:tblW w:type="pct" w:w="{value}"/><w:tblLayout w:type="{layout}"/></w:tblPr><w:tblGrid><w:gridCol w:w="2000"/><w:gridCol w:w="4000"/><w:gridCol w:w="6000"/></w:tblGrid><w:tr><w:tc><w:p><w:r><w:t>FIRST</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>SECOND</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>THIRD</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#
            );
            let laid = units(&source);
            let row = rows(&laid)[0];
            for (cell, fraction) in row.cells.iter().zip([1.0 / 6.0, 2.0 / 6.0, 3.0 / 6.0]) {
                assert!(
                    (cell.w - expected * fraction).abs() < 0.01,
                    "{layout} {value}"
                );
            }
            let text: String = row
                .cells
                .iter()
                .flat_map(|cell| &cell.slices)
                .flat_map(|slice| &slice.items)
                .filter_map(|item| match item {
                    Item::Text(run) => Some(run.text.as_str()),
                    _ => None,
                })
                .collect();
            assert_eq!(text, "FIRSTSECONDTHIRD");
        }
    }
}

#[test]
fn percentage_grid_preserves_fixed_and_invalid_preferences() {
    for preference in [
        None,
        Some(0.0),
        Some(-1.0),
        Some(f64::NAN),
        Some(f64::INFINITY),
    ] {
        let mut grid = [100.0, 200.0, 300.0];
        resolve_percentage_grid(&mut grid, preference, 240.0);
        assert_eq!(grid, [100.0, 200.0, 300.0]);
    }
    let source = r#"<w:tbl><w:tblPr><w:tblW w:type="dxa" w:w="12000"/><w:tblLayout w:type="fixed"/></w:tblPr><w:tblGrid><w:gridCol w:w="12000"/></w:tblGrid><w:tr><w:tc><w:p><w:r><w:t>FIXED</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#;
    let laid = units(source);
    assert_eq!(rows(&laid)[0].cells[0].w, 600.0);
}

#[test]
fn autofit_inline_picture_redistributes_columns_but_fixed_preserves_preference() {
    for layout in ["fixed", "autofit"] {
        let source = format!(
            r#"<w:tbl><w:tblPr><w:tblW w:type="pct" w:w="2500"/><w:tblLayout w:type="{layout}"/><w:tblCellMar><w:left w:w="0" w:type="dxa"/><w:right w:w="0" w:type="dxa"/></w:tblCellMar></w:tblPr><w:tblGrid><w:gridCol w:w="4000"/><w:gridCol w:w="4000"/><w:gridCol w:w="4000"/></w:tblGrid><w:tr><w:tc><w:p><w:r><w:t>LEFT</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:drawing><wp:inline xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"><wp:extent cx="1524000" cy="254000"/></wp:inline></w:drawing></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>RIGHT</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#
        );
        let laid = units(&source);
        let row = rows(&laid)[0];
        assert!((row.cells.iter().map(|c| c.w).sum::<f64>() - 234.0).abs() < 0.001);
        if layout == "autofit" {
            assert!(row.cells[1].w >= 120.0 - 0.001);
        } else {
            assert!((row.cells[1].w - 78.0).abs() < 0.001);
        }
    }
}

#[test]
fn autofit_nested_missing_grid_and_row_exception_keep_inline_minimum() {
    for grid in [
        "",
        "<w:gridCol w:w=\"1600\"/>",
        "<w:gridCol w:w=\"1600\"/><w:gridCol w:w=\"1600\"/>",
    ] {
        for fixed_parent in [false, true] {
            let layout = if fixed_parent { "fixed" } else { "autofit" };
            let nested = format!(
                r#"<w:tbl><w:tblPr><w:tblW w:type="pct" w:w="5000"/><w:tblLayout w:type="{layout}"/><w:tblCellMar><w:left w:w="0" w:type="dxa"/><w:right w:w="0" w:type="dxa"/></w:tblCellMar></w:tblPr><w:tblGrid>{grid}</w:tblGrid><w:tr><w:tblPrEx><w:tblLayout w:type="autofit"/></w:tblPrEx><w:tc><w:p/></w:tc><w:tc><w:p><w:r><w:drawing><wp:inline xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"><wp:extent cx="1524000" cy="254000"/></wp:inline></w:drawing></w:r></w:p></w:tc></w:tr></w:tbl>"#
            );
            let source = format!(
                r#"<w:tbl><w:tblPr><w:tblW w:type="pct" w:w="2500"/><w:tblCellMar><w:left w:w="0" w:type="dxa"/><w:right w:w="0" w:type="dxa"/></w:tblCellMar></w:tblPr><w:tblGrid><w:gridCol w:w="1600"/><w:gridCol w:w="1600"/><w:gridCol w:w="1600"/></w:tblGrid><w:tr><w:tc><w:p><w:r><w:t>LEFT</w:t></w:r></w:p></w:tc><w:tc>{nested}<w:p/></w:tc><w:tc><w:p><w:r><w:t>RIGHT</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#
            );
            let laid = units(&source);
            let row = rows(&laid)[0];
            assert!(
                row.cells[1].w >= 120.0 - 0.001,
                "grid={grid} fixed={fixed_parent}"
            );
            assert!((row.cells.iter().map(|c| c.w).sum::<f64>() - 234.0).abs() < 0.01);
        }
    }
}

#[test]
fn autofit_rotated_cells_measure_transverse_lines_and_inline_height() {
    for direction in ["btLr", "tbRl"] {
        for (payload, minimum) in [
            (
                "<w:r><w:rPr><w:sz w:val=\"16\"/></w:rPr><w:t>ABCDEFGHIJKLMNOPQRSTUVWXYZ</w:t><w:br/><w:t>SECOND</w:t></w:r>",
                20.0,
            ),
            (
                r#"<w:r><w:drawing><wp:inline xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"><wp:extent cx="1524000" cy="1270000"/></wp:inline></w:drawing></w:r>"#,
                100.0,
            ),
        ] {
            for rule in ["atLeast", "exact"] {
                let source = format!(
                    r#"<w:tbl><w:tblPr><w:tblW w:type="pct" w:w="2500"/><w:tblCellMar><w:left w:w="0" w:type="dxa"/><w:right w:w="0" w:type="dxa"/><w:top w:w="0" w:type="dxa"/><w:bottom w:w="0" w:type="dxa"/></w:tblCellMar></w:tblPr><w:tblGrid><w:gridCol w:w="1600"/><w:gridCol w:w="1600"/><w:gridCol w:w="1600"/></w:tblGrid><w:tr><w:trPr><w:trHeight w:val="6000" w:hRule="{rule}"/></w:trPr><w:tc><w:p><w:r><w:t>LEFT</w:t></w:r></w:p></w:tc><w:tc><w:tcPr><w:textDirection w:val="{direction}"/></w:tcPr><w:p><w:pPr><w:spacing w:before="0" w:after="0" w:line="240" w:lineRule="auto"/></w:pPr>{payload}</w:p></w:tc><w:tc><w:p><w:r><w:t>RIGHT</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#
                );
                let laid = units(&source);
                let row = rows(&laid)[0];
                if minimum < 78.0 {
                    assert!((row.cells[1].w - 78.0).abs() < 0.01, "{direction} {rule}");
                } else {
                    assert!(row.cells[1].w >= minimum - 0.01, "{direction} {rule}");
                }
                assert!((row.height - 300.0).abs() < 0.01);
                assert!((row.cells.iter().map(|c| c.w).sum::<f64>() - 234.0).abs() < 0.01);
            }
        }
    }
}

#[test]
fn autofit_rotated_exact_row_uses_shared_bottom_margin_and_spacing_capacity() {
    for direction in ["btLr", "tbRl"] {
        let source = format!(
            r#"<w:tbl><w:tblPr><w:tblW w:type="pct" w:w="2500"/><w:tblCellSpacing w:type="dxa" w:w="60"/><w:tblCellMar><w:left w:w="0" w:type="dxa"/><w:right w:w="0" w:type="dxa"/><w:top w:w="0" w:type="dxa"/><w:bottom w:w="0" w:type="dxa"/></w:tblCellMar></w:tblPr><w:tblGrid><w:gridCol w:w="1600"/><w:gridCol w:w="1600"/><w:gridCol w:w="1600"/></w:tblGrid><w:tr><w:trPr><w:trHeight w:val="400" w:hRule="exact"/></w:trPr><w:tc><w:p><w:r><w:t>LEFT</w:t></w:r></w:p></w:tc><w:tc><w:tcPr><w:textDirection w:val="{direction}"/></w:tcPr><w:p><w:pPr><w:spacing w:before="0" w:after="0"/></w:pPr><w:r><w:rPr><w:sz w:val="16"/></w:rPr><w:t>ABCDEFGHIJKLMNOPQRSTUVWXYZABCDEFGHIJKLMNOPQRSTUVWXYZ</w:t></w:r></w:p></w:tc><w:tc><w:tcPr><w:tcMar><w:bottom w:w="2400" w:type="dxa"/></w:tcMar></w:tcPr><w:p><w:r><w:t>RIGHT</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#
        );
        let laid = units(&source);
        let row = rows(&laid)[0];
        // 20 pt exact + 120 pt bottom padding + twice the 3 pt spacing above and below.
        assert!((row.height - 152.0).abs() < 0.01, "{}", row.height);
        assert!(
            (row.cells[1].w - 78.0).abs() < 0.01,
            "{direction}: {}",
            row.cells[1].w
        );
    }
}

#[test]
fn upright_shape_text_preserves_shape_paint_and_source_once() {
    for rotation in [90, 270] {
        let make = |upright| {
            growing_shape_body(
                100.0,
                &format!(r#"wrap="square" upright="{upright}""#),
                "<a:noAutofit/>",
                4,
            )
            .replace(
                "<wps:spPr>",
                &format!(r#"<wps:spPr><a:xfrm rot="{}"/>"#, rotation * 60000),
            )
        };
        let source = crate::office::docx::testutil::pages(docx(&make(0), None, None));
        let upright = crate::office::docx::testutil::pages(docx(&make(1), None, None));
        let paths = |pages: &[crate::office::docx::paginate::OutPage]| {
            pages
                .iter()
                .flat_map(|p| &p.items)
                .filter_map(|item| match item {
                    Item::Path(p) => Some(format!("{p:?}")),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(paths(&source), paths(&upright));
        for index in 0..4 {
            let marker = format!("BOX{index:04}");
            for (pages, expected_upright) in [(&source, false), (&upright, true)] {
                let runs: Vec<_> = pages
                    .iter()
                    .flat_map(|p| &p.items)
                    .filter_map(|item| match item {
                        Item::Text(r) if r.text == marker => Some(r),
                        _ => None,
                    })
                    .collect();
                assert_eq!(runs.len(), 1, "{marker}");
                assert_eq!(runs[0].transform.ky.abs() < 0.001, expected_upright);
                assert_eq!(runs[0].transform.sx.abs() > 0.999, expected_upright);
            }
        }
    }
}

#[test]
fn upright_group_uses_ancestor_rotation_once_and_keeps_vertical_flow() {
    for (child_rotation, parent_rotation, flip, vertical, expected_vertical) in [
        (0, 90, "", "horz", false),
        (90, 270, "", "horz", false),
        (90, 0, r#"flipV="1""#, "horz", false),
        (0, 90, r#"flipH="1""#, "vert", true),
    ] {
        let body = growing_shape_body(100.0, &format!(r#"upright="1" vert="{vertical}""#), "<a:noAutofit/>", 1)
            .replace("<wps:spPr>", &format!(r#"<wps:spPr><a:xfrm rot="{}" {flip}><a:off x="0" y="0"/><a:ext cx="2286000" cy="1270000"/></a:xfrm>"#, child_rotation * 60000))
            .replace("<wps:wsp>", &format!(r#"<wpg:wgp xmlns:wpg="http://schemas.microsoft.com/office/word/2010/wordprocessingGroup"><wpg:grpSpPr><a:xfrm rot="{}"><a:off x="0" y="0"/><a:ext cx="2286000" cy="1270000"/><a:chOff x="0" y="0"/><a:chExt cx="2286000" cy="1270000"/></a:xfrm></wpg:grpSpPr><wps:wsp>"#, parent_rotation * 60000))
            .replace("</wps:wsp>", "</wps:wsp></wpg:wgp>")
            .replace("graphicData uri=\"http://schemas.microsoft.com/office/word/2010/wordprocessingShape\"", "graphicData uri=\"http://schemas.microsoft.com/office/word/2010/wordprocessingGroup\"");
        let pages = crate::office::docx::testutil::pages(docx(&body, None, None));
        let runs: Vec<_> = pages
            .iter()
            .flat_map(|p| &p.items)
            .filter_map(|i| match i {
                Item::Text(r) if r.text == "BOX0000" => Some(r),
                _ => None,
            })
            .collect();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].transform.ky.abs() > 0.999, expected_vertical);
        assert_eq!(runs[0].transform.sx.abs() < 0.001, expected_vertical);
    }
}

fn textbox_image_body(
    vertical: bool,
    anchored: bool,
    inline_outer: bool,
    reference: &str,
) -> String {
    let kind = if anchored { "anchor" } else { "inline" };
    let horizontal = if reference == "local" {
        "column"
    } else {
        reference
    };
    let vertical_reference = if reference == "local" {
        "paragraph"
    } else {
        reference
    };
    let position = if anchored {
        format!(
            r#"<wp:positionH relativeFrom="{horizontal}"><wp:posOffset>127000</wp:posOffset></wp:positionH><wp:positionV relativeFrom="{vertical_reference}"><wp:posOffset>254000</wp:posOffset></wp:positionV>"#
        )
    } else {
        String::new()
    };
    let image = format!(
        r#"<w:drawing><wp:{kind}>{position}<wp:extent cx="381000" cy="381000"/><wp:wrapNone/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:blipFill><a:blip r:embed="rimage"/></pic:blipFill><pic:spPr><a:prstGeom prst="rect"/></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:{kind}></w:drawing>"#
    );
    let mut outer = growing_shape_body(220.0, r#"wrap="square""#, "<a:noAutofit/>", 1);
    if vertical {
        outer = outer.replace("<wps:bodyPr ", r#"<wps:bodyPr vert="vert" "#);
    }
    if inline_outer {
        outer = outer.replace("wp:anchor", "wp:inline");
    }
    outer = outer.replace(
        "<w:t>BOX0000</w:t>",
        &format!("<w:t>OUTER_SENTINEL</w:t>{image}"),
    );
    outer
}

fn textbox_image_document(
    vertical: bool,
    anchored: bool,
    inline_outer: bool,
    reference: &str,
) -> Vec<u8> {
    crate::office::docx::testutil::with_related_part(
        docx(
            &textbox_image_body(vertical, anchored, inline_outer, reference),
            None,
            None,
        ),
        "image",
        "media/test.png",
        "",
        false,
    )
}

#[test]
fn textbox_anchored_pictures_survive_horizontal_and_vertical_projection() {
    for vertical in [false, true] {
        for anchored in [false, true] {
            for inline_outer in [false, true] {
                let pages = crate::office::docx::testutil::pages(textbox_image_document(
                    vertical,
                    anchored,
                    inline_outer,
                    "local",
                ));
                let images: Vec<_> = pages
                    .iter()
                    .flat_map(|p| p.items.iter())
                    .filter_map(|item| {
                        if let Item::Image(image) = item {
                            Some(image)
                        } else {
                            None
                        }
                    })
                    .collect();
                assert_eq!(
                    images.len(),
                    1,
                    "vertical={vertical}, anchored={anchored}, inline_outer={inline_outer}"
                );
                assert_eq!(images[0].image.part, "word/media/test.png");
                assert!(
                    (images[0].transform.sx.abs() + images[0].transform.ky.abs() - 30.0).abs()
                        < 0.001
                );
            }
        }
    }
}

#[test]
fn textbox_picture_page_and_margin_references_use_accepted_page() {
    for inline_outer in [false, true] {
        for reference in ["page", "margin"] {
            let pages = crate::office::docx::testutil::pages(textbox_image_document(
                false,
                true,
                inline_outer,
                reference,
            ));
            let page = &pages[0];
            let image = page
                .items
                .iter()
                .find_map(|item| {
                    if let Item::Image(image) = item {
                        Some(image)
                    } else {
                        None
                    }
                })
                .unwrap();
            let (x, y) = if reference == "page" {
                (10.0, 20.0)
            } else {
                (page.info.cols[0].0 + 10.0, page.info.top + 20.0)
            };
            assert!((image.transform.tx as f64 - x).abs() < 0.001, "{reference}");
            assert!((image.transform.ty as f64 - y).abs() < 0.001, "{reference}");
        }
    }
}

#[test]
fn textbox_inline_image_owners_rebuild_once_without_shifting_source_anchors() {
    let opened = crate::office::docx::extract::open(
        textbox_image_document(false, true, true, "local"),
        &Default::default(),
    )
    .unwrap();
    let fonts = crate::office::pptx::fonts::FontSystem::new(&[], true, &Default::default());
    let env = TextEnv {
        fonts: &fonts,
        default_tab: 36.0,
        fields: Default::default(),
        grid: None,
        char_grid: Default::default(),
        expand_shift_return: false,
        mark_in_last_line: true,
        html_spacing: true,
        grow_autofit: false,
        in_cell: std::cell::Cell::new(false),
        in_header: std::cell::Cell::new(false),
        grid_in_table: false,
        no_leading: false,
        compat_mode: 15,
    };
    let flow = Flow::new(&env, 15).for_notes();
    let initial = flow.units(&opened.sections[0].blocks, 468.0, true);
    let Unit::Para(initial) = &initial[0] else {
        panic!("paragraph")
    };
    let owner = initial
        .lines
        .iter()
        .flat_map(|line| &line.anchors)
        .find(|a| inline_box_owner(a))
        .unwrap();
    let mut source = initial.src.as_ref().unwrap().as_ref().clone();
    let mut direct = owner.frame_children[0].anchor.clone();
    direct.pic.w = 41.0;
    direct.wrap = crate::office::docx::model::Wrap::Square;
    direct.v_rel = crate::office::docx::model::VRel::Line;
    source.inlines.insert(
        0,
        crate::office::docx::model::Inline::Anchor(Box::new(direct.clone())),
    );
    direct.pic.w = 17.0;
    direct.pic.part = Some("word/media/second.png".into());
    direct.wrap = crate::office::docx::model::Wrap::None;
    source
        .inlines
        .push(crate::office::docx::model::Inline::Break(
            crate::office::docx::model::BreakKind::Line,
            source.mark.clone(),
        ));
    source
        .inlines
        .push(crate::office::docx::model::Inline::Anchor(Box::new(direct)));
    let original = flow.units(&[Block::Para(source.clone())], 468.0, true);
    let Unit::Para(paragraph) = &original[0] else {
        panic!("paragraph")
    };
    let mut rebuilt = layout_paragraph(&source, 280.0, &env);
    crate::office::docx::paginate::restore_reflow_anchors(&paragraph.lines, &[], &mut rebuilt);
    flow.resolve_inline_textboxes(&mut rebuilt);
    flow.resolve_inline_textboxes(&mut rebuilt);
    let anchors: Vec<_> = rebuilt.iter().flat_map(|line| &line.anchors).collect();
    assert_eq!(anchors.iter().filter(|a| inline_box_owner(a)).count(), 1);
    assert_eq!(anchors.iter().filter(|a| !inline_box_owner(a)).count(), 2);
    assert_eq!(
        anchors.iter().find(|a| !inline_box_owner(a)).unwrap().pic.w,
        41.0
    );
    let (slices, _) = cells::note_stack(&original, 468.0, &env);
    let (remeasured, _) =
        note_source::remeasure_slices(&slices, 280.0, &env).expect("note source remeasurement");
    let anchors: Vec<_> = remeasured.iter().flat_map(|slice| &slice.anchors).collect();
    assert_eq!(
        anchors
            .iter()
            .filter(|(a, _, _)| inline_box_owner(a))
            .count(),
        1
    );
    assert_eq!(
        anchors
            .iter()
            .filter(|(a, _, _)| !inline_box_owner(a))
            .count(),
        2
    );
}

#[test]
fn textbox_table_picture_keeps_paragraph_origin_across_source_lines() {
    for table in [false, true] {
        let mut positions = Vec::new();
        for second_line in [false, true] {
            let mut body = textbox_image_body(false, true, true, "local");
            if second_line {
                body = body.replace("OUTER_SENTINEL</w:t>", "OUTER_SENTINEL</w:t><w:br/>");
            }
            if table {
                body = body.replace("<w:txbxContent>", r#"<w:txbxContent><w:tbl><w:tblPr><w:tblLayout w:type="fixed"/></w:tblPr><w:tblGrid><w:gridCol w:w="3000"/></w:tblGrid><w:tr><w:tc>"#)
                    .replace("</w:txbxContent>", "</w:tc></w:tr></w:tbl></w:txbxContent>");
            }
            let bytes = crate::office::docx::testutil::with_related_part(
                docx(&body, None, None),
                "image",
                "media/test.png",
                "",
                false,
            );
            let pages = crate::office::docx::testutil::pages(bytes);
            let image = pages
                .iter()
                .flat_map(|p| &p.items)
                .find_map(|item| {
                    if let Item::Image(image) = item {
                        Some(image)
                    } else {
                        None
                    }
                })
                .unwrap();
            positions.push((image.transform.tx, image.transform.ty));
        }
        assert_eq!(positions[0], positions[1], "table={table}");
    }
}

#[test]
fn textbox_nested_row_anchor_includes_retained_outer_spacing() {
    let source = table(&row(0, "", "<w:p><w:r><w:t>LINE</w:t></w:r></w:p>"));
    let layout = units(&source);
    let mut row = rows(&layout)[0].clone();
    row.outer_before = 9.0;
    row.cells[0].slices[0]
        .anchors
        .push((box_anchor(Vec::new()), 0.0, 0.0));
    let margin = row.cells[0].margins[0];
    let slice = nested_row_slice(Rc::new(row), 7.0);
    let mut anchors = Vec::new();
    box_slice_anchors(
        &slice,
        tiny_skia::Transform::identity(),
        468.0,
        0.0,
        0.0,
        &mut anchors,
    );
    assert_eq!(anchors.len(), 1);
    assert_eq!(anchors[0].line_y, 7.0 + 9.0 + margin);
}

#[test]
fn textbox_rotated_picture_global_alignment_uses_painted_bounds() {
    for inner_turn in [false, true] {
        for reference in ["page", "margin"] {
            for (horizontal, vertical) in [
                ("left", "top"),
                ("center", "center"),
                ("right", "bottom"),
                ("offset", "offset"),
                ("negative", "negative"),
            ] {
                let mut body = textbox_image_body(true, true, true, reference)
                    .replace("cx=\"381000\" cy=\"381000\"", "cx=\"508000\" cy=\"254000\"");
                if horizontal == "negative" {
                    body = body
                        .replace(">127000<", ">-127000<")
                        .replace(">254000<", ">-254000<");
                } else if horizontal != "offset" {
                    body = body
                        .replace(
                            "<wp:posOffset>127000</wp:posOffset>",
                            &format!("<wp:align>{horizontal}</wp:align>"),
                        )
                        .replace(
                            "<wp:posOffset>254000</wp:posOffset>",
                            &format!("<wp:align>{vertical}</wp:align>"),
                        );
                }
                if inner_turn {
                    body = body.replace("<pic:spPr>", "<pic:spPr><a:xfrm rot=\"5400000\"/>");
                }
                let bytes = crate::office::docx::testutil::with_related_part(
                    docx(&body, None, None),
                    "image",
                    "media/test.png",
                    "",
                    false,
                );
                let pages = crate::office::docx::testutil::pages(bytes);
                let page = &pages[0];
                let images: Vec<_> = page
                    .items
                    .iter()
                    .filter_map(|item| match item {
                        Item::Image(image) => Some(image),
                        _ => None,
                    })
                    .collect();
                assert_eq!(images.len(), 1);
                let mut corners = [
                    tiny_skia::Point::from_xy(0.0, 0.0),
                    tiny_skia::Point::from_xy(1.0, 0.0),
                    tiny_skia::Point::from_xy(0.0, 1.0),
                    tiny_skia::Point::from_xy(1.0, 1.0),
                ];
                images[0].transform.map_points(&mut corners);
                let left = corners
                    .iter()
                    .map(|p| p.x as f64)
                    .fold(f64::INFINITY, f64::min);
                let top = corners
                    .iter()
                    .map(|p| p.y as f64)
                    .fold(f64::INFINITY, f64::min);
                let right = corners
                    .iter()
                    .map(|p| p.x as f64)
                    .fold(f64::NEG_INFINITY, f64::max);
                let bottom = corners
                    .iter()
                    .map(|p| p.y as f64)
                    .fold(f64::NEG_INFINITY, f64::max);
                let (paint_w, paint_h) = if inner_turn {
                    (40.0, 20.0)
                } else {
                    (20.0, 40.0)
                };
                assert!((right - left - paint_w).abs() < 0.001);
                assert!((bottom - top - paint_h).abs() < 0.001);
                let (x, y, w, h) = if reference == "page" {
                    (0.0, 0.0, page.info.w, page.info.h)
                } else {
                    let first = page.info.cols[0];
                    let last = page.info.cols.last().unwrap();
                    (
                        first.0,
                        page.info.top,
                        last.0 + last.1 - first.0,
                        page.info.bottom - page.info.top,
                    )
                };
                let expected_x = x + match horizontal {
                    "center" => (w - paint_w) / 2.0,
                    "right" => w - paint_w,
                    "offset" => 10.0,
                    "negative" => -10.0,
                    _ => 0.0,
                };
                let expected_y = y + match vertical {
                    "center" => (h - paint_h) / 2.0,
                    "bottom" => h - paint_h,
                    "offset" => 20.0,
                    "negative" => -20.0,
                    _ => 0.0,
                };
                assert!(
                    (left - expected_x).abs() < 0.001,
                    "{reference}/{horizontal}: {left} != {expected_x}"
                );
                assert!(
                    (top - expected_y).abs() < 0.001,
                    "{reference}/{vertical}: {top} != {expected_y}"
                );
            }
        }
    }
}
#[test]
fn fixed_nowrap_textbox_retains_source_overflow() {
    // Explicit no-wrap with noAutofit permits text beyond the saved rectangle
    // (ISO/IEC 29500-1, ST_TextWrappingType and noAutofit).
    let content = "ALPHA BETA GAMMA ".repeat(20);
    let normalized = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
    for wrap in ["none", "square"] {
        let body = growing_shape_body(
            240.0,
            &format!(r#"wrap="{wrap}" horzOverflow="overflow" vertOverflow="overflow""#),
            "<a:noAutofit/>",
            1,
        )
        .replace("BOX0000", &content);
        let us = units(&body);
        let anchor = shape_anchor(&us);
        assert_eq!(anchor.pic.w, 180.0);
        assert_eq!(anchor.pic.h, 240.0);
        let runs: Vec<_> = anchor
            .text_items
            .iter()
            .filter_map(|item| match item {
                Item::Text(run) => Some(run),
                _ => None,
            })
            .collect();
        let text: String = runs.iter().map(|run| run.text.as_str()).collect();
        assert_eq!(normalized(&text), normalized(&content));
        let baselines: std::collections::HashSet<_> =
            runs.iter().map(|run| run.transform.ty.to_bits()).collect();
        if wrap == "none" {
            assert_eq!(baselines.len(), 1);
            assert!(runs.iter().any(|run| run.width > 612.0));
        } else {
            assert!(baselines.len() > 1);
            assert!(runs.iter().all(|run| run.width <= 180.0));
        }
    }
}

#[path = "hidden_paragraph_tests.rs"]
mod hidden_paragraph_tests;

#[test]
fn list_items_keep_space_before_when_automatic_space_after_is_dropped() {
    let numbering = r#"<w:abstractNum w:abstractNumId="0"><w:lvl w:ilvl="0"><w:numFmt w:val="bullet"/><w:lvlText w:val="o"/></w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num>"#;
    let item = r#"<w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="1"/></w:numPr><w:spacing w:before="120" w:after="100" w:afterAutospacing="1"/></w:pPr><w:r><w:t>item</w:t></w:r></w:p>"#;
    let us = units_package(
        docx(&item.repeat(3), None, Some(numbering)),
        false,
        false,
        false,
    );
    let spacing: Vec<_> = us
        .iter()
        .map(|u| match u {
            Unit::Para(p) => (p.before, p.after),
            _ => panic!("paragraphs only"),
        })
        .collect();
    // Word for Mac: 6 pt between items, 14 pt automatic space after the last one.
    assert_eq!(spacing, vec![(6.0, 0.0), (6.0, 0.0), (6.0, 14.0)]);
}

#[test]
fn final_cell_line_must_leave_room_for_the_row_border() {
    let para = r#"<w:p><w:pPr><w:spacing w:before="0" w:after="0" w:line="240" w:lineRule="exact"/><w:widowControl w:val="0"/></w:pPr><w:r><w:t>line</w:t></w:r></w:p>"#;
    let body = format!(
        r#"<w:tbl><w:tblPr><w:tblBorders><w:top w:val="single" w:sz="8"/><w:bottom w:val="single" w:sz="8"/></w:tblBorders><w:tblCellMar><w:top w:w="0" w:type="dxa"/><w:bottom w:w="0" w:type="dxa"/></w:tblCellMar></w:tblPr><w:tblGrid><w:gridCol w:w="3000"/></w:tblGrid><w:tr><w:tc>{}</w:tc></w:tr></w:tbl>"#,
        para.repeat(4)
    );
    let us = units(&body);
    let r = rows(&us)[0];
    assert!(r.natural_addition > 0.0);
    // The lines fit exactly; the border does not, so the last line moves on.
    let (top, rest) = split_row(r, 48.0 + r.outer_before, true).expect("three lines stay");
    assert_eq!(top.cells[0].slices.len(), 3);
    assert_eq!(rest.cells[0].slices.len(), 1);
}

#[test]
fn automatic_space_before_is_dropped_at_the_top_of_a_header() {
    let auto = r#"<w:p><w:pPr><w:spacing w:before="100" w:beforeAutospacing="1"/></w:pPr><w:r><w:t>Head</w:t></w:r></w:p><w:p><w:pPr><w:spacing w:before="100" w:beforeAutospacing="1"/></w:pPr><w:r><w:t>Next</w:t></w:r></w:p>"#;
    let before = |in_header: bool| -> Vec<f64> {
        units_context(auto, false, in_header)
            .iter()
            .map(|u| match u {
                Unit::Para(p) => p.before,
                _ => -1.0,
            })
            .collect()
    };
    // The header's first paragraph loses its automatic space; later ones keep it.
    assert_eq!(before(true)[0], 0.0);
    assert!(before(true)[1] > 0.0);
    // In the body the first paragraph keeps it here (page-top rules are pagination's).
    assert!(before(false)[0] > 0.0);
}

#[test]
fn a_wrapped_floating_picture_grows_its_cell() {
    // A 72 pt square-wrapped picture anchored in a one-line cell: the row holds it.
    // Behind-text and unwrapped pictures leave the row at its text height.
    let pic = |wrap: &str, behind: &str| {
        format!(
            r#"<w:p><w:r><w:drawing><wp:anchor xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" layoutInCell="1" behindDoc="{behind}"><wp:positionH relativeFrom="column"><wp:posOffset>0</wp:posOffset></wp:positionH><wp:positionV relativeFrom="paragraph"><wp:posOffset>0</wp:posOffset></wp:positionV><wp:extent cx="914400" cy="914400"/>{wrap}</wp:anchor></w:drawing><w:t>TEXT</w:t></w:r></w:p>"#
        )
    };
    let height = |p: &str| rows(&units(&table(&row(0, "", p))))[0].height;
    assert!(height(&pic(r#"<wp:wrapSquare wrapText="bothSides"/>"#, "0")) >= 72.0);
    assert!(height(&pic("<wp:wrapNone/>", "0")) < 30.0);
    assert!(height(&pic(r#"<wp:wrapSquare wrapText="bothSides"/>"#, "1")) < 30.0);
}

#[test]
fn hide_mark_drops_the_line_a_cells_final_line_break_opens() {
    // Without `w:hideMark` the break opens an empty line for the end-of-cell mark;
    // with it the mark takes no height and the row is one line tall.
    let height = |tc_pr: &str| {
        let body = format!(
            r#"<w:tbl><w:tblGrid><w:gridCol w:w="3000"/></w:tblGrid><w:tr><w:tc><w:tcPr>{tc_pr}</w:tcPr><w:p><w:pPr><w:spacing w:after="0"/></w:pPr><w:r><w:t>Text</w:t></w:r><w:r><w:br/></w:r></w:p></w:tc></w:tr></w:tbl>"#
        );
        rows(&units(&body))[0].height
    };
    let (plain, hidden) = (height(""), height("<w:hideMark/>"));
    assert!(hidden > 0.0);
    assert!((plain - 2.0 * hidden).abs() < 1.0, "{plain} vs {hidden}");
}

#[test]
fn auto_spacing_is_dropped_at_both_ends_of_a_cell() {
    // One paragraph with automatic space before and after (HTML spacing) in a cell:
    // the row is as tall as the line, the same as with no spacing at all.
    let height = |spacing: &str| {
        let body = format!(
            r#"<w:tbl><w:tblPr><w:tblCellMar><w:top w:w="0" w:type="dxa"/><w:bottom w:w="0" w:type="dxa"/></w:tblCellMar></w:tblPr><w:tblGrid><w:gridCol w:w="3000"/></w:tblGrid><w:tr><w:tc><w:p><w:pPr>{spacing}</w:pPr><w:r><w:t>Text</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#
        );
        rows(&units(&body))[0].height
    };
    let auto = height(r#"<w:spacing w:beforeAutospacing="1" w:afterAutospacing="1"/>"#);
    let none = height(r#"<w:spacing w:before="0" w:after="0"/>"#);
    assert!((auto - none).abs() < 0.01, "{auto} vs {none}");
}

#[test]
fn a_centered_table_wider_than_the_column_overhangs_both_margins() {
    // A 600 pt table centered in the 468 pt column starts 66 pt left of it.
    let body = r#"<w:tbl><w:tblPr><w:tblW w:w="12000" w:type="dxa"/><w:jc w:val="center"/><w:tblLayout w:type="fixed"/></w:tblPr><w:tblGrid><w:gridCol w:w="12000"/></w:tblGrid><w:tr><w:tc><w:tcPr><w:tcW w:w="12000" w:type="dxa"/></w:tcPr><w:p><w:r><w:t>X</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#;
    let units = units(body);
    let cell = &rows(&units)[0].cells[0];
    assert!((cell.x + 66.0).abs() < 6.0, "{}", cell.x);
}

#[test]
fn a_tables_top_border_takes_its_full_width() {
    // A 4.5 pt top border on a one-row table: the row takes the whole border,
    // which has no row above to share it with.
    let height = |borders: &str| {
        let body = format!(
            r#"<w:tbl><w:tblPr>{borders}</w:tblPr><w:tblGrid><w:gridCol w:w="3000"/></w:tblGrid><w:tr><w:tc><w:p><w:r><w:t>X</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#
        );
        rows(&units(&body))[0].flow_height()
    };
    let thick = height(
        r#"<w:tblBorders><w:top w:val="single" w:sz="36" w:space="0" w:color="000000"/></w:tblBorders>"#,
    );
    assert!((thick - height("") - 4.5).abs() < 0.01, "{thick}");
}

#[test]
fn a_thin_picture_line_in_a_cell_keeps_the_marks_line_height() {
    // A cell holding only a 0.05 pt VML horizontal rule in 10 pt text: the row is
    // as tall as a line of the paragraph mark's font, as in the body (Word for Mac).
    let height = |content: &str| {
        let body = format!(
            r#"<w:tbl><w:tblPr><w:tblCellMar><w:top w:w="0" w:type="dxa"/><w:bottom w:w="0" w:type="dxa"/></w:tblCellMar></w:tblPr><w:tblGrid><w:gridCol w:w="3000"/></w:tblGrid><w:tr><w:tc>{content}</w:tc></w:tr></w:tbl>"#
        );
        rows(&units(&body))[0].height
    };
    let rule = height(
        r#"<w:p><w:pPr><w:spacing w:after="0"/><w:rPr><w:sz w:val="20"/></w:rPr></w:pPr><w:r><w:rPr><w:sz w:val="20"/></w:rPr><w:pict><v:rect xmlns:v="urn:schemas-microsoft-com:vml" xmlns:o="urn:schemas-microsoft-com:office:office" style="width:100pt;height:.05pt" o:hr="t" stroked="f"/></w:pict></w:r></w:p>"#,
    );
    let text = height(
        r#"<w:p><w:pPr><w:spacing w:after="0"/></w:pPr><w:r><w:rPr><w:sz w:val="20"/></w:rPr><w:t>x</w:t></w:r></w:p>"#,
    );
    assert!((rule - text).abs() < 0.5, "{rule} vs {text}");
}

#[test]
fn a_row_of_merged_cells_takes_its_minimum_height() {
    // Row 2 holds only the lower halves of two vertical merges, under an 8.05 pt
    // "at least" height: Word for Mac draws it 8.05 pt tall (plus the border), not
    // as tall as an empty row's 12 pt mark.
    let cell = |merge: &str, text: &str| {
        format!(
            r#"<w:tc><w:tcPr><w:tcW w:w="1500" w:type="dxa"/>{merge}</w:tcPr><w:p><w:pPr><w:spacing w:after="0" w:line="161" w:lineRule="exact"/></w:pPr><w:r><w:t>{text}</w:t></w:r></w:p></w:tc>"#
        )
    };
    let restart = r#"<w:vMerge w:val="restart"/>"#;
    let body = format!(
        r#"<w:tbl><w:tblPr><w:tblCellMar><w:top w:w="0" w:type="dxa"/><w:bottom w:w="0" w:type="dxa"/></w:tblCellMar></w:tblPr><w:tblGrid><w:gridCol w:w="1500"/><w:gridCol w:w="1500"/></w:tblGrid><w:tr>{}{}</w:tr><w:tr><w:trPr><w:trHeight w:val="161"/></w:trPr>{}{}</w:tr><w:tr>{}{}</w:tr></w:tbl>"#,
        cell(restart, "A"),
        cell(restart, "B"),
        cell("<w:vMerge/>", ""),
        cell("<w:vMerge/>", ""),
        cell("", "C"),
        cell("", "D"),
    );
    let height = rows(&units(&body))[1].height;
    assert!((height - 8.05).abs() < 0.5, "{height}");
}

#[test]
fn a_hidden_mark_keeps_its_label_and_absorbs_the_empty_paragraph_after_it() {
    // A numbered paragraph whose mark is hidden, then an empty paragraph, in one cell:
    // Word for Mac draws "1. Text" on one line and gives the empty paragraph none.
    let numbering = r#"<w:abstractNum w:abstractNumId="0"><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:lvlText w:val="%1."/></w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num>"#;
    let cell = |mark: &str| {
        format!(
            r#"<w:tbl><w:tblGrid><w:gridCol w:w="4000"/></w:tblGrid><w:tr><w:tc><w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="1"/></w:numPr><w:spacing w:after="0"/><w:rPr>{mark}</w:rPr></w:pPr><w:r><w:t>Text</w:t></w:r></w:p><w:p><w:pPr><w:spacing w:after="0"/></w:pPr></w:p></w:tc></w:tr></w:tbl>"#
        )
    };
    let shown = units_package(docx(&cell(""), None, Some(numbering)), false, false, false);
    let hidden = units_package(
        docx(&cell("<w:vanish/>"), None, Some(numbering)),
        false,
        false,
        false,
    );
    let (shown, hidden) = (rows(&shown)[0].height, rows(&hidden)[0].height);
    assert!(hidden < shown * 0.7, "{hidden} vs {shown}");
    let text =
        crate::office::docx::testutil::pages(docx(&cell("<w:vanish/>"), None, Some(numbering)))[0]
            .items
            .iter()
            .filter_map(|i| match i {
                crate::office::pptx::scene::Item::Text(r) => Some(r.text.clone()),
                _ => None,
            })
            .collect::<String>();
    assert!(text.contains("1."), "{text}");
}

#[test]
fn an_at_least_row_height_leaves_out_the_cell_padding() {
    // A one-line row with a 17 pt "at least" height and 100 tw top and bottom padding:
    // Word for Mac draws it 27 pt tall (17 pt plus the padding), as without padding
    // it draws 17 pt.
    let height = |padding: u32| {
        let body = format!(
            r#"<w:tbl><w:tblPr><w:tblCellMar><w:top w:w="{padding}" w:type="dxa"/><w:bottom w:w="{padding}" w:type="dxa"/></w:tblCellMar></w:tblPr><w:tblGrid><w:gridCol w:w="4000"/></w:tblGrid><w:tr><w:trPr><w:trHeight w:val="340"/></w:trPr><w:tc><w:p><w:pPr><w:spacing w:after="0"/></w:pPr><w:r><w:t>x</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#
        );
        rows(&units(&body))[0].height
    };
    assert!(
        (height(100) - height(0) - 10.0).abs() < 0.1,
        "{} {}",
        height(100),
        height(0)
    );
}

#[test]
fn a_hidden_cell_mark_drops_the_closing_empty_paragraph_after_text() {
    // A cell with a line of text and an empty closing paragraph under w:hideMark:
    // Word for Mac draws one line.
    let height = |hide: &str| {
        let body = format!(
            r#"<w:tbl><w:tblGrid><w:gridCol w:w="4000"/></w:tblGrid><w:tr><w:tc><w:tcPr>{hide}</w:tcPr><w:p><w:pPr><w:spacing w:after="0"/></w:pPr><w:r><w:t>x</w:t></w:r></w:p><w:p><w:pPr><w:spacing w:after="0"/></w:pPr></w:p></w:tc></w:tr></w:tbl>"#
        );
        rows(&units(&body))[0].height
    };
    assert!(
        height("<w:hideMark/>") < height("") * 0.7,
        "{}",
        height("<w:hideMark/>")
    );
}

#[test]
fn cell_spacing_narrows_the_cell_text_on_both_sides() {
    // A 3 inch fixed cell holding a 30-letter line: it fits on one line without cell
    // spacing and wraps with half an inch of spacing on each side.
    let lines = |spacing: u32| {
        let body = format!(
            r#"<w:tbl><w:tblPr><w:tblW w:w="4320" w:type="dxa"/><w:tblLayout w:type="fixed"/><w:tblCellSpacing w:w="{spacing}" w:type="dxa"/></w:tblPr><w:tblGrid><w:gridCol w:w="4320"/></w:tblGrid><w:tr><w:tc><w:tcPr><w:tcW w:w="4320" w:type="dxa"/></w:tcPr><w:p><w:r><w:t>nnnnn nnnnn nnnnn nnnnn nnnnn nnnnn</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:p/>"#
        );
        let out = crate::office::docx::testutil::pages(docx(&body, None, None));
        let mut ys: Vec<i64> = out[0]
            .items
            .iter()
            .filter_map(|i| match i {
                crate::office::pptx::scene::Item::Text(r) if r.text.contains('n') => {
                    Some(r.transform.ty.round() as i64)
                }
                _ => None,
            })
            .collect();
        ys.dedup();
        ys.len()
    };
    assert_eq!(lines(0), 1);
    assert!(lines(720) > 1);
}

#[test]
fn cell_spacing_takes_twice_its_value_between_and_around_rows() {
    // Three one-line rows with 100 tw cell spacing: Word for Mac draws them 40 pt
    // taller than without spacing (2 x 5 pt above, between and below the rows).
    let height = |spacing: &str| {
        let row = r#"<w:tr><w:tc><w:p><w:pPr><w:spacing w:after="0"/></w:pPr><w:r><w:t>x</w:t></w:r></w:p></w:tc></w:tr>"#;
        let body = format!(
            r#"<w:tbl><w:tblPr>{spacing}</w:tblPr><w:tblGrid><w:gridCol w:w="4000"/></w:tblGrid>{}</w:tbl>"#,
            row.repeat(3)
        );
        stack(&units(&body)).1
    };
    let spaced = height(r#"<w:tblCellSpacing w:w="100" w:type="dxa"/>"#);
    assert!((spaced - height("") - 40.0).abs() < 0.1, "{spaced}");
}

#[test]
fn a_tables_bottom_border_takes_its_full_width() {
    // One-line rows under 3 pt borders: Word for Mac draws each horizontal border
    // in full, 6 pt for one row and 12 pt for three (rows + 1 lines).
    let extra = |rows: usize| {
        let table = |borders: &str| {
            let row = r#"<w:tr><w:tc><w:p><w:pPr><w:spacing w:after="0"/></w:pPr><w:r><w:t>x</w:t></w:r></w:p></w:tc></w:tr>"#;
            let body = format!(
                r#"<w:tbl><w:tblPr>{borders}</w:tblPr><w:tblGrid><w:gridCol w:w="4000"/></w:tblGrid>{}</w:tbl>"#,
                row.repeat(rows)
            );
            stack(&units(&body)).1
        };
        table(
            r#"<w:tblBorders><w:top w:val="single" w:sz="24"/><w:bottom w:val="single" w:sz="24"/><w:insideH w:val="single" w:sz="24"/></w:tblBorders>"#,
        ) - table("")
    };
    assert!((extra(1) - 6.0).abs() < 0.1, "{}", extra(1));
    assert!((extra(3) - 12.0).abs() < 0.1, "{}", extra(3));
}

#[test]
fn spaced_cells_keep_their_own_borders_inside_the_tables() {
    // 100 tw cell spacing under 3 pt borders: Word for Mac adds 12 pt to one row
    // (table top and bottom, then the cell's own top and bottom) and 24 pt to three.
    let extra = |rows: usize| {
        let table = |borders: &str| {
            let row = r#"<w:tr><w:tc><w:p><w:pPr><w:spacing w:after="0"/></w:pPr><w:r><w:t>x</w:t></w:r></w:p></w:tc></w:tr>"#;
            let body = format!(
                r#"<w:tbl><w:tblPr><w:tblCellSpacing w:w="100" w:type="dxa"/>{borders}</w:tblPr><w:tblGrid><w:gridCol w:w="4000"/></w:tblGrid>{}</w:tbl>"#,
                row.repeat(rows)
            );
            stack(&units(&body)).1
        };
        table(
            r#"<w:tblBorders><w:top w:val="single" w:sz="24"/><w:bottom w:val="single" w:sz="24"/><w:insideH w:val="single" w:sz="24"/></w:tblBorders>"#,
        ) - table("")
    };
    assert!((extra(1) - 12.0).abs() < 0.1, "{}", extra(1));
    assert!((extra(3) - 24.0).abs() < 0.1, "{}", extra(3));
}

#[test]
fn a_legacy_horizontal_merge_spans_its_continuation_cells() {
    // w:hMerge restart + two continuations: one cell across the three grid columns,
    // as Word lays it out (a header table's title row in a sampling report).
    let cell = |merge: &str, text: &str| {
        format!(
            r#"<w:tc><w:tcPr><w:tcW w:w="2000" w:type="dxa"/>{merge}</w:tcPr><w:p><w:r><w:t>{text}</w:t></w:r></w:p></w:tc>"#
        )
    };
    let body = format!(
        r#"<w:tbl><w:tblGrid><w:gridCol w:w="2000"/><w:gridCol w:w="2000"/><w:gridCol w:w="2000"/></w:tblGrid><w:tr>{}{}{}</w:tr></w:tbl>"#,
        cell(r#"<w:hMerge w:val="restart"/>"#, "Wide"),
        cell(r#"<w:hMerge w:val="continue"/>"#, ""),
        cell("<w:hMerge/>", "")
    );
    let us = units(&body);
    let row = rows(&us)[0];
    assert_eq!(row.cells.len(), 1);
    assert!((row.cells[0].w - 300.0).abs() < 0.5, "{}", row.cells[0].w);
}

#[test]
fn a_cell_without_widow_control_splits_after_one_text_line() {
    // Room for one 10 pt line: a cell of four one-line text paragraphs without widow
    // control splits after the first (Word for Mac); empty paragraphs stay together.
    let para = |text: &str| {
        format!(
            r#"<w:p><w:pPr><w:spacing w:before="0" w:after="0" w:line="200" w:lineRule="exact"/><w:widowControl w:val="0"/></w:pPr><w:r><w:t>{text}</w:t></w:r></w:p>"#
        )
    };
    let split = |content: String| {
        let body = format!(
            r#"<w:tbl><w:tblPr><w:tblCellMar><w:top w:w="0" w:type="dxa"/><w:bottom w:w="0" w:type="dxa"/></w:tblCellMar></w:tblPr><w:tblGrid><w:gridCol w:w="3000"/></w:tblGrid><w:tr><w:tc>{content}</w:tc></w:tr></w:tbl>"#
        );
        let us = units(&body);
        split_row(rows(&us)[0], 15.0, true).map(|(top, _)| top.cells[0].slices.len())
    };
    assert_eq!(split(para("line").repeat(4)), Some(1));
    assert_eq!(split(para("").repeat(4)), None);
}
