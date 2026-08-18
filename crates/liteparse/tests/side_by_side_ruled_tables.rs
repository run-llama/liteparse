use liteparse::config::ImageMode;
use liteparse::output::markdown::format_markdown;
use liteparse::projection::project_pages_to_grid;
use liteparse::types::{GraphicPrimitive, Page, TextItem};

fn text_item(text: &str, x: f32, y: f32, width: f32, size: f32, bold: bool) -> TextItem {
    TextItem {
        text: text.to_string(),
        x,
        y,
        width,
        height: size * 1.116,
        font_name: Some(if bold {
            "LiberationSans-Bold".to_string()
        } else {
            "LiberationSans".to_string()
        }),
        font_size: Some(size),
        font_height: Some(size),
        ..TextItem::default()
    }
}

fn stroke(x1: f32, y1: f32, x2: f32, y2: f32) -> GraphicPrimitive {
    GraphicPrimitive::Stroke {
        x1,
        y1,
        x2,
        y2,
        color: Some("ff000000".to_string()),
        width: 0.5,
    }
}

fn ruled_grid(xs: &[f32], ys: &[f32]) -> Vec<GraphicPrimitive> {
    let mut graphics = Vec::new();
    for &y in ys {
        graphics.push(stroke(xs[0], y, *xs.last().unwrap(), y));
    }
    for &x in xs {
        graphics.push(stroke(x, ys[0], x, *ys.last().unwrap()));
    }
    graphics
}

fn markdown_for(text_items: Vec<TextItem>, graphics: Vec<GraphicPrimitive>) -> String {
    let pages = project_pages_to_grid(vec![Page {
        page_number: 1,
        page_width: 792.0,
        page_height: 612.0,
        content_bounds: None,
        text_items,
        graphics,
        vector_graphics: None,
        struct_nodes: Vec::new(),
        image_refs: Vec::new(),
        annotations: None,
        form_fields: None,
        structure_tree: None,
    }]);
    format_markdown(&pages, &[], ImageMode::Off)
}

#[test]
fn side_by_side_ruled_tables_keep_their_own_headings_in_markdown() {
    let mut text_items = vec![
        text_item("Public Release Reference", 274.4, 50.4, 243.26, 20.0, true),
        text_item(
            "A public, synthetic document for evaluating side-by-side table extraction",
            237.45,
            78.7,
            317.13,
            10.0,
            false,
        ),
        text_item(
            "This document contains generic release-planning examples only.",
            50.5,
            105.35,
            350.0,
            10.0,
            false,
        ),
        text_item("Release channels", 50.5, 129.99, 100.57, 12.0, true),
        text_item("Support windows", 406.9, 130.99, 99.88, 12.0, true),
    ];

    for (y, values) in [
        (154.655, ["Channel", "Status", "Owner"]),
        (174.705, ["Stable", "Ready", "Team A"]),
        (194.755, ["Beta", "Testing", "Team B"]),
        (214.805, ["Nightly", "Active", "Team C"]),
    ] {
        for ((text, x), width) in values
            .into_iter()
            .zip([50.5, 118.5, 180.5])
            .zip([35.46, 29.5, 32.0])
        {
            text_items.push(text_item(text, x, y, width, 9.0, y == 154.655));
        }
    }

    for (y, values) in [
        (155.655, ["Region", "Window", "Contact"]),
        (175.705, ["East", "Morning", "Desk 1"]),
        (195.755, ["West", "Afternoon", "Desk 2"]),
        (215.805, ["Central", "Evening", "Desk 3"]),
    ] {
        for ((text, x), width) in values
            .into_iter()
            .zip([406.9, 474.9, 536.9])
            .zip([35.0, 39.0, 34.0])
        {
            text_items.push(text_item(text, x, y, width, 9.0, y == 155.655));
        }
    }

    let mut graphics = ruled_grid(
        &[44.0, 112.0, 174.0, 236.0],
        &[148.0, 168.0, 188.0, 208.0, 228.0],
    );
    graphics.extend(ruled_grid(
        &[400.0, 468.0, 530.0, 594.0],
        &[149.0, 169.0, 189.0, 209.0, 229.0],
    ));

    let markdown = markdown_for(text_items, graphics);
    let expected = "# Public Release Reference\n\n\
A public, synthetic document for evaluating side-by-side table extraction\n\n\
This document contains generic release-planning examples only.\n\n\
## Release channels\n\n\
| Channel | Status | Owner |\n\
|---|---|---|\n\
| Stable | Ready | Team A |\n\
| Beta | Testing | Team B |\n\
| Nightly | Active | Team C |\n\n\
## Support windows\n\n\
| Region | Window | Contact |\n\
|---|---|---|\n\
| East | Morning | Desk 1 |\n\
| West | Afternoon | Desk 2 |\n\
| Central | Evening | Desk 3 |";

    assert_eq!(markdown, expected);
}

