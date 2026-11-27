//! Rebuild remaining note-cell source at an accepted destination width.

use super::*;

pub(crate) fn supported_blocks(blocks: &[Block]) -> bool {
    blocks.iter().all(|block| match block {
        Block::Para(_) => true,
        Block::Table(table) => supported_table(table),
    })
}

pub(crate) fn supported_table(table: &Table) -> bool {
    table.float.is_none()
        && table.rows.iter().all(|row| {
            row.cells
                .iter()
                .all(|cell| cell.text_dir.is_none() && supported_blocks(&cell.blocks))
        })
}

pub(crate) fn remeasure_row(row: &RowUnit, width: f64, env: &TextEnv<'_>) -> Option<RowUnit> {
    let prior = env.in_cell.get();
    let result = remeasure_row_source(row, width, env);
    env.in_cell.set(prior);
    result
}

fn remeasure_row_source(row: &RowUnit, width: f64, env: &TextEnv<'_>) -> Option<RowUnit> {
    let source = row.source.as_ref()?;
    if (source.width - width).abs() < 0.01 {
        return Some(row.clone());
    }
    if !supported_table(&source.table) {
        return None;
    }
    let flow = Flow::new(env, env.compat_mode).for_notes();
    let mut measured = Vec::new();
    flow.table(&source.table, width, true, &mut measured);
    let Unit::Row(template) = measured.get(row.index)? else {
        return None;
    };
    let mut out = template.clone();
    out.table = row.table;
    out.outer_before = row.outer_before;
    out.outer_after = row.outer_after;
    out.bookmarks_before = row.bookmarks_before.clone();
    out.bookmarks_after = row.bookmarks_after.clone();
    out.repeating_header = row.repeating_header;
    out.source = Some(Rc::new(RowSource {
        table: source.table.clone(),
        width,
        minimum: source.minimum,
        addition: source.addition,
    }));
    let mut height = source.minimum;
    for (cell, prior) in out.cells.iter_mut().zip(&row.cells) {
        let inner = (cell.w - cell.margins[1] - cell.margins[3]).max(1.0);
        let (mut slices, content) = remeasure_slices(&prior.slices, inner, env)?;
        if table_width::clips_inline_images(&source.table, row.index, env) {
            cells::clip_inline_images(&mut slices, inner);
        }
        cell.moved = if let Some(moved) = &prior.moved {
            let (mut tail, _) = remeasure_slices(&moved.0, inner, env)?;
            if table_width::clips_inline_images(&source.table, row.index, env) {
                cells::clip_inline_images(&mut tail, inner);
            }
            Some(Rc::new((tail, moved.1)))
        } else {
            None
        };
        cell.slices = slices;
        cell.content_h = content;
        cell.terminal_after = prior.terminal_after;
        height = height.max(content + cell.margins[0] + cell.margins[2] + source.addition);
    }
    if !row.exact_height {
        out.height = height.max(1.0);
        out.merge_base_height = out.height;
    }
    let headers = Rc::make_mut(&mut out.headers);
    for header in headers {
        header.table = row.table;
    }
    Some(out)
}

fn shift(slice: &mut Slice, delta: f64) {
    slice.y += delta;
    for item in &mut slice.items {
        translate_item(item, 0.0, delta);
    }
    for (_, _, y) in &mut slice.anchors {
        *y += delta;
    }
}

pub(crate) fn remeasure_slices(
    slices: &[Slice],
    width: f64,
    env: &TextEnv<'_>,
) -> Option<(Vec<Slice>, f64)> {
    let prior = env.in_cell.replace(true);
    let result = remeasure_cell_source(slices, width, env);
    env.in_cell.set(prior);
    result
}

/// Logical grid coordinates survive cell spans, omitted cells and visual reversal.
pub(crate) fn cell_interval(row: &RowUnit, cell: usize) -> Option<(usize, usize)> {
    let source = row.source.as_ref()?.table.rows.get(row.index)?;
    let column = source.grid_before
        + source
            .cells
            .get(..cell)?
            .iter()
            .map(|c| c.span)
            .sum::<usize>();
    Some((column, source.cells.get(cell)?.span))
}

pub(crate) fn cell_in_interval(row: &RowUnit, interval: (usize, usize)) -> Option<usize> {
    (0..row.cells.len()).find(|&cell| cell_interval(row, cell) == Some(interval))
}

