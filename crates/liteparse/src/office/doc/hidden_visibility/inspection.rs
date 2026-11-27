use super::{HiddenProperty, MAX_PROPERTY_WORK, charge, resolve_with_budget};
use crate::office::doc::{
    Error, Inspection, Result, formatting_inspection::FormattingInspection,
    paragraph_formatting::ParagraphInspection, styles::Stylesheet,
};
use serde::Serialize;
use std::collections::BTreeMap;

const MAX_SPANS: usize = 262_144;

#[derive(Debug, Serialize)]
pub struct HiddenEvidence {
    pub status: &'static str,
    pub view: &'static str,
    pub spans: Vec<HiddenSpan>,
}
#[derive(Debug, Serialize)]
pub struct HiddenSpan {
    pub start_cp: usize,
    pub end_cp: usize,
    pub source_span_index: usize,
    pub piece_index: usize,
    pub paragraph_end_cp: Option<usize>,
    pub paragraph_table_depth: Option<u32>,
    pub table_baseline_basis: &'static str,
    #[serde(flatten)]
    pub property: HiddenProperty,
}
struct Paragraph {
    start: usize,
    end: usize,
    style: Option<usize>,
    mark: Option<usize>,
    non_table: bool,
    table_depth: Option<u32>,
}

/// Combine validated source paragraph ends with ordered character and piece properties.
/// A proven zero-depth paragraph or a stylesheet without table styles establishes an
/// empty table contribution; conditional formatting of other tables remains unresolved.
/// <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/d8b66123-1a3d-4e06-94c5-5ab16e9b6417>
pub fn resolve_hidden_evidence(
    source: &Inspection,
    formatting: &FormattingInspection,
    paragraphs: &ParagraphInspection,
    styles: &Stylesheet,
) -> Result<HiddenEvidence> {
    resolve(source, formatting, paragraphs, styles, None)
}

/// Resolve table contributions using source row ownership and complete revision evidence.
/// Final projected byte ranges must be recomputed after hidden text is suppressed.
pub fn resolve_hidden_evidence_with_tables(
    source: &Inspection,
    formatting: &FormattingInspection,
    paragraphs: &ParagraphInspection,
    styles: &Stylesheet,
    structure: &crate::office::doc::structure::StructurePlan,
    revisions: &crate::office::doc::visibility::RevisionEvidence,
) -> Result<(HiddenEvidence, super::table::TableHiddenEvidence)> {
    let tables = crate::office::doc::table_structure::plan_tables(
        source, structure, paragraphs, formatting,
    )?;
    let table = super::table::resolve(
        source, formatting, paragraphs, styles, structure, &tables, revisions,
    )?;
    let evidence = resolve(source, formatting, paragraphs, styles, Some(&table))?;
    Ok((evidence, table))
}

