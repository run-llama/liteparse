use super::*;

fn record(kind: u16, version: u16, instance: u16, data: &[u8]) -> Vec<u8> {
    let mut out = ((instance << 4) | version).to_le_bytes().to_vec();
    out.extend(kind.to_le_bytes());
    out.extend((data.len() as u32).to_le_bytes());
    out.extend(data);
    out
}

fn fixture(fill: u32, extra_default: bool, extra_shape: bool) -> Vec<u8> {
    let mut info = vec![0; 16];
    info[4..8].copy_from_slice(&1u32.to_le_bytes());
    let mut root = record(0xf006, 0, 0, &info);
    let mut defaults = Vec::new();
    for (key, value) in [(0xc186u16, 0u32), (0xc1c5, 0)] {
        defaults.extend(key.to_le_bytes());
        defaults.extend(value.to_le_bytes());
    }
    if extra_default {
        defaults.extend(0x023fu16.to_le_bytes());
        defaults.extend(0x20002u32.to_le_bytes());
    }
    root.extend(record(
        0xf00b,
        3,
        if extra_default { 3 } else { 2 },
        &defaults,
    ));
    let mut out = record(0xf000, 15, 0, &root);
    out.push(0);
    let mut group_shape = record(0xf009, 1, 0, &[0; 16]);
    group_shape.extend(record(0xf00a, 2, 0, &[1, 0, 0, 0, 5, 0, 0, 0]));
    let group = record(0xf003, 15, 0, &record(0xf004, 15, 0, &group_shape));
    let mut drawing = record(0xf008, 0, 1, &[1, 0, 0, 0, 2, 0, 0, 0]);
    drawing.extend(group);
    let mut shape = record(0xf00a, 2, 1, &[2, 0, 0, 0, 0, 14, 0, 0]);
    let mut props = Vec::new();
    for (key, value) in [
        (0x1bfu16, fill),
        (0x1cb, 0),
        (0x1ff, 0x80000),
        (0x304, 9),
        (0x33f, 0x10001),
    ] {
        props.extend(key.to_le_bytes());
        props.extend(value.to_le_bytes());
    }
    shape.extend(record(0xf00b, 3, 5, &props));
    shape.extend(record(0xf011, 0, 0, &[1, 0, 0, 0]));
    if extra_shape {
        shape.extend(record(0xf00d, 0, 0, &[1, 0, 0, 0]));
    }
    drawing.extend(record(0xf004, 15, 0, &shape));
    out.extend(record(0xf002, 15, 0, &drawing));
    out
}

#[test]
fn nonpainting_background_retains_record_ownership() {
    let data = fixture(0x100000, false, false);
    let proof = qualify_content(&data, 100).unwrap();
    assert_eq!(proof.background_shapes, 1);
    assert_eq!(proof.table_range, [100, 100 + data.len()]);
    assert!(
        proof
            .records
            .iter()
            .all(|r| r.table_range[0] >= 100 && r.table_range[1] <= 100 + data.len())
    );
    let properties: Vec<_> = proof.records.iter().flat_map(|r| &r.properties).collect();
    assert_eq!(properties.len(), 7);
    for property in properties {
        let offset = property.table_range[0] - 100;
        assert_eq!(property.table_range[1] - property.table_range[0], 6);
        assert_eq!(u16_at(&data, offset).unwrap(), property.raw_property_id);
        assert_eq!(u32_at(&data, offset + 2).unwrap(), property.raw_operand);
    }
}

#[test]
fn explicit_painted_background_and_text_remain_rejected() {
    for data in [
        fixture(0x100010, false, false),
        fixture(0x100000, false, true),
    ] {
        assert!(qualify_content(&data, 0).is_err());
    }
    let mut data = fixture(0x100000, false, false);
    let pattern = [0xff, 1, 0, 0, 8, 0];
    let offset = data.windows(6).position(|w| w == pattern).unwrap();
    data[offset + 2] = 8;
    assert!(qualify_content(&data, 0).is_err());
}

#[test]
fn truncated_records_and_duplicate_stories_are_rejected() {
    let data = fixture(0x100000, false, false);
    for end in 0..data.len() {
        // A complete drawing-group alone is a valid no-object document.
        if end != 8 + u32_at(&data, 4).unwrap() as usize {
            assert!(qualify_content(&data[..end], 0).is_err(), "{end}");
        }
    }
    let root_end = 8 + u32_at(&data, 4).unwrap() as usize;
    let mut duplicate = data.clone();
    duplicate.extend_from_slice(&data[root_end..]);
    assert!(qualify_content(&duplicate, 0).is_err());
}

