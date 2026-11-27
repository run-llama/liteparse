use super::*;
use crate::office::doc::render_model::tables::{CellLayout, Width};

fn p(sprm: u16, bytes: &[u8]) -> PropertyModifier {
    PropertyModifier {
        sprm,
        operand: bytes.to_vec(),
    }
}
fn defaults() -> Vec<CellDefaults> {
    vec![CellDefaults {
        selected_style: 11,
        base_to_derived: vec![11],
        cell: CellLayout {
            edges_twips: [0, 0],
            borders: Default::default(),
            preferred_width: None,
            margins_twips: [0, 108, 0, 108],
            vertical_alignment: 0,
            raw_flags: 0,
            no_wrap: false,
            shading_rgb: None,
        },
        row_borders: Default::default(),
        applied: vec![(11, p(0x563a, &[11, 0])), (11, p(0xf617, &[3, 0, 0]))],
    }]
}
#[test]
fn style_reset_retains_grid_height_gap_and_table_width_but_resets_other_row_state() {
    let input = vec![
        p(0x9601, &400i16.to_le_bytes()),
        p(0x9602, &108i16.to_le_bytes()),
        p(0x7621, &[0, 2, 232, 3]),
        p(0x7623, &[1, 2, 208, 7]),
        p(0x9407, &(-500i16).to_le_bytes()),
        p(0xf614, &[3, 184, 11]),
        p(0x5400, &[2, 0]),
        p(0x3466, &[1]),
        p(0x3404, &[1]),
        p(0xd634, &[6, 0, 1, 10, 3, 54, 0]),
        p(0x563a, &[11, 0]),
    ];
    let row = decode(0x112, &input, &defaults()).unwrap();
    assert_eq!(row.cells[0].edges_twips, [-108, 892]);
    assert_eq!(row.cells[1].edges_twips, [892, 2892]);
    assert_eq!(row.cells[0].margins_twips, [0, 108, 0, 108]);
    assert_eq!(row.height, Height::Exact(500));
    assert_eq!(row.gap_half_twips, 108);
    assert!(!row.cant_split && !row.header);
    assert_eq!(row.physical_alignment, None);
    assert_eq!(row.applied, input);
}
#[test]
fn padding_and_left_indent_after_style_selection_apply_in_order() {
    let row = decode(
        0x112,
        &[
            p(0x563a, &[11, 0]),
            p(0x9601, &[200, 0]),
            p(0xd634, &[6, 0, 1, 10, 3, 54, 0]),
            p(0x7621, &[0, 1, 232, 3]),
            p(0xd634, &[6, 0, 1, 2, 3, 72, 0]),
            p(0x7621, &[1, 1, 232, 3]),
        ],
        &defaults(),
    )
    .unwrap();
    assert_eq!(row.cells[0].edges_twips, [200, 1200]);
    assert_eq!(row.cells[1].edges_twips, [1200, 2200]);
    assert!(row.cells.iter().all(|c| c.margins_twips == [0, 72, 0, 54]));
}
#[test]
fn preferred_width_does_not_replace_physical_grid_and_ambiguous_reset_is_guarded() {
    let mut input = vec![p(0x7621, &[0, 1, 232, 3]), p(0xd635, &[5, 0, 1, 3, 208, 7])];
    let row = decode(0x112, &input, &defaults()).unwrap();
    assert_eq!(row.cells[0].edges_twips, [0, 1000]);
    assert_eq!(row.cells[0].preferred_width, Some(Width::Twips(2000)));
    input.push(p(0x563a, &[11, 0]));
    assert!(matches!(
        decode(0x112, &input, &defaults()),
        Err(Error::Unsupported(_))
    ));
}
#[test]
fn malformed_or_unknown_properties_and_mutated_style_evidence_remain_errors() {
    let insert = p(0x7621, &[0, 1, 232, 3]);
    for bad in [
        p(0x563a, &[11]),
        p(0x7621, &[0, 1]),
        p(0xd634, &[6, 0, 2, 10, 3, 0, 0]),
        p(0x7479, &[0]),
        p(0x3615, &[2]),
    ] {
        assert!(decode(0x112, &[insert.clone(), bad], &defaults()).is_err());
    }
    for version in [0xc1, 0xd9, 0x100, 0x102, 0xffff] {
        assert!(decode(version, std::slice::from_ref(&insert), &defaults()).is_err());
    }
    for version in [0x101, 0x10c, 0x112] {
        assert!(decode(version, std::slice::from_ref(&insert), &defaults()).is_ok());
    }
    let mut changed = defaults();
    changed[0].cell.margins_twips[0] = 10;
    assert!(decode(0x112, &[insert], &changed).is_err());
}

