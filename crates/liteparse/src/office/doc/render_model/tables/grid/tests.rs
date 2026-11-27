use super::*;
fn cell() -> CellLayout {
    CellLayout {
        edges_twips: [0, 0],
        borders: Default::default(),
        preferred_width: None,
        margins_twips: [0, 108, 0, 108],
        vertical_alignment: 0,
        raw_flags: 0,
        no_wrap: false,
        shading_rgb: None,
    }
}
fn op(first: u8, limit: u8, width: u16) -> Vec<u8> {
    let [a, b] = width.to_le_bytes();
    vec![first, limit, a, b]
}
#[test]
fn insertion_and_resize_preserve_identity_and_shift_following_edges() {
    let mut grid = RowGrid::new(-108);
    grid.insert(&op(0, 2, 1000), Some(&cell())).unwrap();
    grid.prefer_width(&[5, 1, 2, 2, 0xc4, 9]).unwrap();
    let mut inserted = cell();
    inserted.vertical_alignment = 2;
    grid.insert(&op(1, 1, 200), Some(&inserted)).unwrap();
    assert_eq!(
        grid.cells()
            .iter()
            .map(|c| c.edges_twips)
            .collect::<Vec<_>>(),
        [[-108, 892], [892, 1092], [1092, 2092]]
    );
    assert_eq!(grid.cells()[1].vertical_alignment, 2);
    assert_eq!(
        grid.cells()[2].preferred_width,
        Some(Width::PercentFiftieths(2500))
    );
    grid.resize(&op(0, 2, 500)).unwrap();
    assert_eq!(
        grid.cells()
            .iter()
            .map(|c| c.edges_twips)
            .collect::<Vec<_>>(),
        [[-108, 392], [392, 892], [892, 1892]]
    );
    assert_eq!(
        grid.cells()[2].preferred_width,
        Some(Width::PercentFiftieths(2500))
    );
}
#[test]
fn preferred_width_is_not_physical_width_and_nil_remains_unresolved() {
    let mut grid = RowGrid::new(0);
    grid.insert(&op(0, 1, 100), Some(&cell())).unwrap();
    grid.prefer_width(&[5, 0, 1, 3, 0xe8, 3]).unwrap();
    assert_eq!(grid.cells()[0].edges_twips, [0, 100]);
    assert_eq!(grid.cells()[0].preferred_width, Some(Width::Twips(1000)));
    assert!(grid.prefer_width(&[5, 0, 1, 0, 255, 255]).is_err());
    assert_eq!(grid.cells()[0].preferred_width, Some(Width::Twips(1000)));
    grid.prefer_width(&[5, 0, 0, 3, 0, 0]).unwrap();
}
#[test]
fn invalid_mutations_are_atomic_and_missing_defaults_remain_unresolved() {
    let mut grid = RowGrid::new(0);
    assert!(grid.insert(&op(0, 1, 100), None).is_err());
    for data in [
        vec![],
        op(0, 0, 100),
        op(1, 1, 100),
        op(0, 64, 1),
        op(0, 1, 32768),
    ] {
        assert!(grid.insert(&data, Some(&cell())).is_err());
        assert!(grid.cells().is_empty());
    }
    grid.insert(&op(0, 2, 100), Some(&cell())).unwrap();
    let before = grid.clone();
    for data in [
        vec![],
        op(2, 3, 100),
        op(1, 0, 100),
        op(0, 3, 100),
        op(0, 2, 20000),
    ] {
        assert!(grid.resize(&data).is_err());
        assert_eq!(grid, before);
    }
    for data in [
        vec![],
        vec![4, 0, 1, 3, 0, 0],
        vec![5, 0, 1, 2, 255, 255],
        vec![5, 0, 1, 3, 255, 255],
        vec![5, 0, 1, 0x13, 0, 0],
    ] {
        assert!(grid.prefer_width(&data).is_err());
        assert_eq!(grid, before);
    }
}
#[test]
fn zero_width_source_and_signed_origin_are_preserved_but_extent_is_bounded() {
    let mut grid = RowGrid::new(-108);
    grid.insert(&op(0, 1, 0), Some(&cell())).unwrap();
    assert_eq!(grid.cells()[0].edges_twips, [-108, -108]);
    let mut edge = RowGrid::new(32760);
    assert!(edge.insert(&op(0, 1, 8), Some(&cell())).is_err());
    assert!(edge.cells().is_empty());
    let mut bounded = RowGrid::new(0);
    bounded.insert(&op(0, 63, 1), Some(&cell())).unwrap();
    assert!(bounded.insert(&op(0, 1, 1), Some(&cell())).is_err());
    assert_eq!(bounded.cells().len(), 63);
}
