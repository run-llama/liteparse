use super::*;
use crate::office::docx::testutil::{W, docx, pages, with_related_part};

fn para(text: &str, note: Option<i64>, column_break: bool) -> String {
    let reference = note.map_or(String::new(), |id| {
        format!(r#"<w:footnoteReference w:id="{id}"/>"#)
    });
    let control = if column_break {
        r#"<w:br w:type="column"/>"#
    } else {
        ""
    };
    format!(
        r#"<w:p><w:pPr><w:spacing w:before="0" w:after="0" w:line="360" w:lineRule="exact"/><w:widowControl w:val="0"/></w:pPr><w:r><w:t>{text}</w:t>{reference}{control}</w:r></w:p>"#
    )
}

fn section(columns: usize) -> String {
    format!(
        r#"<w:sectPr><w:pgSz w:w="8000" w:h="10000"/><w:pgMar w:top="800" w:bottom="800" w:left="800" w:right="800"/><w:cols w:num="{columns}" w:space="400"/></w:sectPr>"#
    )
}

fn source(body: &str) -> Vec<u8> {
    let mut notes = format!(
        r#"<w:footnotes xmlns:w="{W}"><w:footnote w:type="separator" w:id="-1"><w:p><w:pPr><w:spacing w:line="120" w:lineRule="exact"/></w:pPr><w:r><w:separator/></w:r></w:p></w:footnote>"#
    );
    for id in 1..=3 {
        notes.push_str(&format!(r#"<w:footnote w:id="{id}"><w:p><w:pPr><w:spacing w:before="0" w:after="0" w:line="240" w:lineRule="exact"/></w:pPr><w:r><w:t>NOTE_{id}</w:t></w:r></w:p></w:footnote>"#));
    }
    notes.push_str("</w:footnotes>");
    with_related_part(
        docx(body, None, None),
        "footnotes",
        "footnotes.xml",
        &notes,
        false,
    )
}

#[test]
fn equal_column_notes_follow_first_accepted_reference_and_keep_global_order() {
    let body = para("left", Some(1), true)
        + &para("right", Some(2), false)
        + &para("repeat", Some(1), false)
        + &section(2);
    let out = pages(source(&body));
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].info.notes, vec![1, 2]);
    let areas = &out[0].info.note_areas;
    assert_eq!(areas.len(), 2);
    assert_eq!(areas[0].references.len(), 1);
    assert_eq!(areas[1].references.len(), 1);
    assert_eq!((areas[0].context.x, areas[1].context.x), (40.0, 210.0));
    let notes: Vec<_> = out[0]
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Text(run) if run.text.starts_with("NOTE_") => {
                Some((run.text.clone(), run.transform.tx))
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        notes,
        vec![("NOTE_1".into(), 40.0), ("NOTE_2".into(), 210.0)]
    );
}

#[test]
fn later_column_note_does_not_reserve_first_column_or_overlap_it() {
    let mut body = String::new();
    for i in 0..32 {
        body.push_str(&para(
            &format!("BODY_{i:02}"),
            (i == 24).then_some(1),
            false,
        ));
    }
    body.push_str(&section(2));
    let out = pages(source(&body));
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].info.note_areas[0].context.x, 210.0);
    let texts: Vec<_> = out[0]
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Text(run) => Some(run),
            _ => None,
        })
        .collect();
    let left_last = texts.iter().find(|r| r.text == "BODY_22").unwrap();
    let note = texts.iter().find(|r| r.text == "NOTE_1").unwrap();
    assert_eq!(left_last.transform.tx, 40.0);
    assert_eq!(note.transform.tx, 210.0);
    assert_eq!(
        texts.iter().filter(|r| r.text.starts_with("BODY_")).count(),
        32
    );
}

