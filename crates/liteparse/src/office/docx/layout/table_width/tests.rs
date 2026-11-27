use super::*;

#[test]
fn overlapping_spans_share_feasible_width_independent_of_row_order() {
    let widths = ContentWidths {
        minimum: 180.0,
        maximum: 180.0,
        atomic: 180.0,
        unclippable: 180.0,
    };
    for constraints in [
        vec![
            Constraint {
                preferred_floor: 0.0,
                start: 0,
                end: 2,
                widths,
            },
            Constraint {
                preferred_floor: 0.0,
                start: 1,
                end: 3,
                widths,
            },
        ],
        vec![
            Constraint {
                preferred_floor: 0.0,
                start: 1,
                end: 3,
                widths,
            },
            Constraint {
                preferred_floor: 0.0,
                start: 0,
                end: 2,
                widths,
            },
        ],
    ] {
        assert!((feasible_grid(3, &constraints, 1.0).iter().sum::<f64>() - 180.0).abs() < 1e-6);
        let mut grid = [80.0, 80.0, 80.0];
        project(&mut grid, &constraints, 1.0, 240.0);
        for (actual, expected) in grid.iter().zip([60.0, 120.0, 60.0]) {
            assert!((actual - expected).abs() < 0.001);
        }
    }
}

#[test]
fn inline_minimum_borrows_available_width_without_changing_total() {
    let constraints = [Constraint {
        preferred_floor: 0.0,
        start: 1,
        end: 2,
        widths: ContentWidths {
            minimum: 120.0,
            maximum: 120.0,
            atomic: 120.0,
            unclippable: 120.0,
        },
    }];
    let mut grid = [80.0, 80.0, 80.0];
    project(&mut grid, &constraints, 1.0, 240.0);
    for (actual, expected) in grid.iter().zip([60.0, 120.0, 60.0]) {
        assert!((actual - expected).abs() < 0.001);
    }
}

#[test]
fn intrinsic_measurement_retains_cell_breaks_tabs_and_first_indent() {
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
        in_cell: std::cell::Cell::new(true),
        in_header: Default::default(),
        grid_in_table: false,
        no_leading: false,
        compat_mode: 15,
    };
    let measure = |body: &str| {
        let opened = crate::office::docx::extract::open(
            crate::office::docx::testutil::docx(body, None, None),
            &Default::default(),
        )
        .unwrap();
        let Block::Para(p) = &opened.sections[0].blocks[0] else {
            panic!("paragraph")
        };
        content_widths(p, 400.0, &env)
    };
    let plain = measure("<w:p><w:r><w:t>FIRSTSECOND</w:t></w:r></w:p>");
    for kind in ["page", "column"] {
        let measured = measure(&format!(
            r#"<w:p><w:r><w:t>FIRST</w:t><w:br w:type="{kind}"/><w:t>SECOND</w:t></w:r></w:p>"#
        ));
        assert!((measured.maximum - plain.maximum).abs() < 0.01);
    }
    let line = measure("<w:p><w:r><w:t>FIRST</w:t><w:br/><w:t>SECOND</w:t></w:r></w:p>");
    assert!(line.maximum < plain.maximum);
    let tab = measure(
        r#"<w:p><w:pPr><w:tabs><w:tab w:val="right" w:pos="2000"/></w:tabs></w:pPr><w:r><w:t>A</w:t><w:tab/><w:t>B</w:t></w:r></w:p>"#,
    );
    assert!((tab.maximum - 100.0).abs() < 0.01);
    let picture = measure(
        r#"<w:p><w:pPr><w:ind w:left="200" w:right="100" w:firstLine="400"/></w:pPr><w:r><w:drawing><wp:inline xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"><wp:extent cx="1524000" cy="254000"/></wp:inline></w:drawing></w:r></w:p>"#,
    );
    assert!((picture.minimum - 155.0).abs() < 0.01);
    assert!((picture.atomic - 155.0).abs() < 0.01);
    let hanging = measure(
        r#"<w:p><w:pPr><w:ind w:left="800" w:hanging="800"/></w:pPr><w:r><w:drawing><wp:inline xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"><wp:extent cx="1524000" cy="254000"/></wp:inline></w:drawing></w:r></w:p>"#,
    );
    assert!((hanging.atomic - 120.0).abs() < 0.01);
    assert!((hanging.minimum - 120.0).abs() < 0.01);
    for fixed in [false, true] {
        let layout = if fixed {
            r#"<w:tblLayout w:type="fixed"/>"#
        } else {
            ""
        };
        let body = format!(
            r#"<w:tbl><w:tblPr><w:tblW w:type="pct" w:w="5000"/>{layout}</w:tblPr><w:tblGrid><w:gridCol w:w="2000"/><w:gridCol w:w="2000"/></w:tblGrid><w:tr><w:tc><w:p><w:r><w:t>A</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>B</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#
        );
        let opened = crate::office::docx::extract::open(
            crate::office::docx::testutil::docx(&body, None, None),
            &Default::default(),
        )
        .unwrap();
        let measured = blocks_widths(&opened.sections[0].blocks, 200.0, &env);
        if fixed {
            assert!((measured.atomic - 200.0).abs() < 0.01);
        } else {
            assert!(measured.minimum < 100.0);
            assert!(measured.atomic <= measured.minimum);
        }
    }
}

