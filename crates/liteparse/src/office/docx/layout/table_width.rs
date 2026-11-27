//! Initial AutoFit content constraints on a shared logical column grid.

use super::{Flow, resolve_percentage_grid, stack};
use crate::office::docx::model::{Block, HeightRule, Table, TableLayout, TableWidth, VMerge};
use crate::office::docx::text::{ContentWidths, TextEnv, content_widths};

// Intrinsic measurement and final cell layout share synthesized logical columns.
fn percentage_basis(table: &Table, width: f64, compat_mode: u32) -> f64 {
    // Legacy saved Word grids include the outer cell margins in the percentage
    // basis, matching the legacy edge shift used by final table placement.
    if compat_mode < 15 {
        let outer = table
            .rows
            .first()
            .map(|row| {
                row.cells.first().map_or(0.0, |c| c.margins[1])
                    + row.cells.last().map_or(0.0, |c| c.margins[3])
            })
            .unwrap_or(0.0);
        width + outer
    } else {
        width
    }
}

pub(super) fn initial_grid(table: &Table, width: f64, compat_mode: u32) -> Vec<f64> {
    let mut grid = table.grid.clone();
    let ncols = table
        .rows
        .iter()
        .map(|r| r.grid_before + r.cells.iter().map(|c| c.span).sum::<usize>())
        .max()
        .unwrap_or(0);
    if grid.len() < ncols {
        let fill = if grid.is_empty() {
            width / ncols.max(1) as f64
        } else {
            grid.iter().sum::<f64>() / grid.len() as f64
        };
        grid.resize(ncols, fill);
    }
    resolve_percentage_grid(
        &mut grid,
        table.preferred_width.fraction(),
        percentage_basis(table, width, compat_mode),
    );
    if grid.iter().sum::<f64>() <= 1.0 {
        let w = width / grid.len().max(1) as f64;
        grid.iter_mut().for_each(|g| *g = w);
    }
    grid
}

#[derive(Clone, Copy)]
struct Constraint {
    start: usize,
    end: usize,
    widths: ContentWidths,
    preferred_floor: f64,
}

fn blocks_widths(blocks: &[Block], width: f64, env: &TextEnv) -> ContentWidths {
    let mut out = ContentWidths::default();
    for block in blocks {
        let measured = match block {
            Block::Para(p) => content_widths(p, width, env),
            Block::Table(table) if table.float.is_none() => {
                let mut grid = initial_grid(table, width, env.compat_mode);
                let indent = table.indent.max(0.0);
                if table.rows.iter().any(|r| r.layout == TableLayout::AutoFit) {
                    let preferred = preferred_grid(table, &mut grid, width);
                    let requested = if table.preferred_width.fraction().is_none() {
                        preferred
                    } else {
                        0.0
                    };
                    let constraints = constraints(table, &grid, env);
                    ContentWidths {
                        minimum: feasible_grid(grid.len(), &constraints, 1.0)
                            .iter()
                            .sum::<f64>()
                            .max(requested)
                            + indent,
                        atomic: feasible_grid(grid.len(), &constraints, 0.0)
                            .iter()
                            .sum::<f64>()
                            + indent,
                        maximum: grid.iter().sum::<f64>() + indent,
                        unclippable: unclippable_grid(grid.len(), &constraints)
                            .iter()
                            .sum::<f64>()
                            + indent,
                    }
                } else {
                    let extent = grid.iter().sum::<f64>() + indent;
                    ContentWidths {
                        minimum: extent,
                        maximum: extent,
                        atomic: extent,
                        unclippable: extent,
                    }
                }
            }
            Block::Table(_) => continue,
        };
        out.minimum = out.minimum.max(measured.minimum);
        out.maximum = out.maximum.max(measured.maximum);
        out.atomic = out.atomic.max(measured.atomic);
        out.unclippable = out.unclippable.max(measured.unclippable);
    }
    out.maximum = out.maximum.max(out.minimum);
    out
}

fn unclippable_grid(count: usize, constraints: &[Constraint]) -> Vec<f64> {
    let mut bounds = constraints.to_vec();
    for bound in &mut bounds {
        bound.widths.atomic = bound.widths.unclippable;
    }
    feasible_grid(count, &bounds, 0.0)
}

