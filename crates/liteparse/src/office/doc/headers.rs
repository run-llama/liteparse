//! Source ownership of header/footer variants; page instances remain unresolved.
use crate::office::doc::formatting::section::outline_operand;
use crate::office::doc::{
    Error, Inspection, Result, bytes, compound::Compound, document::fib,
    formatting::PropertyModifier, u16_at, u32_at,
};
use serde::Serialize;

const MAX_SECTIONS: usize = 8192;
const MAX_SLOTS: usize = 6 + MAX_SECTIONS * 6;
const MAX_PROPERTY_BYTES: usize = 8 * 1024 * 1024;
const MAX_UNITS: usize = 16 * 1024 * 1024;
const ROLES: [&str; 6] = [
    "even_header",
    "odd_header",
    "even_footer",
    "odd_footer",
    "first_header",
    "first_footer",
];
const SEPARATORS: [&str; 6] = [
    "footnote_separator",
    "footnote_continuation_separator",
    "footnote_continuation_notice",
    "endnote_separator",
    "endnote_continuation_separator",
    "endnote_continuation_notice",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SourceRange {
    pub start_cp: usize,
    pub end_cp: usize,
}
#[derive(Debug, Serialize)]
pub struct HeaderStory {
    pub index: usize,
    pub role: Option<&'static str>,
    pub section_index: Option<usize>,
    pub source: SourceRange,
    pub content: Option<SourceRange>,
    pub guard: Option<SourceRange>,
    pub explicit_blank: bool,
    pub unresolved: Vec<&'static str>,
}
#[derive(Debug, Serialize)]
pub struct HeaderReference {
    pub role: &'static str,
    pub status: &'static str,
    pub local_story_index: Option<usize>,
    pub origin_story_index: Option<usize>,
    pub origin_section_index: Option<usize>,
}
#[derive(Debug, Serialize)]
pub struct HeaderSection {
    pub index: usize,
    pub plc_end_cp: usize,
    pub source: SourceRange,
    pub sepx_offset: Option<usize>,
    pub properties: Vec<PropertyModifier>,
    pub different_first_page: Option<bool>,
    pub legacy_presence_flags: Option<u8>,
    pub references: Vec<HeaderReference>,
    pub unresolved: Vec<&'static str>,
}
#[derive(Debug, Serialize)]
pub struct HeaderInspection {
    pub status: &'static str,
    pub binary_version: u16,
    pub source: SourceRange,
    pub different_even_and_odd_pages: Option<bool>,
    pub dop_first_byte: Option<u8>,
    pub stories: Vec<HeaderStory>,
    pub sections: Vec<HeaderSection>,
    pub terminal_sentinel: Option<SourceRange>,
    pub ignored_final_plc_cp: Option<u32>,
    pub diagnostics: Vec<&'static str>,
    pub unresolved: [&'static str; 3],
}

/// Reads PlcfHdd after validated FIB layout and source-story identity checks.
/// Header ranges contain a separate guard CR; empty slots inherit their role.
/// <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/8465bee7-6c79-45a9-812e-58b0c5fd6cdc>
/// <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/8f336b7e-66cb-4346-9fd4-88ede9a4a9db>
pub fn inspect_headers(data: &[u8], source: &Inspection) -> Result<HeaderInspection> {
    let compound = Compound::open(data)?;
    let word = compound.stream("WordDocument")?;
    let (version, table_name, lengths, _, _) = fib(&word)?;
    validate_source(version, &lengths, source)?;
    let table = compound.stream(table_name)?;
    let body: Vec<_> = source.stories[0].text.encode_utf16().collect();
    let header: Vec<_> = source.stories[2].text.encode_utf16().collect();
    let sections = section_records(&word, fib_range(&word, &table, 6)?, &body)?;
    let dop = fib_range(&word, &table, 31)?;
    decode(
        version,
        source.stories[2].start_cp,
        &header,
        fib_range(&word, &table, 11)?,
        sections,
        dop.first().copied(),
    )
}

fn validate_source(version: u16, lengths: &[usize; 8], source: &Inspection) -> Result<()> {
    let kinds = [
        "body",
        "footnotes",
        "headers",
        "reserved_macro",
        "comments",
        "endnotes",
        "textboxes",
        "header_textboxes",
    ];
    if source.binary_version != version || source.stories.len() != 8 {
        return Err(Error::Invalid("header source identity"));
    }
    let total_bytes = source
        .stories
        .iter()
        .try_fold(0usize, |n, s| n.checked_add(s.text.len()))
        .ok_or(Error::Limit("header source bytes"))?;
    if total_bytes > MAX_UNITS * 4 {
        return Err(Error::Limit("header source bytes"));
    }
    let mut cp = 0usize;
    for ((story, length), kind) in source.stories.iter().zip(lengths).zip(kinds) {
        cp = cp
            .checked_add(*length)
            .filter(|n| *n <= MAX_UNITS)
            .ok_or(Error::Limit("header source units"))?;
        if story.kind != kind
            || story.start_cp != cp - length
            || story.utf16_units != *length
            || story.text.encode_utf16().count() != *length
        {
            return Err(Error::Invalid("header source story identity"));
        }
    }
    Ok(())
}

fn fib_range<'a>(word: &[u8], table: &'a [u8], pair: usize) -> Result<&'a [u8]> {
    let length = u32_at(word, 158 + pair * 8)? as usize;
    if length == 0 {
        return Ok(&[]);
    }
    bytes(table, u32_at(word, 154 + pair * 8)? as usize, length)
}

