use super::*;
use crate::office::docx::testutil::{W, docx, pages, with_footnote, with_related_part};

fn section(position: &str, kind: &str) -> String {
    format!(
        r#"<w:sectPr>{position}{kind}<w:pgSz w:w="8000" w:h="10000"/><w:pgMar w:top="800" w:bottom="800" w:left="800" w:right="800"/></w:sectPr>"#
    )
}

fn paragraph(text: &str, after: u32, note: bool) -> String {
    let reference = if note {
        r#"<w:footnoteReference w:id="1"/>"#
    } else {
        ""
    };
    format!(
        r#"<w:p><w:pPr><w:spacing w:before="0" w:after="{after}" w:line="400" w:lineRule="exact"/></w:pPr><w:r><w:t>{text}</w:t>{reference}</w:r></w:p>"#
    )
}

#[test]
fn bottom_position_and_ignored_document_position_preserve_drawn_items() {
    let render = |section_position: &str, document_position: bool| {
        let body = paragraph("body", 0, true) + &section(section_position, "");
        let mut bytes = with_footnote(docx(&body, None, None), "note");
        if document_position {
            let settings = format!(
                r#"<w:settings xmlns:w="{W}"><w:footnotePr><w:pos w:val="beneathText"/></w:footnotePr></w:settings>"#
            );
            bytes = with_related_part(bytes, "settings", "settings.xml", &settings, false);
        }
        pages(bytes)
    };
    let baseline = render("", false);
    for out in [
        render(
            r#"<w:footnotePr><w:pos w:val="pageBottom"/></w:footnotePr>"#,
            false,
        ),
        render("", true),
    ] {
        assert_eq!(out.len(), baseline.len());
        assert_eq!(
            format!("{:?}", out[0].items),
            format!("{:?}", baseline[0].items)
        );
        assert_eq!(
            out[0].info.note_regions[0].position,
            FootnotePosition::PageBottom
        );
    }
}

#[test]
fn final_note_metadata_distinguishes_body_bottom_from_terminal_spacing() {
    for after in [0, 600] {
        let body = paragraph("body", after, true) + &section("", "");
        let out = pages(with_footnote(docx(&body, None, None), "note"));
        let region = &out[0].info.note_regions[0];
        assert_eq!(region.notes, vec![1]);
        assert!((region.body_bottom - 60.0).abs() < 0.01);
        assert_eq!((region.x, region.width), (40.0, 320.0));
        assert!(region.exclusions.is_empty());
    }
}

#[test]
fn continuous_note_owner_is_separate_from_header_section() {
    let first = paragraph("first", 0, false);
    let boundary = format!("<w:p><w:pPr>{}</w:pPr></w:p>", section("", ""));
    let last = section(
        r#"<w:footnotePr><w:pos w:val="sectEnd"/></w:footnotePr>"#,
        r#"<w:type w:val="continuous"/>"#,
    );
    let body = first + &boundary + &paragraph("second", 0, true) + &last;
    let out = pages(with_footnote(docx(&body, None, None), "note"));
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].info.section, 0);
    assert_eq!(out[0].info.note_regions.len(), 2);
    let owner = out[0]
        .info
        .note_regions
        .iter()
        .find(|r| !r.notes.is_empty())
        .unwrap();
    assert_eq!(owner.section, 1);
    assert_eq!(owner.column, 0);
    assert_eq!(owner.position, FootnotePosition::BeneathText);
    assert_eq!(owner.notes, vec![1]);
}

#[test]
fn column_reference_metadata_keeps_the_actual_column() {
    let first = paragraph("first", 0, false);
    let break_column = r#"<w:p><w:r><w:br w:type="column"/></w:r></w:p>"#;
    let last = section("", "").replace(
        "</w:sectPr>",
        r#"<w:cols w:num="2" w:space="400"/></w:sectPr>"#,
    );
    let body = first + break_column + &paragraph("second", 0, true) + &last;
    let out = pages(with_footnote(docx(&body, None, None), "note"));
    let owner = out[0]
        .info
        .note_regions
        .iter()
        .find(|r| !r.notes.is_empty())
        .unwrap();
    assert_eq!(owner.column, 1);
    assert_eq!((owner.x, owner.width), (210.0, 150.0));
    assert_eq!(owner.notes, vec![1]);
}

