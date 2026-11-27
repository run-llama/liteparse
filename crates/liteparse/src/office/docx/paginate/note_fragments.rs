//! Immutable source cursors for note fragments; destination placement is separate.

use super::{Placed, merges};
use std::rc::Rc;

#[cfg(test)]
use crate::office::docx::layout::stack;
use crate::office::docx::layout::{RowUnit, Slice, Unit};
use crate::office::docx::model::BreakKind;

const EPS: f64 = 0.01;

#[derive(Clone)]
pub struct NoteLayout {
    units: Rc<Vec<Unit>>,
    legacy: Rc<(Vec<Slice>, f64)>,
    occupied: Rc<Vec<Vec<f64>>>,
    width: Option<f64>,
}

impl NoteLayout {
    #[cfg(test)]
    pub fn new(units: Vec<Unit>) -> Self {
        let legacy = Rc::new(stack(&units));
        Self {
            units: Rc::new(units),
            legacy,
            occupied: Rc::new(Vec::new()),
            width: None,
        }
    }

    pub fn with_local_pictures(
        units: Vec<Unit>,
        width: f64,
        env: &crate::office::docx::text::TextEnv<'_>,
    ) -> Self {
        let mut projection = crate::office::docx::layout::note_projection(units, width, env);
        projection.reserve_occupied();
        Self {
            units: Rc::new(projection.units),
            legacy: Rc::new((projection.slices, projection.occupied_height)),
            occupied: Rc::new(projection.occupied),
            width: Some(width),
        }
    }

