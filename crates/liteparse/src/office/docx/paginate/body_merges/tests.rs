use super::*;

impl Cursor {
    fn trial(&self, destination: Destination, top: f64, bottom: f64) -> Trial {
        self.trial_in_area(destination, top, bottom, bottom - top, true)
    }
}

fn cursor() -> Cursor {
    Cursor::new(Rc::new(
        (0..3)
            .map(|i| Slice {
                y: i as f64 * 12.0,
                h: 12.0,
                items: Vec::new(),
                anchors: Vec::new(),
                paragraph_offset: 0.0,
                widow: None,
                notes: vec![i + 1],
                nested_row: None,
                nested_table: None,
                paragraph_source: None,
            })
            .collect(),
    ))
}

#[test]
fn adjacent_rows_share_origin_and_rejected_trials_preserve_source() {
    let source = cursor();
    let destination = Destination { page: 0, column: 0 };
    let first = source.trial(destination, 100.0, 118.0);
    assert_eq!(first.notes, vec![1]);
    let rejected = first.cursor.trial(destination, 118.0, 136.0);
    assert_eq!(rejected.notes, vec![2, 3]);
    let accepted = first.cursor.trial(destination, 118.0, 125.0);
    assert_eq!(accepted.notes, vec![2]);
    assert_eq!(accepted.slices[0].y, 112.0);
    assert!(!accepted.cursor.complete());
    assert_eq!(source.trial(destination, 100.0, 118.0).notes, vec![1]);
}

#[test]
fn next_column_rebases_only_unconsumed_payload() {
    let first = cursor().trial(Destination { page: 0, column: 0 }, 100.0, 118.0);
    let next = first
        .cursor
        .trial(Destination { page: 0, column: 1 }, 20.0, 44.0);
    assert_eq!(next.notes, vec![2, 3]);
    assert_eq!(
        next.slices.iter().map(|s| s.y).collect::<Vec<_>>(),
        vec![20.0, 32.0]
    );
    assert!(next.cursor.complete());
}

#[test]
fn atomic_oversize_waits_for_fresh_destination_then_progresses() {
    let mut source = cursor();
    let slices = Rc::make_mut(&mut source.source);
    slices.truncate(1);
    slices[0].h = 300.0;
    let destination = Destination { page: 0, column: 0 };
    assert!(
        !source
            .trial_in_area(destination, 100.0, 200.0, 200.0, false)
            .cursor
            .complete()
    );
    assert!(
        source
            .trial_in_area(destination, 0.0, 200.0, 200.0, true)
            .cursor
            .complete()
    );
}

#[test]
fn top_merge_retains_late_reference_and_its_source_text() {
    use crate::office::docx::testutil::{W, docx, pages, with_related_part};
    let paragraph = |text: &str, reference: bool| {
        format!(
            r#"<w:p><w:pPr><w:spacing w:before="0" w:after="0" w:line="240" w:lineRule="exact"/><w:widowControl w:val="0"/></w:pPr><w:r><w:rPr><w:sz w:val="12"/></w:rPr><w:t>{text}</w:t></w:r>{}</w:p>"#,
            if reference {
                r#"<w:r><w:footnoteReference w:id="1"/></w:r>"#
            } else {
                ""
            }
        )
    };
    let source = (0..45)
        .map(|i| paragraph(&format!("MRG{i:04}"), i == 24))
        .collect::<String>();
    let mut rows = String::new();
    for row in 0..3 {
        let merge = if row == 0 {
            r#"<w:vMerge w:val="restart"/>"#
        } else {
            "<w:vMerge/>"
        };
        let content = if row == 0 {
            source.clone()
        } else {
            paragraph("", false)
        };
        let sibling = (0..10)
            .map(|i| paragraph(&format!("R{row}X{i:04}"), false))
            .collect::<String>();
        rows.push_str(&format!(r#"<w:tr><w:tc><w:tcPr>{merge}<w:vAlign w:val="top"/></w:tcPr>{content}</w:tc><w:tc>{sibling}</w:tc></w:tr>"#));
    }
    let body = format!(
        r#"<w:tbl><w:tblPr><w:tblCellMar><w:top w:w="0" w:type="dxa"/><w:bottom w:w="0" w:type="dxa"/></w:tblCellMar></w:tblPr><w:tblGrid><w:gridCol w:w="2400"/><w:gridCol w:w="2400"/></w:tblGrid>{rows}</w:tbl><w:sectPr><w:pgSz w:w="7200" w:h="6000"/><w:pgMar w:top="400" w:bottom="400" w:left="400" w:right="400"/></w:sectPr>"#
    );
    let bytes = with_related_part(
        docx(&body, None, None),
        "footnotes",
        "footnotes.xml",
        &format!(
            r#"<w:footnotes xmlns:w="{W}"><w:footnote w:id="1">{}</w:footnote></w:footnotes>"#,
            paragraph("NOTE0001", false)
        ),
        false,
    );
    let output = pages(bytes);
    for page in &output {
        for item in &page.items {
            if let crate::office::pptx::scene::Item::Text(run) = item
                && run.text.starts_with("MRG")
            {
                assert!(
                    run.transform.ty >= 20.0 && run.transform.ty < 280.0,
                    "{} outside body at {}",
                    run.text,
                    run.transform.ty
                );
            }
        }
    }
    let texts: Vec<_> = output
        .iter()
        .map(|page| {
            page.items
                .iter()
                .filter_map(|item| match item {
                    crate::office::pptx::scene::Item::Text(run) => Some(run.text.as_str()),
                    _ => None,
                })
                .collect::<String>()
        })
        .collect();
    let all = texts.concat();
    for index in 0..45 {
        let label = format!("MRG{index:04}");
        assert_eq!(all.matches(&label).count(), 1, "{label}");
    }
    let owner = texts
        .iter()
        .position(|text| text.contains("MRG0024"))
        .unwrap();
    let note = texts
        .iter()
        .position(|text| text.contains("NOTE0001"))
        .unwrap();
    assert_eq!(
        owner,
        note,
        "page refs {:?}; note texts {:?}",
        output.iter().map(|p| &p.info.notes).collect::<Vec<_>>(),
        texts
    );
}

#[test]
fn rejected_occupied_extent_keeps_source_and_references_for_next_area() {
    let source = cursor();
    let rejected = source.trial_with_geometry(
        Destination { page: 0, column: 0 },
        260.0,
        300.0,
        280.0,
        false,
        |slice, dy, _| slice.y + dy + 115.0 <= 300.0,
    );
    assert!(rejected.slices.is_empty());
    assert!(rejected.notes.is_empty());
    assert_eq!(rejected.cursor.next, 0);
    let accepted = rejected.cursor.trial_with_geometry(
        Destination { page: 1, column: 0 },
        20.0,
        300.0,
        280.0,
        true,
        |slice, dy, _| slice.y + dy + 115.0 <= 300.0,
    );
    assert_eq!(accepted.notes, vec![1, 2, 3]);
    assert!(accepted.cursor.complete());
    assert_eq!(source.next, 0);
}
