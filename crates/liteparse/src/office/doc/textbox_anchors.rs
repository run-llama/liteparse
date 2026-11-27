//! Source textbox ranges and shape anchors; reading order and layout remain unresolved.
use crate::office::doc::{
    Error, Inspection, Result, Story, bytes, compound::Compound, document::fib, u16_at, u32_at,
};
use serde::Serialize;
use std::collections::BTreeMap;

const MAX_RECORDS: usize = 65_536;
const MAX_STORY_UNITS: usize = 16 * 1024 * 1024;

#[derive(Debug, Serialize)]
pub struct TextboxInspection {
    pub status: &'static str,
    pub parts: Vec<TextboxPart>,
    pub unresolved: [&'static str; 5],
}

#[derive(Debug, Serialize)]
pub struct TextboxPart {
    pub text_story: &'static str,
    pub anchor_story: &'static str,
    pub shapes: Vec<ShapeAnchor>,
    /// Records retain PLC storage order, including reusable slots.
    pub records: Vec<TextboxRecord>,
}

#[derive(Debug, Serialize)]
pub struct ShapeAnchor {
    pub shape_id: u32,
    /// Absolute source UTF-16 position of the shape's 0x08 character.
    pub source_cp: usize,
    /// Source rectangle in twips; its origin and wrapping require the flags below.
    pub rectangle_twips: [i32; 4],
    pub raw_flags: u16,
}

#[derive(Debug, Serialize)]
pub struct TextboxRange {
    pub start_cp: usize,
    pub end_cp: usize,
}

#[derive(Debug, Serialize)]
pub struct TextboxRecord {
    pub record_index: usize,
    /// Relative PLC values are preserved even for the ignored terminal spare slot.
    pub raw_cp_range: [u32; 2],
    pub reusable: bool,
    /// Active ranges include their terminating paragraph mark.
    pub source: Option<TextboxRange>,
    pub shape_id: Option<u32>,
    pub chain_shape_count: Option<u32>,
    /// Index into this part's shapes; linked chains identify only their first shape.
    pub anchor_index: Option<usize>,
    pub anchor_status: &'static str,
}

/// Inspect body/header textbox PLCs independently, preserving source story ownership.
/// MS-DOC 2.3.6 and 2.3.7 link FTXBXS.lid to shape IDs in the corresponding PlcfSpa.
/// <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/f87b3560-2c23-4d10-9751-ff141d307308>
pub fn inspect_textbox_anchors(data: &[u8], source: &Inspection) -> Result<TextboxInspection> {
    let compound = Compound::open(data)?;
    let word = compound.stream("WordDocument")?;
    let (version, table_name, _, _, _) = fib(&word)?;
    if version != source.binary_version {
        return Err(Error::Invalid("textbox source version mismatch"));
    }
    let table = compound.stream(table_name)?;
    let mut remaining = MAX_RECORDS;
    let mut parts = Vec::with_capacity(2);
    for (text_kind, anchor_kind, text_pair, shape_pair) in [
        ("textboxes", "body", 56, 40),
        ("header_textboxes", "headers", 58, 41),
    ] {
        let story = find_story(source, text_kind)?;
        let owner = find_story(source, anchor_kind)?;
        parts.push(decode_part(
            story,
            owner,
            fib_range(&word, &table, text_pair)?,
            fib_range(&word, &table, shape_pair)?,
            &mut remaining,
        )?);
    }
    Ok(TextboxInspection {
        status: "source_textbox_anchor_metadata",
        parts,
        unresolved: [
            "anchor CFSpec and revision visibility",
            "OfficeArt shape hierarchy and text IDs",
            "linked shape chains and textbox break ranges",
            "page placement, wrapping and reading order",
            "header/footer section ownership",
        ],
    })
}

fn find_story<'a>(source: &'a Inspection, kind: &str) -> Result<&'a Story> {
    let mut matches = source.stories.iter().filter(|story| story.kind == kind);
    let story = matches.next().ok_or(Error::Invalid("textbox story"))?;
    if matches.next().is_some() {
        return Err(Error::Invalid("duplicate textbox story"));
    }
    Ok(story)
}

fn fib_range<'a>(word: &[u8], table: &'a [u8], pair: usize) -> Result<&'a [u8]> {
    let size = u32_at(word, 158 + pair * 8)? as usize;
    if size == 0 {
        return Ok(&[]);
    }
    bytes(table, u32_at(word, 154 + pair * 8)? as usize, size)
}

fn story_units(story: &Story) -> Result<Vec<u16>> {
    if story.utf16_units > MAX_STORY_UNITS {
        return Err(Error::Limit("textbox story units"));
    }
    story
        .start_cp
        .checked_add(story.utf16_units)
        .ok_or(Error::Invalid("textbox source CP overflow"))?;
    if story.text.encode_utf16().count() != story.utf16_units {
        return Err(Error::Invalid("textbox story length"));
    }
    Ok(story.text.encode_utf16().collect())
}

fn plc_count(plc: &[u8], record_size: usize, remaining: &mut usize) -> Result<usize> {
    if plc.is_empty() {
        return Ok(0);
    }
    if plc.len() < 4 || !(plc.len() - 4).is_multiple_of(record_size + 4) {
        return Err(Error::Invalid("textbox PLC size"));
    }
    let count = (plc.len() - 4) / (record_size + 4);
    *remaining = remaining
        .checked_sub(count)
        .ok_or(Error::Limit("textbox and shape records"))?;
    Ok(count)
}

