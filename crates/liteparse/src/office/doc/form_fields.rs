//! Current form values retain their Data-stream ownership and accepted-view eligibility.
mod decoder;
use crate::office::doc::{
    Error, Inspection, Result, Transcription,
    compound::Compound,
    field_inspection::FieldInspection,
    fields::{FieldCharacter, FieldRange},
    formatting_inspection::FormattingInspection,
    hidden_visibility::HiddenEvidence,
    visibility::{RevisionEvidence, RevisionSpan},
};
pub use decoder::FormData;
use serde::Serialize;

// Rendering may use an owned saved result after validating its inert FFData record.
pub(crate) fn decode_saved_data(data: &[u8], budget: &mut usize) -> Result<FormData> {
    let (value, length) = decoder::decode(data, 0, budget)?;
    if length != data.len() {
        return Err(Error::Invalid("form record extent"));
    }
    Ok(value)
}

const MAX_RECORDS: usize = 16_384;
const MAX_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Serialize)]
pub struct FormInspection {
    pub status: &'static str,
    pub fields: Vec<FormField>,
    pub unresolved: Vec<&'static str>,
}
#[derive(Debug, Serialize)]
pub struct FormIdentity {
    pub raw_instruction_utf16: Vec<u16>,
    pub cached_type: Option<u8>,
    pub end_flags: Option<u8>,
    pub basis: &'static str,
    /// Type identity is established independently of projection visibility and eligibility.
    pub qualified: bool,
}

#[derive(Debug, Serialize)]
pub struct FormField {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity: Option<FormIdentity>,
    pub story: &'static str,
    pub field_start_cp: usize,
    pub field_end_cp: usize,
    pub marker_cp: Option<usize>,
    pub data_offset: Option<usize>,
    pub data_bytes: Option<usize>,
    pub field_type: &'static str,
    pub data: Option<FormData>,
    pub eligible: bool,
    pub diagnostic: Option<String>,
}

/// Decode indexed form fields. Defaults and macro names remain inert source metadata.
/// CFData and CPicLocation identify NilPICFAndBinData inside the containing field.
/// <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/5830324c-4f03-462a-b1f7-45707cd4036e>
pub fn inspect_forms(
    data: &[u8],
    source: &Inspection,
    fields: &FieldInspection,
    formatting: &FormattingInspection,
    revisions: &RevisionEvidence,
    hidden: &HiddenEvidence,
) -> Result<FormInspection> {
    let container = Compound::open(data)?;
    let stream = container.stream("Data");
    inspect_stream(
        stream.as_deref().ok(),
        source,
        fields,
        formatting,
        revisions,
        hidden,
    )
}

