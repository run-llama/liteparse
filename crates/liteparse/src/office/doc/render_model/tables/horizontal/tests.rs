use super::*;
use crate::office::doc::render_model::tables::CellLayout;

fn row(flags: &[u16]) -> RowLayout {
    RowLayout {
        horizontal_merge_basis: Some(WORD97_TC_BITS),
        cells: flags
            .iter()
            .enumerate()
            .map(|(index, flags)| CellLayout {
                edges_twips: [index as i16 * 200, (index + 1) as i16 * 200],
                borders: Default::default(),
                preferred_width: None,
                margins_twips: [0; 4],
                vertical_alignment: 0,
                raw_flags: *flags,
                no_wrap: false,
                shading_rgb: None,
            })
            .collect(),
        ..Default::default()
    }
}

#[test]
fn independent_chains_preserve_first_cell_formatting_and_physical_owners() {
    let mut source = row(&[0, 1, 2, 1, 2, 0]);
    source.cells[1].shading_rgb = Some(0x112233);
    source.cells[2].shading_rgb = Some(0xff0000);
    let p = project(&source).unwrap();
    assert_eq!(p.physical_cells, [0, 1, 3, 5]);
    assert_eq!(p.owners, [0, 1, 1, 3, 3, 5]);
    assert_eq!(p.row.cells[1].edges_twips, [200, 600]);
    assert_eq!(p.row.cells[1].shading_rgb, Some(0x112233));
    assert_eq!(source.cells[2].shading_rgb, Some(0xff0000));
    assert_eq!(source.cells[2].raw_flags, 2);
}

#[test]
fn orphan_gapped_mixed_and_unbound_encodings_cannot_suppress_cell_content() {
    for flags in [&[2, 0][..], &[1, 0], &[3, 2], &[1, 2 | 0x60]] {
        assert!(project(&row(flags)).is_err());
    }
    let mut source = row(&[1, 2]);
    source.cells[1].edges_twips[0] += 1;
    assert!(project(&source).is_err());
    source.cells[1].edges_twips[0] -= 1;
    source.horizontal_merge_basis = None;
    assert!(project(&source).is_err());
}
