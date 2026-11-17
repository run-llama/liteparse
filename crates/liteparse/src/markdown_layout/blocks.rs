use super::inline::escape_inline;
use super::paragraphs::{ParaAccum, is_soft_hyphen_break};
use super::tables::escape_table_cell;
use crate::types::Rect;
use serde::{Deserialize, Serialize};

/// One table cell: rendered text plus, when the cell came from real page
/// content, the region it occupied. `bbox` is `None` for cells that exist only
/// to square off a ragged grid (padding inserted when rows disagree on column
/// count) — those occupy no ink on the page, and reporting a rect for them
/// would invent geometry the classifier never saw.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Cell {
    pub text: String,
    pub bbox: Option<Rect>,
}

impl Cell {
    /// Cell carrying text and the region it was read from.
    pub fn located(text: impl Into<String>, bbox: Rect) -> Self {
        Cell {
            text: text.into(),
            bbox: Some(bbox),
        }
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
        }
    }
}

impl From<String> for Cell {
    fn from(text: String) -> Self {
        Cell { text, bbox: None }
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

/// One cell of a [`Block::MergedTable`]. `colspan`/`rowspan` are 1 for an
/// ordinary cell; the cells they cover are absent from `rows` rather than
/// present-and-empty, matching HTML's occupancy model. Distinct from [`Cell`]:
/// that type describes a cell a classifier *read off the page*, this one a
/// merge structure a producer *declared* — but both carry the region the cell
/// occupies when the producer knows it, so merged tables ground exactly like
/// plain ones.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpanCell {
    pub text: String,
    pub colspan: u16,
    pub rowspan: u16,
    /// Region the cell occupies on the page (the *whole* merged region for a
    /// spanning cell), in the same viewport space as `text_items`. `None` when
    /// the producer has no geometry for it.
    pub bbox: Option<Rect>,
}

impl SpanCell {
    /// A plain 1x1 cell.
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            colspan: 1,
            rowspan: 1,
            bbox: None,
        }
    }

    /// A cell spanning `colspan` columns and `rowspan` rows. Zero is coerced to
    /// 1 — a zero span is meaningless in HTML and `rowspan="0"` means "to the
    /// end of the section", which is not what any producer here intends.
    pub fn spanning(text: impl Into<String>, colspan: u16, rowspan: u16) -> Self {
        Self {
            text: text.into(),
            colspan: colspan.max(1),
            rowspan: rowspan.max(1),
            bbox: None,
        }
    }

    /// Attach the region this cell occupies.
    pub fn with_bbox(mut self, bbox: Rect) -> Self {
        self.bbox = Some(bbox);
        self
    }

    fn is_plain(&self) -> bool {
        self.colspan <= 1 && self.rowspan <= 1
    }
}

// Like `Cell`, geometry is derived metadata, not identity: two cells with the
// same text and spans are the same table structure whether or not either one
// knows its coordinates, and assertions compare against plain literals.
impl PartialEq for SpanCell {
    fn eq(&self, other: &Self) -> bool {
        self.text == other.text && self.colspan == other.colspan && self.rowspan == other.rowspan
    }
}

impl Eq for SpanCell {}

