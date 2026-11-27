use super::*;
use crate::office::doc::transcription::{Diagnostic, SourceSegment, TranscribedStory};
use std::collections::{BTreeMap, BTreeSet};

/// Insert proven current form values after accepted revision and hidden-text projection.
/// Zero-width generated_form_value anchors refer to FormField's full source field range.
/// Existing cached output must be empty, preventing duplication or replacement of source text.
pub fn apply_forms(input: &Transcription, forms: &FormInspection) -> Result<Transcription> {
    let mut stories = Vec::with_capacity(input.stories.len());
    let mut added = 0;
    let mut unresolved_fields = 0;
    for story in &input.stories {
        let mut end_cp = None;
        for segment in &story.segments {
            if segment.start_cp > segment.end_cp {
                return Err(Error::Invalid("form input source bounds"));
            }
            if segment.start_cp != segment.end_cp {
                if end_cp.is_some_and(|end| end != segment.start_cp) {
                    return Err(Error::Invalid("form input source partition"));
                }
                end_cp = Some(segment.end_cp);
            }
        }
        let mut pending = BTreeMap::new();
        let mut skipped = Vec::new();
        for field in forms
            .fields
            .iter()
            .filter(|f| f.story == story.kind && f.eligible)
        {
            let Some(data) = field.data.as_ref() else {
                continue;
            };
            // Checkbox glyphs normalize the effective state; raw undefined state remains metadata.
            let value = match data.kind {
                "checkbox" => {
                    let checked = match data.raw_current_state {
                        0 | 25 => false,
                        1 => true,
                        _ => return Err(Error::Invalid("form checkbox current state")),
                    };
                    if data.effective_checked != Some(checked) {
                        return Err(Error::Invalid("form checkbox effective state"));
                    }
                    if checked { "☒" } else { "☐" }
                }
                "dropdown" => match data.selected_text.as_deref().filter(|s| !s.is_empty()) {
                    Some(value) => value,
                    None => continue,
                },
                _ => continue,
            };
            if value.chars().any(char::is_control) {
                return Err(Error::Unsupported("form selected value controls"));
            }
            let start = story
                .segments
                .partition_point(|s| s.end_cp <= field.field_start_cp);
            let mut cp = field.field_start_cp;
            let mut owned_empty = true;
            for s in story.segments[start..]
                .iter()
                .take_while(|s| s.start_cp < field.field_end_cp)
            {
                if s.start_cp == s.end_cp {
                    continue;
                }
                if s.start_cp > cp {
                    return Err(Error::Invalid("form projection source coverage"));
                }
                let crosses_boundary = s.start_cp < cp || s.end_cp > field.field_end_cp;
                if (crosses_boundary && s.kind != "field_marker") || s.output_start != s.output_end
                {
                    owned_empty = false;
                    break;
                }
                cp = s.end_cp.min(field.field_end_cp);
            }
            if !owned_empty {
                unresolved_fields += 1;
                skipped.push(Diagnostic {
                    code: "form_value_existing_output_or_ownership",
                    cp: field.field_start_cp,
                });
                continue;
            }
            if cp != field.field_end_cp {
                return Err(Error::Invalid("form projection source coverage"));
            }
            if pending
                .insert(field.field_start_cp, (field, value))
                .is_some()
            {
                return Err(Error::Invalid("duplicate form projection anchor"));
            }
        }
        // Only queued, independently owned fields can introduce cuts in empty marker spans.
        let boundaries: BTreeSet<_> = pending
            .values()
            .flat_map(|(field, _)| [field.field_start_cp, field.field_end_cp])
            .collect();
        let source_segments = split_markers(&story.segments, &boundaries, pending.len())?;
        let mut text = String::new();
        let mut segments = Vec::with_capacity(source_segments.len() + pending.len());
        let mut expected_byte = 0;
        for segment in &source_segments {
            if segment.kind == "generated_form_value" {
                return Err(Error::Invalid("form projection already applied"));
            }
            if segment.output_start != expected_byte {
                return Err(Error::Invalid("form input output partition"));
            }
            let content = story
                .text
                .get(segment.output_start..segment.output_end)
                .ok_or(Error::Invalid("form input output bounds"))?;
            expected_byte = segment.output_end;
            if segment.start_cp != segment.end_cp
                && let Some((_, value)) = pending.remove(&segment.start_cp)
            {
                let output_start = text.len();
                text.push_str(value);
                segments.push(SourceSegment {
                    kind: "generated_form_value",
                    start_cp: segment.start_cp,
                    end_cp: segment.start_cp,
                    output_start,
                    output_end: text.len(),
                });
                added += 1;
            }
            let start = text.len();
            text.push_str(content);
            if text.len() > 64 * 1024 * 1024 {
                return Err(Error::Limit("form output bytes"));
            }
            segments.push(SourceSegment {
                kind: segment.kind,
                start_cp: segment.start_cp,
                end_cp: segment.end_cp,
                output_start: start,
                output_end: text.len(),
            });
        }
        if expected_byte != story.text.len() || !pending.is_empty() {
            return Err(Error::Invalid("form projection boundary"));
        }
        stories.push(TranscribedStory {
            kind: story.kind,
            text,
            segments,
            diagnostics: story
                .diagnostics
                .iter()
                .map(|d| Diagnostic {
                    code: d.code,
                    cp: d.cp,
                })
                .chain(skipped)
                .collect(),
            fields_balanced: story.fields_balanced,
            unresolved_control_units: story.unresolved_control_units,
        });
    }
    let mut unresolved = input.unresolved.clone();
    if unresolved_fields > 0 {
        unresolved.push("form_value_existing_output_or_ownership");
    }
    if added > 0 {
        unresolved.push("form_value_visual_appearance_unresolved");
    }
    Ok(Transcription {
        schema_version: input.schema_version,
        status: input.status,
        unresolved,
        stories,
    })
}

// Cuts preserve the original marker's kind and empty output interval, plus every source CP.
pub(crate) fn split_markers(
    input: &[SourceSegment],
    boundaries: &BTreeSet<usize>,
    generated: usize,
) -> Result<Vec<SourceSegment>> {
    if input
        .len()
        .checked_add(generated)
        .is_none_or(|n| n > 262_144)
    {
        return Err(Error::Limit("form projection segments"));
    }
    let mut output = Vec::with_capacity(input.len());
    for segment in input {
        let mut start = segment.start_cp;
        if segment.kind == "field_marker"
            && segment.output_start == segment.output_end
            && start < segment.end_cp
        {
            for &cut in boundaries.range((
                std::ops::Bound::Excluded(start),
                std::ops::Bound::Excluded(segment.end_cp),
            )) {
                output.push(SourceSegment {
                    kind: segment.kind,
                    start_cp: start,
                    end_cp: cut,
                    output_start: segment.output_start,
                    output_end: segment.output_end,
                });
                start = cut;
                if output.len() + generated >= 262_144 {
                    return Err(Error::Limit("form projection segments"));
                }
            }
        }
        output.push(SourceSegment {
            kind: segment.kind,
            start_cp: start,
            end_cp: segment.end_cp,
            output_start: segment.output_start,
            output_end: segment.output_end,
        });
        if output
            .len()
            .checked_add(generated)
            .is_none_or(|n| n > 262_144)
        {
            return Err(Error::Limit("form projection segments"));
        }
    }
    Ok(output)
}