fn requirement(c: &Constraint, fraction: f64) -> f64 {
    c.widths.atomic + fraction * (c.widths.minimum - c.widths.atomic).max(0.0)
}

// Prefix differences encode all shared-grid interval minima. Forward longest
// paths give the smallest feasible total, including overlapping spanning cells.
fn feasible_grid(count: usize, constraints: &[Constraint], fraction: f64) -> Vec<f64> {
    let mut prefix = vec![0.0_f64; count + 1];
    for end in 1..=count {
        prefix[end] = prefix[end - 1];
        for c in constraints.iter().filter(|c| c.end == end) {
            prefix[end] = prefix[end].max(prefix[c.start] + requirement(c, fraction));
        }
    }
    prefix.windows(2).map(|p| p[1] - p[0]).collect()
}

// Weighted Dykstra projections preserve preferred-width proportions as columns
// yield space, stopping at shared interval minima. Corrections use the same
// fixed weights for each constraint and the table total.
fn project(grid: &mut [f64], constraints: &[Constraint], fraction: f64, total: f64) {
    let count = grid.len();
    if count == 0 {
        return;
    }
    // A zero-width source column has no proportional scale; use the mean of
    // specified columns so its content constraints can still receive width.
    let positive: Vec<f64> = grid.iter().copied().filter(|w| *w > 0.0).collect();
    let fallback = if positive.is_empty() {
        1.0
    } else {
        positive.iter().sum::<f64>() / positive.len() as f64
    };
    let weights: Vec<f64> = grid
        .iter()
        .map(|w| if *w > 0.0 { *w } else { fallback })
        .collect();
    let weight_total: f64 = weights.iter().sum();
    let mut bounds: Vec<(usize, usize, f64)> = constraints
        .iter()
        .map(|c| (c.start, c.end, requirement(c, fraction)))
        .collect();
    bounds.extend((0..count).map(|i| (i, i + 1, 0.0)));
    let mut corrections = vec![0.0; bounds.len()];
    let mut total_correction = 0.0;
    for _ in 0..2048 {
        let before = grid.to_vec();
        for ((start, end, minimum), correction) in bounds.iter().zip(&mut corrections) {
            let span = &mut grid[*start..*end];
            let span_weights = &weights[*start..*end];
            for (x, weight) in span.iter_mut().zip(span_weights) {
                *x += *correction * weight;
            }
            let addition =
                ((*minimum - span.iter().sum::<f64>()) / span_weights.iter().sum::<f64>()).max(0.0);
            for (x, weight) in span.iter_mut().zip(span_weights) {
                *x += addition * weight;
            }
            *correction = -addition;
        }
        for (x, weight) in grid.iter_mut().zip(&weights) {
            *x += total_correction * weight;
        }
        let shift = (total - grid.iter().sum::<f64>()) / weight_total;
        for (x, weight) in grid.iter_mut().zip(&weights) {
            *x += shift * weight;
        }
        total_correction = -shift;
        let settled = grid.iter().zip(before).all(|(a, b)| (a - b).abs() < 1e-7);
        let valid = bounds
            .iter()
            .all(|(a, b, min)| grid[*a..*b].iter().sum::<f64>() + 1e-6 >= *min);
        if settled && valid {
            return;
        }
    }
    let feasible = feasible_grid(count, constraints, fraction);
    let extra = (total - feasible.iter().sum::<f64>()).max(0.0) / weight_total;
    for ((x, minimum), weight) in grid.iter_mut().zip(feasible).zip(weights) {
        *x = minimum + extra * weight;
    }
}

