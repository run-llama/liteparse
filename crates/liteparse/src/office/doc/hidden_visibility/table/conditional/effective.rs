//! Final typed character properties for source-owned conditional cell text.
use super::EffectiveRun;
use crate::office::doc::{
    Result,
    formatting_inspection::FormattingInspection,
    hidden_visibility::{HiddenEvidence, TableHiddenEvidence},
    styles::Stylesheet,
    symbols::FontTable,
};

/// Resolve each source span after cell selection, retaining unsupported run properties
/// explicitly. Capitalization has its own typed consumer and source-preserving projection.
pub(in crate::office::doc::hidden_visibility::table) fn populate(
    table: &mut TableHiddenEvidence,
    hidden: &HiddenEvidence,
    formatting: &FormattingInspection,
    styles: &Stylesheet,
    fonts: &FontTable,
) -> Result<()> {
    populate_with_budget(table, hidden, formatting, styles, fonts, &mut 0)
}

pub(in crate::office::doc::hidden_visibility::table) fn populate_with_budget(
    table: &mut TableHiddenEvidence,
    hidden: &HiddenEvidence,
    formatting: &FormattingInspection,
    styles: &Stylesheet,
    fonts: &FontTable,
    work: &mut usize,
) -> Result<()> {
    if table
        .rows
        .iter()
        .flat_map(|r| &r.conditional_cell_text)
        .any(|c| !c.effective_runs.is_empty())
    {
        return Err(crate::office::doc::Error::Invalid(
            "conditional effective runs already resolved",
        ));
    }
    for span in &hidden.spans {
        let Some(end) = span.paragraph_end_cp.and_then(|cp| cp.checked_add(1)) else {
            continue;
        };
        let Some(&(row, cell)) = table.conditional_paragraphs.get(&end) else {
            continue;
        };
        let selected = &mut table.rows[row].conditional_cell_text[cell];
        let mut resolve = || {
            let direct = formatting
                .character_spans
                .get(span.source_span_index)
                .filter(|p| p.start_cp <= span.start_cp && span.end_cp <= p.end_cp)
                .ok_or(crate::office::doc::Error::Invalid(
                    "conditional character span ownership",
                ))?;
            let piece = formatting
                .pieces
                .get(span.piece_index)
                .filter(|p| p.start_cp <= span.start_cp && span.end_cp <= p.end_cp)
                .ok_or(crate::office::doc::Error::Invalid(
                    "conditional character piece ownership",
                ))?;
            let piece_bytes = if piece.raw_prm & 1 != 0 {
                formatting
                    .raw_property_groups
                    .get(usize::from(piece.raw_prm >> 1))
                    .map_or(0, Vec::len)
            } else {
                3
            };
            let cost =
                direct
                    .properties
                    .iter()
                    .map(|p| p.operand.len() + 32)
                    .chain(
                        selected
                            .applied
                            .iter()
                            .map(|p| p.property.operand.len() + 32),
                    )
                    .try_fold(
                        64usize.checked_add(piece_bytes).ok_or(
                            crate::office::doc::Error::Limit("conditional run source work"),
                        )?,
                        |a, b| {
                            a.checked_add(b).ok_or(crate::office::doc::Error::Limit(
                                "conditional run source work",
                            ))
                        },
                    )?;
            *work = work
                .checked_add(cost)
                .filter(|n| *n <= 8 * 1024 * 1024)
                .ok_or(crate::office::doc::Error::Limit(
                    "conditional run source work",
                ))?;
            let mut properties = direct.properties.clone();
            properties.extend(crate::office::doc::visibility::piece_modifiers(
                piece.raw_prm,
                &formatting.raw_property_groups,
            )?);
            let baseline: Vec<_> = selected
                .applied
                .iter()
                .map(|p| &p.property)
                .filter(|p| {
                    !(matches!(p.sprm, 0x083a | 0x083b) || p.sprm == 0x4a60 && p.operand == [0, 0])
                })
                .cloned()
                .collect();
            properties.retain(|p| !matches!(p.sprm, 0x083a | 0x083b));
            crate::office::doc::render_model::runs::resolve_table_text(
                styles,
                fonts,
                span.property
                    .paragraph_style
                    .ok_or(crate::office::doc::Error::Invalid(
                        "conditional paragraph style",
                    ))?,
                &properties,
                &baseline,
                work,
            )
        };
        let (properties, unresolved) = match resolve() {
            Ok(properties) => (Some(properties), None),
            Err(error @ crate::office::doc::Error::Limit(_)) => return Err(error),
            Err(error) => (None, Some(error.to_string())),
        };
        selected.effective_runs.push(EffectiveRun {
            source_range: [span.start_cp, span.end_cp],
            properties,
            unresolved,
        });
    }
    Ok(())
}
