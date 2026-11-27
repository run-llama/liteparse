use super::{decode, roles::Position};
use crate::office::doc::{
    Error, Result,
    formatting_inspection::FormattingInspection,
    hidden_visibility::{charge, paragraph, table::TableHiddenEvidence},
    paragraph_formatting::ParagraphInspection,
    structure::StructurePlan,
    styles::Stylesheet,
    table_structure::{CellContent, TableOwnership},
};
use std::collections::BTreeMap;

/// Bind conditional roles to contiguous, rectangular, unmerged source tables.
/// Any unresolved physical row excludes the group because first/last and band roles depend on it.
pub(in crate::office::doc::hidden_visibility::table) fn enrich(
    result: &mut TableHiddenEvidence,
    styles: &Stylesheet,
    physical: &TableOwnership,
    structure: &StructurePlan,
    paragraphs: &ParagraphInspection,
    formatting: &FormattingInspection,
    budget: &mut usize,
) -> Result<()> {
    if !result.rows.iter().any(|row| {
        row.unresolved
            .contains(&"table_style_text_properties_unresolved")
    }) {
        return Ok(());
    }
    let indices: BTreeMap<_, _> = result
        .rows
        .iter()
        .enumerate()
        .map(|(i, r)| (r.source_range, i))
        .collect();
    for (story, part) in physical.stories.iter().zip(&structure.stories) {
        let mut first = 0;
        while first < story.rows.len() {
            let mut limit = first + 1;
            while limit < story.rows.len()
                && story.rows[limit - 1].source_range[1] == story.rows[limit].source_range[0]
            {
                limit += 1;
            }
            let group = &story.rows[first..limit];
            first = limit;
            let columns = group[0].cells.len();
            if columns == 0
                || group.iter().any(|row| {
                    row.depth != 1
                        || !row.unresolved.is_empty()
                        || row.cells.len() != columns
                        || row.definition_cell_count != Some(columns)
                        || row.cells.iter().any(|c| {
                            c.definition
                                .as_ref()
                                .is_none_or(|d| d.horizontal_merge != 0 || d.vertical_merge != 0)
                                || c.content
                                    .iter()
                                    .any(|p| matches!(p, CellContent::NestedRow(_)))
                        })
                })
            {
                continue;
            }
            let mut options = Vec::new();
            let mut headers = Vec::new();
            let mut valid = true;
            for row in group {
                charge(budget, 64 + columns)?;
                let Some(&index) = indices.get(&row.source_range) else {
                    valid = false;
                    break;
                };
                if result.rows[index]
                    .unresolved
                    .iter()
                    .any(|r| *r != "table_style_text_properties_unresolved")
                {
                    valid = false;
                    break;
                }
                let end = row.source_range[1];
                let pap = paragraphs
                    .spans
                    .get(paragraphs.spans.partition_point(|p| p.end_cp < end))
                    .filter(|p| p.end_cp == end)
                    .ok_or(Error::Invalid("conditional row PAPX"))?;
                let piece = formatting
                    .pieces
                    .get(formatting.pieces.partition_point(|p| p.end_cp < end))
                    .filter(|p| p.start_cp < end && end <= p.end_cp)
                    .ok_or(Error::Invalid("conditional row piece"))?;
                let mut properties = paragraph::group(&pap.raw_modifiers, 5, budget)?;
                properties.extend(
                    paragraph::piece(piece.raw_prm, &formatting.raw_property_groups, 5, budget)?
                        .into_iter()
                        .filter(|p| (p.sprm >> 10) & 7 == 5),
                );
                let mut flags = 0;
                let mut header = false;
                let mut bidi = [false; 2];
                for p in properties {
                    match (p.sprm, p.operand.as_slice()) {
                        (0x740a, [_, _, a, b]) if u16::from_le_bytes([*a, *b]) & 0xf800 == 0 => {
                            flags = u16::from_le_bytes([*a, *b])
                        }
                        (0x3404, [b @ 0..=1]) => header = *b != 0,
                        (0x560b | 0x5664, [b @ 0..=1, 0]) => {
                            bidi[usize::from(p.sprm == 0x5664)] = *b != 0
                        }
                        (0x740a | 0x3404 | 0x560b | 0x5664, _) => {
                            valid = false;
                            break;
                        }
                        _ => {}
                    }
                }
                headers.push(header);
                options.push((index, flags, bidi[0] || bidi[1]));
            }
            if !valid {
                continue;
            }
            let mut preceding = 0;
            let non_header_prefix: Vec<_> = headers
                .iter()
                .enumerate()
                .map(|(i, &header)| {
                    let count = preceding;
                    preceding += usize::from(i != 0 && !header);
                    count
                })
                .collect();
            for (row_number, row) in group.iter().enumerate() {
                let (index, flags, right_to_left) = options[row_number];
                if !result.rows[index]
                    .unresolved
                    .contains(&"table_style_text_properties_unresolved")
                {
                    continue;
                }
                let Some(style_index) = result.rows[index].selected_style else {
                    continue;
                };
                let style = match decode(style_index, styles, budget) {
                    Ok(s) => s,
                    Err(Error::Unsupported(_)) => continue,
                    Err(e) => return Err(e),
                };
                let mut cells = Vec::new();
                let mut owners = Vec::new();
                let mut qualified = true;
                for (column, cell) in row.cells.iter().enumerate() {
                    let state = match style.cell(
                        cell.source_range,
                        Position {
                            row: row_number,
                            column,
                            columns,
                            headers: &headers,
                            right_to_left,
                            preceding_non_header_rows: non_header_prefix[row_number],
                        },
                        flags,
                        budget,
                    ) {
                        Ok(s) => s,
                        Err(Error::Unsupported(_)) => {
                            qualified = false;
                            break;
                        }
                        Err(e) => return Err(e),
                    };
                    for content in &cell.content {
                        let CellContent::Paragraph(p) = *content else {
                            return Err(Error::Invalid("conditional nested cell"));
                        };
                        let paragraph = part
                            .paragraphs
                            .get(p)
                            .ok_or(Error::Invalid("conditional paragraph owner"))?;
                        if !paragraph.unresolved.is_empty()
                            || paragraph.source_range[0] < cell.source_range[0]
                            || paragraph.source_range[1] > cell.source_range[1]
                        {
                            qualified = false;
                            break;
                        }
                        owners.push((paragraph.source_range[1], column));
                    }
                    cells.push(state);
                }
                if !qualified {
                    continue;
                }
                for (end, column) in owners {
                    if result
                        .conditional_paragraphs
                        .insert(end, (index, column))
                        .is_some()
                    {
                        return Err(Error::Invalid("conditional duplicate paragraph"));
                    }
                }
                result.rows[index].style_ancestors = style.ancestors;
                result.rows[index].conditional_cell_text = cells;
                result.rows[index]
                    .unresolved
                    .retain(|r| *r != "table_style_text_properties_unresolved");
            }
        }
    }
    Ok(())
}
