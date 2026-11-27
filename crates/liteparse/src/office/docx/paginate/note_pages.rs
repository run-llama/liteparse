//! Transactional note-area plans start from immutable incoming cursors.

#[cfg(test)]
use super::note_fragments::plan_fragment;

use super::note_areas::{NoteContext, NoteReference};
use super::note_fragments::{
    FragmentBudget, FragmentOutcome, NoteCursor, NoteLayout, NotePlacement, UnitCursor,
};
use super::*;
use crate::office::docx::model::BreakKind;

#[derive(Clone)]
pub(super) struct PendingNote {
    reference: NoteReference,
    layout: NoteLayout,
    cursor: NoteCursor,
    first_page: Option<usize>,
    special: bool,
    suppress_separator: bool,
}

#[derive(Clone)]
struct NotePiece {
    source: PendingNote,
    placements: Vec<NotePlacement>,
    y: f64,
}

#[derive(Clone)]
struct NoteAreaPlan {
    context: NoteContext,
    column: usize,
    outgoing: Vec<PendingNote>,
    pieces: Vec<NotePiece>,
    body_bottom: f64,
    height: f64,
    explicit_break: Option<BreakKind>,
    atomic_overflow: bool,
    notice_height: f64,
    separator_layout: Option<NoteLayout>,
    notice_layout: Option<NoteLayout>,
    overflow_story: Option<PendingNote>,
    special_overflow: bool,
}

fn row_horizontal_span(row: &RowUnit) -> (f64, f64) {
    let mut span = (0.0_f64, 0.0_f64);
    for cell in &row.cells {
        span.0 = span.0.min(cell.x);
        span.1 = span.1.max(cell.x + cell.w);
        for slice in cell
            .slices
            .iter()
            .chain(cell.moved.iter().flat_map(|m| &m.0))
        {
            if let Some(child) = &slice.nested_table {
                for placement in &child.plan.placements {
                    if let NotePlacement::Row { row, .. } = placement {
                        let nested = row_horizontal_span(row);
                        span.0 = span.0.min(cell.x + cell.margins[1] + nested.0);
                        span.1 = span.1.max(cell.x + cell.margins[1] + nested.1);
                    }
                }
            }
            if let Some(nested) = &slice.nested_row {
                let nested = row_horizontal_span(nested);
                span.0 = span.0.min(cell.x + cell.margins[1] + nested.0);
                span.1 = span.1.max(cell.x + cell.margins[1] + nested.1);
            }
        }
    }
    span
}

impl NoteAreaPlan {
    fn occupied_span(&self) -> (f64, f64) {
        let mut span = (self.context.x, self.context.x + self.context.width);
        for placement in self.pieces.iter().flat_map(|p| &p.placements) {
            if let NotePlacement::Row { row, .. } = placement {
                let row = row_horizontal_span(row);
                span.0 = span.0.min(self.context.x + row.0);
                span.1 = span.1.max(self.context.x + row.1);
            }
        }
        span
    }

    fn defer(&mut self, queue: &[PendingNote]) {
        self.outgoing = queue.to_vec();
        self.pieces.clear();
        self.height = 0.0;
        self.explicit_break = None;
        self.atomic_overflow = false;
        self.notice_height = 0.0;
        self.separator_layout = None;
        self.notice_layout = None;
        self.overflow_story = None;
        self.special_overflow = false;
    }
}

#[derive(Clone, Default)]
pub(super) struct PageNotes {
    pub incoming: Vec<PendingNote>,
    pub outgoing: Vec<PendingNote>,
    areas: Vec<NoteAreaPlan>,
    // Accepted body extents retain their physical horizontal interval across columns.
    body_regions: Vec<(f64, f64, f64)>,
    pub active: bool,
}

impl PageNotes {
    pub(super) fn new(incoming: Vec<PendingNote>, _body_bottom: f64) -> Self {
        Self {
            incoming,
            ..Default::default()
        }
    }

    pub(super) fn height_at(&self, x: f64, width: f64) -> f64 {
        self.areas
            .iter()
            .filter(|a| {
                let (left, right) = a.occupied_span();
                spans_overlap(x, width, left, right - left)
            })
            .map(|a| a.height)
            .fold(0.0, f64::max)
    }

    fn body_bottom_at(&self, x: f64, width: f64, top: f64) -> f64 {
        self.body_regions
            .iter()
            .filter(|(left, w, _)| spans_overlap(x, width, *left, *w))
            .map(|(_, _, bottom)| *bottom)
            .fold(top, f64::max)
    }
}

fn spans_overlap(x: f64, width: f64, other: f64, other_width: f64) -> bool {
    x < other + other_width - EPS && x + width > other + EPS
}

impl std::fmt::Debug for PageNotes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PageNotes")
            .field("incoming", &self.incoming.len())
            .field("outgoing", &self.outgoing.len())
            .field("areas", &self.areas.len())
            .finish_non_exhaustive()
    }
}

