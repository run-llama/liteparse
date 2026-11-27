//! Footnote areas retain the layout context of their accepted references.

use super::*;
use crate::office::docx::model::{FootnoteColumns, FootnotePosition};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct NoteKey {
    id: i64,
    width: u64,
    grid: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct NoteContext {
    pub section: usize,
    pub position: FootnotePosition,
    pub body_columns: usize,
    pub x: f64,
    pub width: f64,
    pub grid: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct NoteReference {
    pub id: i64,
    pub context: NoteContext,
}

#[derive(Debug, Clone)]
pub struct NoteArea {
    pub context: NoteContext,
    pub references: Vec<NoteReference>,
}

/// Mixed continuous owners retain one area from the first reference on each page.
/// Preflight avoids committing column areas before a later incompatible owner.
pub(super) fn mixed_geometry_sections(
    sections: &[Section],
    notes: &HashMap<i64, Vec<Block>>,
) -> HashSet<usize> {
    fn has_notes(blocks: &[Block]) -> bool {
        blocks.iter().any(|block| match block {
            Block::Para(p) => p.inlines.iter().any(|i| match i {
                crate::office::docx::model::Inline::Note(_) => true,
                crate::office::docx::model::Inline::Picture(p, _) => p
                    .shape
                    .as_ref()
                    .is_some_and(|s| s.texts.iter().any(|t| has_notes(&t.blocks))),
                crate::office::docx::model::Inline::Anchor(a) => a
                    .pic
                    .shape
                    .as_ref()
                    .is_some_and(|s| s.texts.iter().any(|t| has_notes(&t.blocks))),
                _ => false,
            }),
            Block::Table(t) => t
                .rows
                .iter()
                .any(|r| r.cells.iter().any(|c| has_notes(&c.blocks))),
        })
    }
    fn has_table_note(blocks: &[Block], notes: &HashMap<i64, Vec<Block>>) -> bool {
        blocks.iter().any(|block| match block {
            Block::Para(p) => p.inlines.iter().any(|inline| match inline {
                crate::office::docx::model::Inline::Note(id) => {
                    notes.get(id).is_some_and(|blocks| {
                        !crate::office::docx::layout::note_source::supported_blocks(blocks)
                    })
                }
                crate::office::docx::model::Inline::Picture(p, _) => p
                    .shape
                    .as_ref()
                    .is_some_and(|s| s.texts.iter().any(|t| has_table_note(&t.blocks, notes))),
                crate::office::docx::model::Inline::Anchor(a) => a
                    .pic
                    .shape
                    .as_ref()
                    .is_some_and(|s| s.texts.iter().any(|t| has_table_note(&t.blocks, notes))),
                _ => false,
            }),
            Block::Table(t) => t
                .rows
                .iter()
                .any(|r| r.cells.iter().any(|c| has_table_note(&c.blocks, notes))),
        })
    }
    let mut legacy = HashSet::new();
    let mut start = 0;
    while start < sections.len() {
        let mut end = start + 1;
        while end < sections.len()
            && matches!(
                sections[end].props.start,
                SectionStart::Continuous | SectionStart::NextColumn
            )
            && (sections[end].props.page_w - sections[end - 1].props.page_w).abs() <= 0.5
            && (sections[end].props.page_h - sections[end - 1].props.page_h).abs() <= 0.5
        {
            end += 1;
        }
        // Table cursors retain measured row widths. Preflight the whole continuous
        // cohort so a later table reference keeps earlier accepted contexts stable.
        if (start..end).any(|i| {
            let columns = sections[i].props.columns();
            columns.iter().any(|c| (c.1 - columns[0].1).abs() > EPS)
                && has_table_note(&sections[i].blocks, notes)
        }) {
            legacy.extend(start..end);
        }
        let owners: Vec<_> = (start..end)
            .filter(|i| has_notes(&sections[*i].blocks))
            .collect();
        if let Some(first) = owners.first() {
            let p = &sections[*first].props;
            if owners.iter().skip(1).any(|i| {
                let q = &sections[*i].props;
                let selected = |s: &SectPr| match s.note_columns {
                    FootnoteColumns::Auto => Some(s.columns().len() as u32),
                    FootnoteColumns::Count(n) => Some(n),
                    FootnoteColumns::Invalid => None,
                };
                p.columns() != q.columns() || selected(p) != selected(q)
            }) {
                legacy.extend(start..end);
            }
        }
        start = end;
    }
    legacy
}

fn intersects(x: f64, width: f64, left: f64, right: f64) -> bool {
    x < right - EPS && x + width > left + EPS
}

/// Choose the origin of an already accepted area without changing its source allocation.
/// Beneath-text placement uses final body geometry and the full note rectangle.
pub(super) fn accepted_note_top(
    page: &mut PageInfo,
    context: &NoteContext,
    height: f64,
    span: (f64, f64),
    single_owner: bool,
) -> f64 {
    let bottom_aligned = page.bottom - height;
    if context.position != FootnotePosition::BeneathText {
        return bottom_aligned;
    }
    if !single_owner
        || page.cols.len() != 1
        || context.body_columns != 1
        || page.note_regions.iter().any(|r| r.body_columns != 1)
        || page
            .note_regions
            .iter()
            .any(|r| !r.notes.is_empty() && r.section != context.section)
    {
        page.note_diagnostics.push("Beneath-text footnote placement with multiple columns or section owners retains bottom alignment".into());
        return bottom_aligned;
    }
    let mut top = page
        .note_regions
        .iter()
        .map(|r| r.body_bottom)
        .fold(page.top, f64::max);
    loop {
        let next = page
            .note_regions
            .iter()
            .flat_map(|r| &r.exclusions)
            .filter(|&&(a, b, left, right)| {
                intersects(span.0, span.1 - span.0, left, right)
                    && top < b - EPS
                    && top + height > a + EPS
            })
            .map(|&(_, bottom, _, _)| bottom)
            .fold(top, f64::max);
        if next <= top + EPS {
            break;
        }
        top = next;
    }
    if top > bottom_aligned + EPS {
        page.note_diagnostics.push("Beneath-text footnote area cannot clear final body exclusions within its accepted capacity; bottom alignment retained".into());
        bottom_aligned
    } else {
        top
    }
}

/// Assign source slots recursively before the cached row projection is painted.
/// Nested row provenance follows the same cell/slice traversal as cached items
/// and anchors; slots keep textbox source order independent of floating z-order.
pub(crate) fn note_slice_keys(
    slice: &crate::office::docx::layout::Slice,
    next: &mut usize,
    text: &mut Vec<usize>,
    anchors: &mut Vec<(usize, f64)>,
) {
    if let Some(child) = &slice.nested_table {
        text.extend(child.text_keys.iter().map(|key| *next + key));
        anchors.extend(
            child
                .anchor_keys
                .iter()
                .map(|(key, offset)| (*next + key, *offset)),
        );
        *next += child.layout.reading_stride().max(
            child
                .text_keys
                .iter()
                .copied()
                .chain(child.anchor_keys.iter().map(|(key, _)| *key))
                .max()
                .unwrap_or(0)
                + 1,
        );
    } else if let Some(row) = &slice.nested_row {
        text.extend(std::iter::repeat_n(*next, row.bookmarks_before.len()));
        for cell in &row.cells {
            for child in cell.slices.iter().chain(cell.moved_slices()) {
                note_slice_keys(child, next, text, anchors);
            }
        }
        text.extend(std::iter::repeat_n(*next, row.bookmarks_after.len()));
        if !row.bookmarks_after.is_empty() {
            *next += 1;
        }
    } else {
        text.extend(slice.items.iter().filter_map(|item| match item {
            Item::Text(_) | Item::Bookmark(_) => Some(
                *next
                    + if slice.anchors.is_empty() {
                        0
                    } else {
                        crate::office::docx::text::reading_key(item).unwrap()
                    },
            ),
            _ => None,
        }));
        for (index, _) in slice.anchors.iter().enumerate() {
            anchors.push((*next + 2 * index + 1, slice.paragraph_offset));
        }
        *next += 2 * slice.anchors.len() + 1;
    }
}

/// Paint only accepted slices at their existing note-area origin and height.
/// Anchor rendering shares the page context used by body and fragmented notes.
pub(super) fn paint_note_slices(
    slices: Vec<crate::office::docx::layout::Slice>,
    page: &PageInfo,
    x: f64,
    y: f64,
) -> Vec<Item> {
    let ordered = slices.iter().any(|slice| !slice.anchors.is_empty());
    let mut items = Vec::new();
    let mut floats = Vec::new();
    let mut next = 0;
    for slice in slices {
        let mut text_keys = Vec::new();
        let mut anchor_keys = Vec::new();
        if ordered {
            note_slice_keys(&slice, &mut next, &mut text_keys, &mut anchor_keys);
        }
        let mut text_keys = text_keys.into_iter();
        for mut item in slice.items {
            translate_item(&mut item, x, y);
            if let Some(key) = crate::office::docx::text::reading_key_mut(&mut item) {
                *key = if ordered {
                    text_keys.next().expect("note slice text key")
                } else {
                    0
                };
            }
            items.push(item);
        }
        debug_assert!(text_keys.next().is_none());
        if ordered {
            assert_eq!(
                slice.anchors.len(),
                anchor_keys.len(),
                "note slice anchor keys"
            );
        }
        for ((anchor, ax, ay), (key, paragraph_offset)) in
            slice.anchors.into_iter().zip(anchor_keys)
        {
            if let Some((mut drawn, behind)) =
                anchor_item(&anchor, page, x + ax, y + ay + paragraph_offset, y + ay)
            {
                crate::office::docx::text::set_reading_key(&mut drawn, key);
                floats.push((behind, anchor.z, drawn));
            }
        }
    }
    floats.sort_by_key(|(behind, z, _)| (!*behind, *z));
    let mut back = Vec::new();
    for (behind, _, drawn) in floats {
        if behind {
            back.extend(drawn);
        } else {
            items.extend(drawn);
        }
    }
    back.extend(items);
    back
}

pub(super) fn append_note_items(page: &mut OutPage, mut items: Vec<Item>, next: &mut usize) {
    let mut keys: Vec<_> = items
        .iter()
        .filter_map(|item| match item {
            Item::Text(run) => Some(run.paragraph),
            Item::Bookmark(marker) => Some(marker.reading_key),
            _ => None,
        })
        .collect();
    keys.sort_unstable();
    keys.dedup();
    for item in &mut items {
        if let Some(key) = crate::office::docx::text::reading_key_mut(item) {
            *key = *next + keys.binary_search(key).unwrap();
        }
    }
    *next += keys.len();
    page.items.extend(items);
}

impl Paginator<'_> {
    pub(super) fn current_note_context(&self) -> (NoteContext, Option<String>) {
        let (section, props) = &self.note_section;
        let columns = props.columns();
        let count_matches = matches!(props.note_columns, FootnoteColumns::Auto)
            || matches!(props.note_columns, FootnoteColumns::Count(n) if n as usize == columns.len());
        let mixed = self.legacy_note_sections.contains(section);
        let supported = count_matches && !mixed;
        let (x, width) = if supported {
            columns.get(self.st.col).copied().unwrap_or(columns[0])
        } else {
            (
                columns[0].0,
                columns.iter().map(|c| c.1).sum::<f64>().max(36.0),
            )
        };
        let (x, width) = if mixed {
            self.pages
                .last()
                .and_then(|p| p.note_areas.first())
                .map_or((x, width), |a| (a.context.x, a.context.width))
        } else {
            (x, width)
        };
        let warning = (!supported).then(|| {
            format!("section {section}: custom footnote columns, width-dependent table notes or mixed continuous note geometry retain one legacy area")
        });
        (
            NoteContext {
                section: *section,
                position: props.note_position,
                body_columns: columns.len(),
                x,
                width,
                grid: props.character_grid(),
            },
            warning,
        )
    }

    #[cfg(test)]
    pub(super) fn note_cache_key(&self, id: i64) -> NoteKey {
        let c = self.current_note_context().0;
        NoteKey {
            id,
            width: c.width.to_bits(),
            grid: c.grid.map(f64::to_bits),
        }
    }

    /// Keep source units alongside the unchanged stack projection for current drawing.
    pub(super) fn note_in(
        &self,
        context: &NoteContext,
        id: i64,
    ) -> (Vec<crate::office::docx::layout::Slice>, f64) {
        self.note_layout_in(context, id).legacy_slices()
    }

    pub(super) fn note_layout_in(
        &self,
        context: &NoteContext,
        id: i64,
    ) -> super::note_fragments::NoteLayout {
        let key = NoteKey {
            id,
            width: context.width.to_bits(),
            grid: context.grid.map(f64::to_bits),
        };
        if let Some(hit) = self.note_cache.borrow().get(&key) {
            return hit.clone();
        }
        let blocks = if id == i64::MIN {
            Some(self.hf.separator)
        } else if id == i64::MIN + 1 {
            self.hf.continuation_separator
        } else if id == i64::MIN + 2 {
            self.hf.continuation_notice
        } else {
            self.hf.notes.get(&id)
        };
        let laid = self.layout_note_blocks(context, blocks);
        self.note_cache.borrow_mut().insert(key, laid.clone());
        laid
    }

    pub(super) fn layout_note_blocks(
        &self,
        context: &NoteContext,
        blocks: Option<&Vec<Block>>,
    ) -> super::note_fragments::NoteLayout {
        self.with_note_env(context, |env| {
            let units = blocks.map_or_else(Vec::new, |b| {
                Flow::new(env, self.compat_mode)
                    .for_notes()
                    .units(b, context.width, false)
            });
            super::note_fragments::NoteLayout::with_local_pictures(units, context.width, env)
        })
    }

    pub(super) fn with_note_env<T>(
        &self,
        context: &NoteContext,
        use_env: impl FnOnce(&TextEnv<'_>) -> T,
    ) -> T {
        let env = TextEnv {
            fonts: self.env.fonts,
            default_tab: self.env.default_tab,
            fields: self.env.fields.clone(),
            grid: self.env.grid,
            char_grid: std::cell::Cell::new(context.grid),
            expand_shift_return: self.env.expand_shift_return,
            mark_in_last_line: self.env.mark_in_last_line,
            html_spacing: self.env.html_spacing,
            grow_autofit: self.env.grow_autofit,
            in_cell: Default::default(),
            in_header: Default::default(),
            grid_in_table: self.env.grid_in_table,
            no_leading: self.env.no_leading,
            compat_mode: self.env.compat_mode,
        };
        use_env(&env)
    }

    pub(super) fn special_note_layout(
        &self,
        context: &NoteContext,
        id: i64,
        capacity: f64,
    ) -> (super::note_fragments::NoteLayout, bool) {
        let original = self.note_layout_in(context, id);
        if original.height() <= capacity + 0.01 {
            return (original, false);
        }
        let source = match id {
            i64::MIN => Some(self.hf.separator),
            id if id == i64::MIN + 1 => self.hf.continuation_separator,
            _ => self.hf.continuation_notice,
        };
        let Some(source) = source else {
            return (original, false);
        };
        let mut blocks = source.clone();
        let mut changed = false;
        // Only outer spacing is relaxed; internal paragraph spacing remains source flow.
        if let Some(Block::Para(p)) = blocks.first_mut()
            && p.style.before > 0.0
        {
            let style = std::rc::Rc::make_mut(&mut p.style);
            style.before = 0.0;
            style.before_auto = false;
            changed = true;
        }
        if let Some(Block::Para(p)) = blocks.last_mut()
            && p.style.after > 0.0
        {
            let style = std::rc::Rc::make_mut(&mut p.style);
            style.after = 0.0;
            style.after_auto = false;
            changed = true;
        }
        if changed {
            (self.layout_note_blocks(context, Some(&blocks)), true)
        } else {
            (original, false)
        }
    }

    pub(super) fn pending_areas(&self, page: &PageInfo, pending: &[i64]) -> Vec<NoteArea> {
        let mut areas = page.note_areas.clone();
        if pending.iter().all(|id| page.notes.contains(id)) {
            return areas;
        }
        let mut seen: HashSet<i64> = page.notes.iter().copied().collect();
        let (context, _) = self.current_note_context();
        for id in pending {
            if !seen.insert(*id) {
                continue;
            }
            let index = areas
                .iter()
                .position(|a| {
                    (a.context.x - context.x).abs() < EPS
                        && (a.context.width - context.width).abs() < EPS
                })
                .unwrap_or_else(|| {
                    areas.push(NoteArea {
                        context: context.clone(),
                        references: Vec::new(),
                    });
                    areas.len() - 1
                });
            areas[index].references.push(NoteReference {
                id: *id,
                context: context.clone(),
            });
        }
        areas
    }

    pub(super) fn area_height(&self, area: &NoteArea, page: &PageInfo) -> f64 {
        if area.references.is_empty() {
            return 0.0;
        }
        let mut height = self.note_in(&area.context, i64::MIN).1;
        for reference in &area.references {
            if !self.hf.notes.contains_key(&reference.id) {
                continue;
            }
            let h = self.note_in(&reference.context, reference.id).1;
            height += match page.note_cut {
                Some((id, cut)) if id == reference.id => h.min(cut),
                _ => h,
            };
        }
        height
    }

    fn carry_height(&self, page: &PageInfo) -> f64 {
        page.note_carry.map_or(0.0, |(id, offset)| {
            let context = page
                .note_areas
                .iter()
                .flat_map(|a| &a.references)
                .find(|r| r.id == id)
                .map(|r| r.context.clone())
                .unwrap_or_else(|| self.current_note_context().0);
            (self.note_in(&context, id).1 - offset).max(0.0)
                + if page.notes.is_empty() {
                    self.note_in(&context, i64::MIN).1
                } else {
                    0.0
                }
        })
    }

    pub(super) fn note_limit(&self, page: &PageInfo, pending: &[i64]) -> f64 {
        if pending.is_empty() && self.current_note_page().active {
            let (x, width) = page.cols[self.st.col];
            return page.bottom - self.current_note_page().height_at(x, width);
        }
        if page.notes.is_empty()
            && pending.is_empty()
            && page.note_carry.is_none()
            && self.current_note_page().incoming.is_empty()
        {
            return page.bottom;
        }
        let (x, width) = page.cols.get(self.st.col).copied().unwrap_or((72.0, 468.0));
        let reserve = self
            .pending_areas(page, pending)
            .iter()
            .filter(|a| intersects(x, width, a.context.x, a.context.x + a.context.width))
            .map(|a| self.area_height(a, page))
            .fold(0.0, f64::max);
        page.bottom - reserve - self.carry_height(page)
    }

    pub(super) fn accept_notes(&mut self, notes: &[i64]) {
        let Some(page) = self.pages.last() else {
            return;
        };
        let areas = self.pending_areas(page, notes);
        let (_, warning) = self.current_note_context();
        let page = self.pages.last_mut().unwrap();
        page.note_areas = areas;
        for id in notes {
            if !page.notes.contains(id) {
                page.notes.push(*id);
            }
        }
        if !notes.is_empty()
            && let Some(warning) = warning
            && !page.note_diagnostics.contains(&warning)
        {
            page.note_diagnostics.push(warning);
        }
    }

    pub(super) fn note_picture_rects(
        &self,
        line: &crate::office::docx::text::Line,
        para_top: f64,
        y: f64,
    ) -> Vec<(f64, f64, f64, f64)> {
        let Some(page) = self.pages.last() else {
            return Vec::new();
        };
        line.anchors
            .iter()
            .filter(|a| Self::movable_picture(a, page))
            .flat_map(|a| {
                let (x, y, w, h) = anchor_rect(a, page, self.col_x(), para_top, y);
                wrap_boxes(a, x, y, w, h)
            })
            .collect()
    }

    pub(super) fn picture_note_conflict(
        &self,
        rectangles: &[(f64, f64, f64, f64)],
        pending: &[i64],
    ) -> bool {
        let Some(page) = self.pages.last() else {
            return false;
        };
        if rectangles.is_empty()
            || (page.notes.is_empty()
                && pending.is_empty()
                && page.note_carry.is_none()
                && self.current_note_page().incoming.is_empty())
        {
            return false;
        }
        if let Some(plan) = self.plan_page_notes_with_pictures(page, pending, self.st.y, rectangles)
        {
            return plan.is_err()
                || self.new_note_start_conflict(page, pending, self.st.y, false, rectangles);
        }
        let areas = self.pending_areas(page, pending);
        let carry = self.carry_height(page);
        rectangles.iter().any(|&(top, bottom, left, right)| {
            areas.iter().any(|area| {
                let height = self.area_height(area, page);
                height > EPS
                    && intersects(area.context.x, area.context.width, left, right)
                    && bottom > page.bottom - height - carry + EPS
                    && top < page.bottom - EPS
            }) || (carry > EPS && bottom > page.bottom - carry + EPS && top < page.bottom - EPS)
        })
    }

    /// Emit note text in accepted global order even when area membership interleaves.
    pub(super) fn draw_notes(&self, out: &mut [OutPage]) {
        for (index, page) in out.iter_mut().enumerate() {
            if self.draw_note_fragments(page, index) {
                continue;
            }
            let mut positioned = HashMap::new();
            let mut next_key = crate::office::docx::text::NOTE_KEY;
            let mut areas = page.info.note_areas.clone();
            if areas.is_empty() && page.info.note_carry.is_some() {
                let mut context = self.current_note_context().0;
                context.position = FootnotePosition::PageBottom;
                context.x = page.info.cols.first().map_or(72.0, |c| c.0);
                context.width = page.info.cols.iter().map(|c| c.1).sum::<f64>().max(36.0);
                areas.push(NoteArea {
                    context,
                    references: Vec::new(),
                });
            }
            for (area_index, area) in areas.iter().enumerate() {
                let carry = if area_index == 0 {
                    page.info.note_carry
                } else {
                    None
                };
                let carry_height = carry.map_or(0.0, |(id, from)| {
                    (self.note_in(&area.context, id).1 - from).max(0.0)
                });
                let height = self.area_height(area, &page.info)
                    + carry_height
                    + if area.references.is_empty() {
                        self.note_in(&area.context, i64::MIN).1
                    } else {
                        0.0
                    };
                let single_owner = carry.is_none()
                    && areas.len() == 1
                    && area
                        .references
                        .iter()
                        .all(|r| r.context.section == area.context.section);
                let mut y = accepted_note_top(
                    &mut page.info,
                    &area.context,
                    height,
                    (area.context.x, area.context.x + area.context.width),
                    single_owner,
                );
                if page.info.note_regions.iter().any(|region| {
                    intersects(
                        area.context.x,
                        area.context.width,
                        region.x,
                        region.x + region.width,
                    ) && region.body_bottom > y + EPS
                }) {
                    page.info.note_diagnostics.push("Unfragmented footnote area intersects accepted body flow bounds; continuation or mixed-area reflow remains unsupported".into());
                }
                let (separator, height) = self.note_in(&area.context, i64::MIN);
                let items = paint_note_slices(separator, &page.info, area.context.x, y);
                append_note_items(page, items, &mut next_key);
                y += height;
                if let Some((id, from)) = carry {
                    let (slices, h) = self.note_in(&area.context, id);
                    let slices = slices
                        .into_iter()
                        .filter(|slice| slice.y + EPS >= from)
                        .collect();
                    let items = paint_note_slices(slices, &page.info, area.context.x, y - from);
                    append_note_items(page, items, &mut next_key);
                    y += (h - from).max(0.0);
                }
                for reference in &area.references {
                    let (slices, h) = self.note_in(&reference.context, reference.id);
                    let end = page
                        .info
                        .note_cut
                        .filter(|(id, _)| *id == reference.id)
                        .map_or(f64::INFINITY, |(_, h)| h);
                    let slices = slices
                        .into_iter()
                        .filter(|slice| slice.y + slice.h <= end + EPS)
                        .collect();
                    let items = paint_note_slices(slices, &page.info, area.context.x, y);
                    positioned.insert(reference.id, items);
                    y += h.min(end);
                }
            }
            for id in page.info.notes.clone() {
                if let Some(items) = positioned.remove(&id) {
                    append_note_items(page, items, &mut next_key);
                }
            }
            if page.info.note_carry.is_some() {
                page.info
                    .note_diagnostics
                    .push("Footnote continuation area ownership is not implemented".into());
            }
        }
    }
}

#[cfg(test)]
mod tests;
