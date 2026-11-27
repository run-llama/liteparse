//! Bounded compatibility decoding for versionless, byte-string checkbox payloads.
//! Apache OpenOffice's Apache-2.0 WW8FormulaControl::FormulaRead documents the
//! observed legacy header and current-state interpretation. Unproven variants stay explicit.
use super::FormData;
use crate::office::doc::{Error, Result, bytes, u16_at};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct LegacyEncoding {
    pub format: &'static str,
    pub string_encoding: &'static str,
    pub payload_bytes: usize,
    pub raw_reserved_word: u16,
    pub checkbox_half_points: u16,
    pub interpretation_basis: &'static str,
}

pub(super) fn decode(payload: &[u8]) -> Result<FormData> {
    // These records begin with flags, not a version DWORD. Complete framing selects
    // this compatibility subset; unsupported flag/string combinations remain rejected.
    let bits = u16_at(payload, 0)?;
    if bits & 3 != 1 {
        return Err(Error::Unsupported("versionless form type"));
    }
    if bits & !0x007f != 0 {
        return Err(Error::Unsupported("versionless checkbox flags"));
    }
    let state = ((bits >> 2) & 31) as u8;
    if !matches!(state, 0 | 1 | 25) {
        return Err(Error::Unsupported("versionless checkbox current state"));
    }
    let max_length = u16_at(payload, 2)?;
    let size = u16_at(payload, 4)?;
    let reserved = u16_at(payload, 6)?;
    if max_length != 0 || !(2..=3168).contains(&size) || reserved != 0 {
        return Err(Error::Unsupported("versionless checkbox header"));
    }
    let mut offset = 8;
    let name = ascii_string(payload, &mut offset, 20)?;
    let default = u16_at(payload, offset)?;
    offset += 2;
    if default > 1 {
        return Err(Error::Invalid("versionless checkbox default"));
    }
    for limit in [64, 255, 138, 32, 32] {
        if !ascii_string(payload, &mut offset, limit)?.is_empty() {
            return Err(Error::Unsupported("versionless checkbox auxiliary string"));
        }
    }
    if offset != payload.len() {
        return Err(Error::Unsupported("versionless checkbox trailing bytes"));
    }
    Ok(FormData {
        legacy_encoding: Some(LegacyEncoding {
            format: "versionless_checkbox",
            string_encoding: "ascii_subset_of_source_byte_strings",
            payload_bytes: payload.len(),
            raw_reserved_word: reserved,
            checkbox_half_points: size,
            interpretation_basis: "source_framing_and_apache_openoffice_compatibility",
        }),
        kind: "checkbox",
        raw_bits: bits,
        raw_current_state: state,
        default_state: Some(default),
        effective_checked: Some(state == 1),
        selected_index: None,
        selected_text: None,
        name,
        default_text: None,
        text_format: String::new(),
        help_text: String::new(),
        status_text: String::new(),
        entry_macro: String::new(),
        exit_macro: String::new(),
        items: Vec::new(),
    })
}

fn ascii_string(data: &[u8], offset: &mut usize, limit: usize) -> Result<String> {
    let count = bytes(data, *offset, 1)?[0] as usize;
    *offset += 1;
    if count > limit {
        return Err(Error::Limit("versionless form string bytes"));
    }
    let text = bytes(data, *offset, count)?;
    *offset += count;
    if bytes(data, *offset, 1)? != [0] {
        return Err(Error::Invalid("versionless form string terminator"));
    }
    *offset += 1;
    if !text.iter().all(|c| (0x20..=0x7e).contains(c)) {
        return Err(Error::Unsupported("versionless form string code page"));
    }
    Ok(text.iter().map(|b| char::from(*b)).collect())
}

#[cfg(test)]
mod tests;
