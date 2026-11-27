//! Ordered physical grid mutations with separately retained preferred widths.
//! Callers supply source-qualified cell defaults; this state does not qualify text,
//! table styles, painting or autofit for rendering.
use super::{CellLayout, MAX_CELLS, Width};
use crate::office::doc::{Error, Result};

#[derive(Debug, Clone, PartialEq)]
pub struct RowGrid {
    origin: i16,
    cells: Vec<CellLayout>,
}
impl RowGrid {
    pub fn new(origin_twips: i16) -> Self {
        Self {
            origin: origin_twips,
            cells: Vec::new(),
        }
    }
    pub fn from_cells(cells: Vec<CellLayout>) -> Result<Self> {
        if cells.len() > MAX_CELLS
            || cells
                .windows(2)
                .any(|w| w[0].edges_twips[1] != w[1].edges_twips[0])
        {
            return Err(Error::Invalid("render existing grid"));
        }
        let origin = cells.first().map_or(0, |c| c.edges_twips[0]);
        let result = Self { origin, cells };
        let mut checked = result.cells.clone();
        Self::position(origin, &mut checked, &result.widths())?;
        Ok(result)
    }
    pub fn into_cells(self) -> Vec<CellLayout> {
        self.cells
    }
    pub fn move_origin(&mut self, origin: i16) -> Result<()> {
        let mut next = self.cells.clone();
        Self::position(origin, &mut next, &self.widths())?;
        self.origin = origin;
        self.cells = next;
        Ok(())
    }
    pub fn cells(&self) -> &[CellLayout] {
        &self.cells
    }

    /// Insert physical cells while retaining the defaults selected by the caller.
    /// <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/082d49fd-8f96-4fd9-ac6e-61bd7c00b90d>
    pub fn insert(&mut self, data: &[u8], defaults: Option<&CellLayout>) -> Result<()> {
        let [first, count, lo, hi] = *data else {
            return Err(Error::Invalid("render grid insertion operand"));
        };
        let defaults = defaults.ok_or(Error::Unsupported("render grid selected cell defaults"))?;
        let (first, count) = (usize::from(first), usize::from(count));
        if count == 0 || first > self.cells.len() || self.cells.len() + count > MAX_CELLS {
            return Err(Error::Invalid("render grid insertion range"));
        }
        let width = u16::from_le_bytes([lo, hi]);
        if width > 31680 {
            return Err(Error::Invalid("render grid insertion width"));
        }
        let mut next = self.cells.clone();
        let mut widths = self.widths();
        next.splice(first..first, std::iter::repeat_n(defaults.clone(), count));
        widths.splice(first..first, std::iter::repeat_n(i32::from(width), count));
        Self::position(self.origin, &mut next, &widths)?;
        self.cells = next;
        Ok(())
    }

    /// TDxaCol assigns physical widths, shifting all following cell edges.
    /// Preferred widths remain separate inputs to a later layout algorithm.
    pub fn resize(&mut self, data: &[u8]) -> Result<()> {
        let [first, limit, lo, hi] = *data else {
            return Err(Error::Invalid("render grid resize operand"));
        };
        let range = self.range(first, limit)?;
        let width = u16::from_le_bytes([lo, hi]);
        if width > 31680 {
            return Err(Error::Invalid("render grid column width"));
        }
        let mut widths = self.widths();
        for slot in &mut widths[range] {
            *slot = i32::from(width);
        }
        let mut next = self.cells.clone();
        Self::position(self.origin, &mut next, &widths)?;
        self.cells = next;
        Ok(())
    }

    /// Preserve the source preference without changing physical cell edges.
    /// <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/e0d4c0db-623d-4a6f-94d2-48e05f34f564>
    pub fn prefer_width(&mut self, data: &[u8]) -> Result<()> {
        let [5, first, limit, unit, lo, hi] = *data else {
            return Err(Error::Invalid("render grid preferred width operand"));
        };
        let range = self.range(first, limit)?;
        let value = u16::from_le_bytes([lo, hi]);
        let width = match unit {
            0 => return Err(Error::Unsupported("render undefined preferred cell width")),
            1 if value == 0 => {
                return Err(Error::Unsupported("render automatic preferred cell width"));
            }
            2 if value <= 5000 => Some(Width::PercentFiftieths(value)),
            3 if value <= 31680 => Some(Width::Twips(value)),
            _ => return Err(Error::Invalid("render grid preferred width value")),
        };
        for cell in &mut self.cells[range] {
            cell.preferred_width = width;
        }
        Ok(())
    }

    /// Apply vertical alignment only to existing physical cells.
    /// <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/89cad471-901c-4733-98b2-32befc34e467>
    pub fn align(&mut self, data: &[u8]) -> Result<()> {
        let [3, first, limit, value] = *data else {
            return Err(Error::Invalid("render cell vertical alignment operand"));
        };
        if value > 2 {
            return Err(Error::Invalid("render cell vertical alignment value"));
        }
        let range = self.range(first, limit)?;
        for cell in &mut self.cells[range] {
            cell.vertical_alignment = value;
        }
        Ok(())
    }

    pub(super) fn range(&self, first: u8, limit: u8) -> Result<std::ops::Range<usize>> {
        let (first, limit) = (usize::from(first), usize::from(limit));
        if first >= self.cells.len() || first > limit || limit > self.cells.len() {
            return Err(Error::Invalid("render grid existing cell range"));
        }
        Ok(first..limit)
    }
    fn widths(&self) -> Vec<i32> {
        self.cells
            .iter()
            .map(|c| i32::from(c.edges_twips[1]) - i32::from(c.edges_twips[0]))
            .collect()
    }
    fn position(origin: i16, cells: &mut [CellLayout], widths: &[i32]) -> Result<()> {
        let total: i32 = widths.iter().sum();
        if total > 31680 || widths.iter().any(|&w| w < 0) {
            return Err(Error::Invalid("render grid aggregate width"));
        }
        let mut edge = i32::from(origin);
        for (cell, width) in cells.iter_mut().zip(widths) {
            let end = edge + width;
            cell.edges_twips = [
                i16::try_from(edge).map_err(|_| Error::Invalid("render grid origin extent"))?,
                i16::try_from(end).map_err(|_| Error::Invalid("render grid edge extent"))?,
            ];
            edge = end;
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests;