fn inspect_stream(
    data: Option<&[u8]>,
    source: &Inspection,
    fields: &FieldInspection,
    formatting: &FormattingInspection,
    revisions: &RevisionEvidence,
    hidden: &HiddenEvidence,
) -> Result<FormInspection> {
    if fields.binary_version != source.binary_version {
        return Err(Error::Invalid("form source version"));
    }
    let mut result = FormInspection {
        status: "partial_source_form_values",
        fields: Vec::new(),
        unresolved: vec![
            "checkbox_normalized_glyph_visual_appearance",
            "form_macros_not_executed",
        ],
    };
    let mut budget = MAX_BYTES;
    for story in &source.stories {
        if story.utf16_units > 16 * 1024 * 1024 {
            return Err(Error::Limit("form story units"));
        }
        let units: Vec<u16> = story.text.encode_utf16().collect();
        if units.len() != story.utf16_units {
            return Err(Error::Invalid("form story units"));
        }
        let Some(table) = fields
            .stories
            .iter()
            .find(|s| s.kind == story.kind)
            .and_then(|s| s.table.as_ref())
            .filter(|t| t.topology_valid)
        else {
            continue;
        };
        if table.ranges.len() > 262_144 {
            return Err(Error::Limit("form field index records"));
        }
        let mut ordered: Vec<_> = table.ranges.iter().collect();
        ordered.sort_by_key(|f| f.source.start);
        for (index, field) in ordered.iter().enumerate() {
            let story_end = story
                .start_cp
                .checked_add(story.utf16_units)
                .ok_or(Error::Invalid("form story CP overflow"))?;
            if field.source.start < story.start_cp
                || field.source.end > story_end
                || field.source.start.checked_add(1) != Some(field.instruction.start)
                || field.instruction.end >= field.source.end
            {
                return Err(Error::Invalid("form indexed field ownership"));
            }
            let start = field
                .instruction
                .start
                .checked_sub(story.start_cp)
                .ok_or(Error::Invalid("form instruction start"))?;
            let end = field
                .instruction
                .end
                .checked_sub(story.start_cp)
                .ok_or(Error::Invalid("form instruction end"))?;
            let instruction = units
                .get(start..end)
                .ok_or(Error::Invalid("form instruction bounds"))?;
            let Some((kind, keyword)) = field_kind(instruction) else {
                continue;
            };
            if result.fields.len() >= MAX_RECORDS {
                return Err(Error::Limit("form records"));
            }
            let localized = keyword == "FORMCASECOCHER";
            let mut item = FormField {
                identity: (localized && instruction.len() <= 1024).then(|| FormIdentity {
                    raw_instruction_utf16: instruction.to_vec(),
                    cached_type: table
                        .events
                        .get(table.events.partition_point(|e| e.cp < field.source.start))
                        .filter(|e| e.cp == field.source.start && e.kind == FieldCharacter::Begin)
                        .map(|e| e.metadata),
                    end_flags: table
                        .events
                        .get(
                            table
                                .events
                                .partition_point(|e| e.cp < field.source.end - 1),
                        )
                        .filter(|e| e.cp == field.source.end - 1 && e.kind == FieldCharacter::End)
                        .map(|e| e.metadata),
                    basis: "exact_localized_token_index_and_typed_data_compatibility",
                    qualified: false,
                }),
                story: story.kind,
                field_start_cp: field.source.start,
                field_end_cp: field.source.end,
                marker_cp: None,
                data_offset: None,
                data_bytes: None,
                field_type: kind,
                data: None,
                eligible: false,
                diagnostic: None,
            };
            let decoded = (|| {
                if instruction.len() > 1024 {
                    return Err(Error::Limit("form instruction units"));
                }
                budget = budget
                    .checked_sub(instruction.len() * 2)
                    .ok_or(Error::Limit("form property bytes"))?;
                let markers: Vec<_> = instruction
                    .iter()
                    .enumerate()
                    .filter(|(_, u)| **u == 1)
                    .map(|(i, _)| field.instruction.start + i)
                    .collect();
                if markers.len() != 1 {
                    return Err(Error::Invalid("form binary marker count"));
                }
                let first = instruction
                    .iter()
                    .position(|u| !matches!(*u, 9 | 32))
                    .unwrap();
                if instruction[first + keyword.len()..]
                    .iter()
                    .any(|u| !matches!(*u, 1 | 9 | 32))
                {
                    return Err(Error::Unsupported("form instruction arguments"));
                }
                if let Some(identity) = &item.identity {
                    if identity.cached_type != Some(71) {
                        return Err(Error::Unsupported("localized form cached type"));
                    }
                    let flags = identity
                        .end_flags
                        .ok_or(Error::Invalid("localized form end flags"))?;
                    // Dirty/private flags describe cached results. This compatibility path
                    // requires absent results and independently owned typed binary form data.
                    if field.result.is_some() || flags & !0x34 != 0 {
                        return Err(Error::Unsupported("localized form result context"));
                    }
                }
                let cp = markers[0];
                item.marker_cp = Some(cp);
                let span = revision_at(revisions, cp)?;
                if span.properties.special != Some(true) || !span.unresolved.is_empty() {
                    return Err(Error::Unsupported("form marker properties"));
                }
                let direct = formatting
                    .character_spans
                    .get(span.source_span_index)
                    .filter(|s| s.start_cp <= cp && cp < s.end_cp)
                    .ok_or(Error::Invalid("form CHPX ownership"))?;
                let piece = formatting
                    .pieces
                    .get(span.piece_index)
                    .filter(|p| p.start_cp <= cp && cp < p.end_cp)
                    .ok_or(Error::Invalid("form piece ownership"))?;
                let modifiers = crate::office::doc::visibility::piece_modifiers(
                    piece.raw_prm,
                    &formatting.raw_property_groups,
                )?;
                let mut binary = false;
                for property in direct.properties.iter().chain(&modifiers) {
                    budget = budget
                        .checked_sub(property.operand.len() + 2)
                        .ok_or(Error::Limit("form property bytes"))?;
                    if property.sprm == 0x0806 {
                        binary = match property.operand.as_slice() {
                            [0] => false,
                            [1] => true,
                            _ => return Err(Error::Invalid("form CFData Boolean")),
                        };
                    }
                }
                if !binary {
                    return Err(Error::Unsupported("form marker is not binary data"));
                }
                let location = span
                    .evidence
                    .iter()
                    .rev()
                    .find(|p| p.sprm == 0x6a03)
                    .ok_or(Error::Unsupported("form data location"))?;
                let offset = crate::office::doc::u32_at(&location.operand, 0)? as usize;
                item.data_offset = Some(offset);
                let (decoded, length) = decoder::decode(
                    data.ok_or(Error::Invalid("missing form Data stream"))?,
                    offset,
                    &mut budget,
                )?;
                if decoded.kind != kind {
                    return Err(Error::Invalid("form instruction and data type"));
                }
                if let Some(identity) = &mut item.identity {
                    identity.qualified = true;
                }
                item.data_bytes = Some(length);
                item.data = Some(decoded);
                if field.parent_begin_cp.is_some()
                    || ordered
                        .get(index + 1)
                        .is_some_and(|f| f.source.start < field.source.end)
                    || index
                        .checked_sub(1)
                        .and_then(|i| ordered.get(i))
                        .is_some_and(|f| f.source.end > field.source.start)
                {
                    return Err(Error::Unsupported("nested form field"));
                }
                visible_field(field, revisions, hidden, &mut budget)?;
                item.eligible = true;
                Ok(())
            })();
            if let Err(error) = decoded {
                if matches!(error, Error::Limit(_)) {
                    return Err(error);
                }
                item.diagnostic = Some(error.to_string());
            }
            result.fields.push(item);
        }
    }
    Ok(result)
}

