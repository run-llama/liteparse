//! Block classification for the markdown emitter.
//!
//! Consumes `ProjectedLine` entries from each `ParsedPage` and groups them into
//! a sequence of `Block`s: headings, paragraphs, list items, code blocks,
//! tables (ruled and borderless), horizontal rules, and figures. Tabular
//! regions that can't be classified confidently fall back to a fenced grid
//! projection rather than a mangled pipe table.

mod blocks;
mod classify;
mod cross_region;
mod flags;
mod headings;
mod hr;
mod inline;
mod lists;
mod paragraphs;
mod repetition;
mod table_header;
mod table_spans;
mod tables;

pub use blocks::{Block, Cell, PositionedBlock, SpanCell, render_blocks, splice_soft_hyphens};
pub use classify::classify_page_with_filters;
pub use headings::{build_heading_map, compute_body_size};
pub use inline::{apply_link, escape_inline};
pub use repetition::{compute_header_footer_set, detect_single_page_chrome};
pub use tables::detect_table_rects;
pub(crate) use tables::{count_text_table_runs, detect_table_grids, validated_ruled_table_rects};

/// Minimum plausible text-row height in points. Floors a `bbox.height` before
/// it is multiplied into a band / tolerance window, so a degenerate near-zero
/// glyph box can't collapse that window to nothing. Shared by the table
/// HR-suppress headroom (`classify.rs`) and outline y-tolerance
/// (`headings.rs`) so the two stay in sync.
const MIN_ROW_HEIGHT_PT: f32 = 8.0;

#[cfg(test)]
pub(crate) mod test_helpers;
