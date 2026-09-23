//! Public, serializable view of the markdown classifier's block decomposition.
//!
//! The classifier's internal [`Block`](crate::markdown_layout::Block) is a Rust
//! enum with per-variant payloads. That shape can't cross the napi or PyO3
//! boundary — neither supports enums carrying data — so the public type is a
//! single flat struct discriminated by `kind`, with the variant-specific fields
//! as `Option`s. This mirrors how the crate already exposes
//! `StructureAttributeValue` to the bindings, and keeps one block shape across
//! Rust, JSON, Node, Python, and WASM instead of three divergent ones.
//!
//! Every field that doesn't apply to a block's `kind` is `None` and is skipped
//! during serialization, so a heading serializes as `{kind, text, level, bbox}`
//! rather than a wall of nulls.

use serde::{Deserialize, Serialize};

use crate::markdown_layout::{Block, Cell, PositionedBlock, SpanCell};
use crate::types::{ParsedPage, Rect};

/// One table cell: its rendered text and the region it occupied.
///
/// `bbox` is `None` for cells with no ink behind them — padding inserted to
/// square off a ragged grid, or halves of a merged run split at an estimated
/// position rather than an observed boundary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayoutCell {
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bbox: Option<Rect>,
    /// Merge spans, present only on `merged_table` cells (and only when > 1).
    /// A cell absorbed by a neighbour's span is absent from its row entirely,
    /// matching HTML's occupancy model.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub colspan: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rowspan: Option<u16>,
}

impl From<&Cell> for LayoutCell {
    fn from(c: &Cell) -> Self {
        LayoutCell {
            text: c.text.clone(),
            bbox: c.bbox.clone(),
            colspan: None,
            rowspan: None,
        }
    }
}

impl From<&SpanCell> for LayoutCell {
    fn from(c: &SpanCell) -> Self {
        LayoutCell {
            text: c.text.clone(),
            bbox: c.bbox.clone(),
            colspan: (c.colspan > 1).then_some(c.colspan),
            rowspan: (c.rowspan > 1).then_some(c.rowspan),
        }
    }
}

/// A classified block plus where it sits on the page.
///
/// `kind` discriminates the block; see each field for which kinds populate it.
/// Blocks appear in reading order, matching the order the markdown renderer
/// emits them, so the Nth block here is the Nth block of that page's markdown.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayoutBlock {
    /// One of `heading`, `paragraph`, `list_item`, `code`, `table`,
    /// `merged_table`, `grid_fallback`, `rule`, `figure`.
    pub kind: String,
    /// Rendered text for the text-bearing kinds (`heading`, `paragraph`,
    /// `list_item`). Table text lives in `header`/`rows`; code and grid text in
    /// `lines`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Heading level (1–6), or list nesting depth for `list_item`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<u8>,
    /// Whether the block's text is uniformly bold / italic. `paragraph` and
    /// `list_item` only; omitted when false.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub bold: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub italic: bool,
    /// `list_item`: whether the list is ordered, and the original marker as it
    /// appeared on the page (`138.`, `iii)`, `•`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ordered: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub marker: Option<String>,
    /// Verbatim source lines for `code` and `grid_fallback`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lines: Option<Vec<String>>,
    /// Best-effort language hint for `code`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lang: Option<String>,
    /// `table`: the header row, when one was detected.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub header: Option<Vec<LayoutCell>>,
    /// `table`: the body rows. `merged_table`: all rows (header rows lead,
    /// counted by `header_rows`); rows are ragged — cells covered by a
    /// neighbour's span are absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rows: Option<Vec<Vec<LayoutCell>>>,
    /// `merged_table`: how many leading rows of `rows` are header rows.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub header_rows: Option<usize>,
    /// `figure`: the image's page-scoped id and encoded format, matching the
    /// `img_{id}.{format}` target the markdown renderer emits.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    /// Region of the page this block occupies, in the same top-left, 72-DPI
    /// viewport space as `text_items`. The union of every source line that fed
    /// the block, so a wrapped heading or multi-line paragraph reports its full
    /// band. `None` when the block has no page geometry behind it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bbox: Option<Rect>,
}

