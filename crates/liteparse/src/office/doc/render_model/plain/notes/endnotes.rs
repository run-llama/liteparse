//! Endnote references and labels remain source-owned; placement is an explicit partial policy.
use super::*;

pub fn inspect(data: &[u8], source: &Inspection) -> Result<Option<FootnoteDocument>> {
    let story = source
        .stories
        .iter()
        .find(|s| s.kind == "endnotes")
        .ok_or(Error::Invalid("endnote story missing"))?;
    if story.utf16_units == 0 {
        return Ok(None);
    }
    if source
        .stories
        .iter()
        .any(|s| s.kind == "footnotes" && s.utf16_units != 0)
    {
        return Err(Error::Unsupported(
            "render mixed footnote and endnote placement",
        ));
    }
    let links = crate::office::doc::notes::inspect_notes(data, source)?;
    let sections = crate::office::doc::notes::inspect_note_sections(data, source)?;
    let formatting = crate::office::doc::formatting_inspection::inspect_character_formatting(data)?;
    let revisions = crate::office::doc::visibility::resolve_revision_evidence(&formatting)?;
    let labels =
        crate::office::doc::notes::resolve_note_labels(source, &links, &sections, &revisions)?;
    prepare(source, &links, &labels, &sections)
}

fn prepare(
    source: &Inspection,
    links: &crate::office::doc::notes::NoteInspection,
    labels: &crate::office::doc::notes::NoteLabels,
    sections: &crate::office::doc::notes::NoteSections,
) -> Result<Option<FootnoteDocument>> {
    let body = source
        .stories
        .iter()
        .find(|s| s.kind == "body")
        .ok_or(Error::Invalid("endnote body"))?;
    let story = source
        .stories
        .iter()
        .find(|s| s.kind == "endnotes")
        .ok_or(Error::Invalid("endnote story"))?;
    if story.utf16_units > 1024 * 1024
        || links.endnotes.len() > 4096
        || body.utf16_units > 1024 * 1024
    {
        return Err(Error::Limit("render endnote source"));
    }
    if !links.footnotes.is_empty() || links.endnotes.is_empty() || sections.sections.len() != 1 {
        return Err(Error::Unsupported(
            "render endnote section or mixed note scope",
        ));
    }
    let section = &sections.sections[0];
    if section.start_cp != 0 || section.end_cp != body.utf16_units || !section.unresolved.is_empty()
    {
        return Err(Error::Unsupported("render endnote section evidence"));
    }
    let raw: Vec<_> = story.text.encode_utf16().collect();
    let body_raw: Vec<_> = body.text.encode_utf16().collect();
    let end = story
        .start_cp
        .checked_add(story.utf16_units)
        .ok_or(Error::Limit("endnote CP extent"))?;
    if raw.len() != story.utf16_units
        || raw.last() != Some(&13)
        || body_raw.len() != body.utf16_units
    {
        return Err(Error::Invalid("endnote source extent"));
    }
    let expected: std::collections::BTreeSet<_> = links
        .endnotes
        .iter()
        .filter(|l| l.automatic)
        .map(|l| l.body_cp)
        .collect();
    let actual: std::collections::BTreeSet<_> = labels.labels.iter().map(|l| l.body_cp).collect();
    if expected != actual || actual.len() != labels.labels.len() {
        return Err(Error::Invalid("endnote label coverage"));
    }
    let label_by_cp: std::collections::BTreeMap<_, _> =
        labels.labels.iter().map(|l| (l.body_cp, l)).collect();
    let mut cursor = story.start_cp;
    let mut prior = None;
    let mut notes = Vec::new();
    for (index, link) in links.endnotes.iter().enumerate() {
        if link.note_start_cp != cursor
            || link.note_end_cp <= cursor
            || link.note_end_cp >= end
            || prior.is_some_and(|p| link.body_cp <= p)
            || link.body_cp >= body_raw.len()
            || link.automatic != (link.raw_index != 0)
        {
            return Err(Error::Invalid("endnote reference partition"));
        }
        let range = &raw[cursor - story.start_cp..link.note_end_cp - story.start_cp];
        if range.last() != Some(&13) {
            return Err(Error::Invalid("endnote paragraph terminator"));
        }
        let (marker_cp, marker_status, authored_reference, label) = if link.automatic {
            let label = label_by_cp
                .get(&link.body_cp)
                .copied()
                .ok_or(Error::Invalid("endnote label identity"))?;
            let text = label
                .text
                .as_ref()
                .ok_or(Error::Unsupported("endnote numbering unresolved"))?;
            if body_raw[link.body_cp] != 2
                || label.section_index != Some(0)
                || text.is_empty()
                || text.len() > 64
                || text.chars().any(|c| c.is_control() || c.is_whitespace())
            {
                return Err(Error::Invalid("endnote automatic reference"));
            }
            let marks: Vec<_> = range
                .iter()
                .enumerate()
                .filter_map(|(i, c)| (*c == 2).then_some(cursor + i))
                .collect();
            let status = match label.note_cp {
                Some(cp) if marks == [cp] => {
                    crate::office::doc::notes::FootnoteMarkerStatus::AutomaticSpecial
                }
                None if marks.is_empty() && label.note_status == "note_label_marker_absent" => {
                    crate::office::doc::notes::FootnoteMarkerStatus::AutomaticAbsent
                }
                _ => return Err(Error::Unsupported("endnote marker ambiguity")),
            };
            (label.note_cp, status, None, text.clone())
        } else {
            let code = body_raw[link.body_cp];
            if code < 32 || (0xd800..=0xdfff).contains(&code) || range.contains(&2) {
                return Err(Error::Unsupported("endnote authored reference"));
            }
            (
                None,
                crate::office::doc::notes::FootnoteMarkerStatus::AuthoredReference,
                Some(code),
                String::new(),
            )
        };
        notes.push(Footnote {
            id: (index + 1) as i64,
            body_cp: link.body_cp,
            raw_reference_index: link.raw_index,
            section_index: 0,
            source_cp: [cursor, link.note_end_cp],
            marker_cp,
            marker_status,
            authored_reference,
            label,
            paragraphs: Vec::new(),
        });
        cursor = link.note_end_cp;
        prior = Some(link.body_cp);
    }
    if cursor != end - 1 {
        return Err(Error::Invalid("endnote terminal guard"));
    }
    Ok(Some(FootnoteDocument {
        kind: NoteKind::Endnote,
        source_cp: [story.start_cp, end],
        source_text: story.text.clone(),
        terminal_guard: [end - 1, end],
        position: Some(NotePosition::SectionEnd),
        position_basis: Some("partial_endnotes_after_single_section_body"),
        sections: vec![crate::office::doc::notes::FootnoteSection {
            body_cp: [0, body.utf16_units],
            position: NotePosition::SectionEnd,
            position_basis: "partial_endnotes_after_single_section_body",
            numbering: section.endnotes,
            source_properties: section.properties.clone(),
        }],
        coexisting_endnotes: Vec::new(),
        coexisting_endnote_source: None,
        notes,
        separators: Vec::new(),
    }))
}

#[cfg(test)]
mod tests;
