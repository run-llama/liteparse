//! Source-owned footnote references and story ranges, independent of pagination.
use crate::office::doc::Result;
use crate::office::doc::{Error, Inspection, notes::NoteSections};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NotePosition {
    PageBottom,
    BeneathText,
    SectionEnd,
}

#[derive(Debug, Serialize)]
pub struct FootnoteDocument {
    pub source_cp: [usize; 2],
    pub source_text: String,
    pub terminal_guard: [usize; 2],
    pub position: Option<NotePosition>,
    pub position_basis: Option<&'static str>,
    pub sections: Vec<FootnoteSection>,
    pub notes: Vec<Footnote>,
    pub coexisting_endnotes: Vec<crate::office::doc::notes::NoteLink>,
    pub coexisting_endnote_source: Option<[usize; 2]>,
    pub separators: Vec<FootnoteSeparator>,
}

#[derive(Debug, Serialize)]
pub struct FootnoteSection {
    pub body_cp: [usize; 2],
    pub position: NotePosition,
    pub position_basis: &'static str,
    pub numbering: crate::office::doc::notes::Numbering,
    pub source_properties: Vec<crate::office::doc::formatting::PropertyModifier>,
}

#[derive(Debug, Serialize)]
pub struct FootnoteSeparator {
    pub index: usize,
    pub source_cp: [usize; 2],
    pub source_text: String,
}

#[derive(Debug, Serialize)]
pub struct Footnote {
    pub id: i64,
    pub body_cp: usize,
    pub raw_reference_index: u16,
    pub section_index: usize,
    pub source_cp: [usize; 2],
    pub marker_cp: Option<usize>,
    pub marker_status: FootnoteMarkerStatus,
    pub authored_reference: Option<u16>,
    pub label: String,
}

/// Source identity of the note-body mark, separate from the body reference label.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FootnoteMarkerStatus {
    AutomaticSpecial,
    AutomaticAbsent,
    AuthoredReference,
}

/// Decode source-owned footnote references, note ranges and separator contents.
/// Automatic labels require resolved numbering; authored references retain the exact
/// source code unit and do not infer a custom marker length or glyph. Consumers must
/// separately qualify formatting, visibility, separator geometry and pagination.
/// An absent automatic note-body marker does not authorize inserting a note prefix.
pub fn inspect_note_ownership(data: &[u8]) -> Result<Option<FootnoteDocument>> {
    let source = crate::office::doc::inspect(data)?;
    inspect_owned_source(data, &source)
}

fn inspect_owned_source(data: &[u8], source: &Inspection) -> Result<Option<FootnoteDocument>> {
    let story = source
        .stories
        .get(1)
        .ok_or(Error::Invalid("footnote ownership story"))?;
    if story.kind != "footnotes" {
        return Err(Error::Invalid("footnote ownership story identity"));
    }
    if story.utf16_units == 0 {
        return Ok(None);
    }
    let links = crate::office::doc::notes::inspect_notes(data, source)?;
    let sections = crate::office::doc::notes::inspect_note_sections(data, source)?;
    let formatting = crate::office::doc::formatting_inspection::inspect_character_formatting(data)?;
    let revisions = crate::office::doc::visibility::resolve_revision_evidence(&formatting)?;
    let labels =
        crate::office::doc::notes::resolve_note_labels(source, &links, &sections, &revisions)?;
    let container = crate::office::doc::compound::Compound::open(data)?;
    let word = container.stream("WordDocument")?;
    let (version, table_name, _, _, _) = crate::office::doc::document::fib(&word)?;
    if version != source.binary_version {
        return Err(Error::Invalid("footnote ownership source version"));
    }
    let table = container.stream(table_name)?;
    // fib() validates the fixed FIB arrays before these pair offsets are used.
    let dop = crate::office::doc::bytes(
        &table,
        crate::office::doc::u32_at(&word, 154 + 31 * 8)? as usize,
        crate::office::doc::u32_at(&word, 158 + 31 * 8)? as usize,
    )?;
    let section_plans = section_plans(version, dop, &sections, source.stories[0].utf16_units)?;
    let mut result = prepare_sections(source, &links, &labels, section_plans)?;
    if let Some(plan) = &mut result {
        let headers = crate::office::doc::headers::inspect_headers(data, source)?;
        let story = &source.stories[2];
        let raw: Vec<u16> = story.text.encode_utf16().collect();
        for slot in headers.stories.iter().take(3) {
            if let Some(content) = slot.content {
                let start = content
                    .start_cp
                    .checked_sub(story.start_cp)
                    .ok_or(Error::Invalid("note separator source start"))?;
                let end = content
                    .end_cp
                    .checked_sub(story.start_cp)
                    .ok_or(Error::Invalid("note separator source end"))?;
                let text = String::from_utf16(
                    raw.get(start..end)
                        .ok_or(Error::Invalid("note separator source range"))?,
                )
                .map_err(|_| Error::Invalid("note separator UTF16"))?;
                plan.separators.push(FootnoteSeparator {
                    index: slot.index,
                    source_cp: [content.start_cp, content.end_cp],
                    source_text: text,
                });
            }
        }
    }
    Ok(result)
}