/// PlcfSed terminal CP can extend beyond the body; section ownership stops at ccpText.
/// <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/68a959a6-11e0-4f3e-9a99-76ca8cc4dddc>
fn section_records(word: &[u8], plc: &[u8], body: &[u16]) -> Result<Vec<HeaderSection>> {
    if plc.len() < 20 || !(plc.len() - 4).is_multiple_of(16) {
        return Err(Error::Invalid("header section PLC"));
    }
    let count = (plc.len() - 4) / 16;
    if count > MAX_SECTIONS || body.len() > MAX_UNITS {
        return Err(Error::Limit("header sections"));
    }
    if u32_at(plc, 0)? != 0 {
        return Err(Error::Invalid("header initial section CP"));
    }
    let mut budget = MAX_PROPERTY_BYTES;
    let mut modifier_budget = 262_144usize;
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let start = u32_at(plc, i * 4)? as usize;
        let end = u32_at(plc, i * 4 + 4)? as usize;
        if start >= end
            || start >= body.len()
            || (i + 1 < count && (end >= body.len() || body[end - 1] != 12))
            || (i + 1 == count && end < body.len())
        {
            return Err(Error::Invalid("header section source range"));
        }
        let fc = u32_at(plc, (count + 1) * 4 + i * 12 + 2)? as usize;
        let mut section = HeaderSection {
            index: i,
            plc_end_cp: end,
            source: SourceRange {
                start_cp: start,
                end_cp: end.min(body.len()),
            },
            sepx_offset: None,
            properties: Vec::new(),
            different_first_page: None,
            legacy_presence_flags: None,
            references: Vec::new(),
            unresolved: Vec::new(),
        };
        if fc == u32::MAX as usize {
            section.unresolved.push("section_properties_absent");
        } else {
            section.sepx_offset = Some(fc);
            let length = u16_at(word, fc)? as usize;
            if length > 32767 {
                return Err(Error::Invalid("header SEPX size"));
            }
            budget = budget
                .checked_sub(length)
                .ok_or(Error::Limit("header section property bytes"))?;
            properties(bytes(word, fc + 2, length)?, &mut section)?;
            modifier_budget = modifier_budget
                .checked_sub(section.properties.len())
                .ok_or(Error::Limit("header section modifier count"))?;
        }
        out.push(section);
    }
    Ok(out)
}

