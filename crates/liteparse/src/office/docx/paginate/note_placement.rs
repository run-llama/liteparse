//! Final body extents and footnote-reference ownership after pagination trials.

use super::{PageInfo, Placed, Unit, anchor_rect, leading_column_transition, wrap_boxes};
use crate::office::docx::model::{FootnotePosition, SectPr, Wrap};

#[derive(Debug, Clone)]
pub struct NoteRegion {
    pub section: usize,
    /// Source body column containing the reference; note-area columns are separate.
    pub column: usize,
    pub body_columns: usize,
    pub position: FootnotePosition,
    pub x: f64,
    pub width: f64,
    /// Normal-flow bottom, excluding terminal paragraph after-spacing.
    pub body_bottom: f64,
    pub notes: Vec<i64>,
    /// Foreground wrapping intervals: (top, bottom, left, right), in page points.
    pub exclusions: Vec<(f64, f64, f64, f64)>,
}

/// Capture accepted placements before drawing drains their text items. Section
/// records survive later continuous sections without changing header ownership;
/// rejected keep-chain and column-balance trials contribute no geometry.
pub(super) fn record(
    placed: &[Placed],
    units: &[Unit],
    section: usize,
    props: &SectPr,
    pages: &mut [PageInfo],
) {
    let columns = props.columns();
    for placement in placed {
        let (page, column, bottom, notes, exclusions) = match placement {
            Placed::Line {
                unit,
                line,
                page,
                column,
                x,
                y,
                pad_bottom,
                para_y,
                alt,
                ..
            } => {
                let Unit::Para(original) = &units[*unit] else {
                    continue;
                };
                let para = alt.as_deref().unwrap_or(original);
                let text = &para.lines[*line];
                let bottom = if leading_column_transition(para, *line) {
                    *y
                } else {
                    y + text.height + pad_bottom
                };
                let exclusions = text
                    .anchors
                    .iter()
                    .filter(|a| !a.behind && a.wrap != Wrap::None)
                    .flat_map(|a| {
                        let (ax, ay, w, h) = anchor_rect(a, &pages[*page], *x, *para_y, *y);
                        wrap_boxes(a, ax, ay, w, h)
                    })
                    .collect();
                (*page, *column, bottom, text.notes.clone(), exclusions)
            }
            Placed::Row {
                row,
                page,
                column,
                normal_flow,
                x,
                y,
                ..
            } => {
                let bottom = if *normal_flow {
                    y + row.height + row.outer_after
                } else {
                    pages[*page].top
                };
                let exclusions = if *normal_flow {
                    Vec::new()
                } else {
                    vec![(
                        *y,
                        y + row.height,
                        *x,
                        x + row.cells.iter().map(|c| c.w).sum::<f64>(),
                    )]
                };
                (*page, *column, bottom, row.notes(), exclusions)
            }
        };
        let info = &mut pages[page];
        let index = match info
            .note_regions
            .iter()
            .position(|r| r.section == section && r.column == column)
        {
            Some(index) => index,
            None => {
                let (x, width) = columns[column];
                info.note_regions.push(NoteRegion {
                    section,
                    column,
                    body_columns: columns.len(),
                    position: props.note_position,
                    x,
                    width,
                    body_bottom: info.top,
                    notes: Vec::new(),
                    exclusions: Vec::new(),
                });
                info.note_regions.len() - 1
            }
        };
        let region = &mut info.note_regions[index];
        region.body_bottom = region.body_bottom.max(bottom);
        for id in notes {
            if !region.notes.contains(&id) {
                region.notes.push(id);
            }
        }
        region.exclusions.extend(exclusions);
    }
}

#[cfg(test)]
mod tests;