fn remeasure_cell_source(
    slices: &[Slice],
    width: f64,
    env: &TextEnv<'_>,
) -> Option<(Vec<Slice>, f64)> {
    if slices.is_empty() {
        return Some((Vec::new(), 0.0));
    }
    let mut units = Vec::new();
    let mut index = 0;
    let mut consumed = 0;
    let mut children = Vec::new();
    while let Some(slice) = slices.get(index) {
        if let Some(child) = &slice.nested_table {
            let projected = super::note_children::remeasure(child, width, env, 0.0)?;
            let Unit::Row(first) = child.layout.units().first()? else {
                return None;
            };
            let mut placeholder = first.clone();
            placeholder.cells.clear();
            placeholder.height = projected.h;
            placeholder.outer_before = 0.0;
            placeholder.outer_after = 0.0;
            children.push((units.len(), projected));
            units.push(Unit::Row(placeholder));
            index += 1;
            continue;
        }
        if let Some(row) = &slice.nested_row {
            units.push(Unit::Row(remeasure_row(row, width, env)?));
            index += 1;
            continue;
        }
        let (source, first) = slice.paragraph_source.as_ref()?;
        let mut end = index + 1;
        while slices.get(end).is_some_and(|s| {
            s.paragraph_source
                .as_ref()
                .is_some_and(|(p, _)| Rc::ptr_eq(p, source))
        }) {
            end += 1;
        }
        if index == 0 {
            consumed = *first;
        }
        let mut para = source.as_ref().clone();
        let prefix = &source.lines[..*first];
        para.lines = crate::office::docx::text::layout_paragraph_resuming(
            para.src.as_ref()?,
            width,
            env,
            prefix,
            &crate::office::docx::text::WrapIn::default(),
        );
        if para.lines.len() < *first
            || para.lines.iter().zip(prefix).any(|(a, b)| {
                a.replay.map(|m| m.source_end) != b.replay.map(|m| m.source_end)
                    || a.break_after != b.break_after
                    || a.notes != b.notes
                    || a.anchors
                        .iter()
                        .filter(|a| !crate::office::docx::layout::inline_box_owner(a))
                        .count()
                        != b.anchors
                            .iter()
                            .filter(|a| !crate::office::docx::layout::inline_box_owner(a))
                            .count()
            })
        {
            return None;
        }
        crate::office::docx::paginate::restore_reflow_anchors(
            &source.lines,
            &source.frame_anchors,
            &mut para.lines,
        );
        units.push(Unit::Para(para));
        index = end;
    }
    // Whole-cell projection keeps side-wrapped pictures active across paragraphs.
    let mut projection = note_projection_resuming(units, width, env, 0, consumed);
    projection.reserve_occupied();
    cells::retain_paragraph_sources(&projection.units, &mut projection.slices);
    for slice in &mut projection.slices {
        if slice
            .paragraph_source
            .as_ref()
            .is_some_and(|(p, _)| p.clip_inline_images)
        {
            cells::clip_inline_images(std::slice::from_mut(slice), width);
        }
    }
    for (unit, mut child) in children {
        let index = projection.units[..unit]
            .iter()
            .map(|unit| match unit {
                Unit::Para(p) => p.lines.len(),
                Unit::Row(_) => 1,
            })
            .sum::<usize>();
        let origin = projection.slices[index].y;
        shift(&mut child, origin);
        projection.slices[index] = child;
    }
    let origin = if consumed > 0 {
        projection.slices.get(consumed)?.y
    } else {
        0.0
    };
    let out = projection
        .slices
        .into_iter()
        .skip(consumed)
        .map(|mut slice| {
            shift(&mut slice, -origin);
            slice
        })
        .collect();
    Some((out, projection.height - origin))
}

#[cfg(test)]
mod tests;

pub(crate) fn cell_in_source_row(
    row: &RowUnit,
    index: usize,
    interval: (usize, usize),
) -> Option<usize> {
    let source = row.source.as_ref()?.table.rows.get(index)?;
    let mut column = source.grid_before;
    source.cells.iter().position(|cell| {
        let here = (column, cell.span);
        column += cell.span;
        here == interval
    })
}
