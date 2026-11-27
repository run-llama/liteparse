//! Source relationships between body references and footnote/endnote text ranges.
use crate::office::doc::{
    Error, Inspection, Result, Story, bytes, compound::Compound, document::fib, u16_at, u32_at,
};
use serde::Serialize;

mod numbering;
mod ownership;
pub use ownership::{
    Footnote, FootnoteDocument, FootnoteMarkerStatus, FootnoteSection, FootnoteSeparator,
    NotePosition, inspect_note_ownership,
};
mod projection;
pub use numbering::{LegacyNumbering, NoteSection, NoteSections, Numbering, inspect_note_sections};
pub use projection::{NoteLabel, NoteLabels, apply_notes, resolve_note_labels};

const MAX_NOTES: usize = 65_536;

#[derive(Debug, Serialize)]
pub struct NoteInspection {
    pub status: &'static str,
    pub footnotes: Vec<NoteLink>,
    pub endnotes: Vec<NoteLink>,
    pub unresolved: [&'static str; 3],
}

#[derive(Debug, Serialize)]
pub struct NoteLink {
    pub body_cp: usize,
    pub note_start_cp: usize,
    pub note_end_cp: usize,
    pub automatic: bool,
    pub raw_index: u16,
}

/// Inspect FIB pairs 2/3 and 46/47 without changing the preserved source stories.
/// Ref PLCs contain two-byte automatic/custom indicators, not note-number values.
pub fn inspect_notes(data: &[u8], source: &Inspection) -> Result<NoteInspection> {
    let c = Compound::open(data)?;
    let word = c.stream("WordDocument")?;
    let (_, table_name, _, _, _) = fib(&word)?;
    let table = c.stream(table_name)?;
    let body = source
        .stories
        .iter()
        .find(|s| s.kind == "body")
        .ok_or(Error::Invalid("note body story"))?;
    let mut groups = Vec::new();
    let mut remaining = MAX_NOTES;
    for (kind, reference_pair, text_pair) in [("footnotes", 2, 3), ("endnotes", 46, 47)] {
        let note = source
            .stories
            .iter()
            .find(|s| s.kind == kind)
            .ok_or(Error::Invalid("note text story"))?;
        let refs = fib_range(&word, &table, reference_pair)?;
        let texts = fib_range(&word, &table, text_pair)?;
        let group = decode_links(body, note, refs, texts, &mut remaining)?;
        groups.push(group);
    }
    let endnotes = groups.pop().unwrap();
    let footnotes = groups.pop().unwrap();
    Ok(NoteInspection {
        status: "source_note_relationships",
        footnotes,
        endnotes,
        unresolved: [
            "section numbering properties and restart rules",
            "reference and label CFSpec/revision visibility",
            "note label projection",
        ],
    })
}

fn fib_range<'a>(word: &[u8], table: &'a [u8], pair: usize) -> Result<&'a [u8]> {
    let length = u32_at(word, 158 + pair * 8)? as usize;
    if length == 0 {
        return Ok(&[]);
    }
    bytes(table, u32_at(word, 154 + pair * 8)? as usize, length)
}

/// Pair PLC entries in source order, excluding each table's undefined terminal CP.
/// <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/3ed81d85-ea74-4790-8579-ab7f8eb651f7>
/// <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/b30472e7-569e-4c9c-a8ca-07fd5432f365>
fn decode_links(
    body: &Story,
    note: &Story,
    refs: &[u8],
    texts: &[u8],
    remaining: &mut usize,
) -> Result<Vec<NoteLink>> {
    if refs.is_empty() && texts.is_empty() {
        if note.utf16_units != 0 {
            return Err(Error::Invalid("missing note relationship tables"));
        }
        return Ok(Vec::new());
    }
    if refs.len() < 4 || !(refs.len() - 4).is_multiple_of(6) {
        return Err(Error::Invalid("note reference PLC size"));
    }
    let count = (refs.len() - 4) / 6;
    // A reference PLC may contain only its ignored terminal CP when no notes exist.
    if count == 0 && texts.is_empty() && note.utf16_units == 0 {
        return Ok(Vec::new());
    }
    *remaining = remaining
        .checked_sub(count)
        .ok_or(Error::Limit("note relationships"))?;
    if texts.len() != (count + 2) * 4 {
        return Err(Error::Invalid("note relationship count mismatch"));
    }
    if body.utf16_units > 16 * 1024 * 1024 || note.utf16_units > 16 * 1024 * 1024 {
        return Err(Error::Limit("note story units"));
    }
    let body_units: Vec<u16> = body.text.encode_utf16().collect();
    let note_units: Vec<u16> = note.text.encode_utf16().collect();
    if body_units.len() != body.utf16_units || note_units.len() != note.utf16_units {
        return Err(Error::Invalid("note story length"));
    }
    let terminal = u32_at(texts, count * 4)? as usize;
    if Some(terminal) != note.utf16_units.checked_sub(1) {
        return Err(Error::Invalid("note terminal text CP"));
    }
    body.start_cp
        .checked_add(body.utf16_units)
        .ok_or(Error::Invalid("note source CP overflow"))?;
    note.start_cp
        .checked_add(note.utf16_units)
        .ok_or(Error::Invalid("note source CP overflow"))?;
    let mut links = Vec::with_capacity(count);
    let mut previous_body = None;
    for index in 0..count {
        let body_cp = u32_at(refs, index * 4)? as usize;
        if body_cp >= body.utf16_units || previous_body.is_some_and(|cp| cp >= body_cp) {
            return Err(Error::Invalid("note reference CP"));
        }
        previous_body = Some(body_cp);
        let start = u32_at(texts, index * 4)? as usize;
        let end = u32_at(texts, (index + 1) * 4)? as usize;
        if start >= end || end >= note.utf16_units || note_units[end - 1] != 0x0d {
            return Err(Error::Invalid("note text range"));
        }
        let raw_index = u16_at(refs, (count + 1) * 4 + index * 2)?;
        if raw_index != 0 && body_units[body_cp] != 2 {
            return Err(Error::Invalid("automatic note reference marker"));
        }
        links.push(NoteLink {
            body_cp: body.start_cp + body_cp,
            note_start_cp: note.start_cp + start,
            note_end_cp: note.start_cp + end,
            automatic: raw_index != 0,
            raw_index,
        });
    }
    Ok(links)
}

#[cfg(test)]
mod tests;