/// Coarse block representation: the output of page classification, consumed by
/// `render_blocks` to produce the final markdown string.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// Table whose cells carry explicit spans. Emitted by producers that know
    /// the merge structure outright (e.g. a structural reader with access to
    /// `gridSpan` / `vMerge`) rather than inferring it from coordinates.
    ///
    /// Renders as a GFM pipe table when it degenerates to a plain grid, and as
    /// HTML `<table>` otherwise.
    MergedTable {
        rows: Vec<Vec<SpanCell>>,
        header_rows: usize,
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PositionedBlock {
    pub block: Block,
    pub bbox: Option<Rect>,
}

impl PositionedBlock {
    pub fn new(block: Block, bbox: Option<Rect>) -> Self {
        PositionedBlock { block, bbox }
    }

    /// A block with no known geometry.
    pub fn unlocated(block: Block) -> Self {
        PositionedBlock { block, bbox: None }
    }

    /// Grow this block's box to also cover `other`.
    fn absorb(&mut self, other: &Option<Rect>) {
        if let Some(r) = other {
            Rect::extend(&mut self.bbox, r);
        }
    }
}

/// Rejoin a paragraph that a figure cut in two. Figures are placed by their top edge, so one set
/// beside a paragraph lands between two of its lines ("... including ![](img) glasses, ...").
/// When a paragraph is followed by figures and then a paragraph that continues it (the first
/// does not end a sentence, the second starts lowercase, both share the paragraph style), the
/// halves join (de-hyphenated) and the figures follow the joined paragraph.
pub fn rejoin_paragraphs_around_figures(blocks: Vec<PositionedBlock>) -> Vec<PositionedBlock> {
    let mut out: Vec<PositionedBlock> = Vec::with_capacity(blocks.len());
    let mut it = blocks.into_iter().peekable();
    while let Some(pb) = it.next() {
        let Block::Paragraph { text, bold, italic } = pb.block else {
            out.push(pb);
            continue;
        };
        let (mut text, mut bbox) = (text, pb.bbox);
        let mut held: Vec<PositionedBlock> = Vec::new();
        loop {
            // figures directly after this paragraph
            let mut figs: Vec<PositionedBlock> = Vec::new();
            while it
                .peek()
                .is_some_and(|b| matches!(b.block, Block::Figure { .. }))
            {
                figs.push(it.next().unwrap());
            }
            let continues = !figs.is_empty()
                && !ends_sentence(&text)
                && it.peek().is_some_and(|b| match &b.block {
                    Block::Paragraph {
                        text: t,
                        bold: b2,
                        italic: i2,
                    } => *b2 == bold && *i2 == italic && starts_continuation(t),
                    _ => false,
                });
            if !continues {
                held.extend(figs);
                break;
            }
            let next = it.next().unwrap();
            if let Block::Paragraph { text: t, .. } = next.block {
                let raw = super::paragraphs::collapse_whitespace(&t);
                super::inline::append_inline_continuation(&mut text, &raw, &t);
            }
            if let Some(r) = &next.bbox {
                Rect::extend(&mut bbox, r);
            }
            held.extend(figs);
        }
        out.push(PositionedBlock {
            block: Block::Paragraph { text, bold, italic },
            bbox,
        });
        out.extend(held);
    }
    out
}

fn ends_sentence(text: &str) -> bool {
    let t = text
        .trim_end()
        .trim_end_matches(['*', '_', '~', '`', ')', '"', '\'', '”', '’', ']']);
    t.ends_with(['.', '!', '?', ':', ';']) || t.is_empty()
}

fn starts_continuation(text: &str) -> bool {
    text.trim_start()
        .trim_start_matches(['*', '_', '~'])
        .chars()
        .next()
        .is_some_and(|c| c.is_lowercase())
}

/// Split tables whose rows repeat an earlier column-header row: several tables with the same
/// columns, stacked (rate tables per scenario, one schedule per year), that table detection read
/// as one. A header-like row (three or more filled cells, at most a third of them numbers) that
/// repeats one of the current table's first three rows starts a new table, taking with it a
/// one-cell title row just above it ("Proposed Base Rate"). Tables set side by side with the
/// same columns split at their shared empty gutter column ([`side_by_side_gutter`]).
/// ParseBench v1 tables 65.6 -> 66.8; OPB, odl unchanged; ParseBench v2 tables 63.8 -> 63.7 core,
/// 55.3 -> 55.5 full.
pub fn split_repeated_header_tables(blocks: Vec<PositionedBlock>) -> Vec<PositionedBlock> {
    let mut out = Vec::with_capacity(blocks.len());
    for pb in blocks {
        match pb.block {
            Block::Table { header, rows } => {
                let mut all: Vec<Vec<Cell>> = header.into_iter().collect();
                let had_header = !all.is_empty();
                all.extend(rows);
                // side by side first: each half is then checked for stacked repeats
                if let Some(gutter) = side_by_side_gutter(&all) {
                    let (mut left, mut right): (Vec<Vec<Cell>>, Vec<Vec<Cell>>) =
                        (Vec::new(), Vec::new());
                    for mut r in all {
                        r.resize(r.len().max(gutter + 1), Cell::from(""));
                        let tail = r.split_off(gutter + 1);
                        r.truncate(gutter);
                        left.push(r);
                        right.push(tail);
                    }
                    for part in [left, right] {
                        let bbox =
                            union_cells(part.iter().flatten().filter_map(|c| c.bbox.as_ref()))
                                .or(pb.bbox.clone());
                        let mut it = part.into_iter();
                        let header = if had_header { it.next() } else { None };
                        out.extend(split_repeated_header_tables(vec![PositionedBlock {
                            block: Block::Table {
                                header,
                                rows: it.collect(),
                            },
                            bbox,
                        }]));
                    }
                    continue;
                }
                let texts: Vec<Vec<&str>> = all
                    .iter()
                    .map(|r| r.iter().map(|c| c.as_str()).collect())
                    .collect();
                let cuts = repeated_header_cuts(&texts, &vec![false; all.len()]);
                if cuts.is_empty() {
                    let mut it = all.into_iter();
                    let header = if had_header { it.next() } else { None };
                    out.push(PositionedBlock {
                        block: Block::Table {
                            header,
                            rows: it.collect(),
                        },
                        bbox: pb.bbox,
                    });
                    continue;
                }
                let mut start = 0;
                for (k, end) in cuts.iter().copied().chain([all.len()]).enumerate() {
                    let seg: Vec<Vec<Cell>> = all[start..end].to_vec();
                    let bbox = union_cells(seg.iter().flatten().filter_map(|c| c.bbox.as_ref()))
                        .or(pb.bbox.clone());
                    let mut it = seg.into_iter();
                    let header = if k > 0 || had_header { it.next() } else { None };
                    out.push(PositionedBlock {
                        block: Block::Table {
                            header,
                            rows: it.collect(),
                        },
                        bbox,
                    });
                    start = end;
                }
            }
            Block::MergedTable { rows, header_rows } => {
                // a cut may not fall inside a row span
                let mut spanned = vec![false; rows.len()];
                for (r, row) in rows.iter().enumerate() {
                    for c in row {
                        let end = (r + c.rowspan.max(1) as usize).min(rows.len());
                        for s in spanned.iter_mut().take(end).skip(r + 1) {
                            *s = true;
                        }
                    }
                }
                let texts: Vec<Vec<&str>> = rows
                    .iter()
                    .map(|r| r.iter().map(|c| c.text.as_str()).collect())
                    .collect();
                let cuts = repeated_header_cuts(&texts, &spanned);
                if cuts.is_empty() {
                    out.push(PositionedBlock {
                        block: Block::MergedTable { rows, header_rows },
                        bbox: pb.bbox,
                    });
                    continue;
                }
                let mut start = 0;
                for (k, end) in cuts.iter().copied().chain([rows.len()]).enumerate() {
                    let seg: Vec<Vec<SpanCell>> = rows[start..end].to_vec();
                    let bbox = union_cells(seg.iter().flatten().filter_map(|c| c.bbox.as_ref()))
                        .or(pb.bbox.clone());
                    // later parts: their title row and repeated header row head the part
                    let hr = if k == 0 {
                        header_rows.min(seg.len())
                    } else {
                        (end - start).min(
                            if seg
                                .first()
                                .is_some_and(|r| nonempty(r.iter().map(|c| c.text.as_str())) == 1)
                            {
                                2
                            } else {
                                1
                            },
                        )
                    };
                    out.push(PositionedBlock {
                        block: Block::MergedTable {
                            rows: seg,
                            header_rows: hr,
                        },
                        bbox,
                    });
                    start = end;
                }
            }
            block => out.push(PositionedBlock {
                block,
                bbox: pb.bbox,
            }),
        }
    }
    out
}

/// A column empty in every row that separates two tables set side by side: each side keeps at
/// least two columns and the sides' first rows share at least half of the smaller side's
/// non-empty header cells (the same columns repeated), so a spacer column inside one table
/// (empty between a currency sign and its figures) is never a cut.
fn side_by_side_gutter(rows: &[Vec<Cell>]) -> Option<usize> {
    let width = rows.iter().map(|r| r.len()).max()?;
    let empty = |c: usize| {
        rows.iter()
            .all(|r| r.get(c).is_none_or(|x| x.as_str().trim().is_empty()))
    };
    let norm = |s: &str| {
        s.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
    };
    let head = rows.first()?;
    (2..width.saturating_sub(2)).find(|&g| {
        if !empty(g) || (0..g).all(empty) || (g + 1..width).all(empty) {
            return false;
        }
        let left: Vec<String> = head
            .iter()
            .take(g)
            .map(|c| norm(c.as_str()))
            .filter(|s| !s.is_empty())
            .collect();
        let right: Vec<String> = head
            .iter()
            .skip(g + 1)
            .map(|c| norm(c.as_str()))
            .filter(|s| !s.is_empty())
            .collect();
        let shared = right.iter().filter(|s| left.contains(s)).count();
        left.len() >= 2 && right.len() >= 2 && shared * 2 >= left.len().min(right.len())
    })
}

fn nonempty<'a>(cells: impl Iterator<Item = &'a str>) -> usize {
    cells.filter(|c| !c.trim().is_empty()).count()
}

