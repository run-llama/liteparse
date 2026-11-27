use crate::office::docx::testutil::{docx, pages, para};

#[test]
fn picture_leading_can_cross_the_page_bottom_without_moving_the_picture() {
    let body = r#"<w:p><w:pPr><w:spacing w:after="0" w:line="2000" w:lineRule="exact"/></w:pPr><w:r><w:t>before</w:t></w:r></w:p><w:p><w:pPr><w:spacing w:after="0" w:line="480" w:lineRule="auto"/></w:pPr><w:r><w:drawing><wp:inline xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"><wp:extent cx="914400" cy="914400"/><a:graphic><a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"><wps:wsp><wps:spPr><a:prstGeom prst="rect"/><a:solidFill><a:srgbClr val="008080"/></a:solidFill></wps:spPr><wps:bodyPr/></wps:wsp></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p><w:sectPr><w:pgSz w:w="12240" w:h="6330"/><w:pgMar w:top="1440" w:bottom="1440" w:left="1440" w:right="1440"/></w:sectPr>"#;
    let out = pages(docx(body, None, None));

    assert_eq!(out.len(), { 1 });
    let paths = out
        .last()
        .unwrap()
        .items
        .iter()
        .filter(|i| matches!(i, crate::office::pptx::scene::Item::Path(_)))
        .count();
    assert!(paths > 0, "the picture must be rendered");
}

#[test]
fn a_spaced_text_lines_trailing_leading_can_cross_the_page_bottom() {
    // 150 pt of a 172.5 pt body is taken; the double-spaced 12 pt line needs about
    // 14 pt of text and 28 pt with its leading.
    let body = r#"<w:p><w:pPr><w:spacing w:after="0" w:line="3000" w:lineRule="exact"/></w:pPr><w:r><w:t>before</w:t></w:r></w:p><w:p><w:pPr><w:spacing w:after="0" w:line="480" w:lineRule="auto"/></w:pPr><w:r><w:rPr><w:sz w:val="24"/></w:rPr><w:t>last</w:t></w:r></w:p><w:sectPr><w:pgSz w:w="12240" w:h="6330"/><w:pgMar w:top="1440" w:bottom="1440" w:left="1440" w:right="1440"/></w:sectPr>"#;
    assert_eq!(pages(docx(body, None, None)).len(), 1);
}

#[test]
fn a_table_runs_beside_a_wrapped_shape_it_clears() {
    // A 100 pt square shape 300 pt into the column; a table 200 pt wide below its
    // anchor paragraph stays beside it, and a full-width one moves below it.
    let ys = |table_w: u32| {
        let body = format!(
            r#"<w:p><w:pPr><w:spacing w:after="0"/></w:pPr><w:r><w:drawing><wp:anchor xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" distL="0" distR="0"><wp:positionH relativeFrom="column"><wp:posOffset>3810000</wp:posOffset></wp:positionH><wp:positionV relativeFrom="paragraph"><wp:posOffset>0</wp:posOffset></wp:positionV><wp:extent cx="1270000" cy="1270000"/><wp:wrapSquare wrapText="bothSides"/></wp:anchor></w:drawing><w:t>A</w:t></w:r></w:p><w:tbl><w:tblPr><w:tblW w:w="{table_w}" w:type="dxa"/><w:tblLayout w:type="fixed"/></w:tblPr><w:tblGrid><w:gridCol w:w="{table_w}"/></w:tblGrid><w:tr><w:tc><w:tcPr><w:tcW w:w="{table_w}" w:type="dxa"/></w:tcPr><w:p><w:r><w:t>CELL</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:sectPr><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440"/></w:sectPr>"#
        );
        let page = &pages(docx(&body, None, None))[0];
        let y = |t: &str| {
            page.items.iter().find_map(|i| match i {
                crate::office::pptx::scene::Item::Text(r) if r.text == t => Some(r.transform.ty),
                _ => None,
            })
        };
        (y("A").unwrap(), y("CELL").unwrap())
    };
    let (a, beside) = ys(4000);
    assert!(beside - a < 40.0, "{a} {beside}");
    let (a, below) = ys(9360);
    assert!(below - a > 90.0, "{a} {below}");
}