fn section_plans(
    version: u16,
    dop: &[u8],
    sections: &NoteSections,
    body_end: usize,
) -> Result<Vec<FootnoteSection>> {
    if sections.sections.is_empty()
        || sections.sections.len() > 65536
        || sections.sections[0].start_cp != 0
        || sections
            .sections
            .last()
            .is_none_or(|s| s.end_cp != body_end)
        || sections.sections.iter().any(|s| s.start_cp >= s.end_cp)
        || sections
            .sections
            .windows(2)
            .any(|s| s[0].end_cp != s[1].start_cp)
    {
        return Err(Error::Invalid("note section ownership partition"));
    }
    let mut work = 0usize;
    sections
        .sections
        .iter()
        .map(|section| {
            let (position, basis) = position_for_section(version, dop, section)?;
            for p in &section.properties {
                work = work
                    .checked_add(p.operand.len() + 2)
                    .filter(|n| *n <= 8 * 1024 * 1024)
                    .ok_or(Error::Limit("note section ownership properties"))?;
            }
            Ok(FootnoteSection {
                body_cp: [section.start_cp, section.end_cp],
                position,
                position_basis: basis,
                numbering: section.footnotes,
                source_properties: section.properties.clone(),
            })
        })
        .collect()
}

#[cfg(test)]
fn position(
    version: u16,
    dop: &[u8],
    sections: &NoteSections,
) -> Result<(NotePosition, &'static str)> {
    let [section] = sections.sections.as_slice() else {
        return Err(Error::Unsupported("test section count"));
    };
    position_for_section(version, dop, section)
}

fn position_for_section(
    version: u16,
    dop: &[u8],
    section: &crate::office::doc::notes::NoteSection,
) -> Result<(NotePosition, &'static str)> {
    if !matches!(version, 0xc1 | 0xd9 | 0x101 | 0x10c | 0x112) {
        return Err(Error::Unsupported("footnote ownership binary version"));
    }
    if !section.unresolved.is_empty() || section.footnotes.restart > 1 {
        return Err(Error::Unsupported("footnote ownership numbering context"));
    }
    // DopBase fpc applies through Word 2000; later files use the section SFpc enum.
    // https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/f885b87a-15b3-460e-aecc-213bd17f960e
    if version <= 0xd9 {
        let byte = *dop
            .first()
            .ok_or(Error::Invalid("footnote ownership DOP"))?;
        let value = match (byte >> 5) & 3 {
            0 => NotePosition::SectionEnd,
            1 => NotePosition::PageBottom,
            2 => NotePosition::BeneathText,
            _ => return Err(Error::Invalid("footnote ownership legacy position")),
        };
        return Ok((value, "dop_base_fpc"));
    }
    let mut result = NotePosition::PageBottom;
    for modifier in &section.properties {
        match modifier.sprm {
            0x303b => {
                result = match modifier.operand.as_slice() {
                    [1] => NotePosition::PageBottom,
                    [2] => NotePosition::BeneathText,
                    _ => return Err(Error::Invalid("footnote ownership section position")),
                };
            }
            0x3239 => {
                return Err(Error::Unsupported(
                    "footnote ownership section revision wall",
                ));
            }
            _ => {}
        }
    }
    Ok((result, "section_sfpc_or_bottom_page_default"))
}