fn constraints(table: &Table, grid: &[f64], env: &TextEnv) -> Vec<Constraint> {
    let was_in_cell = env.in_cell.replace(true);
    let mut constraints = Vec::new();
    for (row_index, row) in table.rows.iter().enumerate() {
        let mut start = row.grid_before;
        for cell in &row.cells {
            let end = start.saturating_add(cell.span).min(grid.len());
            if row.layout == TableLayout::AutoFit && start < end {
                let padding = cell.margins[1] + cell.margins[3];
                let inner = (grid[start..end].iter().sum::<f64>() - padding).max(0.0);
                let mut widths = blocks_widths(&cell.blocks, inner, env);
                if cell.text_dir.is_some() {
                    // Rotation maps stacked line height onto the horizontal axis.
                    // Exact unmerged rows also bound the longitudinal layout length.
                    let length = if row.height_rule == HeightRule::Exact
                        && row.height > 0.0
                        && cell.vmerge == VMerge::None
                    {
                        let bottom = row
                            .cells
                            .iter()
                            .map(|c| c.margins[2])
                            .fold(0.0_f64, f64::max);
                        let spacing =
                            table.cell_spacing.max(0.0) * if row_index == 0 { 4.0 } else { 2.0 };
                        (row.height + bottom + spacing - cell.margins[0] - cell.margins[2]).max(4.0)
                    } else {
                        widths.maximum.max(4.0) + 0.01
                    };
                    let flow = Flow::new(env, env.compat_mode);
                    let (_, extent) = stack(&flow.units(&cell.blocks, length, true));
                    widths = ContentWidths {
                        minimum: extent,
                        maximum: extent,
                        atomic: extent,
                        unclippable: extent,
                    };
                }
                let preferred_floor = if cell.no_wrap {
                    match cell.preferred_width {
                        TableWidth::Dxa(value) => value,
                        TableWidth::Auto | TableWidth::Percent(_) => {
                            widths.minimum = widths.maximum;
                            0.0
                        }
                    }
                } else {
                    0.0
                };
                widths.minimum += padding;
                widths.maximum += padding;
                widths.atomic += padding;
                widths.unclippable += padding;
                widths.minimum = widths.minimum.max(preferred_floor);
                widths.maximum = widths.maximum.max(widths.minimum);
                constraints.push(Constraint {
                    start,
                    end,
                    widths,
                    preferred_floor,
                });
            }
            start = end;
        }
    }
    env.in_cell.set(was_in_cell);
    // Repeated rows often impose the same interval; keep their strongest bounds.
    constraints.sort_by_key(|c| (c.start, c.end));
    let mut unique: Vec<Constraint> = Vec::new();
    for c in constraints {
        if let Some(last) = unique
            .last_mut()
            .filter(|last| last.start == c.start && last.end == c.end)
        {
            last.preferred_floor = last.preferred_floor.max(c.preferred_floor);
            last.widths.minimum = last.widths.minimum.max(c.widths.minimum);
            last.widths.atomic = last.widths.atomic.max(c.widths.atomic);
            last.widths.unclippable = last.widths.unclippable.max(c.widths.unclippable);
            last.widths.maximum = last.widths.maximum.max(c.widths.maximum);
        } else {
            unique.push(c);
        }
    }
    unique
}

pub(super) fn clips_inline_images(table: &Table, row: usize, env: &TextEnv<'_>) -> bool {
    table.preferred_width.fraction().is_none()
        && !env.grow_autofit
        && table
            .rows
            .get(row)
            .is_some_and(|r| r.layout == TableLayout::AutoFit)
}

// Resolve source preferred cells on the shared grid before content constraints.
// Explicit preferences may span margins; the content-growth cap is separate.
fn preferred_grid(table: &Table, grid: &mut [f64], width: f64) -> f64 {
    if table.preferred_width.fraction().is_some() {
        return grid.iter().sum();
    }
    let resolved = grid.to_vec();
    let resolved_total: f64 = resolved.iter().sum();
    let mut explicit_cells = false;
    let table_width = match table.preferred_width {
        TableWidth::Dxa(w) => Some(w),
        _ => None,
    };
    for (row_index, row) in table.rows.iter().enumerate() {
        let mut start = row.grid_before;
        for cell in &row.cells {
            let end = (start + cell.span).min(grid.len());
            let requested = match cell.preferred_width {
                TableWidth::Dxa(w) => Some(w),
                TableWidth::Percent(f) => Some(f * table_width.unwrap_or(width)),
                TableWidth::Auto => None,
            };
            if let Some(requested) = requested.filter(|_| start < end) {
                explicit_cells = true;
                let current: f64 = grid[start..end].iter().sum();
                if row_index == 0 {
                    if current > 0.0 {
                        for column in &mut grid[start..end] {
                            *column *= requested / current;
                        }
                    } else {
                        grid[end - 1] = requested;
                    }
                } else if requested > current {
                    grid[end - 1] += requested - current;
                }
            }
            start = end;
        }
        if let Some(target) = table_width {
            let current: f64 = grid.iter().sum();
            if current > target {
                for column in grid.iter_mut() {
                    *column *= target / current;
                }
            }
        }
    }
    let requested: f64 = grid.iter().sum();
    if table_width.is_none() && explicit_cells {
        // Cell preferences negotiate within the resolved Auto table envelope.
        // Interpolating the shared grid retains its partition when no room remains.
        let envelope = width.max(resolved_total);
        if requested > envelope && requested > resolved_total {
            let fraction = (envelope - resolved_total) / (requested - resolved_total);
            for (column, original) in grid.iter_mut().zip(&resolved) {
                *column = original + fraction * (*column - original);
            }
        }
    }
    let saved: f64 = grid.iter().sum();
    let target = table_width.unwrap_or_else(|| {
        if explicit_cells {
            saved
        } else {
            saved.min(width)
        }
    });
    if saved > 0.0 {
        for column in grid {
            *column *= target / saved;
        }
    }
    target
}