#[test]
fn complete_post_style_width_assignments_resolve_earlier_width_uncertainty() {
    let mut input = vec![
        p(0x7621, &[0, 2, 232, 3]),
        p(0xd635, &[5, 0, 2, 3, 208, 7]),
        p(0x563a, &[11, 0]),
        p(0xd635, &[5, 0, 1, 3, 232, 3]),
    ];
    assert!(decode(0x112, &input, &defaults()).is_err());
    input.push(p(0xd635, &[5, 1, 2, 3, 232, 3]));
    let row = decode(0x112, &input, &defaults()).unwrap();
    assert!(
        row.cells
            .iter()
            .all(|c| c.preferred_width == Some(Width::Twips(1000)))
    );
    input.push(p(0x563a, &[11, 0]));
    assert!(decode(0x112, &input, &defaults()).is_err());
}

#[test]
fn vertical_alignment_targets_existing_cells_and_resets_with_style() {
    let mut input = vec![p(0x7621, &[0, 2, 232, 3]), p(0xd62c, &[3, 1, 2, 2])];
    let row = decode(0x112, &input, &defaults()).unwrap();
    assert_eq!(
        row.cells
            .iter()
            .map(|c| c.vertical_alignment)
            .collect::<Vec<_>>(),
        [0, 2]
    );
    input.push(p(0x563a, &[11, 0]));
    assert!(
        decode(0x112, &input, &defaults())
            .unwrap()
            .cells
            .iter()
            .all(|c| c.vertical_alignment == 0)
    );
    for bytes in [
        vec![3, 0, 3, 1],
        vec![3, 0, 2, 3],
        vec![3, 2, 2, 0],
        vec![3, 0, 2],
    ] {
        let props = [p(0x7621, &[0, 2, 232, 3]), p(0xd62c, &bytes)];
        assert!(decode(0x112, &props, &defaults()).is_err());
    }
}

#[test]
fn modern_border_color_width_and_ignored_reserved_bits_are_preserved() {
    let bytes = [12, 34, 56, 0, 1, 1, 128, 255];
    let b = border(&bytes).unwrap();
    assert_eq!(b.color_rgb, Some(0x0c2238));
    assert_eq!(b.eighth_points, 2);
    assert_eq!(b.raw, u64::from_le_bytes(bytes));
    assert_eq!(border(&[0, 0, 0, 255, 4, 1, 0, 0]).unwrap().color_rgb, None);
    for invalid in [
        vec![0; 7],
        vec![0, 0, 0, 1, 4, 1, 0, 0],
        vec![0, 0, 0, 0, 4, 1, 32, 0],
        vec![0, 0, 0, 0, 4, 2, 0, 0],
    ] {
        assert!(border(&invalid).is_err());
    }
    let mut bytes = vec![48];
    bytes.extend([0, 0, 0, 255, 4, 1, 0, 0].repeat(6));
    let mut input = vec![p(0x7621, &[0, 1, 232, 3]), p(0xd613, &bytes)];
    assert!(
        decode(0x112, &input, &defaults())
            .unwrap()
            .borders
            .iter()
            .all(Option::is_some)
    );
    input.push(p(0x563a, &[11, 0]));
    assert!(
        decode(0x112, &input, &defaults())
            .unwrap()
            .borders
            .iter()
            .all(Option::is_none)
    );
}

#[test]
fn nil_padding_requires_coincident_current_style_and_standard_margins() {
    let mut input = vec![
        p(0x7621, &[0, 1, 232, 3]),
        p(0xd634, &[6, 0, 1, 5, 0, 100, 0]),
    ];
    assert_eq!(
        decode(0x112, &input, &defaults()).unwrap().cells[0].margins_twips,
        [0, 108, 0, 108]
    );
    input.push(p(0xd634, &[6, 0, 1, 1, 3, 40, 0]));
    input.push(p(0xd634, &[6, 0, 1, 1, 0, 0, 0]));
    assert!(decode(0x112, &input, &defaults()).is_err());
    input.pop();
    input.push(p(0x563a, &[11, 0]));
    input.push(p(0xd634, &[6, 0, 1, 1, 0, 0, 0]));
    assert!(decode(0x112, &input, &defaults()).is_ok());
}

