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
//!
//! # Provenance (fork)
//!
//! Every block and every table cell additionally carries
//! `text_item_indices`: indices into the page's RETURNED `text_items` array
//! (post-projection order — the order the caller receives, not PDF paint
//! order). The field is always serialized, `[]` when empty. Contract:
//!
//! - block indices are sorted and deduped; for a `table` block they equal the
//!   union of its cells' indices;
//! - cell indices are in insertion order (reading order: line order,
//!   x-ascending within a line) and never repeat within one cell — but one
//!   index may appear in several cells (a merged span split across column
//!   tracks attributes each fragment to the whole source span);
//! - `rule` blocks from vector strokes and `figure` blocks (no text behind
//!   them) carry `[]`, as do padding cells inserted to square off a ragged
//!   grid; a `rule` detected from a decorative text flourish (`* * *`)
//!   carries the flourish line's items.

use serde::Serialize;

use crate::markdown_layout::{Block, Cell, PositionedBlock};
use crate::types::Rect;

/// One table cell: its rendered text and the region it occupied.
///
/// `bbox` is `None` for cells with no ink behind them — padding inserted to
/// square off a ragged grid, or halves of a merged run split at an estimated
/// position rather than an observed boundary.
#[derive(Debug, Clone, Serialize)]
pub struct LayoutCell {
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bbox: Option<Rect>,
    /// Indices into the page's returned `text_items`, in reading order,
    /// never repeating within the cell (see the module docs). Always
    /// serialized; `[]` for padding cells.
    pub text_item_indices: Vec<usize>,
}

impl From<&Cell> for LayoutCell {
    fn from(c: &Cell) -> Self {
        // Insertion order preserved; belt-and-suspenders dedup upholds the
        // "never twice within one cell" contract even if an accumulation
        // path double-pushed.
        let mut indices = c.text_item_indices.clone();
        let mut seen = std::collections::HashSet::new();
        indices.retain(|i| seen.insert(*i));
        LayoutCell {
            text: c.text.clone(),
            bbox: c.bbox.clone(),
            text_item_indices: indices,
        }
    }
}

/// A classified block plus where it sits on the page.
///
/// `kind` discriminates the block; see each field for which kinds populate it.
/// Blocks appear in reading order, matching the order the markdown renderer
/// emits them, so the Nth block here is the Nth block of that page's markdown.
#[derive(Debug, Clone, Serialize)]
pub struct LayoutBlock {
    /// One of `heading`, `paragraph`, `list_item`, `code`, `table`,
    /// `grid_fallback`, `rule`, `figure`.
    pub kind: &'static str,
    /// Indices into the page's returned `text_items`, sorted and deduped
    /// (see the module docs). Always serialized; `[]` for text-less blocks.
    pub text_item_indices: Vec<usize>,
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
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub bold: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
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
    /// `table`: the body rows.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rows: Option<Vec<Vec<LayoutCell>>>,
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
            kind,
            text_item_indices: Vec::new(),
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
        // Block-level indices are sorted + deduped at this public boundary;
        // insertion order (a classifier-internal detail) is not part of the
        // block-level contract.
        let mut text_item_indices = pb.text_item_indices.clone();
        text_item_indices.sort_unstable();
        text_item_indices.dedup();
        // A table block's indices must be exactly the union of its cells'.
        #[cfg(debug_assertions)]
        if let Block::Table { header, rows } = &pb.block {
            let mut cell_union: Vec<usize> = header
                .iter()
                .flatten()
                .chain(rows.iter().flatten())
                .flat_map(|c| c.text_item_indices.iter().copied())
                .collect();
            cell_union.sort_unstable();
            cell_union.dedup();
            debug_assert_eq!(
                text_item_indices, cell_union,
                "table block indices must equal the union of its cell indices"
            );
        }
        let mut out = match &pb.block {
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
        };
        out.text_item_indices = text_item_indices;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_indices_are_sorted_and_deduped_at_the_boundary() {
        let pb = PositionedBlock::new(
            Block::Paragraph {
                text: "x".into(),
                bold: false,
                italic: false,
            },
            None,
            vec![5, 2, 5, 9, 2],
        );
        let lb = LayoutBlock::from(&pb);
        assert_eq!(lb.text_item_indices, vec![2, 5, 9]);
    }

    #[test]
    fn cell_indices_keep_reading_order() {
        let mut c = Cell::from("a b");
        c.add_indices([9, 3, 9, 7]);
        let lc = LayoutCell::from(&c);
        // Insertion order preserved, repeats dropped — never re-sorted.
        assert_eq!(lc.text_item_indices, vec![9, 3, 7]);
    }
}
