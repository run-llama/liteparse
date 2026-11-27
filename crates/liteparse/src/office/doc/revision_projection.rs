//! A partial final-revision view with source ranges retained for omitted text.
use crate::office::doc::{
    Error, Result,
    transcription::{Diagnostic, SourceSegment, TranscribedStory, Transcription},
    visibility::{RevisionEvidence, RevisionSpan},
};

const MAX_SEGMENTS: usize = 262_144;
const MAX_DIAGNOSTICS: usize = 256;

/// Apply known source deletions to an existing field/control projection.
/// Unknown properties and split Unicode scalars retain their projected text with diagnostics.
/// Hidden text, revised field structure and other existing unresolved semantics remain explicit.
pub fn apply_revisions(
    input: &Transcription,
    revisions: &RevisionEvidence,
) -> Result<Transcription> {
    if revisions
        .spans
        .windows(2)
        .any(|w| w[0].end_cp != w[1].start_cp)
        || revisions.spans.iter().any(|s| s.start_cp >= s.end_cp)
    {
        return Err(Error::Invalid("revision projection span order"));
    }
    let mut stories = Vec::new();
    for story in &input.stories {
        stories.push(project_story(story, &revisions.spans)?);
    }
    let mut unresolved = input.unresolved.clone();
    unresolved.retain(|s| *s != "character_properties_and_revision_visibility");
    unresolved.push("hidden_text_and_unresolved_revision_properties");
    unresolved.push("field_revision_structure");
    Ok(Transcription {
        schema_version: input.schema_version,
        status: "partial_transcription",
        unresolved,
        stories,
    })
}

fn project_story(story: &TranscribedStory, spans: &[RevisionSpan]) -> Result<TranscribedStory> {
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
        unresolved_control_units: 0,
    };
    let mut previous_output = 0;
    let mut previous_cp = story.segments.first().map_or(0, |s| s.start_cp);
    let mut index = spans.partition_point(|s| s.end_cp <= previous_cp);
    for segment in &story.segments {
        if segment.start_cp != previous_cp
            || segment.end_cp <= segment.start_cp
            || segment.output_start != previous_output
            || segment.output_end < segment.output_start
        {
            return Err(Error::Invalid("revision projection source segments"));
        }
        let text = story
            .text
            .get(segment.output_start..segment.output_end)
            .ok_or(Error::Invalid("revision projection output offsets"))?;
        let mut cp = segment.start_cp;
        if text.is_empty() {
            while cp < segment.end_cp {
                advance(spans, &mut index, cp);
                let span = spans
                    .get(index)
                    .filter(|s| s.start_cp <= cp)
                    .ok_or(Error::Invalid("revision projection coverage"))?;
                let end = segment.end_cp.min(span.end_cp);
                let deleted = span.properties.deleted == Some(true);
                if span.properties.deleted.is_none() {
                    diagnostic(&mut output, "revision_visibility_unresolved", cp);
                }
                if deleted && segment.kind == "field_marker" {
                    diagnostic(&mut output, "field_revision_structure_unresolved", cp);
                }
                append(
                    &mut output,
                    if deleted {
                        "deleted_revision"
                    } else {
                        segment.kind
                    },
                    cp,
                    end,
                    None,
                )?;
                cp = end;
            }
        } else {
            for ch in text.chars() {
                let end = cp
                    .checked_add(ch.len_utf16())
                    .ok_or(Error::Invalid("revision projection CP overflow"))?;
                if end > segment.end_cp {
                    return Err(Error::Invalid("revision projection character mapping"));
                }
                advance(spans, &mut index, cp);
                let first = spans
                    .get(index)
                    .filter(|s| s.start_cp <= cp)
                    .ok_or(Error::Invalid("revision projection coverage"))?;
                let mut deleted = first.properties.deleted;
                if first.end_cp < end {
                    let second = spans
                        .get(index + 1)
                        .filter(|s| s.start_cp == first.end_cp && s.end_cp >= end)
                        .ok_or(Error::Invalid("revision projection coverage"))?;
                    if second.properties.deleted != deleted {
                        deleted = None;
                        diagnostic(&mut output, "revision_splits_unicode_scalar", cp);
                    }
                }
                if deleted.is_none() {
                    diagnostic(&mut output, "revision_visibility_unresolved", cp);
                }
                let omit = deleted == Some(true);
                if !omit && segment.kind == "unresolved_control" {
                    output.unresolved_control_units += ch.len_utf16();
                }
                append(
                    &mut output,
                    if omit {
                        "deleted_revision"
                    } else {
                        segment.kind
                    },
                    cp,
                    end,
                    if omit { None } else { Some(ch) },
                )?;
                cp = end;
            }
            if cp != segment.end_cp {
                return Err(Error::Invalid("revision projection character mapping"));
            }
        }
        previous_cp = segment.end_cp;
        previous_output = segment.output_end;
    }
    if previous_output != story.text.len() {
        return Err(Error::Invalid("revision projection output coverage"));
    }
    Ok(output)
}

fn advance(spans: &[RevisionSpan], index: &mut usize, cp: usize) {
    while spans.get(*index).is_some_and(|s| s.end_cp <= cp) {
        *index += 1;
    }
}

fn diagnostic(story: &mut TranscribedStory, code: &'static str, cp: usize) {
    if story.diagnostics.len() < MAX_DIAGNOSTICS
        && !story.diagnostics.iter().any(|d| d.code == code)
    {
        story.diagnostics.push(Diagnostic { code, cp });
    }
}

fn append(
    story: &mut TranscribedStory,
    kind: &'static str,
    start_cp: usize,
    end_cp: usize,
    ch: Option<char>,
) -> Result<()> {
    let output_start = story.text.len();
    if let Some(ch) = ch {
        story.text.push(ch);
    }
    if let Some(last) = story
        .segments
        .last_mut()
        .filter(|s| s.kind == kind && s.end_cp == start_cp)
    {
        last.end_cp = end_cp;
        last.output_end = story.text.len();
    } else {
        if story.segments.len() == MAX_SEGMENTS {
            return Err(Error::Limit("revision projection segments"));
        }
        story.segments.push(SourceSegment {
            kind,
            start_cp,
            end_cp,
            output_start,
            output_end: story.text.len(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests;