#[test]
fn nil_width_cannot_resolve_a_pre_style_assignment() {
    let mut input = vec![
        p(0x7621, &[0, 1, 232, 3]),
        p(0xd635, &[5, 0, 1, 3, 232, 3]),
        p(0x563a, &[11, 0]),
        p(0xd635, &[5, 0, 1, 0, 0, 0]),
    ];
    assert!(decode(0x112, &input, &defaults()).is_err());
    input.push(p(0xd635, &[5, 0, 1, 3, 232, 3]));
    assert!(decode(0x112, &input, &defaults()).is_ok());
}

#[test]
fn autofit_preserves_preferences_and_style_selection_keeps_layout_mode() {
    let input = [
        p(0x7621, &[0, 2, 232, 3]),
        p(0xf614, &[3, 184, 11]),
        p(0x3615, &[1]),
        p(0x563a, &[11, 0]),
        p(0xd635, &[5, 0, 2, 2, 196, 9]),
    ];
    let row = decode(0x112, &input, &defaults()).unwrap();
    assert!(row.autofit);
    assert_eq!(row.autofit_preferred_width, Some(Width::Twips(3000)));
    assert_eq!(row.cells[0].edges_twips, [0, 1000]);
    assert_eq!(row.cells[1].edges_twips, [1000, 2000]);
    assert!(
        row.cells
            .iter()
            .all(|c| c.preferred_width == Some(Width::PercentFiftieths(2500)))
    );
    assert_eq!(row.applied, input);
}

#[test]
fn autofit_reset_retains_fixed_width_validation_and_strict_bool() {
    let prefix = [p(0x7621, &[0, 1, 232, 3]), p(0xf614, &[3, 208, 7])];
    let mut input = prefix.to_vec();
    input.extend([p(0x3615, &[1]), p(0x3615, &[0])]);
    assert!(decode(0x112, &input, &defaults()).is_err());
    for operand in [&[2][..], &[1, 0][..], &[][..]] {
        let mut input = prefix.to_vec();
        input.extend([p(0x3615, operand), p(0x3615, &[1])]);
        assert!(decode(0x112, &input, &defaults()).is_err());
    }
}

#[test]
fn nowrap_cell_state_follows_ranges_and_style_reset() {
    let input = [
        p(0x7621, &[0, 2, 232, 3]),
        p(0xd639, &[3, 0, 2, 1]),
        p(0xd639, &[3, 1, 2, 0]),
    ];
    let row = decode(0x112, &input, &defaults()).unwrap();
    assert!(row.cells[0].no_wrap);
    assert!(!row.cells[1].no_wrap);
    let mut reset = input.to_vec();
    reset.push(p(0x563a, &[11, 0]));
    assert!(
        decode(0x112, &reset, &defaults())
            .unwrap()
            .cells
            .iter()
            .all(|c| !c.no_wrap)
    );
    for operand in [
        &[3, 0, 2, 2][..],
        &[3, 1, 3, 1][..],
        &[2, 0, 2, 1][..],
        &[3, 0, 2][..],
    ] {
        assert!(decode(0x112, &[input[0].clone(), p(0xd639, operand)], &defaults()).is_err());
    }
}

#[test]
fn cell_borders_apply_in_order_and_style_selection_resets_them() {
    let single = [11, 0, 2, 15, 0, 0, 0, 255, 4, 1, 0, 0];
    let nil = [11, 1, 2, 2, 0, 0, 0, 0, 255, 255, 255, 255];
    let mut input = vec![
        p(0x7621, &[0, 2, 232, 3]),
        p(0xd62f, &single),
        p(0xd62f, &nil),
    ];
    let row = decode(0x112, &input, &defaults()).unwrap();
    assert_eq!(
        row.cells[1].borders[1].as_ref().unwrap().kind,
        BorderKind::Nil
    );
    assert_eq!(
        row.cells[0].borders[1].as_ref().unwrap().kind,
        BorderKind::Single
    );
    input.push(p(0x563a, &[11, 0]));
    let row = decode(0x112, &input, &defaults()).unwrap();
    assert!(
        row.cells
            .iter()
            .all(|c| c.borders.iter().all(Option::is_none))
    );
    input.push(p(0xd62f, &single));
    let row = decode(0x112, &input, &defaults()).unwrap();
    assert!(
        row.cells
            .iter()
            .all(|c| c.borders.iter().all(Option::is_some))
    );
}

