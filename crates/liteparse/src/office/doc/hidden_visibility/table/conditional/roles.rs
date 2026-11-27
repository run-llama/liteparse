use crate::office::doc::{Error, Result};

pub(super) struct Position<'a> {
    pub row: usize,
    pub column: usize,
    pub columns: usize,
    pub headers: &'a [bool],
    pub right_to_left: bool,
    pub preceding_non_header_rows: usize,
}

/// MS-DOC §2.4.6.6 orders bands, columns, rows, then corners.
/// Header rows and the first logical column are excluded from enabled band counts.
pub(super) fn conditions(
    position: Position<'_>,
    flags: u16,
    row_band: Option<u8>,
    column_band: Option<u8>,
) -> Result<Vec<u16>> {
    let Position {
        row,
        column,
        columns,
        headers,
        right_to_left,
        preceding_non_header_rows,
    } = position;
    if columns == 0 || column >= columns || row >= headers.len() || flags & 0xf800 != 0 {
        return Err(Error::Invalid("conditional table position or options"));
    }
    if [row_band, column_band]
        .into_iter()
        .flatten()
        .any(|n| !(1..=3).contains(&n))
    {
        return Err(Error::Invalid("conditional table band width"));
    }
    let logical_column = if right_to_left {
        columns - 1 - column
    } else {
        column
    };
    let first_row = flags & 0x20 != 0 && (row == 0 || headers[row]);
    let last_row = flags & 0x40 != 0 && row + 1 == headers.len() && !first_row;
    let first_column = flags & 0x80 != 0 && logical_column == 0;
    let last_column = flags & 0x100 != 0 && logical_column + 1 == columns && !first_column;
    let mut result = Vec::with_capacity(5);
    if !first_row
        && !last_row
        && flags & 0x200 == 0
        && let Some(width) = row_band
    {
        let index = if flags & 0x20 == 0 {
            row
        } else {
            preceding_non_header_rows
        };
        result.push(if index / usize::from(width) % 2 == 0 {
            0x40
        } else {
            0x80
        });
    }
    if !first_column
        && !last_column
        && flags & 0x400 == 0
        && let Some(width) = column_band
    {
        let index = logical_column - usize::from(flags & 0x80 != 0);
        result.push(if index / usize::from(width) % 2 == 0 {
            0x10
        } else {
            0x20
        });
    }
    if first_column {
        result.push(4);
    } else if last_column {
        result.push(8);
    }
    if first_row {
        result.push(1);
    } else if last_row {
        result.push(2);
    }
    let top_left = row == 0 && first_column && flags & 0x20 != 0;
    let top_right = row == 0 && last_column && flags & 0x20 != 0 && !top_left;
    let bottom_left =
        row + 1 == headers.len() && first_column && flags & 0x40 != 0 && !top_left && !top_right;
    let bottom_right = row + 1 == headers.len()
        && last_column
        && flags & 0x40 != 0
        && !top_left
        && !top_right
        && !bottom_left;
    if top_left {
        result.push(0x200);
    } else if top_right {
        result.push(0x100);
    } else if bottom_left {
        result.push(0x800);
    } else if bottom_right {
        result.push(0x400);
    }
    Ok(result)
}