fn resolve(
    source: &Inspection,
    formatting: &FormattingInspection,
    paragraphs: &ParagraphInspection,
    styles: &Stylesheet,
    tables: Option<&super::table::TableHiddenEvidence>,
) -> Result<HiddenEvidence> {
    if source.binary_version != formatting.binary_version
        || source.binary_version != paragraphs.binary_version
    {
        return Err(Error::Invalid("hidden source version"));
    }
    if formatting.character_spans.len() > MAX_SPANS
        || formatting.pieces.len() > MAX_SPANS
        || paragraphs.spans.len() > MAX_SPANS
    {
        return Err(Error::Limit("hidden source spans"));
    }
    let mut piece_end = 0;
    for piece in &formatting.pieces {
        if piece.start_cp != piece_end || piece.end_cp <= piece.start_cp {
            return Err(Error::Invalid("hidden piece order"));
        }
        piece_end = piece.end_cp;
    }
    let mut previous = 0;
    for span in &paragraphs.spans {
        if span.start_cp != previous || span.end_cp <= span.start_cp {
            return Err(Error::Invalid("hidden paragraph span order"));
        }
        previous = span.end_cp;
    }
    let mut work = MAX_PROPERTY_WORK;
    let logical = paragraph_context(source, formatting, paragraphs, styles, tables, &mut work)?;
    let source_end = logical.last().map_or(0, |p| p.end);
    let table_base =
        (!styles.styles.iter().flatten().any(|style| style.kind == 3)).then_some(false);
    let mut result = Vec::new();
    let mut metadata_budget = MAX_PROPERTY_WORK;
    let mut next_cp = 0;
    for (source_span_index, span) in formatting.character_spans.iter().enumerate() {
        if span.start_cp != next_cp || span.end_cp <= span.start_cp {
            return Err(Error::Invalid("hidden character span order"));
        }
        next_cp = span.end_cp;
        if span.start_cp >= source_end {
            break;
        }
        let piece_index = formatting
            .pieces
            .partition_point(|piece| piece.end_cp <= span.start_cp);
        let piece = formatting
            .pieces
            .get(piece_index)
            .filter(|piece| piece.start_cp <= span.start_cp && piece.end_cp >= span.end_cp)
            .ok_or(Error::Invalid("hidden piece coverage"))?;
        let piece_properties =
            super::paragraph::piece(piece.raw_prm, &formatting.raw_property_groups, 2, &mut work);
        charge(
            &mut work,
            span.properties.iter().map(|p| p.operand.len() + 2).sum(),
        )?;
        let mut direct = span.properties.clone();
        if let Ok(properties) = &piece_properties {
            charge(
                &mut work,
                properties.iter().map(|p| p.operand.len() + 2).sum(),
            )?;
            direct.extend_from_slice(properties);
        }
        let mut cp = span.start_cp;
        while cp < span.end_cp.min(source_end) {
            let paragraph = logical
                .get(logical.partition_point(|p| p.end <= cp))
                .filter(|p| p.start <= cp)
                .ok_or(Error::Invalid("hidden logical paragraph coverage"))?;
            let end = paragraph.end.min(span.end_cp).min(source_end);
            let proven_table = tables.and_then(|t| t.visibility_basis(paragraph.end));
            let conditional = tables.and_then(|t| t.conditional_text(paragraph.end));
            let mut property = resolve_with_budget(
                styles,
                paragraph.style.unwrap_or(usize::MAX),
                if let Some(state) = conditional {
                    Some(state.hidden)
                } else if paragraph.non_table || proven_table.is_some() {
                    Some(false)
                } else {
                    table_base
                },
                &direct,
                &mut work,
            )?;
            if piece_properties.is_err() {
                property.hidden = None;
                property
                    .unresolved
                    .push("piece_character_properties_unresolved");
            }
            if result.len() == MAX_SPANS {
                return Err(Error::Limit("hidden evidence spans"));
            }
            charge(
                &mut metadata_budget,
                std::mem::size_of::<HiddenSpan>() + property.unresolved.len() * 32,
            )?;
            result.push(HiddenSpan {
                start_cp: cp,
                end_cp: end,
                source_span_index,
                piece_index,
                paragraph_end_cp: paragraph.mark,
                paragraph_table_depth: paragraph.table_depth,
                table_baseline_basis: if paragraph.non_table {
                    "source_paragraph_depth_zero"
                } else if conditional.is_some() {
                    "selected_table_conditional_cell_properties"
                } else if let Some(basis) = proven_table {
                    basis
                } else if table_base.is_some() {
                    "stylesheet_has_no_table_styles"
                } else {
                    "conditional_table_properties_unresolved"
                },
                property,
            });
            cp = end;
        }
    }
    if result.last().map_or(0, |span| span.end_cp) != source_end {
        return Err(Error::Invalid("hidden incomplete source coverage"));
    }
    Ok(HiddenEvidence {
        status: "partial_source_hidden_properties",
        view: "hidden_text_display_off",
        spans: result,
    })
}

