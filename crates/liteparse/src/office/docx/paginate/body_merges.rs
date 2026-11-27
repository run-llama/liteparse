//! Transactional source cursors for top-aligned body merge chains.

use std::rc::Rc;

use crate::office::docx::layout::{Slice, Unit};

use super::EPS;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Destination {
    pub page: usize,
    pub column: usize,
}

#[derive(Clone)]
struct Area {
    destination: Destination,
    top: f64,
    source_top: f64,
}

#[derive(Clone)]
pub(super) struct Cursor {
    source: Rc<Vec<Slice>>,
    next: usize,
    area: Option<Area>,
}

pub(super) struct Trial {
    pub cursor: Cursor,
    pub slices: Vec<Slice>,
    pub notes: Vec<i64>,
}

#[derive(Clone)]
pub(super) struct Chain {
    pub table: usize,
    pub first: usize,
    pub last: usize,
    pub cell: usize,
    pub x: f64,
    pub width: f64,
    pub margins: [f64; 4],
    reading_stride: usize,
    pub cursor: Cursor,
}

/// Finds retained top-aligned payloads using the layout's validated merge edges.
pub(super) fn chains(units: &[Unit]) -> Vec<Chain> {
    let rows: Vec<_> = units
        .iter()
        .filter_map(|unit| match unit {
            Unit::Row(row) => Some(row),
            _ => None,
        })
        .collect();
    let mut chains = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        if row.header || row.repeating_header || row.exact_height {
            continue;
        }
        for (cell, source) in row.cells.iter().enumerate() {
            if source.merged
                || !source.merge_continues
                || source.slices.is_empty()
                || matches!(source.v_align.as_str(), "center" | "bottom")
                || !source
                    .slices
                    .iter()
                    .any(|s| !s.anchors.is_empty() || !s.notes.is_empty())
            {
                continue;
            }
            let mut last = row.index;
            for next in &rows[index + 1..] {
                if next.table != row.table || next.index != last + 1 {
                    break;
                }
                let Some(part) = next.cells.iter().find(|c| {
                    c.merged && (c.x - source.x).abs() < EPS && (c.w - source.w).abs() < EPS
                }) else {
                    break;
                };
                last = next.index;
                if !part.merge_continues {
                    break;
                }
            }
            if last > row.index {
                chains.push(Chain {
                    table: row.table,
                    first: row.index,
                    last,
                    cell,
                    x: source.x,
                    width: source.w,
                    margins: source.margins,
                    reading_stride: source
                        .slices
                        .iter()
                        .map(|slice| slice.anchors.len() + 1)
                        .max()
                        .unwrap_or(1),
                    cursor: Cursor::new(Rc::new(source.slices.clone())),
                });
            }
        }
    }
    chains
}

impl Cursor {
    pub fn new(source: Rc<Vec<Slice>>) -> Self {
        Self {
            source,
            next: 0,
            area: None,
        }
    }

    /// Extends one physical area without charging its top padding again.
    /// The returned cursor belongs to the trial until the row and notes commit.
    #[cfg(test)]
    pub fn trial_in_area(
        &self,
        destination: Destination,
        top: f64,
        bottom: f64,
        capacity: f64,
        fresh: bool,
    ) -> Trial {
        self.trial_with_geometry(destination, top, bottom, capacity, fresh, |_, _, _| true)
    }