impl Paginator<'_> {
    pub(super) fn current_note_page(&self) -> &PageNotes {
        self.st
            .note_pages
            .last()
            .expect("note page follows physical page")
    }

    pub(super) fn seed_note_page(&mut self) {
        let planned = self
            .pages
            .last()
            .and_then(|p| self.plan_page_notes(p, &[], p.top));
        if let Some(Ok(plan)) = planned {
            *self.st.note_pages.last_mut().unwrap() = std::rc::Rc::new(plan);
        }
    }

    pub(super) fn drain_note_pages(&mut self, sections: &[Section], out: &mut Vec<OutPage>) {
        let Some(last) = sections.last() else {
            return;
        };
        while self
            .st
            .note_pages
            .last()
            .is_some_and(|p| !p.outgoing.is_empty())
        {
            self.new_page(sections.len() - 1, &last.props, sections);
            if !self.current_note_page().active {
                let fallback =
                    self.remaining_note_fallback(self.current_note_page().incoming.clone());
                *self.st.note_pages.last_mut().unwrap() = std::rc::Rc::new(fallback);
                self.pages.last_mut().unwrap().note_diagnostics.push("Oversized or unsupported note continuation retains remaining source with unconstrained geometry".into());
            }
            let info = self.pages.last_mut().unwrap();
            out.push(OutPage {
                info: info.clone(),
                items: Vec::new(),
            });
        }
    }

    fn remaining_note_fallback(&self, queue: Vec<PendingNote>) -> PageNotes {
        let mut result = PageNotes::new(queue.clone(), 0.0);
        let Some(first) = queue.first() else {
            return result;
        };
        let mut area = NoteAreaPlan {
            context: first.reference.context.clone(),
            column: 0,
            height: self.note_in(&first.reference.context, i64::MIN).1,
            body_bottom: 0.0,
            pieces: Vec::new(),
            outgoing: Vec::new(),
            explicit_break: None,
            atomic_overflow: false,
            notice_height: 0.0,
            separator_layout: Some(self.note_layout_in(&first.reference.context, i64::MIN)),
            notice_layout: None,
            overflow_story: None,
            special_overflow: false,
        };
        for source in queue {
            let mut cursor = source.cursor.clone();
            while !cursor.complete() {
                let fragment = self
                    .plan_note_fragment(
                        &source,
                        &cursor,
                        FragmentBudget {
                            height: 1.0e12,
                            fresh_capacity: 1.0e12,
                            fresh: true,
                            allow_atomic_overflow: true,
                        },
                    )
                    .expect("retained note cursor belongs to its immutable layout");
                let progressed = !fragment.placements.is_empty() || fragment.after.complete();
                area.pieces.push(NotePiece {
                    source: source.clone(),
                    placements: fragment.placements,
                    y: area.height,
                });
                area.height += fragment.used_height;
                cursor = fragment.after;
                if !progressed {
                    let Unit::Row(original) = &source.layout.units()[cursor.unit] else {
                        panic!("unconstrained paragraph must progress")
                    };
                    let (row, fragment) = match &cursor.part {
                        UnitCursor::Row {
                            remainder,
                            fragment,
                        } => (remainder.clone(), *fragment),
                        _ => (std::rc::Rc::new(original.clone()), 0),
                    };
                    let height = row.flow_height();
                    area.pieces.push(NotePiece {
                        source: source.clone(),
                        y: area.height,
                        placements: vec![NotePlacement::Row {
                            unit: cursor.unit,
                            row,
                            y: 0.0,
                            fragment,
                            history: usize::MAX,
                            merge_paint: Vec::new(),
                        }],
                    });
                    area.height += height;
                    cursor.advance();
                }
            }
        }
        result.active = true;
        result.areas.push(area);
        result
    }

    /// Each trial starts with immutable incoming cursors and accepted reference ownership.
    pub(super) fn plan_page_notes(
        &self,
        page: &PageInfo,
        pending: &[i64],
        bottom: f64,
    ) -> Option<Result<PageNotes, ()>> {
        self.plan_page_notes_with_pictures(page, pending, bottom, &[])
    }

    pub(super) fn plan_page_notes_with_pictures(
        &self,
        page: &PageInfo,
        pending: &[i64],
        bottom: f64,
        pictures: &[(f64, f64, f64, f64)],
    ) -> Option<Result<PageNotes, ()>> {
        self.plan_page_note_stories(page, pending, bottom, pictures, &HashSet::new())
    }

    fn plan_page_note_stories(
        &self,
        page: &PageInfo,
        pending: &[i64],
        bottom: f64,
        pictures: &[(f64, f64, f64, f64)],
        notices: &HashSet<usize>,
    ) -> Option<Result<PageNotes, ()>> {
        let previous = self.current_note_page();
        let references = self.pending_areas(page, pending);
        if references.is_empty() && previous.incoming.is_empty() {
            return None;
        }
        let current = self.current_note_context().0;
        let columns = self.note_section.1.columns();
        let paragraph_sources = references.iter().flat_map(|a| &a.references).all(|r| {
            self.note_layout_in(&r.context, r.id)
                .units()
                .iter()
                .all(|u| match u {
                    Unit::Para(_) => true,
                    Unit::Row(r) => r.source.as_ref().is_some_and(|s| {
                        crate::office::docx::layout::note_source::supported_table(&s.table)
                    }),
                })
        }) && previous.incoming.iter().all(|n| {
            n.layout.units().iter().all(|u| match u {
                Unit::Para(_) => true,
                Unit::Row(r) => r.source.as_ref().is_some_and(|s| {
                    crate::office::docx::layout::note_source::supported_table(&s.table)
                }),
            })
        });
        let equal_widths = columns.iter().all(|c| (c.1 - current.width).abs() < EPS)
            && references
                .iter()
                .all(|a| (a.context.width - current.width).abs() < EPS)
            && previous
                .incoming
                .iter()
                .all(|n| (n.reference.context.width - current.width).abs() < EPS);
        let compatible = columns == page.cols
            && (equal_widths || paragraph_sources)
            && !self.legacy_note_sections.contains(&self.note_section.0);
        // A changed-width carry keeps its captured layout until logical-source reflow is available.
        let destinations = if compatible {
            columns
                .iter()
                .enumerate()
                .map(|(column, &(x, width))| {
                    (
                        column,
                        NoteContext {
                            x,
                            width,
                            ..current.clone()
                        },
                    )
                })
                .collect::<Vec<_>>()
        } else if let Some(first) = previous.incoming.first() {
            vec![(0, first.reference.context.clone())]
        } else if page.cols.len() == 1 && columns.len() == 1 && references.len() <= 1 {
            vec![(0, current)]
        } else {
            return previous.active.then_some(Err(()));
        };
        if references.iter().flat_map(|a| &a.references).any(|r| {
            self.note_layout_in(&r.context, r.id)
                .units()
                .iter()
                .any(|u| matches!(u, Unit::Row(row) if !super::note_fragments::supported_row(row)))
        }) {
            return (previous.active || !previous.incoming.is_empty()).then_some(Err(()));
        }
        let (body_x, body_width) = page.cols[self.st.col];
        if !previous.active
            && previous.incoming.is_empty()
            && references.iter().all(|area| {
                let context = &area.context;
                let picture_bottom = self
                    .st
                    .picture_rects
                    .iter()
                    .chain(pictures)
                    .filter(|(_, _, left, right)| {
                        spans_overlap(context.x, context.width, *left, right - left)
                    })
                    .map(|(_, y, _, _)| *y)
                    .fold(page.top, f64::max);
                let body_bottom = previous
                    .body_bottom_at(context.x, context.width, page.top)
                    .max(
                        if spans_overlap(context.x, context.width, body_x, body_width) {
                            bottom
                        } else {
                            page.top
                        },
                    )
                    .max(picture_bottom);
                self.area_height(area, page) <= page.bottom - body_bottom + EPS
                    && area.references.iter().all(|r| {
                        !self
                            .note_layout_in(&r.context, r.id)
                            .units()
                            .iter()
                            .any(|u| match u {
                                Unit::Para(p) => {
                                    p.page_break_before
                                        || p.lines.iter().any(|l| l.break_after.is_some())
                                }
                                Unit::Row(r) => {
                                    let (left, right) = row_horizontal_span(r);
                                    r.page_break_before
                                        || left < -EPS
                                        || right > context.width + EPS
                                }
                            })
                    })
            })
        {
            return None;
        }

        let mut result = PageNotes {
            incoming: previous.incoming.clone(),
            body_regions: previous.body_regions.clone(),
            active: true,
            ..Default::default()
        };
        let mut reference_ids = page.notes.clone();
        for id in pending {
            if !reference_ids.contains(id) {
                reference_ids.push(*id);
            }
        }
        let ordered_references: Vec<_> = reference_ids
            .iter()
            .filter_map(|id| {
                references
                    .iter()
                    .flat_map(|a| &a.references)
                    .find(|r| r.id == *id)
            })
            .collect();
        let mut queue = previous.incoming.clone();
        let mut introduced: HashSet<_> = queue.iter().map(|n| n.reference.id).collect();
        let mut needs_fragments = previous.active || !queue.is_empty();
        let mut page_break = false;
        for (column, context) in destinations {
            let area_references: Vec<_> = references
                .iter()
                .filter(|a| spans_overlap(context.x, context.width, a.context.x, a.context.width))
                .flat_map(|a| &a.references)
                .collect();
            let old = previous.areas.iter().find(|a| {
                a.column == column
                    && (a.context.x - context.x).abs() < EPS
                    && (a.context.width - context.width).abs() < EPS
            });
            let (occupied_left, occupied_right) = old.map_or(
                (context.x, context.x + context.width),
                NoteAreaPlan::occupied_span,
            );
            let picture_bottom = self
                .st
                .picture_rects
                .iter()
                .chain(pictures)
                .filter(|(_, _, left, right)| {
                    spans_overlap(
                        occupied_left,
                        occupied_right - occupied_left,
                        *left,
                        right - left,
                    )
                })
                .map(|(_, y, _, _)| *y)
                .fold(page.top, f64::max);
            let proposed_bottom = if spans_overlap(
                occupied_left,
                occupied_right - occupied_left,
                body_x,
                body_width,
            ) {
                bottom
            } else {
                page.top
            };
            let body_bottom = previous
                .body_bottom_at(occupied_left, occupied_right - occupied_left, page.top)
                .max(old.map_or(page.top, |a| a.body_bottom))
                .max(proposed_bottom)
                .max(picture_bottom);
            if previous.active {
                if let Some(old) = old
                    && old.height > EPS
                    && body_bottom > page.bottom - old.height + EPS
                {
                    return Some(Err(()));
                }
            } else {
                let reserved = page
                    .note_areas
                    .iter()
                    .filter(|a| {
                        spans_overlap(context.x, context.width, a.context.x, a.context.width)
                    })
                    .map(|a| self.area_height(a, page))
                    .fold(0.0, f64::max);
                if reserved > EPS && body_bottom > page.bottom - reserved + EPS {
                    return Some(Err(()));
                }
            }
            // Earlier body columns are closed; later trials retain their accepted fragments.
            if column < self.st.col
                && notices.is_empty()
                && let Some(old) = old
            {
                for r in &area_references {
                    introduced.insert(r.id);
                }
                queue = old.outgoing.clone();
                page_break |= old.explicit_break == Some(BreakKind::Page);
                result.areas.push(old.clone());
                continue;
            }
            for reference in &ordered_references {
                if introduced.contains(&reference.id) {
                    continue;
                }
                if reference.context.x > context.x + EPS {
                    break;
                }
                introduced.insert(reference.id);
                let layout = self.note_layout_in(&reference.context, reference.id);
                queue.push(PendingNote {
                    reference: (*reference).clone(),
                    cursor: layout.start(),
                    first_page: None,
                    special: false,
                    suppress_separator: false,
                    layout,
                });
            }
            for source in &mut queue {
                let measured = self.with_note_env(&source.reference.context, |env| {
                    source.layout.remeasure(&source.cursor, context.width, env)
                });
                let Some((layout, cursor)) = measured else {
                    return Some(Err(()));
                };
                source.layout = layout;
                source.cursor = cursor;
            }
            if queue.is_empty() || page_break {
                result.areas.push(NoteAreaPlan {
                    context,
                    column,
                    outgoing: queue.clone(),
                    pieces: Vec::new(),
                    body_bottom,
                    height: 0.0,
                    explicit_break: None,
                    atomic_overflow: false,
                    notice_height: 0.0,
                    separator_layout: None,
                    notice_layout: None,
                    overflow_story: None,
                    special_overflow: false,
                });
                continue;
            }
            let separator = self.note_in(&context, i64::MIN).1;
            let full_height = queue.iter().map(|n| n.layout.height()).sum::<f64>() + separator;
            needs_fragments |= full_height > page.bottom - body_bottom + EPS
                || queue.iter().any(|n| {
                    n.layout.units().iter().any(|u| match u {
                        Unit::Para(p) => {
                            p.page_break_before || p.lines.iter().any(|l| l.break_after.is_some())
                        }
                        Unit::Row(r) => {
                            let (left, right) = row_horizontal_span(r);
                            r.page_break_before || left < -EPS || right > context.width + EPS
                        }
                    })
                });
            let Ok(mut area) = self.plan_note_area(
                page,
                column,
                context,
                body_bottom,
                &queue,
                notices.contains(&column),
            ) else {
                return Some(Err(()));
            };
            // Fixed table grids can occupy neighboring columns without changing
            // their logical destination width. Replan against all intersected body.
            loop {
                let (left, right) = area.occupied_span();
                let occupied_bottom = previous
                    .body_bottom_at(left, right - left, page.top)
                    .max(if spans_overlap(left, right - left, body_x, body_width) {
                        bottom
                    } else {
                        page.top
                    })
                    .max(
                        self.st
                            .picture_rects
                            .iter()
                            .chain(pictures)
                            .filter(|(_, _, x0, x1)| {
                                spans_overlap(left, right - left, *x0, x1 - x0)
                            })
                            .map(|(_, y, _, _)| *y)
                            .fold(page.top, f64::max),
                    );
                if area.pieces.is_empty() || occupied_bottom <= area.body_bottom + EPS {
                    break;
                }
                area = match self.plan_note_area(
                    page,
                    column,
                    area.context.clone(),
                    occupied_bottom,
                    &queue,
                    notices.contains(&column),
                ) {
                    Ok(area) => area,
                    Err(()) => return Some(Err(())),
                };
            }
            let (left, right) = area.occupied_span();
            let first_x = page.cols.first().map_or(area.context.x, |c| c.0);
            let shift = first_x - area.context.x;
            let fits_first = left + shift >= -EPS && right + shift <= page.w + EPS;
            let outside_page = left < -EPS || right > page.w + EPS;
            let intersects_note = result.areas.iter().any(|prior| {
                let (x0, x1) = prior.occupied_span();
                prior.height > EPS && spans_overlap(left, right - left, x0, x1 - x0)
            });
            if !area.pieces.is_empty() && (intersects_note || (fits_first && outside_page)) {
                area.defer(&queue);
                needs_fragments = true;
            }
            queue = area.outgoing.clone();
            page_break |= area.explicit_break == Some(BreakKind::Page);
            result.areas.push(area);
        }
        result.outgoing = queue;
        if self.hf.continuation_notice.is_some() {
            let mut required = notices.clone();
            for outgoing in &result.outgoing {
                if outgoing.special || outgoing.first_page.is_none() {
                    continue;
                }
                if let Some(area) = result.areas.iter().rev().find(|area| {
                    area.pieces
                        .iter()
                        .any(|piece| piece.source.cursor.same_occurrence(&outgoing.cursor))
                }) {
                    required.insert(area.column);
                }
            }
            if required != *notices {
                return self.plan_page_note_stories(page, pending, bottom, pictures, &required);
            }
        }
        let overflow_stories = result
            .areas
            .iter()
            .filter_map(|area| area.overflow_story.clone())
            .collect::<Vec<_>>();
        result.outgoing.splice(0..0, overflow_stories);
        if !needs_fragments {
            None
        } else {
            Some(Ok(result))
        }
    }

    fn plan_note_fragment(
        &self,
        source: &PendingNote,
        cursor: &NoteCursor,
        budget: FragmentBudget,
    ) -> Result<super::note_fragments::FragmentPlan, super::note_fragments::PlanError> {
        self.with_note_env(&source.reference.context, |env| {
            super::note_fragments::plan_fragment_in_env(&source.layout, cursor, budget, Some(env))
        })
    }

    fn plan_note_area(
        &self,
        page: &PageInfo,
        column: usize,
        context: NoteContext,
        body_bottom: f64,
        queue: &[PendingNote],
        notice: bool,
    ) -> Result<NoteAreaPlan, ()> {
        if queue.first().is_some_and(|source| source.special) {
            return self.plan_special_overflow(page, column, context, body_bottom, queue);
        }
        let incoming = queue.iter().any(|source| {
            !source.special
                && source
                    .first_page
                    .is_some_and(|first| first < self.pages.len() - 1)
        });
        let separator_id = if incoming && self.hf.continuation_separator.is_some() {
            i64::MIN + 1
        } else {
            i64::MIN
        };
        let capacity = (page.bottom - page.top).max(EPS);
        let (separator_layout, separator_relaxed) =
            self.special_note_layout(&context, separator_id, capacity);
        let (notice_layout, notice_relaxed) =
            self.special_note_layout(&context, i64::MIN + 2, capacity);
        let separator_layout = (!queue
            .first()
            .is_some_and(|source| source.suppress_separator))
        .then_some(separator_layout);
        let mut notice_layout = notice.then_some(notice_layout);
        let separator = separator_layout.as_ref().map_or(0.0, NoteLayout::height);
        let mut notice_height = notice_layout.as_ref().map_or(0.0, NoteLayout::height);
        let mut overflow_story = None;
        let starts = |height: f64| {
            queue.first().is_none_or(|source| {
                self.plan_note_fragment(
                    source,
                    &source.cursor,
                    FragmentBudget {
                        height: height.max(0.0),
                        fresh_capacity: height.max(EPS),
                        fresh: true,
                        allow_atomic_overflow: false,
                    },
                )
                .is_ok_and(|fragment| !fragment.placements.is_empty() || fragment.after.complete())
            })
        };
        let starts_without_stories = starts(capacity);
        let separator_blocks_start = separator > EPS
            && separator_id == i64::MIN + 1
            && starts_without_stories
            && !starts(capacity - separator);
        if separator >= capacity - EPS || separator_blocks_start {
            // A suppressed separator reaches here only when the page body is at most EPS tall.
            let layout = separator_layout.ok_or(())?;
            let story = PendingNote {
                reference: NoteReference {
                    id: separator_id,
                    context: context.clone(),
                },
                cursor: layout.start(),
                layout,
                first_page: None,
                special: true,
                suppress_separator: false,
            };
            let mut pending = vec![story];
            pending.extend(queue.iter().cloned().map(|mut source| {
                source.suppress_separator = true;
                source
            }));
            return self.plan_special_overflow(page, column, context, body_bottom, &pending);
        }
        if (separator + notice_height >= capacity - EPS
            || (notice_height > EPS
                && starts(capacity - separator)
                && !starts(capacity - separator - notice_height)))
            && let Some(layout) = notice_layout.take()
        {
            overflow_story = Some(PendingNote {
                reference: NoteReference {
                    id: i64::MIN + 2,
                    context: context.clone(),
                },
                cursor: layout.start(),
                layout,
                first_page: None,
                special: true,
                suppress_separator: false,
            });
            notice_height = 0.0;
        }
        let available = (page.bottom - body_bottom - separator - notice_height).max(0.0);
        let full = (capacity - separator - notice_height).max(EPS);
        let mut plan = NoteAreaPlan {
            context,
            column,
            body_bottom,
            height: separator,
            outgoing: Vec::new(),
            pieces: Vec::new(),
            explicit_break: None,
            atomic_overflow: false,
            notice_height,
            separator_layout,
            notice_layout,
            special_overflow: separator_relaxed
                || (notice && notice_relaxed)
                || overflow_story.is_some(),
            overflow_story,
        };
        let mut room = available;
        for (index, source) in queue.iter().enumerate() {
            if source.special {
                plan.outgoing.extend(queue[index..].iter().cloned());
                break;
            }
            let Ok(fragment) = self.plan_note_fragment(
                source,
                &source.cursor,
                FragmentBudget {
                    height: room,
                    fresh_capacity: full,
                    fresh: plan.pieces.is_empty(),
                    allow_atomic_overflow: body_bottom <= page.top + EPS,
                },
            ) else {
                return Err(());
            };
            if fragment.placements.is_empty() && !fragment.after.complete() {
                if index == 0 {
                    plan.height = 0.0;
                }
                plan.outgoing.extend(queue[index..].iter().cloned());
                break;
            }
            if fragment.outcome == FragmentOutcome::NeedsStructuredRow {
                return Err(());
            }
            let y = plan.height;
            // Fresh atomic overflow keeps its source geometry at the area's top.
            // Reservation uses physical capacity so the owner remains visible.
            let reserved = if fragment.outcome == FragmentOutcome::AtomicOverflow {
                plan.atomic_overflow = true;
                fragment.used_height.min(room)
            } else {
                fragment.used_height
            };
            plan.height += reserved;
            room = (room - reserved).max(0.0);
            let mut accepted_source = source.clone();
            accepted_source
                .first_page
                .get_or_insert(self.pages.len() - 1);
            plan.pieces.push(NotePiece {
                source: accepted_source.clone(),
                placements: fragment.placements,
                y,
            });
            plan.explicit_break = match fragment.outcome {
                FragmentOutcome::Break(kind) => Some(kind),
                _ => None,
            };
            if !fragment.after.complete() {
                let mut remaining = accepted_source;
                remaining.cursor = fragment.after;
                remaining.suppress_separator = false;
                plan.outgoing.push(remaining);
                plan.outgoing.extend(queue[index + 1..].iter().cloned());
                break;
            }
            if matches!(fragment.outcome, FragmentOutcome::Break(_)) {
                plan.outgoing.extend(queue[index + 1..].iter().cloned());
                break;
            }
        }
        if !plan.pieces.is_empty() {
            plan.height += notice_height;
        } else {
            plan.overflow_story = None;
        }
        Ok(plan)
    }

    fn plan_special_overflow(
        &self,
        page: &PageInfo,
        column: usize,
        context: NoteContext,
        body_bottom: f64,
        queue: &[PendingNote],
    ) -> Result<NoteAreaPlan, ()> {
        let source = &queue[0];
        let capacity = (page.bottom - page.top).max(EPS);
        let room = (page.bottom - body_bottom).max(0.0);
        let mut fragment = self
            .plan_note_fragment(
                source,
                &source.cursor,
                FragmentBudget {
                    height: room,
                    fresh_capacity: capacity,
                    fresh: true,
                    allow_atomic_overflow: body_bottom <= page.top + EPS,
                },
            )
            .map_err(|_| ())?;
        if fragment.outcome == FragmentOutcome::NeedsStructuredRow
            && fragment.placements.is_empty()
            && body_bottom <= page.top + EPS
        {
            // Unsupported source rows consume one atomic occurrence on a fresh area.
            // Physical reservation is capped below; retained row geometry stays intact.
            let cursor = &fragment.after;
            let Unit::Row(original) = &source.layout.units()[cursor.unit] else {
                return Err(());
            };
            let (row, part) = match &cursor.part {
                UnitCursor::Row {
                    remainder,
                    fragment,
                } => (remainder.clone(), *fragment),
                _ => (std::rc::Rc::new(original.clone()), 0),
            };
            fragment.used_height = row.flow_height();
            fragment.placements.push(NotePlacement::Row {
                unit: cursor.unit,
                row,
                y: 0.0,
                fragment: part,
                history: usize::MAX,
                merge_paint: Vec::new(),
            });
            fragment.after.advance();
            fragment.outcome = FragmentOutcome::AtomicOverflow;
        }
        let mut area = NoteAreaPlan {
            context,
            column,
            outgoing: Vec::new(),
            pieces: Vec::new(),
            body_bottom,
            height: 0.0,
            explicit_break: None,
            atomic_overflow: false,
            notice_height: 0.0,
            separator_layout: None,
            notice_layout: None,
            overflow_story: None,
            special_overflow: true,
        };
        if fragment.placements.is_empty() && !fragment.after.complete() {
            area.outgoing = queue.to_vec();
            return Ok(area);
        }
        area.atomic_overflow = fragment.outcome == FragmentOutcome::AtomicOverflow;
        area.height = if area.atomic_overflow {
            fragment.used_height.min(room)
        } else {
            fragment.used_height
        };
        area.pieces.push(NotePiece {
            source: source.clone(),
            placements: fragment.placements,
            y: 0.0,
        });
        if !fragment.after.complete() {
            let mut rest = source.clone();
            rest.cursor = fragment.after;
            area.outgoing.push(rest);
        }
        area.outgoing.extend(queue[1..].iter().cloned());
        area.explicit_break = match fragment.outcome {
            FragmentOutcome::Break(kind) => Some(kind),
            _ => None,
        };
        Ok(area)
    }

    pub(super) fn new_note_start_conflict(
        &self,
        page: &PageInfo,
        pending: &[i64],
        bottom: f64,
        fresh_line: bool,
        pictures: &[(f64, f64, f64, f64)],
    ) -> bool {
        let Some(Ok(plan)) = self.plan_page_notes_with_pictures(page, pending, bottom, pictures)
        else {
            return false;
        };
        let origin = self.current_note_context().0;
        let deferred_new = pending.iter().any(|id| {
            !page.notes.contains(id)
                && !plan.incoming.iter().any(|old| old.reference.id == *id)
                && !self.note_layout_in(&origin, *id).start().complete()
                && !plan
                    .areas
                    .iter()
                    .filter(|a| (a.context.x - origin.x).abs() < EPS)
                    .flat_map(|a| &a.pieces)
                    .any(|piece| piece.source.reference.id == *id && !piece.placements.is_empty())
        });
        if !deferred_new || plan.areas.iter().any(|a| a.explicit_break.is_some()) {
            return false;
        }
        // An indivisible first body line can reference more notes than a fresh
        // area holds. Keep their source order in the outgoing queue for progress.
        let same_line_overflow = fresh_line
            && page.notes.is_empty()
            && plan.incoming.is_empty()
            && plan.areas.first().is_some_and(|area| {
                let context = &area.context;
                let total = pending
                    .iter()
                    .enumerate()
                    .filter(|(index, id)| !pending[..*index].contains(id))
                    .map(|(_, id)| self.note_layout_in(context, *id).height())
                    .sum::<f64>();
                total + self.note_in(context, i64::MIN).1 > page.bottom - bottom + EPS
            });
        !same_line_overflow
    }

    pub(super) fn fragment_retry(&self, page: &PageInfo, pending: &[i64], bottom: f64) -> bool {
        (self.current_note_page().active || !self.current_note_page().incoming.is_empty())
            && matches!(self.plan_page_notes(page, pending, bottom), Some(Err(())))
    }

    pub(super) fn note_limit_with_pictures(
        &self,
        page: &PageInfo,
        pending: &[i64],
        bottom: f64,
        pictures: &[(f64, f64, f64, f64)],
    ) -> f64 {
        match self.plan_page_notes_with_pictures(page, pending, bottom, pictures) {
            Some(Ok(plan)) => {
                let (x, width) = page.cols[self.st.col];
                page.bottom - plan.height_at(x, width)
            }
            Some(Err(())) => page.top - 1.0,
            None => self.note_limit(page, pending),
        }
    }

    pub(super) fn commit_note_body(&mut self, pending: &[i64], bottom: f64) {
        let planned = self
            .pages
            .last()
            .and_then(|p| self.plan_page_notes(p, pending, bottom));
        self.commit_note_plan(pending, bottom, planned);
    }

    pub(super) fn commit_note_plan(
        &mut self,
        pending: &[i64],
        bottom: f64,
        planned: Option<Result<PageNotes, ()>>,
    ) {
        let extra = if matches!(planned, Some(Err(()))) && self.current_note_page().active {
            let context = self.current_note_context().0;
            pending
                .iter()
                .filter(|id| !self.pages.last().unwrap().notes.contains(id))
                .map(|id| {
                    let layout = self.note_layout_in(&context, *id);
                    PendingNote {
                        reference: NoteReference {
                            id: *id,
                            context: context.clone(),
                        },
                        cursor: layout.start(),
                        first_page: None,
                        special: false,
                        suppress_separator: false,
                        layout,
                    }
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        self.accept_notes(pending);
        if let Some(page) = self.pages.last_mut() {
            match planned {
                Some(Ok(plan)) => *self.st.note_pages.last_mut().unwrap() = std::rc::Rc::new(plan),
                Some(Err(())) => page.note_diagnostics.push("Footnote fragment could not fit the accepted body; an oversized source unit remains unresolved".into()),
                None => {},
            }
            let ledger = std::rc::Rc::make_mut(self.st.note_pages.last_mut().unwrap());
            let (x, width) = page.cols[self.st.col];
            if let Some(region) = ledger
                .body_regions
                .iter_mut()
                .find(|r| (r.0 - x).abs() < EPS && (r.1 - width).abs() < EPS)
            {
                region.2 = region.2.max(bottom);
            } else {
                ledger.body_regions.push((x, width, bottom));
            }
            ledger.outgoing.extend(extra);
        }
    }

    fn note_merge_paint(&self, source: &PendingNote, owner: usize) -> Vec<merges::Paint> {
        self.st
            .note_pages
            .iter()
            .flat_map(|page| &page.areas)
            .flat_map(|area| &area.pieces)
            .filter(|piece| piece.source.cursor.same_occurrence(&source.cursor))
            .flat_map(|piece| &piece.placements)
            .filter_map(|placement| match placement {
                NotePlacement::Row { merge_paint, .. } => Some(merge_paint),
                _ => None,
            })
            .flatten()
            .filter(|paint| paint.owner == owner)
            .cloned()
            .collect()
    }

    fn note_occurrence_stride(&self, cursor: &NoteCursor) -> usize {
        // Earlier placements can receive payload after a later merged row closes.
        // Reserve one slot domain from every accepted width of this occurrence.
        self.st
            .note_pages
            .iter()
            .flat_map(|page| &page.areas)
            .flat_map(|area| &area.pieces)
            .filter(|piece| piece.source.cursor.same_occurrence(cursor))
            .map(|piece| {
                let paint_end = piece
                    .placements
                    .iter()
                    .filter_map(|placement| {
                        let NotePlacement::Row { merge_paint, .. } = placement else {
                            return None;
                        };
                        Some(
                            merge_paint
                                .iter()
                                .map(|paint| {
                                    paint
                                        .items
                                        .iter()
                                        .filter_map(|item| {
                                            crate::office::docx::text::reading_key(item)
                                                .map(|key| key + 1)
                                        })
                                        .chain(paint.anchor_keys.iter().map(|key| key + 1))
                                        .max()
                                        .unwrap_or(0)
                                })
                                .max()
                                .unwrap_or(0),
                        )
                    })
                    .max()
                    .unwrap_or(0);
                piece.source.layout.reading_stride().max(paint_end)
            })
            .max()
            .unwrap_or(1)
    }

    pub(super) fn draw_note_fragments(&self, page: &mut OutPage, index: usize) -> bool {
        if page.info.blank {
            return true;
        }
        if !self.st.note_pages[index].active {
            return false;
        }
        let plan = self.st.note_pages[index].as_ref().clone();
        let mut occurrences: Vec<NoteCursor> = Vec::new();
        let mut occurrence_bases = Vec::new();
        // Ordinary fragments keep physical destination order; local ranks insert
        // anchored text beside its source without interleaving later fragments.
        let mut ordinary_offsets = Vec::new();
        let mut next_base = 1usize;
        let single_area = plan.areas.iter().filter(|a| !a.pieces.is_empty()).count() == 1;
        for area in plan.areas {
            if area.pieces.is_empty() {
                continue;
            }
            let context = &area.context;
            if area.special_overflow {
                page.info.note_diagnostics.push("Oversized continuation story uses reduced paragraph spacing or separate source-preserving overflow fragments".into());
            }
            if area.atomic_overflow {
                page.info.note_diagnostics.push(
                    "An oversized footnote object exceeds a fresh note area; source geometry is retained at the area top".into(),
                );
            }
            let owner = &area.pieces[0].source.reference.context;
            let single_owner = single_area
                && area
                    .pieces
                    .iter()
                    .all(|p| p.source.reference.context.section == owner.section);
            let top = super::note_areas::accepted_note_top(
                &mut page.info,
                owner,
                area.height,
                area.occupied_span(),
                single_owner,
            );
            let mut special_key = crate::office::docx::text::NOTE_KEY + next_base;
            let items = super::note_areas::paint_note_slices(
                area.separator_layout
                    .as_ref()
                    .map_or_else(Vec::new, |layout| layout.legacy_slices().0),
                &page.info,
                context.x,
                top,
            );
            super::note_areas::append_note_items(page, items, &mut special_key);
            next_base = special_key - crate::office::docx::text::NOTE_KEY;
            let notice_y = top + area.height - area.notice_height;
            let notice_context = context.clone();
            let has_notice = area.notice_height > 0.0;
            for piece in area.pieces {
                // Cached child layouts can appear in separate parent occurrences.
                // Restrict backfill to this accepted parent source history.
                let mut child_plans = Vec::new();
                for plan in &self.st.note_pages {
                    for area in &plan.areas {
                        for accepted_piece in &area.pieces {
                            if !accepted_piece
                                .source
                                .cursor
                                .same_occurrence(&piece.source.cursor)
                            {
                                continue;
                            }
                            for placement in &accepted_piece.placements {
                                if let NotePlacement::Row {
                                    row, merge_paint, ..
                                } = placement
                                {
                                    crate::office::docx::layout::note_children::collect(
                                        row,
                                        &mut child_plans,
                                    );
                                    for paint in merge_paint {
                                        crate::office::docx::layout::note_children::collect_paint(
                                            paint,
                                            &mut child_plans,
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
                let reading_stride = self.note_occurrence_stride(&piece.source.cursor);
                let ordinal = occurrences
                    .iter()
                    .position(|c| c.same_occurrence(&piece.source.cursor))
                    .unwrap_or_else(|| {
                        occurrence_bases.push(next_base);
                        ordinary_offsets.push(0usize);
                        next_base +=
                            (piece.source.layout.units().len() + 1) * 1024 * reading_stride;
                        occurrences.push(piece.source.cursor.clone());
                        occurrences.len() - 1
                    });
                let ordered_merge = piece.source.layout.units().iter().any(
                    |u| matches!(u, Unit::Row(r) if r.cells.iter().any(|c| c.moved.is_some())),
                );
                let mut source_keys = Vec::new();
                let mut units = piece.source.layout.units().to_vec();
                let mut placed = Vec::new();
                for placement in piece.placements {
                    match placement {
                        NotePlacement::Paragraph {
                            unit,
                            first,
                            end,
                            y,
                            line_tops,
                        } => {
                            let Unit::Para(p) = &units[unit] else {
                                continue;
                            };
                            for (line, line_y) in (first..end).zip(line_tops) {
                                source_keys.push(unit);
                                placed.push(Placed::Line {
                                    unit,
                                    line,
                                    page: 0,
                                    column: area.column,
                                    x: context.x,
                                    y: top + piece.y + line_y,
                                    pad_top: if line == 0 { p.pad_top } else { 0.0 },
                                    pad_bottom: if line + 1 == p.lines.len() {
                                        p.pad_bottom
                                    } else {
                                        0.0
                                    },
                                    para_y: top + piece.y + y,
                                    alt: None,
                                });
                            }
                        }
                        NotePlacement::Row {
                            unit,
                            row,
                            y,
                            fragment,
                            history,
                            ..
                        } => {
                            debug_assert!(matches!(&units[unit], Unit::Row(_)));
                            debug_assert!(fragment == 0 || row.outer_before.abs() < EPS);
                            let local = placed.len();
                            let mut merge_paint = self.note_merge_paint(&piece.source, history);
                            for paint in &mut merge_paint {
                                crate::office::docx::layout::note_children::materialize_paint(
                                    paint,
                                    &child_plans,
                                );
                                paint.owner = local;
                                paint.first = local;
                                let restart = units.iter().position(|u| matches!(u, Unit::Row(r) if (r.table, r.index) == paint.restart)).unwrap_or(unit);
                                paint.reading_key = Some(
                                    crate::office::docx::text::NOTE_KEY
                                        + (restart * 1024 + paint.cell.min(1023)) * reading_stride,
                                );
                                for item in &mut paint.items {
                                    translate_item(item, context.x, top + piece.y);
                                }
                                for (_, x, y) in &mut paint.anchors {
                                    *x += context.x;
                                    *y += top + piece.y;
                                }
                            }
                            source_keys.push(unit);
                            placed.push(Placed::Row {
                                row: Box::new(
                                    crate::office::docx::layout::note_children::materialize_row(
                                        &row,
                                        &child_plans,
                                    ),
                                ),
                                merge_paint,
                                page: 0,
                                column: area.column,
                                normal_flow: true,
                                x: context.x,
                                y: top + piece.y + y + row.outer_before,
                            });
                        }
                    }
                }
                let mut output = vec![OutPage {
                    info: page.info.clone(),
                    items: Vec::new(),
                }];
                let deferred_keys = draw_placed(&placed, &mut units, &mut output);
                let mut local_keys: Vec<_> = output[0]
                    .items
                    .iter()
                    .filter_map(crate::office::docx::text::reading_key)
                    .collect();
                local_keys.sort_unstable();
                local_keys.dedup();
                for item in &mut output[0].items {
                    if let Some(key) = crate::office::docx::text::reading_key_mut(item) {
                        let (owner, slot) = deferred_keys.get(key).copied().unwrap_or((*key, 0));
                        let source = if !ordered_merge {
                            ordinary_offsets[ordinal] + local_keys.binary_search(key).unwrap()
                        } else if owner >= crate::office::docx::text::NOTE_KEY {
                            owner - crate::office::docx::text::NOTE_KEY + slot
                        } else {
                            let local = (owner / 1024).saturating_sub(1);
                            (source_keys.get(local).copied().unwrap_or(0) * 1024 + owner % 1024)
                                * reading_stride
                                + slot
                        };
                        *key = crate::office::docx::text::NOTE_KEY
                            + occurrence_bases[ordinal]
                            + source;
                    }
                }
                ordinary_offsets[ordinal] += local_keys.len();
                page.items.append(&mut output[0].items);
            }
            if has_notice {
                let items = super::note_areas::paint_note_slices(
                    area.notice_layout
                        .as_ref()
                        .map_or_else(Vec::new, |layout| layout.legacy_slices().0),
                    &page.info,
                    notice_context.x,
                    notice_y,
                );
                let mut key = crate::office::docx::text::NOTE_KEY + next_base;
                super::note_areas::append_note_items(page, items, &mut key);
                next_base = key - crate::office::docx::text::NOTE_KEY;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests;
