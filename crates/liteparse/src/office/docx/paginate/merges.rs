//! Bounds merged-cell slices before committing physical row fragments.

use super::{EPS, Placed};
use crate::office::docx::layout::{MergeReservation, RowUnit, Slice};
use crate::office::docx::text::translate_item;
use crate::office::pptx::scene::Item;
use std::rc::Rc;

#[derive(Clone)]
pub struct Paint {
    pub fragment_source: Option<std::sync::Arc<crate::office::pptx::scene::TextSource>>,
    pub owner: usize,
    pub first: usize,
    pub cell: usize,
    pub items: Vec<Item>,
    pub anchors: Vec<(crate::office::docx::model::Anchored, f64, f64)>,
    pub notes: Vec<i64>,
    pub anchor_keys: Vec<usize>,
    pub restart: (usize, usize),
    pub reading_key: Option<usize>,
    pub body_source_order: bool,
    pub retained: Option<RetainedPaint>,
}

#[derive(Clone)]
pub struct RetainedPaint {
    pub slices: Vec<Slice>,
    pub slot: usize,
    pub x: f64,
    pub y: f64,
}

pub(super) struct NoteHistory<'a, 'fonts> {
    pub fresh_capacity: f64,
    pub fresh_rows: &'a [bool],
    pub capacities: &'a [f64],
    pub env: Option<&'a crate::office::docx::text::TextEnv<'fonts>>,
}

pub(super) struct SourceProjection<'a, 'fonts> {
    pub row: Option<&'a RowUnit>,
    pub env: Option<&'a crate::office::docx::text::TextEnv<'fonts>>,
}

#[derive(Clone, Copy)]
struct Part {
    source_row: usize,
    page: usize,
    x: f64,
    top: f64,
    bottom: f64,
    first: usize,
    last: usize,
    fresh: bool,
    // Accepted cell x, width, owner margins and fresh content capacity, in points.
    cell_geometry: Option<(f64, f64, [f64; 4], f64)>,
}

// Adjacent rows share one merged content interval when their cell geometry agrees.
// The first accepted interval retains its fresh-area fitting capacity.
fn same_cell_geometry(
    first: Option<(f64, f64, [f64; 4], f64)>,
    second: Option<(f64, f64, [f64; 4], f64)>,
) -> bool {
    first.map(|(x, width, margins, _)| (x, width, margins))
        == second.map(|(x, width, margins, _)| (x, width, margins))
}

#[derive(Clone)]
struct Cursor {
    row_sources:
        std::collections::HashMap<usize, std::sync::Arc<crate::office::pptx::scene::TextSource>>,
    cell: usize,
    source_cell: usize,
    x: f64,
    margins: [f64; 4],
    source: Rc<(Vec<Slice>, f64)>,
    next: usize,
    start: f64,
    part: Option<Part>,
    nested: bool,
    restart: (usize, usize),
    fresh_capacity: f64,
    source_width: f64,
    source_offset: usize,
    reading_offset: usize,
}

fn shift_slice(slice: &mut Slice, delta: f64) {
    slice.y += delta;
    for item in &mut slice.items {
        translate_item(item, 0.0, delta);
    }
    for (_, _, y) in &mut slice.anchors {
        *y += delta;
    }
}

impl Cursor {
    // Historical parts retain their geometry; only unconsumed source is reflowed.
    fn project(
        &mut self,
        part: Part,
        env: Option<&crate::office::docx::text::TextEnv<'_>>,
    ) -> bool {
        let Some((x, width, margins, capacity)) = part.cell_geometry else {
            return true;
        };
        let inner = (width - margins[1] - margins[3]).max(1.0);
        if (inner - self.source_width).abs() > EPS {
            let Some(env) = env else { return false };
            let Some((slices, height)) = crate::office::docx::layout::note_source::remeasure_slices(
                &self.source.0[self.next..],
                inner,
                env,
            ) else {
                return false;
            };
            self.source_offset += self.next;
            self.source = Rc::new((slices, height));
            self.next = 0;
            self.start = 0.0;
            self.source_width = inner;
        }
        self.x = x;
        self.margins = margins;
        self.fresh_capacity = capacity;
        true
    }

    fn end(&self) -> f64 {
        crate::office::docx::layout::slices_height(&self.source.0[self.next..], 0.0)
    }

    fn fit(&self, height: f64) -> usize {
        let room = height - self.margins[0] - self.margins[2];
        let mut end = self.next;
        while let Some(s) = self.source.0.get(end) {
            if s.y + s.h - self.start > room + EPS {
                break;
            }
            end += 1;
        }
        end
    }