fn paragraph_context(
    source: &Inspection,
    formatting: &FormattingInspection,
    paragraphs: &ParagraphInspection,
    styles: &Stylesheet,
    tables: Option<&super::table::TableHiddenEvidence>,
    work: &mut usize,
) -> Result<Vec<Paragraph>> {
    let mut result = Vec::new();
    let mut expected_start = 0;
    let mut style_cache = BTreeMap::new();
    let no_table_styles = !styles.styles.iter().flatten().any(|s| s.kind == 3);
    let mut source_budget = 16 * 1024 * 1024usize;
    for story in &source.stories {
        if story.start_cp != expected_start {
            return Err(Error::Invalid("hidden story order"));
        }
        source_budget = source_budget
            .checked_sub(story.utf16_units)
            .ok_or(Error::Limit("hidden source units"))?;
        let mut start = story.start_cp;
        let mut count = 0;
        let mut line_breaks = Vec::new();
        for unit in story.text.encode_utf16() {
            let cp = story
                .start_cp
                .checked_add(count)
                .ok_or(Error::Limit("hidden source positions"))?;
            count += 1;
            if unit == 11 {
                charge(work, std::mem::size_of::<usize>())?;
                line_breaks.push(cp);
            }
            let canonical = paragraphs.status == "reference_expanded_endpoint_selected_paragraphs";
            if !canonical && !matches!(unit, 7 | 12 | 13) {
                continue;
            }
            let candidate = paragraphs
                .spans
                .get(paragraphs.spans.partition_point(|p| p.end_cp <= cp));
            if canonical && candidate.is_some_and(|p| p.start_cp <= cp && p.end_cp > cp + 1) {
                continue;
            }
            let pap = candidate.filter(|p| p.start_cp <= cp && p.end_cp == cp + 1);
            let piece = formatting
                .pieces
                .get(formatting.pieces.partition_point(|p| p.end_cp <= cp))
                .filter(|p| p.start_cp <= cp && p.end_cp > cp);
            let mut context = match (pap, piece) {
                (Some(pap), Some(piece)) if matches!(unit, 7 | 12 | 13) => {
                    match super::paragraph::resolve(
                        pap.style_index.unwrap_or(0) as usize,
                        &pap.raw_modifiers,
                        piece.raw_prm,
                        &formatting.raw_property_groups,
                        unit,
                        styles,
                        &mut style_cache,
                        work,
                    ) {
                        Ok(context) => Some(context),
                        Err(Error::Unsupported(_)) => None,
                        Err(error) => return Err(error),
                    }
                }
                _ => None,
            };
            if let Some(paragraph) = &context {
                let visibility_neutral_table = paragraph.non_table
                    || tables.is_some_and(|t| t.visibility_basis(cp + 1).is_some())
                    || no_table_styles;
                for &line_break in &line_breaks {
                    if !ordinary_line_break(
                        line_break,
                        formatting,
                        styles,
                        paragraph.style,
                        visibility_neutral_table.then_some(false),
                        work,
                    )? {
                        context = None;
                        break;
                    }
                }
            }
            if result.len() == MAX_SPANS {
                return Err(Error::Limit("hidden logical paragraphs"));
            }
            result.push(Paragraph {
                start,
                end: cp + 1,
                style: context.as_ref().and_then(|p| p.style),
                mark: pap.map(|_| cp),
                non_table: context.as_ref().is_some_and(|p| p.non_table),
                table_depth: context.as_ref().and_then(|p| p.table_depth),
            });
            start = cp + 1;
            line_breaks.clear();
        }
        if count != story.utf16_units {
            return Err(Error::Invalid("hidden story UTF-16 length"));
        }
        expected_start = story
            .start_cp
            .checked_add(count)
            .ok_or(Error::Limit("hidden source positions"))?;
        if start < expected_start {
            if result.len() == MAX_SPANS {
                return Err(Error::Limit("hidden logical paragraphs"));
            }
            result.push(Paragraph {
                start,
                end: expected_start,
                style: None,
                mark: None,
                non_table: false,
                table_depth: None,
            });
        }
    }
    Ok(result)
}

/// U000B is an ordinary line break unless its CFSpecVanish toggle resolves true.
/// Unknown style/PCD contributions preserve the paragraph-context guard.
/// <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/7022285b-9621-42e9-ad4d-4e02c115ef18>
fn ordinary_line_break(
    cp: usize,
    formatting: &FormattingInspection,
    styles: &Stylesheet,
    paragraph_style: Option<usize>,
    baseline: Option<bool>,
    work: &mut usize,
) -> Result<bool> {
    let Some(style) = paragraph_style else {
        return Ok(false);
    };
    let Some(span) = formatting
        .character_spans
        .get(
            formatting
                .character_spans
                .partition_point(|s| s.end_cp <= cp),
        )
        .filter(|s| s.start_cp <= cp && cp < s.end_cp)
    else {
        return Ok(false);
    };
    let Some(piece) = formatting
        .pieces
        .get(formatting.pieces.partition_point(|p| p.end_cp <= cp))
        .filter(|p| p.start_cp <= cp && cp < p.end_cp)
    else {
        return Ok(false);
    };
    charge(
        work,
        span.properties.iter().map(|p| p.operand.len() + 2).sum(),
    )?;
    let mut direct = span.properties.clone();
    match super::paragraph::piece(piece.raw_prm, &formatting.raw_property_groups, 2, work) {
        Ok(properties) => direct.extend(properties),
        Err(Error::Unsupported(_)) => return Ok(false),
        Err(error) => return Err(error),
    }
    let result = super::resolve_toggle_with_budget(styles, style, baseline, &direct, work, 0x0818)?;
    Ok(result.hidden == Some(false))
}
