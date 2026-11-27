use super::super::textbox::RawAnchor;
use super::*;

fn record(kind: u16, version: u16, instance: u16, data: &[u8]) -> Vec<u8> {
    let mut out = ((instance << 4) | version).to_le_bytes().to_vec();
    out.extend(kind.to_le_bytes());
    out.extend((data.len() as u32).to_le_bytes());
    out.extend(data);
    out
}

fn fixture(index: u32, reference: u32) -> Vec<u8> {
    fixture_layer(index, reference, None)
}

fn fixture_layer(index: u32, reference: u32, behind: Option<bool>) -> Vec<u8> {
    let mut shape = record(0xf00a, 2, 75, &[2, 0, 0, 0, 0, 10, 0, 0]);
    let mut props = Vec::new();
    for (id, value) in [(0x4104u16, index), (0x1ff, 0x80000)] {
        props.extend(id.to_le_bytes());
        props.extend(value.to_le_bytes());
    }
    shape.extend(record(0xf00b, 3, 2, &props));
    let mut tertiary = 0x390u16.to_le_bytes().to_vec();
    tertiary.extend(reference.to_le_bytes());
    if let Some(behind) = behind {
        tertiary.extend(0x3bfu16.to_le_bytes());
        tertiary.extend((0x200000u32 | if behind { 0x20 } else { 0 }).to_le_bytes());
    }
    shape.extend(record(
        0xf122,
        3,
        if behind.is_some() { 2 } else { 1 },
        &tertiary,
    ));
    shape.extend(record(0xf010, 0, 0, &[0; 4]));
    shape.extend(record(0xf011, 0, 0, &[0; 4]));
    record(0xf004, 15, 0, &shape)
}

fn reader() -> Reader {
    Reader {
        records: Vec::new(),
        shape_ids: Default::default(),
        backgrounds: 0,
        anchors: Vec::new(),
        used_anchors: Default::default(),
        textboxes: Vec::new(),
        images: Vec::new(),
        lines: Vec::new(),
        rectangles: Vec::new(),
        used_header_anchors: Default::default(),
        omitted_shapes: vec![],
        group_approximations: vec![],
        document_defaults: vec![],
        default_text_properties: Default::default(),
        store_count: 1,
        header_anchors: vec![RawAnchor {
            shape_id: 2,
            cp: 100,
            rectangle: [0, 0, 200, 300],
            flags: 0xc060,
        }],
    }
}

#[test]
fn header_image_joins_exact_story_anchor_and_store_index() {
    let data = fixture(1, 0);
    let mut reader = reader();
    let record = reader.record(&data, &mut 0, 700).unwrap();
    let image = decode(&mut reader, record, 1).unwrap();
    assert_eq!(image.anchor_story, "headers");
    assert_eq!(image.anchor_cp, 100);
    assert_eq!(image.table_range, [700, 700 + data.len()]);
    assert!(image.behind_text && image.locked);
    assert_eq!(image.properties.blip_index, 1);
    assert_eq!(reader.used_header_anchors.len(), 1);
}

#[test]
fn wrong_store_reference_story_identity_and_repeated_anchor_reject() {
    for (index, reference, story, id) in [(2, 0, 1, 2), (1, 1, 1, 2), (1, 0, 0, 2), (1, 0, 1, 3)] {
        let data = fixture(index, reference);
        let mut reader = reader();
        reader.header_anchors[0].shape_id = id;
        let record = reader.record(&data, &mut 0, 0).unwrap();
        assert!(decode(&mut reader, record, story).is_err());
    }
    let data = fixture(1, 0);
    let mut reader = reader();
    reader.used_header_anchors.insert(0);
    let record = reader.record(&data, &mut 0, 0).unwrap();
    assert!(decode(&mut reader, record, 1).is_err());
}

#[test]
fn every_truncated_shape_and_unowned_tail_rejects() {
    let data = fixture(1, 0);
    for end in 0..data.len() {
        let mut reader = reader();
        assert!(reader.record(&data[..end], &mut 0, 0).is_err());
    }
    let mut data = fixture(1, 0);
    data.push(0);
    let length = (data.len() - 8) as u32;
    data[4..8].copy_from_slice(&length.to_le_bytes());
    let mut reader = reader();
    let record = reader.record(&data, &mut 0, 0).unwrap();
    assert!(decode(&mut reader, record, 1).is_err());
}

#[test]
fn officeart_layer_must_agree_with_word_anchor_layer() {
    for source_behind in [false, true] {
        for anchor_behind in [false, true] {
            let data = fixture_layer(1, 0, Some(source_behind));
            let mut reader = reader();
            reader.header_anchors[0].flags = 0x8060 | if anchor_behind { 0x4000 } else { 0 };
            let record = reader.record(&data, &mut 0, 0).unwrap();
            let result = decode(&mut reader, record, 1);
            assert_eq!(result.is_ok(), source_behind == anchor_behind);
            if let Ok(image) = result {
                assert_eq!(image.properties.behind_document, Some(source_behind));
                assert_eq!(image.behind_text, source_behind);
            }
        }
    }
}
