use super::*;
use crate::office::doc::tests::{put16, put32};

fn piece(start: usize, end: usize, fc: usize, compressed: bool) -> PieceProperties {
    PieceProperties {
        start_cp: start,
        end_cp: end,
        file_offset: fc,
        compressed,
        raw_flags: 0,
        raw_prm: 0,
    }
}

fn fixture() -> (Vec<u8>, Vec<u8>, Vec<PieceProperties>, [usize; 8]) {
    let mut word = vec![0; 2048];
    let mut plc = vec![0; 12];
    put32(&mut plc, 0, 100);
    put32(&mut plc, 4, 104);
    put32(&mut plc, 8, 1);
    put32(&mut word, 512, 100);
    put32(&mut word, 516, 104);
    word[512 + 8] = 240;
    word[512 + 480] = 0;
    word[512 + 481] = 1;
    put16(&mut word, 512 + 482, 7);
    word[1023] = 1;
    word[103] = 13;
    word[1500..1503].copy_from_slice(b"abc");
    let mut a = piece(0, 3, 1500, true);
    a.raw_prm = 1;
    let mut b = piece(3, 7, 100, true);
    b.raw_prm = 3;
    (word, plc, vec![a, b], [7, 0, 0, 0, 0, 0, 0, 0])
}

#[test]
fn later_logical_piece_owns_properties_despite_lower_physical_offset() {
    let (word, plc, pieces, lengths) = fixture();
    let result = select(&word, &plc, &pieces, 0xc1, &lengths).unwrap();
    assert_eq!(result.formatting.spans.len(), 1);
    let span = &result.formatting.spans[0];
    assert_eq!(
        (span.start_cp, span.end_cp, span.style_index),
        (0, 7, Some(7))
    );
    assert!(span.raw_modifiers.is_empty());
    assert_eq!(result.endpoints[0].piece_index, 1);
    assert_eq!(pieces[result.endpoints[0].piece_index].raw_prm, 3);
    assert_eq!(result.uncovered_fragments[0].source_cp, [0, 3]);
    assert_eq!(result.uncovered_fragments[0].physical_fc, [1500, 1503]);
}

#[test]
fn missing_actual_endpoint_or_invalid_terminal_cannot_supply_properties() {
    let (mut word, plc, mut pieces, lengths) = fixture();
    word[103] = b'x';
    let result = select(&word, &plc, &pieces, 0xc1, &lengths).unwrap();
    assert_eq!(result.uncertain_terminals[0].source_cp, 6);
    assert_eq!(result.uncertain_terminals[0].unit, b'x' as u16);
    pieces[1].file_offset = 1600;
    word[1603] = 13;
    assert!(select(&word, &plc, &pieces, 0xc1, &lengths).is_err());
}

#[test]
fn multiple_gaps_and_field_revision_controls_do_not_establish_endpoints() {
    let (mut word, plc, _, _) = fixture();
    word[1500..1503].copy_from_slice(&[19, 20, 21]);
    word[1600..1603].copy_from_slice(&[13, 7, 12]);
    let pieces = [
        piece(0, 3, 1500, true),
        piece(3, 6, 1600, true),
        piece(6, 10, 100, true),
    ];
    let result = select(&word, &plc, &pieces, 0xc1, &[10, 0, 0, 0, 0, 0, 0, 0]).unwrap();
    assert_eq!(result.formatting.spans.len(), 1);
    assert_eq!(result.endpoints[0].end_cp, 10);
    assert_eq!(result.uncovered_fragments.len(), 2);
}

#[test]
fn cell_section_and_unicode_terminal_units_use_physical_endpoints() {
    for terminal in [7, 12, 13] {
        let (mut word, plc, mut pieces, _) = fixture();
        pieces[1] = piece(3, 5, 100, false);
        put16(&mut word, 100, 0xd83d);
        put16(&mut word, 102, terminal);
        let result = select(&word, &plc, &pieces, 0xc1, &[5, 0, 0, 0, 0, 0, 0, 0]).unwrap();
        assert_eq!(result.endpoints[0].end_cp, 5);
    }
}

#[test]
fn story_boundaries_and_extra_terminal_cp_are_explicit() {
    let (word, plc, mut pieces, lengths) = fixture();
    assert!(select(&word, &plc, &pieces, 0xc1, &[3, 4, 0, 0, 0, 0, 0, 0]).is_err());
    pieces.push(piece(7, 8, 1700, true));
    let result = select(&word, &plc, &pieces, 0xc1, &lengths).unwrap();
    assert_eq!(result.trailing_nonstory_cp_range, Some([7, 8]));
    assert_eq!(result.uncovered_fragments.len(), 1);
}

#[test]
fn reused_physical_properties_are_charged_for_each_logical_paragraph() {
    let (mut word, plc, _, _) = fixture();
    word[512 + 8] = 120;
    word[512 + 240] = 0;
    word[512 + 241] = 126;
    put16(&mut word, 512 + 242, 0);
    let pieces: Vec<_> = (0..32768)
        .map(|i| piece(i * 4, i * 4 + 4, 100, true))
        .collect();
    assert!(matches!(
        select(&word, &plc, &pieces, 0xc1, &[131072, 0, 0, 0, 0, 0, 0, 0]),
        Err(Error::Limit("paragraph endpoint property bytes"))
    ));
}