#[test]
fn automatic_shading_requires_existing_cells_and_complete_style_proof() {
    let shading = [10, 0, 0, 0, 255, 0, 0, 0, 255, 0, 0];
    let input = [p(0x7621, &[0, 2, 232, 3]), p(0xd670, &shading)];
    let row = decode(0x112, &input, &defaults()).unwrap();
    assert_eq!(row.cells.len(), 2);
    assert_eq!(row.applied, input);
    assert!(decode(0x112, &[p(0xd670, &shading)], &defaults()).is_err());
    let mut styles = defaults();
    styles[0]
        .applied
        .push((11, p(0xd660, &[10, 0, 0, 0, 255, 0, 0, 0, 255, 0, 0])));
    assert!(decode(0x112, &input, &styles).is_err());
}

#[test]
fn preferred_indent_uses_signed_units_and_resets_without_moving_cached_grid() {
    let mut input = vec![p(0x7621, &[0, 2, 232, 3]), p(0xf661, &[3, 52, 252])];
    let row = decode(0x112, &input, &defaults()).unwrap();
    assert_eq!(row.preferred_indent_twips, Some(-972));
    assert_eq!(row.cells[0].edges_twips, [0, 1000]);
    input.push(p(0x563a, &[11, 0]));
    assert_eq!(
        decode(0x112, &input, &defaults())
            .unwrap()
            .preferred_indent_twips,
        None
    );
    for unit in [0, 1] {
        assert_eq!(preferred_indent(&[unit, 0, 0]).unwrap(), None);
        assert!(preferred_indent(&[unit, 1, 0]).is_err());
    }
    for value in [-31560i16, -972, 0, 31680] {
        let b = value.to_le_bytes();
        assert_eq!(preferred_indent(&[3, b[0], b[1]]).unwrap(), Some(value));
    }
    for value in [-31561i16, 31681] {
        let b = value.to_le_bytes();
        assert!(preferred_indent(&[3, b[0], b[1]]).is_err());
    }
    for bytes in [&[2, 0, 0][..], &[19, 0, 0], &[3], &[3, 0, 0, 0]] {
        assert!(preferred_indent(bytes).is_err());
    }
}

#[test]
fn modern_style_interpretation_ignores_legacy_shading_without_losing_current_fill() {
    let gray = [10, 0, 0, 0, 255, 217, 217, 217, 0, 0, 0];
    let input = vec![
        p(0x7621, &[0, 2, 232, 3]),
        p(0xd670, &gray),
        p(0xd609, &[2, 0xff, 0xff]),
        p(
            0xd612,
            &[
                10, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
            ],
        ),
    ];
    let row = decode(0x112, &input, &defaults()).unwrap();
    assert_eq!(row.cells[0].shading_rgb, Some(0xd9d9d9));
    assert_eq!(row.cells[1].shading_rgb, None);
    assert_eq!(row.applied, input);
    assert!(decode(0xd9, &input, &defaults()).is_err());
    for (op, b) in [
        (0xd609, &[1, 0][..]),
        (0xd609, &[6, 0, 0, 0, 0, 0, 0]),
        (0xd612, &[10, 0]),
    ] {
        let bad = [p(0x7621, &[0, 2, 232, 3]), p(op, b)];
        assert!(decode(0x112, &bad, &defaults()).is_err());
    }
}

#[test]
fn modern_double_and_triple_borders_preserve_rgb_and_width_floor() {
    for (code, kind) in [(3, BorderKind::Double), (10, BorderKind::Triple)] {
        for width in [0u8, 1, 2, 4, 255] {
            let bytes = [12, 34, 56, 0, width, code, 128, 255];
            let value = border(&bytes).unwrap();
            assert_eq!(value.kind, kind);
            assert_eq!(value.eighth_points, width.max(2));
            assert_eq!(value.color_rgb, Some(0x0c2238));
            assert_eq!(value.raw, u64::from_le_bytes(bytes));
        }
    }
    for bytes in [
        [0, 0, 0, 255, 24, 13, 0, 0],
        [255, 255, 255, 255, 255, 255, 224, 255],
        [0, 0, 0, 255, 4, 3, 32, 0],
        [0, 0, 0, 1, 4, 10, 0, 0],
    ] {
        assert!(border(&bytes).is_err());
    }
}