/// Row indices where a new table starts (see [`split_repeated_header_tables`]); a row with
/// `blocked[i]` (inside a row span) is never a cut.
fn repeated_header_cuts(rows: &[Vec<&str>], blocked: &[bool]) -> Vec<usize> {
    // a header row: three or more filled cells, mostly words (a repeated data row such as a
    // regression table's reference category, "Non-snacker | 1.00 | 1.00", is no header)
    let numeric = |c: &str| {
        let c = c.trim();
        !c.is_empty()
            && c.chars().any(|ch| ch.is_ascii_digit())
            && c.chars()
                .all(|ch| ch.is_ascii_digit() || " .,%$()-–+*/".contains(ch))
    };
    let key = |r: &[&str]| -> Option<String> {
        let filled = nonempty(r.iter().copied());
        let nums = r.iter().filter(|c| numeric(c)).count();
        (filled >= 3 && nums * 3 <= filled).then(|| {
            r.iter()
                .map(|c| {
                    c.split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" ")
                        .to_lowercase()
                })
                .collect::<Vec<_>>()
                .join("\u{1f}")
        })
    };
    let mut cuts = Vec::new();
    let mut seg_start = 0;
    for i in 1..rows.len() {
        let Some(k) = key(&rows[i]) else { continue };
        // the earlier copy heads the current table (its first three rows)
        if !(seg_start..i.min(seg_start + 3)).any(|j| key(&rows[j]).as_deref() == Some(k.as_str()))
        {
            continue;
        }
        // the title row above moves with the repeated header
        let cut = if i - 1 > seg_start && nonempty(rows[i - 1].iter().copied()) == 1 {
            i - 1
        } else {
            i
        };
        if cut > seg_start && !blocked[cut] {
            cuts.push(cut);
            seg_start = cut;
        }
    }
    cuts
}