#[test]
fn note_cache_captures_width_and_grid_without_mutating_body_context() {
    let body = para("reference", Some(1), false) + &section(2);
    let text = "漢字測試".repeat(40);
    let notes = format!(
        r#"<w:footnotes xmlns:w="{W}"><w:footnote w:id="1"><w:p><w:r><w:rPr><w:snapToGrid/></w:rPr><w:t>{text}</w:t></w:r></w:p></w:footnote></w:footnotes>"#
    );
    let bytes = with_related_part(
        docx(&body, None, None),
        "footnotes",
        "footnotes.xml",
        &notes,
        false,
    );
    super::super::tests::with_column_pager(bytes, |pager, _| {
        let mut context = pager.current_note_context().0;
        pager.env.char_grid.set(Some(7.0));
        let narrow = pager.note_in(&context, 1);
        context.width *= 2.0;
        let wide = pager.note_in(&context, 1);
        assert!(narrow.1 > wide.1, "narrow={}, wide={}", narrow.1, wide.1);
        context.grid = Some(3.0);
        let tracked = pager.note_in(&context, 1);
        assert!(tracked.1 > wide.1, "tracked={}, wide={}", tracked.1, wide.1);
        assert_eq!(pager.note_cache.borrow().len(), 3);
        assert_eq!(pager.env.char_grid.get(), Some(7.0));
        context.width /= 2.0;
        context.grid = None;
        assert_eq!(
            format!("{:?}", pager.note_in(&context, 1)),
            format!("{narrow:?}")
        );
    });
}

#[test]
fn interleaved_continuous_areas_emit_notes_in_global_reference_order() {
    let first = section(2);
    let second = section(2).replace("<w:sectPr>", r#"<w:sectPr><w:type w:val="continuous"/>"#);
    let body = para("left", Some(1), true)
        + &para("right", Some(2), false)
        + &format!("<w:p><w:pPr>{first}</w:pPr></w:p>")
        + &para("later left", Some(3), false)
        + &second;
    let out = pages(source(&body));
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].info.notes, vec![1, 2, 3]);
    assert_eq!(out[0].info.note_areas.len(), 2);
    assert_eq!(
        out[0].info.note_areas[0]
            .references
            .iter()
            .map(|r| r.id)
            .collect::<Vec<_>>(),
        vec![1, 3]
    );
    let notes = out[0]
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Text(run) if run.text.starts_with("NOTE_") => Some(run.text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(notes, vec!["NOTE_1", "NOTE_2", "NOTE_3"]);
}

#[test]
fn mixed_continuous_geometry_uses_one_captured_legacy_area() {
    let first = section(1);
    let second = section(2).replace("<w:sectPr>", r#"<w:sectPr><w:type w:val="continuous"/>"#);
    let body = para("wide", Some(1), false)
        + &format!("<w:p><w:pPr>{first}</w:pPr></w:p>")
        + &para("narrow", Some(2), false)
        + &second;
    let out = pages(source(&body));
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].info.note_areas.len(), 1);
    assert_eq!(out[0].info.note_areas[0].context.width, 320.0);
    assert_eq!(out[0].info.notes, vec![1, 2]);
    assert!(!out[0].info.note_diagnostics.is_empty());
    let refs = &out[0].info.note_areas[0].references;
    assert_eq!(
        refs.iter().map(|r| r.context.section).collect::<Vec<_>>(),
        vec![0, 1]
    );
}

#[test]
fn picture_reservation_intersects_the_note_area_and_survives_column_changes() {
    let body = para("body", None, false) + &section(2);
    super::super::tests::with_column_pager(source(&body), |pager, sections| {
        let props = &sections[0].props;
        pager.next_column(0, props, sections);
        assert!(!pager.picture_note_conflict(&[(430.0, 460.0, 40.0, 180.0)], &[1]));
        assert!(pager.picture_note_conflict(&[(430.0, 460.0, 40.0, 360.0)], &[1]));
        pager.accept_notes(&[1]);
        pager.st.col = 0;
        assert_eq!(pager.bottom(), pager.pages[0].bottom);
        assert!(pager.picture_note_conflict(&[(430.0, 460.0, 210.0, 360.0)], &[]));
        assert!(!pager.picture_note_conflict(&[(100.0, 150.0, 40.0, 360.0)], &[]));
    });
}

#[test]
fn carried_note_rendering_retains_existing_fragment_with_diagnostic() {
    let body = para("body", None, false) + &section(1);
    super::super::tests::with_column_pager(source(&body), |pager, _| {
        pager.pages[0].note_carry = Some((2, 0.0));
        let mut out = vec![OutPage {
            info: pager.pages[0].clone(),
            items: Vec::new(),
        }];
        pager.draw_notes(&mut out);
        assert_eq!(
            out[0]
                .items
                .iter()
                .filter(|item| matches!(item, Item::Text(run) if run.text == "NOTE_2"))
                .count(),
            1
        );
        assert!(!out[0].info.note_diagnostics.is_empty());
    });
}

