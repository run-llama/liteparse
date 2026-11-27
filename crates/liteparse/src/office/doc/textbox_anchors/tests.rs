use super::*;

fn story(kind: &'static str, start_cp: usize, text: &str) -> Story {
    Story {
        kind,
        start_cp,
        utf16_units: text.encode_utf16().count(),
        text: text.to_owned(),
    }
}

fn shapes(entries: &[(u32, u32)], terminal: u32) -> Vec<u8> {
    let mut result = Vec::new();
    for &(cp, _) in entries {
        result.extend(cp.to_le_bytes());
    }
    result.extend(terminal.to_le_bytes());
    for &(_, id) in entries {
        result.extend(id.to_le_bytes());
        for value in [-10i32, 20, 300, 400] {
            result.extend(value.to_le_bytes());
        }
        result.extend(0u16.to_le_bytes());
        result.extend(u32::MAX.to_le_bytes());
    }
    result
}

fn textboxes(cps: &[u32], entries: &[(u16, u32, u32)]) -> Vec<u8> {
    assert_eq!(cps.len(), entries.len() + 1);
    let mut result = Vec::new();
    for cp in cps {
        result.extend(cp.to_le_bytes());
    }
    for &(reusable, id, chain) in entries {
        result.extend(chain.to_le_bytes());
        result.extend(0u32.to_le_bytes());
        result.extend(reusable.to_le_bytes());
        result.extend(0u32.to_le_bytes());
        result.extend(id.to_le_bytes());
        result.extend(0u32.to_le_bytes());
    }
    result
}

#[test]
fn maps_shape_ids_without_reordering_textbox_storage() {
    let owner = story("body", 0, "a\u{8}bc\u{8}\r");
    let text = story("textboxes", 100, "late\rearly\r\r\r");
    let plc = textboxes(
        &[0, 5, 11, u32::MAX],
        &[(0, 20, 1), (0, 10, 1), (0, 0, u32::MAX)],
    );
    let result = decode_part(
        &text,
        &owner,
        &plc,
        &shapes(&[(1, 10), (4, 20)], 99),
        &mut { MAX_RECORDS },
    )
    .unwrap();
    assert_eq!(result.records[0].anchor_index, Some(1));
    assert_eq!(result.records[1].anchor_index, Some(0));
    assert_eq!(result.records[0].source.as_ref().unwrap().start_cp, 100);
    assert_eq!(result.records[1].source.as_ref().unwrap().end_cp, 111);
    assert!(result.records[2].reusable);
    assert!(result.records[2].source.is_none());
    assert_eq!(result.shapes[0].rectangle_twips, [-10, 20, 300, 400]);
}

#[test]
fn terminal_spare_ignores_text_bounds_and_union_values() {
    let owner = story("body", 0, "\u{8}");
    let text = story("textboxes", 1, "x\r\r\r");
    let plc = textboxes(&[0, 2, u32::MAX], &[(0, 7, 1), (0, 900, u32::MAX)]);
    let result = decode_part(&text, &owner, &plc, &shapes(&[(0, 7)], 1), &mut {
        MAX_RECORDS
    })
    .unwrap();
    let spare = &result.records[1];
    assert!(spare.reusable);
    assert!(spare.shape_id.is_none());
    assert!(spare.chain_shape_count.is_none());
    assert!(spare.anchor_index.is_none());
}

#[test]
fn retains_nonterminal_spare_and_linked_chain_uncertainty() {
    let owner = story("body", 0, "\u{8}");
    let text = story("textboxes", 20, "\rabc\r\r");
    let plc = textboxes(&[0, 1, 5, 6], &[(3, 0, u32::MAX), (0, 7, 3), (0, 0, 0)]);
    let result = decode_part(&text, &owner, &plc, &shapes(&[(0, 7)], 1), &mut {
        MAX_RECORDS
    })
    .unwrap();
    assert!(result.records[0].source.is_none());
    assert_eq!(result.records[1].anchor_index, Some(0));
    assert_eq!(result.records[1].chain_shape_count, Some(3));
    assert_eq!(result.records[1].anchor_status, "first_chain_anchor_only");
}