// A noWrap dxa preference yields only after other cells reach their hard floors.
// The same interval solver handles spanning preferences and content constraints.
fn protect_preferred_floors(count: usize, constraints: &mut [Constraint], limit: f64) {
    if constraints
        .iter()
        .all(|constraint| constraint.preferred_floor == 0.0)
    {
        return;
    }
    let mut priorities = constraints.to_vec();
    for constraint in &mut priorities {
        constraint.widths.minimum = constraint.preferred_floor.max(constraint.widths.atomic);
    }
    let required = |fraction| {
        feasible_grid(count, &priorities, fraction)
            .iter()
            .sum::<f64>()
    };
    let mut fraction = 1.0;
    if required(fraction) > limit {
        let (mut low, mut high) = (0.0, 1.0);
        for _ in 0..40 {
            let mid = (low + high) / 2.0;
            if required(mid) <= limit {
                low = mid;
            } else {
                high = mid;
            }
        }
        fraction = low;
    }
    for (constraint, priority) in constraints.iter_mut().zip(&priorities) {
        constraint.widths.atomic = requirement(priority, fraction);
        constraint.widths.minimum = constraint.widths.minimum.max(constraint.widths.atomic);
    }
}

pub(super) fn autofit(table: &Table, grid: &mut [f64], width: f64, env: &TextEnv) {
    if grid.is_empty()
        || !table
            .rows
            .iter()
            .any(|row| row.layout == TableLayout::AutoFit)
    {
        return;
    }
    let preferred = preferred_grid(table, grid, width);
    if !preferred.is_finite() || preferred <= 0.0 {
        return;
    }
    let mut constraints = constraints(table, grid, env);
    if constraints.is_empty() {
        return;
    }
    // Word breaking and raster clipping share the same available width budget.
    if !env.grow_autofit && table.preferred_width.fraction().is_none() {
        for constraint in &mut constraints {
            constraint.widths.atomic = constraint.widths.unclippable;
        }
    }
    let limit = if env.grow_autofit {
        f64::INFINITY
    } else {
        preferred.max(match table.preferred_width {
            TableWidth::Percent(_) => percentage_basis(table, width, env.compat_mode),
            _ => width,
        })
    };
    protect_preferred_floors(grid.len(), &mut constraints, limit);
    let required = |fraction| {
        feasible_grid(grid.len(), &constraints, fraction)
            .iter()
            .sum::<f64>()
    };
    let mut fraction = 1.0;
    if required(fraction) > limit {
        if required(0.0) > limit {
            // Indivisible source objects retain their extent when the page is too narrow.
            fraction = 0.0;
        } else {
            let (mut lo, mut hi) = (0.0, 1.0);
            for _ in 0..40 {
                let mid = (lo + hi) / 2.0;
                if required(mid) <= limit {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            fraction = lo;
        }
    }
    let target = preferred.max(required(fraction));
    if (grid.iter().sum::<f64>() - target).abs() < 1e-6
        && constraints
            .iter()
            .all(|c| grid[c.start..c.end].iter().sum::<f64>() + 1e-6 >= requirement(c, fraction))
    {
        return;
    }
    project(grid, &constraints, fraction, target);
}

#[cfg(test)]
mod tests;
