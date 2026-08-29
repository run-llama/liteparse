use super::inline::escape_inline;
use super::paragraphs::{ParaAccum, is_soft_hyphen_break};
use super::tables::escape_table_cell;
use crate::types::Rect;

/// One table cell: rendered text plus, when the cell came from real page
/// content, the region it occupied. `bbox` is `None` for cells that exist only
/// to square off a ragged grid (padding inserted when rows disagree on column
/// count) — those occupy no ink on the page, and reporting a rect for them
/// would invent geometry the classifier never saw.
#[derive(Debug, Clone, Default)]
pub struct Cell {
    pub text: String,
    pub bbox: Option<Rect>,
    /// Indices of the source items this cell's text was read from, into the
    /// page's *returned* `text_items` array (see
    /// `ProjectedLine::span_item_indices` for the index contract). Insertion
    /// order is reading order (line order, x-ascending within a line) and is
    /// preserved as-is — never sorted or deduped at cell level, because
    /// consumers use it as the cell's canonical word order. One index may
    /// legitimately appear in several cells (a merged span split across
    /// column tracks attributes to each), but never twice within one cell.
    /// Empty for padding cells and cells synthesized without page content.
    pub text_item_indices: Vec<usize>,
}

impl Cell {
    /// Cell carrying text and the region it was read from, with no
    /// source-item attribution (tests and legacy paths).
    pub fn located(text: impl Into<String>, bbox: Rect) -> Self {
        Cell {
            text: text.into(),
            bbox: Some(bbox),
            text_item_indices: Vec::new(),
        }
    }

    /// Cell carrying text, the region it was read from, and the returned
    /// text-item indices that produced it.
    pub fn located_with(
        text: impl Into<String>,
        bbox: Rect,
        text_item_indices: Vec<usize>,
    ) -> Self {
        Cell {
            text: text.into(),
            bbox: Some(bbox),
            text_item_indices,
        }
    }

    /// Append more contributing source-item indices, keeping insertion order.
    pub fn add_indices(&mut self, indices: impl IntoIterator<Item = usize>) {
        self.text_item_indices.extend(indices);
    }

    /// Borrow the cell's text. Lets call sites that only care about content
    /// read a `Cell` about as tersely as the `String` it replaced.
    pub fn as_str(&self) -> &str {
        &self.text
    }
}

// Text-only construction (`"a".into()`, `s.to_string().into()`)
impl From<&str> for Cell {
    fn from(text: &str) -> Self {
        Cell {
            text: text.to_string(),
            bbox: None,
            text_item_indices: Vec::new(),
        }
    }
}

impl From<String> for Cell {
    fn from(text: String) -> Self {
        Cell {
            text,
            bbox: None,
            text_item_indices: Vec::new(),
        }
    }
}

// Cells compare by content alone. A cell's box is derived metadata describing
// where the text was read from, not part of its identity — two cells holding
// the same text are the same table content whether or not either one knows its
// coordinates. This is also what lets assertions compare a detected grid
// against a plain literal of expected strings.
impl PartialEq for Cell {
    fn eq(&self, other: &Self) -> bool {
        self.text == other.text
    }
}

impl Eq for Cell {}

impl PartialEq<str> for Cell {
    fn eq(&self, other: &str) -> bool {
        self.text == other
    }
}

impl PartialEq<&str> for Cell {
    fn eq(&self, other: &&str) -> bool {
        self.text == *other
    }
}

impl PartialEq<String> for Cell {
    fn eq(&self, other: &String) -> bool {
        self.text == *other
    }
}

