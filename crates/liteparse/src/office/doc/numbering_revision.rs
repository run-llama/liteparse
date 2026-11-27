//! Historical numbering evidence, separate from current accepted-view list selection.
use crate::office::doc::{Error, Result, u16_at, u32_at};
use serde::Serialize;

/// MS-DOC §2.9.159 records the number saved when revision tracking began.
/// Its template is historical evidence; current labels come from resolved list properties.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct NumberingRevision {
    pub was_numbered: bool,
    /// Index into SttbfRMark; author identity remains unresolved without that table.
    pub author_index: u16,
    /// Raw DTTM bits, preserved without interpreting a calendar date.
    pub timestamp_raw: u32,
    /// One-based positions in the historical template; zero means that level is unused.
    pub placeholder_offsets: [u8; 9],
    pub number_formats: [u8; 9],
    pub number_values: [u32; 9],
    /// UTF-16 units can include placeholders and font-dependent symbol values.
    pub template_utf16: Vec<u16>,
    /// All 128 NumRM bytes, including ignored fields and unused template storage.
    pub raw_numrm: Vec<u8>,
}

/// A source-anchored snapshot for accepted-current-numbering projection.
/// Historical markup and rejected numbering require a separate paragraph-sequence view.
#[derive(Clone, Debug, Serialize)]
pub struct NumberingRevisionEvidence {
    pub paragraph_mark_cp: usize,
    pub numbering_view: &'static str,
    pub current_display_status: &'static str,
    pub historical_view: &'static str,
    pub number_inserted_since_revision: bool,
    pub snapshot: Option<NumberingRevision>,
}

impl NumberingRevisionEvidence {
    pub fn accepted_current(
        paragraph_mark_cp: usize,
        snapshot: Option<NumberingRevision>,
        number_inserted_since_revision: bool,
    ) -> Result<Self> {
        validate_flags(snapshot.as_ref(), number_inserted_since_revision)?;
        Ok(Self {
            paragraph_mark_cp,
            numbering_view: "accepted_current_numbering",
            current_display_status: "not_determined_by_revision_metadata",
            historical_view: "unresolved",
            number_inserted_since_revision,
            snapshot,
        })
    }
}

/// Decode the fixed-size C645 operand before allocating its bounded historical template.
/// Placeholder offsets refer to xst positions including its length unit at position zero.
/// MS-DOC does not require the unit at a placeholder position to equal its level index.
pub fn decode_numrm_operand(operand: &[u8]) -> Result<NumberingRevision> {
    if operand.len() != 129 || operand[0] != 128 {
        return Err(Error::Invalid("numbering revision operand length"));
    }
    let raw = &operand[1..];
    if raw[0] > 1 {
        return Err(Error::Invalid("numbering revision Boolean"));
    }
    let length = usize::from(u16_at(raw, 64)?);
    if length > 31 {
        return Err(Error::Invalid("numbering revision template length"));
    }
    let mut placeholder_offsets = [0u8; 9];
    placeholder_offsets.copy_from_slice(&raw[8..17]);
    if placeholder_offsets
        .iter()
        .any(|&offset| usize::from(offset) > length)
    {
        return Err(Error::Invalid("numbering revision placeholder offset"));
    }
    let mut number_values = [0; 9];
    for (index, value) in number_values.iter_mut().enumerate() {
        *value = u32_at(raw, 28 + index * 4)?;
    }
    Ok(NumberingRevision {
        was_numbered: raw[0] != 0,
        author_index: u16_at(raw, 2)?,
        timestamp_raw: u32_at(raw, 4)?,
        placeholder_offsets,
        number_formats: {
            let mut formats = [0u8; 9];
            formats.copy_from_slice(&raw[17..26]);
            formats
        },
        number_values,
        template_utf16: raw[66..66 + length * 2]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|unit| u16::from_le_bytes([unit[0], unit[1]]))
            .collect(),
        raw_numrm: raw.to_vec(),
    })
}

/// PFNumRMIns records that numbering was added after the previous revision.
/// It does not select a current list or determine whether the paragraph is visible.
pub fn decode_numbering_inserted(operand: &[u8]) -> Result<bool> {
    match operand {
        [0] => Ok(false),
        [1] => Ok(true),
        _ => Err(Error::Invalid("numbering insertion revision Boolean")),
    }
}

/// Validate the final ordered property state, after later modifiers have replaced earlier ones.
/// Word's binary specification permits fNumRMIns only when the old paragraph was unnumbered.
pub fn validate_flags(snapshot: Option<&NumberingRevision>, inserted: bool) -> Result<()> {
    if inserted && snapshot.is_some_and(|old| old.was_numbered) {
        return Err(Error::Unsupported("inconsistent numbering revision flags"));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
