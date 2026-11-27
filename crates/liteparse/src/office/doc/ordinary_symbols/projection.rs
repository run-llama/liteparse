use super::{MAX, OrdinarySymbols};
use crate::office::doc::{
    Error, Inspection, Result, Transcription,
    transcription::{Diagnostic, SourceSegment, TranscribedStory},
};

fn append(
    out: &mut TranscribedStory,
    kind: &'static str,
    start: usize,
    end: usize,
    text: &str,
) -> Result<()> {
    if out.segments.len() >= MAX || out.text.len().saturating_add(text.len()) > 64 * 1024 * 1024 {
        return Err(Error::Limit("ordinary symbol output"));
    }
    let offset = out.text.len();
    out.text.push_str(text);
    out.segments.push(SourceSegment {
        kind,
        start_cp: start,
        end_cp: end,
        output_start: offset,
        output_end: out.text.len(),
    });
    Ok(())
}
/// Replace only resolved one-CP text characters; preserve all generated and suppressed atoms.
/// Call before projections that publish byte ranges, including form and ruby annotations.
pub fn apply(
    source: &Inspection,
    input: &Transcription,
    evidence: &OrdinarySymbols,
) -> Result<Transcription> {
    if input.stories.len() > 8
        || source.stories.len() != input.stories.len()
        || evidence.spans.len() > MAX
    {
        return Err(Error::Limit("ordinary symbol projection"));
    }
    if evidence
        .spans
        .windows(2)
        .any(|w| w[0].source_cp >= w[1].source_cp)
    {
        return Err(Error::Invalid("ordinary symbol evidence order"));
    }
    let mut stories = Vec::new();
    let mut units_total = 0usize;
    let mut bytes_total = 0usize;
    let mut segment_total = 0usize;
    let mut applied = 0usize;
    for story in &input.stories {
        let raw = source
            .stories
            .iter()
            .find(|s| s.kind == story.kind)
            .ok_or(Error::Invalid("ordinary symbol source story"))?;
        units_total = units_total
            .checked_add(raw.utf16_units)
            .ok_or(Error::Limit("ordinary symbol source units"))?;
        bytes_total = bytes_total
            .checked_add(story.text.len())
            .ok_or(Error::Limit("ordinary symbol input bytes"))?;
        segment_total = segment_total
            .checked_add(story.segments.len())
            .ok_or(Error::Limit("ordinary symbol segments"))?;
        if units_total > 16 * 1024 * 1024 || bytes_total > 64 * 1024 * 1024 || segment_total > MAX {
            return Err(Error::Limit("ordinary symbol input"));
        }
        let units: Vec<_> = raw.text.encode_utf16().collect();
        if units.len() != raw.utf16_units {
            return Err(Error::Invalid("ordinary symbol source length"));
        }
        let mut out = TranscribedStory {
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
        let mut cp = raw.start_cp;
        let mut output = 0;
        for s in &story.segments {
            if s.start_cp != cp || s.end_cp < s.start_cp || s.output_start != output {
                return Err(Error::Invalid("ordinary symbol input partition"));
            }
            let text = story
                .text
                .get(s.output_start..s.output_end)
                .ok_or(Error::Invalid("ordinary symbol input bytes"))?;
            cp = s.end_cp;
            output = s.output_end;
            let first = evidence.spans.partition_point(|e| e.source_cp < s.start_cp);
            let candidates: Vec<_> = evidence.spans[first..]
                .iter()
                .take_while(|e| e.source_cp < s.end_cp)
                .filter(|e| e.unicode.is_some() && e.unresolved.is_none())
                .collect();
            if candidates.is_empty() {
                append(&mut out, s.kind, s.start_cp, s.end_cp, text)?;
                continue;
            }
            if s.kind != "text" || text.encode_utf16().count() != s.end_cp - s.start_cp {
                return Err(Error::Invalid(
                    "ordinary symbol resolved projection mapping",
                ));
            }
            let mut position = s.start_cp;
            let mut chunk_cp = position;
            let mut chunk_byte = 0;
            let mut next = 0;
            for (byte, ch) in text.char_indices() {
                if let Some(e) = candidates.get(next).filter(|e| e.source_cp == position) {
                    let raw_index = position
                        .checked_sub(raw.start_cp)
                        .ok_or(Error::Invalid("ordinary symbol raw CP"))?;
                    if e.story != story.kind
                        || ch as u32 != u32::from(e.source_code)
                        || ch.len_utf16() != 1
                        || units.get(raw_index) != Some(&e.source_code)
                    {
                        return Err(Error::Invalid("ordinary symbol source character"));
                    }
                    if byte > chunk_byte {
                        append(
                            &mut out,
                            s.kind,
                            chunk_cp,
                            position,
                            &text[chunk_byte..byte],
                        )?;
                    }
                    let mapped = e
                        .unicode
                        .ok_or(Error::Invalid("ordinary symbol mapped character"))?;
                    append(
                        &mut out,
                        "ordinary_font_symbol",
                        position,
                        position + 1,
                        mapped.encode_utf8(&mut [0; 4]),
                    )?;
                    chunk_cp = position + 1;
                    chunk_byte = byte + ch.len_utf8();
                    next += 1;
                    applied += 1;
                }
                position += ch.len_utf16();
            }
            if next != candidates.len() {
                return Err(Error::Invalid("ordinary symbol unmatched CP"));
            }
            if chunk_byte < text.len() {
                append(&mut out, s.kind, chunk_cp, s.end_cp, &text[chunk_byte..])?;
            }
        }
        if cp != raw.start_cp + raw.utf16_units || output != story.text.len() {
            return Err(Error::Invalid("ordinary symbol story coverage"));
        }
        stories.push(out);
    }
    if applied
        != evidence
            .spans
            .iter()
            .filter(|e| e.unicode.is_some() && e.unresolved.is_none())
            .count()
    {
        return Err(Error::Invalid("ordinary symbol unmatched evidence"));
    }
    Ok(Transcription {
        stories,
        unresolved: input.unresolved.clone(),
        schema_version: input.schema_version,
        status: input.status,
    })
}