#[test]
fn table_look_preserves_current_cell_geometry_and_paint() {
    let prefix = vec![
        p(0x7621, &[0, 2, 232, 3]),
        p(0xd670, &[10, 0, 0, 0, 255, 217, 217, 217, 0, 0, 0]),
    ];
    let baseline = decode(0x112, &prefix, &defaults()).unwrap();
    for index in [i16::MIN, -1, 0, 1, i16::MAX] {
        for flags in [0u16, 0x1f, 0x1e0, 0x600, 0x7ff] {
            let mut input = prefix.clone();
            let [a, b] = index.to_le_bytes();
            let [c, d] = flags.to_le_bytes();
            input.push(p(0x740a, &[a, b, c, d]));
            let mut after = decode(0x112, &input, &defaults()).unwrap();
            assert_eq!(after.applied, input);
            after.applied = baseline.applied.clone();
            assert_eq!(after, baseline);
        }
    }
}

#[test]
fn table_look_framing_and_reserved_bits_stay_rejected() {
    for bytes in [
        &[][..],
        &[0],
        &[0, 0, 0],
        &[0, 0, 0, 0, 0],
        &[0, 0, 0, 8],
        &[0, 0, 0, 128],
    ] {
        let input = [p(0x7621, &[0, 2, 232, 3]), p(0x740a, bytes)];
        assert!(decode(0x112, &input, &defaults()).is_err());
    }
}

#[test]
fn table_look_does_not_qualify_conditional_or_unknown_style_contributions() {
    for flags in [0u16, 0x1e0, 0x7ff] {
        let [a, b] = flags.to_le_bytes();
        let input = [p(0x7621, &[0, 2, 232, 3]), p(0x740a, &[0, 0, a, b])];
        for modifier in [p(0xd66a, &[0]), p(0x0835, &[1]), p(0xdead, &[])] {
            let mut unresolved = defaults();
            unresolved[0].applied.push((11, modifier));
            assert!(decode(0x112, &input, &unresolved).is_err());
        }
        assert!(decode(0x112, &input, &[]).is_err());
    }
}

#[test]
fn table_look_preserves_ordered_style_reset_and_explicit_cell_fill() {
    let look = p(0x740a, &[0, 0, 224, 1]);
    let fill = p(0xd670, &[10, 0, 0, 0, 255, 217, 217, 217, 0, 0, 0]);
    let first = [
        look.clone(),
        p(0x7621, &[0, 2, 232, 3]),
        fill.clone(),
        p(0x563a, &[11, 0]),
    ];
    let reset = decode(0x112, &first, &defaults()).unwrap();
    assert_eq!(reset.cells[0].shading_rgb, None);
    let second = [p(0x7621, &[0, 2, 232, 3]), p(0x563a, &[11, 0]), fill, look];
    let explicit = decode(0x112, &second, &defaults()).unwrap();
    assert_eq!(explicit.cells[0].shading_rgb, Some(0xd9d9d9));
    assert_eq!(explicit.cells[0].edges_twips, reset.cells[0].edges_twips);
}

#[test]
fn floating_table_coordinates_remain_outside_inline_row_model() {
    for placement in [
        p(0x360d, &[0]),
        p(0x940e, &100i16.to_le_bytes()),
        p(0x940f, &100i16.to_le_bytes()),
    ] {
        assert!(decode(0x112, &[p(0x7621, &[0, 1, 232, 3]), placement], &defaults()).is_err());
    }
}

#[test]
fn selected_table_style_resets_previous_cell_rotation() {
    let mut input = vec![p(0x7621, &[0, 1, 0xe8, 3]), p(0x7629, &[0, 1, 3, 0])];
    let before = decode(0x10c, &input, &defaults()).unwrap();
    assert_eq!(
        super::super::text_direction::code(before.cells[0].raw_flags),
        3
    );
    input.push(p(0x563a, &[11, 0]));
    let reset = decode(0x10c, &input, &defaults()).unwrap();
    assert_eq!(
        super::super::text_direction::code(reset.cells[0].raw_flags),
        0
    );
    input.push(p(0x7629, &[0, 1, 1, 0]));
    let after = decode(0x10c, &input, &defaults()).unwrap();
    assert_eq!(
        super::super::text_direction::code(after.cells[0].raw_flags),
        1
    );
}