/// Final property evaluation applies ordered modifiers across SWall; its old
/// property boundary remains in evidence. Page placement and revisions of the
/// section-break character are outside this source ownership model.
/// <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/46c3ec54-53ff-4c0a-b0d6-07ad15d2546e>
fn properties(raw: &[u8], section: &mut HeaderSection) -> Result<()> {
    let mut offset = 0;
    let mut title = false;
    let mut known = true;
    while offset < raw.len() {
        let sprm = u16_at(raw, offset)?;
        offset += 2;
        if (sprm >> 10) & 7 != 4 {
            return Err(Error::Invalid("header non-section modifier"));
        }
        let length = match sprm >> 13 {
            0 | 1 => 1,
            2 | 4 | 5 => 2,
            3 => 4,
            6 => 1 + bytes(raw, offset, 1)?[0] as usize,
            7 => 3,
            _ => unreachable!(),
        };
        let operand = bytes(raw, offset, length)?;
        match sprm {
            0xd1ff if crate::office::doc::formatting::section::ignored_d1ff(sprm, operand) => {}
            0x300a => {
                if operand[0] > 1 {
                    known = false;
                    if !section.unresolved.contains(&"invalid_title_page_boolean") {
                        section.unresolved.push("invalid_title_page_boolean");
                    }
                } else {
                    title = operand[0] == 1;
                }
            }
            // The legacy outline definition and its color extension do not modify
            // title-page selection. Keep unsupported payload forms unresolved.
            0xd202 | 0xd238 => {
                if !outline_operand(sprm, operand) {
                    known = false;
                    if !section
                        .unresolved
                        .contains(&"invalid_outline_section_operand")
                    {
                        section.unresolved.push("invalid_outline_section_operand");
                    }
                }
            }
            0x3014 => {
                section.legacy_presence_flags = Some(operand[0]);
            }
            0x3239 => {
                if operand[0] > 1 {
                    known = false;
                    if !section.unresolved.contains(&"invalid_section_wall_boolean") {
                        section.unresolved.push("invalid_section_wall_boolean");
                    }
                }
            }
            0x3000 | 0x3001 | 0xf203 | 0xf204 | 0x3005 | 0x3006 | 0x5007 | 0x5008 | 0x3009
            | 0x500b | 0x900c | 0x300e | 0x3011 | 0x3012 | 0x3013 | 0x5015 | 0x9016 | 0xb017
            | 0xb018 | 0x3019 | 0x301a | 0x501b | 0x501c | 0x301d | 0xb01f | 0xb020 | 0xb021
            | 0xb022 | 0x9023 | 0x9024 | 0xb025 | 0x5026 | 0x3228 | 0x322a | 0x702b | 0x702c
            | 0x702d | 0x702e | 0x522f | 0x7030 | 0x9031 | 0x5032 | 0x5033 | 0xd234 | 0xd235
            | 0xd236 | 0xd237 | 0x703a | 0x303b | 0x303c | 0x303e | 0x503f | 0x5040 | 0x5041
            | 0x5042 | 0xd243 | 0x7044 => {}
            _ => {
                known = false;
                if !section.unresolved.contains(&"unknown_section_modifier") {
                    section.unresolved.push("unknown_section_modifier");
                }
            }
        }
        section.properties.push(PropertyModifier {
            sprm,
            operand: operand.to_vec(),
        });
        offset += length;
    }
    section.different_first_page = known.then_some(title);
    Ok(())
}