#[test]
fn table_reference_uses_accepted_column_area() {
    let table = r#"<w:tbl><w:tblGrid><w:gridCol w:w="2000"/></w:tblGrid><w:tr><w:tc><w:p><w:r><w:t>cell</w:t><w:footnoteReference w:id="2"/></w:r></w:p></w:tc></w:tr></w:tbl>"#;
    let body = para("left", Some(1), true) + table + &section(2);
    let out = pages(source(&body));
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].info.note_areas.len(), 2);
    assert_eq!(out[0].info.note_areas[1].references[0].id, 2);
    assert_eq!(out[0].info.note_areas[1].context.x, 210.0);
}

#[test]
fn preflight_normalizes_equivalent_counts_and_resized_pages() {
    let first = section(2);
    let second = section(2).replace("<w:sectPr>", r#"<w:sectPr xmlns:w15="http://schemas.microsoft.com/office/word/2012/wordml"><w:type w:val="continuous"/><w15:footnoteColumns w:val="2"/>"#);
    let make = |tail: &str| {
        source(
            &(para("first", Some(1), false)
                + &format!("<w:p><w:pPr>{first}</w:pPr></w:p>")
                + &para("second", Some(2), false)
                + tail),
        )
    };
    let opened = crate::office::docx::extract::open(make(&second), &Default::default()).unwrap();
    assert!(mixed_geometry_sections(&opened.sections, &opened.notes).is_empty());
    let resized = second.replace("w:w=\"8000\"", "w:w=\"10000\"");
    let opened = crate::office::docx::extract::open(make(&resized), &Default::default()).unwrap();
    assert!(mixed_geometry_sections(&opened.sections, &opened.notes).is_empty());
    let custom = second.replace("w:val=\"2\"", "w:val=\"1\"");
    let opened = crate::office::docx::extract::open(make(&custom), &Default::default()).unwrap();
    assert_eq!(
        mixed_geometry_sections(&opened.sections, &opened.notes).len(),
        2
    );
}

#[test]
fn unequal_mixed_table_cohort_uses_source_destination_contexts() {
    for table_first in [false, true] {
        let body = para("LEFT", Some(if table_first { 2 } else { 1 }), true)
            + &para("RIGHT", Some(if table_first { 1 } else { 2 }), false)
            + r#"<w:sectPr><w:pgSz w:w="6000" w:h="6000"/><w:pgMar w:top="600" w:bottom="600" w:left="600" w:right="600"/><w:cols w:num="2" w:equalWidth="0"><w:col w:w="1600" w:space="400"/><w:col w:w="2800"/></w:cols></w:sectPr>"#;
        let notes = format!(
            r#"<w:footnotes xmlns:w="{W}"><w:footnote w:id="1"><w:p><w:r><w:t>PLAIN_NOTE</w:t></w:r></w:p></w:footnote><w:footnote w:id="2"><w:tbl><w:tblPr><w:tblW w:w="1200" w:type="dxa"/></w:tblPr><w:tblGrid><w:gridCol w:w="1200"/></w:tblGrid><w:tr><w:tc><w:p><w:r><w:t>TABLE_NOTE</w:t></w:r></w:p></w:tc></w:tr></w:tbl></w:footnote></w:footnotes>"#
        );
        let out = pages(with_related_part(
            docx(&body, None, None),
            "footnotes",
            "footnotes.xml",
            &notes,
            false,
        ));
        let areas: Vec<_> = out.iter().flat_map(|p| &p.info.note_areas).collect();
        assert!(!areas.is_empty());
        assert!(areas.iter().all(|a| {
            [80.0, 140.0]
                .iter()
                .any(|w| (a.context.width - w).abs() < EPS)
        }));
        let text: String = out
            .iter()
            .flat_map(|p| &p.items)
            .filter_map(|i| match i {
                Item::Text(t) => Some(t.text.as_str()),
                _ => None,
            })
            .collect();
        for token in ["PLAIN_NOTE", "TABLE_NOTE"] {
            assert_eq!(text.matches(token).count(), 1, "{text}");
        }
    }
}