    fn fit_source(
        &mut self,
        height: f64,
        fresh: bool,
        env: Option<&crate::office::docx::text::TextEnv<'_>>,
    ) -> usize {
        let end = self.fit(height);
        if self.nested
            && let Some(source) = self
                .source
                .0
                .get(end)
                .filter(|s| s.nested_row.is_some() || s.nested_table.is_some())
        {
            let mut local = source.clone();
            shift_slice(&mut local, -self.start);
            let room = height - self.margins[0] - self.margins[2];
            if let Some((mut first, mut rest)) = crate::office::docx::layout::split_slice_in_env(
                &local,
                room,
                !fresh,
                (self.fresh_capacity - self.margins[0] - self.margins[2]).max(0.0),
                env,
            ) {
                shift_slice(&mut first, self.start);
                shift_slice(&mut rest, self.start);
                let delta = first.h + rest.h - source.h;
                let slices = &mut Rc::make_mut(&mut self.source).0;
                for later in &mut slices[end + 1..] {
                    shift_slice(later, delta);
                }
                slices.splice(end..=end, [first, rest]);
            }
        }
        self.fit(height)
    }

    #[expect(
        clippy::unwrap_in_result,
        reason = "`text_keys` holds one key per keyed item in the same slices the loop walks"
    )]
    fn consume(&mut self, part: Part, end: usize) -> Option<Paint> {
        let retained = self.nested.then(|| RetainedPaint {
            slices: self.source.0[self.next..end].to_vec(),
            slot: self.reading_offset,
            x: part.x + self.x + self.margins[1],
            y: part.top + self.margins[0] - self.start,
        });
        let mut items = Vec::new();
        let mut anchors = Vec::new();
        let mut notes = Vec::new();
        let mut anchor_keys = Vec::new();
        let mut text_keys = Vec::new();
        let mut anchor_slots = Vec::new();
        if self.nested {
            for slice in &self.source.0[self.next..end] {
                super::note_areas::note_slice_keys(
                    slice,
                    &mut self.reading_offset,
                    &mut text_keys,
                    &mut anchor_slots,
                );
            }
            anchor_keys.extend(anchor_slots.iter().map(|(key, _)| *key));
        }
        let mut text_keys = text_keys.into_iter();
        for s in &self.source.0[self.next..end] {
            for it in &s.items {
                let mut it = it.clone();
                if self.nested && crate::office::docx::text::reading_key(&it).is_some() {
                    crate::office::docx::text::set_reading_key(
                        std::slice::from_mut(&mut it),
                        text_keys
                            .next()
                            .expect("source text traversal matches cached slices"),
                    );
                }
                translate_item(
                    &mut it,
                    part.x + self.x + self.margins[1],
                    part.top + self.margins[0] - self.start,
                );
                items.push(it);
            }
        }
        for (offset, s) in self.source.0[self.next..end].iter().enumerate() {
            if !self.nested {
                anchor_keys.extend(
                    s.anchors
                        .iter()
                        .map(|_| (self.source_offset + self.next + offset) * 2 + 1),
                );
            }
            anchors.extend(s.anchors.iter().map(|(a, x, y)| {
                (
                    a.clone(),
                    x + part.x + self.x + self.margins[1],
                    y + part.top + self.margins[0] - self.start,
                )
            }));
            notes.extend(s.notes.iter().copied());
        }
        self.next = end;
        if let Some(s) = self.source.0.get(end)
            && end > 0
        {
            self.start = s.y;
        }
        (!items.is_empty()
            || !anchors.is_empty()
            || !notes.is_empty()
            || retained.as_ref().is_some_and(|source| {
                source
                    .slices
                    .iter()
                    .any(|s| s.nested_table.is_some() || s.nested_row.is_some())
            }))
        .then_some(Paint {
            fragment_source: self.row_sources.get(&part.source_row).cloned(),
            owner: part.last,
            first: part.first,
            cell: self.source_cell,
            items,
            anchors,
            notes,
            anchor_keys,
            restart: self.restart,
            reading_key: None,
            body_source_order: false,
            retained,
        })
    }
}

#[derive(Clone)]
pub(super) struct Batch {
    cursors: Vec<Cursor>,
    pending: Vec<Paint>,
    valid: bool,
}

impl Batch {
    pub fn new(
        row: &mut RowUnit,
        placed: &[Placed],
        page: usize,
        x: f64,
        y: f64,
        avail: f64,
    ) -> Option<Self> {
        Self::from_history(row, placed, page, x, y, avail, None)
    }

    pub fn for_note(
        row: &mut RowUnit,
        placed: &[Placed],
        destination: usize,
        y: f64,
        avail: f64,
        history: NoteHistory<'_, '_>,
    ) -> Option<Self> {
        Self::from_history(row, placed, destination, 0.0, y, avail, Some(history))
    }