/// Coarse block representation: the output of page classification, consumed by
/// `render_blocks` to produce the final markdown string.
#[derive(Debug, Clone)]
pub enum Block {
    Heading {
        level: u8,
        text: String,
    },
    Paragraph {
        text: String,
        bold: bool,
        italic: bool,
    },
    ListItem {
        ordered: bool,
        marker: String,
        level: u8,
        text: String,
        bold: bool,
        italic: bool,
    },
    /// Fenced code block — content rendered between triple-backtick fences.
    /// Each entry in `lines` is one source line; preserved as-is (only trailing
    /// whitespace stripped) so indentation survives. `lang` is a best-effort
    /// language hint emitted as the fence info-string (e.g. ```` ```python ````)
    /// when the body matches a known language; `None` emits a bare fence.
    CodeBlock {
        lines: Vec<String>,
        lang: Option<String>,
    },
    /// Confident table emitted as a markdown pipe table. `header` is `None`
    /// when the first row didn't qualify (e.g. wasn't bold and the table mode
    /// can't otherwise distinguish it).
    Table {
        header: Option<Vec<Cell>>,
        rows: Vec<Vec<Cell>>,
    },
    /// Tabular-looking region we couldn't classify confidently — rendered
    /// verbatim inside a fenced block to preserve visual structure for the
    /// downstream LLM. Strictly better than emitting a mangled pipe table.
    GridFallback {
        lines: Vec<String>,
    },
    /// A horizontal rule detected from a long thin horizontal stroke in the
    /// page's vector graphics (e.g. divider line between sections).
    HorizontalRule,
    /// Reference to a raster image on the page. Rendered as
    /// `![](img_{id}.{format})`. Suppressed entirely when `ImageMode::Off`.
    Figure {
        id: String,
        format: String,
    },
}

/// Resolve a `ParaAccum` to a `Block::Paragraph`. When the paragraph was
/// uniformly styled across all lines, return the raw text with block-level
/// `bold`/`italic` flags set so the renderer wraps it once. Otherwise return
/// the per-line inline-styled text with the flags cleared.
pub(super) fn paragraph_from_accum(accum: ParaAccum) -> Block {
    match accum.uniform {
        Some((bold, italic)) if bold || italic => Block::Paragraph {
            text: escape_inline(&accum.raw),
            bold,
            italic,
        },
        Some(_) => Block::Paragraph {
            // Uniformly plain — no emphasis markers anywhere, so the raw text
            // (with markdown specials escaped) is the right rendering.
            text: escape_inline(&accum.raw),
            bold: false,
            italic: false,
        },
        None => Block::Paragraph {
            text: accum.inline,
            bold: false,
            italic: false,
        },
    }
}

/// Wrap `text` in markdown emphasis markers based on `bold`/`italic`. Both →
/// `***text***`. Headings deliberately skip this (the `#` is the emphasis).
fn wrap_emphasis(text: &str, bold: bool, italic: bool) -> String {
    if text.trim().is_empty() {
        return text.to_string();
    }
    match (bold, italic) {
        (true, true) => format!("***{text}***"),
        (true, false) => format!("**{text}**"),
        (false, true) => format!("*{text}*"),
        (false, false) => text.to_string(),
    }
}

/// A classified block plus the region of the page it was read from.
///
/// `bbox` is the union of the boxes of every source line that fed the block, so
/// a wrapped heading or a multi-line paragraph reports the whole band it
/// occupies. It is `None` only for blocks with no page geometry behind them —
/// synthesized content, or sources that never carried coordinates.
#[derive(Debug, Clone)]
pub struct PositionedBlock {
    pub block: Block,
    pub bbox: Option<Rect>,
    /// Indices of the source items this block's content was read from, into
    /// the page's *returned* `text_items` array (see
    /// `ProjectedLine::span_item_indices` for the index contract).
    /// Accumulated in insertion order here; the public `LayoutBlock`
    /// conversion sorts + dedups. Empty for blocks with no text behind them
    /// (rules, figures) and for paths not yet attributing provenance.
    pub text_item_indices: Vec<usize>,
}

impl PositionedBlock {
    pub fn new(block: Block, bbox: Option<Rect>, text_item_indices: Vec<usize>) -> Self {
        PositionedBlock {
            block,
            bbox,
            text_item_indices,
        }
    }

    /// A block with no known geometry (and no source-item attribution).
    pub fn unlocated(block: Block) -> Self {
        PositionedBlock {
            block,
            bbox: None,
            text_item_indices: Vec::new(),
        }
    }

