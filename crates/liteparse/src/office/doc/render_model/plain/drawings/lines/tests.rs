use super::*;

#[test]
fn editing_arrow_flags_do_not_invent_arrowheads_or_accept_effects() {
    assert_eq!(style(&[]).unwrap(), (0, 9525));
    assert_eq!(
        style(&[[0x1ff, 0x100010], [0x1cb, 12700]]).unwrap(),
        (0, 12700)
    );
    for property in [
        [0x1d1, 1],
        [0x1ce, 2],
        [0x1ff, 0x400040],
        [0x1c0, 0x08000000],
        [0x1cb, 0],
    ] {
        assert!(style(&[property]).is_err(), "{property:?}");
    }
}

#[test]
fn source_flips_change_line_endpoints_without_reversing_frame_ownership() {
    let mut line = LineShape {
        shape_id: 1,
        anchor_story: "headers",
        anchor_cp: 3,
        rectangle_twips: [100, 200, 500, 800],
        horizontal_origin: anchor::HorizontalOrigin::Column,
        vertical_origin: anchor::VerticalOrigin::Paragraph,
        behind_text: false,
        flip_horizontal: false,
        flip_vertical: false,
        stroke_rgb: 0,
        stroke_width_emu: 9525,
        table_range: [0, 8],
        raw_properties: Vec::new(),
    };
    assert_eq!(line.endpoints_pt(), [[0.0, 0.0], [20.0, 30.0]]);
    line.flip_horizontal = true;
    assert_eq!(line.endpoints_pt(), [[20.0, 0.0], [0.0, 30.0]]);
    line.flip_vertical = true;
    assert_eq!(line.endpoints_pt(), [[20.0, 30.0], [0.0, 0.0]]);
    assert_eq!(line.rectangle_twips, [100, 200, 500, 800]);
}

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
            flags: 0x74,
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
    fsp.extend(0xa40u32.to_le_bytes());
    let mut raw = record(0xf00a, 2, 20, &fsp);
    raw.extend(record(0xf00b, 3, 1, &[0xff, 0x01, 0x10, 0, 0x10, 0]));
    raw.extend(record(0xf010, 0, 0, &0u32.to_le_bytes()));
    raw.extend(record(0xf011, 0, 0, &0u32.to_le_bytes()));
    raw
}

fn decode_raw(reader: &mut Reader, raw: &[u8]) -> Result<LineShape> {
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
fn raw_line_join_retains_control_owner_and_rejects_text_or_wrong_anchor() {
    let raw = raw_shape();
    for rect in [[0, 0, 100, 0], [0, 0, 0, 100], [0, 0, 100, 100]] {
        let mut r = reader(rect);
        let shape = decode_raw(&mut r, &raw).unwrap();
        assert_eq!((shape.shape_id, shape.anchor_cp), (42, 7));
        assert_eq!(shape.rectangle_twips, rect);
        assert!(shape.flip_horizontal);
        assert!(r.used_header_anchors.contains(&0));
        assert!(decode_raw(&mut r, &raw).is_err());
    }
    let mut wrong = reader([0, 0, 100, 0]);
    wrong.header_anchors[0].shape_id = 43;
    assert!(decode_raw(&mut wrong, &raw).is_err());
    let mut text = raw.clone();
    text.extend(record(0xf00d, 0, 0, &0x10000u32.to_le_bytes()));
    assert!(decode_raw(&mut reader([0, 0, 100, 0]), &text).is_err());
    assert!(decode_raw(&mut reader([0, 0, 0, 0]), &raw).is_err());
    assert!(decode_raw(&mut reader([0, 0, -1, 100]), &raw).is_err());
    assert!(decode_raw(&mut reader([0, 0, 100, 0]), &raw[..raw.len() - 1]).is_err());
}

#[test]
fn use_bits_bind_inset_and_opaque_back_line_without_treating_editability_as_paint() {
    assert!(style(&[[0x1ff, 0x200]]).is_ok());
    assert!(style(&[[0x1ff, 0x2000200]]).is_err());
    assert!(style(&[[0x1ff, 0x600040]]).is_ok());
    assert!(style(&[[0x1ff, 0x400040]]).is_err());
    assert!(style(&[[0x1ff, 0x80000]]).is_err());
    assert!(style(&[[0x1ff, 0x180018]]).is_ok());
    assert!(style(&[[0x1cd, 1]]).is_err());
}
