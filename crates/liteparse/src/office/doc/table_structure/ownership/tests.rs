use super::*;
use crate::office::doc::structure::ParagraphEndpoint;
fn p(start: usize, end: usize, terminal: u16) -> ParagraphOwnership {
    ParagraphOwnership {
        source_range: [start, end],
        projected_bytes: Some([start, end]),
        terminal_unit: Some(terminal),
        endpoint: Some(ParagraphEndpoint {
            end_cp: end,
            piece_index: 0,
            fkp_offset: 512,
            fkp_run: 0,
            end_fc: end,
        }),
        terminal_projection: "text",
        unresolved: vec![],
    }
}
#[expect(
    clippy::unnecessary_wraps,
    reason = "fixtures compare against the parser's Option"
)]
fn state(depth: usize, cell: bool, row: bool, count: usize) -> Option<State> {
    Some(State {
        depth,
        cell,
        row,
        definitions: (0..count)
            .map(|_| CellDefinition {
                horizontal_merge: 0,
                vertical_merge: 0,
                raw_flags: 0,
                preferred_width_raw: 100,
                preferred_width_unit: 3,
                definition_source: "synthetic",
            })
            .collect(),
        definition_known: true,
        revision: false,
        paragraph_group_id: None,
        frame_anchor_locked: None,
    })
}
#[test]
fn nested_cells_retain_order_and_each_source_paragraph() {
    let ps = vec![
        p(0, 2, 13),
        p(2, 4, 13),
        p(4, 5, 13),
        p(5, 6, 7),
        p(6, 7, 7),
    ];
    let out = assemble(
        "body",
        &ps,
        vec![
            state(1, false, false, 0),
            state(2, true, false, 0),
            state(2, false, true, 1),
            state(1, true, false, 0),
            state(1, false, true, 1),
        ],
    )
    .unwrap();
    assert_eq!(out.rows.len(), 2);
    assert_eq!(out.rows[0].source_range, [2, 5]);
    assert_eq!(out.rows[1].cells[0].source_range, [0, 6]);
    assert_eq!(out.rows[1].cells[0].projected_bytes, Some([0, 6]));
    assert!(matches!(
        out.rows[1].cells[0].content.as_slice(),
        [
            CellContent::Paragraph(0),
            CellContent::NestedRow(0),
            CellContent::Paragraph(3)
        ]
    ));
    assert!(out.rows[1].unresolved.is_empty());
}
#[test]
fn merged_cell_text_is_retained_and_visible_ownership_explicit() {
    let ps = vec![p(0, 2, 7), p(2, 4, 7), p(4, 5, 7)];
    let mut row = state(1, false, true, 2).unwrap();
    row.definitions[1].vertical_merge = 1;
    let out = assemble(
        "body",
        &ps,
        vec![
            state(1, true, false, 0),
            state(1, true, false, 0),
            Some(row),
        ],
    )
    .unwrap();
    assert_eq!(out.rows[0].cells.len(), 2);
    assert_eq!(out.rows[0].cells[1].projected_bytes, Some([2, 4]));
    assert!(
        out.rows[0]
            .unresolved
            .contains(&"merged_cell_visible_ownership_unresolved")
    );
}
#[test]
fn incomplete_and_mismatched_rows_keep_source_ranges() {
    let ps = vec![p(0, 2, 7), p(2, 3, 7)];
    let out = assemble(
        "body",
        &ps,
        vec![state(1, true, false, 0), state(1, false, true, 2)],
    )
    .unwrap();
    assert!(
        out.rows[0]
            .unresolved
            .contains(&"row_cell_definition_count_unresolved")
    );
    let out = assemble(
        "body",
        &ps,
        vec![state(1, true, false, 0), state(0, false, false, 0)],
    )
    .unwrap();
    assert_eq!(out.unresolved_ranges[0].paragraph_range, [0, 1]);
    assert_eq!(out.outside_paragraphs, vec![1]);
}
#[test]
fn direct_piece_override_and_legacy_depth_follow_source_order() {
    let pap = [0x16, 0x24, 1];
    let s = resolve(&pap, 0, &[], Some(7), 0xc1, &mut (8 * 1024 * 1024)).unwrap();
    assert_eq!(s.depth, 1);
    assert!(s.cell);
    assert!(resolve(&pap, 0, &[], Some(7), 0x112, &mut (8 * 1024 * 1024)).is_err());
    let s = resolve(&pap, 0x18 << 1, &[], Some(13), 0xc1, &mut (8 * 1024 * 1024)).unwrap();
    assert_eq!(s.depth, 0);
}
#[test]
fn ordered_insert_delete_and_merge_modify_definition_evidence() {
    let data = [
        0x16, 0x24, 1, 0x17, 0x24, 1, 0x21, 0x76, 0, 3, 100, 0, 0x22, 0x56, 1, 2, 0x24, 0x56, 0, 2,
    ];
    let s = resolve(&data, 0, &[], Some(7), 0xc1, &mut (8 * 1024 * 1024)).unwrap();
    assert_eq!(s.definitions.len(), 2);
    assert_eq!(s.definitions[0].horizontal_merge, 2);
    assert_eq!(s.definitions[1].horizontal_merge, 1);
}
#[test]
fn malformed_and_excessive_depth_are_bounded() {
    assert!(resolve(&[0x16, 0x24, 2], 0, &[], Some(7), 0xc1, &mut 100).is_err());
    assert!(resolve(&[], 0, &[], Some(13), 0xc1, &mut 0).is_err());
    assert!(assemble("body", &[p(0, 1, 13)], vec![state(65, false, false, 0)]).is_err());
}

