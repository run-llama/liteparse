use super::*;
use crate::office::doc::tests::{put16, put32};

fn headers(simple: bool) -> Vec<u8> {
    let mut data = vec![0; 30];
    put16(&mut data, 0, 1);
    put32(&mut data, 2, 7);
    for i in 0..9 {
        put16(&mut data, 10 + i * 2, 0xfff);
    }
    data[28] = u8::from(simple);
    data
}
fn level(nfc: u8, text: &[u16]) -> Vec<u8> {
    let mut data = vec![0; 28];
    put32(&mut data, 0, 1);
    data[4] = nfc;
    if nfc == 0 {
        data[6] = 1;
    }
    data.extend((text.len() as u16).to_le_bytes());
    for u in text {
        data.extend(u.to_le_bytes());
    }
    data
}
fn overrides(flags: Option<u32>, format: &[u8]) -> Vec<u8> {
    let mut data = vec![0; 24];
    put32(&mut data, 0, 1);
    put32(&mut data, 4, 7);
    put32(&mut data, 20, u32::MAX);
    if let Some(flags) = flags {
        data[16] = 1;
        data.extend(5u32.to_le_bytes());
        data.extend(flags.to_le_bytes());
        data.extend(format);
    }
    data
}

#[test]
fn simple_decimal_and_bullet_levels_preserve_source_formatting() {
    let mut decimal = level(0, &[0, b'.' as u16]);
    decimal.extend([9, 8, 7]);
    let decoded = decode_lists(&headers(true), &decimal, &overrides(None, &[])).unwrap();
    assert_eq!(decoded.appended_level_bytes, 34);
    assert_eq!(decoded.lists[0].levels[0].number_text_utf16, [0, 46]);
    assert_eq!(decoded.overrides[0].list_index, 0);
    assert_eq!(decoded.overrides[0].first_paragraph_cp, None);
    let bullet = decode_lists(&headers(true), &level(0x17, &[0xf0b7]), &[]).unwrap();
    assert_eq!(bullet.lists[0].levels[0].number_text_utf16, [0xf0b7]);
    assert_eq!(bullet.status, "source_list_definitions_only");
}

#[test]
fn multilevel_offsets_and_property_groups_are_preserved() {
    let mut raw = Vec::new();
    for i in 0..9 {
        let mut v = level(0, &[i, 46]);
        v[24] = 3;
        v[25] = 3;
        v.splice(28..28, [6, 0x24, 1, 0x35, 0x08, 1]);
        raw.extend(v);
    }
    let decoded = decode_lists(&headers(false), &raw, &[]).unwrap();
    assert_eq!(decoded.lists[0].levels.len(), 9);
    assert_eq!(decoded.lists[0].levels[8].raw_papx, [6, 0x24, 1]);
    assert_eq!(decoded.lists[0].levels[8].raw_chpx, [0x35, 0x08, 1]);
    assert_eq!(decoded.appended_level_bytes, raw.len());
}

#[test]
fn start_and_format_overrides_have_distinct_source_semantics() {
    let base = level(0, &[0, 46]);
    let start = decode_lists(&headers(true), &base, &overrides(Some(0x10), &[])).unwrap();
    assert_eq!(start.overrides[0].levels[0].start_override, Some(5));
    let format = decode_lists(
        &headers(true),
        &base,
        &overrides(Some(0x30), &level(0x17, &[0x2022])),
    )
    .unwrap();
    let change = &format.overrides[0].levels[0];
    assert_eq!(change.start_override, None);
    assert_eq!(change.raw_start_at, 5);
    assert_eq!(change.format_override.as_ref().unwrap().number_format, 0x17);
}

#[test]
fn truncated_levels_and_overrides_are_errors() {
    let base = level(0, &[0, 46]);
    for end in 0..base.len() {
        assert!(decode_lists(&headers(true), &base[..end], &[]).is_err());
    }
    let raw = overrides(Some(0x30), &base);
    for end in 1..raw.len() {
        assert!(decode_lists(&headers(true), &base, &raw[..end]).is_err());
    }
    let mut trailing = overrides(None, &[]);
    trailing.push(0);
    assert!(decode_lists(&headers(true), &base, &trailing).is_err());
}

#[test]
fn placeholder_and_level_constraints_reject_invalid_metadata() {
    // Word opens a bullet level with empty text and draws no bullet.
    assert!(decode_lists(&headers(true), &level(0x17, &[]), &[]).is_ok());
    assert!(decode_lists(&headers(true), &level(0x17, &[0x2022, 0x2022]), &[]).is_err());
    for mode in 0..7 {
        let mut base = level(0, &[0, 46]);
        match mode {
            0 => base[6] = 3,
            1 => base[7] = 1,
            2 => base[30] = 1,
            3 => base[4] = 0x17,
            4 => base[4] = 8,
            5 => base[15] = 3,
            _ => {
                base[5] = 8;
                base[26] = 1;
            }
        }
        assert!(decode_lists(&headers(true), &base, &[]).is_err());
    }
    let mut bad = level(0x17, &[0xd800]);
    assert!(decode_lists(&headers(true), &bad, &[]).is_err());
    bad[4] = 0xff;
    assert!(decode_lists(&headers(true), &bad, &[]).is_err());
}

#[test]
fn overrides_validate_references_and_levels() {
    let base = level(0, &[0, 46]);
    for mode in 0..2 {
        let mut raw = overrides(Some(0x10), &[]);
        match mode {
            0 => put32(&mut raw, 4, 8),
            _ => put32(&mut raw, 24, u32::MAX),
        }
        assert!(decode_lists(&headers(true), &base, &raw).is_err());
    }
    let mut field = overrides(Some(0x10), &[]);
    field[17] = 0xfe;
    assert!(decode_lists(&headers(true), &base, &field).is_ok());
    let mut duplicate = overrides(Some(0x10), &[]);
    duplicate[16] = 2;
    duplicate.extend(5u32.to_le_bytes());
    duplicate.extend(0x10u32.to_le_bytes());
    assert!(decode_lists(&headers(true), &base, &duplicate).is_err());
}

#[test]
fn empty_tables_and_resource_limits_are_explicit() {
    let empty = decode_lists(&[], &[], &[]).unwrap();
    assert!(empty.lists.is_empty() && empty.overrides.is_empty());
    assert!(decode_lists(&[], &[], &u32::MAX.to_le_bytes()).is_err());
    assert!(decode_lists(&vec![0; MAX_BYTES + 1], &[], &[]).is_err());
    let mut budget = Budget {
        levels: 0,
        bytes: MAX_BYTES,
    };
    assert!(decode_level(&level(0, &[0, 46]), &mut 0, 0, &mut budget).is_err());
    let mut budget = Budget {
        levels: 1,
        bytes: 1,
    };
    assert!(decode_level(&level(0, &[0, 46]), &mut 0, 0, &mut budget).is_err());
}

#[test]
fn an_override_of_a_level_a_simple_list_lacks_is_skipped() {
    let base = level(0, &[0, 46]);
    // Level 1 with a format override: its LVL bytes are read, then the level is dropped.
    let raw = overrides(Some(0x21), &level(0, &[1, 46]));
    let decoded = decode_lists(&headers(true), &base, &raw).unwrap();
    assert!(decoded.overrides[0].levels.is_empty());
}
