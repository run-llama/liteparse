//! Blocks → layout units. A paragraph becomes its lines with the spacing,
//! keep and widow settings pagination needs; a table becomes one unit per row
//! with every cell's content already laid out. Cell contents are stacked
//! without pagination into slices, the pieces a row splits between pages.

mod cells;
mod dropcaps;
pub(crate) mod note_children;
pub(crate) mod note_source;
mod table_width;

use std::rc::Rc;

use crate::office::pptx::color::Rgba;
use crate::office::pptx::scene::{Item, Paint, PathItem};

use crate::office::docx::model::{Block, Cell, HeightRule, Inline, Paragraph, Table, VMerge};
use crate::office::docx::props::Border;
use crate::office::docx::text::{Line, TextEnv, layout_paragraph, translate_item};

/// `beforeAutospacing`/`afterAutospacing`: Word's HTML-style 14 pt.
const AUTO_SPACING: f64 = 14.0;

#[derive(Debug, Clone)]
pub struct ParaBox {
    /// Horizontal extent of borders and shading (column-relative).
    pub x0: f64,
    pub x1: f64,
    pub top: Option<Border>,
    pub bottom: Option<Border>,
    pub left: Option<Border>,
    pub right: Option<Border>,
    pub shading: Option<Rgba>,
    /// Paragraph spacing inside a shared box: above the first line (space
    /// before, when joined to the previous paragraph) and below the last.
    pub grow: (f64, f64),
}

#[derive(Debug, Clone)]
pub struct ParaUnit {
    pub lines: Vec<Line>,
    /// Body frames injected from preceding blocks, retained when source text reflows.
    pub frame_anchors: Vec<crate::office::docx::model::Anchored>,
    /// Index of the generated drop-cap anchor in the paragraph anchor sequence.
    pub drop_cap_anchor: Option<usize>,
    /// Source cell policy retained when merged paragraph payload is remeasured.
    pub clip_inline_images: bool,
    pub before: f64,
    pub after: f64,
    /// Room taken by a top/bottom border (width plus its spacing).
    pub pad_top: f64,
    pub pad_bottom: f64,
    pub keep_next: bool,
    pub keep_lines: bool,
    pub widow: bool,
    pub page_break_before: bool,
    /// The paragraph holds a clearing break: the text after it goes below floats.
    pub clear_after: bool,
    /// The paragraph's first line holds only a column break.
    pub column_break_before: bool,
    pub decor: Option<ParaBox>,
    /// The paragraph, to lay it out again beside a square-wrapped object.
    pub src: Option<Rc<Paragraph>>,
}

/// Resolve the initial preferred grid against the supplied story width.
/// The resulting columns supply cell text, anchors, borders and continuation geometry.
fn resolve_percentage_grid(grid: &mut [f64], percentage: Option<f64>, width: f64) {
    let Some(fraction) = percentage.filter(|f| f.is_finite() && *f > 0.0) else {
        return;
    };
    let sum: f64 = grid.iter().sum();
    let target = fraction * width;
    if sum > 1.0
        && sum.is_finite()
        && target.is_finite()
        && target > 0.0
        && grid.iter().all(|g| g.is_finite() && *g >= 0.0)
    {
        let factor = target / sum;
        grid.iter_mut().for_each(|g| *g *= factor);
    }
}

/// A stacked piece of laid-out content: a line or a retained nested row.
#[derive(Debug, Clone)]
pub struct Slice {
    pub y: f64,
    pub h: f64,
    pub items: Vec<Item>,
    /// Floating objects anchored here: (object, column x, line top), in the
    /// same coordinates as `items`; placed once the page is known.
    pub anchors: Vec<(crate::office::docx::model::Anchored, f64, f64)>,
    /// Paragraph origin minus line origin, in points; unchanged by slice translation.
    pub paragraph_offset: f64,
    /// A paragraph line under widow control: (line index, line count).
    pub widow: Option<(usize, usize)>,
    /// Footnotes referenced in this slice (a line, or a nested row).
    pub notes: Vec<i64>,
    /// Source row in local coordinates; cached paint follows the enclosing slice origin.
    pub nested_row: Option<Rc<RowUnit>>,
    pub nested_table: Option<Rc<note_children::ChildTable>>,
    /// Immutable paragraph projection and source line retained through cell cuts.
    pub paragraph_source: Option<(Rc<ParaUnit>, usize)>,
}

#[derive(Debug, Clone)]
pub struct CellBox {
    pub x: f64,
    pub w: f64,
    pub slices: Vec<Slice>,
    pub content_h: f64,
    /// Final ordinary paragraph spacing carried with its last line when splitting.
    pub terminal_after: f64,
    pub margins: [f64; 4],
    pub shading: Option<Rgba>,
    /// top, left, bottom, right.
    pub borders: [Option<Border>; 4],
    pub v_align: String,
    /// Continuation of a vertically merged cell: draws only its sides.
    pub merged: bool,
    /// A valid same-grid vertical merge continues into the following source row.
    pub merge_continues: bool,
    /// Content moved here from a vertical merge that starts this many rows
    /// above (reading order keys it at its first row); 0 otherwise.
    pub merged_from: usize,
    /// Diagonal borders (top-left to bottom-right, top-right to bottom-left).
    pub diagonals: [Option<Border>; 2],
    /// Content of a vertical merge starting `merged_from` rows above, in its own
    /// coordinates, and its offset from this cell's content top when every
    /// spanned row is on one page.
    pub moved: Option<Rc<(Vec<Slice>, f64)>>,
}

#[derive(Debug, Clone)]
pub struct RowSource {
    pub table: Rc<Table>,
    pub width: f64,
    /// Remaining authored minimum row extent, in points, excluding content growth.
    pub minimum: f64,
    /// Source border and cell-spacing extent added after natural content sizing.
    pub addition: f64,
}

#[derive(Debug, Clone)]
pub struct RowUnit {
    /// Semantic container edges retained independently of cell content and flow height.
    pub bookmarks_before: Vec<std::sync::Arc<crate::office::pptx::scene::SourceBookmark>>,
    pub bookmarks_after: Vec<std::sync::Arc<crate::office::pptx::scene::SourceBookmark>>,
    /// Source row descriptor retained before merged content moves between rows.
    pub provenance: Option<std::sync::Arc<crate::office::pptx::scene::TextSource>>,
    /// Note-owned source used to rebuild a remaining row at a new destination width.
    pub source: Option<Rc<RowSource>>,
    /// Distance between horizontal border centerlines, in points.
    pub height: f64,
    /// Authored "at least" row height with its border share (points; 0 without one):
    /// Word moves such a row whole rather than split it where this does not fit.
    pub min_height: f64,
    /// Content height without the authored minimum, with the same border share.
    pub natural_height: f64,
    /// The previous row of the table keeps with this one.
    pub after_keep: bool,
    /// Resolved border and cell-spacing extent added to natural content height.
    pub natural_addition: f64,
    /// Height before spanning-cell content enlarges a terminal row.
    pub merge_base_height: f64,
    /// Painted natural-table edges outside the first/last row centerlines.
    pub outer_before: f64,
    pub outer_after: f64,
    pub cells: Vec<CellBox>,
    /// A leading first-cell paragraph requests a page boundary before this row.
    pub page_break_before: bool,
    pub cant_split: bool,
    pub exact_height: bool,
    pub header: bool,
    /// Member of the leading header block that is actually repeated.
    pub repeating_header: bool,
    pub table: usize,
    /// The table's repeated header rows (leading `tblHeader` rows).
    pub headers: Rc<Vec<RowUnit>>,
    /// First row of a floating table: its vertical anchor and offset.
    pub float_y: Option<(String, Result<f64, String>)>,
    /// Numeric horizontal anchor of a body floating table, resolved at placement.
    pub float_x: Option<String>,
    /// A floating table that may not overlap other floating objects.
    pub float_clear: bool,
    /// Every paragraph of the row keeps with the next: the row stays on the
    /// page of the row (or paragraph) after it.
    pub keep_next: bool,
    /// Row number in its table.
    pub index: usize,
}

#[derive(Debug, Clone)]
#[expect(
    clippy::large_enum_variant,
    reason = "units live in one Vec per layout pass; boxing would add an allocation per unit"
)]
pub enum Unit {
    Para(ParaUnit),
    Row(RowUnit),
}

impl RowUnit {
    pub fn flow_height(&self) -> f64 {
        self.outer_before + self.height + self.outer_after
    }

    /// Cross-row merge ownership needs a shared continuation ledger.
    pub fn fragmentable_structure(&self) -> bool {
        self.float_y.is_none() && self.fragmentable_cells()
    }

    /// Stacked nested rows already have cell-local placement, including tblpPr rows.
    fn fragmentable_cells(&self) -> bool {
        self.cells.iter().all(|c| {
            c.moved.is_none()
                && !c.merged
                && !c.merge_continues
                && c.merged_from == 0
                && c.slices
                    .iter()
                    .all(|s| s.nested_row.as_ref().is_none_or(|r| r.fragmentable_cells()))
        })
    }

    /// Footnotes referenced in the row's cells, in order.
    pub fn notes(&self) -> Vec<i64> {
        let mut out: Vec<i64> = Vec::new();
        for id in self
            .cells
            .iter()
            .flat_map(|c| c.slices.iter().chain(c.moved_slices()))
            .flat_map(|s| s.notes.iter())
        {
            if !out.contains(id) {
                out.push(*id);
            }
        }
        out
    }
}

impl CellBox {
    /// Slices of merged content moved into this cell.
    pub fn moved_slices(&self) -> impl Iterator<Item = &Slice> {
        self.moved.iter().flat_map(|m| m.0.iter())
    }
}

impl Unit {
    pub fn keep_next(&self) -> bool {
        match self {
            Unit::Para(p) => p.keep_next,
            Unit::Row(r) => r.keep_next,
        }
    }
}

#[derive(Default)]
struct BoxContent {
    items: Vec<Item>,
    anchors: Vec<crate::office::docx::model::FrameAnchor>,
}

impl BoxContent {
    fn extend(&mut self, other: Self) {
        self.items.extend(other.items);
        self.anchors.extend(other.anchors);
    }
}

fn box_content(
    slices: Vec<Slice>,
    transform: tiny_skia::Transform,
    width: f64,
    ea_vertical: bool,
    paint: impl Fn(&mut Item),
) -> BoxContent {
    let mut out = BoxContent::default();
    for slice in slices {
        for mut item in slice.items.iter().cloned() {
            paint(&mut item);
            if ea_vertical && let Item::Text(run) = &mut item {
                out.items.extend(upright_cjk(run));
            }
            out.items.push(item);
        }
        box_slice_anchors(&slice, transform, width, 0.0, 0.0, &mut out.anchors);
    }
    out
}

// Retained rows supply the leaf paragraph origin independently of flattened paint.
fn box_slice_anchors(
    slice: &Slice,
    transform: tiny_skia::Transform,
    width: f64,
    x: f64,
    y: f64,
    out: &mut Vec<crate::office::docx::model::FrameAnchor>,
) {
    if let Some(row) = &slice.nested_row {
        for cell in &row.cells {
            let free = (row.height - cell.margins[0] - cell.margins[2] - cell.content_h).max(0.0);
            let dy = match cell.v_align.as_str() {
                "center" => free / 2.0,
                "bottom" => free,
                _ => 0.0,
            };
            let cx = x + cell.x + cell.margins[1];
            let cy = y + slice.y + row.outer_before + cell.margins[0];
            let inner = (cell.w - cell.margins[1] - cell.margins[3]).max(0.0);
            for child in &cell.slices {
                box_slice_anchors(child, transform, inner, cx, cy + dy, out);
            }
            if let Some(moved) = &cell.moved {
                for child in &moved.0 {
                    box_slice_anchors(child, transform, inner, cx, cy + moved.1, out);
                }
            }
        }
        return;
    }
    for (index, (anchor, ax, ay)) in slice.anchors.iter().enumerate() {
        let offset = slice
            .nested_table
            .as_ref()
            .and_then(|table| table.anchor_keys.get(index))
            .map_or(slice.paragraph_offset, |(_, offset)| *offset);
        out.push(crate::office::docx::model::FrameAnchor {
            placement: Some((transform, x + width)),
            anchor: anchor.clone(),
            column_x: x + ax,
            paragraph_y: y + ay + offset,
            line_y: y + ay,
        });
    }
}

pub(crate) fn inline_box_owner(anchor: &crate::office::docx::model::Anchored) -> bool {
    anchor.pic.w == 0.0
        && anchor.pic.h == 0.0
        && anchor.pic.part.is_none()
        && anchor.pic.shape.is_none()
        && !anchor.frame_children.is_empty()
        && anchor
            .frame_children
            .iter()
            .all(|child| child.placement.is_some())
}

