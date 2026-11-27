//! Physical-to-display ownership for source-qualified vertical table merges.
use super::RowLayout;
use crate::office::doc::{Error, Result};
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct DisplayOwner {
    pub row: usize,
    pub cell: usize,
}

/// Resolve consecutive vertical chains by exact cached-grid intervals.
/// Physical cells and their source text remain in the source table. A continuation
/// contributes geometry while its restart owns displayed content (MS-DOC 2.9.342).
pub fn vertical_owners(rows: &[RowLayout]) -> Result<Vec<Vec<DisplayOwner>>> {
    if rows.iter().any(|row| row.horizontal_merge_basis.is_some()) {
        if rows
            .iter()
            .flat_map(|row| &row.cells)
            .any(|cell| cell.raw_flags & 0x60 != 0)
        {
            return Err(Error::Unsupported(
                "mixed horizontal and vertical table ownership",
            ));
        }
        return rows
            .iter()
            .enumerate()
            .map(|(row, source)| {
                Ok(super::horizontal::project(source)?
                    .owners
                    .into_iter()
                    .map(|cell| DisplayOwner { row, cell })
                    .collect())
            })
            .collect();
    }
    let mut previous = BTreeMap::new();
    let mut result = Vec::with_capacity(rows.len());
    for (row_index, row) in rows.iter().enumerate() {
        let mut active = BTreeMap::new();
        let mut owners = Vec::with_capacity(row.cells.len());
        for (cell_index, cell) in row.cells.iter().enumerate() {
            if cell.raw_flags & 3 != 0 {
                return Err(Error::Unsupported("render horizontal merge ownership"));
            }
            if cell.edges_twips[0] >= cell.edges_twips[1] {
                return Err(Error::Invalid("render merge cell interval"));
            }
            let own = DisplayOwner {
                row: row_index,
                cell: cell_index,
            };
            let owner = match (cell.raw_flags >> 5) & 3 {
                0 => own,
                1 => *previous.get(&cell.edges_twips).ok_or(Error::Invalid(
                    "render orphan or mismatched vertical continuation",
                ))?,
                3 => own,
                _ => return Err(Error::Invalid("render vertical merge flag")),
            };
            if cell.raw_flags & 0x60 != 0 && active.insert(cell.edges_twips, owner).is_some() {
                return Err(Error::Invalid("render duplicate vertical merge interval"));
            }
            owners.push(owner);
        }
        previous = active;
        result.push(owners);
    }
    Ok(result)
}

/// Apply TVertMerge after the current grid has established the addressed cell.
pub(super) fn apply(cells: &mut [super::CellLayout], bytes: &[u8]) -> Result<()> {
    let [2, index, flag] = *bytes else {
        return Err(Error::Invalid("render vertical merge operand"));
    };
    if !matches!(flag, 0 | 1 | 3) {
        return Err(Error::Invalid("render vertical merge value"));
    }
    let cell = cells
        .get_mut(usize::from(index))
        .ok_or(Error::Invalid("render vertical merge cell"))?;
    cell.raw_flags = (cell.raw_flags & !0x60) | (u16::from(flag) << 5);
    Ok(())
}

#[cfg(test)]
mod tests;
