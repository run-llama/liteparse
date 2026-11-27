use super::*;

fn operand(was_numbered: bool, template: &[u16]) -> Vec<u8> {
    let mut data = vec![0; 129];
    data[0] = 128;
    data[1] = u8::from(was_numbered);
    data[65..67].copy_from_slice(&(template.len() as u16).to_le_bytes());
    for (index, unit) in template.iter().enumerate() {
        data[67 + index * 2..69 + index * 2].copy_from_slice(&unit.to_le_bytes());
    }
    data
}

#[test]
fn decodes_historical_fields_without_selecting_current_numbering() {
    let mut data = operand(true, &[40, 1, 41]);
    data[2] = 0xfe;
    data[3..5].copy_from_slice(&17u16.to_le_bytes());
    data[5..9].copy_from_slice(&0x12345678u32.to_le_bytes());
    data[9] = 2;
    data[18] = 4;
    data[29..33].copy_from_slice(&27u32.to_le_bytes());
    data[27] = 0xaa;
    data[128] = 0xbb;
    let old = decode_numrm_operand(&data).unwrap();
    assert!(old.was_numbered);
    assert_eq!(old.author_index, 17);
    assert_eq!(old.timestamp_raw, 0x12345678);
    assert_eq!(old.placeholder_offsets[0], 2);
    assert_eq!(old.number_formats[0], 4);
    assert_eq!(old.number_values[0], 27);
    assert_eq!(old.template_utf16, [40, 1, 41]);
    assert_eq!(old.raw_numrm, data[1..]);
    let evidence = NumberingRevisionEvidence::accepted_current(123, Some(old), false).unwrap();
    assert_eq!(evidence.paragraph_mark_cp, 123);
    assert_eq!(evidence.numbering_view, "accepted_current_numbering");
    assert_eq!(evidence.historical_view, "unresolved");
}

#[test]
fn bounds_operand_and_template_before_access_or_allocation() {
    let base = operand(true, &[0, 46]);
    for length in 0..129 {
        assert!(decode_numrm_operand(&base[..length]).is_err());
    }
    let mut extra = base.clone();
    extra.push(0);
    assert!(decode_numrm_operand(&extra).is_err());
    for (index, value) in [(0, 127), (1, 2), (65, 32), (66, 1), (9, 3)] {
        let mut bad = base.clone();
        bad[index] = value;
        assert!(decode_numrm_operand(&bad).is_err());
    }
    let mut maximum = operand(true, &[0xd800; 31]);
    maximum[17] = 31;
    let old = decode_numrm_operand(&maximum).unwrap();
    assert_eq!(old.template_utf16.len(), 31);
    assert_eq!(old.template_utf16[30], 0xd800);
}

#[test]
fn unnumbered_history_preserves_residue_and_does_not_imply_insertion() {
    for template in [&[][..], &[0, 46][..]] {
        let old = decode_numrm_operand(&operand(false, template)).unwrap();
        assert!(!old.was_numbered);
        assert_eq!(old.template_utf16, template);
        validate_flags(Some(&old), false).unwrap();
        validate_flags(Some(&old), true).unwrap();
    }
    let mut bad = operand(false, &[]);
    bad[9] = 1;
    assert!(decode_numrm_operand(&bad).is_err());
}

#[test]
fn validates_final_revision_flags_without_using_them_as_list_membership() {
    let old = decode_numrm_operand(&operand(true, &[0, 46])).unwrap();
    assert!(validate_flags(Some(&old), true).is_err());
    validate_flags(Some(&old), false).unwrap();
    validate_flags(None, true).unwrap();
    assert!(!decode_numbering_inserted(&[0]).unwrap());
    assert!(decode_numbering_inserted(&[1]).unwrap());
    for invalid in [&[][..], &[2][..], &[0xff][..], &[1, 0][..]] {
        assert!(decode_numbering_inserted(invalid).is_err());
    }
}
