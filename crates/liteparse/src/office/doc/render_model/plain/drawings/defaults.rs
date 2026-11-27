//! Retain document-wide drawing defaults and apply supported text insets.
use super::{Record, require, unsupported};
use crate::office::doc::{Result, u16_at, u32_at};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct DefaultEvidence {
    pub table_range: [usize; 2],
    pub record_type: u16,
    pub source_payload: Vec<u8>,
    pub applied_text_properties: Vec<(u16, u32)>,
    pub limitation: &'static str,
}

pub(super) fn decode(record: &Record<'_>) -> Result<Option<DefaultEvidence>> {
    require(record, record.kind, 3, None)?;
    let count = usize::from(record.instance);
    let prefix = count * 6;
    if record.data.len() < prefix {
        return Err(unsupported());
    }
    let mut expected = prefix;
    let mut prior = None;
    let mut values = Vec::with_capacity(count);
    let mut applied = Vec::new();
    for offset in (0..prefix).step_by(6) {
        let raw = u16_at(record.data, offset)?;
        let key = raw & 0x3fff;
        let value = u32_at(record.data, offset + 2)?;
        if prior.is_some_and(|old| old >= key) {
            return Err(unsupported());
        }
        prior = Some(key);
        if raw & 0x8000 != 0 {
            expected = expected
                .checked_add(value as usize)
                .ok_or_else(unsupported)?;
        }
        if raw == key && matches!(key, 0x81..=0x84) && value <= 0x0132_f540 {
            applied.push((key, value));
        }
        values.push((raw, value));
    }
    if expected != record.data.len() {
        return Err(unsupported());
    }
    if values.is_empty()
        || values == [(0xc186, 0), (0xc1c5, 0)]
        || values == [(0x4186, 0), (0x41c5, 0)]
    {
        return Ok(None);
    }
    Ok(Some(DefaultEvidence {
        table_range: [
            record.payload_offset - 8,
            record.payload_offset + record.data.len(),
        ],
        record_type: record.kind,
        source_payload: record.data.to_vec(),
        applied_text_properties: applied,
        limitation: "Supported textbox insets are inherited before local overrides; other document drawing defaults remain unapplied in this partial scene.",
    }))
}

#[cfg(test)]
mod tests;
