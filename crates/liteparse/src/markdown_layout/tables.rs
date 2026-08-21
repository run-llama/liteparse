use crate::types::{GraphicPrimitive, ProjectedLine, Rect, TextItem};

use super::blocks::{Block, Cell};
use super::paragraphs::collapse_whitespace;
use crate::projection::is_bold_item;

/// Minimum cells per row for a region to qualify as a table.
pub(super) const TABLE_MIN_COLUMNS: usize = 3;

/// Minimum consecutive rows for a region to qualify as a table.
const TABLE_MIN_ROWS: usize = 2;

/// Gap between adjacent spans (in multiples of dominant font size) above which
/// we treat the gap as a cell boundary.
const TABLE_CELL_GAP_FONT_MULTIPLIER: f32 = 1.0;

/// Tolerance (points) for matching a cell's start-x to an existing column
/// track when extending a candidate table run.
const TABLE_TRACK_TOLERANCE_PT: f32 = 6.0;

/// Minimum gap (in multiples of the dominant font size) required between two
/// adjacent column tracks for them to count as distinct columns. The two
/// callers — inferred-track detection and the ruled-grid track derivation —
/// must use the same value or one stage can fuse columns the other split.
const TABLE_MIN_TRACK_GAP_FONT_MULT: f32 = 1.5;
/// Absolute floor (points) on the inter-track gap, paired with
/// `TABLE_MIN_TRACK_GAP_FONT_MULT` so tiny-font pages keep a sane minimum.
const TABLE_MIN_TRACK_GAP_FLOOR_PT: f32 = 12.0;

/// Minimum fraction of a row's cells that must carry text for that row to
/// anchor the header/body split in ruled-grid collapse.
const TABLE_ROW_MIN_FILL: f32 = 0.9;

/// Two horizontal rules closer than this bound a heading underline or a boxed
/// caption, not a table body.
const RULE_BAND_MIN_HEIGHT_PT: f32 = 10.0;

/// A rule band with fewer lines than this is a rule under a heading or a run of
/// hyperlink underlines - never a table.
const RULE_BAND_MIN_LINES: usize = 3;

/// Floor for the sparse-new-row path: a partial-cell line whose bottom-gap
/// exceeds this fraction qualifies as a real new row (with empty cells at
/// missing tracks) instead of being treated as a wrap continuation. Below
/// this fraction, the existing wrap-merge path runs unchanged.
const TABLE_SPARSE_ROW_MIN_BOTTOM_GAP_FRAC: f32 = 0.5;

/// Maximum vertical gap between consecutive table rows, expressed in multiples
/// of the line height. Looser than the paragraph rule because table rows often
/// have more vertical padding than prose lines.
const TABLE_ROW_GAP_MULTIPLIER: f32 = 2.5;

/// Maximum coefficient-of-variation for row spacing within a confident table
/// (rejecting irregular spacing that's more likely prose or a footer block).
const TABLE_ROW_SPACING_MAX_CV: f32 = 0.5;

/// Minimum ratio between the smallest "wide" inter-word gap and the largest
/// ordinary one for a span's internal gaps to read as bimodal - i.e. for the
/// wide ones to be column gutters rather than stretched justification spaces.
///
/// Measured separation is large: real gutters run 3.4–4.4× the in-cell word
/// gap (7.29 vs 2.13 on a booktabs worksheet header, 7.73 vs 1.76 on a
/// two-column-label results table), while fully-justified prose - the one
/// thing that reliably counterfeits column structure - tops out around 1.3×.
const SPAN_GUTTER_MIN_RATIO: f32 = 2.5;

/// Absolute floor (points) on a gap treated as an in-span column gutter, so
/// tightly-kerned small text can't manufacture columns out of a 1pt jitter.
const SPAN_GUTTER_MIN_PT: f32 = 3.0;

/// Floor on the *ordinary* side of the bimodality ratio, as a fraction of the
/// run's font size. The ratio is meant to compare candidate gutters against
/// typical word spacing, but the sorted-gap scan takes whatever gap sits below
/// the jump - and one degenerate sub-point gap (kerning jitter, a run-on
/// superscript) would make an ordinary 3pt word space clear the ratio and read
/// as a gutter tier of its own. A word space in `f`pt type is never far below
/// `0.2 × f`, so nothing smaller may serve as the comparison base.
const SPAN_GUTTER_MIN_ORDINARY_EM: f32 = 0.2;

/// One horizontal piece of a PDFium text run: the run split at its internal
/// column gutters. Most runs yield a single piece.
#[derive(Debug, Clone)]
pub(super) struct SpanPiece<'a> {
    pub(super) x: f32,
    pub(super) end_x: f32,
    pub(super) text: String,
    /// The run this piece came from - for bold/font lookups.
    pub(super) span: &'a TextItem,
    /// The piece's own word boxes, in reading order, but **only** when they
    /// were verified to reconstruct `text` exactly. Empty otherwise, so a
    /// consumer can treat non-empty as "real geometry for every word here".
    pub(super) words: Vec<&'a crate::types::WordBox>,
    /// Index of the run's source item in the page's *returned* `text_items`
    /// array (the `ProjectedLine::span_item_indices` contract). Stamped by
    /// `line_pieces` from the owning line; `None` when the piece was built
    /// without line context (or the line's index vector was misaligned, in
    /// which case the piece degrades to carrying no attribution rather than
    /// dropping the span).
    pub(super) source_index: Option<usize>,
}

/// A run's own word boxes, in reading order, but only when they reconstruct the
/// run's text exactly. Anything less and the boxes can't be used to slice the
/// text, because the split points wouldn't correspond to it.
///
/// The x-filter matters: projection can append a merged neighbour's word boxes
/// onto a run without re-slicing the text, making `span.words` a superset.
pub(super) fn verified_words(span: &TextItem) -> Option<Vec<&crate::types::WordBox>> {
    let x1 = span.x + span.width.max(0.0);
    let words: Vec<&crate::types::WordBox> = span
        .words
        .iter()
        .filter(|w| !w.text.trim().is_empty() && w.x >= span.x - 1.0 && w.x <= x1 + 1.0)
        .collect();
    if words.is_empty() {
        return None;
    }
    let rebuilt = collapse_whitespace(
        &words
            .iter()
            .map(|w| w.text.trim())
            .collect::<Vec<_>>()
            .join(" "),
    );
    (rebuilt == collapse_whitespace(span.text.trim())).then_some(words)
}

/// Split one PDFium run into column pieces at its internal gutters.
///
/// PDFium routinely emits a whole table row (several cells) as a single run,
/// which is the root of the track chicken-and-egg: the detector needs tracks to
/// split runs, but the runs are where the track evidence lives. Word boxes
/// break the cycle, because the gutter is plainly visible in the geometry
/// before any table hypothesis exists.
///
/// Detection is *relative to the run itself*, never a constant threshold: sort
/// the inter-word gaps, find the largest ratio jump, and accept the split only
/// when that jump clears [`SPAN_GUTTER_MIN_RATIO`]. A fixed gap threshold
/// cannot work here: the same 4pt gap is a gutter in 6pt type and an ordinary
/// space in 12pt type.
///
/// Returns a single piece covering the whole run when it has no word boxes,
/// fewer than three words, or no bimodal gap.
pub(super) fn split_span_at_gutters(span: &TextItem) -> Vec<SpanPiece<'_>> {
    // Unverified word boxes leave `words` empty, so `whole()` then carries no
    // word geometry, exactly as the `SpanPiece::words` contract requires.
    let words: Vec<&crate::types::WordBox> = verified_words(span).unwrap_or_default();
    let whole = || {
        vec![SpanPiece {
            x: span.x,
            end_x: span.x + span.width.max(0.0),
            text: collapse_whitespace(span.text.trim()),
            span,
            words: words.clone(),
            source_index: None,
        }]
    };
    // Two words give exactly one gap, which is trivially "the largest": no
    // ratio to test against, so there is no evidence of bimodality. The piece
    // still carries its words, so a track-driven split can use them later.
    if words.len() < 3 {
        return whole();
    }

    let gaps: Vec<f32> = words
        .windows(2)
        .map(|w| w[1].x - (w[0].x + w[0].width.max(0.0)))
        .collect();
    let mut sorted = gaps.clone();
    sorted.sort_by(f32::total_cmp);
    // Largest ratio jump between adjacent sorted gaps: the boundary between
    // "ordinary space" and "gutter", if there is one.
    let font = span.font_size.unwrap_or(span.height).max(1.0);
    let lo_floor = font * SPAN_GUTTER_MIN_ORDINARY_EM;
    let mut cut = None;
    let mut best_ratio = SPAN_GUTTER_MIN_RATIO;
    for i in 0..sorted.len() - 1 {
        let (lo, hi) = (sorted[i], sorted[i + 1]);
        if hi < SPAN_GUTTER_MIN_PT {
            continue;
        }
        let ratio = hi / lo.max(lo_floor);
        if ratio >= best_ratio {
            best_ratio = ratio;
            cut = Some(hi);
        }
    }
    let Some(threshold) = cut else {
        return whole();
    };

    let mut pieces: Vec<SpanPiece<'_>> = Vec::new();
    let mut current: Vec<&crate::types::WordBox> = vec![words[0]];
    for (i, gap) in gaps.iter().enumerate() {
        if *gap >= threshold {
            pieces.push(piece_from_words(&current, span));
            current.clear();
        }
        current.push(words[i + 1]);
    }
    pieces.push(piece_from_words(&current, span));
    pieces.retain(|p| !p.text.is_empty());
    if pieces.len() < 2 {
        return whole();
    }
    if *super::flags::DEBUG_GUTTER {
        eprintln!(
            "[gutter] thr={:.2} ratio={:.2} {:?}",
            threshold,
            best_ratio,
            pieces.iter().map(|p| p.text.as_str()).collect::<Vec<_>>()
        );
    }
    pieces
}

fn piece_from_words<'a>(words: &[&'a crate::types::WordBox], span: &'a TextItem) -> SpanPiece<'a> {
    let x = words.first().map_or(span.x, |w| w.x);
    let end_x = words
        .last()
        .map_or(span.x + span.width.max(0.0), |w| w.x + w.width.max(0.0));
    SpanPiece {
        x,
        end_x,
        text: collapse_whitespace(
            &words
                .iter()
                .map(|w| w.text.trim())
                .collect::<Vec<_>>()
                .join(" "),
        ),
        span,
        words: words.to_vec(),
        source_index: None,
    }
}

/// All column pieces on a line, left to right. This is the tabular view of a
/// row: what PDFium calls a run is not what the page calls a cell.
pub(super) fn line_pieces(line: &ProjectedLine) -> Vec<SpanPiece<'_>> {
    let mut pieces: Vec<SpanPiece<'_>> = Vec::new();
    for (i, s) in line.spans.iter().enumerate() {
        if s.text.trim().is_empty() {
            continue;
        }
        // Positional zip with the aligned index vector. `.get` so a length
        // mismatch degrades to an unattributed piece instead of dropping the
        // span (or panicking) — text fidelity outranks provenance.
        let source_index = line.span_item_indices.get(i).copied();
        for mut piece in split_span_at_gutters(s) {
            piece.source_index = source_index;
            pieces.push(piece);
        }
    }
    pieces.sort_by(|a, b| a.x.total_cmp(&b.x));
    pieces
}

/// `line_pieces` for every line up front. The detection scan tries most lines
/// as a seed and re-reads its neighbours' pieces for each try, so computing
/// them per call would redo the same splits O(lines × window) times.
fn compute_line_pieces(lines: &[ProjectedLine]) -> Vec<Vec<SpanPiece<'_>>> {
    lines.iter().map(line_pieces).collect()
}

/// One cell within a tabular row: contributing spans aggregated to text and
/// its leftmost x position, used to align cells across rows into column
/// "tracks".
#[derive(Debug, Clone)]
pub(super) struct TableCell {
    pub(super) start_x: f32,
    /// Right edge of the cell (x of the last span's right). Used by
    /// `recover_merged_cell` to detect cells that straddle two column tracks
    /// when the projection merged two adjacent words into one span.
    pub(super) end_x: f32,
    pub(super) text: String,
    pub(super) bold: bool,
    /// Source-item indices of the spans/pieces aggregated into this cell, in
    /// insertion (reading) order — the returned-text-items contract of
    /// `ProjectedLine::span_item_indices`. A piece split across tracks
    /// duplicates its index into each receiving cell; padding cells carry
    /// none.
    pub(super) text_item_indices: Vec<usize>,
}

/// A contiguous tabular run: line indices `[start, end)` plus the detected
/// rows. Used so the line-classifier can skip the consumed range and so
/// fallback rendering can reach back for the original projected text.
#[derive(Debug, Clone)]
pub(super) struct TableRun {
    pub(super) start: usize,
    pub(super) end: usize,
    /// First line index of the table's *body* rows. Differs from `start` when
    /// header lines above the body were absorbed into the run; the cluster
    /// re-extraction pass must not re-bin those header lines as body rows.
    pub(super) body_start: usize,
    pub(super) block: Block,
    /// Region the whole table occupies. For ruled tables this is the drawn
    /// grid; for the borderless detectors it is the union of the lines the run
    /// consumed. `None` when the run was built without geometry (tests, and
    /// synthetic merges with nothing to measure).
    pub(super) bbox: Option<Rect>,
    /// Block-level source-item indices for runs whose block does NOT carry
    /// cells (`GridFallback`): the union of the consumed lines' indices.
    /// Empty for `Table` runs — their block indices derive from the cells via
    /// [`TableRun::block_item_indices`], so there is no second copy to fall
    /// out of sync.
    pub(super) block_indices: Vec<usize>,
}

impl TableRun {
    /// Source-item indices the emitted block should carry: for `Table`, the
    /// concatenation of every header/body cell's indices (dedup happens at
    /// the public-DTO conversion); otherwise the run-carried union.
    pub(super) fn block_item_indices(&self) -> Vec<usize> {
        match &self.block {
            Block::Table { header, rows } => {
                let mut out: Vec<usize> = Vec::new();
                if let Some(h) = header {
                    for c in h {
                        out.extend_from_slice(&c.text_item_indices);
                    }
                }
                for row in rows {
                    for c in row {
                        out.extend_from_slice(&c.text_item_indices);
                    }
                }
                out
            }
            _ => self.block_indices.clone(),
        }
    }
}

/// Union of the boxes of `lines[start..end]` — the region a table run covers
/// when it has no drawn grid to fall back on.
fn lines_bbox(lines: &[ProjectedLine], start: usize, end: usize) -> Option<Rect> {
    let mut out: Option<Rect> = None;
    for l in lines.get(start..end.min(lines.len()))? {
        Rect::extend(&mut out, &l.bbox);
    }
    out
}

/// Region occupied by one borderless-table cell: its own x-extent (tracked by
/// `split_cells` from the spans it absorbed) crossed with the vertical band of
/// the line it sits on. A borderless table has no ruled boundaries, so a cell
/// covers exactly the ink of its spans — not a full column track.
fn cell_rect(cell: &TableCell, line: &ProjectedLine) -> Rect {
    Rect {
        x: cell.start_x,
        y: line.bbox.y,
        width: (cell.end_x - cell.start_x).max(0.0),
        height: line.bbox.height,
    }
}

/// Build a located `Cell` from a `TableCell` and its owning line, carrying
/// the cell's source-item attribution.
fn located_cell(cell: &TableCell, line: &ProjectedLine) -> Cell {
    Cell::located_with(
        cell.text.clone(),
        cell_rect(cell, line),
        cell.text_item_indices.clone(),
    )
}

/// Split a `ProjectedLine`'s spans into cells. A gap larger than
/// `TABLE_CELL_GAP_FONT_MULTIPLIER × font_size` between adjacent spans starts
/// a new cell; otherwise spans join into the same cell with a single space.
pub(super) fn split_cells(line: &ProjectedLine) -> Vec<TableCell> {
    // Skip whitespace-only spans before computing gaps — leading/trailing
    // empty items would otherwise add spurious cell boundaries. Each span is
    // paired with its source-item index (positional zip, `.get` so a
    // misaligned index vector degrades to unattributed spans, never dropped
    // ones).
    let mut spans: Vec<(&TextItem, Option<usize>)> = line
        .spans
        .iter()
        .enumerate()
        .filter(|(_, s)| !s.text.trim().is_empty())
        .map(|(i, s)| (s, line.span_item_indices.get(i).copied()))
        .collect();
    spans.sort_by(|a, b| a.0.x.total_cmp(&b.0.x));
    if spans.is_empty() {
        return Vec::new();
    }
    let font_size = if line.dominant_font_size > 0.0 {
        line.dominant_font_size
    } else {
        line.bbox.height.max(1.0)
    };
    let gap_threshold = font_size * TABLE_CELL_GAP_FONT_MULTIPLIER;

    let mut cells: Vec<TableCell> = Vec::new();
    let mut current_text = String::new();
    let mut current_start = spans[0].0.x;
    let mut current_bold_chars: usize = 0;
    let mut current_total_chars: usize = 0;
    let mut current_indices: Vec<usize> = Vec::new();
    let mut prev_right = spans[0].0.x;

    for (i, (span, source_index)) in spans.iter().enumerate() {
        let gap = span.x - prev_right;
        let break_cell = i > 0 && gap > gap_threshold;
        if break_cell {
            let bold = current_total_chars > 0 && current_bold_chars * 2 > current_total_chars;
            cells.push(TableCell {
                start_x: current_start,
                end_x: prev_right,
                text: collapse_whitespace(current_text.trim()),
                bold,
                text_item_indices: std::mem::take(&mut current_indices),
            });
            current_text.clear();
            current_start = span.x;
            current_bold_chars = 0;
            current_total_chars = 0;
        }
        if !current_text.is_empty() && !current_text.ends_with(' ') {
            current_text.push(' ');
        }
        current_text.push_str(&span.text);
        if let Some(idx) = source_index {
            current_indices.push(*idx);
        }
        let n = span.text.chars().count();
        current_total_chars += n;
        if is_bold_item(span) {
            current_bold_chars += n;
        }
        prev_right = span.x + span.width.max(0.0);
    }
    if !current_text.trim().is_empty() {
        let bold = current_total_chars > 0 && current_bold_chars * 2 > current_total_chars;
        cells.push(TableCell {
            start_x: current_start,
            end_x: prev_right,
            text: collapse_whitespace(current_text.trim()),
            bold,
            text_item_indices: current_indices,
        });
    }
    cells
}

/// When a candidate row has fewer cells than the established column count,
/// look for cells whose x-range straddles multiple column tracks (likely two
/// or more adjacent words that PDFium merged into a single text run) and
/// split each on internal whitespace at the boundaries nearest to the
/// straddled tracks.
///
/// Returns the patched cells if every short cell could be cleanly split to
/// recover `tracks.len()` cells total; otherwise `None`.
fn recover_merged_cell(mut cells: Vec<TableCell>, tracks: &[f32]) -> Option<Vec<TableCell>> {
    let target = tracks.len();
    if cells.len() >= target {
        return None;
    }
    // Repeatedly find the cell that straddles the most tracks (≥2) and split
    // it. Each iteration strictly grows `cells.len()`, so termination is
    // guaranteed; if no cell straddles ≥2 tracks before we hit the target,
    // recovery fails.
    while cells.len() < target {
        let mut best_i: Option<usize> = None;
        let mut best_count: usize = 1;
        let mut best_contained: Vec<f32> = Vec::new();
        for (i, cell) in cells.iter().enumerate() {
            let contained: Vec<f32> = tracks
                .iter()
                .copied()
                .filter(|t| {
                    *t >= cell.start_x - TABLE_TRACK_TOLERANCE_PT
                        && *t <= cell.end_x + TABLE_TRACK_TOLERANCE_PT
                })
                .collect();
            if contained.len() > best_count {
                best_count = contained.len();
                best_i = Some(i);
                best_contained = contained;
            }
        }
        let i = best_i?;
        let cell = cells[i].clone();
        // Split the merged cell text at each contained track after the first.
        let pieces = split_text_at_x_anchors(
            cell.text.trim(),
            cell.start_x,
            cell.end_x - cell.start_x,
            &best_contained[1..],
        )?;
        if pieces.iter().any(|p| p.is_empty()) {
            return None;
        }
        // Synthesize new TableCells aligned with each track.
        let mut new_cells: Vec<TableCell> = Vec::with_capacity(pieces.len());
        for (p, piece) in pieces.iter().enumerate() {
            let start_x = if p == 0 {
                cell.start_x
            } else {
                best_contained[p]
            };
            let end_x = if p + 1 < best_contained.len() {
                (best_contained[p + 1] - 1.0).max(start_x)
            } else {
                cell.end_x
            };
            new_cells.push(TableCell {
                start_x,
                end_x,
                text: piece.clone(),
                bold: cell.bold,
                // The split position inside a merged span is an estimate, so
                // each derived cell attributes to the full source set of the
                // cell it was carved from.
                text_item_indices: cell.text_item_indices.clone(),
            });
        }
        cells.remove(i);
        for (offset, c) in new_cells.into_iter().enumerate() {
            cells.insert(i + offset, c);
        }
    }
    if cells.len() == target {
        Some(cells)
    } else {
        None
    }
}

/// Vertical-gap check for table rows. Looser than paragraph continuation
/// because table rows often have extra padding between them.
fn table_rows_adjacent(prev: &ProjectedLine, cur: &ProjectedLine) -> bool {
    // Intentionally don't require region_path equality. Indented sub-group
    // rows (e.g. an indented "MEMORYBANK" row in a grouped academic results
    // table) sometimes land in a different XY-cut leaf than the rest of the
    // table — but the column-track alignment and y-gap checks below are
    // strong enough signals on their own to keep us from spuriously
    // bridging unrelated regions.
    let prev_bottom = prev.bbox.y + prev.bbox.height;
    let gap = cur.bbox.y - prev_bottom;
    let line_height = prev.bbox.height.max(cur.bbox.height).max(1.0);
    gap >= -line_height && gap <= line_height * TABLE_ROW_GAP_MULTIPLIER
}

/// Coefficient of variation (std-dev / mean) of inter-row vertical gaps.
/// Returns 0.0 for runs with <2 gaps (nothing to compare). Used to reject
/// runs whose row spacing is too irregular to be a real table.
fn row_spacing_cv(rows: &[(usize, &ProjectedLine, Vec<TableCell>)]) -> f32 {
    if rows.len() < 3 {
        return 0.0;
    }
    let gaps: Vec<f32> = rows
        .windows(2)
        .map(|w| (w[1].1.bbox.y - w[0].1.bbox.y).abs())
        .collect();
    let mean = gaps.iter().sum::<f32>() / gaps.len() as f32;
    if mean <= 0.0 {
        return f32::INFINITY;
    }
    let var = gaps.iter().map(|g| (g - mean).powi(2)).sum::<f32>() / gaps.len() as f32;
    var.sqrt() / mean
}

/// Test whether a candidate cell aligns with the column at index `k` in
/// `track_ranges`. Track ranges are the `(start_x, end_x)` of the header
/// (first-row) cell that defined the column. A body cell aligns to column `k`
/// if any of these hold:
///
/// - its centroid sits inside the header cell's x-range (handles centered or
///   wider body cells like `Offset Binary` under a narrower `OUTPUT FORMAT`);
/// - its start_x matches the header start_x within tolerance (left alignment);
/// - its end_x matches the header end_x within tolerance (right alignment).
///
/// Accepting any of these (rather than start_x alone) recovers tables whose
/// body cells are center- or right-aligned within the column.
fn cell_aligns_track(cell: &TableCell, track_range: (f32, f32)) -> bool {
    let (ts, te) = track_range;
    let tol = TABLE_TRACK_TOLERANCE_PT;
    let center = (cell.start_x + cell.end_x) * 0.5;
    if center >= ts - tol && center <= te + tol {
        return true;
    }
    if (cell.start_x - ts).abs() <= tol {
        return true;
    }
    if (cell.end_x - te).abs() <= tol {
        return true;
    }
    false
}

/// Pick the best matching column index for `cell`, preferring center
/// containment, then start_x match, then end_x match. Returns `None` when no
/// column aligns.
fn match_track_idx(cell: &TableCell, track_ranges: &[(f32, f32)]) -> Option<usize> {
    let tol = TABLE_TRACK_TOLERANCE_PT;
    let center = (cell.start_x + cell.end_x) * 0.5;
    // Prefer centroid-in-range.
    if let Some((i, _)) = track_ranges
        .iter()
        .enumerate()
        .filter(|(_, (s, e))| center >= s - tol && center <= e + tol)
        .min_by(|(_, (s1, e1)), (_, (s2, e2))| {
            let c1 = (s1 + e1) * 0.5;
            let c2 = (s2 + e2) * 0.5;
            (center - c1).abs().total_cmp(&(center - c2).abs())
        })
    {
        return Some(i);
    }
    // Fall back to nearest start_x within tolerance.
    if let Some((i, _)) = track_ranges
        .iter()
        .enumerate()
        .filter(|(_, (s, _))| (cell.start_x - s).abs() <= tol)
        .min_by(|(_, (s1, _)), (_, (s2, _))| {
            (cell.start_x - s1)
                .abs()
                .total_cmp(&(cell.start_x - s2).abs())
        })
    {
        return Some(i);
    }
    // Fall back to nearest end_x within tolerance (right-aligned cells).
    track_ranges
        .iter()
        .enumerate()
        .filter(|(_, (_, e))| (cell.end_x - e).abs() <= tol)
        .min_by(|(_, (_, e1)), (_, (_, e2))| {
            (cell.end_x - e1).abs().total_cmp(&(cell.end_x - e2).abs())
        })
        .map(|(i, _)| i)
}

/// Maximum number of rows to walk forward when inferring tracks from raw
/// item positions. 12 covers most real tables while bounding the cost.
const TABLE_TRACK_INFERENCE_MAX_ROWS: usize = 12;

