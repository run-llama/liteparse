//! Logical property validation independent of supported border painting styles.

// Character borders use Brc80/Brc, not the table-only MayBeNil wrappers.
// Spacing, shadow, frame and reserved bits do not modify character visibility.
// https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/7022285b-9621-42e9-ad4d-4e02c115ef18
pub(crate) fn character_border(sprm: u16, data: &[u8]) -> bool {
    match sprm {
        0x6865 => data.len() == 4 && data[2] <= 16 && matches!(data[1], 0 | 1 | 3 | 5..=25 | 255),
        0xca72 => {
            data.len() == 9
                && data[0] == 8
                && matches!(data[4], 0 | 255)
                && matches!(data[6], 0 | 1 | 3 | 5..=27 | 255)
        }
        _ => false,
    }
}