fn field_kind(units: &[u16]) -> Option<(&'static str, &'static str)> {
    let start = units.iter().position(|u| !matches!(*u, 9 | 32))?;
    for (name, kind) in [
        ("FORMTEXT", "text"),
        ("FORMCHECKBOX", "checkbox"),
        ("FORMCASECOCHER", "checkbox"),
        ("FORMDROPDOWN", "dropdown"),
    ] {
        if units.get(start..start + name.len()).is_some_and(|s| {
            s.iter()
                .zip(name.bytes())
                .all(|(u, b)| *u <= 127 && (*u as u8).eq_ignore_ascii_case(&b))
        }) && units
            .get(start + name.len())
            .is_none_or(|u| matches!(*u, 1 | 9 | 32))
        {
            return Some((kind, name));
        }
    }
    None
}
fn revision_at(revisions: &RevisionEvidence, cp: usize) -> Result<&RevisionSpan> {
    revisions
        .spans
        .get(revisions.spans.partition_point(|s| s.end_cp <= cp))
        .filter(|s| s.start_cp <= cp)
        .ok_or(Error::Invalid("form revision coverage"))
}
fn visible_field(
    field: &FieldRange,
    revisions: &RevisionEvidence,
    hidden: &HiddenEvidence,
    budget: &mut usize,
) -> Result<()> {
    if field.source.start >= field.source.end {
        return Err(Error::Invalid("form field range"));
    }
    let mut cp = field.source.start;
    while cp < field.source.end {
        *budget = budget
            .checked_sub(1)
            .ok_or(Error::Limit("form visibility work"))?;
        let s = revision_at(revisions, cp)?;
        if s.properties.deleted != Some(false) || !s.unresolved.is_empty() {
            return Err(Error::Unsupported("form accepted revision visibility"));
        }
        cp = s.end_cp.min(field.source.end);
    }
    for cp in [field.source.start, field.source.end - 1] {
        let s = revision_at(revisions, cp)?;
        if s.properties.special != Some(true) || s.properties.field_hidden != Some(false) {
            return Err(Error::Unsupported("form delimiter properties"));
        }
    }
    let mut cp = field.source.start;
    while cp < field.source.end {
        *budget = budget
            .checked_sub(1)
            .ok_or(Error::Limit("form visibility work"))?;
        let s = hidden
            .spans
            .get(hidden.spans.partition_point(|s| s.end_cp <= cp))
            .filter(|s| s.start_cp <= cp)
            .ok_or(Error::Invalid("form hidden coverage"))?;
        if s.property.hidden != Some(false) || !s.property.unresolved.is_empty() {
            return Err(Error::Unsupported("form hidden visibility"));
        }
        cp = s.end_cp.min(field.source.end);
    }
    Ok(())
}

pub(crate) mod projection;
pub use projection::apply_forms;
#[cfg(test)]
mod tests;
