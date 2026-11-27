use super::*;
fn record(kind: u16, version: u16, instance: u16, data: &[u8]) -> Vec<u8> {
    let mut out = ((instance << 4) | version).to_le_bytes().to_vec();
    out.extend(kind.to_le_bytes());
    out.extend((data.len() as u32).to_le_bytes());
    out.extend(data);
    out
}
fn rectangle(values: [i32; 4]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}
fn shape(id: u32, kind: u16, text: bool) -> Vec<u8> {
    let mut data = id.to_le_bytes().to_vec();
    data.extend(0xa02u32.to_le_bytes());
    let mut out = record(0xf00a, 2, kind, &data);
    let mut props = 0x80u16.to_le_bytes().to_vec();
    props.extend(0x10000u32.to_le_bytes());
    if text {
        props.extend(0x81u16.to_le_bytes());
        props.extend(91440u32.to_le_bytes());
        props.extend(0x8au16.to_le_bytes());
        props.extend(id.to_le_bytes());
        out.extend(record(0xf00b, 3, 3, &props));
    } else {
        out.extend(record(0xf00b, 3, 0, &[]));
    }
    out.extend(record(0xf00f, 0, 0, &rectangle([175, 250, 325, 550])));
    out.extend(record(0xf011, 0, 0, &0u32.to_le_bytes()));
    if text {
        out.extend(record(0xf00d, 0, 0, &0x10000u32.to_le_bytes()));
    }
    record(0xf004, 15, 0, &out)
}
fn group(child_id: u32, extra_text: bool) -> Vec<u8> {
    let mut head = record(0xf009, 1, 0, &rectangle([100, 200, 400, 600]));
    let mut prop = 7u32.to_le_bytes().to_vec();
    prop.extend(0x201u32.to_le_bytes());
    head.extend(record(0xf00a, 2, 0, &prop));
    head.extend(record(0xf010, 0, 0, &0u32.to_le_bytes()));
    head.extend(record(0xf011, 0, 0, &0u32.to_le_bytes()));
    let mut children = record(0xf004, 15, 0, &head);
    children.extend(shape(child_id, 202, true));
    children.extend(shape(9, 1, extra_text));
    record(0xf003, 15, 0, &children)
}
fn reader() -> Reader {
    Reader {
        records: vec![],
        shape_ids: Default::default(),
        backgrounds: 0,
        anchors: vec![textbox::RawAnchor {
            shape_id: 7,
            cp: 5,
            rectangle: [20, 40, 220, 240],
            flags: 116,
        }],
        used_anchors: Default::default(),
        textboxes: vec![],
        images: vec![],
        lines: Vec::new(),
        rectangles: Vec::new(),
        header_anchors: vec![],
        used_header_anchors: Default::default(),
        store_count: 0,
        omitted_shapes: vec![],
        group_approximations: vec![],
        document_defaults: vec![],
        default_text_properties: Default::default(),
    }
}
#[test]
fn child_text_identity_and_group_transform_survive_sibling_paint_omission() {
    let data = group(8, false);
    let mut reader = reader();
    let mut offset = 0;
    let parsed = reader.record(&data, &mut offset, 100).unwrap();
    decode(&mut reader, parsed, 0, None).unwrap();
    let child = &reader.textboxes[0];
    assert_eq!(child.shape_id, 8);
    assert_eq!(child.source_next_shape, Some(8));
    assert_eq!(child.textbox_record_index, 0);
    assert_eq!(child.anchor_cp, 5);
    assert_eq!(
        child.group_rectangle_twips,
        Some([70.0, 65.0, 170.0, 215.0])
    );
    assert_eq!(child.group.as_ref().unwrap().anchor_shape_id, 7);
    assert_eq!(reader.omitted_shapes.len(), 1);
    assert_eq!(reader.omitted_shapes[0].shape_id, 9);
    let range = reader.omitted_shapes[0].table_range;
    assert_eq!(u16_at(&data, range[0] - 100 + 2).unwrap(), 0xf004);
}
#[test]
fn duplicate_group_child_or_unmodeled_text_cannot_be_omitted() {
    for data in [group(7, false), group(8, true)] {
        let mut reader = reader();
        let mut offset = 0;
        let parsed = reader.record(&data, &mut offset, 0).unwrap();
        assert!(decode(&mut reader, parsed, 0, None).is_err());
    }
}

#[test]
fn a_different_next_shape_cannot_reuse_single_owner_text() {
    let mut data = group(8, false);
    let needle = [0x8a, 0, 8, 0, 0, 0];
    let offset = data.windows(6).position(|window| window == needle).unwrap();
    data[offset + 2..offset + 6].copy_from_slice(&9u32.to_le_bytes());
    let mut reader = reader();
    let mut offset = 0;
    let parsed = reader.record(&data, &mut offset, 0).unwrap();
    assert!(matches!(
        decode(&mut reader, parsed, 0, None),
        Err(Error::Unsupported("render textbox linked target"))
    ));
}

#[test]
fn local_insets_override_defaults_and_other_sides_remain_inherited() {
    let data = group(8, false);
    let mut reader = reader();
    reader
        .default_text_properties
        .extend([(0x81, 72000), (0x82, 60000)]);
    let mut offset = 0;
    let parsed = reader.record(&data, &mut offset, 0).unwrap();
    decode(&mut reader, parsed, 0, None).unwrap();
    assert_eq!(
        reader.textboxes[0].text_insets_emu,
        [91440, 60000, 91440, 45720]
    );
}

#[test]
fn text_on_an_ordinary_shape_keeps_exact_textbox_identity() {
    let mut data = group(8, false);
    let header = ((202u16 << 4) | 2).to_le_bytes();
    let offset = data
        .windows(4)
        .position(|w| w == [header[0], header[1], 0x0a, 0xf0])
        .unwrap();
    data[offset..offset + 2].copy_from_slice(&((1u16 << 4) | 2).to_le_bytes());
    let mut reader = reader();
    let mut at = 0;
    let record = reader.record(&data, &mut at, 0).unwrap();
    decode(&mut reader, record, 0, None).unwrap();
    assert_eq!(reader.textboxes.len(), 1);
    let shape = &reader.textboxes[0];
    assert_eq!(
        (
            shape.shape_id,
            shape.textbox_record_index,
            shape.source_shape_type
        ),
        (8, 0, Some(1))
    );
    assert_eq!(shape.anchor_cp, 5);
    assert_eq!(reader.omitted_shapes.len(), 1);
}