#[cfg(test)]
fn prepare(
    source: &Inspection,
    links: &crate::office::doc::notes::NoteInspection,
    labels: &crate::office::doc::notes::NoteLabels,
    position: NotePosition,
    position_basis: &'static str,
) -> Result<Option<FootnoteDocument>> {
    prepare_sections(
        source,
        links,
        labels,
        vec![FootnoteSection {
            body_cp: [0, source.stories[0].utf16_units],
            position,
            position_basis,
            numbering: crate::office::doc::notes::Numbering {
                restart: 0,
                offset: 1,
                format: 0,
            },
            source_properties: vec![],
        }],
    )
}

fn prepare_sections(
    source: &Inspection,
    links: &crate::office::doc::notes::NoteInspection,
    labels: &crate::office::doc::notes::NoteLabels,
    sections: Vec<FootnoteSection>,
) -> Result<Option<FootnoteDocument>> {
    let body = source
        .stories
        .first()
        .ok_or(Error::Invalid("note ownership body"))?;
    let story = source
        .stories
        .get(1)
        .ok_or(Error::Invalid("note ownership story"))?;
    if links.footnotes.len() > 4096 || story.utf16_units > 1024 * 1024 {
        return Err(Error::Limit("footnote ownership work limit"));
    }
    let footnote_references: std::collections::BTreeSet<_> = links
        .footnotes
        .iter()
        .filter(|n| n.automatic)
        .map(|n| n.body_cp)
        .collect();
    let endnote_references: std::collections::BTreeSet<_> =
        links.endnotes.iter().map(|n| n.body_cp).collect();
    if links
        .footnotes
        .iter()
        .any(|n| endnote_references.contains(&n.body_cp))
    {
        return Err(Error::Invalid("overlapping footnote and endnote reference"));
    }
    let all_label_references: std::collections::BTreeSet<_> = links
        .footnotes
        .iter()
        .chain(&links.endnotes)
        .filter(|n| n.automatic)
        .map(|n| n.body_cp)
        .collect();
    let label_references: std::collections::BTreeSet<_> =
        labels.labels.iter().map(|n| n.body_cp).collect();
    if label_references.len() != labels.labels.len() || label_references != all_label_references {
        return Err(Error::Invalid("note label reference coverage"));
    }
    let endnote_source = if links.endnotes.is_empty() {
        None
    } else {
        let story = source
            .stories
            .iter()
            .find(|s| s.kind == "endnotes")
            .ok_or(Error::Invalid("coexisting endnote story"))?;
        let end = story
            .start_cp
            .checked_add(story.utf16_units)
            .ok_or(Error::Limit("coexisting endnote source"))?;
        if story.utf16_units == 0
            || links.endnotes.iter().any(|n| {
                n.body_cp >= body.utf16_units
                    || n.note_start_cp < story.start_cp
                    || n.note_end_cp <= n.note_start_cp
                    || n.note_end_cp >= end
            })
        {
            return Err(Error::Invalid("coexisting endnote source ownership"));
        }
        Some([story.start_cp, end])
    };
    if body.start_cp != 0
        || story.start_cp != body.utf16_units
        || story.kind != "footnotes"
        || labels
            .labels
            .iter()
            .filter(|n| footnote_references.contains(&n.body_cp))
            .count()
            != links.footnotes.iter().filter(|n| n.automatic).count()
    {
        return Err(Error::Invalid("footnote ownership source partition"));
    }
    let body_units: Vec<_> = body.text.encode_utf16().collect();
    let units: Vec<_> = story.text.encode_utf16().collect();
    if body_units.len() != body.utf16_units
        || units.len() != story.utf16_units
        || units.last() != Some(&13)
        || links.footnotes.is_empty()
    {
        return Err(Error::Invalid("footnote ownership source extent"));
    }
    let end = story
        .start_cp
        .checked_add(units.len())
        .ok_or(Error::Limit("footnote ownership CP"))?;
    let mut cp = story.start_cp;
    let mut previous_body = None;
    let mut notes = Vec::with_capacity(links.footnotes.len());
    let mut automatic_labels = labels
        .labels
        .iter()
        .filter(|n| footnote_references.contains(&n.body_cp));
    for (index, link) in links.footnotes.iter().enumerate() {
        if link.note_start_cp != cp
            || link.note_end_cp <= cp
            || link.note_end_cp >= end
            || previous_body.is_some_and(|p| link.body_cp <= p)
            || link.body_cp >= body_units.len()
            || link.automatic != (link.raw_index != 0)
        {
            return Err(Error::Unsupported("footnote ownership reference ownership"));
        }
        let section_index = sections.partition_point(|s| s.body_cp[1] <= link.body_cp);
        if !sections
            .get(section_index)
            .is_some_and(|s| s.body_cp[0] <= link.body_cp && link.body_cp < s.body_cp[1])
        {
            return Err(Error::Invalid("note reference section ownership"));
        }
        if !link.automatic {
            let code = body_units[link.body_cp];
            if code < 32
                || (0xd800..=0xdfff).contains(&code)
                || units[link.note_end_cp - story.start_cp - 1] != 13
            {
                return Err(Error::Unsupported("authored note reference source"));
            }
            notes.push(Footnote {
                id: (index + 1) as i64,
                body_cp: link.body_cp,
                raw_reference_index: link.raw_index,
                section_index,
                source_cp: [cp, link.note_end_cp],
                marker_cp: None,
                marker_status: FootnoteMarkerStatus::AuthoredReference,
                authored_reference: Some(code),
                label: String::new(),
            });
            cp = link.note_end_cp;
            previous_body = Some(link.body_cp);
            continue;
        }
        if body_units[link.body_cp] != 2 {
            return Err(Error::Invalid("automatic note reference source"));
        }
        let label = automatic_labels
            .next()
            .ok_or(Error::Invalid("automatic note label count"))?;
        let (marker, marker_status) = match label.note_cp {
            Some(cp) => (Some(cp), FootnoteMarkerStatus::AutomaticSpecial),
            None if label.note_status == "note_label_marker_absent"
                && !units[cp - story.start_cp..link.note_end_cp - story.start_cp].contains(&2) =>
            {
                (None, FootnoteMarkerStatus::AutomaticAbsent)
            }
            None => return Err(Error::Unsupported("unresolved note marker ownership")),
        };
        let text = label
            .text
            .as_ref()
            .ok_or(Error::Unsupported("unresolved note label"))?;
        if label.body_cp != link.body_cp
            || label.section_index != Some(section_index)
            || marker.is_some_and(|marker| {
                marker < cp
                    || marker >= link.note_end_cp
                    || units.get(marker - story.start_cp) != Some(&2)
            })
            || units[link.note_end_cp - story.start_cp - 1] != 13
            || text.is_empty()
            || text.len() > 64
            || text.chars().any(char::is_control)
        {
            return Err(Error::Invalid("note ownership label source identity"));
        }
        notes.push(Footnote {
            id: (index + 1) as i64,
            body_cp: link.body_cp,
            raw_reference_index: link.raw_index,
            section_index,
            source_cp: [cp, link.note_end_cp],
            marker_cp: marker,
            marker_status,
            authored_reference: None,
            label: text.clone(),
        });
        cp = link.note_end_cp;
        previous_body = Some(link.body_cp);
    }
    if cp != end - 1 {
        return Err(Error::Invalid("footnote ownership terminal ownership"));
    }
    Ok(Some(FootnoteDocument {
        source_cp: [story.start_cp, end],
        source_text: story.text.clone(),
        terminal_guard: [cp, end],
        position: sections
            .first()
            .map(|s| s.position)
            .filter(|p| sections.iter().all(|s| s.position == *p)),
        position_basis: sections
            .first()
            .map(|s| s.position_basis)
            .filter(|b| sections.iter().all(|s| s.position_basis == *b)),
        sections,
        notes,
        coexisting_endnote_source: endnote_source,
        coexisting_endnotes: links
            .endnotes
            .iter()
            .map(|n| crate::office::doc::notes::NoteLink {
                body_cp: n.body_cp,
                note_start_cp: n.note_start_cp,
                note_end_cp: n.note_end_cp,
                automatic: n.automatic,
                raw_index: n.raw_index,
            })
            .collect(),
        separators: vec![],
    }))
}

#[cfg(test)]
mod tests;
