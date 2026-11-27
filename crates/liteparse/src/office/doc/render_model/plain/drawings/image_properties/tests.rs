use super::*;

fn group(items: &[(u16, u32, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    for (id, value, _) in items {
        out.extend(id.to_le_bytes());
        out.extend(value.to_le_bytes());
    }
    for (_, _, tail) in items {
        out.extend(*tail);
    }
    out
}

#[test]
fn embedded_picture_name_is_not_a_link_and_zero_origin_stays_raw() {
    let name = [b'A', 0, 0, 0];
    let data = group(&[
        (0x4104, 2, &[]),
        (0xc105, 4, &name),
        (0x106, 2, &[]),
        (0x1ff, 0x80000, &[]),
    ]);
    let extra = group(&[(0x38f, 2, &[]), (0x390, 0, &[])]);
    let result = decode((&data, 4), Some((&extra, 2))).unwrap();
    assert_eq!(result.blip_index, 2);
    assert_eq!(result.source_name.as_deref(), Some("A"));
    assert_eq!(result.source_name_kind, 2);
    assert_eq!(result.positioning, [Some(2), Some(0), None, None]);
    assert_eq!(result.crop_fixed, [0; 4]);
}

#[test]
fn record_partition_and_link_crop_effect_guards_remain_explicit() {
    let valid = group(&[(0x4104, 1, &[]), (0x1ff, 0x80000, &[])]);
    decode((&valid, 2), None).unwrap();
    let mut tail = valid.clone();
    tail.push(0);
    assert!(decode((&tail, 2), None).is_err());
    assert!(decode((&valid[..11], 2), None).is_err());
    for extra in [
        group(&[(0x106, 10, &[])]),
        group(&[(0x100, 65536, &[])]),
        group(&[(0x106, 3, &[])]),
        group(&[(0x4104, 0, &[])]),
        group(&[(0x104, 1, &[])]),
        group(&[(0x1ff, 0x80008, &[])]),
        group(&[(0x390, 5, &[])]),
    ] {
        assert!(decode((&valid, 2), Some((&extra, 1))).is_err());
    }
}

#[test]
fn complex_names_and_polygons_require_exact_owned_framing() {
    let valid = group(&[(0x4104, 1, &[]), (0x1ff, 0x80000, &[])]);
    for malformed in [
        &[b'A', 0][..],
        &[0, 0, b'A', 0, 0, 0],
        &[0, 0xd8, 0, 0],
        &[0],
    ] {
        let extra = group(&[(0xc105, malformed.len() as u32, malformed)]);
        assert!(decode((&valid, 2), Some((&extra, 1))).is_err());
    }
    let mut poly = vec![2, 0, 2, 0, 8, 0];
    poly.extend([0; 16]);
    let extra = group(&[(0xc383, poly.len() as u32, &poly)]);
    assert_eq!(
        decode((&valid, 2), Some((&extra, 1)))
            .unwrap()
            .wrap_polygon_points,
        Some(2)
    );
    for offset in [0, 4] {
        let mut bad = poly.clone();
        bad[offset] = 3;
        let extra = group(&[(0xc383, bad.len() as u32, &bad)]);
        assert!(decode((&valid, 2), Some((&extra, 1))).is_err());
    }
}

#[test]
fn later_property_group_replaces_source_asset_identity() {
    let first = group(&[(0x4104, 1, &[]), (0x1ff, 0x80000, &[])]);
    let later = group(&[(0x4104, 3, &[])]);
    assert_eq!(
        decode((&first, 2), Some((&later, 1))).unwrap().blip_index,
        3
    );
    let duplicate = group(&[(0x4104, 1, &[]), (0x4104, 2, &[])]);
    assert!(decode((&duplicate, 2), None).is_err());
}

#[test]
fn inactive_blip_effect_flags_do_not_reject_unchanged_raster_appearance() {
    let primary = group(&[(0x4104, 1, &[]), (0x1ff, 0x80000, &[])]);
    let expected = decode((&primary, 2), None).unwrap();
    for value in [0, 0x60000, 0x70000, 0x10000, 0xff80ff80] {
        let extra = group(&[(0x13f, value, &[])]);
        assert_eq!(decode((&primary, 2), Some((&extra, 1))).unwrap(), expected);
    }
    for bit in 0..7 {
        for value in [1 << bit, 1 << (bit + 16)] {
            let extra = group(&[(0x13f, value, &[])]);
            assert_eq!(decode((&primary, 2), Some((&extra, 1))).unwrap(), expected);
        }
    }
}

#[test]
fn active_blip_effect_flags_remain_explicitly_unsupported() {
    let primary = group(&[(0x4104, 1, &[]), (0x1ff, 0x80000, &[])]);
    for bit in 0..7 {
        let extra = group(&[(0x13f, (1 << bit) | (1 << (bit + 16)), &[])]);
        assert!(decode((&primary, 2), Some((&extra, 1))).is_err());
    }
}

#[test]
fn editing_locks_preserve_saved_raster_but_require_simple_property_framing() {
    let primary = group(&[(0x4104, 1, &[]), (0x1ff, 0x80000, &[])]);
    let expected = decode((&primary, 2), None).unwrap();
    for value in [0, 0x800080, 0x3ff03ff, u32::MAX] {
        let extra = group(&[(0x7f, value, &[])]);
        assert_eq!(decode((&primary, 2), Some((&extra, 1))).unwrap(), expected);
    }
    for id in [0x407f, 0x807f, 0xc07f] {
        let extra = group(&[(id, 0, &[])]);
        assert!(decode((&primary, 2), Some((&extra, 1))).is_err());
    }
    let cropped = group(&[(0x7f, u32::MAX, &[]), (0x100, 65536, &[])]);
    assert!(decode((&primary, 2), Some((&cropped, 2))).is_err());
}

#[test]
fn shape_metadata_retains_unicode_without_replacing_image_filename() {
    let filename: Vec<_> = "source.png\0"
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    let title: Vec<_> = "Figure 🖼\0"
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    let description: Vec<_> = "図の説明\0"
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    for bid in [0, 0x4000] {
        let primary = group(&[
            (0x4104, 1, &[]),
            (0xc105, filename.len() as u32, &filename),
            (0x1ff, 0x80000, &[]),
            (0x8380 | bid, title.len() as u32, &title),
            (0x8381 | bid, description.len() as u32, &description),
        ]);
        let result = decode((&primary, 5), None).unwrap();
        assert_eq!(result.source_name.as_deref(), Some("source.png"));
        assert_eq!(result.shape_name.as_deref(), Some("Figure 🖼"));
        assert_eq!(result.shape_description.as_deref(), Some("図の説明"));
        let reset = group(&[(0x380 | bid, 0, &[]), (0x381 | bid, 0, &[])]);
        let cleared = decode((&primary, 5), Some((&reset, 2))).unwrap();
        assert!(cleared.shape_name.is_none());
        assert!(cleared.shape_description.is_none());
        assert_eq!(cleared.source_name, result.source_name);
    }
}

#[test]
fn shape_metadata_rejects_bad_utf16_lengths_simple_values_and_duplicate_keys() {
    let primary = group(&[(0x4104, 1, &[]), (0x1ff, 0x80000, &[])]);
    for key in [0x380, 0x381] {
        for bid in [0, 0x4000] {
            assert!(decode((&primary, 2), Some((&group(&[(key | bid, 2, &[])]), 1))).is_err());
            for bad in [
                &[][..],
                &[65, 0],
                &[0, 0xd8, 0, 0],
                &[0],
                &[65, 0, 0, 0, 66, 0, 0, 0],
            ] {
                let extra = group(&[(key | bid | 0x8000, bad.len() as u32, bad)]);
                assert!(decode((&primary, 2), Some((&extra, 1))).is_err());
            }
            let valid = [65, 0, 0, 0];
            for declared in [2, 6] {
                let extra = group(&[(key | bid | 0x8000, declared, &valid)]);
                assert!(decode((&primary, 2), Some((&extra, 1))).is_err());
            }
        }
        let duplicate = group(&[(key, 0, &[]), (key | 0x4000, 0, &[])]);
        assert!(decode((&primary, 2), Some((&duplicate, 2))).is_err());
    }
}

#[test]
fn fill_boolean_use_bits_preserve_raster_defaults_and_reject_visual_changes() {
    let primary = group(&[(0x4104, 1, &[]), (0x1ff, 0x80000, &[])]);
    let expected = decode((&primary, 2), None).unwrap();
    for value in [0, 0x600000, 0x100010, 0x140014, 0x90009, 0xff80ffff] {
        let extra = group(&[(0x1bf, value, &[])]);
        assert_eq!(decode((&primary, 2), Some((&extra, 1))).unwrap(), expected);
    }
    // Changing each visual bit must fail only when its corresponding use bit is set.
    for bit in [1, 2, 5, 6] {
        let changed = 0x14 ^ (1 << bit);
        let inactive = group(&[(0x1bf, changed, &[])]);
        assert_eq!(
            decode((&primary, 2), Some((&inactive, 1))).unwrap(),
            expected
        );
        let active = group(&[(0x1bf, changed | (1 << (bit + 16)), &[])]);
        assert!(decode((&primary, 2), Some((&active, 1))).is_err());
    }
    for id in [0x41bf, 0x81bf, 0xc1bf] {
        let invalid = group(&[(id, 0, &[])]);
        assert!(decode((&primary, 2), Some((&invalid, 1))).is_err());
    }
}

#[test]
fn raster_selection_survives_explicit_no_fill_and_inert_text_flags() {
    let primary = group(&[
        (0xbf, 0x40004, &[]),
        (0x4104, 1, &[]),
        (0x1bf, 0x100000, &[]),
        (0x1ff, 0x80000, &[]),
        (0x3bf, 0x20000, &[]),
    ]);
    let result = decode((&primary, 5), None).unwrap();
    assert_eq!(result.blip_index, 1);
    assert!(!result.fill_enabled);
    assert_eq!(result.behind_document, None);
    let reset = group(&[(0x1bf, 0, &[])]);
    assert!(
        decode((&primary, 5), Some((&reset, 1)))
            .unwrap()
            .fill_enabled
    );
    for (property, value) in [(0xbf, 0x20002), (0xbf, 0x80008), (0x3bf, 0x20002)] {
        let changed = group(&[(property, value, &[])]);
        assert!(decode((&primary, 5), Some((&changed, 1))).is_err());
    }
    for property in [0x40bf, 0x80bf, 0xc0bf, 0x43bf, 0x83bf, 0xc3bf] {
        let changed = group(&[(property, 0, &[])]);
        assert!(decode((&primary, 5), Some((&changed, 1))).is_err());
    }
}

#[test]
fn editor_resize_and_explicit_non_script_flags_preserve_saved_picture() {
    let valid = group(&[(0x4104, 1, &[]), (0x1ff, 0x80000, &[])]);
    let baseline = decode((&valid, 2), None).unwrap();
    let flags = group(&[(0x33f, 0x100010, &[]), (0x3bf, 0x1800000, &[])]);
    assert_eq!(decode((&valid, 2), Some((&flags, 2))).unwrap(), baseline);
    for (key, value) in [(0x33f, 0x800080), (0x3bf, 0x1800180), (0x3bf, 0x20002)] {
        let flags = group(&[(key, value, &[])]);
        assert!(decode((&valid, 2), Some((&flags, 1))).is_err());
    }
}

#[test]
fn imported_ole_metadata_and_disabled_fill_keep_preview_visible() {
    let data = group(&[
        (0x4104, 1, &[]),
        (0x181, 0x10000011, &[]),
        (0x1bf, 0x1f000c, &[]),
        (0x1ff, 0x80000, &[]),
        (0x43f, 0x40004, &[]),
    ]);
    assert!(!decode((&data, 5), None).unwrap().fill_enabled);
    let active_fill = group(&[(0x1bf, 0x100010, &[])]);
    assert_eq!(
        decode((&data, 5), Some((&active_fill, 1)))
            .unwrap()
            .unpainted_fill_color,
        Some(0x10000011)
    );
    let fake_master = group(&[(0x43f, 0x20002, &[])]);
    assert!(decode((&data, 5), Some((&fake_master, 1))).is_err());
}

#[test]
fn crop_edges_keep_signed_fractions_and_reject_empty_source_windows() {
    let data = group(&[
        (0x100, 8192, &[]),
        (0x101, 16384, &[]),
        (0x102, (-4096i32) as u32, &[]),
        (0x103, 32768, &[]),
        (0x4104, 1, &[]),
    ]);
    let result = decode((&data, 5), None).unwrap();
    assert_eq!(result.crop_fixed, [8192, 16384, -4096, 32768]);
    assert_eq!(result.crop().unwrap(), (-0.0625, 0.125, 0.5, 0.25));
    let override_group = group(&[(0x100, 0, &[])]);
    assert_eq!(
        decode((&data, 5), Some((&override_group, 1)))
            .unwrap()
            .crop()
            .unwrap()
            .1,
        0.0
    );
    for edges in [
        [0, 0, 32768, 32768],
        [40000, 30000, 0, 0],
        [65536, 0, 0, 0],
        [-65537, 0, 0, 0],
    ] {
        let mut changed = decode((&data, 5), None).unwrap();
        changed.crop_fixed = edges;
        assert!(changed.crop().is_err());
    }
    for id in [0x4100, 0x8100] {
        let malformed = group(&[(id, 0, &[])]);
        assert!(decode((&data, 5), Some((&malformed, 1))).is_err());
    }
}

#[test]
fn explicit_default_layout_overlap_flags_preserve_appearance_but_active_changes_reject() {
    let data = group(&[(0x4104, 1, &[]), (0x1ff, 0x80000, &[])]);
    let default = decode((&data, 2), None).unwrap();
    for value in [0x82008200, 0x80008000, 0x02000200, 0x00008200] {
        let extra = group(&[(0x3bf, value, &[])]);
        assert_eq!(decode((&data, 2), Some((&extra, 1))).unwrap(), default);
    }
    for value in [0x80000000, 0x02000000, 0x82000200, 0x00020002, 0x01800180] {
        let extra = group(&[(0x3bf, value, &[])]);
        assert!(decode((&data, 2), Some((&extra, 1))).is_err());
    }
    let behind = group(&[(0x3bf, 0x00200020, &[])]);
    assert_eq!(
        decode((&data, 2), Some((&behind, 1)))
            .unwrap()
            .behind_document,
        Some(true)
    );
}

#[test]
fn inline_layer_is_retained_in_order_but_floating_layer_stays_guarded() {
    let primary = group(&[(0x4104, 1, &[]), (0x3aa, 0x0f000000, &[])]);
    let later = group(&[(0x3aa, 7, &[])]);
    assert!(decode((&primary, 2), None).is_err());
    let inline = decode_inline((&primary, 2), Some((&later, 1))).unwrap();
    assert_eq!(inline.inline_relative_layer, Some(7));
    for key in [0x43aa, 0x83aa] {
        let bad = group(&[(0x4104, 1, &[]), (key, 0, &[])]);
        assert!(decode_inline((&bad, 2), None).is_err());
    }
}