#[test]
fn one_wide_ruled_table_remains_one_table() {
    let mut text_items = vec![text_item(
        "Quarterly schedule",
        50.0,
        80.0,
        130.0,
        14.0,
        true,
    )];
    for (y, values) in [
        (124.0, ["Quarter", "Status", "Owner"]),
        (148.0, ["Q1", "Ready", "Team A"]),
        (172.0, ["Q2", "Testing", "Team B"]),
    ] {
        for ((text, x), width) in values
            .into_iter()
            .zip([56.0, 260.0, 464.0])
            .zip([60.0, 70.0, 70.0])
        {
            text_items.push(text_item(text, x, y, width, 10.0, y == 124.0));
        }
    }
    let graphics = ruled_grid(&[50.0, 250.0, 454.0, 660.0], &[116.0, 140.0, 164.0, 188.0]);

    let markdown = markdown_for(text_items, graphics);
    let expected = "# Quarterly schedule\n\n\
---\n\n\
| Quarter | Status | Owner |\n\
|---|---|---|\n\
| Q1 | Ready | Team A |\n\
| Q2 | Testing | Team B |";

    assert_eq!(markdown, expected);
}

#[test]
fn vertically_stacked_ruled_tables_keep_top_to_bottom_order() {
    let mut text_items = vec![
        text_item("Operations handbook", 250.0, 35.0, 210.0, 20.0, true),
        text_item("Release status", 56.0, 80.0, 100.0, 14.0, true),
        text_item("Support status", 56.0, 240.0, 105.0, 14.0, true),
    ];
    for (y, values) in [
        (108.0, ["Channel", "Status"]),
        (132.0, ["Stable", "Ready"]),
        (156.0, ["Beta", "Testing"]),
        (268.0, ["Region", "Window"]),
        (292.0, ["East", "Morning"]),
        (316.0, ["West", "Afternoon"]),
    ] {
        for ((text, x), width) in values.into_iter().zip([56.0, 206.0]).zip([80.0, 90.0]) {
            text_items.push(text_item(text, x, y, width, 10.0, y == 108.0 || y == 268.0));
        }
    }

    let mut graphics = ruled_grid(&[50.0, 200.0, 350.0], &[100.0, 124.0, 148.0, 172.0]);
    graphics.extend(ruled_grid(
        &[50.0, 200.0, 350.0],
        &[260.0, 284.0, 308.0, 332.0],
    ));

    let markdown = markdown_for(text_items, graphics);
    let expected = "# Operations handbook\n\n\
## Release status\n\n\
| Channel | Status |\n\
|---|---|\n\
| Stable | Ready |\n\
| Beta | Testing |\n\n\
---\n\n\
### Support status\n\n\
| Region | Window |\n\
|---|---|\n\
| East | Morning |\n\
| West | Afternoon |";

    assert_eq!(markdown, expected);
}

#[test]
fn ordinary_two_column_prose_is_not_rendered_as_a_table() {
    let mut text_items = vec![text_item(
        "Two-column article",
        270.0,
        40.0,
        180.0,
        20.0,
        true,
    )];
    for row in 0..5 {
        let y = 110.0 + row as f32 * 18.0;
        text_items.push(text_item(
            &format!("Left paragraph sentence number {} continues.", row + 1),
            50.0,
            y,
            270.0,
            10.0,
            false,
        ));
        text_items.push(text_item(
            &format!("Right paragraph sentence number {} continues.", row + 1),
            430.0,
            y + 0.8,
            270.0,
            10.0,
            false,
        ));
    }

    let markdown = markdown_for(text_items, Vec::new());

    assert!(!markdown.contains("|---"), "unexpected table:\n{markdown}");
}

