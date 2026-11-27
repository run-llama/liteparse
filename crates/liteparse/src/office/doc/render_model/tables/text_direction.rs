//! Ordered TC textFlow and TTextFlow cell-range assignments.
use super::CellLayout;
use crate::office::doc::{Error, Result};

pub fn code(flags: u16) -> u16 {
    (flags >> 2) & 7
}

pub fn validate(flags: u16) -> Result<()> {
    match code(flags) {
        0 | 1 | 3 | 4 | 5 => Ok(()),
        _ => Err(Error::Invalid("table text flow enumeration")),
    }
}

/// Preserve valid source directions through ordered assignments, including ones
/// whose East Asian per-character orientation needs a separate paint adapter.
pub(super) fn apply(cells: &mut [CellLayout], operand: &[u8]) -> Result<()> {
    let [first, limit, low, high] = *operand else {
        return Err(Error::Invalid("table text flow operand"));
    };
    let direction = u16::from_le_bytes([low, high]);
    if direction > 7 {
        return Err(Error::Invalid("table text flow enumeration"));
    }
    validate(direction << 2)?;
    if first >= limit || usize::from(limit) > cells.len() {
        return Err(Error::Invalid("table text flow cell range"));
    }
    for cell in &mut cells[usize::from(first)..usize::from(limit)] {
        cell.raw_flags = (cell.raw_flags & !0x1c) | (direction << 2);
    }
    Ok(())
}

pub fn shared(flags: u16) -> Result<Option<&'static str>> {
    validate(flags)?;
    match code(flags) {
        0 => Ok(None),
        1 => Ok(Some("tbRl")),
        3 => Ok(Some("btLr")),
        _ => Err(Error::Unsupported("table upright East Asian text flow")),
    }
}

#[cfg(test)]
mod tests;
