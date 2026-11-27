use super::*;
use crate::office::doc::formatting::ToggleOperand;
use crate::office::doc::tests::{fixture, put16, put32};

fn formatted_fixture() -> Vec<u8> {
    let mut data = fixture();
    let word = &mut data[1536..5632];
    put32(word, 154 + 12 * 8, 64);
    put32(word, 158 + 12 * 8, 12);
    let page = &mut word[3072..3584];
    put32(page, 0, 2048);
    put32(page, 4, 2058);
    page[8] = 10;
    page[20..27].copy_from_slice(&[6, 0x3c, 0x08, 1, 0x55, 0x08, 0x80]);
    page[511] = 1;
    let table = &mut data[5632..9728];
    put32(table, 64, 2048);
    put32(table, 68, 2058);
    put32(table, 72, 6);
    data
}

#[test]
fn inspects_container_formatting_without_resolving_visibility() {
    let data = formatted_fixture();
    let result = inspect_character_formatting(&data).unwrap();
    assert_eq!(result.status, "unresolved_character_formatting");
    assert_eq!(result.binary_version, 0x112);
    assert_eq!(result.pieces.len(), 1);
    assert_eq!(result.pieces[0].file_offset, 2048);
    assert_eq!(result.pieces[0].raw_prm, 0);
    let span = &result.character_spans[0];
    assert_eq!((span.start_cp, span.end_cp), (0, 10));
    assert_eq!(
        span.properties[0].toggle().unwrap(),
        Some(ToggleOperand::On)
    );
    assert_eq!(
        span.properties[1].toggle().unwrap(),
        Some(ToggleOperand::Style)
    );
    assert_eq!(
        crate::office::doc::inspect(&data).unwrap().stories[0].text,
        "Body\r"
    );
}

#[test]
fn preserves_raw_piece_properties_and_complex_group_reference() {
    let mut data = formatted_fixture();
    let table = &mut data[5632..9728];
    table.copy_within(0..21, 6);
    table[..6].copy_from_slice(&[1, 3, 0, 0x3c, 0x08, 0]);
    put16(table, 6 + 19, 1);
    put32(&mut data, 1536 + 158 + 33 * 8, 27);
    let result = inspect_character_formatting(&data).unwrap();
    assert_eq!(result.raw_property_groups, [vec![0x3c, 0x08, 0]]);
    assert_eq!(result.pieces[0].raw_prm, 1);
    // The piece requests hidden=off; the diagnostic CHPX span still records hidden=on.
    assert_eq!(result.character_spans[0].properties[0].operand, [1]);
    put16(&mut data, 5632 + 6 + 19, 3);
    assert!(matches!(
        inspect_character_formatting(&data),
        Err(Error::Invalid("CLX property group reference"))
    ));
}

#[test]
fn absent_or_invalid_formatting_keeps_raw_extraction_available() {
    let data = fixture();
    assert!(crate::office::doc::inspect(&data).is_ok());
    assert!(matches!(
        inspect_character_formatting(&data),
        Err(Error::Invalid("missing character formatting PLC"))
    ));
    let mut data = formatted_fixture();
    put32(&mut data, 5632 + 72, 8000);
    assert!(crate::office::doc::inspect(&data).is_ok());
    assert!(inspect_character_formatting(&data).is_err());
}

#[test]
fn shares_fib_revision_encryption_and_story_validation() {
    let mut data = formatted_fixture();
    put16(&mut data, 1536 + 10, 0x300);
    assert!(matches!(
        inspect_character_formatting(&data),
        Err(Error::Unsupported("encrypted binary Word"))
    ));
    let mut data = formatted_fixture();
    put16(&mut data, 1536 + 156 + 183 * 8, 0xffff);
    // An unlisted extended revision reads by its offset table's size.
    assert!(inspect_character_formatting(&data).is_ok());
    let mut data = formatted_fixture();
    put32(&mut data, 1536 + 76, 100);
    assert!(matches!(
        inspect_character_formatting(&data),
        Err(Error::Invalid("formatting piece story coverage"))
    ));
}

