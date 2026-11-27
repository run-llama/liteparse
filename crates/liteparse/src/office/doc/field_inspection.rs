//! Per-story source field tables, independently diagnosed from transcription.
use crate::office::doc::{
    Error, Result, bytes, compound::Compound, document::fib, fields, inspect, u32_at,
};
use serde::Serialize;

const MAX_DOCUMENT_EVENTS: usize = 262_144;

#[derive(Debug, Serialize)]
pub struct FieldInspection {
    pub schema_version: u32,
    pub status: &'static str,
    pub binary_version: u16,
    pub stories: Vec<StoryFields>,
    pub unresolved: [&'static str; 3],
}

#[derive(Debug, Serialize)]
pub struct StoryFields {
    pub status: &'static str,
    pub kind: &'static str,
    pub start_cp: usize,
    pub utf16_units: usize,
    pub table_present: bool,
    pub table: Option<fields::FieldTable>,
    pub error: Option<String>,
}

/// Inspect field tables without allowing malformed metadata to suppress raw story text.
/// Container, FIB and raw-story failures return an error. Each table's failure is reported
/// independently; CFSpec, revision visibility and omitted field kinds remain unresolved.
pub fn inspect_fields(data: &[u8]) -> Result<FieldInspection> {
    let source = inspect(data)?;
    let container = Compound::open(data)?;
    let word = container.stream("WordDocument")?;
    let (version, table_name, _, _, _) = fib(&word)?;
    let table = container.stream(table_name)?;
    let mut remaining_events = MAX_DOCUMENT_EVENTS;
    let mut stories = Vec::with_capacity(7);
    for story in &source.stories {
        let Some(index) = fields::fib_pair_index(story.kind) else {
            continue;
        };
        let offset = u32_at(&word, 154 + index * 8)? as usize;
        let length = u32_at(&word, 158 + index * 8)? as usize;
        let decoded = decode_table(story, &table, offset, length, &mut remaining_events);
        let (table, error) = match decoded {
            Ok(value) => (Some(value), None),
            Err(error) => (None, Some(error.to_string())),
        };
        let status = match &table {
            None => "unavailable",
            Some(value) if !value.topology_valid => "invalid_topology",
            Some(_) if length == 0 => "absent",
            Some(_) => "indexed_cf_spec_unresolved",
        };
        stories.push(StoryFields {
            status,
            kind: story.kind,
            start_cp: story.start_cp,
            utf16_units: story.utf16_units,
            table_present: length != 0,
            table,
            error,
        });
    }
    Ok(FieldInspection {
        schema_version: 1,
        status: "source_field_metadata_only",
        binary_version: version,
        stories,
        unresolved: [
            "resolved CFSpec and revision visibility",
            "XE, TC, RD, TA and PRIVATE fields omitted from Plcfld",
            "cached field result freshness and display evaluation",
        ],
    })
}

fn decode_table(
    story: &crate::office::doc::Story,
    table: &[u8],
    offset: usize,
    length: usize,
    remaining_events: &mut usize,
) -> Result<fields::FieldTable> {
    // A zero length leaves the FIB offset undefined, including out-of-bounds values.
    let plc = if length == 0 {
        &[]
    } else {
        bytes(table, offset, length)?
    };
    if !plc.is_empty() && (plc.len() < 4 || (plc.len() - 4) % 6 != 0) {
        return Err(Error::Invalid("field PLC size"));
    }
    let count = plc.len().saturating_sub(4) / 6;
    // Reserve before parsing so repeated malformed tables also consume the work budget.
    *remaining_events = remaining_events
        .checked_sub(count)
        .ok_or(Error::Limit("document field events"))?;
    fields::decode(story, plc)
}

#[cfg(test)]
mod tests;
