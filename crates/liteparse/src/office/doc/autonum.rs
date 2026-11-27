//! Conservative generation of default body AUTONUM fields after revision projection.
//!
//! ECMA-376 Part 4 §14.10.4.1 defines ascending paragraph labels and the default period.
//! Heading resets, switches and binary list emulation require additional resolution.
use crate::office::doc::{
    Error, Inspection, Result, Story, bytes,
    compound::Compound,
    document::fib,
    field_inspection::FieldInspection,
    fields::FieldRange,
    formatting::{PropertyModifier, decode_character_modifiers},
    formatting_inspection::FormattingInspection,
    paragraph_formatting::ParagraphInspection,
    styles::Stylesheet,
    transcription::{Diagnostic, SourceSegment, TranscribedStory, Transcription},
    u16_at, u32_at,
    visibility::RevisionEvidence,
};
use serde::Serialize;
use std::collections::BTreeMap;

const MAX_FIELDS: usize = 65_536;
const MAX_SEGMENTS: usize = 262_144;
const MAX_UNITS: usize = 16 * 1024 * 1024;

/// Evidence from fixed list headers; level formatting remains undecoded.
#[derive(Debug, Serialize)]
pub struct ListEmulationEvidence {
    pub list_count: usize,
    pub override_count: usize,
    pub has_autonum_emulation: bool,
}

/// Read FibRgFcLcb97 pairs 73 and 74; zero-length tables have undefined offsets.
pub fn inspect_list_emulation(data: &[u8]) -> Result<ListEmulationEvidence> {
    let container = Compound::open(data)?;
    let word = container.stream("WordDocument")?;
    let (_, table_name, _, _, _) = fib(&word)?;
    let table = container.stream(table_name)?;
    let read = |index: usize| {
        let length = u32_at(&word, 158 + index * 8)? as usize;
        if length == 0 {
            Ok(&[][..])
        } else {
            bytes(&table, u32_at(&word, 154 + index * 8)? as usize, length)
        }
    };
    decode_list_emulation(read(73)?, read(74)?)
}

/// Inspect PlfLst's fixed LSTF array and PlfLfo's fixed LFO array.
/// LSTF.fAutoNum and LFO.ibstFltAutoNum identify source field-list customization.
/// <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/ba4375ad-a629-40ad-b312-c5b6889584dd>
pub fn decode_list_emulation(lst: &[u8], lfo: &[u8]) -> Result<ListEmulationEvidence> {
    let count = if lst.is_empty() {
        0
    } else {
        u16_at(lst, 0)? as usize
    };
    if count > 0x7fff || (!lst.is_empty() && lst.len() != 2 + 28 * count) {
        return Err(Error::Invalid("AUTONUM list header array"));
    }
    let mut lists = BTreeMap::new();
    let mut custom = false;
    for i in 0..count {
        let record = bytes(lst, 2 + i * 28, 28)?;
        let id = u32_at(record, 0)?;
        let autonum = record[26] & 4 != 0;
        if id == u32::MAX || lists.insert(id, autonum).is_some() {
            return Err(Error::Invalid("AUTONUM list identifier"));
        }
        custom |= autonum;
    }
    let overrides = if lfo.is_empty() {
        0
    } else {
        u32_at(lfo, 0)? as usize
    };
    if overrides > MAX_FIELDS {
        return Err(Error::Limit("AUTONUM list overrides"));
    }
    if !lfo.is_empty() {
        bytes(lfo, 4, overrides * 16)?;
    }
    for i in 0..overrides {
        let record = bytes(lfo, 4 + i * 16, 16)?;
        let is_field = match record[13] {
            0 | 0xff => false,
            0xfc..=0xfe => true,
            _ => return Err(Error::Invalid("AUTONUM override field type")),
        };
        if lists.get(&u32_at(record, 0)?) != Some(&is_field) || record[12] > 9 {
            return Err(Error::Invalid("AUTONUM override list reference"));
        }
        custom |= is_field;
    }
    Ok(ListEmulationEvidence {
        list_count: count,
        override_count: overrides,
        has_autonum_emulation: custom,
    })
}

