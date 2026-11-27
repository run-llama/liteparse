//! Source-owned table rows and cells; visual merging and table grouping remain separate.
use crate::office::doc::{Error, Result, bytes, u16_at};
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct CellDefinition {
    pub horizontal_merge: u8,
    pub vertical_merge: u8,
    pub raw_flags: u16,
    pub preferred_width_raw: i16,
    pub preferred_width_unit: u8,
    pub definition_source: &'static str,
}
#[derive(Clone, Debug, Serialize)]
pub struct TableDefinition {
    pub grid_twips: Vec<i16>,
    pub cells: Vec<CellDefinition>,
}

/// Decode the complete TDefTable operand, including its two-byte length prefix.
/// Missing TC80 records use defaults; excess complete records are ignored.
/// Zero columns are legal as an intermediate definition, but cannot complete a row.
/// <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/de06ec41-a0ac-4046-9096-cdfaa0091ad9>
pub fn decode_table_definition(data: &[u8]) -> Result<TableDefinition> {
    if usize::from(u16_at(data, 0)?) + 1 != data.len() {
        return Err(Error::Invalid("table definition operand size"));
    }
    let columns = bytes(data, 2, 1)?[0] as usize;
    if columns > 63 {
        return Err(Error::Invalid("table definition cell count"));
    }
    let end = 3 + 2 * (columns + 1);
    let edges = bytes(data, 3, end - 3)?;
    let grid_twips: Vec<_> = edges
        .as_chunks::<2>()
        .0
        .iter()
        .map(|x| i16::from_le_bytes([x[0], x[1]]))
        .collect();
    if grid_twips.windows(2).any(|w| w[0] > w[1]) {
        return Err(Error::Invalid("table definition edge order"));
    }
    let records = data
        .get(end..)
        .ok_or(Error::Invalid("table definition records"))?;
    if records.len() % 20 != 0 {
        return Err(Error::Invalid("table definition TC80 size"));
    }
    let mut cells = Vec::with_capacity(columns);
    for index in 0..columns {
        let (flags, width) = if let Some(record) = records.as_chunks::<20>().0.get(index) {
            (
                u16_at(record, 0)?,
                i16::from_le_bytes([record[2], record[3]]),
            )
        } else {
            (0, 0)
        };
        if !matches!((flags >> 2) & 7, 0 | 1 | 3 | 4 | 5)
            || ((flags >> 5) & 3) == 2
            || ((flags >> 7) & 3) == 3
            || ((flags >> 9) & 7) > 3
            || width < 0
        {
            return Err(Error::Invalid("table definition TC80 flags"));
        }
        cells.push(CellDefinition {
            horizontal_merge: (flags & 3) as u8,
            vertical_merge: ((flags >> 5) & 3) as u8,
            raw_flags: flags,
            preferred_width_raw: width,
            preferred_width_unit: ((flags >> 9) & 7) as u8,
            definition_source: "TDefTable_TC80",
        });
    }
    Ok(TableDefinition { grid_twips, cells })
}

mod ownership;
pub(crate) use ownership::{CellContent, TableStory};
pub use ownership::{TableOwnership, plan_tables};
#[cfg(test)]
mod tests;
