use super::*;

fn record(kind: u16, version: u16, instance: u16, body: &[u8]) -> Vec<u8> {
    let mut r = Vec::new();
    r.extend(((instance << 4) | version).to_le_bytes());
    r.extend(kind.to_le_bytes());
    r.extend((body.len() as u32).to_le_bytes());
    r.extend(body);
    r
}
fn jpeg() -> Vec<u8> {
    let mut b = vec![0; 17];
    b.extend([0xff, 0xd8, 0xff, 0xd9]);
    record(0xf01d, 0, 0x46a, &b)
}
fn block(embedded: bool, offset: u32) -> Vec<u8> {
    let child = jpeg();
    let mut b = vec![0; 36];
    b[0] = 5;
    b[1] = 5;
    b[20..24].copy_from_slice(&(child.len() as u32).to_le_bytes());
    b[24..28].copy_from_slice(&1u32.to_le_bytes());
    b[28..32].copy_from_slice(&offset.to_le_bytes());
    if embedded {
        b.extend(child);
    }
    record(0xf007, 2, 5, &b)
}
fn root(stores: &[Vec<u8>]) -> Vec<u8> {
    record(0xf000, 15, 0, &stores.concat())
}

#[test]
fn store_indices_and_source_payload_ranges_preserve_embedded_and_delayed_bytes() {
    let mut blocks = block(true, u32::MAX);
    blocks.extend(block(false, 3));
    let table = root(&[record(0xf001, 15, 2, &blocks)]);
    let mut word = vec![0; 3];
    word.extend(jpeg());
    let assets = decode(&table, 0, table.len(), &word).unwrap();
    assert_eq!(assets.len(), 2);
    assert_eq!(assets[0].one_based_index, 1);
    assert_eq!(assets[1].one_based_index, 2);
    assert_eq!(assets[0].asset.bytes(), assets[1].asset.bytes());
    assert_eq!(assets[0].asset.source.stream, "Table");
    assert_eq!(assets[1].asset.source.stream, "WordDocument");
    let [start, end] = assets[1].asset.source.payload_range;
    assert_eq!(&word[start..end], assets[1].asset.bytes());
}

#[test]
fn failed_or_duplicate_store_slots_do_not_compact_indices() {
    let valid = record(0xf001, 15, 1, &block(true, 0));
    let duplicate = root(&[valid.clone(), valid]);
    assert!(decode(&duplicate, 0, duplicate.len(), &[]).is_err());
    for table in [
        root(&[record(0xf001, 15, 2, &block(true, 0))]),
        root(&[record(0xf001, 15, 0, &block(true, 0))]),
        root(&[record(0xf001, 15, 1, &block(false, u32::MAX))]),
        root(&[record(0xf001, 15, 1, &jpeg())]),
    ] {
        assert!(decode(&table, 0, table.len(), &[]).is_err());
    }
}
