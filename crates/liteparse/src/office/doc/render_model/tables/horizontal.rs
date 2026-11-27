//! Word 97 TC bit ownership, retaining every physical source cell.
use super::RowLayout;
use crate::office::doc::{Error, Result};

pub const WORD97_TC_BITS: &str = "Word97_TC_fFirstMerged_fMerged";

pub struct Projection {
    pub row: RowLayout,
    pub physical_cells: Vec<usize>,
    pub owners: Vec<usize>,
}

/// The legacy TC record uses bit 0 for the first cell and bit 1 for continuations.
/// Microsoft Word 97–2007 Binary File Format, Table Cell Descriptors (TC).
/// Modern TCGRF's numeric enum differs; the caller binds this policy to Word 97.
pub fn project(row: &RowLayout) -> Result<Projection> {
    let has_merge = row.cells.iter().any(|cell| cell.raw_flags & 3 != 0);
    if has_merge && row.horizontal_merge_basis != Some(WORD97_TC_BITS) {
        return Err(Error::Unsupported("horizontal merge source encoding"));
    }
    if has_merge && row.cells.iter().any(|cell| cell.raw_flags & 0x60 != 0) {
        return Err(Error::Unsupported("combined horizontal and vertical merge"));
    }
    let mut result = Projection {
        row: row.clone(),
        physical_cells: Vec::new(),
        owners: Vec::with_capacity(row.cells.len()),
    };
    result.row.cells.clear();
    result.row.horizontal_merge_basis = None;
    let mut index = 0;
    while index < row.cells.len() {
        let source = &row.cells[index];
        let flag = source.raw_flags & 3;
        if flag == 2 || flag == 3 {
            return Err(Error::Invalid(
                "orphan or ambiguous horizontal continuation",
            ));
        }
        let mut end = index + 1;
        if flag & 1 != 0 {
            while end < row.cells.len() && row.cells[end].raw_flags & 3 == 2 {
                if row.cells[end - 1].edges_twips[1] != row.cells[end].edges_twips[0] {
                    return Err(Error::Invalid("horizontal merge interval gap"));
                }
                end += 1;
            }
            if end == index + 1 {
                return Err(Error::Invalid("horizontal restart without continuation"));
            }
        }
        let mut cell = source.clone();
        cell.edges_twips[1] = row.cells[end - 1].edges_twips[1];
        cell.raw_flags &= !3;
        result.row.cells.push(cell);
        result.physical_cells.push(index);
        result
            .owners
            .extend(std::iter::repeat_n(index, end - index));
        index = end;
    }
    Ok(result)
}

#[cfg(test)]
mod tests;
