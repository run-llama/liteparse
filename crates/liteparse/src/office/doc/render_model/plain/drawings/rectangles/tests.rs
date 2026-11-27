use super::*;
fn record(kind: u16, version: u16, instance: u16, payload: &[u8]) -> Vec<u8> {
    let mut bytes = ((instance << 4) | version).to_le_bytes().to_vec();
    bytes.extend(kind.to_le_bytes());
    bytes.extend((payload.len() as u32).to_le_bytes());
    bytes.extend(payload);
    bytes
}

fn reader(rectangle: [i32; 4]) -> Reader {
    Reader {
        records: vec![],
        shape_ids: Default::default(),
        backgrounds: 0,
        header_anchors: vec![super::super::textbox::RawAnchor {
            shape_id: 42,
            cp: 7,
            rectangle,
            flags: 0x6b,
        }],
        used_anchors: Default::default(),
        textboxes: vec![],
        images: vec![],
        lines: vec![],
        rectangles: Vec::new(),
        anchors: vec![],
        used_header_anchors: Default::default(),
        store_count: 0,
        omitted_shapes: vec![],
        group_approximations: vec![],
        document_defaults: vec![],
        default_text_properties: Default::default(),
    }
}

fn raw_shape() -> Vec<u8> {
    let mut fsp = 42u32.to_le_bytes().to_vec();
    fsp.extend(0xa00u32.to_le_bytes());
    let mut raw = record(0xf00a, 2, 1, &fsp);
    let mut props = Vec::new();
    for (id, value) in [
        (0x181u16, 0u32),
        (0x1bf, 0x100010),
        (0x1cb, 6350),
        (0x23f, 0x20000),
    ] {
        props.extend(id.to_le_bytes());
        props.extend(value.to_le_bytes());
    }
    raw.extend(record(0xf00b, 3, 4, &props));
    raw.extend(record(0xf010, 0, 0, &0u32.to_le_bytes()));
    raw.extend(record(0xf011, 0, 0, &0u32.to_le_bytes()));
    raw
}

fn decode_raw(reader: &mut Reader, raw: &[u8]) -> Result<RectangleShape> {
    decode(
        reader,
        Record {
            version: 15,
            instance: 0,
            kind: 0xf004,
            data: raw,
            payload_offset: 108,
        },
        1,
    )
}

#[test]
fn solid_rectangle_joins_raw_header_frame_and_rejects_degenerate_or_text_shapes() {
    let raw = raw_shape();
    let parsed = decode_raw(&mut reader([720, 720, 4041, 816]), &raw).unwrap();
    assert_eq!(parsed.frame.rectangle_twips, [720, 720, 4041, 816]);
    assert_eq!(parsed.frame.stroke_width_emu, 6350);
    assert_eq!(parsed.fill_rgb, 0);
    for frame in [[0, 0, 100, 0], [0, 0, 0, 100], [100, 0, 0, 100]] {
        assert!(decode_raw(&mut reader(frame), &raw).is_err());
    }
    let mut text = raw;
    text.extend(record(0xf00d, 0, 0, &0x10000u32.to_le_bytes()));
    assert!(decode_raw(&mut reader([0, 0, 100, 100]), &text).is_err());
}

#[test]
fn rectangle_style_rejects_unpainted_fill_custom_geometry_and_active_effects() {
    assert_eq!(
        style(&[
            [0x181, 0],
            [0x1bf, 0x100010],
            [0x23f, 0x20000],
            [0x390, 1],
            [0x392, 1],
            [0x3bf, 0x80000000]
        ])
        .unwrap(),
        (0, 9525)
    );
    for property in [
        [0x1bf, 0x100000],
        [0x144, 4],
        [0x23f, 0x20002],
        [0x181, 0x80000000],
        [0x1ff, 0x80000],
        [0x390, 2],
    ] {
        assert!(style(&[property]).is_err(), "{property:?}");
    }
}