pub struct AutonumContext<'a> {
    pub source: &'a Inspection,
    pub fields: &'a FieldInspection,
    pub paragraphs: &'a ParagraphInspection,
    pub styles: &'a Stylesheet,
    pub formatting: &'a FormattingInspection,
    pub revisions: &'a RevisionEvidence,
    pub lists: &'a ListEmulationEvidence,
}

/// Generate only the independently resolved default Normal-style body sequence.
/// Any unsupported AUTONUM blocks that sequence so later labels cannot silently drift.
/// A generated segment owns the original whole field range, with UTF-8 output offsets.
pub fn apply_autonum(input: &Transcription, context: &AutonumContext<'_>) -> Result<Transcription> {
    let mut output = Transcription {
        schema_version: input.schema_version,
        status: "partial_transcription",
        unresolved: input.unresolved.clone(),
        stories: Vec::new(),
    };
    output
        .unresolved
        .push("autonum_heading_switch_and_list_emulation");
    for story in &input.stories {
        let source = context
            .source
            .stories
            .iter()
            .find(|s| s.kind == story.kind)
            .ok_or(Error::Invalid("AUTONUM source story"))?;
        validate_projection(story, source)?;
        let mut copied = copy_story(story);
        if story.kind == "body" {
            match plan(source, context)? {
                Plan::Ready(fields) if !fields.is_empty() => {
                    if fields.iter().all(|f| suppressed(story, f)) {
                        copied = generate(story, &fields)?;
                    } else {
                        diagnostic(
                            &mut copied,
                            "autonum_projection_unresolved",
                            source.start_cp,
                        );
                    }
                }
                Plan::Blocked(code, cp) => diagnostic(&mut copied, code, cp),
                _ => {}
            }
        }
        output.stories.push(copied);
    }
    Ok(output)
}