fn union_cells<'a>(rects: impl Iterator<Item = &'a Rect>) -> Option<Rect> {
    let mut b: Option<(f32, f32, f32, f32)> = None;
    for r in rects {
        let (x0, y0, x1, y1) = (r.x, r.y, r.x + r.width, r.y + r.height);
        b = Some(match b {
            None => (x0, y0, x1, y1),
            Some((a, c, d, e)) => (a.min(x0), c.min(y0), d.max(x1), e.max(y1)),
        });
    }
    b.map(|(x0, y0, x1, y1)| Rect {
        x: x0,
        y: y0,
        width: x1 - x0,
        height: y1 - y0,
    })
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
            continue;
        }
        out.push(pb);
    }
    out
}

/// Render a GFM pipe table. `head` is the header row (already chosen by the
/// caller); `body` the remaining rows. No-ops when there are no columns.
fn render_pipe_table(head: Option<&[Cell]>, body: &[Vec<Cell>], out: &mut String) {
    let column_count = head.map(|h| h.len()).unwrap_or(0);
    if column_count == 0 {
        return;
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

/// Escape a cell for HTML `<td>` content. Markdown specials are deliberately
/// left alone — inside an HTML block they are literal text, and escaping them
/// would surface backslashes in the rendered output.
fn escape_html_cell(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\n' => out.push(' '),
            _ => out.push(c),
        }
    }
    out
}

