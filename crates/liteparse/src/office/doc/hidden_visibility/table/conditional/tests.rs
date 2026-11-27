use super::*;
fn property(sprm: u16, operand: &[u8]) -> PropertyModifier {
    PropertyModifier {
        sprm,
        operand: operand.to_vec(),
    }
}
fn cell(style: &Style, row: usize, column: usize, flags: u16) -> CellText {
    style
        .cell(
            [10, 20],
            roles::Position {
                row,
                column,
                columns: 2,
                headers: &[false, false, false],
                right_to_left: false,
                preceding_non_header_rows: row.saturating_sub(1),
            },
            flags,
            &mut 1_000_000,
        )
        .unwrap()
}
#[test]
fn corner_overrides_row_and_column_but_direct_caps_still_win() {
    let style = Style {
        ancestors: vec![11],
        row_band: None,
        column_band: None,
        global: vec![Applied {
            style_index: 11,
            condition: None,
            property: property(0x0836, &[1]),
        }],
        branches: vec![
            Branch {
                condition: 0x200,
                style: 11,
                properties: vec![property(0x0835, &[0])],
            },
            Branch {
                condition: 1,
                style: 11,
                properties: vec![property(0x083b, &[1]), property(0x0835, &[1])],
            },
            Branch {
                condition: 4,
                style: 11,
                properties: vec![property(0x0836, &[0])],
            },
        ],
    };
    let corner = cell(&style, 0, 0, 0x1e0);
    assert_eq!(corner.conditions, [4, 1, 0x200]);
    assert!(!corner.state.bold);
    assert!(!corner.state.italic);
    assert!(corner.state.all_caps);
    assert!(!cell(&style, 1, 1, 0x1e0).state.all_caps);
    let mut sheet = crate::office::doc::hidden_visibility::tests::sheet();
    let resolve = |sheet: &Stylesheet, direct: &[PropertyModifier]| {
        crate::office::doc::hidden_visibility::resolve_capitalization_toggle(
            sheet,
            0,
            Some(corner.state.all_caps),
            direct,
            &mut 1_000_000,
            0x083b,
        )
        .unwrap()
        .hidden
    };
    assert_eq!(resolve(&sheet, &[]), Some(true));
    assert_eq!(resolve(&sheet, &[property(0x083b, &[0])]), Some(false));
    sheet.styles[0].as_mut().unwrap().properties[0].bytes = vec![0x3b, 8, 0];
    assert_eq!(resolve(&sheet, &[]), Some(false));
}
#[test]
fn bands_exclude_repeated_headers_and_logical_first_column() {
    let roles = roles::conditions(
        roles::Position {
            row: 3,
            column: 0,
            columns: 3,
            headers: &[false, true, false, false, false],
            right_to_left: true,
            preceding_non_header_rows: 1,
        },
        0xa0,
        Some(1),
        Some(1),
    )
    .unwrap();
    assert_eq!(roles, [0x80, 0x20]);
    let one = roles::conditions(
        roles::Position {
            row: 0,
            column: 0,
            columns: 1,
            headers: &[false],
            right_to_left: false,
            preceding_non_header_rows: 0,
        },
        0x1e0,
        Some(1),
        Some(1),
    )
    .unwrap();
    assert_eq!(one, [4, 1, 0x200]);
}
#[test]
fn malformed_condition_and_relative_toggle_remain_guarded() {
    assert!(cnf(&[2, 3, 0]).is_err());
    assert!(cnf(&[3, 1, 0]).is_err());
    assert!(apply(&mut TextState::default(), &property(0x0835, &[129])).is_err());
}
