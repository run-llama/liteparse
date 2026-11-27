use super::*;
use crate::office::docx::layout::ParaUnit;
use crate::office::docx::text::Line;

fn paragraph(count: usize, height: f64) -> ParaUnit {
    ParaUnit {
        lines: (0..count)
            .map(|i| Line {
                height,
                replay: None,
                trailing_leading: 0.0,
                baseline: height * 0.8,
                items: Vec::new(),
                break_after: None,
                anchors: Vec::new(),
                notes: Vec::new(),
                text: format!("LINE{i}"),
                skip: 0.0,
                boxes: Vec::new(),
            })
            .collect(),
        frame_anchors: Vec::new(),
        drop_cap_anchor: None,
        clip_inline_images: false,
        before: 0.0,
        after: 0.0,
        pad_top: 0.0,
        pad_bottom: 0.0,
        keep_next: false,
        keep_lines: false,
        widow: false,
        page_break_before: false,
        clear_after: false,
        column_break_before: false,
        decor: None,
        src: None,
    }
}

fn budget(height: f64, full: f64, fresh: bool) -> FragmentBudget {
    FragmentBudget {
        height,
        fresh_capacity: full,
        fresh,
        allow_atomic_overflow: false,
    }
}

fn lines(plan: &FragmentPlan) -> Vec<(usize, usize)> {
    plan.placements
        .iter()
        .flat_map(|p| match p {
            NotePlacement::Paragraph {
                unit, first, end, ..
            } => (*first..*end).map(|l| (*unit, l)).collect(),
            _ => Vec::new(),
        })
        .collect()
}

#[test]
fn long_note_consumes_every_source_line_once_with_deterministic_trials() {
    let layout = NoteLayout::new(vec![Unit::Para(paragraph(550, 12.0))]);
    let mut cursor = layout.start();
    let mut seen = Vec::new();
    let mut count = 0;
    while !cursor.complete() {
        let plan = plan_fragment(&layout, &cursor, budget(200.0, 200.0, true)).unwrap();
        let replay = plan_fragment(&layout, &cursor, budget(200.0, 200.0, true)).unwrap();
        assert_eq!(lines(&plan), lines(&replay));
        assert_eq!(plan.used_height, replay.used_height);
        assert!(plan.used_height <= 200.0);
        assert!(!plan.placements.is_empty());
        seen.extend(lines(&plan));
        cursor = plan.after;
        count += 1;
        assert!(count < 100);
    }
    assert_eq!(seen, (0..550).map(|i| (0, i)).collect::<Vec<_>>());
    assert_eq!(count, 35);
}

#[test]
fn spacing_and_clearance_belong_to_their_source_fragments() {
    let mut p = paragraph(4, 10.0);
    p.before = 3.0;
    p.pad_top = 2.0;
    p.after = 7.0;
    p.pad_bottom = 1.0;
    p.lines[2].skip = 4.0;
    let layout = NoteLayout::new(vec![Unit::Para(p)]);
    assert_eq!(layout.legacy_slices().1, 53.0);
    let first = plan_fragment(&layout, &layout.start(), budget(25.0, 40.0, false)).unwrap();
    assert_eq!(lines(&first), vec![(0, 0), (0, 1)]);
    assert_eq!(first.used_height, 25.0);
    let last = plan_fragment(&layout, &first.after, budget(40.0, 40.0, true)).unwrap();
    assert_eq!(lines(&last), vec![(0, 2), (0, 3)]);
    assert_eq!(last.used_height, 32.0);
    assert!(last.after.complete());
    match &last.placements[0] {
        NotePlacement::Paragraph { line_tops, .. } => assert_eq!(line_tops, &vec![4.0, 14.0]),
        _ => panic!(),
    }
}

#[test]
fn keeps_use_full_fresh_capacity_not_remaining_space() {
    let mut p = paragraph(4, 10.0);
    p.keep_lines = true;
    let layout = NoteLayout::new(vec![Unit::Para(p)]);
    let retry = plan_fragment(&layout, &layout.start(), budget(20.0, 50.0, true)).unwrap();
    assert_eq!(retry.outcome, FragmentOutcome::Retry);
    assert!(retry.placements.is_empty());
    let oversized = plan_fragment(&layout, &layout.start(), budget(20.0, 20.0, true)).unwrap();
    assert_eq!(lines(&oversized), vec![(0, 0), (0, 1)]);
}

#[test]
fn widow_keep_chain_and_manual_breaks_preserve_cursor_boundaries() {
    let mut a = paragraph(1, 10.0);
    a.keep_next = true;
    let mut b = paragraph(3, 10.0);
    b.widow = true;
    let layout = NoteLayout::new(vec![Unit::Para(a), Unit::Para(b)]);
    let retry = plan_fragment(&layout, &layout.start(), budget(20.0, 50.0, false)).unwrap();
    assert!(retry.placements.is_empty());
    let plan = plan_fragment(&layout, &layout.start(), budget(40.0, 50.0, true)).unwrap();
    assert!(plan.after.complete());
    let mut p = paragraph(3, 10.0);
    p.lines[0].break_after = Some(BreakKind::Page);
    let layout = NoteLayout::new(vec![Unit::Para(p)]);
    let plan = plan_fragment(&layout, &layout.start(), budget(100.0, 100.0, true)).unwrap();
    assert_eq!(lines(&plan), vec![(0, 0)]);
    assert_eq!(plan.outcome, FragmentOutcome::Break(BreakKind::Page));
    let rest = plan_fragment(&layout, &plan.after, budget(100.0, 100.0, true)).unwrap();
    assert_eq!(lines(&rest), vec![(0, 1), (0, 2)]);
}