impl LayoutBlock {
    /// Base value with every variant-specific field cleared.
    fn of(kind: &'static str, bbox: Option<Rect>) -> Self {
        LayoutBlock {
            kind: kind.to_string(),
            text: None,
            level: None,
            bold: false,
            italic: false,
            ordered: None,
            marker: None,
            lines: None,
            lang: None,
            header: None,
            rows: None,
            header_rows: None,
            id: None,
            format: None,
            bbox,
        }
    }
}

fn cells(row: &[Cell]) -> Vec<LayoutCell> {
    row.iter().map(LayoutCell::from).collect()
}

impl From<&PositionedBlock> for LayoutBlock {
    fn from(pb: &PositionedBlock) -> Self {
        let bbox = pb.bbox.clone();
        match &pb.block {
            Block::Heading { level, text } => LayoutBlock {
                text: Some(text.clone()),
                level: Some(*level),
                ..LayoutBlock::of("heading", bbox)
            },
            Block::Paragraph { text, bold, italic } => LayoutBlock {
                text: Some(text.clone()),
                bold: *bold,
                italic: *italic,
                ..LayoutBlock::of("paragraph", bbox)
            },
            Block::ListItem {
                ordered,
                marker,
                level,
                text,
                bold,
                italic,
            } => LayoutBlock {
                text: Some(text.clone()),
                level: Some(*level),
                ordered: Some(*ordered),
                marker: Some(marker.clone()),
                bold: *bold,
                italic: *italic,
                ..LayoutBlock::of("list_item", bbox)
            },
            Block::CodeBlock { lines, lang } => LayoutBlock {
                lines: Some(lines.clone()),
                lang: lang.clone(),
                ..LayoutBlock::of("code", bbox)
            },
            Block::Table { header, rows } => LayoutBlock {
                header: header.as_ref().map(|h| cells(h)),
                rows: Some(rows.iter().map(|r| cells(r)).collect()),
                ..LayoutBlock::of("table", bbox)
            },
            Block::MergedTable { rows, header_rows } => LayoutBlock {
                header_rows: Some(*header_rows),
                rows: Some(
                    rows.iter()
                        .map(|r| r.iter().map(LayoutCell::from).collect())
                        .collect(),
                ),
                ..LayoutBlock::of("merged_table", bbox)
            },
            Block::GridFallback { lines } => LayoutBlock {
                lines: Some(lines.clone()),
                ..LayoutBlock::of("grid_fallback", bbox)
            },
            Block::HorizontalRule => LayoutBlock::of("rule", bbox),
            Block::Figure { id, format } => LayoutBlock {
                id: Some(id.clone()),
                format: Some(format.clone()),
                ..LayoutBlock::of("figure", bbox)
            },
        }
    }
}

/// Public block list for one page: the classifier's blocks converted to the
/// flat serializable shape, with geometry in the page's viewport frame.
///
/// Block and cell boxes are unioned from `ProjectedLine.bbox`, which lives in
/// the *projection* frame. On pages where rotation handling moved text (see
/// `ParsedPage::projected_item_frames`) that frame is a virtual canvas —
/// unrotated sidebars, body text pushed a page-height down — so those boxes
/// are mapped back here. Pages without rotated text have an empty table and
/// pass through unchanged.
pub(crate) fn blocks_for_page(page: &ParsedPage, blocks: &[PositionedBlock]) -> Vec<LayoutBlock> {
    let mut out: Vec<LayoutBlock> = blocks.iter().map(LayoutBlock::from).collect();
    if !page.projected_item_frames.is_empty() {
        remap_to_page_frame(&mut out, &page.projected_item_frames);
    }
    out
}