#[test]
fn floating_rows_keep_exclusions_without_advancing_normal_body_extent() {
    let row = |text: &str| {
        format!(
            r#"<w:tr><w:trPr><w:trHeight w:val="800" w:hRule="atLeast"/></w:trPr><w:tc><w:tcPr><w:tcW w:w="3000"/></w:tcPr>{}</w:tc></w:tr>"#,
            paragraph(text, 0, false)
        )
    };
    let table = format!(
        r#"<w:tbl><w:tblPr><w:tblpPr w:horzAnchor="margin" w:vertAnchor="page" w:tblpY="4000"/><w:tblW w:w="3000"/></w:tblPr><w:tblGrid><w:gridCol w:w="3000"/></w:tblGrid>{}{}</w:tbl>"#,
        row("floating one"),
        row("floating two")
    );
    let body =
        paragraph("before", 0, true) + &table + &paragraph("after", 0, false) + &section("", "");
    let out = pages(with_footnote(docx(&body, None, None), "note"));
    let region = &out[0].info.note_regions[0];
    assert!((region.body_bottom - 80.0).abs() < 0.01, "{region:?}");
    assert_eq!(region.exclusions.len(), 2);
    assert!(region.exclusions.iter().all(|r| r.0 >= 200.0 && r.1 > r.0));
}

#[test]
fn accepted_row_extent_counts_outer_before_once() {
    let body = r#"<w:tbl><w:tblPr><w:tblBorders><w:top w:val="single" w:sz="32"/><w:bottom w:val="single" w:sz="48"/></w:tblBorders></w:tblPr><w:tblGrid><w:gridCol w:w="4000"/></w:tblGrid><w:tr><w:tc><w:p><w:r><w:t>row</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#;
    super::super::tests::with_column_pager(docx(body, None, None), |pager, sections| {
        let props = &sections[0].props;
        let mut units = crate::office::docx::layout::Flow::new(pager.env, 15).units(
            &sections[0].blocks,
            props.columns()[0].1,
            false,
        );
        // Inject outer spacing to exercise the placement-coordinate invariant.
        let crate::office::docx::layout::Unit::Row(row) = &mut units[0] else {
            panic!("row unit")
        };
        row.outer_before = 7.0;
        row.outer_after = 9.0;
        pager.place_units(&units, 0, props);
        let Placed::Row {
            row,
            y,
            normal_flow,
            ..
        } = &pager.placed[0]
        else {
            panic!("row")
        };
        assert!(*normal_flow);
        assert!(row.outer_before > 0.0);
        let expected = y + row.height + row.outer_after;
        record(&pager.placed, &units, 0, props, &mut pager.pages);
        assert!((pager.pages[0].note_regions[0].body_bottom - expected).abs() < 0.01);
    });
}

