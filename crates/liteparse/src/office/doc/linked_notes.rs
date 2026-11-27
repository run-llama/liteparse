//! Readable note content joined to source references without page placement.
use crate::office::doc::{Error, Inspection, Result, Transcription, notes::NoteInspection};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct LinkedNotes {
    pub status: &'static str,
    pub ordering: &'static str,
    pub visibility_evidence: &'static str,
    pub source_relationship_unresolved: Vec<&'static str>,
    pub notes: Vec<LinkedNote>,
}
#[derive(Debug, Serialize)]
pub struct LinkedNote {
    pub kind: &'static str,
    pub body_cp: usize,
    pub source_cp: [usize; 2],
    pub raw_reference_index: u16,
    pub automatic_reference: bool,
    pub logical_utf8_range: [usize; 2],
    pub text: String,
    pub unresolved: Vec<&'static str>,
}

/// Link existing logical note text to each independently decoded source reference.
/// Ranges refer to the corresponding transcription story. Raw text remains in
/// inspection stories; labels and hidden/revision decisions come from transcription.
/// Reference order does not assert page order or an insertion point in body text.
pub fn extract(
    source: &Inspection,
    transcription: &Transcription,
    links: &NoteInspection,
    hidden: Option<&crate::office::doc::hidden_visibility::HiddenEvidence>,
) -> Result<LinkedNotes> {
    crate::office::doc::extraction_summary::summarize(source, transcription)?;
    if links.footnotes.len().saturating_add(links.endnotes.len()) > 65_536 {
        return Err(Error::Limit("linked note count"));
    }
    let body = source
        .stories
        .iter()
        .find(|s| s.kind == "body")
        .ok_or(Error::Invalid("linked note body"))?;
    let body_end = body
        .start_cp
        .checked_add(body.utf16_units)
        .ok_or(Error::Limit("linked note body range"))?;
    if let Some(evidence) = hidden
        && (evidence
            .spans
            .windows(2)
            .any(|w| w[0].end_cp > w[1].start_cp)
            || evidence.spans.iter().any(|s| s.start_cp >= s.end_cp))
    {
        return Err(Error::Invalid("linked note visibility partition"));
    }
    let body_projection = transcription
        .stories
        .iter()
        .find(|s| s.kind == "body")
        .ok_or(Error::Invalid("linked note body projection"))?;
    let mut work = 8 * 1024 * 1024usize;
    let mut references = std::collections::BTreeSet::new();
    let mut output = Vec::new();
    let mut copied = 0usize;
    for (kind, group) in [
        ("footnotes", &links.footnotes),
        ("endnotes", &links.endnotes),
    ] {
        let raw = source
            .stories
            .iter()
            .find(|s| s.kind == kind)
            .ok_or(Error::Invalid("linked note source story"))?;
        let story = transcription
            .stories
            .iter()
            .find(|s| s.kind == kind)
            .ok_or(Error::Invalid("linked note projected story"))?;
        let end = raw
            .start_cp
            .checked_add(raw.utf16_units)
            .ok_or(Error::Limit("linked note source range"))?;
        let mut prior = None;
        let mut cursor = 0;
        for link in group {
            if !references.insert(link.body_cp) {
                return Err(Error::Invalid("linked note duplicate body reference"));
            }
            let [start, stop] = [link.note_start_cp, link.note_end_cp];
            if start < raw.start_cp
                || stop > end
                || start >= stop
                || link.body_cp < body.start_cp
                || link.body_cp >= body_end
                || prior.is_some_and(|(cp, limit)| link.body_cp <= cp || start < limit)
            {
                return Err(Error::Invalid("linked note ownership"));
            }
            prior = Some((link.body_cp, stop));
            while cursor < story.segments.len()
                && story.segments[cursor].end_cp <= start
                && story.segments[cursor].start_cp < start
            {
                cursor += 1;
            }
            let first = story
                .segments
                .get(cursor)
                .ok_or(Error::Invalid("linked note start"))?;
            let begin = boundary(first, start, &story.text)?;
            let mut finish = begin;
            let mut cp = start;
            let mut unresolved = Vec::new();
            let reference_index = body_projection
                .segments
                .partition_point(|s| s.end_cp <= link.body_cp);
            if let Some(reference) = body_projection.segments.get(reference_index) {
                if reference.start_cp > link.body_cp
                    || reference.output_start == reference.output_end
                {
                    unresolved.push("body_reference_suppressed_or_unresolved");
                }
            } else {
                unresolved.push("body_reference_suppressed_or_unresolved");
            }
            work = work
                .checked_sub(body_projection.diagnostics.len())
                .ok_or(Error::Limit("linked note work"))?;
            if body_projection
                .diagnostics
                .iter()
                .any(|d| d.cp == link.body_cp)
            {
                unresolved.push("body_reference_transcription_diagnostic");
            }
            while let Some(segment) = story.segments.get(cursor) {
                if segment.start_cp >= stop {
                    break;
                }
                if segment.start_cp > cp || segment.end_cp < cp {
                    return Err(Error::Invalid("linked note segment gap"));
                }
                if matches!(segment.kind, "unresolved_control" | "unprojected_tail") {
                    unresolved.push("logical_projection_unresolved");
                }
                cp = segment.end_cp.min(stop);
                finish = boundary(segment, cp, &story.text)?;
                if segment.end_cp > stop {
                    break;
                }
                cursor += 1;
            }
            if cp != stop {
                return Err(Error::Invalid("linked note incomplete partition"));
            }
            let text = story
                .text
                .get(begin..finish)
                .ok_or(Error::Invalid("linked note UTF8 range"))?;
            copied = copied
                .checked_add(text.len())
                .ok_or(Error::Limit("linked note text"))?;
            if copied > 64 * 1024 * 1024 {
                return Err(Error::Limit("linked note text"));
            }
            // Visibility is separate from source links and already-projected text.
            // Check both the body reference and the complete note range.
            for (a, b) in [(link.body_cp, link.body_cp + 1), (start, stop)] {
                let mut covered = a;
                if let Some(evidence) = hidden {
                    let first = evidence.spans.partition_point(|s| s.end_cp <= a);
                    for span in evidence.spans[first..]
                        .iter()
                        .take_while(|s| s.start_cp < b)
                    {
                        work = work
                            .checked_sub(1)
                            .ok_or(Error::Limit("linked note work"))?;
                        if span.start_cp > covered || span.property.hidden.is_none() {
                            unresolved.push("hidden_visibility_unresolved");
                        }
                        if span.property.hidden == Some(true) {
                            unresolved.push("contains_source_hidden_text");
                        }
                        covered = covered.max(span.end_cp.min(b));
                    }
                }
                if covered < b {
                    unresolved.push("hidden_visibility_unresolved");
                }
            }
            work = work
                .checked_sub(story.diagnostics.len())
                .ok_or(Error::Limit("linked note work"))?;
            if story
                .diagnostics
                .iter()
                .any(|d| start <= d.cp && d.cp < stop)
            {
                unresolved.push("transcription_diagnostics_in_note");
            }
            unresolved.sort_unstable();
            unresolved.dedup();
            output.push(LinkedNote {
                kind,
                body_cp: link.body_cp,
                source_cp: [start, stop],
                raw_reference_index: link.raw_index,
                automatic_reference: link.automatic,
                logical_utf8_range: [begin, finish],
                text: text.to_owned(),
                unresolved,
            });
        }
    }
    output.sort_by_key(|n| (n.body_cp, n.kind));
    Ok(LinkedNotes {
        status: "linked_logical_note_content",
        ordering: "body_reference_source_cp",
        visibility_evidence: "hidden_properties_and_existing_transcription",
        source_relationship_unresolved: links.unresolved.to_vec(),
        notes: output,
    })
}

// Consecutive paragraph marks may share a segment across a note boundary.
// Split only the established one-CP to one-newline mapping; other transforms
// require complete segment ownership.
fn boundary(segment: &crate::office::doc::SourceSegment, cp: usize, text: &str) -> Result<usize> {
    if cp == segment.start_cp {
        return Ok(segment.output_start);
    }
    if cp == segment.end_cp {
        return Ok(segment.output_end);
    }
    if cp > segment.start_cp && cp < segment.end_cp && segment.kind == "paragraph" {
        let slice = text
            .get(segment.output_start..segment.output_end)
            .ok_or(Error::Invalid("linked note paragraph bytes"))?;
        if slice.len() == segment.end_cp - segment.start_cp && slice.bytes().all(|b| b == b'\n') {
            return Ok(segment.output_start + cp - segment.start_cp);
        }
    }
    Err(Error::Unsupported("linked note boundary inside projection"))
}

#[cfg(test)]
mod tests;
