use super::*;

fn property(sprm: u16, operand: Vec<u8>) -> PropertyModifier {
    PropertyModifier { sprm, operand }
}
fn definition(edges: &[i16], border: [u8; 4]) -> PropertyModifier {
    let count = edges.len() - 1;
    let mut data = vec![0, 0, count as u8];
    for edge in edges {
        data.extend_from_slice(&edge.to_le_bytes());
    }
    for _ in 0..count {
        data.extend_from_slice(&[0; 4]);
        for _ in 0..4 {
            data.extend_from_slice(&border);
        }
    }
    let length = (data.len() - 1) as u16;
    data[..2].copy_from_slice(&length.to_le_bytes());
    property(0xd608, data)
}

#[test]
fn unequal_rows_preserve_physical_cells_and_common_grid_spans() {
    let wide = decode_row(
        0x101,
        &[definition(&[900, 3600, 4140, 4680, 5220], [4, 1, 0, 0])],
    )
    .unwrap();
    let narrow = decode_row(0x101, &[definition(&[900, 3600, 5220], [255; 4])]).unwrap();
    let grid = common_grid(&[wide.clone(), narrow.clone()]).unwrap();
    assert_eq!(grid.edges_twips, vec![900, 3600, 4140, 4680, 5220]);
    assert_eq!(
        grid.cell_intervals,
        vec![vec![[0, 1], [1, 2], [2, 3], [3, 4]], vec![[0, 1], [1, 4]]]
    );
    assert_eq!(wide.cells.len(), 4);
    assert_eq!(narrow.cells.len(), 2);
    assert_eq!(wide.cells[0].borders[0].as_ref().unwrap().eighth_points, 4);
    assert_eq!(
        narrow.cells[0].borders[0].as_ref().unwrap().kind,
        BorderKind::Nil
    );
}

#[test]
fn row_height_borders_and_gap_keep_ordered_source_values() {
    let mut borders = vec![24];
    for _ in 0..6 {
        borders.extend_from_slice(&[4, 1, 0, 0]);
    }
    let properties = [
        property(0x9602, 108i16.to_le_bytes().to_vec()),
        property(0x9407, (-320i16).to_le_bytes().to_vec()),
        property(0x3403, vec![1]),
        property(0xd605, borders),
        definition(&[900, 3600], [255; 4]),
    ];
    let row = decode_row(0xc1, &properties).unwrap();
    assert_eq!(row.height, Height::Exact(320));
    assert!(row.cant_split);
    assert_eq!(row.gap_half_twips, 108);
    assert_eq!(row.cells[0].margins_twips, [0, 108, 0, 108]);
    assert_eq!(row.applied, properties);
    assert_eq!(row.borders[0].as_ref().unwrap().kind, BorderKind::Single);
    assert_eq!(
        row.cells[0].borders[0].as_ref().unwrap().kind,
        BorderKind::Nil
    );
    let mut reordered = properties.to_vec();
    reordered.swap(0, 4);
    assert!(decode_row(0xc1, &reordered).is_err());
}

#[test]
fn malformed_borders_and_unsupported_paint_are_explicit() {
    assert_eq!(border80(&[0, 1, 0, 0]).unwrap().eighth_points, 2);
    assert_eq!(border80(&[4, 1, 6, 0]).unwrap().color_rgb, Some(0xff0000));
    assert_eq!(border80(&[0, 0, 0, 0]).unwrap().kind, BorderKind::None);
    for data in [
        &[4, 1, 17, 0][..],
        &[4, 26, 0, 0],
        &[4, 1, 0, 1],
        &[4, 1, 0, 32],
        &[4, 2, 0, 0],
        &[4, 1, 0],
    ] {
        assert!(border80(data).is_err());
    }
    assert!(border80(&[4, 1, 0, 128]).is_ok());
}

#[test]
fn malformed_source_row_and_cell_semantics_do_not_default() {
    let valid = definition(&[0, 1000], [0; 4]);
    for end in 0..valid.operand.len() {
        assert!(decode_row(0x101, &[property(0xd608, valid.operand[..end].to_vec())]).is_err());
    }
    for flags in [1u16, 8, 24, 28, 64, 0x1000, 0x2000, 0x4000] {
        let mut invalid = valid.clone();
        invalid.operand[7..9].copy_from_slice(&flags.to_le_bytes());
        assert!(decode_row(0x101, &[invalid]).is_err());
    }
    assert!(decode_row(0x101, &[definition(&[0, 0], [0; 4])]).is_err());
    assert!(decode_row(0x101, &[property(0x3403, vec![2]), valid.clone()]).is_err());
    assert!(decode_row(0x112, &[property(0x3403, vec![1]), valid.clone()]).is_err());
    assert!(decode_row(0x101, &[property(0x563a, vec![11, 0]), valid]).is_err());
}

#[test]
fn incomplete_or_discontinuous_grid_cannot_claim_cell_coverage() {
    let row = decode_row(0x101, &[definition(&[0, 1000, 2000], [0; 4])]).unwrap();
    let mut broken = row.clone();
    broken.cells[1].edges_twips[0] += 1;
    assert!(common_grid(&[row.clone(), broken]).is_err());
    let shifted = decode_row(0x101, &[definition(&[100, 1100, 2100], [0; 4])]).unwrap();
    assert!(common_grid(&[row, shifted]).is_err());
    assert!(common_grid(&[]).is_err());
}

#[test]
fn historical_gap_sets_padding_but_modern_padding_is_independent() {
    let def = definition(&[0, 1000], [0; 4]);
    assert!(decode_row(0xc1, std::slice::from_ref(&def)).is_err());
    for gap in [0i16, 54, 108, 216] {
        let properties = [property(0x9602, gap.to_le_bytes().to_vec()), def.clone()];
        let old = decode_row(0xc1, &properties).unwrap();
        assert_eq!(old.cells[0].margins_twips, [0, gap as u16, 0, gap as u16]);
        let modern = decode_row(0x101, &properties).unwrap();
        assert_eq!(modern.cells[0].margins_twips, [0, 108, 0, 108]);
    }
    assert!(
        decode_row(
            0xc1,
            &[property(0x9602, (-1i16).to_le_bytes().to_vec()), def]
        )
        .is_err()
    );
}

#[test]
fn double_and_triple_borders_keep_each_stripe_width_and_source_color() {
    for (code, kind) in [(3, BorderKind::Double), (10, BorderKind::Triple)] {
        for width in [0u8, 1, 2, 4, 255] {
            let bytes = [width, code, 6, 128];
            let value = border80(&bytes).unwrap();
            assert_eq!(value.kind, kind);
            assert_eq!(value.eighth_points, width.max(2));
            assert_eq!(value.color_rgb, Some(0xff0000));
            assert_eq!(value.raw, u64::from(u32::from_le_bytes(bytes)));
        }
    }
    for bytes in [
        [24, 13, 0, 0],
        [255, 255, 0, 0],
        [4, 3, 0, 32],
        [4, 10, 17, 0],
    ] {
        assert!(border80(&bytes).is_err());
    }
}