#[test]
fn legacy_percentage_basis_uses_own_outer_margins_before_fraction() {
    for fraction in [0.5, 1.0, 1.25] {
        let source = format!(
            r#"<w:tbl><w:tblPr><w:tblW w:type="pct" w:w="{}"/><w:tblCellMar><w:left w:type="dxa" w:w="80"/><w:right w:type="dxa" w:w="240"/></w:tblCellMar></w:tblPr><w:tblGrid><w:gridCol w:w="2000"/><w:gridCol w:w="4000"/></w:tblGrid><w:tr><w:tc><w:p/></w:tc><w:tc><w:p/></w:tc></w:tr></w:tbl>"#,
            fraction * 5000.0
        );
        let opened = crate::office::docx::extract::open(
            crate::office::docx::testutil::docx(&source, None, None),
            &Default::default(),
        )
        .unwrap();
        let Block::Table(table) = &opened.sections[0].blocks[0] else {
            panic!("table")
        };
        for mode in [11, 14, 15] {
            // The supplied containing-cell width already excludes parent padding.
            for available in [240.0, 110.0] {
                let grid = initial_grid(table, available, mode);
                let expected = (available + if mode < 15 { 16.0 } else { 0.0 }) * fraction;
                assert!((grid.iter().sum::<f64>() - expected).abs() < 0.001);
                assert!((grid[1] / grid[0] - 2.0).abs() < 0.001);
            }
        }
    }
}

