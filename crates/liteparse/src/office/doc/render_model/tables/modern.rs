//! Modern table rows constructed through ordered insertion and width properties.
use super::{
    Border, BorderKind, Height, RowLayout, bool8,
    grid::RowGrid,
    signed,
    style_defaults::{self, CellDefaults},
};
use crate::office::doc::{Error, Result, formatting::PropertyModifier};
mod paint;

pub fn decode(
    version: u16,
    properties: &[PropertyModifier],
    styles: &[CellDefaults],
) -> Result<RowLayout> {
    if !matches!(version, 0x101 | 0x10c | 0x112) {
        return Err(Error::Unsupported("render inserted table binary version"));
    }
    if properties.len() > 262144 || styles.len() > 64 {
        return Err(Error::Limit("render modern table properties"));
    }
    let mut style_budget = 8 * 1024 * 1024usize;
    for style in styles {
        style_budget = style_budget
            .checked_sub(std::mem::size_of::<CellDefaults>())
            .ok_or(Error::Limit("render row style evidence"))?;
        for (_, property) in &style.applied {
            style_budget = style_budget
                .checked_sub(property.operand.len() + std::mem::size_of::<PropertyModifier>())
                .ok_or(Error::Limit("render row style evidence"))?;
        }
        style_defaults::validate(style)?;
    }
    let find = |index: usize| -> Result<&CellDefaults> {
        let mut matches = styles.iter().filter(|s| s.selected_style == index);
        let value = matches
            .next()
            .ok_or(Error::Unsupported("render inserted table style defaults"))?;
        if matches.next().is_some() {
            return Err(Error::Invalid("render duplicate selected table style"));
        }
        Ok(value)
    };
    let mut selected = find(11)?;
    let mut insertion_defaults = selected.cell.clone();
    let mut grid = RowGrid::new(0);
    let mut row = RowLayout {
        borders: selected.row_borders.clone(),
        ..RowLayout::default()
    };
    let mut left = 0i16;
    let mut gap = 0i16;
    let mut preferred = None;
    let mut unresolved_cell_widths = std::collections::BTreeSet::new();
    let mut budget = 8 * 1024 * 1024usize;
    for p in properties {
        budget = budget
            .checked_sub(p.operand.len() + 1)
            .ok_or(Error::Limit("render modern row work"))?;
        let b = &p.operand;
        match p.sprm {
            0x9601 => {
                left = signed(b)?;
                grid.move_origin(
                    left.checked_sub(gap)
                        .ok_or(Error::Invalid("render table origin"))?,
                )?;
            }
            0x9602 => {
                gap = signed(b)?;
                if gap < 0 {
                    return Err(Error::Invalid("render table half gap"));
                }
                row.gap_half_twips = gap as u16;
                grid.move_origin(
                    left.checked_sub(gap)
                        .ok_or(Error::Invalid("render table origin"))?,
                )?;
            }
            0x740a => table_look(b)?,
            0x7621 => {
                if !unresolved_cell_widths.is_empty() {
                    return Err(Error::Unsupported(
                        "render insertion after unresolved style widths",
                    ));
                }
                grid.insert(b, Some(&insertion_defaults))?;
            }
            0x7623 => grid.resize(b)?,
            0xd635 => {
                if let [5, first, limit, 0, _, _] = b.as_slice() {
                    let range = grid.range(*first, *limit)?;
                    for index in range {
                        if grid.cells()[index].preferred_width.is_some() {
                            unresolved_cell_widths.insert(index);
                        }
                    }
                } else {
                    grid.prefer_width(b)?;
                    for index in usize::from(b[1])..usize::from(b[2]) {
                        unresolved_cell_widths.remove(&index);
                    }
                }
            }
            // These compatibility arrays are ignored by modern table-style readers.
            0xd609 => paint::legacy_shading(b, grid.cells().len(), 2)?,
            0xd612 => paint::legacy_shading(b, grid.cells().len(), 10)?,
            0xd62b => {
                let mut cells = grid.into_cells();
                super::merges::apply(&mut cells, b)?;
                grid = RowGrid::from_cells(cells)?;
            }
            0x7629 => {
                let mut cells = grid.into_cells();
                super::text_direction::apply(&mut cells, b)?;
                grid = RowGrid::from_cells(cells)?;
            }
            0xd62c => grid.align(b)?,
            0xd62f => grid = paint::cell_borders(grid, b)?,
            0xd670 => grid = paint::cell_shading(grid, b)?,
            0xd639 => {
                let [3, first, limit, value] = *b.as_slice() else {
                    return Err(Error::Invalid("render cell no-wrap operand"));
                };
                let active = bool8(&[value])?;
                let range = grid.range(first, limit)?;
                let mut cells = grid.into_cells();
                for cell in &mut cells[range] {
                    cell.no_wrap = active;
                }
                grid = RowGrid::from_cells(cells)?;
            }
            0x563a => {
                let index = usize::from(crate::office::doc::u16_at(b, 0)?);
                if b.len() != 2 {
                    return Err(Error::Invalid("render table style operand"));
                }
                selected = find(index)?;
                // A later explicit width resolves the selected style's width interaction.
                // Until then, retain uncertainty for every previously assigned cell.
                unresolved_cell_widths.extend(
                    grid.cells()
                        .iter()
                        .enumerate()
                        .filter(|(_, cell)| cell.preferred_width.is_some())
                        .map(|(index, _)| index),
                );
                // TIstd preserves the physical grid and its listed row exceptions.
                // Other supported row state comes from the complete selected style.
                left = 0;
                row.borders = selected.row_borders.clone();
                row.physical_alignment = None;
                row.preferred_indent_twips = None;
                row.cant_split = false;
                row.header = false;
                insertion_defaults = selected.cell.clone();
                let mut cells = grid.into_cells();
                for c in &mut cells {
                    c.margins_twips = selected.cell.margins_twips;
                    c.borders = selected.cell.borders.clone();
                    c.vertical_alignment = selected.cell.vertical_alignment;
                    c.raw_flags = (c.raw_flags & !0x1c) | (selected.cell.raw_flags & 0x1c);
                    c.no_wrap = selected.cell.no_wrap;
                    c.shading_rgb = selected.cell.shading_rgb;
                }
                grid = RowGrid::from_cells(cells)?;
                grid.move_origin(
                    left.checked_sub(gap)
                        .ok_or(Error::Invalid("render table origin"))?,
                )?;
            }
            0xd634 => {
                if matches!(b.as_slice(), [6, 0, 1, _, 0, _, _]) {
                    nil_padding(b, &selected.cell, &insertion_defaults, grid.cells())?;
                } else {
                    style_defaults::padding(b, &mut insertion_defaults.margins_twips)?;
                    let mut cells = grid.into_cells();
                    for cell in &mut cells {
                        style_defaults::padding(b, &mut cell.margins_twips)?;
                    }
                    grid = RowGrid::from_cells(cells)?;
                    grid.move_origin(
                        left.checked_sub(gap)
                            .ok_or(Error::Invalid("render table origin"))?,
                    )?;
                }
            }
            0xd613 => {
                if b.len() != 49 || b[0] != 48 {
                    return Err(Error::Invalid("render modern row borders"));
                }
                for (target, bytes) in row.borders.iter_mut().zip(b[1..].as_chunks::<8>().0) {
                    *target = Some(border(bytes)?);
                }
            }
            0x7479 if b.len() == 4 => {}
            0x5400 => {
                let x = signed(b)?;
                if !(0..=2).contains(&x) {
                    return Err(Error::Invalid("render table alignment"));
                }
                row.physical_alignment = Some(x as u16);
            }
            0x9407 => {
                let x = signed(b)?;
                row.height = match x {
                    0 => Height::Auto,
                    1.. => Height::AtLeast(x as u16),
                    _ => Height::Exact(x.unsigned_abs()),
                };
            }
            0x3403 => {
                bool8(b)?;
            }
            0x3466 => row.cant_split = bool8(b)?,
            0x3404 => row.header = bool8(b)?,
            0x3615 => row.autofit = bool8(b)?,
            0xf614 => preferred = table_width(b)?,
            0xf661 => row.preferred_indent_twips = preferred_indent(b)?,
            0xf617 | 0xf618 => {
                if table_width(b)?.is_some_and(|x| x != 0) {
                    return Err(Error::Unsupported("render extra table indent"));
                }
            }
            _ => return Err(Error::Unsupported("render modern table property")),
        }
        row.applied.push(p.clone());
    }
    if !unresolved_cell_widths.is_empty() {
        return Err(Error::Unsupported(
            "render style reset after preferred cell width",
        ));
    }
    row.cells = grid.into_cells();
    if row.cells.is_empty() {
        return Err(Error::Invalid("render inserted grid missing cells"));
    }
    if row
        .cells
        .iter()
        .any(|c| c.edges_twips[0] >= c.edges_twips[1])
    {
        return Err(Error::Unsupported("render nonpositive cell width"));
    }
    let actual = i32::from(row.cells.last().unwrap().edges_twips[1])
        - i32::from(row.cells[0].edges_twips[0]);
    if !row.autofit && preferred.is_some_and(|w| i32::from(w) != actual) {
        return Err(Error::Unsupported(
            "render preferred table width requires resizing",
        ));
    }
    if row.autofit {
        row.autofit_preferred_width = preferred.map(super::Width::Twips);
    }
    row.style_defaults = styles.to_vec();
    Ok(row)
}
pub(super) fn table_width(data: &[u8]) -> Result<Option<u16>> {
    let [unit, lo, hi] = *data else {
        return Err(Error::Invalid("render table preferred width operand"));
    };
    let value = u16::from_le_bytes([lo, hi]);
    match unit {
        0 => Ok(None),
        1 if value == 0 => Ok(None),
        3 if value <= 31680 => Ok(Some(value)),
        2 if value <= 5000 => Err(Error::Unsupported("render percentage table width")),
        _ => Err(Error::Invalid("render table preferred width value")),
    }
}