#[test]
fn keep_chain_retries_leave_only_accepted_reference_metadata() {
    let first = paragraph("first", 0, false).replace(r#"w:line="400""#, r#"w:line="7200""#);
    let note = paragraph("kept note", 0, true).replace("<w:pPr>", "<w:pPr><w:keepNext/>");
    let last = paragraph("last", 0, false).replace(r#"w:line="400""#, r#"w:line="1200""#);
    let out = pages(with_footnote(
        docx(&(first + &note + &last + &section("", "")), None, None),
        "UNIQUE_FOOTNOTE",
    ));
    assert_eq!(out.len(), 2);
    assert!(out[0].info.notes.is_empty(), "{:?}", out[0].info.notes);
    assert_eq!(out[1].info.notes, vec![1]);
    let note_counts: Vec<usize> = out
        .iter()
        .map(|page| {
            page.items
                .iter()
                .filter(|item| {
                    matches!(item, crate::office::pptx::scene::Item::Text(run)
                    if run.text == "UNIQUE_FOOTNOTE")
                })
                .count()
        })
        .collect();
    assert_eq!(note_counts, vec![0, 1]);
    assert!(out[0].info.note_regions.iter().all(|r| r.notes.is_empty()));
    assert_eq!(
        out[1]
            .info
            .note_regions
            .iter()
            .flat_map(|r| &r.notes)
            .copied()
            .collect::<Vec<_>>(),
        vec![1]
    );
    assert!((out[0].info.note_regions[0].body_bottom - 400.0).abs() < 0.01);
}

#[test]
fn balanced_placements_determine_final_region_extents() {
    let mut body = String::new();
    for i in 0..8 {
        body.push_str(&paragraph(&format!("line {i}"), 0, i == 6));
    }
    let two_columns = section("", "").replace(
        "</w:sectPr>",
        r#"<w:cols w:num="2" w:space="400"/></w:sectPr>"#,
    );
    body.push_str(&format!("<w:p><w:pPr>{two_columns}</w:pPr></w:p>"));
    body.push_str(&paragraph("next section", 0, false));
    body.push_str(&section("", r#"<w:type w:val="continuous"/>"#));
    let out = pages(with_footnote(docx(&body, None, None), "note"));
    assert_eq!(out.len(), 1);
    let regions: Vec<_> = out[0]
        .info
        .note_regions
        .iter()
        .filter(|r| r.section == 0)
        .collect();
    assert_eq!(regions.len(), 2);
    assert!(regions.iter().all(|r| r.body_bottom < 200.0));
    assert_eq!(
        regions
            .iter()
            .flat_map(|r| &r.notes)
            .copied()
            .collect::<Vec<_>>(),
        vec![1]
    );
    assert_eq!(
        regions.iter().find(|r| !r.notes.is_empty()).unwrap().column,
        1
    );
}

#[test]
fn rejected_trial_restores_existing_page_geometry_and_note_fragments() {
    let body = paragraph("accepted", 0, true) + &section("", "");
    super::super::tests::with_column_pager(
        with_footnote(docx(&body, None, None), "accepted note"),
        |pager, sections| {
            let props = &sections[0].props;
            let units = crate::office::docx::layout::Flow::new(pager.env, 15).units(
                &sections[0].blocks,
                props.columns()[0].1,
                false,
            );
            pager.place_units(&units, 0, props);
            record(&pager.placed, &units, 0, props, &mut pager.pages);
            // Exercise fragment metadata independently of continuation support.
            pager.pages[0].note_cut = Some((1, 12.0));
            pager.pages[0].note_carry = Some((2, 6.0));
            let before = format!("{:?}", pager.pages);
            let placed = pager.placed.len();
            let snap = pager.snapshot();
            let page = &mut pager.pages[0];
            page.notes.push(3);
            page.note_cut = None;
            page.note_carry = None;
            page.note_regions.clear();
            page.cols = vec![(90.0, 180.0), (290.0, 180.0)];
            page.section = 7;
            page.first_of_section = false;
            pager.new_page(0, props, sections);
            pager.restore(&snap);
            assert_eq!(format!("{:?}", pager.pages), before);
            assert_eq!(pager.placed.len(), placed);
        },
    );
}

#[test]
fn beneath_text_places_short_notes_after_accepted_body() {
    let render = |position: &str, after: u32| {
        let pos = format!(r#"<w:footnotePr><w:pos w:val="{position}"/></w:footnotePr>"#);
        pages(with_footnote(
            docx(
                &(paragraph("body", after, true) + &section(&pos, "")),
                None,
                None,
            ),
            "note",
        ))
    };
    let note_top = |out: &[crate::office::docx::paginate::OutPage]| {
        out[0]
            .items
            .iter()
            .find_map(|item| match item {
                crate::office::pptx::scene::Item::Text(run) if run.text.contains("note") => {
                    Some(run.transform.ty as f64 - run.ascent)
                }
                _ => None,
            })
            .unwrap()
    };
    let bottom = render("pageBottom", 0);
    let beneath = render("beneathText", 0);
    assert_eq!(bottom.len(), 1);
    assert_eq!(beneath.len(), 1);
    assert!(
        note_top(&beneath) < 100.0,
        "note top {}",
        note_top(&beneath)
    );
    assert!(note_top(&bottom) > 400.0);
    assert!((note_top(&render("beneathText", 600)) - note_top(&beneath)).abs() < 0.01);
    assert!((note_top(&render("sectEnd", 0)) - note_top(&beneath)).abs() < 0.01);
}

#[test]
fn beneath_text_origin_sweeps_full_area_and_retains_reference_owner() {
    use super::super::note_areas::accepted_note_top;
    let body = paragraph("body", 0, true)
        + &section(
            r#"<w:footnotePr><w:pos w:val="beneathText"/></w:footnotePr>"#,
            "",
        );
    let out = pages(with_footnote(docx(&body, None, None), "note"));
    let mut page = out[0].info.clone();
    let context = page.note_areas[0].context.clone();
    let span = (context.x, context.x + context.width);
    page.note_regions[0].exclusions = vec![
        (70.0, 100.0, 40.0, 100.0),
        (120.0, 150.0, 40.0, 100.0),
        (150.0, 300.0, 380.0, 400.0),
    ];
    assert_eq!(
        accepted_note_top(&mut page, &context, 30.0, span, true),
        150.0
    );
    page.note_regions[0].exclusions.clear();
    let mut later = page.note_regions[0].clone();
    later.section = 1;
    later.position = FootnotePosition::PageBottom;
    later.notes.clear();
    later.body_bottom = 120.0;
    page.note_regions.push(later);
    assert_eq!(
        accepted_note_top(&mut page, &context, 30.0, span, true),
        120.0
    );
    page.note_regions[1].notes.push(2);
    let bottom = page.bottom - 30.0;
    assert_eq!(
        accepted_note_top(&mut page, &context, 30.0, span, true),
        bottom
    );
    page.note_regions[1].notes.clear();
    page.note_regions[1].body_columns = 2;
    assert_eq!(
        accepted_note_top(&mut page, &context, 30.0, span, true),
        bottom
    );
    page.note_regions.clear();
    page.section = 2;
    let top = page.top;
    assert_eq!(
        accepted_note_top(&mut page, &context, 30.0, span, true),
        top
    );
    let mut multi = context.clone();
    multi.body_columns = 2;
    assert_eq!(
        accepted_note_top(&mut page, &multi, 30.0, span, true),
        bottom
    );
}
