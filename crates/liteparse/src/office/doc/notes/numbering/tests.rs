use super::*;
fn fixture(group: &[u8]) -> (Vec<u8>, Vec<u8>, Story) {
    let word = (group.len() as u16)
        .to_le_bytes()
        .into_iter()
        .chain(group.iter().copied())
        .collect();
    let mut plc = vec![0u8; 20];
    plc[4..8].copy_from_slice(&3u32.to_le_bytes());
    let body = Story {
        kind: "body",
        start_cp: 0,
        utf16_units: 3,
        text: "x\u{2}\r".into(),
    };
    (word, plc, body)
}
#[test]
fn modern_defaults_and_final_revision_overrides() {
    let (word, plc, body) = fixture(&[]);
    let result = decode_sections(&word, &plc, &body).unwrap();
    assert_eq!(result.sections[0].footnotes.offset, 1);
    assert_eq!(result.sections[0].endnotes.format, 2);
    let group = [
        0x3f, 0x50, 6, 0, 0x39, 0x32, 1, 0x3f, 0x50, 9, 0, 0x3c, 0x30, 1,
    ];
    let (word, plc, body) = fixture(&group);
    let result = decode_sections(&word, &plc, &body).unwrap();
    assert_eq!(result.sections[0].footnotes.offset, 9);
    assert_eq!(result.sections[0].footnotes.restart, 1);
    assert!(result.sections[0].unresolved.is_empty());
    let (word, plc, body) = fixture(&[0x3f, 0x50, 6, 0, 0x39, 0x32, 1]);
    assert_eq!(
        decode_sections(&word, &plc, &body).unwrap().sections[0]
            .footnotes
            .offset,
        6
    );
}
#[test]
fn unknown_operators_remain_unresolved_and_ranges_are_checked() {
    let (word, mut plc, body) = fixture(&[0x7f, 0x30, 0]);
    assert_eq!(
        decode_sections(&word, &plc, &body).unwrap().sections[0].unresolved,
        ["unknown_section_operator"]
    );
    plc[4..8].copy_from_slice(&2u32.to_le_bytes());
    assert!(decode_sections(&word, &plc, &body).is_err());
    assert!(decode_section_properties(&[0x3f, 0x50, 0]).is_err());
    assert!(decode_section_properties(&[0x55, 0x08, 1]).is_err());
}

#[test]
fn legacy_dop_uses_full_width_formats_and_packed_start_restart() {
    let (word, plc, body) = fixture(&[0x3f, 0x50, 99, 0]);
    let mut sections = decode_sections(&word, &plc, &body).unwrap();
    let mut dop = vec![0u8; 500];
    dop[2..4].copy_from_slice(&((17u16 << 2) | 1).to_le_bytes());
    dop[52..54].copy_from_slice(&((33u16 << 2) | 2).to_le_bytes());
    dop[54..56].copy_from_slice(&u16::MAX.to_le_bytes());
    dop[492..494].copy_from_slice(&2u16.to_le_bytes());
    dop[494..496].copy_from_slice(&0u16.to_le_bytes());
    apply_legacy_numbering(&mut sections, &dop).unwrap();
    assert_eq!(sections.sections[0].footnotes.offset, 17);
    assert_eq!(sections.sections[0].footnotes.restart, 1);
    assert_eq!(sections.sections[0].footnotes.format, 2);
    assert_eq!(sections.sections[0].endnotes.offset, 33);
    assert_eq!(sections.sections[0].endnotes.restart, 2);
    assert_eq!(
        sections
            .legacy_numbering
            .unwrap()
            .raw_footnote_start_restart,
        69
    );
}
#[test]
fn legacy_dop_rejects_truncation_and_reserved_restart() {
    let (word, plc, body) = fixture(&[]);
    let mut sections = decode_sections(&word, &plc, &body).unwrap();
    assert!(apply_legacy_numbering(&mut sections, &[0; 499]).is_err());
    let mut dop = vec![0; 500];
    dop[52] = 3;
    assert!(apply_legacy_numbering(&mut sections, &dop).is_err());
}

