//! Named bookmark ranges and raw target text, independent of displayed transcription.
use crate::office::doc::{
    Error, Inspection, Result, bytes, compound::Compound, document::fib, u16_at, u32_at,
};
use serde::Serialize;
use std::collections::BTreeSet;

const MAX_BOOKMARKS: usize = 0x3ffb;
const MAX_METADATA: usize = 8 * 1024 * 1024;
const MAX_TARGET_UNITS: usize = 16 * 1024 * 1024;

#[derive(Debug, Serialize)]
pub struct Bookmarks {
    pub status: &'static str,
    pub entries: Vec<Bookmark>,
    pub ignored_terminal_cps: Option<[u32; 2]>,
}

#[derive(Debug, Serialize)]
pub struct Bookmark {
    pub name: String,
    pub hidden_name: bool,
    pub source_cp: [usize; 2],
    pub end_index: usize,
    pub raw_flags: u16,
    pub story: Option<&'static str>,
    pub raw_target_text: Option<String>,
    pub unresolved: Vec<&'static str>,
}

/// Resolves exact saved names to raw source targets without changing visible text.
/// Hidden names designate navigation entries, independently of text visibility.
/// MS-DOC §§2.8.10, 2.9.70, 2.9.279 and §3.3 define the three parallel tables.
/// <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/b5e6b16c-4c09-4eda-81bb-9757385dc9ba>
pub fn inspect_bookmarks(data: &[u8]) -> Result<Bookmarks> {
    let source = crate::office::doc::inspect(data)?;
    let compound = Compound::open(data)?;
    let word = compound.stream("WordDocument")?;
    let (_, table_name, _, _, _) = fib(&word)?;
    let table = compound.stream(table_name)?;
    let mut blocks = Vec::new();
    let mut remaining = MAX_METADATA;
    for index in [21, 22, 23] {
        let length = u32_at(&word, 158 + index * 8)? as usize;
        remaining = remaining
            .checked_sub(length)
            .ok_or(Error::Limit("bookmark metadata"))?;
        blocks.push(if length == 0 {
            &[][..]
        } else {
            bytes(&table, u32_at(&word, 154 + index * 8)? as usize, length)?
        });
    }
    decode(blocks[0], blocks[1], blocks[2], &source)
}

fn names(data: &[u8]) -> Result<Vec<String>> {
    if data.is_empty() {
        return Ok(Vec::new());
    }
    if u16_at(data, 0)? != 0xffff || u16_at(data, 4)? != 0 {
        return Err(Error::Unsupported("bookmark name encoding"));
    }
    let count = u16_at(data, 2)? as usize;
    if count > MAX_BOOKMARKS {
        return Err(Error::Limit("bookmark count"));
    }
    let mut at = 6;
    let mut seen = BTreeSet::new();
    let mut result = Vec::new();
    for _ in 0..count {
        let count = u16_at(data, at)? as usize;
        at += 2;
        if !(1..40).contains(&count) {
            return Err(Error::Invalid("bookmark name length"));
        }
        let raw = bytes(data, at, count * 2)?;
        at += count * 2;
        let units: Vec<_> = raw
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        let name =
            String::from_utf16(&units).map_err(|_| Error::Invalid("bookmark name UTF-16"))?;
        if !seen.insert(name.clone()) {
            return Err(Error::Invalid("duplicate bookmark name"));
        }
        result.push(name);
    }
    if at != data.len() {
        return Err(Error::Invalid("bookmark name trailing bytes"));
    }
    Ok(result)
}

fn decode(names_data: &[u8], starts: &[u8], ends: &[u8], source: &Inspection) -> Result<Bookmarks> {
    let names = names(names_data)?;
    let count = names.len();
    if count == 0 && starts.is_empty() && ends.is_empty() {
        return Ok(Bookmarks {
            status: "no_named_bookmarks",
            entries: vec![],
            ignored_terminal_cps: None,
        });
    }
    if starts.len() != 4 + count * 8 || ends.len() != 4 + count * 4 {
        return Err(Error::Invalid("bookmark parallel table lengths"));
    }
    for plc in [starts, ends] {
        for i in 1..count {
            if u32_at(plc, (i - 1) * 4)? > u32_at(plc, i * 4)? {
                return Err(Error::Invalid("bookmark CP order"));
            }
        }
    }
    let stories: Vec<_> = source
        .stories
        .iter()
        .map(|s| (s, s.text.encode_utf16().collect::<Vec<_>>()))
        .collect();
    let mut entries = Vec::new();
    let mut end_owners = BTreeSet::new();
    let mut budget = MAX_TARGET_UNITS;
    for (index, name) in names.into_iter().enumerate() {
        let record = (count + 1) * 4 + index * 4;
        let end_index = u16_at(starts, record)? as usize;
        if end_index >= count || !end_owners.insert(end_index) {
            return Err(Error::Invalid("bookmark end ownership"));
        }
        let start = u32_at(starts, index * 4)? as usize;
        let end = u32_at(ends, end_index * 4)? as usize;
        let flags = u16_at(starts, record + 2)?;
        let mut entry = Bookmark {
            hidden_name: name.starts_with('_'),
            name,
            source_cp: [start, end],
            end_index,
            raw_flags: flags,
            story: None,
            raw_target_text: None,
            unresolved: vec![],
        };
        let target = stories.iter().find(|(s, _)| {
            s.utf16_units > 0
                && start >= s.start_cp
                && end >= start
                && end <= s.start_cp + s.utf16_units
        });
        if let Some((story, units)) = target {
            entry.story = Some(story.kind);
            if flags & 0x8000 != 0 {
                entry
                    .unresolved
                    .push("column_selection_requires_table_projection");
            } else if [start - story.start_cp, end - story.start_cp]
                .iter()
                .any(|&cp| units.get(cp).is_some_and(|u| (0xdc00..=0xdfff).contains(u)))
            {
                entry.unresolved.push("bookmark_surrogate_boundary");
            } else {
                budget = budget
                    .checked_sub(end - start)
                    .ok_or(Error::Limit("bookmark target text"))?;
                let selected = &units[start - story.start_cp..end - story.start_cp];
                match String::from_utf16(selected) {
                    Ok(text) => entry.raw_target_text = Some(text),
                    Err(_) => entry.unresolved.push("bookmark_surrogate_boundary"),
                }
            }
        } else {
            entry.unresolved.push("bookmark_range_outside_single_story");
        }
        // Raw targets retain controls, field instructions and revision-hidden text.
        // Selection validity and visible-output navigation require separate projection.
        entry
            .unresolved
            .push("visible_target_projection_unimplemented");
        entries.push(entry);
    }
    Ok(Bookmarks {
        status: "named_raw_source_targets",
        entries,
        ignored_terminal_cps: Some([u32_at(starts, count * 4)?, u32_at(ends, count * 4)?]),
    })
}

#[cfg(test)]
mod tests;
