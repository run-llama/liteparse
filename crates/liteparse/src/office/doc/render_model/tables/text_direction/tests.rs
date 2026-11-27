use super::*;
use crate::office::doc::{formatting::PropertyModifier, render_model::tables::decode_row};

fn row_properties(flags: u16) -> Vec<PropertyModifier> {
    let mut data = vec![0, 0, 1, 0, 0, 0xd0, 7];
    data.extend_from_slice(&flags.to_le_bytes());
    data.extend_from_slice(&[0; 18]);
    let length = (data.len() - 1) as u16;
    data[..2].copy_from_slice(&length.to_le_bytes());
    vec![
        PropertyModifier {
            sprm: 0x9602,
            operand: vec![108, 0],
        },
        PropertyModifier {
            sprm: 0xd608,
            operand: data,
        },
    ]
}

#[test]
fn ordered_cell_direction_overrides_definition_without_clearing_other_flags() {
    let mut properties = row_properties((3 << 2) | 0x80);
    let before = decode_row(0xd9, &properties).unwrap();
    assert_eq!(shared(before.cells[0].raw_flags).unwrap(), Some("btLr"));
    properties.push(PropertyModifier {
        sprm: 0x7629,
        operand: vec![0, 1, 1, 0],
    });
    let after = decode_row(0xd9, &properties).unwrap();
    assert_eq!(shared(after.cells[0].raw_flags).unwrap(), Some("tbRl"));
    assert_eq!(after.cells[0].raw_flags & 0x80, 0x80);
    properties.push(PropertyModifier {
        sprm: 0x7629,
        operand: vec![0, 1, 0, 0],
    });
    assert_eq!(
        shared(decode_row(0xd9, &properties).unwrap().cells[0].raw_flags).unwrap(),
        None
    );
}

#[test]
fn malformed_ranges_and_directions_reject_but_upright_state_can_be_overridden() {
    for operand in [
        vec![0, 2, 1, 0],
        vec![1, 1, 1, 0],
        vec![0, 1, 2, 0],
        vec![0, 1, 1, 1],
        vec![0, 1, 3],
    ] {
        let mut properties = row_properties(0);
        properties.push(PropertyModifier {
            sprm: 0x7629,
            operand,
        });
        assert!(decode_row(0xd9, &properties).is_err());
    }
    let mut properties = row_properties(5 << 2);
    assert!(shared(decode_row(0xd9, &properties).unwrap().cells[0].raw_flags).is_err());
    properties.push(PropertyModifier {
        sprm: 0x7629,
        operand: vec![0, 1, 3, 0],
    });
    assert_eq!(
        shared(decode_row(0xd9, &properties).unwrap().cells[0].raw_flags).unwrap(),
        Some("btLr")
    );
}