#[test]
fn outline_definitions_retain_source_order_and_do_not_change_note_numbering() {
    let mut group = vec![0x3f, 0x50, 6, 0];
    group.extend([0x02, 0xd2, 212]);
    group.extend([0u8; 212]);
    group.extend([0x3f, 0x50, 9, 0]);
    group.extend([0x38, 0xd2, 36]);
    group.extend([0xffu8; 36]);
    let (word, plc, body) = fixture(&group);
    let result = decode_sections(&word, &plc, &body).unwrap();
    let section = &result.sections[0];
    assert!(section.unresolved.is_empty());
    assert_eq!(section.footnotes.offset, 9);
    assert_eq!(
        section
            .properties
            .iter()
            .map(|p| p.sprm)
            .collect::<Vec<_>>(),
        [0x503f, 0xd202, 0x503f, 0xd238]
    );
    assert_eq!(
        section.properties[1].operand,
        [&[212][..], &[0u8; 212]].concat()
    );
    assert_eq!(
        section.properties[3].operand,
        [&[36][..], &[0xffu8; 36]].concat()
    );
    let mut legacy = result;
    let mut dop = vec![0u8; 500];
    dop[2..4].copy_from_slice(&4u16.to_le_bytes());
    dop[52..54].copy_from_slice(&4u16.to_le_bytes());
    apply_legacy_numbering(&mut legacy, &dop).unwrap();
    assert_eq!(legacy.sections[0].footnotes.offset, 1);
    assert!(legacy.sections[0].unresolved.is_empty());
}

#[test]
fn unsupported_outline_grammars_and_unknown_operators_remain_explicit() {
    for (opcode, size) in [(0xd202u16, 211u8), (0xd202, 248), (0xd238, 35)] {
        let mut group = opcode.to_le_bytes().to_vec();
        group.push(size);
        group.resize(3 + usize::from(size), 0);
        let (word, plc, body) = fixture(&group);
        let result = decode_sections(&word, &plc, &body).unwrap();
        assert_eq!(
            result.sections[0].unresolved,
            ["invalid_outline_section_operand"]
        );
        group.pop();
        let (word, plc, body) = fixture(&group);
        assert!(decode_sections(&word, &plc, &body).is_err());
    }
    let mut group = vec![0x02, 0xd2, 212];
    group.extend([0u8; 212]);
    group.extend([0xff, 0xd1, 0]);
    let (word, plc, body) = fixture(&group);
    assert_eq!(
        decode_sections(&word, &plc, &body).unwrap().sections[0].unresolved,
        ["unknown_section_operator"]
    );
}

#[test]
fn ignored_section_extension_preserves_order_and_numbering() {
    let (word, plc, body) = fixture(&[0x3f, 0x50, 6, 0, 0xff, 0xd1, 2, 3, 0, 0x3f, 0x50, 9, 0]);
    let result = decode_sections(&word, &plc, &body).unwrap();
    let section = &result.sections[0];
    assert!(section.unresolved.is_empty());
    assert_eq!(section.footnotes.offset, 9);
    assert_eq!(
        section
            .properties
            .iter()
            .map(|p| p.sprm)
            .collect::<Vec<_>>(),
        [0x503f, 0xd1ff, 0x503f]
    );
    assert_eq!(section.properties[1].operand, [2, 3, 0]);
    let (word, plc, body) = fixture(&[0xff, 0xd1, 2, 3]);
    assert!(decode_sections(&word, &plc, &body).is_err());
    for group in [&[0xff, 0xd1, 1, 3][..], &[0xff, 0xd1, 2, 4, 0][..]] {
        let (word, plc, body) = fixture(group);
        assert_eq!(
            decode_sections(&word, &plc, &body).unwrap().sections[0].unresolved,
            ["unknown_section_operator"]
        );
    }
}