// Preferred indentation is signed and independent of the cached physical grid.
// Its upper edge is checked again against actual laid-out table widths.
pub(super) fn preferred_indent(bytes: &[u8]) -> Result<Option<i16>> {
    let [unit, lo, hi] = *bytes else {
        return Err(Error::Invalid("render preferred indent operand"));
    };
    let value = i16::from_le_bytes([lo, hi]);
    match unit {
        0 | 1 if value == 0 => Ok(None),
        3 if (-31560..=31680).contains(&value) => Ok(Some(value)),
        _ => Err(Error::Invalid("render preferred indent value")),
    }
}

// Nil leaves the size undefined; admission requires all possible fallback sizes
// to coincide, including source-selected style, ordinary defaults and current state.
fn nil_padding(
    bytes: &[u8],
    selected: &super::CellLayout,
    inserted: &super::CellLayout,
    cells: &[super::CellLayout],
) -> Result<()> {
    let [6, 0, 1, sides, 0, lo, hi] = *bytes else {
        return Err(Error::Invalid("render nil row padding operand"));
    };
    if sides & !15 != 0 || u16::from_le_bytes([lo, hi]) > 31680 {
        return Err(Error::Invalid("render nil row padding value"));
    }
    for (index, default) in [0, 108, 0, 108].into_iter().enumerate() {
        if sides & (1 << index) != 0
            && (selected.margins_twips[index] != default
                || inserted.margins_twips[index] != default
                || cells
                    .iter()
                    .any(|cell| cell.margins_twips[index] != default))
        {
            return Err(Error::Unsupported("render ambiguous nil row padding"));
        }
    }
    Ok(())
}