    fn from_history(
        row: &mut RowUnit,
        placed: &[Placed],
        page: usize,
        x: f64,
        y: f64,
        avail: f64,
        history: Option<NoteHistory<'_, '_>>,
    ) -> Option<Self> {
        let note_capacity = history.as_ref().map(|h| (h.fresh_capacity, h.fresh_rows));
        let env = history.as_ref().and_then(|h| h.env);
        let capacities = history.as_ref().map_or(&[][..], |h| h.capacities);
        let note = history.is_some();
        if row.repeating_header || row.exact_height {
            return None;
        }
        let mut out = Self {
            cursors: Vec::new(),
            pending: Vec::new(),
            valid: true,
        };
        let intervals = (0..row.cells.len())
            .map(|cell| crate::office::docx::layout::note_source::cell_interval(row, cell))
            .collect::<Vec<_>>();
        let source_cells = row
            .cells
            .iter()
            .enumerate()
            .map(|(ci, cell)| {
                intervals[ci]
                    .and_then(|interval| {
                        crate::office::docx::layout::note_source::cell_in_source_row(
                            row,
                            row.index.saturating_sub(cell.merged_from),
                            interval,
                        )
                    })
                    .unwrap_or(ci)
            })
            .collect::<Vec<_>>();
        for (ci, cell) in row.cells.iter_mut().enumerate() {
            let Some(source) = cell.moved.clone() else {
                continue;
            };
            if cell.merged_from == 0
                || cell.merged_from > row.index
                || (!note
                    && source
                        .0
                        .iter()
                        .any(|s| !s.anchors.is_empty() || !s.notes.is_empty()))
            {
                continue;
            }
            let mut parts: Vec<Part> = Vec::new();
            for (index, p) in placed.iter().enumerate() {
                let Placed::Row {
                    row: prior,
                    page: pg,
                    x: px,
                    y: py,
                    ..
                } = p
                else {
                    continue;
                };
                if prior.table != row.table
                    || prior.index < row.index - cell.merged_from
                    || prior.index >= row.index
                    || prior.repeating_header
                {
                    continue;
                }
                let cell_geometry = if note {
                    intervals[ci].and_then(|interval| {
                        let cell_index =
                            crate::office::docx::layout::note_source::cell_in_interval(
                                prior, interval,
                            )?;
                        let accepted = &prior.cells[cell_index];
                        Some((
                            accepted.x,
                            accepted.w,
                            cell.margins,
                            capacities
                                .get(index)
                                .copied()
                                .unwrap_or(note_capacity.unwrap().0),
                        ))
                    })
                } else {
                    None
                };
                match parts.last_mut() {
                    Some(last)
                        if last.page == *pg
                            && (last.x - px).abs() < EPS
                            && (last.bottom - py).abs() < EPS
                            && same_cell_geometry(last.cell_geometry, cell_geometry) =>
                    {
                        last.bottom = py + prior.height;
                        last.last = index;
                    }
                    _ => parts.push(Part {
                        source_row: prior.index,
                        cell_geometry,
                        page: *pg,
                        x: *px,
                        top: *py,
                        bottom: py + prior.height,
                        first: index,
                        last: index,
                        fresh: note_capacity
                            .and_then(|(_, rows)| rows.get(index))
                            .copied()
                            .unwrap_or(true),
                    }),
                }
            }
            let contiguous = parts.len() == 1
                && parts.last().is_some_and(|p| {
                    p.page == page && (p.x - x).abs() < EPS && (p.bottom - y).abs() < EPS
                });
            if !note && contiguous && row.height + row.outer_before + row.outer_after <= avail + EPS
            {
                continue;
            }
            let mut cursor = Cursor {
                row_sources: placed
                    .iter()
                    .filter_map(|p| match p {
                        Placed::Row { row: prior, .. } if prior.table == row.table => {
                            prior.provenance.clone().map(|source| (prior.index, source))
                        }
                        _ => None,
                    })
                    .chain(row.provenance.clone().map(|source| (row.index, source)))
                    .collect(),
                cell: ci,
                source_cell: source_cells[ci],
                x: cell.x,
                margins: cell.margins,
                source,
                next: 0,
                start: 0.0,
                part: parts.pop(),
                nested: note,
                restart: (row.table, row.index - cell.merged_from),
                fresh_capacity: note_capacity.map_or(f64::INFINITY, |(height, _)| height),
                source_width: (cell.w - cell.margins[1] - cell.margins[3]).max(1.0),
                source_offset: 0,
                reading_offset: 0,
            };
            for part in parts {
                out.valid &= cursor.project(part, env);
                let end = cursor.fit_source(part.bottom - part.top, part.fresh, env);
                if let Some(paint) = cursor.consume(part, end) {
                    out.pending.push(paint);
                }
            }
            if let Some(part) = cursor.part {
                out.valid &= cursor.project(part, env);
            }
            cell.moved = None;
            out.cursors.push(cursor);
        }
        (!out.cursors.is_empty()).then_some(out)
    }

