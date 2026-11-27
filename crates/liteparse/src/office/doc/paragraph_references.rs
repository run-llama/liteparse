//! Resolve paragraph property indirections while preserving original CP and style ownership.
use crate::office::doc::{
    Error, Result, bytes,
    compound::Compound,
    paragraph_formatting::{ParagraphInspection, ParagraphSpan},
    u16_at, u32_at,
};

const MAX_BYTES: usize = 8 * 1024 * 1024;
const MAX_SPANS: usize = 262_144;
const MAX_CHAIN: usize = 64;

/// Expand Data/PrcData references only; property application and style inheritance remain unresolved.
/// MS-DOC §2.6.2 defines replacement tails; the PrcData structure defines bounded records.
/// The original inspection retains full PAPX provenance; this expansion covers authored stories.
/// PlcPcd may include one terminal CP outside those stories (MS-DOC §2.8.35).
/// <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/1caae71f-35c4-49d7-adf0-af5fc766331c>
pub fn inspect_paragraph_references(
    data: &[u8],
    source: &ParagraphInspection,
) -> Result<ParagraphInspection> {
    let container = Compound::open(data)?;
    let word = container.stream("WordDocument")?;
    let (version, _, lengths, _, _) = crate::office::doc::document::fib(&word)?;
    if version != source.binary_version {
        return Err(Error::Invalid("paragraph reference version alignment"));
    }
    let mut stream = None;
    resolve(Some(lengths.iter().sum()), source, |offset| {
        if stream.is_none() {
            stream = Some(container.stream("Data")?);
        }
        let data = stream
            .as_deref()
            .ok_or(Error::Invalid("paragraph Data stream"))?;
        let len = u16_at(data, offset)? as i16;
        if !(10..=0x3fa2).contains(&len) {
            return Err(Error::Invalid("paragraph PrcData length"));
        }
        Ok(bytes(
            data,
            offset
                .checked_add(2)
                .ok_or(Error::Invalid("paragraph Data offset"))?,
            len as usize,
        )?
        .to_vec())
    })
}

fn resolve(
    authored_end: Option<usize>,
    source: &ParagraphInspection,
    mut record: impl FnMut(usize) -> Result<Vec<u8>>,
) -> Result<ParagraphInspection> {
    if source.spans.len() > MAX_SPANS {
        return Err(Error::Limit("paragraph reference spans"));
    }
    let excluded_nonstory_cp_range = if let Some(end) = authored_end {
        let mut next = 0;
        for span in &source.spans {
            if span.start_cp != next || span.end_cp <= span.start_cp {
                return Err(Error::Invalid("paragraph authored span order"));
            }
            next = span.end_cp;
        }
        if next < end || next - end > 1 {
            return Err(Error::Invalid("paragraph authored span coverage"));
        }
        (next > end).then_some([end, next])
    } else {
        None
    };
    let mut budget = MAX_BYTES;
    let mut spans = Vec::with_capacity(source.spans.len());
    for span in &source.spans {
        if authored_end.is_some_and(|end| span.start_cp >= end) {
            continue;
        }
        let mut current = span.raw_modifiers.as_slice();
        let mut owned;
        let mut result = Vec::new();
        let mut visited = Vec::new();
        loop {
            let mut offset = 0;
            let mut replacement = None;
            while offset < current.len() {
                let start = offset;
                let sprm = u16_at(current, offset)?;
                offset += 2;
                let len = operand_length(sprm, &current[offset..])?;
                let operand = bytes(current, offset, len)?;
                offset += len;
                budget = budget
                    .checked_sub(offset - start)
                    .ok_or(Error::Limit("paragraph reference work"))?;
                if sprm == 0x6646 && start != 0 {
                    continue;
                }
                if matches!(sprm, 0x6646 | 0x646b) {
                    if sprm == 0x6646
                        && visited.is_empty()
                        && (span.style_index != Some(0) || offset != current.len())
                    {
                        return Err(Error::Invalid("huge paragraph style index"));
                    }
                    let target = u32_at(operand, 0)? as usize;
                    if visited.contains(&target) {
                        return Err(Error::Invalid("paragraph reference cycle"));
                    }
                    if visited.len() >= MAX_CHAIN {
                        return Err(Error::Limit("paragraph reference chain"));
                    }
                    visited.push(target);
                    replacement = Some(target);
                    break;
                }
                result.extend_from_slice(&current[start..offset]);
            }
            match replacement {
                Some(target) => {
                    owned = record(target)?;
                    if !(10..=0x3fa2).contains(&owned.len()) {
                        return Err(Error::Invalid("paragraph PrcData length"));
                    }
                    current = &owned;
                }
                None => break,
            }
        }
        spans.push(ParagraphSpan {
            start_cp: span.start_cp,
            end_cp: authored_end.map_or(span.end_cp, |end| end.min(span.end_cp)),
            style_index: span.style_index,
            raw_modifiers: result,
        });
    }
    Ok(ParagraphInspection {
        schema_version: source.schema_version,
        status: if source.status == "source_endpoint_selected_paragraph_properties" {
            "reference_expanded_endpoint_selected_paragraphs"
        } else {
            "reference_expanded_paragraph_fragments"
        },
        binary_version: source.binary_version,
        spans,
        excluded_nonstory_cp_range,
        unresolved: [
            "paragraph-end selection of PAPX and PCD properties",
            "stylesheet built-in identity and inheritance",
            "paragraph/table modifier semantics",
            "outline levels, numbering and revision-aware paragraph boundaries",
        ],
    })
}

/// Operand byte length includes variable-length prefixes; TDefTable uses a two-byte prefix.
pub(crate) fn operand_length(sprm: u16, operand: &[u8]) -> Result<usize> {
    Ok(match sprm >> 13 {
        0 | 1 => 1,
        2 | 4 | 5 => 2,
        3 => 4,
        7 => 3,
        6 if sprm == 0xd608 => {
            let cb = u16_at(operand, 0)? as usize;
            if cb == 0 {
                return Err(Error::Invalid("table definition operand length"));
            }
            cb + 1
        }
        6 => {
            let cb = bytes(operand, 0, 1)?[0] as usize;
            if sprm == 0xc615 && cb == 255 {
                return Err(Error::Unsupported("extended paragraph tabs"));
            }
            cb + 1
        }
        _ => unreachable!(),
    })
}

#[cfg(test)]
mod tests;
