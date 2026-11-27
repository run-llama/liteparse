//! Paragraph and table-row revision annotations for current-property projection.
use crate::office::doc::{Error, Result, u16_at, u32_at};
use serde::Serialize;

/// MS-DOC §§2.9.217–218 attach revision provenance to properties.
/// This annotation does not contain the old properties or establish text visibility.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PropertyRevision {
    pub revision_present: bool,
    /// Signed SttbfRMark index; resolving an author requires that separate string table.
    pub author_index: i16,
    pub timestamp_raw: u32,
    pub raw_proprmark: [u8; 7],
}

/// Final ordered annotation state at one physical paragraph endpoint.
/// Historical and markup views need property-history and visibility resolution separately.
#[derive(Clone, Debug, Serialize)]
pub struct PropertyRevisionEvidence {
    pub paragraph_mark_cp: usize,
    pub property_view: &'static str,
    pub current_display_status: &'static str,
    pub historical_view: &'static str,
    pub paragraph: Option<PropertyRevision>,
    pub table_row: Option<PropertyRevision>,
}

impl PropertyRevisionEvidence {
    pub fn accepted_current(
        paragraph_mark_cp: usize,
        paragraph: Option<PropertyRevision>,
        table_row: Option<PropertyRevision>,
    ) -> Self {
        Self {
            paragraph_mark_cp,
            property_view: "accepted_current_properties",
            current_display_status: "not_determined_by_revision_metadata",
            historical_view: "unresolved",
            paragraph,
            table_row,
        }
    }
}

/// Decode C66F/D667 without assigning calendar or author semantics to preserved raw fields.
pub fn decode_operand(operand: &[u8]) -> Result<PropertyRevision> {
    if operand.len() != 8 || operand[0] != 7 {
        return Err(Error::Invalid("property revision operand length"));
    }
    if operand[1] > 1 {
        return Err(Error::Invalid("property revision Boolean"));
    }
    Ok(PropertyRevision {
        revision_present: operand[1] == 1,
        author_index: u16_at(operand, 2)? as i16,
        timestamp_raw: u32_at(operand, 4)?,
        raw_proprmark: {
            let mut mark = [0u8; 7];
            mark.copy_from_slice(&operand[1..8]);
            mark
        },
    })
}

#[cfg(test)]
mod tests;
