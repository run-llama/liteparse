use super::*;
use crate::office::docx::testutil::{W, docx, pages, with_related_part};

fn source(count: usize, two: bool) -> Vec<u8> {
    let body = format!(
        r#"<w:p><w:pPr><w:spacing w:before="0" w:after="0" w:line="240" w:lineRule="exact"/></w:pPr><w:r><w:t>BODY</w:t><w:footnoteReference w:id="1"/>{}</w:r></w:p><w:sectPr><w:pgSz w:w="6000" w:h="4800"/><w:pgMar w:top="400" w:bottom="400" w:left="400" w:right="400"/></w:sectPr>"#,
        if two {
            r#"<w:footnoteReference w:id="2"/>"#
        } else {
            ""
        }
    );
    let mut notes = format!(
        r#"<w:footnotes xmlns:w="{W}"><w:footnote w:type="separator" w:id="-1"><w:p><w:pPr><w:spacing w:line="120" w:lineRule="exact"/></w:pPr><w:r><w:separator/></w:r></w:p></w:footnote>"#
    );
    for id in 1..=if two { 2 } else { 1 } {
        notes.push_str(&format!(r#"<w:footnote w:id="{id}">"#));
        for i in 0..count {
            notes.push_str(&format!(r#"<w:p><w:pPr><w:widowControl w:val="0"/><w:spacing w:before="0" w:after="0" w:line="240" w:lineRule="exact"/></w:pPr><w:r><w:t>NOTE{id}_{i:03}</w:t></w:r></w:p>"#));
        }
        notes.push_str("</w:footnote>");
    }
    notes.push_str("</w:footnotes>");
    with_related_part(
        docx(&body, None, None),
        "footnotes",
        "footnotes.xml",
        &notes,
        false,
    )
}

#[test]
fn long_note_drains_tail_pages_with_all_lines_inside_body_bounds() {
    let out = pages(source(84, false));
    assert!(out.len() > 1 && out.len() < 10);
    let mut found = Vec::new();
    for page in &out {
        for item in &page.items {
            if let Item::Text(r) = item
                && r.text.starts_with("NOTE")
            {
                found.push(r.text.clone());
                assert!(r.transform.ty as f64 >= page.info.top - EPS);
                assert!(r.transform.ty as f64 <= page.info.bottom + EPS);
            }
        }
    }
    assert_eq!(
        found,
        (0..84).map(|i| format!("NOTE1_{i:03}")).collect::<Vec<_>>()
    );
    assert_eq!(
        out.iter()
            .flat_map(|p| &p.items)
            .filter(|i| matches!(i,Item::Text(r)if r.text.starts_with("BODY")))
            .count(),
        1
    );
}

#[test]
fn two_long_notes_on_one_reference_line_keep_queue_order_and_content() {
    let out = pages(source(40, true));
    assert!(out.len() < 10);
    let found = out
        .iter()
        .flat_map(|p| &p.items)
        .filter_map(|i| match i {
            Item::Text(r) if r.text.starts_with("NOTE") => Some(r.text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        found,
        (1..=2)
            .flat_map(|id| (0..40).map(move |i| format!("NOTE{id}_{i:03}")))
            .collect::<Vec<_>>()
    );
}

fn authored(body: &str, notes: &str) -> Vec<u8> {
    with_related_part(
        docx(body, None, None),
        "footnotes",
        "footnotes.xml",
        &format!(r#"<w:footnotes xmlns:w="{W}">{notes}</w:footnotes>"#),
        false,
    )
}
fn line(label: &str) -> String {
    format!(
        r#"<w:p><w:pPr><w:widowControl w:val="0"/><w:spacing w:before="0" w:after="0" w:line="240" w:lineRule="exact"/></w:pPr><w:r><w:t>{label}</w:t></w:r></w:p>"#
    )
}
fn section(columns: usize) -> String {
    format!(
        r#"<w:sectPr><w:pgSz w:w="6000" w:h="4800"/><w:pgMar w:top="400" w:bottom="400" w:left="400" w:right="400"/><w:cols w:num="{columns}" w:space="200"/></w:sectPr>"#
    )
}
fn reference(id: usize) -> String {
    format!(
        r#"<w:p><w:pPr><w:spacing w:after="0" w:line="240" w:lineRule="exact"/></w:pPr><w:r><w:t>BODY{id}</w:t><w:footnoteReference w:id="{id}"/></w:r></w:p>"#
    )
}
fn note(id: usize, content: &str) -> String {
    format!(r#"<w:footnote w:id="{id}">{content}</w:footnote>"#)
}
fn text(out: &[OutPage]) -> String {
    out.iter()
        .flat_map(|p| &p.items)
        .filter_map(|i| match i {
            Item::Text(r) => Some(r.text.as_str()),
            _ => None,
        })
        .collect()
}

#[test]
fn ordinary_note_table_continues_without_losing_cell_text_or_tail_spacing() {
    let content = (0..40)
        .map(|i| line(&format!("CELL{i:03}")))
        .collect::<String>();
    let table = format!(
        r#"<w:tbl><w:tblGrid><w:gridCol w:w="4000"/></w:tblGrid><w:tr><w:tc><w:tcPr><w:tcMar><w:top w:w="80"/><w:bottom w:w="80"/></w:tcMar></w:tcPr>{content}</w:tc></w:tr></w:tbl>{}"#,
        line("TAIL")
    );
    let out = pages(authored(&(reference(1) + &section(1)), &note(1, &table)));
    assert!(out.len() > 1 && out.len() < 10);
    let text = text(&out);
    for i in 0..40 {
        assert_eq!(text.matches(&format!("CELL{i:03}")).count(), 1);
    }
    assert_eq!(text.matches("TAIL").count(), 1);
    assert!(text.find("CELL039").unwrap() < text.find("TAIL").unwrap());
}

#[test]
fn incoming_note_survives_transition_to_two_body_columns() {
    let notes = note(
        1,
        &(0..40)
            .map(|i| line(&format!("CARRY{i:03}")))
            .collect::<String>(),
    );
    let first = section(1).replace("<w:sectPr>", "<w:sectPr><w:type w:val=\"nextPage\"/>");
    let body = reference(1)
        + &format!("<w:p><w:pPr>{first}</w:pPr><w:r><w:t>SECTIONEND</w:t></w:r></w:p>")
        + &line("AFTERSECTION")
        + &section(2);
    let out = pages(authored(&body, &notes));
    let all = text(&out);
    for i in 0..40 {
        assert_eq!(all.matches(&format!("CARRY{i:03}")).count(), 1);
    }
    assert_eq!(all.matches("AFTERSECTION").count(), 1);
    assert!(out.len() < 10);
}

#[test]
fn later_unsupported_note_cannot_disappear_behind_an_active_fragment_plan() {
    let first = note(
        1,
        &(0..40)
            .map(|i| line(&format!("FIRST{i:03}")))
            .collect::<String>(),
    );
    let nested = format!(
        r#"<w:tbl><w:tblGrid><w:gridCol w:w="4000"/></w:tblGrid><w:tr><w:tc><w:tbl><w:tblGrid><w:gridCol w:w="3000"/></w:tblGrid><w:tr><w:tc>{}{}</w:tc></w:tr></w:tbl></w:tc></w:tr></w:tbl>"#,
        line("NESTEDA"),
        line("NESTEDB")
    );
    let out = pages(authored(
        &(reference(1) + &reference(2) + &section(1)),
        &(first + &note(2, &nested)),
    ));
    let all = text(&out);
    for i in 0..40 {
        assert_eq!(all.matches(&format!("FIRST{i:03}")).count(), 1);
    }
    assert_eq!(all.matches("NESTEDA").count(), 1);
    assert_eq!(all.matches("NESTEDB").count(), 1);
    assert_eq!(all.matches("BODY2").count(), 1);
    assert!(out.len() < 10);
}

#[test]
fn committed_plan_and_carry_rollback_with_body_trials() {
    super::super::tests::with_column_pager(source(84, false), |pager, sections| {
        let top = pager.pages[0].top;
        pager.commit_note_body(&[1], top + 12.0);
        let accepted = pager.st.note_pages[0].clone();
        assert!(accepted.active);
        assert!(!accepted.outgoing.is_empty());
        let snap = pager.snapshot();
        let trial = pager
            .plan_page_notes(&pager.pages[0], &[], top + 80.0)
            .unwrap();
        assert!(trial.is_err());
        assert!(std::rc::Rc::ptr_eq(&accepted, &pager.st.note_pages[0]));
        pager.new_page(0, &sections[0].props, sections);
        assert_eq!(pager.st.note_pages.len(), 2);
        pager.restore(&snap);
        assert_eq!(pager.st.note_pages.len(), 1);
        assert!(std::rc::Rc::ptr_eq(&accepted, &pager.st.note_pages[0]));
        assert_eq!(pager.pages[0].notes, vec![1]);
    });
}

#[test]
fn completed_note_manual_boundary_moves_the_next_note() {
    let first = line("FIRST").replace("</w:t>", "</w:t><w:br w:type=\"page\"/>");
    let body =
        reference(1).replace("</w:r>", "<w:footnoteReference w:id=\"2\"/></w:r>") + &section(1);
    let out = pages(authored(
        &body,
        &(note(1, &first) + &note(2, &line("SECOND"))),
    ));
    let locate = |needle: &str| {
        out.iter()
            .position(|p| {
                p.items
                    .iter()
                    .any(|i| matches!(i,Item::Text(r)if r.text==needle))
            })
            .unwrap()
    };
    assert!(locate("SECOND") > locate("FIRST"));
    assert_eq!(text(&out).matches("FIRST").count(), 1);
    assert_eq!(text(&out).matches("SECOND").count(), 1);
}

#[test]
fn remaining_cursor_fallback_does_not_restart_the_consumed_prefix() {
    super::super::tests::with_column_pager(source(10, false), |pager, _| {
        let context = pager.current_note_context().0;
        let layout = pager.note_layout_in(&context, 1);
        let prefix = plan_fragment(
            &layout,
            &layout.start(),
            FragmentBudget {
                height: 24.0,
                fresh_capacity: 24.0,
                fresh: true,
                allow_atomic_overflow: false,
            },
        )
        .unwrap();
        assert_eq!(prefix.after.unit, 2);
        let fallback = pager.remaining_note_fallback(vec![PendingNote {
            reference: NoteReference { id: 1, context },
            layout,
            cursor: prefix.after,
            first_page: Some(0),
            special: false,
            suppress_separator: false,
        }]);
        *pager.st.note_pages.last_mut().unwrap() = std::rc::Rc::new(fallback);
        let mut page = OutPage {
            info: pager.pages[0].clone(),
            items: Vec::new(),
        };
        assert!(pager.draw_note_fragments(&mut page, 0));
        let all = text(&[page]);
        assert!(!all.contains("NOTE1_000"));
        assert!(!all.contains("NOTE1_001"));
        for i in 2..10 {
            assert_eq!(all.matches(&format!("NOTE1_{i:03}")).count(), 1);
        }
    });
}

#[test]
fn parity_padding_pages_forward_incoming_notes_without_painting_duplicates() {
    for (kind, start) in [("oddPage", 1), ("evenPage", 2)] {
        let first = section(1).replace(
            "</w:sectPr>",
            &format!(r#"<w:pgNumType w:start="{start}"/></w:sectPr>"#),
        );
        let last = section(1).replace(
            "<w:sectPr>",
            &format!(r#"<w:sectPr><w:type w:val="{kind}"/>"#),
        );
        let body =
            reference(1).replace("</w:pPr>", &(first + "</w:pPr>")) + &line("NEXTSECTION") + &last;
        let notes = note(
            1,
            &(0..84)
                .map(|i| line(&format!("PARITY{i:03}")))
                .collect::<String>(),
        );
        let out = pages(authored(&body, &notes));
        let blanks = out.iter().filter(|p| p.info.blank).collect::<Vec<_>>();
        assert_eq!(blanks.len(), 1, "{kind}");
        assert!(
            blanks[0]
                .items
                .iter()
                .all(|i| !matches!(i,Item::Text(r)if r.text.contains("PARITY")))
        );
        let found = out
            .iter()
            .flat_map(|p| &p.items)
            .filter_map(|i| match i {
                Item::Text(r) if r.text.starts_with("PARITY") => Some(r.text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            found,
            (0..84).map(|i| format!("PARITY{i:03}")).collect::<Vec<_>>(),
            "{kind}"
        );
        assert_eq!(text(&out).matches("NEXTSECTION").count(), 1);
    }
}

#[test]
fn later_body_lines_preserve_a_previously_accepted_complete_note() {
    let body = reference(1)
        + &(0..24)
            .map(|i| line(&format!("LATER{i:03}")))
            .collect::<String>()
        + &section(1);
    let notes = note(
        1,
        &(0..6)
            .map(|i| line(&format!("RESERVED{i:03}")))
            .collect::<String>(),
    );
    let out = pages(authored(&body, &notes));
    let first = out[0]
        .items
        .iter()
        .filter_map(|i| match i {
            Item::Text(r) if r.text.starts_with("RESERVED") => Some(r.text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        first,
        (0..6)
            .map(|i| format!("RESERVED{i:03}"))
            .collect::<Vec<_>>()
    );
    let all = text(&out);
    for i in 0..24 {
        assert_eq!(all.matches(&format!("LATER{i:03}")).count(), 1);
    }
    for i in 0..6 {
        assert_eq!(all.matches(&format!("RESERVED{i:03}")).count(), 1);
    }
}

#[test]
fn accepted_paragraph_prefix_commits_one_note_plan_after_late_references() {
    for filler in [4, 6, 8, 10] {
        let mut body = (1..=4).map(reference).collect::<String>();
        body += &(0..filler)
            .map(|i| line(&format!("FILL{i:03}")))
            .collect::<String>();
        let runs = (0..10)
            .map(|i| {
                let reference = match i {
                    3 => "<w:footnoteReference w:id=\"5\"/>",
                    6 => "<w:footnoteReference w:id=\"6\"/>",
                    _ => "",
                };
                format!(
                    "<w:r><w:t>LATE{i:03}</w:t>{reference}{}</w:r>",
                    if i < 9 { "<w:br/>" } else { "" }
                )
            })
            .collect::<String>();
        body += &line("").replace("<w:r><w:t></w:t></w:r>", &runs);
        body += &section(1)
            .replace("6000", "8000")
            .replace("4800", "10000")
            .replace("400", "800");
        let notes = (1..=6)
            .map(|id| {
                note(
                    id,
                    &(0..3)
                        .map(|i| line(&format!("NOTE{id}_{i:03}")))
                        .collect::<String>(),
                )
            })
            .collect::<String>();
        let out = pages(authored(&body, &notes));
        let all = text(&out);
        for i in 0..10 {
            assert_eq!(all.matches(&format!("LATE{i:03}")).count(), 1);
        }
        for id in 1..=6 {
            for i in 0..3 {
                assert_eq!(all.matches(&format!("NOTE{id}_{i:03}")).count(), 1);
            }
        }
        for page in &out {
            let mut body_bottom = f64::NEG_INFINITY;
            let mut note_top = f64::INFINITY;
            for item in &page.items {
                if let Item::Text(run) = item {
                    if run.text.starts_with("NOTE") {
                        note_top = note_top.min(run.transform.ty as f64);
                    }
                    if ["BODY", "FILL", "LATE"]
                        .iter()
                        .any(|prefix| run.text.starts_with(prefix))
                    {
                        body_bottom = body_bottom.max(run.transform.ty as f64 + 10.0);
                    }
                }
            }
            assert!(
                body_bottom <= note_top + EPS,
                "filler {filler}: body {body_bottom} note {note_top}"
            );
        }
    }
}

#[test]
fn incoming_fragment_reserves_space_against_trial_pictures_without_new_references() {
    super::super::tests::with_column_pager(source(84, false), |pager, sections| {
        let top = pager.pages[0].top;
        pager.commit_note_body(&[1], top + 12.0);
        pager.new_page(0, &sections[0].props, sections);
        let page = pager.pages.last().unwrap();
        assert!(page.notes.is_empty());
        assert!(page.note_carry.is_none());
        assert!(!pager.current_note_page().incoming.is_empty());
        let limit = pager.note_limit(page, &[]);
        let (left, width) = page.cols[0];
        let picture = (page.top, limit + 12.0, left, left + width);
        assert!(pager.picture_note_conflict(&[picture], &[]));
        assert!(!pager.picture_note_conflict(&[], &[]));
        let before = pager.st.note_pages.last().unwrap().clone();
        assert!(pager.picture_note_conflict(&[picture], &[]));
        assert!(std::rc::Rc::ptr_eq(
            &before,
            pager.st.note_pages.last().unwrap()
        ));
    });
}

#[test]
fn new_short_note_starts_on_its_reference_page() {
    for filler in [8, 10] {
        let mut body = (1..=4).map(reference).collect::<String>();
        body += &(0..filler)
            .map(|i| line(&format!("FILL{i:03}")))
            .collect::<String>();
        let runs = (0..10)
            .map(|i| {
                let reference = match i {
                    6 => "<w:footnoteReference w:id=\"5\"/>",
                    _ => "",
                };
                format!(
                    "<w:r><w:t>LATE{i:03}</w:t>{reference}{}</w:r>",
                    if i < 9 { "<w:br/>" } else { "" }
                )
            })
            .collect::<String>();
        body += &line("").replace("<w:r><w:t></w:t></w:r>", &runs);
        body += &section(1)
            .replace("6000", "8000")
            .replace("4800", "10000")
            .replace("400", "800");
        let notes = (1..=5)
            .map(|id| {
                note(
                    id,
                    &(0..if id == 5 { 1 } else { 3 })
                        .map(|i| line(&format!("NOTE{id}_{i:03}")))
                        .collect::<String>(),
                )
            })
            .collect::<String>();
        let out = pages(authored(&body, &notes));
        let locate = |label: &str| {
            out.iter()
                .position(|page| {
                    page.items
                        .iter()
                        .any(|item| matches!(item, Item::Text(run) if run.text.contains(label)))
                })
                .unwrap()
        };
        assert_eq!(locate("LATE006"), locate("NOTE5_000"), "filler {filler}");
        let all = text(&out);
        for i in 0..10 {
            assert_eq!(all.matches(&format!("LATE{i:03}")).count(), 1);
        }
        for id in 1..=5 {
            for i in 0..if id == 5 { 1 } else { 3 } {
                assert_eq!(all.matches(&format!("NOTE{id}_{i:03}")).count(), 1);
            }
        }
        for page in &out {
            let mut body_bottom = f64::NEG_INFINITY;
            let mut note_top = f64::INFINITY;
            for item in &page.items {
                if let Item::Text(run) = item {
                    if run.text.starts_with("NOTE") {
                        note_top = note_top.min(run.transform.ty as f64);
                    }
                    if ["BODY", "FILL", "LATE"]
                        .iter()
                        .any(|prefix| run.text.starts_with(prefix))
                    {
                        body_bottom = body_bottom.max(run.transform.ty as f64 + 10.0);
                    }
                }
            }
            assert!(
                body_bottom <= note_top + EPS,
                "filler {filler}: body {body_bottom} note {note_top}"
            );
        }
    }
}

#[test]
fn manual_boundary_inside_note_preserves_later_reference_queue() {
    let first = line("FIRST").replace("</w:t>", "</w:t><w:br w:type=\"page\"/><w:t>TAIL</w:t>");
    let body =
        reference(1).replace("</w:r>", "<w:footnoteReference w:id=\"2\"/></w:r>") + &section(1);
    let out = pages(authored(
        &body,
        &(note(1, &first) + &note(2, &line("SECOND"))),
    ));
    assert!(out.len() >= 2 && out.len() < 5);
    let all = text(&out);
    for label in ["FIRST", "TAIL", "SECOND"] {
        assert_eq!(all.matches(label).count(), 1);
    }
    let locate = |label: &str| {
        out.iter()
            .position(|page| {
                page.items
                    .iter()
                    .any(|item| matches!(item, Item::Text(run) if run.text.contains(label)))
            })
            .unwrap()
    };
    assert!(locate("FIRST") < locate("TAIL"));
    assert!(locate("TAIL") <= locate("SECOND"));
}

fn column_source(count: usize, columns: usize, origin: usize) -> Vec<u8> {
    let transitions = (0..origin)
        .map(|_| r#"<w:p><w:r><w:br w:type="column"/></w:r></w:p>"#)
        .collect::<String>();
    let body = format!(
        r#"{transitions}<w:p><w:pPr><w:spacing w:before="0" w:after="0" w:line="240" w:lineRule="exact"/></w:pPr><w:r><w:t>REFERENCE</w:t><w:footnoteReference w:id="1"/></w:r></w:p><w:sectPr><w:pgSz w:w="12000" w:h="4800"/><w:pgMar w:top="400" w:bottom="400" w:left="400" w:right="400"/><w:cols w:num="{columns}" w:space="400"/></w:sectPr>"#
    );
    let lines = (0..count).map(|i| format!(r#"<w:p><w:pPr><w:widowControl w:val="0"/><w:spacing w:before="0" w:after="0" w:line="240" w:lineRule="exact"/></w:pPr><w:r><w:t>AREA{i:03}</w:t></w:r></w:p>"#)).collect::<String>();
    let notes = format!(
        r#"<w:footnotes xmlns:w="{W}"><w:footnote w:id="1">{lines}</w:footnote></w:footnotes>"#
    );
    with_related_part(
        docx(&body, None, None),
        "footnotes",
        "footnotes.xml",
        &notes,
        false,
    )
}

#[test]
fn equal_columns_route_each_note_from_its_reference_area_in_order() {
    for columns in [2, 3] {
        for origin in 0..columns {
            let out = pages(column_source(84, columns, origin));
            let mut found = Vec::new();
            let mut destinations = Vec::new();
            for (page_index, page) in out.iter().enumerate() {
                for item in &page.items {
                    if let Item::Text(run) = item
                        && run.text.starts_with("AREA")
                    {
                        let x = run.transform.tx as f64;
                        let y = run.transform.ty as f64;
                        let column = page
                            .info
                            .cols
                            .iter()
                            .position(|(left, width)| x >= *left - EPS && x < left + width)
                            .unwrap();
                        assert!(y >= page.info.top - EPS && y <= page.info.bottom + EPS);
                        found.push(run.text.clone());
                        destinations.push((page_index, column));
                    }
                }
            }
            assert_eq!(
                found,
                (0..84).map(|i| format!("AREA{i:03}")).collect::<Vec<_>>()
            );
            assert_eq!(destinations[0], (0, origin));
            assert!(destinations.windows(2).all(|w| w[0] <= w[1]));
            assert!(destinations.contains(&(1, 0)));
            if origin + 1 < columns {
                assert!(destinations.contains(&(0, origin + 1)));
            }
        }
    }
}

#[test]
fn destination_reservations_filter_picture_spans_and_preserve_closed_areas() {
    super::super::tests::with_column_pager(column_source(84, 2, 1), |pager, _| {
        pager.st.col = 1;
        let top = pager.pages[0].top;
        pager.commit_note_body(&[1], top + 12.0);
        let page = &pager.pages[0];
        let left = page.cols[0];
        let right = page.cols[1];
        let limit = pager.note_limit(page, &[]);
        let rectangle = |span: (f64, f64)| (top, limit + 12.0, span.0, span.0 + span.1);
        assert!(!pager.picture_note_conflict(&[rectangle(left)], &[]));
        assert!(pager.picture_note_conflict(&[rectangle(right)], &[]));
        assert!(
            pager.picture_note_conflict(&[(top, limit + 12.0, left.0, right.0 + right.1)], &[])
        );
        assert!(pager.current_note_page().areas[0].pieces.is_empty());
        assert_eq!(pager.current_note_page().height_at(left.0, left.1), 0.0);
    });
    super::super::tests::with_column_pager(column_source(20, 2, 0), |pager, sections| {
        let top = pager.pages[0].top;
        pager.commit_note_body(&[1], top + 12.0);
        let initial = pager.st.note_pages[0].clone();
        pager.next_column(0, &sections[0].props, sections);
        pager.commit_note_body(&[], top + 12.0);
        let accepted = &pager.current_note_page().areas[0];
        assert_eq!(accepted.height, initial.areas[0].height);
        assert_eq!(accepted.pieces.len(), initial.areas[0].pieces.len());
        assert_eq!(
            accepted.outgoing[0].cursor.unit,
            initial.areas[0].outgoing[0].cursor.unit
        );
        assert_eq!(accepted.body_bottom, initial.areas[0].body_bottom);
    });
}

#[test]
fn authored_note_page_and_column_boundaries_keep_distinct_destinations() {
    for kind in ["page", "column"] {
        let body = reference(1) + &section(2);
        let note_body = line("FIRSTBOUNDARY")
            .replace("</w:r>", &format!(r#"<w:br w:type="{kind}"/></w:r>"#))
            + &line("SECONDBOUNDARY");
        let out = pages(authored(&body, &note(1, &note_body)));
        let locate = |needle: &str| {
            out.iter()
                .enumerate()
                .find_map(|(index, page)| {
                    page.items.iter().find_map(|item| match item {
                        Item::Text(run) if run.text == needle => Some((index, run.transform.tx)),
                        _ => None,
                    })
                })
                .unwrap()
        };
        let first = locate("FIRSTBOUNDARY");
        let second = locate("SECONDBOUNDARY");
        if kind == "page" {
            assert!(second.0 > first.0);
        } else {
            assert_eq!(second.0, first.0);
            assert!(second.1 > first.1);
        }
    }
}

#[test]
fn blocked_future_note_area_forwards_the_unconsumed_cursor_without_a_separator() {
    super::super::tests::with_column_pager(column_source(38, 2, 0), |pager, _| {
        let page = &pager.pages[0];
        let right = page.cols[1];
        let picture = (page.top, page.bottom, right.0, right.0 + right.1);
        let planned = pager
            .plan_page_notes_with_pictures(page, &[1], page.top + 12.0, &[picture])
            .unwrap()
            .unwrap();
        assert!(!planned.areas[0].pieces.is_empty());
        assert!(planned.areas[1].pieces.is_empty());
        assert_eq!(planned.areas[1].height, 0.0);
        assert_eq!(planned.outgoing.len(), 1);
        assert!(planned.outgoing[0].cursor.unit > 0);
        *pager.st.note_pages.last_mut().unwrap() = std::rc::Rc::new(planned);
        let mut output = OutPage {
            info: pager.pages[0].clone(),
            items: Vec::new(),
        };
        assert!(pager.draw_note_fragments(&mut output, 0));
        assert!(output.items.iter().all(|item| match item {
            Item::Text(run) => (run.transform.tx as f64) < right.0,
            _ => true,
        }));
    });
}

#[test]
fn active_continuous_areas_keep_global_reference_queue_order() {
    let first = section(2);
    let second = section(2).replace("<w:sectPr>", r#"<w:sectPr><w:type w:val="continuous"/>"#);
    let body = reference(1).replace("</w:r>", r#"<w:br w:type="column"/></w:r>"#)
        + &reference(2)
        + &format!("<w:p><w:pPr>{first}</w:pPr></w:p>")
        + &reference(3)
        + &second;
    let notes = (1..=3)
        .map(|id| {
            note(
                id,
                &(0..8)
                    .map(|i| line(&format!("ORDER{id}_{i:03}")))
                    .collect::<String>(),
            )
        })
        .collect::<String>();
    let out = pages(authored(&body, &notes));
    let found = out
        .iter()
        .flat_map(|p| &p.items)
        .filter_map(|item| match item {
            Item::Text(run) if run.text.starts_with("ORDER") => Some(run.text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        found,
        (1..=3)
            .flat_map(|id| (0..8).map(move |i| format!("ORDER{id}_{i:03}")))
            .collect::<Vec<_>>()
    );
}

#[test]
fn note_page_break_carry_survives_closed_intermediate_body_columns() {
    let shift = |name| line(name).replace("<w:t>", r#"<w:br w:type="column"/><w:t>"#);
    let body = reference(1) + &shift("MIDDLEBODY") + &shift("RIGHTBODY") + &section(3);
    let notes =
        line("FIRSTPAGE").replace("</w:r>", r#"<w:br w:type="page"/></w:r>"#) + &line("NEXTPAGE");
    let out = pages(authored(&body, &note(1, &notes)));
    let locate = |needle: &str| {
        out.iter()
            .position(|page| {
                page.items
                    .iter()
                    .any(|item| matches!(item,Item::Text(run) if run.text==needle))
            })
            .unwrap()
    };
    assert_eq!(locate("MIDDLEBODY"), 0);
    assert_eq!(locate("RIGHTBODY"), 0);
    assert!(locate("NEXTPAGE") > locate("FIRSTPAGE"));
    assert_eq!(text(&out).matches("NEXTPAGE").count(), 1);
}

#[test]
fn merged_note_text_is_backfilled_into_its_physical_fragments_once() {
    let paragraphs = (0..38).map(|i| format!(r#"<w:p><w:pPr><w:widowControl w:val="0"/><w:spacing w:before="0" w:after="0" w:line="240" w:lineRule="exact"/></w:pPr><w:r><w:t>MRG{i:04}</w:t></w:r></w:p>"#)).collect::<String>();
    let mut rows = String::new();
    for i in 0..4 {
        rows.push_str(&format!(r#"<w:tr><w:tc><w:tcPr><w:vMerge {}/></w:tcPr>{}</w:tc><w:tc><w:p><w:r><w:t>SIB{i:04}</w:t></w:r></w:p></w:tc></w:tr>"#,
            if i == 0 { "w:val=\"restart\"" } else { "" },
            if i == 0 { paragraphs.as_str() } else { "<w:p/>" }));
    }
    let note = format!(
        r#"<w:footnotes xmlns:w="{W}"><w:footnote w:id="1"><w:tbl><w:tblGrid><w:gridCol w:w="2000"/><w:gridCol w:w="2000"/></w:tblGrid>{rows}</w:tbl></w:footnote></w:footnotes>"#
    );
    let body = r#"<w:p><w:r><w:t>BODY</w:t><w:footnoteReference w:id="1"/></w:r></w:p><w:sectPr><w:pgSz w:w="6000" w:h="4800"/><w:pgMar w:top="400" w:bottom="400" w:left="400" w:right="400"/></w:sectPr>"#;
    let out = pages(with_related_part(
        docx(body, None, None),
        "footnotes",
        "footnotes.xml",
        &note,
        false,
    ));
    let mut labels = Vec::new();
    for page in &out {
        for item in &page.items {
            if let Item::Text(t) = item
                && t.text.starts_with("MRG")
            {
                assert!(t.transform.ty as f64 >= page.info.top - EPS);
                assert!(t.transform.ty as f64 <= page.info.bottom + EPS);
                labels.push(t.text.clone());
            }
        }
    }
    assert_eq!(
        labels,
        (0..38).map(|i| format!("MRG{i:04}")).collect::<Vec<_>>()
    );
    let keys: Vec<(&str, usize)> = out[0]
        .items
        .iter()
        .filter_map(|i| match i {
            Item::Text(t) => Some((t.text.as_ref(), t.paragraph)),
            _ => None,
        })
        .collect::<Vec<_>>();
    let merged = keys
        .iter()
        .find(|(text, _)| text.starts_with("MRG0000"))
        .unwrap()
        .1;
    let sibling = keys
        .iter()
        .find(|(text, _)| text.starts_with("SIB0000"))
        .unwrap()
        .1;
    assert!(merged < sibling);
    assert!(out.len() > 1 && out.len() < 10);
}

#[test]
fn unequal_note_columns_retain_source_in_actual_destination_rectangles() {
    for single_paragraph in [false, true] {
        let body = r#"<w:p><w:r><w:t>BODY</w:t><w:footnoteReference w:id="1"/></w:r></w:p><w:sectPr><w:pgSz w:w="5600" w:h="4800"/><w:pgMar w:top="400" w:bottom="400" w:left="400" w:right="400"/><w:cols w:num="2" w:equalWidth="0"><w:col w:w="1600" w:space="400"/><w:col w:w="2800"/></w:cols></w:sectPr>"#;
        let lines = (0..55).map(|i| format!(r#"<w:p><w:pPr><w:spacing w:before="0" w:after="0" w:line="240" w:lineRule="exact"/></w:pPr><w:r><w:t>NOTE1_{i:03}</w:t></w:r></w:p>"#)).collect::<String>();
        let lines = if single_paragraph {
            let text = (0..120)
                .map(|i| format!("NOTE1_{i:03} "))
                .collect::<String>();
            format!(
                r#"<w:p><w:pPr><w:spacing w:before="0" w:after="0" w:line="240" w:lineRule="exact"/></w:pPr><w:r><w:t>{text}</w:t></w:r></w:p>"#
            )
        } else {
            lines
        };
        let notes = format!(
            r#"<w:footnotes xmlns:w="{W}"><w:footnote w:id="1">{lines}</w:footnote></w:footnotes>"#
        );
        let bytes = with_related_part(
            docx(body, None, None),
            "footnotes",
            "footnotes.xml",
            &notes,
            false,
        );
        let out = pages(bytes);
        let found = out
            .iter()
            .flat_map(|p| &p.items)
            .filter_map(|item| match item {
                Item::Text(run) if run.text.starts_with("NOTE1_") => Some(run.text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        let tokens = found
            .iter()
            .flat_map(|s| s.split_whitespace())
            .map(str::to_string)
            .collect::<Vec<_>>();
        let count = if single_paragraph { 120 } else { 55 };
        assert_eq!(
            tokens,
            (0..count)
                .map(|i| format!("NOTE1_{i:03}"))
                .collect::<Vec<_>>()
        );
        assert!(out.len() > 1 && out.len() < 8);
        for page in &out {
            for item in &page.items {
                if let Item::Text(run) = item
                    && run.text.starts_with("NOTE1_")
                {
                    assert!(run.transform.ty as f64 >= page.info.top - EPS);
                    assert!(run.transform.ty as f64 <= page.info.bottom + EPS);
                    assert!(page.info.cols.iter().any(|(x, width)| {
                        let left = run.transform.tx as f64;
                        left >= *x - EPS && left + run.width <= x + width + EPS
                    }));
                }
            }
        }
    }
}

#[test]
fn continued_note_paragraph_anchor_uses_destination_fragment_origin() {
    for (count, owner) in [(55, 20), (5, 2)] {
        let body = r#"<w:p><w:r><w:t>BODY</w:t><w:footnoteReference w:id="1"/></w:r></w:p><w:sectPr><w:pgSz w:w="5600" w:h="4800"/><w:pgMar w:top="400" w:bottom="400" w:left="400" w:right="400"/><w:cols w:num="2" w:equalWidth="0"><w:col w:w="1600" w:space="400"/><w:col w:w="2800"/></w:cols></w:sectPr>"#;
        let mut runs = String::new();
        for i in 0..count {
            runs.push_str(&format!(r#"<w:r><w:t>N{i:03}</w:t>"#));
            if i == owner {
                runs.push_str(r##"<w:pict xmlns:v="urn:schemas-microsoft-com:vml"><v:shape id="late" type="#_x0000_t202" style="position:absolute;margin-left:20pt;margin-top:0pt;width:55pt;height:12pt"><v:textbox inset="0,0,0,0"><w:txbxContent><w:p><w:r><w:rPr><w:sz w:val="14"/></w:rPr><w:t>ANCHOR</w:t></w:r></w:p></w:txbxContent></v:textbox></v:shape></w:pict><w:t>TAIL</w:t>"##);
            }
            runs.push_str("<w:br/></w:r>");
        }
        let notes = format!(
            r#"<w:footnotes xmlns:w="{W}"><w:footnote w:id="1"><w:p><w:pPr><w:widowControl w:val="0"/><w:spacing w:before="0" w:after="0" w:line="240" w:lineRule="exact"/></w:pPr>{runs}</w:p></w:footnote></w:footnotes>"#
        );
        let out = pages(with_related_part(
            docx(body, None, None),
            "footnotes",
            "footnotes.xml",
            &notes,
            false,
        ));
        let anchors: Vec<_> = out
            .iter()
            .flat_map(|page| {
                page.items.iter().filter_map(move |item| match item {
                    Item::Text(t) if t.text == "ANCHOR" => Some((page, t)),
                    _ => None,
                })
            })
            .collect();
        assert_eq!(anchors.len(), 1);
        let (page, anchor) = anchors[0];
        assert!(anchor.transform.ty as f64 >= page.info.top - EPS);
        assert!(anchor.transform.ty as f64 <= page.info.bottom + EPS);
        assert!(page.items.iter().any(
        |i| matches!(i, Item::Text(t) if t.text == format!("N{owner:03}") && t.transform.ty >= anchor.transform.ty)
    ));
        let mut ordered: Vec<_> = page
            .items
            .iter()
            .filter_map(|i| match i {
                Item::Text(t) => Some((t.paragraph, t.text.as_str())),
                _ => None,
            })
            .collect();
        ordered.sort_by_key(|(key, _)| *key);
        let text: String = ordered.into_iter().map(|(_, text)| text).collect();
        assert!(text.contains(&format!("N{owner:03}ANCHORTAIL")), "{text}");
    }
}

#[test]
fn retained_note_rows_interleave_same_line_anchor_source_slots() {
    for nested in [false, true] {
        let body =
            line("BODY").replace("</w:r>", "<w:footnoteReference w:id=\"1\"/></w:r>") + &section(1);
        let anchor = r##"<w:pict xmlns:v="urn:schemas-microsoft-com:vml"><v:shape id="inline-order" type="#_x0000_t202" style="position:absolute;margin-left:20pt;margin-top:0pt;width:55pt;height:12pt;mso-position-vertical-relative:line"><v:textbox inset="0,0,0,0"><w:txbxContent><w:p><w:r><w:t>BOX</w:t></w:r></w:p></w:txbxContent></v:textbox></v:shape></w:pict>"##;
        let mut content = format!(
            r#"<w:p><w:pPr><w:spacing w:before="0" w:after="0" w:line="240" w:lineRule="exact"/></w:pPr><w:r><w:t>OWNER</w:t>{anchor}<w:t>TAIL</w:t></w:r></w:p>"#
        );
        content += &(0..45)
            .map(|i| line(&format!("ROW{i:03}")))
            .collect::<String>();
        let table = |s: &str| {
            format!(
                r#"<w:tbl><w:tblPr><w:tblW w:w="4000" w:type="dxa"/></w:tblPr><w:tblGrid><w:gridCol w:w="4000"/></w:tblGrid><w:tr><w:tc>{s}</w:tc></w:tr></w:tbl>"#
            )
        };
        let content = table(&content);
        let content = if nested { table(&content) } else { content };
        let notes = format!(
            r#"<w:footnotes xmlns:w="{W}"><w:footnote w:id="1">{content}</w:footnote></w:footnotes>"#
        );
        let out = pages(with_related_part(
            docx(&body, None, None),
            "footnotes",
            "footnotes.xml",
            &notes,
            false,
        ));
        assert!(out.len() > 1);
        let mut text = String::new();
        for page in out {
            let mut runs: Vec<_> = page
                .items
                .iter()
                .filter_map(|i| match i {
                    Item::Text(t) => Some(t),
                    _ => None,
                })
                .collect();
            runs.sort_by_key(|r| r.paragraph);
            text.extend(runs.iter().map(|r| r.text.as_str()));
        }
        assert!(text.contains("OWNERBOXTAIL"), "{text}");
        assert_eq!(text.matches("BOX").count(), 1);
    }
}

#[test]
fn inline_textbox_keeps_source_position_between_floating_anchor_and_tail() {
    for (note, long) in [(false, false), (true, false), (true, true)] {
        let shape = |id: &str, floating: bool| {
            format!(
                r##"<w:pict xmlns:v="urn:schemas-microsoft-com:vml"><v:shape id="{id}" type="#_x0000_t202" style="{}width:35pt;height:12pt"><v:textbox inset="0,0,0,0"><w:txbxContent><w:p><w:r><w:rPr><w:sz w:val="14"/></w:rPr><w:t>{id}</w:t></w:r></w:p></w:txbxContent></v:textbox></v:shape></w:pict>"##,
                if floating {
                    "position:absolute;margin-left:0pt;margin-top:0pt;"
                } else {
                    ""
                }
            )
        };
        let content = format!(
            r#"<w:p><w:r><w:t>START</w:t>{}<w:t>MID</w:t>{}<w:t>END</w:t></w:r></w:p>"#,
            shape("FLOAT", true),
            shape("INLINE", false)
        );
        let content = if long {
            content
                .replace(
                    "<w:t>START</w:t>",
                    &format!(
                        "<w:t>{}START</w:t>",
                        (0..40).map(|i| format!("PRE{i:03} ")).collect::<String>()
                    ),
                )
                .replace(
                    "<w:t>END</w:t>",
                    &format!(
                        "<w:t>END {}</w:t>",
                        (0..40).map(|i| format!("POST{i:03} ")).collect::<String>()
                    ),
                )
        } else {
            content
        };
        let section = if long {
            r#"<w:sectPr><w:pgSz w:w="6000" w:h="6000"/><w:pgMar w:top="600" w:bottom="600" w:left="600" w:right="600"/><w:cols w:num="2" w:equalWidth="0"><w:col w:w="1600" w:space="400"/><w:col w:w="2800"/></w:cols></w:sectPr>"#.to_string()
        } else {
            section(1)
        };
        let bytes = if note {
            authored(
                &(line("BODY").replace("</w:r>", "<w:footnoteReference w:id=\"1\"/></w:r>")
                    + &section),
                &format!(r#"<w:footnote w:id="1">{content}</w:footnote>"#),
            )
        } else {
            docx(&(content + &section), None, None)
        };
        let out = pages(bytes);
        let mut text = String::new();
        for page in out {
            let mut runs: Vec<_> = page
                .items
                .iter()
                .filter_map(|i| match i {
                    Item::Text(t) => Some(t),
                    _ => None,
                })
                .collect();
            runs.sort_by_key(|r| r.paragraph);
            text.extend(runs.iter().map(|r| r.text.as_str()));
        }
        assert!(text.contains("STARTFLOATMIDINLINEEND"), "{text}");
    }
}

#[test]
fn unequal_note_table_remeasures_remaining_cell_source() {
    for nested in [false, true] {
        for (fixed, reverse) in [(false, false), (true, false), (false, true)] {
            let body = r#"<w:p><w:r><w:t>BODY</w:t><w:footnoteReference w:id="1"/></w:r></w:p><w:sectPr><w:pgSz w:w="5600" w:h="4800"/><w:pgMar w:top="400" w:bottom="400" w:left="400" w:right="400"/><w:cols w:num="2" w:equalWidth="0"><w:col w:w="1600" w:space="400"/><w:col w:w="2800"/></w:cols></w:sectPr>"#;
            let body = if reverse {
                body.replace("w:w=\"1600\" w:space", "w:w=\"2800\" w:space")
                    .replace("<w:col w:w=\"2800\"/>", "<w:col w:w=\"1600\"/>")
            } else {
                body.to_owned()
            };
            let text = (0..120).map(|i| format!("T{i:03} ")).collect::<String>();
            let notes = format!(
                r#"<w:footnotes xmlns:w="{W}"><w:footnote w:id="1"><w:tbl><w:tblPr><w:tblW w:type="pct" w:w="5000"/><w:tblCellMar><w:top w:w="0" w:type="dxa"/><w:left w:w="0" w:type="dxa"/><w:bottom w:w="0" w:type="dxa"/><w:right w:w="0" w:type="dxa"/></w:tblCellMar></w:tblPr><w:tblGrid><w:gridCol w:w="200"/><w:gridCol w:w="200"/></w:tblGrid><w:tr><w:tc><w:p><w:pPr><w:widowControl w:val="0"/><w:spacing w:before="0" w:after="0" w:line="240" w:lineRule="exact"/></w:pPr><w:r><w:rPr><w:sz w:val="12"/></w:rPr><w:t>{text}</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:rPr><w:sz w:val="12"/></w:rPr><w:t>SIBLING</w:t></w:r></w:p></w:tc></w:tr></w:tbl></w:footnote></w:footnotes>"#
            );
            let notes = if fixed {
                notes
                    .replace(
                        r#"<w:tblW w:type="pct" w:w="5000"/>"#,
                        r#"<w:tblLayout w:type="fixed"/>"#,
                    )
                    .replace(r#"<w:gridCol w:w="200"/>"#, r#"<w:gridCol w:w="1800"/>"#)
            } else {
                notes
            };
            let notes = if reverse {
                notes.replace(r#"<w:gridCol w:w="200"/>"#, r#"<w:gridCol w:w="1400"/>"#)
            } else {
                notes
            };
            let notes = if nested {
                let start = notes.find("<w:tbl>").unwrap();
                let end = notes.rfind("</w:tbl>").unwrap() + "</w:tbl>".len();
                format!(
                    "{}<w:tbl><w:tblPr><w:tblW w:type=\"pct\" w:w=\"5000\"/><w:tblCellMar><w:top w:w=\"0\" w:type=\"dxa\"/><w:left w:w=\"0\" w:type=\"dxa\"/><w:bottom w:w=\"0\" w:type=\"dxa\"/><w:right w:w=\"0\" w:type=\"dxa\"/></w:tblCellMar></w:tblPr><w:tblGrid><w:gridCol w:w=\"400\"/></w:tblGrid><w:tr><w:tc>{}</w:tc></w:tr></w:tbl>{}",
                    &notes[..start],
                    &notes[start..end],
                    &notes[end..]
                )
            } else {
                notes
            };
            let out = pages(with_related_part(
                docx(&body, None, None),
                "footnotes",
                "footnotes.xml",
                &notes,
                false,
            ));
            let mut tokens = Vec::new();
            let mut widths = Vec::new();
            for page in &out {
                for item in &page.items {
                    if let Item::Text(run) = item
                        && run.text.starts_with('T')
                    {
                        tokens.extend(run.text.split_whitespace().map(str::to_owned));
                        let x = run.transform.tx as f64;
                        if fixed {
                            assert!(x + run.width <= page.info.w + EPS);
                            assert!(
                                x < page.info.cols[1].0,
                                "wide source table defers an incompatible right destination"
                            );
                        } else {
                            let (_, width) = page
                                .info
                                .cols
                                .iter()
                                .find(|(left, width)| {
                                    x >= *left - EPS && x + run.width <= left + width + EPS
                                })
                                .expect("table text stays in destination column");
                            widths.push(*width);
                        }
                        assert!(run.transform.ty as f64 <= page.info.bottom + EPS);
                    }
                }
            }
            assert_eq!(
                tokens,
                (0..120).map(|i| format!("T{i:03}")).collect::<Vec<_>>()
            );
            if !fixed {
                assert!(widths.iter().any(|w| (*w - 80.0).abs() < EPS));
                assert!(widths.iter().any(|w| (*w - 140.0).abs() < EPS));
            }
            assert!(out.len() < 10);
        }
    }
}

#[test]
fn unequal_note_merge_retains_source_across_destination_widths() {
    for nested in [false, true] {
        let body = r#"<w:p><w:r><w:t>BODY</w:t><w:footnoteReference w:id="1"/></w:r></w:p><w:sectPr><w:pgSz w:w="5600" w:h="4800"/><w:pgMar w:top="400" w:bottom="400" w:left="400" w:right="400"/><w:cols w:num="2" w:equalWidth="0"><w:col w:w="1600" w:space="400"/><w:col w:w="2800"/></w:cols></w:sectPr>"#;
        let paragraphs = (0..55).map(|i| format!(r#"<w:p><w:pPr><w:spacing w:before="0" w:after="0" w:line="240" w:lineRule="exact"/></w:pPr><w:r><w:rPr><w:sz w:val="12"/></w:rPr><w:t>MRG{i:04}</w:t></w:r></w:p>"#)).collect::<String>();
        let notes = format!(
            r#"<w:footnotes xmlns:w="{W}"><w:footnote w:id="1"><w:tbl><w:tblPr><w:tblW w:type="pct" w:w="5000"/></w:tblPr><w:tblGrid><w:gridCol w:w="800"/><w:gridCol w:w="800"/></w:tblGrid><w:tr><w:tc><w:tcPr><w:vMerge w:val="restart"/></w:tcPr>{paragraphs}</w:tc><w:tc><w:p><w:r><w:t>SIB0000</w:t></w:r></w:p></w:tc></w:tr><w:tr><w:tc><w:tcPr><w:vMerge/></w:tcPr><w:p/></w:tc><w:tc><w:p><w:r><w:t>SIB0001</w:t></w:r></w:p></w:tc></w:tr></w:tbl></w:footnote></w:footnotes>"#
        );
        let notes = if nested {
            let start = notes.find("<w:tbl>").unwrap();
            let end = notes.rfind("</w:tbl>").unwrap() + "</w:tbl>".len();
            format!(
                r#"{}<w:tbl><w:tblPr><w:tblW w:type="pct" w:w="5000"/></w:tblPr><w:tblGrid><w:gridCol w:w="1600"/></w:tblGrid><w:tr><w:tc>{}</w:tc></w:tr></w:tbl>{}"#,
                &notes[..start],
                &notes[start..end],
                &notes[end..]
            )
        } else {
            notes
        };
        let out = pages(with_related_part(
            docx(body, None, None),
            "footnotes",
            "footnotes.xml",
            &notes,
            false,
        ));
        let mut source_text = String::new();
        for page in &out {
            for item in &page.items {
                if let Item::Text(run) = item {
                    source_text.push_str(&run.text);
                    if run.text.starts_with("MRG") || run.text.chars().all(|c| c.is_ascii_digit()) {
                        let x = run.transform.tx as f64;
                        assert!(
                            page.info.cols.iter().any(|(left, width)| x >= left - EPS
                                && x + run.width <= left + width + EPS)
                        );
                        assert!(
                            run.transform.ty as f64 >= page.info.top - EPS,
                            "merged source must remain inside the accepted note area: {} y={} nested={}",
                            run.text,
                            run.transform.ty,
                            nested
                        );
                        assert!(run.transform.ty as f64 <= page.info.bottom + EPS);
                    }
                }
            }
        }
        let mut previous = 0;
        for index in 0..55 {
            let token = format!("MRG{index:04}");
            assert_eq!(source_text.matches(&token).count(), 1, "{token}");
            let position = source_text.find(&token).unwrap();
            assert!(position >= previous, "merged cell source order");
            previous = position;
        }
        assert!(out.len() < 10);
    }
}

#[test]
fn cached_nested_merge_occurrences_keep_separate_paint_history() {
    let body = r#"<w:p><w:r><w:t>BODY</w:t><w:footnoteReference w:id="1"/></w:r></w:p><w:sectPr><w:pgSz w:w="5600" w:h="4800"/><w:pgMar w:top="400" w:bottom="400" w:left="400" w:right="400"/><w:cols w:num="2" w:equalWidth="0"><w:col w:w="1600" w:space="400"/><w:col w:w="2800"/></w:cols></w:sectPr>"#;
    let paragraphs = (0..55).map(|i| format!(r#"<w:p><w:pPr><w:spacing w:before="0" w:after="0" w:line="240" w:lineRule="exact"/></w:pPr><w:r><w:rPr><w:sz w:val="12"/></w:rPr><w:t>MRG{i:04}</w:t></w:r></w:p>"#)).collect::<String>();
    let notes = format!(
        r#"<w:footnotes xmlns:w="{W}"><w:footnote w:id="1"><w:tbl><w:tblPr><w:tblW w:type="pct" w:w="5000"/></w:tblPr><w:tblGrid><w:gridCol w:w="800"/><w:gridCol w:w="800"/></w:tblGrid><w:tr><w:tc><w:tcPr><w:vMerge w:val="restart"/></w:tcPr>{paragraphs}</w:tc><w:tc><w:p><w:r><w:t>SIB0000</w:t></w:r></w:p></w:tc></w:tr><w:tr><w:tc><w:tcPr><w:vMerge/></w:tcPr><w:p/></w:tc><w:tc><w:p><w:r><w:t>SIB0001</w:t></w:r></w:p></w:tc></w:tr></w:tbl></w:footnote></w:footnotes>"#
    );
    let notes = if true {
        let start = notes.find("<w:tbl>").unwrap();
        let end = notes.rfind("</w:tbl>").unwrap() + "</w:tbl>".len();
        format!(
            r#"{}<w:tbl><w:tblPr><w:tblW w:type="pct" w:w="5000"/></w:tblPr><w:tblGrid><w:gridCol w:w="1600"/></w:tblGrid><w:tr><w:tc>{}</w:tc></w:tr></w:tbl>{}"#,
            &notes[..start],
            &notes[start..end],
            &notes[end..]
        )
    } else {
        notes
    };
    let body = body.replace("<w:sectPr>", r#"<w:p><w:r><w:br w:type="page"/><w:t>SECOND</w:t><w:footnoteReference w:id="1"/></w:r></w:p><w:sectPr>"#);
    let out = pages(with_related_part(
        docx(&body, None, None),
        "footnotes",
        "footnotes.xml",
        &notes,
        false,
    ));
    let all = text(&out);
    for i in 0..55 {
        let token = format!("MRG{i:04}");
        assert_eq!(all.matches(&token).count(), 2, "{token}");
    }
}

#[test]
fn delayed_child_terminal_backfills_inside_an_outer_merge() {
    let payload = (0..55)
        .map(|i| {
            line(&format!("C{i:04}")).replace("<w:r>", r#"<w:r><w:rPr><w:sz w:val="10"/></w:rPr>"#)
        })
        .collect::<String>();
    let middle = (0..23)
        .map(|i| {
            line(&format!("D{i:04}")).replace("<w:r>", r#"<w:r><w:rPr><w:sz w:val="10"/></w:rPr>"#)
        })
        .collect::<String>();
    let properties = r#"<w:tblPr><w:tblW w:type="pct" w:w="5000"/></w:tblPr><w:tblGrid><w:gridCol w:w="800"/><w:gridCol w:w="800"/></w:tblGrid>"#;
    let inner = format!(
        r#"<w:tbl>{properties}<w:tr><w:tc><w:tcPr><w:vMerge w:val="restart"/></w:tcPr>{payload}</w:tc><w:tc>{}</w:tc></w:tr><w:tr><w:tc><w:tcPr><w:vMerge/></w:tcPr><w:p/></w:tc><w:tc>{middle}</w:tc></w:tr><w:tr><w:tc><w:tcPr><w:vMerge/></w:tcPr><w:p/></w:tc><w:tc>{}</w:tc></w:tr></w:tbl>"#,
        line("FIRST"),
        line("LAST")
    );
    let outer = format!(
        r#"<w:tbl>{properties}<w:tr><w:tc><w:tcPr><w:vMerge w:val="restart"/></w:tcPr>{inner}</w:tc><w:tc>{}</w:tc></w:tr><w:tr><w:tc><w:tcPr><w:vMerge/></w:tcPr><w:p/></w:tc><w:tc>{}</w:tc></w:tr></w:tbl>"#,
        line("OUTERFIRST"),
        line("OUTERLAST")
    );
    let out = pages(authored(&(reference(1) + &section(2)), &note(1, &outer)));
    // The narrower merged cell can split a token across physical pages, with
    // the sibling cell emitted between its pieces. Numeric-only pieces belong
    // to the C payload in this fixture; reconstruct that cell's complete stream.
    let runs: Vec<&str> = out
        .iter()
        .flat_map(|p| &p.items)
        .filter_map(|i| match i {
            Item::Text(t) => Some(t.text.as_str()),
            _ => None,
        })
        .collect();
    let left: String = runs
        .iter()
        .copied()
        .filter(|s| s.starts_with('C') || (!s.is_empty() && s.bytes().all(|c| c.is_ascii_digit())))
        .collect();
    let right: String = runs
        .iter()
        .copied()
        .filter(|s| s.starts_with('D'))
        .collect();
    assert_eq!(
        left,
        (0..55).map(|i| format!("C{i:04}")).collect::<String>()
    );
    assert_eq!(
        right,
        (0..23).map(|i| format!("D{i:04}")).collect::<String>()
    );
    assert!(out.len() < 15);
}
#[test]
fn merged_header_capacity_change_preserves_contiguous_source_line_spacing() {
    let para =
        |value: &str| line(value).replace("<w:r>", r#"<w:r><w:rPr><w:sz w:val="20"/></w:rPr>"#);
    let payload = (0..38)
        .map(|i| para(&format!("MRG{i:04}")))
        .collect::<String>();
    let mut table = format!(
        r#"<w:tbl><w:tblPr><w:tblW w:w="4200" w:type="dxa"/><w:tblCellMar><w:top w:w="60"/><w:bottom w:w="60"/><w:left w:w="60"/><w:right w:w="60"/></w:tblCellMar><w:tblBorders><w:top w:val="single" w:sz="4"/><w:bottom w:val="single" w:sz="4"/><w:insideH w:val="single" w:sz="4"/></w:tblBorders></w:tblPr><w:tblGrid><w:gridCol w:w="2100"/><w:gridCol w:w="2100"/></w:tblGrid><w:tr><w:trPr><w:tblHeader/></w:trPr><w:tc>{}</w:tc><w:tc>{}</w:tc></w:tr>"#,
        para("HEADER0"),
        para("HEADER1")
    );
    for row in 0..4 {
        let merge = if row == 0 {
            r#"<w:vMerge w:val="restart"/>"#
        } else {
            "<w:vMerge/>"
        };
        let content = if row == 0 {
            payload.clone()
        } else {
            "<w:p/>".into()
        };
        let sibling = (0..7)
            .map(|i| para(&format!("S{row}_{i}")))
            .collect::<String>();
        table += &format!(
            "<w:tr><w:tc><w:tcPr>{merge}</w:tcPr>{content}</w:tc><w:tc>{sibling}</w:tc></w:tr>"
        );
    }
    table += "</w:tbl>";
    let body = reference(1)
        + r#"<w:sectPr><w:pgSz w:w="10000" w:h="6000"/><w:pgMar w:top="600" w:bottom="600" w:left="600" w:right="600"/><w:cols w:num="2" w:space="400"/></w:sectPr>"#;
    let out = pages(authored(&body, &note(1, &(para("NOTE") + &table))));
    let mut pairs = 0;
    let mut previous: Option<(usize, f64, f64)> = None;
    for (page, output) in out.iter().enumerate() {
        for item in &output.items {
            let Item::Text(run) = item else { continue };
            if !run.text.starts_with("MRG") {
                continue;
            }
            let x = run.transform.tx as f64;
            let y = run.transform.ty as f64;
            if let Some((prior_page, prior_x, prior_y)) = previous
                && prior_page == page
                && (prior_x - x).abs() < EPS
            {
                assert!(
                    (y - prior_y - 12.0).abs() < EPS,
                    "{} gap {}",
                    run.text,
                    y - prior_y
                );
                pairs += 1;
            }
            previous = Some((page, x, y));
        }
    }
    assert!(pairs > 20);
}