#[test]
fn spanning_heading_above_side_by_side_tables_remains_spanning() {
    let mut text_items = vec![text_item(
        "Shared service matrix",
        250.0,
        80.0,
        290.0,
        18.0,
        true,
    )];
    for (y, left, right) in [
        (
            124.0,
            ["Channel", "Status", "Owner"],
            ["Region", "Window", "Contact"],
        ),
        (
            148.0,
            ["Stable", "Ready", "Team A"],
            ["East", "Morning", "Desk 1"],
        ),
        (
            172.0,
            ["Beta", "Testing", "Team B"],
            ["West", "Afternoon", "Desk 2"],
        ),
        (
            196.0,
            ["Nightly", "Active", "Team C"],
            ["Central", "Evening", "Desk 3"],
        ),
    ] {
        for ((text, x), width) in left
            .into_iter()
            .zip([56.0, 136.0, 216.0])
            .zip([60.0, 65.0, 65.0])
        {
            text_items.push(text_item(text, x, y, width, 10.0, y == 124.0));
        }
        for ((text, x), width) in right
            .into_iter()
            .zip([426.0, 506.0, 586.0])
            .zip([60.0, 65.0, 65.0])
        {
            text_items.push(text_item(text, x, y + 0.8, width, 10.0, y == 124.0));
        }
    }

    let mut graphics = ruled_grid(
        &[50.0, 130.0, 210.0, 290.0],
        &[116.0, 140.0, 164.0, 188.0, 212.0],
    );
    graphics.extend(ruled_grid(
        &[420.0, 500.0, 580.0, 660.0],
        &[116.8, 140.8, 164.8, 188.8, 212.8],
    ));

    let markdown = markdown_for(text_items, graphics);
    let expected = "# Shared service matrix\n\n\
---\n\n\
| Channel | Status | Owner |\n\
|---|---|---|\n\
| Stable | Ready | Team A |\n\
| Beta | Testing | Team B |\n\
| Nightly | Active | Team C |\n\n\
| Region | Window | Contact |\n\
|---|---|---|\n\
| East | Morning | Desk 1 |\n\
| West | Afternoon | Desk 2 |\n\
| Central | Evening | Desk 3 |";

    assert_eq!(markdown, expected);
}

#[test]
fn spanning_line_between_rows_does_not_split_side_by_side_tables() {
    // A page-spanning note sitting vertically between the data rows of two
    // side-by-side grids belongs to neither one, so it has no grid rank to
    // sort by. Reordering the tables around it must not strand it mid-table:
    // every row of both grids has to survive as table content.
    let mut text_items = vec![text_item(
        "Shared service matrix",
        250.0,
        80.0,
        290.0,
        18.0,
        true,
    )];
    for row in 0..6 {
        let y = 124.0 + row as f32 * 24.0;
        for ((text, x), width) in [format!("L{row}a"), format!("L{row}b"), format!("L{row}c")]
            .into_iter()
            .zip([56.0, 136.0, 216.0])
            .zip([60.0, 65.0, 65.0])
        {
            text_items.push(text_item(&text, x, y, width, 10.0, row == 0));
        }
        for ((text, x), width) in [format!("R{row}a"), format!("R{row}b"), format!("R{row}c")]
            .into_iter()
            .zip([426.0, 506.0, 586.0])
            .zip([60.0, 65.0, 65.0])
        {
            text_items.push(text_item(&text, x, y + 0.8, width, 10.0, row == 0));
        }
    }
    text_items.push(text_item(
        "Note: spanning remark placed after the second data row of both tables.",
        56.0,
        184.0,
        604.0,
        10.0,
        false,
    ));

    let ys = [116.0, 140.0, 164.0, 188.0, 212.0, 236.0, 260.0];
    let mut graphics = ruled_grid(&[50.0, 130.0, 210.0, 290.0], &ys);
    graphics.extend(ruled_grid(
        &[420.0, 500.0, 580.0, 660.0],
        &ys.map(|y| y + 0.8),
    ));

    let markdown = markdown_for(text_items, graphics);
    for row in 0..6 {
        for label in [format!("L{row}a"), format!("R{row}a")] {
            assert!(
                markdown
                    .lines()
                    .any(|line| line.starts_with('|') && line.contains(&label)),
                "{label} is not table content:\n{markdown}"
            );
        }
    }
}
