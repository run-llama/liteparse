use super::*;

#[test]
fn language_baseline_preserves_order_and_last_source_value() {
    let mut budget = 4096;
    let groups = [
        (11, &[0x5f, 0x48, 0x09, 0x04][..]),
        (12, &[0x5f, 0x48, 0, 4, 0x5f, 0x48, 0x0c, 4][..]),
    ];
    let result = decode(&groups, &mut budget).unwrap().unwrap();
    assert_eq!(result.complex_script_language, 0x040c);
    assert_eq!(result.ordered_properties.len(), 3);
    assert_eq!(result.ordered_properties[0].style_index, 11);
    assert_eq!(result.ordered_properties[1].style_index, 12);
    assert_eq!(result.ordered_properties[1].operand, [0, 4]);
    assert_eq!(
        result.scope,
        "table_style_baseline_before_paragraph_and_direct"
    );
}

#[test]
fn language_baseline_preserves_raw_lid_without_guessing_a_locale() {
    for lid in [0, 0x0400u16, 0x0401, 0xffff] {
        let mut bytes = vec![0x5f, 0x48];
        bytes.extend(lid.to_le_bytes());
        let result = decode(&[(11, &bytes)], &mut 4096).unwrap().unwrap();
        assert_eq!(result.complex_script_language, lid);
        assert_eq!(result.ordered_properties[0].operand, lid.to_le_bytes());
    }
    assert!(decode(&[(11, &[])], &mut 4096).unwrap().is_none());
}

#[test]
fn language_baseline_rejects_hidden_revision_reset_and_conditional_properties() {
    for bytes in [
        vec![0x3c, 0x08, 0],
        vec![0x04, 0x34, 1],
        vec![0x3b, 0x08, 1],
        vec![0x00, 0x08, 0],
        vec![0x83, 0x2a, 0],
        vec![0x33, 0x2a, 0],
        vec![0x85, 0xca, 5, 1, 0, 0x3c, 0x08, 1],
    ] {
        let mut group = vec![0x5f, 0x48, 0, 4];
        group.extend(bytes);
        assert!(decode(&[(11, &group)], &mut 4096).is_err());
    }
}

#[test]
fn language_baseline_is_bounded_and_rejects_truncated_properties() {
    for bytes in [&[0x5f][..], &[0x5f, 0x48], &[0x5f, 0x48, 0]] {
        assert!(decode(&[(11, bytes)], &mut 4096).is_err());
    }
    assert!(decode(&[(11, &[0x5f, 0x48, 0, 4])], &mut 0).is_err());
    assert!(decode(&vec![(11, &[][..]); 65], &mut 4096).is_err());
}