#[test]
fn metadata_maps_unicode_and_reordered_pieces() {
    let word = vec![0; 128];
    let mut clx = vec![0; 33];
    clx[0] = 2;
    put32(&mut clx, 1, 28);
    put32(&mut clx, 5, 0);
    put32(&mut clx, 9, 2);
    put32(&mut clx, 13, 4);
    put32(&mut clx, 19, 80);
    put32(&mut clx, 27, 0x4000_0028);
    put16(&mut clx, 25, 0x8000);
    let (pieces, _, total) = decode_piece_metadata(&word, &clx).unwrap();
    assert_eq!(total, 4);
    assert_eq!(pieces[0].file_offset, 80);
    assert!(!pieces[0].compressed);
    assert_eq!(pieces[1].file_offset, 20);
    assert!(pieces[1].compressed);
    assert_eq!(pieces[1].raw_flags, 0x8000);
    put32(&mut clx, 27, 0x4000_0029);
    assert!(decode_piece_metadata(&word, &clx).is_err());
}

#[test]
fn rejects_negative_group_lengths_and_truncated_clx() {
    assert!(matches!(
        decode_piece_metadata(&[], &[1, 0xff, 0xff]),
        Err(Error::Invalid("CLX property group length"))
    ));
    let data = formatted_fixture();
    let word = &data[1536..5632];
    let clx = &data[5632..5653];
    for end in 0..clx.len() {
        assert!(decode_piece_metadata(word, &clx[..end]).is_err());
    }
}

#[test]
fn bounds_empty_group_metadata() {
    let mut clx = [1, 0, 0].repeat(MAX_GROUPS + 1);
    clx.extend_from_slice(&[2, 4, 0, 0, 0, 0, 0, 0, 0]);
    assert!(matches!(
        decode_piece_metadata(&[], &clx),
        Err(Error::Limit("CLX property groups"))
    ));
}

#[test]
fn nonstory_terminal_cp_preserves_piece_but_needs_no_character_properties() {
    let mut data = formatted_fixture();
    put32(&mut data, 5632 + 9, 11);
    let result = inspect_character_formatting(&data).unwrap();
    assert_eq!(result.authored_story_end_cp, 10);
    assert_eq!(result.trailing_nonstory_cp_range, Some([10, 11]));
    assert_eq!(result.pieces[0].end_cp, 11);
    assert_eq!(result.character_spans.last().unwrap().end_cp, 10);
    assert_eq!(
        crate::office::doc::visibility::resolve_revision_evidence(&result)
            .unwrap()
            .spans
            .last()
            .unwrap()
            .end_cp,
        10
    );
    assert_eq!(
        crate::office::doc::inspect(&data).unwrap().stories[0].text,
        "Body\r"
    );
}

#[test]
fn last_authored_character_gap_remains_an_error() {
    let mut data = formatted_fixture();
    put32(&mut data, 5632 + 9, 11);
    put32(&mut data, 1536 + 3072 + 4, 2057);
    put32(&mut data, 5632 + 68, 2057);
    assert!(matches!(
        inspect_character_formatting(&data),
        Err(Error::Invalid("character formatting coverage gap"))
    ));
}

#[test]
fn authored_mapping_clips_logical_order_with_reordered_physical_pieces() {
    let piece = |start_cp, end_cp, file_offset| PieceProperties {
        start_cp,
        end_cp,
        file_offset,
        compressed: true,
        raw_flags: 0,
        raw_prm: 0,
    };
    let pieces = vec![piece(0, 2, 100), piece(2, 4, 20), piece(4, 5, 200)];
    let mapped = authored_mappings(&pieces, 4).unwrap();
    assert_eq!(mapped.len(), 2);
    assert_eq!(mapped[0].file_offset, 100);
    assert_eq!(mapped[1].file_offset, 20);
    assert_eq!(mapped[1].end_cp, 4);
    assert!(authored_mappings(&[piece(1, 2, 100)], 2).is_err());
    assert!(authored_mappings(&pieces, 3).is_err());
}
