//! Shared source-owned literal insertion after field semantics and visibility are resolved.
use crate::office::doc::transcription::{Diagnostic, SourceSegment, TranscribedStory};
use crate::office::doc::{Error, Inspection, Result, Transcription};
use std::collections::BTreeMap;
pub(crate) struct Candidate<'a> {
    pub story: &'static str,
    pub field_range: [usize; 2],
    pub source_range: [usize; 2],
    pub text: &'a str,
}
pub(crate) struct Projection {
    pub transcription: Transcription,
    pub projected: Vec<(usize, [usize; 2])>,
    pub skipped: Vec<usize>,
}
/// Apply all literal components together so annotation offsets describe one final output.
pub(crate) fn project(
    input: &Transcription,
    source: &Inspection,
    candidates: &[Candidate<'_>],
) -> Result<Projection> {
    if candidates.len() > 16_384 {
        return Err(Error::Limit("literal field candidates"));
    }
    let mut stories = Vec::with_capacity(input.stories.len());
    let mut projected = Vec::new();
    let mut skipped = Vec::new();
    let mut work = 8 * 1024 * 1024usize;
    for story in &input.stories {
        let raw = source
            .stories
            .iter()
            .find(|s| s.kind == story.kind)
            .ok_or(Error::Invalid("literal field source story"))?;
        let end = raw
            .start_cp
            .checked_add(raw.utf16_units)
            .ok_or(Error::Invalid("literal field source end"))?;
        let mut cp = raw.start_cp;
        let mut byte = 0;
        for s in &story.segments {
            if s.start_cp > s.end_cp || s.start_cp != cp || s.output_start != byte {
                return Err(Error::Invalid("literal field input partition"));
            }
            story
                .text
                .get(s.output_start..s.output_end)
                .ok_or(Error::Invalid("literal field input output bounds"))?;
            cp = s.end_cp;
            byte = s.output_end;
        }
        if cp != end || byte != story.text.len() {
            return Err(Error::Invalid("literal field input coverage"));
        }
        let mut pending = BTreeMap::new();
        if raw.utf16_units > 16 * 1024 * 1024 {
            return Err(Error::Limit("literal field source units"));
        }
        let raw_units: Vec<u16> = raw.text.encode_utf16().collect();
        if raw_units.len() != raw.utf16_units {
            return Err(Error::Invalid("literal field source units"));
        }
        for (index, field) in candidates
            .iter()
            .enumerate()
            .filter(|(_, f)| f.story == story.kind)
        {
            let [start, finish] = field.source_range;
            if field.field_range[0] < raw.start_cp
                || field.field_range[1] > end
                || start >= finish
                || field.field_range[0] >= start
                || finish >= field.field_range[1]
                || start < raw.start_cp
            {
                return Err(Error::Invalid("literal field base ownership"));
            }
            let units = raw_units
                .get(start - raw.start_cp..finish - raw.start_cp)
                .ok_or(Error::Invalid("literal field base bounds"))?;
            if !units.iter().copied().eq(field.text.encode_utf16()) {
                return Err(Error::Invalid("literal field base source mismatch"));
            }
            let first = story
                .segments
                .partition_point(|s| s.end_cp <= field.field_range[0]);
            let mut valid = true;
            for segment in story.segments[first..]
                .iter()
                .take_while(|s| s.start_cp < field.field_range[1])
            {
                work = work
                    .checked_sub(1)
                    .ok_or(Error::Limit("literal field projection work"))?;
                if segment.output_start != segment.output_end
                    || !matches!(segment.kind, "field_instruction" | "field_marker")
                {
                    valid = false;
                    break;
                }
            }
            if !valid {
                skipped.push(index);
                continue;
            }
            if pending.insert(start, (finish, index)).is_some() {
                return Err(Error::Invalid("literal field duplicate base"));
            }
        }
        let mut previous_end = 0;
        for (start, (finish, _)) in &pending {
            if *start < previous_end {
                return Err(Error::Invalid("literal field overlapping bases"));
            }
            previous_end = *finish;
        }
        let mut text = String::with_capacity(story.text.len());
        let mut segments = Vec::new();
        let mut output_ranges = BTreeMap::<usize, [usize; 2]>::new();
        for s in &story.segments {
            let mut cuts = vec![s.start_cp];
            for (start, (finish, _)) in pending.range(..s.end_cp).rev() {
                if *finish <= s.start_cp {
                    break;
                }
                for cut in [*start, *finish] {
                    if s.start_cp < cut && cut < s.end_cp {
                        cuts.push(cut);
                    }
                }
            }
            cuts.push(s.end_cp);
            cuts.sort_unstable();
            cuts.dedup();
            if s.start_cp == s.end_cp {
                cuts.push(s.end_cp);
            }
            for pair in cuts.windows(2) {
                let [start_cp, end_cp] = [pair[0], pair[1]];
                let selected = pending
                    .range(..=start_cp)
                    .next_back()
                    .filter(|(_, (finish, _))| start_cp < end_cp && end_cp <= *finish);
                let output_start = text.len();
                let kind = if let Some((_, (_, index))) = selected {
                    if s.kind != "field_instruction" || s.output_start != s.output_end {
                        return Err(Error::Invalid("literal field base segment ownership"));
                    }
                    let literal = String::from_utf16(
                        &raw_units[start_cp - raw.start_cp..end_cp - raw.start_cp],
                    )
                    .map_err(|_| Error::Invalid("literal field source scalar boundary"))?;
                    text.push_str(&literal);
                    output_ranges
                        .entry(*index)
                        .and_modify(|r| r[1] = text.len())
                        .or_insert([output_start, text.len()]);
                    "field_display_argument"
                } else {
                    if cuts.len() != 2 && s.output_start != s.output_end {
                        return Err(Error::Invalid("literal field nonempty split"));
                    }
                    text.push_str(&story.text[s.output_start..s.output_end]);
                    s.kind
                };
                if text.len() > 64 * 1024 * 1024 || segments.len() >= 262_144 {
                    return Err(Error::Limit("literal field projected output"));
                }
                segments.push(SourceSegment {
                    kind,
                    start_cp,
                    end_cp,
                    output_start,
                    output_end: text.len(),
                });
            }
        }
        for (_, (_, index)) in pending {
            let range = output_ranges
                .remove(&index)
                .ok_or(Error::Invalid("literal field missing projected base"))?;
            if story.kind != candidates[index].story
                || &text[range[0]..range[1]] != candidates[index].text
            {
                return Err(Error::Invalid("literal field projected base mismatch"));
            }
            projected.push((index, range));
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
                .collect(),
            fields_balanced: story.fields_balanced,
            unresolved_control_units: story.unresolved_control_units,
        });
    }
    Ok(Projection {
        transcription: Transcription {
            schema_version: input.schema_version,
            status: input.status,
            unresolved: input.unresolved.clone(),
            stories,
        },
        projected,
        skipped,
    })
}
