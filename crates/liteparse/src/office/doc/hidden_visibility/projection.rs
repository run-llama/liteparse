use super::{HiddenEvidence, HiddenSpan};
use crate::office::doc::{
    Diagnostic, Error, Result, SourceSegment, TranscribedStory, Transcription,
};

const MAX_SEGMENTS: usize = 262_144;

/// Hide only source ranges whose ordinary hidden property is known true.
/// Generated text with an ambiguous source mapping and unknown properties is retained.
/// The input remains an accepted-source projection for independent inspection.
pub fn apply_hidden(input: &Transcription, evidence: &HiddenEvidence) -> Result<Transcription> {
    if input.stories.len() > 8 || input.unresolved.len() > 256 {
        return Err(Error::Limit("hidden projection stories or diagnostics"));
    }
    let mut segment_budget = MAX_SEGMENTS;
    let mut text_budget = 64 * 1024 * 1024usize;
    let mut source_budget = 16 * 1024 * 1024usize;
    for story in &input.stories {
        if story.diagnostics.len() > 256 {
            return Err(Error::Limit("hidden projection diagnostics"));
        }
        segment_budget = segment_budget
            .checked_sub(story.segments.len())
            .ok_or(Error::Limit("hidden projection input segments"))?;
        text_budget = text_budget
            .checked_sub(story.text.len())
            .ok_or(Error::Limit("hidden projection text bytes"))?;
        for segment in &story.segments {
            let length = segment
                .end_cp
                .checked_sub(segment.start_cp)
                .ok_or(Error::Invalid("hidden projection reversed source range"))?;
            source_budget = source_budget
                .checked_sub(length)
                .ok_or(Error::Limit("hidden projection source units"))?;
        }
    }
    if evidence.spans.len() > MAX_SEGMENTS
        || evidence
            .spans
            .iter()
            .any(|s| s.start_cp >= s.end_cp || s.end_cp > 16 * 1024 * 1024)
        || evidence
            .spans
            .windows(2)
            .any(|w| w[0].end_cp != w[1].start_cp)
    {
        return Err(Error::Invalid("hidden projection evidence order"));
    }
    let mut stories = Vec::new();
    let mut output_budget = MAX_SEGMENTS;
    for story in &input.stories {
        let projected = project_story(story, &evidence.spans)?;
        output_budget = output_budget
            .checked_sub(projected.segments.len())
            .ok_or(Error::Limit("hidden projection output segments"))?;
        stories.push(projected);
    }
    let mut unresolved = input.unresolved.clone();
    unresolved.push("unresolved_hidden_properties_and_generated_label_visibility");
    unresolved.sort_unstable();
    unresolved.dedup();
    Ok(Transcription {
        schema_version: input.schema_version,
        status: "partial_visible_transcription",
        unresolved,
        stories,
    })
}

fn state(spans: &[HiddenSpan], start: usize, end: usize) -> Result<Option<bool>> {
    if start >= end {
        return Ok(None);
    }
    let mut index = spans.partition_point(|s| s.end_cp <= start);
    let mut cp = start;
    let mut value = None;
    let mut initial = true;
    while cp < end {
        let span = spans
            .get(index)
            .filter(|s| s.start_cp <= cp)
            .ok_or(Error::Invalid("hidden projection evidence coverage"))?;
        if initial {
            value = span.property.hidden;
            initial = false;
        } else if value != span.property.hidden {
            value = None;
        }
        cp = span.end_cp.min(end);
        index += 1;
    }
    Ok(value)
}