#[test]
fn invalid_budgets_layout_reuse_and_atomic_overflow_are_explicit() {
    let layout = NoteLayout::new(vec![Unit::Para(paragraph(1, 300.0))]);
    for height in [f64::NAN, -1.0, f64::INFINITY] {
        assert!(matches!(
            plan_fragment(&layout, &layout.start(), budget(height, 200.0, true)),
            Err(PlanError::InvalidBudget)
        ));
    }
    let other = NoteLayout::new(vec![Unit::Para(paragraph(1, 300.0))]);
    assert!(matches!(
        plan_fragment(&layout, &other.start(), budget(200.0, 200.0, true)),
        Err(PlanError::WrongLayout)
    ));
    let mut b = budget(200.0, 200.0, true);
    b.allow_atomic_overflow = true;
    let overflow = plan_fragment(&layout, &layout.start(), b).unwrap();
    assert_eq!(overflow.outcome, FragmentOutcome::AtomicOverflow);
    assert!(overflow.after.complete());
    assert_eq!(overflow.used_height, 300.0);
}

#[test]
fn explicit_break_overrides_keep_chain_without_replaying_the_line() {
    let mut p = paragraph(1, 12.0);
    p.keep_next = true;
    p.lines[0].break_after = Some(BreakKind::Page);
    let layout = NoteLayout::new(vec![Unit::Para(p), Unit::Para(paragraph(1, 12.0))]);
    let first = plan_fragment(&layout, &layout.start(), budget(100.0, 100.0, true)).unwrap();
    assert_eq!(first.outcome, FragmentOutcome::Break(BreakKind::Page));
    assert_eq!(lines(&first), vec![(0, 0)]);
    assert_eq!(first.after.unit, 1);
    let last = plan_fragment(&layout, &first.after, budget(100.0, 100.0, true)).unwrap();
    assert_eq!(lines(&last), vec![(1, 0)]);
    assert!(last.after.complete());
}

#[test]
fn spacing_only_paragraph_requires_capacity_or_explicit_atomic_overflow() {
    let mut p = paragraph(0, 10.0);
    p.before = 60.0;
    p.after = 60.0;
    let layout = NoteLayout::new(vec![Unit::Para(p)]);
    let retry = plan_fragment(&layout, &layout.start(), budget(100.0, 100.0, true)).unwrap();
    assert!(retry.placements.is_empty());
    assert_eq!(retry.outcome, FragmentOutcome::Retry);
    let mut b = budget(100.0, 100.0, true);
    b.allow_atomic_overflow = true;
    let overflow = plan_fragment(&layout, &layout.start(), b).unwrap();
    assert_eq!(overflow.outcome, FragmentOutcome::AtomicOverflow);
    assert_eq!(overflow.used_height, 120.0);
    assert!(overflow.after.complete());
}

#[test]
fn impossible_widow_pairs_relax_from_the_current_source_cursor() {
    let mut p = paragraph(3, 10.0);
    p.widow = true;
    let layout = NoteLayout::new(vec![Unit::Para(p)]);
    let first = plan_fragment(&layout, &layout.start(), budget(20.0, 20.0, true)).unwrap();
    assert_eq!(lines(&first), vec![(0, 0), (0, 1)]);
    let mut p = paragraph(4, 10.0);
    p.widow = true;
    p.lines[2].height = 80.0;
    p.lines[3].height = 80.0;
    let layout = NoteLayout::new(vec![Unit::Para(p)]);
    let mut cursor = layout.start();
    cursor.part = UnitCursor::Paragraph { line: 2 };
    let first = plan_fragment(&layout, &cursor, budget(100.0, 100.0, true)).unwrap();
    assert_eq!(lines(&first), vec![(0, 2)]);
    let last = plan_fragment(&layout, &first.after, budget(100.0, 100.0, true)).unwrap();
    assert_eq!(lines(&last), vec![(0, 3)]);
    assert!(last.after.complete());
}

fn empty_row(height: f64, outer_after: f64) -> RowUnit {
    RowUnit {
        bookmarks_before: Vec::new(),
        bookmarks_after: Vec::new(),
        provenance: None,
        source: None,
        height,
        min_height: 0.0,
        natural_height: height,
        after_keep: false,
        natural_addition: 0.0,
        merge_base_height: height,
        outer_before: 0.0,
        outer_after,
        cells: Vec::new(),
        page_break_before: false,
        cant_split: false,
        exact_height: false,
        header: false,
        repeating_header: false,
        table: 0,
        headers: Rc::new(Vec::new()),
        float_y: None,
        float_x: None,
        float_clear: false,
        keep_next: false,
        index: 0,
    }
}

