use super::*;

fn with_row(body: &str, check: impl FnOnce(RowUnit, &TextEnv<'_>)) {
    let opened = crate::office::docx::extract::open(
        crate::office::docx::testutil::docx(body, None, None),
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
    let units = Flow::new(&env, 15)
        .for_notes()
        .units(&opened.sections[0].blocks, 80.0, false);
    let Unit::Row(row) = &units[0] else {
        panic!("table")
    };
    check(row.clone(), &env);
}

#[test]
fn remeasured_cell_retains_additive_edges_and_remaining_minimum() {
    let body = r#"<w:tbl><w:tblPr><w:tblW w:w="5000" w:type="pct"/><w:tblCellSpacing w:w="60" w:type="dxa"/><w:tblBorders><w:top w:val="single" w:sz="16"/><w:bottom w:val="single" w:sz="16"/></w:tblBorders></w:tblPr><w:tblGrid><w:gridCol w:w="200"/></w:tblGrid><w:tr><w:trPr><w:trHeight w:val="7200" w:hRule="atLeast"/></w:trPr><w:tc><w:p><w:r><w:t>OWNER</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#;
    with_row(body, |row, env| {
        let reflow = remeasure_row(&row, 140.0, env).unwrap();
        assert!((reflow.height - row.height).abs() < 0.01);
        let (top, rest) = split_row_in_area(&row, 100.0, false, None, 200.0).unwrap();
        let after = remeasure_row(&rest, 140.0, env).unwrap();
        assert!((after.height - rest.height).abs() < 0.01);
        assert!(after.height < row.height - 90.0);
        assert_eq!(
            row.source.as_ref().unwrap().minimum,
            reflow.source.as_ref().unwrap().minimum
        );
        assert!(top.cells.iter().any(|cell| !cell.slices.is_empty()));
    });
    let content_driven = body.replace(
        "<w:trPr><w:trHeight w:val=\"7200\" w:hRule=\"atLeast\"/></w:trPr>",
        "",
    );
    with_row(&content_driven, |row, env| {
        let after = remeasure_row(&row, 140.0, env).unwrap();
        assert!((after.height - row.height).abs() < 0.01);
        assert!(after.source.as_ref().unwrap().addition > 0.0);
    });
}

#[test]
fn remeasured_cell_keeps_cell_text_context() {
    let body = r#"<w:tbl><w:tblPr><w:tblW w:w="5000" w:type="pct"/></w:tblPr><w:tblGrid><w:gridCol w:w="200"/></w:tblGrid><w:tr><w:tc><w:p><w:r><w:t>OWNER</w:t><w:br w:type="page"/><w:t>TAIL</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#;
    with_row(body, |row, env| {
        env.in_cell.set(false);
        assert!(!env.in_cell.get());
        let after = remeasure_row(&row, 140.0, env).unwrap();
        assert!(!env.in_cell.get());
        for cell in &after.cells {
            for slice in &cell.slices {
                let (source, _) = slice.paragraph_source.as_ref().unwrap();
                assert!(source.lines.iter().all(|line| line.break_after.is_none()));
            }
        }
    });
}

#[test]
fn percentage_note_grid_tracks_destination_in_both_directions() {
    for (percentage, fraction) in [(3750, 0.75), (5000, 1.0), (6250, 1.25)] {
        let body = format!(
            r#"<w:tbl><w:tblPr><w:tblW w:w="{percentage}" w:type="pct"/></w:tblPr><w:tblGrid><w:gridCol w:w="1400"/><w:gridCol w:w="1400"/></w:tblGrid><w:tr><w:tc><w:p/></w:tc><w:tc><w:p/></w:tc></w:tr></w:tbl>"#
        );
        with_row(&body, |row, env| {
            assert!((row.cells.iter().map(|c| c.w).sum::<f64>() - 80.0 * fraction).abs() < 0.01);
            let wide = remeasure_row(&row, 140.0, env).unwrap();
            assert!((wide.cells.iter().map(|c| c.w).sum::<f64>() - 140.0 * fraction).abs() < 0.01);
            let narrow = remeasure_row(&wide, 80.0, env).unwrap();
            assert!((narrow.cells.iter().map(|c| c.w).sum::<f64>() - 80.0 * fraction).abs() < 0.01);
        });
    }
}

#[test]
fn merge_identity_uses_logical_grid_interval() {
    let body = r#"<w:tbl><w:tblPr><w:tblW w:w="5000" w:type="pct"/></w:tblPr><w:tblGrid><w:gridCol w:w="400"/><w:gridCol w:w="400"/><w:gridCol w:w="400"/><w:gridCol w:w="400"/></w:tblGrid><w:tr><w:tc><w:tcPr><w:gridSpan w:val="2"/></w:tcPr><w:p/></w:tc><w:tc><w:tcPr><w:gridSpan w:val="2"/><w:vMerge w:val="restart"/></w:tcPr><w:p><w:r><w:t>OWNER</w:t></w:r></w:p></w:tc></w:tr><w:tr><w:tc><w:p/></w:tc><w:tc><w:p/></w:tc><w:tc><w:tcPr><w:gridSpan w:val="2"/><w:vMerge/></w:tcPr><w:p/></w:tc></w:tr></w:tbl>"#;
    with_row(body, |row, env| {
        assert_eq!(cell_interval(&row, 1), Some((2, 2)));
        let mut units = Vec::new();
        Flow::new(env, 15).for_notes().table(
            &row.source.as_ref().unwrap().table,
            140.0,
            true,
            &mut units,
        );
        let Unit::Row(last) = &units[1] else {
            panic!("continuation row")
        };
        assert_eq!(cell_in_interval(last, (2, 2)), Some(2));
        assert_eq!(cell_interval(last, 1), Some((1, 1)));
    });
}