#[test]
fn nonpercentage_autofit_resolves_preferred_width_with_content_constraints() {
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
        in_cell: std::cell::Cell::new(true),
        in_header: Default::default(),
        grid_in_table: false,
        no_leading: false,
        compat_mode: 15,
    };

    for (kind, value, fixed, expected) in [
        ("dxa", "12000", false, 600.0),
        ("auto", "0", false, 240.0),
        ("dxa", "4400", false, 220.0),
        ("dxa", "12000", true, 600.0),
        ("pct", "6250", false, 300.0),
    ] {
        let layout = if fixed {
            r#"<w:tblLayout w:type="fixed"/>"#
        } else {
            ""
        };
        let source = format!(
            r#"<w:tbl><w:tblPr><w:tblW w:type="{kind}" w:w="{value}"/>{layout}</w:tblPr><w:tblGrid><w:gridCol w:w="4000"/><w:gridCol w:w="4000"/><w:gridCol w:w="4000"/></w:tblGrid><w:tr><w:tc><w:p><w:r><w:t>LEFT</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:drawing><wp:inline xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"><wp:extent cx="1524000" cy="254000"/></wp:inline></w:drawing></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>RIGHT</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#
        );
        let mut opened = crate::office::docx::extract::open(
            crate::office::docx::testutil::docx(&source, None, None),
            &Default::default(),
        )
        .unwrap();
        let Block::Table(table) = &mut opened.sections[0].blocks[0] else {
            panic!("table")
        };
        for row in &mut table.rows {
            for cell in &mut row.cells {
                for block in &mut cell.blocks {
                    if let Block::Para(p) = block {
                        for inline in &mut p.inlines {
                            if let crate::office::docx::model::Inline::Picture(pic, _) = inline {
                                pic.part = Some("word/media/test.png".into());
                            }
                        }
                    }
                }
            }
        }
        let mut grid = initial_grid(table, 240.0, 15);
        autofit(table, &mut grid, 240.0, &env);
        assert!(
            (grid.iter().sum::<f64>() - expected).abs() < 0.01,
            "{kind}/{fixed}: {grid:?}"
        );
        assert!(
            grid[1] >= 130.8 - 0.01,
            "source image and cell margins: {grid:?}"
        );
    }
}

#[test]
fn nonpercentage_autofit_keeps_proportions_and_bounds_oversized_images() {
    let fonts = crate::office::pptx::fonts::FontSystem::new(&[], true, &Default::default());
    let mut env = TextEnv {
        fonts: &fonts,
        default_tab: 36.0,
        fields: Default::default(),
        grid: None,
        char_grid: Default::default(),
        expand_shift_return: false,
        mark_in_last_line: true,
        html_spacing: true,
        grow_autofit: false,
        in_cell: std::cell::Cell::new(true),
        in_header: Default::default(),
        grid_in_table: false,
        no_leading: false,
        compat_mode: 15,
    };

    for oversized in [false, true] {
        let content = if oversized {
            r#"<w:r><w:drawing><wp:inline xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"><wp:extent cx="4572000" cy="254000"/></wp:inline></w:drawing></w:r>"#
        } else {
            ""
        };
        let source = format!(
            r#"<w:tbl><w:tblPr><w:tblW w:type="dxa" w:w="2400"/></w:tblPr><w:tblGrid><w:gridCol w:w="3600"/><w:gridCol w:w="1200"/></w:tblGrid><w:tr><w:tc><w:p>{content}</w:p></w:tc><w:tc><w:p/></w:tc></w:tr></w:tbl>"#
        );
        let source = if oversized {
            source.replace(
                "<w:tc><w:p/></w:tc>",
                "<w:tc><w:p><w:r><w:t>NEIGHBOR</w:t></w:r></w:p></w:tc>",
            )
        } else {
            source
        };
        let mut opened = crate::office::docx::extract::open(
            crate::office::docx::testutil::docx(&source, None, None),
            &Default::default(),
        )
        .unwrap();
        let Block::Table(table) = &mut opened.sections[0].blocks[0] else {
            panic!("table")
        };
        for row in &mut table.rows {
            for cell in &mut row.cells {
                for block in &mut cell.blocks {
                    if let Block::Para(p) = block {
                        for inline in &mut p.inlines {
                            if let crate::office::docx::model::Inline::Picture(pic, _) = inline {
                                pic.part = Some("word/media/test.png".into());
                            }
                        }
                    }
                }
            }
        }
        for grow in [false, true] {
            env.grow_autofit = grow;
            let mut grid = initial_grid(table, 240.0, 15);
            autofit(table, &mut grid, 240.0, &env);
            if !oversized {
                assert!((grid[0] - 90.0).abs() < 0.01 && (grid[1] - 30.0).abs() < 0.01);
            } else if grow {
                assert!(grid[0] >= 370.8 - 0.01);
            } else {
                assert!(grid.iter().sum::<f64>() <= 240.01);
                let bounds = constraints(table, &grid, &env);
                let neighbor = bounds.iter().find(|c| c.start == 1).unwrap();
                assert!(
                    grid[1] > neighbor.widths.unclippable + 1.0,
                    "joint relaxation retains more than a glyph-only neighbor: {grid:?}"
                );
            }
        }
    }
}

