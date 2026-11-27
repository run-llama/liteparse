use super::*;
use crate::office::doc::tests::{fixture, put16, put32};

#[test]
fn compressed_mapping_is_not_windows_1252() {
    assert_eq!(compressed_unit(0x80), 0x80);
    assert_eq!(compressed_unit(0x8e), 0x8e);
    assert_eq!(compressed_unit(0x91), 0x2018);
    assert_eq!(compressed_unit(0x9f), 0x178);
}
#[test]
fn mixed_pieces_retain_logical_order_and_utf16() {
    let mut word = vec![0; 128];
    word[20..22].copy_from_slice(b"A\r");
    put16(&mut word, 40, 0x4e2d);
    put16(&mut word, 42, 13);
    let mut clx = vec![0; 33];
    clx[0] = 2;
    put32(&mut clx, 1, 28);
    for (i, v) in [0, 2, 4].into_iter().enumerate() {
        put32(&mut clx, 5 + i * 4, v);
    }
    put32(&mut clx, 19, 0x4000_0028);
    put32(&mut clx, 27, 40);
    let (raw, n) = pieces(&word, &clx).unwrap();
    assert_eq!(n, 2);
    assert_eq!(String::from_utf16(&raw).unwrap(), "A\r中\r");
    put32(&mut clx, 9, 0);
    assert!(pieces(&word, &clx).is_err());
}
#[test]
fn short_text_and_declared_excess_are_errors() {
    let mut b = fixture();
    put32(&mut b, 1536 + 76, 11);
    assert!(inspect(&b).is_err());
    let mut b = fixture();
    put32(&mut b, 5632 + 9, 20_000_000);
    assert!(matches!(inspect(&b), Err(Error::Limit(_))));
}
#[test]
fn supports_both_table_stream_selections() {
    let mut b = fixture();
    put16(&mut b, 1536 + 10, 0);
    put16(&mut b, 1024 + 256, b'0' as u16);
    assert_eq!(inspect(&b).unwrap().stories[0].text, "Body\r");
}
#[test]
fn rejects_unassigned_text_and_preserves_terminal_unit() {
    let mut b = fixture();
    put32(&mut b, 1536 + 80, 4);
    let result = inspect(&b).unwrap();
    assert_eq!(result.trailing_utf16_units, [13]);
    put32(&mut b, 1536 + 80, 3);
    assert!(inspect(&b).is_err());
}

#[test]
fn splits_all_document_stories_in_cp_order() {
    let mut b = fixture();
    let lengths = [2, 1, 1, 0, 1, 1, 2, 2];
    for (i, len) in lengths.into_iter().enumerate() {
        put32(&mut b, 1536 + 76 + i * 4, len);
    }
    b[1536 + 2048..1536 + 2058].copy_from_slice(b"abfhcetTHH");
    let result = inspect(&b).unwrap();
    assert_eq!(
        result
            .stories
            .iter()
            .map(|s| s.text.as_str())
            .collect::<Vec<_>>(),
        ["ab", "f", "h", "", "c", "e", "tT", "HH"]
    );
}

#[test]
fn recognizes_declared_revisions_and_reads_unknown_ones_by_table_size() {
    for version in VERSIONS {
        let mut b = fixture();
        put16(&mut b, 1536 + 156 + 183 * 8, *version);
        assert_eq!(inspect(&b).unwrap().binary_version, *version);
    }
    let mut b = fixture();
    put16(&mut b, 1536 + 156 + 183 * 8, 0xffff);
    // 183 offset-table entries is the Word 2007 layout.
    assert_eq!(inspect(&b).unwrap().binary_version, 0x0112);
}

#[test]
fn an_unlisted_revision_with_an_odd_offset_table_is_rejected() {
    // FIB header, 93-entry table plus one extra entry, then nFibNew 0xC3.
    let mut word = vec![0u8; 154 + 94 * 8 + 4];
    word[0..2].copy_from_slice(&0xa5ecu16.to_le_bytes());
    word[2..4].copy_from_slice(&0xc1u16.to_le_bytes());
    word[32..34].copy_from_slice(&14u16.to_le_bytes());
    word[62..64].copy_from_slice(&22u16.to_le_bytes());
    word[152..154].copy_from_slice(&94u16.to_le_bytes());
    let end = 154 + 94 * 8;
    word[end..end + 2].copy_from_slice(&1u16.to_le_bytes());
    word[end + 2..end + 4].copy_from_slice(&0xc3u16.to_le_bytes());
    assert!(matches!(
        fib(&word),
        Err(Error::Unsupported("extended binary Word revision"))
    ));
    word[152..154].copy_from_slice(&93u16.to_le_bytes());
    let end = 154 + 93 * 8;
    word[end..end + 2].copy_from_slice(&1u16.to_le_bytes());
    word[end + 2..end + 4].copy_from_slice(&0xc3u16.to_le_bytes());
    assert_eq!(fib(&word).unwrap().0, 0x00c1);
}
