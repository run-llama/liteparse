use super::*;
use crate::office::doc::{
    Diagnostic, SourceSegment, TranscribedStory, Transcription, visibility::RevisionEvidence,
};

#[derive(Debug, Serialize)]
pub struct NoteLabels {
    pub status: &'static str,
    pub labels: Vec<NoteLabel>,
}
#[derive(Debug, Serialize)]
pub struct NoteLabel {
    pub body_cp: usize,
    pub note_cp: Option<usize>,
    pub note_status: &'static str,
    pub section_index: Option<usize>,
    pub text: Option<String>,
    pub reason: &'static str,
}

/// Resolve automatic note labels only for a complete, unrevised reference sequence.
/// Custom source marks retain their existing text or symbol projection.
/// Page restarts require pagination and remain unresolved.
/// Ordinary hidden-text visibility is unresolved; these are accepted-source labels, not display claims.
pub fn resolve_note_labels(
    source: &Inspection,
    notes: &NoteInspection,
    sections: &NoteSections,
    revisions: &RevisionEvidence,
) -> Result<NoteLabels> {
    if notes.footnotes.len() + notes.endnotes.len() > MAX_NOTES || revisions.spans.len() > 262144 {
        return Err(Error::Limit("note label evidence"));
    }
    if sections
        .sections
        .windows(2)
        .any(|w| w[0].end_cp != w[1].start_cp)
        || sections.sections.iter().any(|s| s.start_cp >= s.end_cp)
        || revisions.spans.iter().any(|s| s.start_cp >= s.end_cp)
        || revisions
            .spans
            .windows(2)
            .any(|w| w[0].end_cp != w[1].start_cp)
    {
        return Err(Error::Invalid("note evidence ordering"));
    }
    let body = source
        .stories
        .iter()
        .find(|s| s.kind == "body")
        .ok_or(Error::Invalid("note body source"))?;
    if body.utf16_units > 16 * 1024 * 1024 {
        return Err(Error::Limit("note body units"));
    }
    let body_units: Vec<_> = body.text.encode_utf16().collect();
    if body_units.len() != body.utf16_units || body.start_cp != 0 {
        return Err(Error::Invalid("note body source length"));
    }
    let mut labels = Vec::new();
    for (kind, links) in [
        ("footnotes", &notes.footnotes),
        ("endnotes", &notes.endnotes),
    ] {
        if links
            .windows(2)
            .any(|w| w[0].body_cp >= w[1].body_cp || w[0].note_end_cp != w[1].note_start_cp)
            || links.iter().any(|l| l.note_start_cp >= l.note_end_cp)
        {
            return Err(Error::Invalid("note link ordering"));
        }
        let raw = source
            .stories
            .iter()
            .find(|s| s.kind == kind)
            .ok_or(Error::Invalid("note label source story"))?;
        if raw.utf16_units > 16 * 1024 * 1024 {
            return Err(Error::Limit("note label source"));
        }
        let units: Vec<_> = raw.text.encode_utf16().collect();
        if units.len() != raw.utf16_units {
            return Err(Error::Invalid("note text source length"));
        }
        let mut ordinal = 0u32;
        let mut section_ordinal = 0u32;
        let mut previous_section = None;
        let sequence_known = links
            .iter()
            .all(|link| properties_known(revisions, link.body_cp, false));
        for link in links {
            if !link.automatic {
                continue;
            }
            if link
                .body_cp
                .checked_sub(body.start_cp)
                .and_then(|i| body_units.get(i))
                != Some(&2)
            {
                return Err(Error::Invalid("note body marker source"));
            }
            ordinal += 1;
            let index = sections
                .sections
                .partition_point(|s| s.end_cp <= link.body_cp);
            let section_index = sections
                .sections
                .get(index)
                .filter(|s| s.start_cp <= link.body_cp && link.body_cp < s.end_cp)
                .map(|_| index);
            if section_index != previous_section {
                section_ordinal = 0;
                previous_section = section_index;
            }
            section_ordinal += 1;
            let marker = find_note_marker(&units, raw.start_cp, link, revisions)?;
            let mut label = NoteLabel {
                body_cp: link.body_cp,
                note_cp: match marker {
                    NoteMarker::Known(cp) => Some(cp),
                    NoteMarker::Unresolved(_) => None,
                },
                note_status: match marker {
                    NoteMarker::Known(_) => "source_special_note_marker",
                    NoteMarker::Unresolved(reason) => reason,
                },
                section_index,
                text: None,
                reason: "note_section_unavailable",
            };
            if !sequence_known {
                label.reason = "note_reference_revision_sequence_unresolved";
            } else if !properties_known(revisions, link.body_cp, true) {
                label.reason = "note_marker_properties_unresolved";
            } else if let Some(section) = section_index.and_then(|i| sections.sections.get(i)) {
                let n = if kind == "footnotes" {
                    section.footnotes
                } else {
                    section.endnotes
                };
                if !section.unresolved.is_empty() {
                    label.reason = "note_section_properties_unresolved";
                } else if n.restart == 2 {
                    label.reason = "note_page_restart_requires_layout";
                } else {
                    let number = if n.restart == 0 {
                        ordinal
                            .checked_add(u32::from(n.offset))
                            .and_then(|n| n.checked_sub(1))
                    } else if n.restart == 1 {
                        if sections.legacy_numbering.is_some() {
                            section_ordinal
                                .checked_add(u32::from(n.offset))
                                .and_then(|n| n.checked_sub(1))
                        } else {
                            Some(section_ordinal)
                        }
                    } else {
                        None
                    };
                    label.text = number.and_then(|number| format_number(number, n.format));
                    label.reason = if label.text.is_some() {
                        "source_number_and_body_reference"
                    } else {
                        "note_number_format_unresolved"
                    };
                }
            }
            labels.push(label);
        }
    }
    Ok(NoteLabels {
        status: "partial_source_note_labels",
        labels,
    })
}