fn decode(
    version: u16,
    origin: usize,
    units: &[u16],
    plc: &[u8],
    mut sections: Vec<HeaderSection>,
    dop: Option<u8>,
) -> Result<HeaderInspection> {
    if sections.len() > MAX_SECTIONS || units.len() > MAX_UNITS || plc.len() > (MAX_SLOTS + 2) * 4 {
        return Err(Error::Limit("header metadata"));
    }
    let end = origin
        .checked_add(units.len())
        .ok_or(Error::Invalid("header source overflow"))?;
    let mut output = HeaderInspection {
        status: "source_header_footer_ownership",
        binary_version: version,
        source: SourceRange {
            start_cp: origin,
            end_cp: end,
        },
        different_even_and_odd_pages: dop.map(|v| v & 1 != 0),
        dop_first_byte: dop,
        stories: Vec::new(),
        sections: Vec::new(),
        terminal_sentinel: None,
        ignored_final_plc_cp: None,
        diagnostics: Vec::new(),
        unresolved: [
            "page_instances_and_page_field_values",
            "visible_section_topology_and_revision_merges",
            "header_visibility_layout_and_linked_drawing_placement",
        ],
    };
    if dop.is_none() {
        output
            .diagnostics
            .push("document_header_selection_properties_absent");
    }
    if units.is_empty() {
        if !plc.is_empty() {
            output
                .diagnostics
                .push("stale_header_plc_without_header_story");
        }
        for section in &mut sections {
            section.references = empty_references("absent");
        }
        output.sections = sections;
        return Ok(output);
    }
    if plc.len() < 8 || !plc.len().is_multiple_of(4) {
        return Err(Error::Invalid("header PLC size"));
    }
    let cp_count = plc.len() / 4;
    let count = cp_count - 2;
    let expected = 6 + sections.len() * 6;
    let count_valid = count == expected;
    if !count_valid {
        output.diagnostics.push("header_slot_count_mismatch");
    }
    if u32_at(plc, 0)? != 0 || u32_at(plc, (cp_count - 2) * 4)? as usize != units.len() - 1 {
        return Err(Error::Invalid("header PLC coverage"));
    }
    output.ignored_final_plc_cp = Some(u32_at(plc, (cp_count - 1) * 4)?);
    output.terminal_sentinel = Some(SourceRange {
        start_cp: end - 1,
        end_cp: end,
    });
    for i in 0..count {
        let a = u32_at(plc, i * 4)? as usize;
        let b = u32_at(plc, i * 4 + 4)? as usize;
        if a > b || b >= units.len() {
            return Err(Error::Invalid("header PLC order"));
        }
        let mut story = HeaderStory {
            index: i,
            role: if i < 6 {
                Some(SEPARATORS[i])
            } else if count_valid {
                Some(ROLES[(i - 6) % 6])
            } else {
                None
            },
            section_index: if i >= 6 && count_valid {
                Some((i - 6) / 6)
            } else {
                None
            },
            source: SourceRange {
                start_cp: origin + a,
                end_cp: origin + b,
            },
            content: None,
            guard: None,
            explicit_blank: false,
            unresolved: Vec::new(),
        };
        if a < b {
            if a > 0
                && (0xdc00..=0xdfff).contains(&units[a])
                && (0xd800..=0xdbff).contains(&units[a - 1])
            {
                story.unresolved.push("header_surrogate_boundary");
            }
            if units[b - 1] == 13 {
                story.guard = Some(SourceRange {
                    start_cp: origin + b - 1,
                    end_cp: origin + b,
                });
                story.content = Some(SourceRange {
                    start_cp: origin + a,
                    end_cp: origin + b - 1,
                });
                if i >= 6 && (b - a < 2 || units[b - 2] != 13) {
                    story
                        .unresolved
                        .push("header_content_paragraph_end_missing");
                } else if i >= 6 {
                    story.explicit_blank = b - a == 2;
                }
            } else {
                story.unresolved.push("header_guard_missing");
            }
        }
        output.stories.push(story);
    }
    let mut previous: [Option<usize>; 6] = [None; 6];
    for section in &mut sections {
        if !count_valid {
            section.references = empty_references("unresolved_slot_count");
            continue;
        }
        for (role, prior) in previous.iter_mut().enumerate() {
            let i = 6 + section.index * 6 + role;
            let story = &output.stories[i];
            let local = story.source.start_cp < story.source.end_cp;
            if local {
                *prior = Some(i);
            }
            let origin_index = *prior;
            let valid = origin_index.is_none_or(|idx| output.stories[idx].unresolved.is_empty());
            section.references.push(HeaderReference {
                role: ROLES[role],
                status: if !valid {
                    "unresolved_content"
                } else if local {
                    "local"
                } else if origin_index.is_some() {
                    "inherited"
                } else {
                    "absent"
                },
                local_story_index: Some(i),
                origin_story_index: origin_index,
                origin_section_index: origin_index.map(|idx| (idx - 6) / 6),
            });
        }
    }
    output.sections = sections;
    Ok(output)
}

fn empty_references(status: &'static str) -> Vec<HeaderReference> {
    ROLES
        .iter()
        .map(|role| HeaderReference {
            role,
            status,
            local_story_index: None,
            origin_story_index: None,
            origin_section_index: None,
        })
        .collect()
}

#[cfg(test)]
mod tests;