// The zero-sized owner follows its inline source line without adding wrap extent.
fn box_anchor(
    children: Vec<crate::office::docx::model::FrameAnchor>,
) -> crate::office::docx::model::Anchored {
    use crate::office::docx::model::*;
    Anchored {
        pic: Picture {
            part: None,
            w: 0.0,
            h: 0.0,
            crop: (0.0, 0.0, 0.0, 0.0),
            alt: None,
            shape: None,
            effects: Vec::new(),
            alpha: 1.0,
            graphic: None,
            shadow: None,
        },
        h_rel: HRel::Column,
        x: Ok(0.0),
        v_rel: VRel::Line,
        y: Ok(0.0),
        behind: false,
        z: 0,
        wrap: Wrap::None,
        contour: None,
        dist: [0.0; 4],
        wrap_side: "bothSides".into(),
        text_items: Vec::new(),
        frame_children: children,
        text_frame: false,
        turned: false,
    }
}

pub struct Flow<'a> {
    pub env: &'a TextEnv<'a>,
    pub compat_mode: u32,
    footer_frames: bool,
    note_story: bool,
    next_table: std::cell::Cell<usize>,
}

impl<'a> Flow<'a> {
    pub fn new(env: &'a TextEnv<'a>, compat_mode: u32) -> Flow<'a> {
        Flow {
            env,
            compat_mode,
            footer_frames: false,
            note_story: false,
            next_table: std::cell::Cell::new(0),
        }
    }

    /// Note fragment ledgers retain merged anchors with their source slices.
    pub fn for_notes(mut self) -> Self {
        self.note_story = true;
        self
    }

    /// Footer text frames float from the following ordinary paragraph.
    pub fn with_footer_frames(mut self) -> Self {
        self.footer_frames = true;
        self
    }

    /// Lays out blocks at `width` into units.
    pub fn units(&self, blocks: &[Block], width: f64, in_cell: bool) -> Vec<Unit> {
        let mut out = Vec::new();
        // Text frames waiting for the paragraph they are anchored to.
        let mut frames: Vec<crate::office::docx::model::Anchored> = Vec::new();
        let mut skip_to = 0;
        let hf = self.env.in_header.get();
        let page_frames = !hf && !in_cell;
        let selected_frame = |b| {
            if page_frames {
                page_frame_of(b).or_else(|| body_text_frame_of(b))
            } else if hf && !in_cell && self.footer_frames {
                footer_frame_of(b)
            } else if hf && !in_cell {
                // A header frame at a page position, or anchored to the text,
                // floats and leaves the header's height alone.
                page_frame_of(b).or_else(|| footer_frame_of(b))
            } else {
                None
            }
        };
        for (bi, b) in blocks.iter().enumerate() {
            if bi < skip_to {
                continue;
            }
            if let Some(f) = selected_frame(b) {
                let end = (bi..blocks.len())
                    .find(|k| selected_frame(&blocks[*k]) != Some(f))
                    .unwrap_or(blocks.len());
                // Header/footer frames use the following ordinary paragraph as
                // their anchor, past any further frames (a header's title, page
                // number and blank frames side by side). Unanchored groups stay in flow.
                let anchor = blocks[end..].iter().find(|b| selected_frame(b).is_none());
                if (!hf && f.v_anchor == "page") || anchor.is_some_and(ordinary_frame_anchor) {
                    frames.push(self.frame(&blocks[bi..end], f, width));
                    skip_to = end;
                    continue;
                }
            }
            let paired = if !hf && !in_cell {
                blocks
                    .get(bi + 1)
                    .and_then(|next| self.drop_cap_pair(b, next, width))
            } else {
                None
            };
            let anchor_index = if paired.is_some() { bi + 1 } else { bi };
            if paired.is_some() {
                skip_to = bi + 2;
            }
            let paired_block = paired.map(Block::Para);
            let b = paired_block.as_ref().unwrap_or(b);
            match b {
                Block::Para(p) => {
                    // A cell's closing empty paragraph after a nested table takes
                    // no room in Word (page-1 thumbnails).
                    if in_cell
                        && bi + 1 == blocks.len()
                        && bi > 0
                        && matches!(blocks[bi - 1], Block::Table(_))
                        && is_empty_para(p)
                    {
                        if let Some(Unit::Row(row)) = out.last_mut() {
                            row.bookmarks_after.extend(paragraph_markers(p));
                        }
                        continue;
                    }
                    // Floating tables are out of the flow: spacing and contextual
                    // spacing see the paragraphs around them as neighbours
                    let skip = |b: &&Block| {
                        matches!(b, Block::Table(t) if t.float.is_some())
                            || (!in_cell && matches!(b, Block::Para(p) if hidden_paragraph(p)))
                    };
                    let prev = blocks[..bi]
                        .iter()
                        .rev()
                        .find(|b| !skip(b))
                        .and_then(para_of);
                    let next = blocks[anchor_index + 1..]
                        .iter()
                        .find(|b| !skip(b))
                        .and_then(para_of);
                    let framed = frame_aligned(p);
                    let p = framed.as_ref().unwrap_or(p);
                    let mut unit = self.para(p, prev, next, width, in_cell, bi == 0);
                    // Auto spacing after a cell's last paragraph is dropped, as before its
                    // first one (HTML margins collapse into the cell).
                    if in_cell && anchor_index + 1 == blocks.len() && p.style.after_auto {
                        unit.after = 0.0;
                    }
                    if paired_block.is_some() {
                        unit.drop_cap_anchor = Some(0);
                    }
                    if let Some(l) = unit.lines.first_mut() {
                        if page_frames {
                            unit.frame_anchors.extend(frames.iter().cloned());
                        }
                        l.anchors.append(&mut frames);
                    }
                    out.push(Unit::Para(unit));
                }
                Block::Table(t) => self.table(t, width, !in_cell && !hf, &mut out),
            }
        }
        if !frames.is_empty() && page_frames {
            // A section ending in frames needs an anchor even without body text.
            // Zero flow height keeps its explicit page coordinates independent of spacing.
            out.push(Unit::Para(ParaUnit {
                lines: vec![Line {
                    height: 0.0,
                    trailing_leading: 0.0,
                    baseline: 0.0,
                    items: Vec::new(),
                    break_after: None,
                    anchors: frames,
                    notes: Vec::new(),
                    text: String::new(),
                    skip: 0.0,
                    boxes: Vec::new(),
                    replay: None,
                }],
                frame_anchors: Vec::new(),
                drop_cap_anchor: None,
                clip_inline_images: false,
                before: 0.0,
                after: 0.0,
                pad_top: 0.0,
                pad_bottom: 0.0,
                keep_next: false,
                keep_lines: false,
                widow: false,
                page_break_before: false,
                clear_after: false,
                column_break_before: false,
                decor: None,
                src: None,
            }));
        } else if !frames.is_empty()
            && let Some(Unit::Para(u)) = out.iter_mut().rev().find(|u| matches!(u, Unit::Para(_)))
            && let Some(l) = u.lines.last_mut()
        {
            l.anchors.append(&mut frames);
        }
        out
    }

    /// Paragraphs of a text frame laid out as a floating box anchored to the
    /// paragraph that follows them. Without a width the frame fits its text.
    fn frame(
        &self,
        blocks: &[Block],
        f: &crate::office::docx::props::Frame,
        width: f64,
    ) -> crate::office::docx::model::Anchored {
        let plain: Vec<Block> = blocks
            .iter()
            .map(|b| match b {
                Block::Para(p) => {
                    let mut p = p.clone();
                    Rc::make_mut(&mut p.style).frame = None;
                    Rc::make_mut(&mut p.style).resolved_frame = None;
                    Block::Para(p)
                }
                other => other.clone(),
            })
            .collect();
        let mut w = f.w.unwrap_or(width);
        let mut frame_units = self.units(&plain, w, false);
        let (mut slices, mut h) = stack(&frame_units);
        if f.w.is_none() {
            let right = slices
                .iter()
                .flat_map(|s| s.items.iter())
                .filter_map(|it| match it {
                    Item::Text(r) => Some(r.transform.tx as f64 + r.width),
                    _ => None,
                })
                .fold(0.0_f64, f64::max);
            if right > 1.0 && right < width {
                w = right.ceil() + 1.0;
                frame_units = self.units(&plain, w, false);
                (slices, h) = stack(&frame_units);
            }
        }
        // Final frame width governs visibility; rejected lines retain their advance.
        if self.discard_outside_frame_lines(&mut frame_units, w) {
            (slices, h) = stack(&frame_units);
        }
        let h = match f.h {
            Some(v) if f.h_exact => v,
            Some(v) => h.max(v),
            None => h,
        };
        let frame_children = if !self.env.in_header.get() || self.footer_frames {
            frame_child_anchors(&frame_units)
        } else {
            Vec::new()
        };
        let text_items = slices.into_iter().flat_map(|s| s.items).collect();
        let (h_rel, v_rel) = (
            match f.h_anchor.as_str() {
                "page" => crate::office::docx::model::HRel::Page,
                "margin" => crate::office::docx::model::HRel::Margin,
                _ => crate::office::docx::model::HRel::Column,
            },
            match f.v_anchor.as_str() {
                "page" => crate::office::docx::model::VRel::Page,
                "margin" => crate::office::docx::model::VRel::Margin,
                _ => crate::office::docx::model::VRel::Paragraph,
            },
        );
        let wrap = match f.wrap.as_str() {
            "notBeside" => crate::office::docx::model::Wrap::TopAndBottom,
            "none" => crate::office::docx::model::Wrap::None,
            _ => crate::office::docx::model::Wrap::Square,
        };
        crate::office::docx::model::Anchored {
            pic: crate::office::docx::model::Picture {
                part: None,
                w,
                h,
                crop: (0.0, 0.0, 0.0, 0.0),
                alt: None,
                shape: None,
                effects: Vec::new(),
                alpha: 1.0,
                graphic: None,
                shadow: None,
            },
            h_rel,
            x: f.x.clone(),
            v_rel,
            y: f.y.clone(),
            behind: false,
            z: 0,
            wrap,
            contour: None,
            dist: [f.v_space, f.v_space, f.h_space, f.h_space],
            wrap_side: "bothSides".into(),
            text_items,
            frame_children,
            text_frame: true,
            turned: false,
        }
    }

    /// Text-only frame lines paint as a whole when their horizontal bounds
    /// intersect the frame. Decorations and nested objects keep their policies.
    fn discard_outside_frame_lines(&self, units: &mut [Unit], width: f64) -> bool {
        let mut changed = false;
        for unit in units {
            let Unit::Para(para) = unit else { continue };
            if para.decor.is_some() {
                continue;
            }
            for line in &mut para.lines {
                if !line.anchors.is_empty() || !line.boxes.is_empty() || !line.notes.is_empty() {
                    continue;
                }
                if self.frame_line_outside(line, width) {
                    line.items.clear();
                    changed = true;
                }
            }
        }
        changed
    }

    fn frame_line_outside(&self, line: &Line, width: f64) -> bool {
        if !width.is_finite() || width <= 0.0 || crate::office::docx::text::paint_empty(&line.items)
        {
            return false;
        }
        let mut left = f64::INFINITY;
        let mut right = f64::NEG_INFINITY;
        for item in &line.items {
            if matches!(item, Item::Bookmark(_)) {
                continue;
            }
            let Item::Text(run) = item else { return false };
            let t = run.transform;
            if !run.drawn
                || run.glyphs.is_empty()
                || run.warp.is_some()
                || run.outline.is_some()
                || run.fake_bold
                || run.fake_italic
                || t.sx != 1.0
                || t.sy != 1.0
                || t.kx != 0.0
                || t.ky != 0.0
                || !t.tx.is_finite()
                || !t.ty.is_finite()
                || !run.width.is_finite()
                || run
                    .glyphs
                    .iter()
                    .any(|g| !g.x.is_finite() || !g.y.is_finite())
            {
                return false;
            }
            left = left.min(t.tx as f64);
            right = right.max(t.tx as f64 + run.width);
        }
        // Advance bounds offer a cheap conservative acceptance for ordinary lines.
        if left < width && right > 0.0 {
            return false;
        }
        let mut ink = false;
        for item in &line.items {
            let Item::Text(run) = item else { return false };
            let Some(face) = self.env.fonts.face(run.face) else {
                return false;
            };
            let scale = run.size / face.metrics.units_per_em as f64;
            if !scale.is_finite() || scale <= 0.0 {
                return false;
            }
            for glyph in &run.glyphs {
                let Some(path) = self.env.fonts.outline(run.face, glyph.id) else {
                    return false;
                };
                let bounds = path.bounds();
                let x = run.transform.tx as f64 + glyph.x;
                left = left.min(x + bounds.left() as f64 * scale);
                right = right.max(x + bounds.right() as f64 * scale);
                ink = true;
            }
        }
        ink && left.is_finite() && right.is_finite() && (right < -0.001 || left > width + 0.001)
    }

    /// Resolve the text box's source wrapping once for both sizing and paint.
    /// Explicit no-wrap widens the text layout to its longest line while keeping
    /// paragraph boundaries and hard breaks; the saved shape width stays intact.
    fn text_box_units(&self, tb: &crate::office::docx::shapes::TextBox) -> Vec<Unit> {
        let inner_w = (tb.rect.w - tb.inset[0] - tb.inset[2]).max(4.0);
        let lay = |width: f64| {
            crate::office::docx::text::with_auto_text(tb.font_color, || {
                crate::office::docx::text::with_background(tb.bg, || {
                    self.units(&tb.blocks, width, true)
                })
            })
        };
        let mut units = lay(inner_w);
        if tb.nowrap {
            let need = longest_line(&lay(1.0e5));
            if need > inner_w {
                units = lay(need + 0.5);
            }
        }
        units
    }

    /// Refit a single whole-box `spAutoFit` shape to its text and vertical
    /// insets, in points. Laid-out text can grow the shape; saved height is
    /// reduced only by at least one point. The returned geometry and height
    /// also define the owning anchor's body exclusion.
    fn refit(
        &self,
        spec: &crate::office::docx::shapes::ShapeSpec,
        w: f64,
        h: f64,
    ) -> Option<(crate::office::docx::shapes::ShapeSpec, f64)> {
        if h <= 0.0 {
            return None;
        }
        let [tb] = spec.texts.as_slice() else {
            return None;
        };
        let whole = tb.rect.x.abs() < 0.5
            && tb.rect.y.abs() < 0.5
            && (tb.rect.w - w).abs() < 0.5
            && (tb.rect.h - h).abs() < 0.5;
        if !tb.grow || !whole || tb.fit || tb.vert.is_some() || tb.transform.is_some() {
            return None;
        }
        let [_, it, _, ib] = tb.inset;
        let new_h = stack(&self.text_box_units(tb)).1 + it + ib;
        let grows = new_h > h + 0.001;
        if !grows && new_h > h - 1.0 {
            return None;
        }
        let mut out = spec.clone();
        let t = tiny_skia::Transform::from_scale(1.0, (new_h / h) as f32);
        for item in out.items.iter_mut() {
            transform_item(item, t);
        }
        out.texts[0].rect.h = new_h;
        Some((out, new_h))
    }

    /// Text box contents of a shape, laid out at each box's own width, in
    /// the shape's local points.
    fn text_boxes(&self, spec: &crate::office::docx::shapes::ShapeSpec) -> BoxContent {
        let mut out = BoxContent::default();
        for tb in &spec.texts {
            let upright;
            let tb = if tb.upright {
                upright = tb.upright_frame();
                &upright
            } else {
                tb
            };
            if tb.fit {
                out.extend(self.fitted_text(tb));
                continue;
            }
            let [il, it, ir, ib] = tb.inset;
            let inner_w = (tb.rect.w - il - ir).max(4.0);
            let inner_h = (tb.rect.h - it - ib).max(0.0);
            if let Some(v) = tb.vert.as_deref() {
                out.extend(self.vertical_box(tb, v, inner_w, inner_h));
                continue;
            }
            let units = self.text_box_units(tb);
            let (mut slices, h) = stack(&units);
            // Text taller than its box starts at the top whatever the anchor.
            let dy = match tb.v_anchor.as_str() {
                "ctr" => (inner_h - h) / 2.0,
                "b" => inner_h - h,
                _ => 0.0,
            }
            .max(0.0);
            // Lines starting below the bottom of a box that does not grow are hidden.
            if tb.clip {
                slices.retain(|sl| sl.y + dy < inner_h);
            }
            let t = tiny_skia::Transform::from_translate(
                (tb.rect.x + il) as f32,
                (tb.rect.y + it + dy) as f32,
            );
            let t = tb.transform.map_or(t, |r| t.post_concat(r));
            out.extend(box_content(slices, t, inner_w, false, |item| {
                translate_item(item, tb.rect.x + il, tb.rect.y + it + dy);
                if let Some(transform) = tb.transform {
                    transform_item(item, transform);
                }
            }));
        }
        out
    }

    /// A text box whose lines run down it: laid out across the box height,
    /// then turned 90° clockwise (`vert`, `eaVert`, `wordArtVert`...) or 270°
    /// (`vert270`). Anchoring moves the block across the box width.
    fn vertical_box(
        &self,
        tb: &crate::office::docx::shapes::TextBox,
        vert: &str,
        inner_w: f64,
        inner_h: f64,
    ) -> BoxContent {
        let [il, it, _, _] = tb.inset;
        let units = crate::office::docx::text::with_auto_text(tb.font_color, || {
            crate::office::docx::text::with_background(tb.bg, || {
                self.units(&tb.blocks, inner_h.max(4.0), true)
            })
        });
        let (slices, h) = stack(&units);
        let off = match tb.v_anchor.as_str() {
            "ctr" => (inner_w - h) / 2.0,
            "b" => inner_w - h,
            _ => 0.0,
        }
        .max(0.0);
        let (x0, y0) = ((tb.rect.x + il) as f32, (tb.rect.y + it) as f32);
        // Line-local (x along the flow, y across) to box space.
        let t = if vert == "vert270" {
            tiny_skia::Transform::from_row(
                0.0,
                -1.0,
                1.0,
                0.0,
                x0 + off as f32,
                y0 + inner_h as f32,
            )
        } else {
            tiny_skia::Transform::from_row(0.0, 1.0, -1.0, 0.0, x0 + (inner_w - off) as f32, y0)
        };
        let anchor_transform = tb.transform.map_or(t, |r| t.post_concat(r));
        box_content(
            slices,
            anchor_transform,
            inner_h.max(4.0),
            vert == "eaVert",
            |item| {
                transform_item(item, t);
                if let Some(rotation) = tb.transform {
                    transform_item(item, rotation);
                }
            },
        )
    }

    /// WordArt text scaled to fill its box: laid out on one line, then
    /// stretched from its ink extent (advance width, ascent + descent).
    fn fitted_text(&self, tb: &crate::office::docx::shapes::TextBox) -> BoxContent {
        let (slices, _) = stack(&self.units(&tb.blocks, 1.0e5, true));
        let items: Vec<&Item> = slices.iter().flat_map(|s| s.items.iter()).collect();
        let (mut x0, mut x1, mut y0, mut y1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
        for it in &items {
            if let Item::Text(r) = it {
                let (x, y) = (r.transform.tx as f64, r.transform.ty as f64);
                x0 = x0.min(x);
                x1 = x1.max(x + r.width);
                y0 = y0.min(y - r.ascent);
                y1 = y1.max(y + r.descent);
            }
        }
        if x1 <= x0 || y1 <= y0 {
            let t = tiny_skia::Transform::from_translate(tb.rect.x as f32, tb.rect.y as f32);
            let t = tb.transform.map_or(t, |r| t.post_concat(r));
            let mut content = box_content(slices, t, tb.rect.w, false, |_| {});
            content.items.clear();
            return content;
        }
        let t = tiny_skia::Transform::from_translate(-x0 as f32, -y0 as f32)
            .post_scale(
                (tb.rect.w / (x1 - x0)) as f32,
                (tb.rect.h / (y1 - y0)) as f32,
            )
            .post_translate(tb.rect.x as f32, tb.rect.y as f32);
        let t = tb.transform.map_or(t, |r| t.post_concat(r));
        box_content(slices, t, tb.rect.w, false, |item| transform_item(item, t))
    }

    /// Deferred inline text keeps its logical position after line remeasurement.
    pub(crate) fn resolve_inline_textboxes(&self, lines: &mut [Line]) {
        for l in lines {
            let mut inserted_text = 0;
            for textbox in std::mem::take(&mut l.boxes) {
                let mut content = self.text_boxes(&textbox.spec);
                let placement =
                    tiny_skia::Transform::from_translate(textbox.x as f32, textbox.top as f32);
                for child in &mut content.anchors {
                    if let Some((transform, _)) = &mut child.placement {
                        *transform = transform.post_concat(placement);
                    }
                }
                if !content.anchors.is_empty() {
                    l.anchors.push(box_anchor(content.anchors));
                }
                let mut items = content.items;
                for it in items.iter_mut() {
                    translate_item(it, textbox.x, textbox.top);
                }
                crate::office::docx::text::set_reading_key(&mut items, textbox.source_slot);
                let at = l
                    .items
                    .iter()
                    .enumerate()
                    .filter(|(_, item)| matches!(item, Item::Text(_)))
                    .nth(textbox.text_before + inserted_text)
                    .map_or(l.items.len(), |(index, _)| index);
                inserted_text += items.iter().filter(|i| matches!(i, Item::Text(_))).count();
                l.items.splice(at..at, items);
            }
        }
    }

    fn para(
        &self,
        p: &Paragraph,
        prev: Option<&Paragraph>,
        next: Option<&Paragraph>,
        width: f64,
        in_cell: bool,
        first_in_flow: bool,
    ) -> ParaUnit {
        crate::office::docx::deadline::check();
        let st = &p.style;
        self.env.in_cell.set(in_cell);
        let mut lines = crate::office::docx::text::with_background(st.shading, || {
            layout_paragraph(p, width, self.env)
        });
        // Hidden runs have already been removed. A hidden paragraph mark also
        // removes the empty paragraph's flow and decoration; keep bookmark
        // placements on its zero-height line and retain required cell marks.
        // An empty paragraph after one with a hidden mark joins that paragraph's
        // last line (Word for Mac: no line of its own, in the body or a cell).
        let joined = is_bare_para(p) && prev.is_some_and(|q| q.mark.hidden && !is_bare_para(q));
        if (!in_cell && hidden_paragraph(p)) || joined {
            for line in &mut lines {
                line.height = 0.0;
                line.baseline = 0.0;
                line.skip = 0.0;
                for item in &mut line.items {
                    if let Item::Bookmark(bookmark) = item {
                        bookmark.point.y = 0.0;
                    }
                }
            }
            return ParaUnit {
                lines,
                frame_anchors: Vec::new(),
                drop_cap_anchor: None,
                clip_inline_images: false,
                before: 0.0,
                after: 0.0,
                pad_top: 0.0,
                pad_bottom: 0.0,
                keep_next: false,
                keep_lines: false,
                widow: false,
                page_break_before: false,
                clear_after: false,
                column_break_before: false,
                decor: None,
                src: Some(Rc::new(p.clone())),
            };
        }
        self.resolve_inline_textboxes(&mut lines);
        // Text boxes of shapes anchored here are laid out at their own width.
        for l in lines.iter_mut() {
            for a in l.anchors.iter_mut() {
                let Some(spec) = a.pic.shape.clone() else {
                    continue;
                };
                let spec = match self.refit(&spec, a.pic.w, a.pic.h) {
                    Some((spec, h)) => {
                        a.pic.h = h;
                        let spec = Rc::new(spec);
                        a.pic.shape = Some(spec.clone());
                        spec
                    }
                    None => spec,
                };
                let content = self.text_boxes(&spec);
                a.text_items.extend(content.items);
                a.frame_children.extend(content.anchors);
            }
        }
        self.env.in_cell.set(in_cell);
        // A page break that ends a paragraph keeps the paragraph mark on its line, so
        // no empty line opens the next page. After a column break the mark's empty
        // line stays at the top of the next column (Word, modes 11 and 15).
        // In Word 2003 layout (mode 11) a paragraph holding only a page break carries
        // its mark's empty line to the top of the next page.
        let n = lines.len();
        let break_only = n == 2 && crate::office::docx::text::paint_empty(&lines[0].items);
        if n >= 2
            && lines[n - 2].break_after == Some(crate::office::docx::model::BreakKind::Page)
            && crate::office::docx::text::paint_empty(&lines[n - 1].items)
            && lines[n - 1].anchors.is_empty()
            && !(break_only && self.compat_mode <= 11)
        {
            crate::office::docx::text::pop_empty_terminal_line(&mut lines);
        }
        // A paragraph that opens with a column break (paginate decides whether the
        // break reaches another column on the page).
        let column_break_before = lines.len() >= 2
            && lines[0].break_after == Some(crate::office::docx::model::BreakKind::Column)
            && crate::office::docx::text::paint_empty(&lines[0].items)
            && lines[0].anchors.is_empty()
            && !crate::office::docx::text::paint_empty(&lines[1].items);
        let same = |o: Option<&Paragraph>| o.is_some_and(|o| o.style.style_id == st.style_id);
        // With `doNotUseHTMLParagraphAutoSpacing`, Word ignores the automatic spacing
        // flags and adds the written before and after values (list items 5 + 5 pt
        // apart in Word's PDF, where automatic spacing would give 14 or none).
        let html = self.env.html_spacing;
        let before_auto = st.before_auto && html;
        let after_auto = st.after_auto && html;
        let auto = |pv: &Paragraph| pv.style.after_auto && html;
        let mut before = if before_auto {
            // Auto spacing is dropped at the top of a cell (unconfirmed) and of a
            // header or footer (a header's first line sits 14 pt higher in Word).
            if (in_cell || self.env.in_header.get()) && first_in_flow {
                0.0
            } else {
                AUTO_SPACING
            }
        } else {
            st.before
        };
        let mut after = if after_auto { AUTO_SPACING } else { st.after };
        // HTML lists: auto spacing is dropped between items of a list (unconfirmed).
        let listed = |o: Option<&Paragraph>| o.is_some_and(|o| o.num.is_some());
        if p.num.is_some() {
            if before_auto && listed(prev) {
                before = 0.0;
            }
            if after_auto && listed(next) {
                after = 0.0;
            }
        }
        if st.contextual && same(prev) {
            before = 0.0;
        }
        if st.contextual && same(next) {
            after = 0.0;
        }
        // HTML auto spacing collapses like CSS margins: the gap is the larger of
        // the previous paragraph's space after and this one's space before.
        if let Some(pv) = prev {
            // Without `doNotUseHTMLParagraphAutoSpacing`, Word collapses paragraph
            // spacing HTML-style: the gap is the larger of the space after and the
            // space before (8 of 9 page-1 thumbnails; page starts land exactly).
            let auto_pair = before_auto || auto(pv);
            if auto_pair || self.env.html_spacing {
                let pv_after = if pv.style.contextual && pv.style.style_id == st.style_id {
                    0.0
                } else if auto(pv) {
                    // The item's automatic space after is already dropped before another item.
                    if pv.num.is_some() && p.num.is_some() {
                        0.0
                    } else {
                        AUTO_SPACING
                    }
                } else {
                    pv.style.after
                };
                before = (before - pv_after).max(0.0);
            }
        }
        // Consecutive paragraphs with the same borders share one box.
        let b = &st.borders;
        let sides = [&b.top, &b.bottom, &b.left, &b.right, &b.between];
        let has_box = sides
            .iter()
            .any(|x| x.as_ref().is_some_and(|x| x.visible()))
            || st.shading.is_some();
        let same_box = |o: Option<&Paragraph>| {
            o.is_some_and(|o| {
                o.style.borders == st.borders
                    && o.style.left == st.left
                    && o.style.right == st.right
            })
        };
        // A `none` border draws nothing but keeps its spacing, like a zero-width
        // border: runs of paragraphs with the same borders share one box.
        let spaced = |x: &Option<Border>| {
            x.clone()
                .filter(|x| x.visible() || (x.style == "none" && x.space > 0.0))
        };
        let spaced_box = sides.iter().any(|x| spaced(x).is_some());
        let joined_prev = (has_box || spaced_box) && same_box(prev);
        let joined_next = (has_box || spaced_box) && same_box(next);
        let vis = |x: &Option<Border>| x.clone().filter(|x| x.visible());
        let edge = |x: &Option<Border>, visible: bool| if visible { vis(x) } else { spaced(x) };
        let top_of = |visible: bool| {
            if joined_prev {
                edge(&b.between, visible)
            } else {
                edge(&b.top, visible)
            }
        };
        let bottom_of = |visible: bool| {
            if joined_next {
                None
            } else {
                edge(&b.bottom, visible)
            }
        };
        let (top, bottom) = (top_of(true), bottom_of(true));
        // A between border keeps its spacing above and below the shared edge.
        let pad_top = top_of(false).as_ref().map_or(0.0, |t| {
            t.width + t.space * if joined_prev { 2.0 } else { 1.0 }
        });
        let pad_bottom = bottom_of(false).as_ref().map_or(0.0, |t| t.width + t.space);
        let decor = has_box.then(|| {
            let lpad = vis(&b.left).map_or(0.0, |l| l.space + l.width);
            let rpad = vis(&b.right).map_or(0.0, |r| r.space + r.width);
            ParaBox {
                x0: st.left.min(st.left + st.first) - lpad,
                x1: width - st.right + rpad,
                top,
                bottom,
                left: vis(&b.left),
                right: vis(&b.right),
                shading: st.shading,
                grow: (
                    if joined_prev { before } else { 0.0 },
                    if joined_next { after } else { 0.0 },
                ),
            }
        });
        // A page break opening a paragraph that already breaks the page before it
        // starts no second page (a Heading 1 with page break before and a leading
        // page break run, in Word's PDF): its empty first line goes.
        if st.page_break_before
            && lines.len() > 1
            && crate::office::docx::text::paint_empty(&lines[0].items)
            && lines[0].anchors.is_empty()
            && matches!(
                lines[0].break_after,
                Some(crate::office::docx::model::BreakKind::Page)
            )
        {
            lines.remove(0);
        }
        ParaUnit {
            lines,
            frame_anchors: Vec::new(),
            drop_cap_anchor: None,
            clip_inline_images: false,
            before,
            after,
            pad_top,
            pad_bottom,
            keep_next: st.keep_next,
            keep_lines: st.keep_lines,
            widow: st.widow,
            page_break_before: st.page_break_before,
            clear_after: st.clear_break,
            column_break_before,
            decor,
            src: Some(Rc::new(p.clone())),
        }
    }

    fn table(&self, t: &Table, width: f64, body: bool, out: &mut Vec<Unit>) {
        let id = self.next_table.get();
        self.next_table.set(id + 1);
        let mut grid = table_width::initial_grid(t, width, self.compat_mode);
        table_width::autofit(t, &mut grid, width, self.env);
        let mut xs = vec![0.0];
        for g in &grid {
            xs.push(xs.last().unwrap() + g);
        }
        let total = *xs.last().unwrap();
        // Before Word 2013 (compatibility mode 15) the table edge sits one cell
        // margin left of the indent, so cell text lines up with body text.
        let first_margin = t
            .rows
            .first()
            .and_then(|r| r.cells.first())
            .map(|c| c.margins[1])
            .unwrap_or(5.4);
        let shift = if self.compat_mode < 15 {
            first_margin
        } else {
            0.0
        };
        let x0 = match &t.float {
            // Body numeric anchors retain their source coordinate until pagination.
            Some(f) => match &f.x {
                Ok(x) => {
                    *x - if (!body || self.note_story) && f.h_anchor == "page" {
                        72.0
                    } else {
                        0.0
                    }
                }
                Err(spec) => match spec.as_str() {
                    "center" => (width - total) / 2.0,
                    "right" | "outside" => width - total,
                    _ => 0.0,
                },
            },
            // A centered table wider than the column overhangs both margins equally;
            // a wider right-aligned one keeps its left indent and runs past the right
            // margin (Word for Mac PDFs of pasted web forms).
            _ => match t.jc.as_str() {
                "center" => (width - total) / 2.0,
                "right" | "end" if total <= width => width - total,
                _ => t.indent - shift,
            },
        };
        let nrows = t.rows.len();
        // Pass 1: lay out cells and natural row heights.
        struct Pending {
            cell: CellBox,
            col: usize,
            span: usize,
            vmerge: VMerge,
        }
        let source_table = self.note_story.then(|| Rc::new(t.clone()));
        let mut minimum_heights = Vec::new();
        let mut additions = Vec::new();
        // Each row's content height without its authored minimum, points.
        let mut natural_heights = Vec::new();
        // Top border of a first row that grows with its content, points.
        let mut first_top = 0.0;
        let mut rows: Vec<(Vec<Pending>, f64)> = Vec::new();
        let mut prev_bottom = 0.0_f64;
        for (ri, row) in t.rows.iter().enumerate() {
            let top_margin = row.cells.iter().map(|c| c.margins[0]).fold(0.0, f64::max);
            let bottom_margin = row.cells.iter().map(|c| c.margins[2]).fold(0.0, f64::max);
            let mut col = row.grid_before;
            let mut cells = Vec::new();
            let mut h: f64 = 0.0;
            let ncells = row.cells.len();
            for (ci, c) in row.cells.iter().enumerate() {
                let mut margins = c.margins;
                // Automatic rows share their largest vertical cell margins.
                // Store the resolved pair so sizing, alignment and fragment
                // capacity account for the same padding exactly once.
                if row.height_rule != HeightRule::Exact {
                    margins[0] = top_margin;
                    margins[2] = bottom_margin;
                }
                let span = c.span.min(grid.len().saturating_sub(col)).max(1);
                let cx = xs[col.min(grid.len())];
                let cw = xs[(col + span).min(grid.len())] - cx;
                // Cell spacing insets the cell from its grid column on both sides
                // (Word wraps a 44% table's line that fits by under 1.5 pt).
                let inner =
                    (cw - c.margins[1] - c.margins[3] - 2.0 * t.cell_spacing.max(0.0)).max(1.0);
                let merged = c.vmerge == VMerge::Continue;
                // Vertical text is laid out once the row heights are known.
                let (slices, content_h, terminal_after) = if merged || c.text_dir.is_some() {
                    (Vec::new(), 0.0, 0.0)
                } else {
                    // `w:hideMark` suppresses the cell's closing empty paragraph, after
                    // other content too (Word for Mac: a title line and an empty one take
                    // one line); a paragraph with a space still contributes its height.
                    let hidden = c.hide_mark
                        && matches!(c.blocks.last(), Some(Block::Para(p)) if is_bare_para(p));
                    let blocks = if hidden {
                        &c.blocks[..c.blocks.len() - 1]
                    } else {
                        &c.blocks[..]
                    };
                    let mut units = crate::office::docx::text::with_background(c.shading, || {
                        self.units(blocks, inner, true)
                    });
                    // With `w:hideMark`, a line break that ends the cell's last paragraph
                    // opens no line: only the end-of-cell mark would sit on it.
                    if c.hide_mark
                        && matches!(blocks.last(), Some(Block::Para(p)) if ends_with_line_break(p))
                        && let Some(Unit::Para(p)) = units.last_mut()
                        && p.lines.len() >= 2
                        && p.lines.last().is_some_and(|l| {
                            crate::office::docx::text::paint_empty(&l.items) && l.anchors.is_empty()
                        })
                    {
                        crate::office::docx::text::pop_empty_terminal_line(&mut p.lines);
                    }
                    // In a table cell Word measures margin-relative positions from the
                    // cell's text and page-relative ones from its edge (Word for Mac probe).
                    for unit in &mut units {
                        if let Unit::Para(p) = unit {
                            for a in p.lines.iter_mut().flat_map(|l| l.anchors.iter_mut()) {
                                match a.h_rel {
                                    crate::office::docx::model::HRel::Margin => {
                                        a.h_rel = crate::office::docx::model::HRel::Column;
                                    }
                                    // Page offsets count from the cell's outer edge.
                                    crate::office::docx::model::HRel::Page if a.x.is_ok() => {
                                        a.h_rel = crate::office::docx::model::HRel::Column;
                                        a.x = a.x.clone().map(|v| v - c.margins[1]);
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                    if table_width::clips_inline_images(t, ri, self.env) {
                        for unit in &mut units {
                            if let Unit::Para(p) = unit {
                                p.clip_inline_images = true;
                            }
                        }
                    }
                    let (mut sl, h) = if self.note_story {
                        cells::note_stack(&units, inner, self.env)
                    } else {
                        cells::stack(&units, inner, self.env)
                    };
                    if hidden && let Some(Block::Para(p)) = c.blocks.last() {
                        let items = paragraph_markers(p)
                            .into_iter()
                            .map(|source| {
                                Item::Bookmark(crate::office::pptx::scene::BookmarkItem {
                                    source,
                                    point: tiny_skia::Point::from_xy(0.0, 0.0),
                                    reading_key: 0,
                                })
                            })
                            .collect::<Vec<_>>();
                        if !items.is_empty() {
                            sl.push(Slice {
                                y: 0.0,
                                h: 0.0,
                                items,
                                anchors: Vec::new(),
                                paragraph_offset: 0.0,
                                widow: None,
                                notes: Vec::new(),
                                nested_row: None,
                                nested_table: None,
                                paragraph_source: None,
                            });
                        }
                    }
                    if table_width::clips_inline_images(t, ri, self.env) {
                        cells::clip_inline_images(&mut sl, inner);
                    }
                    // Ordinary cells keep terminal spacing with the final line.
                    // Merged and rotated cells use their separate fragment allocation.
                    let after = if c.vmerge == VMerge::None {
                        match units.last() {
                            Some(Unit::Para(p)) if !p.lines.is_empty() => p.after.max(0.0),
                            _ => 0.0,
                        }
                    } else {
                        0.0
                    };
                    // A wrapped floating object laid out in the cell grows it to the
                    // object's bottom (margin and page offsets count from the cell top).
                    let float_bottom = sl
                        .iter()
                        .flat_map(|s| s.anchors.iter())
                        .filter(|(a, ..)| {
                            !a.behind && a.wrap != crate::office::docx::model::Wrap::None
                        })
                        .filter_map(|(a, _, ay)| {
                            let off = a.y.clone().ok();
                            let top = match a.v_rel {
                                crate::office::docx::model::VRel::Paragraph
                                | crate::office::docx::model::VRel::Line => ay + off?,
                                crate::office::docx::model::VRel::Margin
                                | crate::office::docx::model::VRel::Page => off.unwrap_or(0.0),
                            };
                            Some(top + a.pic.h)
                        })
                        .fold(0.0_f64, f64::max);
                    (sl, h.max(float_bottom), after)
                };
                let borders = cell_borders(c, t, ri, nrows, ci, ncells);
                if c.vmerge == VMerge::None {
                    h = h.max(content_h + margins[0] + margins[2]);
                }
                if c.vmerge == VMerge::Restart && content_h <= 0.0 {
                    h = h.max(margins[0] + margins[2]);
                }
                cells.push(Pending {
                    cell: CellBox {
                        // Right-to-left tables mirror the grid within the table width.
                        x: x0 + if t.bidi_visual { total - cx - cw } else { cx },
                        w: cw,
                        slices,
                        content_h,
                        terminal_after,
                        margins,
                        shading: c.shading,
                        borders,
                        v_align: c.v_align.clone(),
                        merged,
                        merge_continues: false,
                        merged_from: 0,
                        diagonals: if c.vmerge == VMerge::None {
                            c.diagonals.clone()
                        } else {
                            [None, None]
                        },
                        moved: None,
                    },
                    col,
                    span,
                    vmerge: c.vmerge,
                });
                col += span;
            }
            if cells.is_empty() {
                h = h.max(1.0);
            }
            // An empty row still has its end-of-row mark's line height; a row of
            // merged cells takes only its minimum height (Word for Mac: 8.7 pt for an
            // 8.05 pt "at least" row under a merge).
            if h <= 0.0 && cells.iter().all(|c| c.vmerge == VMerge::None) {
                h = 12.0;
            }
            // Horizontal borders contribute half their width above the natural
            // row minimum; shared edges use the larger adjoining border.
            let w = |k: usize| {
                cells
                    .iter()
                    .filter_map(|c| c.cell.borders[k].as_ref().map(|b| b.width))
                    .fold(0.0_f64, f64::max)
            };
            let top = w(0).max(prev_bottom);
            prev_bottom = w(2);
            if ri == 0 && row.height_rule != HeightRule::Exact {
                first_top = top;
            }
            // Spaced cells keep their own full borders; touching cells share a line.
            let border = if t.cell_spacing > 0.0 {
                w(0) + w(2)
            } else {
                (top + w(2)) / 2.0
            };
            // Word draws twice `w:tblCellSpacing` between rows and at the table's top
            // and bottom (probe: 5 pt spacing adds 20 pt to one row, 40 pt to three).
            let gap = 2.0 * t.cell_spacing.max(0.0);
            let sp_total = if gap > 0.0 {
                gap + if ri == 0 { gap } else { 0.0 }
            } else {
                0.0
            };
            natural_heights.push(h + border + sp_total);
            let mut h = match row.height_rule {
                HeightRule::Exact if row.height > 0.0 => {
                    // Word adds the largest bottom cell padding to an exact row
                    // height (MS-OI29500 17.4.80); validate separately from auto rows.
                    row.height + bottom_margin
                }
                // An "at least" height leaves out the cell padding (Word for Mac: 100 tw
                // top and bottom padding add 10 pt to a 17 pt row holding one line).
                HeightRule::AtLeast => h.max(row.height + top_margin + bottom_margin),
                HeightRule::Auto => h.max(row.height),
                HeightRule::Exact => h,
            };
            if row.height_rule != HeightRule::Exact {
                h += border;
            }
            let sp = gap;
            if sp > 0.0 {
                h += sp + if ri == 0 { sp } else { 0.0 };
            }
            additions.push(
                border
                    + if sp > 0.0 {
                        sp + if ri == 0 { sp } else { 0.0 }
                    } else {
                        0.0
                    },
            );
            let padding = if row.height_rule == HeightRule::AtLeast && row.height > 0.0 {
                top_margin + bottom_margin
            } else {
                0.0
            };
            minimum_heights.push(
                row.height.max(0.0)
                    + padding
                    + border
                    + if sp > 0.0 {
                        sp + if ri == 0 { sp } else { 0.0 }
                    } else {
                        0.0
                    },
            );
            rows.push((cells, h));
        }
        // The last row's bottom border likewise has no row below to share it.
        let last_bottom = match t.rows.last() {
            Some(r) if r.height_rule != HeightRule::Exact => prev_bottom,
            _ => 0.0,
        };
        let spaced = t.cell_spacing > 0.0;
        let table_edge = |b: &Option<crate::office::docx::props::Border>| {
            b.as_ref().map_or(0.0, |b| b.width.max(0.0))
        };
        let merge_base_heights: Vec<_> = rows.iter().map(|r| r.1).collect();
        // Pass 2: a vertically merged cell's content stretches the last row it spans.
        let row_provenance: Vec<_> = rows
            .iter()
            .map(|(cells, _)| {
                cells
                    .iter()
                    .flat_map(|cell| &cell.cell.slices)
                    .flat_map(|slice| &slice.items)
                    .find_map(|item| match item {
                        Item::Text(run) => run.source.clone().filter(|source| {
                            source.cells.first().is_some_and(|cell| cell.text_only)
                        }),
                        _ => None,
                    })
            })
            .collect();
        for ri in 0..rows.len() {
            for k in 0..rows[ri].0.len() {
                if rows[ri].0[k].vmerge != VMerge::Restart {
                    continue;
                }
                let col = rows[ri].0[k].col;
                let mut last = ri;
                while last + 1 < rows.len()
                    && rows[last + 1]
                        .0
                        .iter()
                        .any(|c| c.col == col && c.vmerge == VMerge::Continue)
                {
                    last += 1;
                }
                let c = &rows[ri].0[k].cell;
                let need = c.content_h + c.margins[0] + c.margins[2];
                let have: f64 = rows[ri..=last].iter().map(|r| r.1).sum();
                if need > have + 0.01 && t.rows[last].height_rule != HeightRule::Exact {
                    rows[last].1 += need - have;
                }
            }
        }
        let header_count = t.rows.iter().take_while(|r| r.header).count();
        // Pass 2b: a merged cell aligns its content in the height of all its rows.
        for ri in 0..rows.len() {
            for k in 0..rows[ri].0.len() {
                let p = &rows[ri].0[k];
                // Top-aligned text can continue through the same physical fragments
                // as aligned merges. Anchors and notes retain their placement owner.
                let top_flow = (body || self.note_story)
                    && !matches!(p.cell.v_align.as_str(), "center" | "bottom")
                    && (self.note_story
                        || p.cell
                            .slices
                            .iter()
                            .all(|s| s.anchors.is_empty() && s.notes.is_empty()))
                    && t.rows[ri].cells[k].text_dir.is_none();
                if p.vmerge != VMerge::Restart
                    || !(matches!(p.cell.v_align.as_str(), "center" | "bottom") || top_flow)
                {
                    continue;
                }
                let col = p.col;
                let span = p.span;
                let mut last = ri;
                while last + 1 < rows.len()
                    && rows[last + 1]
                        .0
                        .iter()
                        .any(|c| c.col == col && c.span == span && c.vmerge == VMerge::Continue)
                {
                    last += 1;
                }
                if last == ri || (top_flow && ri < header_count && last >= header_count) {
                    continue;
                }
                let span_h: f64 = rows[ri..=last].iter().map(|r| r.1).sum();
                let Some(tk) = rows[last]
                    .0
                    .iter()
                    .position(|c| c.col == col && c.span == span)
                else {
                    continue;
                };
                // Drawn from the last spanned row, after the shading of the rows above.
                let above: f64 = rows[ri..last].iter().map(|r| r.1).sum();
                let c = &mut rows[ri].0[k].cell;
                let free = (span_h - c.margins[0] - c.margins[2] - c.content_h).max(0.0);
                let dy = if c.v_align == "center" {
                    free / 2.0
                } else if c.v_align == "bottom" {
                    free
                } else {
                    0.0
                };
                let slices = std::mem::take(&mut c.slices);
                let margins = c.margins;
                let cb = &mut rows[last].0[tk].cell;
                cb.margins = margins;
                cb.v_align = "top".into();
                cb.merged_from = last - ri;
                cb.moved = Some(Rc::new((slices, dy - above)));
            }
        }
        // Pass 3: vertical text runs along the height of the rows it spans.
        for ri in 0..rows.len() {
            for k in 0..rows[ri].0.len() {
                let ci = rows[ri].0.iter().take(k).count();
                let Some(cell) = t.rows[ri].cells.get(ci) else {
                    continue;
                };
                let Some(dir) = cell.text_dir.clone() else {
                    continue;
                };
                if cell.vmerge == VMerge::Continue {
                    continue;
                }
                let col = rows[ri].0[k].col;
                let mut last = ri;
                while cell.vmerge == VMerge::Restart
                    && last + 1 < rows.len()
                    && rows[last + 1]
                        .0
                        .iter()
                        .any(|c| c.col == col && c.vmerge == VMerge::Continue)
                {
                    last += 1;
                }
                let span_h: f64 = rows[ri..=last].iter().map(|r| r.1).sum();
                let c = &rows[ri].0[k].cell;
                let owner_margins = c.margins;
                let len = (span_h - c.margins[0] - c.margins[2]).max(4.0);
                let inner_w = (c.w - c.margins[1] - c.margins[3]).max(1.0);
                let (slices, h) = stack(&self.units(&cell.blocks, len, true));
                // btLr reads bottom to top; tbRl top to bottom, lines right to left.
                let tr = if dir == "btLr" || dir == "lr" {
                    let x0 = match c.v_align.as_str() {
                        "center" => (inner_w - h) / 2.0,
                        "bottom" => inner_w - h,
                        _ => 0.0,
                    };
                    tiny_skia::Transform::from_row(0.0, -1.0, 1.0, 0.0, x0 as f32, len as f32)
                } else {
                    tiny_skia::Transform::from_row(0.0, 1.0, -1.0, 0.0, inner_w as f32, 0.0)
                };
                let mut items = Vec::new();
                for sl in slices {
                    for mut it in sl.items {
                        transform_item(&mut it, tr);
                        items.push(it);
                    }
                }
                // Drawn from the last spanned row, after the shading of the rows above.
                let above: f64 = rows[ri..last].iter().map(|r| r.1).sum();
                for it in items.iter_mut() {
                    translate_item(it, 0.0, -above);
                }
                let target = if last > ri {
                    rows[last].0.iter().position(|c| c.col == col)
                } else {
                    Some(k)
                };
                let Some(tk) = target else { continue };
                let cb = &mut rows[last].0[tk].cell;
                // Rotated content retains the margins used to measure its owner.
                cb.margins = owner_margins;
                cb.v_align = "top".into();
                cb.content_h = 0.0;
                cb.merged_from = last - ri;
                cb.slices = vec![Slice {
                    y: 0.0,
                    h: 0.0,
                    items,
                    anchors: Vec::new(),
                    paragraph_offset: 0.0,
                    widow: None,
                    notes: Vec::new(),
                    nested_row: None,
                    nested_table: None,
                    paragraph_source: None,
                }];
            }
        }
        {
            // Mark logical interiors while retaining original fragment-edge borders.
            // Matching both grid start and span excludes malformed merge chains.
            for ri in 0..rows.len() {
                for k in 0..rows[ri].0.len() {
                    let start = &rows[ri].0[k];
                    if start.vmerge != VMerge::Restart {
                        continue;
                    }
                    let (col, span) = (start.col, start.span);
                    let mut previous = (ri, k);
                    for next in (ri + 1)..rows.len() {
                        let Some(ki) = rows[next].0.iter().position(|c| {
                            c.col == col && c.span == span && c.vmerge == VMerge::Continue
                        }) else {
                            break;
                        };
                        rows[previous.0].0[previous.1].cell.merge_continues = true;
                        previous = (next, ki);
                    }
                }
            }
        }
        let mut units: Vec<RowUnit> = rows
            .into_iter()
            .zip(&t.rows)
            .enumerate()
            .map(|(index, ((cells, h), row))| RowUnit {
                bookmarks_before: if index == 0 {
                    t.bookmarks_before.clone()
                } else {
                    Vec::new()
                },
                bookmarks_after: if index + 1 == t.rows.len() {
                    t.bookmarks_after.clone()
                } else {
                    Vec::new()
                },
                provenance: row_provenance[index].clone(),
                source: source_table.as_ref().map(|table| {
                    Rc::new(RowSource {
                        table: table.clone(),
                        width,
                        minimum: minimum_heights[index],
                        addition: additions[index],
                    })
                }),
                index,
                height: h,
                min_height: if row.height > 0.0 && row.height_rule == HeightRule::AtLeast {
                    minimum_heights[index]
                } else {
                    0.0
                },
                natural_height: natural_heights[index],
                after_keep: index > 0 && row_keep(&t.rows[index - 1]),
                natural_addition: additions[index],
                merge_base_height: merge_base_heights[index],
                // The table's top border has no row above to share it: the half the
                // row height leaves out sits above the first row and moves its text.
                // With cell spacing the table's own border lies outside the cells.
                outer_before: match index {
                    0 if spaced => table_edge(&t.borders.top),
                    0 => first_top / 2.0,
                    _ => 0.0,
                },
                outer_after: if index + 1 < t.rows.len() {
                    0.0
                } else if spaced {
                    table_edge(&t.borders.bottom)
                } else {
                    last_bottom / 2.0
                },
                cells: cells
                    .into_iter()
                    .map(|p| {
                        let _ = (p.span, p.col);
                        p.cell
                    })
                    .collect(),
                page_break_before: body && t.float.is_none() && row_page_break(row),
                cant_split: row.cant_split,
                exact_height: row.height_rule == HeightRule::Exact,
                header: row.header,
                repeating_header: index < header_count && header_count < t.rows.len(),
                table: id,
                headers: Rc::new(Vec::new()),
                float_y: None,
                float_x: t
                    .float
                    .as_ref()
                    .filter(|f| body && !self.note_story && f.x.is_ok())
                    .map(|f| f.h_anchor.clone()),
                float_clear: false,
                keep_next: row_keep(row),
            })
            .collect();
        if let (Some(f), Some(first)) = (&t.float, units.first_mut()) {
            {
                first.float_y = Some((f.v_anchor.clone(), f.y.clone()));
                first.float_clear = f.no_overlap;
            }
        }
        if header_count > 0 && header_count < units.len() {
            let headers = Rc::new(units[..header_count].to_vec());
            for u in units.iter_mut().skip(header_count) {
                u.headers = headers.clone();
            }
        }
        out.extend(units.into_iter().map(Unit::Row));
    }
}

/// Preserve the leading paragraph's page boundary before cell stacking flattens it.
/// Other cells and interior paragraphs need separate within-row break semantics.
fn row_page_break(row: &crate::office::docx::model::Row) -> bool {
    row.cells.first().is_some_and(|cell| {
        cell.vmerge != VMerge::Continue
            && matches!(cell.blocks.first(), Some(Block::Para(p)) if p.style.page_break_before)
    })
}

/// A row keeps with the next when every paragraph of every cell has
/// keep-with-next (A/B: 4 documents better, 1 worse; "any paragraph" lost).
fn row_keep(row: &crate::office::docx::model::Row) -> bool {
    let keeps = |c: &Cell| {
        let mut paras = c.blocks.iter().filter_map(|b| match b {
            Block::Para(p) => Some(p.style.keep_next),
            _ => None,
        });
        paras.clone().next().is_some() && paras.all(|k| k)
    };
    !row.cells.is_empty() && row.cells.iter().all(keeps)
}

fn ordinary_frame_anchor(b: &Block) -> bool {
    matches!(b, Block::Para(p) if p.style.frame.is_none() && p.style.resolved_frame.is_none())
}

// Text-relative footer frames use the next normal paragraph as their origin.
// Page/margin-relative frames retain their existing placement pending controls.
fn footer_frame_of(b: &Block) -> Option<&crate::office::docx::props::Frame> {
    let Block::Para(p) = b else { return None };
    p.style
        .resolved_frame
        .as_ref()
        .or(p.style.frame.as_ref())
        .filter(|f| f.drop_cap.is_none() && f.v_anchor == "text" && f.y.is_ok())
}

/// Body frames anchored to the text; an aligned `yAlign` sits at the paragraph top.
fn body_text_frame_of(b: &Block) -> Option<&crate::office::docx::props::Frame> {
    let Block::Para(p) = b else { return None };
    p.style
        .resolved_frame
        .as_ref()
        .or(p.style.frame.as_ref())
        .filter(|f| f.drop_cap.is_none() && f.v_anchor == "text")
}

/// Body frames with explicit numeric page-relative vertical offsets.
fn page_frame_of(b: &Block) -> Option<&crate::office::docx::props::Frame> {
    let Block::Para(p) = b else {
        return None;
    };
    p.style
        .resolved_frame
        .as_ref()
        .or(p.style.frame.as_ref())
        .filter(|f| {
            f.drop_cap.is_none()
                && f.v_anchor == "page"
                && f.y.is_ok()
                && f.attrs.iter().any(|(k, _)| k == "w:y" || k == "y")
        })
}

/// An in-flow frame paragraph whose frame sizes to its text and aligns
/// right or centred on
/// the margin: the text takes that alignment (page-number frames).
fn frame_aligned(p: &Paragraph) -> Option<Paragraph> {
    let f = p.style.frame.as_ref()?;
    if f.w.is_some() || f.drop_cap.is_some() || !matches!(f.h_anchor.as_str(), "margin" | "text") {
        return None;
    }
    let jc = match f.x.as_ref().err()?.as_str() {
        "right" | "outside" => crate::office::docx::styles::Jc::Right,
        "center" => crate::office::docx::styles::Jc::Center,
        _ => return None,
    };
    let mut q = p.clone();
    Rc::make_mut(&mut q.style).jc = jc;
    Some(q)
}

fn hidden_paragraph(p: &Paragraph) -> bool {
    p.mark.hidden && is_bare_para(p)
}

fn is_empty_para(p: &Paragraph) -> bool {
    p.num.is_none()
        && p.inlines.iter().all(|i| match i {
            crate::office::docx::model::Inline::Bookmark(_) => true,
            crate::office::docx::model::Inline::Text(t, _) => t.trim().is_empty(),
            _ => false,
        })
}

/// Whether the paragraph's last content (after trailing spaces) is a line break.
fn ends_with_line_break(p: &Paragraph) -> bool {
    matches!(
        p.inlines
            .iter()
            .rev()
            .find(|i| !matches!(i, Inline::Text(t, _) if t.trim().is_empty())),
        Some(Inline::Break(
            crate::office::docx::model::BreakKind::Line,
            _
        ))
    )
}

/// A paragraph with no characters at all; a space still takes its line.
fn is_bare_para(p: &Paragraph) -> bool {
    p.num.is_none()
        && p.inlines.iter().all(|i| match i {
            crate::office::docx::model::Inline::Bookmark(_) => true,
            crate::office::docx::model::Inline::Text(t, _) => t.is_empty(),
            _ => false,
        })
}

fn para_of(b: &Block) -> Option<&Paragraph> {
    match b {
        Block::Para(p) => Some(p),
        _ => None,
    }
}

/// The borders a cell draws: its own, else the table's outer or inside ones.
fn cell_borders(
    c: &Cell,
    t: &Table,
    ri: usize,
    nrows: usize,
    ci: usize,
    ncells: usize,
) -> [Option<Border>; 4] {
    let merged;
    let tb = match t.rows.get(ri).and_then(|r| r.borders.as_ref()) {
        Some(b) => {
            let mut m = t.borders.clone();
            m.apply(b);
            merged = m;
            &merged
        }
        None => &t.borders,
    };
    let pick =
        |own: &Option<Border>, outer: &Option<Border>, inner: &Option<Border>, edge: bool| {
            own.clone()
                .or_else(|| if edge { outer.clone() } else { inner.clone() })
                .filter(|b| b.visible() && b.width > 0.0)
        };
    [
        pick(&c.borders.top, &tb.top, &tb.inside_h, ri == 0),
        pick(
            &c.borders.left,
            &tb.left,
            &tb.inside_v,
            if t.bidi_visual {
                ci + 1 == ncells
            } else {
                ci == 0
            },
        ),
        pick(&c.borders.bottom, &tb.bottom, &tb.inside_h, ri + 1 == nrows),
        pick(
            &c.borders.right,
            &tb.right,
            &tb.inside_v,
            if t.bidi_visual {
                ci == 0
            } else {
                ci + 1 == ncells
            },
        ),
    ]
}

/// Floating table positioning (`w:tblpPr`): tables sit at their offset from the
/// page, margin or text (page-1 thumbnails: two text-anchored tables move 39 and
/// East Asian vertical text (`eaVert`) keeps ideographs and kana upright: the
/// turned run stays as the text item (not drawn), and its glyphs are drawn by
/// unreported runs, each wide glyph turned back about the centre of its cell.
fn upright_cjk(run: &mut crate::office::pptx::scene::GlyphRun) -> Vec<Item> {
    let chars: Vec<char> = run.text.chars().collect();
    if chars.len() != run.glyphs.len() || !chars.iter().any(|c| is_wide(*c)) {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut rest = run.clone();
    rest.text = String::new();
    rest.glyphs.clear();
    for (k, (g, c)) in run.glyphs.iter().zip(&chars).enumerate() {
        if !is_wide(*c) {
            rest.glyphs.push(*g);
            continue;
        }
        let next = run.glyphs.get(k + 1).map_or(run.width, |n| n.x);
        let (cx, cy) = (
            ((g.x + next) / 2.0) as f32,
            ((run.descent - run.ascent) / 2.0) as f32,
        );
        let back = tiny_skia::Transform::from_translate(cx, cy)
            .pre_rotate(-90.0)
            .pre_translate(-cx, -cy);
        let mut one = rest.clone();
        one.glyphs = vec![*g];
        one.transform = run.transform.pre_concat(back);
        out.push(Item::Text(one));
    }
    out.push(Item::Text(rest));
    run.drawn = false;
    out
}

/// Characters set upright in vertical East Asian text: ideographs, kana,
/// Hangul and full-width forms.
fn is_wide(c: char) -> bool {
    matches!(c as u32,
        0x1100..=0x11FF | 0x2E80..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF | 0xFE30..=0xFE4F | 0xFF00..=0xFF60 | 0xFFE0..=0xFFE6 | 0x20000..=0x3FFFF)
}

/// Applies a general transform to an item (vertical cell text).
pub fn transform_item(item: &mut Item, t: tiny_skia::Transform) {
    match item {
        Item::Bookmark(marker) => t.map_point(&mut marker.point),
        Item::Text(r) => r.transform = r.transform.post_concat(t),
        Item::Image(i) => {
            i.transform = i.transform.post_concat(t);
            if let Some(c) = i.clip.take() {
                i.clip = c.transform(t);
            }
        }
        Item::Path(p) | Item::Multiply(p) => {
            if let Some(moved) = p.path.clone().transform(t) {
                p.path = moved;
            }
            // Gradients and picture fills are placed in page space too.
            if let Some(f) = p.fill.take() {
                p.fill = Some(crate::office::pptx::slide::transform_paint(f, &t));
            }
            if let Some(s) = p.stroke.as_mut() {
                s.paint = crate::office::pptx::slide::transform_paint(s.paint.clone(), &t);
            }
        }
        Item::PushClip(path) => {
            if let Some(moved) = path.clone().transform(t) {
                *path = moved;
            }
        }
        Item::PushSoftEdge(e) => {
            if let Some(moved) = e.path.clone().transform(t) {
                e.path = moved;
            }
        }
        // Shadows move with their source; the offset stays in page points.
        Item::Shadow(s) => match &mut s.source {
            crate::office::pptx::scene::ShadowSource::Path(p) => {
                if let Some(moved) = p.clone().transform(t) {
                    *p = moved;
                }
            }
            crate::office::pptx::scene::ShadowSource::Glyphs(r) => {
                r.transform = r.transform.post_concat(t)
            }
            crate::office::pptx::scene::ShadowSource::Image(i) => {
                i.transform = i.transform.post_concat(t);
                if let Some(c) = i.clip.take() {
                    i.clip = c.transform(t);
                }
            }
        },
        Item::PopClip | Item::PopSoftEdge => {}
    }
}

// Contiguous frame groups contain only paragraphs, so their units contain no rows.
// Preserve each drawing's paragraph and line
// origin while the surrounding frame still has local point coordinates.
fn frame_child_anchors(units: &[Unit]) -> Vec<crate::office::docx::model::FrameAnchor> {
    debug_assert!(units.iter().all(|u| matches!(u, Unit::Para(_))));
    let mut anchors = Vec::new();
    let mut y = 0.0;
    for unit in units {
        if let Unit::Para(p) = unit {
            let paragraph_y = y;
            y += p.before + p.pad_top;
            for line in &p.lines {
                anchors.extend(line.anchors.iter().cloned().map(|anchor| {
                    crate::office::docx::model::FrameAnchor {
                        placement: None,
                        anchor,
                        column_x: 0.0,
                        paragraph_y,
                        line_y: y,
                    }
                }));
                y += line.height;
            }
            y += p.pad_bottom + p.after;
        }
    }
    anchors
}

pub(crate) struct LocalProjection {
    pub units: Vec<Unit>,
    pub slices: Vec<Slice>,
    pub height: f64,
    /// Required height below each projected line top, in points; flow advance is separate.
    pub occupied: Vec<Vec<f64>>,
    pub occupied_height: f64,
}

impl LocalProjection {
    fn resolve_inline_textboxes(&mut self, flow: &Flow<'_>) {
        let mut changed = false;
        for unit in &mut self.units {
            if let Unit::Para(p) = unit
                && p.lines.iter().any(|l| !l.boxes.is_empty())
            {
                flow.resolve_inline_textboxes(&mut p.lines);
                changed = true;
            }
        }
        if changed {
            self.slices = stack_with_clearance(&self.units, !self.occupied.is_empty()).0;
        }
    }

    pub fn reserve_occupied(&mut self) {
        let mut index = 0;
        for (ui, unit) in self.units.iter().enumerate() {
            match unit {
                Unit::Para(p) => {
                    for (li, line) in p.lines.iter().enumerate() {
                        if let Some(extent) = self.occupied.get(ui).and_then(|lines| lines.get(li))
                        {
                            self.slices[index].h = self.slices[index].h.max(line.skip + extent);
                        }
                        index += 1;
                    }
                }
                Unit::Row(_) => index += 1,
            }
        }
        self.height = self.height.max(self.occupied_height);
    }
}

/// Share local picture exclusions between note units and their whole-note projection.
pub(crate) fn note_projection(units: Vec<Unit>, width: f64, env: &TextEnv<'_>) -> LocalProjection {
    let mut projection = cells::projection(units, width, env);
    projection.resolve_inline_textboxes(&Flow::new(env, env.compat_mode).for_notes());
    projection
}

pub(crate) fn note_projection_resuming(
    units: Vec<Unit>,
    width: f64,
    env: &TextEnv<'_>,
    unit: usize,
    line: usize,
) -> LocalProjection {
    let mut projection = cells::projection_resuming(units, width, env, Some((unit, line)));
    projection.resolve_inline_textboxes(&Flow::new(env, env.compat_mode).for_notes());
    projection
}

/// Stacks units top to bottom without pagination (cell content).
pub fn stack(units: &[Unit]) -> (Vec<Slice>, f64) {
    stack_with_clearance(units, false)
}

fn stack_with_clearance(units: &[Unit], clearance: bool) -> (Vec<Slice>, f64) {
    let mut out = Vec::new();
    let mut y = 0.0;
    for u in units {
        match u {
            Unit::Para(p) => {
                let paragraph_y = y;
                y += p.before + p.pad_top;
                let n = p.lines.len();
                for (k, l) in p.lines.iter().enumerate() {
                    let slice_y = y;
                    let skip = if clearance { l.skip } else { 0.0 };
                    y += skip;
                    let mut items = Vec::new();
                    if let Some(d) = &p.decor {
                        let top = if k == 0 { p.pad_top } else { 0.0 };
                        let bottom = if k + 1 == n { p.pad_bottom } else { 0.0 };
                        para_decor(
                            d,
                            -top,
                            l.height + top + bottom,
                            k == 0,
                            k + 1 == n,
                            &mut items,
                        );
                    }
                    items.extend(l.items.iter().cloned());
                    for it in items.iter_mut() {
                        translate_item(it, 0.0, y);
                    }
                    let anchors = l.anchors.iter().map(|a| (a.clone(), 0.0, y)).collect();
                    out.push(Slice {
                        y: slice_y,
                        h: skip + l.height,
                        items,
                        anchors,
                        paragraph_offset: paragraph_y - y,
                        widow: (p.widow && n >= 2).then_some((k, n)),
                        notes: l.notes.clone(),
                        nested_row: None,
                        nested_table: None,
                        paragraph_source: None,
                    });
                    y += l.height;
                }
                y += p.pad_bottom + p.after;
            }
            Unit::Row(r) => {
                // A text-anchored floating table (a header's) sits `tblpY` from where
                // the text reaches, and what follows starts below it (Word for Mac:
                // a header table at tblpY -453 rises 22.65 pt above the header top).
                if let Some((_, Ok(v))) = r.float_y.as_ref().filter(|f| f.0 == "text") {
                    y += v;
                }
                out.push(nested_row_slice(Rc::new(r.clone()), y));
                y += r.flow_height();
            }
        }
    }
    (out, y)
}

fn solid(path: Option<tiny_skia::Path>, color: Rgba) -> Option<Item> {
    Some(Item::Path(PathItem {
        path: path?,
        fill: Some(Paint::Solid(color)),
        stroke: None,
        has_curve: false,
        shape: None,
        from_text: false,
    }))
}

pub fn border_color(b: &Border) -> Rgba {
    Rgba::from_hex(&b.color).unwrap_or(Rgba {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 1.0,
    })
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> Option<tiny_skia::Path> {
    crate::office::pptx::textlayout::rect_path(x, y, w.max(0.0), h.max(0.0))
}

/// Shading and borders of one line's part of a paragraph box, line-local:
/// `y` is the box top relative to the line top, `h` its height.
pub fn para_decor(d: &ParaBox, y: f64, h: f64, first: bool, last: bool, out: &mut Vec<Item>) {
    let (y, h) = if first {
        (y - d.grow.0, h + d.grow.0)
    } else {
        (y, h)
    };
    let h = if last { h + d.grow.1 } else { h };
    if let Some(c) = d.shading {
        out.extend(solid(rect(d.x0, y, d.x1 - d.x0, h), c));
    }
    use crate::office::docx::decor::{band_width as bw, edge};
    if let (Some(b), true) = (&d.top, first) {
        edge(b, d.x0, y, d.x1 - d.x0, true, out);
    }
    if let (Some(b), true) = (&d.bottom, last) {
        edge(b, d.x0, y + h - bw(b), d.x1 - d.x0, true, out);
    }
    if let Some(b) = &d.left {
        edge(b, d.x0, y, h, false, out);
    }
    if let Some(b) = &d.right {
        edge(b, d.x1 - bw(b), y, h, false, out);
    }
}

/// Floating objects anchored in a row's cells, for a row drawn at (x, y).
pub fn row_anchors(
    r: &RowUnit,
    x: f64,
    y: f64,
) -> Vec<(crate::office::docx::model::Anchored, f64, f64)> {
    let mut out = Vec::new();
    for c in &r.cells {
        let free = (r.height - c.margins[0] - c.margins[2] - c.content_h).max(0.0);
        let dy = match c.v_align.as_str() {
            "center" => free / 2.0,
            "bottom" => free,
            _ => 0.0,
        };
        for s in &c.slices {
            for (a, ax, ay) in &s.anchors {
                out.push((
                    a.clone(),
                    x + c.x + c.margins[1] + ax,
                    y + c.margins[0] + dy + ay,
                ));
            }
        }
        if let Some(m) = &c.moved {
            for (a, ax, ay) in m.0.iter().flat_map(|s| s.anchors.iter()) {
                out.push((
                    a.clone(),
                    x + c.x + c.margins[1] + ax,
                    y + c.margins[0] + m.1 + ay,
                ));
            }
        }
    }
    out
}

/// A row's drawing at (x, y): shading, cell content, borders.
pub fn row_items(r: &RowUnit, x: f64, y: f64) -> Vec<Item> {
    row_items_fragment(r, x, y, true)
}

/// Paints a row, preserving its bottom perimeter when its successor is on
/// another page or column. Stacked nested tables have contiguous source rows.
pub fn row_items_fragment(r: &RowUnit, x: f64, y: f64, adjacent_next: bool) -> Vec<Item> {
    let (mut content, borders) = row_items_fragment_layers(r, x, y, adjacent_next);
    content.extend(borders);
    content
}

/// Separates backgrounds and cell content from perimeter strokes so relocated
/// merge content can paint above its owner background and below its borders.
pub fn row_items_fragment_layers(
    r: &RowUnit,
    x: f64,
    y: f64,
    adjacent_next: bool,
) -> (Vec<Item>, Vec<Item>) {
    let mut out = row_boundary_items(r, x, y, false);
    for c in &r.cells {
        if let Some(s) = c.shading {
            out.extend(solid(rect(x + c.x, y, c.w, r.height), s));
        }
    }
    for (ci, c) in r.cells.iter().enumerate() {
        let free = (r.height - c.margins[0] - c.margins[2] - c.content_h).max(0.0);
        let dy = match c.v_align.as_str() {
            "center" => free / 2.0,
            "bottom" => free,
            _ => 0.0,
        };
        let moved = c
            .moved
            .iter()
            .flat_map(|m| m.0.iter().map(move |s| (s, m.1)));
        for (s, off) in c.slices.iter().map(|s| (s, dy)).chain(moved) {
            for it in &s.items {
                let mut it = it.clone();
                translate_item(&mut it, x + c.x + c.margins[1], y + c.margins[0] + off);
                // Reading position within the row: cell index, and rows back to a
                // merge's first row (`paginate::draw_placed` turns it into a key).
                crate::office::docx::text::set_reading_key(
                    std::slice::from_mut(&mut it),
                    c.merged_from * 1024 + ci.min(1023),
                );
                out.push(it);
            }
        }
    }
    out.extend(row_boundary_items(r, x, y, true));
    let mut borders = Vec::new();
    for c in &r.cells {
        let (cx, cw, h) = (x + c.x, c.w, r.height);
        use crate::office::docx::decor::{band_width as bw, edge};
        for (k, d) in c.diagonals.iter().enumerate() {
            if let Some(b) = d {
                let (y0, y1) = if k == 0 { (y, y + h) } else { (y + h, y) };
                borders.extend(crate::office::docx::decor::line(b, (cx, y0), (cx + cw, y1)));
            }
        }
        if let (Some(b), false) = (&c.borders[0], c.merged) {
            edge(b, cx, y - bw(b) / 2.0, cw, true, &mut borders);
        }
        if let Some(b) = &c.borders[1] {
            edge(b, cx - bw(b) / 2.0, y, h, false, &mut borders);
        }
        if !(c.merge_continues && adjacent_next)
            && let Some(b) = &c.borders[2]
        {
            edge(b, cx, y + h - bw(b) / 2.0, cw, true, &mut borders);
        }
        if let Some(b) = &c.borders[3] {
            edge(b, cx + cw - bw(b) / 2.0, y, h, false, &mut borders);
        }
    }
    (out, borders)
}

/// Builds cached drawing and ownership from one immutable nested-row fragment.
fn nested_row_slice(row: Rc<RowUnit>, y: f64) -> Slice {
    let paint_y = y + row.outer_before;
    Slice {
        y,
        h: row.flow_height(),
        items: row_items(&row, 0.0, paint_y),
        anchors: row_anchors(&row, 0.0, paint_y),
        paragraph_offset: 0.0,
        widow: None,
        notes: row.notes(),
        nested_row: Some(row),
        nested_table: None,
        paragraph_source: None,
    }
}

/// Occupied slices can extend below later side-flow text; spacing belongs to the last slice.
pub(crate) fn slices_height(slices: &[Slice], terminal_after: f64) -> f64 {
    slices
        .iter()
        .enumerate()
        .map(|(i, slice)| {
            slice.y
                + slice.h
                + if i + 1 == slices.len() {
                    terminal_after
                } else {
                    0.0
                }
        })
        .fold(0.0, f64::max)
}

/// Ordinary nested rows retain source line ownership through recursive cuts.
pub(crate) fn split_slice(
    s: &Slice,
    limit: f64,
    every_cell: bool,
    fresh_capacity: f64,
) -> Option<(Slice, Slice)> {
    split_slice_in_env(s, limit, every_cell, fresh_capacity, None)
}

pub(crate) fn split_slice_in_env(
    s: &Slice,
    limit: f64,
    every_cell: bool,
    fresh_capacity: f64,
    env: Option<&TextEnv<'_>>,
) -> Option<(Slice, Slice)> {
    split_slice_with_progress(s, limit, every_cell, fresh_capacity, env, false)
}

fn split_slice_with_progress(
    s: &Slice,
    limit: f64,
    every_cell: bool,
    fresh_capacity: f64,
    env: Option<&TextEnv<'_>>,
    force_first: bool,
) -> Option<(Slice, Slice)> {
    if s.nested_table.is_some() {
        return note_children::split(s, limit, every_cell, fresh_capacity, env);
    }
    let row = s.nested_row.as_ref()?;
    if row.exact_height
        || !row.fragmentable_cells()
        || (row.cant_split
            && (every_cell || s.y > 0.01 || row.flow_height() <= fresh_capacity + 0.01))
    {
        return None;
    }
    let (top, rest) = split_row_with_progress(
        row,
        limit - s.y,
        every_cell,
        None,
        fresh_capacity,
        env,
        force_first,
    )?;
    // A fully consumed nested row remains one atomic source slice.
    if force_first
        && rest.flow_height() <= 0.01
        && rest.cells.iter().all(|cell| cell.slices.is_empty())
    {
        return None;
    }
    let rest_y = s.y + top.flow_height();
    Some((
        nested_row_slice(Rc::new(top), s.y),
        nested_row_slice(Rc::new(rest), rest_y),
    ))
}

/// `every_cell`: the row splits only when each cell with content keeps a line
/// (Word's rule away from the top of a page).
pub fn split_row(r: &RowUnit, avail: f64, every_cell: bool) -> Option<(RowUnit, RowUnit)> {
    split_row_reserved(r, avail, every_cell, None)
}

/// Slice-aware merged content that shares a row's physical fragments.
/// Heights are row-local; progress counts fitted source slices, including ones
/// occupying the same fragment's preceding rows.
#[derive(Clone, Copy, Default)]
pub struct MergeReservation {
    pub top: f64,
    pub rest: f64,
    pub progress: bool,
}

pub fn split_row_reserved(
    r: &RowUnit,
    avail: f64,
    every_cell: bool,
    reserve: Option<MergeReservation>,
) -> Option<(RowUnit, RowUnit)> {
    split_row_in_area(r, avail, every_cell, reserve, f64::INFINITY)
}

/// Full destination capacity keeps a fitting nested cantSplit row atomic while
/// allowing an oversized row to continue after a fresh-area start.
pub fn split_row_in_area(
    r: &RowUnit,
    avail: f64,
    every_cell: bool,
    reserve: Option<MergeReservation>,
    fresh_capacity: f64,
) -> Option<(RowUnit, RowUnit)> {
    split_row_in_env(r, avail, every_cell, reserve, fresh_capacity, None)
}

pub(crate) fn split_row_in_env(
    r: &RowUnit,
    avail: f64,
    every_cell: bool,
    reserve: Option<MergeReservation>,
    fresh_capacity: f64,
    env: Option<&TextEnv<'_>>,
) -> Option<(RowUnit, RowUnit)> {
    split_row_with_progress(r, avail, every_cell, reserve, fresh_capacity, env, false)
}

/// On an empty destination, consume an atomic first line while retaining the
/// remaining source slices. Recursive ordinary rows use the same progress rule.
pub(crate) fn split_row_first_line(
    r: &RowUnit,
    avail: f64,
    reserve: Option<MergeReservation>,
    fresh_capacity: f64,
) -> Option<(RowUnit, RowUnit)> {
    if r.exact_height {
        return None;
    }
    split_row_with_progress(r, avail, false, reserve, fresh_capacity, None, true)
}

fn split_row_with_progress(
    r: &RowUnit,
    avail: f64,
    every_cell: bool,
    reserve: Option<MergeReservation>,
    fresh_capacity: f64,
    env: Option<&TextEnv<'_>>,
    force_first: bool,
) -> Option<(RowUnit, RowUnit)> {
    // Natural table edges stay with the first and final pieces. Internal page
    // cuts keep the existing border painting; their extra extent is unproven.
    let avail = avail - r.outer_before;
    if avail <= 0.0 && !force_first {
        return None;
    }
    let mut top = r.clone();
    let mut rest = r.clone();
    top.outer_after = 0.0;
    rest.outer_before = 0.0;
    top.bookmarks_after.clear();
    rest.bookmarks_before.clear();
    let mut any = reserve.is_some_and(|r| r.progress);
    let mut top_h = reserve.map_or(0.0, |r| r.top);
    let mut rest_h = reserve.map_or(0.0, |r| r.rest);
    for (k, c0) in r.cells.iter().enumerate() {
        let room = avail - c0.margins[0] - c0.margins[2];
        // A nested row taller than the room left is cut between its lines.
        let mut c = std::borrow::Cow::Borrowed(c0);
        if let Some(i) = c0.slices.iter().position(|s| s.y + s.h > room + 0.01) {
            let s = &c0.slices[i];
            if (s.nested_row.is_some() || s.nested_table.is_some())
                && (s.y < room || (force_first && i == 0))
            {
                let inner_capacity = (fresh_capacity
                    - r.outer_before
                    - r.outer_after
                    - c0.margins[0]
                    - c0.margins[2])
                    .max(0.0);
                if let Some((a, b)) = split_slice_with_progress(
                    s,
                    room,
                    every_cell,
                    inner_capacity,
                    env,
                    force_first && i == 0,
                ) {
                    let delta = a.h + b.h - s.h;
                    let cell = c.to_mut();
                    for later in &mut cell.slices[i + 1..] {
                        later.y += delta;
                        for item in &mut later.items {
                            translate_item(item, 0.0, delta);
                        }
                        for anchor in &mut later.anchors {
                            anchor.2 += delta;
                        }
                    }
                    cell.slices.splice(i..=i, [a, b]);
                }
            }
        }
        let c = c.as_ref();
        let n_sl = c.slices.len();
        let mut cut = c
            .slices
            .iter()
            .enumerate()
            .position(|(i, s)| {
                // The final line must also leave room for the row's border extent.
                let tail = if i + 1 == n_sl {
                    c.terminal_after + r.natural_addition + r.outer_after
                } else {
                    0.0
                };
                s.y + s.h + tail > room + 0.01
            })
            .unwrap_or(n_sl);
        // Widow and orphan control inside the cell: a paragraph leaves neither
        // its last nor its first line alone (A/B: breaks per file +0.8 points).
        if cut > 0
            && cut < c.slices.len()
            && let Some((k, n)) = c.slices[cut].widow
        {
            if k + 1 == n && k >= 1 {
                cut -= 1;
            }
            if let Some((k2, _)) = c.slices[cut].widow
                && k2 == 1
            {
                cut = cut.saturating_sub(1);
            }
        }
        if force_first && cut == 0 && !c.slices.is_empty() {
            cut = 1;
        }
        // Away from the page top, a continuing cell keeps at least two lines, unless
        // no paragraph in it has two lines to protect and the line left behind holds
        // text (Word for Mac splits a cell of one-line address paragraphs after one).
        let one_text_line = c.slices.iter().all(|s| s.widow.is_none())
            && c.slices[..cut.min(c.slices.len())].iter().any(|s| {
                s.items
                    .iter()
                    .any(|i| matches!(i, Item::Text(t) if !t.text.trim().is_empty()))
            });
        if cut > 0 && cut < c.slices.len() && cut < 2 && every_cell && !one_text_line {
            return None;
        }
        if cut > 0 {
            any = true;
        } else if every_cell && !c.slices.is_empty() {
            // A row splits only when every cell with content keeps a line on the
            // page (A/B: 593 set 4 better, none worse; new 300 1 better, none worse).
            return None;
        }
        let (a, b) = c.slices.split_at(cut);
        let a_after = if cut == n_sl { c.terminal_after } else { 0.0 };
        let b_after = if cut < n_sl { c.terminal_after } else { 0.0 };
        let a_h = slices_height(a, a_after);
        let shift = b.first().map_or(0.0, |s| s.y);
        let b: Vec<Slice> = b
            .iter()
            .map(|s| {
                let mut s = s.clone();
                for it in s.items.iter_mut() {
                    translate_item(it, 0.0, -shift);
                }
                for a in s.anchors.iter_mut() {
                    a.2 -= shift;
                }
                s.y -= shift;
                s
            })
            .collect();
        let b_h = slices_height(&b, b_after);
        top.cells[k].slices = a.to_vec();
        top.cells[k].content_h = a_h;
        top.cells[k].terminal_after = a_after;
        rest.cells[k].slices = b;
        rest.cells[k].content_h = b_h;
        rest.cells[k].terminal_after = b_after;
        rest.cells[k].moved = None;
        top_h = top_h.max(a_h + c.margins[0] + c.margins[2]);
        if b_h > 0.0 {
            rest_h = rest_h.max(b_h + c.margins[0] + c.margins[2]);
        }
    }
    // A trailing painted edge alone cannot create a content-free final piece.
    // The caller moves the whole row, or accepts its overflow on a fresh page.
    if rest_h <= 0.0 && r.outer_after > 0.0 && r.height <= avail + 0.01 {
        return None;
    }
    // Away from the page top, a row whose content all fits moves whole.
    if !any || (rest_h <= 0.0 && every_cell) {
        return None;
    }
    top.height = top_h;
    if force_first && rest.cells.iter().all(|cell| cell.slices.is_empty()) {
        // Resolved border and spacing extent stays with the final atomic content.
        if r.height <= top.height + r.natural_addition + 0.01 {
            top.height = top.height.max(r.height);
        }
    }
    let consumed = if reserve.is_some() || force_first {
        top.height
    } else {
        avail
    };
    rest.height = rest_h
        .max((r.height - consumed).max(0.0))
        .max(if force_first { 0.0 } else { 1.0 });
    if force_first && rest.height <= 0.01 && rest.cells.iter().all(|c| c.slices.is_empty()) {
        top.outer_after = r.outer_after;
        rest.outer_after = 0.0;
        top.bookmarks_after = std::mem::take(&mut rest.bookmarks_after);
    }
    if let Some(source) = &mut rest.source {
        let source = Rc::make_mut(source);
        source.minimum = (source.minimum - consumed).max(0.0);
        source.addition = 0.0;
    }
    // The part of a split row left on the page ends below its last line and
    // ends it at the page bottom without them).
    for (c, orig) in rest.cells.iter_mut().zip(&r.cells) {
        c.borders[0] = orig.borders[0].clone();
    }
    Some((top, rest))
}

/// Widest span of text and pictures over the lines of `units` (points),
/// from the leftmost to the rightmost item of a line.
fn longest_line(units: &[Unit]) -> f64 {
    let mut best: f64 = 0.0;
    for sl in stack(units).0 {
        let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
        for it in &sl.items {
            let (x0, x1) = match it {
                Item::Text(r) => {
                    let x = r.transform.tx as f64;
                    (x, x + r.width)
                }
                Item::Image(i) => {
                    let x = i.transform.tx as f64;
                    (x, x + i.transform.sx as f64)
                }
                _ => continue,
            };
            lo = lo.min(x0);
            hi = hi.max(x1);
        }
        if hi > lo {
            best = best.max(hi - lo);
        }
    }
    best
}

/// Attribute moved text to its accepted row while preserving its declared cell owner.
pub(crate) fn stamp_fragment_source(
    items: &mut [Item],
    row: Option<&crate::office::pptx::scene::TextSource>,
) {
    let Some(row) = row else {
        return;
    };
    let Some(destination) = row.cells.first() else {
        return;
    };
    let stamp = |source: &crate::office::pptx::scene::TextSource, text_only: bool| {
        let owner = source.cells.first()?;
        if source.part != row.part
            || owner.table != destination.table
            || (text_only && !owner.text_only)
        {
            return None;
        }
        let chain = owner
            .vertical_merges
            .iter()
            .find(|merge| merge.cell == owner.cell)?;
        let member = destination.vertical_merges.iter().find(|merge| {
            merge.grid_start == chain.grid_start
                && merge.span == chain.span
                && merge.start_row == chain.start_row
                && merge.end_row == chain.end_row
        })?;
        if owner.row == destination.row {
            return None;
        }
        let mut source = source.clone();
        let mut fragment = destination.clone();
        fragment.cell = member.cell;
        fragment.span = member.span;
        fragment.fragment = None;
        source.cells[0].fragment = Some(Box::new(fragment));
        Some(source)
    };
    for item in items {
        match item {
            Item::Text(run) => {
                if let Some(source) = run.source.as_deref().and_then(|source| stamp(source, true)) {
                    run.source = Some(std::sync::Arc::new(source));
                }
                for span in run
                    .bookmarks
                    .iter_mut()
                    .flat_map(|metadata| metadata.source_spans.iter_mut())
                {
                    if let Some(source) = span
                        .source
                        .as_deref()
                        .and_then(|source| stamp(source, true))
                    {
                        span.source = Some(std::sync::Arc::new(source));
                    }
                }
                for marker in run
                    .bookmarks
                    .iter_mut()
                    .flat_map(|bookmarks| &mut bookmarks.boundaries)
                {
                    if let Some(source) = stamp(&marker.source.source, false) {
                        std::sync::Arc::make_mut(&mut marker.source).source = source;
                    }
                }
            }
            Item::Bookmark(marker) => {
                if let Some(source) = stamp(&marker.source.source, false) {
                    std::sync::Arc::make_mut(&mut marker.source).source = source;
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests;

/// Table-edge markers follow the accepted first/final row fragment, outside cell ownership.
fn row_boundary_items(row: &RowUnit, x: f64, y: f64, after: bool) -> Vec<Item> {
    let (markers, y, key) = if after {
        (&row.bookmarks_after, y + row.height + row.outer_after, 1023)
    } else {
        (&row.bookmarks_before, y - row.outer_before, 0)
    };
    markers
        .iter()
        .map(|source| {
            Item::Bookmark(crate::office::pptx::scene::BookmarkItem {
                source: source.clone(),
                point: tiny_skia::Point::from_xy(x as f32, y as f32),
                reading_key: key,
            })
        })
        .collect()
}

fn paragraph_markers(
    p: &Paragraph,
) -> Vec<std::sync::Arc<crate::office::pptx::scene::SourceBookmark>> {
    p.inlines
        .iter()
        .filter_map(|inline| match inline {
            crate::office::docx::model::Inline::Bookmark(marker) => Some(marker.clone()),
            _ => None,
        })
        .collect()
}