#[test]
fn empty_row_height_debt_conserves_geometry_and_trailing_edge_ownership() {
    for (height, edge) in [(95.0, 10.0), (20.0, 90.0), (0.0, 150.0)] {
        let layout = NoteLayout::new(vec![Unit::Row(empty_row(height, edge))]);
        let p = plan_fragment(&layout, &layout.start(), budget(100.0, 200.0, true)).unwrap();
        assert_eq!(p.outcome, FragmentOutcome::Retry);
        assert!(p.placements.is_empty());
    }
    let layout = NoteLayout::new(vec![Unit::Row(empty_row(250.0, 10.0))]);
    let mut cursor = layout.start();
    let mut height = 0.0;
    let mut n = 0;
    while !cursor.complete() {
        let p = plan_fragment(&layout, &cursor, budget(100.0, 100.0, true)).unwrap();
        assert!(p.used_height > 0.0 && p.used_height <= 100.0);
        height += p.used_height;
        cursor = p.after;
        n += 1;
        assert!(n < 5);
    }
    assert_eq!(height, 260.0);
    assert_eq!(n, 3);
}

#[test]
fn keep_requirement_stops_at_manual_boundary_before_later_lines() {
    let mut first = paragraph(1, 10.0);
    first.keep_next = true;
    let mut second = paragraph(10, 10.0);
    second.keep_next = true;
    second.keep_lines = true;
    second.lines[0].break_after = Some(BreakKind::Column);
    let layout = NoteLayout::new(vec![Unit::Para(first), Unit::Para(second)]);
    let plan = plan_fragment(&layout, &layout.start(), budget(20.0, 50.0, false)).unwrap();
    assert_eq!(lines(&plan), vec![(0, 0), (1, 0)]);
    assert_eq!(plan.outcome, FragmentOutcome::Break(BreakKind::Column));
}

#[test]
fn row_page_break_overrides_preceding_keep_and_widows_stop_at_manual_segments() {
    let mut p = paragraph(1, 12.0);
    p.keep_next = true;
    let mut r = empty_row(12.0, 0.0);
    r.page_break_before = true;
    let layout = NoteLayout::new(vec![Unit::Para(p), Unit::Row(r)]);
    let first = plan_fragment(&layout, &layout.start(), budget(100.0, 100.0, true)).unwrap();
    assert_eq!(first.outcome, FragmentOutcome::Break(BreakKind::Page));
    assert_eq!(lines(&first), vec![(0, 0)]);
    assert_eq!(first.after.unit, 1);
    let next = plan_fragment(&layout, &first.after, budget(100.0, 100.0, true)).unwrap();
    assert!(next.after.complete());
    let mut p = paragraph(3, 10.0);
    p.widow = true;
    p.lines[1].break_after = Some(BreakKind::Page);
    let layout = NoteLayout::new(vec![Unit::Para(p)]);
    let first = plan_fragment(&layout, &layout.start(), budget(10.0, 10.0, true)).unwrap();
    assert_eq!(lines(&first), vec![(0, 0)]);
    let next = plan_fragment(&layout, &first.after, budget(10.0, 10.0, true)).unwrap();
    assert_eq!(lines(&next), vec![(0, 1)]);
    assert_eq!(next.outcome, FragmentOutcome::Break(BreakKind::Page));
}

fn source_units(body: &str) -> Vec<Unit> {
    let opened = crate::office::docx::extract::open(
        crate::office::docx::testutil::docx(body, None, None),
        &Default::default(),
    )
    .unwrap();
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
    crate::office::docx::layout::Flow::new(&env, 15).units(&opened.sections[0].blocks, 240.0, false)
}

