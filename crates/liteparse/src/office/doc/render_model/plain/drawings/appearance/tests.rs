use super::*;
fn bytes(values: &[(u16, u32)]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|(key, value)| key.to_le_bytes().into_iter().chain(value.to_le_bytes()))
        .collect()
}
fn record(data: &[u8], kind: u16) -> Record<'_> {
    Record {
        version: 3,
        instance: (data.len() / 6) as u16,
        kind,
        data,
        payload_offset: 108,
    }
}
#[test]
fn unknown_effects_preserve_text_id_and_every_raw_property_byte() {
    let data = bytes(&[(0x80, 0x10000), (0x81, 72000), (0x8a, 8), (0x23f, 0x20002)]);
    let a = resolve(&[record(&data, 0xf00b)], &BTreeMap::new(), 8).unwrap();
    assert!(a.properties.contains(&(0x80, 0x10000)));
    assert!(a.properties.contains(&(0x81, 72000)));
    assert_eq!(a.source_next_shape, Some(8));
    assert_eq!(a.approximations[0].source_payload, data);
}
#[test]
fn secondary_identity_or_inset_conflicts_cannot_be_hidden_by_partial_effects() {
    let first = bytes(&[(0x80, 0x10000), (0x81, 72000)]);
    for second in [
        bytes(&[(0x80, 0x20000)]),
        bytes(&[(0x81, 60000)]),
        bytes(&[(0x8a, 9)]),
    ] {
        assert!(
            resolve(
                &[record(&first, 0xf00b), record(&second, 0xf122)],
                &BTreeMap::new(),
                8
            )
            .is_err()
        );
    }
}

#[test]
fn wrapping_distances_keep_signed_source_values_and_reject_conflicting_tables() {
    let data = bytes(&[
        (0x80, 0x10000),
        (0x384, 12700),
        (0x385, (-1i32) as u32),
        (0x386, 25400),
        (0x387, 38100),
    ]);
    let a = resolve(&[record(&data, 0xf00b)], &BTreeMap::new(), 8).unwrap();
    assert_eq!(
        a.wrap_distances,
        [Some(12700), Some(-1), Some(25400), Some(38100)]
    );
    assert_eq!(a.approximations[0].source_payload, data);
    assert_eq!(a.properties, [(0x80, 0x10000)]);
    let conflicting = bytes(&[(0x384, 999)]);
    assert!(
        resolve(
            &[record(&data, 0xf00b), record(&conflicting, 0xf121)],
            &BTreeMap::new(),
            8
        )
        .is_err()
    );
    let flagged = bytes(&[(0x4384, 12700)]);
    assert!(resolve(&[record(&flagged, 0xf00b)], &BTreeMap::new(), 8).is_err());
}
