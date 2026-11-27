use super::*;
use crate::office::doc::tests::{fixture, put16, put32};

fn fixture_pages() -> (Vec<u8>, Vec<u8>) {
    let mut word = vec![0; 1024];
    let mut plc = vec![0; 12];
    put32(&mut plc, 0, 20);
    put32(&mut plc, 4, 40);
    put32(&mut plc, 8, 1 | 0xffc0_0000);
    let page = &mut word[512..];
    for (offset, value) in [(0, 20), (4, 30), (8, 40)] {
        put32(page, offset, value);
    }
    page[12] = 240;
    page[25] = 248;
    page[511] = 2;
    page[480] = 2;
    put16(page, 481, 7);
    page[483] = 0x42;
    page[496] = 0;
    page[497] = 2;
    put16(page, 498, 9);
    page[500] = 0x11;
    page[501] = 0x22;
    (word, plc)
}

#[test]
fn decodes_both_papx_length_forms_and_preserves_source_styles() {
    let (word, plc) = fixture_pages();
    let runs = decode_papx(&word, &plc).unwrap();
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].style_index, Some(7));
    assert_eq!(runs[0].raw_modifiers, [0x42]);
    assert_eq!(runs[1].style_index, Some(9));
    assert_eq!(runs[1].raw_modifiers, [0x11, 0x22]);
}

#[test]
fn default_property_record_is_distinct_from_explicit_style_zero() {
    let (mut word, plc) = fixture_pages();
    word[512 + 12] = 0;
    put16(&mut word, 512 + 498, 0);
    let runs = decode_papx(&word, &plc).unwrap();
    assert_eq!(runs[0].style_index, None);
    assert!(runs[0].raw_modifiers.is_empty());
    assert_eq!(runs[1].style_index, Some(0));
}

#[test]
fn rejects_bad_counts_offsets_lengths_and_page_bounds() {
    for (offset, value) in [(511, 0), (511, 30), (12, 1), (480, 1), (497, 0), (497, 255)] {
        let (mut word, plc) = fixture_pages();
        word[512 + offset] = value;
        assert!(decode_papx(&word, &plc).is_err());
    }
    let (word, mut plc) = fixture_pages();
    put32(&mut plc, 8, 3);
    assert!(decode_papx(&word, &plc).is_err());
    let (mut word, plc) = fixture_pages();
    put32(&mut word, 512 + 4, 20);
    assert!(decode_papx(&word, &plc).is_err());
    assert!(decode_papx(&word, &[0]).is_err());
}

#[test]
fn maps_reordered_and_unicode_pieces_as_fragments() {
    let (word, plc) = fixture_pages();
    let runs = decode_papx(&word, &plc).unwrap();
    let pieces = [
        TextPiece {
            start_cp: 0,
            end_cp: 5,
            file_offset: 30,
            compressed: false,
        },
        TextPiece {
            start_cp: 5,
            end_cp: 15,
            file_offset: 20,
            compressed: true,
        },
    ];
    let spans = map_runs_to_pieces(&runs, &pieces).unwrap();
    assert_eq!(
        (spans[0].start_cp, spans[0].end_cp, spans[0].style_index),
        (0, 5, Some(9))
    );
    assert_eq!(
        (spans[1].start_cp, spans[1].end_cp, spans[1].style_index),
        (5, 15, Some(7))
    );
}

#[test]
fn rejects_gaps_utf16_splits_and_piece_overflow() {
    let (word, plc) = fixture_pages();
    let mut runs = decode_papx(&word, &plc).unwrap();
    let mut piece = TextPiece {
        start_cp: 0,
        end_cp: 10,
        file_offset: 20,
        compressed: false,
    };
    runs[0].end_fc = 29;
    assert!(map_runs_to_pieces(&runs, &[piece]).is_err());
    runs[0].end_fc = 28;
    assert!(map_runs_to_pieces(&runs, &[piece]).is_err());
    piece.file_offset = usize::MAX;
    assert!(map_runs_to_pieces(&runs, &[piece]).is_err());
}

#[test]
fn metadata_byte_budget_precedes_clone_allocation() {
    let runs = [ParagraphRun {
        start_fc: 0,
        end_fc: 1,
        style_index: None,
        raw_modifiers: vec![0; MAX_METADATA_BYTES],
    }];
    let piece = TextPiece {
        start_cp: 0,
        end_cp: 1,
        file_offset: 0,
        compressed: true,
    };
    assert!(matches!(
        map_runs_to_pieces(&runs, &[piece]),
        Err(Error::Limit("paragraph formatting bytes"))
    ));
    let plc = vec![0; (MAX_RECORDS + 1) * 8 + 4];
    assert!(matches!(
        decode_papx(&[], &plc),
        Err(Error::Limit("paragraph formatting pages"))
    ));
}

#[test]
fn container_inspection_is_independent_of_character_formatting() {
    let mut data = fixture();
    let word = &mut data[1536..5632];
    put32(word, 154 + 13 * 8, 64);
    put32(word, 158 + 13 * 8, 12);
    let page = &mut word[3072..3584];
    put32(page, 0, 2048);
    put32(page, 4, 2058);
    page[8] = 240;
    page[480] = 0;
    page[481] = 1;
    put16(page, 482, 2);
    page[511] = 1;
    let table = &mut data[5632..];
    put32(table, 64, 2048);
    put32(table, 68, 2058);
    put32(table, 72, 6);
    let result = inspect_paragraph_formatting(&data).unwrap();
    assert_eq!(result.status, "unresolved_paragraph_fragments");
    assert_eq!(result.spans.len(), 1);
    assert_eq!(result.spans[0].style_index, Some(2));
    assert!(
        crate::office::doc::formatting_inspection::inspect_character_formatting(&data).is_err()
    );
    assert!(crate::office::doc::inspect(&data).is_ok());
}

#[test]
fn missing_paragraph_metadata_remains_an_explicit_failure() {
    assert!(matches!(
        inspect_paragraph_formatting(&fixture()),
        Err(Error::Invalid("missing paragraph formatting PLC"))
    ));
}