    /// Grow this block's box to also cover `other`.
    fn absorb(&mut self, other: &Option<Rect>) {
        if let Some(r) = other {
            Rect::extend(&mut self.bbox, r);
        }
    }

    /// Union another block's source-item indices into this one (used when a
    /// pass fuses two blocks into one — the survivor covers both sources).
    fn absorb_indices(&mut self, other: &[usize]) {
        self.text_item_indices.extend_from_slice(other);
    }
}

/// Heal words hyphenated across a soft line wrap that the classifier split into
/// two *separate* paragraph blocks: `…they dis-` ‖ `lodged…`. When a plain
/// paragraph ends with `<letter>-` and the next is a plain paragraph starting
/// lowercase, fuse them — dropping the hyphen and joining with no separator —
/// and union their boxes, since the result spans both source regions.
///
/// This runs as a pass over the block list rather than inside the renderer so
/// that the blocks callers receive are the same ones that produced the
/// markdown. The lowercase and plain-text gates keep real compounds (`well-`
/// then a capitalized `Known`) and emphasised/heading/table starts intact.
pub fn splice_soft_hyphens(blocks: Vec<PositionedBlock>) -> Vec<PositionedBlock> {
    let mut out: Vec<PositionedBlock> = Vec::with_capacity(blocks.len());
    for pb in blocks {
        // Both sides must be unemphasised: an emphasised predecessor renders
        // with a trailing `**`/`*`, so its final character was never the
        // hyphen this splice keys off.
        let joinable = match (out.last().map(|p| &p.block), &pb.block) {
            (
                Some(Block::Paragraph {
                    text: prev,
                    bold: false,
                    italic: false,
                }),
                Block::Paragraph {
                    text,
                    bold: false,
                    italic: false,
                },
            ) => is_soft_hyphen_break(prev, text).then(|| text.clone()),
            _ => None,
        };
        if let Some(tail) = joinable {
            let prev = out.last_mut().expect("gate matched on a previous block");
            if let Block::Paragraph { text, .. } = &mut prev.block {
                while text.ends_with(|c: char| c.is_whitespace()) {
                    text.pop();
                }
                text.pop(); // the soft hyphen
                text.push_str(&tail);
            }
            prev.absorb(&pb.bbox);
            prev.absorb_indices(&pb.text_item_indices);
            continue;
        }
        out.push(pb);
    }
    out
}

