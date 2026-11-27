//! Border appearance records preserve table text visibility after exact framing validation.

pub(super) fn legacy_array(data: &[u8]) -> bool {
    data.len() == 25
        && data[0] == 24
        && data[1..]
            .as_chunks::<4>()
            .0
            .iter()
            .all(|b| legacy_border(b))
}

// Brc80MayBeNil has an all-ones sentinel. Table borders exclude page-art values;
// Brc80 excludes types 0x1A/0x1B. Spacing, effects and reserved bits affect no text state.
// https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/cfab8014-e477-4e33-b50f-a23b8476f6f3
fn legacy_border(data: &[u8]) -> bool {
    data == [255; 4]
        || (data.len() == 4 && data[2] <= 16 && matches!(data[1], 0 | 1 | 3 | 5..=25 | 255))
}

// The source contains one COLORREF for every currently defined physical cell.
// Nil color and automatic color both preserve character properties.
pub(super) fn cell_colors(data: &[u8], columns: usize) -> bool {
    columns > 0
        && columns <= 63
        && data.len() == 1 + 4 * columns
        && usize::from(data[0]) == 4 * columns
        && data[1..]
            .as_chunks::<4>()
            .0
            .iter()
            .all(|color| matches!(color[3], 0 | 255))
}