#[test]
fn header_shape_positions_are_header_relative() {
    let owner = story("headers", 100, "a\u{8}");
    let text = story("header_textboxes", 200, "x\r\r");
    let plc = textboxes(&[0, 2, 3], &[(0, 7, 1), (0, 0, 0)]);
    let result = decode_part(&text, &owner, &plc, &shapes(&[(1, 7)], 2), &mut {
        MAX_RECORDS
    })
    .unwrap();
    assert_eq!(result.shapes[0].source_cp, 101);
    assert_eq!(result.records[0].source.as_ref().unwrap().start_cp, 200);
    assert!(
        decode_part(&text, &story("body", 0, "\u{8}"), &plc, &[], &mut {
            MAX_RECORDS
        })
        .is_err()
    );
}

#[test]
fn absent_and_duplicate_shape_ids_remain_unresolved() {
    let owner = story("body", 0, "\u{8}\u{8}");
    let text = story("textboxes", 2, "a\rb\r\r");
    let plc = textboxes(&[0, 2, 4, 5], &[(0, 7, 1), (0, 9, 1), (0, 0, 0)]);
    let result = decode_part(&text, &owner, &plc, &shapes(&[(0, 7), (1, 7)], 2), &mut {
        MAX_RECORDS
    })
    .unwrap();
    assert_eq!(result.records[0].anchor_status, "duplicate_shape_id");
    assert_eq!(
        result.records[1].anchor_status,
        "shape_hierarchy_unresolved"
    );
    assert!(
        result
            .records
            .iter()
            .all(|record| record.anchor_index.is_none())
    );
}

#[test]
fn rejects_invalid_shape_cp_and_source_character() {
    let owner = story("body", 0, "\u{8}a\u{8}");
    assert!(decode_shapes(&owner, &[8, 97, 8], &shapes(&[(1, 1)], 3), 1).is_err());
    assert!(decode_shapes(&owner, &[8, 97, 8], &shapes(&[(0, 1), (0, 2)], 3), 2).is_err());
    assert!(decode_shapes(&owner, &[8, 97, 8], &shapes(&[(2, 1)], 2), 1).is_err());
}

#[test]
fn rejects_active_range_and_spare_invariants() {
    let owner = story("body", 0, "\u{8}");
    let text = story("textboxes", 1, "ab\r\r");
    for plc in [
        textboxes(&[0, 2, 4], &[(0, 7, 1), (0, 0, 0)]),
        textboxes(&[0, 5, 6], &[(0, 7, 1), (0, 0, 0)]),
        textboxes(&[0, 3, 4], &[(2, 7, 1), (0, 0, 0)]),
        textboxes(&[0, 3, 4], &[(1, 0, 0), (0, 0, 0)]),
        textboxes(&[0, 3, 4], &[(0, 7, 0), (0, 0, 0)]),
    ] {
        assert!(
            decode_part(&text, &owner, &plc, &shapes(&[(0, 7)], 1), &mut {
                MAX_RECORDS
            })
            .is_err()
        );
    }
}

#[test]
fn rejects_utf16_split_and_bounds_allocations() {
    let owner = story("body", 0, "\u{8}");
    let text = story("textboxes", 1, "😀\r\r");
    let plc = textboxes(&[1, 3, 4], &[(0, 7, 1), (0, 0, 0)]);
    assert!(
        decode_part(&text, &owner, &plc, &shapes(&[(0, 7)], 1), &mut {
            MAX_RECORDS
        })
        .is_err()
    );
    assert!(plc_count(&[0; 30], 26, &mut { MAX_RECORDS }).is_err());
    assert!(plc_count(&shapes(&[(0, 1)], 1), 26, &mut 0).is_err());
    let mut invalid = story("body", 0, "a");
    invalid.utf16_units = 0;
    assert!(story_units(&invalid).is_err());
    invalid.utf16_units = MAX_STORY_UNITS + 1;
    assert!(matches!(story_units(&invalid), Err(Error::Limit(_))));
    invalid.utf16_units = 1;
    invalid.start_cp = usize::MAX;
    assert!(story_units(&invalid).is_err());
}

#[test]
fn empty_textbox_story_can_still_have_picture_shapes() {
    let owner = story("body", 0, "\u{8}");
    let text = story("textboxes", 1, "");
    let result = decode_part(&text, &owner, &[], &shapes(&[(0, 7)], 1), &mut {
        MAX_RECORDS
    })
    .unwrap();
    assert_eq!(result.shapes.len(), 1);
    assert!(result.records.is_empty());
    assert!(
        decode_part(&story("textboxes", 1, "x"), &owner, &[], &[], &mut {
            MAX_RECORDS
        })
        .is_err()
    );
}