/// Render a list of blocks to a markdown string.
pub fn render_blocks(blocks: &[PositionedBlock]) -> String {
    let mut out = String::new();
    for (i, positioned) in blocks.iter().enumerate() {
        let block = &positioned.block;
        if i > 0 {
            // Consecutive list items render as a tight list (single newline).
            // Everything else gets a blank line between blocks.
            let tight = matches!(block, Block::ListItem { .. })
                && matches!(blocks[i - 1].block, Block::ListItem { .. });
            if tight {
                out.push('\n');
            } else {
                out.push_str("\n\n");
            }
        }
        match block {
            Block::Heading { level, text } => {
                let level = (*level).clamp(1, 6) as usize;
                out.push_str(&"#".repeat(level));
                out.push(' ');
                out.push_str(text);
            }
            Block::Paragraph { text, bold, italic } => {
                out.push_str(&wrap_emphasis(text, *bold, *italic));
            }
            Block::ListItem {
                ordered,
                marker,
                level,
                text,
                bold,
                italic,
            } => {
                let indent = "  ".repeat((*level).min(6) as usize);
                out.push_str(&indent);
                if *ordered {
                    // Preserve the original marker (e.g. `138.` for footnotes
                    // or `iii)` for roman numerals) so semantic numbering
                    // survives the round-trip. CommonMark requires `1.` /
                    // `1)` style but most LLM consumers tolerate alt forms,
                    // and the alternative — renumbering as `1.` — drops info.
                    out.push_str(marker);
                    out.push(' ');
                } else {
                    out.push_str("- ");
                }
                out.push_str(&wrap_emphasis(text, *bold, *italic));
            }
            Block::Table { header, rows } => {
                // GFM requires a header row before the separator. When the
                // detector found no header, promote the first body row instead
                // of synthesizing a blank `|   |   |` header — a visible empty
                // row reads as sloppy output and carries no information.
                let (head, body): (Option<&[Cell]>, &[Vec<Cell>]) = match header {
                    Some(h) => (Some(h.as_slice()), rows.as_slice()),
                    None => match rows.split_first() {
                        Some((first, rest)) => (Some(first.as_slice()), rest),
                        None => (None, rows.as_slice()),
                    },
                };
                let column_count = head.map(|h| h.len()).unwrap_or(0);
                if column_count == 0 {
                    continue;
                }
                out.push_str("| ");
                for (i, cell) in head.unwrap().iter().enumerate() {
                    if i > 0 {
                        out.push_str(" | ");
                    }
                    out.push_str(&escape_table_cell(&cell.text));
                }
                out.push_str(" |\n");
                out.push('|');
                for _ in 0..column_count {
                    out.push_str("---|");
                }
                for row in body {
                    out.push_str("\n| ");
                    for (i, cell) in row.iter().enumerate() {
                        if i > 0 {
                            out.push_str(" | ");
                        }
                        out.push_str(&escape_table_cell(&cell.text));
                    }
                    out.push_str(" |");
                }
            }
            Block::GridFallback { lines } => {
                out.push_str("```text\n");
                for line in lines {
                    out.push_str(line);
                    out.push('\n');
                }
                out.push_str("```");
            }
            Block::CodeBlock { lines, lang } => {
                // Pick a fence that doesn't appear inside the body. Standard
                // triple-backtick handles ~all real-world code; bump to ~~~ if
                // the body itself contains ``` (rare).
                let fence = if lines.iter().any(|l| l.contains("```")) {
                    "~~~"
                } else {
                    "```"
                };
                out.push_str(fence);
                if let Some(lang) = lang {
                    out.push_str(lang);
                }
                out.push('\n');
                for line in lines {
                    out.push_str(line);
                    out.push('\n');
                }
                out.push_str(fence);
            }
            Block::HorizontalRule => {
                out.push_str("---");
            }
            Block::Figure { id, format } => {
                out.push_str("![](img_");
                out.push_str(id);
                out.push('.');
                out.push_str(format);
                out.push(')');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Render content blocks that carry no geometry — the shape almost every
    /// rendering assertion below cares about.
    fn render(blocks: Vec<Block>) -> String {
        let positioned: Vec<PositionedBlock> =
            blocks.into_iter().map(PositionedBlock::unlocated).collect();
        render_blocks(&positioned)
    }

    #[test]
    fn render_blocks_formats_markdown() {
        let blocks = vec![
            Block::Heading {
                level: 1,
                text: "Title".into(),
            },
            Block::Paragraph {
                text: "A paragraph.".into(),
                bold: false,
                italic: false,
            },
            Block::Heading {
                level: 2,
                text: "Sub".into(),
            },
        ];
        let s = render(blocks);
        assert_eq!(s, "# Title\n\nA paragraph.\n\n## Sub");
    }

    #[test]
    fn render_figure_uses_extracted_format() {
        assert_eq!(
            render(vec![Block::Figure {
                id: "p1_1".into(),
                format: "jpg".into(),
            }]),
            "![](img_p1_1.jpg)"
        );
    }

    #[test]
    fn render_lists_are_tight() {
        let blocks = vec![
            Block::Paragraph {
                text: "Intro.".into(),
                bold: false,
                italic: false,
            },
            Block::ListItem {
                ordered: false,
                marker: "•".into(),
                level: 0,
                text: "a".into(),
                bold: false,
                italic: false,
            },
            Block::ListItem {
                ordered: false,
                marker: "•".into(),
                level: 0,
                text: "b".into(),
                bold: false,
                italic: false,
            },
            Block::Paragraph {
                text: "Outro.".into(),
                bold: false,
                italic: false,
            },
        ];
        let s = render(blocks);
        assert_eq!(s, "Intro.\n\n- a\n- b\n\nOutro.");

        // Ordered: original marker preserved
        let s = render(vec![
            Block::ListItem {
                ordered: true,
                marker: "138.".into(),
                level: 0,
                text: "footnote".into(),
                bold: false,
                italic: false,
            },
            Block::ListItem {
                ordered: true,
                marker: "139.".into(),
                level: 0,
                text: "next footnote".into(),
                bold: false,
                italic: false,
            },
        ]);
        assert_eq!(s, "138. footnote\n139. next footnote");
    }

    #[test]
    fn render_emphasis_combinations() {
        assert_eq!(wrap_emphasis("hi", false, false), "hi");
        assert_eq!(wrap_emphasis("hi", true, false), "**hi**");
        assert_eq!(wrap_emphasis("hi", false, true), "*hi*");
        assert_eq!(wrap_emphasis("hi", true, true), "***hi***");
    }

    #[test]
    fn code_block_escapes_internal_fence() {
        let blocks = vec![Block::CodeBlock {
            lines: vec!["body containing ``` backticks".into()],
            lang: None,
        }];
        let s = render(blocks);
        assert!(s.starts_with("~~~\n"));
        assert!(s.ends_with("~~~"));
    }

    #[test]
    fn renders_table_to_pipe_format() {
        let blocks = vec![Block::Table {
            header: Some(vec!["a".into(), "b".into()]),
            rows: vec![vec!["1".into(), "2".into()], vec!["3".into(), "4".into()]],
        }];
        let s = render(blocks);
        assert_eq!(s, "| a | b |\n|---|---|\n| 1 | 2 |\n| 3 | 4 |");
    }

    #[test]
    fn splices_hyphen_split_across_paragraph_blocks() {
        let p = |t: &str| Block::Paragraph {
            text: t.into(),
            bold: false,
            italic: false,
        };
        let spliced = |a: &str, b: &str| {
            splice_soft_hyphens(vec![
                PositionedBlock::unlocated(p(a)),
                PositionedBlock::unlocated(p(b)),
            ])
        };
        let rendered = |a: &str, b: &str| render_blocks(&spliced(a, b));
        // Mid-word hyphen split into two paragraphs heals into one.
        assert_eq!(
            rendered("they dis-", "lodged the part"),
            "they dislodged the part"
        );
        // Capitalized continuation is a real compound break — left intact.
        assert_eq!(
            rendered("the well-", "Known fact"),
            "the well-\n\nKnown fact"
        );
        // Trailing dash not preceded by a letter doesn't splice.
        assert_eq!(rendered("a -", "dash line"), "a -\n\ndash line");
        // A spliced block covers both source regions; an unspliced pair stays
        // as two blocks with their own boxes.
        let r = |y: f32| Rect {
            x: 10.0,
            y,
            width: 100.0,
            height: 12.0,
        };
        let joined = splice_soft_hyphens(vec![
            PositionedBlock::new(p("they dis-"), Some(r(50.0)), vec![0, 1]),
            PositionedBlock::new(p("lodged the part"), Some(r(70.0)), vec![2, 3]),
        ]);
        assert_eq!(joined.len(), 1);
        let bbox = joined[0].bbox.clone().expect("merged block keeps geometry");
        assert_eq!((bbox.y, bbox.height), (50.0, 32.0));
        // The survivor also absorbs the spliced block's source items.
        assert_eq!(joined[0].text_item_indices, vec![0, 1, 2, 3]);
    }

    #[test]
    fn render_table_without_header_promotes_first_row() {
        let blocks = vec![Block::Table {
            header: None,
            rows: vec![vec!["h1".into(), "h2".into()], vec!["1".into(), "2".into()]],
        }];
        let s = render(blocks);
        // No blank `|   |   |` header: the first row becomes the header.
        assert_eq!(s, "| h1 | h2 |\n|---|---|\n| 1 | 2 |");
    }
}
