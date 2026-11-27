use super::*;
use std::io::Write;
fn record(kind: u16, instance: u16, payload: &[u8]) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(&(instance << 4).to_le_bytes());
    b.extend_from_slice(&kind.to_le_bytes());
    b.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    b.extend_from_slice(payload);
    b
}
#[test]
fn raster_uid_count_preserves_exact_source_payload() {
    for instance in [0x6e0, 0x6e1] {
        let mut p = vec![0; if instance == 0x6e0 { 17 } else { 33 }];
        p.extend_from_slice(b"\x89PNG\r\n\x1a\nsynthetic");
        let b = record(0xf01e, instance, &p);
        let mut budget = Budget::default();
        let r = budget.record(&b, 0, b.len()).unwrap();
        let a = file_block(&b, r, "Data", &[], &mut budget).unwrap();
        assert_eq!(a.bytes, b"\x89PNG\r\n\x1a\nsynthetic");
        assert_eq!(
            &b[a.source.payload_range[0]..a.source.payload_range[1]],
            a.bytes
        );
    }
}
#[test]
fn zlib_requires_exact_size_end_and_no_trailing_bytes() {
    let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    e.write_all(b"metafile").unwrap();
    let b = e.finish().unwrap();
    assert_eq!(inflate(&b, 8).unwrap(), b"metafile");
    assert!(inflate(&b, 7).is_err());
    assert!(inflate(&b, 9).is_err());
    assert!(inflate(&b[..b.len() - 1], 8).is_err());
    let mut tail = b;
    tail.push(0);
    assert!(inflate(&tail, 8).is_err());
    assert!(inflate(&[], MAX_ASSET_BYTES + 1).is_err());
}
#[test]
fn budgets_and_record_bounds_are_independent() {
    let b = record(0xf01e, 0x6e0, &[]);
    let mut budget = Budget {
        records: 1,
        input_bytes: 100,
        remaining_bytes: 3,
    };
    assert!(budget.record(&b, 0, b.len()).is_ok());
    assert!(budget.record(&b, 0, b.len()).is_err());
    assert!(budget.charge(4).is_err());
    assert!(Budget::default().record(&b, usize::MAX, b.len()).is_err());
}