/// Walk forward from `start_idx` collecting raw text-item start-x positions
/// across all adjacent rows, then single-link cluster them at
/// `TABLE_TRACK_TOLERANCE_PT`. Returns cluster centroids sorted ascending.
///
/// Unlike `split_cells`-derived tracks, this is immune to the
/// `TABLE_CELL_GAP_FONT_MULTIPLIER` knife-edge that collapses tightly-kerned
/// numeric columns into a single cell (e.g. `$448 $427 7%` at 14pt with
/// 13.9pt inter-item gaps). It also surfaces tracks witnessed by even a
/// single row when other rows in the same table have PDFium-level merged
/// spans that hide the full column geometry.
fn infer_tracks_from_raw_items(
    lines: &[ProjectedLine],
    pieces: &[Vec<SpanPiece<'_>>],
    start_idx: usize,
) -> Vec<f32> {
    let mut xs: Vec<f32> = Vec::new();
    let push_row_xs = |xs: &mut Vec<f32>, idx: usize| {
        // Piece x's, not span x's: a row PDFium emitted as one merged run
        // still witnesses its columns through its internal gutters.
        // Skip 0- or 1-item rows — they don't carry column info and can
        // introduce noise from single-cell prose lines.
        if pieces[idx].len() >= 2 {
            xs.extend(pieces[idx].iter().map(|p| p.x));
        }
    };
    push_row_xs(&mut xs, start_idx);
    let mut j = start_idx + 1;
    let mut rows_used = 1;
    while j < lines.len() && rows_used < TABLE_TRACK_INFERENCE_MAX_ROWS {
        if !table_rows_adjacent(&lines[j - 1], &lines[j]) {
            break;
        }
        push_row_xs(&mut xs, j);
        j += 1;
        rows_used += 1;
    }
    xs.sort_by(f32::total_cmp);
    // Each cluster carries its support = how many raw item x's fell into it.
    // A genuine column recurs once per body row, so its support ≈ the row
    // count; a header line sitting a few points off the data anchor (a
    // centered "Number of" above a right-aligned numeric column) injects a
    // separate cluster supported by a single item.
    let mut clusters: Vec<(f32, usize)> = Vec::new();
    let mut current_sum = 0.0f32;
    let mut current_count = 0usize;
    let mut current_anchor = f32::NEG_INFINITY;
    for &x in &xs {
        if current_count == 0 || (x - current_anchor).abs() <= TABLE_TRACK_TOLERANCE_PT {
            current_sum += x;
            current_count += 1;
            current_anchor = current_sum / current_count as f32;
        } else {
            clusters.push((current_sum / current_count as f32, current_count));
            current_sum = x;
            current_count = 1;
            current_anchor = x;
        }
    }
    if current_count > 0 {
        clusters.push((current_sum / current_count as f32, current_count));
    }
    // Prune single-item phantom tracks when there is a clear multi-row body
    // (some cluster supported by ≥3 items). Without this, header cells offset
    // from their data column manufacture extra close-spaced tracks that
    // collapse `min_gap` and force the inferred path to bail, dropping back to
    // the header-seeded path that silently discards an unaligned column.
    // Conservative: only prunes when a strong body signal exists, and only the
    // weakest (single-item) clusters, so small/sparse tables are untouched.
    let max_support = clusters.iter().map(|c| c.1).max().unwrap_or(0);
    if max_support >= 3 {
        clusters.retain(|c| c.1 >= 2);
    }
    clusters.into_iter().map(|c| c.0).collect()
}

/// Build a row's cells against a fixed set of column anchors. Each raw item
/// is assigned to the track its x-extent covers; items that span multiple
/// tracks (PDFium-level merged spans like `$1,298 $1,263 5%` at one anchor
/// reaching past two more anchors) are split on internal whitespace at
/// boundaries closest to each crossed anchor.
///
/// Returns `Some(cells)` of length `tracks.len()` (some cells may have empty
/// text), or `None` if any item can't be assigned cleanly (out-of-band item,
/// or a multi-track item with no usable whitespace boundary).
fn cells_from_raw_items_with_tracks(
    line: &ProjectedLine,
    tracks: &[f32],
) -> Option<Vec<TableCell>> {
    cells_from_pieces(&line_pieces(line), tracks, false)
}

/// `allow_single_piece` admits a row holding one piece. Off by default: a
/// single piece spanning multiple tracks is almost always prose (a wrapped
/// paragraph whose x-range merely overlaps the track region), and shredding it
/// at whitespace anchors corrupts the body text. It is only safe where the row
/// is known to be inside a table already - a rule band whose second column is
/// blank, where every body row legitimately has one piece.
fn cells_from_pieces(
    spans: &[SpanPiece<'_>],
    tracks: &[f32],
    allow_single_piece: bool,
) -> Option<Vec<TableCell>> {
    if spans.len() < 2 && !(allow_single_piece && spans.len() == 1) {
        return None;
    }
    let tol = TABLE_TRACK_TOLERANCE_PT;
    let mut cells: Vec<TableCell> = tracks
        .iter()
        .map(|&t| TableCell {
            start_x: t,
            end_x: t,
            text: String::new(),
            bold: false,
            text_item_indices: Vec::new(),
        })
        .collect();
    let push_text = |dst: &mut String, src: &str| {
        let src = src.trim();
        if src.is_empty() {
            return;
        }
        if !dst.is_empty() && !dst.ends_with(' ') {
            dst.push(' ');
        }
        dst.push_str(src);
    };
    for span in spans {
        let x0 = span.x;
        let x1 = span.end_x;
        let covered: Vec<usize> = tracks
            .iter()
            .enumerate()
            .filter(|&(_, &t)| t >= x0 - tol && t <= x1 + tol)
            .map(|(i, _)| i)
            .collect();
        // For spans that cover multiple tracks (multi-column-spanning items
        // we'd want to split), the span's leftmost x must anchor at the
        // leftmost covered track within tolerance. Otherwise the item is
        // non-tabular content (a wrapped paragraph / footnote whose x-range
        // merely happens to overlap the track region) that we shouldn't
        // shred at whitespace boundaries.
        if covered.len() > 1 {
            let left_track = tracks[covered[0]];
            if (x0 - left_track).abs() > tol {
                return None;
            }
        }
        match covered.len() {
            0 => return None,
            1 => {
                let idx = covered[0];
                push_text(&mut cells[idx].text, &span.text);
                cells[idx].end_x = cells[idx].end_x.max(x1);
                if let Some(si) = span.source_index {
                    cells[idx].text_item_indices.push(si);
                }
                if is_bold_item(span.span) {
                    cells[idx].bold = true;
                }
            }
            _ => {
                let anchors: Vec<f32> = covered[1..].iter().map(|&i| tracks[i]).collect();
                // Real word x's first; the character-index estimate only when
                // this piece has no verified word geometry to cut on.
                let pieces = split_words_at_x_anchors(&span.words, &anchors).or_else(|| {
                    split_text_at_x_anchors(&span.text, span.x, span.end_x - span.x, &anchors)
                })?;
                let bold = is_bold_item(span.span);
                for (idx, piece) in covered.iter().zip(pieces.iter()) {
                    if piece.is_empty() {
                        return None;
                    }
                    push_text(&mut cells[*idx].text, piece);
                    // A piece split across tracks attributes its source item
                    // to every receiving cell — word-anchored and
                    // char-estimate splits alike.
                    if let Some(si) = span.source_index {
                        cells[*idx].text_item_indices.push(si);
                    }
                    if bold {
                        cells[*idx].bold = true;
                    }
                }
            }
        }
    }
    for cell in &mut cells {
        cell.text = collapse_whitespace(cell.text.trim());
    }
    Some(cells)
}

/// Letters strictly outnumber digits — discriminates word-like labels
/// (group headers) from merged numeric runs ("$448 $427 7%").
fn is_alpha_dominant(text: &str) -> bool {
    let letters = text.chars().filter(|c| c.is_alphabetic()).count();
    let digits = text.chars().filter(|c| c.is_ascii_digit()).count();
    letters > digits
}

/// Measurement-value shape: a decimal number ("2.28"), currency/percent,
/// comma-grouped number ("1,240"), or a dash placeholder ("--" / "—").
/// Bare integers (years, codes) deliberately do NOT match — they appear in
/// legitimate header layers.
fn is_value_like(text: &str) -> bool {
    let t = text.trim();
    if t.is_empty() {
        return false;
    }
    if t.chars().all(|c| matches!(c, '-' | '—' | '–')) {
        return true;
    }
    if t.contains('$') || t.contains('%') || t.contains('±') {
        return true;
    }
    let chars: Vec<char> = t.chars().collect();
    chars
        .windows(3)
        .any(|w| w[0].is_ascii_digit() && (w[1] == '.' || w[1] == ',') && w[2].is_ascii_digit())
}

/// Split `text` (treated as occupying `[x0, x0 + width]`) into
/// `anchors.len() + 1` trimmed pieces by mapping each anchor x to the
/// whitespace boundary whose linearly-interpolated x is closest. Returns
/// `None` if the text is empty, there are no anchors, or any anchor has no
/// usable whitespace boundary (e.g. unbroken text like a long hex string).
/// Pieces may be empty strings — callers that require non-empty pieces must
/// check.
/// Split a word sequence into `anchors.len() + 1` pieces using the words' own
/// x positions: each anchor picks the inter-word boundary whose following word
/// starts closest to it.
///
/// This is the geometric counterpart of [`split_text_at_x_anchors`], which can
/// only *interpolate* an x from a character index and so assumes every glyph is
/// the same width - false for any proportional font, and the reason a split can
/// land one character off (`10 .1%`). Prefer this whenever the piece carries
/// verified word boxes.
///
/// Returns `None` when there are too few words to host every anchor, when two
/// anchors want the same boundary, or when an anchor's nearest boundary is
/// further away than the track tolerance - a miss that large means no word
/// actually starts at this column, so the caller should fall back rather than
/// cut where the geometry says nothing happens.
fn split_words_at_x_anchors(
    words: &[&crate::types::WordBox],
    anchors: &[f32],
) -> Option<Vec<String>> {
    if anchors.is_empty() || words.len() < anchors.len() + 1 {
        return None;
    }
    let mut cuts: Vec<usize> = Vec::with_capacity(anchors.len());
    for &target in anchors {
        let (k, dist) = (1..words.len())
            .filter(|k| !cuts.contains(k))
            .map(|k| (k, (words[k].x - target).abs()))
            .min_by(|a, b| a.1.total_cmp(&b.1))?;
        if dist > TABLE_TRACK_TOLERANCE_PT {
            return None;
        }
        cuts.push(k);
    }
    cuts.sort_unstable();
    let mut pieces: Vec<String> = Vec::with_capacity(cuts.len() + 1);
    let mut prev = 0usize;
    for &k in cuts.iter().chain(std::iter::once(&words.len())) {
        pieces.push(collapse_whitespace(
            &words[prev..k]
                .iter()
                .map(|w| w.text.trim())
                .collect::<Vec<_>>()
                .join(" "),
        ));
        prev = k;
    }
    Some(pieces)
}

fn split_text_at_x_anchors(
    text: &str,
    x0: f32,
    width: f32,
    anchors: &[f32],
) -> Option<Vec<String>> {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    if n == 0 || anchors.is_empty() {
        return None;
    }
    let w = width.max(1.0);
    let mut split_indices: Vec<usize> = Vec::new();
    for &target in anchors {
        let mut best: Option<(usize, f32)> = None;
        for (k, ch) in chars.iter().enumerate() {
            if !ch.is_whitespace() || split_indices.contains(&k) {
                continue;
            }
            let x = x0 + (k as f32 / n as f32) * w;
            let d = (x - target).abs();
            if best.as_ref().is_none_or(|b| d < b.1) {
                best = Some((k, d));
            }
        }
        let (k, _) = best?;
        split_indices.push(k);
    }
    split_indices.sort();
    let mut pieces: Vec<String> = Vec::new();
    let mut prev = 0usize;
    for &k in &split_indices {
        pieces.push(chars[prev..k].iter().collect::<String>().trim().to_string());
        prev = k;
    }
    pieces.push(chars[prev..].iter().collect::<String>().trim().to_string());
    Some(pieces)
}

/// Split a multi-track-spanning span's text into one piece per covered track
/// by picking whitespace positions whose linearly-interpolated x is closest
/// to each subsequent anchor. Returns `Some(pieces)` of length
/// `covered.len()` when every split lands on a real whitespace boundary;
/// `None` if no usable boundary exists (e.g. unbroken text like a long
/// hex string).
fn split_span_at_anchors(
    span: &TextItem,
    covered: &[usize],
    tracks: &[f32],
) -> Option<Vec<String>> {
    if covered.len() < 2 {
        return None;
    }
    let anchors: Vec<f32> = covered[1..].iter().map(|&idx| tracks[idx]).collect();
    split_text_at_x_anchors(&span.text, span.x, span.width, &anchors)
}

/// Like `try_detect_table` but seeds column tracks from the union of raw
/// item start-x positions across the candidate window rather than from the
/// first row's `split_cells` output. Use this first to unlock tables where
/// the cell-gap heuristic collapses adjacent numeric columns into one cell
/// in every row. Returns `None` when (a) inferred tracks are no richer than
/// per-row bucketing (no win to be had), or (b) the inferred-track candidate
/// fails any soundness check — in which case `try_detect_table`'s existing
/// logic should run.
/// Shared epilogue for the table detectors. Given the accumulated `rows`, walk
/// back to absorb wrapped header lines, optionally promote a bold first row to
/// the header (only when `bold_first_row_eligible` and no header was absorbed),
/// build the body, and construct the `TableRun`. The bold-eligibility predicate
/// differs between callers (the inferred path additionally requires non-empty
/// cells), so it's computed at the call site and passed in.
fn finalize_table_run(
    lines: &[ProjectedLine],
    start_idx: usize,
    floor: usize,
    rows: &[(usize, &ProjectedLine, Vec<TableCell>)],
    track_ranges: &[(f32, f32)],
    column_count: usize,
    end: usize,
    bold_first_row_eligible: bool,
) -> Option<TableRun> {
    // Walk back above the detected body and absorb header lines that align to
    // the same column tracks but weren't includable as body rows (merged /
    // partial header cells). Multiple wrapped header lines collapse into one
    // markdown header row, joined per-column top-to-bottom.
    let absorbed = absorb_header_lines(lines, start_idx, track_ranges, column_count, floor);

    // Promote the first body row to header iff it qualifies and we didn't
    // already absorb an explicit header above. `row_start` is the index of the
    // first body row within `rows`: 0 when the header came from absorbed lines,
    // 1 when the bold-first-row promotion consumes rows[0].
    let first_row = &rows[0].2;
    let bold_header_qualifies = absorbed.is_none() && bold_first_row_eligible;
    let (run_start, header, row_start) = match absorbed {
        Some((hstart, header_texts)) => (hstart, Some(header_texts), 0),
        None if bold_header_qualifies => (
            start_idx,
            Some(
                first_row
                    .iter()
                    .map(|c| located_cell(c, rows[0].1))
                    .collect(),
            ),
            1,
        ),
        None => (start_idx, None, 0),
    };
    let body_rows: Vec<Vec<Cell>> = rows[row_start..]
        .iter()
        .map(|(_, line, cells)| cells.iter().map(|c| located_cell(c, line)).collect())
        .collect();
    if header.is_none() && body_rows.len() < TABLE_MIN_ROWS {
        return None;
    }
    // A header with no body is not a table. Reachable only via the bold-first-row
    // promotion, which consumes rows[0] - harmless before the two-row relaxation,
    // but with it rows[0] can be the *only* row.
    if body_rows.is_empty() {
        return None;
    }

    if *super::flags::DEBUG_TABLE {
        eprintln!(
            "[tbl-detect @{start_idx}..{end}] cols={column_count} header={header:?} rows={}",
            body_rows.len()
        );
    }
    Some(TableRun {
        start: run_start,
        end,
        body_start: start_idx,
        bbox: lines_bbox(lines, run_start, end),
        block: Block::Table {
            header,
            rows: body_rows,
        },
        block_indices: Vec::new(),
    })
}

/// `allow_two_row` relaxes the two body-length gates below so a header row plus
/// a *single* data row can form a table. Only ever set by the last-resort second
/// pass in `detect_tables_impl`, which runs exclusively in regions where the
/// normal pass found no table at all - see the comment there for why relaxing
/// these gates globally is unsafe.
fn try_detect_table_inferred(
    lines: &[ProjectedLine],
    pieces: &[Vec<SpanPiece<'_>>],
    start_idx: usize,
    floor: usize,
    allow_two_row: bool,
    allow_two_col: bool,
) -> Option<TableRun> {
    let min_columns = if allow_two_col { 2 } else { TABLE_MIN_COLUMNS };
    let dbgt = *super::flags::DEBUG_TABLE;
    let seed_txt: String = lines[start_idx]
        .spans
        .iter()
        .map(|s| s.text.trim())
        .collect::<Vec<_>>()
        .join("|");
    macro_rules! bail {
        ($($a:tt)*) => {{
            if dbgt {
                eprintln!("[tbl-inferred bail @{start_idx} \"{:.40}\"] {}", seed_txt, format!($($a)*));
            }
            return None;
        }};
    }

    let baseline_cells = split_cells(&lines[start_idx]);
    let tracks = infer_tracks_from_raw_items(lines, pieces, start_idx);
    if dbgt {
        eprintln!(
            "[tbl-inferred try @{start_idx} \"{:.40}\"] tracks={} baseline={} xs=[{}]",
            seed_txt,
            tracks.len(),
            baseline_cells.len(),
            tracks
                .iter()
                .map(|t| format!("{t:.0}"))
                .collect::<Vec<_>>()
                .join(",")
        );
    }
    if tracks.len() < min_columns {
        bail!("tracks {} < MIN_COLUMNS", tracks.len());
    }
    // Only bother if we'd actually unlock more columns than the default path.
    if tracks.len() <= baseline_cells.len() {
        bail!(
            "tracks {} <= baseline {}",
            tracks.len(),
            baseline_cells.len()
        );
    }
    // Reject "tracks" that are really inter-word positions in prose. Real
    // table columns are separated by visible whitespace gutters wider than
    // the body font; word positions in running prose cluster at < 1× font
    // size apart. Threshold at 1.5× the seed line's dominant font size, with
    // a 12pt absolute floor for small fonts.
    let font_size = if lines[start_idx].dominant_font_size > 0.0 {
        lines[start_idx].dominant_font_size
    } else {
        lines[start_idx].bbox.height.max(1.0)
    };
    let min_track_gap =
        (font_size * TABLE_MIN_TRACK_GAP_FONT_MULT).max(TABLE_MIN_TRACK_GAP_FLOOR_PT);
    let min_gap = tracks
        .windows(2)
        .map(|w| w[1] - w[0])
        .fold(f32::INFINITY, f32::min);
    if min_gap < min_track_gap {
        bail!("min_gap {min_gap:.1} < {min_track_gap:.1}");
    }
    let column_count = tracks.len();
    let track_ranges: Vec<(f32, f32)> = tracks.iter().map(|&t| (t, t)).collect();
    let tracks_right_edge = *tracks.last().unwrap() + TABLE_TRACK_TOLERANCE_PT.max(8.0);

    // Seed row: require a strong structural signal — its raw PDFium span
    // count must be ≥ tracks.len() AND each span must single-cover (no
    // multi-track splits in the seeding row). This rejects prose lines
    // where 2-3 spans happen to anchor at inferred tracks but actually
    // each span's x-extent covers multiple tracks, which would produce a
    // shred-on-whitespace seed that's indistinguishable from a real
    // table. Subsequent rows can still have merged spans recovered via
    // the multi-cover split path.
    //
    // The strong row need not be `start_idx` itself: a stacked or partial
    // header (a centered "Number of" above a right-aligned numeric column,
    // fewer cells than the body) legitimately leads the table. Scan forward
    // for the first strong row and seed the body there; `finalize_table_run`
    // absorbs the header lines above it. Without this the body would be seeded
    // on the header, the run would break at the first unalignable header cell,
    // and the table would fall to the header-seeded path that drops a column.
    let tol = TABLE_TRACK_TOLERANCE_PT;
    let is_strong_row = |idx: usize| -> bool {
        let spans = &pieces[idx];
        if spans.len() < tracks.len() {
            return false;
        }
        spans.iter().all(|s| {
            let x0 = s.x;
            let x1 = s.end_x;
            tracks
                .iter()
                .filter(|&&t| t >= x0 - tol && t <= x1 + tol)
                .count()
                == 1
        })
    };
    let mut body_start = None;
    {
        let mut k = start_idx;
        let mut used = 0;
        while k < lines.len() && used < TABLE_TRACK_INFERENCE_MAX_ROWS {
            if k > start_idx && !table_rows_adjacent(&lines[k - 1], &lines[k]) {
                break;
            }
            if is_strong_row(k) {
                body_start = Some(k);
                break;
            }
            k += 1;
            used += 1;
        }
    }
    let Some(body_start) = body_start else {
        bail!("no strong body row in window");
    };
    let Some(first) = cells_from_pieces(&pieces[body_start], &tracks, allow_two_col) else {
        bail!("body row cells unassignable");
    };
    if first.iter().filter(|c| !c.text.is_empty()).count() < min_columns {
        bail!("body populated cells < MIN_COLUMNS");
    }
    let mut rows: Vec<(usize, &ProjectedLine, Vec<TableCell>)> =
        vec![(body_start, &lines[body_start], first)];

    let mut j = body_start + 1;
    while j < lines.len() {
        if lines[j].bbox.x > tracks_right_edge {
            j += 1;
            continue;
        }
        if !table_rows_adjacent(rows.last().unwrap().1, &lines[j]) {
            break;
        }
        let Some(cells) = cells_from_pieces(&pieces[j], &tracks, allow_two_col) else {
            if dbgt {
                let rt: String = lines[j]
                    .spans
                    .iter()
                    .map(|s| s.text.trim())
                    .collect::<Vec<_>>()
                    .join("|");
                eprintln!("[tbl-inferred trunc @{j} \"{:.40}\"] row unassignable", rt);
            }
            break;
        };
        // Drop rows that contribute zero populated cells (all out-of-band
        // or empty after splitting) — they'd add noise without content.
        if cells.iter().all(|c| c.text.is_empty()) {
            break;
        }
        rows.push((j, &lines[j], cells));
        j += 1;
    }
    let min_rows = if allow_two_row { 1 } else { TABLE_MIN_ROWS };
    if rows.len() < min_rows {
        bail!("rows {} < MIN_ROWS", rows.len());
    }
    // When the body was seeded below `start_idx` (the lead line was a header
    // we skipped over to find a strong row), demand a clearly multi-row body
    // before committing. A strong row found inside a header band can otherwise
    // anchor a 2-row pseudo-table that consumes the lead lines and starves the
    // header-seeded path of the real table below it. Already-strong seeds
    // (body_start == start_idx) keep the
    // standard MIN_ROWS threshold and are unaffected.
    if body_start > start_idx && rows.len() < 3 && !allow_two_row {
        bail!("advanced body_start but only {} rows", rows.len());
    }
    let cv = row_spacing_cv(&rows);
    if cv > TABLE_ROW_SPACING_MAX_CV {
        // Defer to the existing path, which can fall back to GridFallback.
        bail!("row spacing cv {cv:.2} > {TABLE_ROW_SPACING_MAX_CV}");
    }
    let end = j;

    let bold_eligible = rows[0].2.iter().all(|c| c.bold && !c.text.is_empty());
    let run = finalize_table_run(
        lines,
        body_start,
        floor,
        &rows,
        &track_ranges,
        column_count,
        end,
        bold_eligible,
    )?;
    // Runs that exist *only* because the relaxation fired must clear the extra
    // content gates. Runs that would have passed the normal floor anyway are
    // untouched, so this can never reject something the normal pass accepted.
    if allow_two_row && rows.len() < TABLE_MIN_ROWS && !two_row_run_plausible(lines, &run) {
        bail!("two-row run failed plausibility gates");
    }
    Some(run)
}

/// Plausibility gates for a header + single-data-row run.
///
/// Two rows is the weakest possible structural signal, and there is one thing
/// that reliably counterfeits it: **fully-justified prose**. Justification
/// stretches inter-word spaces until they read as column gutters, so any two
/// consecutive lines of a justified paragraph infer clean tracks and shred into
/// `| when travelling | to | conflict | zones, | more |`.
///
/// Two signals separate the real thing from the counterfeit:
///
/// 1. **Isolation.** A real 2-row table is surrounded by whitespace. A prose
///    pair is mid-paragraph, so its neighbours are table-adjacent lines.
/// 2. **Header shape.** Header cells are labels: they start with a capital or
///    a digit and don't trail mid-sentence punctuation. Prose "headers" are
///    lowercase words, often ending in a comma.
fn two_row_run_plausible(lines: &[ProjectedLine], run: &TableRun) -> bool {
    let Block::Table {
        header: Some(header),
        ..
    } = &run.block
    else {
        return false;
    };

    if run.start > 0 && table_rows_adjacent(&lines[run.start - 1], &lines[run.start]) {
        return false;
    }
    if run.end < lines.len() && table_rows_adjacent(&lines[run.end - 1], &lines[run.end]) {
        return false;
    }

    let mut non_empty = 0;
    for cell in header {
        let t = cell.text.trim();
        if t.is_empty() {
            continue;
        }
        non_empty += 1;
        let first = t.chars().next().unwrap();
        if !(first.is_uppercase() || first.is_ascii_digit()) {
            return false;
        }
        if t.ends_with(',') || t.ends_with(';') {
            return false;
        }
    }
    non_empty >= TABLE_MIN_COLUMNS
}

/// Try to extend a candidate table starting at `start_idx`. On success returns
/// a `TableRun` with `Block::Table` or `Block::GridFallback`; on failure
/// returns `None` (and the caller should fall through to per-line
/// classification).
fn try_detect_table(lines: &[ProjectedLine], start_idx: usize, floor: usize) -> Option<TableRun> {
    let first_cells = split_cells(&lines[start_idx]);
    if first_cells.len() < TABLE_MIN_COLUMNS {
        return None;
    }

    let mut rows: Vec<(usize, &ProjectedLine, Vec<TableCell>)> =
        vec![(start_idx, &lines[start_idx], first_cells.clone())];
    let column_count = first_cells.len();
    let tracks: Vec<f32> = first_cells.iter().map(|c| c.start_x).collect();
    let track_ranges: Vec<(f32, f32)> = first_cells.iter().map(|c| (c.start_x, c.end_x)).collect();

    // Right edge of the established column tracks (last track + a track-width
    // worth of slack). Used to identify lines that sit entirely in a different
    // page column and should be skipped over rather than breaking the run —
    // common on two-column pages where the projection interleaves left and
    // right column lines in y-order.
    let track_max_x = first_cells
        .iter()
        .map(|c| c.end_x.max(c.start_x))
        .fold(f32::NEG_INFINITY, f32::max);
    let tracks_right_edge = track_max_x + TABLE_TRACK_TOLERANCE_PT.max(8.0);

    let mut j = start_idx + 1;
    while j < lines.len() {
        // Skip lines that sit entirely to the right of the table's column
        // tracks — almost certainly content from a different page column.
        // Use the line's leftmost span x; if it's past the table's right edge
        // we won't break the run, just step over.
        if lines[j].bbox.x > tracks_right_edge {
            j += 1;
            continue;
        }
        if !table_rows_adjacent(rows.last().unwrap().1, &lines[j]) {
            break;
        }
        let mut cells = split_cells(&lines[j]);
        if cells.len() < column_count && cells.len() >= TABLE_MIN_COLUMNS {
            // PDFium occasionally merges two (or more) adjacent words into one
            // text run when inter-word kerning is tighter than the gap
            // threshold — common in tightly-set numeric tables (e.g. the
            // "MEMORYBANK 5.00 4.77" case on page 6 of the AMEM paper).
            // Recover by splitting straddling cells on internal whitespace.
            if let Some(patched) = recover_merged_cell(cells.clone(), &tracks) {
                cells = patched;
            }
        }
        // Partial-cell line handling: when a line has *fewer* cells than the
        // established column count, decide between (a) wrap of prior row's
        // multi-line cell, (b) sparse new row (some columns just empty),
        // (c) break-run. Order matters — the wrap path runs first so
        // tightly-stacked continuation baselines fold into the prior row; the
        // sparse-row path only triggers when there's a clear inter-row gap
        // *AND* every cell maps to a distinct column track.
        if cells.len() < column_count && !cells.is_empty() {
            let prev_line = rows.last().unwrap().1;
            let prev_y_top = prev_line.bbox.y;
            let prev_bottom = prev_line.bbox.y + prev_line.bbox.height;
            let line_height = prev_line.bbox.height.max(lines[j].bbox.height).max(1.0);
            let centroid_dy = lines[j].bbox.y - prev_y_top;
            let bottom_gap = lines[j].bbox.y - prev_bottom;
            // Map each cell to its column track once. `match_track_idx`
            // returns `Some` exactly when `cell_aligns_track` would return
            // `true`, so `mapping.len() == cells.len()` is the same
            // "every cell aligns to a track" predicate — and the mapping is
            // already in hand for the paths below, no separate unwrap whose
            // safety depends on the two functions staying in lockstep.
            let mapping: Vec<usize> = cells
                .iter()
                .filter_map(|c| match_track_idx(c, &track_ranges))
                .collect();
            let all_align_track = mapping.len() == cells.len();
            // Sparse-new-row path runs FIRST. When the line sits a clear
            // inter-row gap below the previous row AND its cells map to
            // distinct tracks, treat it as a new row with empty cells at
            // the missing tracks. This catches a sparse data row following a
            // wide header (e.g. `"1.0 April 30, Original"` under a 5-column
            // header), which a naive wrap path would merge into the header.
            if all_align_track
                && cells.len() >= 2
                && bottom_gap >= line_height * TABLE_SPARSE_ROW_MIN_BOTTOM_GAP_FRAC
            {
                let mut distinct = mapping.clone();
                distinct.sort_unstable();
                distinct.dedup();
                if distinct.len() == mapping.len() {
                    let mut padded: Vec<TableCell> = (0..column_count)
                        .map(|i| TableCell {
                            start_x: tracks[i],
                            end_x: tracks[i],
                            text: String::new(),
                            bold: false,
                            text_item_indices: Vec::new(),
                        })
                        .collect();
                    for (c, &idx) in cells.iter().zip(&mapping) {
                        padded[idx] = c.clone();
                    }
                    rows.push((j, &lines[j], padded));
                    j += 1;
                    continue;
                }
            }
            // Wrap path (existing, unchanged): tight stack against prior
            // row, multi-line cell continuation.
            if centroid_dy <= line_height * 1.5 && all_align_track {
                let prev_cells = &mut rows.last_mut().unwrap().2;
                for (c, &idx) in cells.iter().zip(&mapping) {
                    if !prev_cells[idx].text.is_empty() && !c.text.is_empty() {
                        prev_cells[idx].text.push(' ');
                    }
                    prev_cells[idx].text.push_str(&c.text);
                    prev_cells[idx]
                        .text_item_indices
                        .extend(c.text_item_indices.iter().copied());
                }
                j += 1;
                continue;
            }
        }
        // If the row has *more* cells than column_count, it likely picked up
        // content from an adjacent page column that the projection placed on
        // the same line (e.g. left-table-row + right-column body text). Try
        // to recover by keeping only the cells whose center lands inside one
        // of our established column tracks; drop the rest.
        if cells.len() > column_count {
            let kept: Vec<TableCell> = cells
                .iter()
                .filter(|c| match_track_idx(c, &track_ranges).is_some())
                .cloned()
                .collect();
            // Only cells sitting entirely outside the table's x-extent can be
            // foreign page-column bleed. A cell that lands *between* two
            // established tracks is in-table content, so discarding it would
            // silently lose document text — break the run instead and let the
            // wider row seed its own table.
            let first_track_x = track_ranges
                .first()
                .map(|r| r.0)
                .unwrap_or(f32::NEG_INFINITY);
            let extras_all_outside = cells
                .iter()
                .filter(|c| match_track_idx(c, &track_ranges).is_none())
                .all(|c| c.end_x < first_track_x || c.start_x > tracks_right_edge);
            if kept.len() == column_count && extras_all_outside {
                cells = kept;
            } else {
                break;
            }
        }
        if cells.len() != column_count {
            break;
        }
        // Allow at most one column track to drift out of tolerance, which lets
        // grouped row-labels in academic tables (e.g. an indented "MEMORYBANK"
        // row whose label column shifts right by ~30pt while the numeric
        // columns stay aligned) stay in the same run. Without this slack a
        // single indented label fragments a 6-row table into three 2-row chunks.
        let misaligned = cells
            .iter()
            .zip(track_ranges.iter())
            .filter(|(c, r)| !cell_aligns_track(c, **r))
            .count();
        if misaligned > 1 {
            break;
        }
        rows.push((j, &lines[j], cells));
        j += 1;
    }

    if rows.len() < TABLE_MIN_ROWS {
        return None;
    }

    let cv = row_spacing_cv(&rows);
    let end = j;

    if cv > TABLE_ROW_SPACING_MAX_CV {
        // Suggestive layout but the row cadence is too irregular to trust as a
        // clean table — surface as a fenced fallback so the structure is at
        // least preserved.
        let raw: Vec<String> = rows
            .iter()
            .map(|(_, line, _)| line.text.trim_end().to_string())
            .collect();
        // Block-level provenance only: the fallback renders the raw line
        // text, so it attributes to exactly the lines it displays.
        let block_indices: Vec<usize> = rows
            .iter()
            .flat_map(|(_, line, _)| line.span_item_indices.iter().copied())
            .collect();
        return Some(TableRun {
            start: start_idx,
            end,
            body_start: start_idx,
            bbox: lines_bbox(lines, start_idx, end),
            block: Block::GridFallback { lines: raw },
            block_indices,
        });
    }

    // Promote the first body row to header iff every cell in it is bold
    // (a bold-or-filled heuristic; fills require fork data). Skipped inside
    // `finalize_table_run` when a header was absorbed.
    let bold_eligible = rows[0].2.iter().all(|c| c.bold);
    finalize_table_run(
        lines,
        start_idx,
        floor,
        &rows,
        &track_ranges,
        column_count,
        end,
        bold_eligible,
    )
}

/// Walk backward from `start_idx` (not below `floor`), pulling in lines whose
/// cells all align to the table's `tracks` as header rows. Returns the new
/// start index and a single merged header row (`column_count` columns) with
/// each absorbed line's text appended into its nearest column track.
fn absorb_header_lines(
    lines: &[ProjectedLine],
    start_idx: usize,
    track_ranges: &[(f32, f32)],
    column_count: usize,
    floor: usize,
) -> Option<(usize, Vec<Cell>)> {
    let dbgt = *super::flags::DEBUG_TABLE;
    // Line index is kept alongside the cells so the assembled header cells can
    // report the band they were read from — a wrapped header spans every line
    // that contributed to it, not just the last one.
    let mut absorbed: Vec<(usize, Vec<TableCell>)> = Vec::new();
    let mut j = start_idx;
    while j > floor {
        let cand = j - 1;
        let mut cells = split_cells(&lines[cand]);
        // PDFium routinely emits a header row's words as one merged text run,
        // so an 8-column table arrives with a 2-cell header whose second cell
        // spans seven tracks. `match_track_idx` below can only bind that blob
        // to a single column, collapsing the whole header band into one cell.
        // Body rows already get merged-run recovery; headers need it too.
        //
        // Prose carries whitespace everywhere, so recovery "succeeds" on a
        // paragraph sitting above the table just as readily as on a real
        // header - manufacturing a header out of prose and dragging the
        // paragraph into the table. Cell count can't separate them (a projected
        // prose line splits on its own internal gaps), but length can: header
        // labels are terse, shredded prose is not.
        //
        // Restricted to the first candidate (the line directly above the body):
        // a merged-run header is a single line, whereas letting recovery
        // track-align line after line lets absorption climb an entire
        // paragraph, one plausible-looking row at a time.
        if absorbed.is_empty() && cells.len() < column_count {
            const HEADER_MAX_CELL_CHARS: usize = 30;
            let tracks: Vec<f32> = track_ranges.iter().map(|r| r.0).collect();
            if let Some(recovered) = recover_merged_cell(cells.clone(), &tracks)
                && recovered
                    .iter()
                    .all(|c| c.text.chars().count() <= HEADER_MAX_CELL_CHARS)
            {
                cells = recovered;
            }
        }
        if dbgt {
            let texts: Vec<&str> = cells.iter().map(|c| c.text.as_str()).collect();
            eprintln!(
                "[tbl-absorb cand @{cand} {:?}] cells={texts:?} extents={:?}",
                lines[cand].text.chars().take(40).collect::<String>(),
                cells
                    .iter()
                    .map(|c| (c.start_x as i32, c.end_x as i32))
                    .collect::<Vec<_>>()
            );
        }
        // A header line must carry at least two cells (a single cell is a
        // title/caption, not a header) and sit tight above the row below it.
        if cells.len() < 2 {
            break;
        }
        if !table_rows_adjacent(&lines[cand], &lines[j]) {
            break;
        }
        if cells.len() > column_count {
            break;
        }
        let all_align = cells
            .iter()
            .all(|c| track_ranges.iter().any(|r| cell_aligns_track(c, *r)));
        if !all_align {
            break;
        }
        absorbed.push((cand, cells));
        j = cand;
    }
    if absorbed.is_empty() {
        return None;
    }
    // Collected bottom-up; reverse so text reads top-to-bottom per column.
    absorbed.reverse();
    let mut header = vec![Cell::default(); column_count];
    for (line_idx, cells) in &absorbed {
        for c in cells {
            let Some(idx) = match_track_idx(c, track_ranges) else {
                continue;
            };
            if !header[idx].text.is_empty() && !c.text.is_empty() {
                header[idx].text.push(' ');
            }
            header[idx].text.push_str(&c.text);
            header[idx].add_indices(c.text_item_indices.iter().copied());
            Rect::extend(&mut header[idx].bbox, &cell_rect(c, &lines[*line_idx]));
        }
    }
    Some((j, header))
}

/// Scan `lines` once and return all detected tabular regions (sorted by
/// `start`). Caller uses these as cut-points so the per-line classifier never
/// sees lines inside a table.
pub(super) fn detect_tables(lines: &[ProjectedLine]) -> Vec<TableRun> {
    detect_tables_impl(lines, true)
}

/// `detect_tables` plus the booktabs rule-band pass: a table drawn as nothing
/// but a top and bottom hairline has no grid for the ruled detector and, when
/// it has only two columns, is below `TABLE_MIN_COLUMNS` for the borderless
/// one. The rules themselves are the missing evidence - see `rule_bands`.
pub(super) fn detect_tables_banded(
    lines: &[ProjectedLine],
    segs: &RuleSegments,
    page_height: f32,
) -> Vec<TableRun> {
    let pieces = compute_line_pieces(lines);
    let runs = detect_tables_with_pieces(lines, &pieces, true);
    let bands = rule_bands(segs, page_height);
    if bands.is_empty() {
        return runs;
    }
    two_col_band_pass(lines, &pieces, runs, &bands)
}

/// A booktabs rule band: a pair of horizontal rules with nothing ruled between
/// them, i.e. the top and bottom of a table drawn without a grid.
///
/// Requires the two rules to overlap in x (they bound the same block), to sit
/// far enough apart to hold rows but not so far as to be page furniture, and -
/// crucially - to have **no vertical rule crossing between them**. A band with
/// verticals is a real grid, and the ruled detector owns it.
fn rule_bands(segs: &RuleSegments, page_height: f32) -> Vec<(f32, f32)> {
    let RuleSegments { hs, raw_vs: vs, .. } = segs;
    let mut out = Vec::new();
    for pair in hs.windows(2) {
        let (top, bot) = (&pair[0], &pair[1]);
        let sep = bot.y - top.y;
        if !(RULE_BAND_MIN_HEIGHT_PT..page_height * 0.5).contains(&sep) {
            continue;
        }
        let overlap = (top.x_max.min(bot.x_max) - top.x_min.max(bot.x_min)).max(0.0);
        let extent = (top.x_max - top.x_min).max(bot.x_max - bot.x_min).max(1.0);
        if overlap / extent < 0.6 {
            continue;
        }
        if vs
            .iter()
            .any(|v| v.y_min < bot.y - 1.0 && v.y_max > top.y + 1.0)
        {
            continue;
        }
        out.push((top.y, bot.y));
    }
    out
}

/// Last-resort pass for two-column tables enclosed in a rule band.
///
/// Modelled on `two_row_second_pass` and gated the same way - only in the index
/// gaps where the normal pass found nothing, and only for runs that stay inside
/// the gap. The extra requirement is that the seed line lie inside a band and
/// that the band hold enough lines to be a table rather than a rule under a
/// heading or a hyperlink underline.
fn two_col_band_pass(
    lines: &[ProjectedLine],
    pieces: &[Vec<SpanPiece<'_>>],
    runs: Vec<TableRun>,
    bands: &[(f32, f32)],
) -> Vec<TableRun> {
    let claimed = |a: usize, b: usize| runs.iter().any(|r| r.start < b && a < r.end);
    let mut extra: Vec<TableRun> = Vec::new();
    for &(y0, y1) in bands {
        let inside: Vec<usize> = (0..lines.len())
            .filter(|&i| {
                let c = lines[i].bbox.y + lines[i].bbox.height * 0.5;
                c > y0 && c < y1
            })
            .collect();
        // The band's lines must be a contiguous run of the region and numerous
        // enough to be a table body rather than a rule under a heading or a
        // stack of hyperlink underlines.
        if inside.len() < RULE_BAND_MIN_LINES
            || inside.last().unwrap() - inside[0] + 1 != inside.len()
        {
            continue;
        }
        let (bs, be) = (inside[0], inside.last().unwrap() + 1);
        if claimed(bs, be) {
            continue;
        }
        // The seed must read as a genuine two-cell row, not a wrapped prose
        // line that happens to sit under a rule.
        if pieces[bs].len() != 2 {
            continue;
        }
        // Detect against the band's lines alone: a run can then never reach
        // past the rules that are the whole justification for relaxing
        // `TABLE_MIN_COLUMNS` here.
        if let Some(mut run) =
            try_detect_table_inferred(&lines[bs..be], &pieces[bs..be], 0, 0, false, true)
        {
            run.start += bs;
            run.end += bs;
            run.body_start += bs;
            extra.push(run);
        }
    }
    if extra.is_empty() {
        return runs;
    }
    let mut all = runs;
    all.extend(extra);
    all.sort_by_key(|r| r.start);
    all
}

/// Count borderless table runs for the layout-complexity stats. Excludes
/// description lists — a label/value pair block reads fine as text, so it
/// should not flag the page as table-bearing. `GridFallback` runs count:
/// tabular structure the emitter couldn't extract cleanly still marks the
/// page as table-bearing.
pub(crate) fn count_text_table_runs(lines: &[ProjectedLine]) -> usize {
    detect_tables_impl(lines, false).len()
}

/// Validated ruled-table rects for the layout-complexity stats. Unlike the
/// raw grid components (`detect_table_rects`, which deliberately over-captures
/// for use as XY-cut obstacles), each candidate grid here is confirmed against
/// the projected text by `build_ruled_table` — so a plain boxed callout or a
/// chart's axis gridlines, which have the lines but not the cell structure,
/// don't count. Returns the bounding rect of each run's consumed lines.
pub(crate) fn validated_ruled_table_rects(
    lines: &[ProjectedLine],
    graphics: &[GraphicPrimitive],
    page_width: f32,
    page_height: f32,
) -> Vec<Rect> {
    let segs = extract_rule_segments(graphics);
    detect_ruled_tables_global(lines, &segs, page_width, page_height)
        .into_iter()
        .filter_map(|(_, consumed)| {
            let mut x0 = f32::INFINITY;
            let mut y0 = f32::INFINITY;
            let mut x1 = f32::NEG_INFINITY;
            let mut y1 = f32::NEG_INFINITY;
            for &i in &consumed {
                let b = &lines[i].bbox;
                x0 = x0.min(b.x);
                y0 = y0.min(b.y);
                x1 = x1.max(b.x + b.width);
                y1 = y1.max(b.y + b.height);
            }
            (x1 > x0 && y1 > y0).then(|| Rect {
                x: x0,
                y: y0,
                width: x1 - x0,
                height: y1 - y0,
            })
        })
        .collect()
}

fn detect_tables_impl(lines: &[ProjectedLine], include_desc_lists: bool) -> Vec<TableRun> {
    let pieces = compute_line_pieces(lines);
    detect_tables_with_pieces(lines, &pieces, include_desc_lists)
}

fn detect_tables_with_pieces(
    lines: &[ProjectedLine],
    pieces: &[Vec<SpanPiece<'_>>],
    include_desc_lists: bool,
) -> Vec<TableRun> {
    let mut out = Vec::new();
    let mut i = 0;
    let mut floor = 0;
    while i < lines.len() {
        if let Some(run) = try_detect_table_inferred(lines, pieces, i, floor, false, false) {
            floor = run.end;
            i = run.end;
            out.push(run);
        } else if let Some(run) = try_detect_table(lines, i, floor) {
            floor = run.end;
            i = run.end;
            out.push(run);
        } else if let Some(run) = include_desc_lists
            .then(|| try_detect_description_list(lines, i))
            .flatten()
        {
            floor = run.end;
            i = run.end;
            out.push(run);
        } else {
            i += 1;
        }
    }
    let mut merged = merge_consecutive_table_runs(out, lines);
    // Iterate to fixpoint (bounded): a merged cluster is itself a table run
    // that may now sit adjacent to the next fragment cluster.
    for _ in 0..4 {
        let before = merged.len();
        merged = merge_fragmented_table_clusters(merged, lines);
        if merged.len() == before {
            break;
        }
    }
    two_row_second_pass(lines, pieces, merged)
}

/// Last-resort pass for header + single-data-row tables.
///
/// Some real tables are a header row plus exactly one data row. Both length
/// gates in `try_detect_table_inferred` reject them. Relaxing those gates
/// *directly* is not safe: a 2-row run forms early, higher up the page, and
/// consumes the header of the real multi-row table below it.
///
/// So instead we keep the normal pass exactly as-is and only retry, with the
/// relaxation on, inside the index gaps where it found *no* table at all. By
/// construction a run detected here cannot steal rows from a real table: there
/// isn't one in the gap. Runs that would spill past the gap end are discarded
/// for the same reason.
fn two_row_second_pass(
    lines: &[ProjectedLine],
    pieces: &[Vec<SpanPiece<'_>>],
    runs: Vec<TableRun>,
) -> Vec<TableRun> {
    // Gaps between (and around) the runs the normal pass claimed.
    let mut gaps: Vec<(usize, usize)> = Vec::new();
    let mut cursor = 0;
    for r in &runs {
        if r.start > cursor {
            gaps.push((cursor, r.start));
        }
        cursor = cursor.max(r.end);
    }
    if cursor < lines.len() {
        gaps.push((cursor, lines.len()));
    }

    let mut extra: Vec<TableRun> = Vec::new();
    for (gs, ge) in gaps {
        let mut i = gs;
        // `floor` starts at the gap start so header absorption can never walk
        // back into the preceding table run.
        let mut floor = gs;
        while i < ge {
            match try_detect_table_inferred(lines, pieces, i, floor, true, false) {
                // A run that reaches past the gap is exactly the theft case the
                // gap restriction exists to prevent.
                Some(run) if run.end <= ge && run.start >= gs => {
                    floor = run.end;
                    i = run.end;
                    extra.push(run);
                }
                _ => i += 1,
            }
        }
    }
    if extra.is_empty() {
        return runs;
    }
    let mut all = runs;
    all.extend(extra);
    all.sort_by_key(|r| r.start);
    all
}

// ── Description-list 2-column table detector ──────────────────────────────
//
// Catches borderless 2-column tables that the main `try_detect_table` rejects
// because `TABLE_MIN_COLUMNS = 3`. Signature:
//
//   - ≥ DESC_LIST_MIN_ROWS rows where col 0 is a short label (≤ DESC_LIST_LABEL_MAX_CHARS)
//     and col 1 is anything (typically a paragraph or bullet list).
//   - Stable x-anchors for both columns (within DESC_LIST_TRACK_TOL_PT).
//   - Clear inter-column gap (col1.start_x - col0.end_x ≥ DESC_LIST_MIN_COL_GAP_PT).
//   - Asymmetric content: at least one row's col 1 is meaningfully longer than
//     its col 0 — rules out symmetric two-column body prose / newspaper layouts.
//
// Handles two PDFium quirks:
//   - Wrap continuations: a single-cell line at col 1's anchor extends the
//     previous row's col 1.
//   - Merged-span rows: PDFium occasionally emits both columns of a row as a
//     single text item starting at col 0's anchor (kerning happens to be tight
//     across the column gap). We split on the whitespace position closest to
//     col 1's anchor and treat the result as a normal 2-cell row.

const DESC_LIST_MIN_ROWS: usize = 2;
const DESC_LIST_LABEL_MAX_CHARS: usize = 40;
const DESC_LIST_LABEL_MAX_WORDS: usize = 4;
const DESC_LIST_TRACK_TOL_PT: f32 = 8.0;
const DESC_LIST_MIN_COL_GAP_PT: f32 = 12.0;

/// Discriminates "label-like" col-0 text from prose fragments. Real
/// description-list labels are short noun-phrases (1-4 words, no terminal
/// sentence punctuation, no internal sentence boundary). Body prose that
/// happens to be projection-merged with a right-column line tends to fail at
/// least one of these.
fn is_label_like(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }
    if trimmed.chars().count() > DESC_LIST_LABEL_MAX_CHARS {
        return false;
    }
    // Pure bullet glyph (or bullet+digit like "1.") is not a label — that's a
    // list item, which the list classifier handles. Lets us avoid claiming
    // bulleted lists as 2-col description tables.
    if is_bullet_only(trimmed) {
        return false;
    }
    let word_count = trimmed.split_whitespace().count();
    if word_count == 0 || word_count > DESC_LIST_LABEL_MAX_WORDS {
        return false;
    }
    // Internal sentence boundary ("foo. Bar") = prose, not a label.
    // A trailing period is fine ("Item.") and a trailing colon is fine
    // ("Note:"); both are common in real labels.
    let bytes = trimmed.as_bytes();
    for i in 0..bytes.len().saturating_sub(2) {
        if bytes[i] == b'.' && bytes[i + 1] == b' ' {
            let next = bytes[i + 2];
            if next.is_ascii_uppercase() {
                return false;
            }
        }
    }
    true
}

/// Cell text reads as a page number reference: pure digits, pure roman
/// numerals (i, ii, iv, …, IX, X), or a digit followed by trivial punctuation.
fn is_page_ref(text: &str) -> bool {
    let t = text.trim();
    if t.is_empty() {
        return false;
    }
    if t.chars().all(|c| c.is_ascii_digit()) {
        return true;
    }
    let lower = t.to_ascii_lowercase();
    if lower
        .chars()
        .all(|c| matches!(c, 'i' | 'v' | 'x' | 'l' | 'c' | 'd' | 'm'))
    {
        // Cap length so multi-word lowercase Latin words don't pass (e.g.
        // "mix", "civil" would all be made of roman-numeral letters).
        if t.chars().count() <= 6 {
            return true;
        }
    }
    false
}

fn is_bullet_only(text: &str) -> bool {
    let t = text.trim();
    if t.is_empty() {
        return false;
    }
    let only_glyph = t.chars().all(|c| {
        matches!(
            c,
            '•' | '●'
                | '○'
                | '◦'
                | '▪'
                | '■'
                | '□'
                | '‣'
                | '⁃'
                | '*'
                | '-'
                | '–'
                | '—'
                | '⮚'
                | '►'
                | '▶'
                // Symbol-font bullet in the Private Use Area (undecoded 0xB7).
                | '\u{f0b7}'
        )
    });
    if only_glyph {
        return true;
    }
    // Numeric list marker: "1.", "1)", "(1)", "i.", "ii.", "a.", "(a)" etc. —
    // all are list markers, not table labels.
    let chars: Vec<char> = t.chars().collect();
    // Paren-wrapped marker: "(1)" or single-letter "(a)".
    let is_paren_marker = chars.first() == Some(&'(') && chars.last() == Some(&')') && {
        let inner = &chars[1..chars.len() - 1];
        inner.iter().all(|c| c.is_ascii_digit())
            || (inner.len() == 1 && inner[0].is_ascii_alphabetic())
    };
    if is_paren_marker && chars.len() <= 5 {
        return true;
    }
    let trailing = chars.last().copied();
    if matches!(trailing, Some('.') | Some(')')) {
        let body: String = chars[..chars.len() - 1].iter().collect();
        if !body.is_empty()
            && (body.chars().all(|c| c.is_ascii_digit())
                || body
                    .chars()
                    .all(|c| matches!(c, 'i' | 'v' | 'x' | 'I' | 'V' | 'X'))
                // Single-letter lettered-list marker: "a.", "b)", "A." — a
                // short marker column beside wrapped body text is a nested
                // ordered list, not a 2-column description table.
                || (body.chars().count() == 1
                    && body.chars().next().is_some_and(|c| c.is_ascii_alphabetic())))
        {
            return true;
        }
    }
    false
}

/// Heuristic: line text reads like a figure or table caption.
/// Used to break a description-list run before absorbing a caption that
/// happens to straddle the table's column anchors.
fn looks_like_caption(text: &str) -> bool {
    let trimmed = text.trim_start();
    let lower = trimmed.to_ascii_lowercase();
    for prefix in ["figure ", "fig. ", "fig ", "table ", "tab. ", "tab "] {
        if let Some(rest) = lower.strip_prefix(prefix) {
            // Require a digit (or roman) right after to avoid matching prose
            // sentences that happen to start with "Table" / "Figure".
            if rest.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                return true;
            }
        }
    }
    false
}

fn try_detect_description_list(lines: &[ProjectedLine], start_idx: usize) -> Option<TableRun> {
    let first = split_cells(&lines[start_idx]);
    if first.len() != 2 {
        return None;
    }
    let col0_x = first[0].start_x;
    let col0_end = first[0].end_x;
    let col1_x = first[1].start_x;
    if col1_x - col0_end < DESC_LIST_MIN_COL_GAP_PT {
        return None;
    }
    if !is_label_like(&first[0].text) {
        return None;
    }

    let mut rows: Vec<(usize, Cell, Cell)> = vec![(
        start_idx,
        located_cell(&first[0], &lines[start_idx]),
        located_cell(&first[1], &lines[start_idx]),
    )];
    // Track how many rows came from the *actual* 2-cell path (i.e. PDFium
    // emitted two distinct spans with a clear gap). The merged-span split path
    // is a recovery hack for tight-kerning cases — when it's the only thing
    // extending the run, we're almost certainly slicing prose, not a table.
    let mut real_two_cell_rows: usize = 1;

    let mut j = start_idx + 1;
    while j < lines.len() {
        let prev_line = &lines[rows.last().unwrap().0];
        if !table_rows_adjacent(prev_line, &lines[j]) {
            break;
        }
        // Caption / divider guard: a line whose text begins with a figure or
        // table caption marker is never a row in the *current* description
        // list — it's the caption sitting below it. Stop here rather than
        // greedily splitting it on whitespace into a bogus row.
        if looks_like_caption(&lines[j].text) {
            break;
        }
        // Spacing guard: if rows have a clear inter-row cadence and this line
        // sits markedly farther below than the run's typical row gap, treat
        // it as a different block (caption / next paragraph) even though
        // `table_rows_adjacent` is generous up to 2.5× line height.
        if rows.len() >= 2 {
            let prev_y = prev_line.bbox.y;
            let cur_y = lines[j].bbox.y;
            let cur_gap = cur_y - prev_y;
            let prior_gaps: Vec<f32> = rows
                .windows(2)
                .map(|w| lines[w[1].0].bbox.y - lines[w[0].0].bbox.y)
                .collect();
            if let Some(&max_prior) = prior_gaps.iter().max_by(|a, b| a.total_cmp(b))
                && cur_gap > max_prior * 1.6
                && cur_gap > lines[j].bbox.height.max(prev_line.bbox.height)
            {
                break;
            }
        }
        let cells = split_cells(&lines[j]);
        match cells.len() {
            2 => {
                let c0_aligned = (cells[0].start_x - col0_x).abs() <= DESC_LIST_TRACK_TOL_PT;
                let c1_aligned = (cells[1].start_x - col1_x).abs() <= DESC_LIST_TRACK_TOL_PT;
                if c0_aligned && c1_aligned && is_label_like(&cells[0].text) {
                    rows.push((
                        j,
                        located_cell(&cells[0], &lines[j]),
                        located_cell(&cells[1], &lines[j]),
                    ));
                    real_two_cell_rows += 1;
                    j += 1;
                    continue;
                }
                break;
            }
            1 => {
                let cell = &cells[0];
                let c0_aligned = (cell.start_x - col0_x).abs() <= DESC_LIST_TRACK_TOL_PT;
                let c1_aligned = (cell.start_x - col1_x).abs() <= DESC_LIST_TRACK_TOL_PT;
                if c1_aligned {
                    let tail = &mut rows.last_mut().unwrap().2;
                    if !tail.text.is_empty() {
                        tail.text.push(' ');
                    }
                    tail.text.push_str(&cell.text);
                    // The wrapped continuation is part of the same logical
                    // cell, so it grows that cell's box rather than starting a
                    // new one — and extends its provenance likewise.
                    Rect::extend(&mut tail.bbox, &cell_rect(cell, &lines[j]));
                    tail.add_indices(cell.text_item_indices.iter().copied());
                    j += 1;
                    continue;
                }
                // Merged-span row: single cell starts at col 0 but extends past
                // col 1's anchor. Split on the whitespace closest to col 1.
                let straddles = c0_aligned && cell.end_x > col1_x + DESC_LIST_TRACK_TOL_PT;
                if straddles
                    && let Some((left, right)) =
                        split_merged_at_anchor(&cell.text, cell.start_x, cell.end_x, col1_x)
                    && is_label_like(&left)
                {
                    // Split position is a linear estimate inside one merged
                    // span, not an observed boundary — the halves carry text
                    // only rather than a fabricated rect. BOTH halves
                    // attribute to the single merged span's source items.
                    let mut lcell: Cell = left.into();
                    lcell.add_indices(cell.text_item_indices.iter().copied());
                    let mut rcell: Cell = right.into();
                    rcell.add_indices(cell.text_item_indices.iter().copied());
                    rows.push((j, lcell, rcell));
                    j += 1;
                    continue;
                }
                break;
            }
            _ => break,
        }
    }

    if rows.len() < DESC_LIST_MIN_ROWS {
        return None;
    }

    // Anti-false-positive #1: require ≥2 rows that came from the actual
    // 2-cell path. A run extended entirely by the merged-span split is almost
    // certainly slicing body prose where a heading happens to have a section
    // number cleanly tab-stopped left of the title.
    if real_two_cell_rows < 2 {
        return None;
    }
    // Anti-false-positive #1b: at least one row must have BOTH columns
    // containing alphabetic characters. Filters two common shapes that are
    // *not* description-list tables: TOC entries (col 1 = page number) and
    // footnote lists (col 0 = footnote number). Real description-list tables
    // have at least one row of word-on-word.
    let has_alpha_pair = rows.iter().any(|(_, c0, c1)| {
        c0.text.chars().any(|c| c.is_alphabetic()) && c1.text.chars().any(|c| c.is_alphabetic())
    });
    if !has_alpha_pair {
        return None;
    }
    // Anti-false-positive #1c: if *every* col 1 reads as a page-number
    // (digits or roman numerals), the run is a TOC. TOCs match the alpha
    // pair check only when one of the page refs happens to be a roman
    // numeral like "v" or "vi" alongside an alpha col 0.
    let all_page_refs = rows.iter().all(|(_, _, c1)| is_page_ref(&c1.text));
    if all_page_refs {
        return None;
    }
    // Anti-false-positive #2: at least one of
    //   (a) ≥3 rows (cadence is the signal — short symmetric pairs that
    //       repeat 3+ times are tabular),
    //   (b) one row's col 1 is substantially longer than col 0 (paragraph
    //       cell next to a label cell — the classic description-list shape).
    let asymmetric = rows.iter().any(|(_, c0, c1)| {
        c1.text.chars().count() >= c0.text.chars().count().saturating_mul(2).max(20)
    });
    if rows.len() < 3 && !asymmetric {
        return None;
    }

    let body: Vec<Vec<Cell>> = rows
        .iter()
        .map(|(_, c0, c1)| vec![c0.clone(), c1.clone()])
        .collect();
    Some(TableRun {
        start: start_idx,
        end: j,
        body_start: start_idx,
        bbox: lines_bbox(lines, start_idx, j),
        block: Block::Table {
            header: None,
            rows: body,
        },
        block_indices: Vec::new(),
    })
}

/// Split a merged-column text item on the whitespace position whose linear
/// x-estimate is closest to `anchor_x`. Returns trimmed (left, right) halves,
/// or `None` if no usable whitespace split exists.
fn split_merged_at_anchor(
    text: &str,
    start_x: f32,
    end_x: f32,
    anchor_x: f32,
) -> Option<(String, String)> {
    let width = (end_x - start_x).max(1.0);
    let ratio = ((anchor_x - start_x) / width).clamp(0.0, 1.0);
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return None;
    }
    let target = ((chars.len() as f32) * ratio) as usize;
    let mut best: Option<usize> = None;
    let mut best_dist = usize::MAX;
    for (i, c) in chars.iter().enumerate() {
        if c.is_whitespace() {
            let d = i.abs_diff(target);
            if d < best_dist {
                best_dist = d;
                best = Some(i);
            }
        }
    }
    let split = best?;
    let left: String = chars[..split].iter().collect();
    let right: String = chars[split + 1..].iter().collect();
    let left = left.trim().to_string();
    let right = right.trim().to_string();
    if left.is_empty() || right.is_empty() {
        return None;
    }
    Some((left, right))
}

// ── Cross-run merging (post-pass over `detect_tables` output) ──────────────
//
// `try_detect_table` walks lines top-to-bottom and breaks the run whenever the
// column count or track alignment changes. That breaks two common shapes into
// separate runs:
//
//   B1 — multi-line header with row-label-column missing. The header rows have
//        N cells aligned to body tracks 2..N+1; the body rows have N+1 cells
//        including a leading row-label. Detect_tables emits an N-col "header"
//        run + an (N+1)-col body run.
//
//   B4 — a single table interrupted by a category divider that fragments it
//        into two sibling runs with identical column structure.
//
// The pass below walks adjacent run pairs and merges them when they're
// vertically immediate (A.end == B.start), reasonably close in y, and share
// either identical tracks (Case Same) or A's tracks are a 1-column-shorter
// subset of B's tracks (Case Subset). Subset merges fold A into B's header.
//
// Guards:
//   - Only merge `Block::Table` pairs (skip `GridFallback`).
//   - A.end must equal B.start so no non-table content between the runs
//     gets dropped.
//   - A's body row count is capped (`TABLE_HEADER_MAX_ABSORB_ROWS`) so a
//     real standalone table that happens to neighbor another isn't absorbed.
//   - Vertical gap between A's last line and B's first line is capped by a
//     small multiple of the line height.

/// A run with this many or fewer body rows can be folded as header content of
/// a following table. Above this we treat A as its own complete table.
const TABLE_HEADER_MAX_ABSORB_ROWS: usize = 3;

/// Cap on the y-gap between two consecutive runs for them to be merge
/// candidates, in multiples of line height. Larger gaps mean visually
/// distinct tables.
const TABLE_MERGE_MAX_Y_GAP_LINES: f32 = 2.0;

fn merge_consecutive_table_runs(runs: Vec<TableRun>, lines: &[ProjectedLine]) -> Vec<TableRun> {
    if runs.len() < 2 {
        return runs;
    }
    let mut out: Vec<TableRun> = Vec::with_capacity(runs.len());
    for run in runs {
        if let Some(prev) = out.last()
            && let Some(merged) = try_merge_pair(prev, &run, lines)
        {
            out.pop();
            out.push(merged);
            continue;
        }
        out.push(run);
    }
    out
}

// ── Fragmented-cluster re-extraction ───────────────────────────────────────
//
// `try_detect_table` closes a run whenever the per-row cell count changes, so
// one real table with sparse rows (row-label column present on some rows
// only, empty value columns on others) fragments into several runs at
// different column counts, with stranded single rows between them. The pairs
// of fragments rarely satisfy `try_merge_pair`'s same-cols / |A|+1==|B|
// cases. This pass takes the opposite approach: instead of mapping fragment
// columns onto each other, it re-derives the *union* column track set from
// the raw PDFium span positions across the whole cluster and re-extracts
// every line against those tracks. Sparse rows get empty cells, merged spans
// split at track anchors, and the cluster emits as one table.
//
// Only fires when ≥2 table runs are already y-adjacent — prose never enters
// this path, so the false-positive surface is limited to "two genuinely
// separate stacked tables", which the both-complete guard rejects.

/// Max non-table lines between two runs for them to join one cluster.
const TABLE_CLUSTER_MAX_INTERSTITIAL_LINES: usize = 2;

/// Abort the cluster merge when more than this fraction of the window's
/// lines can't be binned into the union tracks.
const TABLE_CLUSTER_MAX_FAILED_ROW_FRAC: f32 = 0.3;

/// Max header lines walked above the cluster body by the union header pass.
const TABLE_CLUSTER_MAX_HEADER_LINES: usize = 4;

fn merge_fragmented_table_clusters(runs: Vec<TableRun>, lines: &[ProjectedLine]) -> Vec<TableRun> {
    if runs.len() < 2 {
        return runs;
    }
    let dbgt = *super::flags::DEBUG_TABLE;
    let mut out: Vec<TableRun> = Vec::with_capacity(runs.len());
    let mut i = 0;
    while i < runs.len() {
        let mut j = i + 1;
        while j < runs.len() && cluster_adjacent(&runs[j - 1], &runs[j], lines) {
            j += 1;
        }
        if j - i >= 2 {
            let floor = out.last().map(|r| r.end).unwrap_or(0);
            if let Some(merged) = build_union_table(&runs[i..j], lines, floor) {
                if dbgt {
                    eprintln!(
                        "[tbl-cluster] merged {} runs (lines {}..{}) into one table",
                        j - i,
                        merged.start,
                        merged.end
                    );
                }
                out.push(merged);
                i = j;
                continue;
            } else if dbgt {
                eprintln!(
                    "[tbl-cluster] union build failed for {} runs @{}..{}",
                    j - i,
                    runs[i].start,
                    runs[j - 1].end
                );
            }
        }
        out.push(runs[i].clone());
        i += 1;
    }
    out
}

fn cluster_adjacent(a: &TableRun, b: &TableRun, lines: &[ProjectedLine]) -> bool {
    let (a_header, a_rows_len) = match &a.block {
        Block::Table { header, rows } => (header.is_some(), rows.len()),
        _ => return false,
    };
    let (b_header, b_rows_len) = match &b.block {
        Block::Table { header, rows } => (header.is_some(), rows.len()),
        _ => return false,
    };
    if b.start.saturating_sub(a.end) > TABLE_CLUSTER_MAX_INTERSTITIAL_LINES {
        return false;
    }
    if a.end == 0 || a.end > lines.len() || b.start >= lines.len() {
        return false;
    }
    let a_last = &lines[a.end - 1];
    let b_first = &lines[b.start];
    let line_height = a_last.bbox.height.max(b_first.bbox.height).max(1.0);
    let gap = b_first.bbox.y - (a_last.bbox.y + a_last.bbox.height);
    if gap > line_height * TABLE_MERGE_MAX_Y_GAP_LINES || gap < -line_height {
        return false;
    }
    // Two complete-looking tables (both with explicit headers and real bodies)
    // separated by a visible gap are most likely genuinely separate tables.
    let both_complete = a_header && b_header && a_rows_len >= 3 && b_rows_len >= 3;
    if both_complete && gap > line_height {
        return false;
    }
    // Track compatibility: fragments of one table share column geometry (the
    // narrower fragment's tracks are a subset of the wider one's), while two
    // genuinely different stacked tables (e.g. a 2-col label/value list above
    // a 4-col transaction table) do not. Without this gate the union merge
    // fuses them and shreds both. Require ≥75% of the narrower run's tracks
    // to align to the wider run's tracks.
    let dbgt = *super::flags::DEBUG_TABLE;
    let (Some(a_tracks), Some(b_tracks)) = (run_body_tracks(a, lines), run_body_tracks(b, lines))
    else {
        // Inferred-path runs often have no line whose split_cells count
        // matches the declared column count, so tracks can't be re-derived.
        // Stay permissive — build_union_table's own soundness guards (width
        // check, failed-row fraction) still gate the actual merge.
        return true;
    };
    let (narrow, wide) = if a_tracks.len() <= b_tracks.len() {
        (&a_tracks, &b_tracks)
    } else {
        (&b_tracks, &a_tracks)
    };
    let matched = narrow
        .iter()
        .filter(|&&t| {
            wide.iter()
                .any(|&w| subset_match_score(t, w, TABLE_SUBSET_TRACK_TOLERANCE_PT).is_some())
        })
        .count();
    let ok = (matched as f32) >= (narrow.len() as f32) * 0.75;
    if !ok && dbgt {
        eprintln!(
            "[tbl-cluster] adjacency reject @{}..{}: tracks {}/{} matched (narrow=[{}] wide=[{}])",
            a.start,
            b.end,
            matched,
            narrow.len(),
            narrow
                .iter()
                .map(|t| format!("{:.0}-{:.0}", t.0, t.1))
                .collect::<Vec<_>>()
                .join(","),
            wide.iter()
                .map(|t| format!("{:.0}-{:.0}", t.0, t.1))
                .collect::<Vec<_>>()
                .join(",")
        );
    }
    ok
}

/// Union column tracks across a window: every row with ≥2 gap-split cells
/// contributes its cell start-x positions; positions cluster at
/// `TABLE_TRACK_TOLERANCE_PT`. Cells (not raw spans) are the unit here —
/// multi-word cell content emits several PDFium spans whose x positions are
/// word starts, not column starts, and would shred the track set.
fn union_tracks_in_window(lines: &[ProjectedLine], start: usize, end: usize) -> Vec<(f32, usize)> {
    let mut xs: Vec<f32> = Vec::new();
    for line in &lines[start..end.min(lines.len())] {
        let cells = split_cells(line);
        if cells.len() >= 2 {
            xs.extend(cells.iter().map(|c| c.start_x));
        }
    }
    xs.sort_by(f32::total_cmp);
    let mut clusters: Vec<(f32, usize)> = Vec::new();
    let mut current_sum = 0.0f32;
    let mut current_count = 0usize;
    let mut current_anchor = f32::NEG_INFINITY;
    for &x in &xs {
        // Looser tolerance than the per-run paths: center-aligned columns
        // jitter their cell start-x by the half-width difference of their
        // content (±10pt is routine); near-duplicate tracks get coalesced
        // by the caller.
        if current_count == 0 || (x - current_anchor).abs() <= TABLE_SUBSET_TRACK_TOLERANCE_PT {
            current_sum += x;
            current_count += 1;
            current_anchor = current_sum / current_count as f32;
        } else {
            clusters.push((current_sum / current_count as f32, current_count));
            current_sum = x;
            current_count = 1;
            current_anchor = x;
        }
    }
    if current_count > 0 {
        clusters.push((current_sum / current_count as f32, current_count));
    }
    clusters
}

/// Bin a line into the union tracks via its gap-split cells when the raw-span
/// path fails: each cell maps to the nearest track (by start-x, within the
/// loose subset tolerance) and all cells must map to distinct tracks. Unlike
/// the body-detection paths this accepts 1-cell rows — inside a confirmed
/// table cluster a lone label at a track is a sparse row (e.g. a row-label
/// city with every value column empty), not prose.
fn sparse_row_via_cells(line: &ProjectedLine, tracks: &[f32]) -> Option<Vec<Cell>> {
    let cells = split_cells(line);
    if cells.is_empty() {
        return None;
    }
    let tol = TABLE_SUBSET_TRACK_TOLERANCE_PT;
    let mut mapping: Vec<usize> = Vec::with_capacity(cells.len());
    for c in &cells {
        let (idx, d) = tracks
            .iter()
            .enumerate()
            .map(|(i, &t)| (i, (c.start_x - t).abs()))
            .min_by(|a, b| a.1.total_cmp(&b.1))?;
        if d > tol {
            return None;
        }
        mapping.push(idx);
    }
    let mut distinct = mapping.clone();
    distinct.sort_unstable();
    distinct.dedup();
    if distinct.len() != mapping.len() {
        return None;
    }
    let mut row = vec![Cell::default(); tracks.len()];
    for (c, &idx) in cells.iter().zip(&mapping) {
        row[idx] = located_cell(c, line);
    }
    Some(row)
}

/// Walk upward from the cluster's body start, binning each candidate header
/// line's raw spans into the union tracks. A span covering multiple tracks
/// (a group header like `EFETIVO` spanning its `ENFERMARIA`/`QUARTO`
/// sub-columns) replicates its text across every covered track — the pipe
/// table flattening of a colspan'd grid header. Layers stack top-to-bottom
/// per column. Returns `(new_start, header)`.
fn union_header_from_above(
    lines: &[ProjectedLine],
    body_start: usize,
    floor: usize,
    tracks: &[f32],
) -> Option<(usize, Vec<Cell>)> {
    let tol = TABLE_TRACK_TOLERANCE_PT;
    // Fallback assignment for centered header cells whose extent doesn't
    // reach the track anchor: nearest track within half the local gap.
    let assign_nearest = |x_center: f32| -> Option<usize> {
        let (idx, d) = tracks
            .iter()
            .enumerate()
            .map(|(i, &t)| (i, (x_center - t).abs()))
            .min_by(|a, b| a.1.total_cmp(&b.1))?;
        let local_gap = if idx + 1 < tracks.len() {
            tracks[idx + 1] - tracks[idx]
        } else if idx > 0 {
            tracks[idx] - tracks[idx - 1]
        } else {
            f32::INFINITY
        };
        if d <= (local_gap * 0.5).max(TABLE_SUBSET_TRACK_TOLERANCE_PT) {
            Some(idx)
        } else {
            None
        }
    };
    let mut layers: Vec<Vec<Cell>> = Vec::new();
    let mut j = body_start;
    while j > floor && layers.len() < TABLE_CLUSTER_MAX_HEADER_LINES {
        let cand = j - 1;
        if !table_rows_adjacent(&lines[cand], &lines[j]) {
            break;
        }
        let spans: Vec<(&TextItem, Option<usize>)> = lines[cand]
            .spans
            .iter()
            .enumerate()
            .filter(|(_, s)| !s.text.trim().is_empty())
            .map(|(i, s)| (s, lines[cand].span_item_indices.get(i).copied()))
            .collect();
        if spans.len() < 2 {
            break;
        }
        let mut layer = vec![Cell::default(); tracks.len()];
        let mut ok = true;
        for (s, source_index) in &spans {
            let x0 = s.x;
            let x1 = s.x + s.width.max(0.0);
            let covered: Vec<usize> = tracks
                .iter()
                .enumerate()
                .filter(|&(_, &t)| t >= x0 - tol && t <= x1 + tol)
                .map(|(i, _)| i)
                .collect();
            let targets: Vec<usize> = if !covered.is_empty() {
                covered
            } else if let Some(idx) = assign_nearest((x0 + x1) * 0.5) {
                vec![idx]
            } else {
                ok = false;
                break;
            };
            // A span covering several tracks replicates into each of them, so
            // every covered cell reports the span it was flattened from.
            let span_rect = Rect {
                x: s.x,
                y: s.y,
                width: s.width.max(0.0),
                height: s.height.max(0.0),
            };
            for idx in targets {
                let dst = &mut layer[idx];
                if !dst.text.is_empty() {
                    dst.text.push(' ');
                }
                dst.text.push_str(s.text.trim());
                dst.add_indices(source_index.iter().copied());
                Rect::extend(&mut dst.bbox, &span_rect);
            }
        }
        if !ok || layer.iter().filter(|t| !t.text.is_empty()).count() < 2 {
            break;
        }
        layers.push(layer);
        j = cand;
    }
    if layers.is_empty() {
        return None;
    }
    layers.reverse();
    let header: Vec<Cell> = (0..tracks.len())
        .map(|col| {
            let mut parts: Vec<&str> = Vec::new();
            let mut indices: Vec<usize> = Vec::new();
            let mut bbox: Option<Rect> = None;
            for layer in &layers {
                let s = layer[col].as_str();
                if let Some(r) = &layer[col].bbox {
                    Rect::extend(&mut bbox, r);
                }
                if s.is_empty() {
                    continue;
                }
                // Like the bbox union above: a layer whose text de-duplicates
                // away still contributed real source items — attribute them.
                indices.extend(layer[col].text_item_indices.iter().copied());
                if parts.last() == Some(&s) {
                    continue;
                }
                parts.push(s);
            }
            dedup_preserving_order(&mut indices);
            Cell {
                text: parts.join(" "),
                bbox,
                text_item_indices: indices,
            }
        })
        .collect();
    if header.iter().all(|h| h.text.is_empty()) {
        return None;
    }
    Some((j, header))
}

/// Re-extract a cluster of adjacent table runs as one table against the
/// union track set. Returns `None` (leaving the original runs untouched)
/// when the union tracks are unsound or too many lines fail to bin.
fn build_union_table(
    cluster: &[TableRun],
    lines: &[ProjectedLine],
    floor: usize,
) -> Option<TableRun> {
    let dbgt = *super::flags::DEBUG_TABLE;
    let window_start = cluster.first()?.body_start;
    let window_end = cluster.last()?.end.min(lines.len());
    if window_start >= window_end {
        return None;
    }
    let mut supported = union_tracks_in_window(lines, window_start, window_end);
    // Drop low-support tracks: a column that only 1-2 cells across the whole
    // cluster ever start at is noise (wrapped-cell continuation indents,
    // intra-cell splits on widely-tracked text), not a real column.
    let window_len = window_end - window_start;
    let min_support = 2.max(window_len / 10);
    supported.retain(|&(_, n)| n >= min_support);
    // Coalesce adjacent tracks closer than a real column gutter — these are
    // start-x jitter of one center-aligned column (content of different
    // widths), not two columns. Real word-gap prose would coalesce down to a
    // handful of wide tracks and then fail the width check below.
    let mut font_sizes: Vec<f32> = lines[window_start..window_end]
        .iter()
        .map(|l| {
            if l.dominant_font_size > 0.0 {
                l.dominant_font_size
            } else {
                l.bbox.height.max(1.0)
            }
        })
        .collect();
    font_sizes.sort_by(f32::total_cmp);
    let median_font = font_sizes[font_sizes.len() / 2];
    let min_track_gap =
        (median_font * TABLE_MIN_TRACK_GAP_FONT_MULT).max(TABLE_MIN_TRACK_GAP_FLOOR_PT);
    let mut coalesced: Vec<(f32, usize)> = Vec::with_capacity(supported.len());
    for (t, n) in supported {
        match coalesced.last_mut() {
            Some((last, last_n)) if t - *last < min_track_gap => {
                // Support-weighted midpoint keeps the anchor near the
                // dominant alignment.
                *last = (*last * *last_n as f32 + t * n as f32) / (*last_n + n) as f32;
                *last_n += n;
            }
            _ => coalesced.push((t, n)),
        }
    }
    let tracks: Vec<f32> = coalesced.into_iter().map(|(t, _)| t).collect();
    if tracks.len() < TABLE_MIN_COLUMNS {
        return None;
    }
    // The union must be at least as wide as the widest fragment — otherwise
    // re-extraction would lose columns the per-run detection already found
    // (and a word-gap prose union that coalesced to nothing lands here too).
    let max_run_cols = cluster.iter().filter_map(run_column_count).max()?;
    if tracks.len() < max_run_cols {
        if dbgt {
            eprintln!(
                "[tbl-cluster] reject: {} tracks < widest fragment {max_run_cols} ([{}])",
                tracks.len(),
                tracks
                    .iter()
                    .map(|t| format!("{t:.0}"))
                    .collect::<Vec<_>>()
                    .join(",")
            );
        }
        return None;
    }

    // NOTE: no vertical wrap-merge here. Gap-based merging of multi-line
    // cells doesn't work — inside uniformly-leaded tables the gap between a
    // wrapped cell line and a genuine next row is identical, so any threshold
    // either shreds multi-line cells (no merge) or fuses adjacent logical rows
    // (merge). Re-extracted clusters keep one row per line; multi-line-cell
    // recovery needs a stronger signal (ruled-grid row boundaries, or
    // label-column row anchoring).
    let mut rows: Vec<Vec<Cell>> = Vec::new();
    let mut failed_count = 0usize;
    for line in &lines[window_start..window_end] {
        if let Some(cells) = cells_from_raw_items_with_tracks(line, &tracks) {
            if cells.iter().any(|c| !c.text.is_empty()) {
                rows.push(cells.iter().map(|c| located_cell(c, line)).collect());
            }
        } else if let Some(row) = sparse_row_via_cells(line, &tracks) {
            rows.push(row);
        } else {
            // The line couldn't bin but still carries content — keep its
            // full text as a column-0 row rather than dropping it. Counted
            // toward the abort threshold below.
            failed_count += 1;
            let text = line.text.trim();
            if !text.is_empty() {
                // Unbinnable line kept whole in column 0; it occupies the
                // entire line, so that is the cell's box.
                let mut row = vec![Cell::default(); tracks.len()];
                row[0] = Cell::located_with(
                    collapse_whitespace(text),
                    line.bbox.clone(),
                    line.span_item_indices.clone(),
                );
                rows.push(row);
            }
        }
    }
    let window_len = window_end - window_start;
    if (failed_count as f32) > (window_len as f32) * TABLE_CLUSTER_MAX_FAILED_ROW_FRAC {
        if dbgt {
            eprintln!("[tbl-cluster] reject: {failed_count}/{window_len} lines unbinnable");
        }
        return None;
    }
    if rows.len() < TABLE_MIN_ROWS {
        return None;
    }

    let absorbed = union_header_from_above(lines, window_start, floor, &tracks);
    let (start, header) = match absorbed {
        Some((hstart, header)) => (hstart, Some(header)),
        None => (window_start, None),
    };
    Some(TableRun {
        start,
        end: window_end,
        body_start: window_start,
        bbox: lines_bbox(lines, start, window_end),
        block: Block::Table { header, rows },
        block_indices: Vec::new(),
    })
}

fn run_column_count(run: &TableRun) -> Option<usize> {
    match &run.block {
        Block::Table { header, rows } => header
            .as_ref()
            .map(|h| h.len())
            .or_else(|| rows.first().map(|r| r.len())),
        _ => None,
    }
}

/// Re-derive column tracks from the run's source lines. Aggregates min start_x
/// and max end_x across *every* line whose `split_cells` count matches the
/// run's declared column count, so a column with tight per-row content (e.g.
/// a right-aligned numeric body cell) still produces a track wide enough to
/// match a wider header cell that aligns to the same column.
fn run_body_tracks(run: &TableRun, lines: &[ProjectedLine]) -> Option<Vec<(f32, f32)>> {
    let n_cols = run_column_count(run)?;
    let mut acc: Option<Vec<(f32, f32)>> = None;
    for line in &lines[run.start..run.end.min(lines.len())] {
        let cells = split_cells(line);
        if cells.len() != n_cols {
            continue;
        }
        let row: Vec<(f32, f32)> = cells.iter().map(|c| (c.start_x, c.end_x)).collect();
        acc = Some(match acc {
            None => row,
            Some(prev) => prev
                .into_iter()
                .zip(row)
                .map(|((ps, pe), (s, e))| (ps.min(s), pe.max(e)))
                .collect(),
        });
    }
    acc
}

fn tracks_align_same(a: &[(f32, f32)], b: &[(f32, f32)]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b.iter()).all(|(ta, tb)| {
        let ca = (ta.0 + ta.1) * 0.5;
        let cb = (tb.0 + tb.1) * 0.5;
        (ca - cb).abs() <= TABLE_TRACK_TOLERANCE_PT
    })
}

/// Score the alignment between two tracks. Returns `None` if they don't align.
/// Distance is `min(start_diff, end_diff, center_interior_match)` so a header
/// cell sitting at the edge of a wide body cell doesn't spuriously match.
fn subset_match_score(ta: (f32, f32), tb: (f32, f32), tol: f32) -> Option<f32> {
    let d_start = (ta.0 - tb.0).abs();
    let d_end = (ta.1 - tb.1).abs();
    let ca = (ta.0 + ta.1) * 0.5;
    // Center-in-range only counts when a's center falls in b's interior
    // half — guards against a narrow header touching the edge of a wide
    // row-label cell next door.
    let interior_lo = tb.0 + (tb.1 - tb.0) * 0.25;
    let interior_hi = tb.1 - (tb.1 - tb.0) * 0.25;
    let d_center = if ca >= interior_lo && ca <= interior_hi {
        0.0
    } else {
        f32::INFINITY
    };
    let d = d_start.min(d_end).min(d_center);
    if d <= tol { Some(d) } else { None }
}

/// Looser tolerance for cross-run subset matching. Header cells and body
/// cells often have different content widths (e.g. `(percent)` header is
/// 65pt wide while the body's `12` is 10pt wide), so the per-row track
/// tolerance is too tight here. Combined with the interior-only center
/// check in `subset_match_score`, this stays conservative.
const TABLE_SUBSET_TRACK_TOLERANCE_PT: f32 = 12.0;

/// Map A's tracks to B's tracks (requires `|A| + 1 == |B|`). Tries every
/// possible "skip one B column" assignment and picks the lowest-total-error
/// option. Returns `None` when no skip yields a fully-aligned mapping.
fn subset_mapping(a: &[(f32, f32)], b: &[(f32, f32)]) -> Option<Vec<usize>> {
    if a.len() + 1 != b.len() {
        return None;
    }
    let tol = TABLE_SUBSET_TRACK_TOLERANCE_PT;
    let mut best: Option<(Vec<usize>, f32)> = None;
    for skip in 0..b.len() {
        let mut mapping = Vec::with_capacity(a.len());
        let mut total = 0.0f32;
        let mut ok = true;
        for (i, &ai) in a.iter().enumerate() {
            let bi = if i < skip { i } else { i + 1 };
            match subset_match_score(ai, b[bi], tol) {
                Some(d) => {
                    mapping.push(bi);
                    total += d;
                }
                None => {
                    ok = false;
                    break;
                }
            }
        }
        if ok && best.as_ref().is_none_or(|(_, e)| total < *e) {
            best = Some((mapping, total));
        }
    }
    best.map(|(m, _)| m)
}

/// Insert empty strings into `row` so that its content lands at the mapped
/// columns in a `target_len`-wide row.
fn pad_row_to_layout(row: &[Cell], mapping: &[usize], target_len: usize) -> Vec<Cell> {
    // Padding cells are pure layout filler and stay geometry-less; the real
    // cells carry their boxes across into the new column positions.
    let mut out: Vec<Cell> = vec![Cell::default(); target_len];
    for (a_idx, &b_idx) in mapping.iter().enumerate() {
        if b_idx < target_len && a_idx < row.len() {
            out[b_idx] = row[a_idx].clone();
        }
    }
    out
}

/// Maximum number of non-table lines allowed between two runs being merged.
/// Each interstitial line must be a short single-cell label (category
/// divider, group header) to qualify — anything longer or multi-cell is
/// real content and rejects the merge.
const TABLE_MERGE_MAX_INTERSTITIAL: usize = 1;

/// Cap on the character count of an interstitial label line that the merge
/// will absorb as a body row.
const TABLE_MERGE_MAX_INTERSTITIAL_CHARS: usize = 60;

fn is_absorbable_interstitial(line: &ProjectedLine) -> bool {
    let cells = split_cells(line);
    if cells.len() > 1 {
        return false;
    }
    let text = line.text.trim();
    if text.len() > TABLE_MERGE_MAX_INTERSTITIAL_CHARS {
        return false;
    }
    // Reject sentence-shaped prose: ends in . ! ?  (a real label rarely does)
    if let Some(last) = text.chars().last()
        && matches!(last, '.' | '!' | '?')
        && text.len() > 6
    {
        return false;
    }
    true
}

fn try_merge_pair(a: &TableRun, b: &TableRun, lines: &[ProjectedLine]) -> Option<TableRun> {
    // Allow up to `TABLE_MERGE_MAX_INTERSTITIAL` short label lines between
    // A's end and B's start. Each interstitial gets preserved as a body
    // row of the merged table so no content is dropped.
    let interstitial = b.start.saturating_sub(a.end);
    if interstitial > TABLE_MERGE_MAX_INTERSTITIAL {
        return None;
    }
    // Absorbed label lines become single-cell body rows; each keeps the box of
    // the line it came from so the merged table's geometry stays complete.
    let interstitial_texts: Vec<Cell> = if interstitial == 0 {
        Vec::new()
    } else {
        let slice = &lines[a.end..b.start];
        if !slice.iter().all(is_absorbable_interstitial) {
            return None;
        }
        slice
            .iter()
            .map(|l| {
                Cell::located_with(
                    l.text.trim().to_string(),
                    l.bbox.clone(),
                    l.span_item_indices.clone(),
                )
            })
            .collect()
    };
    let (a_header, a_rows) = match &a.block {
        Block::Table { header, rows } => (header.clone(), rows.clone()),
        _ => return None,
    };
    let (b_header, b_rows) = match &b.block {
        Block::Table { header, rows } => (header.clone(), rows.clone()),
        _ => return None,
    };
    let a_cols = run_column_count(a)?;
    let b_cols = run_column_count(b)?;
    let a_tracks = run_body_tracks(a, lines)?;
    let b_tracks = run_body_tracks(b, lines)?;

    if a.end == 0 || a.end > lines.len() || b.start >= lines.len() {
        return None;
    }
    let a_last = &lines[a.end - 1];
    let b_first = &lines[b.start];
    let line_height = a_last.bbox.height.max(b_first.bbox.height).max(1.0);
    let gap = b_first.bbox.y - (a_last.bbox.y + a_last.bbox.height);
    if gap > line_height * TABLE_MERGE_MAX_Y_GAP_LINES {
        return None;
    }
    if gap < -line_height {
        return None;
    }

    // Case Same: identical tracks, concat rows.
    if a_cols == b_cols && tracks_align_same(&a_tracks, &b_tracks) {
        // Don't merge two complete-looking tables across a noticeable gap.
        let both_complete =
            a_header.is_some() && b_header.is_some() && a_rows.len() >= 3 && b_rows.len() >= 3;
        if both_complete && gap > line_height * 1.0 {
            return None;
        }
        let header = a_header.clone().or_else(|| b_header.clone());
        let mut rows = a_rows.clone();
        // Preserve interstitial label lines as body rows, content in col 0.
        for text in &interstitial_texts {
            let mut row = vec![Cell::default(); b_cols];
            row[0] = text.clone();
            rows.push(row);
        }
        // If both runs had explicit headers, we kept A's; preserve B's
        // header text as a body row so its content isn't dropped.
        if a_header.is_some()
            && b_header.is_some()
            && let Some(bh) = b_header.clone()
        {
            rows.push(bh);
        }
        rows.extend(b_rows.iter().cloned());
        return Some(TableRun {
            start: a.start,
            end: b.end,
            body_start: a.body_start,
            // The merged run spans both source runs and anything absorbed
            // between them.
            bbox: lines_bbox(lines, a.start, b.end),
            block: Block::Table { header, rows },
            block_indices: Vec::new(),
        });
    }

    // Case Subset: A has 1 fewer column; fold A into B's header.
    if a_cols + 1 == b_cols && a_rows.len() <= TABLE_HEADER_MAX_ABSORB_ROWS {
        let mapping = subset_mapping(&a_tracks, &b_tracks)?;

        // Compose header rows top-to-bottom: A.header -> A.rows -> B.header.
        let mut header_layers: Vec<Vec<Cell>> = Vec::new();
        if let Some(h) = &a_header {
            header_layers.push(pad_row_to_layout(h, &mapping, b_cols));
        }
        for row in &a_rows {
            header_layers.push(pad_row_to_layout(row, &mapping, b_cols));
        }
        if let Some(h) = &b_header {
            header_layers.push(h.clone());
        }
        if header_layers.is_empty() {
            return None;
        }
        let merged_header: Vec<Cell> = (0..b_cols)
            .map(|col| {
                let mut parts: Vec<String> = Vec::new();
                let mut indices: Vec<usize> = Vec::new();
                let mut bbox: Option<Rect> = None;
                for layer in &header_layers {
                    let Some(cell) = layer.get(col) else {
                        continue;
                    };
                    // Union every contributing layer's box, including ones
                    // whose text de-duplicates away, so the merged header cell
                    // still covers the full stacked band it was built from.
                    if let Some(r) = &cell.bbox {
                        Rect::extend(&mut bbox, r);
                    }
                    let s = cell.as_str();
                    if s.is_empty() {
                        continue;
                    }
                    // Same policy as the bbox union: attribute a layer's
                    // source items even when its text de-duplicates away.
                    indices.extend(cell.text_item_indices.iter().copied());
                    if parts.last().map(|p| p.as_str()) == Some(s) {
                        continue;
                    }
                    parts.push(s.to_string());
                }
                dedup_preserving_order(&mut indices);
                Cell {
                    text: parts.join(" "),
                    bbox,
                    text_item_indices: indices,
                }
            })
            .collect();
        // Preserve interstitial label lines as body rows ahead of B's rows.
        let mut merged_rows: Vec<Vec<Cell>> = Vec::new();
        for text in &interstitial_texts {
            let mut row = vec![Cell::default(); b_cols];
            row[0] = text.clone();
            merged_rows.push(row);
        }
        merged_rows.extend(b_rows.iter().cloned());
        return Some(TableRun {
            start: a.start,
            end: b.end,
            bbox: lines_bbox(lines, a.start, b.end),
            // A was folded into B's header, so the merged table's body
            // begins where B's body did.
            body_start: b.body_start,
            block: Block::Table {
                header: Some(merged_header),
                rows: merged_rows,
            },
            block_indices: Vec::new(),
        });
    }

    None
}

// ── Ruled-grid table detection ─────────────────────────────────────────────
//
// Detect tables drawn with explicit horizontal + vertical rules (the "Strong"
// mode in MARKDOWN_PLAN.md). Strokes are clustered into H/V grid lines, then
// union-find groups crossing lines into table regions. For each region the
// distinct row/column boundaries form a cell grid; text lines are assigned to
// cells by centroid containment.
//
// Ruled tables are detected before the borderless `detect_tables`. The caller
// merges the two outputs; overlapping ranges defer to the ruled run because
// path-based geometry is a strictly stronger signal than text alignment alone.

/// Horizontal segment in viewport coords (top-left origin). `y` is the rule's
/// y-position; `x_min..x_max` is its horizontal span. Endpoints of multiple
/// short segments sharing a y get unioned into one wider segment during
/// clustering.
#[derive(Debug, Clone, Copy)]
struct HSeg {
    x_min: f32,
    x_max: f32,
    y: f32,
}

#[derive(Debug, Clone, Copy)]
struct VSeg {
    y_min: f32,
    y_max: f32,
    x: f32,
}

/// Strokes are considered "axis-aligned" when the perpendicular delta is at
/// most this many points. Generous to absorb antialiased near-pixel strokes.
const TABLE_AXIS_TOLERANCE_PT: f32 = 1.0;

/// Two H lines (or two V lines) are merged into one grid line when their
/// perpendicular coords are within this many points. Slightly looser than the
/// axis tolerance because rules drawn at the same row can have ±1pt jitter
/// from different stroke widths.
const TABLE_GRID_CLUSTER_PT: f32 = 2.0;

/// Slack added when checking whether a V line "crosses" an H line. Helps
/// when rules don't quite reach the corner because the PDF drew them as
/// individual segments with small gaps.
const TABLE_CROSS_TOLERANCE_PT: f32 = 3.0;

/// Cluster tolerance for ruled column boundaries. Wider than
/// `TABLE_GRID_CLUSTER_PT` because adjacent cell-border rects draw paired
/// edges 4-6pt apart; real columns are ≥ ~10pt wide so the mean-centered
/// merge cannot eat a genuine narrow column.
const TABLE_COL_BOUNDARY_CLUSTER_PT: f32 = 6.0;

/// Reject ruled-table candidates whose empty-cell fraction exceeds this.
/// This can't be loosened to recover blank worksheets/forms: a real sparse
/// table (e.g. a 4-col version history, ~75% empty) and a spurious grid from
/// decorative layout boxes (e.g. a TOC, also ~75% empty) are indistinguishable
/// on empty-fraction, and relaxing it surfaces more false tables than real
/// forms recovered.
const TABLE_MAX_EMPTY_CELL_FRACTION: f32 = 0.30;

/// Fraction of a row or column that must be populated to qualify the grid as
/// a structural fill-in form (e.g. comparison charts with row labels + header
/// row but otherwise empty cells). When this signature is met, the empty-cell
/// fraction filter relaxes to `TABLE_MAX_EMPTY_CELL_FRACTION_WITH_SPINE`.
const TABLE_SPINE_FILL_FRACTION: f32 = 0.7;

/// Max characters in any single col-0 or row-0 cell when applying the spine
/// bypass. Real labels and headers are short (1-5 words ≈ 50 chars); a column
/// of multi-sentence prose triggers `col0_fill` but isn't a structural label
/// column.
const TABLE_SPINE_MAX_CELL_CHARS: usize = 60;

/// Ceiling on empty-cell fraction even when a spine is detected. Caps how
/// aggressively the fill-in-form bypass can override the base filter — past
/// 75% empty, even a strong spine isn't enough to distinguish from decorative
/// page chrome.
const TABLE_MAX_EMPTY_CELL_FRACTION_WITH_SPINE: f32 = 0.75;

/// Reject candidates whose grid covers nearly the whole page — almost always
/// a page border, not a real table.
const TABLE_MAX_PAGE_COVERAGE: f32 = 0.95;

/// Global-pass only: minimum fraction of the column extent a horizontal rule
/// must span to count as a row boundary. A full-height left/right border can
/// union a stray box (logo, sidebar) into a grid component through the shared
/// vertical line; that box's short rules would otherwise become phantom top
/// rows that vacuum surrounding prose into the table. Real row boundaries span
/// most of the table width.
const RULED_HLINE_MIN_COVERAGE: f32 = 0.5;

/// Minimum fraction of the component's row extent a vertical rule must span to
/// count as a column boundary, the mirror image of `RULED_HLINE_MIN_COVERAGE`.
///
/// Slide decks routinely draw a table as one stroked rect per cell, so a single
/// decorative highlight box behind a phrase *inside* a cell contributes a pair
/// of vertical edges that split that column in two for the whole table.
/// `collapse_gutter_columns` can't fuse the sliver back because text centres do
/// land inside it.
///
/// Kept well below the horizontal counterpart: a genuine divider that rules only
/// the header band of an otherwise open table is a real layout, and must survive.
const RULED_VLINE_MIN_COVERAGE: f32 = 0.2;

/// H/V rule segments extracted once from a page's graphics and shared by every
/// table-detection pass on the page. The global ruled pass, the per-region
/// ruled pass, the leaf-veto re-check, and the rule-band pass all consume the
/// same segments; extracting per call would redo the work once per region.
pub(super) struct RuleSegments {
    /// Horizontal segments, clustered by y.
    hs: Vec<HSeg>,
    /// Vertical segments as extracted, before same-x clustering. The component
    /// splitter needs these: clustering unions y-ranges across gaps.
    raw_vs: Vec<VSeg>,
    /// Vertical segments, clustered by x.
    vs: Vec<VSeg>,
}

pub(super) fn extract_rule_segments(graphics: &[GraphicPrimitive]) -> RuleSegments {
    let (hs, raw_vs) = extract_h_v_segments(graphics);
    let hs = cluster_h_segments(hs);
    let vs = cluster_v_segments(raw_vs.clone());
    RuleSegments { hs, raw_vs, vs }
}

/// Extract horizontal and vertical line segments from a page's graphics. Each
/// `Stroke` becomes one HSeg or VSeg depending on orientation; each stroked
/// `Rect` contributes its four edges (cell-border rects, table frames).
fn extract_h_v_segments(graphics: &[GraphicPrimitive]) -> (Vec<HSeg>, Vec<VSeg>) {
    let mut hs = Vec::new();
    let mut vs = Vec::new();
    for g in graphics {
        match g {
            GraphicPrimitive::Stroke { x1, y1, x2, y2, .. } => {
                let (x1, y1, x2, y2) = (*x1, *y1, *x2, *y2);
                let dy = (y1 - y2).abs();
                let dx = (x1 - x2).abs();
                if dy <= TABLE_AXIS_TOLERANCE_PT && dx > 1.0 {
                    hs.push(HSeg {
                        x_min: x1.min(x2),
                        x_max: x1.max(x2),
                        y: (y1 + y2) * 0.5,
                    });
                } else if dx <= TABLE_AXIS_TOLERANCE_PT && dy > 1.0 {
                    vs.push(VSeg {
                        y_min: y1.min(y2),
                        y_max: y1.max(y2),
                        x: (x1 + x2) * 0.5,
                    });
                }
            }
            GraphicPrimitive::Rect { bbox, stroke, .. } => {
                if stroke.is_none() {
                    continue;
                }
                let top = bbox.y;
                let bottom = bbox.y + bbox.height;
                let left = bbox.x;
                let right = bbox.x + bbox.width;
                if bbox.width > 1.0 {
                    hs.push(HSeg {
                        x_min: left,
                        x_max: right,
                        y: top,
                    });
                    hs.push(HSeg {
                        x_min: left,
                        x_max: right,
                        y: bottom,
                    });
                }
                if bbox.height > 1.0 {
                    vs.push(VSeg {
                        y_min: top,
                        y_max: bottom,
                        x: left,
                    });
                    vs.push(VSeg {
                        y_min: top,
                        y_max: bottom,
                        x: right,
                    });
                }
            }
        }
    }
    (hs, vs)
}

/// Cluster H segments sharing a y-coordinate (within `TABLE_GRID_CLUSTER_PT`)
/// into a single wider grid line whose x-extent is the union of the inputs.
fn cluster_h_segments(mut segs: Vec<HSeg>) -> Vec<HSeg> {
    if segs.is_empty() {
        return segs;
    }
    segs.sort_by(|a, b| a.y.total_cmp(&b.y));
    let mut out: Vec<HSeg> = Vec::with_capacity(segs.len());
    for seg in segs {
        if let Some(last) = out.last_mut()
            && (last.y - seg.y).abs() <= TABLE_GRID_CLUSTER_PT
        {
            last.x_min = last.x_min.min(seg.x_min);
            last.x_max = last.x_max.max(seg.x_max);
            continue;
        }
        out.push(seg);
    }
    out
}

fn cluster_v_segments(mut segs: Vec<VSeg>) -> Vec<VSeg> {
    if segs.is_empty() {
        return segs;
    }
    segs.sort_by(|a, b| a.x.total_cmp(&b.x));
    let mut out: Vec<VSeg> = Vec::with_capacity(segs.len());
    for seg in segs {
        if let Some(last) = out.last_mut()
            && (last.x - seg.x).abs() <= TABLE_GRID_CLUSTER_PT
        {
            last.y_min = last.y_min.min(seg.y_min);
            last.y_max = last.y_max.max(seg.y_max);
            continue;
        }
        out.push(seg);
    }
    out
}

/// Union-find root with path compression.
fn uf_find(parent: &mut [usize], mut x: usize) -> usize {
    while parent[x] != x {
        parent[x] = parent[parent[x]];
        x = parent[x];
    }
    x
}

fn uf_union(parent: &mut [usize], a: usize, b: usize) {
    let ra = uf_find(parent, a);
    let rb = uf_find(parent, b);
    if ra != rb {
        parent[ra] = rb;
    }
}

/// Group H/V grid lines that cross each other into connected components.
/// Each component is a candidate ruled table — typically one component per
/// distinct table on the page. Returns `(h_indices, v_indices)` per component,
/// dropping components without ≥2 H and ≥2 V lines (a single L-shape doesn't
/// make a table).
fn find_grid_components(hs: &[HSeg], vs: &[VSeg]) -> Vec<(Vec<usize>, Vec<usize>)> {
    let n_h = hs.len();
    let n_v = vs.len();
    if n_h < 2 || n_v < 2 {
        return Vec::new();
    }
    let n = n_h + n_v;
    let mut parent: Vec<usize> = (0..n).collect();
    let mut connected = vec![false; n];

    let tol = TABLE_CROSS_TOLERANCE_PT;
    for (i, h) in hs.iter().enumerate() {
        for (j, v) in vs.iter().enumerate() {
            let v_crosses_h_x = v.x >= h.x_min - tol && v.x <= h.x_max + tol;
            let h_crosses_v_y = h.y >= v.y_min - tol && h.y <= v.y_max + tol;
            if v_crosses_h_x && h_crosses_v_y {
                uf_union(&mut parent, i, n_h + j);
                connected[i] = true;
                connected[n_h + j] = true;
            }
        }
    }

    use std::collections::HashMap;
    let mut groups: HashMap<usize, (Vec<usize>, Vec<usize>)> = HashMap::new();
    for (i, &is_connected) in connected[..n_h].iter().enumerate() {
        if !is_connected {
            continue;
        }
        let r = uf_find(&mut parent, i);
        groups.entry(r).or_default().0.push(i);
    }
    for j in 0..n_v {
        if !connected[n_h + j] {
            continue;
        }
        let r = uf_find(&mut parent, n_h + j);
        groups.entry(r).or_default().1.push(j);
    }
    let mut comps: Vec<(Vec<usize>, Vec<usize>)> = groups
        .into_values()
        .filter(|(h_idx, v_idx)| h_idx.len() >= 2 && v_idx.len() >= 2)
        .collect();
    // `HashMap::into_values` yields components in nondeterministic order, which
    // leaks into table emission order and downstream overlap resolution. Sort
    // by the topmost horizontal-segment index (h_idx is ascending by
    // construction) so the output is stable run-to-run.
    comps.sort_by_key(|(h_idx, _)| h_idx[0]);
    comps
}

/// Which ruled-table detection pass is running. The global pass reassembles a
/// grid from page-level graphics (lines arrive in xy_cut leaf order and grid
/// rules may include short decorative horizontals), so it applies extra
/// filtering the per-region pass does not need.
#[derive(Clone, Copy, PartialEq, Eq)]
enum RuledPass {
    PerRegion,
    Global,
}

/// Filter a vector down to the elements whose `keep` flag is set. `keep` must
/// be at least as long as `items`.
fn filter_by<T>(items: Vec<T>, keep: &[bool]) -> Vec<T> {
    items
        .into_iter()
        .zip(keep.iter())
        .filter(|(_, k)| **k)
        .map(|(v, _)| v)
        .collect()
}

/// The per-cell grids `build_ruled_table` fills, collapses, and reads as one
/// unit. Bundling them lets the row/column-collapse passes filter every layer
/// through a single `retain_rows`/`retain_cols` call instead of five parallel
/// `filter().map().collect()` chains — removing the "forgot to filter one
/// array" bug class.
struct CellGrid {
    /// Rendered cell text (each multi-column span whitespace-split at the
    /// crossed column boundaries), each carrying the ruled cell it occupies.
    text: Vec<Vec<Cell>>,
    /// Per-cell bold flag — starts `true`, cleared when any non-bold span
    /// lands in the cell.
    is_bold: Vec<Vec<bool>>,
    /// Per-cell "carries text" flag.
    has_text: Vec<Vec<bool>>,
    /// Colspan-semantics grid: a span crossing ≥2 column boundaries replicates
    /// its FULL text into every covered cell instead of being split. Consulted
    /// only for header-band flattening — a group header like "North America"
    /// centered over its sub-columns must replicate, while the same geometry in
    /// a data row is a merged run that should split.
    repl: Vec<Vec<String>>,
    /// Source-item indices behind each `repl` cell, in the same push order.
    /// A parallel layer (rather than a Cell) because `repl` is a plain text
    /// grid; it filters through every retain in lockstep. The `text` layer
    /// needs no twin — its cells carry their own indices.
    repl_indices: Vec<Vec<Vec<usize>>>,
    /// Per-row flag: the row contains an alpha-dominant multi-column span (a
    /// group-header label, as opposed to a mostly-digit merged data run).
    row_alpha_spanner: Vec<bool>,
    /// Per-cell flag: the rules say this cell is vertically merged with the one
    /// above it, so it is empty by construction. See `count_rowspan_cells`.
    /// Travels with the row/column collapses so the density gate forgives the
    /// empties that are actually here, not ones from a discarded row.
    spanned: Vec<Vec<bool>>,
    /// Fraction of raw spans that cross an interior column boundary. Recorded
    /// so `build_ruled_table` can tell whether dropping short vertical rules
    /// actually stopped the grid cutting through text.
    straddle_frac: f32,
}

impl CellGrid {
    /// `xs`/`ys` are the column/row boundaries (lengths `n_cols + 1` /
    /// `n_rows + 1`). Unlike the borderless detectors, a ruled table has real
    /// drawn boundaries, so every cell's box is known up front — before any
    /// text lands in it — and the collapse/merge passes below carry those boxes
    /// with the text they reshape instead of re-deriving them from indices that
    /// no longer line up.
    fn new(n_rows: usize, n_cols: usize, xs: &[f32], ys: &[f32]) -> Self {
        let text = (0..n_rows)
            .map(|r| {
                (0..n_cols)
                    .map(|c| Cell {
                        text: String::new(),
                        bbox: Some(Rect {
                            x: xs[c],
                            y: ys[r],
                            width: xs[c + 1] - xs[c],
                            height: ys[r + 1] - ys[r],
                        }),
                        text_item_indices: Vec::new(),
                    })
                    .collect()
            })
            .collect();
        CellGrid {
            text,
            is_bold: vec![vec![true; n_cols]; n_rows],
            has_text: vec![vec![false; n_cols]; n_rows],
            repl: vec![vec![String::new(); n_cols]; n_rows],
            repl_indices: vec![vec![Vec::new(); n_cols]; n_rows],
            row_alpha_spanner: vec![false; n_rows],
            spanned: vec![vec![false; n_cols]; n_rows],
            straddle_frac: 0.0,
        }
    }

    fn n_rows(&self) -> usize {
        self.text.len()
    }

    fn n_cols(&self) -> usize {
        self.text.first().map(|r| r.len()).unwrap_or(0)
    }

    /// Append trimmed text to a cell, marking it as text-bearing and
    /// attributing the contributing source items. No-op for blank input (no
    /// attribution either — provenance follows the text that actually landed).
    fn push_text(&mut self, row: usize, col: usize, txt: &str, sources: &[usize]) {
        let txt = txt.trim();
        if txt.is_empty() {
            return;
        }
        if !self.text[row][col].text.is_empty() {
            self.text[row][col].text.push(' ');
        }
        self.text[row][col].text.push_str(txt);
        self.text[row][col]
            .text_item_indices
            .extend_from_slice(sources);
        self.has_text[row][col] = true;
    }

    /// Append trimmed text to the colspan-semantics (`repl`) cell. No-op for
    /// blank input.
    fn push_repl(&mut self, row: usize, col: usize, txt: &str, sources: &[usize]) {
        let txt = txt.trim();
        if txt.is_empty() {
            return;
        }
        let dst = &mut self.repl[row][col];
        if !dst.is_empty() {
            dst.push(' ');
        }
        dst.push_str(txt);
        self.repl_indices[row][col].extend_from_slice(sources);
    }

    /// Keep only rows `r` where `keep[r]`; every layer filters identically.
    fn retain_rows(&mut self, keep: &[bool]) {
        self.text = filter_by(std::mem::take(&mut self.text), keep);
        self.is_bold = filter_by(std::mem::take(&mut self.is_bold), keep);
        self.has_text = filter_by(std::mem::take(&mut self.has_text), keep);
        self.repl = filter_by(std::mem::take(&mut self.repl), keep);
        self.repl_indices = filter_by(std::mem::take(&mut self.repl_indices), keep);
        self.row_alpha_spanner = filter_by(std::mem::take(&mut self.row_alpha_spanner), keep);
        self.spanned = filter_by(std::mem::take(&mut self.spanned), keep);
    }

    /// Keep only columns `c` where `keep[c]`; the per-cell layers filter, the
    /// per-row `row_alpha_spanner` is unaffected.
    fn retain_cols(&mut self, keep: &[bool]) {
        self.text = std::mem::take(&mut self.text)
            .into_iter()
            .map(|row| filter_by(row, keep))
            .collect();
        self.is_bold = std::mem::take(&mut self.is_bold)
            .into_iter()
            .map(|row| filter_by(row, keep))
            .collect();
        self.has_text = std::mem::take(&mut self.has_text)
            .into_iter()
            .map(|row| filter_by(row, keep))
            .collect();
        self.repl = std::mem::take(&mut self.repl)
            .into_iter()
            .map(|row| filter_by(row, keep))
            .collect();
        self.repl_indices = std::mem::take(&mut self.repl_indices)
            .into_iter()
            .map(|row| filter_by(row, keep))
            .collect();
        self.spanned = std::mem::take(&mut self.spanned)
            .into_iter()
            .map(|row| filter_by(row, keep))
            .collect();
    }

    /// Drop "phantom rows" produced by stacked thin border-strip rects (some
    /// generators rule each visual row as top-strip ~1pt / body ~22pt /
    /// bottom-strip ~5pt, each surviving the 2pt clustering as its own grid
    /// row). A row is dropped
    /// iff it has no text in any cell AND its height is < 80% of the median
    /// non-empty row height — the height gate preserves real fill-in-the-blank
    /// forms whose empty body rows are full height. `ys` are the row boundaries
    /// (length `n_rows + 1`). Returns the kept rows' heights.
    fn collapse_phantom_rows(&mut self, ys: &[f32]) -> Vec<f32> {
        let n_rows = self.n_rows();
        let row_heights: Vec<f32> = (0..n_rows).map(|r| ys[r + 1] - ys[r]).collect();
        let nonempty_heights: Vec<f32> = (0..n_rows)
            .filter(|r| self.has_text[*r].iter().any(|t| *t))
            .map(|r| row_heights[r])
            .collect();
        let median_h = if !nonempty_heights.is_empty() {
            let mut s = nonempty_heights.clone();
            s.sort_by(|a, b| a.total_cmp(b));
            s[s.len() / 2]
        } else {
            let mut s = row_heights.clone();
            s.sort_by(|a, b| a.total_cmp(b));
            s[s.len() / 2]
        };
        let keep: Vec<bool> = (0..n_rows)
            .map(|r| {
                let has_text = self.has_text[r].iter().any(|t| *t);
                has_text || row_heights[r] >= median_h * 0.8
            })
            .collect();
        let kept_row_heights: Vec<f32> = (0..n_rows)
            .filter(|r| keep[*r])
            .map(|r| row_heights[r])
            .collect();
        self.retain_rows(&keep);
        kept_row_heights
    }

    /// Mirror `collapse_phantom_rows` on columns: some ruled tables draw
    /// left/right borders as thin strip rects ~5pt wide, which become
    /// phantom text-less columns. Drop a column iff it is both empty AND
    /// narrower than 30% of the median text-bearing column. `xs` are the column
    /// boundaries (length `n_cols + 1`).
    fn collapse_phantom_cols(&mut self, xs: &[f32]) {
        let n_rows = self.n_rows();
        let n_cols = self.n_cols();
        let col_widths: Vec<f32> = (0..n_cols).map(|c| xs[c + 1] - xs[c]).collect();
        let nonempty_col_widths: Vec<f32> = (0..n_cols)
            .filter(|c| (0..n_rows).any(|r| self.has_text[r][*c]))
            .map(|c| col_widths[c])
            .collect();
        let median_w = if !nonempty_col_widths.is_empty() {
            let mut s = nonempty_col_widths.clone();
            s.sort_by(|a, b| a.total_cmp(b));
            s[s.len() / 2]
        } else {
            let mut s = col_widths.clone();
            s.sort_by(|a, b| a.total_cmp(b));
            s[s.len() / 2]
        };
        let keep_col: Vec<bool> = (0..n_cols)
            .map(|c| {
                let has_text = (0..n_rows).any(|r| self.has_text[r][c]);
                has_text || col_widths[c] >= median_w * 0.3
            })
            .collect();
        self.retain_cols(&keep_col);
    }
}

/// Bin each line's text into the grid cells defined by column boundaries `xs`
/// and row boundaries `ys`. A projected line frequently spans several ruled
/// columns (one baseline = one line), so binning whole lines by centroid would
/// lump an entire row into one cell and leave the rest empty — the empty-cell
/// filter would then reject the real table. Instead bin each line's raw spans
/// by span center; spans whose x-extent crosses one or more interior column
/// boundaries are split at the whitespace nearest each crossed boundary (same
/// interpolation as the inferred-track path). Returns the filled grid and the
/// consumed line indices, or `None` if no line landed inside the grid or too
/// many spans straddle interior boundaries (a decorative box over prose, not a
/// table).
fn assign_cells(
    lines: &[ProjectedLine],
    xs: &[f32],
    ys: &[f32],
    dbg: bool,
) -> Option<(CellGrid, Vec<usize>)> {
    let n_rows = ys.len() - 1;
    let n_cols = xs.len() - 1;
    let mut grid = CellGrid::new(n_rows, n_cols, xs, ys);
    let mut consumed_indices: Vec<usize> = Vec::new();
    const GRID_X_SLACK_PT: f32 = 6.0;
    // Straddle census: spans that cross an interior column boundary by a
    // clear margin on both sides. A real ruled table keeps text inside cells
    // (the occasional PDFium merged run aside); decorative slide/layout boxes
    // over flowing prose slice through most runs.
    const STRADDLE_MARGIN_PT: f32 = 3.0;
    let mut span_total = 0usize;
    let mut span_straddle = 0usize;

    // Iterate lines top-to-bottom. The global pass receives lines in xy_cut
    // leaf order (column-by-column), which scrambles a multi-line cell's text
    // when concatenated in array order; y-sorted iteration restores reading
    // order within each cell. Per-region lines are already y-ordered, so this
    // is a no-op there.
    let mut line_order: Vec<usize> = (0..lines.len()).collect();
    line_order.sort_by(|&a, &b| lines[a].bbox.y.total_cmp(&lines[b].bbox.y));
    for idx in line_order {
        let line = &lines[idx];
        let cy = line.bbox.y + line.bbox.height * 0.5;
        if cy < ys[0] || cy > ys[n_rows] {
            continue;
        }
        // Line-level row bucket. Used by the no-spans fallback and as the
        // default when a span's own y doesn't resolve. Spans bin by their own
        // baseline below — a projected line can merge text from several ruled
        // rows (e.g. a multi-baseline wrapped header emitted as one line).
        let row = match find_bucket(ys, cy) {
            Some(r) => r,
            None => continue,
        };
        if line.text.trim().is_empty() {
            continue;
        }

        // Only consume the line when its text sits inside the grid
        // horizontally — a line poking past the grid edge belongs (at least
        // partly) to surrounding prose, and consuming it would lose text.
        let line_x0 = line.bbox.x;
        let line_x1 = line.bbox.x + line.bbox.width;
        if line_x0 < xs[0] - GRID_X_SLACK_PT || line_x1 > xs[n_cols] + GRID_X_SLACK_PT {
            if dbg {
                eprintln!(
                    "[ruled]   skip-overhang row={row} x={line_x0:.0}..{line_x1:.0} grid={:.0}..{:.0} text={:?}",
                    xs[0],
                    xs[n_cols],
                    &line.text.chars().take(60).collect::<String>()
                );
            }
            continue;
        }

        // Spans paired with their source-item indices (positional zip; `.get`
        // so a misaligned index vector degrades to unattributed spans).
        let mut text_spans: Vec<(&TextItem, Option<usize>)> = line
            .spans
            .iter()
            .enumerate()
            .filter(|(_, s)| !s.text.trim().is_empty())
            .map(|(i, s)| (s, line.span_item_indices.get(i).copied()))
            .collect();
        text_spans.sort_by(|a, b| a.0.x.total_cmp(&b.0.x));

        if text_spans.is_empty() {
            // No raw text spans (synthetic/OCR lines): old whole-line centroid path.
            let cx = line.bbox.x + line.bbox.width * 0.5;
            if let Some(col) = find_bucket(xs, cx.clamp(xs[0], xs[n_cols])) {
                grid.push_text(row, col, &line.text, &line.span_item_indices);
                grid.push_repl(row, col, &line.text, &line.span_item_indices);
                if !line.all_bold {
                    grid.is_bold[row][col] = false;
                }
                consumed_indices.push(idx);
            }
            continue;
        }

        for (span, source_index) in text_spans {
            // One-element source slice for the push helpers; empty when the
            // span carries no attribution.
            let src_buf: Vec<usize> = source_index.into_iter().collect();
            let srcs: &[usize] = &src_buf;
            // Per-span row: a span carries its own baseline y, which may sit
            // in a different ruled row than the line centroid.
            let span_cy = span.y + span.height * 0.5;
            let row = find_bucket(ys, span_cy.clamp(ys[0], ys[n_rows])).unwrap_or(row);
            let sx0 = (span.x).clamp(xs[0], xs[n_cols]);
            let sx1 = (span.x + span.width).clamp(xs[0], xs[n_cols]);
            let c_lo = find_bucket(xs, sx0).unwrap_or(0);
            let c_hi = find_bucket(xs, sx1).unwrap_or(n_cols - 1);
            span_total += 1;
            {
                let m0 = (span.x + STRADDLE_MARGIN_PT).clamp(xs[0], xs[n_cols]);
                let m1 = (span.x + span.width - STRADDLE_MARGIN_PT).clamp(xs[0], xs[n_cols]);
                if m1 > m0 && find_bucket(xs, m0) != find_bucket(xs, m1) {
                    span_straddle += 1;
                }
            }
            if c_lo == c_hi {
                grid.push_text(row, c_lo, &span.text, srcs);
                grid.push_repl(row, c_lo, &span.text, srcs);
                if !line.all_bold {
                    grid.is_bold[row][c_lo] = false;
                }
                continue;
            }
            // Multi-column span: replicate the full text into every covered
            // cell of the colspan-semantics grid, and flag the row when the
            // label is alpha-dominant (group headers are words; merged data
            // runs are mostly digits).
            for col in c_lo..=c_hi {
                grid.push_repl(row, col, &span.text, srcs);
            }
            if is_alpha_dominant(&span.text) {
                grid.row_alpha_spanner[row] = true;
            }
            // Span crosses interior boundaries: split at the whitespace
            // nearest each crossed boundary x (xs[k] is column k's left
            // boundary, which is exactly the split target).
            let covered: Vec<usize> = (c_lo..=c_hi).collect();
            // Prefer real word geometry: each word lands in the ruled column
            // its own centre falls in, which is exact. `split_span_at_anchors`
            // has to guess an x from a character index, so on a proportional
            // font it can cut a word or two off from the true boundary.
            if let Some(by_word) = bucket_words_into_columns(span, xs) {
                for (col, text) in by_word {
                    // A straddling span attributes its source item to every
                    // cell that receives a fragment of it.
                    grid.push_text(row, col, &text, srcs);
                    if !line.all_bold {
                        grid.is_bold[row][col] = false;
                    }
                }
            } else if let Some(pieces) = split_span_at_anchors(span, &covered, xs) {
                for (k, piece) in pieces.iter().enumerate() {
                    grid.push_text(row, c_lo + k, piece, srcs);
                    if !line.all_bold {
                        grid.is_bold[row][c_lo + k] = false;
                    }
                }
            } else {
                // No whitespace to split on — assign whole span by center.
                let cx = (span.x + span.width * 0.5).clamp(xs[0], xs[n_cols]);
                if let Some(col) = find_bucket(xs, cx) {
                    grid.push_text(row, col, &span.text, srcs);
                    if !line.all_bold {
                        grid.is_bold[row][col] = false;
                    }
                }
            }
        }
        consumed_indices.push(idx);
    }

    if consumed_indices.is_empty() {
        if dbg {
            eprintln!("[ruled]   REJECT no-lines-consumed");
        }
        return None;
    }

    let straddle_frac = if span_total > 0 {
        span_straddle as f32 / span_total as f32
    } else {
        0.0
    };
    if dbg {
        eprintln!("[ruled]   straddle {span_straddle}/{span_total} = {straddle_frac:.2}");
    }
    if span_total >= 6 && straddle_frac > 0.45 {
        if dbg {
            eprintln!("[ruled]   REJECT straddle-frac {straddle_frac:.2}");
        }
        return None;
    }
    grid.straddle_frac = straddle_frac;

    Some((grid, consumed_indices))
}

/// Colspan header-band flattening: a sparse top row whose alpha spanning label
/// covers several columns ("North America" over its Revenue/Units sub-columns)
/// followed by a dense label row is a stacked header. Flatten rows `0..=b` from
/// the colspan-semantics (`repl`) grid into one header row (per-column
/// top-to-bottom join), so each column carries its full layer chain ("North
/// America Revenue") — that chain is what header-keyed consumers (and readers)
/// need. Returns `(header_row, body_start)` or `None`.
#[allow(clippy::too_many_arguments)]
fn flatten_header_band(
    cells: &[Vec<Cell>],
    cell_has_text: &[Vec<bool>],
    cells_repl: &[Vec<String>],
    cells_repl_indices: &[Vec<Vec<usize>>],
    row_alpha_spanner: &[bool],
    n_rows: usize,
    n_cols: usize,
    dbg: bool,
) -> Option<(Vec<Cell>, usize)> {
    let row_fill =
        |r: usize| cell_has_text[r].iter().filter(|t| **t).count() as f32 / n_cols as f32;
    (0..n_rows)
        .find(|r| row_fill(*r) >= TABLE_ROW_MIN_FILL)
        .and_then(|b| {
            let nonempty = cell_has_text[b].iter().filter(|t| **t).count();
            let alpha_cells = (0..n_cols)
                .filter(|c| cell_has_text[b][*c] && is_alpha_dominant(cells[b][*c].as_str()))
                .count();
            // The bottom header layer may carry digit labels (years like
            // "2024") but never measurement values. A decimal / % / $ /
            // dash-placeholder / comma-grouped number in the anchor row means
            // it's the first DATA row of a table whose real header just
            // missed the 0.9-fill anchor (colspan header covering <90% of
            // columns) — folding it would eat a data row (DS5795A_page4).
            let has_value_cell =
                (0..n_cols).any(|c| cell_has_text[b][c] && is_value_like(cells[b][c].as_str()));
            let qualifies = (1..=3).contains(&b)
                && b + 1 < n_rows
                && (0..b).any(|r| row_alpha_spanner[r])
                && alpha_cells * 2 >= nonempty
                && !has_value_cell;
            if !qualifies {
                return None;
            }
            // Text comes from the colspan-replicated grid, but geometry comes
            // from the real cell grid: the flattened header cell covers every
            // band row 0..=b it was folded from.
            let header: Vec<Cell> = (0..n_cols)
                .map(|c| {
                    let mut parts: Vec<&str> = Vec::new();
                    let mut indices: Vec<usize> = Vec::new();
                    for (row, row_idx) in cells_repl
                        .iter()
                        .take(b + 1)
                        .zip(cells_repl_indices.iter())
                    {
                        let s = row[c].as_str();
                        if s.is_empty() {
                            continue;
                        }
                        // A band row whose text de-duplicates away still
                        // contributed real source items (two stacked rows can
                        // repeat a group label from *different* items) —
                        // attribute them even when the text is skipped.
                        indices.extend(row_idx[c].iter().copied());
                        if parts.last() == Some(&s) {
                            continue;
                        }
                        parts.push(s);
                    }
                    // A spanning header replicated across band rows can repeat
                    // a source item; a cell never lists one twice.
                    dedup_preserving_order(&mut indices);
                    let mut bbox: Option<Rect> = None;
                    for row in cells.iter().take(b + 1) {
                        if let Some(r) = &row[c].bbox {
                            Rect::extend(&mut bbox, r);
                        }
                    }
                    Cell {
                        text: parts.join(" "),
                        bbox,
                        text_item_indices: indices,
                    }
                })
                .collect();
            if header.iter().all(|h| h.text.is_empty()) {
                return None;
            }
            if dbg {
                let texts: Vec<&str> = header.iter().map(|h| h.as_str()).collect();
                eprintln!("[ruled]   colspan header flatten: rows 0..={b} -> {texts:?}");
            }
            Some((header, b + 1))
        })
}

/// Stacked-header merge: some generators rule every text baseline of a
/// wrapped header cell, slicing one logical header row into several thin sparse
/// rows ("Rated" / "Voltage" / "(VDC)" each in its own band). When the top
/// `k ≥ 2` rows are individually sparse but their union covers most columns AND
/// a dense data row follows, collapse them into a single header row (per-column
/// top-to-bottom join). `flattened` is whether `flatten_header_band` already
/// fired (the two are mutually exclusive). Returns the possibly-merged grid
/// plus a flag for whether the merge happened.
#[allow(clippy::type_complexity)]
fn merge_stacked_header(
    cells: Vec<Vec<Cell>>,
    cell_has_text: Vec<Vec<bool>>,
    cell_is_bold: Vec<Vec<bool>>,
    n_rows: usize,
    n_cols: usize,
    kept_row_heights: &[f32],
    flattened: bool,
    dbg: bool,
) -> (Vec<Vec<Cell>>, Vec<Vec<bool>>, Vec<Vec<bool>>, usize, bool) {
    let row_fill =
        |r: usize, has: &[Vec<bool>]| has[r].iter().filter(|t| **t).count() as f32 / n_cols as f32;
    // Anchor on the first fully-dense row (the first real data row).
    let k = (0..n_rows)
        .find(|r| row_fill(*r, &cell_has_text) >= TABLE_ROW_MIN_FILL)
        .unwrap_or(0);
    let union_cols = (0..n_cols)
        .filter(|c| (0..k).any(|r| cell_has_text[r][*c]))
        .count();
    // Tightness: the band rows must be noticeably shorter than the data
    // rows below — ruled-per-baseline header bands sit at text leading
    // (~7pt) while real rows carry cell padding. Without this, a table
    // whose first body rows are legitimately sparse would get merged.
    let band_tight = if k >= 2 && k < kept_row_heights.len() {
        let mut below: Vec<f32> = kept_row_heights[k..].to_vec();
        below.sort_by(|a, b| a.total_cmp(b));
        let median_below = below[below.len() / 2];
        kept_row_heights[..k]
            .iter()
            .all(|h| *h <= 0.75 * median_below)
    } else {
        false
    };
    if flattened || k < 2 || k >= n_rows || !band_tight || (union_cols as f32) < 0.7 * n_cols as f32
    {
        return (cells, cell_has_text, cell_is_bold, n_rows, false);
    }
    if dbg {
        eprintln!("[ruled]   stacked-header merge: top {k} rows → 1");
    }
    let mut merged_row = vec![Cell::default(); n_cols];
    let mut merged_has = vec![false; n_cols];
    let mut merged_bold = vec![true; n_cols];
    for r in 0..k {
        for c in 0..n_cols {
            // Every folded band row grows the merged cell's box, including the
            // text-less ones: the logical header cell spans the whole band the
            // generator sliced it into, not just the rows that carried glyphs.
            if let Some(rect) = &cells[r][c].bbox {
                Rect::extend(&mut merged_row[c].bbox, rect);
            }
            if cell_has_text[r][c] {
                if !merged_row[c].text.is_empty() {
                    merged_row[c].text.push(' ');
                }
                merged_row[c].text.push_str(cells[r][c].as_str());
                merged_row[c].add_indices(cells[r][c].text_item_indices.iter().copied());
                merged_has[c] = true;
                if !cell_is_bold[r][c] {
                    merged_bold[c] = false;
                }
            }
        }
    }
    let mut new_cells = vec![merged_row];
    let mut new_has = vec![merged_has];
    let mut new_bold = vec![merged_bold];
    new_cells.extend(cells[k..].iter().cloned());
    new_has.extend(cell_has_text[k..].iter().cloned());
    new_bold.extend(cell_is_bold[k..].iter().cloned());
    let nr = new_cells.len();
    (new_cells, new_has, new_bold, nr, true)
}

/// Density gate: a grid that is mostly empty cells is rejected unless it shows
/// strong table evidence. Cells the rules say are vertically merged into the
/// cell above (`spanned`) don't count as empty at all - they are empty by
/// construction, and a rowspan label column ("1. Embodying sustainability
/// values" beside three competence rows) otherwise reads as a sparse grid and
/// dies here. Beyond that, three escape hatches keep real tables:
/// - **col0 spine**: a filled, short-text first column (a label column).
/// - **long-prose table**: a large (≥5×3) grid with a bold header band covering
///   ≥3 columns and a dense (≥70%-fill) inner description column — a
///   multi-line legal/reference table whose description wraps over many empty
///   continuation rows.
/// - **flattened header**: a fired colspan header flatten is table evidence on
///   par with a spine.
///
/// With a spine but above the higher WITH_SPINE ceiling, still reject (unless a
/// long-prose table). `flattened` is whether `flatten_header_band` fired.
///
/// Returns `true` to keep the table, `false` to reject it.
///
/// Mark the grid cells that are *vertically merged* with the cell above them,
/// read straight off the rules: a cell whose top boundary carries no horizontal
/// stroke across its own column is the continuation of a rowspan.
///
/// This is the geometric explanation for legitimately-empty cells. Every
/// interior `ys` boundary exists because *some* horizontal segment sits there,
/// so a column only scores when that particular rule stops short of it - which
/// is exactly how a PDF draws a merged cell. A fully-ruled grid scores zero.
fn rowspan_mask(
    hs: &[HSeg],
    h_indices: &[usize],
    vs: &[VSeg],
    v_indices: &[usize],
    xs: &[f32],
    ys: &[f32],
) -> Vec<Vec<bool>> {
    let tol = TABLE_GRID_CLUSTER_PT;
    let mut mask = vec![vec![false; xs.len() - 1]; ys.len() - 1];
    for (r, &y) in ys[1..ys.len() - 1].iter().enumerate() {
        // Row `r + 1` is the one below boundary `ys[r + 1]`.
        let row = r + 1;
        // Some vertical rule must run *through* the boundary. That is what
        // distinguishes "the grid continues, this one cell is merged" from
        // "the grid ended here" - a page-frame component whose stray rules
        // cross unruled prose has no vertical continuing past them, and
        // forgiving its empties would turn body text into a two-column table.
        let grid_continues = v_indices
            .iter()
            .map(|&i| &vs[i])
            .any(|v| v.y_min <= y - tol && v.y_max >= y + tol);
        if !grid_continues {
            continue;
        }
        let at_y: Vec<&HSeg> = h_indices
            .iter()
            .map(|&i| &hs[i])
            .filter(|h| (h.y - y).abs() <= tol)
            .collect();
        for c in 0..xs.len() - 1 {
            // Test the column's *centre*, not containment of the whole column:
            // `xs` comes from clustered and gutter-collapsed boundaries, so the
            // outer ones can sit tens of points off the drawn border, and cell
            // rules are inset by their padding. The centre is the one point
            // that is unambiguously inside this cell.
            let cx = (xs[c] + xs[c + 1]) * 0.5;
            if !at_y.iter().any(|h| h.x_min <= cx && h.x_max >= cx) {
                mask[row][c] = true;
            }
        }
    }
    mask
}

fn passes_density_gate(
    cells: &[Vec<Cell>],
    cell_has_text: &[Vec<bool>],
    cell_is_bold: &[Vec<bool>],
    n_rows: usize,
    n_cols: usize,
    flattened: bool,
    spanned: &[Vec<bool>],
    dbg: bool,
) -> bool {
    let total = n_rows * n_cols;
    // A spanned cell only excuses itself when the cell it is merged into
    // actually holds text. Walk up the run of merged cells to its head: an
    // empty head means nothing was merged here, just an unruled hole - the
    // shape a chart's vertical gridlines make, where forgiving the empties
    // would turn a plot into a 27-column table.
    let merged_into_text = |r: usize, c: usize| {
        let mut i = r;
        while i > 0 && spanned[i][c] {
            i -= 1;
        }
        cell_has_text[i][c]
    };
    let empty_count = (0..n_rows)
        .flat_map(|r| (0..n_cols).map(move |c| (r, c)))
        .filter(|&(r, c)| !cell_has_text[r][c] && !(spanned[r][c] && merged_into_text(r, c)))
        .count();
    let empty_frac = (empty_count as f32) / (total as f32);
    if empty_frac <= TABLE_MAX_EMPTY_CELL_FRACTION {
        return true;
    }
    let col0_fill = (0..n_rows).filter(|r| cell_has_text[*r][0]).count() as f32 / n_rows as f32;
    let col0_max_chars = (0..n_rows)
        .filter(|r| cell_has_text[*r][0])
        .map(|r| cells[r][0].text.len())
        .max()
        .unwrap_or(0);
    let col0_spine =
        col0_fill >= TABLE_SPINE_FILL_FRACTION && col0_max_chars <= TABLE_SPINE_MAX_CELL_CHARS;
    // Header may span multiple visual rows (the grid detector slices on each
    // text baseline). Treat the first ≤4 rows as the header band and require
    // their *union* to cover most columns AND be all-bold.
    let header_band = n_rows.min(4);
    let mut header_cols_covered = vec![false; n_cols];
    let mut header_all_bold = true;
    for r in 0..header_band {
        for c in 0..n_cols {
            if cell_has_text[r][c] {
                header_cols_covered[c] = true;
                if !cell_is_bold[r][c] {
                    header_all_bold = false;
                }
            }
        }
    }
    let header_coverage = header_cols_covered.iter().filter(|t| **t).count();
    let dense_inner_col = (1..n_cols).any(|c| {
        let col_fill = (0..n_rows).filter(|r| cell_has_text[*r][c]).count() as f32 / n_rows as f32;
        col_fill >= TABLE_SPINE_FILL_FRACTION
    });
    // Header coverage doesn't need to span every column — wide-cell legal
    // tables often spread the header across many visual baselines and only a
    // few columns land in the top-4-rows band. Require ≥3 columns covered as
    // evidence of a real header, not just a title.
    let long_prose_table =
        n_rows >= 5 && n_cols >= 3 && header_coverage >= 3 && header_all_bold && dense_inner_col;
    if !col0_spine && !long_prose_table && !flattened {
        if dbg {
            let fills: Vec<usize> = (0..n_rows)
                .map(|r| cell_has_text[r].iter().filter(|t| **t).count())
                .collect();
            eprintln!(
                "[ruled]   REJECT empty-frac {empty_frac:.2} ({n_rows}x{n_cols}, no spine/long-prose) row_fills={fills:?}"
            );
        }
        return false;
    }
    if empty_frac > TABLE_MAX_EMPTY_CELL_FRACTION_WITH_SPINE && !long_prose_table {
        if dbg {
            eprintln!("[ruled]   REJECT empty-frac-with-spine {empty_frac:.2}");
        }
        return false;
    }
    true
}

/// Build a `TableRun` for one ruled-grid component. Returns `None` if the
/// resulting grid is too small (< 2 cols or < 2 rows), covers nearly the
/// whole page (likely the page border), or is mostly empty cells.
/// Fuse "gutter" column boundaries left behind by paired cell-border edges.
///
/// A bordered cell contributes two vertical edges - its own right edge and the
/// next cell's left edge - separated by the table's inter-cell padding.
/// `TABLE_COL_BOUNDARY_CLUSTER_PT` fuses the tight pairs (4-6pt), but a
/// generously padded table puts 15-25pt between them, leaving a sliver column
/// between every real column. Those slivers double the column count, and
/// because `split_span_at_anchors` shreds text into them they are *not* empty,
/// so `collapse_phantom_cols` can't remove them - the grid then fails the
/// empty-cell gate and a perfectly good table is thrown away.
///
/// Width alone can't identify them - plenty of real tables carry a genuinely
/// narrow column (a `#` or `No.` column beside a wide description). What marks
/// a sliver is that **no text center ever lands inside it**: text steps over a
/// gutter, but a narrow real column still holds its own values. So fuse a
/// column only when it holds no span center *and* is far narrower than the
/// columns that do hold text. Span centers are read straight off the raw spans,
/// before `split_span_at_anchors` runs, so shredded fragments can't disguise a
/// sliver as occupied.
///
/// The width guard matters independently: a worksheet's blank answer column is
/// also centerless, but it is as wide as its neighbours and must survive.
fn collapse_gutter_columns(xs: &mut Vec<f32>, lines: &[ProjectedLine], dbg: bool) {
    /// A sliver must be at most this fraction of the median *content-bearing*
    /// column width. Blank-but-full-width cells (worksheets, forms) sit well
    /// above it and are preserved.
    const GUTTER_MAX_WIDTH_FRAC: f32 = 0.5;

    if xs.len() < 4 {
        return; // fewer than 3 columns: nothing to fuse without destroying the table
    }
    let centers: Vec<f32> = lines
        .iter()
        .flat_map(|l| l.spans.iter())
        .filter(|s| !s.text.trim().is_empty())
        .map(|s| s.x + s.width * 0.5)
        .collect();
    let has_center = |lo: f32, hi: f32| centers.iter().any(|&c| c >= lo && c < hi);

    let before = xs.len();
    while xs.len() > 3 {
        // Median width of the columns that actually hold text.
        let mut occupied: Vec<f32> = xs
            .windows(2)
            .filter(|w| has_center(w[0], w[1]))
            .map(|w| w[1] - w[0])
            .collect();
        if occupied.is_empty() {
            break;
        }
        occupied.sort_by(|a, b| a.total_cmp(b));
        let median = occupied[occupied.len() / 2];
        let max_sliver = median * GUTTER_MAX_WIDTH_FRAC;

        // Narrowest centerless column, if any is slim enough to be a gutter.
        let victim = xs
            .windows(2)
            .enumerate()
            .filter(|(_, w)| !has_center(w[0], w[1]))
            .map(|(i, w)| (i, w[1] - w[0]))
            .filter(|&(_, width)| width < max_sliver)
            .min_by(|a, b| a.1.total_cmp(&b.1));
        let Some((i, _)) = victim else { break };
        xs[i] = (xs[i] + xs[i + 1]) * 0.5;
        xs.remove(i + 1);
    }
    if dbg && xs.len() != before {
        eprintln!(
            "[ruled]   gutter-collapse {before} -> {} boundaries",
            xs.len()
        );
    }
}

/// Drop vertical rules too short to be column boundaries (see
/// `RULED_VLINE_MIN_COVERAGE`). Coverage is measured against the row extent the
/// horizontals describe, since that is the height a real divider has to reach.
fn filter_short_vlines(
    hs: &[HSeg],
    h_indices: &[usize],
    vs: &[VSeg],
    v_indices: &[usize],
) -> Vec<usize> {
    let y_lo = h_indices.iter().map(|&i| hs[i].y).fold(f32::MAX, f32::min);
    let y_hi = h_indices.iter().map(|&i| hs[i].y).fold(f32::MIN, f32::max);
    let extent = (y_hi - y_lo).max(1.0);
    let kept: Vec<usize> = v_indices
        .iter()
        .copied()
        .filter(|&i| (vs[i].y_max - vs[i].y_min) / extent >= RULED_VLINE_MIN_COVERAGE)
        .collect();
    // Two boundaries is one column, which is not a grid; below that the caller
    // has nothing to refine and keeps the raw set.
    if kept.len() >= 3 {
        kept
    } else {
        v_indices.to_vec()
    }
}

/// Widen a boundary list to the extent the *perpendicular* rules imply.
///
/// A table drawn with interior dividers but no outer frame gives `xs` that stop
/// at the first and last vertical rule, so its outermost columns are missing
/// entirely: every line in them reads as overhang and the component collapses.
/// The evidence is already on the page: the horizontal rules know how wide the
/// table is, and the verticals know how tall.
///
/// Three guards, all load-bearing:
///   - **median, not min/max** of the perpendicular rules, so one overshooting
///     stroke cannot widen the grid;
///   - **the new outer band must hold text**, the same idiom
///     `collapse_gutter_columns` uses; otherwise a pen-cap overshoot or a
///     full-width section rule manufactures an empty outer column;
///   - **the band must be at least `min_band` wide/tall.** Callers pass the
///     smallest existing row height for the row axis: verticals routinely
///     overrun the last horizontal by a few points, and a band shorter than any
///     real row is that overshoot, not a row. A phantom row here is enough for
///     the ruled grid to outrank a better borderless table. The column axis
///     passes only the clustering tolerance, because an unruled *label* column
///     is both common and legitimately much narrower than the ruled data
///     columns it sits beside.
fn extend_to_perpendicular_extent(
    axis: &mut Vec<f32>,
    perp_los: &[f32],
    perp_his: &[f32],
    min_band: f32,
    has_content: impl Fn(f32, f32) -> bool,
) {
    let min_extension = min_band.max(TABLE_COL_BOUNDARY_CLUSTER_PT);
    let median = |v: &[f32]| {
        let mut s = v.to_vec();
        s.sort_by(f32::total_cmp);
        s.get(s.len() / 2).copied()
    };
    let (Some(lo), Some(hi)) = (median(perp_los), median(perp_his)) else {
        return;
    };
    if axis.is_empty() {
        return;
    }
    let dbg = *super::flags::DEBUG_RULED;
    if lo < axis[0] - min_extension && has_content(lo, axis[0]) {
        if dbg {
            eprintln!("[ruled]   extend lo {:.1} -> {:.1}", axis[0], lo);
        }
        axis.insert(0, lo);
    }
    let last = axis[axis.len() - 1];
    if hi > last + min_extension && has_content(last, hi) {
        if dbg {
            eprintln!("[ruled]   extend hi {last:.1} -> {hi:.1}");
        }
        axis.push(hi);
    }
}

/// Build a ruled table from one grid component.
///
/// Runs `build_ruled_table_from` twice when short vertical rules are present:
/// once on the rules as drawn, once with the stubs filtered out. The unfiltered
/// build is the gatekeeper: **the filtered result is only ever allowed to
/// replace a table that would have been produced anyway**. Filtering first
/// instead lets the filter *remove the evidence the gates reject junk on*: a
/// chart loses the very rules that made it too sparse or too small to pass,
/// and a rejected component flips into an accepted junk table.
fn build_ruled_table(
    hs: &[HSeg],
    vs: &[VSeg],
    h_indices: &[usize],
    v_indices: &[usize],
    lines: &[ProjectedLine],
    page_width: f32,
    page_height: f32,
    pass: RuledPass,
) -> Option<(TableRun, Vec<usize>)> {
    let base = build_ruled_table_from(
        hs,
        vs,
        h_indices,
        v_indices,
        lines,
        page_width,
        page_height,
        pass,
    )?;
    let v_kept = filter_short_vlines(hs, h_indices, vs, v_indices);
    let strip = |(run, consumed, _straddle)| (run, consumed);
    if v_kept.len() == v_indices.len() {
        return Some(strip(base));
    }
    let dbg = *super::flags::DEBUG_RULED;
    let retry = build_ruled_table_from(
        hs,
        vs,
        h_indices,
        &v_kept,
        lines,
        page_width,
        page_height,
        pass,
    );
    // Take the refined grid only when dropping the stubs measurably stopped the
    // boundaries cutting through text. A stub that sits on a *real* column edge
    // is the common case in dense financial tables; there the straddle census
    // is unchanged, and dropping a rule that was the only evidence for its
    // boundary just merges a real column away.
    match retry {
        Some(r) if r.2 < base.2 - STRADDLE_IMPROVEMENT => {
            if dbg {
                eprintln!(
                    "[ruled]   vline-coverage retry {} -> {} rules, straddle {:.2} -> {:.2}",
                    v_indices.len(),
                    v_kept.len(),
                    base.2,
                    r.2
                );
            }
            Some(strip(r))
        }
        _ => Some(strip(base)),
    }
}

/// How much the straddle fraction must fall for the short-vline refinement to
/// be worth taking. Removing a genuine phantom column drops it by an order of
/// magnitude; a stub on a real column edge leaves it flat.
const STRADDLE_IMPROVEMENT: f32 = 0.05;

#[allow(clippy::too_many_arguments)]
fn build_ruled_table_from(
    hs: &[HSeg],
    vs: &[VSeg],
    h_indices: &[usize],
    v_kept: &[usize],
    lines: &[ProjectedLine],
    page_width: f32,
    page_height: f32,
    pass: RuledPass,
) -> Option<(TableRun, Vec<usize>, f32)> {
    let dbg = *super::flags::DEBUG_RULED;
    let mut xs: Vec<f32> = v_kept.iter().map(|&i| vs[i].x).collect();
    xs.sort_by(|a, b| a.total_cmp(b));
    let spans = || lines.iter().flat_map(|l| l.spans.iter());
    extend_to_perpendicular_extent(
        &mut xs,
        &h_indices.iter().map(|&i| hs[i].x_min).collect::<Vec<_>>(),
        &h_indices.iter().map(|&i| hs[i].x_max).collect::<Vec<_>>(),
        0.0,
        |lo, hi| {
            spans().filter(|s| !s.text.trim().is_empty()).any(|s| {
                let c = s.x + s.width * 0.5;
                c >= lo && c < hi
            })
        },
    );
    // Coarser, mean-centered clustering for column boundaries: cell-border
    // rects contribute paired edges 4-6pt apart that would otherwise become
    // phantom 5pt "columns" the span splitter then shreds text into.
    cluster_boundaries(&mut xs, TABLE_COL_BOUNDARY_CLUSTER_PT);
    collapse_gutter_columns(&mut xs, lines, dbg);

    // Distinct row y-coords (cluster again — multiple H lines may share a y).
    // In the global pass, first drop horizontal rules that span only a small
    // fraction of the column extent (see `RULED_HLINE_MIN_COVERAGE`).
    let raw_ys = |idxs: &[usize]| {
        let mut v: Vec<f32> = idxs.iter().map(|&i| hs[i].y).collect();
        v.sort_by(|a, b| a.total_cmp(b));
        dedup_close(&mut v, TABLE_GRID_CLUSTER_PT);
        v
    };
    let ys: Vec<f32> = if pass == RuledPass::Global && xs.len() >= 2 {
        let col_lo = xs[0];
        let col_hi = xs[xs.len() - 1];
        let extent = (col_hi - col_lo).max(1.0);
        let kept: Vec<usize> = h_indices
            .iter()
            .copied()
            .filter(|&i| {
                let h = &hs[i];
                let ov = (h.x_max.min(col_hi) - h.x_min.max(col_lo)).max(0.0);
                ov / extent >= RULED_HLINE_MIN_COVERAGE
            })
            .collect();
        let filtered = raw_ys(&kept);
        if filtered.len() >= 3 {
            filtered
        } else {
            raw_ys(h_indices)
        }
    } else {
        raw_ys(h_indices)
    };
    let mut ys = ys;
    let min_row_band = ys.windows(2).map(|w| w[1] - w[0]).fold(f32::MAX, f32::min);
    extend_to_perpendicular_extent(
        &mut ys,
        &v_kept.iter().map(|&i| vs[i].y_min).collect::<Vec<_>>(),
        &v_kept.iter().map(|&i| vs[i].y_max).collect::<Vec<_>>(),
        min_row_band,
        |lo, hi| {
            spans().filter(|s| !s.text.trim().is_empty()).any(|s| {
                let c = s.y + s.height * 0.5;
                c >= lo && c < hi
            })
        },
    );
    dedup_close(&mut ys, TABLE_GRID_CLUSTER_PT);
    let ys = ys;
    if dbg {
        eprintln!(
            "[ruled] component: ys={:?} xs={:?} ({} lines in scope)",
            ys,
            xs,
            lines.len()
        );
    }

    // Need ≥2 row boundaries (1 row) and ≥2 column boundaries (1 col); but
    // a 1×1 grid is just a callout box, so also require ≥1 inner divider
    // (i.e. ys.len() ≥ 3 for ≥2 rows). Single-column tables (`xs.len() == 2`)
    // are accepted when row evidence is strong enough — extra guards apply
    // below after the empty-row collapse.
    if ys.len() < 3 || xs.len() < 2 {
        if dbg {
            eprintln!(
                "[ruled]   REJECT grid-too-small ys={} xs={}",
                ys.len(),
                xs.len()
            );
        }
        return None;
    }

    let n_rows = ys.len() - 1;
    let n_cols = xs.len() - 1;
    let bbox = crate::types::Rect {
        x: xs[0],
        y: ys[0],
        width: xs[n_cols] - xs[0],
        height: ys[n_rows] - ys[0],
    };

    // Reject page-border-as-table.
    if page_width > 0.0 && page_height > 0.0 {
        let coverage = (bbox.width / page_width) * (bbox.height / page_height);
        if coverage > TABLE_MAX_PAGE_COVERAGE {
            if dbg {
                eprintln!("[ruled]   REJECT page-coverage {coverage:.2}");
            }
            return None;
        }
    }

    // Assign text to cells, then reject if no lines landed inside the grid or
    // too many spans straddle interior column boundaries (decorative box, not
    // a table).
    let (mut grid, consumed_indices) = assign_cells(lines, &xs, &ys, dbg)?;
    let straddle_frac = grid.straddle_frac;
    grid.spanned = rowspan_mask(hs, h_indices, vs, v_kept, &xs, &ys);

    // Collapse phantom rows (text-less thin border-strip rects) then phantom
    // columns (text-less narrow border strips). A real table with one phantom
    // border-strip column drops exactly one; a chart whose vertical grid-lines
    // merged with text data collapses to 1 col and is rejected below, letting
    // the borderless detector handle it.
    let kept_row_heights = grid.collapse_phantom_rows(&ys);
    let n_rows = grid.n_rows();
    if n_rows < 2 {
        if dbg {
            eprintln!("[ruled]   REJECT rows-after-collapse {n_rows}");
        }
        return None;
    }
    grid.collapse_phantom_cols(&xs);
    let n_cols = grid.n_cols();
    if n_cols == 0 {
        return None;
    }
    // Single-column tables are ambiguous (could be a captioned card) — require
    // ≥3 rows of geometric + textual evidence.
    if n_cols == 1 && n_rows < 3 {
        return None;
    }

    // Hand the collapsed grids back to plain locals for the header-detection
    // stages below.
    let CellGrid {
        text: cells,
        is_bold: cell_is_bold,
        has_text: cell_has_text,
        repl: cells_repl,
        repl_indices: cells_repl_indices,
        row_alpha_spanner,
        spanned,
        straddle_frac: _,
    } = grid;

    let flattened_header = flatten_header_band(
        &cells,
        &cell_has_text,
        &cells_repl,
        &cells_repl_indices,
        &row_alpha_spanner,
        n_rows,
        n_cols,
        dbg,
    );

    let (cells, cell_has_text, cell_is_bold, n_rows, merged_stacked_header) = merge_stacked_header(
        cells,
        cell_has_text,
        cell_is_bold,
        n_rows,
        n_cols,
        &kept_row_heights,
        flattened_header.is_some(),
        dbg,
    );

    // `merge_stacked_header` folds the top `k` rows into one, so realign the
    // rowspan mask: the new first row is the merged header (never a rowspan
    // continuation), and every later row keeps its own mask.
    let spanned = if spanned.len() > n_rows {
        let dropped = spanned.len() - n_rows;
        let mut m = vec![vec![false; n_cols]; 1];
        m.extend(spanned[dropped + 1..].iter().cloned());
        m
    } else {
        spanned
    };

    if !passes_density_gate(
        &cells,
        &cell_has_text,
        &cell_is_bold,
        n_rows,
        n_cols,
        flattened_header.is_some(),
        &spanned,
        dbg,
    ) {
        return None;
    }

    // Header preference order: flattened colspan band (carries the full
    // per-column layer chain) > merged stacked-header band > bold first row.
    let header_qualifies = merged_stacked_header
        || (cell_has_text[0]
            .iter()
            .zip(cell_is_bold[0].iter())
            .all(|(has, bold)| !has || *bold)
            && cell_has_text[0].iter().any(|has| *has));
    let (header, body_start) = match flattened_header {
        Some((h, bs)) => (Some(h), bs),
        None if header_qualifies => (Some(cells[0].clone()), 1),
        None => (None, 0),
    };
    let body_rows: Vec<Vec<Cell>> = cells[body_start..].to_vec();
    if body_rows.is_empty() {
        return None;
    }

    // Line index span this table covers.
    let start = *consumed_indices.iter().min().unwrap();
    let end = *consumed_indices.iter().max().unwrap() + 1;

    Some((
        TableRun {
            start,
            end,
            body_start: start,
            // A ruled table's region is the grid the generator actually drew.
            bbox: Some(bbox),
            block: Block::Table {
                header,
                rows: body_rows,
            },
            block_indices: Vec::new(),
        },
        consumed_indices,
        straddle_frac,
    ))
}

/// Single-link cluster a sorted Vec of boundary coordinates, replacing each
/// chain of entries (adjacent gap ≤ `tol`) with its mean. Unlike
/// `dedup_close` (keep-first), the mean centers the boundary between the
/// paired edges that cell-border rects produce (left border strip + right
/// border strip of adjacent cells, typically 4-6pt apart), so the split
/// target lands between cells rather than inside one.
fn cluster_boundaries(v: &mut Vec<f32>, tol: f32) {
    if v.len() < 2 {
        return;
    }
    let mut out: Vec<f32> = Vec::with_capacity(v.len());
    let mut cluster_sum = v[0];
    let mut cluster_n = 1usize;
    let mut last = v[0];
    for &x in v.iter().skip(1) {
        if x - last <= tol {
            cluster_sum += x;
            cluster_n += 1;
        } else {
            out.push(cluster_sum / cluster_n as f32);
            cluster_sum = x;
            cluster_n = 1;
        }
        last = x;
    }
    out.push(cluster_sum / cluster_n as f32);
    *v = out;
}

/// In-place dedup of a sorted Vec, collapsing entries within `tol` to the
/// first of each cluster.
fn dedup_close(v: &mut Vec<f32>, tol: f32) {
    if v.len() < 2 {
        return;
    }
    let mut out: Vec<f32> = Vec::with_capacity(v.len());
    for x in v.iter().copied() {
        if let Some(&last) = out.last()
            && (x - last).abs() <= tol
        {
            continue;
        }
        out.push(x);
    }
    *v = out;
}

/// Find the bucket index `i` such that `boundaries[i] <= val < boundaries[i+1]`.
/// Returns `None` if `val` is outside the boundaries.
/// Distribute a multi-column run's words into ruled columns by each word's own
/// centre, returning `(column, text)` pairs in column order.
///
/// This replaces guessing a split point from a character index: with word boxes
/// the answer is simply which cell each word sits in. Returns `None` when the
/// run has no verified word geometry, or when every word lands in one column -
/// in that case the caller's whole-span fallbacks are the honest answer, since
/// the run only *overlaps* the neighbouring cell (padding, an overhanging
/// descender) rather than genuinely spanning it.
fn bucket_words_into_columns(span: &TextItem, xs: &[f32]) -> Option<Vec<(usize, String)>> {
    let words = verified_words(span)?;
    let mut out: Vec<(usize, String)> = Vec::new();
    for w in words {
        let cx = (w.x + w.width.max(0.0) * 0.5).clamp(xs[0], *xs.last()?);
        let col = find_bucket(xs, cx)?;
        match out.last_mut() {
            Some((c, text)) if *c == col => {
                text.push(' ');
                text.push_str(w.text.trim());
            }
            _ => out.push((col, w.text.trim().to_string())),
        }
    }
    // Words must not zig-zag back into an earlier column, and the run must
    // really occupy more than one.
    if out.len() < 2 || out.windows(2).any(|p| p[1].0 <= p[0].0) {
        return None;
    }
    Some(out)
}

fn find_bucket(boundaries: &[f32], val: f32) -> Option<usize> {
    if boundaries.len() < 2 || val < boundaries[0] || val > *boundaries.last().unwrap() {
        return None;
    }
    for (i, w) in boundaries.windows(2).enumerate() {
        if val >= w[0] && val <= w[1] {
            return Some(i);
        }
    }
    None
}

/// Detect candidate ruled-table bounding rectangles from page graphics alone.
///
/// Unlike `detect_ruled_tables`, this runs *before* projection and ignores text
/// content entirely — its only job is to find the bbox of every H/V grid
/// component so the XY-cut layout pass can treat those regions as obstacles
/// and avoid slicing tables column-wise (the failure mode that produces
/// column-major reading order). Empty-cell-fraction
/// and other quality filters are deliberately skipped here: we want the bbox
/// even of sparse forms or partially-filled grids, because the obstacle
/// machinery only cares about geometry.
pub fn detect_table_rects(
    graphics: &[GraphicPrimitive],
    page_width: f32,
    page_height: f32,
) -> Vec<Rect> {
    let (hs, vs) = extract_h_v_segments(graphics);
    let hs = cluster_h_segments(hs);
    let vs = cluster_v_segments(vs);
    if hs.len() < 2 || vs.len() < 2 {
        return Vec::new();
    }
    let components = find_grid_components(&hs, &vs);
    let mut out = Vec::new();
    for (h_idx, v_idx) in components {
        let ys: Vec<f32> = h_idx.iter().map(|&i| hs[i].y).collect();
        let xs: Vec<f32> = v_idx.iter().map(|&i| vs[i].x).collect();
        let y_min = ys.iter().copied().fold(f32::INFINITY, f32::min);
        let y_max = ys.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let x_min = xs.iter().copied().fold(f32::INFINITY, f32::min);
        let x_max = xs.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let w = x_max - x_min;
        let h = y_max - y_min;
        if w < 5.0 || h < 5.0 {
            continue;
        }
        // Skip whole-page borders — same rationale as `TABLE_MAX_PAGE_COVERAGE`
        // in the post-projection detector.
        if page_width > 0.0
            && page_height > 0.0
            && w / page_width >= TABLE_MAX_PAGE_COVERAGE
            && h / page_height >= TABLE_MAX_PAGE_COVERAGE
        {
            continue;
        }
        out.push(Rect {
            x: x_min,
            y: y_min,
            width: w,
            height: h,
        });
    }
    out
}

/// One band of a grid component after splitting, with the vertical rules
/// clipped to the band's own y-range.
struct GridBand {
    h_idx: Vec<usize>,
    v_idx: Vec<usize>,
    /// A copy of the page's `vs` whose y-extents are clamped to this band, so
    /// the shared spine rule cannot carry the neighbouring table's height into
    /// `filter_short_vlines` or `extend_to_perpendicular_extent`.
    vs: Vec<VSeg>,
}

/// Split one grid component at row bands that no vertical rule actually spans.
///
/// `cluster_v_segments` merges same-x verticals by taking the *union* of their
/// y-ranges with no gap check, so two tables stacked in one column, each
/// drawing its own short strokes at the same left edge, fuse into a single
/// component. Each table is then evaluated with all of the other's rows empty
/// and dies on `TABLE_MAX_EMPTY_CELL_FRACTION`, and when the two tables have
/// different layouts their column sets are unioned too, over-segmenting both.
///
/// The cut signal is geometric and needs no threshold on emptiness: a band
/// between consecutive horizontals that **no raw, pre-cluster `VSeg` spans**.
/// That is strictly stronger than "a big gap", and it distinguishes this from a
/// rowspan, where the vertical does continue through the boundary.
fn split_component_at_grid_gaps(
    hs: &[HSeg],
    vs: &[VSeg],
    raw_vs: &[VSeg],
    h_idx: &[usize],
    v_idx: &[usize],
) -> Vec<GridBand> {
    /// Row pitch below this means the component is decoration, not a table -
    /// vector-drawn maths glyphs make components with a 3-4pt pitch.
    const MIN_PITCH_PT: f32 = 8.0;
    /// A gap must dwarf the component's own row pitch *and* clear an absolute
    /// floor, so ordinary row-height variation never cuts.
    const GAP_PITCH_MULT: f32 = 2.5;
    const GAP_MIN_PT: f32 = 30.0;
    /// Each side must keep a real table: 3 boundaries is 2 rows, the minimum
    /// `build_ruled_table` will look at.
    const MIN_BAND_HLINES: usize = 3;

    let whole = || {
        vec![GridBand {
            h_idx: h_idx.to_vec(),
            v_idx: v_idx.to_vec(),
            vs: vs.to_vec(),
        }]
    };
    let mut sorted = h_idx.to_vec();
    sorted.sort_by(|&a, &b| hs[a].y.total_cmp(&hs[b].y));
    if sorted.len() < MIN_BAND_HLINES * 2 {
        return whole();
    }
    let mut pitches: Vec<f32> = sorted.windows(2).map(|w| hs[w[1]].y - hs[w[0]].y).collect();
    pitches.sort_by(f32::total_cmp);
    let pitch = pitches[pitches.len() / 2];
    if pitch < MIN_PITCH_PT {
        return whole();
    }
    let min_gap = (pitch * GAP_PITCH_MULT).max(GAP_MIN_PT);

    let tol = TABLE_CROSS_TOLERANCE_PT;
    let mut cuts: Vec<usize> = Vec::new(); // index into `sorted`: cut after this line
    for (i, w) in sorted.windows(2).enumerate() {
        let (lo, hi) = (hs[w[0]].y, hs[w[1]].y);
        if hi - lo < min_gap {
            continue;
        }
        let spanned = raw_vs
            .iter()
            .any(|v| v.y_min <= lo + tol && v.y_max >= hi - tol);
        if !spanned {
            cuts.push(i + 1);
        }
    }
    if cuts.is_empty() {
        return whole();
    }

    let mut bounds = vec![0usize];
    bounds.extend(cuts);
    bounds.push(sorted.len());
    if bounds.windows(2).any(|b| b[1] - b[0] < MIN_BAND_HLINES) {
        return whole(); // a sliver on one side: the cut is not describing two tables
    }

    bounds
        .windows(2)
        .map(|b| {
            let band = &sorted[b[0]..b[1]];
            let y_lo = hs[band[0]].y;
            let y_hi = hs[band[band.len() - 1]].y;
            let v_idx: Vec<usize> = v_idx
                .iter()
                .copied()
                .filter(|&i| vs[i].y_max > y_lo + tol && vs[i].y_min < y_hi - tol)
                .collect();
            let vs: Vec<VSeg> = vs
                .iter()
                .map(|v| VSeg {
                    x: v.x,
                    y_min: v.y_min.max(y_lo),
                    y_max: v.y_max.min(y_hi),
                })
                .collect();
            GridBand {
                h_idx: band.to_vec(),
                v_idx,
                vs,
            }
        })
        .collect()
}

/// Detect ruled-grid tables on a page from its vector graphics. Returns runs
/// in document order (sorted by `start`).
fn detect_ruled_tables_impl(
    lines: &[ProjectedLine],
    segs: &RuleSegments,
    page_width: f32,
    page_height: f32,
    pass: RuledPass,
) -> Vec<(TableRun, Vec<usize>)> {
    let RuleSegments { hs, raw_vs, vs } = segs;
    if hs.len() < 2 || vs.len() < 2 {
        return Vec::new();
    }
    let components = find_grid_components(hs, vs);
    let mut out = Vec::new();
    for (h_idx, v_idx) in components {
        for band in split_component_at_grid_gaps(hs, vs, raw_vs, &h_idx, &v_idx) {
            if let Some(run) = build_ruled_table(
                hs,
                &band.vs,
                &band.h_idx,
                &band.v_idx,
                lines,
                page_width,
                page_height,
                pass,
            ) {
                out.push(run);
            }
        }
    }
    out.sort_by_key(|(r, _)| r.start);
    out
}

/// Per-region ruled-table detection. Drops the consumed-line bookkeeping the
/// global pass needs.
pub(super) fn detect_ruled_tables(
    lines: &[ProjectedLine],
    segs: &RuleSegments,
    page_width: f32,
    page_height: f32,
) -> Vec<TableRun> {
    detect_ruled_tables_impl(lines, segs, page_width, page_height, RuledPass::PerRegion)
        .into_iter()
        .map(|(r, _)| r)
        .collect()
}

/// Page-level ruled-table detection over *all* lines. A table whose rows
/// scatter across several xy_cut leaves never has its full text in any single
/// leaf, so per-region detection rejects it as mostly-empty; running once
/// globally reassembles it. Returns each run with the set of line indices it
/// consumed so the caller can pull them out of the region pipeline.
pub(super) fn detect_ruled_tables_global(
    lines: &[ProjectedLine],
    segs: &RuleSegments,
    page_width: f32,
    page_height: f32,
) -> Vec<(TableRun, Vec<usize>)> {
    detect_ruled_tables_impl(lines, segs, page_width, page_height, RuledPass::Global)
}

/// Count filled (non-empty) cells in a TableRun. GridFallback returns 0 so
/// it never beats a real Table in density comparisons.
fn run_filled_cells(run: &TableRun) -> usize {
    match &run.block {
        Block::Table { header, rows } => {
            let header_filled = header
                .as_ref()
                .map(|h| h.iter().filter(|c| !c.text.trim().is_empty()).count())
                .unwrap_or(0);
            let body_filled: usize = rows
                .iter()
                .flat_map(|r| r.iter())
                .filter(|c| !c.text.trim().is_empty())
                .count();
            header_filled + body_filled
        }
        _ => 0,
    }
}

/// Merge ruled-grid runs with borderless runs into a single sorted list. When
/// ranges overlap the ruled run normally wins (path-based geometry is a
/// stronger signal than text-alignment heuristics), with two exceptions:
///   1. A single-column ruled run yields to a multi-column borderless run
///      covering the same range (vertical separators may be implicit).
///   2. A sparse ruled run yields to a denser borderless run — decorative
///      vector boxes around titles / callout banners produce ruled "tables"
///      with few filled cells; when a borderless detector finds a much denser
///      real table in the same region, prefer it.
pub(super) fn merge_table_runs(
    mut ruled: Vec<TableRun>,
    borderless: Vec<TableRun>,
) -> Vec<TableRun> {
    let mut kept: Vec<TableRun> = Vec::with_capacity(ruled.len());
    for r in ruled.drain(..) {
        let is_one_col = matches!(&r.block, Block::Table { rows, .. } if rows.first().map(|row| row.len()).unwrap_or(0) <= 1);
        if is_one_col {
            let beaten = borderless.iter().any(|b| {
                let overlaps = !(b.end <= r.start || b.start >= r.end);
                if !overlaps {
                    return false;
                }
                matches!(&b.block, Block::Table { rows, .. } if rows.first().map(|row| row.len()).unwrap_or(0) >= 2)
            });
            if beaten {
                continue;
            }
        }
        // Density check: if a borderless run overlaps and carries
        // substantially more filled cells, the ruled run is most likely
        // a decorative grid (page chrome, title banner) wrapping the real
        // table the borderless detector already found.
        let ruled_density = run_filled_cells(&r);
        let beaten_by_density = borderless.iter().any(|b| {
            let overlaps = !(b.end <= r.start || b.start >= r.end);
            if !overlaps {
                return false;
            }
            run_filled_cells(b) >= ruled_density * 2 + 4
        });
        if beaten_by_density {
            continue;
        }
        kept.push(r);
    }
    for b in borderless {
        let overlaps = kept.iter().any(|r| !(b.end <= r.start || b.start >= r.end));
        if !overlaps {
            kept.push(b);
        }
    }
    kept.sort_by_key(|r| r.start);
    for run in &mut kept {
        if let Block::Table { rows, .. } = &mut run.block {
            merge_continuation_rows(rows);
        }
    }
    kept
}

/// Fold soft-wrapped cell continuations back into the row they belong to.
///
/// A table cell holding a sentence wraps across several projected lines, and
/// every one of those lines becomes its own grid row:
///
/// ```text
/// | Knowledge |  | ● To understand the meaning of reducing, reusing and recycling |
/// |           |  | and how they connect ● To understand the importance of the 3 Rs |
/// ```
///
/// As geometry this is unsolvable: the gap between a wrapped line and a genuine
/// next row is the same gap. But once the grid exists it is tractable from the
/// text alone.
///
/// A row is a continuation of its predecessor when all of these hold:
///
/// - it has the same width and an **empty first cell** (the row label lives on
///   the first line of the row and is never repeated);
/// - its filled columns are a **subset** of the predecessor's filled columns:
///   a continuation can only extend cells that already have text;
/// - it carries **no value-like cell**; numbers don't soft-wrap, so a
///   blank-label row holding one is a real data row (typically a `Total`);
/// - at least one extended cell reads as a soft wrap rather than a fresh
///   sentence after a completed one.
fn merge_continuation_rows(rows: &mut Vec<Vec<Cell>>) {
    if rows.len() < 2 {
        return;
    }
    // The whole rule keys off "empty first cell", which only means "wrapped
    // line" when the first column is a *label* column. Plenty of tables just
    // have a sparse first column: a timetable whose `Notes` column is blank on
    // every run, a size chart, a hierarchical header. There, every row looks
    // like a continuation and the table collapses into a single row. Requiring
    // the first column to be populated in at least half the rows separates the
    // two.
    let first_col_filled = rows
        .iter()
        .filter(|r| r.first().is_some_and(|c| !c.text.trim().is_empty()))
        .count();
    if first_col_filled * 2 < rows.len() {
        return;
    }
    let mut out: Vec<Vec<Cell>> = Vec::with_capacity(rows.len());
    for row in rows.drain(..) {
        if let Some(prev) = out.last_mut() {
            if is_continuation_row(prev, &row) {
                for (i, cell) in row.iter().enumerate() {
                    let t = cell.text.trim();
                    if t.is_empty() {
                        continue;
                    }
                    // `prev[i]` is non-empty for every filled column of `row`;
                    // that is the subset rule `is_continuation_row` enforces.
                    prev[i].text.push(' ');
                    prev[i].text.push_str(t);
                    // The wrapped line is part of the same logical cell, so it
                    // grows that cell's box down to cover the continuation —
                    // and extends its provenance the same way.
                    if let Some(r) = &cell.bbox {
                        Rect::extend(&mut prev[i].bbox, r);
                    }
                    prev[i].add_indices(cell.text_item_indices.iter().copied());
                }
                continue;
            }
        }
        out.push(row);
    }
    *rows = out;
}

fn is_continuation_row(prev: &[Cell], cur: &[Cell]) -> bool {
    if cur.len() != prev.len() || cur.len() < 2 {
        return false;
    }
    // The row label is never repeated on a wrapped line.
    if !cur[0].text.trim().is_empty() || prev[0].text.trim().is_empty() {
        return false;
    }
    let filled: Vec<usize> = (0..cur.len())
        .filter(|&i| !cur[i].text.trim().is_empty())
        .collect();
    if filled.is_empty() {
        return false;
    }
    // A continuation extends existing text; it cannot introduce a new column.
    if filled.iter().any(|&i| prev[i].text.trim().is_empty()) {
        return false;
    }
    // Numbers never soft-wrap. A blank-label row carrying any value-like cell is
    // a real data row - most often a `Total` line, whose label column is blank
    // precisely because it isn't the row's own label. Vetoing on *any* value
    // (not all of them) is what separates it from a genuine wrap, since such
    // rows pair a short summary word with the figures.
    if filled.iter().any(|&i| is_value_like(cur[i].text.trim())) {
        return false;
    }
    filled
        .iter()
        .any(|&i| cell_continues(prev[i].text.trim_end(), cur[i].text.trim_start()))
}

/// Whether `cur` reads as the tail of `prev` rather than a new statement. The
/// only shape we reject is a fresh capitalised sentence following a cell that
/// already closed with terminal punctuation - that is a sub-row, not a wrap.
fn cell_continues(prev: &str, cur: &str) -> bool {
    let starts_new_sentence = cur
        .chars()
        .find(|c| c.is_alphabetic())
        .is_some_and(|c| c.is_uppercase());
    let prev_closed = prev.ends_with(['.', '!', '?']);
    !(starts_new_sentence && prev_closed)
}

/// Drop repeated source-item indices, keeping first-occurrence order — the
/// within-cell contract is reading order with no repeats.
fn dedup_preserving_order(v: &mut Vec<usize>) {
    let mut seen = std::collections::HashSet::new();
    v.retain(|i| seen.insert(*i));
}

/// Escape `|` and `\n` inside a markdown table cell so the pipe-table grammar
/// stays valid. Newlines should be impossible inside a single cell (we built
/// cells from spans on the same projected line) but guard anyway.
pub(super) fn escape_table_cell(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('|', "\\|")
        .replace('\n', " ")
}

#[cfg(test)]
mod tests {
    use super::super::test_helpers::{line, line_with_spans, rect_borders, stroke};
    use super::*;

    /// One PDFium run whose words sit at the given (x, width) positions.
    fn run_with_words(words: &[(&str, f32, f32)]) -> TextItem {
        let x = words[0].1;
        let last = words[words.len() - 1];
        TextItem {
            text: words
                .iter()
                .map(|(t, _, _)| *t)
                .collect::<Vec<_>>()
                .join(" "),
            x,
            width: last.1 + last.2 - x,
            height: 10.0,
            font_size: Some(10.0),
            words: words
                .iter()
                .map(|(t, wx, ww)| crate::types::WordBox {
                    text: (*t).into(),
                    x: *wx,
                    y: 0.0,
                    width: *ww,
                    height: 10.0,
                })
                .collect(),
            ..Default::default()
        }
    }

    fn piece_texts(span: &TextItem) -> Vec<String> {
        split_span_at_gutters(span)
            .into_iter()
            .map(|p| p.text)
            .collect()
    }

    #[test]
    fn merged_run_splits_at_its_column_gutters() {
        // doc 190's header: PDFium emits eight cells as one run. In-cell word
        // gaps are 1.76pt; the gutters are 7.7–12.2pt.
        let span = run_with_words(&[
            ("Merge", 172.5, 18.3),
            ("Method", 192.5, 21.9),
            ("H6", 226.6, 8.6),
            ("(Avg.)", 237.0, 18.1),
            ("ARC", 263.5, 14.5),
        ]);
        assert_eq!(piece_texts(&span), ["Merge Method", "H6 (Avg.)", "ARC"]);
    }

    #[test]
    fn justified_prose_does_not_split() {
        // Justification stretches spaces to 3.4pt against a 2.73pt norm - a
        // 1.25× jump, far under the bimodality bar. This is the counterfeit the
        // ratio test exists to reject.
        let span = run_with_words(&[
            ("may", 306.1, 19.0),
            ("not", 327.8, 14.1),
            ("be", 344.6, 10.4),
            ("as", 357.8, 9.2),
            ("crucial.", 369.7, 32.7),
            ("Thus,", 405.8, 24.8),
            ("we", 433.3, 12.8),
        ]);
        assert_eq!(piece_texts(&span).len(), 1);
    }

    #[test]
    fn run_without_word_boxes_stays_whole() {
        let mut span = run_with_words(&[("A", 0.0, 5.0), ("B", 20.0, 5.0), ("C", 26.0, 5.0)]);
        span.words.clear();
        assert_eq!(piece_texts(&span), ["A B C"]);
    }

    #[test]
    fn kerning_outlier_gap_does_not_fake_bimodality() {
        // One degenerate 0.1pt gap beside ordinary ~3.4pt word spaces. The
        // raw ratio between them is huge, but 3.4pt in 10pt type is a word
        // space, not a gutter tier; the font-relative floor must reject it.
        let span = run_with_words(&[
            ("wide", 100.0, 20.0),
            ("gap", 120.1, 15.0),
            ("then", 138.5, 18.0),
            ("normal", 159.9, 25.0),
        ]);
        assert_eq!(piece_texts(&span).len(), 1);
    }

    #[test]
    fn two_word_run_has_no_bimodality_to_measure() {
        // One gap is trivially the largest; there is nothing to compare it to.
        let span = run_with_words(&[("Added", 60.3, 24.1), ("Relative", 118.0, 31.1)]);
        assert_eq!(piece_texts(&span), ["Added Relative"]);
    }

    fn rows(v: &[&[&str]]) -> Vec<Vec<Cell>> {
        v.iter()
            .map(|r| r.iter().map(|s| Cell::from(*s)).collect())
            .collect()
    }

    #[test]
    fn wrapped_cell_lines_fold_into_their_row() {
        let mut r = rows(&[
            &[
                "Knowledge",
                "● To understand the meaning of reducing and recycling",
            ],
            &["", "and how they connect ● To be familiar with the 7 Rs"],
            &["Skills", "● To implement waste management into daily"],
            &["", "life ● To promote reducing before recycling"],
        ]);
        merge_continuation_rows(&mut r);
        assert_eq!(r.len(), 2);
        assert_eq!(
            r[0][1],
            "● To understand the meaning of reducing and recycling and how they connect ● To be familiar with the 7 Rs"
        );
        assert_eq!(r[1][0], "Skills");
    }

    #[test]
    fn totals_row_with_blank_label_is_not_a_continuation() {
        // Regression: doc 45/47. A `Total` line legitimately has an empty label
        // column; folding it into the last data row destroys both rows.
        let mut r = rows(&[
            &[
                "7",
                "Traditional and Modern Mental Health Organization",
                "15",
            ],
            &["", "Total", "27,926"],
        ]);
        merge_continuation_rows(&mut r);
        assert_eq!(r.len(), 2);
        assert_eq!(r[1][1], "Total");
    }

    #[test]
    fn sparse_first_column_disables_continuation_merging() {
        // Regression: a timetable whose `Notes` column is blank on every run.
        // Every row looks like a continuation, and merging collapses the whole
        // table into one row (0.859 -> 0.006 GriTS on ParseBench).
        let mut r = rows(&[
            &["", "8:24", "8:28", "8:41"],
            &["", "8:29", "8:33", "8:46"],
            &["", "8:33", "8:37", "8:50"],
            &["", "8:38", "8:42", "8:55"],
        ]);
        merge_continuation_rows(&mut r);
        assert_eq!(r.len(), 4);
    }

    #[test]
    fn continuation_may_not_introduce_a_new_column() {
        let mut r = rows(&[&["Label", "some text", ""], &["", "more text", "brand new"]]);
        merge_continuation_rows(&mut r);
        assert_eq!(r.len(), 2, "col 2 was empty above, so this is a real row");
    }

    #[test]
    fn fresh_sentence_after_closed_cell_is_not_a_continuation() {
        let mut r = rows(&[
            &["Label", "A complete thought ending here."],
            &["", "Another separate statement entirely."],
        ]);
        merge_continuation_rows(&mut r);
        assert_eq!(r.len(), 2);
    }

    #[test]
    fn gutter_columns_fused_but_narrow_real_column_kept() {
        // Padded cell borders: real columns 40..140, 160..260, 280..380 with
        // 20pt gutters between them. Text sits inside the real columns only.
        let rows = [
            line_with_spans(
                &[("alpha", 45.0), ("beta", 165.0), ("gamma", 285.0)],
                100.0,
                10.0,
            ),
            line_with_spans(
                &[("delta", 45.0), ("eps", 165.0), ("zeta", 285.0)],
                120.0,
                10.0,
            ),
        ];
        let mut xs = vec![40.0, 140.0, 160.0, 260.0, 280.0, 380.0];
        collapse_gutter_columns(&mut xs, &rows, false);
        assert_eq!(xs.len(), 4, "two 20pt gutters should fuse: {xs:?}");

        // A genuinely narrow but *occupied* column must survive, even though it
        // is the same width as the gutters above.
        let rows = [
            line_with_spans(
                &[("1", 45.0), ("desc", 65.0), ("more text", 205.0)],
                100.0,
                10.0,
            ),
            line_with_spans(
                &[("2", 45.0), ("desc two", 65.0), ("text here", 205.0)],
                120.0,
                10.0,
            ),
        ];
        let mut xs = vec![40.0, 60.0, 200.0, 380.0];
        let before = xs.clone();
        collapse_gutter_columns(&mut xs, &rows, false);
        assert_eq!(xs, before, "occupied narrow column must not fuse");
    }

    #[test]
    fn blank_worksheet_column_survives_gutter_collapse() {
        // A worksheet's empty answer column is centerless like a gutter but is
        // full width, so the width guard must keep it.
        let rows = [
            line_with_spans(&[("K+", 45.0)], 100.0, 10.0),
            line_with_spans(&[("Na+", 45.0)], 120.0, 10.0),
        ];
        let mut xs = vec![40.0, 240.0, 440.0, 640.0];
        let before = xs.clone();
        collapse_gutter_columns(&mut xs, &rows, false);
        assert_eq!(xs, before, "wide blank columns must not fuse: {xs:?}");
    }

    #[test]
    fn split_cells_splits_on_wide_gaps() {
        let l = line_with_spans(&[("A", 50.0), ("B", 150.0), ("C", 250.0)], 100.0, 10.0);
        let cells = split_cells(&l);
        assert_eq!(cells.len(), 3);
        assert_eq!(cells[0].text, "A");
        assert_eq!(cells[1].text, "B");
        assert_eq!(cells[2].text, "C");
    }

    fn hairline(y: f32, x: f32, w: f32) -> GraphicPrimitive {
        GraphicPrimitive::Rect {
            bbox: Rect {
                x,
                y,
                width: w,
                height: 0.8,
            },
            stroke: Some("#000".to_string()),
            fill: None,
        }
    }

    #[test]
    fn booktabs_rule_pair_is_a_band() {
        // doc 165: the whole table is two hairlines 98pt apart, no verticals.
        let g = vec![hairline(139.5, 56.7, 225.9), hairline(237.4, 56.7, 225.9)];
        assert_eq!(
            rule_bands(&extract_rule_segments(&g), 792.0),
            [(139.5, 237.4)]
        );
    }

    #[test]
    fn ruled_grid_is_not_a_band() {
        // Same rules, but a vertical runs between them: this is a real grid and
        // the ruled detector owns it. Relaxing TABLE_MIN_COLUMNS here would be
        // a second, worse opinion about the same table.
        let mut g = vec![hairline(139.5, 56.7, 225.9), hairline(237.4, 56.7, 225.9)];
        g.push(GraphicPrimitive::Rect {
            bbox: Rect {
                x: 150.0,
                y: 139.5,
                width: 0.8,
                height: 97.9,
            },
            stroke: Some("#000".to_string()),
            fill: None,
        });
        assert!(rule_bands(&extract_rule_segments(&g), 792.0).is_empty());
    }

    #[test]
    fn heading_underline_pair_is_not_a_band() {
        // Two rules 6pt apart bound a heading underline, not a table body.
        let g = vec![hairline(100.0, 56.7, 225.9), hairline(106.0, 56.7, 225.9)];
        assert!(rule_bands(&extract_rule_segments(&g), 792.0).is_empty());
    }

    /// Four full-height dividers plus one short pair from a highlight box drawn
    /// inside a cell - the doc-200 shape.
    fn vlines_with_intruder() -> (Vec<HSeg>, Vec<VSeg>) {
        let hs = (0..5)
            .map(|r| HSeg {
                x_min: 0.0,
                x_max: 300.0,
                y: r as f32 * 25.0,
            })
            .collect();
        let mut vs: Vec<VSeg> = [0.0, 100.0, 200.0, 300.0]
            .iter()
            .map(|&x| VSeg {
                y_min: 0.0,
                y_max: 100.0,
                x,
            })
            .collect();
        vs.push(VSeg {
            y_min: 40.0,
            y_max: 49.0,
            x: 140.0,
        });
        vs.push(VSeg {
            y_min: 40.0,
            y_max: 49.0,
            x: 162.0,
        });
        (hs, vs)
    }

    #[test]
    fn in_cell_highlight_box_does_not_become_a_column() {
        let (hs, vs) = vlines_with_intruder();
        let kept = filter_short_vlines(&hs, &[0, 1, 2, 3, 4], &vs, &[0, 1, 2, 3, 4, 5]);
        assert_eq!(kept, vec![0, 1, 2, 3]);
    }

    #[test]
    fn filter_never_leaves_fewer_than_three_columns() {
        // Only two dividers reach full height; filtering would leave a 1-column
        // grid, so the raw set survives and the usual gates decide.
        let (hs, mut vs) = vlines_with_intruder();
        vs[2].y_min = 40.0;
        vs[2].y_max = 49.0;
        vs[3].y_min = 40.0;
        vs[3].y_max = 49.0;
        let kept = filter_short_vlines(&hs, &[0, 1, 2, 3, 4], &vs, &[0, 1, 2, 3, 4, 5]);
        assert_eq!(kept, vec![0, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn unruled_outer_column_is_recovered_from_the_horizontals() {
        // Interior dividers at 124/383/652; the horizontals say the table runs
        // 30..930, so the label column and the last data column are missing.
        let mut xs = vec![123.8, 382.9, 652.1];
        extend_to_perpendicular_extent(
            &mut xs,
            &[29.7, 29.7, 29.8],
            &[930.3, 930.3, 930.2],
            0.0,
            |_, _| true,
        );
        assert_eq!(xs, vec![29.7, 123.8, 382.9, 652.1, 930.3]);
    }

    #[test]
    fn outer_band_shorter_than_any_row_is_rule_overshoot() {
        // Verticals overrun the last horizontal by 15.5pt on 26pt rows - a tail,
        // not a row.
        let mut ys = vec![181.4, 207.8, 234.2];
        extend_to_perpendicular_extent(&mut ys, &[181.4], &[249.7], 26.4, |_, _| true);
        assert_eq!(ys, vec![181.4, 207.8, 234.2]);
    }

    #[test]
    fn empty_outer_band_is_not_a_column() {
        let mut xs = vec![123.8, 382.9, 652.1];
        extend_to_perpendicular_extent(&mut xs, &[29.7], &[930.3], 0.0, |_, _| false);
        assert_eq!(xs, vec![123.8, 382.9, 652.1]);
    }

    #[test]
    fn one_overshooting_rule_does_not_widen_the_grid() {
        // Median, not min: a single stroke reaching to 29.7 is noise.
        let mut xs = vec![123.8, 382.9, 652.1];
        extend_to_perpendicular_extent(
            &mut xs,
            &[29.7, 124.0, 123.9],
            &[652.0, 652.2, 930.3],
            0.0,
            |_, _| true,
        );
        assert_eq!(xs, vec![123.8, 382.9, 652.1]);
    }

    #[test]
    fn rowspan_mask_marks_unruled_cell_boundaries() {
        // Two columns; the boundary at y=20 is ruled only across column 1, so
        // column 0's cell there is merged with the one above it.
        let hs = vec![
            HSeg {
                x_min: 0.0,
                x_max: 100.0,
                y: 0.0,
            },
            HSeg {
                x_min: 50.0,
                x_max: 100.0,
                y: 20.0,
            },
            HSeg {
                x_min: 0.0,
                x_max: 100.0,
                y: 40.0,
            },
        ];
        let vs = vec![VSeg {
            y_min: 0.0,
            y_max: 40.0,
            x: 50.0,
        }];
        let mask = rowspan_mask(
            &hs,
            &[0, 1, 2],
            &vs,
            &[0],
            &[0.0, 50.0, 100.0],
            &[0.0, 20.0, 40.0],
        );
        assert_eq!(mask, vec![vec![false, false], vec![true, false]]);
    }

    #[test]
    fn rowspan_mask_ignores_boundary_where_grid_ends() {
        // No vertical crosses y=20, so the grid simply stops there - nothing is
        // merged, and forgiving these empties would let a page frame shred
        // prose into columns.
        let hs = vec![
            HSeg {
                x_min: 0.0,
                x_max: 10.0,
                y: 0.0,
            },
            HSeg {
                x_min: 0.0,
                x_max: 10.0,
                y: 20.0,
            },
            HSeg {
                x_min: 0.0,
                x_max: 10.0,
                y: 40.0,
            },
        ];
        let vs = vec![VSeg {
            y_min: 0.0,
            y_max: 15.0,
            x: 50.0,
        }];
        let mask = rowspan_mask(
            &hs,
            &[0, 1, 2],
            &vs,
            &[0],
            &[0.0, 50.0, 100.0],
            &[0.0, 20.0, 40.0],
        );
        assert!(mask.iter().flatten().all(|m| !*m));
    }

    #[test]
    fn recover_merged_cell_splits_off_by_one() {
        // Mimics the page-6 case: row 0 establishes 3 tracks at 50/150/250.
        // Row 1's projection merges "MEMORYBANK" + "5.00" into one span at
        // x=50 width=110, so split_cells yields 2 cells while the table
        // expects 3. Recovery must split on whitespace at the missing track.
        let row = vec![
            TableCell {
                start_x: 50.0,
                end_x: 160.0,
                text: "MEMORYBANK 5.00".into(),
                bold: false,
                text_item_indices: vec![],
            },
            TableCell {
                start_x: 250.0,
                end_x: 280.0,
                text: "4.77".into(),
                bold: false,
                text_item_indices: vec![],
            },
        ];
        let tracks = vec![50.0, 150.0, 250.0];
        let out = recover_merged_cell(row, &tracks).expect("recovery should succeed");
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].text, "MEMORYBANK");
        assert_eq!(out[1].text, "5.00");
        assert_eq!(out[2].text, "4.77");
    }

    #[test]
    fn recover_merged_cell_splits_off_by_two() {
        // Three merged tokens in one cell: "MEMORYBANK 13.18 10.03" straddles
        // tracks at 50/150/250 and the row has only 2 cells, off by 2.
        let row = vec![
            TableCell {
                start_x: 50.0,
                end_x: 260.0,
                text: "MEMORYBANK 13.18 10.03".into(),
                bold: false,
                text_item_indices: vec![],
            },
            TableCell {
                start_x: 350.0,
                end_x: 380.0,
                text: "7.61".into(),
                bold: false,
                text_item_indices: vec![],
            },
        ];
        let tracks = vec![50.0, 150.0, 250.0, 350.0];
        let out = recover_merged_cell(row, &tracks).expect("recovery should succeed");
        assert_eq!(out.len(), 4);
        assert_eq!(out[0].text, "MEMORYBANK");
        assert_eq!(out[1].text, "13.18");
        assert_eq!(out[2].text, "10.03");
        assert_eq!(out[3].text, "7.61");
    }

    #[test]
    fn recover_merged_cell_bails_without_enough_whitespace() {
        // A cell that straddles two tracks but has no internal whitespace
        // (e.g. a hyphenated token) can't be safely split — return None.
        let row = vec![TableCell {
            start_x: 50.0,
            end_x: 200.0,
            text: "ABC-DEF-GHI".into(),
            bold: false,
                text_item_indices: vec![],
        }];
        let tracks = vec![50.0, 150.0];
        assert!(recover_merged_cell(row, &tracks).is_none());
    }

    #[test]
    fn split_cells_keeps_close_spans_together() {
        // Two spans 2pt apart at 10pt font (gap < font_size) → same cell.
        let l = line_with_spans(&[("Hello", 50.0), ("world", 80.0)], 100.0, 10.0);
        let cells = split_cells(&l);
        assert_eq!(cells.len(), 1);
        assert_eq!(cells[0].text, "Hello world");
    }

    #[test]
    fn absorbs_partial_header_line_above_body() {
        // A header line with only two track-aligned cells sits above a clean
        // 3-column body. It can't start the table on its own (fewer than
        // TABLE_MIN_COLUMNS cells) but should be walked back in as the header.
        let lines = vec![
            line_with_spans(&[("Name", 50.0), ("Scores", 150.0)], 100.0, 10.0),
            line_with_spans(&[("A", 50.0), ("1", 150.0), ("2", 250.0)], 115.0, 10.0),
            line_with_spans(&[("B", 50.0), ("3", 150.0), ("4", 250.0)], 130.0, 10.0),
            line_with_spans(&[("C", 50.0), ("5", 150.0), ("6", 250.0)], 145.0, 10.0),
        ];
        let runs = detect_tables(&lines);
        assert_eq!(runs.len(), 1);
        let run = &runs[0];
        assert_eq!(run.start, 0, "header line should be absorbed into the run");
        assert_eq!(run.end, 4);
        match &run.block {
            Block::Table { header, rows } => {
                let header = header.as_ref().expect("header should be present");
                assert_eq!(
                    header,
                    &vec!["Name".to_string(), "Scores".to_string(), String::new()]
                );
                // All three body rows survive — the header came from above, so
                // rows[0] is not consumed as a header.
                assert_eq!(rows.len(), 3);
            }
            other => panic!("expected Block::Table, got {other:?}"),
        }
    }

    #[test]
    fn does_not_absorb_single_cell_title_above_body() {
        // A one-cell title/caption above a table is NOT a header row and must
        // not be absorbed.
        let lines = vec![
            line_with_spans(&[("Results", 50.0)], 100.0, 10.0),
            line_with_spans(&[("A", 50.0), ("1", 150.0), ("2", 250.0)], 115.0, 10.0),
            line_with_spans(&[("B", 50.0), ("3", 150.0), ("4", 250.0)], 130.0, 10.0),
            line_with_spans(&[("C", 50.0), ("5", 150.0), ("6", 250.0)], 145.0, 10.0),
        ];
        let runs = detect_tables(&lines);
        assert_eq!(runs.len(), 1);
        assert_eq!(
            runs[0].start, 1,
            "single-cell title must stay out of the run"
        );
    }

    #[test]
    fn count_text_table_runs_excludes_description_lists() {
        // A label/value stack detects as a description-list run, but must not
        // count toward the layout-complexity table signal.
        let lines = vec![
            line_with_spans(&[("Name:", 50.0), ("Alice", 200.0)], 100.0, 10.0),
            line_with_spans(&[("Role:", 50.0), ("Engineer", 200.0)], 115.0, 10.0),
            line_with_spans(&[("Team:", 50.0), ("Platform", 200.0)], 130.0, 10.0),
            line_with_spans(&[("Start:", 50.0), ("2020", 200.0)], 145.0, 10.0),
        ];
        assert_eq!(
            detect_tables(&lines).len(),
            1,
            "sanity: the emitter path should still claim this as a desc-list run"
        );
        assert_eq!(count_text_table_runs(&lines), 0);
    }

    #[test]
    fn count_text_table_runs_counts_borderless_tables() {
        let lines = vec![
            line_with_spans(&[("A", 50.0), ("1", 150.0), ("2", 250.0)], 100.0, 10.0),
            line_with_spans(&[("B", 50.0), ("3", 150.0), ("4", 250.0)], 115.0, 10.0),
            line_with_spans(&[("C", 50.0), ("5", 150.0), ("6", 250.0)], 130.0, 10.0),
        ];
        assert_eq!(count_text_table_runs(&lines), 1);
    }

    #[test]
    fn rejects_table_when_row_count_too_low() {
        let lines = vec![line_with_spans(
            &[("A", 50.0), ("B", 150.0), ("C", 250.0)],
            100.0,
            10.0,
        )];
        let runs = detect_tables(&lines);
        assert!(runs.is_empty());
    }

    #[test]
    fn rejects_table_when_column_count_too_low() {
        let lines = vec![
            line_with_spans(&[("A", 50.0), ("B", 200.0)], 100.0, 10.0),
            line_with_spans(&[("C", 50.0), ("D", 200.0)], 115.0, 10.0),
        ];
        let runs = detect_tables(&lines);
        assert!(runs.is_empty());
    }

    #[test]
    fn escapes_pipe_inside_cell() {
        assert_eq!(escape_table_cell("a|b"), "a\\|b");
    }

    #[test]
    fn ruled_table_2x2_detected() {
        // 2 rows × 2 cols grid: 3 H lines (y=100,140,180), 3 V lines (x=50,150,250)
        // Cell text dropped in the centroid of each cell.
        let mut graphics = Vec::new();
        for y in [100.0_f32, 140.0, 180.0] {
            graphics.push(stroke(50.0, y, 250.0, y, 0.5));
        }
        for x in [50.0_f32, 150.0, 250.0] {
            graphics.push(stroke(x, 100.0, x, 180.0, 0.5));
        }

        // Text lines: one per cell, centered.
        let lines = vec![
            line("a", 90.0, 115.0, 10.0, 10.0),  // row 0, col 0
            line("b", 190.0, 115.0, 10.0, 10.0), // row 0, col 1
            line("c", 90.0, 155.0, 10.0, 10.0),  // row 1, col 0
            line("d", 190.0, 155.0, 10.0, 10.0), // row 1, col 1
        ];

        let runs = detect_ruled_tables(&lines, &extract_rule_segments(&graphics), 612.0, 792.0);
        assert_eq!(runs.len(), 1, "expected 1 ruled table, got {runs:?}");
        match &runs[0].block {
            Block::Table { header, rows } => {
                assert!(header.is_none(), "no bold first row → no header");
                assert_eq!(rows.len(), 2);
                assert_eq!(rows[0], vec!["a", "b"]);
                assert_eq!(rows[1], vec!["c", "d"]);
            }
            other => panic!("expected Block::Table, got {other:?}"),
        }
    }

    #[test]
    fn ruled_table_cells_carry_their_grid_rect() {
        // Same 2×2 grid as above: column boundaries at x=50/150/250, row
        // boundaries at y=100/140/180. A ruled table's cell geometry is the
        // drawn cell itself, so each cell's box must be exactly one grid
        // rectangle — independent of where the glyphs sit inside it.
        let mut graphics = Vec::new();
        for y in [100.0_f32, 140.0, 180.0] {
            graphics.push(stroke(50.0, y, 250.0, y, 0.5));
        }
        for x in [50.0_f32, 150.0, 250.0] {
            graphics.push(stroke(x, 100.0, x, 180.0, 0.5));
        }
        let lines = vec![
            line("a", 90.0, 115.0, 10.0, 10.0),
            line("b", 190.0, 115.0, 10.0, 10.0),
            line("c", 90.0, 155.0, 10.0, 10.0),
            line("d", 190.0, 155.0, 10.0, 10.0),
        ];

        let runs = detect_ruled_tables(&lines, &extract_rule_segments(&graphics), 612.0, 792.0);
        let Block::Table { rows, .. } = &runs[0].block else {
            panic!("expected Block::Table");
        };
        let rect = |r: usize, c: usize| {
            let b = rows[r][c].bbox.clone().expect("ruled cell must be located");
            (b.x, b.y, b.width, b.height)
        };
        assert_eq!(rows[0][0].text, "a");
        assert_eq!(rect(0, 0), (50.0, 100.0, 100.0, 40.0));
        assert_eq!(rect(0, 1), (150.0, 100.0, 100.0, 40.0));
        assert_eq!(rect(1, 0), (50.0, 140.0, 100.0, 40.0));
        assert_eq!(rect(1, 1), (150.0, 140.0, 100.0, 40.0));
    }

    #[test]
    fn ruled_table_cells_carry_source_item_indices() {
        // 2×2 ruled grid; every span carries a globally distinct source-item
        // index, and each grid cell must attribute to exactly its own span.
        let mut graphics = Vec::new();
        for y in [100.0_f32, 140.0, 180.0] {
            graphics.push(stroke(50.0, y, 250.0, y, 0.5));
        }
        for x in [50.0_f32, 150.0, 250.0] {
            graphics.push(stroke(x, 100.0, x, 180.0, 0.5));
        }
        let mut lines = vec![
            line_with_spans(&[("a", 90.0), ("b", 190.0)], 115.0, 10.0),
            line_with_spans(&[("c", 90.0), ("d", 190.0)], 155.0, 10.0),
        ];
        lines[0].span_item_indices = vec![10, 11];
        lines[1].span_item_indices = vec![12, 13];
        let runs = detect_ruled_tables(&lines, &extract_rule_segments(&graphics), 612.0, 792.0);
        assert_eq!(runs.len(), 1);
        let Block::Table { rows, .. } = &runs[0].block else {
            panic!("expected Block::Table");
        };
        assert_eq!(rows[0][0].text_item_indices, vec![10]);
        assert_eq!(rows[0][1].text_item_indices, vec![11]);
        assert_eq!(rows[1][0].text_item_indices, vec![12]);
        assert_eq!(rows[1][1].text_item_indices, vec![13]);
        let mut block = runs[0].block_item_indices();
        block.sort_unstable();
        assert_eq!(block, vec![10, 11, 12, 13]);
    }

    #[test]
    fn ruled_straddle_split_duplicates_source_index() {
        // Row 1's single span crosses the interior column boundary at x=150;
        // the char-estimate split carves it into both cells, and BOTH cells
        // attribute to that one span.
        let mut graphics = Vec::new();
        for y in [100.0_f32, 140.0, 180.0] {
            graphics.push(stroke(50.0, y, 250.0, y, 0.5));
        }
        for x in [50.0_f32, 150.0, 250.0] {
            graphics.push(stroke(x, 100.0, x, 180.0, 0.5));
        }
        let mut lines = vec![
            line_with_spans(&[("a", 90.0), ("b", 190.0)], 115.0, 10.0),
            line_with_spans(
                &[("aaaaaaaaaaaaaaaaaaa bbbbbbbbbbbbbbbbbbbb", 55.0)],
                155.0,
                10.0,
            ),
        ];
        lines[0].span_item_indices = vec![0, 1];
        lines[1].span_item_indices = vec![2];
        let runs = detect_ruled_tables(&lines, &extract_rule_segments(&graphics), 612.0, 792.0);
        assert_eq!(runs.len(), 1);
        let Block::Table { rows, .. } = &runs[0].block else {
            panic!("expected Block::Table");
        };
        assert!(!rows[1][0].text.is_empty() && !rows[1][1].text.is_empty());
        assert_eq!(rows[1][0].text_item_indices, vec![2]);
        assert_eq!(rows[1][1].text_item_indices, vec![2]);
    }

    #[test]
    fn union_header_fold_carries_span_indices() {
        // A spanning group header ("GroupGroupGroup" covering tracks 1 and 2)
        // replicates into both columns; each folded header cell must carry
        // the contributing spans' indices, deduped within the cell.
        let mut header_line =
            line_with_spans(&[("Region", 50.0), ("GroupGroupGroup", 145.0)], 100.0, 20.0);
        header_line.span_item_indices = vec![7, 8];
        let mut body = line_with_spans(&[("x", 50.0), ("1", 150.0), ("2", 250.0)], 125.0, 10.0);
        body.span_item_indices = vec![9, 10, 11];
        let lines = vec![header_line, body];
        let (start, header) = union_header_from_above(&lines, 1, 0, &[50.0, 150.0, 250.0])
            .expect("header should absorb");
        assert_eq!(start, 0);
        assert_eq!(header.len(), 3);
        assert_eq!(header[0].text_item_indices, vec![7]);
        assert_eq!(header[1].text_item_indices, vec![8]);
        assert_eq!(header[2].text_item_indices, vec![8]);
    }

    #[test]
    fn union_header_fold_attributes_duplicate_text_layers() {
        // Two stacked header layers repeat the same text in column 0 from
        // DIFFERENT source items ("Rated" printed on both band rows). The
        // fold de-duplicates the text but must still attribute both items —
        // otherwise the second row's item belongs to no block.
        let mut top = line_with_spans(&[("Rated", 50.0), ("Voltage", 150.0)], 100.0, 10.0);
        top.span_item_indices = vec![1, 2];
        let mut bottom = line_with_spans(&[("Rated", 50.0), ("(VDC)", 150.0)], 115.0, 10.0);
        bottom.span_item_indices = vec![3, 4];
        let mut body = line_with_spans(&[("x", 50.0), ("1", 150.0)], 130.0, 10.0);
        body.span_item_indices = vec![5, 6];
        let lines = vec![top, bottom, body];
        let (start, header) = union_header_from_above(&lines, 2, 0, &[50.0, 150.0])
            .expect("header should absorb");
        assert_eq!(start, 0);
        assert_eq!(header[0].text, "Rated");
        assert_eq!(header[0].text_item_indices, vec![1, 3]);
        assert_eq!(header[1].text, "Voltage (VDC)");
        assert_eq!(header[1].text_item_indices, vec![2, 4]);
    }

    #[test]
    fn borderless_table_cells_carry_span_extents() {
        // A borderless table has no drawn boundaries, so a cell covers the ink
        // of its own spans on that row — not a full column track. Each cell's
        // box must therefore start at its span's x and sit in its line's band.
        let lines = vec![
            line_with_spans(
                &[("Name", 50.0), ("Qty", 150.0), ("Cost", 250.0)],
                100.0,
                10.0,
            ),
            line_with_spans(
                &[("Bolt", 50.0), ("12", 150.0), ("3.50", 250.0)],
                115.0,
                10.0,
            ),
            line_with_spans(
                &[("Nut", 50.0), ("40", 150.0), ("1.25", 250.0)],
                130.0,
                10.0,
            ),
        ];
        let runs = detect_tables(&lines);
        assert_eq!(runs.len(), 1, "expected 1 borderless table, got {runs:?}");
        let Block::Table { rows, .. } = &runs[0].block else {
            panic!("expected Block::Table");
        };
        // Every detected cell knows where it came from.
        for row in rows {
            for cell in row {
                assert!(cell.bbox.is_some(), "cell {:?} lost its box", cell.text);
            }
        }
        let first = rows[0][0].bbox.clone().unwrap();
        assert_eq!(first.x, 50.0, "cell starts at its span's x");
        // The row's own band, not the whole table's.
        let row0_y = rows[0][0].bbox.clone().unwrap().y;
        let row1_y = rows[1][0].bbox.clone().unwrap().y;
        assert!(
            row1_y > row0_y,
            "rows must occupy distinct bands ({row0_y} then {row1_y})"
        );
        // Columns stay separated horizontally.
        assert_eq!(rows[0][1].bbox.clone().unwrap().x, 150.0);
        assert_eq!(rows[0][2].bbox.clone().unwrap().x, 250.0);
    }

    #[test]
    fn borderless_table_cells_carry_source_item_indices() {
        // Same shape as the span-extents test, but every span carries a
        // globally distinct source-item index: cell (r, c) must attribute to
        // exactly index r*3 + c.
        let mut lines = vec![
            line_with_spans(
                &[("Name", 50.0), ("Qty", 150.0), ("Cost", 250.0)],
                100.0,
                10.0,
            ),
            line_with_spans(
                &[("Bolt", 50.0), ("12", 150.0), ("3.50", 250.0)],
                115.0,
                10.0,
            ),
            line_with_spans(
                &[("Nut", 50.0), ("40", 150.0), ("1.25", 250.0)],
                130.0,
                10.0,
            ),
        ];
        for (r, l) in lines.iter_mut().enumerate() {
            l.span_item_indices = (r * 3..r * 3 + 3).collect();
        }
        let runs = detect_tables(&lines);
        assert_eq!(runs.len(), 1, "expected 1 borderless table, got {runs:?}");
        let Block::Table { header, rows } = &runs[0].block else {
            panic!("expected Block::Table");
        };
        assert!(header.is_none());
        for (r, row) in rows.iter().enumerate() {
            for (c, cell) in row.iter().enumerate() {
                assert_eq!(
                    cell.text_item_indices,
                    vec![r * 3 + c],
                    "cell ({r},{c}) '{}' mis-attributed",
                    cell.text
                );
            }
        }
        // The run's block-level indices are the union of the cells'.
        let mut block = runs[0].block_item_indices();
        block.sort_unstable();
        assert_eq!(block, (0..9).collect::<Vec<_>>());
    }

    #[test]
    fn merged_span_split_duplicates_source_index_into_each_cell() {
        // One span whose x-extent covers two tracks: the char-estimate split
        // carves it into two cells, and BOTH cells attribute to the single
        // source span.
        let l = line_with_spans(&[("aaaaaaaaaa bbbbbbbbbb", 50.0)], 100.0, 10.0);
        let pieces = line_pieces(&l);
        assert_eq!(pieces.len(), 1);
        assert_eq!(pieces[0].source_index, Some(0));
        let cells =
            cells_from_pieces(&pieces, &[50.0, 150.0], true).expect("split should succeed");
        assert_eq!(cells.len(), 2);
        assert_eq!(cells[0].text, "aaaaaaaaaa");
        assert_eq!(cells[1].text, "bbbbbbbbbb");
        assert_eq!(cells[0].text_item_indices, vec![0]);
        assert_eq!(cells[1].text_item_indices, vec![0]);
    }

    #[test]
    fn track_cells_without_content_carry_no_indices() {
        // Three tracks, two pieces: the unpopulated track's cell stays empty
        // of both text and attribution.
        let l = line_with_spans(&[("alpha", 50.0), ("beta", 150.0)], 100.0, 10.0);
        let pieces = line_pieces(&l);
        let cells =
            cells_from_pieces(&pieces, &[50.0, 150.0, 250.0], false).expect("bin should succeed");
        assert_eq!(cells.len(), 3);
        assert_eq!(cells[0].text_item_indices, vec![0]);
        assert_eq!(cells[1].text_item_indices, vec![1]);
        assert!(cells[2].text.is_empty());
        assert!(cells[2].text_item_indices.is_empty());
    }

    #[test]
    fn split_cells_groups_indices_with_their_cells() {
        // Spans 0+1 sit close (one cell); span 2 is across a wide gap.
        let l = line_with_spans(&[("a", 50.0), ("b", 58.0), ("c", 250.0)], 100.0, 10.0);
        let cells = split_cells(&l);
        assert_eq!(cells.len(), 2);
        assert_eq!(cells[0].text_item_indices, vec![0, 1]);
        assert_eq!(cells[1].text_item_indices, vec![2]);
    }

    #[test]
    fn continuation_row_merge_folds_indices() {
        let mut r = rows(&[
            &["Knowledge", "To understand the meaning of reducing"],
            &["", "and how they connect"],
        ]);
        r[0][0].text_item_indices = vec![0];
        r[0][1].text_item_indices = vec![1];
        r[1][1].text_item_indices = vec![2];
        merge_continuation_rows(&mut r);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0][1].text_item_indices, vec![1, 2]);
    }

    #[test]
    fn ruled_table_rect_borders_detected() {
        // Same 2×2 table but drawn as 4 individual cell rects (each cell is a
        // stroked rectangle). Each rect contributes 4 strokes via
        // extract_h_v_segments.
        let mut graphics = Vec::new();
        graphics.extend(rect_borders(50.0, 100.0, 100.0, 40.0)); // r0 c0
        graphics.extend(rect_borders(150.0, 100.0, 100.0, 40.0)); // r0 c1
        graphics.extend(rect_borders(50.0, 140.0, 100.0, 40.0)); // r1 c0
        graphics.extend(rect_borders(150.0, 140.0, 100.0, 40.0)); // r1 c1

        let lines = vec![
            line("a", 90.0, 115.0, 10.0, 10.0),
            line("b", 190.0, 115.0, 10.0, 10.0),
            line("c", 90.0, 155.0, 10.0, 10.0),
            line("d", 190.0, 155.0, 10.0, 10.0),
        ];
        let runs = detect_ruled_tables(&lines, &extract_rule_segments(&graphics), 612.0, 792.0);
        assert_eq!(runs.len(), 1);
    }

    #[test]
    fn ruled_table_page_border_rejected() {
        // Single big rect covering ~the whole page → should NOT be treated as a
        // table even though it has H+V lines on all four sides.
        let graphics = rect_borders(10.0, 10.0, 590.0, 770.0);
        let lines = vec![line("body text", 50.0, 400.0, 10.0, 10.0)];
        let runs = detect_ruled_tables(&lines, &extract_rule_segments(&graphics), 612.0, 792.0);
        assert!(
            runs.is_empty(),
            "page-border rect should not become a table, got {runs:?}"
        );
    }

    #[test]
    fn ruled_table_mostly_empty_rejected() {
        // 3×3 grid with text in only one cell — empty fraction 8/9 ≈ 89% >> 30%.
        let mut graphics = Vec::new();
        for y in [100.0_f32, 130.0, 160.0, 190.0] {
            graphics.push(stroke(50.0, y, 350.0, y, 0.5));
        }
        for x in [50.0_f32, 150.0, 250.0, 350.0] {
            graphics.push(stroke(x, 100.0, x, 190.0, 0.5));
        }
        let lines = vec![line("only", 90.0, 115.0, 10.0, 10.0)];
        let runs = detect_ruled_tables(&lines, &extract_rule_segments(&graphics), 612.0, 792.0);
        assert!(runs.is_empty());
    }

    #[test]
    fn ruled_table_first_row_bold_becomes_header() {
        // 2×2 with first row text marked all_bold → header promotion.
        let mut graphics = Vec::new();
        for y in [100.0_f32, 140.0, 180.0] {
            graphics.push(stroke(50.0, y, 250.0, y, 0.5));
        }
        for x in [50.0_f32, 150.0, 250.0] {
            graphics.push(stroke(x, 100.0, x, 180.0, 0.5));
        }
        let mut a = line("Name", 90.0, 115.0, 10.0, 10.0);
        let mut b = line("Score", 190.0, 115.0, 10.0, 10.0);
        a.all_bold = true;
        b.all_bold = true;
        let lines = vec![
            a,
            b,
            line("alice", 90.0, 155.0, 10.0, 10.0),
            line("99", 190.0, 155.0, 10.0, 10.0),
        ];
        let runs = detect_ruled_tables(&lines, &extract_rule_segments(&graphics), 612.0, 792.0);
        assert_eq!(runs.len(), 1);
        match &runs[0].block {
            Block::Table { header, rows } => {
                assert_eq!(
                    header.as_deref(),
                    Some(&["Name".into(), "Score".into()][..])
                );
                assert_eq!(rows.len(), 1);
                assert_eq!(rows[0], vec!["alice", "99"]);
            }
            other => panic!("expected Block::Table, got {other:?}"),
        }
    }

    #[test]
    fn merge_prefers_ruled_when_overlapping() {
        let ruled = vec![TableRun {
            start: 5,
            end: 10,
            body_start: 5,
            block_indices: vec![],
            bbox: None,
            block: Block::Table {
                header: None,
                rows: vec![vec!["ruled".into()]],
            },
        }];
        let borderless = vec![TableRun {
            start: 6,
            end: 11,
            body_start: 6,
            block_indices: vec![],
            bbox: None,
            block: Block::GridFallback {
                lines: vec!["bl".into()],
            },
        }];
        let merged = merge_table_runs(ruled, borderless);
        assert_eq!(merged.len(), 1);
        assert!(matches!(&merged[0].block, Block::Table { .. }));
    }

    // ── merge_consecutive_table_runs ─────────────────────────────────────
    //
    // Lines fixtures used by these tests are synthetic 3-cell and 4-cell
    // rows at known x positions so the re-derived tracks match the runs we
    // construct manually.

    fn three_col_line(label: &str, y: f32) -> ProjectedLine {
        line_with_spans(&[(label, 50.0), (label, 150.0), (label, 250.0)], y, 10.0)
    }

    fn four_col_line(label: &str, y: f32) -> ProjectedLine {
        line_with_spans(
            &[
                (label, 50.0),
                (label, 150.0),
                (label, 250.0),
                (label, 350.0),
            ],
            y,
            10.0,
        )
    }

    // A row whose three cells sit at tracks 2..4 of a 4-col layout
    // (subset of the 4-col tracks: missing leftmost column at x=50).
    fn three_col_subset_line(label: &str, y: f32) -> ProjectedLine {
        line_with_spans(&[(label, 150.0), (label, 250.0), (label, 350.0)], y, 10.0)
    }

    #[test]
    fn merge_same_column_count_concatenates_rows() {
        let lines = vec![
            three_col_line("h1", 10.0),
            three_col_line("h2", 25.0),
            three_col_line("b1", 40.0),
            three_col_line("b2", 55.0),
            three_col_line("b3", 70.0),
        ];
        let a = TableRun {
            start: 0,
            end: 2,
            body_start: 0,
            block_indices: vec![],
            bbox: None,
            block: Block::Table {
                header: Some(vec!["A".into(), "B".into(), "C".into()]),
                rows: vec![vec!["1".into(), "2".into(), "3".into()]],
            },
        };
        let b = TableRun {
            start: 2,
            end: 5,
            body_start: 2,
            block_indices: vec![],
            bbox: None,
            block: Block::Table {
                header: None,
                rows: vec![
                    vec!["x".into(), "y".into(), "z".into()],
                    vec!["p".into(), "q".into(), "r".into()],
                    vec!["m".into(), "n".into(), "o".into()],
                ],
            },
        };
        let merged = merge_consecutive_table_runs(vec![a, b], &lines);
        assert_eq!(merged.len(), 1, "expected single merged run");
        match &merged[0].block {
            Block::Table { header, rows } => {
                assert_eq!(header.as_deref().map(|h| h.len()), Some(3));
                assert_eq!(rows.len(), 4);
                assert_eq!(rows[0], vec!["1", "2", "3"]);
                assert_eq!(rows[3], vec!["m", "n", "o"]);
            }
            other => panic!("expected Block::Table, got {other:?}"),
        }
    }

    #[test]
    fn merge_subset_columns_folds_into_header() {
        // A is a 2-row 3-col "header" whose tracks land on columns 2..4 of
        // B's 4-col body (i.e. the row-label column is missing in A). After
        // merge: one 4-col table whose header has empty col 0 and B's body
        // rows.
        let lines = vec![
            three_col_subset_line("2011", 10.0),
            three_col_subset_line("(pct)", 25.0),
            four_col_line("body", 40.0),
            four_col_line("body", 55.0),
            four_col_line("body", 70.0),
        ];
        let a = TableRun {
            start: 0,
            end: 2,
            body_start: 0,
            block_indices: vec![],
            bbox: None,
            block: Block::Table {
                header: None,
                rows: vec![
                    vec!["2011".into(), "2010".into(), "Avg".into()],
                    vec!["(pct)".into(), "(pct)".into(), "(pct)".into()],
                ],
            },
        };
        let b = TableRun {
            start: 2,
            end: 5,
            body_start: 2,
            block_indices: vec![],
            bbox: None,
            block: Block::Table {
                header: None,
                rows: vec![
                    vec!["Q3".into(), "10".into(), "20".into(), "30".into()],
                    vec!["Q4".into(), "11".into(), "21".into(), "31".into()],
                    vec!["YR".into(), "12".into(), "22".into(), "32".into()],
                ],
            },
        };
        let merged = merge_consecutive_table_runs(vec![a, b], &lines);
        assert_eq!(merged.len(), 1);
        match &merged[0].block {
            Block::Table { header, rows } => {
                let h = header.as_deref().expect("expected header");
                assert_eq!(h.len(), 4);
                assert_eq!(h[0], "");
                // Adjacent identical pieces are deduped per column.
                assert_eq!(h[1], "2011 (pct)");
                assert_eq!(h[2], "2010 (pct)");
                assert_eq!(h[3], "Avg (pct)");
                assert_eq!(rows.len(), 3);
                assert_eq!(rows[0], vec!["Q3", "10", "20", "30"]);
            }
            other => panic!("expected Block::Table, got {other:?}"),
        }
    }

    #[test]
    fn merge_skips_distant_runs() {
        // Same shape as the same-column test but B is far below A.
        let lines = vec![
            three_col_line("h1", 10.0),
            three_col_line("h2", 25.0),
            three_col_line("b1", 200.0), // ~16× line height below
            three_col_line("b2", 215.0),
        ];
        let a = TableRun {
            start: 0,
            end: 2,
            body_start: 0,
            block_indices: vec![],
            bbox: None,
            block: Block::Table {
                header: Some(vec!["A".into(), "B".into(), "C".into()]),
                rows: vec![vec!["1".into(), "2".into(), "3".into()]],
            },
        };
        let b = TableRun {
            start: 2,
            end: 4,
            body_start: 2,
            block_indices: vec![],
            bbox: None,
            block: Block::Table {
                header: None,
                rows: vec![
                    vec!["x".into(), "y".into(), "z".into()],
                    vec!["p".into(), "q".into(), "r".into()],
                ],
            },
        };
        let merged = merge_consecutive_table_runs(vec![a, b], &lines);
        assert_eq!(merged.len(), 2, "distant runs should not merge");
    }

    #[test]
    fn merge_skips_large_prior_run() {
        // A has 5 body rows — large enough that it's a real standalone
        // table, not a header to fold into B.
        let lines: Vec<ProjectedLine> = (0..10)
            .map(|i| three_col_subset_line("x", 10.0 + i as f32 * 15.0))
            .chain((0..3).map(|i| four_col_line("y", 160.0 + i as f32 * 15.0)))
            .collect();
        let a = TableRun {
            start: 0,
            end: 10,
            body_start: 0,
            block_indices: vec![],
            bbox: None,
            block: Block::Table {
                header: None,
                rows: (0..10)
                    .map(|_| vec!["a".into(), "b".into(), "c".into()])
                    .collect(),
            },
        };
        let b = TableRun {
            start: 10,
            end: 13,
            body_start: 10,
            block_indices: vec![],
            bbox: None,
            block: Block::Table {
                header: None,
                rows: (0..3)
                    .map(|_| vec!["1".into(), "2".into(), "3".into(), "4".into()])
                    .collect(),
            },
        };
        let merged = merge_consecutive_table_runs(vec![a, b], &lines);
        assert_eq!(merged.len(), 2, "large prior run should not be absorbed");
    }

    #[test]
    fn merge_skips_two_col_diff() {
        // A is 3-col, B is 5-col — too large a column-count delta to be a
        // header-vs-body relationship.
        let lines = vec![
            three_col_subset_line("x", 10.0),
            three_col_subset_line("y", 25.0),
            line_with_spans(
                &[
                    ("a", 50.0),
                    ("b", 150.0),
                    ("c", 250.0),
                    ("d", 350.0),
                    ("e", 450.0),
                ],
                40.0,
                10.0,
            ),
            line_with_spans(
                &[
                    ("a", 50.0),
                    ("b", 150.0),
                    ("c", 250.0),
                    ("d", 350.0),
                    ("e", 450.0),
                ],
                55.0,
                10.0,
            ),
        ];
        let a = TableRun {
            start: 0,
            end: 2,
            body_start: 0,
            block_indices: vec![],
            bbox: None,
            block: Block::Table {
                header: None,
                rows: vec![
                    vec!["x".into(), "y".into(), "z".into()],
                    vec!["x".into(), "y".into(), "z".into()],
                ],
            },
        };
        let b = TableRun {
            start: 2,
            end: 4,
            body_start: 2,
            block_indices: vec![],
            bbox: None,
            block: Block::Table {
                header: None,
                rows: vec![
                    vec!["1".into(), "2".into(), "3".into(), "4".into(), "5".into()],
                    vec!["1".into(), "2".into(), "3".into(), "4".into(), "5".into()],
                ],
            },
        };
        let merged = merge_consecutive_table_runs(vec![a, b], &lines);
        assert_eq!(merged.len(), 2, "2-col difference should not merge");
    }

    #[test]
    fn merge_grid_fallback_left_alone() {
        let lines = vec![
            three_col_line("a", 10.0),
            three_col_line("b", 25.0),
            three_col_line("c", 40.0),
        ];
        let a = TableRun {
            start: 0,
            end: 2,
            body_start: 0,
            block_indices: vec![],
            bbox: None,
            block: Block::Table {
                header: None,
                rows: vec![
                    vec!["a".into(), "b".into(), "c".into()],
                    vec!["a".into(), "b".into(), "c".into()],
                ],
            },
        };
        let b = TableRun {
            start: 2,
            end: 3,
            body_start: 2,
            block_indices: vec![],
            bbox: None,
            block: Block::GridFallback {
                lines: vec!["fallback".into()],
            },
        };
        let merged = merge_consecutive_table_runs(vec![a, b], &lines);
        assert_eq!(merged.len(), 2, "grid fallback should not be merged");
    }

    #[test]
    fn merge_rejects_long_prose_interstitial() {
        // A multi-cell or long prose line between runs must not be silently
        // dropped by a merge.
        let lines = vec![
            three_col_line("h", 10.0),
            three_col_line("h", 25.0),
            line_with_spans(
                &[
                    ("This", 50.0),
                    ("is", 150.0),
                    ("real", 250.0),
                    ("content", 350.0),
                ],
                40.0,
                10.0,
            ),
            three_col_line("b", 55.0),
            three_col_line("b", 70.0),
        ];
        let a = TableRun {
            start: 0,
            end: 2,
            body_start: 0,
            block_indices: vec![],
            bbox: None,
            block: Block::Table {
                header: Some(vec!["A".into(), "B".into(), "C".into()]),
                rows: vec![vec!["1".into(), "2".into(), "3".into()]],
            },
        };
        let b = TableRun {
            start: 3,
            end: 5,
            body_start: 3,
            block_indices: vec![],
            bbox: None,
            block: Block::Table {
                header: None,
                rows: vec![
                    vec!["x".into(), "y".into(), "z".into()],
                    vec!["p".into(), "q".into(), "r".into()],
                ],
            },
        };
        let merged = merge_consecutive_table_runs(vec![a, b], &lines);
        assert_eq!(merged.len(), 2, "multi-cell interstitial should not merge");
    }

    #[test]
    fn merge_absorbs_single_cell_interstitial_as_body_row() {
        // Apple-earnings / NASS shape: 4-col header rows + a single-cell
        // category divider ("Topsoil") + 5-col body. Divider must be
        // preserved as a body row in the merged table.
        let lines = vec![
            three_col_subset_line("h", 10.0),
            three_col_subset_line("h", 25.0),
            line_with_spans(&[("Topsoil", 50.0)], 40.0, 10.0),
            four_col_line("body", 55.0),
            four_col_line("body", 70.0),
            four_col_line("body", 85.0),
        ];
        let a = TableRun {
            start: 0,
            end: 2,
            body_start: 0,
            block_indices: vec![],
            bbox: None,
            block: Block::Table {
                header: None,
                rows: vec![
                    vec!["2011".into(), "2010".into(), "Avg".into()],
                    vec!["(pct)".into(), "(pct)".into(), "(pct)".into()],
                ],
            },
        };
        let b = TableRun {
            start: 3,
            end: 6,
            body_start: 3,
            block_indices: vec![],
            bbox: None,
            block: Block::Table {
                header: None,
                rows: vec![
                    vec!["Q3".into(), "10".into(), "20".into(), "30".into()],
                    vec!["Q4".into(), "11".into(), "21".into(), "31".into()],
                    vec!["YR".into(), "12".into(), "22".into(), "32".into()],
                ],
            },
        };
        let merged = merge_consecutive_table_runs(vec![a, b], &lines);
        assert_eq!(merged.len(), 1);
        match &merged[0].block {
            Block::Table { header, rows } => {
                assert!(header.is_some());
                assert_eq!(rows.len(), 4, "interstitial + 3 body rows");
                assert_eq!(rows[0][0], "Topsoil");
                assert_eq!(rows[1], vec!["Q3", "10", "20", "30"]);
            }
            other => panic!("expected Block::Table, got {other:?}"),
        }
    }

    #[test]
    fn is_bullet_only_recognizes_lettered_markers() {
        // Digit and roman markers were already handled; lettered ordered-list
        // markers must be too, so a nested a./b./c. list is not mistaken for a
        // description-list table label.
        for m in ["a.", "b.", "c.", "z.", "A.", "a)", "B)", "(a)", "(B)"] {
            assert!(is_bullet_only(m), "{m:?} should be a bullet/list marker");
            assert!(
                !is_label_like(m),
                "{m:?} should not be treated as a table label"
            );
        }
        // Genuine short labels stay labels.
        for l in ["Name:", "Term", "Rate", "Fee.", "AB.", "ab."] {
            assert!(!is_bullet_only(l), "{l:?} should not be a bullet marker");
        }
    }

    #[test]
    fn lettered_list_is_not_a_description_table() {
        // Two-level lettered list: short marker column (col 0) beside wrapped
        // body text (col 1). Regression for the nested-list → pipe-table bug.
        let lines = vec![
            line_with_spans(
                &[
                    ("a.", 80.0),
                    ("The Committee may retain outside advisors.", 120.0),
                ],
                100.0,
                10.0,
            ),
            line_with_spans(
                &[
                    ("b.", 80.0),
                    ("The Committee shall have sole authority.", 120.0),
                ],
                88.0,
                10.0,
            ),
            line_with_spans(
                &[
                    ("c.", 80.0),
                    ("In retaining advice the Committee considers:", 120.0),
                ],
                76.0,
                10.0,
            ),
        ];
        assert!(
            try_detect_description_list(&lines, 0).is_none(),
            "lettered list must not be detected as a description-list table"
        );
    }
}
