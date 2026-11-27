use super::*;
use crate::office::doc::tests::{fixture, put16, put32};

fn style(base_size: usize, sti: u16, parent: u16) -> Vec<u8> {
    let mut value = vec![0; base_size];
    put16(&mut value, 0, sti);
    put16(&mut value, 2, (parent << 4) | 1);
    put16(&mut value, 4, 2);
    value.extend([1, 0, b'X', 0, 0, 0]);
    value.extend([2, 0, 0, 0]);
    value.extend([3, 0, 0x55, 0x08, 1, 0]);
    let len = value.len() as u16;
    put16(&mut value, 6, len);
    value
}
fn sheet(base_size: usize, definitions: &[Option<Vec<u8>>]) -> Vec<u8> {
    let count = definitions.len().max(15);
    let mut data = vec![0; 20];
    put16(&mut data, 0, 18);
    put16(&mut data, 2, count as u16);
    put16(&mut data, 4, base_size as u16);
    put16(&mut data, 10, 15);
    for index in 0..count {
        if let Some(Some(value)) = definitions.get(index) {
            data.extend((value.len() as u16).to_le_bytes());
            data.extend(value);
            if !data.len().is_multiple_of(2) {
                data.push(0);
            }
        } else {
            data.extend([0, 0]);
        }
    }
    data
}

#[test]
fn old_and_new_style_layouts_preserve_identity_and_raw_groups() {
    for size in [10, 18] {
        let data = sheet(
            size,
            &[Some(style(size, 0, 0xfff)), Some(style(size, 1, 0))],
        );
        let value = decode(&data).unwrap();
        let heading = value.styles[1].as_ref().unwrap();
        assert_eq!(value.std_base_bytes, size);
        assert_eq!(heading.builtin_id, 1);
        assert_eq!(heading.base_style, Some(0));
        assert_eq!(heading.properties[0].kind, "paragraph");
        assert_eq!(heading.properties[1].bytes, [0x55, 0x08, 1]);
        assert!(value.styles[13].is_none());
    }
}

#[test]
fn rejects_self_cycles_longer_cycles_and_missing_parent() {
    for defs in [
        vec![Some(style(10, 0, 0))],
        vec![Some(style(10, 0, 1)), Some(style(10, 1, 0))],
        vec![Some(style(10, 0, 12))],
    ] {
        assert!(decode(&sheet(10, &defs)).is_err());
    }
}

#[test]
fn custom_index_does_not_imply_builtin_identity() {
    let mut defs = vec![Some(style(10, 0, 0xfff))];
    defs.resize_with(15, || None);
    defs.push(Some(style(10, 0xffe, 0)));
    let value = decode(&sheet(10, &defs)).unwrap();
    let custom = value.styles[15].as_ref().unwrap();
    assert_eq!(custom.index, 15);
    assert_eq!(custom.builtin_id, 0xffe);
    assert_eq!(custom.base_style, Some(0));
}

#[test]
fn preserves_revision_payload_without_resolving_visibility() {
    let mut revision = style(18, 0, 0xfff);
    put16(&mut revision, 10, 0x1000);
    put16(&mut revision, 4, 3);
    revision.extend([0, 0]);
    let len = revision.len() as u16;
    put16(&mut revision, 6, len);
    let value = decode(&sheet(18, &[Some(revision)])).unwrap();
    let style = value.styles[0].as_ref().unwrap();
    assert!(style.revision_marked);
    assert_eq!(style.raw_revision, [0, 0]);
}

#[test]
fn rejects_bad_lengths_layouts_names_fixed_identity_and_links() {
    let valid = sheet(10, &[Some(style(10, 0, 0xfff))]);
    for end in [0, 1, 10, 19, 20, 21, valid.len() - 1] {
        assert!(decode(&valid[..end]).is_err());
    }
    for (offset, value) in [
        (2, 14),
        (4, 11),
        (20, 0xffff),
        (22, 1),
        (22 + 6, 1),
        (22 + 10, 0xffff),
        (22 + 14, 1),
        (22 + 4, 0xfff2),
    ] {
        let mut data = valid.clone();
        put16(&mut data, offset, value);
        assert!(decode(&data).is_err(), "offset {offset}");
    }
}

#[test]
fn stylesheet_input_bound_precedes_allocations() {
    assert!(matches!(
        decode(&vec![0; MAX_STYLESHEET_BYTES + 1]),
        Err(Error::Limit("stylesheet bytes"))
    ));
}

#[test]
fn reads_stylesheet_from_selected_fib_table_stream() {
    let mut data = fixture();
    let stsh = sheet(10, &[Some(style(10, 0, 0xfff))]);
    put32(&mut data, 1536 + 154 + 8, 128);
    put32(&mut data, 1536 + 158 + 8, stsh.len() as u32);
    data[5632 + 128..5632 + 128 + stsh.len()].copy_from_slice(&stsh);
    let result = inspect_styles(&data).unwrap();
    assert_eq!(result.styles[0].as_ref().unwrap().builtin_id, 0);
    assert!(crate::office::doc::inspect(&data).is_ok());
}

#[test]
fn fixed_style_slots_validate_builtin_identity_and_reserved_emptiness() {
    let mut definitions = vec![Some(style(10, 0, 0xfff))];
    definitions.resize_with(15, || None);
    for (index, identity) in [(10, 65), (11, 105), (12, 107)] {
        definitions[index] = Some(style(10, identity, 0));
    }
    assert!(decode(&sheet(10, &definitions)).is_ok());
    definitions[10] = Some(style(10, 10, 0));
    assert!(matches!(
        decode(&sheet(10, &definitions)),
        Err(Error::Invalid("fixed stylesheet identity"))
    ));
    definitions[10] = None;
    definitions[13] = Some(style(10, 0xffe, 0));
    assert!(matches!(
        decode(&sheet(10, &definitions)),
        Err(Error::Invalid("fixed stylesheet identity"))
    ));
}