    fn trial_with_geometry(
        &self,
        destination: Destination,
        top: f64,
        bottom: f64,
        capacity: f64,
        fresh: bool,
        can_place: impl Fn(&Slice, f64, bool) -> bool,
    ) -> Trial {
        let mut cursor = self.clone();
        if cursor
            .area
            .as_ref()
            .is_none_or(|a| a.destination != destination)
        {
            cursor.area = Some(Area {
                destination,
                top,
                source_top: if cursor.next == 0 {
                    0.0
                } else {
                    cursor.source.get(cursor.next).map_or(0.0, |s| s.y)
                },
            });
        }
        let area = cursor.area.as_ref().unwrap().clone();
        let dy = area.top - area.source_top;
        let mut slices = Vec::new();
        let mut notes = Vec::new();
        while let Some(source) = cursor.source.get(cursor.next).cloned() {
            if source.y + source.h + dy > bottom + EPS {
                let room = bottom - area.top;
                let require_move = !fresh || source.y + dy > top + EPS;
                let mut local = source.clone();
                shift(&mut local, -area.source_top);
                if let Some((mut first, mut rest)) =
                    crate::office::docx::layout::split_slice(&local, room, require_move, capacity)
                {
                    shift(&mut first, area.source_top);
                    shift(&mut rest, area.source_top);
                    let delta = first.h + rest.h - source.h;
                    let source = Rc::make_mut(&mut cursor.source);
                    for later in &mut source[cursor.next + 1..] {
                        shift(later, delta);
                    }
                    source.splice(cursor.next..=cursor.next, [first, rest]);
                    continue;
                }
                if require_move || source.h <= capacity + EPS || !slices.is_empty() {
                    break;
                }
            }
            if !can_place(&source, dy, slices.is_empty()) {
                break;
            }
            let mut slice = source.clone();
            slice.y += dy;
            for item in &mut slice.items {
                crate::office::docx::text::translate_item(item, 0.0, dy);
            }
            for (_, _, y) in &mut slice.anchors {
                *y += dy;
            }
            for id in &slice.notes {
                if !notes.contains(id) {
                    notes.push(*id);
                }
            }
            slices.push(slice);
            cursor.next += 1;
        }
        Trial {
            cursor,
            slices,
            notes,
        }
    }

    pub fn complete(&self) -> bool {
        self.next == self.source.len()
    }

    fn remaining(&self, destination: Destination, top: f64) -> f64 {
        let end = self.source.last().map_or(0.0, |s| s.y + s.h);
        if self.complete() {
            return 0.0;
        }
        self.area
            .as_ref()
            .filter(|a| a.destination == destination)
            .map_or(end - self.source[self.next].y, |a| {
                (a.top + end - a.source_top - top).max(0.0)
            })
    }
}

fn shift(slice: &mut Slice, dy: f64) {
    slice.y += dy;
    for item in &mut slice.items {
        crate::office::docx::text::translate_item(item, 0.0, dy);
    }
    for (_, _, y) in &mut slice.anchors {
        *y += dy;
    }
}

/// Occupied geometry keeps a locally anchored object inside its destination.
/// Text exclusion remains a separate decision governed by the source wrap mode.
fn occupied_bottom(
    anchor: &crate::office::docx::model::Anchored,
    page: &super::PageInfo,
    x: f64,
    y: f64,
) -> f64 {
    use crate::office::pptx::scene::Item;
    let (_, top, _, height) = super::anchor_rect(anchor, page, x, y, y);
    let mut bottom = top + height;
    let Some((items, _)) = super::anchor_item(anchor, page, x, y, y) else {
        return bottom;
    };
    for item in items {
        let edge = match item {
            Item::Path(path) => {
                path.path.bounds().bottom() as f64
                    + path.stroke.map_or(0.0, |stroke| stroke.width / 2.0)
            }
            Item::Image(image) => transformed_bottom(image.transform, 0.0, 0.0, 1.0, 1.0),
            Item::Text(run) => {
                let left = run.glyphs.iter().map(|g| g.x).fold(0.0, f64::min);
                let low = run
                    .glyphs
                    .iter()
                    .map(|g| g.y + run.descent)
                    .fold(run.descent, f64::max);
                transformed_bottom(run.transform, left, -run.ascent, run.width, low)
            }
            _ => continue,
        };
        if edge.is_finite() {
            bottom = bottom.max(edge);
        }
    }
    bottom
}

fn transformed_bottom(
    transform: tiny_skia::Transform,
    left: f64,
    top: f64,
    right: f64,
    bottom: f64,
) -> f64 {
    [(left, top), (left, bottom), (right, top), (right, bottom)]
        .into_iter()
        .map(|(x, y)| transform.ky as f64 * x + transform.sy as f64 * y + transform.ty as f64)
        .fold(f64::NEG_INFINITY, f64::max)
}

