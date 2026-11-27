//! Table appearance operands qualified against ordered physical cell ownership.

// No-wrap and empty-cell mark height affect layout, not ordinary hidden text.
// Validate their half-open ranges against cells already defined in this row.
// https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/b39a6648-501c-4361-8366-4f042f579469
pub(super) fn preserves_text(sprm: u16, data: &[u8], columns: usize) -> bool {
    match sprm {
        0xd639 | 0xd642 => {
            data.len() == 4
                && data[0] == 3
                && columns <= 63
                && usize::from(data[1]) < columns
                && data[2] >= data[1]
                && usize::from(data[2]) <= columns
                && data[3] <= 1
        }
        0xd633 => {
            data.len() == 7
                && data[..4] == [6, 0, 1, 15]
                && matches!(data[4], 0 | 3 | 0x13)
                && u16::from_le_bytes([data[5], data[6]]) <= 15840
                && (data[4] != 0 || data[5..7] == [0, 0])
        }
        0xd660 => {
            data.len() == 11
                && data[0] == 10
                && matches!(data[4], 0 | 255)
                && matches!(data[8], 0 | 255)
                && matches!(u16::from_le_bytes([data[9], data[10]]), 0..=0x19 | 0x23..=0x3d | 0xffff)
        }
        _ => false,
    }
}