fn project_story(input: &TranscribedStory, spans: &[HiddenSpan]) -> Result<TranscribedStory> {
    let mut output = TranscribedStory {
        kind: input.kind,
        text: String::new(),
        segments: Vec::new(),
        diagnostics: input
            .diagnostics
            .iter()
            .map(|d| Diagnostic {
                code: d.code,
                cp: d.cp,
            })
            .collect(),
        fields_balanced: input.fields_balanced,
        unresolved_control_units: 0,
    };
    let mut prior_cp = input.segments.first().map_or(0, |s| s.start_cp);
    let mut prior_output = 0;
    for segment in &input.segments {
        if segment.start_cp != prior_cp
            || segment.end_cp < segment.start_cp
            || segment.output_start != prior_output
            || segment.output_end < segment.output_start
        {
            return Err(Error::Invalid("hidden projection segment order"));
        }
        let text = input
            .text
            .get(segment.output_start..segment.output_end)
            .ok_or(Error::Invalid("hidden projection output offsets"))?;
        let source_len = segment.end_cp - segment.start_cp;
        if source_len == 0 {
            diagnostic(
                &mut output,
                "generated_label_hidden_visibility_unresolved",
                segment.start_cp,
            );
            append(
                &mut output,
                segment.kind,
                segment.start_cp,
                segment.end_cp,
                text,
            )?;
        } else if text.is_empty() {
            if segment.kind == "field_marker"
                && state(spans, segment.start_cp, segment.end_cp)? == Some(true)
            {
                diagnostic(
                    &mut output,
                    "hidden_field_structure_unresolved",
                    segment.start_cp,
                );
            }
            append(
                &mut output,
                segment.kind,
                segment.start_cp,
                segment.end_cp,
                text,
            )?;
        } else if !segment.kind.starts_with("generated_")
            && text.encode_utf16().count() == source_len
        {
            let mut cp = segment.start_cp;
            for ch in text.chars() {
                let end = cp
                    .checked_add(ch.len_utf16())
                    .ok_or(Error::Limit("hidden projection CP"))?;
                let hidden = state(spans, cp, end)?;
                if hidden.is_none() {
                    diagnostic(&mut output, "hidden_visibility_unresolved", cp);
                }
                let mut utf8 = [0; 4];
                append(
                    &mut output,
                    if hidden == Some(true) {
                        "hidden_text"
                    } else {
                        segment.kind
                    },
                    cp,
                    end,
                    if hidden == Some(true) {
                        ""
                    } else {
                        ch.encode_utf8(&mut utf8)
                    },
                )?;
                cp = end;
            }
        } else {
            let hidden = state(spans, segment.start_cp, segment.end_cp)?;
            if hidden.is_none() {
                diagnostic(
                    &mut output,
                    "generated_text_hidden_visibility_unresolved",
                    segment.start_cp,
                );
            }
            append(
                &mut output,
                if hidden == Some(true) {
                    "hidden_text"
                } else {
                    segment.kind
                },
                segment.start_cp,
                segment.end_cp,
                if hidden == Some(true) { "" } else { text },
            )?;
        }
        prior_cp = segment.end_cp;
        prior_output = segment.output_end;
    }
    if prior_output != input.text.len() {
        return Err(Error::Invalid("hidden projection output coverage"));
    }
    Ok(output)
}
fn diagnostic(story: &mut TranscribedStory, code: &'static str, cp: usize) {
    if story.diagnostics.len() < 256 && !story.diagnostics.iter().any(|d| d.code == code) {
        story.diagnostics.push(Diagnostic { code, cp });
    }
}
fn append(
    story: &mut TranscribedStory,
    kind: &'static str,
    start: usize,
    end: usize,
    text: &str,
) -> Result<()> {
    if story.segments.len() == MAX_SEGMENTS {
        return Err(Error::Limit("hidden projection segments"));
    }
    let output_start = story.text.len();
    story.text.push_str(text);
    if kind == "unresolved_control" {
        story.unresolved_control_units += end - start;
    }
    if let Some(last) = story.segments.last_mut().filter(|s| {
        s.kind == kind
            && !kind.starts_with("generated_")
            && s.end_cp == start
            && s.start_cp != s.end_cp
            && start != end
    }) {
        last.end_cp = end;
        last.output_end = story.text.len();
    } else {
        story.segments.push(SourceSegment {
            kind,
            start_cp: start,
            end_cp: end,
            output_start,
            output_end: story.text.len(),
        });
    }
    Ok(())
}