#[test]
fn source_ordinary_row_remainders_keep_cell_text_and_terminal_spacing_once() {
    let paras=(0..20).map(|i|format!(r#"<w:p><w:pPr><w:widowControl w:val="0"/><w:spacing w:after="{}" w:line="240" w:lineRule="exact"/></w:pPr><w:r><w:t>ROW{i:02}</w:t></w:r></w:p>"#,if i==19 {140}else{0})).collect::<String>();
    let xml = format!(
        r#"<w:tbl><w:tblPr><w:tblCellMar><w:top w:w="80"/><w:bottom w:w="80"/></w:tblCellMar></w:tblPr><w:tblGrid><w:gridCol w:w="4800"/></w:tblGrid><w:tr><w:tc>{paras}</w:tc></w:tr></w:tbl>"#
    );
    let layout = NoteLayout::new(source_units(&xml));
    let mut cursor = layout.start();
    let mut text = String::new();
    let mut tails = Vec::new();
    let mut pieces = 0;
    while !cursor.complete() {
        let plan = plan_fragment(&layout, &cursor, budget(64.0, 64.0, true)).unwrap();
        assert!(!plan.placements.is_empty());
        assert!(plan.used_height <= 64.01);
        for p in &plan.placements {
            if let NotePlacement::Row { row, .. } = p {
                for c in &row.cells {
                    tails.push(c.terminal_after);
                    for s in &c.slices {
                        for it in &s.items {
                            if let crate::office::pptx::scene::Item::Text(r) = it {
                                text.push_str(&r.text);
                            }
                        }
                    }
                }
            }
        }
        cursor = plan.after;
        pieces += 1;
        assert!(pieces < 20);
    }
    assert_eq!(
        text,
        (0..20).map(|i| format!("ROW{i:02}")).collect::<String>()
    );
    assert_eq!(tails.iter().sum::<f64>(), 7.0);
    assert_eq!(tails.last(), Some(&7.0));
    assert!(pieces > 1);
}

#[test]
fn resumed_paragraph_and_row_tails_keep_their_successor() {
    let mut p = paragraph(2, 12.0);
    p.keep_next = true;
    p.lines[0].break_after = Some(BreakKind::Page);
    let layout = NoteLayout::new(vec![Unit::Para(p), Unit::Para(paragraph(1, 12.0))]);
    let first = plan_fragment(&layout, &layout.start(), budget(24.0, 24.0, true)).unwrap();
    let retry = plan_fragment(&layout, &first.after, budget(12.0, 24.0, true)).unwrap();
    assert!(retry.placements.is_empty());
    assert_eq!(retry.outcome, FragmentOutcome::Retry);
    let last = plan_fragment(&layout, &first.after, budget(24.0, 24.0, true)).unwrap();
    assert!(last.after.complete());
    assert_eq!(lines(&last), vec![(0, 1), (1, 0)]);
    let mut r = empty_row(30.0, 0.0);
    r.keep_next = true;
    let layout = NoteLayout::new(vec![Unit::Row(r), Unit::Para(paragraph(1, 10.0))]);
    let first = plan_fragment(&layout, &layout.start(), budget(20.0, 20.0, true)).unwrap();
    let retry = plan_fragment(&layout, &first.after, budget(10.0, 20.0, true)).unwrap();
    assert!(retry.placements.is_empty());
    assert_eq!(retry.outcome, FragmentOutcome::Retry);
    let last = plan_fragment(&layout, &first.after, budget(20.0, 20.0, true)).unwrap();
    assert!(last.after.complete());
    assert_eq!(last.used_height, 20.0);
}

#[test]
fn paragraph_flag_matrix_always_consumes_source_on_a_full_fresh_destination() {
    for count in 1..=5 {
        for heights in 0..(1 << count) {
            for flags in 0..8 {
                for cap in [10.0, 20.0, 40.0] {
                    for boundary in 0..=count {
                        let mut p = paragraph(count, 10.0);
                        p.keep_lines = flags & 1 != 0;
                        p.keep_next = flags & 2 != 0;
                        p.widow = flags & 4 != 0;
                        for (i, l) in p.lines.iter_mut().enumerate() {
                            if heights & (1 << i) != 0 {
                                l.height = 30.0;
                            }
                        }
                        if boundary < count {
                            p.lines[boundary].break_after = Some(if boundary % 2 == 0 {
                                BreakKind::Page
                            } else {
                                BreakKind::Column
                            });
                        }
                        let layout =
                            NoteLayout::new(vec![Unit::Para(p), Unit::Para(paragraph(2, 10.0))]);
                        let mut cursor = layout.start();
                        let mut seen = Vec::new();
                        for _ in 0..count + 3 {
                            if cursor.complete() {
                                break;
                            }
                            let mut b = budget(cap, cap, true);
                            b.allow_atomic_overflow = true;
                            let plan = plan_fragment(&layout, &cursor, b).unwrap();
                            let consumed = lines(&plan);
                            assert!(
                                !consumed.is_empty(),
                                "count={count} heights={heights} flags={flags} cap={cap} boundary={boundary} unit={} outcome={:?}",
                                cursor.unit,
                                plan.outcome
                            );
                            seen.extend(consumed);
                            cursor = plan.after;
                        }
                        assert!(cursor.complete());
                        assert_eq!(
                            seen,
                            (0..count)
                                .map(|i| (0, i))
                                .chain((0..2).map(|i| (1, i)))
                                .collect::<Vec<_>>()
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn deferred_anchors_follow_original_line_identity_across_fragments() {
    let xml = r##"<w:p><w:pPr><w:widowControl w:val="0"/><w:spacing w:line="240" w:lineRule="exact"/></w:pPr><w:r><w:t>FIRST</w:t><w:br/><w:t>SECOND</w:t><w:br/></w:r><w:r><w:pict><v:shape xmlns:v="urn:schemas-microsoft-com:vml" id="box" type="#_x0000_t202" style="position:absolute;margin-left:10pt;margin-top:20pt;width:30pt;height:10pt"><v:textbox><w:txbxContent><w:p><w:r><w:t>ANCHOR</w:t></w:r></w:p></w:txbxContent></v:textbox></v:shape></w:pict></w:r><w:r><w:t>THIRD</w:t></w:r></w:p>"##;
    let layout = NoteLayout::new(source_units(xml));
    let expected = layout
        .units()
        .iter()
        .enumerate()
        .flat_map(|(u, p)| match p {
            Unit::Para(p) => p
                .lines
                .iter()
                .enumerate()
                .flat_map(move |(l, line)| {
                    line.anchors
                        .iter()
                        .map(move |a| (u, l, a.pic.w.to_bits(), a.pic.h.to_bits()))
                })
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        })
        .collect::<Vec<_>>();
    assert!(!expected.is_empty());
    let mut cursor = layout.start();
    let mut anchors = Vec::new();
    let mut pages = 0;
    while !cursor.complete() {
        let plan = plan_fragment(&layout, &cursor, budget(12.0, 12.0, true)).unwrap();
        assert!(!plan.placements.is_empty());
        for (u, l) in lines(&plan) {
            if let Unit::Para(p) = &layout.units()[u] {
                anchors.extend(
                    p.lines[l]
                        .anchors
                        .iter()
                        .map(|a| (u, l, a.pic.w.to_bits(), a.pic.h.to_bits())),
                );
            }
        }
        cursor = plan.after;
        pages += 1;
        assert!(pages < 10);
    }
    assert_eq!(anchors, expected);
    assert!(pages > 1);
}

#[test]
fn structured_rows_stop_with_source_and_prior_prefix_intact() {
    let mut row = empty_row(20.0, 0.0);
    let units = source_units(
        r#"<w:tbl><w:tblGrid><w:gridCol w:w="2000"/></w:tblGrid><w:tr><w:tc><w:p><w:r><w:t>MERGED_SOURCE</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#,
    );
    if let Unit::Row(source) = &units[0] {
        row.cells = source.cells.clone();
    }
    assert_eq!(row.cells.len(), 1);
    row.cells[0].moved = Some(Rc::new((row.cells[0].slices.clone(), 0.0)));
    let layout = NoteLayout::new(vec![Unit::Para(paragraph(1, 10.0)), Unit::Row(row)]);
    let plan = plan_fragment(&layout, &layout.start(), budget(100.0, 100.0, true)).unwrap();
    assert_eq!(plan.outcome, FragmentOutcome::NeedsStructuredRow);
    assert_eq!(lines(&plan), vec![(0, 0)]);
    assert_eq!(plan.after.unit, 1);
    assert!(!plan.after.complete());
    let next = plan_fragment(&layout, &plan.after, budget(100.0, 100.0, true)).unwrap();
    assert_eq!(next.outcome, FragmentOutcome::NeedsStructuredRow);
    assert!(next.placements.is_empty());
    assert_eq!(next.after.unit, 1);
    assert!(matches!(&layout.units()[1],Unit::Row(r)if r.cells[0].moved.is_some()));
}

#[test]
fn ordinary_nested_rows_continue_with_immutable_source_and_late_ownership() {
    let ps = (0..30).map(|i| format!(r#"<w:p><w:pPr><w:widowControl w:val="0"/><w:spacing w:before="0" w:after="0" w:line="240" w:lineRule="exact"/></w:pPr><w:r><w:t>NEST{i:03}</w:t></w:r></w:p>"#)).collect::<String>();
    let tbl = |inside: &str| {
        format!(
            r#"<w:tbl><w:tblPr><w:tblCellMar><w:top w:w="40" w:type="dxa"/><w:bottom w:w="60" w:type="dxa"/></w:tblCellMar></w:tblPr><w:tblGrid><w:gridCol w:w="2000"/></w:tblGrid><w:tr><w:tc>{inside}</w:tc></w:tr></w:tbl>"#
        )
    };
    let layout = NoteLayout::new(source_units(&tbl(&format!("{}<w:p/>", tbl(&ps)))));
    let source_height = layout.legacy_slices().1;
    let mut cursor = layout.start();
    let mut text = String::new();
    for _ in 0..20 {
        if cursor.complete() {
            break;
        }
        let plan = plan_fragment(&layout, &cursor, budget(90.0, 90.0, true)).unwrap();
        let repeat = plan_fragment(&layout, &cursor, budget(90.0, 90.0, true)).unwrap();
        assert_eq!(plan.used_height, repeat.used_height);
        assert!(plan.used_height <= 90.01);
        assert!(!plan.placements.is_empty(), "{:?}", plan.outcome);
        for piece in &plan.placements {
            if let NotePlacement::Row { row, .. } = piece {
                for item in crate::office::docx::layout::row_items(row, 0.0, 0.0) {
                    if let crate::office::pptx::scene::Item::Text(t) = item {
                        text.push_str(&t.text);
                    }
                }
            }
        }
        cursor = plan.after;
    }
    assert!(cursor.complete());
    assert_eq!(layout.legacy_slices().1, source_height);
    let mut previous = 0;
    for i in 0..30 {
        let label = format!("NEST{i:03}");
        assert_eq!(text.matches(&label).count(), 1);
        let at = text.find(&label).unwrap();
        assert!(at >= previous);
        previous = at;
    }
}

#[test]
fn oversized_nested_cant_split_row_continues_after_fresh_start() {
    let ps = (0..30).map(|i| format!(r#"<w:p><w:pPr><w:widowControl w:val="0"/><w:spacing w:before="0" w:after="0" w:line="240" w:lineRule="exact"/></w:pPr><w:r><w:t>ATOMIC{i:03}</w:t></w:r></w:p>"#)).collect::<String>();
    let table = |content: &str, cant: bool| {
        format!(
            r#"<w:tbl><w:tblPr><w:tblCellMar><w:top w:w="0" w:type="dxa"/><w:bottom w:w="0" w:type="dxa"/></w:tblCellMar></w:tblPr><w:tblGrid><w:gridCol w:w="2000"/></w:tblGrid><w:tr>{}<w:tc>{content}</w:tc></w:tr></w:tbl>"#,
            if cant {
                "<w:trPr><w:cantSplit/></w:trPr>"
            } else {
                ""
            }
        )
    };
    let short = ps
        .split("</w:p>")
        .take(5)
        .map(|p| format!("{p}</w:p>"))
        .collect::<String>();
    let short_layout = NoteLayout::new(source_units(&table(
        &format!("{}<w:p/>", table(&short, true)),
        false,
    )));
    let short_retry = plan_fragment(
        &short_layout,
        &short_layout.start(),
        budget(36.0, 120.0, true),
    )
    .unwrap();
    assert_eq!(short_retry.outcome, FragmentOutcome::Retry);
    assert!(short_retry.placements.is_empty());
    let short_fit = plan_fragment(
        &short_layout,
        &short_layout.start(),
        budget(120.0, 120.0, true),
    )
    .unwrap();
    assert!(short_fit.after.complete());
    let layout = NoteLayout::new(source_units(&table(
        &format!("{}<w:p/>", table(&ps, true)),
        false,
    )));
    let retry = plan_fragment(&layout, &layout.start(), budget(60.0, 120.0, false)).unwrap();
    assert_eq!(retry.outcome, FragmentOutcome::Retry);
    assert!(retry.placements.is_empty());
    let mut cursor = layout.start();
    let mut text = String::new();
    for _ in 0..10 {
        if cursor.complete() {
            break;
        }
        let mut b = budget(120.0, 120.0, true);
        b.allow_atomic_overflow = true;
        let plan = plan_fragment(&layout, &cursor, b).unwrap();
        assert!(
            plan.used_height <= 120.01,
            "{} {:?}",
            plan.used_height,
            plan.outcome
        );
        assert!(!plan.placements.is_empty());
        for placement in &plan.placements {
            if let NotePlacement::Row { row, .. } = placement {
                for item in crate::office::docx::layout::row_items(row, 0.0, 0.0) {
                    if let crate::office::pptx::scene::Item::Text(t) = item {
                        text.push_str(&t.text);
                    }
                }
            }
        }
        cursor = plan.after;
    }
    assert!(cursor.complete());
    for i in 0..30 {
        assert_eq!(text.matches(&format!("ATOMIC{i:03}")).count(), 1);
    }
}

fn merged_units(count: usize) -> Vec<Unit> {
    let owner = (0..count).map(|i| format!(r#"<w:p><w:pPr><w:widowControl w:val="0"/><w:spacing w:before="0" w:after="0" w:line="240" w:lineRule="exact"/></w:pPr><w:r><w:t>MERGE{i:03}</w:t></w:r></w:p>"#)).collect::<String>();
    let mut rows = String::new();
    for i in 0..4 {
        rows.push_str(&format!(r#"<w:tr><w:tc><w:tcPr><w:vMerge {}/></w:tcPr>{}</w:tc><w:tc><w:p><w:r><w:t>SIB{i}</w:t></w:r></w:p></w:tc></w:tr>"#,
            if i == 0 { "w:val=\"restart\"" } else { "" }, if i == 0 { owner.as_str() } else { "<w:p/>" }));
    }
    source_units(&format!(
        r#"<w:tbl><w:tblPr><w:tblCellMar><w:top w:w="40" w:type="dxa"/><w:bottom w:w="60" w:type="dxa"/></w:tblCellMar></w:tblPr><w:tblGrid><w:gridCol w:w="2000"/><w:gridCol w:w="2000"/></w:tblGrid>{rows}</w:tbl>"#
    ))
}

#[test]
fn merged_note_rows_keep_local_payload_across_immutable_trials() {
    let layout = NoteLayout::new(merged_units(38));
    let mut cursor = layout.start();
    let mut painted = Vec::new();
    for _ in 0..30 {
        if cursor.complete() {
            break;
        }
        let rejected = plan_fragment(&layout, &cursor, budget(35.0, 100.0, false)).unwrap();
        let plan = plan_fragment(&layout, &cursor, budget(100.0, 100.0, true)).unwrap();
        let repeat = plan_fragment(&layout, &cursor, budget(100.0, 100.0, true)).unwrap();
        assert_eq!(plan.used_height, repeat.used_height);
        assert!(plan.used_height <= 100.01, "{}", plan.used_height);
        assert!(!plan.placements.is_empty(), "{:?}", plan.outcome);
        drop(rejected);
        for placement in &plan.placements {
            if let NotePlacement::Row { merge_paint, .. } = placement {
                for paint in merge_paint {
                    assert!(paint.owner < plan.after.rows.len());
                    for item in &paint.items {
                        if let crate::office::pptx::scene::Item::Text(t) = item {
                            painted.push(t.text.clone());
                        }
                    }
                }
            }
        }
        cursor = plan.after;
    }
    assert!(cursor.complete());
    let text = painted.join("");
    let mut prior = 0;
    for i in 0..38 {
        let label = format!("MERGE{i:03}");
        assert_eq!(text.matches(&label).count(), 1, "{label}");
        let at = text.find(&label).unwrap();
        assert!(at >= prior);
        prior = at;
    }
}

#[test]
fn merged_anchor_only_slice_and_occurrence_identity_survive_rejected_trials() {
    let anchor_units = source_units(
        r##"<w:p><w:r><w:pict><v:shape xmlns:v="urn:schemas-microsoft-com:vml" id="box" type="#_x0000_t202" style="position:absolute;margin-left:0pt;margin-top:0pt;width:20pt;height:10pt"><v:textbox><w:txbxContent><w:p><w:r><w:t>ANCHOR</w:t></w:r></w:p></w:txbxContent></v:textbox></v:shape></w:pict></w:r></w:p>"##,
    );
    let Unit::Para(p) = &anchor_units[0] else {
        panic!()
    };
    let anchor = p
        .lines
        .iter()
        .flat_map(|l| &l.anchors)
        .next()
        .unwrap()
        .clone();
    let mut units = merged_units(38);
    let Unit::Row(last) = units.last_mut().unwrap() else {
        panic!()
    };
    let moved = last
        .cells
        .iter_mut()
        .find_map(|c| c.moved.as_mut())
        .unwrap();
    let source = Rc::make_mut(moved);
    let slice = &mut source.0[20];
    slice.items.clear();
    slice.anchors.push((anchor, 0.0, slice.y));
    slice.notes.push(987);
    let layout = NoteLayout::new(units);
    let first = layout.start();
    let other = layout.start();
    assert!(!first.same_occurrence(&other));
    assert!(first.same_occurrence(&first.clone()));
    let mut cursor = first.clone();
    let mut count = 0;
    for _ in 0..30 {
        if cursor.complete() {
            break;
        }
        let _rejected = plan_fragment(&layout, &cursor, budget(20.0, 100.0, false)).unwrap();
        let plan = plan_fragment(&layout, &cursor, budget(100.0, 100.0, true)).unwrap();
        assert!(plan.after.same_occurrence(&first));
        for placement in &plan.placements {
            if let NotePlacement::Row { merge_paint, .. } = placement {
                for paint in merge_paint {
                    count += paint.anchors.len();
                    if !paint.anchors.is_empty() {
                        assert_eq!(paint.notes, vec![987]);
                        assert_eq!(paint.anchor_keys, vec![21]);
                    }
                }
            }
        }
        cursor = plan.after;
    }
    assert!(cursor.complete());
    assert_eq!(count, 1);
}

#[test]
fn nested_merged_source_slices_make_progress_and_exact_rows_terminate() {
    for exact in [false, true] {
        let content = (0..30).map(|i| format!(r#"<w:p><w:pPr><w:widowControl w:val="0"/><w:spacing w:before="0" w:after="0" w:line="240" w:lineRule="exact"/></w:pPr><w:r><w:t>INNER{i:03}</w:t></w:r></w:p>"#)).collect::<String>();
        let nested = source_units(&format!(
            r#"<w:tbl><w:tblGrid><w:gridCol w:w="2000"/></w:tblGrid><w:tr>{}<w:tc>{content}</w:tc></w:tr></w:tbl>"#,
            if exact {
                r#"<w:trPr><w:trHeight w:val="7200" w:hRule="exact"/></w:trPr>"#
            } else {
                ""
            }
        ));
        let (slices, height) = stack(&nested);
        let mut units = merged_units(30);
        let Unit::Row(last) = units.last_mut().unwrap() else {
            panic!()
        };
        let moved = last
            .cells
            .iter_mut()
            .find_map(|c| c.moved.as_mut())
            .unwrap();
        *moved = Rc::new((slices, 0.0));
        last.height = height;
        let layout = NoteLayout::new(units);
        let mut cursor = layout.start();
        let mut text = String::new();
        let mut atomic = false;
        for _ in 0..20 {
            if cursor.complete() {
                break;
            }
            let mut b = budget(100.0, 100.0, true);
            b.allow_atomic_overflow = true;
            let plan = plan_fragment(&layout, &cursor, b).unwrap();
            atomic |= plan.outcome == FragmentOutcome::AtomicOverflow;
            assert!(!plan.placements.is_empty(), "{:?}", plan.outcome);
            if !exact {
                assert!(plan.used_height <= 100.01);
            }
            for placement in &plan.placements {
                if let NotePlacement::Row { merge_paint, .. } = placement {
                    for item in merge_paint.iter().flat_map(|p| &p.items) {
                        if let crate::office::pptx::scene::Item::Text(t) = item {
                            text.push_str(&t.text);
                        }
                    }
                }
            }
            cursor = plan.after;
        }
        assert!(cursor.complete());
        assert_eq!(atomic, exact);
        for i in 0..30 {
            assert_eq!(text.matches(&format!("INNER{i:03}")).count(), 1);
        }
    }
}

#[test]
fn repeated_headers_reduce_fresh_row_capacity_without_retry_loop() {
    let mut header = empty_row(20.0, 0.0);
    header.header = true;
    header.repeating_header = true;
    header.index = 0;
    let mut first = empty_row(10.0, 0.0);
    first.index = 1;
    let mut row = empty_row(90.0, 0.0);
    row.index = 2;
    row.cant_split = true;
    row.headers = Rc::new(vec![header.clone()]);
    let layout = NoteLayout::new(vec![Unit::Row(header), Unit::Row(first), Unit::Row(row)]);
    let prefix = plan_fragment(&layout, &layout.start(), budget(30.0, 100.0, true)).unwrap();
    assert_eq!(prefix.after.unit, 2);
    let next = plan_fragment(&layout, &prefix.after, budget(100.0, 100.0, true)).unwrap();
    assert_eq!(next.used_height, 100.0);
    assert_eq!(next.placements.len(), 2);
    let tail = plan_fragment(&layout, &next.after, budget(100.0, 100.0, true)).unwrap();
    assert!(tail.after.complete());
    assert_eq!(tail.used_height, 30.0);
}

#[test]
fn merged_nested_cant_split_uses_full_destination_capacity() {
    for count in [7, 12] {
        let ps = (0..count).map(|i| format!(r#"<w:p><w:pPr><w:widowControl w:val="0"/><w:spacing w:before="0" w:after="0" w:line="200" w:lineRule="exact"/></w:pPr><w:r><w:t>INNER{i:02}</w:t></w:r></w:p>"#)).collect::<String>();
        let nested = source_units(&format!(
            r#"<w:tbl><w:tblPr><w:tblCellMar><w:top w:w="0" w:type="dxa"/><w:bottom w:w="0" w:type="dxa"/></w:tblCellMar></w:tblPr><w:tblGrid><w:gridCol w:w="2000"/></w:tblGrid><w:tr><w:trPr><w:cantSplit/></w:trPr><w:tc>{ps}</w:tc></w:tr></w:tbl>"#
        ));
        let (slices, _) = stack(&nested);
        let mut units = merged_units(7);
        let Unit::Row(last) = units.last_mut().unwrap() else {
            panic!()
        };
        let moved = last
            .cells
            .iter_mut()
            .find_map(|c| c.moved.as_mut())
            .unwrap();
        *moved = Rc::new((slices, 0.0));
        let mut batch = merges::Batch::for_note(
            last,
            &[],
            0,
            0.0,
            30.0,
            merges::NoteHistory {
                fresh_capacity: 100.0,
                fresh_rows: &[],
                capacities: &[],
                env: None,
            },
        )
        .unwrap();
        batch.prepare_note(0, 0.0, 0.0, 0, false);
        assert!(!batch.preview(0.0, 30.0).progress);
        batch.prepare_note(1, 0.0, 0.0, 1, true);
        assert_eq!(batch.clone().preview(0.0, 30.0).progress, count == 12);
        assert!(batch.preview(0.0, 100.0).progress);
        assert_eq!(batch.consumed(), 0);
        let mut paint = batch.commit(0.0, 100.0, 1);
        if !batch.complete() {
            batch.prepare_note(2, 0.0, 0.0, 2, true);
            paint.extend(batch.commit(0.0, 100.0, 2));
        }
        assert!(batch.complete());
        let text = paint
            .iter()
            .flat_map(|p| &p.items)
            .filter_map(|i| match i {
                crate::office::pptx::scene::Item::Text(t) => Some(t.text.clone()),
                _ => None,
            })
            .collect::<String>();
        for i in 0..count {
            assert_eq!(text.matches(&format!("INNER{i:02}")).count(), 1);
        }
    }
}

#[test]
fn original_header_stays_with_first_data_fragment() {
    let mut header = empty_row(20.0, 0.0);
    header.header = true;
    let mut row = empty_row(90.0, 0.0);
    row.index = 1;
    row.cant_split = true;
    row.headers = Rc::new(vec![header.clone()]);
    let layout = NoteLayout::new(vec![Unit::Row(header), Unit::Row(row)]);
    let partial = plan_fragment(&layout, &layout.start(), budget(60.0, 100.0, false)).unwrap();
    assert!(partial.placements.is_empty());
    assert_eq!(partial.after.unit, 0);
    let first = plan_fragment(&layout, &layout.start(), budget(100.0, 100.0, true)).unwrap();
    assert_eq!(first.placements.len(), 2);
    assert_eq!(first.used_height, 100.0);
    let tail = plan_fragment(&layout, &first.after, budget(100.0, 100.0, true)).unwrap();
    assert!(tail.after.complete());
    assert_eq!(tail.used_height, 30.0);
}

#[test]
fn explicit_row_break_ends_initial_header_transaction() {
    let mut header = empty_row(20.0, 0.0);
    header.header = true;
    let mut row = empty_row(10.0, 0.0);
    row.index = 1;
    row.page_break_before = true;
    row.headers = Rc::new(vec![header.clone()]);
    let layout = NoteLayout::new(vec![Unit::Row(header), Unit::Row(row)]);
    let first = plan_fragment(&layout, &layout.start(), budget(100.0, 100.0, true)).unwrap();
    assert_eq!(first.outcome, FragmentOutcome::Break(BreakKind::Page));
    assert_eq!(first.after.unit, 1);
    assert_eq!(first.placements.len(), 1);
    let second = plan_fragment(&layout, &first.after, budget(100.0, 100.0, true)).unwrap();
    assert!(second.after.complete());
}

#[test]
fn occupied_picture_and_terminal_spacing_share_the_same_atomic_budget() {
    let mut p = paragraph(1, 12.0);
    p.after = 30.0;
    let mut layout = NoteLayout::new(vec![Unit::Para(p)]);
    layout.occupied = Rc::new(vec![vec![80.0]]);
    let mut partial = budget(70.0, 90.0, true);
    partial.allow_atomic_overflow = true;
    let plan = plan_fragment(&layout, &layout.start(), partial).unwrap();
    assert_eq!(plan.outcome, FragmentOutcome::Retry);
    assert!(plan.placements.is_empty());
    let plan = plan_fragment(&layout, &layout.start(), budget(90.0, 90.0, true)).unwrap();
    assert_eq!(plan.outcome, FragmentOutcome::Complete);
    assert_eq!(plan.used_height, 80.0);
}
