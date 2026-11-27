//! Source list definitions and overrides, before paragraph resolution or label generation.
use crate::office::doc::{Error, Result, bytes, compound::Compound, document::fib, u16_at, u32_at};
use serde::Serialize;
use std::collections::BTreeMap;

const MAX_LEVELS: usize = 65_536;
const MAX_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Serialize)]
pub struct ListDefinitions {
    pub schema_version: u32,
    pub status: &'static str,
    pub lists: Vec<ListDefinition>,
    pub overrides: Vec<ListOverride>,
    /// LVLs follow PlfLst but are excluded from its FIB byte length.
    pub appended_level_bytes: usize,
    pub unresolved: [&'static str; 4],
}

#[derive(Debug, Serialize)]
pub struct ListDefinition {
    pub lsid: u32,
    pub template_code: u32,
    pub style_links: [u16; 9],
    pub flags: u8,
    pub html_flags: u8,
    pub levels: Vec<ListLevel>,
}

#[derive(Debug, Serialize)]
pub struct ListLevel {
    pub start_at: i32,
    pub number_format: u8,
    pub flags: u8,
    pub placeholder_offsets: [u8; 9],
    pub following_character: u8,
    pub saved_indent_twips: i32,
    pub restart_limit: u8,
    pub html_flags: u8,
    pub raw_papx: Vec<u8>,
    pub raw_chpx: Vec<u8>,
    /// UTF-16 units include level placeholders and font-dependent symbol code points.
    pub number_text_utf16: Vec<u16>,
}

#[derive(Debug, Serialize)]
pub struct ListOverride {
    pub lsid: u32,
    pub list_index: usize,
    pub field_type: u8,
    pub html_flags: u8,
    pub first_paragraph_cp: Option<u32>,
    pub levels: Vec<LevelOverride>,
}

#[derive(Debug, Serialize)]
pub struct LevelOverride {
    pub level: u8,
    pub raw_flags: u32,
    pub raw_start_at: i32,
    /// Only active when fStartAt is set and fFormatting is clear.
    pub start_override: Option<i32>,
    pub format_override: Option<ListLevel>,
}

struct Budget {
    levels: usize,
    bytes: usize,
}
impl Budget {
    fn charge(&mut self, bytes: usize) -> Result<()> {
        self.bytes = self
            .bytes
            .checked_sub(bytes)
            .ok_or(Error::Limit("list metadata bytes"))?;
        Ok(())
    }
    fn level(&mut self) -> Result<()> {
        self.levels = self
            .levels
            .checked_sub(1)
            .ok_or(Error::Limit("list levels"))?;
        self.charge(std::mem::size_of::<ListLevel>())
    }
}

/// Read FibRgFcLcb97 pairs 73/74 and the bounded LVL array immediately after PlfLst.
/// <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/0c9df81f-98d0-454e-ad84-b612cd05b1a4>
pub fn inspect_lists(data: &[u8]) -> Result<ListDefinitions> {
    let container = Compound::open(data)?;
    let word = container.stream("WordDocument")?;
    let (_, table_name, _, _, _) = fib(&word)?;
    let table = container.stream(table_name)?;
    let list_length = u32_at(&word, 158 + 73 * 8)? as usize;
    let (headers, appended) = if list_length == 0 {
        (&[][..], &[][..])
    } else {
        let offset = u32_at(&word, 154 + 73 * 8)? as usize;
        let headers = bytes(&table, offset, list_length)?;
        (headers, &table[offset + list_length..])
    };
    let override_length = u32_at(&word, 158 + 74 * 8)? as usize;
    let overrides = if override_length == 0 {
        &[][..]
    } else {
        bytes(
            &table,
            u32_at(&word, 154 + 74 * 8)? as usize,
            override_length,
        )?
    };
    decode_lists(headers, appended, overrides)
}

/// Decode fixed headers, their appended levels, and parallel override data.
/// The appended slice may contain later Table-stream records; consumed bytes are reported.
/// <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/da06b036-d4e8-4dcf-8759-f157b26c5aaf>
pub fn decode_lists(
    headers: &[u8],
    appended: &[u8],
    override_bytes: &[u8],
) -> Result<ListDefinitions> {
    if headers.len() > MAX_BYTES || override_bytes.len() > MAX_BYTES {
        return Err(Error::Limit("list input bytes"));
    }
    let count = if headers.is_empty() {
        0
    } else {
        u16_at(headers, 0)? as usize
    };
    if count > 0x7fff || (!headers.is_empty() && headers.len() != 2 + 28 * count) {
        return Err(Error::Invalid("list header count"));
    }
    let mut budget = Budget {
        levels: MAX_LEVELS,
        bytes: MAX_BYTES,
    };
    budget.charge(count * std::mem::size_of::<ListDefinition>())?;
    let mut lists = Vec::with_capacity(count);
    let mut identities = BTreeMap::new();
    let mut offset = 0;
    for index in 0..count {
        let raw = bytes(headers, 2 + index * 28, 28)?;
        let id = u32_at(raw, 0)?;
        if id == u32::MAX || identities.insert(id, index).is_some() {
            return Err(Error::Invalid("list duplicate or reserved identifier"));
        }
        let mut links = [0; 9];
        for (i, link) in links.iter_mut().enumerate() {
            *link = u16_at(raw, 8 + i * 2)?;
            if *link > 0xfff {
                return Err(Error::Invalid("list style index"));
            }
        }
        let level_count = if raw[26] & 1 != 0 { 1 } else { 9 };
        let mut levels = Vec::with_capacity(level_count);
        for level in 0..level_count {
            levels.push(decode_level(appended, &mut offset, level, &mut budget)?);
        }
        lists.push(ListDefinition {
            lsid: id,
            template_code: u32_at(raw, 4)?,
            style_links: links,
            flags: raw[26],
            html_flags: raw[27],
            levels,
        });
    }
    let overrides = decode_overrides(override_bytes, &lists, &identities, &mut budget)?;
    Ok(ListDefinitions {
        schema_version: 1,
        status: "source_list_definitions_only",
        lists,
        overrides,
        appended_level_bytes: offset,
        unresolved: [
            "paragraph/style/PCD list selection",
            "revision-aware numbering and restart state",
            "font-dependent bullets and number formatting",
            "paragraph indentation and generated-label provenance",
        ],
    })
}

fn decode_level(
    data: &[u8],
    offset: &mut usize,
    level: usize,
    budget: &mut Budget,
) -> Result<ListLevel> {
    budget.level()?;
    let raw = bytes(data, *offset, 28)?;
    *offset += 28;
    let start_at = u32_at(raw, 0)? as i32;
    let nfc = raw[4];
    let has_sequence = ![0x17, 0xff].contains(&nfc);
    if [8, 9, 15, 19].contains(&nfc)
        || raw[5] & 3 == 3
        || raw[15] > 2
        || (has_sequence && !(0..=0x7fff).contains(&start_at))
        || (has_sequence && raw[5] & 8 != 0 && raw[26] as usize > level)
    {
        return Err(Error::Invalid("list level formatting"));
    }
    let papx = bytes(data, *offset, raw[25] as usize)?;
    *offset += papx.len();
    let chpx = bytes(data, *offset, raw[24] as usize)?;
    *offset += chpx.len();
    let count = u16_at(data, *offset)? as usize;
    *offset += 2;
    let text = bytes(data, *offset, count * 2)?;
    *offset += text.len();
    budget.charge(papx.len() + chpx.len() + text.len())?;
    let units: Vec<_> = text
        .as_chunks::<2>()
        .0
        .iter()
        .map(|p| u16::from_le_bytes([p[0], p[1]]))
        .collect();
    let mut positions = [0; 9];
    positions.copy_from_slice(&raw[6..15]);
    let mut previous = 0;
    let mut placeholder_count = 0;
    for position in positions.iter().copied().take_while(|p| *p != 0) {
        if position <= previous
            || position as usize > units.len()
            || units[position as usize - 1] as usize > level
        {
            return Err(Error::Invalid("list level placeholder"));
        }
        previous = position;
        placeholder_count += 1;
    }
    if placeholder_count > level + 1 || (nfc == 0x17 && (units.len() > 1 || placeholder_count != 0))
    {
        return Err(Error::Invalid("list level number text"));
    }
    // Preserve raw symbol codes and placeholder units; malformed Unicode is still diagnosed.
    if String::from_utf16(&units).is_err() {
        return Err(Error::Invalid("list level UTF-16"));
    }
    Ok(ListLevel {
        start_at,
        number_format: nfc,
        flags: raw[5],
        placeholder_offsets: positions,
        following_character: raw[15],
        saved_indent_twips: u32_at(raw, 16)? as i32,
        restart_limit: raw[26],
        html_flags: raw[27],
        raw_papx: papx.to_vec(),
        raw_chpx: chpx.to_vec(),
        number_text_utf16: units,
    })
}

/// LFOData follows the entire LFO header array, with variable LFOLVL records per entry.
/// <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/2555c6ec-604b-48fd-8484-a2fdbb95b42c>
fn decode_overrides(
    data: &[u8],
    lists: &[ListDefinition],
    identities: &BTreeMap<u32, usize>,
    budget: &mut Budget,
) -> Result<Vec<ListOverride>> {
    if data.is_empty() {
        return Ok(Vec::new());
    }
    let count = u32_at(data, 0)? as usize;
    if count > MAX_LEVELS {
        return Err(Error::Limit("list overrides"));
    }
    bytes(data, 4, count * 16)?;
    budget.charge(count * std::mem::size_of::<ListOverride>())?;
    let mut offset = 4 + count * 16;
    let mut output = Vec::with_capacity(count);
    for index in 0..count {
        let raw = bytes(data, 4 + index * 16, 16)?;
        let lsid = u32_at(raw, 0)?;
        let list_index = *identities
            .get(&lsid)
            .ok_or(Error::Invalid("list override missing definition"))?;
        let list = &lists[list_index];
        // Field-type lists (0xfc..0xfe) can disagree with the list's own flag; Word opens both.
        if !matches!(raw[13], 0 | 0xfc..=0xff) {
            return Err(Error::Invalid("list override field type"));
        }
        if raw[12] > 9 {
            return Err(Error::Invalid("list override header"));
        }
        let cp = u32_at(data, offset)?;
        offset += 4;
        let mut levels = Vec::new();
        let mut seen = 0u16;
        for _ in 0..raw[12] {
            budget.charge(std::mem::size_of::<LevelOverride>())?;
            let start = u32_at(data, offset)? as i32;
            let flags = u32_at(data, offset + 4)?;
            offset += 8;
            let level = (flags & 15) as u8;
            if seen & (1 << level) != 0 {
                return Err(Error::Invalid("list override duplicate level"));
            }
            seen |= 1 << level;
            // A simple list has one level; Word ignores overrides of the levels it lacks.
            let missing = level as usize >= list.levels.len();
            let start_override = if flags & 0x30 == 0x10 {
                if !(0..=0x7fff).contains(&start) {
                    return Err(Error::Invalid("list override start value"));
                }
                Some(start)
            } else {
                None
            };
            let format_override = if flags & 0x20 != 0 {
                Some(decode_level(data, &mut offset, level as usize, budget)?)
            } else {
                None
            };
            if missing {
                continue;
            }
            levels.push(LevelOverride {
                level,
                raw_flags: flags,
                raw_start_at: start,
                start_override,
                format_override,
            });
        }
        output.push(ListOverride {
            lsid,
            list_index,
            field_type: raw[13],
            html_flags: raw[14],
            first_paragraph_cp: (cp != u32::MAX).then_some(cp),
            levels,
        });
    }
    if offset != data.len() {
        return Err(Error::Invalid("list override trailing bytes"));
    }
    Ok(output)
}

#[cfg(test)]
mod tests;