    /// Replays accepted source intervals while measuring the remaining paragraphs.
    /// Accepted pieces keep their prior layout; occurrence identity stays on the cursor.
    #[expect(
        clippy::unwrap_in_result,
        reason = "only test constructors leave `width` unset; measured layouts always record it"
    )]
    pub(crate) fn remeasure(
        &self,
        cursor: &NoteCursor,
        width: f64,
        env: &crate::office::docx::text::TextEnv<'_>,
    ) -> Option<(Self, NoteCursor)> {
        let changed_width = self.width.is_some_and(|old| (old - width).abs() >= EPS);
        let resumed_anchor = match (self.units.get(cursor.unit), &cursor.part) {
            (Some(Unit::Para(p)), UnitCursor::Paragraph { line }) if *line > 0 => {
                p.lines[*line..].iter().any(|l| !l.anchors.is_empty())
                    && p.src.as_ref().is_some_and(|src| src.inlines.iter().any(|i| {
                        matches!(i, crate::office::docx::model::Inline::Anchor(a) if a.v_rel == crate::office::docx::model::VRel::Paragraph)
                    }))
            }
            _ => false,
        };
        if !changed_width && !resumed_anchor {
            return Some((self.clone(), cursor.clone()));
        }
        let mut units = self.units.as_ref().clone();
        for (index, unit) in units.iter_mut().enumerate().skip(cursor.unit) {
            if let Unit::Row(row) = unit {
                *row = crate::office::docx::layout::note_source::remeasure_row(row, width, env)?;
                continue;
            }
            let Unit::Para(p) = unit else { continue };
            let consumed = if index == cursor.unit {
                match cursor.part {
                    UnitCursor::Paragraph { line } => line,
                    _ => 0,
                }
            } else {
                0
            };
            let src = p.src.as_ref()?;
            let prefix = &p.lines[..consumed];
            let mut lines = crate::office::docx::text::layout_paragraph_resuming(
                src,
                width,
                env,
                prefix,
                &crate::office::docx::text::WrapIn::default(),
            );
            if lines.len() < consumed
                || lines.iter().zip(prefix).any(|(a, b)| {
                    a.text != b.text
                        || a.replay.map(|m| m.source_end) != b.replay.map(|m| m.source_end)
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
            super::restore_reflow_anchors(&p.lines, &p.frame_anchors, &mut lines);
            p.lines = lines;
            if let Some(decor) = p.decor.as_mut() {
                decor.x1 += width - self.width.unwrap();
            }
        }
        let consumed = match cursor.part {
            UnitCursor::Paragraph { line } => line,
            _ => 0,
        };
        let mut projection = crate::office::docx::layout::note_projection_resuming(
            units,
            width,
            env,
            cursor.unit,
            consumed,
        );
        projection.reserve_occupied();
        if let (Some(Unit::Para(old)), Some(Unit::Para(new))) = (
            self.units.get(cursor.unit),
            projection.units.get(cursor.unit),
        ) && (new.lines.len() < consumed
            || old.lines[..consumed].iter().zip(&new.lines).any(|(a, b)| {
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
            }))
        {
            return None;
        }
        let layout = Self {
            units: Rc::new(projection.units),
            legacy: Rc::new((projection.slices, projection.occupied_height)),
            occupied: Rc::new(projection.occupied),
            width: Some(width),
        };
        let mut next = cursor.clone();
        next.layout = layout.units.clone();
        if let UnitCursor::Row {
            remainder,
            fragment,
        } = &cursor.part
        {
            next.part = UnitCursor::Row {
                remainder: Rc::new(crate::office::docx::layout::note_source::remeasure_row(
                    remainder, width, env,
                )?),
                fragment: *fragment,
            };
        }
        Some((layout, next))
    }

    /// Preserve the current unsplit renderer while fragment consumers are wired.
    pub fn legacy_slices(&self) -> (Vec<Slice>, f64) {
        self.legacy.as_ref().clone()
    }

    #[cfg(test)]
    pub fn test_height(height: f64) -> Self {
        Self {
            units: Rc::new(Vec::new()),
            legacy: Rc::new((Vec::new(), height)),
            occupied: Rc::new(Vec::new()),
            width: None,
        }
    }

    pub fn height(&self) -> f64 {
        self.legacy.1
    }

    pub fn start(&self) -> NoteCursor {
        NoteCursor {
            layout: self.units.clone(),
            unit: 0,
            part: UnitCursor::Start,
            rows: Rc::new(Vec::new()),
            fresh_rows: Rc::new(Vec::new()),
            capacities: Rc::new(Vec::new()),
            merges: None,
            destination: 0,
            occurrence: Rc::new(()),
        }
    }

    /// Space source cell keys far enough apart for every retained nested slice.
    pub fn reading_stride(&self) -> usize {
        fn slices_count(slices: &[Slice]) -> usize {
            slices
                .iter()
                .map(|s| {
                    1 + s.anchors.len()
                        + s.nested_table
                            .as_ref()
                            .map_or(0, |child| child.layout.reading_stride())
                        + s.nested_row.as_ref().map_or(0, |r| {
                            r.cells
                                .iter()
                                .map(|c| slices_count(&c.slices))
                                .sum::<usize>()
                        })
                })
                .sum()
        }
        1 + 2 * self
            .units
            .iter()
            .map(|u| match u {
                Unit::Para(p) => p.lines.iter().map(|l| 1 + l.anchors.len()).sum::<usize>(),
                Unit::Row(r) => r
                    .cells
                    .iter()
                    .map(|c| {
                        slices_count(&c.slices) + c.moved.as_ref().map_or(0, |m| slices_count(&m.0))
                    })
                    .sum(),
            })
            .sum::<usize>()
    }

    pub fn units(&self) -> &[Unit] {
        &self.units
    }
}

#[derive(Clone)]
pub struct NoteCursor {
    layout: Rc<Vec<Unit>>,
    pub unit: usize,
    pub part: UnitCursor,
    rows: Rc<Vec<Placed>>,
    fresh_rows: Rc<Vec<bool>>,
    capacities: Rc<Vec<f64>>,
    merges: Option<merges::Batch>,
    destination: usize,
    occurrence: Rc<()>,
}

#[derive(Clone)]
pub enum UnitCursor {
    Start,
    Paragraph {
        line: usize,
    },
    Row {
        remainder: Rc<RowUnit>,
        fragment: usize,
    },
}

impl NoteCursor {
    pub(crate) fn same_occurrence(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.occurrence, &other.occurrence)
    }

    pub fn complete(&self) -> bool {
        self.unit == self.layout.len()
    }
    pub(super) fn advance(&mut self) {
        self.unit += 1;
        self.part = UnitCursor::Start;
    }
}

#[derive(Clone, Copy)]
pub struct FragmentBudget {
    /// Remaining destination space after body, separator and notice reservation.
    pub height: f64,
    /// Full destination capacity, used to distinguish movable keeps from oversized keeps.
    pub fresh_capacity: f64,
    pub fresh: bool,
    pub allow_atomic_overflow: bool,
}