#[derive(Clone, Copy)]
enum NoteMarker {
    Known(usize),
    Unresolved(&'static str),
}

/// U+0002 with CFSpec identifies an automatic note mark within its paired note text range.
/// Literal prefixes are source text; multiple candidate marks remain ambiguous.
/// <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/7022285b-9621-42e9-ad4d-4e02c115ef18>
fn find_note_marker(
    units: &[u16],
    start_cp: usize,
    link: &NoteLink,
    revisions: &RevisionEvidence,
) -> Result<NoteMarker> {
    let start = link
        .note_start_cp
        .checked_sub(start_cp)
        .ok_or(Error::Invalid("note marker start CP"))?;
    let end = link
        .note_end_cp
        .checked_sub(start_cp)
        .ok_or(Error::Invalid("note marker end CP"))?;
    let range = units
        .get(start..end)
        .ok_or(Error::Invalid("note marker source range"))?;
    let mut found = None;
    for (index, unit) in range.iter().enumerate() {
        if *unit != 2 {
            continue;
        }
        let cp = link
            .note_start_cp
            .checked_add(index)
            .ok_or(Error::Invalid("note marker CP overflow"))?;
        let span = revisions
            .spans
            .get(revisions.spans.partition_point(|s| s.end_cp <= cp));
        if span.is_some_and(|s| {
            s.start_cp <= cp
                && cp < s.end_cp
                && s.unresolved.is_empty()
                && s.properties.special == Some(false)
                && s.properties.deleted == Some(false)
                && s.properties.inserted == Some(false)
                && s.properties.field_hidden == Some(false)
        }) {
            continue;
        }
        if found.is_some() {
            return Ok(NoteMarker::Unresolved("multiple_note_label_markers"));
        }
        found = Some(
            link.note_start_cp
                .checked_add(index)
                .ok_or(Error::Invalid("note marker CP overflow"))?,
        );
    }
    Ok(match found {
        Some(cp) if properties_known(revisions, cp, true) => NoteMarker::Known(cp),
        Some(_) => NoteMarker::Unresolved("note_label_marker_properties_unresolved"),
        None => NoteMarker::Unresolved("note_label_marker_absent"),
    })
}

// Eligibility checks revision deletion, field instructions and special-character state.
// Ordinary CFVanish and style inheritance remain part of the transcription's unresolved visibility.
fn properties_known(revisions: &RevisionEvidence, cp: usize, special: bool) -> bool {
    let i = revisions.spans.partition_point(|s| s.end_cp <= cp);
    revisions.spans.get(i).is_some_and(|s| {
        s.start_cp <= cp
            && s.end_cp > cp
            && s.unresolved.is_empty()
            && s.properties.deleted == Some(false)
            && s.properties.field_hidden == Some(false)
            && (!special || s.properties.special == Some(true))
    })
}
/// MS-OSHARED MSONFC maps 9 to ECMA376 §17.18.59 chicago: *, dagger, double dagger, section.
/// Microsoft specifies repeated cycles and a display limit of 116; larger values stay unresolved.
/// <https://learn.microsoft.com/en-us/openspecs/office_standards/ms-oi29500/ea3612c8-a099-46c3-99a9-93658a30cb01>
fn format_number(number: u32, format: u16) -> Option<String> {
    if format == 9 {
        if !(1..=116).contains(&number) {
            return None;
        }
        return Some(
            ["*", "†", "‡", "§"][((number - 1) % 4) as usize]
                .repeat(((number - 1) / 4 + 1) as usize),
        );
    }
    if format == 0 {
        return Some(number.to_string());
    }
    if !(1..=3999).contains(&number) || !matches!(format, 1 | 2) {
        return None;
    }
    let mut number = number;
    let mut text = String::new();
    for (value, symbol) in [
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ] {
        while number >= value {
            text.push_str(symbol);
            number -= value;
        }
    }
    Some(if format == 2 {
        text.to_ascii_lowercase()
    } else {
        text
    })
}

/// Replace source-owned U+0002 controls, keeping the existing CP partition and output bytes elsewhere.
/// Generated numbers and zero-output revision/field segments are carried through unchanged.
pub fn apply_notes(input: &Transcription, labels: &NoteLabels) -> Result<Transcription> {
    use std::collections::BTreeMap;
    if labels.labels.len() > MAX_NOTES {
        return Err(Error::Limit("note projection labels"));
    }
    let mut replacements = BTreeMap::new();
    for label in &labels.labels {
        if let Some(text) = &label.text {
            if text.is_empty() || text.len() > 96 {
                return Err(Error::Limit("note generated label length"));
            }
            for cp in std::iter::once(label.body_cp).chain(label.note_cp) {
                if replacements.insert(cp, text.as_str()).is_some() {
                    return Err(Error::Invalid("duplicate note label CP"));
                }
            }
        }
    }
    if replacements.len() > MAX_NOTES * 2 {
        return Err(Error::Limit("note label replacements"));
    }
    let mut applied = std::collections::BTreeSet::new();
    let mut stories = Vec::new();
    for story in &input.stories {
        let mut output = TranscribedStory {
            kind: story.kind,
            text: String::new(),
            segments: Vec::new(),
            diagnostics: story
                .diagnostics
                .iter()
                .map(|d| Diagnostic {
                    code: d.code,
                    cp: d.cp,
                })
                .collect(),
            fields_balanced: story.fields_balanced,
            unresolved_control_units: story.unresolved_control_units,
        };
        let mut previous_output = 0;
        let mut previous_cp = story.segments.first().map_or(0, |s| s.start_cp);
        for segment in &story.segments {
            if segment.end_cp < segment.start_cp
                || segment.start_cp != previous_cp
                || segment.output_start != previous_output
            {
                return Err(Error::Invalid("note output partition"));
            }
            let text = story
                .text
                .get(segment.output_start..segment.output_end)
                .ok_or(Error::Invalid("note output offset"))?;
            let intersects = replacements
                .range(segment.start_cp..segment.end_cp)
                .next()
                .is_some();
            if !intersects
                || segment.kind != "unresolved_control"
                || text.encode_utf16().count() != segment.end_cp - segment.start_cp
            {
                append(
                    &mut output,
                    segment.kind,
                    segment.start_cp,
                    segment.end_cp,
                    text,
                )?;
            } else {
                let mut cp = segment.start_cp;
                for ch in text.chars() {
                    let end = cp + ch.len_utf16();
                    if let Some(label) = replacements
                        .get(&cp)
                        .filter(|_| ch == '\u{fffc}' && end == cp + 1)
                    {
                        if !applied.insert(cp) {
                            return Err(Error::Invalid("note label applied twice"));
                        }
                        append(&mut output, "note_label", cp, end, label)?;
                        output.unresolved_control_units =
                            output.unresolved_control_units.saturating_sub(1);
                    } else {
                        append(
                            &mut output,
                            segment.kind,
                            cp,
                            end,
                            ch.encode_utf8(&mut [0; 4]),
                        )?;
                    }
                    cp = end;
                }
            }
            previous_output = segment.output_end;
            previous_cp = segment.end_cp;
        }
        if previous_output != story.text.len() {
            return Err(Error::Invalid("note incomplete output partition"));
        }
        stories.push(output);
    }
    if applied.len() != replacements.len() {
        return Err(Error::Unsupported("note label source projection mapping"));
    }
    let mut unresolved = input.unresolved.clone();
    if labels
        .labels
        .iter()
        .any(|l| l.text.is_none() || l.note_cp.is_none())
    {
        unresolved.push("unsupported_note_numbering_or_markers");
    }
    Ok(Transcription {
        schema_version: input.schema_version,
        status: input.status,
        unresolved,
        stories,
    })
}
fn append(
    story: &mut TranscribedStory,
    kind: &'static str,
    start_cp: usize,
    end_cp: usize,
    text: &str,
) -> Result<()> {
    if story.segments.len() >= 262144 {
        return Err(Error::Limit("note projection segments"));
    }
    let output_start = story.text.len();
    story.text.push_str(text);
    story.segments.push(SourceSegment {
        kind,
        start_cp,
        end_cp,
        output_start,
        output_end: story.text.len(),
    });
    Ok(())
}