#[test]
fn interrupted_parent_preserves_nested_ownership_without_duplicate_unresolved_ranges() {
    let ps = vec![p(0, 2, 13), p(2, 4, 13), p(4, 5, 13), p(5, 6, 13)];
    let out = assemble(
        "body",
        &ps,
        vec![
            state(1, false, false, 0),
            state(2, true, false, 0),
            state(2, false, true, 1),
            None,
        ],
    )
    .unwrap();
    assert_eq!(out.rows.len(), 1);
    assert!(
        out.rows[0]
            .unresolved
            .contains(&"enclosing_cell_ownership_unresolved")
    );
    assert_eq!(
        out.unresolved_ranges
            .iter()
            .map(|r| r.paragraph_range)
            .collect::<Vec<_>>(),
        vec![[0, 1], [3, 4]]
    );
}
#[test]
fn nonrow_revision_and_suppressed_mark_propagate_to_row() {
    let ps = vec![p(0, 2, 7), p(2, 3, 7)];
    let mut first = state(1, true, false, 0).unwrap();
    first.revision = true;
    let out = assemble("body", &ps, vec![Some(first), state(1, false, true, 1)]).unwrap();
    assert!(
        out.rows[0]
            .unresolved
            .contains(&"revised_or_hidden_row_topology_unresolved")
    );
}

#[test]
fn nested_table_can_start_outer_cell_before_its_first_paragraph() {
    let ps = vec![p(0, 2, 13), p(2, 3, 13), p(3, 4, 7), p(4, 5, 7)];
    let out = assemble(
        "body",
        &ps,
        vec![
            state(2, true, false, 0),
            state(2, false, true, 1),
            state(1, true, false, 0),
            state(1, false, true, 1),
        ],
    )
    .unwrap();
    assert_eq!(out.rows.len(), 2);
    assert!(out.unresolved_ranges.is_empty());
    assert_eq!(out.rows[1].cells[0].source_range, [0, 4]);
    assert!(matches!(
        out.rows[1].cells[0].content[0],
        CellContent::NestedRow(0)
    ));
    assert!(out.rows[0].unresolved.is_empty());
}

#[test]
fn missing_pending_ancestor_retains_completed_nested_row() {
    let ps = vec![p(0, 2, 13), p(2, 3, 13)];
    let out = assemble(
        "body",
        &ps,
        vec![state(3, true, false, 0), state(3, false, true, 1)],
    )
    .unwrap();
    assert_eq!(out.rows.len(), 1);
    assert!(out.unresolved_ranges.is_empty());
    assert!(
        out.rows[0]
            .unresolved
            .contains(&"enclosing_cell_ownership_unresolved")
    );
}

#[test]
fn unknown_boundary_cannot_create_pending_ancestors() {
    let ps = vec![
        p(0, 1, 13),
        p(1, 2, 13),
        p(2, 3, 13),
        p(3, 4, 7),
        p(4, 5, 7),
        p(5, 6, 7),
    ];
    let out = assemble(
        "body",
        &ps,
        vec![
            None,
            state(2, true, false, 0),
            state(2, false, true, 1),
            state(1, false, true, 1),
            state(1, true, false, 0),
            state(1, false, true, 1),
        ],
    )
    .unwrap();
    assert_eq!(out.rows.len(), 1);
    assert_eq!(out.rows[0].source_range, [4, 6]);
    assert_eq!(
        out.unresolved_ranges
            .iter()
            .map(|r| r.paragraph_range[1] - r.paragraph_range[0])
            .sum::<usize>(),
        4
    );
}

#[test]
fn enclosing_projection_allows_unsplittable_children_but_rejects_reversed_cuts() {
    let mut ps = vec![p(0, 1, 13), p(1, 2, 13), p(2, 3, 7)];
    assert_eq!(range(&ps, 0, 3).1, Some([0, 3]));
    ps[1].projected_bytes = None;
    assert_eq!(range(&ps, 0, 3).1, Some([0, 3]));
    ps[1].projected_bytes = Some([2, 4]);
    assert_eq!(range(&ps, 0, 3).1, None);
}

#[test]
fn inner_row_flag_refines_cell_mark_designation() {
    let data = [
        0x16, 0x24, 1, 0x49, 0x66, 2, 0, 0, 0, 0x4b, 0x24, 1, 0x4c, 0x24, 1,
    ];
    let s = resolve(&data, 0, &[], Some(13), 0x112, &mut 100).unwrap();
    assert!(s.row);
    assert!(!s.cell);
}