#[test]
fn wrap_element_distance_changes_the_space_beside_a_shape() {
    let x = |distance: &str| {
        let body = format!(
            r#"<w:p><w:r><w:drawing><wp:anchor xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" distR="127000"><wp:positionH relativeFrom="column"><wp:posOffset>0</wp:posOffset></wp:positionH><wp:positionV relativeFrom="paragraph"><wp:posOffset>0</wp:posOffset></wp:positionV><wp:extent cx="1270000" cy="1270000"/><wp:wrapSquare wrapText="bothSides" {distance}/></wp:anchor></w:drawing><w:t>beside shape</w:t></w:r></w:p>"#
        );
        let out = pages(docx(&body, None, None));
        out[0]
            .items
            .iter()
            .find_map(|i| match i {
                crate::office::pptx::scene::Item::Text(r) if r.text == "beside shape" => {
                    Some(r.transform.tx)
                }
                _ => None,
            })
            .unwrap()
    };
    let inherited = x("");
    assert!((inherited - 182.0).abs() < 0.1, "x={inherited}");
    assert!((x(r#"distR="254000""#) - inherited - 10.0).abs() < 0.1);
    assert!((x(r#"distR="0""#) - inherited + 10.0).abs() < 0.1);
}

#[test]
fn a_word_wider_than_the_gap_beside_a_shape_moves_below_it() {
    // A 100 pt tall shape leaves a 33 pt gap at the right of a 523 pt text column.
    let body = r#"<w:p><w:r><w:drawing><wp:anchor xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" distL="0" distR="0"><wp:positionH relativeFrom="column"><wp:posOffset>0</wp:posOffset></wp:positionH><wp:positionV relativeFrom="paragraph"><wp:posOffset>0</wp:posOffset></wp:positionV><wp:extent cx="6223000" cy="1270000"/><wp:wrapSquare wrapText="bothSides"/></wp:anchor></w:drawing><w:t>Hi Certified</w:t></w:r></w:p><w:sectPr><w:pgSz w:w="11900" w:h="16840"/><w:pgMar w:top="720" w:bottom="720" w:left="720" w:right="720"/></w:sectPr>"#;
    let out = pages(docx(body, None, None));
    let at = |word: &str| {
        out[0]
            .items
            .iter()
            .find_map(|i| match i {
                crate::office::pptx::scene::Item::Text(r) if r.text.trim() == word => {
                    Some((r.transform.tx, r.transform.ty))
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("no run {word:?}"))
    };
    let (hi, certified) = (at("Hi"), at("Certified"));
    assert!(hi.0 > 520.0 && hi.1 < 136.0, "Hi at {hi:?}");
    assert!(
        (certified.0 - 36.0).abs() < 0.1 && certified.1 > 136.0,
        "Certified at {certified:?}"
    );
}

#[test]
fn an_empty_anchor_paragraph_stays_beside_its_shape_in_a_narrow_gap() {
    // A 500 x 100 pt shape in a 523 pt column: its anchor paragraph's 18 pt right
    // indent leaves a 5 pt gap, the empty ones after it 23 pt. All three fit beside
    // the shape, so "afterwards" (wider than the gap) starts right below it.
    let body = r#"<w:p><w:pPr><w:spacing w:after="0"/><w:ind w:right="360"/></w:pPr><w:r><w:drawing><wp:anchor xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" distL="0" distR="0"><wp:positionH relativeFrom="column"><wp:posOffset>0</wp:posOffset></wp:positionH><wp:positionV relativeFrom="paragraph"><wp:posOffset>0</wp:posOffset></wp:positionV><wp:extent cx="6350000" cy="1270000"/><wp:wrapSquare wrapText="bothSides"/></wp:anchor></w:drawing></w:r></w:p><w:p><w:pPr><w:spacing w:after="0"/></w:pPr></w:p><w:p><w:pPr><w:spacing w:after="0"/></w:pPr></w:p><w:p><w:r><w:t>afterwards</w:t></w:r></w:p><w:sectPr><w:pgSz w:w="11900" w:h="16840"/><w:pgMar w:top="720" w:bottom="720" w:left="720" w:right="720"/></w:sectPr>"#;
    let out = pages(docx(body, None, None));
    let after = out[0]
        .items
        .iter()
        .find_map(|i| match i {
            crate::office::pptx::scene::Item::Text(r) if r.text.trim() == "afterwards" => {
                Some(r.transform.ty)
            }
            _ => None,
        })
        .expect("afterwards");
    // The shape spans 36..136 pt; one 10 pt line below it ends near 148 pt.
    assert!(after > 136.0 && after < 150.0, "afterwards at {after}");
}

#[test]
fn a_hanging_indent_does_not_wrap_into_the_margin_beside_a_shape() {
    // A shape filling the 523 pt column, anchored in an empty paragraph whose hanging
    // indent starts 18 pt left of the column: there is no gap beside the shape, so
    // the paragraph's line, then "afterwards", go below it (shape: 36..136 pt).
    let body = r#"<w:p><w:pPr><w:spacing w:after="0"/><w:ind w:hanging="360"/></w:pPr><w:r><w:drawing><wp:anchor xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" distL="0" distR="0"><wp:positionH relativeFrom="column"><wp:posOffset>0</wp:posOffset></wp:positionH><wp:positionV relativeFrom="paragraph"><wp:posOffset>0</wp:posOffset></wp:positionV><wp:extent cx="6642100" cy="1270000"/><wp:wrapSquare wrapText="bothSides"/></wp:anchor></w:drawing></w:r></w:p><w:p><w:r><w:t>afterwards</w:t></w:r></w:p><w:sectPr><w:pgSz w:w="11900" w:h="16840"/><w:pgMar w:top="720" w:bottom="720" w:left="720" w:right="720"/></w:sectPr>"#;
    let out = pages(docx(body, None, None));
    let after = out[0]
        .items
        .iter()
        .find_map(|i| match i {
            crate::office::pptx::scene::Item::Text(r) if r.text.trim() == "afterwards" => {
                Some(r.transform.ty)
            }
            _ => None,
        })
        .expect("afterwards");
    assert!(after > 150.0, "afterwards at {after}");
}

#[test]
fn a_text_shadow_makes_its_line_taller() {
    // Word for Mac probe, 12 pt Arial: a shadow 3 pt away with a 4 pt blur adds
    // 2.3 pt to the line pitch (13.8 to 16.1 pt).
    let shadow = r#"<w14:shadow xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml" w14:blurRad="50800" w14:dist="38100" w14:dir="2700000"/>"#;
    let lines = |effect: &str| -> String {
        (0..3)
            .map(|i| {
                format!(
                    r#"<w:p><w:pPr><w:spacing w:after="0"/></w:pPr><w:r><w:rPr><w:rFonts w:ascii="Arial" w:hAnsi="Arial"/><w:sz w:val="24"/>{effect}</w:rPr><w:t>line {i}</w:t></w:r></w:p>"#
                )
            })
            .collect()
    };
    let pitch = |effect: &str| {
        let ys = run_ys(&pages(docx(&lines(effect), None, None))[0]);
        f64::from(ys[1] - ys[0])
    };
    let (plain, shadowed) = (pitch(""), pitch(shadow));
    assert!(
        (shadowed - plain - 2.3).abs() < 0.1,
        "{plain} vs {shadowed}"
    );
}

#[test]
fn a_footer_text_box_hangs_below_the_page_bottom() {
    // A 144 pt non-wrapping text box anchored at the top of a footer paragraph near
    // the page bottom: Word draws it where the anchor puts it, its text at the
    // footer line, instead of lifting the box to fit inside the page.
    let body = r#"<w:p><w:r><w:t>body</w:t></w:r></w:p><w:sectPr><w:footerReference w:type="default" r:id="rfooter"/><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:bottom="1440" w:left="1440" w:right="1440" w:footer="720"/></w:sectPr>"#;
    let footer = format!(
        r#"<w:ftr xmlns:w="{}" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"><w:p><w:r><w:drawing><wp:anchor distT="0" distB="0" distL="0" distR="0" behindDoc="0"><wp:positionH relativeFrom="margin"><wp:align>right</wp:align></wp:positionH><wp:positionV relativeFrom="paragraph"><wp:posOffset>0</wp:posOffset></wp:positionV><wp:extent cx="1828800" cy="1828800"/><wp:wrapNone/><a:graphic><a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"><wps:wsp><wps:spPr><a:prstGeom prst="rect"/><a:noFill/></wps:spPr><wps:txbx><w:txbxContent><w:p><w:r><w:t>PAGENO</w:t></w:r></w:p></w:txbxContent></wps:txbx><wps:bodyPr lIns="0" tIns="0" rIns="0" bIns="0"/></wps:wsp></a:graphicData></a:graphic></wp:anchor></w:drawing></w:r></w:p></w:ftr>"#,
        crate::office::docx::testutil::W
    );
    let out = pages(crate::office::docx::testutil::with_related_part(
        docx(body, None, None),
        "footer",
        "footer.xml",
        &footer,
        false,
    ));
    let y = out[0]
        .items
        .iter()
        .find_map(|i| match i {
            crate::office::pptx::scene::Item::Text(r) if r.text == "PAGENO" => Some(r.transform.ty),
            _ => None,
        })
        .expect("footer text");
    // Fitted, the box would start at 792 - 144 = 648 pt; the footer line is near 750.
    assert!(y > 740.0, "PAGENO at {y}");
}

#[test]
fn compatibility_mode_11_moves_wrapped_tabs_below_a_tight_object() {
    // A 150 pt tall tight-wrapped shape leaves a 27 pt gap; a paragraph of three tabs
    // wraps there, and "P1" follows (Word for Mac probes, modes 11 and 15).
    let body = r#"<w:p><w:pPr><w:spacing w:after="0"/></w:pPr><w:r><w:drawing><wp:anchor xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" distL="0" distR="0"><wp:positionH relativeFrom="column"><wp:posOffset>0</wp:posOffset></wp:positionH><wp:positionV relativeFrom="paragraph"><wp:posOffset>0</wp:posOffset></wp:positionV><wp:extent cx="5388610" cy="1905000"/><wp:wrapTight wrapText="bothSides"><wp:wrapPolygon edited="0"><wp:start x="0" y="0"/><wp:lineTo x="0" y="21600"/><wp:lineTo x="21600" y="21600"/><wp:lineTo x="21600" y="0"/><wp:lineTo x="0" y="0"/></wp:wrapPolygon></wp:wrapTight></wp:anchor></w:drawing><w:t>A</w:t></w:r></w:p><w:p><w:pPr><w:spacing w:after="0"/></w:pPr><w:r><w:tab/><w:tab/><w:tab/></w:r></w:p><w:p><w:pPr><w:spacing w:after="0"/></w:pPr><w:r><w:t>P1</w:t></w:r></w:p><w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440"/></w:sectPr>"#;
    let p1 = |mode: u32| {
        let out = pages(crate::office::docx::testutil::with_compat_mode(
            docx(body, None, None),
            mode,
        ));
        out[0]
            .items
            .iter()
            .find_map(|i| match i {
                crate::office::pptx::scene::Item::Text(r) if r.text.trim() == "P1" => {
                    Some((r.transform.tx, r.transform.ty))
                }
                _ => None,
            })
            .unwrap()
    };
    let (beside, below) = (p1(15), p1(11));
    assert!(beside.0 > 490.0 && beside.1 < 222.0, "mode 15: {beside:?}");
    assert!(
        (below.0 - 72.0).abs() < 0.5 && below.1 > 222.0,
        "mode 11: {below:?}"
    );
}

#[test]
fn an_odd_page_section_restarting_at_one_follows_page_parity_with_even_odd_headers() {
    // A one-page cover, then an odd-page section whose numbering restarts at 1.
    let body = r#"<w:p><w:pPr><w:sectPr><w:type w:val="nextPage"/><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440"/></w:sectPr></w:pPr><w:r><w:t>cover</w:t></w:r></w:p><w:p><w:r><w:t>body</w:t></w:r></w:p><w:sectPr><w:type w:val="oddPage"/><w:pgNumType w:start="1"/><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440"/></w:sectPr>"#;
    // Duplex (even and odd headers): the section starts on physical page 3.
    let duplex = crate::office::docx::testutil::with_settings(
        docx(body, None, None),
        "<w:evenAndOddHeaders/>",
    );
    assert_eq!(pages(duplex).len(), 3);
    // Otherwise the restarted number 1 is odd already.
    assert_eq!(pages(docx(body, None, None)).len(), 2);
}

#[test]
fn a_next_page_section_restarting_numbering_keeps_parity_with_even_odd_headers() {
    // Page 1, then a new-page section restarting at `start`: with even and odd
    // headers a restart at 1 repeats page 1's parity, so a blank page comes first.
    let body = |start: u32| {
        format!(
            r#"<w:p><w:pPr><w:sectPr><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440"/></w:sectPr></w:pPr><w:r><w:t>first</w:t></w:r></w:p><w:p><w:r><w:t>body</w:t></w:r></w:p><w:sectPr><w:pgNumType w:start="{start}"/><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440"/></w:sectPr>"#
        )
    };
    let duplex = |start| {
        pages(crate::office::docx::testutil::with_settings(
            docx(&body(start), None, None),
            "<w:evenAndOddHeaders/>",
        ))
    };
    let out = duplex(1);
    assert_eq!(out.len(), 3);
    assert!(page_texts(&out[2]).contains("body"));
    assert_eq!((out[1].info.number, out[2].info.number), (2, 1));
    assert!(out[2].info.first_of_section);
    assert_eq!(duplex(2).len(), 2);
    assert_eq!(pages(docx(&body(1), None, None)).len(), 2);
    // A first section of type evenPage makes page 1 even: no blank page.
    let even_first = body(1).replacen("<w:sectPr>", r#"<w:sectPr><w:type w:val="evenPage"/>"#, 1);
    let out = pages(crate::office::docx::testutil::with_settings(
        docx(&even_first, None, None),
        "<w:evenAndOddHeaders/>",
    ));
    assert_eq!(out.len(), 2);
}

#[test]
fn a_break_only_paragraph_carries_its_mark_to_the_next_page_in_mode_11() {
    let body = format!(
        r#"{}<w:p><w:r><w:br w:type="page"/></w:r></w:p>{}"#,
        para("one"),
        para("two")
    );
    let top_of_two = |mode: u32| {
        let out = pages(crate::office::docx::testutil::with_compat_mode(
            docx(&body, None, None),
            mode,
        ));
        out[1]
            .items
            .iter()
            .find_map(|i| match i {
                crate::office::pptx::scene::Item::Text(r) if r.text == "two" => {
                    Some(r.transform.ty)
                }
                _ => None,
            })
            .unwrap()
    };
    // Word 2003 layout opens page 2 with the break paragraph's empty mark line.
    assert!(
        top_of_two(11) - top_of_two(15) > 10.0,
        "{} {}",
        top_of_two(11),
        top_of_two(15)
    );
}

#[test]
fn page_break_starts_a_new_page() {
    let body = format!(
        r#"{}<w:p><w:r><w:br w:type="page"/></w:r></w:p>{}"#,
        para("one"),
        para("two")
    );
    assert_eq!(pages(docx(&body, None, None)).len(), 2);
}

#[test]
fn side_wrapping_for_objects_behind_text_respects_the_experiment() {
    for wrap in ["wrapSquare", "wrapTight", "wrapNone"] {
        let polygon = if wrap == "wrapTight" {
            r#"<wp:wrapPolygon edited="0"><wp:start x="0" y="0"/><wp:lineTo x="0" y="21600"/><wp:lineTo x="21600" y="21600"/><wp:lineTo x="21600" y="0"/></wp:wrapPolygon>"#
        } else {
            ""
        };
        let body = format!(
            r#"<w:p><w:r><w:drawing>
            <wp:anchor xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"
                xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
                xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"
                behindDoc="1" relativeHeight="1" distT="0" distB="0" distL="0" distR="0">
                <wp:positionH relativeFrom="column"><wp:posOffset>0</wp:posOffset></wp:positionH>
                <wp:positionV relativeFrom="paragraph"><wp:posOffset>0</wp:posOffset></wp:positionV>
                <wp:extent cx="1270000" cy="1270000"/>
                <wp:{wrap}>{polygon}</wp:{wrap}>
                <a:graphic><a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingShape">
                    <wps:wsp><wps:spPr><a:prstGeom prst="rect"/>
                        <a:solidFill><a:srgbClr val="FF0000"/></a:solidFill>
                    </wps:spPr><wps:bodyPr/></wps:wsp>
                </a:graphicData></a:graphic>
            </wp:anchor></w:drawing></w:r><w:r><w:t>body text</w:t></w:r></w:p>"#
        );
        let out = pages(docx(&body, None, None));
        let (index, text) = out[0]
            .items
            .iter()
            .enumerate()
            .find_map(|(i, item)| match item {
                crate::office::pptx::scene::Item::Text(run) if run.text == "body text" => {
                    Some((i, run))
                }
                _ => None,
            })
            .expect("body text");
        let expected_x = if wrap == "wrapNone" { 72.0 } else { 172.0 };
        assert!((text.transform.tx - expected_x).abs() < 0.1, "{wrap}");
        assert!(index > 0, "the background shape is painted before the text");
    }
}

#[test]
fn page_break_before_and_sections() {
    let body = format!(
        r#"{}<w:p><w:pPr><w:pageBreakBefore/></w:pPr><w:r><w:t>b</w:t></w:r></w:p><w:p><w:pPr><w:sectPr/></w:pPr></w:p>{}"#,
        para("a"),
        para("c")
    );
    assert_eq!(pages(docx(&body, None, None)).len(), 3);
}

#[test]
fn many_lines_overflow_to_more_pages() {
    let body: String = (0..120).map(|i| para(&format!("line {i}"))).collect();
    let p = pages(docx(&body, None, None));
    // 10 pt Times New Roman single lines (~11.5 pt) on a 648 pt body: 56 per page.
    assert_eq!(p.len(), 3);
}

#[test]
fn keep_with_next_moves_heading_to_next_page() {
    // Fill the first page to its last line, then a keep-with-next heading.
    let fill: String = (0..55).map(|i| para(&format!("line {i}"))).collect();
    let heading = r#"<w:p><w:pPr><w:keepNext/></w:pPr><w:r><w:t>heading</w:t></w:r></w:p>"#;
    let body = format!("{fill}{heading}{}", para("body"));
    let p = pages(docx(&body, None, None));
    assert_eq!(p.len(), 2);
    let first_page_text: Vec<String> = p[0]
        .items
        .iter()
        .filter_map(|i| match i {
            crate::office::pptx::scene::Item::Text(r) => Some(r.text.clone()),
            _ => None,
        })
        .collect();
    assert!(!first_page_text.iter().any(|t| t.contains("heading")));
}

fn run_ys(p: &crate::office::docx::paginate::OutPage) -> Vec<f32> {
    p.items
        .iter()
        .filter_map(|i| match i {
            crate::office::pptx::scene::Item::Text(r) => Some(r.transform.ty),
            _ => None,
        })
        .collect()
}

#[test]
fn paragraph_spacing_collapses_html_style() {
    // 12 pt after, then 12 pt before: Word leaves 12 pt between them, not 24.
    let body = r#"<w:p><w:pPr><w:spacing w:before="0" w:after="240"/></w:pPr><w:r><w:t>a</w:t></w:r></w:p><w:p><w:pPr><w:spacing w:before="240" w:after="0"/></w:pPr><w:r><w:t>b</w:t></w:r></w:p>"#;
    let p = pages(docx(body, None, None));
    let ys = run_ys(&p[0]);
    // Times New Roman 10 pt single line: 11.5 pt.
    let gap = (ys[1] - ys[0]) as f64;
    assert!((gap - (11.5 + 12.0)).abs() < 0.2, "gap {gap}");
}

#[test]
fn footnotes_reserve_their_room() {
    let note_text = "note ".repeat(400);
    let fill: String = (0..50).map(|i| para(&format!("line {i}"))).collect();
    let with_ref =
        r#"<w:p><w:r><w:t>ref</w:t></w:r><w:r><w:footnoteReference w:id="1"/></w:r></w:p>"#;
    let body = format!("{fill}{with_ref}");
    let mut bytes = docx(&body, None, None);
    // Add a footnotes part with one long note.
    bytes = crate::office::docx::testutil::with_footnote(bytes, &note_text);
    let p = pages(bytes);
    // Without the note the 51 lines fit one page; with it the reference moves on.
    assert_eq!(p.len(), 2);
}

#[test]
fn page_break_line_at_the_bottom_adds_no_blank_page() {
    // A full first page (56 lines), then a paragraph holding only a page break.
    let fill: String = (0..56).map(|i| para(&format!("line {i}"))).collect();
    let body = format!(
        r#"{fill}<w:p><w:r><w:br w:type="page"/></w:r></w:p>{}"#,
        para("next")
    );
    assert_eq!(pages(docx(&body, None, None)).len(), 2);
}

fn page_texts(p: &crate::office::docx::paginate::OutPage) -> String {
    p.items
        .iter()
        .filter_map(|i| match i {
            crate::office::pptx::scene::Item::Text(r) => Some(r.text.clone()),
            _ => None,
        })
        .collect()
}

fn row(text: &str, keep: bool) -> String {
    let k = if keep { "<w:keepNext/>" } else { "" };
    format!(
        r#"<w:tr><w:tc><w:p><w:pPr>{k}</w:pPr><w:r><w:t>{text}</w:t></w:r></w:p></w:tc></w:tr>"#
    )
}

#[test]
fn rows_with_keep_with_next_stay_together() {
    // 54 lines leave room for two rows only; the three rows keep together.
    let fill: String = (0..54).map(|i| para(&format!("line {i}"))).collect();
    let table = format!(
        r#"<w:tbl><w:tblGrid><w:gridCol w:w="4000"/></w:tblGrid>{}{}{}</w:tbl>"#,
        row("first", true),
        row("second", true),
        row("third", false)
    );
    let body = format!("{fill}{table}{}", para("after"));
    let p = pages(docx(&body, None, None));
    assert_eq!(p.len(), 2);
    assert!(page_texts(&p[1]).contains("first"));
}

#[test]
fn a_split_cell_leaves_no_widow() {
    // A one-cell row of three lines starting two lines above the bottom:
    // the split moves two lines on, not one.
    let fill: String = (0..54).map(|i| para(&format!("line {i}"))).collect();
    let cell =
        r#"<w:tc><w:p><w:r><w:t>a</w:t><w:br/><w:t>b</w:t><w:br/><w:t>c</w:t></w:r></w:p></w:tc>"#;
    let table = format!(
        r#"<w:tbl><w:tblGrid><w:gridCol w:w="4000"/></w:tblGrid><w:tr>{cell}</w:tr></w:tbl>"#
    );
    let body = format!("{fill}{table}");
    let p = pages(docx(&body, None, None));
    assert!(page_texts(&p[1]).contains('b'));
}

#[test]
fn table_cells_ignore_the_line_grid_without_adjust_line_height() {
    // A 24 pt line grid; without `adjustLineHeightInTable` cell lines keep
    // their own height (10 pt Times New Roman, about 11.5 pt).
    let table = format!(
        r#"<w:tbl><w:tblGrid><w:gridCol w:w="4000"/></w:tblGrid>{}{}</w:tbl>"#,
        row("a", false),
        row("b", false)
    );
    let sect = r#"<w:sectPr><w:docGrid w:type="lines" w:linePitch="480"/></w:sectPr>"#;
    let body = format!("{table}{}{sect}", para("x"));
    let p = pages(docx(&body, None, None));
    let ys = run_ys(&p[0]);
    let gap = (ys[1] - ys[0]) as f64;
    assert!(gap < 15.0, "row pitch {gap}");
}

#[test]
fn empty_break_paragraph_into_columns_takes_no_room() {
    // The empty paragraph holding a continuous break into two columns is not a line.
    let brk = r#"<w:p><w:pPr><w:sectPr><w:cols w:space="720"/></w:sectPr></w:pPr></w:p>"#;
    let last =
        r#"<w:sectPr><w:type w:val="continuous"/><w:cols w:num="2" w:space="720"/></w:sectPr>"#;
    let body = format!("{}{brk}{}{last}", para("title"), para("column"));
    let p = pages(docx(&body, None, None));
    let ys = run_ys(&p[0]);
    let gap = (ys[1] - ys[0]) as f64;
    assert!(gap < 15.0, "gap {gap}");
}

#[test]
fn row_property_exceptions_set_cell_margins() {
    // The second row's `w:tblPrEx` gives its cells 10 pt top and bottom margins.
    let prex = r#"<w:tblPrEx><w:tblCellMar><w:top w:w="200" w:type="dxa"/><w:bottom w:w="200" w:type="dxa"/></w:tblCellMar></w:tblPrEx>"#;
    let table = format!(
        r#"<w:tbl><w:tblGrid><w:gridCol w:w="4000"/></w:tblGrid>{}<w:tr>{prex}<w:tc><w:p><w:r><w:t>b</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#,
        row("a", false)
    );
    let body = format!("{table}{}", para("after"));
    let p = pages(docx(&body, None, None));
    let ys = run_ys(&p[0]);
    let gap = (ys[2] - ys[1]) as f64;
    assert!(gap > 20.0, "gap {gap}");
}

#[test]
fn borders_add_to_a_minimum_row_height() {
    // 15 pt minimum rows with 0.5 pt borders: rows are 15.5 pt apart.
    let tr = |t: &str| {
        format!(
            r#"<w:tr><w:trPr><w:trHeight w:val="300"/></w:trPr><w:tc><w:p><w:r><w:t>{t}</w:t></w:r></w:p></w:tc></w:tr>"#
        )
    };
    let borders = r#"<w:tblPr><w:tblBorders><w:top w:val="single" w:sz="4"/><w:bottom w:val="single" w:sz="4"/><w:insideH w:val="single" w:sz="4"/></w:tblBorders></w:tblPr>"#;
    let table = format!(
        r#"<w:tbl>{borders}<w:tblGrid><w:gridCol w:w="4000"/></w:tblGrid>{}{}</w:tbl>"#,
        tr("a"),
        tr("b")
    );
    let p = pages(docx(&format!("{table}{}", para("x")), None, None));
    let ys = run_ys(&p[0]);
    let gap = (ys[1] - ys[0]) as f64;
    assert!((gap - 15.5).abs() < 0.05, "row pitch {gap}");
}

#[test]
fn body_page_fields_show_their_saved_result() {
    let body = r#"<w:p><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText> PAGE </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>7</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p>"#;
    let p = pages(docx(body, None, None));
    assert!(page_texts(&p[0]).contains('7'));
}

#[test]
fn footnotes_referenced_in_table_cells_are_drawn() {
    let cell = r#"<w:tc><w:p><w:r><w:t>cell</w:t></w:r><w:r><w:footnoteReference w:id="1"/></w:r></w:p></w:tc>"#;
    let body = format!(
        r#"<w:tbl><w:tblGrid><w:gridCol w:w="4000"/></w:tblGrid><w:tr>{cell}</w:tr></w:tbl>{}"#,
        para("after")
    );
    let bytes = crate::office::docx::testutil::with_footnote(docx(&body, None, None), "the note");
    let p = pages(bytes);
    assert!(page_texts(&p[0]).contains("the note"));
}

#[test]
fn row_whose_text_fits_moves_whole() {
    // A row of one line but 60 pt minimum height, with 34 pt left on the page.
    let fill: String = (0..53).map(|i| para(&format!("line {i}"))).collect();
    let row = r#"<w:tbl><w:tblGrid><w:gridCol w:w="4000"/></w:tblGrid><w:tr><w:trPr><w:trHeight w:val="1200"/></w:trPr><w:tc><w:p><w:r><w:t>cell</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#;
    let body = format!("{fill}{row}{}", para("end"));
    let p = pages(docx(&body, None, None));
    assert_eq!(p.len(), 2);
    assert!(!page_texts(&p[0]).contains("cell"));
    assert!(page_texts(&p[1]).contains("cell"));
}

#[test]
fn merged_cell_across_pages_stays_on_its_pages() {
    // A centred vertical merge over 80 rows: its text runs down its part on each page.
    let mut rows = String::new();
    for i in 0..80 {
        let merge = if i == 0 {
            r#"<w:vMerge w:val="restart"/>"#
        } else {
            "<w:vMerge/>"
        };
        let text = if i == 0 {
            (0..30)
                .map(|k| para(&format!("merged {k}")))
                .collect::<String>()
        } else {
            "<w:p/>".to_string()
        };
        rows.push_str(&format!(
            r#"<w:tr><w:tc><w:tcPr>{merge}<w:vAlign w:val="center"/></w:tcPr>{text}</w:tc><w:tc>{}</w:tc></w:tr>"#,
            para(&format!("row {i}"))
        ));
    }
    let body = format!(
        r#"<w:tbl><w:tblGrid><w:gridCol w:w="3000"/><w:gridCol w:w="3000"/></w:tblGrid>{rows}</w:tbl>"#
    );
    let p = pages(docx(&body, None, None));
    assert!(p.len() >= 2);
    for page in &p {
        for y in run_ys(page) {
            assert!(y > 0.0 && (y as f64) < page.info.h, "text at {y}");
        }
    }
    let merged: usize = p
        .iter()
        .map(|pg| page_texts(pg).matches("merged").count())
        .sum();
    assert_eq!(merged, 30);
}

#[test]
fn horizontal_rule_line_keeps_the_mark_height() {
    let hr = r##"<w:p><w:r><w:pict xmlns:v="urn:schemas-microsoft-com:vml" xmlns:o="urn:schemas-microsoft-com:office:office"><v:rect style="width:0;height:1.5pt" o:hr="t" o:hrstd="t" fillcolor="#a0a0a0" stroked="f"/></w:pict></w:r></w:p>"##;
    let body = format!("{}{hr}{}", para("a"), para("b"));
    let p = pages(docx(&body, None, None));
    let ys = run_ys(&p[0]);
    // Two Times New Roman 10 pt lines (11.5 pt each) between the baselines.
    let gap = (ys[ys.len() - 1] - ys[0]) as f64;
    assert!((gap - 23.0).abs() < 0.3, "gap {gap}");
}

#[test]
fn a_cell_of_one_line_paragraphs_leaves_one_line_before_a_page_break() {
    // Room for one line of a three-paragraph cell: the row splits after "cell 0"
    // (Word for Mac splits an address cell of one-line paragraphs that way).
    let fill: String = (0..55).map(|i| para(&format!("line {i}"))).collect();
    let cell: String = (0..3).map(|i| para(&format!("cell {i}"))).collect();
    let row = format!(
        r#"<w:tbl><w:tblGrid><w:gridCol w:w="4000"/></w:tblGrid><w:tr><w:tc>{cell}</w:tc></w:tr></w:tbl>"#
    );
    let p = pages(docx(&format!("{fill}{row}"), None, None));
    assert_eq!(p.len(), 2);
    assert!(page_texts(&p[0]).contains("cell 0"));
    assert!(!page_texts(&p[0]).contains("cell 1"));
}

#[test]
fn an_empty_section_end_paragraph_adds_no_page() {
    // A full page, then the empty paragraph holding a next-page section break.
    let fill: String = (0..56).map(|i| para(&format!("line {i}"))).collect();
    let body = format!(
        r#"{fill}<w:p><w:pPr><w:sectPr/></w:pPr></w:p>{}"#,
        para("next section")
    );
    assert_eq!(pages(docx(&body, None, None)).len(), 2);
}

#[test]
fn a_page_anchored_band_moves_earlier_text_below_it() {
    // The third paragraph anchors a full-width top-and-bottom shape 50 pt from
    // the page top, 300 pt tall: the first two paragraphs go below it.
    let shape = r#"<w:r><w:drawing><wp:anchor xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" distT="0" distB="0" distL="0" distR="0" simplePos="0" relativeHeight="1" behindDoc="0" locked="0" layoutInCell="1" allowOverlap="1"><wp:simplePos x="0" y="0"/><wp:positionH relativeFrom="page"><wp:posOffset>0</wp:posOffset></wp:positionH><wp:positionV relativeFrom="page"><wp:posOffset>635000</wp:posOffset></wp:positionV><wp:extent cx="7772400" cy="3810000"/><wp:wrapTopAndBottom/><wp:docPr id="1" name="s"/><a:graphic xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"><wps:wsp xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"><wps:spPr><a:prstGeom prst="rect"/><a:solidFill><a:srgbClr val="FF0000"/></a:solidFill></wps:spPr><wps:bodyPr/></wps:wsp></a:graphicData></a:graphic></wp:anchor></w:drawing></w:r>"#;
    let fill: String = (0..20).map(|i| para(&format!("line {i}"))).collect();
    let body = format!(
        "{}{}<w:p>{shape}<w:r><w:t>anchor</w:t></w:r></w:p>{fill}",
        para("first"),
        para("second")
    );
    let p = pages(docx(&body, None, None));
    let first_y = p[0]
        .items
        .iter()
        .find_map(|i| match i {
            crate::office::pptx::scene::Item::Text(r) if r.text.contains("first") => {
                Some(r.transform.ty)
            }
            _ => None,
        })
        .unwrap();
    assert!(first_y as f64 > 350.0, "first line at {first_y}");
}

fn text_y(p: &crate::office::docx::paginate::OutPage, needle: &str) -> f64 {
    p.items
        .iter()
        .find_map(|i| match i {
            crate::office::pptx::scene::Item::Text(r) if r.text.contains(needle) => {
                Some(r.transform.ty as f64)
            }
            _ => None,
        })
        .unwrap()
}

#[test]
fn a_page_break_in_a_cell_does_nothing() {
    let row = r#"<w:tbl><w:tblGrid><w:gridCol w:w="4000"/></w:tblGrid><w:tr><w:tc><w:p><w:r><w:br w:type="page"/></w:r><w:r><w:t>cell</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#;
    let body = format!("{row}{}", para("after"));
    let p = pages(docx(&body, None, None));
    assert_eq!(p.len(), 1);
    // "cell" stays on the row's first line, below the 72 pt top margin.
    let y = text_y(&p[0], "cell");
    assert!(y < 85.0, "cell at {y}");
}

#[test]
fn a_hidden_mark_cell_takes_no_height() {
    // Each row: a text cell beside a `hideMark` cell holding one empty
    // paragraph of 30 pt text; the rows are as tall as the text cell.
    let big = r#"<w:p><w:pPr><w:rPr><w:sz w:val="60"/></w:rPr></w:pPr></w:p>"#;
    let row = |t: &str| {
        format!(
            r#"<w:tr><w:tc>{}</w:tc><w:tc><w:tcPr><w:hideMark/></w:tcPr>{big}</w:tc></w:tr>"#,
            para(t)
        )
    };
    let body = format!(
        r#"<w:tbl><w:tblGrid><w:gridCol w:w="3000"/><w:gridCol w:w="3000"/></w:tblGrid>{}{}</w:tbl>"#,
        row("one"),
        row("two")
    );
    let p = pages(docx(&body, None, None));
    let gap = text_y(&p[0], "two") - text_y(&p[0], "one");
    assert!(gap < 15.0, "gap {gap}");
}

#[test]
fn a_hidden_mark_cell_holding_a_space_keeps_the_marks_line() {
    // The `hideMark` cell's paragraph holds a 30 pt space; spaces do not size a line,
    // so the row keeps the mark's line (Word for Mac probes, modes 11 and 15).
    let big = r#"<w:p><w:r><w:rPr><w:sz w:val="60"/></w:rPr><w:t xml:space="preserve"> </w:t></w:r></w:p>"#;
    let row = |t: &str| {
        format!(
            r#"<w:tr><w:tc>{}</w:tc><w:tc><w:tcPr><w:hideMark/></w:tcPr>{big}</w:tc></w:tr>"#,
            para(t)
        )
    };
    let body = format!(
        r#"<w:tbl><w:tblGrid><w:gridCol w:w="3000"/><w:gridCol w:w="3000"/></w:tblGrid>{}{}</w:tbl>"#,
        row("one"),
        row("two")
    );
    let p = pages(docx(&body, None, None));
    let gap = text_y(&p[0], "two") - text_y(&p[0], "one");
    assert!(gap > 5.0 && gap < 20.0, "gap {gap}");
}

#[test]
fn spacing_sees_paragraphs_around_a_floating_table() {
    // Contextual spacing between same-style paragraphs ignores the floating
    // table between them.
    let style = r#"<w:pPr><w:spacing w:after="400"/><w:contextualSpacing/></w:pPr>"#;
    let tbl = r#"<w:tbl><w:tblPr><w:tblpPr w:vertAnchor="text" w:horzAnchor="margin" w:tblpY="0"/></w:tblPr><w:tblGrid><w:gridCol w:w="2000"/></w:tblGrid><w:tr><w:tc><w:p><w:r><w:t>float</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#;
    let body = format!(
        r#"<w:p>{style}<w:r><w:t>one</w:t></w:r></w:p>{tbl}<w:p>{style}<w:r><w:t>two</w:t></w:r></w:p>"#
    );
    let p = pages(docx(&body, None, None));
    let gap = text_y(&p[0], "two") - text_y(&p[0], "one");
    assert!(gap < 15.0, "gap {gap}");
}

#[test]
fn a_between_border_takes_its_space_twice() {
    // Two empty paragraphs joined by a between border (1 pt space, 0.5 pt
    // line) sit between two lines of text: 2.5 pt more than without it.
    let empty = |bdr: &str| format!(r#"<w:p><w:pPr>{bdr}</w:pPr></w:p>"#);
    let b = r#"<w:pBdr><w:between w:val="single" w:sz="4" w:space="1" w:color="auto"/></w:pBdr>"#;
    let gap = |bdr: &str| {
        let body = format!("{}{}{}{}", para("one"), empty(bdr), empty(bdr), para("two"));
        let p = pages(docx(&body, None, None));
        text_y(&p[0], "two") - text_y(&p[0], "one")
    };
    let d = gap(b) - gap("");
    assert!((d - 2.5).abs() < 0.05, "extra {d}");
}

#[test]
fn contour_wrapping_places_text_beside_the_slanted_edge() {
    let body = r#"<w:p><w:r><w:drawing><wp:anchor xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape" behindDoc="0"><wp:positionH relativeFrom="column"><wp:posOffset>0</wp:posOffset></wp:positionH><wp:positionV relativeFrom="paragraph"><wp:posOffset>0</wp:posOffset></wp:positionV><wp:extent cx="1270000" cy="1270000"/><wp:wrapTight wrapText="bothSides"><wp:wrapPolygon><wp:start x="10800" y="0"/><wp:lineTo x="21600" y="21600"/><wp:lineTo x="0" y="21600"/></wp:wrapPolygon></wp:wrapTight><a:graphic><a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"><wps:wsp><wps:spPr><a:prstGeom prst="triangle"/></wps:spPr><wps:bodyPr/></wps:wsp></a:graphicData></a:graphic></wp:anchor></w:drawing></w:r><w:r><w:t>beside triangle</w:t></w:r></w:p>"#;
    let out = pages(docx(body, None, None));
    let x = out[0]
        .items
        .iter()
        .find_map(|i| match i {
            crate::office::pptx::scene::Item::Text(run) if run.text == "beside triangle" => {
                Some(run.transform.tx)
            }
            _ => None,
        })
        .unwrap();
    {
        assert!(x > 122.0 && x < 145.0, "text x={x}");
    }
}

#[test]
fn automatic_row_uses_shared_vertical_padding_for_top_aligned_cells() {
    let body = r#"<w:tbl><w:tblGrid><w:gridCol w:w="3000"/><w:gridCol w:w="3000"/></w:tblGrid><w:tr><w:tc><w:tcPr><w:tcMar><w:top w:w="0" w:type="dxa"/><w:bottom w:w="0" w:type="dxa"/></w:tcMar></w:tcPr><w:p><w:r><w:t>first</w:t></w:r></w:p></w:tc><w:tc><w:tcPr><w:tcMar><w:top w:w="200" w:type="dxa"/><w:bottom w:w="100" w:type="dxa"/></w:tcMar></w:tcPr><w:p><w:r><w:t>second</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#;
    let out = pages(docx(body, None, None));
    let ys = run_ys(&out[0]);
    assert!((ys[1] - ys[0]).abs() < 0.01);
}

#[test]
fn footer_frame_uses_the_following_paragraph_without_doubling_footer_height() {
    let render = |framed: bool| {
        let body = r#"<w:p><w:r><w:t>body</w:t></w:r></w:p><w:sectPr><w:footerReference w:type="default" r:id="rfooter"/><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:bottom="720" w:left="1440" w:right="1440" w:footer="432"/></w:sectPr>"#;
        let frame = if framed {
            r#"<w:framePr w:wrap="around" w:vAnchor="text" w:hAnchor="margin" w:xAlign="center" w:y="1"/>"#
        } else {
            ""
        };
        let tail = if framed { "<w:p/>" } else { "" };
        let footer = format!(
            r#"<w:ftr xmlns:w="{}"><w:p><w:pPr>{frame}</w:pPr><w:r><w:t>footer</w:t></w:r></w:p>{tail}</w:ftr>"#,
            crate::office::docx::testutil::W
        );
        let out = pages(crate::office::docx::testutil::with_related_part(
            docx(body, None, None),
            "footer",
            "footer.xml",
            &footer,
            false,
        ));
        out[0]
            .items
            .iter()
            .find_map(|i| match i {
                crate::office::pptx::scene::Item::Text(r) if r.text == "footer" => {
                    Some(r.transform.ty)
                }
                _ => None,
            })
            .unwrap()
    };
    let delta = render(true) - render(false);
    {
        assert!(
            (delta - 0.05).abs() < 0.02,
            "text-relative footer offset {delta}"
        );
    }
}

fn outer_border_table(text: &str, bordered: bool, floating: bool) -> String {
    let borders = if bordered {
        r#"<w:tblBorders><w:top w:val="double" w:sz="16"/><w:bottom w:val="single" w:sz="32"/></w:tblBorders>"#
    } else {
        ""
    };
    let float = if floating {
        r#"<w:tblpPr w:vertAnchor="page" w:horzAnchor="margin" w:tblpY="2000"/>"#
    } else {
        ""
    };
    format!(
        r#"<w:tbl><w:tblPr>{borders}{float}</w:tblPr><w:tblGrid><w:gridCol w:w="9360"/></w:tblGrid><w:tr><w:trPr><w:trHeight w:val="1000" w:hRule="exact"/><w:cantSplit/></w:trPr><w:tc><w:p><w:r><w:t>{text}</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#
    )
}

fn outer_text_y(out: &[super::OutPage], text: &str) -> f64 {
    out.iter()
        .flat_map(|p| &p.items)
        .find_map(|item| match item {
            crate::office::pptx::scene::Item::Text(r) if r.text == text => {
                Some(r.transform.ty as f64)
            }
            _ => None,
        })
        .unwrap()
}

#[test]
fn consecutive_tables_reserve_outer_bands_without_stretching_rows() {
    let render = |bordered| {
        pages(docx(
            &(outer_border_table("first", bordered, false)
                + &outer_border_table("second", bordered, false)
                + &para("after")),
            None,
            None,
        ))
    };
    let plain = render(false);
    let bordered = render(true);

    for text in ["first", "second", "after"] {
        let delta = outer_text_y(&bordered, text) - outer_text_y(&plain, text);
        assert!(delta.abs() < 0.02, "{text}: {delta}");
    }
}

#[test]
fn outer_bands_participate_in_row_page_fit() {
    let before = r#"<w:p><w:pPr><w:spacing w:after="0" w:line="200" w:lineRule="exact"/></w:pPr><w:r><w:t>before</w:t></w:r></w:p>"#;
    let sect = r#"<w:sectPr><w:pgSz w:w="12240" w:h="4160"/><w:pgMar w:top="1440" w:bottom="1440" w:left="1440" w:right="1440"/></w:sectPr>"#;
    let out = pages(docx(
        &format!("{before}{}{sect}", outer_border_table("row", true, false)),
        None,
        None,
    ));

    assert_eq!(out.len(), { 1 });
}

#[test]
fn header_tables_reserve_the_same_outer_bands_as_body_tables() {
    let render = |bordered| {
        let body = r#"<w:p><w:r><w:t>body</w:t></w:r></w:p><w:sectPr><w:headerReference w:type="default" r:id="rheader"/><w:pgMar w:top="20" w:bottom="1440" w:left="1440" w:right="1440" w:header="200"/></w:sectPr>"#;
        let xml = format!(
            r#"<w:hdr xmlns:w="{}">{}</w:hdr>"#,
            crate::office::docx::testutil::W,
            outer_border_table("header", bordered, false)
        );
        pages(crate::office::docx::testutil::with_related_part(
            docx(body, None, None),
            "header",
            "header.xml",
            &xml,
            false,
        ))
    };
    let delta = outer_text_y(&render(true), "body") - outer_text_y(&render(false), "body");

    assert!(delta.abs() < 0.02, "{delta}");
}

#[test]
fn floating_table_band_includes_its_outer_edges() {
    let render = |bordered| {
        let sect = r#"<w:sectPr><w:pgMar w:top="2000" w:bottom="1440" w:left="1440" w:right="1440"/></w:sectPr>"#;
        pages(docx(
            &format!(
                "{}{}{sect}",
                outer_border_table("float", bordered, true),
                para("body")
            ),
            None,
            None,
        ))
    };
    let plain = render(false);
    let bordered = render(true);

    let delta = outer_text_y(&bordered, "body") - outer_text_y(&plain, "body");
    assert!(delta.abs() < 0.02, "{delta}");
}

#[test]
fn merge_border_adjacency_respects_page_column_and_row_identity() {
    use super::{Pieces, adjacent_row};
    let pieces = Pieces::from([((7, 3), vec![(0, 72.0, 120.0, 20.0, 0)])]);
    assert!(adjacent_row(&pieces, (7, 3), (0, 72.0, 120.0)));
    assert!(!adjacent_row(&pieces, (7, 3), (1, 72.0, 120.0)));
    assert!(!adjacent_row(&pieces, (7, 3), (0, 300.0, 120.0)));
    assert!(!adjacent_row(&pieces, (7, 3), (0, 72.0, 100.0)));
    assert!(!adjacent_row(&pieces, (8, 3), (0, 72.0, 120.0)));
    assert!(!adjacent_row(&pieces, (7, 2), (0, 72.0, 120.0)));
}

#[test]
fn empty_no_number_level_does_not_insert_a_tab_before_heading_text() {
    let numbering = r#"<w:abstractNum w:abstractNumId="0"><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="none"/><w:lvlText w:val=""/><w:pPr><w:ind w:left="720" w:hanging="360"/></w:pPr></w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num>"#;
    let styles = r#"<w:style w:type="paragraph" w:styleId="Heading"><w:pPr><w:numPr><w:numId w:val="1"/></w:numPr></w:pPr></w:style>"#;
    let body = r#"<w:p><w:pPr><w:pStyle w:val="Heading"/><w:ind w:left="0" w:firstLine="0"/></w:pPr><w:r><w:t>Heading text</w:t></w:r></w:p>"#;
    let out = pages(docx(body, Some(styles), Some(numbering)));
    let x = out[0]
        .items
        .iter()
        .find_map(|i| match i {
            crate::office::pptx::scene::Item::Text(r) if r.text == "Heading text" => {
                Some(r.transform.tx as f64)
            }
            _ => None,
        })
        .unwrap();

    assert!((x - { 72.0 }).abs() < 0.01, "x={x}");
}

#[test]
fn wide_floating_table_uses_the_available_side_gap() {
    // Word for Mac: a 300 pt table leaves room beside it on either side; a full-width
    // one sends the text below. (width, offset, text beside?)
    for (width, offset, beside) in [(6000, 0, true), (6000, 3360, true), (9360, 0, false)] {
        let body = format!(
            r#"<w:tbl><w:tblPr><w:tblpPr w:vertAnchor="text" w:horzAnchor="margin" w:tblpY="0" w:tblpX="{offset}"/><w:tblW w:w="{width}" w:type="dxa"/><w:tblLayout w:type="fixed"/></w:tblPr><w:tblGrid><w:gridCol w:w="{width}"/></w:tblGrid><w:tr><w:trPr><w:trHeight w:val="2000" w:hRule="exact"/></w:trPr><w:tc><w:tcPr><w:tcW w:w="{width}" w:type="dxa"/></w:tcPr><w:p><w:r><w:t>table</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:p><w:r><w:t>following</w:t></w:r></w:p>"#
        );
        let out = pages(docx(&body, None, None));
        let positions: Vec<_> = out[0]
            .items
            .iter()
            .filter_map(|i| match i {
                crate::office::pptx::scene::Item::Text(r) if r.text == "following" => {
                    Some((r.transform.tx, r.transform.ty))
                }
                _ => None,
            })
            .collect();
        assert_eq!(positions.len(), 1);
        let (x, y) = positions[0];
        if beside {
            assert!(y < 100.0, "x={x}, y={y}");
            let expected = if offset == 0 { 373.0 } else { 72.0 };
            assert!((x - expected).abs() < 2.0, "x={x}, y={y}");
        } else {
            assert!(x < 100.0 && y >= 172.0, "x={x}, y={y}");
        }
    }
}

#[test]
fn a_top_and_bottom_header_object_lowers_the_body() {
    // Word 2013+ layout: a 200 pt tall top-and-bottom box anchored in the header's only
    // paragraph, 36 pt from the top, moves the header's line below the box, and the
    // body starts below that line, past the 72 pt top margin.
    let body = r#"<w:p><w:r><w:t>body</w:t></w:r></w:p><w:sectPr><w:headerReference w:type="default" r:id="rheader"/><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:bottom="1440" w:left="1440" w:right="1440" w:header="720"/></w:sectPr>"#;
    let header = format!(
        r#"<w:hdr xmlns:w="{}"><w:p><w:r><w:drawing><wp:anchor xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" distT="0" distB="0" distL="0" distR="0" simplePos="0" relativeHeight="1" behindDoc="0" locked="0" layoutInCell="1" allowOverlap="1"><wp:simplePos x="0" y="0"/><wp:positionH relativeFrom="column"><wp:posOffset>0</wp:posOffset></wp:positionH><wp:positionV relativeFrom="paragraph"><wp:posOffset>0</wp:posOffset></wp:positionV><wp:extent cx="1905000" cy="2540000"/><wp:wrapTopAndBottom/><wp:docPr id="1" name="s"/><a:graphic xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"><wps:wsp xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"><wps:spPr><a:prstGeom prst="rect"/><a:solidFill><a:srgbClr val="FF0000"/></a:solidFill></wps:spPr><wps:bodyPr/></wps:wsp></a:graphicData></a:graphic></wp:anchor></w:drawing></w:r></w:p></w:hdr>"#,
        crate::office::docx::testutil::W
    );
    let out = pages(crate::office::docx::testutil::with_related_part(
        crate::office::docx::testutil::with_compat_mode(docx(body, None, None), 15),
        "header",
        "header.xml",
        &header,
        false,
    ));
    let y = out[0]
        .items
        .iter()
        .find_map(|i| match i {
            crate::office::pptx::scene::Item::Text(r) if r.text == "body" => Some(r.transform.ty),
            _ => None,
        })
        .unwrap();
    assert!(y > 36.0 + 200.0 + 12.0, "{y}");
}

#[test]
fn a_section_break_paragraph_ignores_its_page_break_before() {
    // A cover page whose section-break paragraph has page break before, then a new
    // page section: Word keeps the break paragraph on the cover, two pages in all.
    let body = format!(
        r#"{}<w:p><w:pPr><w:pageBreakBefore/><w:sectPr/></w:pPr></w:p>{}"#,
        para("cover"),
        para("contents")
    );
    let out = pages(docx(&body, None, None));
    assert_eq!(out.len(), 2);
}

#[test]
fn text_clears_a_floating_table_wider_than_its_first_row() {
    // A full-width floating table whose first row covers only its 200 pt first
    // column: the following text goes below the whole table, not beside that row.
    let cell = |w: u32| {
        format!(
            r#"<w:tc><w:tcPr><w:tcW w:w="{w}" w:type="dxa"/></w:tcPr><w:p><w:r><w:t>cell</w:t></w:r></w:p></w:tc>"#
        )
    };
    let body = format!(
        r#"<w:tbl><w:tblPr><w:tblpPr w:vertAnchor="text" w:horzAnchor="margin" w:tblpY="0"/><w:tblLayout w:type="fixed"/></w:tblPr><w:tblGrid><w:gridCol w:w="4000"/><w:gridCol w:w="5360"/></w:tblGrid><w:tr><w:trPr><w:gridAfter w:val="1"/></w:trPr>{}</w:tr><w:tr>{}{}</w:tr></w:tbl><w:p><w:r><w:t>following</w:t></w:r></w:p>"#,
        cell(4000),
        cell(4000),
        cell(5360)
    );
    let out = pages(docx(&body, None, None));
    let ys = |text: &str| -> Vec<f32> {
        out[0]
            .items
            .iter()
            .filter_map(|i| match i {
                crate::office::pptx::scene::Item::Text(r) if r.text == text => Some(r.transform.ty),
                _ => None,
            })
            .collect()
    };
    let last_cell = ys("cell").into_iter().fold(f32::MIN, f32::max);
    assert!(
        ys("following")[0] > last_cell,
        "{:?} vs {last_cell}",
        ys("following")
    );
}

#[test]
fn text_after_a_clearing_break_starts_below_a_floating_table() {
    // A 150 pt wide, 100 pt tall floating table with text running beside it; a line
    // break with clear="all" sends the next paragraph below the table.
    let table = r#"<w:tbl><w:tblPr><w:tblpPr w:vertAnchor="text" w:horzAnchor="margin" w:tblpY="0"/><w:tblW w:w="3000" w:type="dxa"/><w:tblLayout w:type="fixed"/></w:tblPr><w:tblGrid><w:gridCol w:w="3000"/></w:tblGrid><w:tr><w:trPr><w:trHeight w:val="2000" w:hRule="exact"/></w:trPr><w:tc><w:tcPr><w:tcW w:w="3000" w:type="dxa"/></w:tcPr><w:p><w:r><w:t>table</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#;
    let body = format!(
        r#"{table}<w:p><w:r><w:t>beside</w:t><w:br w:type="textWrapping" w:clear="all"/></w:r></w:p>{}"#,
        para("after")
    );
    let out = pages(docx(&body, None, None));
    let y = |text: &str| {
        out[0]
            .items
            .iter()
            .find_map(|i| match i {
                crate::office::pptx::scene::Item::Text(r) if r.text == text => Some(r.transform.ty),
                _ => None,
            })
            .unwrap()
    };
    assert!(y("beside") < 120.0, "{}", y("beside"));
    assert!(y("after") > 172.0, "{}", y("after"));
}

/// `bytes` with the part `name` replaced by `xml`.
fn replace_part(bytes: Vec<u8>, name: &str, xml: &str) -> Vec<u8> {
    use std::io::{Read, Write};
    let mut zin = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut buf);
        for i in 0..zin.len() {
            let mut f = zin.by_index(i).unwrap();
            let entry = f.name().unwrap().to_string();
            let mut data = Vec::new();
            f.read_to_end(&mut data).unwrap();
            if entry == name {
                data = xml.as_bytes().to_vec();
            }
            z.start_file(entry.as_str(), zip::write::SimpleFileOptions::default())
                .unwrap();
            z.write_all(&data).unwrap();
        }
        z.finish().unwrap();
    }
    buf.into_inner()
}

#[test]
fn a_page_background_lowers_a_page_without_a_header() {
    // Word for Mac gives a document with `w:background` an empty header plus one more
    // empty line: with the header and the top margin both 36 pt, two 11 pt single
    // lines push the body's first line down past the margin.
    let sect = r#"<w:sectPr><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="720" w:bottom="1440" w:left="1440" w:right="1440" w:header="720"/></w:sectPr>"#;
    let render = |background: &str| {
        let doc = format!(
            r#"<w:document xmlns:w="{}">{background}<w:body>{}{sect}</w:body></w:document>"#,
            crate::office::docx::testutil::W,
            para("first")
        );
        let out = pages(replace_part(
            docx("", None, None),
            "word/document.xml",
            &doc,
        ));
        out[0]
            .items
            .iter()
            .find_map(|i| match i {
                crate::office::pptx::scene::Item::Text(r) if r.text == "first" => {
                    Some(r.transform.ty)
                }
                _ => None,
            })
            .unwrap()
    };
    let plain = render("");
    let lowered = render(r#"<w:background w:color="FFFFFF"/>"#);
    assert!(lowered - plain > 18.0, "{plain} -> {lowered}");
}

#[test]
fn without_html_auto_spacing_the_written_paragraph_spacing_applies() {
    // Paragraphs flagged for automatic spacing, written 5 pt before and after: under
    // `doNotUseHTMLParagraphAutoSpacing` Word adds the written values (10 pt between
    // lines), where automatic spacing collapses to 14 pt.
    let p = |t: &str| {
        format!(
            r#"<w:p><w:pPr><w:spacing w:before="100" w:beforeAutospacing="1" w:after="100" w:afterAutospacing="1" w:line="240" w:lineRule="auto"/></w:pPr><w:r><w:t>{t}</w:t></w:r></w:p>"#
        )
    };
    let body = format!("{}{}", p("one"), p("two"));
    let gap = |settings: &str| {
        let out = pages(crate::office::docx::testutil::with_settings(
            docx(&body, None, None),
            settings,
        ));
        let ys = run_ys(&out[0]);
        ys[1] - ys[0]
    };
    let html = gap("");
    let plain = gap("<w:compat><w:doNotUseHTMLParagraphAutoSpacing/></w:compat>");
    assert!((html - plain - 4.0).abs() < 0.1, "{html} vs {plain}");
}

#[test]
fn keep_with_next_does_not_pull_a_paragraph_onto_a_page_break_before() {
    // A nearly full page, an empty keep-with-next paragraph, then a paragraph with
    // page break before: the empty one ends page 1 and no blank page appears.
    let fill: String = (0..50).map(|i| para(&format!("line {i}"))).collect();
    let body = format!(
        r#"{fill}<w:p><w:pPr><w:keepNext/></w:pPr></w:p><w:p><w:pPr><w:pageBreakBefore/></w:pPr><w:r><w:t>next</w:t></w:r></w:p>"#
    );
    let out = pages(docx(&body, None, None));
    assert_eq!(out.len(), 2);
    assert!(page_texts(&out[1]).contains("next"));
}

#[test]
fn a_leading_page_break_in_a_page_break_before_paragraph_starts_one_page() {
    // "intro", then a heading with page break before whose text follows a page break
    // run: Word starts a single new page for it.
    let body = format!(
        r#"{}<w:p><w:pPr><w:pageBreakBefore/></w:pPr><w:r><w:br w:type="page"/></w:r><w:r><w:t>heading</w:t></w:r></w:p>"#,
        para("intro")
    );
    let out = pages(docx(&body, None, None));
    assert_eq!(out.len(), 2);
    assert!(page_texts(&out[1]).contains("heading"));
}

#[test]
fn keep_with_next_chain_stops_at_a_paragraph_ending_in_a_page_break() {
    // Two keep-with-next figures: the first ends with its own page break, the
    // second starts page 2 and its follower overflows. The chain moves no further
    // back than the second figure, so the first stays on page 1.
    let lines = |n: usize| -> String {
        (0..n)
            .map(|i| format!("<w:r><w:t>l{i}</w:t></w:r><w:r><w:br/></w:r>"))
            .collect()
    };
    let fill: String = (0..30).map(|i| para(&format!("line {i}"))).collect();
    let body = format!(
        r#"{fill}<w:p><w:pPr><w:keepNext/></w:pPr><w:r><w:t>one</w:t></w:r><w:r><w:br w:type="page"/></w:r></w:p><w:p><w:pPr><w:keepNext/><w:keepLines/></w:pPr><w:r><w:t>two</w:t></w:r>{}</w:p><w:p><w:pPr><w:keepLines/></w:pPr>{}</w:p>"#,
        lines(48),
        lines(12)
    );
    let out = pages(docx(&body, None, None));
    assert!(page_texts(&out[0]).contains("one"));
    assert!(page_texts(&out[1]).contains("two"));
    assert_eq!(out.len(), 3);
}

#[test]
fn page_break_before_a_new_page_section_break_starts_one_page() {
    // The section ends with a page break then its (empty) mark, as two paragraphs
    // or as one: from mode 14 Word starts a single new page for the next section;
    // mode 11 keeps the blank page.
    let brk = r#"<w:r><w:br w:type="page"/></w:r>"#;
    let sect = r#"<w:sectPr><w:type w:val="nextPage"/></w:sectPr>"#;
    for tail in [
        format!(r#"<w:p>{brk}</w:p><w:p><w:pPr>{sect}</w:pPr></w:p>"#),
        format!(r#"<w:p><w:pPr>{sect}</w:pPr>{brk}</w:p>"#),
    ] {
        let body = format!("{}{tail}{}", para("one"), para("two"));
        for (mode, expected) in [(15, 2), (11, 3)] {
            let out = pages(crate::office::docx::testutil::with_compat_mode(
                docx(&body, None, None),
                mode,
            ));
            assert_eq!(out.len(), expected, "mode {mode}: {tail}");
            assert!(page_texts(&out[expected - 1]).contains("two"));
        }
    }
}

#[test]
fn page_frame_content_survives_section_tails_and_wrapped_reflow() {
    let frame = r#"<w:p><w:pPr><w:framePr w:w="2400" w:hAnchor="page" w:vAnchor="page" w:x="1440" w:y="1440" w:wrap="around"/></w:pPr><w:r><w:t>FRAMED</w:t></w:r></w:p>"#;
    let table = r#"<w:tbl><w:tblGrid><w:gridCol w:w="4000"/></w:tblGrid><w:tr><w:tc><w:p><w:r><w:t>CELL</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#;
    for (tail, expected_pages) in [
        (String::new(), 1),
        (table.to_owned(), 1),
        (r#"<w:p><w:pPr><w:sectPr/></w:pPr></w:p>"#.to_owned(), 2),
        (para("FOLLOWING"), 1),
    ] {
        let out = pages(docx(&format!("{frame}{tail}"), None, None));
        assert_eq!(out.len(), expected_pages);
        let words: Vec<_> = out
            .iter()
            .flat_map(|p| &p.items)
            .filter_map(|item| match item {
                crate::office::pptx::scene::Item::Text(run) => Some(run.text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            words.iter().filter(|word| **word == "FRAMED").count(),
            1,
            "{words:?}"
        );
        if tail == table {
            assert!(words.contains(&"CELL"));
        }
        if tail.contains("FOLLOWING") {
            assert!(words.contains(&"FOLLOWING"));
        }
    }
}

#[test]
fn reflow_keeps_source_anchor_metadata_separate_from_injected_frames() {
    let body = r#"<w:p><w:r><w:drawing><wp:anchor xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"><wp:extent cx="914400" cy="914400"/></wp:anchor></w:drawing></w:r></w:p>"#;
    let opened =
        crate::office::docx::extract::open(docx(body, None, None), &Default::default()).unwrap();
    let crate::office::docx::model::Block::Para(p) = &opened.sections[0].blocks[0] else {
        panic!("paragraph expected")
    };
    let crate::office::docx::model::Inline::Anchor(anchor) = &p.inlines[0] else {
        panic!("anchor expected")
    };
    let make_anchor = |height| {
        let mut a = (**anchor).clone();
        a.pic.h = height;
        a
    };
    let line = |anchors| crate::office::docx::text::Line {
        height: 12.0,
        trailing_leading: 0.0,
        baseline: 10.0,
        items: Vec::new(),
        break_after: None,
        anchors,
        notes: Vec::new(),
        text: String::new(),
        skip: 0.0,
        boxes: Vec::new(),
        replay: None,
    };
    let injected = make_anchor(99.0);
    let original = vec![
        line(vec![make_anchor(11.0), injected.clone()]),
        line(vec![make_anchor(22.0)]),
    ];
    let mut reflowed = vec![line(vec![make_anchor(1.0), make_anchor(2.0)])];
    super::restore_reflow_anchors(&original, &[injected], &mut reflowed);
    assert_eq!(
        reflowed[0]
            .anchors
            .iter()
            .map(|a| a.pic.h)
            .collect::<Vec<_>>(),
        vec![11.0, 22.0, 99.0]
    );
}

#[test]
fn page_frame_preserves_nested_textbox_content() {
    let body = r##"<w:p><w:pPr><w:framePr w:w="4000" w:hAnchor="page" w:vAnchor="page" w:x="2000" w:y="4000"/></w:pPr><w:r><w:pict><v:shape xmlns:v="urn:schemas-microsoft-com:vml" id="box" type="#_x0000_t202" style="position:absolute;margin-left:10pt;margin-top:20pt;width:100pt;height:30pt"><v:textbox><w:txbxContent><w:p><w:r><w:t>NESTED</w:t></w:r></w:p></w:txbxContent></v:textbox></v:shape></w:pict></w:r><w:r><w:t>FRAME</w:t></w:r></w:p><w:p><w:r><w:t>FOLLOWING</w:t></w:r></w:p>"##;
    let out = pages(docx(body, None, None));
    let words: Vec<_> = out
        .iter()
        .flat_map(|p| &p.items)
        .filter_map(|item| match item {
            crate::office::pptx::scene::Item::Text(run) => Some(run.text.as_str()),
            _ => None,
        })
        .collect();
    for wanted in ["NESTED", "FRAME", "FOLLOWING"] {
        assert_eq!(
            words.iter().filter(|text| **text == wanted).count(),
            1,
            "{words:?}"
        );
    }
}

#[test]
fn nested_frame_anchors_keep_page_and_local_reference_origins_distinct() {
    use crate::office::docx::model::{Block, FrameAnchor, HRel, Inline, VRel};
    let body = r#"<w:p><w:r><w:drawing><wp:anchor xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"><wp:extent cx="914400" cy="914400"/></wp:anchor></w:drawing></w:r></w:p>"#;
    let opened =
        crate::office::docx::extract::open(docx(body, None, None), &Default::default()).unwrap();
    let Block::Para(p) = &opened.sections[0].blocks[0] else {
        panic!("paragraph expected")
    };
    let Inline::Anchor(a) = &p.inlines[0] else {
        panic!("anchor expected")
    };
    let mut parent = (**a).clone();
    parent.h_rel = HRel::Page;
    parent.v_rel = VRel::Page;
    parent.x = Ok(100.0);
    parent.y = Ok(200.0);
    let sample = pages(docx(&para("CHILD"), None, None));
    let mut item = sample[0]
        .items
        .iter()
        .find(|i| matches!(i, crate::office::pptx::scene::Item::Text(_)))
        .unwrap()
        .clone();
    if let crate::office::pptx::scene::Item::Text(run) = &mut item {
        run.transform.tx = 0.0;
        run.transform.ty = 0.0;
    }
    for (hrel, vrel, expected_x, expected_y) in [
        (HRel::Column, VRel::Paragraph, 110.0, 222.0),
        (HRel::Column, VRel::Line, 110.0, 236.0),
        (HRel::Page, VRel::Page, 7.0, 5.0),
        (
            HRel::Margin,
            VRel::Margin,
            sample[0].info.cols[0].0 + 7.0,
            sample[0].info.top + 5.0,
        ),
    ] {
        let mut child = (**a).clone();
        child.h_rel = hrel;
        child.v_rel = vrel;
        child.x = Ok(7.0);
        child.y = Ok(5.0);
        child.text_items = vec![item.clone()];
        parent.frame_children = vec![FrameAnchor {
            placement: None,
            anchor: child,
            column_x: 3.0,
            paragraph_y: 17.0,
            line_y: 31.0,
        }];
        let (drawn, _) = super::anchor_item(&parent, &sample[0].info, 72.0, 72.0, 72.0).unwrap();
        let run = drawn
            .iter()
            .find_map(|i| match i {
                crate::office::pptx::scene::Item::Text(run) => Some(run),
                _ => None,
            })
            .unwrap();
        assert!((f64::from(run.transform.tx) - expected_x).abs() < 0.01);
        assert!((f64::from(run.transform.ty) - expected_y).abs() < 0.01);
    }
    for (alignment, expected_x) in [("center", 126.0), ("right", 152.0)] {
        let mut child = (**a).clone();
        child.pic.w = 20.0;
        child.h_rel = HRel::Column;
        child.v_rel = VRel::Paragraph;
        child.x = Err(alignment.to_owned());
        child.y = Ok(0.0);
        child.text_items = vec![item.clone()];
        parent.frame_children = vec![FrameAnchor {
            placement: None,
            anchor: child,
            column_x: 0.0,
            paragraph_y: 0.0,
            line_y: 0.0,
        }];
        let (drawn, _) = super::anchor_item(&parent, &sample[0].info, 72.0, 72.0, 72.0).unwrap();
        let run = drawn
            .iter()
            .find_map(|i| match i {
                crate::office::pptx::scene::Item::Text(run) => Some(run),
                _ => None,
            })
            .unwrap();
        assert!((f64::from(run.transform.tx) - expected_x).abs() < 0.01);
    }
}

#[test]
fn continuous_section_starts_below_all_balanced_columns() {
    let mut body = String::new();
    for n in 0..12 {
        body.push_str(&format!(r#"<w:p><w:pPr><w:spacing w:after="0" w:line="400" w:lineRule="exact"/></w:pPr><w:r><w:t>line{n:02}</w:t></w:r></w:p>"#));
    }
    body.push_str(r#"<w:p><w:pPr><w:sectPr><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:bottom="1440"/><w:cols w:num="2" w:space="720"/></w:sectPr></w:pPr></w:p><w:p><w:r><w:t>successor</w:t></w:r></w:p><w:sectPr><w:type w:val="continuous"/><w:cols w:num="1"/></w:sectPr>"#);
    let out = pages(docx(&body, None, None));
    assert_eq!(out.len(), 1);
    let runs: Vec<_> = out[0]
        .items
        .iter()
        .filter_map(|i| match i {
            crate::office::pptx::scene::Item::Text(t) => Some(t),
            _ => None,
        })
        .collect();
    let successor = runs.iter().find(|t| t.text == "successor").unwrap();
    let lines: Vec<_> = runs.iter().filter(|t| t.text.starts_with("line")).collect();
    assert_eq!(lines.len(), 12);
    {
        let bottom = lines.iter().map(|t| t.transform.ty).fold(0.0_f32, f32::max);
        assert!(
            successor.transform.ty > bottom,
            "successor {} <= previous {}",
            successor.transform.ty,
            bottom
        );
        assert!(
            lines.iter().any(|t| t.transform.tx > 300.0),
            "balancing must fill the second column"
        );
    }
}

pub(super) fn with_column_pager(
    bytes: Vec<u8>,
    test: impl FnOnce(&mut super::Paginator<'_>, &[crate::office::docx::model::Section]),
) {
    let opened = crate::office::docx::extract::open(bytes, &Default::default()).unwrap();
    let fonts = crate::office::pptx::fonts::FontSystem::new(&[], true, &Default::default());
    let env = crate::office::docx::text::TextEnv {
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
    let mut pager = super::Paginator::new(
        &env,
        15,
        false,
        super::HdrFtr {
            continuation_separator: None,
            continuation_notice: None,
            blocks: &opened.hdr_ftr,
            notes: &opened.notes,
            separator: &opened.note_separator,
        },
    );
    pager.start_section(0, &opened.sections[0].props, &opened.sections);
    test(&mut pager, &opened.sections);
}

#[test]
fn column_trial_restores_starting_notes_and_carry_after_failure() {
    with_column_pager(docx(&para("body"), None, None), |pager, _| {
        pager.accept_notes(&[7]);
        pager.pages[0].note_carry = Some((5, 12.0));
        let start = pager.snapshot();
        pager.pages[0].notes.push(8);
        let final_pages = pager.pages.clone();
        for _ in 0..2 {
            pager.pages[0].notes.push(9);
            pager.pages[0].note_cut = Some((9, 4.0));
            pager.pages[0].note_carry = Some((9, 4.0));
            pager.pages.push(pager.pages[0].clone());
            pager.restore_balance_start(&start, &final_pages);
            assert_eq!(pager.pages.len(), 1);
            assert_eq!(pager.pages[0].notes, vec![7]);
            assert_eq!(pager.pages[0].note_carry, Some((5, 12.0)));
            assert_eq!(pager.pages[0].note_cut, None);
        }
    });
}

#[test]
fn column_balance_rejects_forced_tall_line_and_row_trial() {
    let end = r#"<w:sectPr><w:cols w:num="2" w:space="720"/></w:sectPr>"#;
    for content in [
        r#"<w:p><w:pPr><w:spacing w:after="0" w:line="2400" w:lineRule="exact"/></w:pPr><w:r><w:t>tall line</w:t></w:r></w:p>"#,
        r#"<w:tbl><w:tblPr><w:tblW w:w="4000" w:type="dxa"/></w:tblPr><w:tblGrid><w:gridCol w:w="4000"/></w:tblGrid><w:tr><w:trPr><w:cantSplit/><w:trHeight w:val="2400" w:hRule="exact"/></w:trPr><w:tc><w:p><w:pPr><w:spacing w:after="0" w:line="2400" w:lineRule="exact"/></w:pPr><w:r><w:t>tall row</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#,
    ] {
        with_column_pager(
            docx(&format!("{content}{end}"), None, None),
            |pager, sections| {
                let props = &sections[0].props;
                let units = crate::office::docx::layout::Flow::new(pager.env, 15).units(
                    &sections[0].blocks,
                    props.columns()[0].1,
                    false,
                );
                let top = pager.st.y;
                let snaps = pager.place_units(&units, 0, props);
                let occupied = pager.st.flow_bottom;
                assert!(occupied >= top + 120.0);
                pager.balance(&units, &snaps, 0, props);
                assert_eq!(pager.pages.len(), 1);
                assert!(
                    pager.st.y + 0.01 >= occupied,
                    "accepted below forced content: {} < {}",
                    pager.st.y,
                    occupied
                );
            },
        );
    }
}

#[test]
fn column_flow_extent_ignores_floating_table_cursor() {
    with_column_pager(docx(&para("body"), None, None), |pager, _| {
        pager.st.y += 40.0;
        pager.record_flow_bottom();
        let ordinary = pager.st.flow_bottom;
        pager.st.float = Some((0, pager.st.y, pager.st.y, 0, 72.0, 200.0));
        pager.st.y += 100.0;
        pager.record_flow_bottom();
        assert_eq!(pager.st.flow_bottom, ordinary);
        pager.st.float = None;
        pager.st.y = ordinary + 20.0;
        pager.record_flow_bottom();
        assert_eq!(pager.st.flow_bottom, ordinary + 20.0);
    });
}

#[test]
fn column_pending_note_is_reserved_before_the_height_cap() {
    let body = r#"<w:p><w:pPr><w:spacing w:after="0" w:line="600" w:lineRule="exact"/></w:pPr><w:r><w:t>reference</w:t><w:footnoteReference w:id="1"/></w:r></w:p><w:sectPr><w:cols w:num="2" w:space="720"/></w:sectPr>"#;
    with_column_pager(
        crate::office::docx::testutil::with_footnote(docx(body, None, None), "note"),
        |pager, sections| {
            let props = &sections[0].props;
            let units = crate::office::docx::layout::Flow::new(pager.env, 15).units(
                &sections[0].blocks,
                props.columns()[0].1,
                false,
            );
            let top = pager.st.y;
            pager.st.cap = Some(top + 40.0);
            pager.st.at_top = false;
            pager.accept_notes(&[7]);
            pager.note_cache.borrow_mut().insert(
                pager.note_cache_key(7),
                super::note_fragments::NoteLayout::test_height(20.0),
            );
            pager.note_cache.borrow_mut().insert(
                pager.note_cache_key(1),
                super::note_fragments::NoteLayout::test_height(20.0),
            );
            let spot = pager.place_para(&units, 0, false, 0, props, sections);
            assert_eq!(
                spot,
                (0, 0),
                "notes fit physically; subtracting them from the cap moves the line incorrectly"
            );
            assert_eq!(pager.pages[0].notes, vec![7, 1]);
            assert!(pager.st.flow_bottom <= top + 40.0);
        },
    );
}

#[test]
fn column_flow_highwater_survives_column_reset_but_not_new_page() {
    let body = format!(
        r#"{}<w:sectPr><w:cols w:num="2" w:space="720"/></w:sectPr>"#,
        para("body")
    );
    with_column_pager(docx(&body, None, None), |pager, sections| {
        let props = &sections[0].props;
        let top = pager.st.y;
        pager.st.y += 80.0;
        pager.record_flow_bottom();
        pager.next_column(0, props, sections);
        assert_eq!(pager.st.y, top);
        pager.st.y += 20.0;
        pager.record_flow_bottom();
        assert_eq!(pager.st.flow_bottom, top + 80.0);
        pager.new_page(0, props, sections);
        assert_eq!(pager.st.flow_bottom, pager.st.y);
        assert_eq!(pager.st.flow_bottom, top);
    });
}

#[test]
fn column_row_note_is_reserved_before_the_height_cap() {
    let body = r#"<w:tbl><w:tblPr><w:tblW w:w="4000" w:type="dxa"/></w:tblPr><w:tblGrid><w:gridCol w:w="4000"/></w:tblGrid><w:tr><w:trPr><w:cantSplit/><w:trHeight w:val="600" w:hRule="exact"/></w:trPr><w:tc><w:p><w:r><w:t>reference</w:t><w:footnoteReference w:id="1"/></w:r></w:p></w:tc></w:tr></w:tbl><w:sectPr><w:cols w:num="2" w:space="720"/></w:sectPr>"#;
    with_column_pager(
        crate::office::docx::testutil::with_footnote(docx(body, None, None), "note"),
        |pager, sections| {
            let props = &sections[0].props;
            let units = crate::office::docx::layout::Flow::new(pager.env, 15).units(
                &sections[0].blocks,
                props.columns()[0].1,
                false,
            );
            let crate::office::docx::layout::Unit::Row(row) = &units[0] else {
                panic!("row")
            };
            let top = pager.st.y;
            pager.st.cap = Some(top + 40.0);
            pager.st.at_top = false;
            pager.accept_notes(&[7]);
            pager.note_cache.borrow_mut().insert(
                pager.note_cache_key(7),
                super::note_fragments::NoteLayout::test_height(20.0),
            );
            pager.note_cache.borrow_mut().insert(
                pager.note_cache_key(1),
                super::note_fragments::NoteLayout::test_height(20.0),
            );
            assert_eq!(pager.place_row(row, false, 0, props, sections), (0, 0));
            assert_eq!(pager.pages[0].notes, vec![7, 1]);
            assert!(pager.st.flow_bottom <= top + 40.0);
        },
    );
}

#[test]
fn continuous_columns_retry_preserves_real_notes_and_body_once() {
    let page = r#"<w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:bottom="1440" w:left="1440" w:right="1440"/>"#;
    let mut body = format!(
        r#"<w:p><w:pPr><w:sectPr>{page}<w:cols w:num="1"/></w:sectPr></w:pPr><w:r><w:t>prelude</w:t><w:footnoteReference w:id="7"/></w:r></w:p>"#
    );
    body.push_str(r#"<w:p><w:pPr><w:spacing w:after="0" w:line="2400" w:lineRule="exact"/></w:pPr><w:r><w:t>tall</w:t><w:footnoteReference w:id="1"/></w:r></w:p>"#);
    for n in 0..4 {
        body.push_str(&format!(r#"<w:p><w:pPr><w:spacing w:after="0" w:line="400" w:lineRule="exact"/></w:pPr><w:r><w:t>short{n}</w:t></w:r></w:p>"#));
    }
    body.push_str(&format!(r#"<w:p><w:pPr><w:sectPr>{page}<w:type w:val="continuous"/><w:cols w:num="2" w:space="720"/></w:sectPr></w:pPr></w:p><w:p><w:r><w:t>successor</w:t></w:r></w:p><w:sectPr>{page}<w:type w:val="continuous"/><w:cols w:num="1"/></w:sectPr>"#));
    let notes = format!(
        r#"<w:footnotes xmlns:w="{}"><w:footnote w:id="7"><w:p><w:r><w:t>earlier-note</w:t></w:r></w:p></w:footnote><w:footnote w:id="1"><w:p><w:r><w:t>trial-note</w:t></w:r></w:p></w:footnote></w:footnotes>"#,
        crate::office::docx::testutil::W
    );
    let bytes = crate::office::docx::testutil::with_related_part(
        docx(&body, None, None),
        "footnotes",
        "footnotes.xml",
        &notes,
        false,
    );
    let out = pages(bytes);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].info.notes, vec![7, 1]);
    let runs: Vec<_> = out[0]
        .items
        .iter()
        .filter_map(|i| match i {
            crate::office::pptx::scene::Item::Text(t) => Some(t),
            _ => None,
        })
        .collect();
    for label in [
        "prelude",
        "tall",
        "short0",
        "short1",
        "short2",
        "short3",
        "successor",
        "earlier-note",
        "trial-note",
    ] {
        let expected = 1;
        assert_eq!(
            runs.iter().filter(|t| t.text.contains(label)).count(),
            expected,
            "{label}"
        );
    }
    {
        let successor = runs.iter().find(|t| t.text == "successor").unwrap();
        let body_bottom = runs
            .iter()
            .filter(|t| t.text.starts_with("tall") || t.text.starts_with("short"))
            .map(|t| t.transform.ty)
            .fold(0.0_f32, f32::max);
        assert!(successor.transform.ty > body_bottom);
        assert!(
            runs.iter()
                .any(|t| t.text.starts_with("short") && t.transform.tx > 300.0)
        );
    }
}

#[test]
fn column_balance_does_not_turn_search_slack_into_section_spacing() {
    let body = r#"<w:p><w:pPr><w:spacing w:after="0" w:line="2413" w:lineRule="exact"/></w:pPr><w:r><w:t>indivisible</w:t></w:r></w:p><w:sectPr><w:cols w:num="2" w:space="720"/></w:sectPr>"#;
    with_column_pager(docx(body, None, None), |pager, sections| {
        let props = &sections[0].props;
        let units = crate::office::docx::layout::Flow::new(pager.env, 15).units(
            &sections[0].blocks,
            props.columns()[0].1,
            false,
        );
        let top = pager.st.y;
        let snaps = pager.place_units(&units, 0, props);
        pager.balance(&units, &snaps, 0, props);
        assert!((pager.st.flow_bottom - top - 120.65).abs() < 0.01);
        assert!((pager.st.y - pager.st.flow_bottom).abs() < 0.001);
    });
}

#[test]
fn later_single_column_section_renders_its_note_without_moving_body_or_header() {
    let body = r#"<w:p><w:pPr><w:sectPr><w:headerReference w:type="default" r:id="rheader"/><w:cols w:num="1"/></w:sectPr></w:pPr><w:r><w:t>FIRST_BODY</w:t></w:r></w:p><w:p><w:r><w:t>SECOND_BODY</w:t><w:footnoteReference w:id="1"/></w:r></w:p><w:sectPr><w:type w:val="continuous"/><w:cols w:num="1"/></w:sectPr>"#;
    let header = format!(
        r#"<w:hdr xmlns:w="{}"><w:p><w:r><w:t>FIRST_HEADER</w:t></w:r></w:p></w:hdr>"#,
        crate::office::docx::testutil::W
    );
    let note = format!(
        r#"<w:footnotes xmlns:w="{}"><w:footnote w:id="1"><w:p><w:r><w:footnoteRef/><w:t>LATER_NOTE</w:t></w:r></w:p></w:footnote></w:footnotes>"#,
        crate::office::docx::testutil::W
    );
    let input = |body: &str| {
        let bytes = crate::office::docx::testutil::with_related_part(
            docx(body, None, None),
            "header",
            "header.xml",
            &header,
            false,
        );
        crate::office::docx::testutil::with_related_part(
            bytes,
            "footnotes",
            "footnotes.xml",
            &note,
            false,
        )
    };
    let out = pages(input(body));
    let control = pages(input(
        &body.replace(r#"<w:footnoteReference w:id="1"/>"#, ""),
    ));
    assert_eq!(out.len(), 1);
    assert_eq!(control.len(), 1);
    let text = |page: &super::OutPage, marker: &str| {
        page.items
            .iter()
            .filter_map(|i| {
                if let crate::office::pptx::scene::Item::Text(t) = i {
                    Some(t)
                } else {
                    None
                }
            })
            .find(|t| t.text.starts_with(marker))
            .map(|t| (t.transform, t.size))
    };
    for marker in ["FIRST_BODY", "SECOND_BODY", "FIRST_HEADER"] {
        assert!(text(&out[0], marker).is_some());
        assert_eq!(text(&out[0], marker), text(&control[0], marker), "{marker}");
    }
    let notes = out[0]
        .items
        .iter()
        .filter(|i| matches!(i,crate::office::pptx::scene::Item::Text(t) if t.text.contains("LATER_NOTE")))
        .count();
    assert_eq!(notes, 1);
    assert_eq!(out[0].info.section, 0);
    assert_eq!(out[0].info.number, 1);
}

#[test]
fn final_note_metadata_retains_carry_cut_and_original_page_settings() {
    with_column_pager(docx(&para("body"), None, None), |pager, _| {
        let original = pager.pages[0].clone();
        let mut output = vec![super::OutPage {
            info: original.clone(),
            items: Vec::new(),
        }];
        let mut final_info = original.clone();
        final_info.notes = vec![3, 7];
        final_info.note_carry = Some((2, 24.0));
        final_info.note_cut = Some((7, 36.0));
        final_info.section = 9;
        final_info.number = 20;
        final_info.first_of_section = false;
        final_info.blank = true;
        final_info.w = 300.0;
        final_info.h = 400.0;
        final_info.top = 22.0;
        final_info.bottom = 350.0;
        final_info.cols = vec![(20.0, 100.0), (130.0, 100.0)];
        super::refresh_note_metadata(&mut output, &[final_info]);
        let got = &output[0].info;
        assert_eq!(got.notes, vec![3, 7]);
        assert_eq!(got.note_carry, Some((2, 24.0)));
        assert_eq!(got.note_cut, Some((7, 36.0)));
        assert_eq!(
            (got.section, got.number, got.first_of_section, got.blank),
            (
                original.section,
                original.number,
                original.first_of_section,
                original.blank
            )
        );
        assert_eq!(
            (got.w, got.h, got.top, got.bottom, got.cols.clone()),
            (
                original.w,
                original.h,
                original.top,
                original.bottom,
                original.cols.clone()
            )
        );
        // A subsequent completed layout can clear obsolete split state as well.
        super::refresh_note_metadata(&mut output, &[original]);
        assert!(output[0].info.notes.is_empty());
        assert_eq!(output[0].info.note_carry, None);
        assert_eq!(output[0].info.note_cut, None);
    });
}

#[test]
fn leading_column_transition_keeps_three_column_spacing_and_real_blanks() {
    let p = |line: u32, before: u32, after: u32, content: &str| {
        format!(
            r#"<w:p><w:pPr><w:spacing w:before="{before}" w:after="{after}" w:line="{line}" w:lineRule="exact"/></w:pPr><w:r>{content}</w:r></w:p>"#
        )
    };
    let body = format!(
        r#"{}{}{}{}<w:sectPr><w:cols w:num="3" w:space="360"/></w:sectPr>"#,
        p(400, 0, 0, "<w:t>first</w:t>"),
        p(200, 0, 0, ""),
        p(600, 100, 140, "<w:br w:type=\"column\"/><w:t>second</w:t>"),
        p(600, 0, 0, "<w:br w:type=\"column\"/><w:t>third</w:t>")
    );
    with_column_pager(docx(&body, None, None), |pager, sections| {
        let props = &sections[0].props;
        let mut units = crate::office::docx::layout::Flow::new(pager.env, 15).units(
            &sections[0].blocks,
            props.columns()[0].1,
            false,
        );
        let crate::office::docx::layout::Unit::Para(heading) = &mut units[2] else {
            panic!("paragraph")
        };
        // Exercise spacing metadata independently of painted paragraph borders.
        heading.pad_top = 3.0;
        heading.pad_bottom = 2.0;
        let top = pager.st.y;
        let snaps = pager.place_units(&units, 0, props);
        // Column 2: "second" keeps its 5 pt space before (Word sets a column-break
        // paragraph's text on the baseline of its neighbour), 30 pt line, 2 + 7 after.
        let expected = 44.0;
        assert!(
            (pager.st.flow_bottom - top - expected).abs() < 0.01,
            "{}",
            pager.st.flow_bottom - top
        );
        let blank = pager
            .placed
            .iter()
            .find_map(|pl| match pl {
                super::Placed::Line { unit: 1, y, .. } => Some(*y),
                _ => None,
            })
            .unwrap();
        assert!((blank - top - 20.0).abs() < 0.01);
        pager.balance(&units, &snaps, 0, props);
        assert_eq!(pager.pages.len(), 1);
        assert!((pager.st.y - top - expected).abs() < 0.01);
    });
}

#[test]
fn leading_column_transition_scope_excludes_real_content_and_standalone_breaks() {
    let cases = [
        (r#"<w:br w:type="column"/><w:t>following</w:t>"#, true),
        (
            r#"<w:t>before</w:t><w:br w:type="column"/><w:t>following</w:t>"#,
            false,
        ),
        (r#"<w:br w:type="column"/>"#, false),
        (r#"<w:br/><w:t>following</w:t>"#, false),
        (r#"<w:br w:type="page"/><w:t>following</w:t>"#, false),
        (
            r#"<w:t xml:space="preserve"> </w:t><w:br w:type="column"/><w:t>following</w:t>"#,
            false,
        ),
    ];
    for (runs, expected) in cases {
        let body = format!(r#"<w:p><w:r>{runs}</w:r></w:p>"#);
        with_column_pager(docx(&body, None, None), |pager, sections| {
            let units = crate::office::docx::layout::Flow::new(pager.env, 15).units(
                &sections[0].blocks,
                240.0,
                false,
            );
            let crate::office::docx::layout::Unit::Para(p) = &units[0] else {
                panic!("paragraph")
            };
            assert_eq!(super::leading_column_transition(p, 0), expected, "{runs}");
            assert!(!super::leading_column_transition(p, 1));
            if expected {
                let mut noted = p.clone();
                noted.lines[0].notes.push(1);
                assert!(!super::leading_column_transition(&noted, 0));
                let mut empty_following = p.clone();
                empty_following.lines[1].text.clear();
                assert!(!super::leading_column_transition(&empty_following, 0));
            }
        });
    }
}

#[test]
fn leading_column_transition_preserves_anchored_or_decorated_lines() {
    let anchor = r#"<w:drawing><wp:anchor xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"><wp:positionH relativeFrom="column"><wp:posOffset>0</wp:posOffset></wp:positionH><wp:positionV relativeFrom="paragraph"><wp:posOffset>0</wp:posOffset></wp:positionV><wp:extent cx="127000" cy="127000"/><wp:wrapSquare wrapText="bothSides"/></wp:anchor></w:drawing>"#;
    for (props, drawing) in [
        ("", anchor),
        (
            r#"<w:pBdr><w:bottom w:val="single" w:sz="8" w:space="3"/></w:pBdr>"#,
            "",
        ),
        (r#"<w:shd w:fill="FFFF00"/>"#, ""),
    ] {
        let body = format!(
            r#"<w:p><w:pPr>{props}</w:pPr><w:r>{drawing}<w:br w:type="column"/><w:t>following</w:t></w:r></w:p>"#
        );
        with_column_pager(docx(&body, None, None), |pager, sections| {
            let units = crate::office::docx::layout::Flow::new(pager.env, 15).units(
                &sections[0].blocks,
                240.0,
                false,
            );
            let crate::office::docx::layout::Unit::Para(p) = &units[0] else {
                panic!("paragraph")
            };
            assert!(p.decor.is_some() || !p.lines[0].anchors.is_empty());
            assert!(!super::leading_column_transition(p, 0));
        });
    }
}

#[test]
fn leading_cell_page_break_moves_row_once_and_preserves_cover() {
    let table = r#"<w:tbl><w:tblGrid><w:gridCol w:w="4000"/></w:tblGrid><w:tr><w:tc><w:p><w:pPr><w:pageBreakBefore/></w:pPr><w:r><w:t>table content</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#;
    let body = format!("{}{table}{}", para("cover"), para("after"));
    let out = pages(docx(&body, None, None));
    assert_eq!(out.len(), { 2 });
    assert!(page_texts(&out[0]).contains("cover"));
    assert!(!page_texts(&out[0]).contains("table content"));
    let last = page_texts(out.last().unwrap());
    assert!(last.contains("table content") && last.contains("after"));
    assert_eq!(
        pages(docx(table, None, None)).len(),
        1,
        "page-top break adds no blank page"
    );
}

#[test]
fn later_row_page_break_repeats_header_without_repeating_break() {
    let body = r#"<w:tbl><w:tblGrid><w:gridCol w:w="4000"/></w:tblGrid><w:tr><w:trPr><w:tblHeader/></w:trPr><w:tc><w:p><w:r><w:t>heading</w:t></w:r></w:p></w:tc></w:tr><w:tr><w:tc><w:p><w:pPr><w:pageBreakBefore/></w:pPr><w:r><w:t>body row</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#;
    let out = pages(docx(body, None, None));
    assert_eq!(out.len(), { 2 });
    assert!(page_texts(out.last().unwrap()).contains("heading"));
    assert!(page_texts(out.last().unwrap()).contains("body row"));
    assert_eq!(
        out.iter()
            .map(page_texts)
            .collect::<String>()
            .matches("body row")
            .count(),
        1
    );
}

#[test]
fn table_page_boundary_overrides_preceding_keep_chain() {
    let body = format!(
        r#"{}<w:p><w:pPr><w:keepNext/></w:pPr><w:r><w:t>keep predecessor</w:t></w:r></w:p><w:tbl><w:tblGrid><w:gridCol w:w="4000"/></w:tblGrid><w:tr><w:tc><w:p><w:pPr><w:pageBreakBefore/></w:pPr><w:r><w:t>table boundary</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#,
        para("cover")
    );
    let out = pages(docx(&body, None, None));
    assert_eq!(out.len(), { 2 });
    assert!(page_texts(&out[0]).contains("keep predecessor"));
    assert!(page_texts(out.last().unwrap()).contains("table boundary"));
}

#[test]
fn inherited_table_page_break_advances_from_second_column_top() {
    let body = r#"<w:tbl><w:tblGrid><w:gridCol w:w="3000"/></w:tblGrid><w:tr><w:tc><w:p><w:pPr><w:pStyle w:val="BreakChild"/></w:pPr><w:r><w:t>inherited boundary</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:sectPr><w:cols w:num="2"/></w:sectPr>"#;
    let styles = r#"<w:style w:type="paragraph" w:styleId="BreakBase"><w:pPr><w:pageBreakBefore/></w:pPr></w:style><w:style w:type="paragraph" w:styleId="BreakChild"><w:basedOn w:val="BreakBase"/></w:style>"#;
    with_column_pager(docx(body, Some(styles), None), |pager, sections| {
        let props = &sections[0].props;
        pager.next_column(0, props, sections);
        assert_eq!(pager.st.col, 1);
        assert!(pager.st.at_top);
        let units = crate::office::docx::layout::Flow::new(pager.env, 15).units(
            &sections[0].blocks,
            200.0,
            false,
        );
        let crate::office::docx::layout::Unit::Row(row) = &units[0] else {
            panic!("expected row")
        };
        assert!(row.page_break_before);
        pager.place_row(row, false, 0, props, sections);
        assert_eq!(pager.pages.len(), { 2 });
        assert_eq!(pager.st.col, { 0 });
    });
}

#[test]
fn vertical_merge_flow_keeps_top_text_inside_page_and_column_fragments() {
    for align in ["top", "center", "bottom"] {
        for columns in [1, 2] {
            let rows = (0..24).map(|i| {
                let merge = if i == 0 { r#"<w:vMerge w:val="restart"/>"# } else { "<w:vMerge/>" };
                let text = if i == 0 {
                    (0..18).map(|k| format!(r#"<w:p><w:pPr><w:spacing w:before="0" w:after="0" w:line="240" w:lineRule="exact"/></w:pPr><w:r><w:t>cap{k:02}</w:t></w:r></w:p>"#)).collect::<String>()
                } else { "<w:p/>".into() };
                format!(r#"<w:tr><w:trPr><w:cantSplit/><w:trHeight w:val="300"/></w:trPr><w:tc><w:tcPr>{merge}<w:vAlign w:val="{align}"/><w:tcMar><w:top w:w="40" w:type="dxa"/><w:bottom w:w="40" w:type="dxa"/></w:tcMar></w:tcPr>{text}</w:tc><w:tc>{}</w:tc></w:tr>"#,para(&format!("row{i:02}")))
            }).collect::<String>();
            let body = format!(
                r#"<w:tbl><w:tblGrid><w:gridCol w:w="1200"/><w:gridCol w:w="1200"/></w:tblGrid>{rows}</w:tbl><w:sectPr><w:pgSz w:w="7200" w:h="4320"/><w:pgMar w:top="720" w:bottom="720" w:left="720" w:right="720"/><w:cols w:num="{columns}" w:space="360"/></w:sectPr>"#
            );
            let out = pages(docx(&body, None, None));
            let mut seen = Vec::new();
            for (page, pg) in out.iter().enumerate() {
                for item in &pg.items {
                    if let crate::office::pptx::scene::Item::Text(run) = item
                        && run.text.starts_with("cap")
                    {
                        seen.push((run.text.clone(), page, run.transform.tx, run.transform.ty));
                        {
                            assert!(
                                run.transform.ty >= 36.0 && run.transform.ty < 180.0,
                                "{align}/{columns}: {} at {}",
                                run.text,
                                run.transform.ty
                            );
                        }
                    }
                }
            }
            assert_eq!(seen.len(), 18, "{align}/{columns}");
            for k in 0..18 {
                assert_eq!(
                    seen.iter().filter(|r| r.0 == format!("cap{k:02}")).count(),
                    1
                );
            }
            {
                assert!(
                    seen.iter()
                        .any(|r| r.1 != seen[0].1 || (r.2 - seen[0].2).abs() > 50.0),
                    "text must reach a later physical fragment"
                );
            }
        }
    }
}

#[test]
fn top_merge_flow_preserves_single_fragment_spacing() {
    let render = |merge: bool| {
        let start = if merge {
            r#"<w:vMerge w:val="restart"/>"#
        } else {
            ""
        };
        let continuation = if merge { "<w:vMerge/>" } else { "" };
        let body = format!(
            r#"<w:tbl><w:tblGrid><w:gridCol w:w="3000"/></w:tblGrid><w:tr><w:trPr><w:trHeight w:val="800"/></w:trPr><w:tc><w:tcPr>{start}<w:tcMar><w:top w:w="100" w:type="dxa"/></w:tcMar></w:tcPr><w:p><w:pPr><w:spacing w:before="140" w:after="0"/></w:pPr><w:r><w:t>same origin</w:t></w:r></w:p></w:tc></w:tr><w:tr><w:tc><w:tcPr>{continuation}</w:tcPr><w:p/></w:tc></w:tr></w:tbl>"#
        );
        let out = pages(docx(&body, None, None));
        out[0]
            .items
            .iter()
            .find_map(|i| match i {
                crate::office::pptx::scene::Item::Text(r) if r.text == "same origin" => {
                    Some((r.transform.tx, r.transform.ty))
                }
                _ => None,
            })
            .unwrap()
    };
    assert_eq!(render(true), render(false));
}

#[test]
fn vertical_merge_flow_moves_inline_shape_with_its_text_slice() {
    let shape = r#"<w:drawing><wp:inline xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"><wp:extent cx="152400" cy="152400"/><a:graphic><a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"><wps:wsp><wps:spPr><a:prstGeom prst="rect"/><a:solidFill><a:srgbClr val="008080"/></a:solidFill></wps:spPr><wps:bodyPr/></wps:wsp></a:graphicData></a:graphic></wp:inline></w:drawing>"#;
    let mut rows = String::new();
    for i in 0..30 {
        let merge = if i == 0 {
            r#"<w:vMerge w:val="restart"/>"#
        } else {
            "<w:vMerge/>"
        };
        let content = if i == 0 {
            let mut text=(0..14).map(|_| r#"<w:p><w:pPr><w:spacing w:after="0" w:line="240" w:lineRule="exact"/></w:pPr><w:r><w:t>line</w:t></w:r></w:p>"#).collect::<String>();
            text.push_str(&format!(r#"<w:p><w:pPr><w:spacing w:after="0"/></w:pPr><w:r><w:t>before</w:t>{shape}<w:t>after</w:t></w:r></w:p>"#));
            text
        } else {
            "<w:p/>".into()
        };
        rows.push_str(&format!(r#"<w:tr><w:trPr><w:trHeight w:val="300"/><w:cantSplit/></w:trPr><w:tc><w:tcPr>{merge}</w:tcPr>{content}</w:tc><w:tc>{}</w:tc></w:tr>"#,para("row")));
    }
    let body = format!(
        r#"<w:tbl><w:tblGrid><w:gridCol w:w="3000"/><w:gridCol w:w="3000"/></w:tblGrid>{rows}</w:tbl><w:sectPr><w:pgSz w:w="9000" w:h="4320"/><w:pgMar w:top="720" w:bottom="720" w:left="720" w:right="720"/></w:sectPr>"#
    );
    let out = pages(docx(&body, None, None));
    let mut found = Vec::new();
    for (pg, page) in out.iter().enumerate() {
        for (order, item) in page.items.iter().enumerate() {
            if let crate::office::pptx::scene::Item::Path(p) = item
                && let Some(crate::office::pptx::scene::Paint::Solid(c)) = &p.fill
                && c.r == 0.0
                && c.g > 0.49
                && c.g < 0.51
                && c.b > 0.49
                && c.b < 0.51
            {
                found.push((pg, order, p.path.bounds()));
            }
        }
    }
    assert_eq!(found.len(), 1, "inline shape must be emitted once");
    let (pg, order, bounds) = found[0];
    let before = out[pg]
        .items
        .iter()
        .position(|i| matches!(i,crate::office::pptx::scene::Item::Text(r) if r.text=="before"))
        .unwrap();
    let after = out[pg]
        .items
        .iter()
        .position(|i| matches!(i,crate::office::pptx::scene::Item::Text(r) if r.text=="after"))
        .unwrap();
    assert!(
        before < order && order < after,
        "slice paint order is retained"
    );
    {
        assert!(pg > 0);
        assert!(bounds.top() >= 36.0 && bounds.bottom() <= 180.0);
    }
}

#[test]
fn vertical_merge_repeated_headers_keep_each_copy_independent() {
    let headers = r#"<w:tr><w:trPr><w:tblHeader/><w:cantSplit/><w:trHeight w:val="360"/></w:trPr><w:tc><w:tcPr><w:vMerge w:val="restart"/></w:tcPr><w:p><w:pPr><w:spacing w:after="0"/></w:pPr><w:r><w:t>repeat owner</w:t></w:r></w:p></w:tc><w:tc><w:p/></w:tc></w:tr><w:tr><w:trPr><w:tblHeader/><w:cantSplit/><w:trHeight w:val="360"/></w:trPr><w:tc><w:tcPr><w:vMerge/></w:tcPr><w:p/></w:tc><w:tc><w:p/></w:tc></w:tr>"#;
    let rows=(0..25).map(|i|format!(r#"<w:tr><w:trPr><w:cantSplit/><w:trHeight w:val="360"/></w:trPr><w:tc>{}</w:tc><w:tc><w:p/></w:tc></w:tr>"#,para(&format!("body{i:02}")))).collect::<String>();
    let body = format!(
        r#"<w:tbl><w:tblGrid><w:gridCol w:w="3000"/><w:gridCol w:w="3000"/></w:tblGrid>{headers}{rows}</w:tbl><w:sectPr><w:pgSz w:w="9000" w:h="4320"/><w:pgMar w:top="720" w:bottom="720" w:left="720" w:right="720"/></w:sectPr>"#
    );
    let out = pages(docx(&body, None, None));
    assert!(out.len() > 2);
    let mut origins = Vec::new();
    for page in &out {
        let hits: Vec<_> = page
            .items
            .iter()
            .filter_map(|i| match i {
                crate::office::pptx::scene::Item::Text(r) if r.text == "repeat owner" => {
                    Some((r.transform.tx, r.transform.ty))
                }
                _ => None,
            })
            .collect();
        assert_eq!(hits.len(), 1, "every repeated header keeps one owner");
        origins.push(hits[0]);
    }
    assert!(origins.iter().all(|p| *p == origins[0]));
    for i in 0..25 {
        assert_eq!(
            out.iter()
                .map(page_texts)
                .collect::<String>()
                .matches(&format!("body{i:02}"))
                .count(),
            1
        );
    }
}

#[test]
fn merged_continuation_paints_inline_content_above_owner_shading() {
    use crate::office::pptx::scene::{Item, Paint, Rect, Scene};

    let shape = r#"<w:drawing><wp:inline xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"><wp:extent cx="152400" cy="101600"/><a:graphic><a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"><wps:wsp><wps:spPr><a:prstGeom prst="rect"/><a:solidFill><a:srgbClr val="008080"/></a:solidFill></wps:spPr><wps:bodyPr/></wps:wsp></a:graphicData></a:graphic></wp:inline></w:drawing>"#;
    let mut content = (0..11).map(|i| format!(r#"<w:p><w:pPr><w:spacing w:after="0" w:line="240" w:lineRule="exact"/></w:pPr><w:r><w:t>line{i:02}</w:t></w:r></w:p>"#)).collect::<String>();
    content.push_str(&format!(r#"<w:p><w:pPr><w:spacing w:after="0" w:line="240" w:lineRule="exact"/></w:pPr><w:r><w:t>tail</w:t>{shape}</w:r></w:p>"#));
    let body = format!(
        r#"<w:tbl><w:tblGrid><w:gridCol w:w="2400"/><w:gridCol w:w="2400"/></w:tblGrid><w:tr><w:trPr><w:cantSplit/><w:trHeight w:val="2000"/></w:trPr><w:tc><w:tcPr><w:vMerge w:val="restart"/><w:shd w:fill="FFFFFF"/></w:tcPr>{content}</w:tc><w:tc>{}</w:tc></w:tr><w:tr><w:trPr><w:cantSplit/><w:trHeight w:val="2000"/></w:trPr><w:tc><w:tcPr><w:vMerge/><w:shd w:fill="FFFFFF"/></w:tcPr><w:p/></w:tc><w:tc>{}</w:tc></w:tr></w:tbl><w:sectPr><w:pgSz w:w="6000" w:h="3720"/><w:pgMar w:top="360" w:bottom="360" w:left="360" w:right="360"/></w:sectPr>"#,
        para("first row"),
        para("last row")
    );
    let cfg = crate::office::pptx::config::Config::default();
    let opened = crate::office::docx::extract::open(docx(&body, None, None), &cfg).unwrap();
    let fonts = crate::office::pptx::fonts::FontSystem::new(&[], true, &Default::default());
    let laid = crate::office::docx::extract::layout(&opened, &fonts);
    assert_eq!(laid.pages.len(), 2);
    assert_eq!(
        laid.pages
            .iter()
            .map(page_texts)
            .collect::<String>()
            .matches("tail")
            .count(),
        1
    );
    let (page, order, bounds) = laid.pages.iter().enumerate().find_map(|(pi, p)| p.items.iter().enumerate().find_map(|(ii, it)| match it {
        Item::Path(path) if matches!(&path.fill, Some(Paint::Solid(c)) if c.r == 0.0 && c.g > 0.49 && c.g < 0.51 && c.b > 0.49 && c.b < 0.51) => Some((pi, ii, path.path.bounds())),
        _ => None,
    })).unwrap();

    assert_eq!(page, 1, "inline content reaches the continuation");
    let pg = &laid.pages[page];
    let owner = pg.items.iter().enumerate().find_map(|(i, it)| match it {
        Item::Path(path) if matches!(&path.fill, Some(Paint::Solid(c)) if c.r == 1.0 && c.g == 1.0 && c.b == 1.0) && path.path.bounds().left() <= bounds.left() && path.path.bounds().right() >= bounds.right() && path.path.bounds().top() <= bounds.top() && path.path.bounds().bottom() >= bounds.bottom() => Some(i),
        _ => None,
    }).unwrap();
    assert!(
        owner < order,
        "the owning cell background paints before its continuation"
    );
    let doc =
        crate::office::pptx::document::Document::from_package(opened.pkg, 6000 * 635, 3720 * 635);
    let images = crate::office::pptx::render::ImageCache::new(cfg.limits.max_image_pixels, &fonts);
    let scene = Scene {
        width: pg.info.w,
        height: pg.info.h,
        items: pg.items.clone(),
        ..Default::default()
    };
    let target = crate::office::pptx::render::RenderTarget {
        region: Rect {
            x: 0.0,
            y: 0.0,
            w: pg.info.w,
            h: pg.info.h,
        },
        width: pg.info.w as u32,
        height: pg.info.h as u32,
    };
    let pm = crate::office::pptx::render::render(&scene, &target, &doc, &fonts, &images).unwrap();
    let x = ((bounds.left() + bounds.right()) / 2.0) as u32;
    let y = ((bounds.top() + bounds.bottom()) / 2.0) as u32;
    let pixel = pm.pixel(x, y).unwrap();
    assert!(
        pixel.red() < 10
            && pixel.green() > 110
            && pixel.green() < 145
            && pixel.blue() > 110
            && pixel.blue() < 145,
        "relocated inline content remains visible above white cell shading"
    );
}

#[test]
fn merged_slice_capacity_uses_partial_page_and_preserves_every_line() {
    let line = |text: &str| {
        format!(
            r#"<w:p><w:pPr><w:spacing w:before="0" w:after="0" w:line="240" w:lineRule="exact"/></w:pPr><w:r><w:rPr><w:sz w:val="20"/></w:rPr><w:t>{text}</w:t></w:r></w:p>"#
        )
    };
    let content = (0..30)
        .map(|i| line(&format!("slice{i:02}")))
        .collect::<String>();
    let body = format!(
        r#"<w:tbl><w:tblPr><w:tblBorders><w:bottom w:val="single" w:sz="8" w:color="FF0000"/><w:insideH w:val="single" w:sz="8" w:color="FF0000"/></w:tblBorders></w:tblPr><w:tblGrid><w:gridCol w:w="1800"/><w:gridCol w:w="1800"/></w:tblGrid><w:tr><w:tc><w:tcPr><w:vMerge w:val="restart"/></w:tcPr>{content}</w:tc><w:tc>{}</w:tc></w:tr><w:tr><w:tc><w:tcPr><w:vMerge/></w:tcPr>{}</w:tc><w:tc>{}</w:tc></w:tr></w:tbl>{}<w:sectPr><w:pgSz w:w="4800" w:h="4800"/><w:pgMar w:top="400" w:bottom="400" w:left="400" w:right="400"/></w:sectPr>"#,
        line("first row"),
        line(""),
        line("last row"),
        line("after table")
    );
    let out = pages(docx(&body, None, None));

    assert_eq!(
        out.len(),
        2,
        "merge slices use the room remaining after the first row"
    );
    let mut labels = Vec::new();
    let mut final_bottom = 0.0_f64;
    for (page, pg) in out.iter().enumerate() {
        let border=pg.items.iter().filter_map(|it|match it { crate::office::pptx::scene::Item::Path(p) if matches!(&p.fill,Some(crate::office::pptx::scene::Paint::Solid(c))if c.r==1.0&&c.g==0.0&&c.b==0.0)=>Some(p.path.bounds().bottom() as f64),_=>None }).fold(0.0_f64,f64::max);
        assert!(border <= 220.51);
        for it in &pg.items {
            if let crate::office::pptx::scene::Item::Text(r) = it {
                if r.text.starts_with("slice") {
                    labels.push(r.text.clone());
                    let bottom = r.transform.ty as f64 + r.descent;
                    assert!(
                        bottom <= border + 0.01,
                        "{} page{page}: {bottom} > {border}",
                        r.text
                    );
                    assert!(r.transform.ty as f64 - r.ascent >= 19.0);
                    if page == 1 {
                        final_bottom = final_bottom.max(bottom);
                    }
                }
                if r.text == "after table" {
                    assert!(r.transform.ty as f64 - r.ascent >= final_bottom);
                }
            }
        }
    }
    assert_eq!(
        labels,
        (0..30).map(|i| format!("slice{i:02}")).collect::<Vec<_>>()
    );
}

#[test]
fn merged_slice_progress_can_exceed_two_hundred_fragments() {
    let content = (0..3250)
        .map(|i| format!(r#"<w:p><w:pPr><w:spacing w:before="0" w:after="0" w:line="240" w:lineRule="exact"/></w:pPr><w:r><w:rPr><w:sz w:val="20"/></w:rPr><w:t>long{i:04}</w:t></w:r></w:p>"#))
        .collect::<String>();
    let body = format!(
        r#"<w:tbl><w:tblGrid><w:gridCol w:w="1800"/><w:gridCol w:w="1800"/></w:tblGrid><w:tr><w:tc><w:tcPr><w:vMerge w:val="restart"/></w:tcPr>{content}</w:tc><w:tc><w:p/></w:tc></w:tr><w:tr><w:tc><w:tcPr><w:vMerge/></w:tcPr><w:p/></w:tc><w:tc><w:p/></w:tc></w:tr></w:tbl><w:sectPr><w:pgSz w:w="4800" w:h="4800"/><w:pgMar w:top="400" w:bottom="400" w:left="400" w:right="400"/></w:sectPr>"#
    );
    let out = pages(docx(&body, None, None));
    assert!(out.len() > 200);
    let mut labels = Vec::new();
    for pg in &out {
        for item in &pg.items {
            if let crate::office::pptx::scene::Item::Text(run) = item
                && run.text.starts_with("long")
            {
                assert!(run.transform.ty as f64 + run.descent <= 220.01);
                labels.push(run.text.clone());
            }
        }
    }
    assert_eq!(
        labels,
        (0..3250).map(|i| format!("long{i:04}")).collect::<Vec<_>>()
    );
}

#[test]
fn unequal_columns_measure_leading_controls_and_following_paragraphs() {
    for (first, second) in [(6000, 1000), (1000, 6000)] {
        let body = format!(
            r#"<w:p><w:r><w:t>FIRST</w:t></w:r></w:p><w:p><w:pPr><w:jc w:val="center"/><w:spacing w:after="0"/></w:pPr><w:r><w:br w:type="column"/><w:t>ALPHA BETA GAMMA DELTA EPSILON ZETA ETA THETA</w:t></w:r></w:p><w:p><w:pPr><w:jc w:val="center"/></w:pPr><w:r><w:t>AFTER</w:t></w:r></w:p><w:sectPr><w:pgSz w:w="9000" w:h="14000"/><w:pgMar w:top="500" w:bottom="500" w:left="1000" w:right="1000"/><w:cols w:num="2" w:equalWidth="0" w:space="0"><w:col w:w="{first}" w:space="0"/><w:col w:w="{second}"/></w:cols></w:sectPr>"#
        );
        let out = pages(docx(&body, None, None));
        assert_eq!(out.len(), 1);
        let runs: Vec<_> = out[0]
            .items
            .iter()
            .filter_map(|i| {
                if let crate::office::pptx::scene::Item::Text(r) = i {
                    Some(r)
                } else {
                    None
                }
            })
            .collect();
        let text: String = runs
            .iter()
            .flat_map(|r| r.text.chars())
            .filter(|c| !c.is_whitespace())
            .collect();
        assert_eq!(text, "FIRSTALPHABETAGAMMADELTAEPSILONZETAETATHETAAFTER");
        let following: Vec<_> = runs.iter().filter(|r| r.text != "FIRST").collect();
        {
            let left = 50.0 + first as f64 / 20.0;
            assert!(
                following
                    .iter()
                    .all(|r| r.transform.tx as f64 >= left - 0.1)
            );
            // The ASCII fixture's final space has no ink and does not size alignment.
            assert!(following.iter().all(|r| {
                let end = if r.text.ends_with(' ') {
                    r.glyphs.last().unwrap().x
                } else {
                    r.width
                };
                r.transform.tx as f64 + end <= 400.1
            }));
        }
    }
}

#[test]
fn unequal_column_remeasurement_keeps_numbering_and_floating_anchor_once() {
    let numbering = r#"<w:abstractNum w:abstractNumId="0"><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:lvlText w:val="%1."/><w:pPr><w:ind w:left="400" w:hanging="200"/></w:pPr></w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num>"#;
    let body = r#"<w:p><w:r><w:t>FIRST</w:t></w:r></w:p><w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="1"/></w:numPr></w:pPr><w:r><w:br w:type="column"/><w:t>NUMBERED BODY CONTENT</w:t><w:drawing><wp:anchor xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"><wp:positionH relativeFrom="column"><wp:posOffset>0</wp:posOffset></wp:positionH><wp:positionV relativeFrom="paragraph"><wp:posOffset>0</wp:posOffset></wp:positionV><wp:extent cx="127000" cy="127000"/><wp:wrapNone/><a:graphic xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"><wps:wsp xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"><wps:spPr><a:prstGeom prst="rect"/><a:solidFill><a:srgbClr val="008080"/></a:solidFill></wps:spPr><wps:bodyPr/></wps:wsp></a:graphicData></a:graphic></wp:anchor></w:drawing></w:r></w:p><w:sectPr><w:pgSz w:w="9000" w:h="14000"/><w:pgMar w:top="500" w:bottom="500" w:left="500" w:right="500"/><w:cols w:num="2" w:equalWidth="0" w:space="0"><w:col w:w="6000" w:space="0"/><w:col w:w="2000"/></w:cols></w:sectPr>"#;
    let out = pages(docx(body, None, Some(numbering)));
    assert_eq!(out.len(), 1);
    let text: String = out[0]
        .items
        .iter()
        .filter_map(|i| {
            if let crate::office::pptx::scene::Item::Text(r) = i {
                Some(r.text.as_str())
            } else {
                None
            }
        })
        .collect::<String>()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    assert_eq!(text, "FIRST1.NUMBEREDBODYCONTENT");
    assert_eq!(
        out[0]
            .items
            .iter()
            .filter(|i| matches!(i, crate::office::pptx::scene::Item::Path(_)))
            .count(),
        1
    );
}

#[test]
fn unequal_columns_remeasure_automatic_paragraph_continuations() {
    let words = (0..90)
        .map(|i| format!("word{i:02}"))
        .collect::<Vec<_>>()
        .join(" ");
    for (first, second) in [(4800, 1600), (1600, 4800)] {
        let body = format!(
            r#"<w:p><w:pPr><w:widowControl w:val="0"/><w:spacing w:before="0" w:after="0" w:line="240" w:lineRule="exact"/></w:pPr><w:r><w:rPr><w:sz w:val="20"/></w:rPr><w:t>{words}</w:t></w:r></w:p><w:sectPr><w:pgSz w:w="7400" w:h="2600"/><w:pgMar w:top="300" w:bottom="300" w:left="500" w:right="500"/><w:cols w:num="2" w:equalWidth="0" w:space="0"><w:col w:w="{first}" w:space="0"/><w:col w:w="{second}"/></w:cols></w:sectPr>"#
        );
        let out = pages(docx(&body, None, None));
        let mut actual = String::new();
        let mut right_column_runs = 0;
        for page in &out {
            for item in &page.items {
                let crate::office::pptx::scene::Item::Text(run) = item else {
                    continue;
                };
                actual.extend(run.text.chars().filter(|c| !c.is_whitespace()));
                let x = run.transform.tx as f64;
                let split = 25.0 + first as f64 / 20.0;
                let edge = if x >= split - 0.1 {
                    right_column_runs += 1;
                    345.0
                } else {
                    split
                };
                let ink_width = if run.text.ends_with(' ') {
                    run.glyphs.last().unwrap().x
                } else {
                    run.width
                };
                assert!(
                    x + ink_width <= edge + 0.1,
                    "columns {first}/{second}: text extends to {}, edge {edge}",
                    x + ink_width
                );
            }
        }
        assert!(right_column_runs > 0);
        assert_eq!(actual, words.replace(' ', ""));
    }
}

#[test]
fn automatic_column_continuation_preserves_labels_anchors_and_notes_once() {
    let words = (0..100).map(|i| format!("body{i:03}")).collect::<Vec<_>>();
    let anchor = r#"<w:drawing><wp:anchor xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"><wp:positionH relativeFrom="column"><wp:posOffset>0</wp:posOffset></wp:positionH><wp:positionV relativeFrom="paragraph"><wp:posOffset>0</wp:posOffset></wp:positionV><wp:extent cx="127000" cy="127000"/><wp:wrapNone/><a:graphic xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"><wps:wsp xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"><wps:spPr><a:prstGeom prst="rect"/><a:solidFill><a:srgbClr val="008080"/></a:solidFill></wps:spPr><wps:bodyPr/></wps:wsp></a:graphicData></a:graphic></wp:anchor></w:drawing>"#;
    let numbering = r#"<w:abstractNum w:abstractNumId="0"><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:lvlText w:val="%1."/><w:pPr><w:ind w:left="200" w:hanging="200"/></w:pPr></w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num>"#;
    let body = format!(
        r#"<w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="1"/></w:numPr><w:widowControl w:val="0"/><w:spacing w:before="0" w:after="0" w:line="240" w:lineRule="exact"/></w:pPr><w:r><w:rPr><w:sz w:val="20"/></w:rPr><w:t xml:space="preserve">{} </w:t>{anchor}<w:footnoteReference w:id="1"/><w:t xml:space="preserve"> {}</w:t></w:r></w:p><w:sectPr><w:pgSz w:w="7400" w:h="2600"/><w:pgMar w:top="300" w:bottom="300" w:left="500" w:right="500"/><w:cols w:num="2" w:equalWidth="0" w:space="0"><w:col w:w="4800" w:space="0"/><w:col w:w="1600"/></w:cols></w:sectPr>"#,
        words[..45].join(" "),
        words[45..].join(" ")
    );
    let note = format!(
        r#"<w:footnotes xmlns:w="{}"><w:footnote w:id="1"><w:p><w:r><w:t>UNIQUE_NOTE</w:t></w:r></w:p></w:footnote></w:footnotes>"#,
        crate::office::docx::testutil::W
    );
    let bytes = crate::office::docx::testutil::with_related_part(
        docx(&body, None, Some(numbering)),
        "footnotes",
        "footnotes.xml",
        &note,
        false,
    );
    let out = pages(bytes);
    let text: String = out
        .iter()
        .flat_map(|p| &p.items)
        .filter_map(|i| match i {
            crate::office::pptx::scene::Item::Text(r) => Some(r.text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(text.matches("UNIQUE_NOTE").count(), 1);
    assert_eq!(text.matches("1.").count(), 1);
    let mut previous = 0;
    for word in words {
        assert_eq!(text.matches(&word).count(), 1, "{word}");
        let pos = text.find(&word).unwrap();
        assert!(pos >= previous);
        previous = pos;
    }
    assert_eq!(
        out.iter()
            .flat_map(|p| &p.items)
            .filter(|i| matches!(i, crate::office::pptx::scene::Item::Path(_)))
            .count(),
        1
    );
}

#[test]
fn oversized_header_or_footer_retains_minimum_body_capacity() {
    for kind in ["header", "footer"] {
        let content: String = (0..20).map(|i| para(&format!("BODY{i:02}"))).collect();
        let body = format!(
            r#"{content}<w:sectPr><w:{kind}Reference w:type="default" r:id="r{kind}"/><w:pgSz w:w="6000" w:h="3000"/><w:pgMar w:top="300" w:bottom="300" w:left="300" w:right="300" w:header="100" w:footer="100"/></w:sectPr>"#
        );
        let tag = if kind == "header" { "hdr" } else { "ftr" };
        let hf = format!(
            r#"<w:{tag} xmlns:w="{}"><w:p><w:pPr><w:spacing w:line="4000" w:lineRule="exact"/></w:pPr><w:r><w:t>DECORATION</w:t></w:r></w:p></w:{tag}>"#,
            crate::office::docx::testutil::W
        );
        let out = pages(crate::office::docx::testutil::with_related_part(
            docx(&body, None, None),
            kind,
            &format!("{kind}.xml"),
            &hf,
            false,
        ));
        assert!(
            out.iter().all(|p| p.info.bottom >= p.info.top + 12.0),
            "{kind} must leave a positive body interval"
        );
        let text: String = out.iter().map(page_texts).collect();
        for i in 0..20 {
            assert_eq!(text.matches(&format!("BODY{i:02}")).count(), 1);
        }
    }
}

#[test]
fn footer_frame_preserves_body_capacity_and_source_offset() {
    let render = |framed: bool, y: i32| {
        let body = r#"<w:p><w:pPr><w:spacing w:line="10400" w:lineRule="exact"/></w:pPr><w:r><w:t>body</w:t></w:r></w:p><w:p><w:pPr><w:spacing w:line="240" w:lineRule="exact"/></w:pPr><w:r><w:t>tail</w:t></w:r></w:p><w:sectPr><w:footerReference w:type="default" r:id="rfooter"/><w:pgSz w:w="15840" w:h="12240"/><w:pgMar w:top="720" w:bottom="720" w:left="864" w:right="864" w:footer="432"/></w:sectPr>"#;
        let frame = if framed {
            format!(
                r#"<w:framePr w:wrap="around" w:vAnchor="text" w:hAnchor="margin" w:xAlign="center" w:y="{y}"/>"#
            )
        } else {
            String::new()
        };
        let tail = if framed {
            "<w:p><w:pPr><w:spacing w:line=\"240\" w:lineRule=\"exact\"/></w:pPr></w:p>"
        } else {
            ""
        };
        let footer = format!(
            r#"<w:ftr xmlns:w="{}"><w:p><w:pPr>{frame}<w:spacing w:line="240" w:lineRule="exact"/></w:pPr><w:r><w:t>footer</w:t></w:r></w:p>{tail}</w:ftr>"#,
            crate::office::docx::testutil::W
        );
        pages(crate::office::docx::testutil::with_related_part(
            docx(body, None, None),
            "footer",
            "footer.xml",
            &footer,
            false,
        ))
    };
    let ordinary = render(false, 0);
    for y in [-120, 0, 1, 120] {
        let framed = render(true, y);
        assert_eq!(framed.len(), 1, "offset {y}: final body line must fit");
        assert!((framed[0].info.bottom - 576.0).abs() < 0.01);
        assert!((outer_text_y(&framed, "tail") - outer_text_y(&ordinary, "tail")).abs() < 0.01);
        assert!(
            (outer_text_y(&framed, "footer") - outer_text_y(&ordinary, "footer") - y as f64 / 20.0)
                .abs()
                < 0.02
        );
    }
}

#[test]
fn shared_header_footer_part_has_separate_story_measurements() {
    let body = r#"<w:p/><w:sectPr><w:footerReference w:type="default" r:id="rfooter"/></w:sectPr>"#;
    let footer = format!(
        r#"<w:ftr xmlns:w="{}"><w:p><w:pPr><w:framePr w:vAnchor="text" w:y="1"/><w:spacing w:line="240" w:lineRule="exact"/></w:pPr><w:r><w:t>frame</w:t></w:r></w:p><w:p><w:pPr><w:spacing w:line="240" w:lineRule="exact"/></w:pPr></w:p></w:ftr>"#,
        crate::office::docx::testutil::W
    );
    let source = crate::office::docx::testutil::with_related_part(
        docx(body, None, None),
        "footer",
        "footer.xml",
        &footer,
        false,
    );
    with_column_pager(source, |pager, sections| {
        let mut props = sections[0].props.clone();
        props.headers = props.footers.clone();
        pager.hf_heights.clear();
        pager.hf_anchors.clear();
        let (header, footer) = pager.hf_extent(&props, true, 1, sections);
        // The framed paragraph floats in both stories: only the 12 pt paragraph counts.
        assert_eq!(header, Some(12.0));
        assert_eq!(footer, Some(12.0));
        assert_eq!(pager.hf_heights.len(), 2);
        assert_eq!(pager.hf_anchors.len(), 2);
    });
}

// Exact line heights isolate spacing from platform font metrics.
fn auto_before_boundary_target(boundary: &str, spacing: &str, inherited: bool) -> (usize, f32) {
    auto_before_boundary_target_mode(boundary, spacing, inherited, 12)
}

fn auto_before_boundary_target_mode(
    boundary: &str,
    spacing: &str,
    inherited: bool,
    mode: u32,
) -> (usize, f32) {
    let setup = r#"<w:pgSz w:w="6000" w:h="6000"/><w:pgMar w:top="800" w:bottom="800" w:left="800" w:right="800"/>"#;
    let columns = if boundary.contains("column") {
        r#"<w:cols w:num="2" w:space="200"/>"#
    } else {
        ""
    };
    let section = format!("<w:sectPr>{setup}{columns}</w:sectPr>");
    let filler = |height: usize, tail: &str| {
        format!(
            r#"<w:p><w:pPr><w:spacing w:after="0" w:line="{height}" w:lineRule="exact"/></w:pPr><w:r><w:t>PRECEDING</w:t>{tail}</w:r></w:p>"#
        )
    };
    let prefix = match boundary {
        "soft" | "soft-column" => filler(4200, ""),
        "soft-column-page" => format!("{}{}", filler(4200, ""), filler(4200, "")),
        "first" => String::new(),
        "hard" => filler(240, r#"<w:br w:type="page"/>"#),
        "hard-column" => filler(240, r#"<w:br w:type="column"/>"#),
        "section" | "oddSection" | "evenSection" => format!(
            r#"<w:p><w:pPr><w:sectPr>{setup}</w:sectPr></w:pPr><w:r><w:t>PRECEDING</w:t></w:r></w:p>"#
        ),
        "sectionAfter" => format!(
            r#"<w:p><w:pPr><w:spacing w:after="400"/><w:sectPr>{setup}</w:sectPr></w:pPr><w:r><w:t>PRECEDING</w:t></w:r></w:p>"#
        ),
        "continuous" | "continuousPageBefore" => format!(
            r#"<w:p><w:pPr><w:sectPr>{setup}</w:sectPr></w:pPr><w:r><w:t>PRECEDING</w:t></w:r></w:p>"#
        ),
        _ => filler(240, ""),
    };
    let prefix = if matches!(boundary, "oddSectionAfterPage" | "evenSectionAfterPage") {
        format!(
            r#"{}<w:p><w:pPr><w:sectPr>{setup}</w:sectPr></w:pPr><w:r><w:t>SECTION END</w:t></w:r></w:p>"#,
            filler(240, r#"<w:br w:type="page"/>"#)
        )
    } else if boundary == "continuousHard" {
        format!(
            r#"<w:p><w:pPr><w:sectPr>{setup}</w:sectPr></w:pPr><w:r><w:t>SECTION END</w:t></w:r></w:p>{}"#,
            filler(240, r#"<w:br w:type="page"/>"#)
        )
    } else {
        prefix
    };
    let page_before = if matches!(boundary, "pageBefore" | "continuousPageBefore") {
        "<w:pageBreakBefore/>"
    } else {
        ""
    };
    let style = if inherited {
        r#"<w:pStyle w:val="Child"/>"#
    } else {
        ""
    };
    let section = if matches!(
        boundary,
        "continuous" | "continuousHard" | "continuousPageBefore"
    ) {
        section.replace("<w:sectPr>", r#"<w:sectPr><w:type w:val="continuous"/>"#)
    } else {
        section
    };
    let body = format!(
        r#"{prefix}<w:p><w:pPr>{style}{page_before}<w:spacing w:after="0" w:line="240" w:lineRule="exact" {spacing}/></w:pPr><w:r><w:t>TARGET</w:t></w:r></w:p>{section}"#
    );
    let styles = r#"<w:style w:type="paragraph" w:styleId="Base"><w:pPr><w:spacing w:before="280" w:beforeAutospacing="1"/></w:pPr></w:style><w:style w:type="paragraph" w:styleId="Child"><w:basedOn w:val="Base"/></w:style>"#;
    let body = match boundary {
        "oddSection" | "oddSectionAfterPage" => {
            body.replace("<w:sectPr>", "<w:sectPr><w:type w:val=\"oddPage\"/>")
        }
        "evenSection" | "evenSectionAfterPage" => {
            body.replace("<w:sectPr>", "<w:sectPr><w:type w:val=\"evenPage\"/>")
        }
        _ => body,
    };
    let bytes = docx(&body, Some(styles), None);
    let mut input = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut output = std::io::Cursor::new(Vec::new());
    {
        use std::io::{Read, Write};
        let mut zip = zip::ZipWriter::new(&mut output);
        for i in 0..input.len() {
            let mut file = input.by_index(i).unwrap();
            let name = file.name().unwrap().to_string();
            let mut data = String::new();
            file.read_to_string(&mut data).unwrap();
            if name == "word/_rels/document.xml.rels" {
                data = data.replace("</Relationships>", r#"<Relationship Id="rSettings" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings" Target="settings.xml"/></Relationships>"#);
            }
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(data.as_bytes()).unwrap();
        }
        zip.start_file(
            "word/settings.xml",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        write!(zip, r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:compat><w:compatSetting w:name="compatibilityMode" w:uri="http://schemas.microsoft.com/office/word" w:val="{mode}"/></w:compat></w:settings>"#).unwrap();
        zip.finish().unwrap();
    }
    let bytes = output.into_inner();
    let cfg = crate::office::pptx::config::Config::default();
    assert_eq!(
        crate::office::docx::extract::open(bytes.clone(), &cfg)
            .unwrap()
            .settings
            .compat_mode,
        mode
    );
    let out = pages(bytes);
    let hits: Vec<_> = out
        .iter()
        .enumerate()
        .flat_map(|(page, p)| {
            p.items.iter().filter_map(move |item| match item {
                crate::office::pptx::scene::Item::Text(run) if run.text == "TARGET" => {
                    Some((page, run.transform.ty))
                }
                _ => None,
            })
        })
        .collect();
    assert_eq!(hits.len(), 1, "{boundary}");
    hits[0]
}

#[test]
fn before_spacing_collapses_at_soft_page_and_column_tops() {
    for boundary in ["soft", "soft-column-page", "soft-column"] {
        let plain =
            auto_before_boundary_target(boundary, r#"w:before="0" w:beforeAutospacing="0""#, false);
        let page = usize::from(boundary != "soft-column");
        assert_eq!(plain.0, page, "{boundary}");
        for spacing in [
            r#"w:before="280" w:beforeAutospacing="1""#,
            r#"w:before="480" w:beforeAutospacing="0""#,
        ] {
            let spaced = auto_before_boundary_target(boundary, spacing, false);
            assert_eq!(spaced.0, plain.0, "{boundary}");
            assert!(
                (spaced.1 - plain.1).abs() < 0.01,
                "{boundary} {spacing}: {spaced:?} {plain:?}"
            );
        }
    }
}

#[test]
fn automatic_before_spacing_preserves_forced_section_and_column_boundaries() {
    for boundary in ["hard", "pageBefore", "section", "continuous", "hard-column"] {
        let plain =
            auto_before_boundary_target(boundary, r#"w:before="0" w:beforeAutospacing="0""#, false);
        let automatic = auto_before_boundary_target(
            boundary,
            r#"w:before="280" w:beforeAutospacing="1""#,
            false,
        );
        assert_eq!(automatic.0, plain.0, "{boundary}");
        assert!(
            (automatic.1 - plain.1 - 14.0).abs() < 0.01,
            "{boundary}: {automatic:?} {plain:?}"
        );
    }
}

#[test]
fn automatic_before_spacing_respects_style_inheritance_and_false_override() {
    let zero =
        auto_before_boundary_target("first", r#"w:before="0" w:beforeAutospacing="0""#, false);
    let inherited = auto_before_boundary_target("first", "", true);
    let explicit =
        auto_before_boundary_target("first", r#"w:before="280" w:beforeAutospacing="0""#, true);
    assert_eq!(zero.0, inherited.0);
    assert!((zero.1 - inherited.1).abs() < 0.01);
    assert!((explicit.1 - zero.1 - 14.0).abs() < 0.01);
}

#[test]
fn automatic_before_spacing_adds_nothing_to_paragraph_continuations() {
    let render = |spacing: &str| {
        let lines = (0..40)
            .map(|i| {
                format!(
                    "<w:r><w:t>L{i:02}</w:t>{}</w:r>",
                    if i == 39 { "" } else { "<w:br/>" }
                )
            })
            .collect::<String>();
        let body = format!(
            r#"<w:p><w:pPr><w:spacing w:after="0" w:line="240" w:lineRule="exact" {spacing}/></w:pPr>{lines}</w:p><w:sectPr><w:pgSz w:w="6000" w:h="6000"/><w:pgMar w:top="800" w:bottom="800" w:left="800" w:right="800"/></w:sectPr>"#
        );
        pages(docx(&body, None, None))
    };
    let plain = render(r#"w:before="0" w:beforeAutospacing="0""#);
    let automatic = render(r#"w:before="280" w:beforeAutospacing="1""#);
    assert!(plain.len() > 1);
    assert_eq!(automatic.len(), plain.len());
    let mut logical = String::new();
    for (a, b) in automatic.iter().zip(&plain) {
        assert_eq!(page_texts(a), page_texts(b));
        assert_eq!(run_ys(a), run_ys(b));
        logical.push_str(&page_texts(a));
    }
    assert_eq!(
        logical,
        (0..40).map(|i| format!("L{i:02}")).collect::<String>()
    );
}
#[test]
fn wrapped_local_textbox_bottom_uses_visible_overflow_and_preserves_fixed_positions() {
    use crate::office::docx::model::{Block, Inline, VRel, Wrap};
    let source = r#"<w:p><w:r><w:drawing><wp:anchor xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"><wp:extent cx="2540000" cy="254000"/></wp:anchor></w:drawing></w:r></w:p>"#;
    let opened =
        crate::office::docx::extract::open(docx(source, None, None), &Default::default()).unwrap();
    let Block::Para(p) = &opened.sections[0].blocks[0] else {
        panic!("paragraph")
    };
    let Inline::Anchor(a) = &p.inlines[0] else {
        panic!("anchor")
    };
    let mut a = (**a).clone();
    let sample = pages(docx(&para("overflow"), None, None));
    let page = &sample[0].info;
    let mut item = sample[0]
        .items
        .iter()
        .find(|i| matches!(i, crate::office::pptx::scene::Item::Text(_)))
        .unwrap()
        .clone();
    if let crate::office::pptx::scene::Item::Text(run) = &mut item {
        run.transform = tiny_skia::Transform::from_translate(0.0, 70.0);
    }
    a.text_items = vec![item];
    a.wrap = Wrap::Square;
    a.y = Ok(20.0);
    let bottom = super::anchored_text_bottom(&a);
    assert!(bottom > a.pic.h + 40.0);
    for vrel in [VRel::Paragraph, VRel::Line] {
        a.v_rel = vrel;
        let (_, y, _, _) = super::anchor_rect(&a, page, 72.0, page.h - 30.0, page.h - 30.0);
        assert!((y + bottom - page.h).abs() < 0.001);
        let (drawn, _) = super::anchor_item(&a, page, 72.0, page.h - 30.0, page.h - 30.0).unwrap();
        let run = drawn
            .iter()
            .find_map(|i| {
                if let crate::office::pptx::scene::Item::Text(r) = i {
                    Some(r)
                } else {
                    None
                }
            })
            .unwrap();
        assert!((run.transform.ty as f64 + run.descent - page.h).abs() < 0.001);
    }
    for vrel in [VRel::Page, VRel::Margin] {
        a.v_rel = vrel;
        a.y = Ok(page.h);
        let (_, y, _, _) = super::anchor_rect(&a, page, 72.0, 72.0, 72.0);
        assert!(y >= page.h);
    }
    a.v_rel = VRel::Paragraph;
    a.y = Ok(-1.0);
    assert_eq!(
        super::anchor_rect(&a, page, 72.0, page.h, page.h).1,
        page.h - 1.0
    );
    a.y = Ok(20.0);
    a.behind = true;
    assert_eq!(
        super::anchor_rect(&a, page, 72.0, page.h - 30.0, 0.0).1,
        page.h - 10.0
    );
    a.behind = false;
    a.pic.h = page.h + 20.0;
    assert_eq!(super::anchor_rect(&a, page, 72.0, 72.0, 0.0).1, 92.0);
    a.wrap = Wrap::None;
    assert_eq!(super::anchor_rect(&a, page, 72.0, 72.0, 0.0).1, 0.0);
}

#[test]
fn paragraph_relative_textbox_near_page_end_keeps_text_visible() {
    let body = r#"<w:p><w:pPr><w:spacing w:line="14880" w:lineRule="exact"/></w:pPr><w:r><w:t>spacer</w:t></w:r></w:p><w:p><w:pPr><w:spacing w:line="240" w:lineRule="exact"/></w:pPr><w:r><w:t>anchor</w:t><w:drawing><wp:anchor xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape" behindDoc="0"><wp:positionH relativeFrom="column"><wp:posOffset>0</wp:posOffset></wp:positionH><wp:positionV relativeFrom="paragraph"><wp:posOffset>1016000</wp:posOffset></wp:positionV><wp:extent cx="2540000" cy="508000"/><wp:wrapSquare wrapText="bothSides"/><a:graphic><a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"><wps:wsp><wps:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="2540000" cy="508000"/></a:xfrm><a:prstGeom prst="rect"/></wps:spPr><wps:txbx><w:txbxContent><w:p><w:r><w:t>FLOATING CONTENT</w:t></w:r></w:p></w:txbxContent></wps:txbx><wps:bodyPr lIns="0" tIns="0" rIns="0" bIns="0"/></wps:wsp></a:graphicData></a:graphic></wp:anchor></w:drawing></w:r></w:p><w:sectPr><w:pgSz w:w="11900" w:h="16840"/><w:pgMar w:top="720" w:bottom="720" w:left="720" w:right="720"/></w:sectPr>"#;
    let out = pages(docx(body, None, None));
    let mut found = 0;
    for page in &out {
        for item in &page.items {
            if let crate::office::pptx::scene::Item::Text(run) = item
                && run.text == "FLOATING CONTENT"
            {
                found += 1;
                assert!(run.transform.ty as f64 - run.ascent >= 0.0);
                assert!(run.transform.ty as f64 + run.descent <= page.info.h + 0.01);
            }
        }
    }
    assert_eq!(found, 1);
    let runs: Vec<_> = out
        .iter()
        .flat_map(|p| &p.items)
        .filter_map(|i| {
            if let crate::office::pptx::scene::Item::Text(run) = i {
                Some(run)
            } else {
                None
            }
        })
        .collect();
    let anchor = runs.iter().find(|r| r.text == "anchor").unwrap();
    let floating = runs.iter().find(|r| r.text == "FLOATING CONTENT").unwrap();
    assert!(
        anchor.transform.ty as f64 + anchor.descent
            <= floating.transform.ty as f64 - floating.ascent
    );
}

#[test]
fn whole_paragraph_retry_rebuilds_its_own_textbox_clearance() {
    let original = r#"<w:p><w:pPr><w:spacing w:line="14880" w:lineRule="exact"/></w:pPr><w:r><w:t>spacer</w:t></w:r></w:p><w:p><w:pPr><w:spacing w:line="240" w:lineRule="exact"/></w:pPr><w:r><w:t>anchor</w:t><w:drawing><wp:anchor xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape" behindDoc="0"><wp:positionH relativeFrom="column"><wp:posOffset>0</wp:posOffset></wp:positionH><wp:positionV relativeFrom="paragraph"><wp:posOffset>1016000</wp:posOffset></wp:positionV><wp:extent cx="2540000" cy="508000"/><wp:wrapSquare wrapText="bothSides"/><a:graphic><a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"><wps:wsp><wps:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="2540000" cy="508000"/></a:xfrm><a:prstGeom prst="rect"/></wps:spPr><wps:txbx><w:txbxContent><w:p><w:r><w:t>FLOATING CONTENT</w:t></w:r></w:p></w:txbxContent></wps:txbx><wps:bodyPr lIns="0" tIns="0" rIns="0" bIns="0"/></wps:wsp></a:graphicData></a:graphic></wp:anchor></w:drawing></w:r></w:p><w:sectPr><w:pgSz w:w="11900" w:h="16840"/><w:pgMar w:top="720" w:bottom="720" w:left="720" w:right="720"/></w:sectPr>"#;
    for columns in [
        "",
        r#"<w:cols w:equalWidth="0"><w:col w:w="7000" w:space="400"/><w:col w:w="3060"/></w:cols>"#,
    ] {
        let body = original
            .replace("2540000", "6642100")
            .replace("508000", "685800")
            .replace("1016000", "304800")
            .replace("</w:sectPr>", &format!("{columns}</w:sectPr>"));
        let out = pages(docx(&body, None, None));
        let mut anchors = Vec::new();
        let mut floating = Vec::new();
        for (pi, page) in out.iter().enumerate() {
            for item in &page.items {
                if let crate::office::pptx::scene::Item::Text(run) = item {
                    if run.text == "anchor" {
                        anchors.push((pi, run));
                    }
                    if run.text == "FLOATING CONTENT" {
                        floating.push((pi, run));
                    }
                }
            }
        }
        assert_eq!(anchors.len(), 1);
        assert_eq!(floating.len(), 1);
        assert_eq!(anchors[0].0, floating[0].0);
        let a = anchors[0].1;
        let f = floating[0].1;
        assert!(
            a.transform.ty as f64 + a.descent <= f.transform.ty as f64 - f.ascent,
            "anchor and its textbox overlap after retry: {:?} {:?}",
            a.transform,
            f.transform
        );
    }
}

#[test]
fn page_relative_text_frames_fit_the_box_and_preserve_local_indentation() {
    let render = |x: i32, width: Option<i32>, indent: i32| {
        let width = width.map(|w| format!(r#"w:w="{w}""#)).unwrap_or_default();
        let body = r#"<w:p><w:r><w:t>BODY</w:t></w:r></w:p><w:sectPr><w:footerReference w:type="default" r:id="rfooter"/><w:pgSz w:w="6000" w:h="6000"/><w:pgMar w:top="800" w:bottom="800" w:left="800" w:right="800" w:footer="400"/></w:sectPr>"#;
        let footer = format!(
            r#"<w:ftr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:p><w:pPr><w:framePr w:hAnchor="page" w:vAnchor="text" w:y="0" w:x="{x}" {width}/><w:ind w:left="{indent}"/></w:pPr><w:r><w:rPr><w:sz w:val="14"/></w:rPr><w:t>2</w:t></w:r></w:p><w:p/></w:ftr>"#
        );
        let out = pages(crate::office::docx::testutil::with_related_part(
            docx(body, None, None),
            "footer",
            "footer.xml",
            &footer,
            false,
        ));
        let runs: Vec<_> = out
            .iter()
            .flat_map(|p| &p.items)
            .filter_map(|item| match item {
                crate::office::pptx::scene::Item::Text(r) if r.text == "2" => {
                    Some((r.transform.tx as f64, r.width))
                }
                _ => None,
            })
            .collect();
        assert_eq!(runs.len(), 1);
        runs[0]
    };
    for indent in [0, 1400] {
        assert!((render(2000, Some(2000), indent).0 - (100.0 + indent as f64 / 20.0)).abs() < 0.01);
        assert!((render(5200, Some(2000), indent).0 - (200.0 + indent as f64 / 20.0)).abs() < 0.01);
        assert!((render(-400, Some(2000), indent).0 - indent as f64 / 20.0).abs() < 0.01);
    }
    let auto = render(5200, None, 1400);
    assert!(
        auto.0 + auto.1 <= 300.0 && auto.0 + auto.1 >= 298.0,
        "{auto:?}"
    );
    assert!((render(5200, Some(4000), 1400).0 - 170.0).abs() < 0.01);
    // An oversized source frame retains its position until its policy is established.
    assert!((render(2000, Some(8000), 1400).0 - 170.0).abs() < 0.01);
}

#[test]
fn horizontal_frame_bounds_leave_drawings_other_references_and_oversize_unchanged() {
    let body = r#"<w:p><w:r><w:drawing><wp:anchor xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"><wp:extent cx="1270000" cy="508000"/></wp:anchor></w:drawing></w:r></w:p><w:sectPr><w:pgSz w:w="6000" w:h="6000"/><w:pgMar w:top="800" w:bottom="800" w:left="800" w:right="800"/></w:sectPr>"#;
    with_column_pager(docx(body, None, None), |pager, sections| {
        let crate::office::docx::model::Block::Para(p) = &sections[0].blocks[0] else {
            panic!("paragraph")
        };
        let crate::office::docx::model::Inline::Anchor(parsed) = &p.inlines[0] else {
            panic!("anchor")
        };
        assert!(!parsed.text_frame);
        let mut a = (**parsed).clone();
        a.h_rel = crate::office::docx::model::HRel::Page;
        a.x = Ok(260.0);
        let page = &pager.pages[0];
        let rect = |a: &crate::office::docx::model::Anchored| {
            super::anchor_rect(a, page, 40.0, 40.0, 40.0)
        };
        assert_eq!(rect(&a).0, 260.0);
        a.text_frame = true;
        assert_eq!(rect(&a).0, 200.0);
        a.x = Ok(-20.0);
        assert_eq!(rect(&a).0, 0.0);
        a.x = Ok(260.0);
        a.pic.w = 400.0;
        assert_eq!(rect(&a).0, 260.0);
        a.pic.w = 100.0;
        a.turned = true;
        assert_eq!(rect(&a).0, 260.0);
        a.turned = false;
        for reference in [
            crate::office::docx::model::HRel::Margin,
            crate::office::docx::model::HRel::Column,
            crate::office::docx::model::HRel::Character,
        ] {
            a.h_rel = reference;
            assert_eq!(rect(&a).0, 300.0);
        }
        a.h_rel = crate::office::docx::model::HRel::Page;
        a.x = Err("center".into());
        assert_eq!(rect(&a).0, 100.0);
    });
}

#[test]
fn blocked_first_line_in_a_low_continuous_column_moves_to_a_new_page() {
    let body = r#"<w:p xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:pPr><w:spacing w:line="3600" w:lineRule="exact"/><w:sectPr><w:pgSz w:w="6000" w:h="6000"/><w:pgMar w:top="720" w:bottom="720" w:left="720" w:right="720"/></w:sectPr></w:pPr><w:r><w:t>spacer</w:t><w:drawing xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"><wp:anchor behindDoc="0"><wp:positionH relativeFrom="page"><wp:posOffset>1905000</wp:posOffset></wp:positionH><wp:positionV relativeFrom="page"><wp:posOffset>2743200</wp:posOffset></wp:positionV><wp:extent cx="1447800" cy="1270000"/><wp:wrapSquare wrapText="bothSides"/><a:graphic><a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"><wps:wsp><wps:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="1447800" cy="1270000"/></a:xfrm><a:prstGeom prst="rect"/></wps:spPr><wps:txbx><w:txbxContent><w:p><w:r><w:t>FLOATING CONTENT</w:t></w:r></w:p></w:txbxContent></wps:txbx><wps:bodyPr lIns="0" tIns="0" rIns="0" bIns="0"/></wps:wsp></a:graphicData></a:graphic></wp:anchor></w:drawing></w:r></w:p><w:p xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:pPr><w:spacing w:line="240" w:lineRule="exact"/></w:pPr><w:r><w:t>alpha beta gamma delta</w:t></w:r></w:p><w:p xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:r><w:t>one</w:t></w:r></w:p><w:p xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:r><w:t>two</w:t></w:r></w:p><w:p xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:r><w:t>three</w:t></w:r></w:p><w:p xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:r><w:t>LOST FIRST LINE</w:t></w:r></w:p><w:sectPr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:type w:val="continuous"/><w:pgSz w:w="6000" w:h="6000"/><w:pgMar w:top="720" w:bottom="720" w:left="720" w:right="720"/><w:cols w:num="2" w:space="240"/></w:sectPr>"#;
    let out = pages(docx(body, None, None));
    let mut found = Vec::new();
    for (pi, page) in out.iter().enumerate() {
        for item in &page.items {
            if let crate::office::pptx::scene::Item::Text(run) = item
                && run.text == "LOST FIRST LINE"
            {
                found.push(pi);
                assert!(run.transform.ty as f64 + run.descent <= page.info.h);
            }
        }
    }
    assert_eq!(found, vec![1]);
}

#[test]
fn oversized_first_line_at_a_fresh_page_still_makes_progress() {
    for columns in ["", r#"<w:cols w:num="2" w:space="240"/>"#] {
        let body = format!(
            r#"<w:p><w:pPr><w:spacing w:line="8000" w:lineRule="exact"/></w:pPr><w:r><w:t>oversized source line</w:t></w:r></w:p><w:sectPr><w:pgSz w:w="6000" w:h="6000"/><w:pgMar w:top="720" w:bottom="720" w:left="720" w:right="720"/>{columns}</w:sectPr>"#
        );
        let out = pages(docx(&body, None, None));
        assert_eq!(out.len(), 1);
        let count = out[0].items.iter().filter(|item| {
            matches!(item, crate::office::pptx::scene::Item::Text(run) if run.text == "oversized source line")
        }).count();
        assert_eq!(count, 1);
    }
}

fn wrapped_picture_package(
    lines: usize,
    rel: &str,
    offset: i64,
    height: i64,
    wrap: &str,
    behind: bool,
    bare_break: bool,
) -> Vec<u8> {
    use std::io::{Read, Write};
    let mut body = String::new();
    for i in 0..lines {
        body.push_str(&format!(r#"<w:p><w:pPr><w:spacing w:line="200" w:lineRule="exact"/></w:pPr><w:r><w:rPr><w:sz w:val="16"/></w:rPr><w:t>LINE{i}</w:t></w:r></w:p>"#));
    }
    body.push_str(&format!(r#"<w:p><w:pPr><w:spacing w:line="240" w:lineRule="exact"/></w:pPr><w:r><w:t>ANCHOR</w:t><w:drawing><wp:anchor xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture" behindDoc="{}"><wp:positionH relativeFrom="column"><wp:posOffset>0</wp:posOffset></wp:positionH><wp:positionV relativeFrom="{rel}"><wp:posOffset>{offset}</wp:posOffset></wp:positionV><wp:extent cx="2540000" cy="{height}"/><wp:wrap{wrap} wrapText="bothSides"/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic><pic:blipFill><a:blip r:embed="rImage"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="2540000" cy="{height}"/></a:xfrm></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:anchor></w:drawing></w:r></w:p><w:sectPr><w:pgSz w:w="6000" w:h="6000"/><w:pgMar w:top="720" w:bottom="720" w:left="720" w:right="720"/></w:sectPr>"#, usize::from(behind)));
    if matches!(wrap, "Tight" | "Through") {
        body = body.replace(&format!("<wp:wrap{wrap} wrapText=\"bothSides\"/>"), &format!(r#"<wp:wrap{wrap} wrapText="bothSides"><wp:wrapPolygon edited="0"><wp:start x="0" y="0"/><wp:lineTo x="21600" y="0"/><wp:lineTo x="21600" y="21600"/><wp:lineTo x="0" y="21600"/><wp:lineTo x="0" y="0"/></wp:wrapPolygon></wp:wrap{wrap}>"#));
    }
    if bare_break {
        body = body.replace("<w:t>ANCHOR</w:t>", r#"<w:br w:type="page"/>"#);
    }
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
                data = String::from_utf8(data).unwrap().replace("</Relationships>", r#"<Relationship Id="rImage" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/control.png"/></Relationships>"#).into_bytes();
            }
            z.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            z.write_all(&data).unwrap();
        }
        let mut pix = tiny_skia::Pixmap::new(2, 2).unwrap();
        pix.fill(tiny_skia::Color::from_rgba8(0, 0, 255, 255));
        z.start_file(
            "word/media/control.png",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        z.write_all(&pix.encode_png().unwrap()).unwrap();
        z.finish().unwrap();
    }
    output.into_inner()
}

/// [`wrapped_picture_package`] with a 150 pt wide picture: the 78 pt gap beside it
/// holds the anchor line's word, which the 28 pt gap beside the 200 pt one does not.
fn narrow_wrapped_picture(bytes: Vec<u8>) -> Vec<u8> {
    use std::io::{Read, Write};
    let mut input = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut output = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut output);
        for i in 0..input.len() {
            let mut file = input.by_index(i).unwrap();
            let name = file.name().unwrap().to_string();
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes).unwrap();
            if name == "word/document.xml" {
                bytes = String::from_utf8(bytes)
                    .unwrap()
                    .replace(r#"cx="2540000""#, r#"cx="1905000""#)
                    .into_bytes();
            }
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(&bytes).unwrap();
        }
        zip.finish().unwrap();
    }
    output.into_inner()
}

#[test]
fn wrapped_picture_fit_uses_physical_page_and_moves_its_anchor() {
    for wrap in ["Square", "Tight", "Through", "TopAndBottom"] {
        for rel in ["paragraph", "line"] {
            for (lines, expected_page) in [(6, 0), (8, 0), (11, 1), (20, 1)] {
                let out = pages(wrapped_picture_package(
                    lines, rel, 381000, 1905000, wrap, false, false,
                ));
                let anchors: Vec<_> = out
                    .iter()
                    .enumerate()
                    .flat_map(|(i, p)| {
                        p.items.iter().filter_map(move |item| {
                            if matches!(item, crate::office::pptx::scene::Item::Text(r) if r.text == "ANCHOR")
                            {
                                Some(i)
                            } else {
                                None
                            }
                        })
                    })
                    .collect();
                assert_eq!(anchors, vec![expected_page], "lines={lines}, rel={rel}");
                let images: Vec<_> = out
                    .iter()
                    .enumerate()
                    .flat_map(|(i, p)| {
                        p.items.iter().filter_map(move |item| {
                            if let crate::office::pptx::scene::Item::Image(image) = item {
                                Some((i, image))
                            } else {
                                None
                            }
                        })
                    })
                    .collect();
                assert_eq!(images.len(), 1);
                assert_eq!(images[0].0, expected_page);
                let image = images[0].1;
                assert!(image.transform.ty + image.transform.sy <= 300.01);
                let lines_found = out.iter().flat_map(|p| &p.items).filter(|item| matches!(item, crate::office::pptx::scene::Item::Text(r) if r.text.starts_with("LINE"))).count();
                assert_eq!(lines_found, lines);
            }
        }
    }
}

#[test]
fn wrapped_picture_fit_preserves_fixed_negative_and_oversize_policies() {
    for (rel, offset, height, wrap, behind) in [
        ("page", 3378200, 1905000, "Square", false),
        ("margin", 2921000, 1905000, "Square", false),
        ("paragraph", -381000, 1905000, "Square", false),
        ("paragraph", 381000, 4445000, "Square", false),
        ("paragraph", 381000, 1905000, "None", false),
    ] {
        let out = pages(narrow_wrapped_picture(wrapped_picture_package(
            20, rel, offset, height, wrap, behind, false,
        )));
        assert_eq!(out.len(), 1, "{rel}, {offset}, {height}, {wrap}, {behind}");
        assert_eq!(
            out[0]
                .items
                .iter()
                .filter(|item| matches!(item, crate::office::pptx::scene::Item::Image(_)))
                .count(),
            1
        );
    }
    let out = pages(wrapped_picture_package(
        0,
        "paragraph",
        381000,
        4445000,
        "Square",
        false,
        false,
    ));
    assert_eq!(out.len(), 1);
}

#[test]
fn wrapped_picture_on_a_bare_page_break_moves_once_and_keeps_its_image() {
    let out = pages(wrapped_picture_package(
        20,
        "paragraph",
        381000,
        1905000,
        "Square",
        false,
        true,
    ));
    assert!(out.len() <= 3);
    let images: Vec<_> = out
        .iter()
        .enumerate()
        .flat_map(|(i, p)| {
            p.items.iter().filter_map(move |item| {
                if let crate::office::pptx::scene::Item::Image(image) = item {
                    Some((i, image))
                } else {
                    None
                }
            })
        })
        .collect();
    assert_eq!(images.len(), 1);
    assert_eq!(images[0].0, 1);
    assert!(images[0].1.transform.ty + images[0].1.transform.sy <= 300.01);
}

#[test]
fn wrapped_picture_on_a_later_line_keeps_the_consumed_prefix() {
    use std::io::{Read, Write};
    let bytes = narrow_wrapped_picture(wrapped_picture_package(
        0, "line", 381000, 1905000, "Square", false, false,
    ));
    let mut input = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut output = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut output);
        for i in 0..input.len() {
            let mut file = input.by_index(i).unwrap();
            let name = file.name().unwrap().to_string();
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes).unwrap();
            if name == "word/document.xml" {
                let prefix = (0..12)
                    .map(|i| format!("<w:t>PREFIX{i}</w:t><w:br/>"))
                    .collect::<String>();
                bytes = String::from_utf8(bytes)
                    .unwrap()
                    .replace("<w:t>ANCHOR</w:t>", &format!("{prefix}<w:t>ANCHOR</w:t>"))
                    .into_bytes();
            }
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(&bytes).unwrap();
        }
        zip.finish().unwrap();
    }
    let out = pages(output.into_inner());
    let mut prefixes = String::new();
    let mut images = Vec::new();
    for (pi, page) in out.iter().enumerate() {
        for item in &page.items {
            match item {
                crate::office::pptx::scene::Item::Text(run) => prefixes.push_str(&run.text),
                crate::office::pptx::scene::Item::Image(image) => {
                    images.push(pi);
                    assert!(image.transform.ty + image.transform.sy <= page.info.h as f32 + 0.01);
                }
                _ => {}
            }
        }
    }
    assert_eq!(
        prefixes,
        (0..12).map(|i| format!("PREFIX{i}")).collect::<String>() + "ANCHOR"
    );
    assert_eq!(images, vec![1]);
}

fn picture_with_note(reference: &str) -> Vec<u8> {
    use std::io::{Read, Write};
    let bytes = wrapped_picture_package(8, "paragraph", 381000, 1905000, "Square", false, false);
    let mut input = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut output = std::io::Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut output);
        for i in 0..input.len() {
            let mut f = input.by_index(i).unwrap();
            let name = f.name().unwrap().to_string();
            let mut data = Vec::new();
            f.read_to_end(&mut data).unwrap();
            if name == "word/document.xml" {
                let mut xml = String::from_utf8(data)
                    .unwrap()
                    .replace("2540000", "1016000");
                let note = r#"<w:footnoteReference w:id="1"/>"#;
                if reference.starts_with("cross-") {
                    xml = xml
                        .replace("w:w=\"6000\"", "w:w=\"10000\"")
                        .replace(
                            "</w:sectPr>",
                            r#"<w:cols w:num="2" w:space="400"/></w:sectPr>"#,
                        )
                        .replace(
                            "<wp:posOffset>0</wp:posOffset></wp:positionH>",
                            "<wp:posOffset>2844800</wp:posOffset></wp:positionH>",
                        );
                    let text = (0..180)
                        .map(|i| format!("RIGHT{i:03} "))
                        .collect::<String>();
                    if reference == "cross-covered" {
                        xml = xml
                            .replace("1016000", "2540000")
                            .replace("relativeFrom=\"paragraph\"", "relativeFrom=\"page\"")
                            .replace("381000", "457200");
                    }
                    if matches!(reference, "cross-same" | "cross-covered") {
                        xml = xml.replace("</w:drawing>", &format!(r#"</w:drawing><w:br w:type="column"/><w:t xml:space="preserve">{text}</w:t>"#));
                    } else {
                        xml = xml.replace("<w:sectPr>", &format!(r#"<w:p><w:r><w:br w:type="column"/></w:r></w:p><w:p><w:r><w:t xml:space="preserve">{text}</w:t></w:r></w:p><w:sectPr>"#));
                    }
                } else if reference == "fresh-offset" {
                    let start = xml.find("<w:body>").unwrap() + "<w:body>".len();
                    let anchor = xml.find(r#"<w:p><w:pPr><w:spacing w:line="240""#).unwrap();
                    xml.replace_range(start..anchor, "");
                    xml = xml
                        .replace("381000", "5080000")
                        .replace("</w:drawing>", "</w:drawing><w:br/><w:t>LATER</w:t>");
                } else if reference == "carry" {
                    // The test injects an existing carry state without a new reference.
                } else if reference == "next-column" || reference == "table-column" {
                    xml = xml.replace("w:w=\"6000\"", "w:w=\"10000\"").replace(
                        "</w:sectPr>",
                        r#"<w:cols w:num="2" w:space="400"/></w:sectPr>"#,
                    );
                    let tail = if reference == "table-column" {
                        format!(
                            r#"<w:p><w:r><w:br w:type="column"/></w:r></w:p><w:tbl><w:tblPr><w:tblW w:w="1600" w:type="dxa"/></w:tblPr><w:tblGrid><w:gridCol w:w="1600"/></w:tblGrid><w:tr><w:tc><w:p><w:r><w:t>LATER</w:t>{note}</w:r></w:p></w:tc></w:tr></w:tbl>"#
                        )
                    } else {
                        format!(
                            r#"<w:p><w:r><w:br w:type="column"/><w:t>LATER</w:t>{note}</w:r></w:p>"#
                        )
                    };
                    xml = xml.replace("<w:sectPr>", &format!("{tail}<w:sectPr>"));
                } else if matches!(reference, "same-paragraph" | "fresh-keep") {
                    xml = xml.replace(
                        "</w:drawing>",
                        &format!(r#"</w:drawing><w:br/><w:t>LATER</w:t>{note}"#),
                    );
                    if reference == "fresh-keep" {
                        let start = xml.find("<w:body>").unwrap() + "<w:body>".len();
                        let anchor = xml.find(r#"<w:p><w:pPr><w:spacing w:line="240""#).unwrap();
                        xml.replace_range(start..anchor, "");
                        xml = xml
                            .replace("381000", "1270000")
                            .replace("<w:pPr>", "<w:pPr><w:keepLines/><w:widowControl/>");
                    }
                } else if reference == "later" {
                    xml = xml.replace(
                        "<w:sectPr>",
                        &format!(r#"<w:p><w:r><w:t>LATER</w:t>{note}</w:r></w:p><w:sectPr>"#),
                    );
                } else {
                    let word = if matches!(
                        reference,
                        "existing"
                            | "column-local"
                            | "column-pair"
                            | "column-impossible"
                            | "column-tall"
                    ) {
                        "LINE0"
                    } else {
                        "ANCHOR"
                    };
                    xml = xml.replace(
                        &format!("<w:t>{word}</w:t>"),
                        &format!("<w:t>{word}</w:t>{note}"),
                    );
                }
                if reference.starts_with("column-") {
                    xml = xml.replace("w:w=\"6000\"", "w:w=\"10000\"").replace(
                        "</w:sectPr>",
                        r#"<w:cols w:num="2" w:space="400"/></w:sectPr>"#,
                    );
                    if reference == "column-tall" {
                        xml = xml.replace("381000", "2540000");
                    } else if reference == "column-pair" {
                        let start = xml.find("<wp:anchor ").unwrap();
                        let end = xml[start..].find("</wp:anchor>").unwrap()
                            + start
                            + "</wp:anchor>".len();
                        let companion = xml[start..end].replace(
                            "<wp:posOffset>0</wp:posOffset></wp:positionH>",
                            "<wp:posOffset>2844800</wp:posOffset></wp:positionH>",
                        );
                        xml.insert_str(end, &companion);
                    } else if reference == "column-impossible" {
                        xml = xml.replace(
                            "<wp:posOffset>0</wp:posOffset></wp:positionH>",
                            "<wp:posOffset>12700000</wp:posOffset></wp:positionH>",
                        );
                    }
                }
                data = xml.into_bytes();
            } else if name == "word/_rels/document.xml.rels" {
                data = String::from_utf8(data).unwrap().replace("</Relationships>", r#"<Relationship Id="rNote" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/footnotes" Target="footnotes.xml"/></Relationships>"#).into_bytes();
            }
            z.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            z.write_all(&data).unwrap();
        }
        z.start_file(
            "word/footnotes.xml",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        z.write_all(br#"<w:footnotes xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:footnote w:type="separator" w:id="-1"><w:p><w:r><w:separator/></w:r></w:p></w:footnote><w:footnote w:id="1"><w:p><w:pPr><w:spacing w:line="400" w:lineRule="exact"/></w:pPr><w:r><w:t>NOTE</w:t></w:r></w:p></w:footnote></w:footnotes>"#).unwrap();
        z.finish().unwrap();
    }
    output.into_inner()
}

#[test]
fn wrapped_pictures_respect_existing_pending_and_later_footnotes() {
    for reference in [
        "existing",
        "pending",
        "later",
        "same-paragraph",
        "next-column",
        "table-column",
        "fresh-keep",
    ] {
        let out = pages(picture_with_note(reference));
        let disjoint_columns = matches!(reference, "next-column" | "table-column");
        assert_eq!(
            out.len(),
            if disjoint_columns { 1 } else { 2 },
            "{reference}"
        );
        let mut images = 0;
        let mut notes = 0;
        for page in &out {
            let note_y = page.items.iter().find_map(|item| {
                if let crate::office::pptx::scene::Item::Text(r) = item
                    && r.text == "NOTE"
                {
                    notes += 1;
                    return Some(r.transform.ty as f64);
                }
                None
            });
            for item in &page.items {
                if let crate::office::pptx::scene::Item::Image(image) = item {
                    images += 1;
                    let note_overlap_x = page.info.note_areas.iter().any(|area| {
                        let left = image.transform.tx as f64;
                        let right = left + image.transform.sx as f64;
                        area.context.x < right && area.context.x + area.context.width > left
                    });
                    if disjoint_columns {
                        assert!(
                            !note_overlap_x,
                            "{reference}: image x={} width={} note areas={:?}",
                            image.transform.tx, image.transform.sx, page.info.note_areas
                        );
                    }
                    if let Some(y) = note_y.filter(|_| note_overlap_x) {
                        assert!(
                            (image.transform.ty + image.transform.sy) as f64 <= y,
                            "{reference}: image intersects note"
                        );
                    }
                }
            }
        }
        assert_eq!((images, notes), (1, 1));
        let all = out.iter().map(page_texts).collect::<String>();
        assert!(all.contains("ANCHOR"));
        if reference != "fresh-keep" {
            for i in 0..8 {
                assert!(all.contains(&format!("LINE{i}")));
            }
        }
    }
}

#[test]
fn wrapped_picture_fit_reserves_injected_note_carry_and_resets_on_new_page() {
    with_column_pager(picture_with_note("carry"), |pager, sections| {
        pager.pages[0].note_carry = Some((1, 0.0));
        let props = &sections[0].props;
        let units = crate::office::docx::layout::Flow::new(pager.env, 15).units(
            &sections[0].blocks,
            props.columns()[0].1,
            false,
        );
        pager.place_units(&units, 0, props);
        assert_eq!(pager.pages.len(), 2);
        let image_pages: Vec<_> = pager
            .placed
            .iter()
            .filter_map(|placed| {
                if let super::Placed::Line {
                    unit,
                    line,
                    page,
                    alt,
                    ..
                } = placed
                    && let crate::office::docx::layout::Unit::Para(original) = &units[*unit]
                {
                    let p = alt.as_deref().unwrap_or(original);
                    if !p.lines[*line].anchors.is_empty() {
                        return Some(*page);
                    }
                }
                None
            })
            .collect();
        assert_eq!(image_pages, vec![1]);
        let retained = pager.st.picture_bottom;
        let snap = pager.snapshot();
        pager.st.picture_bottom += 100.0;
        pager.restore(&snap);
        assert_eq!(pager.st.picture_bottom, retained);
        pager.new_page(0, props, sections);
        assert_eq!(pager.st.picture_bottom, 0.0);
    });
}

#[test]
fn wrapped_picture_fresh_page_offset_preserves_following_text_without_notes() {
    let out = pages(picture_with_note("fresh-offset"));
    assert_eq!(out.len(), 1);
    assert!(page_texts(&out[0]).contains("ANCHOR"));
    assert!(page_texts(&out[0]).contains("LATER"));
    assert_eq!(
        out[0]
            .items
            .iter()
            .filter(|item| matches!(item, crate::office::pptx::scene::Item::Image(_)))
            .count(),
        1
    );
}

#[test]
fn wrapped_picture_column_retry_preserves_local_fit_and_page_containment() {
    for (kind, expected_page, count) in [
        ("column-local", 0, 1),
        ("column-pair", 1, 2),
        ("column-impossible", 0, 1),
        ("column-tall", 1, 1),
    ] {
        let out = pages(picture_with_note(kind));
        assert_eq!(out.len(), expected_page + 1, "{kind}");
        let images: Vec<_> = out
            .iter()
            .enumerate()
            .flat_map(|(page, p)| {
                p.items.iter().filter_map(move |item| {
                    if let crate::office::pptx::scene::Item::Image(image) = item {
                        Some((page, image))
                    } else {
                        None
                    }
                })
            })
            .collect();
        assert_eq!(images.len(), count);
        assert!(images.iter().all(|(page, _)| *page == expected_page));
        if kind != "column-impossible" {
            assert!(images.iter().all(|(_, image)| image.transform.tx >= 0.0
                && image.transform.tx + image.transform.sx <= 500.01));
        }
        assert!(page_texts(&out[expected_page]).contains("ANCHOR"));
    }
}

#[test]
fn oversized_notes_without_pictures_keep_paragraph_and_row_progress() {
    let reference = r#"<w:p><w:r><w:t>REFERENCE</w:t><w:footnoteReference w:id="1"/></w:r></w:p>"#;
    let section = r#"<w:sectPr><w:pgSz w:w="6000" w:h="6000"/><w:pgMar w:top="720" w:bottom="720" w:left="720" w:right="720"/></w:sectPr>"#;
    for table in [false, true] {
        let body = if table {
            format!(
                r#"<w:tbl><w:tblPr><w:tblW w:w="4000" w:type="dxa"/></w:tblPr><w:tblGrid><w:gridCol w:w="4000"/></w:tblGrid><w:tr><w:tc>{reference}</w:tc></w:tr></w:tbl>{section}"#
            )
        } else {
            format!("{reference}{section}")
        };
        let out = pages(crate::office::docx::testutil::with_footnote(
            docx(&body, None, None),
            &"note ".repeat(400),
        ));
        assert!(out.len() > 1 && out.len() < 30);
        assert_eq!(
            out.iter()
                .map(page_texts)
                .collect::<String>()
                .matches("note")
                .count(),
            400
        );
        assert!(page_texts(&out[0]).contains("REFERENCE"));
    }
}

fn assert_picture_exclusions(kind: &str) {
    let out = pages(picture_with_note(kind));
    assert!(out.len() >= 2, "{kind}");
    let image = out[0]
        .items
        .iter()
        .find_map(|item| {
            if let crate::office::pptx::scene::Item::Image(image) = item {
                Some(image)
            } else {
                None
            }
        })
        .unwrap();
    let x = image.transform.tx as f64;
    let right = x + image.transform.sx as f64;
    let y = image.transform.ty as f64;
    let bottom = y + image.transform.sy as f64;
    let all = out.iter().map(page_texts).collect::<String>();
    assert_eq!(all.matches("RIGHT").count(), 180, "{kind}");
    let normalized = all
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>();
    let expected = (0..180).map(|i| format!("RIGHT{i:03}")).collect::<String>();
    assert!(
        normalized.ends_with(&expected),
        "{kind}: source order changed"
    );
    if kind == "cross-covered" {
        let first = out[0]
            .items
            .iter()
            .find_map(|item| {
                if let crate::office::pptx::scene::Item::Text(run) = item {
                    run.text
                        .contains("RIGHT")
                        .then_some(run.transform.ty as f64)
                } else {
                    None
                }
            })
            .unwrap();
        assert!(
            first >= bottom,
            "fresh destination clearance discarded: {first} < {bottom}"
        );
    }
    for item in &out[0].items {
        if let crate::office::pptx::scene::Item::Text(run) = item {
            if run.text.contains("RIGHT")
                && run.transform.ty as f64 > y
                && (run.transform.ty as f64) < bottom
            {
                assert!(
                    run.transform.tx as f64 >= right - 0.1
                        || run.transform.tx as f64 + run.width <= x + 0.1,
                    "{kind}: text intersects companion"
                );
            }
            if run.text.starts_with("LINE") {
                assert!((run.transform.tx - 36.0).abs() < 0.1);
            }
        }
    }
    assert!(out[1].items.iter().any(|item| matches!(item, crate::office::pptx::scene::Item::Text(run) if run.text.contains("RIGHT") && (run.transform.tx - 36.0).abs() < 0.1)), "{kind}: stale exclusions on next page");
    assert_eq!(
        out.iter()
            .flat_map(|p| &p.items)
            .filter(|item| matches!(item, crate::office::pptx::scene::Item::Image(_)))
            .count(),
        1
    );
}

#[test]
fn wrapped_picture_exclusions_follow_columns_and_release_after_page_transition() {
    for kind in ["cross-later", "cross-same"] {
        assert_picture_exclusions(kind);
    }
}

#[test]
fn wrapped_picture_continuation_keeps_fresh_destination_clearance() {
    assert_picture_exclusions("cross-covered");
}

#[test]
fn modern_automatic_before_collapses_at_forced_page_tops() {
    for mode in [12, 14, 15] {
        for boundary in ["hard", "pageBefore", "continuousHard"] {
            let zero = auto_before_boundary_target_mode(
                boundary,
                r#"w:before="0" w:beforeAutospacing="0""#,
                false,
                mode,
            );
            let auto = auto_before_boundary_target_mode(
                boundary,
                r#"w:before="280" w:beforeAutospacing="1""#,
                false,
                mode,
            );
            assert_eq!(zero.0, 1);
            assert_eq!(auto.0, zero.0);
            // Mode 14 drops spacing after a page break but keeps it on pageBreakBefore.
            let collapses = mode >= 15 || (mode >= 14 && boundary != "pageBefore");
            let expected = if collapses { 0.0 } else { 14.0 };
            assert!(
                (auto.1 - zero.1 - expected).abs() < 0.01,
                "mode={mode} boundary={boundary}: {auto:?} {zero:?}"
            );
        }
    }
}

#[test]
fn modern_automatic_before_preserves_section_and_column_starts() {
    for boundary in [
        "section",
        "oddSection",
        "evenSection",
        "oddSectionAfterPage",
        "evenSectionAfterPage",
        "continuous",
        "continuousPageBefore",
        "hard-column",
    ] {
        let zero = auto_before_boundary_target_mode(
            boundary,
            r#"w:before="0" w:beforeAutospacing="0""#,
            false,
            15,
        );
        let auto = auto_before_boundary_target_mode(
            boundary,
            r#"w:before="280" w:beforeAutospacing="1""#,
            false,
            15,
        );
        assert_eq!(auto.0, zero.0);
        let expected_page = match boundary {
            "oddSection" | "oddSectionAfterPage" => Some(2),
            "evenSection" => Some(1),
            "evenSectionAfterPage" => Some(3),
            _ => None,
        };
        if let Some(page) = expected_page {
            assert_eq!(auto.0, page, "{boundary}: parity boundary");
        }
        assert!(
            (auto.1 - zero.1 - 14.0).abs() < 0.01,
            "{boundary}: {auto:?} {zero:?}"
        );
    }
}

#[test]
fn modern_forced_page_tops_drop_inherited_and_explicit_spacing() {
    for boundary in ["hard", "pageBefore"] {
        let zero = auto_before_boundary_target_mode(
            boundary,
            r#"w:before="0" w:beforeAutospacing="0""#,
            false,
            15,
        );
        let inherited = auto_before_boundary_target_mode(boundary, "", true, 15);
        let explicit = auto_before_boundary_target_mode(
            boundary,
            r#"w:before="280" w:beforeAutospacing="0""#,
            true,
            15,
        );
        assert_eq!(inherited.0, zero.0);
        assert!((inherited.1 - zero.1).abs() < 0.01);
        assert!((explicit.1 - zero.1).abs() < 0.01, "{boundary}");
    }
}

fn behind_note_package(reference: &str, wrap: &str, spanning: bool) -> Vec<u8> {
    use std::io::{Read, Write};
    let mut input =
        zip::ZipArchive::new(std::io::Cursor::new(picture_with_note(reference))).unwrap();
    let mut output = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut output);
        for i in 0..input.len() {
            let mut entry = input.by_index(i).unwrap();
            let name = entry.name().unwrap().to_string();
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).unwrap();
            if name == "word/document.xml" {
                let mut xml = String::from_utf8(bytes)
                    .unwrap()
                    .replace("behindDoc=\"0\"", "behindDoc=\"1\"");
                let wrapping = if matches!(wrap, "Tight" | "Through") {
                    format!(
                        r#"<wp:wrap{wrap} wrapText="bothSides"><wp:wrapPolygon edited="0"><wp:start x="0" y="0"/><wp:lineTo x="21600" y="0"/><wp:lineTo x="21600" y="21600"/><wp:lineTo x="0" y="21600"/><wp:lineTo x="0" y="0"/></wp:wrapPolygon></wp:wrap{wrap}>"#
                    )
                } else {
                    format!(r#"<wp:wrap{wrap} wrapText="bothSides"/>"#)
                };
                xml = xml.replace(r#"<wp:wrapSquare wrapText="bothSides"/>"#, &wrapping);
                if spanning {
                    xml = xml.replace("1016000", "4572000");
                }
                bytes = xml.into_bytes();
            }
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(&bytes).unwrap();
        }
        zip.finish().unwrap();
    }
    output.into_inner()
}

fn assert_behind_note_content(out: &[super::OutPage], reference: &str) {
    let all = out.iter().map(page_texts).collect::<String>();
    assert_eq!(all.matches("NOTE").count(), 1, "{reference}");
    assert_eq!(all.matches("ANCHOR").count(), 1, "{reference}");
    let mut previous = 0;
    if reference != "fresh-keep" {
        for i in 0..8 {
            let marker = format!("LINE{i}");
            assert_eq!(all.matches(&marker).count(), 1, "{reference}: {marker}");
            let at = all.find(&marker).unwrap();
            assert!(at >= previous);
            previous = at;
        }
        assert!(all.find("ANCHOR").unwrap() > previous);
    }
    let mut images = 0;
    for page in out {
        for (image_index, item) in page.items.iter().enumerate() {
            let crate::office::pptx::scene::Item::Image(image) = item else {
                continue;
            };
            images += 1;
            let left = image.transform.tx as f64;
            let right = left + image.transform.sx as f64;
            let top = image.transform.ty as f64;
            let bottom = top + image.transform.sy as f64;
            for text in &page.items {
                if let crate::office::pptx::scene::Item::Text(run) = text
                    && run.text == "NOTE"
                {
                    let x = run.transform.tx as f64;
                    let y = run.transform.ty as f64;
                    assert!(
                        right <= x || left >= x + run.width || bottom <= y || top >= y + run.size,
                        "{reference}: image/note intersection"
                    );
                }
            }
            let anchor_index = page.items.iter().position(|item| matches!(item, crate::office::pptx::scene::Item::Text(run) if run.text.contains("ANCHOR"))).unwrap();
            assert!(
                image_index < anchor_index,
                "behind image must retain its paint layer"
            );
        }
    }
    assert_eq!(images, 1, "{reference}");
}

#[test]
fn behind_wrapped_rasters_reserve_existing_pending_and_later_notes() {
    for wrap in ["Square", "Tight", "Through"] {
        for reference in [
            "existing",
            "pending",
            "later",
            "same-paragraph",
            "fresh-keep",
        ] {
            let out = pages(behind_note_package(reference, wrap, false));
            assert_eq!(out.len(), 2, "{wrap}/{reference}");
            assert_behind_note_content(&out, reference);
        }
    }
}

#[test]
fn behind_wrapped_rasters_distinguish_disjoint_and_spanning_note_columns() {
    for wrap in ["Square", "Tight", "Through"] {
        for reference in ["next-column", "table-column"] {
            for spanning in [false, true] {
                let out = pages(behind_note_package(reference, wrap, spanning));
                assert_eq!(
                    out.len(),
                    if spanning { 2 } else { 1 },
                    "{wrap}/{reference}/{spanning}"
                );
                assert_behind_note_content(&out, reference);
                let all = out.iter().map(page_texts).collect::<String>();
                assert_eq!(all.matches("LATER").count(), 1);
            }
        }
    }
}

#[test]
fn behind_wrapped_rasters_fit_physical_pages_and_keep_nowrap_policies() {
    for wrap in ["Square", "Tight", "Through"] {
        for lines in [0, 20] {
            let back = pages(wrapped_picture_package(
                lines,
                "paragraph",
                381000,
                1905000,
                wrap,
                true,
                false,
            ));
            let front = pages(wrapped_picture_package(
                lines,
                "paragraph",
                381000,
                1905000,
                wrap,
                false,
                false,
            ));
            assert_eq!(back.len(), front.len(), "{wrap}/{lines}");
            let geometry = |out: &[super::OutPage]| {
                out.iter()
                    .enumerate()
                    .flat_map(|(p, page)| {
                        page.items.iter().filter_map(move |item| match item {
                            crate::office::pptx::scene::Item::Image(i) => Some((
                                p,
                                i.transform.tx,
                                i.transform.ty,
                                i.transform.sx,
                                i.transform.sy,
                            )),
                            _ => None,
                        })
                    })
                    .collect::<Vec<_>>()
            };
            assert_eq!(geometry(&back), geometry(&front));
            assert_eq!(
                back.iter().map(page_texts).collect::<String>(),
                front.iter().map(page_texts).collect::<String>()
            );
        }
    }
    let out = pages(behind_note_package("existing", "None", false));
    assert_eq!(out.len(), 1);
    let all = out.iter().map(page_texts).collect::<String>();
    assert_eq!(all.matches("NOTE").count(), 1);
    assert_eq!(all.matches("ANCHOR").count(), 1);
    assert_eq!(
        out[0]
            .items
            .iter()
            .filter(|i| matches!(i, crate::office::pptx::scene::Item::Image(_)))
            .count(),
        1
    );
}

#[test]
fn body_merged_textbox_follows_its_source_slice_across_pages() {
    for align in ["top", "center", "bottom"] {
        for anchor_only in [false, true] {
            let content = (0..45).map(|i| {
                let anchor = if i == 24 {
                    r##"<w:r><w:pict><v:shape xmlns:v="urn:schemas-microsoft-com:vml" id="deferred" type="#_x0000_t202" style="position:absolute;margin-left:45pt;margin-top:0pt;width:50pt;height:10pt"><v:textbox><w:txbxContent><w:p><w:r><w:t>BOX0000</w:t></w:r></w:p></w:txbxContent></v:textbox></v:shape></w:pict></w:r>"##
                } else { "" };
                let text = if anchor_only && i == 24 { String::new() } else { format!("<w:r><w:t>LINE{i:04}</w:t></w:r>") };
                format!(r#"<w:p><w:pPr><w:spacing w:before="0" w:after="0" w:line="240" w:lineRule="exact"/><w:widowControl w:val="0"/></w:pPr>{text}{anchor}</w:p>"#)
            }).collect::<String>();
            let body = format!(
                r#"<w:tbl><w:tblPr><w:tblCellMar><w:top w:w="0" w:type="dxa"/><w:bottom w:w="0" w:type="dxa"/></w:tblCellMar></w:tblPr><w:tblGrid><w:gridCol w:w="2400"/><w:gridCol w:w="2400"/></w:tblGrid><w:tr><w:tc><w:tcPr><w:vMerge w:val="restart"/><w:vAlign w:val="{align}"/></w:tcPr>{content}</w:tc><w:tc>{}</w:tc></w:tr><w:tr><w:tc><w:tcPr><w:vMerge/></w:tcPr>{}</w:tc><w:tc>{}</w:tc></w:tr><w:tr><w:tc><w:tcPr><w:vMerge/></w:tcPr>{}</w:tc><w:tc>{}</w:tc></w:tr></w:tbl><w:sectPr><w:pgSz w:w="7200" w:h="6000"/><w:pgMar w:top="400" w:bottom="400" w:left="400" w:right="400"/></w:sectPr>"#,
                para("SIB0").repeat(10),
                para(""),
                para("SIB1").repeat(10),
                para(""),
                para("SIB2").repeat(10)
            );
            let out = pages(docx(&body, None, None));
            let texts = out
                .iter()
                .map(|p| {
                    p.items
                        .iter()
                        .filter_map(|i| match i {
                            crate::office::pptx::scene::Item::Text(t) => Some(t.text.clone()),
                            _ => None,
                        })
                        .collect::<String>()
                })
                .collect::<Vec<_>>();
            let occurrences = texts
                .iter()
                .enumerate()
                .flat_map(|(page, text)| std::iter::repeat_n(page, text.matches("BOX0000").count()))
                .collect::<Vec<_>>();
            assert_eq!(occurrences.len(), 1, "{align} anchor_only={anchor_only}");
            let owner = if anchor_only { "LINE0025" } else { "LINE0024" };
            let owner_page = out
                .iter()
                .position(|p| {
                    p.items
                        .iter()
                        .any(|it| matches!(it, crate::office::pptx::scene::Item::Text(t) if t.text == owner))
                })
                .unwrap();
            assert_eq!(occurrences[0], owner_page);
            assert!(owner_page > 0);
            let key = |label: &str| {
                out.iter()
                    .flat_map(|p| &p.items)
                    .find_map(|item| match item {
                        crate::office::pptx::scene::Item::Text(run)
                            if run.text.starts_with(label) =>
                        {
                            Some(run.paragraph)
                        }
                        _ => None,
                    })
                    .unwrap()
            };
            assert!(key(if anchor_only { "LINE0023" } else { "LINE0024" }) < key("BOX"));
            assert!(key("BOX") < key("LINE0025"));
        }
    }
}

#[test]
fn retained_header_anchors_follow_source_slices_on_every_occurrence() {
    let p = |text: &str, anchors: &str| {
        format!(
            r#"<w:p><w:pPr><w:spacing w:before="0" w:after="0" w:line="240" w:lineRule="exact"/></w:pPr><w:r><w:rPr><w:sz w:val="12"/></w:rPr><w:t>{text}</w:t></w:r>{anchors}</w:p>"#
        )
    };
    let boxes = (0..2).map(|i| format!(r##"<w:r><w:pict><v:shape xmlns:v="urn:schemas-microsoft-com:vml" id="box{i}" type="#_x0000_t202" style="position:absolute;margin-left:45pt;margin-top:0pt;width:70pt;height:10pt;z-index:{}"><v:textbox><w:txbxContent><w:p><w:r><w:rPr><w:sz w:val="12"/></w:rPr><w:t>BOX000{i}</w:t></w:r></w:p></w:txbxContent></v:textbox></v:shape></w:pict></w:r>"##, 200-i*100)).collect::<String>();
    let header = format!(
        r#"<w:tr><w:trPr><w:tblHeader/></w:trPr><w:tc><w:tcPr><w:vMerge w:val="restart"/></w:tcPr>{}{}{}{}</w:tc><w:tc>{}</w:tc></w:tr><w:tr><w:trPr><w:tblHeader/></w:trPr><w:tc><w:tcPr><w:vMerge/></w:tcPr>{}</w:tc><w:tc>{}</w:tc></w:tr>"#,
        p("HMG0000", ""),
        p("HMG0001", ""),
        p("HMG0002", &boxes),
        p("HMG0003", ""),
        p("SIB0000", ""),
        p("", ""),
        p("SIB0001", "")
    );
    let body_rows = (0..30)
        .map(|i| {
            format!(
                "<w:tr><w:tc>{}</w:tc><w:tc>{}</w:tc></w:tr>",
                p(&format!("BODY{i:04}"), ""),
                p("", "")
            )
        })
        .collect::<String>();
    let body = format!(
        r#"<w:tbl><w:tblPr><w:tblCellMar><w:top w:w="0" w:type="dxa"/><w:bottom w:w="0" w:type="dxa"/></w:tblCellMar></w:tblPr><w:tblGrid><w:gridCol w:w="2600"/><w:gridCol w:w="2600"/></w:tblGrid>{header}{body_rows}</w:tbl><w:sectPr><w:pgSz w:w="7200" w:h="4000"/><w:pgMar w:top="400" w:bottom="400" w:left="400" w:right="400"/></w:sectPr>"#
    );
    let out = pages(docx(&body, None, None));
    assert!(out.len() > 1);
    for page in &out {
        let key = |label: &str| {
            page.items
                .iter()
                .filter_map(|item| match item {
                    crate::office::pptx::scene::Item::Text(run) if run.text == label => {
                        Some(run.paragraph)
                    }
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        let labels = ["HMG0002", "BOX0000", "BOX0001", "HMG0003", "SIB0000"];
        let keys = labels.map(|label| {
            let keys = key(label);
            assert_eq!(keys.len(), 1, "{label}");
            keys[0]
        });
        assert!(keys.windows(2).all(|pair| pair[0] < pair[1]), "{keys:?}");
    }
}

#[test]
fn retained_note_anchor_ordinals_survive_merged_note_source_decoding() {
    for count in [1, 45] {
        for merged in [false, true] {
            let boxes = (0..3).map(|i| format!(r##"<w:r><w:pict><v:shape xmlns:v="urn:schemas-microsoft-com:vml" id="note-box{i}" type="#_x0000_t202" style="position:absolute;margin-left:45pt;margin-top:0pt;width:70pt;height:10pt;z-index:{}"><v:textbox><w:txbxContent><w:p><w:r><w:rPr><w:sz w:val="12"/></w:rPr><w:t>BOX000{i}</w:t></w:r></w:p></w:txbxContent></v:textbox></v:shape></w:pict></w:r>"##, 300-i*100)).collect::<String>();
            let note = format!(
                r#"<w:footnotes xmlns:w="{}"><w:footnote w:id="1"><w:tbl><w:tblGrid><w:gridCol w:w="4200"/></w:tblGrid><w:tr><w:tc><w:p><w:r><w:t>OWNER0000</w:t></w:r>{boxes}</w:p>{}</w:tc></w:tr></w:tbl><w:tbl><w:tblGrid><w:gridCol w:w="2000"/><w:gridCol w:w="2000"/></w:tblGrid><w:tr><w:tc><w:tcPr><w:vMerge w:val="restart"/></w:tcPr>{}</w:tc><w:tc>{}</w:tc></w:tr><w:tr><w:tc><w:tcPr><w:vMerge/></w:tcPr>{}</w:tc><w:tc>{}</w:tc></w:tr></w:tbl></w:footnote></w:footnotes>"#,
                crate::office::docx::testutil::W,
                para("TAIL0000"),
                (0..count)
                    .map(|i| para(&format!("MERGED{i:04}")))
                    .collect::<String>(),
                para("SIB0000"),
                para(""),
                para("SIB0001")
            );
            let note = if merged {
                note
            } else {
                note.replace(r#"<w:vMerge w:val="restart"/>"#, "")
                    .replace("<w:vMerge/>", "")
            };
            let bytes = crate::office::docx::testutil::with_related_part(
                docx(
                    r#"<w:p><w:r><w:t>BODY</w:t><w:footnoteReference w:id="1"/></w:r></w:p><w:sectPr><w:pgSz w:w="7200" w:h="6000"/><w:pgMar w:top="400" w:bottom="400" w:left="400" w:right="400"/></w:sectPr>"#,
                    None,
                    None,
                ),
                "footnotes",
                "footnotes.xml",
                &note,
                false,
            );
            let out = pages(bytes);
            let text = out
                .iter()
                .map(|page| {
                    let mut runs = page
                        .items
                        .iter()
                        .filter_map(|item| match item {
                            crate::office::pptx::scene::Item::Text(run) => Some(run),
                            _ => None,
                        })
                        .collect::<Vec<_>>();
                    runs.sort_by_key(|run| run.paragraph);
                    runs.iter().map(|run| run.text.as_str()).collect::<String>()
                })
                .collect::<String>();
            let labels = [
                "OWNER0000",
                "BOX0000",
                "BOX0001",
                "BOX0002",
                "TAIL0000",
                "MERGED0000",
            ];
            let positions = labels.map(|label| {
                assert_eq!(text.matches(label).count(), 1, "{label}");
                text.find(label).unwrap()
            });
            assert!(positions.windows(2).all(|pair| pair[0] < pair[1]), "{text}");
        }
    }
}

#[test]
fn whole_note_anchor_preserves_paragraph_and_line_origins() {
    let position = |late: bool, relative: &str, count: usize| {
        let anchor = format!(
            r##"<w:r><w:pict><v:shape xmlns:v="urn:schemas-microsoft-com:vml" id="origin-box" type="#_x0000_t202" style="position:absolute;margin-left:150pt;margin-top:0pt;width:60pt;height:10pt;mso-position-vertical-relative:{relative}"><v:textbox><w:txbxContent><w:p><w:r><w:rPr><w:sz w:val="12"/></w:rPr><w:t>ORIGINBOX</w:t></w:r></w:p></w:txbxContent></v:textbox></v:shape></w:pict></w:r>"##
        );
        let prefix = "<w:r><w:t>FIRST</w:t><w:br/><w:t>SECOND</w:t><w:br/></w:r>";
        let content = if late {
            format!("{prefix}{anchor}")
        } else {
            format!("{anchor}{prefix}")
        };
        let tail = (0..count)
            .map(|i| para(&format!("ORIGTAIL{i:04}")))
            .collect::<String>();
        let note = format!(
            r#"<w:footnotes xmlns:w="{}"><w:footnote w:id="1"><w:p><w:pPr><w:spacing w:before="80" w:after="0" w:line="240" w:lineRule="exact"/></w:pPr>{content}<w:r><w:t>LAST</w:t></w:r></w:p>{tail}</w:footnote></w:footnotes>"#,
            crate::office::docx::testutil::W
        );
        let bytes = crate::office::docx::testutil::with_related_part(
            docx(
                r#"<w:p><w:r><w:t>BODY</w:t><w:footnoteReference w:id="1"/></w:r></w:p><w:sectPr><w:pgSz w:w="7200" w:h="6000"/><w:pgMar w:top="400" w:bottom="400" w:left="400" w:right="400"/></w:sectPr>"#,
                None,
                None,
            ),
            "footnotes",
            "footnotes.xml",
            &note,
            false,
        );
        let out = pages(bytes);
        assert_eq!(out.len() > 1, count > 0);
        let owner = out[0]
            .items
            .iter()
            .find_map(|item| match item {
                crate::office::pptx::scene::Item::Text(run) if run.text == "FIRST" => {
                    Some(run.transform.ty)
                }
                _ => None,
            })
            .unwrap();
        out[0]
            .items
            .iter()
            .find_map(|item| match item {
                crate::office::pptx::scene::Item::Text(run) if run.text == "ORIGINBOX" => {
                    Some(run.transform.ty)
                }
                _ => None,
            })
            .expect("retained textbox")
            - owner
    };
    let first = position(false, "text", 0);
    let late = position(true, "text", 0);
    assert!(
        (first - late).abs() < 0.01,
        "paragraph origins {first} {late}"
    );
    assert!((position(false, "text", 45) - first).abs() < 0.01);
    assert!((position(true, "text", 45) - late).abs() < 0.01);
    // Four points before the paragraph and two twelve-point source lines separate the origins.
    let line = position(true, "line", 0);
    assert!(
        (line - late - 28.0).abs() < 0.01,
        "line origin {line}, paragraph {late}"
    );
}

#[test]
fn direct_note_picture_wrap_reserves_the_same_content_in_whole_and_fragmented_notes() {
    use std::io::{Read, Write};
    for count in [0, 45] {
        for (wrap, picture_height) in [
            ("TopAndBottom", 254000),
            ("None", 254000),
            ("Square", 1016000),
            ("Square", 5080000),
        ] {
            let bytes =
                wrapped_picture_package(0, "paragraph", 0, picture_height, wrap, false, false);
            let mut input = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
            let mut output = std::io::Cursor::new(Vec::new());
            {
                let mut z = zip::ZipWriter::new(&mut output);
                let mut note = String::new();
                let mut note_rels = Vec::new();
                for i in 0..input.len() {
                    let mut f = input.by_index(i).unwrap();
                    let name = f.name().unwrap().to_string();
                    let mut data = Vec::new();
                    f.read_to_end(&mut data).unwrap();
                    if name == "word/document.xml" {
                        let xml = String::from_utf8(data).unwrap();
                        let xml = if wrap == "Square" {
                            xml.replace("2540000", "254000")
                        } else {
                            xml
                        };
                        let begin = xml.find("<w:body>").unwrap() + 8;
                        let end = xml.find("<w:sectPr>").unwrap();
                        note = format!(
                            r#"<w:footnotes xmlns:w="{}" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><w:footnote w:id="1">{}{}</w:footnote></w:footnotes>"#,
                            crate::office::docx::testutil::W,
                            &xml[begin..end],
                            (0..count)
                                .map(|i| para(&format!("NOTE{i:04}")))
                                .collect::<String>()
                        );
                        data = format!("{}<w:p><w:r><w:t>BODY</w:t><w:footnoteReference w:id=\"1\"/></w:r></w:p>{}", &xml[..begin], &xml[end..]).into_bytes();
                        if picture_height == 5080000 {
                            note = note.replace("</w:footnotes>", "<w:footnote w:id=\"2\"><w:p><w:r><w:t>FOLLOWINGNOTE</w:t></w:r></w:p></w:footnote></w:footnotes>");
                            data = String::from_utf8(data).unwrap().replace("<w:footnoteReference w:id=\"1\"/>", "<w:footnoteReference w:id=\"1\"/><w:footnoteReference w:id=\"2\"/>").into_bytes();
                        }
                    } else if name == "word/_rels/document.xml.rels" {
                        note_rels = data.clone();
                        data = String::from_utf8(data).unwrap().replace("</Relationships>", r#"<Relationship Id="rNotes" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/footnotes" Target="footnotes.xml"/></Relationships>"#).into_bytes();
                    }
                    z.start_file(name, zip::write::SimpleFileOptions::default())
                        .unwrap();
                    z.write_all(&data).unwrap();
                }
                for (name, data) in [
                    ("word/footnotes.xml", note.as_bytes()),
                    ("word/_rels/footnotes.xml.rels", note_rels.as_slice()),
                ] {
                    z.start_file(name, zip::write::SimpleFileOptions::default())
                        .unwrap();
                    z.write_all(data).unwrap();
                }
                z.finish().unwrap();
            }
            let out = pages(output.into_inner());
            if picture_height < 5080000 {
                assert_eq!(out.len() > 1, count > 0);
            }
            assert!(out.len() < 20);
            let image = out
                .iter()
                .flat_map(|page| &page.items)
                .filter_map(|item| match item {
                    crate::office::pptx::scene::Item::Image(image) => Some(image),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(image.len(), 1);
            let owner = out
                .iter()
                .flat_map(|page| &page.items)
                .find_map(|item| match item {
                    crate::office::pptx::scene::Item::Text(run) if run.text == "ANCHOR" => {
                        Some(run)
                    }
                    _ => None,
                })
                .unwrap();
            let bottom = image[0].transform.ty + image[0].transform.sy;
            if wrap == "TopAndBottom" {
                assert!(
                    bottom <= owner.transform.ty + 0.01,
                    "count={count}: image bottom {bottom}, owner {}",
                    owner.transform.ty
                );
            } else if picture_height == 5080000 {
                assert!(
                    owner.transform.ty >= 36.0,
                    "oversized source owner {}",
                    owner.transform.ty
                );
                let owner_page = out.iter().position(|page| page.items.iter().any(|item| matches!(item, crate::office::pptx::scene::Item::Text(run) if run.text == "ANCHOR"))).unwrap();
                let following_pages = out.iter().enumerate().filter_map(|(index, page)| page.items.iter().any(|item| matches!(item, crate::office::pptx::scene::Item::Text(run) if run.text == "FOLLOWINGNOTE")).then_some(index)).collect::<Vec<_>>();
                assert_eq!(following_pages.len(), 1);
                assert!(following_pages[0] > owner_page);
                assert!(
                    out[owner_page]
                        .info
                        .note_diagnostics
                        .iter()
                        .any(|message| message.contains("oversized footnote object"))
                );
            } else if wrap == "Square" {
                assert!(bottom <= 264.01, "count={count}: occupied bottom {bottom}");
            } else {
                assert!(
                    bottom > owner.transform.ty,
                    "overlay remains source-directed"
                );
            }
            for i in 0..count {
                let label = format!("NOTE{i:04}");
                assert_eq!(out.iter().flat_map(|page| &page.items).filter(|item| matches!(item, crate::office::pptx::scene::Item::Text(run) if run.text == label)).count(), 1);
            }
        }
    }
}

#[test]
fn selected_continuation_stories_follow_a_forced_physical_note_boundary() {
    use crate::office::docx::testutil::{W, with_related_part};
    for (selected, incoming, outgoing) in [
        (true, para("INCOMING_SEPARATOR"), para("OUTGOING_NOTICE")),
        (true, String::new(), para("OUTGOING_NOTICE")),
        (true, para("INCOMING_SEPARATOR"), String::new()),
        (
            true,
            "<w:p><w:pPr><w:spacing w:after=\"400\"/></w:pPr></w:p>".into(),
            para("OUTGOING_NOTICE"),
        ),
        (false, para("INCOMING_SEPARATOR"), para("OUTGOING_NOTICE")),
        (true, "<w:p><w:pPr><w:spacing w:before=\"6000\"/></w:pPr><w:r><w:t>INCOMING_SEPARATOR</w:t></w:r></w:p>".into(), para("OUTGOING_NOTICE")),
        (true, para("INCOMING_SEPARATOR"), "<w:p><w:pPr><w:spacing w:before=\"6000\"/></w:pPr><w:r><w:t>OUTGOING_NOTICE</w:t></w:r></w:p>".into()),
    ] {
        let body = r#"<w:p><w:r><w:t>BODY</w:t><w:footnoteReference w:id="7"/></w:r></w:p><w:sectPr><w:pgSz w:w="6000" w:h="6000"/><w:pgMar w:top="600" w:bottom="600" w:left="600" w:right="600"/></w:sectPr>"#;
        let notes = format!(
            r#"<w:footnotes xmlns:w="{W}"><w:footnote w:id="-1" w:type="separator">{}</w:footnote><w:footnote w:id="3" w:type="continuationSeparator">{}</w:footnote><w:footnote w:id="4" w:type="continuationNotice">{}</w:footnote><w:footnote w:id="7"><w:p><w:r><w:t>NOTE_FIRST</w:t><w:br w:type="page"/></w:r></w:p>{}</w:footnote></w:footnotes>"#,
            para("ORDINARY_SEPARATOR"),
            incoming,
            outgoing,
            para("NOTE_LAST"),
        );
        let settings = format!(
            r#"<w:settings xmlns:w="{W}"><w:footnotePr><w:footnote w:id="-1"/><w:footnote w:id="3"/><w:footnote w:id="4"/></w:footnotePr></w:settings>"#
        );
        let bytes = with_related_part(
            docx(body, None, None),
            "footnotes",
            "footnotes.xml",
            &notes,
            false,
        );
        let bytes = if selected {
            with_related_part(bytes, "settings", "settings.xml", &settings, false)
        } else {
            bytes
        };
        let out = pages(bytes);
        for page in &out {
            for item in &page.items {
                if let crate::office::pptx::scene::Item::Text(run) = item
                    && run.text.starts_with("NOTE_") {
                        assert!(run.transform.ty as f64 >= page.info.top, "source note above fresh area: {}", run.transform.ty);
                        assert!(run.transform.ty as f64 <= page.info.bottom);
                    }
            }
        }
        let locations = |label: &str| {
            out.iter()
                .enumerate()
                .flat_map(|(page, out)| {
                    out.items.iter().filter_map(move |item| match item {
                        crate::office::pptx::scene::Item::Text(run) if run.text == label => Some(page),
                        _ => None,
                    })
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(locations("NOTE_FIRST"), [0]);
        assert_eq!(locations("NOTE_LAST"), [1]);
        assert_eq!(
            locations("OUTGOING_NOTICE"),
            if selected && !outgoing.is_empty() {
                vec![0]
            } else {
                vec![]
            }
        );
        assert_eq!(
            locations("INCOMING_SEPARATOR"),
            if selected && incoming.contains("INCOMING_SEPARATOR") {
                vec![1]
            } else {
                vec![]
            }
        );
        assert_eq!(
            locations("ORDINARY_SEPARATOR"),
            if selected { vec![0] } else { vec![0, 1] }
        );
    }
}

#[test]
fn continuation_rule_uses_the_accepted_note_container_width() {
    use crate::office::docx::testutil::{W, with_related_part};
    for (columns, width) in [
        ("<w:cols w:num=\"1\"/>", 240.0),
        ("<w:cols w:num=\"2\" w:space=\"400\"/>", 110.0),
    ] {
        let body = format!(
            r#"<w:p><w:r><w:t>BODY</w:t><w:footnoteReference w:id="7"/></w:r></w:p><w:sectPr><w:pgSz w:w="6000" w:h="6000"/><w:pgMar w:top="600" w:bottom="600" w:left="600" w:right="600"/>{columns}</w:sectPr>"#
        );
        let notes = format!(
            r#"<w:footnotes xmlns:w="{W}"><w:footnote w:id="3" w:type="continuationSeparator"><w:p><w:r><w:continuationSeparator/></w:r></w:p></w:footnote><w:footnote w:id="7"><w:p><w:r><w:t>FIRST</w:t><w:br w:type="page"/></w:r></w:p>{}</w:footnote></w:footnotes>"#,
            para("LAST")
        );
        let settings = format!(
            r#"<w:settings xmlns:w="{W}"><w:footnotePr><w:footnote w:id="3"/></w:footnotePr></w:settings>"#
        );
        let bytes = with_related_part(
            docx(&body, None, None),
            "footnotes",
            "footnotes.xml",
            &notes,
            false,
        );
        let bytes = with_related_part(bytes, "settings", "settings.xml", &settings, false);
        let out = pages(bytes);
        assert_eq!(out.len(), 2);
        let rules = out[1]
            .items
            .iter()
            .filter_map(|item| match item {
                crate::office::pptx::scene::Item::Path(path) => Some(path.path.bounds()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(
            rules
                .iter()
                .any(|rect| (rect.width() as f64 - width).abs() < 0.01
                    && (rect.height() - 0.75).abs() < 0.01),
            "width={width}, rules={rules:?}"
        );
    }
}

#[test]
fn oversized_visible_continuation_notice_carries_without_recursive_story_copies() {
    use crate::office::docx::testutil::{W, with_related_part};
    let line = |label: String| {
        format!(
            r#"<w:p><w:pPr><w:spacing w:before="0" w:after="0" w:line="240" w:lineRule="exact"/></w:pPr><w:r><w:t>{label}</w:t></w:r></w:p>"#
        )
    };
    let body = r#"<w:p><w:r><w:t>BODY</w:t><w:footnoteReference w:id="7"/></w:r></w:p><w:sectPr><w:pgSz w:w="6000" w:h="6000"/><w:pgMar w:top="600" w:bottom="600" w:left="600" w:right="600"/></w:sectPr>"#;
    let note = (0..55)
        .map(|i| line(format!("NOTE{i:03}")))
        .collect::<String>();
    let notice = (0..26)
        .map(|i| line(format!("STORY{i:03}")))
        .collect::<String>();
    let source = format!(
        r#"<w:footnotes xmlns:w="{W}"><w:footnote w:type="continuationNotice" w:id="4">{notice}</w:footnote><w:footnote w:id="7">{note}</w:footnote></w:footnotes>"#
    );
    let settings = format!(
        r#"<w:settings xmlns:w="{W}"><w:footnotePr><w:footnote w:id="4"/></w:footnotePr></w:settings>"#
    );
    let bytes = with_related_part(
        docx(body, None, None),
        "footnotes",
        "footnotes.xml",
        &source,
        false,
    );
    let out = pages(with_related_part(
        bytes,
        "settings",
        "settings.xml",
        &settings,
        false,
    ));
    assert!(out.len() > 3 && out.len() < 12, "pages={}", out.len());
    let mut normal = Vec::new();
    let mut stories = Vec::new();
    for page in &out {
        for item in &page.items {
            if let crate::office::pptx::scene::Item::Text(run) = item
                && (run.text.starts_with("NOTE") || run.text.starts_with("STORY"))
            {
                assert!(run.transform.ty as f64 >= page.info.top);
                assert!(run.transform.ty as f64 <= page.info.bottom);
                if run.text.starts_with("NOTE") {
                    normal.push(run.text.clone());
                } else {
                    stories.push(run.text.clone());
                }
            }
        }
    }
    assert_eq!(
        normal,
        (0..55).map(|i| format!("NOTE{i:03}")).collect::<Vec<_>>()
    );
    assert_eq!(stories.len(), 52);
    for occurrence in stories.chunks(26) {
        assert_eq!(
            occurrence,
            (0..26).map(|i| format!("STORY{i:03}")).collect::<Vec<_>>()
        );
    }
}

#[test]
fn near_capacity_continuation_notice_preserves_normal_prefix_space() {
    use crate::office::docx::testutil::{W, with_related_part};
    let line = |label: String| {
        format!(
            r#"<w:p><w:pPr><w:spacing w:before="0" w:after="0" w:line="240" w:lineRule="exact"/></w:pPr><w:r><w:t>{label}</w:t></w:r></w:p>"#
        )
    };
    let body = r#"<w:p><w:r><w:t>BODY</w:t><w:footnoteReference w:id="7"/></w:r></w:p><w:sectPr><w:pgSz w:w="6000" w:h="6000"/><w:pgMar w:top="600" w:bottom="600" w:left="600" w:right="600"/></w:sectPr>"#;
    let note = (0..55)
        .map(|i| line(format!("NOTE{i:03}")).replace("w:line=\"240\"", "w:line=\"480\""))
        .collect::<String>();
    let notice = (0..18)
        .map(|i| line(format!("STORY{i:03}")))
        .collect::<String>();
    let source = format!(
        r#"<w:footnotes xmlns:w="{W}"><w:footnote w:type="separator" w:id="-1"><w:p><w:pPr><w:spacing w:before="0" w:after="0" w:line="240" w:lineRule="exact"/></w:pPr><w:r><w:separator/></w:r></w:p></w:footnote><w:footnote w:type="continuationNotice" w:id="4">{notice}</w:footnote><w:footnote w:id="7">{note}</w:footnote></w:footnotes>"#
    );
    let settings = format!(
        r#"<w:settings xmlns:w="{W}"><w:footnotePr><w:footnote w:id="4"/></w:footnotePr></w:settings>"#
    );
    let bytes = with_related_part(
        docx(body, None, None),
        "footnotes",
        "footnotes.xml",
        &source,
        false,
    );
    let out = pages(with_related_part(
        bytes,
        "settings",
        "settings.xml",
        &settings,
        false,
    ));
    assert!(out.len() > 3 && out.len() < 30, "pages={}", out.len());
    let mut normal = Vec::new();
    let mut stories = Vec::new();
    for page in &out {
        for item in &page.items {
            if let crate::office::pptx::scene::Item::Text(run) = item
                && (run.text.starts_with("NOTE") || run.text.starts_with("STORY"))
            {
                assert!(run.transform.ty as f64 >= page.info.top);
                assert!(run.transform.ty as f64 <= page.info.bottom);
                if run.text.starts_with("NOTE") {
                    normal.push(run.text.clone());
                } else {
                    stories.push(run.text.clone());
                }
            }
        }
    }
    assert_eq!(
        normal,
        (0..55).map(|i| format!("NOTE{i:03}")).collect::<Vec<_>>()
    );
    assert!(!stories.is_empty() && stories.len() % 18 == 0);
    for occurrence in stories.chunks(18) {
        assert_eq!(
            occurrence,
            (0..18).map(|i| format!("STORY{i:03}")).collect::<Vec<_>>()
        );
    }
}

fn row_under_tall_header(line_height: usize, count: usize, nested: bool) -> Vec<super::OutPage> {
    row_under_tall_header_borders(line_height, count, nested, false)
}

fn row_under_tall_header_borders(
    line_height: usize,
    count: usize,
    nested: bool,
    borders: bool,
) -> Vec<super::OutPage> {
    let content: String = (0..count).map(|i| format!(r#"<w:p><w:pPr><w:spacing w:before="0" w:after="0" w:line="{}" w:lineRule="exact"/><w:widowControl w:val="0"/></w:pPr><w:r><w:rPr><w:sz w:val="20"/></w:rPr><w:t>ROW{i:04}</w:t></w:r></w:p>"#, line_height * 20)).collect();
    let content = if nested {
        format!(
            r#"<w:tbl><w:tblPr><w:tblCellMar><w:top w:w="0" w:type="dxa"/><w:bottom w:w="0" w:type="dxa"/></w:tblCellMar></w:tblPr><w:tblGrid><w:gridCol w:w="9000"/></w:tblGrid><w:tr><w:tc>{content}</w:tc></w:tr></w:tbl>{}"#,
            if count == 1 { "" } else { "<w:p/>" }
        )
    } else {
        content
    };
    let content = if borders {
        content.replace("<w:tblPr>", r#"<w:tblPr><w:tblBorders><w:top w:val="single" w:sz="8"/><w:bottom w:val="single" w:sz="8"/></w:tblBorders>"#)
    } else {
        content
    };
    let body = format!(
        r#"<w:tbl><w:tblPr><w:tblCellMar><w:top w:w="0" w:type="dxa"/><w:bottom w:w="0" w:type="dxa"/></w:tblCellMar></w:tblPr><w:tblGrid><w:gridCol w:w="9000"/></w:tblGrid><w:tr><w:tc><w:tcPr><w:tcW w:w="9000" w:type="dxa"/></w:tcPr>{content}</w:tc></w:tr></w:tbl><w:sectPr><w:headerReference w:type="default" r:id="rheader"/><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:bottom="1440" w:left="1440" w:right="1440" w:header="720"/></w:sectPr>"#
    );
    let header = format!(
        r#"<w:hdr xmlns:w="{}"><w:p><w:pPr><w:spacing w:line="13800" w:lineRule="exact"/></w:pPr><w:r><w:t>HEADER</w:t></w:r></w:p></w:hdr>"#,
        crate::office::docx::testutil::W
    );
    pages(crate::office::docx::testutil::with_related_part(
        docx(&body, None, None),
        "header",
        "header.xml",
        &header,
        false,
    ))
}

fn assert_row_markers_visible(out: &[super::OutPage], count: usize) {
    let mut found = Vec::new();
    for page in out {
        for item in &page.items {
            if let crate::office::pptx::scene::Item::Text(run) = item
                && run.text.starts_with("ROW")
            {
                let b = crate::office::pptx::textlayout::transformed_bbox(
                    &run.transform,
                    0.0,
                    -run.ascent,
                    run.width,
                    run.ascent + run.descent,
                )
                .unwrap();
                assert!(
                    b.y < page.info.h && b.y + b.h > 0.0,
                    "source row text outside physical page"
                );
                found.push(run.text.clone());
            }
        }
    }
    assert_eq!(
        found,
        (0..count).map(|i| format!("ROW{i:04}")).collect::<Vec<_>>()
    );
}

#[test]
fn fresh_row_oversized_first_line_preserves_continuation() {
    let out = row_under_tall_header(14, 40, false);
    assert_row_markers_visible(&out, 40);
    assert_eq!(out.len(), 40);
}

#[test]
fn successful_row_splits_do_not_exhaust_stalled_progress_guard() {
    for count in [40, 250] {
        let out = row_under_tall_header(10, count, false);
        assert_row_markers_visible(&out, count);
        assert_eq!(out.len(), count);
    }
}

#[test]
fn nested_row_atomic_progress_keeps_source_tail() {
    for (height, count) in [(14, 40), (10, 250)] {
        let out = row_under_tall_header(height, count, true);
        assert_row_markers_visible(&out, count);
        assert!(out.len() >= count && out.len() <= count + 2);
    }
}

#[test]
fn nested_row_atomic_last_line_has_no_empty_continuation() {
    let out = row_under_tall_header(14, 1, true);
    assert_row_markers_visible(&out, 1);
    assert_eq!(out.len(), 1);
}

#[test]
fn nested_atomic_row_keeps_natural_border_on_its_only_page() {
    let out = row_under_tall_header_borders(14, 1, true, true);
    assert_row_markers_visible(&out, 1);
    assert_eq!(out.len(), 1);
}

mod charts;

#[test]
fn oversized_footer_keeps_complete_source_runs_before_physical_page_export() {
    let footer_text: String = (0..100)
        .map(|i| format!(r#"<w:p><w:pPr><w:spacing w:after="0" w:line="240" w:lineRule="exact"/></w:pPr><w:r><w:t>FOOTER{i:03}</w:t></w:r></w:p>"#))
        .collect();
    let footer = format!(
        r#"<w:ftr xmlns:w="{}"><w:p/><w:tbl><w:tblGrid><w:gridCol w:w="6000"/></w:tblGrid><w:tr><w:tc>{footer_text}</w:tc></w:tr></w:tbl><w:p/></w:ftr>"#,
        crate::office::docx::testutil::W
    );
    let body = r#"<w:p><w:r><w:t>BODY</w:t></w:r></w:p><w:sectPr><w:footerReference w:type="default" r:id="rfooter"/><w:pgSz w:w="16838" w:h="11906"/><w:pgMar w:top="1440" w:bottom="1440" w:left="1440" w:right="1440" w:header="142" w:footer="142"/></w:sectPr>"#;
    let out = pages(crate::office::docx::testutil::with_related_part(
        docx(body, None, None),
        "footer",
        "footer.xml",
        &footer,
        false,
    ));
    assert_eq!(out.len(), 1);
    let page = &out[0];
    let runs: Vec<_> = page
        .items
        .iter()
        .filter_map(|item| match item {
            crate::office::pptx::scene::Item::Text(run)
                if run.paragraph == crate::office::docx::text::FOOTER_KEY =>
            {
                Some(run)
            }
            _ => None,
        })
        .collect();
    let actual: String = runs.iter().map(|run| run.text.as_str()).collect();
    let expected: String = (0..100).map(|i| format!("FOOTER{i:03}")).collect();
    assert_eq!(actual, expected);
    let mut offpage = 0;
    let mut visible = 0;
    for run in runs {
        assert_eq!(run.source.as_ref().unwrap().part, "word/footer.xml");
        let bounds = crate::office::pptx::textlayout::transformed_bbox(
            &run.transform,
            0.0,
            -run.ascent,
            run.width,
            run.ascent + run.descent,
        )
        .unwrap();
        if bounds.y + bounds.h <= 0.0 {
            offpage += 1;
        }
        if bounds.y < page.info.h && bounds.y + bounds.h > 0.0 {
            visible += 1;
        }
    }
    assert!(
        offpage > 0,
        "the oversized footer starts above the physical page"
    );
    assert!(visible > 0, "the trailing footer remains on the page");
    assert!(page.items.iter().any(
        |item| matches!(item, crate::office::pptx::scene::Item::Text(run) if run.text == "BODY")
    ));
}

#[test]
fn numeric_floating_table_anchors_use_actual_page_and_column_origins() {
    let render_x = |anchor: &str, left: u32, second_column: bool| {
        let advance = if second_column {
            r#"<w:p><w:r><w:br w:type="column"/></w:r></w:p>"#
        } else {
            ""
        };
        let body = format!(
            r#"{advance}<w:tbl><w:tblPr><w:tblpPr w:horzAnchor="{anchor}" w:vertAnchor="page" w:tblpX="800" w:tblpY="2000"/><w:tblW w:w="1000" w:type="dxa"/><w:tblLayout w:type="fixed"/><w:tblCellMar><w:left w:w="0" w:type="dxa"/></w:tblCellMar></w:tblPr><w:tblGrid><w:gridCol w:w="1000"/></w:tblGrid><w:tr><w:trPr><w:tblHeader/></w:trPr><w:tc><w:p><w:r><w:t>anchor</w:t></w:r></w:p></w:tc></w:tr><w:tr><w:tc><w:p><w:r><w:t>anchor</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:sectPr><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:bottom="1440" w:left="{left}" w:right="1440"/><w:cols w:num="2" w:space="720"/></w:sectPr>"#
        );
        let out = pages(docx(&body, None, None));
        let positions: Vec<_> = out
            .iter()
            .flat_map(|page| &page.items)
            .filter_map(|item| match item {
                crate::office::pptx::scene::Item::Text(run) if run.text == "anchor" => {
                    Some(run.transform.tx as f64)
                }
                _ => None,
            })
            .collect();
        assert_eq!(positions.len(), 2);
        assert!((positions[0] - positions[1]).abs() < 0.1);
        positions[0]
    };
    for left in [720, 1800] {
        for second in [false, true] {
            assert!((render_x("page", left, second) - 40.0).abs() < 0.1);
            assert!((render_x("margin", left, second) - (left as f64 / 20.0 + 40.0)).abs() < 0.1);
        }
        let column_delta = render_x("text", left, true) - render_x("text", left, false);
        let expected = (612.0 - left as f64 / 20.0 - 72.0 + 36.0) / 2.0;
        assert!((column_delta - expected).abs() < 0.1);
    }
}

#[test]
fn text_anchored_tables_ignore_relative_vertical_alignment() {
    let render = |anchor: &str, alignment: &str, offset: &str| {
        let body = format!(
            r#"<w:p><w:pPr><w:spacing w:after="0" w:line="1600" w:lineRule="exact"/></w:pPr><w:r><w:t>lead</w:t></w:r></w:p><w:tbl><w:tblPr><w:tblpPr w:horzAnchor="margin" {anchor} {alignment} {offset}/><w:tblW w:w="8000" w:type="dxa"/><w:tblLayout w:type="fixed"/></w:tblPr><w:tblGrid><w:gridCol w:w="8000"/></w:tblGrid><w:tr><w:trPr><w:trHeight w:val="600" w:hRule="exact"/></w:trPr><w:tc><w:p><w:r><w:t>field</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:p><w:r><w:t>tail</w:t></w:r></w:p><w:sectPr><w:pgSz w:w="12240" w:h="8000"/><w:pgMar w:top="720" w:bottom="720" w:left="1440" w:right="1440"/></w:sectPr>"#
        );
        let out = pages(docx(&body, None, None));
        let positions: Vec<_> = out
            .iter()
            .enumerate()
            .flat_map(|(page, scene)| {
                scene.items.iter().filter_map(move |item| match item {
                    crate::office::pptx::scene::Item::Text(run) => {
                        Some((page, run.text.clone(), run.transform.ty))
                    }
                    _ => None,
                })
            })
            .collect();
        assert_eq!(positions.len(), 3, "all source text occurs once");
        assert_eq!(
            positions.iter().map(|p| p.1.as_str()).collect::<Vec<_>>(),
            ["lead", "field", "tail"]
        );
        (out.len(), positions)
    };
    for offset in ["", r#"w:tblpY="200""#, r#"w:tblpY="-100""#] {
        let expected = render(r#"w:vertAnchor="text""#, "", offset);
        assert_eq!(expected.0, 1);
        for alignment in ["top", "center", "bottom", "inside", "outside"] {
            assert_eq!(
                render(
                    r#"w:vertAnchor="text""#,
                    &format!(r#"w:tblpYSpec="{alignment}""#),
                    offset
                ),
                expected,
                "text anchor with {alignment} and {offset}"
            );
        }
    }
    for anchor in ["", r#"w:vertAnchor="margin""#, r#"w:vertAnchor="page""#] {
        let aligned = render(anchor, r#"w:tblpYSpec="center""#, "");
        assert_eq!(
            aligned,
            render(anchor, r#"w:tblpYSpec="center""#, r#"w:tblpY="200""#)
        );
        assert_ne!(aligned, render(anchor, "", r#"w:tblpY="200""#));
    }
}

#[test]
fn section_start_spacing_loses_the_previous_space_after() {
    // Word, modes 11 and 15: after a next-page section break that ends in a paragraph
    // with 20 pt after, 48 pt before shrinks to 28 pt and 8 pt before to nothing.
    for mode in [11, 15] {
        let at = |before: &str| {
            auto_before_boundary_target_mode(
                "sectionAfter",
                &format!(r#"w:before="{before}" w:beforeAutospacing="0""#),
                false,
                mode,
            )
        };
        let (zero, wide, narrow) = (at("0"), at("960"), at("160"));
        assert_eq!((zero.0, wide.0, narrow.0), (1, 1, 1));
        assert!(
            (wide.1 - zero.1 - 28.0).abs() < 0.01,
            "{mode}: {wide:?} {zero:?}"
        );
        assert!(
            (narrow.1 - zero.1).abs() < 0.01,
            "{mode}: {narrow:?} {zero:?}"
        );
    }
}

#[test]
fn a_column_break_ending_a_paragraph_leaves_its_mark_line() {
    // Word, modes 11 and 15: the paragraph mark after a column break takes the first
    // line of the next column (here a page); after a page break it takes no line.
    let ends = |kind: &str| {
        let body = format!(
            r#"{}<w:p><w:r><w:t>Alpha</w:t></w:r><w:r><w:br w:type="{kind}"/></w:r></w:p>{}{}"#,
            para("Top"),
            para("Bravo"),
            para("Charlie")
        );
        let p = pages(docx(&body, None, None));
        let pitch = text_y(&p[1], "Charlie") - text_y(&p[1], "Bravo");
        (text_y(&p[1], "Bravo") - text_y(&p[0], "Top"), pitch)
    };
    let (column, pitch) = ends("column");
    assert!((column - pitch).abs() < 0.01, "{column} vs line {pitch}");
    let (page, _) = ends("page");
    assert!(page.abs() < 0.01, "{page}");
}

#[test]
fn a_page_positioned_header_frame_floats_without_growing_the_header() {
    let render = |framed: bool| {
        let body = r#"<w:p><w:r><w:t>body</w:t></w:r></w:p><w:sectPr><w:headerReference w:type="default" r:id="rheader"/><w:pgMar w:top="1440" w:bottom="1440" w:left="1440" w:right="1440" w:header="200"/></w:sectPr>"#;
        let frame = if framed {
            r#"<w:p><w:pPr><w:framePr w:wrap="notBeside" w:hAnchor="margin" w:vAnchor="page" w:xAlign="center" w:y="8000"/></w:pPr><w:r><w:rPr><w:sz w:val="120"/></w:rPr><w:t>LOGO</w:t></w:r></w:p>"#
        } else {
            ""
        };
        let xml = format!(
            r#"<w:hdr xmlns:w="{}">{frame}<w:p/></w:hdr>"#,
            crate::office::docx::testutil::W
        );
        pages(crate::office::docx::testutil::with_related_part(
            docx(body, None, None),
            "header",
            "header.xml",
            &xml,
            false,
        ))
    };
    let framed = render(true);
    // The frame sits 400 pt down the page; the body still starts at the top margin.
    assert!(
        text_y(&framed[0], "LOGO") > 400.0,
        "{}",
        text_y(&framed[0], "LOGO")
    );
    let delta = text_y(&framed[0], "body") - text_y(&render(false)[0], "body");
    assert!(delta.abs() < 0.01, "{delta}");
}

#[test]
fn an_object_whose_band_pushes_its_anchor_on_stays_where_it_was_placed() {
    // Two columns. The anchor paragraph near the bottom of column 1 places a text box
    // 300 pt above itself; its band pushes the anchor into column 2, and the box stays
    // in column 1 where it was first placed (Word renders of two-column fact sheets).
    let shape = r#"<w:r><w:drawing><wp:anchor xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" distT="0" distB="0" distL="0" distR="0" simplePos="0" relativeHeight="1" behindDoc="0" locked="0" layoutInCell="1" allowOverlap="1"><wp:simplePos x="0" y="0"/><wp:positionH relativeFrom="column"><wp:posOffset>0</wp:posOffset></wp:positionH><wp:positionV relativeFrom="paragraph"><wp:posOffset>-3810000</wp:posOffset></wp:positionV><wp:extent cx="1905000" cy="2540000"/><wp:wrapTopAndBottom/><wp:docPr id="1" name="s"/><a:graphic xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"><wps:wsp xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"><wps:spPr><a:prstGeom prst="rect"/></wps:spPr><wps:txbx><w:txbxContent><w:p><w:r><w:t>PHOTO</w:t></w:r></w:p></w:txbxContent></wps:txbx><wps:bodyPr/></wps:wsp></a:graphicData></a:graphic></wp:anchor></w:drawing></w:r>"#;
    let fill: String = (0..40).map(|i| para(&format!("line {i}"))).collect();
    let body = format!(
        r#"{fill}<w:p>{shape}<w:r><w:t>anchor</w:t></w:r></w:p><w:sectPr><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:bottom="1440" w:left="1440" w:right="1440"/><w:cols w:num="2" w:space="720"/></w:sectPr>"#
    );
    let p = pages(docx(&body, None, None));
    let x_of = |needle: &str| {
        p[0].items
            .iter()
            .find_map(|i| match i {
                crate::office::pptx::scene::Item::Text(r) if r.text.contains(needle) => {
                    Some(r.transform.tx)
                }
                _ => None,
            })
            .unwrap()
    };
    // Column 2 starts at 72 + 216 + 36 = 324 pt.
    assert!(x_of("anchor") > 300.0, "anchor at {}", x_of("anchor"));
    assert!(x_of("PHOTO") < 300.0, "box at {}", x_of("PHOTO"));
}

#[test]
fn text_runs_beside_a_floating_table_that_leaves_an_inch_free() {
    // A 281 pt table at the left margin of a 468 pt column leaves 187 pt: text sits
    // beside it, as in Word (job sheets with a bordered note beside a photo).
    let tbl = r#"<w:tbl><w:tblPr><w:tblpPr w:leftFromText="180" w:rightFromText="180" w:vertAnchor="text" w:horzAnchor="margin" w:tblpY="0"/><w:tblW w:w="5616" w:type="dxa"/><w:tblLayout w:type="fixed"/></w:tblPr><w:tblGrid><w:gridCol w:w="5616"/></w:tblGrid><w:tr><w:tc><w:tcPr><w:tcW w:w="5616" w:type="dxa"/></w:tcPr><w:p><w:r><w:t>float</w:t></w:r></w:p><w:p><w:r><w:t>more</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#;
    let body = format!("{tbl}{}", para("beside"));
    let p = pages(docx(&body, None, None));
    let x = p[0]
        .items
        .iter()
        .find_map(|i| match i {
            crate::office::pptx::scene::Item::Text(r) if r.text.contains("beside") => {
                Some(r.transform.tx)
            }
            _ => None,
        })
        .unwrap();
    assert!(x > 300.0, "text at {x}");
}

#[test]
fn a_one_row_floating_table_that_does_not_fit_floats_on_the_next_page() {
    // Word for Mac: filler leaves too little room for a 200 pt text-anchored table; the
    // table floats at the top of page 2 and the following text runs beside it there.
    let fill: String = (0..42).map(|i| para(&format!("Filler {i}"))).collect();
    let tbl = r#"<w:tbl><w:tblPr><w:tblpPr w:leftFromText="180" w:rightFromText="180" w:vertAnchor="text" w:horzAnchor="margin" w:tblpXSpec="left" w:tblpY="64"/><w:tblW w:w="4000" w:type="dxa"/><w:tblLayout w:type="fixed"/></w:tblPr><w:tblGrid><w:gridCol w:w="4000"/></w:tblGrid><w:tr><w:trPr><w:trHeight w:val="4000" w:hRule="exact"/></w:trPr><w:tc><w:tcPr><w:tcW w:w="4000" w:type="dxa"/></w:tcPr><w:p><w:r><w:t>TABLE</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#;
    let body = format!("{fill}{tbl}{}", para("After"));
    let p = pages(docx(&body, None, None));
    assert!(p.len() >= 2);
    let after = p[1]
        .items
        .iter()
        .find_map(|i| match i {
            crate::office::pptx::scene::Item::Text(r) if r.text.contains("After") => {
                Some((r.transform.tx, r.transform.ty))
            }
            _ => None,
        })
        .unwrap();
    assert!(text_y(&p[1], "TABLE") < 100.0);
    assert!(after.0 > 250.0 && after.1 < 100.0, "{after:?}");
}

#[test]
fn text_resumes_beside_a_floating_table_continued_on_the_next_page() {
    // Word for Mac: a six-row 200 pt-wide text-anchored table starting near the page
    // bottom splits; on page 2 the following text starts at the top beside its rows.
    let fill: String = (0..42).map(|i| para(&format!("Filler {i}"))).collect();
    let rows: String = (0..6)
        .map(|k| format!(r#"<w:tr><w:trPr><w:trHeight w:val="800" w:hRule="exact"/></w:trPr><w:tc><w:tcPr><w:tcW w:w="4000" w:type="dxa"/></w:tcPr><w:p><w:r><w:t>ROW{k}</w:t></w:r></w:p></w:tc></w:tr>"#))
        .collect();
    let tbl = format!(
        r#"<w:tbl><w:tblPr><w:tblpPr w:leftFromText="180" w:rightFromText="180" w:vertAnchor="text" w:horzAnchor="margin" w:tblpXSpec="left" w:tblpY="64"/><w:tblW w:w="4000" w:type="dxa"/><w:tblLayout w:type="fixed"/></w:tblPr><w:tblGrid><w:gridCol w:w="4000"/></w:tblGrid>{rows}</w:tbl>"#
    );
    let body = format!("{fill}{tbl}{}", para("After"));
    let p = pages(docx(&body, None, None));
    let after = p[1]
        .items
        .iter()
        .find_map(|i| match i {
            crate::office::pptx::scene::Item::Text(r) if r.text.contains("After") => {
                Some((r.transform.tx, r.transform.ty))
            }
            _ => None,
        })
        .unwrap();
    let rows_on = |k: usize| {
        p[k].items.iter().any(
            |i| matches!(i, crate::office::pptx::scene::Item::Text(r) if r.text.starts_with("ROW")),
        )
    };
    assert!(
        rows_on(0) && rows_on(1),
        "the table splits across the pages"
    );
    assert!(after.0 > 250.0 && after.1 < 100.0, "{after:?}");
}

#[test]
fn a_header_styleref_shows_each_pages_heading() {
    // Headings "Alpha" (page 1) and "Beta" (page 2); page 3 has none and keeps Beta.
    // The header's saved result is "Saved".
    let styles = r#"<w:style w:type="paragraph" w:styleId="H1"><w:name w:val="heading 1,Chapter"/></w:style>"#;
    let heading = |t: &str| {
        format!(r#"<w:p><w:pPr><w:pStyle w:val="H1"/></w:pPr><w:r><w:t>{t}</w:t></w:r></w:p>"#)
    };
    let page_break = r#"<w:p><w:r><w:br w:type="page"/></w:r></w:p>"#;
    let body = format!(
        r#"{}{page_break}{}{page_break}{}<w:sectPr><w:headerReference w:type="default" r:id="rheader"/></w:sectPr>"#,
        heading("Alpha"),
        heading("Beta"),
        para("plain")
    );
    let header = r#"<w:hdr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:p><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText> STYLEREF "Heading 1" </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>Saved</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p></w:hdr>"#;
    let out = pages(crate::office::docx::testutil::with_related_part(
        docx(&body, Some(styles), None),
        "header",
        "header1.xml",
        header,
        false,
    ));
    let first_text = |p: &crate::office::docx::paginate::OutPage| {
        p.items
            .iter()
            .filter_map(|i| match i {
                crate::office::pptx::scene::Item::Text(r) => {
                    Some((r.transform.ty, r.text.trim().to_string()))
                }
                _ => None,
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|t| t.1)
            .unwrap()
    };
    let headers: Vec<String> = out.iter().map(first_text).collect();
    assert_eq!(headers, ["Alpha", "Beta", "Beta"]);
}

#[test]
fn a_paragraph_opening_with_a_column_break_keeps_its_space_before_in_the_next_column() {
    // A two-column row: "Left" in column 1, then a paragraph that breaks to column 2
    // before "Right", both with 6 pt before; Word sets them on one baseline.
    let sect = r#"<w:sectPr><w:type w:val="continuous"/><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440"/><w:cols w:num="2" w:space="720"/></w:sectPr>"#;
    let body = format!(
        r#"<w:p><w:pPr><w:spacing w:before="120" w:after="0"/></w:pPr><w:r><w:t>Left</w:t></w:r></w:p><w:p><w:pPr><w:spacing w:before="120" w:after="0"/></w:pPr><w:r><w:br w:type="column"/><w:t>Right</w:t></w:r></w:p><w:p><w:pPr><w:spacing w:after="0"/>{sect}</w:pPr></w:p><w:p><w:pPr><w:spacing w:after="0"/></w:pPr><w:r><w:t>Below</w:t></w:r></w:p><w:sectPr><w:type w:val="continuous"/><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440"/></w:sectPr>"#
    );
    let out = pages(crate::office::docx::testutil::with_compat_mode(
        docx(&body, None, None),
        11,
    ));
    let at = |t: &str| {
        out[0]
            .items
            .iter()
            .find_map(|i| match i {
                crate::office::pptx::scene::Item::Text(r) if r.text.trim() == t => {
                    Some((r.transform.tx, r.transform.ty))
                }
                _ => None,
            })
            .unwrap()
    };
    let (left, right, below) = (at("Left"), at("Right"), at("Below"));
    assert!(right.0 > 300.0, "Right is in column 2: {right:?}");
    assert!((right.1 - left.1).abs() < 0.5, "{left:?} {right:?}");
    // The row is one line tall: the break adds no line to column 1.
    assert!(below.1 - left.1 < 20.0, "{left:?} {below:?}");
}

#[test]
fn rows_of_two_column_sections_with_different_widths_stay_one_line_tall() {
    // Two form rows, each a continuous two-column section ended by an empty paragraph;
    // the column widths change between them. Each row is one line tall.
    let sect = |w1: u32, w2: u32| {
        format!(
            r#"<w:sectPr><w:type w:val="continuous"/><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440"/><w:cols w:num="2" w:space="0" w:equalWidth="0"><w:col w:w="{w1}" w:space="0"/><w:col w:w="{w2}" w:space="0"/></w:cols></w:sectPr>"#
        )
    };
    let row = |a: &str, b: &str, s: String| {
        format!(
            r#"<w:p><w:pPr><w:spacing w:after="0"/></w:pPr><w:r><w:t>{a}</w:t></w:r></w:p><w:p><w:pPr><w:spacing w:after="0"/></w:pPr><w:r><w:br w:type="column"/><w:t>{b}</w:t></w:r></w:p><w:p><w:pPr><w:spacing w:after="0"/>{s}</w:pPr></w:p>"#
        )
    };
    let body = format!(
        r#"{}{}<w:p><w:pPr><w:spacing w:after="0"/></w:pPr><w:r><w:t>Below</w:t></w:r></w:p><w:sectPr><w:type w:val="continuous"/><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440"/></w:sectPr>"#,
        row("Name", "Call", sect(6000, 3360)),
        row("Title", "Group", sect(2000, 7360))
    );
    let out = pages(docx(&body, None, None));
    let y = |t: &str| {
        out[0]
            .items
            .iter()
            .find_map(|i| match i {
                crate::office::pptx::scene::Item::Text(r) if r.text.trim() == t => {
                    Some(r.transform.ty)
                }
                _ => None,
            })
            .unwrap()
    };
    let pitch = y("Title") - y("Name");
    assert!(pitch > 10.0 && pitch < 16.0, "row pitch {pitch}");
    assert!((y("Below") - y("Title") - pitch).abs() < 1.0);
}

#[test]
fn header_lines_do_not_snap_to_the_document_grid() {
    // Four 10.5 pt header lines from 42.55 pt on a 15.6 pt line grid: Word reserves
    // their own height (about 48 pt), not four grid pitches (62.4 pt).
    let header = format!(
        r#"<w:hdr xmlns:w="{}">{}</w:hdr>"#,
        crate::office::docx::testutil::W,
        r#"<w:p><w:pPr><w:spacing w:after="0"/></w:pPr><w:r><w:rPr><w:sz w:val="21"/></w:rPr><w:t>Head</w:t></w:r></w:p>"#.repeat(4)
    );
    let body = r#"<w:p><w:pPr><w:spacing w:after="0"/></w:pPr><w:r><w:t>Body</w:t></w:r></w:p><w:sectPr><w:headerReference w:type="default" r:id="rheader"/><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1000" w:right="1440" w:bottom="1440" w:left="1440" w:header="851" w:footer="720"/><w:docGrid w:type="lines" w:linePitch="312"/></w:sectPr>"#;
    let data = crate::office::docx::testutil::with_related_part(
        docx(body, None, None),
        "header",
        "header1.xml",
        &header,
        false,
    );
    let out = pages(data);
    let body_y = out[0]
        .items
        .iter()
        .find_map(|i| match i {
            crate::office::pptx::scene::Item::Text(r) if r.text.trim() == "Body" => {
                Some(r.transform.ty)
            }
            _ => None,
        })
        .unwrap();
    // The body's first baseline sits below the header's natural extent.
    assert!(body_y < 42.55 + 62.4, "body baseline {body_y}");
    assert!(body_y > 42.55 + 45.0, "body baseline {body_y}");
}

#[test]
fn a_row_moves_whole_where_its_minimum_height_does_not_fit() {
    // Shaped like the Word for Mac probe: filler, then a table whose second row holds
    // 16 lines beside a six-row nested table. Without a minimum the row splits on the
    // first page; with a 212 pt "at least" height it moves whole to the next one.
    let filler: String = (0..40).map(|i| para(&format!("f{i}"))).collect();
    let cell_a: String = (0..16).map(|i| para(&format!("c{i}"))).collect();
    let nested_rows: String = (0..6)
        .map(|i| {
            format!(
                r#"<w:tr><w:trPr><w:trHeight w:val="660"/></w:trPr><w:tc>{}</w:tc></w:tr>"#,
                para(&format!("pic{i}"))
            )
        })
        .collect();
    let nested = format!(
        r#"<w:tbl><w:tblPr><w:tblW w:w="2000" w:type="dxa"/></w:tblPr><w:tblGrid><w:gridCol w:w="2000"/></w:tblGrid>{nested_rows}</w:tbl><w:p/>"#
    );
    let first_cell_page = |tr_pr: &str| {
        let body = format!(
            r#"{filler}<w:tbl><w:tblGrid><w:gridCol w:w="5000"/><w:gridCol w:w="4000"/></w:tblGrid><w:tr><w:tc>{}</w:tc><w:tc>{}</w:tc></w:tr><w:tr><w:trPr>{tr_pr}</w:trPr><w:tc>{cell_a}</w:tc><w:tc>{nested}</w:tc></w:tr></w:tbl>{}"#,
            para("head"),
            para("head2"),
            para("end")
        );
        pages(docx(&body, None, None))
            .iter()
            .position(|page| {
                page.items.iter().any(
                    |i| matches!(i, crate::office::pptx::scene::Item::Text(r) if r.text.as_str() == "c0"),
                )
            })
            .expect("c0 drawn")
    };
    assert_eq!(first_cell_page(""), 0);
    assert_eq!(first_cell_page(r#"<w:trHeight w:val="4249"/>"#), 1);
}

#[test]
fn a_grid_line_may_overrun_the_bottom_margin_by_the_room_below_its_text() {
    // 24 pt grid pitches hold 10 pt lines centred, with about 6 pt below the text:
    // a last line 3 pt past the bottom margin stays, one 9 pt past moves on.
    let lines_on_first_page = |content_tw: u32| {
        let paras: String = ["a", "b", "c", "d"].map(para).concat();
        let sect = format!(
            r#"<w:sectPr><w:pgSz w:w="11906" w:h="{}"/><w:pgMar w:top="720" w:right="1440" w:bottom="720" w:left="1440" w:header="0" w:footer="0"/><w:docGrid w:type="lines" w:linePitch="480"/></w:sectPr>"#,
            content_tw + 1440
        );
        run_ys(&pages(docx(&format!("{paras}{sect}"), None, None))[0]).len()
    };
    assert_eq!(lines_on_first_page(3 * 480 - 60), 3);
    assert_eq!(lines_on_first_page(3 * 480 - 180), 2);
}

#[test]
fn a_lowered_run_lowers_its_inline_picture() {
    // `w:position="-6"` lowers the run 3 pt: its picture's bottom drops 3 pt below
    // where an unmoved picture sits on the shared baseline.
    let shape_bottom_below_text = |position: i32| {
        let shape = r#"<w:drawing><wp:inline xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"><wp:extent cx="127000" cy="127000"/><a:graphic><a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"><wps:wsp><wps:spPr><a:prstGeom prst="rect"/><a:solidFill><a:srgbClr val="FF0000"/></a:solidFill></wps:spPr><wps:bodyPr/></wps:wsp></a:graphicData></a:graphic></wp:inline></w:drawing>"#;
        let body = format!(
            r#"<w:p><w:r><w:t>text</w:t></w:r><w:r><w:rPr><w:position w:val="{position}"/></w:rPr>{shape}</w:r></w:p>"#
        );
        let out = pages(docx(&body, None, None));
        let text = run_ys(&out[0])[0] as f64;
        let bottom = out[0]
            .items
            .iter()
            .find_map(|i| match i {
                crate::office::pptx::scene::Item::Path(p) => Some(p.path.bounds().bottom() as f64),
                _ => None,
            })
            .unwrap();
        bottom - text
    };
    let drop = shape_bottom_below_text(-6) - shape_bottom_below_text(0);
    assert!((drop - 3.0).abs() < 0.01, "drop {drop}");
}

#[test]
fn an_empty_continuous_section_break_paragraph_takes_no_room() {
    // Same columns on both sides: the break's empty paragraph still adds no line,
    // while an ordinary empty paragraph does.
    let brk = r#"<w:p><w:pPr><w:sectPr><w:type w:val="continuous"/></w:sectPr></w:pPr></w:p>"#;
    let last = r#"<w:sectPr><w:type w:val="continuous"/></w:sectPr>"#;
    let gap = |mid: &str| {
        let body = format!("{}{mid}{}{last}", para("a"), para("b"));
        let ys = run_ys(&pages(docx(&body, None, None))[0]);
        (ys[1] - ys[0]) as f64
    };
    let line = gap("");
    assert!(
        (gap(brk) - line).abs() < 0.01,
        "break adds {}",
        gap(brk) - line
    );
    assert!(gap("<w:p/>") > line + 5.0);
}

#[test]
fn an_earlier_line_wraps_around_a_later_page_placed_frame() {
    // The frame is anchored after the long paragraph but placed at the top of the
    // page, beside it: the paragraph's lines wrap and it takes more lines.
    let long = "word ".repeat(40);
    let frame = r#"<w:p><w:pPr><w:framePr w:w="3000" w:h="1440" w:hRule="atLeast" w:wrap="around" w:hAnchor="margin" w:vAnchor="page" w:x="6000" w:y="1440" w:hSpace="240" w:vSpace="240"/></w:pPr><w:r><w:t>frame</w:t></w:r></w:p>"#;
    let lines = |with_frame: bool| {
        let body = format!(
            r#"{}{}<w:sectPr><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440"/></w:sectPr>"#,
            para(&long),
            if with_frame { frame } else { "" }
        );
        let ys = run_ys(&pages(docx(&body, None, None))[0]);
        let mut rows: Vec<i32> = ys.iter().map(|y| y.round() as i32).collect();
        rows.sort();
        rows.dedup();
        rows.len() - usize::from(with_frame)
    };
    assert!(
        lines(true) > lines(false),
        "{} vs {}",
        lines(true),
        lines(false)
    );
}

#[test]
fn a_wide_floating_table_above_its_anchor_pushes_the_line_before_it_down() {
    // The table floats 10 pt above where the text reaches and is wider than the
    // column: the line before it moves below the table, which keeps its place.
    let table = r#"<w:tbl><w:tblPr><w:tblpPr w:vertAnchor="text" w:horzAnchor="margin" w:tblpY="-200"/><w:tblW w:w="9360" w:type="dxa"/></w:tblPr><w:tblGrid><w:gridCol w:w="9360"/></w:tblGrid><w:tr><w:trPr><w:trHeight w:val="1440" w:hRule="exact"/></w:trPr><w:tc><w:tcPr><w:tcW w:w="9360" w:type="dxa"/></w:tcPr><w:p><w:r><w:t>cell</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#;
    let body = format!(
        r#"{}{}{table}{}<w:sectPr><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440"/></w:sectPr>"#,
        para("first"),
        para("before"),
        para("after")
    );
    let out = pages(docx(&body, None, None));
    let y = |t: &str| {
        out[0]
            .items
            .iter()
            .find_map(|i| match i {
                crate::office::pptx::scene::Item::Text(r) if r.text == t => {
                    Some(r.transform.ty as f64)
                }
                _ => None,
            })
            .unwrap()
    };
    assert!(
        y("before") > y("cell") + 60.0,
        "before {} cell {}",
        y("before"),
        y("cell")
    );
    assert!(y("cell") < y("first") + 30.0, "the table keeps its place");
}

#[test]
fn a_continuous_break_into_a_page_one_twip_taller_starts_a_new_page() {
    let size = |h: u32| {
        format!(
            r#"<w:pgSz w:w="11906" w:h="{h}"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440"/>"#
        )
    };
    let count = |h: u32| {
        let body = format!(
            r#"{}<w:p><w:pPr><w:sectPr>{}</w:sectPr></w:pPr></w:p>{}<w:sectPr><w:type w:val="continuous"/>{}</w:sectPr>"#,
            para("a"),
            size(16838),
            para("b"),
            size(h)
        );
        pages(docx(&body, None, None)).len()
    };
    assert_eq!(count(16838), 1);
    assert_eq!(count(16839), 2);
}

#[test]
fn a_header_table_floated_above_its_anchor_lets_the_body_start_higher() {
    // Three 30 pt rows from the 36 pt header distance push the body down; with
    // tblpY -400 the table rises 20 pt and so does the body.
    let body_y = |tblp_y: i32| {
        let row = r#"<w:tr><w:trPr><w:trHeight w:val="600" w:hRule="exact"/></w:trPr><w:tc><w:tcPr><w:tcW w:w="4000" w:type="dxa"/></w:tcPr><w:p><w:r><w:t>h</w:t></w:r></w:p></w:tc></w:tr>"#.repeat(3);
        let header = format!(
            r#"<w:hdr xmlns:w="{}"><w:tbl><w:tblPr><w:tblpPr w:vertAnchor="text" w:horzAnchor="margin" w:tblpY="{tblp_y}"/><w:tblW w:w="4000" w:type="dxa"/></w:tblPr><w:tblGrid><w:gridCol w:w="4000"/></w:tblGrid>{row}</w:tbl><w:p/></w:hdr>"#,
            crate::office::docx::testutil::W
        );
        let body = r#"<w:p><w:pPr><w:spacing w:after="0"/></w:pPr><w:r><w:t>Body</w:t></w:r></w:p><w:sectPr><w:headerReference w:type="default" r:id="rheader"/><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1000" w:right="1440" w:bottom="1440" w:left="1440" w:header="720" w:footer="720"/></w:sectPr>"#;
        let data = crate::office::docx::testutil::with_related_part(
            docx(body, None, None),
            "header",
            "header1.xml",
            &header,
            false,
        );
        pages(data)[0]
            .items
            .iter()
            .find_map(|i| match i {
                crate::office::pptx::scene::Item::Text(r) if r.text.trim() == "Body" => {
                    Some(r.transform.ty as f64)
                }
                _ => None,
            })
            .unwrap()
    };
    let rise = body_y(0) - body_y(-400);
    assert!((rise - 20.0).abs() < 0.5, "rise {rise}");
}

#[test]
fn a_second_paragraph_properties_element_overrides_the_first() {
    // The style element asks for 30 pt before; the second w:pPr sets 0. Word
    // reads both, so the paragraph sits right below the first one.
    let gap = |second: &str| {
        let body = format!(
            r#"{}<w:p><w:pPr><w:spacing w:before="600" w:after="0"/></w:pPr>{second}<w:r><w:t>b</w:t></w:r></w:p>"#,
            para("a")
        );
        let ys = run_ys(&pages(docx(&body, None, None))[0]);
        (ys[1] - ys[0]) as f64
    };
    let plain = gap("");
    let overridden = gap(r#"<w:pPr><w:spacing w:before="0"/></w:pPr>"#);
    assert!(plain - overridden > 25.0, "{plain} vs {overridden}");
}

#[test]
fn a_single_unequal_column_keeps_its_own_width() {
    // `equalWidth="0"` with one 4000-twip column: lines wrap at 200 pt, not at the
    // 468 pt between the margins.
    let lines = |cols: &str| {
        let body = format!(
            r#"{}<w:sectPr><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440"/>{cols}</w:sectPr>"#,
            para(&"word ".repeat(60))
        );
        let mut ys: Vec<i32> = run_ys(&pages(docx(&body, None, None))[0])
            .iter()
            .map(|y| y.round() as i32)
            .collect();
        ys.dedup();
        ys.len()
    };
    let wide = lines("");
    let narrow = lines(r#"<w:cols w:equalWidth="0" w:space="720"><w:col w:w="4000"/></w:cols>"#);
    assert!(narrow >= 2 * wide, "{narrow} lines vs {wide}");
}

#[test]
fn a_line_may_end_after_a_hyphen_before_a_number() {
    // "2012-2013" in a column too narrow for the whole range: Word ends the line
    // after the hyphen, where UAX #14 alone keeps "-2013" together.
    let body = r#"<w:p><w:r><w:t>Years 2012-2013 here</w:t></w:r></w:p><w:sectPr><w:pgSz w:w="2600" w:h="15840"/><w:pgMar w:top="1440" w:right="720" w:bottom="1440" w:left="720"/></w:sectPr>"#;
    let out = pages(docx(body, None, None));
    let first_y = run_ys(&out[0])[0];
    let first_line: String = out[0]
        .items
        .iter()
        .filter_map(|i| match i {
            crate::office::pptx::scene::Item::Text(r) if (r.transform.ty - first_y).abs() < 0.5 => {
                Some(r.text.clone())
            }
            _ => None,
        })
        .collect();
    assert!(first_line.trim_end().ends_with("2012-"), "{first_line:?}");
}

#[test]
fn a_line_of_only_lowered_text_draws_from_the_line_top() {
    // Every run lowered 8 pt: the line's ascent goes down with the text, so the
    // text sits where it would unlowered and only the descent grows (Word).
    let first_y = |position: &str| {
        let body = format!(
            r#"<w:p><w:r><w:rPr>{position}<w:sz w:val="28"/></w:rPr><w:t>Title</w:t></w:r></w:p>"#
        );
        run_ys(&pages(docx(&body, None, None))[0])[0] as f64
    };
    let shift = first_y(r#"<w:position w:val="-16"/>"#) - first_y("");
    assert!(shift.abs() < 0.5, "lowered line moved {shift}");
}

#[test]
fn a_floating_table_too_low_for_its_first_row_leads_the_next_page_with_its_anchor() {
    // A page nearly full, then a three-row text-anchored table 40 pt below its
    // anchor heading whose first row cannot split: both move on, the heading tops
    // the next page, the table below.
    let row = r#"<w:tr><w:trPr><w:cantSplit/><w:trHeight w:val="1200" w:hRule="exact"/></w:trPr><w:tc><w:tcPr><w:tcW w:w="9000" w:type="dxa"/></w:tcPr><w:p><w:r><w:t>cell</w:t></w:r></w:p></w:tc></w:tr>"#.repeat(3);
    let table = format!(
        r#"<w:tbl><w:tblPr><w:tblpPr w:vertAnchor="text" w:horzAnchor="margin" w:tblpY="800"/><w:tblW w:w="9000" w:type="dxa"/></w:tblPr><w:tblGrid><w:gridCol w:w="9000"/></w:tblGrid>{row}</w:tbl>"#
    );
    let filler: String = (0..50).map(|i| para(&format!("line {i}"))).collect();
    let body = format!(
        r#"{filler}{table}{}<w:sectPr><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440"/></w:sectPr>"#,
        para("Heading")
    );
    let out = pages(docx(&body, None, None));
    let page_of = |t: &str| {
        out.iter()
            .position(|p| {
                p.items
                    .iter()
                    .any(|i| matches!(i, crate::office::pptx::scene::Item::Text(r) if r.text.trim() == t))
            })
            .unwrap()
    };
    let y_of = |pg: usize, t: &str| {
        out[pg]
            .items
            .iter()
            .find_map(|i| match i {
                crate::office::pptx::scene::Item::Text(r) if r.text.trim() == t => {
                    Some(r.transform.ty)
                }
                _ => None,
            })
            .unwrap()
    };
    let hp = page_of("Heading");
    assert_eq!(hp, page_of("cell"), "heading and table share a page");
    assert!(
        y_of(hp, "Heading") < y_of(hp, "cell"),
        "heading above the table"
    );
}
