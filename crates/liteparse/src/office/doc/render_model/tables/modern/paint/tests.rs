use super::*;
use crate::office::doc::render_model::tables::CellLayout;

fn grid() -> RowGrid {
    let cell = CellLayout {
        edges_twips: [0, 1000],
        borders: Default::default(),
        preferred_width: None,
        margins_twips: [0, 108, 0, 108],
        vertical_alignment: 0,
        raw_flags: 0,
        no_wrap: false,
        shading_rgb: None,
    };
    let mut second = cell.clone();
    second.edges_twips = [1000, 2000];
    RowGrid::from_cells(vec![cell, second]).unwrap()
}
fn operand(first: u8, limit: u8, sides: u8, border: [u8; 8]) -> Vec<u8> {
    let mut result = vec![11, first, limit, sides];
    result.extend(border);
    result
}

#[test]
fn cell_range_and_each_side_preserve_unselected_borders_and_geometry() {
    let b = [0, 0, 0, 255, 4, 1, 0, 0];
    let mut value = grid();
    for index in 0..4 {
        value = cell_borders(value, &operand(1, 2, 1 << index, b)).unwrap();
    }
    assert!(value.cells()[0].borders.iter().all(Option::is_none));
    assert!(
        value.cells()[1]
            .borders
            .iter()
            .all(|x| x.as_ref().unwrap().kind == BorderKind::Single)
    );
    assert_eq!(value.cells()[1].edges_twips, [1000, 2000]);
    let nil = [1, 2, 3, 17, 255, 255, 255, 255];
    value = cell_borders(value, &operand(1, 2, 2, nil)).unwrap();
    assert_eq!(
        value.cells()[1].borders[1].as_ref().unwrap().kind,
        BorderKind::Nil
    );
    assert_eq!(
        value.cells()[1].borders[0].as_ref().unwrap().kind,
        BorderKind::Single
    );
}

#[test]
fn malformed_and_unqualified_cell_borders_are_rejected() {
    let mut valid = operand(0, 2, 15, [0, 0, 0, 255, 4, 1, 0, 0]);
    for (index, replacement) in [(0, 10), (1, 2), (2, 3), (3, 64), (3, 16)] {
        let mut b = valid.clone();
        b[index] = replacement;
        assert!(cell_borders(grid(), &b).is_err());
    }
    assert!(cell_borders(grid(), &valid[..11]).is_err());
    valid.extend([0]);
    assert!(cell_borders(grid(), &valid).is_err());
    assert!(border_or_nil(&[255, 255, 255, 255, 255, 255, 224, 255]).is_err());
}

#[test]
fn automatic_shading_preserves_omitted_trailing_cells_with_exact_framing() {
    let mut b = vec![10];
    b.extend([0, 0, 0, 255, 0, 0, 0, 255, 0, 0]);
    cell_shading(grid(), &b).unwrap();
    cell_shading(grid(), &[0]).unwrap();
    assert!(cell_shading(RowGrid::new(0), &b).is_err());
    assert!(cell_shading(grid(), &b[..10]).is_err());
    b[9] = 1;
    assert!(cell_shading(grid(), &b).is_err());
    let mut oversized = vec![230];
    oversized.extend(vec![0; 230]);
    assert!(cell_shading(grid(), &oversized).is_err());
}

#[test]
fn clear_background_color_and_shorter_array_reset_are_source_ordered() {
    let gray = [0, 0, 0, 255, 217, 217, 217, 0, 0, 0];
    let mut b = vec![20];
    b.extend(gray);
    b.extend(gray);
    let shaded = cell_shading(grid(), &b).unwrap();
    assert!(
        shaded
            .cells()
            .iter()
            .all(|c| c.shading_rgb == Some(0xd9d9d9))
    );
    b.truncate(11);
    b[0] = 10;
    let shorter = cell_shading(shaded, &b).unwrap();
    assert_eq!(shorter.cells()[0].shading_rgb, Some(0xd9d9d9));
    assert_eq!(shorter.cells()[1].shading_rgb, None);
    let clear = cell_shading(shorter, &[10, 0, 0, 0, 255, 0, 0, 0, 255, 0, 0]).unwrap();
    assert!(clear.cells().iter().all(|c| c.shading_rgb.is_none()));
    b[8] = 1;
    assert!(cell_shading(grid(), &b).is_err());
}
