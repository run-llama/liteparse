//! Source capitalization properties without changing logical transcription text.
//!
//! MS-DOC §2.6.1 defines CFCaps and CFSmallCaps as presentation toggles; §2.9.327
//! makes relative operands depend on the current style, rather than prior direct values.
//! <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/7022285b-9621-42e9-ad4d-4e02c115ef18>
use crate::office::doc::{
    Error, Inspection, Result, Transcription,
    formatting_inspection::FormattingInspection,
    hidden_visibility::{self, HiddenEvidence},
    styles::Stylesheet,
    visibility::RevisionEvidence,
};
use serde::Serialize;

const MAX_RECORDS: usize = 262_144;
const MAX_WORK: usize = 8 * 1024 * 1024;
const MAX_SOURCE_UNITS: usize = 16 * 1024 * 1024;

pub struct Context<'a> {
    pub source: &'a Inspection,
    pub formatting: &'a FormattingInspection,
    pub styles: &'a Stylesheet,
    pub hidden: &'a HiddenEvidence,
    pub revisions: &'a RevisionEvidence,
    pub table_properties: Option<&'a hidden_visibility::TableHiddenEvidence>,
}

#[derive(Debug, Serialize)]
pub struct Capitalization {
    pub schema_version: u32,
    pub status: &'static str,
    pub logical_text_unchanged: bool,
    pub rendered_case_and_size: &'static str,
    pub spans: Vec<CapitalizationSpan>,
    pub generated_output_ranges: Vec<GeneratedOutput>,
}

#[derive(Debug, Serialize)]
pub struct GeneratedOutput {
    pub story: &'static str,
    pub source_anchor: usize,
    pub output_range: [usize; 2],
    pub kind: &'static str,
    pub unresolved: &'static str,
}

#[derive(Debug, Serialize)]
pub struct CapitalizationSpan {
    pub start_cp: usize,
    pub end_cp: usize,
    pub source_span_index: usize,
    pub piece_index: usize,
    pub paragraph_style: Option<usize>,
    pub character_style: usize,
    pub all_caps: Option<bool>,
    pub small_caps: Option<bool>,
    pub hidden: Option<bool>,
    pub deleted: Option<bool>,
    pub field_hidden: Option<bool>,
    /// Visibility properties are known; this does not assert that a source run is displayed.
    pub visibility_resolved: bool,
    pub unresolved: Vec<&'static str>,
    pub output: Vec<OutputOwnership>,
}

#[derive(Debug, Serialize)]
pub struct OutputOwnership {
    pub story: &'static str,
    pub source_range: [usize; 2],
    pub output_range: Option<[usize; 2]>,
    pub segment_kind: &'static str,
    pub basis: &'static str,
}

fn charge(work: &mut usize, amount: usize) -> Result<()> {
    *work = work
        .checked_sub(amount)
        .ok_or(Error::Limit("capitalization work"))?;
    Ok(())
}

