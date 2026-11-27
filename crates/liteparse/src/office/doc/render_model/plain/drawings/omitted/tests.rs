use super::*;

fn record(kind: u16, version: u16, instance: u16, payload: &[u8]) -> Vec<u8> {
    let mut bytes = ((instance << 4) | version).to_le_bytes().to_vec();
    bytes.extend(kind.to_le_bytes());
    bytes.extend((payload.len() as u32).to_le_bytes());
    bytes.extend(payload);
    bytes
}

fn reader() -> Reader {
    Reader {
        records: Vec::new(),
        shape_ids: Default::default(),
        backgrounds: 0,
        anchors: vec![textbox::RawAnchor {
            shape_id: 3,
            cp: 27,
            rectangle: [-18, 65, 9567, 4540],
            flags: 16500,
        }],
        used_anchors: Default::default(),
        textboxes: Vec::new(),
        images: Vec::new(),
        lines: Vec::new(),
        rectangles: Vec::new(),
        header_anchors: Vec::new(),
        used_header_anchors: Default::default(),
        store_count: 0,
        omitted_shapes: Vec::new(),
        group_approximations: Vec::new(),
        document_defaults: Vec::new(),
        default_text_properties: Default::default(),
    }
}

fn payload(property: u16) -> Vec<u8> {
    let mut p = record(0xf00a, 2, 1, &[3, 0, 0, 0, 0, 10, 0, 0]);
    let mut property_bytes = property.to_le_bytes().to_vec();
    property_bytes.extend(0u32.to_le_bytes());
    p.extend(record(0xf00b, 3, 1, &property_bytes));
    p.extend(record(0xf010, 0, 0, &[0; 4]));
    p.extend(record(0xf011, 0, 0, &[0; 4]));
    p
}

fn accept(reader: &mut Reader, payload: &[u8], story: u8) -> Result<()> {
    rectangle(
        reader,
        Record {
            version: 15,
            instance: 0,
            kind: 0xf004,
            data: payload,
            payload_offset: 108,
        },
        story,
    )
}

#[test]
fn decoration_retains_exact_anchor_and_rejects_reuse_or_wrong_story() {
    let p = payload(0x1cb);
    let mut r = reader();
    accept(&mut r, &p, 0).unwrap();
    let shape = &r.omitted_shapes[0];
    assert_eq!(shape.group_anchor_cp, 27);
    assert_eq!(shape.source_rectangle_twips, Some([-18, 65, 9567, 4540]));
    assert_eq!(shape.source_record_payload, p);
    assert_eq!(shape.table_range, [100, 108 + p.len()]);
    assert!(accept(&mut r, &p, 0).is_err());
    assert!(accept(&mut reader(), &p, 1).is_err());
    let mut wrong = reader();
    wrong.anchors[0].shape_id = 4;
    assert!(accept(&mut wrong, &p, 0).is_err());
}

#[test]
fn text_identity_and_trailing_client_records_cannot_be_omitted() {
    assert!(accept(&mut reader(), &payload(0x80), 0).is_err());
    let mut p = payload(0x1cb);
    p.extend(record(0xf00d, 0, 0, &[1, 0, 0, 0]));
    assert!(accept(&mut reader(), &p, 0).is_err());
    let valid = payload(0x1cb);
    for end in 0..valid.len() {
        assert!(accept(&mut reader(), &valid[..end], 0).is_err());
    }
}

#[test]
fn omitted_rectangle_rejects_invalid_line_operands_before_accepting_ownership() {
    for (property, maximum) in [(0x1cb, 0x0132_f540u32), (0x1ce, 10), (0x1d7, 2)] {
        let mut raw = payload(property);
        raw[26..30].copy_from_slice(&maximum.to_le_bytes());
        accept(&mut reader(), &raw, 0).unwrap();
        raw[26..30].copy_from_slice(&(maximum + 1).to_le_bytes());
        let mut owner = reader();
        assert!(accept(&mut owner, &raw, 0).is_err());
        assert!(owner.omitted_shapes.is_empty());
        assert!(owner.used_anchors.is_empty());
    }
}