/// True when a merged table degenerates to a plain rectangular grid, in which
/// case the pipe rendering is both valid and nicer to read.
fn is_plain_grid(rows: &[Vec<SpanCell>], header_rows: usize) -> bool {
    // GFM has exactly one header row, or none.
    if header_rows > 1 {
        return false;
    }
    if !rows.iter().all(|r| r.iter().all(SpanCell::is_plain)) {
        return false;
    }
    // Ragged rows can only have come from spans we've just ruled out, but check
    // anyway: a pipe table with uneven rows renders as a broken table.
    let width = rows.first().map(|r| r.len()).unwrap_or(0);
    rows.iter().all(|r| r.len() == width)
}

/// Render a merged table as HTML. Cells covered by a neighbour's span are
/// simply absent from `rows`, so this is a direct transcription.
fn render_html_table(rows: &[Vec<SpanCell>], header_rows: usize, out: &mut String) {
    out.push_str("<table>");
    for (r, row) in rows.iter().enumerate() {
        out.push_str("\n<tr>");
        let tag = if r < header_rows { "th" } else { "td" };
        for cell in row {
            out.push_str("\n<");
            out.push_str(tag);
            if cell.colspan > 1 {
                out.push_str(&format!(" colspan=\"{}\"", cell.colspan));
            }
            if cell.rowspan > 1 {
                out.push_str(&format!(" rowspan=\"{}\"", cell.rowspan));
            }
            out.push('>');
            out.push_str(&escape_html_cell(&cell.text));
            out.push_str("</");
            out.push_str(tag);
            out.push('>');
        }
        out.push_str("\n</tr>");
    }
    out.push_str("\n</table>");
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
                render_pipe_table(head, body, &mut out);
            }
            Block::MergedTable { rows, header_rows } => {
                if rows.is_empty() {
                    continue;
                }
                if is_plain_grid(rows, *header_rows) {
                    // Degenerate case — no merges, so pipes are expressive
                    // enough. Reuse the exact same rendering `Block::Table`
                    // gets, including the promote-first-row-when-headerless
                    // rule.
                    let plain: Vec<Vec<Cell>> = rows
                        .iter()
                        .map(|r| r.iter().map(|c| Cell::from(c.text.clone())).collect())
                        .collect();
                    // `header_rows` is 0 or 1 here (>1 forces HTML). Either way
                    // the first row leads: as the real header, or promoted into
                    // one because GFM has no headerless table.
                    let (head, body) = match plain.split_first() {
                        Some((first, rest)) => (Some(first.as_slice()), rest),
                        None => (None, plain.as_slice()),
                    };
                    render_pipe_table(head, body, &mut out);
                } else {
                    render_html_table(rows, *header_rows, &mut out);
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

    fn grid(rows: &[&[&str]]) -> PositionedBlock {
        let mut it = rows
            .iter()
            .map(|r| r.iter().map(|c| Cell::from(*c)).collect::<Vec<_>>());
        let header = it.next();
        PositionedBlock::new(
            Block::Table {
                header,
                rows: it.collect(),
            },
            None,
        )
    }

    fn table_count(blocks: &[PositionedBlock]) -> usize {
        blocks
            .iter()
            .filter(|b| matches!(b.block, Block::Table { .. }))
            .count()
    }

    fn para(text: &str) -> PositionedBlock {
        PositionedBlock::new(
            Block::Paragraph {
                text: text.into(),
                bold: false,
                italic: false,
            },
            None,
        )
    }

    fn fig(id: &str) -> PositionedBlock {
        PositionedBlock::new(
            Block::Figure {
                id: id.into(),
                format: "png".into(),
            },
            None,
        )
    }

    #[test]
    fn figure_inside_a_sentence_moves_after_the_paragraph() {
        let out = rejoin_paragraphs_around_figures(vec![
            para("Add the ability to engrave items, including"),
            fig("a"),
            fig("b"),
            para("glasses, bottles, and more."),
        ]);
        assert_eq!(out.len(), 3);
        let Block::Paragraph { text, .. } = &out[0].block else {
            unreachable!()
        };
        assert_eq!(
            text,
            "Add the ability to engrave items, including glasses, bottles, and more."
        );
        assert!(
            matches!(out[1].block, Block::Figure { .. })
                && matches!(out[2].block, Block::Figure { .. })
        );
        // between complete sentences (or before a capitalised start) the figure stays put
        let kept = rejoin_paragraphs_around_figures(vec![
            para("One sentence."),
            fig("a"),
            para("next one."),
        ]);
        assert_eq!(kept.len(), 3);
        let kept = rejoin_paragraphs_around_figures(vec![
            para("A list of"),
            fig("a"),
            para("New section"),
        ]);
        assert_eq!(kept.len(), 3);
    }

    #[test]
    fn repeated_header_row_starts_a_new_table() {
        let b = grid(&[
            &["", "Current Base Rate", "", ""],
            &["CA", "PPO 1500", "PPO 2500", "HDHP 3000"],
            &["Block A", "254.96", "207.94", "173.76"],
            &["", "Proposed Base Rate", "", ""],
            &["CA", "PPO 1500", "PPO 2500", "HDHP 3000"],
            &["Block A", "284.28", "231.85", "193.74"],
        ]);
        let out = split_repeated_header_tables(vec![b]);
        assert_eq!(table_count(&out), 2);
        let Block::Table { header, rows } = &out[1].block else {
            unreachable!()
        };
        assert_eq!(header.as_ref().unwrap()[1].as_str(), "Proposed Base Rate");
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn repeated_data_row_is_not_a_header() {
        // a regression table's reference category repeats in every group: one table
        let b = grid(&[
            &["", "OR", "95% CI", "OR"],
            &["Non-snacker", "1.00", "", "1.00"],
            &["Low snacker", "0.68", "0.44-1.05", "0.80"],
            &["Non-snacker", "1.00", "", "1.00"],
            &["Low snacker", "0.96", "0.70-1.30", "0.36"],
        ]);
        assert_eq!(table_count(&split_repeated_header_tables(vec![b])), 1);
    }

    #[test]
    fn side_by_side_tables_split_at_their_gutter() {
        let b = grid(&[
            &["Tier", "Type", "Rate", "", "Tier", "Type", "Rate"],
            &["001", "AN", "1.000", "", "001", "AN", "1.000"],
            &["001", "AN", "0.170", "", "001", "AN", "0.210"],
        ]);
        assert_eq!(table_count(&split_repeated_header_tables(vec![b])), 2);
        // an empty spacer column inside one table (different headers either side) stays
        let c = grid(&[
            &["Item", "2024", "", "2023", "Change"],
            &["Revenue", "10", "", "9", "1"],
        ]);
        assert_eq!(table_count(&split_repeated_header_tables(vec![c])), 1);
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
            PositionedBlock::new(p("they dis-"), Some(r(50.0))),
            PositionedBlock::new(p("lodged the part"), Some(r(70.0))),
        ]);
        assert_eq!(joined.len(), 1);
        let bbox = joined[0].bbox.clone().expect("merged block keeps geometry");
        assert_eq!((bbox.y, bbox.height), (50.0, 32.0));
    }

    #[test]
    fn merged_table_without_spans_renders_as_pipes() {
        // A merge-aware producer that happens to emit a plain grid must be
        // byte-identical to the PDF path's pipe table — otherwise the same
        // table looks different depending on which pipeline produced it.
        let plain = Block::Table {
            header: Some(vec!["a".into(), "b".into()]),
            rows: vec![vec!["1".into(), "2".into()]],
        };
        let merged = Block::MergedTable {
            header_rows: 1,
            rows: vec![
                vec![SpanCell::new("a"), SpanCell::new("b")],
                vec![SpanCell::new("1"), SpanCell::new("2")],
            ],
        };
        assert_eq!(render(vec![plain]), render(vec![merged]));
    }

    #[test]
    fn merged_table_with_spans_renders_as_html() {
        // Cells covered by a span are absent, not empty — row 1 has one cell.
        let s = render(vec![Block::MergedTable {
            header_rows: 1,
            rows: vec![
                vec![SpanCell::spanning("wide", 2, 1)],
                vec![SpanCell::spanning("tall", 1, 2), SpanCell::new("x")],
                vec![SpanCell::new("y")],
            ],
        }]);
        assert_eq!(
            s,
            "<table>\n<tr>\n<th colspan=\"2\">wide</th>\n</tr>\n\
             <tr>\n<td rowspan=\"2\">tall</td>\n<td>x</td>\n</tr>\n\
             <tr>\n<td>y</td>\n</tr>\n</table>"
        );
    }

    #[test]
    fn merged_table_escapes_html_not_markdown() {
        // Inside an HTML block, markdown specials are literal; `<` is not.
        let s = render(vec![Block::MergedTable {
            header_rows: 0,
            rows: vec![vec![
                SpanCell::new("a|b *c* <d> &e"),
                SpanCell::spanning("f", 2, 1),
            ]],
        }]);
        assert!(s.contains("<td>a|b *c* &lt;d&gt; &amp;e</td>"), "{s}");
    }

    #[test]
    fn merged_table_multiple_header_rows_force_html() {
        // Two header rows have no pipe-table representation even with no spans.
        let s = render(vec![Block::MergedTable {
            header_rows: 2,
            rows: vec![
                vec![SpanCell::new("a"), SpanCell::new("b")],
                vec![SpanCell::new("c"), SpanCell::new("d")],
                vec![SpanCell::new("1"), SpanCell::new("2")],
            ],
        }]);
        assert!(s.starts_with("<table>"), "{s}");
        assert_eq!(s.matches("<th>").count(), 4);
        assert_eq!(s.matches("<td>").count(), 2);
    }

    #[test]
    fn merged_table_ragged_rows_force_html() {
        // Uneven row widths with no declared spans would render as a broken
        // pipe table; HTML at least preserves the cells.
        let s = render(vec![Block::MergedTable {
            header_rows: 1,
            rows: vec![
                vec![SpanCell::new("a"), SpanCell::new("b")],
                vec![SpanCell::new("1")],
            ],
        }]);
        assert!(s.starts_with("<table>"), "{s}");
    }

    #[test]
    fn zero_span_is_coerced_to_one() {
        // `rowspan="0"` means "to the end of the section" in HTML — never what
        // a producer means here, so it must not survive into the output.
        let c = SpanCell::spanning("x", 0, 0);
        assert_eq!((c.colspan, c.rowspan), (1, 1));
        assert!(c.is_plain());
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