impl super::Paginator<'_> {
    pub(super) fn place_body_merge_row(
        &mut self,
        mut row: crate::office::docx::layout::RowUnit,
        si: usize,
        props: &crate::office::docx::model::SectPr,
        sections: &[crate::office::docx::model::Section],
    ) -> (usize, usize) {
        let members: Vec<_> = self
            .st
            .body_merges
            .iter()
            .enumerate()
            .filter_map(|(i, chain)| {
                (chain.table == row.table && (chain.first..=chain.last).contains(&row.index))
                    .then_some(i)
            })
            .collect();
        for &index in &members {
            let chain = &self.st.body_merges[index];
            if let Some(cell) = row
                .cells
                .iter_mut()
                .find(|c| (c.x - chain.x).abs() < EPS && (c.w - chain.width).abs() < EPS)
            {
                cell.slices.clear();
                cell.content_h = 0.0;
            }
        }
        let mut first = None;
        let mut legacy = None;
        let mut legacy_started = false;
        loop {
            let fresh = self.st.at_top;
            let before_headers = self.st.y;
            if self.st.at_top
                && !row.header
                && !row.headers.is_empty()
                && self
                    .st
                    .last_table
                    .is_some_and(|(table, page)| table == row.table && page < self.pages.len() - 1)
            {
                for header in row.headers.iter() {
                    if header.flow_height() < self.bottom() - self.st.y {
                        self.push_row(header.clone());
                    }
                }
            }
            let destination = Destination {
                page: self.pages.len() - 1,
                column: self.st.col,
            };
            let page = self.pages.last().unwrap();
            let physical = self.bottom();
            let fresh_capacity = (physical - page.top - (self.st.y - before_headers)).max(0.0);
            let mut room = (physical - self.st.y).max(0.0);
            if !legacy_started {
                legacy_started = true;
                let row_y = self.st.y + row.outer_before;
                let row_x = self.row_x(&row);
                legacy = super::merges::Batch::new(
                    &mut row,
                    &self.placed,
                    destination.page,
                    row_x,
                    row_y,
                    room,
                );
            }
            if let Some(batch) = &mut legacy {
                batch.prepare(
                    destination.page,
                    self.row_x(&row),
                    self.st.y + row.outer_before,
                    self.placed.len(),
                );
                row.height = row.height.max(batch.required(self.st.y + row.outer_before));
            }
            for &index in &members {
                let chain = &self.st.body_merges[index];
                if chain.last == row.index {
                    row.height = row.height.max(
                        chain.cursor.remaining(
                            destination,
                            self.st.y + row.outer_before + chain.margins[0],
                        ) + chain.margins[0]
                            + chain.margins[2],
                    );
                }
            }
            let accepted = loop {
                let mut batch_trial = legacy.clone();
                let split = if row.flow_height() <= room + EPS {
                    Some((row.clone(), None))
                } else if row.cant_split && !fresh {
                    None
                } else {
                    let mut reservation = crate::office::docx::layout::MergeReservation {
                        top: (room - row.outer_before).max(0.0),
                        rest: (row.flow_height() - room).max(0.0),
                        progress: room > row.outer_before + EPS,
                    };
                    if let Some(batch) = &mut batch_trial {
                        let prior =
                            batch.preview(self.st.y + row.outer_before, room - row.outer_before);
                        reservation.top = reservation.top.max(prior.top);
                        reservation.rest = reservation.rest.max(prior.rest);
                        reservation.progress |= prior.progress;
                    }
                    crate::office::docx::layout::split_row_in_area(
                        &row,
                        room,
                        !fresh,
                        Some(reservation),
                        fresh_capacity,
                    )
                    .map(|(top, rest)| (top, Some(rest)))
                };
                let Some((mut candidate, rest)) = split else {
                    break None;
                };
                let y = self.st.y + candidate.outer_before;
                let mut cursors = Vec::new();
                let mut source_paint = Vec::new();
                let mut missing_start = false;
                for &index in &members {
                    let chain = &self.st.body_merges[index];
                    let page = self.pages.last().unwrap();
                    let geometry_rejected = std::cell::Cell::new(false);
                    let trial = chain.cursor.trial_with_geometry(
                        destination,
                        y + chain.margins[0],
                        y + candidate.height - chain.margins[2],
                        (fresh_capacity - chain.margins[0] - chain.margins[2]).max(0.0),
                        fresh,
                        |slice, dy, first_slice| {
                            let fits = slice.anchors.iter().all(|(anchor, x, ay)| {
                                if !matches!(
                                    anchor.v_rel,
                                    crate::office::docx::model::VRel::Paragraph
                                        | crate::office::docx::model::VRel::Line
                                ) {
                                    return true;
                                }
                                let bottom = occupied_bottom(
                                    anchor,
                                    page,
                                    self.row_x(&row) + chain.x + chain.margins[1] + x,
                                    ay + dy,
                                );
                                let fresh_top = page.top
                                    + (self.st.y - before_headers)
                                    + candidate.outer_before
                                    + chain.margins[0];
                                let fresh_bottom = bottom - (slice.y + dy) + fresh_top;
                                bottom <= page.h + EPS
                                    || (fresh && first_slice && fresh_bottom > page.h + EPS)
                            });
                            geometry_rejected.set(!fits);
                            fits
                        },
                    );
                    missing_start |= geometry_rejected.get() && trial.slices.is_empty();
                    if let Some(cell) = candidate
                        .cells
                        .iter_mut()
                        .find(|c| (c.x - chain.x).abs() < EPS && (c.w - chain.width).abs() < EPS)
                    {
                        cell.margins = chain.margins;
                        cell.v_align = "top".into();
                        cell.merged_from = candidate.index - chain.first;
                        cell.slices = trial.slices;
                        for slice in &mut cell.slices {
                            shift(slice, -y - chain.margins[0]);
                        }
                        cell.content_h = cell.slices.last().map_or(0.0, |s| s.y + s.h);
                        let owner = self.placed.len();
                        let first = self.placed.iter().position(|placed| matches!(placed,
                            super::Placed::Row { row: prior, page, column, .. }
                            if *page == destination.page && *column == destination.column
                                && prior.table == chain.table && (chain.first..=chain.last).contains(&prior.index)
                        )).unwrap_or(owner);
                        let mut paint = super::merges::Paint {
                            fragment_source: self
                                .placed
                                .get(first)
                                .and_then(|p| match p {
                                    super::Placed::Row { row, .. } => row.provenance.clone(),
                                    _ => None,
                                })
                                .or_else(|| candidate.provenance.clone()),
                            owner,
                            first,
                            cell: chain.cell,
                            items: Vec::new(),
                            anchors: Vec::new(),
                            notes: trial.notes,
                            anchor_keys: Vec::new(),
                            restart: (chain.table, chain.first),
                            reading_key: None,
                            body_source_order: true,
                            retained: None,
                        };
                        for (offset, slice) in cell.slices.iter_mut().enumerate() {
                            let source_key = (chain.cursor.next + offset) * chain.reading_stride;
                            for mut item in std::mem::take(&mut slice.items) {
                                crate::office::docx::text::set_reading_key(
                                    std::slice::from_mut(&mut item),
                                    source_key,
                                );
                                crate::office::docx::text::translate_item(
                                    &mut item,
                                    self.row_x(&row) + cell.x + cell.margins[1],
                                    y + cell.margins[0],
                                );
                                paint.items.push(item);
                            }
                            for (ordinal, (anchor, x, ay)) in
                                std::mem::take(&mut slice.anchors).into_iter().enumerate()
                            {
                                paint.anchors.push((
                                    anchor,
                                    x + self.row_x(&row) + cell.x + cell.margins[1],
                                    ay + y + cell.margins[0],
                                ));
                                paint.anchor_keys.push(source_key + 1 + ordinal);
                            }
                        }
                        source_paint.push(paint);
                    }
                    cursors.push((index, trial.cursor));
                }
                if missing_start && !fresh {
                    break None;
                }
                let pending = candidate.notes();
                let bottom = self.st.y + candidate.flow_height();
                let page = self.pages.last().unwrap();
                let mut anchors =
                    crate::office::docx::layout::row_anchors(&candidate, self.row_x(&row), y);
                anchors.extend(
                    source_paint
                        .iter()
                        .flat_map(|paint| paint.anchors.iter().cloned()),
                );
                let pictures: Vec<_> = anchors
                    .iter()
                    .filter(|(anchor, _, _)| Self::movable_picture(anchor, page))
                    .map(|(anchor, x, y)| {
                        let (left, top, width, height) =
                            super::anchor_rect(anchor, page, *x, *y, *y);
                        (top, top + height, left, left + width)
                    })
                    .collect();
                let limit = self.note_limit_with_pictures(page, &pending, bottom, &pictures);
                let allowed = self.st.cap.map_or(limit, |cap| limit.min(cap)) - self.st.y;
                if allowed + EPS < candidate.flow_height() {
                    if allowed <= EPS || allowed >= room - EPS {
                        break None;
                    }
                    room = allowed;
                    continue;
                }
                let slices: Vec<_> = candidate.cells.iter().flat_map(|c| &c.slices).collect();
                let single_source = candidate.cells.iter().all(|cell| cell.slices.len() <= 1);
                if self.new_note_start_conflict(
                    page,
                    &pending,
                    bottom,
                    fresh && single_source,
                    &pictures,
                ) {
                    let step = slices
                        .iter()
                        .filter(|s| s.h > EPS)
                        .map(|s| s.h)
                        .fold(f64::INFINITY, f64::min);
                    if !step.is_finite() || room <= step + EPS {
                        break None;
                    }
                    room -= step;
                    continue;
                }
                let plan = self.plan_page_notes_with_pictures(page, &pending, bottom, &pictures);
                if matches!(plan, Some(Err(()))) {
                    break None;
                }
                break Some((
                    candidate,
                    rest,
                    cursors,
                    pending,
                    pictures,
                    plan,
                    batch_trial,
                    source_paint,
                ));
            };
            let Some((
                mut candidate,
                rest,
                cursors,
                pending,
                pictures,
                plan,
                mut batch_trial,
                mut source_paint,
            )) = accepted
            else {
                self.next_column(si, props, sections);
                continue;
            };
            first.get_or_insert((destination.page, destination.column));
            let height = candidate.flow_height();
            let mut paint = batch_trial.as_mut().map_or_else(Vec::new, |batch| {
                batch.commit(
                    self.st.y + candidate.outer_before,
                    candidate.height,
                    self.placed.len(),
                )
            });
            paint.append(&mut source_paint);
            legacy = batch_trial;
            self.commit_note_plan(&pending, self.st.y + height, plan);
            for (index, cursor) in cursors {
                self.st.body_merges[index].cursor = cursor;
            }
            self.st.picture_rects.extend(pictures);
            let continues = members.iter().any(|&i| {
                self.st.body_merges[i].last == row.index
                    && !self.st.body_merges[i].cursor.complete()
            });
            if rest.is_some() || continues {
                candidate.bookmarks_after.clear();
            }
            self.placed.push(super::Placed::Row {
                row: Box::new(candidate),
                merge_paint: paint,
                page: destination.page,
                column: destination.column,
                normal_flow: self.st.float.is_none(),
                x: self.row_x(&row),
                y: self.st.y + row.outer_before,
            });
            self.st.y += height;
            self.record_flow_bottom();
            self.st.at_top = false;
            self.st.section_start = false;
            self.st.hard_break = false;
            self.st.last_table = Some((row.table, destination.page));
            if let Some(rest) = rest {
                row = rest;
                self.next_column(si, props, sections);
            } else if members.iter().any(|&i| {
                self.st.body_merges[i].last == row.index
                    && !self.st.body_merges[i].cursor.complete()
            }) {
                for cell in &mut row.cells {
                    cell.slices.clear();
                    cell.content_h = 0.0;
                    cell.moved = None;
                }
                row.height = 0.0;
                row.outer_before = 0.0;
                row.bookmarks_before.clear();
                self.next_column(si, props, sections);
            } else {
                break;
            }
        }
        first.unwrap()
    }
}

#[cfg(test)]
mod tests;
