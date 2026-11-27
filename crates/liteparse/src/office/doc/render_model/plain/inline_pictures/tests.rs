use super::*;
#[test]
fn picf_scaling_retains_fractional_dimensions_and_rejects_out_of_range_sizes() {
    let mut h = vec![0; 68];
    h[4..6].copy_from_slice(&68u16.to_le_bytes());
    h[28..30].copy_from_slice(&2265i16.to_le_bytes());
    h[30..32].copy_from_slice(&2025i16.to_le_bytes());
    h[32..34].copy_from_slice(&509u16.to_le_bytes());
    h[34..36].copy_from_slice(&511u16.to_le_bytes());
    assert_eq!(dimensions(&h).unwrap(), [57.64425, 51.73875]);
    h[32..34].copy_from_slice(&0u16.to_le_bytes());
    assert!(dimensions(&h).is_err());
    h[32..34].copy_from_slice(&u16::MAX.to_le_bytes());
    assert!(dimensions(&h).is_err());
}
#[test]
fn picf_border_does_not_silently_disappear() {
    let mut h = vec![0; 68];
    h[4..6].copy_from_slice(&68u16.to_le_bytes());
    for o in [28, 30, 32, 34] {
        h[o..o + 2].copy_from_slice(&1000u16.to_le_bytes());
    }
    assert!(dimensions(&h).is_ok());
    h[46] = 1;
    assert!(dimensions(&h).is_err());
}

#[test]
fn nested_record_cannot_escape_its_parent_or_truncate_header() {
    let mut data = vec![0u8; 24];
    data[4..8].copy_from_slice(&16u32.to_le_bytes());
    assert!(record(&data, 0, 24).is_ok());
    assert!(record(&data, 0, 23).is_err());
    assert!(record(&data[..7], 0, 24).is_err());
    data[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(record(&data, 0, 24).is_err());
}

#[test]
fn invalid_shape_identity_is_rejected_before_decoding_properties() {
    let mut data = vec![0u8; 24];
    data[..2].copy_from_slice(&15u16.to_le_bytes());
    data[2..4].copy_from_slice(&0xf004u16.to_le_bytes());
    data[4..8].copy_from_slice(&16u32.to_le_bytes());
    data[8..10].copy_from_slice(&((75u16 << 4) | 2).to_le_bytes());
    data[10..12].copy_from_slice(&0xf00au16.to_le_bytes());
    data[12..16].copy_from_slice(&8u32.to_le_bytes());
    data[20..24].copy_from_slice(&0xa01u32.to_le_bytes());
    assert!(appearance(&data, 0, 24).is_err());
}

#[test]
fn raster_payload_metadata_cannot_change_length_or_claim_compression() {
    let mut source = crate::office::doc::pictures::PayloadSource {
        stream: "Data",
        record_range: [80, 120],
        payload_range: [100, 120],
        compression: "none",
    };
    assert!(validate_payload(&source, 20, 20).is_ok());
    source.payload_range[1] = 119;
    assert!(validate_payload(&source, 20, 20).is_err());
    source.payload_range[1] = 120;
    source.compression = "deflate";
    assert!(validate_payload(&source, 20, 20).is_err());
    source.compression = "none";
    assert!(validate_payload(&source, 20, 19).is_err());
    source.payload_range = [120, 100];
    assert!(validate_payload(&source, 20, 20).is_err());
}

#[test]
fn inline_shape_ignores_reserved_flags_but_checks_anchor_presence() {
    fn rec(info: u16, kind: u16, payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend(info.to_le_bytes());
        out.extend(kind.to_le_bytes());
        out.extend((payload.len() as u32).to_le_bytes());
        out.extend(payload);
        out
    }
    let make = |flags: u32, anchor: bool, behind: Option<bool>| {
        let mut identity = 1026u32.to_le_bytes().to_vec();
        identity.extend(flags.to_le_bytes());
        let mut records = rec((75 << 4) | 2, 0xf00a, &identity);
        let mut props = 0x4104u16.to_le_bytes().to_vec();
        props.extend(1u32.to_le_bytes());
        props.extend(0x1ffu16.to_le_bytes());
        props.extend(0x80000u32.to_le_bytes());
        if let Some(value) = behind {
            props.extend(0x3bfu16.to_le_bytes());
            props.extend((0x200000u32 | if value { 0x20 } else { 0 }).to_le_bytes());
        }
        let count = if behind.is_some() { 3 } else { 2 };
        records.extend(rec((count << 4) | 3, 0xf00b, &props));
        if anchor {
            records.extend(rec(0, 0xf010, &0x80000001u32.to_le_bytes()));
        }
        rec(15, 0xf004, &records)
    };
    for (flags, anchor) in [(0x800, false), (0xa00, true), (0xfffffa00, true)] {
        let data = make(flags, anchor, None);
        assert!(appearance(&data, 0, data.len()).is_ok());
    }
    for behind in [false, true] {
        let data = make(0xa00, true, Some(behind));
        assert_eq!(appearance(&data, 0, data.len()).is_ok(), !behind);
    }
    for (flags, anchor) in [(0xa00, false), (0x800, true), (0xa40, true)] {
        let data = make(flags, anchor, None);
        assert!(appearance(&data, 0, data.len()).is_err());
    }
}

#[test]
fn inline_blip_index_and_complex_flags_do_not_select_or_consume_asset_bytes() {
    for id in [0x0104u16, 0x4104, 0x8104, 0xc104] {
        for value in [0u32, 37, u32::MAX] {
            let mut raw = id.to_le_bytes().to_vec();
            raw.extend(value.to_le_bytes());
            raw.extend(0x1ffu16.to_le_bytes());
            raw.extend(0x80000u32.to_le_bytes());
            let normalized = inline_options(&raw, 2).unwrap();
            let appearance = image_properties::decode((&normalized, 2), None).unwrap();
            assert_eq!(appearance.blip_index, 1);
            assert_eq!(&raw[..2], &id.to_le_bytes());
            assert_eq!(&raw[2..6], &value.to_le_bytes());
        }
    }
    assert!(inline_options(&[0; 11], 2).is_err());
}

#[test]
fn mixed_floating_and_inline_source_controls_select_only_picf_owners() {
    let text = "\u{8}😀\u{1}\u{8}\u{1}";
    assert_eq!(
        source_controls("body", 10, text, &Default::default()),
        vec![13, 15]
    );
    let selected = [10, 13, 14].into_iter().collect();
    assert_eq!(source_controls("headers", 10, text, &selected), vec![13]);
    assert!(source_controls("footnotes", 10, text, &selected).is_empty());
}