    pub fn fresh_capacity(&mut self, height: f64) {
        for cursor in &mut self.cursors {
            cursor.fresh_capacity = height;
        }
    }

    pub fn prepare(&mut self, page: usize, x: f64, y: f64, index: usize) {
        self.prepare_note(page, x, y, index, true);
    }

    pub fn prepare_note(&mut self, page: usize, x: f64, y: f64, index: usize, fresh: bool) {
        self.prepare_source(
            page,
            x,
            y,
            index,
            fresh,
            SourceProjection {
                row: None,
                env: None,
            },
        );
    }

    pub fn prepare_source(
        &mut self,
        page: usize,
        x: f64,
        y: f64,
        index: usize,
        fresh: bool,
        source: SourceProjection<'_, '_>,
    ) {
        let SourceProjection { row, env } = source;
        for c in &mut self.cursors {
            if let Some(row) = row
                && let Some(provenance) = &row.provenance
            {
                c.row_sources.insert(row.index, provenance.clone());
            }
            let cell_geometry = row.filter(|row| row.source.is_some()).map(|row| {
                let cell = &row.cells[c.cell];
                (cell.x, cell.w, cell.margins, c.fresh_capacity)
            });
            if c.part.is_some_and(|p| {
                p.page == page
                    && (p.x - x).abs() < EPS
                    && (p.bottom - y).abs() < EPS
                    && same_cell_geometry(p.cell_geometry, cell_geometry)
            }) {
                self.valid &= c.project(c.part.unwrap(), env);
                continue;
            }
            if let Some(part) = c.part.take() {
                self.valid &= c.project(part, env);
                let end = c.fit_source(part.bottom - part.top, part.fresh, env);
                if let Some(paint) = c.consume(part, end) {
                    self.pending.push(paint);
                }
            }
            let part = Part {
                source_row: row.map_or(c.restart.1, |row| row.index),
                cell_geometry,
                page,
                x,
                top: y,
                bottom: y,
                first: index,
                last: index,
                fresh,
            };
            self.valid &= c.project(part, env);
            c.part = Some(part);
        }
    }

    pub fn valid(&self) -> bool {
        self.valid
    }

    pub fn required(&self, y: f64) -> f64 {
        self.cursors
            .iter()
            .map(|c| {
                if c.next == c.source.0.len() {
                    return 0.0;
                }
                let prefix = y - c.part.unwrap().top;
                (c.end() - c.start + c.margins[0] + c.margins[2] - prefix).max(0.0)
            })
            .fold(0.0, f64::max)
    }

    pub fn preview(&mut self, y: f64, avail: f64) -> MergeReservation {
        self.preview_in_env(y, avail, None)
    }

    pub fn preview_in_env(
        &mut self,
        y: f64,
        avail: f64,
        env: Option<&crate::office::docx::text::TextEnv<'_>>,
    ) -> MergeReservation {
        let mut reserve = MergeReservation::default();
        for c in &mut self.cursors {
            let prefix = y - c.part.unwrap().top;
            let end = c.fit_source(prefix + avail, c.part.unwrap().fresh, env);
            reserve.progress |= end > c.next;
            if end > c.next {
                let used =
                    crate::office::docx::layout::slices_height(&c.source.0[c.next..end], 0.0)
                        - c.start
                        + c.margins[0]
                        + c.margins[2];
                reserve.top = reserve.top.max((used - prefix).max(0.0));
            }
            if let Some(next) = c.source.0.get(end) {
                reserve.rest = reserve.rest.max(
                    crate::office::docx::layout::slices_height(&c.source.0[end..], 0.0) - next.y
                        + c.margins[0]
                        + c.margins[2],
                );
            }
        }
        reserve
    }

    pub fn commit(&mut self, y: f64, height: f64, index: usize) -> Vec<Paint> {
        self.commit_in_env(y, height, index, None)
    }

    pub fn commit_in_env(
        &mut self,
        y: f64,
        height: f64,
        index: usize,
        env: Option<&crate::office::docx::text::TextEnv<'_>>,
    ) -> Vec<Paint> {
        for c in &mut self.cursors {
            let mut part = c.part.take().unwrap();
            part.bottom = y + height;
            part.last = index;
            let end = c.fit_source(part.bottom - part.top, part.fresh, env);
            if let Some(paint) = c.consume(part, end) {
                self.pending.push(paint);
            }
        }
        std::mem::take(&mut self.pending)
    }

    pub fn consumed(&self) -> usize {
        self.cursors.iter().map(|c| c.source_offset + c.next).sum()
    }

    pub fn complete(&self) -> bool {
        self.cursors.iter().all(|c| c.next == c.source.0.len())
    }
}

#[cfg(test)]
mod tests;