#[test]
fn floating_anchors_require_ownership_when_drawing_group_is_absent() {
    let mut word = vec![0; 154 + 93 * 8 + 2];
    for (offset, value) in [(0, 0xa5ecu16), (2, 0xc1), (32, 14), (62, 22), (152, 93)] {
        word[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }
    assert!(qualify(&word, &[]).unwrap().is_none());
    for pair in [40, 41] {
        let offset = 158 + pair * 8;
        word[offset..offset + 4].copy_from_slice(&4u32.to_le_bytes());
        assert!(matches!(qualify(&word, &[]), Err(Error::Unsupported(_))));
        word[offset..offset + 4].fill(0);
    }
}

fn replace_root_tail(data: &[u8], tail: &[u8]) -> Vec<u8> {
    let old_end = 8 + u32_at(data, 4).unwrap() as usize;
    let info_end = 16 + u32_at(data, 12).unwrap() as usize;
    let mut root = data[8..info_end].to_vec();
    root.extend(tail);
    let mut result = record(0xf000, 15, 0, &root);
    result.extend(&data[old_end..]);
    result
}

#[test]
fn split_menu_colors_do_not_require_a_default_property_record() {
    let data = fixture(0x100000, false, false);
    let colors = record(0xf11e, 0, 4, &[0; 16]);
    let without_defaults = replace_root_tail(&data, &colors);
    let proof = qualify_content(&without_defaults, 0).unwrap();
    assert_eq!(proof.background_shapes, 1);
    assert_eq!(
        proof
            .records
            .iter()
            .filter(|r| r.record_type == 0xf00b)
            .count(),
        1
    );
    assert!(qualify_content(&replace_root_tail(&data, &[]), 0).is_ok());
}

#[test]
fn color_only_root_still_rejects_unknown_duplicate_and_truncated_records() {
    let data = fixture(0x100000, false, false);
    let colors = record(0xf11e, 0, 4, &[0; 16]);
    let mut duplicate = colors.clone();
    duplicate.extend(&colors);
    for tail in [
        duplicate,
        record(0xf123, 3, 0, &[]),
        record(0xf11e, 0, 4, &[0; 12]),
        vec![0, 0],
    ] {
        assert!(qualify_content(&replace_root_tail(&data, &tail), 0).is_err());
    }
}

fn textbox_fixture(client_anchor: u32, client_text: u32) -> Vec<u8> {
    let mut info = vec![0; 16];
    info[4..8].copy_from_slice(&1u32.to_le_bytes());
    let root = record(0xf006, 0, 0, &info);
    let mut output = record(0xf000, 15, 0, &root);
    output.push(0);
    let mut group_shape = record(0xf009, 1, 0, &[0; 16]);
    group_shape.extend(record(0xf00a, 2, 0, &[1, 0, 0, 0, 5, 0, 0, 0]));
    let mut group = record(0xf004, 15, 0, &group_shape);
    let mut shape = record(0xf00a, 2, 202, &[3, 0, 0, 0, 0, 10, 0, 0]);
    let mut props = Vec::new();
    for (key, value) in [
        (0x80u16, 0x10000u32),
        (0x181, 0x800000),
        (0x1bf, 0x100010),
        (0x1ff, 0x80000),
    ] {
        props.extend(key.to_le_bytes());
        props.extend(value.to_le_bytes());
    }
    shape.extend(record(0xf00b, 3, 4, &props));
    let mut tertiary = Vec::new();
    for (key, value) in [
        (0x53fu16, 0x10000u32),
        (0x57f, 0x80000),
        (0x5bf, 0x80000),
        (0x5ff, 0x80000),
        (0x63f, 0x80000),
    ] {
        tertiary.extend(key.to_le_bytes());
        tertiary.extend(value.to_le_bytes());
    }
    shape.extend(record(0xf122, 3, 5, &tertiary));
    shape.extend(record(0xf010, 0, 0, &client_anchor.to_le_bytes()));
    shape.extend(record(0xf011, 0, 0, &[0; 4]));
    shape.extend(record(0xf00d, 0, 0, &client_text.to_le_bytes()));
    group.extend(record(0xf004, 15, 0, &shape));
    let mut drawing = record(0xf008, 0, 1, &[2, 0, 0, 0, 3, 0, 0, 0]);
    drawing.extend(record(0xf003, 15, 0, &group));
    output.extend(record(0xf002, 15, 0, &drawing));
    output
}

fn textbox_anchor(shape_id: u32) -> textbox::RawAnchor {
    textbox::RawAnchor {
        shape_id,
        cp: 7,
        rectangle: [0, -540, 8820, 1050],
        flags: 0x74,
    }
}

#[test]
fn complete_textbox_record_joins_client_anchor_shape_and_text_identity() {
    let data = textbox_fixture(0, 0x10000);
    let proof = qualify_with_anchors(&data, 100, vec![textbox_anchor(3)]).unwrap();
    let shape = &proof.textboxes()[0];
    assert_eq!(
        (shape.shape_id, shape.anchor_cp, shape.textbox_record_index),
        (3, 7, 0)
    );
    assert_eq!(shape.rectangle_twips, [0, -540, 8820, 1050]);
    assert_eq!(shape.fill_rgb, Some(0x000080));
    assert!(shape.table_range[0] >= 100 && shape.table_range[1] <= 100 + data.len());
}

#[test]
fn mismatched_or_unused_anchor_and_linked_textbox_identity_are_rejected() {
    for (index, text, shape_id) in [
        (1, 0x10000, 3),
        (0, 0x10000, 4),
        (0, 0x10001, 3),
        (0, 0x20000, 3),
    ] {
        assert!(
            qualify_with_anchors(
                &textbox_fixture(index, text),
                0,
                vec![textbox_anchor(shape_id)]
            )
            .is_err()
        );
    }
    assert!(
        qualify_with_anchors(
            &textbox_fixture(0, 0x10000),
            0,
            vec![textbox_anchor(3), textbox_anchor(4)]
        )
        .is_err()
    );
}

#[test]
fn additional_or_active_tertiary_properties_remain_unresolved() {
    let data = textbox_fixture(0, 0x10000);
    let proof = qualify_with_anchors(&data, 0, vec![textbox_anchor(3)]).unwrap();
    let tertiary = proof
        .records
        .iter()
        .find(|r| r.record_type == 0xf122)
        .unwrap();
    assert_eq!(tertiary.properties.len(), 5);
    for property in &tertiary.properties {
        let mut altered = data.clone();
        altered[property.table_range[0] + 2] ^= 1;
        let partial = qualify_with_anchors(&altered, 0, vec![textbox_anchor(3)]).unwrap();
        let shape = &partial.textboxes[0];
        assert_eq!((shape.shape_id, shape.textbox_record_index), (3, 0));
        assert_eq!(shape.appearance_approximations.len(), 1);
        let evidence = &shape.appearance_approximations[0];
        assert_eq!(
            &altered[evidence.table_range[0] + 8..evidence.table_range[1]],
            evidence.source_payload
        );
    }
}

#[test]
fn inherited_effects_preserve_source_and_mark_default_appearance_partial() {
    let data = fixture(0x100000, true, false);
    let proof = qualify_content(&data, 0).unwrap();
    assert_eq!(proof.document_defaults.len(), 1);
    let record = &proof.document_defaults[0];
    assert_eq!(
        &data[record.table_range[0] + 8..record.table_range[1]],
        record.source_payload
    );
    assert!(record.applied_text_properties.is_empty());
    assert!(proof.basis.contains("partial_scene"));
}
#[test]
fn recent_colors_are_retained_without_becoming_shape_defaults() {
    let data = fixture(0x100000, false, false);
    let recent = record(0xf11a, 0, 1, &[0x80, 0, 0, 0]);
    let proof = qualify_content(&replace_root_tail(&data, &recent), 0).unwrap();
    assert!(proof.document_defaults.is_empty());
    assert!(
        proof
            .records
            .iter()
            .any(|record| record.record_type == 0xf11a)
    );
}

#[test]
fn regroup_history_is_retained_without_changing_shape_appearance() {
    let original = fixture(0x100000, false, false);
    let drawing = 8 + u32_at(&original, 4).unwrap() as usize + 1;
    let insertion = drawing + 24;
    for (new, old, valid) in [(2u16, 1u16, true), (1, 2, false), (1, 1, false)] {
        let mut payload = new.to_le_bytes().to_vec();
        payload.extend(old.to_le_bytes());
        let history = record(0xf118, 0, 1, &payload);
        let mut data = original.clone();
        let size = u32_at(&data, drawing + 4).unwrap() + history.len() as u32;
        data[drawing + 4..drawing + 8].copy_from_slice(&size.to_le_bytes());
        data.splice(insertion..insertion, history);
        let result = qualify_content(&data, 0);
        if valid {
            let proof = result.unwrap();
            assert_eq!(proof.background_shapes, 1);
            assert!(
                proof
                    .records
                    .iter()
                    .any(|record| record.record_type == 0xf118)
            );
            assert!(proof.document_defaults.is_empty());
        } else {
            assert!(result.is_err());
        }
    }
}

#[test]
fn conflicting_insets_across_default_tables_remain_unresolved_in_either_order() {
    let original = fixture(0x100000, false, false);
    for (first, second) in [(72000u32, 60000u32), (60000, 72000), (72000, 72000)] {
        let mut a = 0x81u16.to_le_bytes().to_vec();
        a.extend(first.to_le_bytes());
        let mut b = 0x81u16.to_le_bytes().to_vec();
        b.extend(second.to_le_bytes());
        let mut tail = record(0xf00b, 3, 1, &a);
        tail.extend(record(0xf122, 3, 1, &b));
        let result = qualify_content(&replace_root_tail(&original, &tail), 0);
        assert_eq!(result.is_ok(), first == second);
    }
}

fn with_nonzero_top_coordinates(mut data: Vec<u8>) -> Vec<u8> {
    let needle = record(0xf009, 1, 0, &[0; 16]);
    let offset = data
        .windows(needle.len())
        .position(|p| p == needle)
        .unwrap()
        + 8;
    for (index, value) in [169i32, 256, 174, 20_758_360].iter().enumerate() {
        data[offset + index * 4..offset + index * 4 + 4].copy_from_slice(&value.to_le_bytes());
    }
    data
}

#[test]
fn unused_top_coordinates_require_empty_group_and_no_orphan_anchors() {
    let data = with_nonzero_top_coordinates(fixture(0x100000, false, false));
    let proof = qualify_content(&data, 100).unwrap();
    let coordinate = proof
        .records
        .iter()
        .find(|r| r.unused_empty_group_coordinates.is_some())
        .unwrap();
    assert_eq!(
        coordinate.unused_empty_group_coordinates,
        Some([169, 256, 174, 20_758_360])
    );
    let start = coordinate.table_range[0] - 100 + 8;
    assert_eq!(
        i32::from_le_bytes(data[start..start + 4].try_into().unwrap()),
        169
    );
    assert!(qualify_with_anchors(&data, 100, vec![textbox_anchor(3)]).is_err());
    let occupied = textbox_fixture(0, 0x10000);
    assert!(qualify_with_anchors(&occupied, 0, vec![textbox_anchor(3)]).is_ok());
    assert!(
        qualify_with_anchors(
            &with_nonzero_top_coordinates(occupied),
            0,
            vec![textbox_anchor(3)]
        )
        .is_err()
    );
}

fn background_variant(properties: &[(u16, u32)], tail: &[u8]) -> Vec<u8> {
    let mut info = vec![0; 16];
    info[4..8].copy_from_slice(&1u32.to_le_bytes());
    let mut out = record(0xf000, 15, 0, &record(0xf006, 0, 0, &info));
    out.push(0);
    let mut head = record(0xf009, 1, 0, &[0; 16]);
    head.extend(record(0xf00a, 2, 0, &[1, 0, 0, 0, 5, 0, 0, 0]));
    let mut drawing = record(0xf008, 0, 1, &[1, 0, 0, 0, 2, 0, 0, 0]);
    drawing.extend(record(0xf003, 15, 0, &record(0xf004, 15, 0, &head)));
    let mut shape = record(0xf00a, 2, 1, &[2, 0, 0, 0, 0, 14, 0, 0]);
    let props: Vec<u8> = properties
        .iter()
        .flat_map(|(k, v)| k.to_le_bytes().into_iter().chain(v.to_le_bytes()))
        .collect();
    shape.extend(record(0xf00b, 3, properties.len() as u16, &props));
    shape.extend(tail);
    drawing.extend(record(0xf004, 15, 0, &shape));
    out.extend(record(0xf002, 15, 0, &drawing));
    out
}

#[test]
fn background_hit_testing_and_optional_client_preserve_explicit_nonpaint() {
    for fill in [0x100000, 0x110000, 0x110001] {
        let properties = [(0x1bf, fill), (0x1ff, 0x80000), (0x33f, 0x10001)];
        for tail in [vec![], record(0xf011, 0, 0, &[1, 0, 0, 0])] {
            let proof = qualify_content(&background_variant(&properties, &tail), 0).unwrap();
            assert_eq!(proof.background_shapes, 1);
            assert!(
                proof
                    .records
                    .iter()
                    .flat_map(|r| &r.properties)
                    .any(|p| p.raw_property_id == 0x1bf && p.raw_operand == fill)
            );
        }
        for tail in [
            record(0xf011, 0, 0, &[0; 3]),
            record(0xf00d, 0, 0, &[0; 4]),
            record(0xf010, 0, 0, &[0; 4]),
        ] {
            assert!(qualify_content(&background_variant(&properties, &tail), 0).is_err());
        }
        let mut active = properties;
        active[0].1 |= 0x10;
        assert!(qualify_content(&background_variant(&active, &[]), 0).is_err());
    }
    for properties in [
        vec![(0x1ff, 0x80000), (0x33f, 0x10001)],
        vec![(0x1bf, 0), (0x1ff, 0x80000), (0x33f, 0x10001)],
        vec![
            (0x104, 1),
            (0x1bf, 0x100000),
            (0x1ff, 0x80000),
            (0x33f, 0x10001),
        ],
        vec![
            (0x80, 0x10000),
            (0x1bf, 0x100000),
            (0x1ff, 0x80000),
            (0x33f, 0x10001),
        ],
    ] {
        assert!(qualify_content(&background_variant(&properties, &[]), 0).is_err());
    }
}

#[test]
fn header_textbox_uses_header_anchor_table_and_rejects_body_alias() {
    let mut data = textbox_fixture(0, 0x10000);
    let story_offset = 8 + u32_at(&data, 4).unwrap() as usize;
    data[story_offset] = 1;
    let mut header = textbox_anchor(3);
    header.cp = 100;
    let proof = qualify_with_story_anchors(&data, 0, vec![], vec![header]).unwrap();
    assert_eq!(proof.textboxes()[0].anchor_cp, 100);
    assert!(qualify_with_story_anchors(&data, 0, vec![textbox_anchor(3)], vec![]).is_err());
    assert!(qualify_with_story_anchors(&data, 0, vec![], vec![textbox_anchor(4)]).is_err());
}

#[test]
fn square_textbox_joins_raw_anchor_and_keeps_side_without_body_overlay_alias() {
    let data = textbox_fixture(0, 0x10000);
    let mut raw = textbox_anchor(3);
    raw.flags = 0x4454;
    let proof = qualify_with_anchors(&data, 100, vec![raw]).unwrap();
    let shape = &proof.textboxes()[0];
    assert_eq!(
        (shape.shape_id, shape.anchor_cp, shape.textbox_record_index),
        (3, 7, 0)
    );
    let wrap = shape.square_wrap.as_ref().unwrap();
    assert_eq!(wrap.side, WrapSide::Right);
    assert_eq!(wrap.raw_anchor_flags, 0x4454);
    assert!(!shape.behind_text);
    let mut wrong = textbox_anchor(4);
    wrong.flags = 0x54;
    assert!(qualify_with_anchors(&data, 100, vec![wrong]).is_err());
}

#[test]
fn top_bottom_textbox_keeps_source_identity_and_ignored_side_bits() {
    let data = textbox_fixture(0, 0x10000);
    let mut raw = textbox_anchor(3);
    raw.flags = 0x5e34;
    let proof = qualify_with_anchors(&data, 100, vec![raw]).unwrap();
    let shape = &proof.textboxes()[0];
    assert_eq!(
        (shape.shape_id, shape.anchor_cp, shape.textbox_record_index),
        (3, 7, 0)
    );
    assert_eq!(
        shape.top_bottom_wrap.as_ref().unwrap().raw_anchor_flags,
        0x5e34
    );
    assert!(shape.square_wrap.is_none());
    assert!(!shape.behind_text);
}
