use super::*;
fn definition(columns: u8, flags: &[u16]) -> Vec<u8> {
    let mut d = vec![0, 0, columns];
    for i in 0..=columns {
        d.extend((i as i16 * 100).to_le_bytes());
    }
    for flag in flags {
        d.extend(flag.to_le_bytes());
        d.extend([0; 18]);
    }
    let cb = (d.len() - 1) as u16;
    d[..2].copy_from_slice(&cb.to_le_bytes());
    d
}
#[test]
fn definition_preserves_merges_and_missing_cell_defaults() {
    let d = decode_table_definition(&definition(3, &[2, 0x21])).unwrap();
    assert_eq!(d.grid_twips, vec![0, 100, 200, 300]);
    assert_eq!(d.cells[0].horizontal_merge, 2);
    assert_eq!(d.cells[1].vertical_merge, 1);
    assert_eq!(d.cells[2].raw_flags, 0);
}
#[test]
fn invalid_vertical_merge_and_layout_are_rejected_but_excess_records_ignored() {
    assert!(decode_table_definition(&definition(1, &[0x40])).is_err());
    assert!(decode_table_definition(&definition(1, &[0x180])).is_err());
    assert!(decode_table_definition(&definition(1, &[0, 0xffff])).is_ok());
    assert!(
        decode_table_definition(&definition(0, &[]))
            .unwrap()
            .cells
            .is_empty()
    );
}
#[test]
fn malformed_definition_lengths_and_edges_are_errors() {
    let mut d = definition(1, &[]);
    d[5..7].copy_from_slice(&(-1i16).to_le_bytes());
    assert!(decode_table_definition(&d).is_err());
    let mut d = definition(1, &[]);
    d.pop();
    assert!(decode_table_definition(&d).is_err());
}
