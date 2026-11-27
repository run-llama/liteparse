use super::*;

fn fixture() -> (Vec<u8>, Vec<u8>) {
    let mut word = vec![0; 1024];
    let page = &mut word[512..];
    for (i, fc) in [100u32, 104, 108].iter().enumerate() {
        page[i * 4..i * 4 + 4].copy_from_slice(&fc.to_le_bytes());
    }
    page[12] = 10;
    page[13] = 0;
    page[20..27].copy_from_slice(&[6, 0x3c, 0x08, 0x81, 0x00, 0x08, 1]);
    page[511] = 2;
    let plc = [100u32, 108, 0xffc0_0001]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect();
    (word, plc)
}

#[test]
fn preserves_visibility_modifiers_and_empty_chpx() {
    let (word, plc) = fixture();
    let runs = decode_chpx(&word, &plc).unwrap();
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].start_fc, 100);
    assert_eq!(runs[0].end_fc, 104);
    assert_eq!(runs[0].properties[0].sprm, 0x083c);
    assert_eq!(
        runs[0].properties[0].toggle().unwrap(),
        Some(ToggleOperand::NotStyle)
    );
    assert_eq!(
        runs[0].properties[1].toggle().unwrap(),
        Some(ToggleOperand::On)
    );
    assert!(runs[1].properties.is_empty());
}

#[test]
fn all_toggle_values_remain_symbolic_and_ordered() {
    let group = [0x55, 8, 0, 0x55, 8, 1, 0x55, 8, 0x80, 0x55, 8, 0x81];
    let properties = decode_character_modifiers(&group).unwrap();
    let values: Vec<_> = properties
        .iter()
        .map(|p| p.toggle().unwrap().unwrap())
        .collect();
    assert_eq!(
        values,
        [
            ToggleOperand::Off,
            ToggleOperand::On,
            ToggleOperand::Style,
            ToggleOperand::NotStyle
        ]
    );
    assert!(decode_character_modifiers(&[0x55, 8, 2]).is_err());
}

#[test]
fn unknown_character_operands_keep_their_boundaries() {
    let mut group = Vec::new();
    for (spra, operand) in [
        (1u16, vec![9]),
        (2, vec![1, 2]),
        (3, vec![1, 2, 3, 4]),
        (4, vec![1, 2]),
        (5, vec![1, 2]),
        (6, vec![3, 1, 2, 3]),
        (7, vec![1, 2, 3]),
    ] {
        group.extend_from_slice(&((spra << 13) | 0x080f).to_le_bytes());
        group.extend(operand);
    }
    let properties = decode_character_modifiers(&group).unwrap();
    assert_eq!(properties.len(), 7);
    assert_eq!(properties[5].operand, [3, 1, 2, 3]);
    assert!(decode_character_modifiers(&group[..group.len() - 1]).is_err());
    assert!(decode_character_modifiers(&[0x08, 0xd6, 0]).is_err());
}

#[test]
fn maps_reordered_compressed_and_unicode_pieces() {
    let (word, plc) = fixture();
    let runs = decode_chpx(&word, &plc).unwrap();
    let pieces = [
        TextPiece {
            start_cp: 0,
            end_cp: 2,
            file_offset: 104,
            compressed: false,
        },
        TextPiece {
            start_cp: 2,
            end_cp: 8,
            file_offset: 100,
            compressed: true,
        },
    ];
    let spans = map_runs_to_pieces(&runs, &pieces).unwrap();
    assert_eq!(
        spans
            .iter()
            .map(|s| (s.start_cp, s.end_cp))
            .collect::<Vec<_>>(),
        [(0, 2), (2, 6), (6, 8)]
    );
    assert!(spans[0].properties.is_empty());
    assert_eq!(spans[1].properties[0].sprm, 0x083c);
}

#[test]
fn rejects_gaps_misaligned_unicode_and_invalid_pieces() {
    let (word, plc) = fixture();
    let runs = decode_chpx(&word, &plc).unwrap();
    for piece in [
        TextPiece {
            start_cp: 0,
            end_cp: 12,
            file_offset: 100,
            compressed: true,
        },
        TextPiece {
            start_cp: 0,
            end_cp: 2,
            file_offset: 101,
            compressed: false,
        },
        TextPiece {
            start_cp: 1,
            end_cp: 2,
            file_offset: 100,
            compressed: true,
        },
        TextPiece {
            start_cp: 0,
            end_cp: 1,
            file_offset: usize::MAX,
            compressed: true,
        },
    ] {
        assert!(map_runs_to_pieces(&runs, &[piece]).is_err());
    }
}

#[test]
fn rejects_corrupt_pages_and_payload_bounds() {
    let (word, plc) = fixture();
    for (offset, value) in [(1023, 0), (1023, 102), (524, 1), (532, 255)] {
        let mut bad = word.clone();
        bad[offset] = value;
        assert!(decode_chpx(&bad, &plc).is_err());
    }
    let mut overrun = word.clone();
    overrun[524] = 255;
    overrun[1022] = 1;
    assert!(decode_chpx(&overrun, &plc).is_err());
    assert!(decode_chpx(&word[..900], &plc).is_err());
    assert!(decode_chpx(&word, &plc[..11]).is_err());
    let mut duplicate = word;
    duplicate[516..520].copy_from_slice(&100u32.to_le_bytes());
    assert!(decode_chpx(&duplicate, &plc).is_err());
}

#[test]
fn rejects_every_truncated_modifier_prefix() {
    for group in [&[0x3c, 0x08, 1][..], &[0x0f, 0xc8, 3, 1, 2, 3][..]] {
        for end in 1..group.len() {
            assert!(decode_character_modifiers(&group[..end]).is_err());
        }
    }
}

#[test]
fn clips_page_runs_to_their_plc_range() {
    let (word, mut plc) = fixture();
    plc[..4].copy_from_slice(&102u32.to_le_bytes());
    plc[4..8].copy_from_slice(&106u32.to_le_bytes());
    let runs = decode_chpx(&word, &plc).unwrap();
    assert_eq!((runs[0].start_fc, runs[0].end_fc), (102, 104));
    assert_eq!((runs[1].start_fc, runs[1].end_fc), (104, 106));
}

#[test]
fn budgets_charge_each_reused_property_and_empty_run() {
    let property = PropertyModifier {
        sprm: 0x083c,
        operand: vec![1],
    };
    let cost = std::mem::size_of::<PropertyModifier>() + 1;
    let mut budget = Budget {
        runs: 3,
        property_bytes: cost * 2,
    };
    budget.reserve(std::slice::from_ref(&property)).unwrap();
    budget.reserve(std::slice::from_ref(&property)).unwrap();
    assert_eq!(
        budget.reserve(&[property]),
        Err(Error::Limit("character formatting property bytes"))
    );
    let mut budget = Budget {
        runs: 1,
        property_bytes: 0,
    };
    budget.reserve(&[]).unwrap();
    assert_eq!(
        budget.reserve(&[]),
        Err(Error::Limit("character formatting runs"))
    );
}