/// Resolve presentation state after every text-producing projection has completed.
/// Source span indexes retain CHPX/PCD identity; final byte ranges refer to `input`.
/// Generated labels retain independent anchors because their formatting is not a source run.
pub fn inspect(input: &Transcription, ctx: &Context<'_>) -> Result<Capitalization> {
    if ctx.source.stories.len() > 8 {
        return Err(Error::Limit("capitalization source stories"));
    }
    let source_units = ctx
        .source
        .stories
        .iter()
        .try_fold(0usize, |n, story| n.checked_add(story.utf16_units))
        .ok_or(Error::Limit("capitalization source units"))?;
    if source_units > MAX_SOURCE_UNITS || ctx.formatting.authored_story_end_cp > MAX_SOURCE_UNITS {
        return Err(Error::Limit("capitalization source units"));
    }
    if ctx.source.binary_version != ctx.formatting.binary_version
        || ctx.hidden.spans.len() > MAX_RECORDS
        || ctx.styles.styles.len() > 4094
        || input.stories.len() != ctx.source.stories.len()
    {
        return Err(Error::Invalid("capitalization source identity or bounds"));
    }
    let mut work = MAX_WORK;
    let mut metadata = MAX_WORK;
    let mut spans = Vec::new();
    let no_table_styles = !ctx.styles.styles.iter().flatten().any(|s| s.kind == 3);
    let mut previous = 0;
    for h in &ctx.hidden.spans {
        if h.start_cp != previous || h.end_cp <= h.start_cp {
            return Err(Error::Invalid("capitalization hidden coverage"));
        }
        previous = h.end_cp;
        let direct = ctx
            .formatting
            .character_spans
            .get(h.source_span_index)
            .filter(|s| s.start_cp <= h.start_cp && h.end_cp <= s.end_cp)
            .ok_or(Error::Invalid("capitalization character identity"))?;
        let piece = ctx
            .formatting
            .pieces
            .get(h.piece_index)
            .filter(|p| p.start_cp <= h.start_cp && h.end_cp <= p.end_cp)
            .ok_or(Error::Invalid("capitalization piece identity"))?;
        charge(
            &mut work,
            direct.properties.iter().map(|p| p.operand.len() + 2).sum(),
        )?;
        let mut props = direct.properties.clone();
        let piece_result = hidden_visibility::capitalization_piece_properties(
            piece.raw_prm,
            &ctx.formatting.raw_property_groups,
            &mut work,
        );
        if let Ok(p) = &piece_result {
            props.extend_from_slice(p);
        }
        // The typed table proof requires empty character and identity-only paragraph
        // groups across the selected style chain, including conditional groups.
        let empty_table = h.paragraph_end_cp.is_some_and(|mark| {
            ctx.table_properties.is_some_and(|table| {
                table.has_empty_text_contribution([h.start_cp, h.end_cp], mark)
            })
        });
        let baseline = (h.paragraph_table_depth == Some(0)
            && h.table_baseline_basis == "source_paragraph_depth_zero"
            || no_table_styles
            || empty_table)
            .then_some(false);
        let conditional = h.paragraph_end_cp.and_then(|mark| {
            ctx.table_properties
                .and_then(|t| t.conditional_text(mark + 1))
        });
        let style = h.property.paragraph_style.unwrap_or(usize::MAX);
        let mut caps = hidden_visibility::resolve_capitalization_toggle(
            ctx.styles,
            style,
            conditional.map(|s| s.all_caps).or(baseline),
            &props,
            &mut work,
            0x083b,
        )?;
        let mut small = hidden_visibility::resolve_capitalization_toggle(
            ctx.styles,
            style,
            conditional.map(|s| s.small_caps).or(baseline),
            &props,
            &mut work,
            0x083a,
        )?;
        let mut unresolved = Vec::new();
        unresolved.append(&mut caps.unresolved);
        unresolved.append(&mut small.unresolved);
        if piece_result.is_err() {
            caps.hidden = None;
            small.hidden = None;
            unresolved.push("piece_capitalization_properties_unresolved");
        }
        if caps.hidden == Some(true) && small.hidden == Some(true) {
            unresolved.push("combined_caps_visual_precedence_unresolved");
        }
        let rev = ctx
            .revisions
            .spans
            .get(
                ctx.revisions
                    .spans
                    .partition_point(|r| r.end_cp <= h.start_cp),
            )
            .filter(|r| {
                r.start_cp <= h.start_cp
                    && h.end_cp <= r.end_cp
                    && r.source_span_index == h.source_span_index
                    && r.piece_index == h.piece_index
            });
        let visibility_resolved = h.property.unresolved.is_empty()
            && h.property.hidden.is_some()
            && rev.is_some_and(|r| {
                r.unresolved.is_empty()
                    && r.properties.deleted.is_some()
                    && r.properties.field_hidden.is_some()
            });
        if !visibility_resolved {
            unresolved.push("accepted_or_hidden_visibility_unresolved");
        }
        unresolved.sort_unstable();
        unresolved.dedup();
        charge(
            &mut metadata,
            std::mem::size_of::<CapitalizationSpan>() + unresolved.len() * 32,
        )?;
        spans.push(CapitalizationSpan {
            start_cp: h.start_cp,
            end_cp: h.end_cp,
            source_span_index: h.source_span_index,
            piece_index: h.piece_index,
            paragraph_style: h.property.paragraph_style,
            character_style: caps.character_style,
            all_caps: caps.hidden,
            small_caps: small.hidden,
            hidden: h.property.hidden,
            deleted: rev.and_then(|r| r.properties.deleted),
            field_hidden: rev.and_then(|r| r.properties.field_hidden),
            visibility_resolved,
            unresolved,
            output: Vec::new(),
        });
    }
    if previous != ctx.formatting.authored_story_end_cp {
        return Err(Error::Invalid("capitalization authored coverage"));
    }
    let mut generated = Vec::new();
    let mut source_end = 0;
    for (raw, story) in ctx.source.stories.iter().zip(&input.stories) {
        if raw.kind != story.kind || raw.start_cp != source_end {
            return Err(Error::Invalid("capitalization story identity"));
        }
        source_end = raw
            .start_cp
            .checked_add(raw.utf16_units)
            .ok_or(Error::Invalid("capitalization source overflow"))?;
        let mut cp = raw.start_cp;
        let mut byte = 0;
        let mut source_byte = 0;
        let mut source_cp = raw.start_cp;
        for seg in &story.segments {
            if seg.start_cp != cp
                || seg.end_cp < cp
                || seg.end_cp > source_end
                || seg.output_start != byte
                || seg.output_end < byte
            {
                return Err(Error::Invalid("capitalization segment partition"));
            }
            let text = story
                .text
                .get(seg.output_start..seg.output_end)
                .ok_or(Error::Invalid("capitalization output boundary"))?;
            byte = seg.output_end;
            if seg.start_cp == seg.end_cp {
                if !seg.kind.starts_with("generated_") {
                    return Err(Error::Invalid("capitalization generated anchor"));
                }
                if generated.len() >= MAX_RECORDS {
                    return Err(Error::Limit("capitalization generated records"));
                }
                charge(&mut metadata, std::mem::size_of::<GeneratedOutput>())?;
                generated.push(GeneratedOutput {
                    story: story.kind,
                    source_anchor: cp,
                    output_range: [seg.output_start, seg.output_end],
                    kind: seg.kind,
                    unresolved: "generated_formatting_requires_own_property_context",
                });
                continue;
            }
            // This forward-only walk is bounded by the authored-source unit cap.
            // Property evaluation and sparse indexes have a separate work budget.
            let source_start = source_byte;
            for ch in raw.text[source_byte..].chars() {
                if source_cp == seg.end_cp {
                    break;
                }
                source_cp += ch.len_utf16();
                source_byte += ch.len_utf8();
                if source_cp > seg.end_cp {
                    return Err(Error::Invalid(
                        "capitalization segment splits source scalar",
                    ));
                }
            }
            if source_cp != seg.end_cp {
                return Err(Error::Invalid("capitalization source text coverage"));
            }
            let source_text = &raw.text[source_start..source_byte];
            let mut at = spans.partition_point(|s| s.end_cp <= cp);
            let mut boundaries = vec![cp];
            for span in &spans[at..] {
                let end = span.end_cp.min(seg.end_cp);
                boundaries.push(end);
                if end == seg.end_cp {
                    break;
                }
            }
            charge(&mut work, boundaries.len() * std::mem::size_of::<usize>())?;
            let offsets = scalar_offsets(source_text, text, seg, &boundaries, &mut work)?;
            let mut query = 0;
            while cp < seg.end_cp {
                let span = spans
                    .get_mut(at)
                    .filter(|s| s.start_cp <= cp)
                    .ok_or(Error::Invalid("capitalization projection coverage"))?;
                let end = span.end_cp.min(seg.end_cp);
                let (range, basis) = output_range(text, seg, cp, end, offsets.as_ref(), query);
                query += 1;
                charge(&mut metadata, std::mem::size_of::<OutputOwnership>())?;
                span.output.push(OutputOwnership {
                    story: story.kind,
                    source_range: [cp, end],
                    output_range: range,
                    segment_kind: seg.kind,
                    basis,
                });
                cp = end;
                at += 1;
            }
        }
        if cp != source_end || byte != story.text.len() || source_byte != raw.text.len() {
            return Err(Error::Invalid("capitalization incomplete projection"));
        }
    }
    if source_end != ctx.formatting.authored_story_end_cp {
        return Err(Error::Invalid("capitalization story coverage"));
    }
    Ok(Capitalization {
        schema_version: 1,
        status: "partial_source_capitalization_presentation",
        logical_text_unchanged: true,
        rendered_case_and_size: "not_rendered",
        spans,
        generated_output_ranges: generated,
    })
}

