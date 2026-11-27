use super::*;
use crate::office::doc::render_model::tables::CellLayout;

fn row(cells: &[(i16, i16, u16)]) -> RowLayout {
    RowLayout {
        cells: cells
            .iter()
            .map(|&(left, right, merge)| CellLayout {
                edges_twips: [left, right],
                borders: Default::default(),
                preferred_width: None,
                margins_twips: [0; 4],
                vertical_alignment: 0,
                raw_flags: merge << 5,
                no_wrap: false,
                shading_rgb: None,
            })
            .collect(),
        ..Default::default()
    }
}

#[test]
fn continuation_matches_interval_even_when_physical_column_index_changes() {
    let rows = [
        row(&[(0, 200, 3), (200, 400, 0)]),
        row(&[(-100, 0, 0), (0, 200, 1), (200, 400, 0)]),
        row(&[(0, 200, 1), (200, 400, 0)]),
    ];
    let owners = vertical_owners(&rows).unwrap();
    assert_eq!(owners[1][1], DisplayOwner { row: 0, cell: 0 });
    assert_eq!(owners[2][0], DisplayOwner { row: 0, cell: 0 });
    assert_eq!(owners[1][2], DisplayOwner { row: 1, cell: 2 });
}

#[test]
fn changed_interval_or_interrupted_chain_cannot_borrow_an_earlier_owner() {
    assert!(vertical_owners(&[row(&[(0, 200, 3)]), row(&[(0, 201, 1)])]).is_err());
    assert!(
        vertical_owners(&[
            row(&[(0, 200, 3)]),
            row(&[(0, 200, 0)]),
            row(&[(0, 200, 1)])
        ])
        .is_err()
    );
    assert!(vertical_owners(&[row(&[(0, 200, 1)])]).is_err());
}

#[test]
fn new_restart_replaces_owner_without_reusing_prior_chain() {
    let owners = vertical_owners(&[
        row(&[(0, 200, 3)]),
        row(&[(0, 200, 3)]),
        row(&[(0, 200, 1)]),
    ])
    .unwrap();
    assert_eq!(owners[2][0], DisplayOwner { row: 1, cell: 0 });
}

#[test]
fn ordered_vertical_operand_replaces_merge_bits_and_rejects_bad_address() {
    let mut rows = row(&[(0, 200, 3)]);
    rows.cells[0].raw_flags |= 0x80;
    apply(&mut rows.cells, &[2, 0, 1]).unwrap();
    apply(&mut rows.cells, &[2, 0, 0]).unwrap();
    assert_eq!(rows.cells[0].raw_flags, 0x80);
    for invalid in [&[2, 0, 2][..], &[2, 1, 3], &[1, 0, 3], &[2, 0]] {
        assert!(apply(&mut rows.cells, invalid).is_err());
    }
}