#[test]
fn explicit_preferred_width_and_cell_widths_can_span_margins() {
    for (properties, cell, expected) in [
        (r#"<w:tblW w:type="dxa" w:w="11680"/>"#, "", 584.0),
        (
            r#"<w:tblW w:type="auto" w:w="0"/>"#,
            r#"<w:tcPr><w:tcW w:type="dxa" w:w="11680"/></w:tcPr>"#,
            468.0,
        ),
        (r#"<w:tblW w:type="dxa" w:w="2400"/>"#, "", 120.0),
    ] {
        let source = format!(
            "<w:tbl><w:tblPr>{properties}</w:tblPr><w:tblGrid><w:gridCol w:w=\"3600\"/><w:gridCol w:w=\"1200\"/></w:tblGrid><w:tr><w:tc>{cell}<w:p/></w:tc><w:tc><w:p/></w:tc></w:tr></w:tbl>"
        );
        let opened = crate::office::docx::extract::open(
            crate::office::docx::testutil::docx(&source, None, None),
            &Default::default(),
        )
        .unwrap();
        let Block::Table(table) = &opened.sections[0].blocks[0] else {
            panic!("table")
        };
        let mut grid = initial_grid(table, 468.0, 15);
        let target = preferred_grid(table, &mut grid, 468.0);
        assert!((target - expected).abs() < 0.001);
        if expected == 120.0 {
            assert!((grid[0] - 90.0).abs() < 0.001);
        }
    }
}

#[test]
fn automatic_cell_preferences_share_the_resolved_grid_envelope() {
    for (saved, cells, width, expected) in [
        ([218.25, 213.75], [221.4, 221.4], 432.0, [218.25, 213.75]),
        ([250.0, 263.9], [250.0, 263.9], 504.0, [250.0, 263.9]),
        ([180.0, 60.0], [360.0, 120.0], 300.0, [225.0, 75.0]),
        ([300.0, 300.0], [300.0, 300.0], 240.0, [300.0, 300.0]),
    ] {
        let source = format!(
            r#"<w:tbl><w:tblPr><w:tblW w:type="auto" w:w="0"/></w:tblPr><w:tblGrid><w:gridCol w:w="{}"/><w:gridCol w:w="{}"/></w:tblGrid><w:tr><w:tc><w:tcPr><w:tcW w:type="dxa" w:w="{}"/></w:tcPr><w:p/></w:tc><w:tc><w:tcPr><w:tcW w:type="dxa" w:w="{}"/></w:tcPr><w:p/></w:tc></w:tr></w:tbl>"#,
            saved[0] * 20.0,
            saved[1] * 20.0,
            cells[0] * 20.0,
            cells[1] * 20.0,
        );
        let opened = crate::office::docx::extract::open(
            crate::office::docx::testutil::docx(&source, None, None),
            &Default::default(),
        )
        .unwrap();
        let Block::Table(table) = &opened.sections[0].blocks[0] else {
            panic!("table")
        };
        let mut grid = initial_grid(table, width, 15);
        preferred_grid(table, &mut grid, width);
        for (actual, expected) in grid.iter().zip(expected) {
            assert!((actual - expected).abs() < 0.001, "{grid:?}");
        }
    }
}

#[test]
fn no_wrap_dxa_preferences_protect_descriptive_columns_before_word_growth() {
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
        in_cell: std::cell::Cell::new(true),
        in_header: Default::default(),
        grid_in_table: false,
        no_leading: false,
        compat_mode: 15,
    };
    let saved = [25.35, 67.65, 68.7, 81.25, 122.15, 67.65, 54.05];
    let texts = [
        "IDX",
        "IDENTIFIER12345678901234567890",
        "REFERENCE12345678901234567890",
        "DATE",
        "A descriptive title with ordinary breakable words and source details",
        "REFERENCE12345678901234567890",
        "END",
    ];
    for (no_wrap, fixed) in [(true, false), (false, false), (true, true)] {
        let grid_xml: String = saved
            .iter()
            .map(|w| format!(r#"<w:gridCol w:w="{}"/>"#, w * 20.0))
            .collect();
        let cells: String = saved.iter().zip(texts).map(|(w, text)| format!(r#"<w:tc><w:tcPr><w:tcW w:type="dxa" w:w="{}"/><w:noWrap w:val="{}"/></w:tcPr><w:p><w:r><w:t>{text}</w:t></w:r></w:p></w:tc>"#, w * 20.0, u8::from(no_wrap))).collect();
        let layout = if fixed {
            r#"<w:tblLayout w:type="fixed"/>"#
        } else {
            ""
        };
        let source = format!(
            r#"<w:tbl><w:tblPr><w:tblW w:type="auto" w:w="0"/>{layout}</w:tblPr><w:tblGrid>{grid_xml}</w:tblGrid><w:tr>{cells}</w:tr></w:tbl>"#
        );
        let opened = crate::office::docx::extract::open(
            crate::office::docx::testutil::docx(&source, None, None),
            &Default::default(),
        )
        .unwrap();
        let Block::Table(table) = &opened.sections[0].blocks[0] else {
            panic!("table")
        };
        let width = saved.iter().sum();
        let mut grid = initial_grid(table, width, 15);
        assert!(
            constraints(table, &grid, &env)
                .iter()
                .all(|c| c.widths.maximum >= c.widths.minimum)
        );
        autofit(table, &mut grid, width, &env);
        if no_wrap || fixed {
            assert!(
                grid.iter().zip(saved).all(|(a, b)| (a - b).abs() < 0.001),
                "{grid:?}"
            );
        } else {
            assert!(
                grid[4] < saved[4] - 1.0,
                "ordinary word negotiation stays active: {grid:?}"
            );
        }
    }
}

#[test]
fn no_wrap_priority_floors_yield_after_other_cells_reach_atomic_widths() {
    let make = |start, end, atomic, minimum, preferred_floor| Constraint {
        start,
        end,
        preferred_floor,
        widths: ContentWidths {
            atomic,
            minimum,
            maximum: minimum,
            unclippable: atomic,
        },
    };
    let mut feasible = vec![make(0, 1, 10.0, 100.0, 100.0), make(1, 2, 20.0, 150.0, 0.0)];
    protect_preferred_floors(2, &mut feasible, 140.0);
    assert_eq!(feasible[0].widths.atomic, 100.0);
    assert_eq!(feasible[1].widths.atomic, 20.0);
    let mut constrained = vec![make(0, 2, 20.0, 180.0, 180.0), make(2, 3, 80.0, 120.0, 0.0)];
    protect_preferred_floors(3, &mut constrained, 240.0);
    assert!((constrained[0].widths.atomic - 160.0).abs() < 0.001);
    assert_eq!(constrained[1].widths.atomic, 80.0);
}

#[test]
fn no_wrap_automatic_and_percentage_cells_measure_explicit_lines() {
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
        in_cell: std::cell::Cell::new(true),
        in_header: Default::default(),
        grid_in_table: false,
        no_leading: false,
        compat_mode: 15,
    };
    for width in [
        r#"<w:tcW w:type="auto" w:w="0"/>"#,
        r#"<w:tcW w:type="pct" w:w="5000"/>"#,
    ] {
        for no_wrap in [false, true] {
            let source = format!(
                r#"<w:tbl><w:tblGrid><w:gridCol w:w="2400"/></w:tblGrid><w:tr><w:tc><w:tcPr>{width}<w:noWrap w:val="{}"/></w:tcPr><w:p><w:r><w:t>alpha beta gamma delta</w:t><w:br/><w:t>tail</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#,
                u8::from(no_wrap)
            );
            let opened = crate::office::docx::extract::open(
                crate::office::docx::testutil::docx(&source, None, None),
                &Default::default(),
            )
            .unwrap();
            let Block::Table(table) = &opened.sections[0].blocks[0] else {
                panic!("table")
            };
            let grid = initial_grid(table, 120.0, 15);
            let measured = constraints(table, &grid, &env);
            let widths = measured[0].widths;
            if no_wrap {
                assert!((widths.minimum - widths.maximum).abs() < 0.001);
            } else {
                assert!(widths.minimum < widths.maximum - 10.0);
            }
            assert_eq!(measured[0].preferred_floor, 0.0);
        }
    }
}

fn width_bound(start: usize, end: usize, minimum: f64) -> Constraint {
    Constraint {
        start,
        end,
        preferred_floor: 0.0,
        widths: ContentWidths {
            minimum,
            maximum: minimum,
            atomic: minimum,
            unclippable: minimum,
        },
    }
}

#[test]
fn proportional_redistribution_retains_short_label_line_capacity() {
    let preferred = [66.5, 140.15, 53.9, 139.25, 34.35, 62.3];
    let minima = [48.32, 10.8, 52.47, 10.8, 51.327, 10.8];
    let constraints: Vec<_> = minima
        .iter()
        .enumerate()
        .map(|(i, min)| width_bound(i, i + 1, *min))
        .collect();
    let mut grid = preferred;
    project(&mut grid, &constraints, 1.0, 496.45);
    assert!((grid.iter().sum::<f64>() - 496.45).abs() < 1e-6);
    for (width, minimum) in grid.iter().zip(minima) {
        assert!(*width + 1e-6 >= minimum);
    }
    assert!((grid[0] - 63.967232974).abs() < 0.001);
    assert!(grid[0] - 10.8 > 52.461);
    let ratio = grid[0] / preferred[0];
    for i in [1, 3, 5] {
        assert!((grid[i] / preferred[i] - ratio).abs() < 1e-6);
    }
}

#[test]
fn weighted_spanning_projection_is_order_independent_and_scale_invariant() {
    let constraints = vec![width_bound(0, 2, 180.0), width_bound(1, 3, 180.0)];
    let mut reversed = constraints.clone();
    reversed.reverse();
    let mut expected = [40.0, 100.0, 100.0];
    project(&mut expected, &constraints, 1.0, 240.0);
    for scale in [0.1, 1.0, 10.0] {
        let mut grid = [40.0 * scale, 100.0 * scale, 100.0 * scale];
        let scaled: Vec<_> = reversed
            .iter()
            .map(|c| width_bound(c.start, c.end, c.widths.minimum * scale))
            .collect();
        project(&mut grid, &scaled, 1.0, 240.0 * scale);
        assert!(grid[0] + grid[1] + 1e-6 >= 180.0 * scale);
        assert!(grid[1] + grid[2] + 1e-6 >= 180.0 * scale);
        for (actual, expected) in grid.iter().zip(expected) {
            assert!((*actual / scale - expected).abs() < 0.001);
        }
    }
}

#[test]
fn weighted_projection_gives_zero_preference_columns_finite_content_width() {
    for mut grid in [[0.0, 120.0, 120.0], [0.0, 0.0, 0.0]] {
        let constraints = [width_bound(0, 1, 80.0), width_bound(1, 3, 160.0)];
        project(&mut grid, &constraints, 1.0, 240.0);
        assert!(grid.iter().all(|w| w.is_finite() && *w >= 0.0));
        assert!(grid[0] + 1e-6 >= 80.0);
        assert!(grid[1] + grid[2] + 1e-6 >= 160.0);
        assert!((grid.iter().sum::<f64>() - 240.0).abs() < 1e-6);
    }
    project(&mut [], &[], 1.0, 0.0);
}