#[derive(Clone)]
pub enum NotePlacement {
    /// Line identities refer to immutable units, including their anchors and boxes.
    Paragraph {
        unit: usize,
        first: usize,
        end: usize,
        y: f64,
        line_tops: Vec<f64>,
    },
    Row {
        unit: usize,
        row: Rc<RowUnit>,
        y: f64,
        fragment: usize,
        history: usize,
        merge_paint: Vec<merges::Paint>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FragmentOutcome {
    Complete,
    Continue,
    Break(BreakKind),
    Retry,
    AtomicOverflow,
    NeedsStructuredRow,
}

#[derive(Clone)]
pub struct FragmentPlan {
    pub placements: Vec<NotePlacement>,
    pub after: NoteCursor,
    pub used_height: f64,
    pub outcome: FragmentOutcome,
}

#[derive(Debug, PartialEq, Eq)]
pub enum PlanError {
    InvalidBudget,
    WrongLayout,
    InvalidCursor,
}

fn segment_end(p: &crate::office::docx::layout::ParaUnit, first: usize) -> usize {
    p.lines[first..]
        .iter()
        .position(|l| l.break_after.is_some())
        .map_or(p.lines.len(), |i| first + i + 1)
}

fn prefix_height(
    p: &crate::office::docx::layout::ParaUnit,
    first: usize,
    end: usize,
    extents: &[f64],
) -> f64 {
    let mut flow = if first == 0 {
        p.before + p.pad_top
    } else {
        0.0
    };
    let mut occupied: f64 = flow;
    for i in first..end {
        let line = &p.lines[i];
        flow += line.skip;
        occupied = occupied.max(flow + extents.get(i).copied().unwrap_or(line.height));
        flow += line.height;
    }
    if end == p.lines.len() {
        flow += p.pad_bottom + p.after;
    }
    flow.max(occupied)
}

fn minimum_prefix(p: &crate::office::docx::layout::ParaUnit, first: usize, extents: &[f64]) -> f64 {
    let end = segment_end(p, first);
    let remaining = end - first;
    let segment_start = first == 0 || p.lines[first - 1].break_after.is_some();
    let widow = p.widow && end == p.lines.len();
    let mut count = if p.keep_lines && segment_start {
        remaining
    } else if widow && first == 0 {
        2.min(remaining)
    } else {
        1.min(remaining)
    };
    if widow && remaining - count == 1 {
        count += 1;
    }
    prefix_height(p, first, first + count, extents)
}

fn required_start(units: &[Unit], from: usize, part: &UnitCursor, occupied: &[Vec<f64>]) -> f64 {
    let mut height = 0.0;
    let mut occupied_height: f64 = 0.0;
    for (offset, u) in units[from..].iter().enumerate() {
        let extents = occupied.get(from + offset).map_or(&[][..], Vec::as_slice);
        if offset > 0
            && (matches!(u, Unit::Para(p) if p.page_break_before)
                || matches!(u, Unit::Row(r) if r.page_break_before))
        {
            break;
        }
        let first = if offset == 0 {
            match part {
                UnitCursor::Paragraph { line } => *line,
                _ => 0,
            }
        } else {
            0
        };
        let required = match u {
            Unit::Row(r) => {
                if offset == 0 {
                    match part {
                        UnitCursor::Row { remainder, .. } => remainder.flow_height(),
                        _ => r.flow_height(),
                    }
                } else {
                    r.flow_height()
                }
            }
            Unit::Para(p) if p.keep_next || p.keep_lines => {
                prefix_height(p, first, segment_end(p, first), extents)
            }
            Unit::Para(p) => minimum_prefix(p, first, extents),
        };
        occupied_height = occupied_height.max(height + required);
        height += match u {
            Unit::Para(p) if p.keep_next || p.keep_lines => {
                prefix_height(p, first, segment_end(p, first), &[])
            }
            _ => required,
        };
        if !u.keep_next()
            || matches!(u, Unit::Para(p) if p.lines[first..].iter().any(|l| l.break_after.is_some()))
        {
            break;
        }
    }
    height.max(occupied_height)
}

/// Top-level merge chains use the note-local history; nested merge ledgers remain separate.
pub(super) fn supported_row(row: &RowUnit) -> bool {
    row.float_y.is_none()
        && row.cells.iter().all(|c| {
            (!row.exact_height || c.moved.is_none())
                && (c.moved.is_none() || (c.merged_from > 0 && c.merged_from <= row.index))
                && c.slices
                    .iter()
                    .chain(c.moved.iter().flat_map(|m| m.0.iter()))
                    .all(|s| {
                        s.nested_row
                            .as_ref()
                            .is_none_or(|r| r.fragmentable_structure())
                    })
        })
}

fn row_content(row: &RowUnit) -> usize {
    row.cells
        .iter()
        .flat_map(|c| &c.slices)
        .map(|s| s.nested_row.as_ref().map_or(1, |r| row_content(r)))
        .sum()
}

/// A trial only clones source state. A cursor is valid for its exact width/grid layout.
#[cfg(test)]
pub fn plan_fragment(
    layout: &NoteLayout,
    from: &NoteCursor,
    budget: FragmentBudget,
) -> Result<FragmentPlan, PlanError> {
    plan_fragment_in_env(layout, from, budget, None)
}

pub(crate) fn plan_fragment_in_env(
    layout: &NoteLayout,
    from: &NoteCursor,
    budget: FragmentBudget,
    env: Option<&crate::office::docx::text::TextEnv<'_>>,
) -> Result<FragmentPlan, PlanError> {
    if !budget.height.is_finite()
        || !budget.fresh_capacity.is_finite()
        || budget.height < 0.0
        || budget.fresh_capacity <= 0.0
        || budget.height > budget.fresh_capacity + EPS
    {
        return Err(PlanError::InvalidBudget);
    }
    if !Rc::ptr_eq(&layout.units, &from.layout) {
        return Err(PlanError::WrongLayout);
    }
    if from.unit > layout.units.len() {
        return Err(PlanError::InvalidCursor);
    }
    let mut out = FragmentPlan {
        placements: Vec::new(),
        after: from.clone(),
        used_height: 0.0,
        outcome: FragmentOutcome::Complete,
    };
    out.after.destination += 1;
    let mut chain: Option<(NoteCursor, usize, f64)> = None;
    let mut initial_headers: Option<(NoteCursor, usize, f64)> = None;
    while !out.after.complete() {
        let ui = out.after.unit;
        let u = &layout.units[ui];
        let start = matches!(out.after.part, UnitCursor::Start);
        let fresh = budget.fresh && out.placements.is_empty();
        let room = budget.height - out.used_height;
        let first = match (&out.after.part, u) {
            (UnitCursor::Start, _) | (UnitCursor::Row { .. }, Unit::Row(_)) => 0,
            (UnitCursor::Paragraph { line }, Unit::Para(p)) if *line < p.lines.len() => *line,
            _ => return Err(PlanError::InvalidCursor),
        };
        let header_height = if fresh {
            match u {
                Unit::Row(row) if !row.header && out.after.rows.last()
                    .is_some_and(|p| matches!(p, Placed::Row { row: prior, .. } if prior.table == row.table)) =>
                    row.headers.iter().map(RowUnit::flow_height).sum::<f64>(),
                _ => 0.0,
            }
        } else {
            0.0
        };
        let fresh_capacity = budget.fresh_capacity
            - if header_height < room - EPS {
                header_height
            } else {
                0.0
            };
        if u.keep_next()
            && !matches!(u, Unit::Para(p) if p.lines[first..].iter().any(|l| l.break_after.is_some()))
        {
            let required = required_start(&layout.units, ui, &out.after.part, &layout.occupied);
            if required <= fresh_capacity + EPS && required > room + EPS
                || required > fresh_capacity + EPS && !fresh
            {
                out.outcome = FragmentOutcome::Retry;
                break;
            }
            if required <= fresh_capacity + EPS && chain.is_none() {
                chain = Some((out.after.clone(), out.placements.len(), out.used_height));
            }
        }
        match u {
            Unit::Para(p) => {
                let extents = layout.occupied.get(ui).map_or(&[][..], Vec::as_slice);
                let first = match out.after.part {
                    UnitCursor::Start => 0,
                    UnitCursor::Paragraph { line } if line < p.lines.len() => line,
                    _ => return Err(PlanError::InvalidCursor),
                };
                if first == 0 && p.page_break_before && !fresh {
                    chain = None;
                    out.outcome = FragmentOutcome::Break(BreakKind::Page);
                    break;
                }
                let before = if first == 0 {
                    p.before + p.pad_top
                } else {
                    0.0
                };
                if p.lines.is_empty() && before + p.pad_bottom + p.after > room + EPS {
                    if fresh
                        && budget.allow_atomic_overflow
                        && before + p.pad_bottom + p.after > budget.fresh_capacity + EPS
                    {
                        out.outcome = FragmentOutcome::AtomicOverflow;
                    } else {
                        out.outcome = FragmentOutcome::Retry;
                        break;
                    }
                }
                let origin = out.used_height + before;
                let mut y = origin;
                let mut tops = Vec::new();
                let mut end = first;
                while end < p.lines.len() {
                    let line = &p.lines[end];
                    let tail = if end + 1 == p.lines.len() {
                        p.pad_bottom + p.after
                    } else {
                        0.0
                    };
                    let bottom = y + line.skip + line.height;
                    let occupied_bottom =
                        y + line.skip + extents.get(end).copied().unwrap_or(line.height);
                    if (bottom + tail).max(occupied_bottom) > budget.height + EPS {
                        break;
                    }
                    tops.push(y + line.skip);
                    y = bottom;
                    end += 1;
                    if line.break_after.is_some() {
                        break;
                    }
                }
                let manual = end > first && p.lines[end - 1].break_after.is_some();
                if end < p.lines.len() && !manual {
                    if (first == 0 || p.lines[first - 1].break_after.is_some())
                        && p.keep_lines
                        && prefix_height(p, first, segment_end(p, first), extents)
                            <= budget.fresh_capacity + EPS
                    {
                        end = first;
                    } else if p.widow
                        && p.lines.len() >= 2
                        && segment_end(p, first) == p.lines.len()
                    {
                        if p.lines.len() - end == 1 && end > first {
                            end -= 1;
                        }
                        if first == 0 && end == 1 {
                            end = 0;
                        }
                    }
                    if end == first && fresh {
                        // Oversized keeps relax only when they cannot fit a full destination.
                        let required = minimum_prefix(p, first, extents);
                        if required > budget.fresh_capacity + EPS && !tops.is_empty() {
                            end = first + tops.len();
                        }
                    }
                }
                if end == first && !p.lines.is_empty() {
                    let line = &p.lines[first];
                    let tail = if first + 1 == p.lines.len() {
                        p.pad_bottom + p.after
                    } else {
                        0.0
                    };
                    let atomic = before
                        + line.skip
                        + (line.height + tail)
                            .max(extents.get(first).copied().unwrap_or(line.height));
                    if fresh && budget.allow_atomic_overflow && atomic > budget.fresh_capacity + EPS
                    {
                        end += 1;
                        tops = vec![origin + line.skip];
                        out.outcome = FragmentOutcome::AtomicOverflow;
                    } else {
                        out.outcome = FragmentOutcome::Retry;
                        break;
                    }
                }
                tops.truncate(end - first);
                y = tops
                    .last()
                    .map_or(origin, |top| top + p.lines[end - 1].height);
                if end == p.lines.len() {
                    y += p.pad_bottom + p.after;
                }
                out.placements.push(NotePlacement::Paragraph {
                    unit: ui,
                    first,
                    end,
                    // Paragraph-relative objects use the accepted fragment's origin,
                    // matching body paragraph continuation. Source spacing applies once.
                    y: origin
                        - if first == 0 {
                            p.before + p.pad_top
                        } else {
                            0.0
                        },
                    line_tops: tops,
                });
                out.used_height = y;
                if end == p.lines.len() {
                    out.after.advance();
                } else {
                    out.after.part = UnitCursor::Paragraph { line: end };
                }
                if !p.keep_next {
                    chain = None;
                }
                if out.outcome == FragmentOutcome::AtomicOverflow {
                    break;
                }
                if end > first
                    && let Some(kind) = p.lines[end - 1].break_after
                {
                    chain = None;
                    out.outcome = FragmentOutcome::Break(kind);
                    break;
                }
                if end < p.lines.len() {
                    out.outcome = FragmentOutcome::Continue;
                    break;
                }
            }
            Unit::Row(original) => {
                let (row, fragment) = match &out.after.part {
                    UnitCursor::Start => (Rc::new(original.clone()), 0),
                    UnitCursor::Row {
                        remainder,
                        fragment,
                    } => (remainder.clone(), *fragment),
                    _ => return Err(PlanError::InvalidCursor),
                };
                if !supported_row(&row) {
                    out.outcome = FragmentOutcome::NeedsStructuredRow;
                    break;
                }
                if start && row.page_break_before && !fresh {
                    chain = None;
                    out.outcome = FragmentOutcome::Break(BreakKind::Page);
                    break;
                }
                if row.header && initial_headers.is_none() {
                    initial_headers =
                        Some((out.after.clone(), out.placements.len(), out.used_height));
                }
                let initial_height = initial_headers
                    .as_ref()
                    .map_or(0.0, |(_, _, height)| out.used_height - height);
                let fresh = fresh
                    || budget.fresh
                        && initial_headers
                            .as_ref()
                            .is_some_and(|(_, count, _)| *count == 0);
                if row.cant_split && row.flow_height() > room + EPS && !fresh {
                    out.outcome = FragmentOutcome::Retry;
                    break;
                }
                let headers = if fresh
                    && initial_headers.is_none()
                    && !row.header
                    && !row.headers.is_empty()
                    && out.after.rows.last().is_some_and(
                        |p| matches!(p, Placed::Row { row: prior, .. } if prior.table == row.table),
                    ) {
                    row.headers.as_ref().clone()
                } else {
                    Vec::new()
                };
                let header_height: f64 = headers.iter().map(RowUnit::flow_height).sum();
                let headers = if header_height < room - EPS {
                    headers
                } else {
                    Vec::new()
                };
                let header_height: f64 = headers.iter().map(RowUnit::flow_height).sum();
                let room = room - header_height;
                let fresh_capacity =
                    (budget.fresh_capacity - header_height - initial_height).max(0.0);
                let mut row = row.as_ref().clone();
                let row_y = out.used_height + header_height + row.outer_before;
                if start {
                    out.after.merges = merges::Batch::for_note(
                        &mut row,
                        &out.after.rows,
                        out.after.destination,
                        row_y,
                        room,
                        merges::NoteHistory {
                            fresh_capacity,
                            fresh_rows: &out.after.fresh_rows,
                            capacities: &out.after.capacities,
                            env,
                        },
                    );
                    if out.after.merges.is_some() {
                        row.height = row.merge_base_height;
                    }
                }
                let history = out.after.rows.len() + headers.len();
                if let Some(batch) = &mut out.after.merges {
                    batch.fresh_capacity(fresh_capacity);
                    batch.prepare_source(
                        out.after.destination,
                        0.0,
                        row_y,
                        history,
                        fresh,
                        merges::SourceProjection {
                            row: Some(&row),
                            env,
                        },
                    );
                    if !batch.valid() {
                        return Err(PlanError::InvalidCursor);
                    }
                    row.height = row.height.max(batch.required(row_y));
                }
                let reserve = out
                    .after
                    .merges
                    .as_mut()
                    .map(|batch| batch.preview_in_env(row_y, room - row.outer_before, env));
                let row = Rc::new(row);
                let (piece, rest) = if row.flow_height() <= room + EPS {
                    (row.clone(), None)
                } else if (row.cant_split || row.exact_height)
                    && row.flow_height() <= fresh_capacity + EPS
                {
                    out.outcome = FragmentOutcome::Retry;
                    break;
                } else {
                    let merge_stalled = out.after.merges.is_some()
                        && reserve.is_some_and(|r| !r.progress)
                        && row_content(&row) == 0;
                    let split = if row.exact_height || merge_stalled {
                        None
                    } else {
                        crate::office::docx::layout::split_row_in_env(
                            &row,
                            room,
                            !fresh,
                            reserve,
                            fresh_capacity,
                            env,
                        )
                    };
                    if let Some((top, rest)) = split.filter(|(top, rest)| {
                        top.flow_height() > EPS
                            && (row_content(rest) < row_content(&row)
                                || rest.flow_height() < row.flow_height() - EPS)
                    }) {
                        (Rc::new(top), Some(Rc::new(rest)))
                    } else if row_content(&row) == 0
                        && !merge_stalled
                        && !row.exact_height
                        && room > row.outer_before + EPS
                        && row.height > room - row.outer_before + EPS
                    {
                        let mut top = row.as_ref().clone();
                        let mut rest = top.clone();
                        top.height = room - row.outer_before;
                        top.outer_after = 0.0;
                        rest.height = (row.height - top.height).max(0.0);
                        rest.outer_before = 0.0;
                        top.bookmarks_after.clear();
                        rest.bookmarks_before.clear();
                        if let Some(source) = &mut rest.source {
                            let source = Rc::make_mut(source);
                            source.minimum = (source.minimum - top.height).max(0.0);
                            source.addition = 0.0;
                        }
                        (Rc::new(top), Some(Rc::new(rest)))
                    } else if fresh
                        && budget.allow_atomic_overflow
                        && row.flow_height() > fresh_capacity + EPS
                    {
                        out.outcome = FragmentOutcome::AtomicOverflow;
                        (row.clone(), None)
                    } else {
                        out.outcome = FragmentOutcome::Retry;
                        break;
                    }
                };
                for mut header in headers {
                    header.repeating_header = true;
                    let header_index = out.after.rows.len();
                    Rc::make_mut(&mut out.after.fresh_rows).push(false);
                    Rc::make_mut(&mut out.after.capacities).push(fresh_capacity);
                    Rc::make_mut(&mut out.after.rows).push(Placed::Row {
                        row: Box::new(header.clone()),
                        merge_paint: Vec::new(),
                        page: out.after.destination,
                        column: 0,
                        normal_flow: true,
                        x: 0.0,
                        y: out.used_height + header.outer_before,
                    });
                    out.placements.push(NotePlacement::Row {
                        unit: layout.units.iter().position(|u| matches!(u, Unit::Row(r) if r.table == header.table && r.index == header.index)).unwrap_or(ui),
                        row: Rc::new(header.clone()), y: out.used_height,
                        fragment: 0, history: header_index, merge_paint: Vec::new(),
                    });
                    out.used_height += header.flow_height();
                }
                let merge_paint = out.after.merges.as_mut().map_or_else(Vec::new, |batch| {
                    batch.commit_in_env(row_y, piece.height, history, env)
                });
                Rc::make_mut(&mut out.after.fresh_rows).push(fresh);
                Rc::make_mut(&mut out.after.capacities).push(fresh_capacity);
                Rc::make_mut(&mut out.after.rows).push(Placed::Row {
                    row: Box::new(piece.as_ref().clone()),
                    merge_paint: Vec::new(),
                    page: out.after.destination,
                    column: 0,
                    normal_flow: true,
                    x: 0.0,
                    y: row_y,
                });
                out.placements.push(NotePlacement::Row {
                    unit: ui,
                    row: piece.clone(),
                    y: out.used_height,
                    fragment,
                    history,
                    merge_paint,
                });
                out.used_height += piece.flow_height();
                if let Some(remainder) = rest {
                    out.after.part = UnitCursor::Row {
                        remainder,
                        fragment: fragment + 1,
                    };
                    out.outcome = FragmentOutcome::Continue;
                } else {
                    out.after.merges = None;
                    out.after.advance();
                }
                if !row.header {
                    initial_headers = None;
                }
                if !row.keep_next {
                    chain = None;
                }
                if out.outcome != FragmentOutcome::Complete {
                    break;
                }
            }
        }
    }
    if matches!(
        out.outcome,
        FragmentOutcome::Retry | FragmentOutcome::NeedsStructuredRow | FragmentOutcome::Break(_)
    ) {
        let header_rollback = if matches!(out.outcome, FragmentOutcome::Break(_)) {
            None
        } else {
            initial_headers
        };
        if let Some((cursor, count, height)) = chain.or(header_rollback) {
            out.after = cursor;
            out.placements.truncate(count);
            out.used_height = height;
        }
    }
    if out.outcome == FragmentOutcome::Retry && !out.placements.is_empty() {
        out.outcome = FragmentOutcome::Continue;
    }
    // Recompute from the final accepted prefix after widow, keep and header rollback.
    // Side-wrapped pictures reserve height without changing subsequent text origins.
    for placement in &out.placements {
        if let NotePlacement::Paragraph {
            unit,
            first,
            line_tops,
            ..
        } = placement
        {
            let Unit::Para(p) = &layout.units[*unit] else {
                unreachable!()
            };
            let extents = layout.occupied.get(*unit).map_or(&[][..], Vec::as_slice);
            for (i, top) in (*first..).zip(line_tops) {
                out.used_height = out
                    .used_height
                    .max(top + extents.get(i).copied().unwrap_or(p.lines[i].height));
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