// A sparse index visits each projected source scalar once, recording only requested
// formatting boundaries. Exact scalar transforms establish offsets without assuming
// that equal UTF-16 lengths imply corresponding source and output positions.
fn scalar_offsets(
    source: &str,
    text: &str,
    segment: &crate::office::doc::SourceSegment,
    boundaries: &[usize],
    work: &mut usize,
) -> Result<Option<Vec<Option<usize>>>> {
    if text.is_empty() || boundaries.len() == 2 {
        return Ok(None);
    }
    let supported = matches!(
        segment.kind,
        "text"
            | "field_display_argument"
            | "paragraph"
            | "tab"
            | "line_break"
            | "column_break"
            | "page_or_section_break"
            | "cell_or_row_mark"
            | "nonbreaking_hyphen"
            | "optional_hyphen"
            | "unresolved_control"
    );
    if !supported {
        return Ok(None);
    }
    charge(
        work,
        source.len() + text.len() + boundaries.len() * std::mem::size_of::<Option<usize>>(),
    )?;
    let mut offsets = vec![None; boundaries.len()];
    let mut query = 0;
    let mut cp = segment.start_cp;
    let mut output = text.char_indices();
    for ch in source.chars() {
        let Some((byte, visible)) = output.next() else {
            return Ok(None);
        };
        let expected = match (segment.kind, ch) {
            ("text" | "field_display_argument", ch) if !ch.is_control() => Some(ch),
            ("paragraph", '\r') | ("line_break", '\u{b}') | ("column_break", '\u{e}') => Some('\n'),
            ("tab", '\t') | ("cell_or_row_mark", '\u{7}') => Some('\t'),
            ("page_or_section_break", '\u{c}') => Some('\u{c}'),
            ("nonbreaking_hyphen", '\u{1e}') => Some('\u{2011}'),
            ("optional_hyphen", '\u{1f}') => Some('\u{ad}'),
            ("unresolved_control", ch)
                if ch.is_control()
                    && !matches!(
                        ch,
                        '\u{7}'
                            | '\t'
                            | '\u{b}'
                            | '\u{c}'
                            | '\r'
                            | '\u{e}'
                            | '\u{13}'
                            | '\u{14}'
                            | '\u{15}'
                            | '\u{1e}'
                            | '\u{1f}'
                    ) =>
            {
                Some('\u{fffc}')
            }
            _ => None,
        };
        if expected != Some(visible) {
            return Ok(None);
        }
        while query < boundaries.len() && boundaries[query] < cp {
            query += 1;
        }
        if query < boundaries.len() && boundaries[query] == cp {
            offsets[query] = Some(segment.output_start + byte);
            query += 1;
        }
        cp += ch.len_utf16();
    }
    if cp != segment.end_cp || output.next().is_some() {
        return Ok(None);
    }
    while query < boundaries.len() && boundaries[query] < cp {
        query += 1;
    }
    if query < boundaries.len() && boundaries[query] == cp {
        offsets[query] = Some(segment.output_end);
    }
    Ok(Some(offsets))
}

fn output_range(
    text: &str,
    segment: &crate::office::doc::SourceSegment,
    start: usize,
    end: usize,
    offsets: Option<&Vec<Option<usize>>>,
    query: usize,
) -> (Option<[usize; 2]>, &'static str) {
    if text.is_empty() {
        return (Some([segment.output_start; 2]), "suppressed_source");
    }
    if start == segment.start_cp && end == segment.end_cp {
        return (
            Some([segment.output_start, segment.output_end]),
            "complete_source_segment",
        );
    }
    let Some(offsets) = offsets else {
        return (None, "partial_transformed_segment_unresolved");
    };
    match (
        offsets.get(query).copied().flatten(),
        offsets.get(query + 1).copied().flatten(),
    ) {
        (Some(a), Some(b)) => (
            Some([a, b]),
            if segment.kind == "text" {
                "utf16_aligned_segment"
            } else {
                "source_verified_scalar_projection"
            },
        ),
        _ => (None, "surrogate_boundary_unresolved"),
    }
}

#[cfg(test)]
mod tests;