/// Map text-derived block and cell boxes from the projection frame back to
/// the page frame. A box is the union of the text items it was built from,
/// so its page-frame counterpart is the union of those items' original
/// rects — found by testing which projected items sit inside it. `figure`
/// and `rule` boxes come from graphics, already in page coordinates, and are
/// left alone; so is any box that contains no projected item (nothing to
/// map it through).
fn remap_to_page_frame(blocks: &mut [LayoutBlock], frames: &[(Rect, Rect)]) {
    for block in blocks.iter_mut() {
        if matches!(block.kind.as_str(), "figure" | "rule") {
            continue;
        }
        remap_opt(&mut block.bbox, frames);
        for cell in block.header.iter_mut().flatten() {
            remap_opt(&mut cell.bbox, frames);
        }
        for cell in block.rows.iter_mut().flatten().flatten() {
            remap_opt(&mut cell.bbox, frames);
        }
    }
}

fn remap_opt(bbox: &mut Option<Rect>, frames: &[(Rect, Rect)]) {
    if let Some(r) = bbox.as_ref().and_then(|r| remap_rect(r, frames)) {
        *bbox = Some(r);
    }
}

/// Union of the original rects of every item whose projected centre lies in
/// `rect` (with a small tolerance for float drift in the unions). `None` when
/// no item does.
fn remap_rect(rect: &Rect, frames: &[(Rect, Rect)]) -> Option<Rect> {
    const TOL: f32 = 0.5;
    let x0 = rect.x - TOL;
    let y0 = rect.y - TOL;
    let x1 = rect.x + rect.width + TOL;
    let y1 = rect.y + rect.height + TOL;
    let mut acc: Option<Rect> = None;
    for (projected, original) in frames {
        let cx = projected.x + projected.width / 2.0;
        let cy = projected.y + projected.height / 2.0;
        if cx >= x0 && cx <= x1 && cy >= y0 && cy <= y1 {
            Rect::extend(&mut acc, original);
        }
    }
    acc
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect {
            x,
            y,
            width: w,
            height: h,
        }
    }

    fn para(bbox: Rect) -> LayoutBlock {
        LayoutBlock {
            text: Some("t".into()),
            ..LayoutBlock::of("paragraph", Some(bbox))
        }
    }

    #[test]
    fn remap_unions_original_rects_of_contained_items() {
        // Body text displaced one page-height (792pt) down by rotation handling.
        let frames = vec![
            (
                rect(50.0, 892.0, 100.0, 10.0),
                rect(50.0, 100.0, 100.0, 10.0),
            ),
            (
                rect(50.0, 904.0, 120.0, 10.0),
                rect(50.0, 112.0, 120.0, 10.0),
            ),
            // A rotated sidebar, unrotated in the projection frame.
            (
                rect(20.0, 30.0, 200.0, 12.0),
                rect(14.0, 300.0, 12.0, 200.0),
            ),
        ];
        let mut blocks = vec![
            para(rect(50.0, 892.0, 120.0, 22.0)),
            para(rect(20.0, 30.0, 200.0, 12.0)),
            LayoutBlock::of("figure", Some(rect(300.0, 300.0, 50.0, 50.0))),
        ];
        remap_to_page_frame(&mut blocks, &frames);
        assert_eq!(blocks[0].bbox, Some(rect(50.0, 100.0, 120.0, 22.0)));
        assert_eq!(blocks[1].bbox, Some(rect(14.0, 300.0, 12.0, 200.0)));
        // Graphics-derived boxes are already in page space.
        assert_eq!(blocks[2].bbox, Some(rect(300.0, 300.0, 50.0, 50.0)));
    }

    #[test]
    fn remap_leaves_boxes_with_no_items_untouched() {
        let frames = vec![(rect(0.0, 900.0, 10.0, 10.0), rect(0.0, 100.0, 10.0, 10.0))];
        let mut blocks = vec![para(rect(200.0, 200.0, 50.0, 10.0))];
        remap_to_page_frame(&mut blocks, &frames);
        assert_eq!(blocks[0].bbox, Some(rect(200.0, 200.0, 50.0, 10.0)));
    }
}
