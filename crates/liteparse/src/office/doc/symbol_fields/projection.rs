use super::*;
use crate::office::doc::{
    Transcription,
    transcription::{SourceSegment, TranscribedStory},
};
use std::collections::{BTreeMap, BTreeSet};

/// Insert generated values at their indexed field anchors while retaining all source segments.
/// This consumes inspector-produced eligibility for the same source and accepted view.
/// Field envelopes are rechecked; font/revision evidence remains the inspector's responsibility.
/// Field instruction and marker spans must own no existing output; cached values are preserved.
pub fn apply_symbol_fields(
    input: &Transcription,
    source: &Inspection,
    fields: &SymbolFields,
) -> Result<Transcription> {
    if fields.fields.len() > 16_384 {
        return Err(Error::Limit("symbol field projection count"));
    }
    let source_kinds: BTreeSet<_> = source.stories.iter().map(|s| s.kind).collect();
    let input_kinds: BTreeSet<_> = input.stories.iter().map(|s| s.kind).collect();
    if source_kinds.len() != source.stories.len()
        || input_kinds.len() != input.stories.len()
        || source_kinds != input_kinds
        || fields
            .fields
            .iter()
            .any(|f| !source_kinds.contains(f.story))
    {
        return Err(Error::Invalid("symbol field story membership"));
    }
    let mut work = 4 * 1024 * 1024usize;
    let mut stories = Vec::with_capacity(input.stories.len());
    let mut added = false;
    for story in &input.stories {
        let raw = source
            .stories
            .iter()
            .find(|s| s.kind == story.kind)
            .ok_or(Error::Invalid("symbol field source story"))?;
        let end = raw
            .start_cp
            .checked_add(raw.utf16_units)
            .ok_or(Error::Invalid("symbol field source end"))?;
        let mut cp = raw.start_cp;
        let mut byte = 0;
        for s in &story.segments {
            if s.kind == "generated_symbol_field"
                || s.start_cp != cp
                || s.end_cp < cp
                || s.output_start != byte
                || story.text.get(s.output_start..s.output_end).is_none()
            {
                return Err(Error::Invalid("symbol field input partition"));
            }
            cp = s.end_cp;
            byte = s.output_end;
        }
        if cp != end || byte != story.text.len() {
            return Err(Error::Invalid("symbol field input coverage"));
        }
        if raw.utf16_units > 16 * 1024 * 1024 {
            return Err(Error::Limit("symbol field source units"));
        }
        let units: Vec<_> = raw.text.encode_utf16().collect();
        if units.len() != raw.utf16_units {
            return Err(Error::Invalid("symbol field source units"));
        }
        let mut pending = BTreeMap::new();
        let mut boundaries = BTreeSet::new();
        for field in fields
            .fields
            .iter()
            .filter(|f| f.story == story.kind && f.eligible)
        {
            let [start, finish] = field.source_range;
            if start < raw.start_cp || finish > end || finish <= start {
                return Err(Error::Invalid("symbol field projection range"));
            }
            let owned = &units[start - raw.start_cp..finish - raw.start_cp];
            let instruction = owned
                .strip_prefix(&[0x13])
                .and_then(|s| s.strip_suffix(&[0x15]))
                .ok_or(Error::Invalid("symbol field source delimiter identity"))?;
            let instruction = instruction.strip_suffix(&[0x14]).unwrap_or(instruction);
            if !instruction
                .iter()
                .copied()
                .eq(field.instruction.encode_utf16())
            {
                return Err(Error::Invalid("symbol field source instruction identity"));
            }
            let value = field
                .unicode
                .ok_or(Error::Invalid("symbol field projected character"))?;
            if value.is_control() {
                return Err(Error::Unsupported("symbol field control output"));
            }
            let first = story.segments.partition_point(|s| s.end_cp <= start);
            let mut cp = start;
            for s in story.segments[first..]
                .iter()
                .take_while(|s| s.start_cp < finish)
            {
                work = work
                    .checked_sub(1)
                    .ok_or(Error::Limit("symbol field projection work"))?;
                if s.start_cp > cp
                    || s.output_start != s.output_end
                    || !matches!(s.kind, "field_marker" | "field_instruction")
                    || ((s.start_cp < start || s.end_cp > finish) && s.kind != "field_marker")
                {
                    return Err(Error::Unsupported(
                        "symbol field existing output or ownership",
                    ));
                }
                cp = s.end_cp.min(finish);
            }
            if cp != finish {
                return Err(Error::Invalid("symbol field projection coverage"));
            }
            if pending.insert(start, (finish, value)).is_some() {
                return Err(Error::Invalid("symbol field duplicate anchor"));
            }
            boundaries.extend([start, finish]);
        }
        let mut previous_end = raw.start_cp;
        for (&start, &(finish, _)) in &pending {
            if start < previous_end {
                return Err(Error::Invalid("symbol field overlapping anchors"));
            }
            previous_end = finish;
        }
        let split = crate::office::doc::form_fields::projection::split_markers(
            &story.segments,
            &boundaries,
            pending.len(),
        )?;
        let mut text = String::new();
        let mut segments = Vec::with_capacity(split.len() + pending.len());
        for s in split {
            if s.start_cp != s.end_cp
                && let Some((_, value)) = pending.remove(&s.start_cp)
            {
                let start = text.len();
                text.push(value);
                segments.push(SourceSegment {
                    kind: "generated_symbol_field",
                    start_cp: s.start_cp,
                    end_cp: s.start_cp,
                    output_start: start,
                    output_end: text.len(),
                });
                added = true;
            }
            let start = text.len();
            text.push_str(&story.text[s.output_start..s.output_end]);
            if text.len() > 64 * 1024 * 1024 {
                return Err(Error::Limit("symbol field output bytes"));
            }
            segments.push(SourceSegment {
                kind: s.kind,
                start_cp: s.start_cp,
                end_cp: s.end_cp,
                output_start: start,
                output_end: text.len(),
            });
        }
        if !pending.is_empty() {
            return Err(Error::Invalid("symbol field missing anchor"));
        }
        stories.push(TranscribedStory {
            kind: story.kind,
            text,
            segments,
            diagnostics: story
                .diagnostics
                .iter()
                .map(|d| crate::office::doc::transcription::Diagnostic {
                    code: d.code,
                    cp: d.cp,
                })
                .collect(),
            fields_balanced: story.fields_balanced,
            unresolved_control_units: story.unresolved_control_units,
        });
    }
    let mut unresolved = input.unresolved.clone();
    if added {
        unresolved.push("symbol_field_glyph_appearance_unresolved");
    }
    Ok(Transcription {
        schema_version: input.schema_version,
        status: input.status,
        unresolved,
        stories,
    })
}