/// Decode supported Brc painting while preserving all eight source bytes.
/// Reserved bits do not change paint; spacing, frame and shadow remain unsupported.
/// <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/5d99e9e0-d7a4-488a-91fb-6c046277e076>
pub(super) fn border(bytes: &[u8]) -> Result<Border> {
    let data: [u8; 8] = bytes
        .try_into()
        .map_err(|_| Error::Invalid("render modern Brc size"))?;
    if !matches!(data[3], 0 | 255) {
        return Err(Error::Invalid("render modern border color"));
    }
    if data[6] & 0x7f != 0 {
        return Err(Error::Unsupported("render border spacing or effects"));
    }
    let kind = match data[5] {
        0 => BorderKind::None,
        1 => BorderKind::Single,
        3 => BorderKind::Double,
        10 => BorderKind::Triple,
        _ => return Err(Error::Unsupported("render modern border type")),
    };
    Ok(Border {
        raw: u64::from_le_bytes(data),
        kind,
        eighth_points: if matches!(
            kind,
            BorderKind::Single | BorderKind::Double | BorderKind::Triple
        ) {
            data[4].max(2)
        } else {
            0
        },
        color_rgb: (data[3] == 0)
            .then_some((u32::from(data[0]) << 16) | (u32::from(data[1]) << 8) | u32::from(data[2])),
    })
}

/// TLP records historical auto-format choices and optional conditional regions.
/// Selected table styles are already qualified without conditional groups.
/// <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/d5bbfcc0-d44c-4e5f-bee4-903728da7787>
pub(super) fn table_look(bytes: &[u8]) -> Result<()> {
    let [_, _, low, high] = *bytes else {
        return Err(Error::Invalid("render table look operand"));
    };
    let flags = u16::from_le_bytes([low, high]);
    if flags & 0xf800 != 0 {
        return Err(Error::Invalid("render table look reserved bits"));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