#[test]
fn repeated_short_insert_operands_cannot_amplify_definition_storage() {
    let data = [0x21, 0x76, 0, 63, 1, 0];
    let s = resolve(&data, 0, &[], Some(13), 0x112, &mut 100).unwrap();
    let mut budget = 100;
    charge_definitions(&mut budget, s.definitions.len()).unwrap();
    assert!(matches!(
        charge_definitions(&mut budget, s.definitions.len()),
        Err(Error::Limit(_))
    ));
}

#[test]
fn paragraph_group_and_frame_lock_preserve_membership_and_context() {
    let data = [0x16, 0x24, 1, 0x65, 0x64, 7, 0, 0, 0, 0x30, 0x24, 1];
    let cell = resolve(&data, 0, &[], Some(7), 0xc1, &mut 100).unwrap();
    assert_eq!(cell.depth, 1);
    assert!(cell.cell);
    let out = assemble(
        "body",
        &[p(0, 2, 7), p(2, 3, 7)],
        vec![Some(cell), state(1, false, true, 1)],
    )
    .unwrap();
    assert_eq!(out.rows.len(), 1);
    assert!(out.unresolved_ranges.is_empty());
    assert_eq!(out.paragraph_contexts.len(), 1);
    let context = &out.paragraph_contexts[0];
    assert_eq!(context.paragraph_index, 0);
    assert_eq!(context.paragraph_group_id, Some(7));
    assert_eq!(context.frame_anchor_locked, Some(true));
    assert_eq!(context.unresolved.len(), 2);
}

#[test]
fn outside_paragraph_context_retains_ordered_frame_override() {
    let data = [0x65, 0x64, 9, 0, 0, 0, 0x30, 0x24, 1, 0x30, 0x24, 0];
    let s = resolve(&data, 0, &[], Some(13), 0x112, &mut 100).unwrap();
    let out = assemble("body", &[p(0, 2, 13)], vec![Some(s)]).unwrap();
    assert_eq!(out.outside_paragraphs, [0]);
    assert_eq!(out.paragraph_contexts[0].paragraph_group_id, Some(9));
    assert_eq!(out.paragraph_contexts[0].frame_anchor_locked, Some(false));
}

#[test]
fn context_operand_validation_keeps_unknown_structure_modifiers_explicit() {
    assert!(resolve(&[0x30, 0x24, 2], 0, &[], Some(13), 0x112, &mut 100).is_err());
    for n in 0..4 {
        let mut data = vec![0x65, 0x64];
        data.extend(vec![0; n]);
        assert!(resolve(&data, 0, &[], Some(13), 0x112, &mut 100).is_err());
    }
    let data = [0x65, 0x64, 7, 0, 0, 0, 0xfe, 0x24, 0];
    assert!(matches!(
        resolve(&data, 0, &[], Some(13), 0x112, &mut 100),
        Err(Error::Unsupported(_))
    ));
}

#[test]
fn invalid_frame_lock_keeps_only_its_paragraph_context_unresolved() {
    assert!(matches!(
        resolve(&[0x30, 0x24, 0xff], 0, &[], Some(13), 0x112, &mut 100),
        Err(Error::Unsupported(_))
    ));
    let out = assemble(
        "body",
        &[p(0, 1, 13), p(1, 2, 7), p(2, 3, 7)],
        vec![None, state(1, true, false, 0), state(1, false, true, 1)],
    )
    .unwrap();
    assert!(out.rows.is_empty());
    let out = assemble(
        "body",
        &[p(0, 1, 13), p(1, 2, 13), p(2, 3, 7), p(3, 4, 7)],
        vec![
            None,
            state(0, false, false, 0),
            state(1, true, false, 0),
            state(1, false, true, 1),
        ],
    )
    .unwrap();
    assert_eq!(out.rows.len(), 1);
    assert_eq!(out.unresolved_ranges[0].paragraph_range, [0, 1]);
}

#[test]
fn unexpected_terminal_preserves_adjacent_supported_ownership() {
    let mut work = 1000;
    assert!(matches!(
        resolve(&[], 0, &[], Some(14), 0xc1, &mut work),
        Err(Error::Unsupported(_))
    ));
    let supported = resolve(&[], 0, &[], Some(13), 0xc1, &mut work).unwrap();
    let out = assemble(
        "body",
        &[p(0, 1, 14), p(1, 2, 13), p(2, 3, 7), p(3, 4, 7)],
        vec![
            None,
            Some(supported),
            state(1, true, false, 0),
            state(1, false, true, 1),
        ],
    )
    .unwrap();
    assert_eq!(out.unresolved_ranges[0].paragraph_range, [0, 1]);
    assert_eq!(out.outside_paragraphs, vec![1]);
    assert_eq!(out.rows.len(), 1);
}
