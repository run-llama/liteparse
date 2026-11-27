use super::*;
fn record(data: &[u8], count: u16) -> Record<'_> {
    Record {
        version: 3,
        kind: 0xf00b,
        instance: count,
        data,
        payload_offset: 108,
    }
}
fn property(key: u16, value: u32) -> Vec<u8> {
    let mut bytes = key.to_le_bytes().to_vec();
    bytes.extend(value.to_le_bytes());
    bytes
}
#[test]
fn inherited_insets_are_retained_beside_unapplied_paint_defaults() {
    let mut bytes = property(0x81, 72000);
    bytes.extend(property(0x181, 0xff));
    let proof = decode(&record(&bytes, 2)).unwrap().unwrap();
    assert_eq!(proof.applied_text_properties, vec![(0x81, 72000)]);
    assert_eq!(proof.source_payload, bytes);
    assert_eq!(proof.table_range, [100, 120]);
}
#[test]
fn malformed_complex_extent_and_duplicate_defaults_are_rejected() {
    let mut bytes = property(0xc186, 4);
    bytes.extend([0, 0]);
    assert!(decode(&record(&bytes, 1)).is_err());
    let mut bytes = property(0x81, 1);
    bytes.extend(property(0x81, 2));
    assert!(decode(&record(&bytes, 2)).is_err());
}
#[test]
fn zero_image_defaults_do_not_mark_a_scene_partial() {
    for flag in [0x4000, 0xc000] {
        let mut bytes = property(flag | 0x186, 0);
        bytes.extend(property(flag | 0x1c5, 0));
        assert!(decode(&record(&bytes, 2)).unwrap().is_none());
    }
}