enum Plan<'a> {
    Ready(Vec<&'a FieldRange>),
    Blocked(&'static str, usize),
}

fn plan<'a>(source: &Story, c: &'a AutonumContext<'_>) -> Result<Plan<'a>> {
    if source.utf16_units > MAX_UNITS || source.text.encode_utf16().count() != source.utf16_units {
        return Err(Error::Limit("AUTONUM source units"));
    }
    if c.fields.binary_version != c.source.binary_version
        || c.paragraphs.binary_version != c.source.binary_version
        || c.formatting.binary_version != c.source.binary_version
        || c.paragraphs.spans.len() > MAX_SEGMENTS
        || c.revisions.spans.len() > MAX_SEGMENTS
        || c.formatting.pieces.len() > MAX_SEGMENTS
    {
        return Err(Error::Invalid("AUTONUM metadata alignment or limit"));
    }
    if c.paragraphs.spans.iter().any(|s| s.start_cp >= s.end_cp)
        || c.paragraphs
            .spans
            .windows(2)
            .any(|s| s[0].end_cp > s[1].start_cp)
        || c.revisions.spans.iter().any(|s| s.start_cp >= s.end_cp)
        || c.revisions
            .spans
            .windows(2)
            .any(|s| s[0].end_cp != s[1].start_cp)
        || c.formatting.pieces.iter().any(|s| s.start_cp >= s.end_cp)
        || c.formatting
            .pieces
            .windows(2)
            .any(|s| s[0].end_cp != s[1].start_cp)
    {
        return Err(Error::Invalid("AUTONUM metadata source order"));
    }
    let units: Vec<_> = source.text.encode_utf16().collect();
    let blocked = |code| Plan::Blocked(code, source.start_cp);
    let Some(row) = c.fields.stories.iter().find(|s| s.kind == source.kind) else {
        return Ok(blocked("autonum_field_metadata_unavailable"));
    };
    if row.start_cp != source.start_cp || row.utf16_units != source.utf16_units {
        return Err(Error::Invalid("AUTONUM field story alignment"));
    }
    let Some(table) = &row.table else {
        return Ok(blocked("autonum_field_metadata_unavailable"));
    };
    if table.ranges.len() > MAX_FIELDS {
        return Err(Error::Limit("AUTONUM fields"));
    }
    let mut fields = Vec::new();
    for field in &table.ranges {
        let instruction = source_units(
            &units,
            source.start_cp,
            field.instruction.start,
            field.instruction.end,
        )?;
        if instruction.len() > 4096 {
            return Ok(blocked("autonum_instruction_limit"));
        }
        let text = String::from_utf16(instruction)
            .map_err(|_| Error::Invalid("AUTONUM instruction boundary"))?;
        if text
            .trim_start()
            .get(..7)
            .is_some_and(|s| s.eq_ignore_ascii_case("AUTONUM"))
        {
            fields.push(field);
            if field.parent_begin_cp.is_some()
                || field.result.is_some()
                || !text.trim().eq_ignore_ascii_case("AUTONUM")
            {
                return Ok(Plan::Blocked(
                    "autonum_field_form_unsupported",
                    field.source.start,
                ));
            }
        }
    }
    if fields.is_empty() {
        return Ok(Plan::Ready(fields));
    }
    if !table.topology_valid || table.unindexed_marker_units != 0 || !table.diagnostics.is_empty() {
        return Ok(blocked("autonum_field_topology_unresolved"));
    }
    if c.lists.has_autonum_emulation {
        return Ok(blocked("autonum_list_emulation_unsupported"));
    }
    fields.sort_by_key(|f| f.source.start);
    if fields
        .windows(2)
        .any(|f| f[0].source.end > f[1].source.start)
    {
        return Err(Error::Invalid("AUTONUM overlapping fields"));
    }
    let end = source
        .start_cp
        .checked_add(source.utf16_units)
        .ok_or(Error::Invalid("AUTONUM source overflow"))?;
    // Unknown or deleted body text can alter field order and paragraph boundaries.
    let mut covered = source.start_cp;
    for span in c
        .revisions
        .spans
        .iter()
        .filter(|s| s.end_cp > source.start_cp && s.start_cp < end)
    {
        if span.start_cp > covered
            || span.end_cp <= covered
            || span.properties.deleted != Some(false)
            || span.properties.field_hidden != Some(false)
        {
            return Ok(blocked("autonum_revision_unresolved"));
        }
        covered = span.end_cp.min(end);
    }
    if covered != end {
        return Ok(blocked("autonum_revision_unresolved"));
    }
    let mut covered = source.start_cp;
    for p in c
        .formatting
        .pieces
        .iter()
        .filter(|p| p.end_cp > source.start_cp && p.start_cp < end)
    {
        if p.start_cp > covered || p.end_cp <= covered || p.raw_prm != 0 {
            return Ok(blocked("autonum_piece_properties_unsupported"));
        }
        covered = p.end_cp.min(end);
    }
    if covered != end {
        return Ok(blocked("autonum_piece_properties_unsupported"));
    }
    let Some(style) = c.styles.styles.first().and_then(Option::as_ref) else {
        return Ok(blocked("autonum_style_unsupported"));
    };
    if style.index != 0
        || style.builtin_id != 0
        || style.kind != 1
        || style.base_style.is_some()
        || style.revision_marked
        || !style.raw_revision.is_empty()
        || style
            .properties
            .iter()
            .filter(|p| p.kind == "paragraph")
            .count()
            != 1
        || style
            .properties
            .iter()
            .filter(|p| p.kind == "paragraph")
            .any(|p| p.bytes != [0, 0])
    {
        return Ok(blocked("autonum_style_unsupported"));
    }
    for style in [
        Some(style),
        c.styles.styles.get(10).and_then(Option::as_ref),
    ]
    .into_iter()
    .flatten()
    {
        if style.base_style.is_some() || style.revision_marked || !style.raw_revision.is_empty() {
            return Ok(blocked("autonum_hidden_style_unresolved"));
        }
        for group in style.properties.iter().filter(|p| p.kind == "character") {
            let modifiers = decode_character_modifiers(&group.bytes)?;
            if !modifiers.iter().all(visibility_neutral) {
                return Ok(blocked("autonum_hidden_style_unresolved"));
            }
        }
    }
    if c.formatting.character_spans.len() > MAX_SEGMENTS
        || c.formatting
            .character_spans
            .iter()
            .any(|s| s.start_cp >= s.end_cp)
        || c.formatting
            .character_spans
            .windows(2)
            .any(|s| s[0].end_cp != s[1].start_cp)
    {
        return Err(Error::Invalid("AUTONUM character span order or limit"));
    }
    let mut last_paragraph_end = None;
    for field in &fields {
        let raw = source_units(
            &units,
            source.start_cp,
            field.source.start,
            field.source.end,
        )?;
        if raw.first() != Some(&0x13)
            || raw.last() != Some(&0x15)
            || field.instruction.start != field.source.start + 1
            || field.instruction.end + 1 != field.source.end
        {
            return Err(Error::Invalid("AUTONUM field source markers"));
        }
        for cp in [field.source.start, field.source.end - 1] {
            let index = c.revisions.spans.partition_point(|s| s.end_cp <= cp);
            if !c
                .revisions
                .spans
                .get(index)
                .is_some_and(|s| s.start_cp <= cp && s.properties.special == Some(true))
            {
                return Ok(Plan::Blocked("autonum_marker_properties_unresolved", cp));
            }
        }
        let start = field.source.start - source.start_cp;
        let relative_end = if c.paragraphs.status
            == "reference_expanded_endpoint_selected_paragraphs"
        {
            c.paragraphs
                .spans
                .get(
                    c.paragraphs
                        .spans
                        .partition_point(|p| p.end_cp <= field.source.start),
                )
                .filter(|p| {
                    p.start_cp <= field.source.start && p.end_cp <= source.start_cp + units.len()
                })
                .map(|p| p.end_cp - field.source.start - 1)
        } else {
            units[start..].iter().position(|&u| u == 13 || u == 7)
        };
        let Some(relative_end) = relative_end else {
            return Ok(Plan::Blocked(
                "autonum_paragraph_unresolved",
                field.source.start,
            ));
        };
        let paragraph_end = source.start_cp + start + relative_end;
        if paragraph_end < field.source.end
            || last_paragraph_end == Some(paragraph_end)
            || units[start + relative_end] != 13
        {
            return Ok(Plan::Blocked(
                "autonum_paragraph_unsupported",
                field.source.start,
            ));
        }
        last_paragraph_end = Some(paragraph_end);
        for (start, end) in [
            (field.source.start, field.source.end),
            (paragraph_end, paragraph_end + 1),
        ] {
            if !visible_character_range(c.formatting, start, end) {
                return Ok(Plan::Blocked(
                    "autonum_hidden_character_properties_unresolved",
                    start,
                ));
            }
        }
        let index = c
            .paragraphs
            .spans
            .partition_point(|s| s.end_cp <= paragraph_end);
        let Some(p) = c
            .paragraphs
            .spans
            .get(index)
            .filter(|s| s.start_cp <= paragraph_end && s.end_cp == paragraph_end + 1)
        else {
            return Ok(Plan::Blocked("autonum_paragraph_unresolved", paragraph_end));
        };
        // PFKeepFollow only changes pagination; all other direct modifiers stay unresolved.
        if p.style_index != Some(0)
            || (!p.raw_modifiers.is_empty() && p.raw_modifiers != [6, 0x24, 1])
        {
            return Ok(Plan::Blocked(
                "autonum_paragraph_properties_unsupported",
                paragraph_end,
            ));
        }
    }
    Ok(Plan::Ready(fields))
}

// These operators alter fonts, language, appearance or source metadata only.
// CFVanish, character-style selection, property resets and unknown operators stay unresolved.
fn visibility_neutral(p: &PropertyModifier) -> bool {
    matches!(
        p.sprm,
        0x6a03
            | 0x6815
            | 0x6816
            | 0x6817
            | 0x0855
            | 0x0835
            | 0x0836
            | 0x0837
            | 0x0838
            | 0x0839
            | 0x083a
            | 0x083b
            | 0x2a3e
            | 0x8840
            | 0x2a42
            | 0x4a43
            | 0x4845
            | 0x2a48
            | 0x484b
            | 0x4a4f
            | 0x4a50
            | 0x4a51
            | 0x4852
            | 0x2a53
            | 0x0854
            | 0x485f
            | 0x4a61
            | 0x486d
            | 0x486e
            | 0x4873
            | 0x4874
    )
}

fn visible_character_range(formatting: &FormattingInspection, start: usize, end: usize) -> bool {
    let index = formatting
        .character_spans
        .partition_point(|s| s.end_cp <= start);
    let mut cp = start;
    for span in formatting.character_spans[index..]
        .iter()
        .take_while(|s| s.start_cp < end)
    {
        if span.start_cp > cp || !span.properties.iter().all(visibility_neutral) {
            return false;
        }
        cp = span.end_cp.min(end);
    }
    cp == end
}

fn source_units(units: &[u16], origin: usize, start: usize, end: usize) -> Result<&[u16]> {
    let start = start
        .checked_sub(origin)
        .ok_or(Error::Invalid("AUTONUM field CP"))?;
    let end = end
        .checked_sub(origin)
        .ok_or(Error::Invalid("AUTONUM field CP"))?;
    units
        .get(start..end)
        .ok_or(Error::Invalid("AUTONUM field CP"))
}

fn validate_projection(story: &TranscribedStory, source: &Story) -> Result<()> {
    if story.segments.len() > MAX_SEGMENTS || story.text.len() > MAX_UNITS * 4 {
        return Err(Error::Limit("AUTONUM source segments"));
    }
    let mut cp = source.start_cp;
    let mut offset = 0;
    for s in &story.segments {
        if s.start_cp != cp
            || s.end_cp <= cp
            || s.output_start != offset
            || s.output_end < offset
            || story.text.get(offset..s.output_end).is_none()
        {
            return Err(Error::Invalid("AUTONUM projection partition"));
        }
        cp = s.end_cp;
        offset = s.output_end;
    }
    if cp.checked_sub(source.start_cp) != Some(source.utf16_units) || offset != story.text.len() {
        return Err(Error::Invalid("AUTONUM projection coverage"));
    }
    Ok(())
}

fn suppressed(story: &TranscribedStory, field: &FieldRange) -> bool {
    let start = story
        .segments
        .partition_point(|s| s.end_cp <= field.source.start);
    story.segments[start..]
        .iter()
        .take_while(|s| s.start_cp < field.source.end)
        .all(|s| {
            s.output_start == s.output_end && matches!(s.kind, "field_marker" | "field_instruction")
        })
}

fn copy_story(story: &TranscribedStory) -> TranscribedStory {
    TranscribedStory {
        kind: story.kind,
        text: story.text.clone(),
        segments: story
            .segments
            .iter()
            .map(|s| SourceSegment {
                kind: s.kind,
                start_cp: s.start_cp,
                end_cp: s.end_cp,
                output_start: s.output_start,
                output_end: s.output_end,
            })
            .collect(),
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
    }
}

fn diagnostic(story: &mut TranscribedStory, code: &'static str, cp: usize) {
    if story.diagnostics.len() < 256 {
        story.diagnostics.push(Diagnostic { code, cp });
    }
}

fn generate(story: &TranscribedStory, fields: &[&FieldRange]) -> Result<TranscribedStory> {
    let mut result = copy_story(story);
    result.text.clear();
    result.segments.clear();
    let mut field_index = 0;
    let mut skip_until = 0;
    for segment in &story.segments {
        let mut cp = segment.start_cp.max(skip_until);
        while cp < segment.end_cp {
            if let Some(field) = fields
                .get(field_index)
                .filter(|f| f.source.start < segment.end_cp)
            {
                if cp < field.source.start {
                    append(&mut result, segment.kind, cp, field.source.start, "")?;
                }
                append(
                    &mut result,
                    "generated_number",
                    field.source.start,
                    field.source.end,
                    &format!("{}.", field_index + 1),
                )?;
                skip_until = field.source.end;
                cp = skip_until;
                field_index += 1;
            } else {
                let text = if cp == segment.start_cp {
                    &story.text[segment.output_start..segment.output_end]
                } else {
                    ""
                };
                append(&mut result, segment.kind, cp, segment.end_cp, text)?;
                cp = segment.end_cp;
            }
        }
    }
    if field_index != fields.len() {
        return Err(Error::Invalid("AUTONUM generation coverage"));
    }
    Ok(result)
}

fn append(
    out: &mut TranscribedStory,
    kind: &'static str,
    start_cp: usize,
    end_cp: usize,
    text: &str,
) -> Result<()> {
    if out.segments.len() == MAX_SEGMENTS {
        return Err(Error::Limit("AUTONUM output segments"));
    }
    let output_start = out.text.len();
    out.text.push_str(text);
    out.segments.push(SourceSegment {
        kind,
        start_cp,
        end_cp,
        output_start,
        output_end: out.text.len(),
    });
    Ok(())
}

#[cfg(test)]
mod tests;
