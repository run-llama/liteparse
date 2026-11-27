use super::super::{BorderKind, PropertyModifier, Width, decode_row};
fn p(sprm: u16, operand: &[u8]) -> PropertyModifier {
    PropertyModifier {
        sprm,
        operand: operand.to_vec(),
    }
}
fn grid() -> PropertyModifier {
    let mut bytes = vec![0, 0, 1, 0, 0, 208, 7];
    bytes.extend_from_slice(&[0; 4]);
    for _ in 0..4 {
        bytes.extend_from_slice(&[4, 1, 0, 0]);
    }
    let length = (bytes.len() - 1) as u16;
    bytes[..2].copy_from_slice(&length.to_le_bytes());
    p(0xd608, &bytes)
}
fn base() -> Vec<PropertyModifier> {
    vec![p(0x9602, &[108, 0]), grid()]
}

#[test]
fn version_and_gap_requirements_remain_explicit() {
    let mut properties = base();
    properties.push(p(0x3615, &[1]));
    assert!(decode_row(0xd9, &properties).unwrap().autofit);
    assert!(decode_row(0xc1, &properties).is_err());
    assert!(decode_row(0xd9, &[grid(), p(0x3615, &[1])]).is_err());
    properties.push(p(0x3615, &[2]));
    assert!(decode_row(0xd9, &properties).is_err());
}
#[test]
fn ordered_padding_survives_grid_and_preserves_other_gap_sides() {
    let mut properties = vec![
        p(0x9602, &[108, 0]),
        p(0xd634, &[6, 0, 1, 2, 3, 70, 0]),
        grid(),
    ];
    let row = decode_row(0xd9, &properties).unwrap();
    assert_eq!(row.cells[0].margins_twips, [0, 70, 0, 108]);
    properties.extend([p(0xd634, &[6, 0, 1, 8, 3, 90, 0]), grid()]);
    assert_eq!(
        decode_row(0xd9, &properties).unwrap().cells[0].margins_twips,
        [0, 70, 0, 90]
    );
    properties.push(p(0xd634, &[6, 0, 1, 2, 0, 0, 0]));
    assert!(decode_row(0xd9, &properties).is_err());
}
#[test]
fn preferred_width_and_indent_are_distinct_from_cached_edges() {
    let mut properties = base();
    properties.extend([
        p(0x3615, &[1]),
        p(0xf614, &[3, 184, 11]),
        p(0xf661, &[3, 230, 5]),
        grid(),
    ]);
    let row = decode_row(0xd9, &properties).unwrap();
    assert_eq!(row.autofit_preferred_width, Some(Width::Twips(3000)));
    assert_eq!(row.cells[0].edges_twips, [0, 2000]);
    assert_eq!(row.preferred_indent_twips, Some(1510));
    properties.push(p(0x3615, &[0]));
    assert!(decode_row(0xd9, &properties).is_err());
}
#[test]
fn colors_require_current_cell_count_and_preserve_shape() {
    let mut properties = base();
    properties.push(p(0xd61a, &[4, 20, 40, 60, 0]));
    let row = decode_row(0xd9, &properties).unwrap();
    let border = row.cells[0].borders[0].as_ref().unwrap();
    assert_eq!(border.kind, BorderKind::Single);
    assert_eq!(border.eighth_points, 4);
    assert_eq!(border.color_rgb, Some(0x14283c));
    properties.push(p(0xd61a, &[4, 0, 0, 0, 255]));
    assert_eq!(
        decode_row(0xd9, &properties).unwrap().cells[0].borders[0]
            .as_ref()
            .unwrap()
            .kind,
        BorderKind::Single
    );
    properties.push(p(0xd61a, &[4, 255, 255, 255, 255]));
    assert_eq!(
        decode_row(0xd9, &properties).unwrap().cells[0].borders[0]
            .as_ref()
            .unwrap()
            .kind,
        BorderKind::Nil
    );
    for bytes in [
        &[0][..],
        &[4, 0, 0, 0][..],
        &[4, 0, 0, 0, 1][..],
        &[8, 0, 0, 0, 0, 0, 0, 0, 0][..],
    ] {
        let mut invalid = base();
        invalid.push(p(0xd61a, bytes));
        assert!(decode_row(0xd9, &invalid).is_err());
    }
    assert!(decode_row(0xd9, &[p(0xd61a, &[4, 0, 0, 0, 0]), grid()]).is_err());
}

