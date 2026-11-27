//! Ordered cell borders and clear-pattern background colors.
use super::{Border, BorderKind, Error, Result, RowGrid, border};

pub(super) fn cell_borders(grid: RowGrid, bytes: &[u8]) -> Result<RowGrid> {
    if bytes.len() != 12 || bytes[0] != 11 {
        return Err(Error::Invalid("render cell border operand"));
    }
    let range = grid.range(bytes[1], bytes[2])?;
    let sides = bytes[3];
    if sides & 0xc0 != 0 {
        return Err(Error::Invalid("render cell border side mask"));
    }
    if sides & 0x30 != 0 {
        return Err(Error::Unsupported("render diagonal cell border"));
    }
    let value = border_or_nil(&bytes[4..])?;
    let mut cells = grid.into_cells();
    for cell in &mut cells[range] {
        for (index, target) in cell.borders.iter_mut().enumerate() {
            if sides & (1 << index) != 0 {
                *target = Some(value.clone());
            }
        }
    }
    RowGrid::from_cells(cells)
}

fn border_or_nil(bytes: &[u8]) -> Result<Border> {
    let raw: [u8; 8] = bytes
        .try_into()
        .map_err(|_| Error::Invalid("render cell border size"))?;
    // NilBrc ignores the color bytes but requires the exact trailing sentinel.
    if raw[4..] == [255; 4] {
        return Ok(Border {
            raw: u64::from_le_bytes(raw),
            kind: BorderKind::Nil,
            eighth_points: 0,
            color_rgb: None,
        });
    }
    border(bytes)
}

pub(super) fn cell_shading(grid: RowGrid, bytes: &[u8]) -> Result<RowGrid> {
    let Some((&count, values)) = bytes.split_first() else {
        return Err(Error::Invalid("render row shading operand"));
    };
    if usize::from(count) != values.len()
        || count > 220
        || count % 10 != 0
        || usize::from(count) / 10 > grid.cells().len()
    {
        return Err(Error::Invalid("render row shading count"));
    }
    let mut cells = grid.into_cells();
    // Trailing unshaded cells are omitted from the complete first-22-cell array.
    // Selected styles currently admit no shading; table-wide paint stays guarded.
    for cell in cells.iter_mut().take(22) {
        cell.shading_rgb = None;
    }
    for (cell, value) in cells.iter_mut().zip(values.as_chunks::<10>().0) {
        if !matches!(value[3], 0 | 255) || !matches!(value[7], 0 | 255) {
            return Err(Error::Invalid("render shading COLORREF"));
        }
        if value[8..] != [0, 0] {
            return Err(Error::Unsupported("render patterned cell shading"));
        }
        cell.shading_rgb = (value[7] == 0).then_some(
            (u32::from(value[4]) << 16) | (u32::from(value[5]) << 8) | u32::from(value[6]),
        );
    }
    RowGrid::from_cells(cells)
}

pub(super) fn legacy_shading(bytes: &[u8], cells: usize, stride: usize) -> Result<()> {
    let Some((&count, values)) = bytes.split_first() else {
        return Err(Error::Invalid("render legacy shading operand"));
    };
    let count = usize::from(count);
    if count != values.len()
        || count % stride != 0
        || count / stride > cells
        || (stride == 10 && count > 220)
    {
        return Err(Error::Invalid("render legacy shading count"));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