fn decode_part(
    text: &Story,
    owner: &Story,
    textbox_plc: &[u8],
    shape_plc: &[u8],
    remaining: &mut usize,
) -> Result<TextboxPart> {
    if !matches!(
        (text.kind, owner.kind),
        ("textboxes", "body") | ("header_textboxes", "headers")
    ) {
        return Err(Error::Invalid("textbox owner story"));
    }
    let shape_count = plc_count(shape_plc, 26, remaining)?;
    let text_count = plc_count(textbox_plc, 22, remaining)?;
    if (text_count == 0) != (text.utf16_units == 0) {
        return Err(Error::Invalid("textbox PLC story mismatch"));
    }
    let owner_units = story_units(owner)?;
    let text_units = story_units(text)?;
    let shapes = decode_shapes(owner, &owner_units, shape_plc, shape_count)?;
    // Duplicate shape IDs are ambiguous, so no active range selects one by accident.
    let mut shape_indices = BTreeMap::new();
    for (index, shape) in shapes.iter().enumerate() {
        shape_indices
            .entry(shape.shape_id)
            .and_modify(|existing| *existing = None)
            .or_insert(Some(index));
    }
    let records = decode_textboxes(text, &text_units, textbox_plc, text_count, &shape_indices)?;
    Ok(TextboxPart {
        text_story: text.kind,
        anchor_story: owner.kind,
        shapes,
        records,
    })
}

/// PlcfSpa uses 26-byte records and a final CP that is outside the anchor sequence.
/// <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/e98fe93c-d57f-4d9b-a519-30e6acbdd414>
fn decode_shapes(
    owner: &Story,
    units: &[u16],
    plc: &[u8],
    count: usize,
) -> Result<Vec<ShapeAnchor>> {
    let mut shapes = Vec::with_capacity(count);
    let mut previous = None;
    for index in 0..count {
        let cp = u32_at(plc, index * 4)? as usize;
        if previous.is_some_and(|previous| cp <= previous) || units.get(cp) != Some(&8) {
            return Err(Error::Invalid("shape anchor character position"));
        }
        previous = Some(cp);
        let record = (count + 1) * 4 + index * 26;
        shapes.push(ShapeAnchor {
            shape_id: u32_at(plc, record)?,
            source_cp: owner.start_cp + cp,
            rectangle_twips: [
                u32_at(plc, record + 4)? as i32,
                u32_at(plc, record + 8)? as i32,
                u32_at(plc, record + 12)? as i32,
                u32_at(plc, record + 16)? as i32,
            ],
            raw_flags: u16_at(plc, record + 20)?,
        });
    }
    if let Some(previous) = previous
        && u32_at(plc, count * 4)? as usize <= previous
    {
        return Err(Error::Invalid("shape terminal CP"));
    }
    Ok(shapes)
}

/// The final FTXBXS is always a spare; its bounding text range is ignored.
/// Nonterminal spare records consume one source unit; active ranges end in 0x0D.
/// <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/d0a81fe8-f9e8-4130-9233-84732e3d9420>
fn decode_textboxes(
    story: &Story,
    units: &[u16],
    plc: &[u8],
    count: usize,
    shapes: &BTreeMap<u32, Option<usize>>,
) -> Result<Vec<TextboxRecord>> {
    let mut records = Vec::with_capacity(count);
    for index in 0..count {
        let start = u32_at(plc, index * 4)?;
        let end = u32_at(plc, (index + 1) * 4)?;
        let record = (count + 1) * 4 + index * 22;
        let flag = u16_at(plc, record + 8)?;
        let terminal = index + 1 == count;
        let reusable = terminal || flag != 0;
        let mut result = TextboxRecord {
            record_index: index,
            raw_cp_range: [start, end],
            reusable,
            source: None,
            shape_id: None,
            chain_shape_count: None,
            anchor_index: None,
            anchor_status: "reusable_slot",
        };
        if !terminal {
            if flag != 0 && flag & 1 == 0 {
                return Err(Error::Invalid("textbox reusable flag"));
            }
            if start >= end || end as usize > units.len() {
                return Err(Error::Invalid("textbox text range"));
            }
            let range = &units[start as usize..end as usize];
            if range.last() != Some(&13)
                || char::decode_utf16(range.iter().copied()).any(|unit| unit.is_err())
            {
                return Err(Error::Invalid("textbox paragraph boundary"));
            }
            if reusable {
                if end - start != 1 {
                    return Err(Error::Invalid("textbox spare range"));
                }
            } else {
                if end - start <= 1 {
                    return Err(Error::Invalid("empty active textbox range"));
                }
                let chain_count = u32_at(plc, record)?;
                if chain_count == 0 {
                    return Err(Error::Invalid("textbox chain count"));
                }
                if chain_count as usize > MAX_RECORDS {
                    return Err(Error::Limit("textbox chain shapes"));
                }
                let id = u32_at(plc, record + 14)?;
                result.source = Some(TextboxRange {
                    start_cp: story.start_cp + start as usize,
                    end_cp: story.start_cp + end as usize,
                });
                result.shape_id = Some(id);
                result.chain_shape_count = Some(chain_count);
                result.anchor_index = shapes.get(&id).copied().flatten();
                result.anchor_status = match shapes.get(&id) {
                    Some(Some(_)) if chain_count == 1 => "source_anchor_cf_spec_unresolved",
                    Some(Some(_)) => "first_chain_anchor_only",
                    Some(None) => "duplicate_shape_id",
                    None => "shape_hierarchy_unresolved",
                };
            }
        }
        records.push(result);
    }
    Ok(records)
}

#[cfg(test)]
mod tests;