#[test]
fn later_cell_definition_replaces_earlier_color_assignments() {
    let mut properties = base();
    properties.push(p(0xd61a, &[4, 20, 40, 60, 0]));
    properties.push(grid());
    let row = decode_row(0xd9, &properties).unwrap();
    assert_eq!(row.cells[0].borders[0].as_ref().unwrap().color_rgb, None);
    assert_eq!(row.applied, properties);
    assert_eq!(super::super::decode_recorded_row(0xd9, &row).unwrap(), row);
}

#[test]
fn modern_row_border_array_does_not_replace_explicit_cell_borders() {
    let mut bytes = vec![48];
    for _ in 0..6 {
        bytes.extend_from_slice(&[20, 40, 60, 0, 8, 1, 0, 0]);
    }
    let mut properties = base();
    properties.push(p(0xd613, &bytes));
    let row = decode_row(0xd9, &properties).unwrap();
    assert_eq!(row.borders[0].as_ref().unwrap().color_rgb, Some(0x14283c));
    assert_eq!(row.cells[0].borders[0].as_ref().unwrap().eighth_points, 4);
    assert_eq!(row.cells[0].borders[0].as_ref().unwrap().color_rgb, None);
    for invalid in [
        bytes[..48].to_vec(),
        [vec![47], bytes[1..].to_vec()].concat(),
    ] {
        properties.pop();
        properties.push(p(0xd613, &invalid));
        assert!(decode_row(0xd9, &properties).is_err());
    }
}

#[test]
fn unrelated_width_and_padding_variants_remain_guarded() {
    for (op, bytes) in [
        (0xf614, vec![2, 136, 19]),
        (0xf614, vec![3, 193, 123]),
        (0xf661, vec![3, 183, 132]),
        (0xf617, vec![3, 1, 0]),
        (0xd634, vec![6, 0, 2, 10, 3, 108, 0]),
        (0xd634, vec![6, 0, 1, 16, 3, 108, 0]),
        (0x740a, vec![0, 0, 0, 8]),
    ] {
        let mut properties = base();
        properties.push(p(op, &bytes));
        assert!(decode_row(0xd9, &properties).is_err(), "{op:x}");
    }
}

#[test]
fn none_inherits_row_border_but_nil_and_real_cell_borders_survive() {
    let mut row_borders = vec![48];
    for _ in 0..6 {
        row_borders.extend_from_slice(&[0, 0, 0, 255, 8, 1, 0, 0]);
    }
    let mut zero = grid();
    zero.operand[11..].fill(0);
    let properties = [p(0x9602, &[108, 0]), zero.clone(), p(0xd613, &row_borders)];
    let row = decode_row(0xd9, &properties).unwrap();
    assert_eq!(row.cells[0].borders, [None, None, None, None]);
    assert_eq!(row.borders[0].as_ref().unwrap().kind, BorderKind::Single);
    assert_eq!(row.applied, properties);
    zero.operand[11..].fill(255);
    let row = decode_row(0xd9, &[p(0x9602, &[108, 0]), zero, p(0xd613, &row_borders)]).unwrap();
    assert!(
        row.cells[0]
            .borders
            .iter()
            .all(|b| b.as_ref().unwrap().kind == BorderKind::Nil)
    );
    let row = decode_row(
        0xd9,
        &[p(0x9602, &[108, 0]), grid(), p(0xd613, &row_borders)],
    )
    .unwrap();
    assert!(
        row.cells[0]
            .borders
            .iter()
            .all(|b| b.as_ref().unwrap().eighth_points == 4)
    );
}

#[test]
fn absent_color_border_stays_suppressed_after_row_fallback() {
    let mut row_borders = vec![48];
    for _ in 0..6 {
        row_borders.extend_from_slice(&[0, 0, 0, 255, 8, 1, 0, 0]);
    }
    let mut zero = grid();
    zero.operand[11..].fill(0);
    let mut properties = vec![p(0x9602, &[108, 0]), zero, p(0xd613, &row_borders)];
    properties.push(p(0xd61a, &[4, 255, 255, 255, 255]));
    let row = decode_row(0xd9, &properties).unwrap();
    assert_eq!(
        row.cells[0].borders[0].as_ref().unwrap().kind,
        BorderKind::Nil
    );
    assert!(row.cells[0].borders[1..].iter().all(Option::is_none));
    properties.pop();
    properties.push(p(0xd61a, &[4, 20, 40, 60, 0]));
    assert!(decode_row(0xd9, &properties).is_err());
}
