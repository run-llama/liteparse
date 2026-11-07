//! Repairs over finished tables, run after the borderless and ruled
//! detectors have built them.
//!
//! - Symbol columns: a column that holds only a currency sign, `%` or a lone
//!   paren folds into the adjacent value column, and a trailing currency sign
//!   that the projection glued onto the previous cell (`3,915.6 $`) moves to
//!   the next one.
//! - Header line above: the line directly above a table becomes its header
//!   when its cells sit over the value columns and the stub (first) column is
//!   empty, such as a year row `2025 2024 2023` above a label column. The
//!   track test in `absorb_header_lines` rejects these lines because a
//!   right-aligned value column infers a zero-width track, so placement here
//!   uses the body cells' boxes instead.
//! - Band: the same absorb, where a cell covers several value columns. The
//!   band becomes a `MergedTable` row with `colspan`.

use crate::types::{ProjectedLine, Rect};

use super::blocks::{Block, Cell, SpanCell};
use super::tables::{TableCell, TableRun, split_cells, table_rows_adjacent};

const PREFIX_SYMBOLS: &[&str] = &["$", "€", "£", "¥", "(", "$(", "($"];
const SUFFIX_SYMBOLS: &[&str] = &["%", ")", ")%", "%)"];
const CURRENCY: &[char] = &['$', '€', '£', '¥'];

fn extend_bbox(dst: &mut Option<Rect>, src: &Option<Rect>) {
    if let Some(s) = src {
        Rect::extend(dst, s);
    }
}

fn join_text(a: &str, b: &str) -> String {
    match (a.trim().is_empty(), b.trim().is_empty()) {
        (true, _) => b.trim().to_string(),
        (_, true) => a.trim().to_string(),
        _ => format!("{} {}", a.trim(), b.trim()),
    }
}

/// Repair a region's table runs, sorted by `start`, with indices into `lines`.
/// Each run may take up to two lines directly above it as header lines, but
/// none that belongs to the previous run.
pub(super) fn repair_region_runs(runs: &mut [TableRun], lines: &[ProjectedLine]) {
    let mut prev_end = 0;
    for run in runs.iter_mut() {
        merge_symbol_columns(&mut run.block);
        for _ in 0..2 {
            if run.start == 0 || run.start - 1 < prev_end {
                break;
            }
            let above = run.start - 1;
            if !absorb_header_above(&mut run.block, &lines[above], &lines[run.start]) {
                break;
            }
            Rect::extend(&mut run.bbox, &lines[above].bbox);
            run.start = above;
        }
        prev_end = run.end;
    }
}

/// Repair a run from the page-level ruled pass and return the index of the
/// line it absorbed as a header, if any. `consumed` is the run's own lines and
/// `taken` the lines earlier ruled runs consumed. The candidate is the lowest
/// line above the run that overlaps it horizontally: a year row often sits
/// above the top rule, outside the drawn grid the ruled builder reads.
pub(super) fn repair_global_run(
    run: &mut TableRun,
    consumed: &[usize],
    taken: &std::collections::HashSet<usize>,
    lines: &[ProjectedLine],
) -> Option<usize> {
    merge_symbol_columns(&mut run.block);
    let top = consumed
        .iter()
        .copied()
        .min_by(|&a, &b| lines[a].bbox.y.total_cmp(&lines[b].bbox.y))?;
    let (x0, x1) = run
        .bbox
        .as_ref()
        .map_or((f32::NEG_INFINITY, f32::INFINITY), |b| (b.x, b.x + b.width));
    let top_y = lines[top].bbox.y;
    let above = lines
        .iter()
        .enumerate()
        .filter(|(i, l)| {
            !consumed.contains(i)
                && !taken.contains(i)
                && l.bbox.y < top_y
                && l.bbox.x < x1
                && l.bbox.x + l.bbox.width > x0
        })
        .max_by(|a, b| a.1.bbox.y.total_cmp(&b.1.bbox.y))
        .map(|(i, _)| i)?;
    if !absorb_header_above(&mut run.block, &lines[above], &lines[top]) {
        return None;
    }
    Rect::extend(&mut run.bbox, &lines[above].bbox);
    Some(above)
}

/// Fold symbol-only columns into their numeric neighbour and re-home trailing
/// currency signs. Rectangular `Block::Table` only.
fn merge_symbol_columns(block: &mut Block) {
    let Block::Table { header, rows } = block else {
        return;
    };
    let Some(width) = rows.first().map(|r| r.len()) else {
        return;
    };
    if width < 2
        || rows.iter().any(|r| r.len() != width)
        || header.as_ref().is_some_and(|h| h.len() != width)
    {
        return;
    }

    // `3,915.6 $ | 4,083.6 $ | 4,065.8`: the projection attached each `$` to
    // the cell on its left. Move it to the start of the next cell.
    for row in rows.iter_mut() {
        if row[width - 1].text.trim().ends_with(CURRENCY) {
            continue;
        }
        for k in 0..width - 1 {
            let t = row[k].text.trim_end();
            let Some(last) = t.chars().last() else {
                continue;
            };
            if !CURRENCY.contains(&last) {
                continue;
            }
            let head = t[..t.len() - last.len_utf8()].trim_end();
            let next = row[k + 1].text.trim();
            if head.is_empty()
                || !head
                    .chars()
                    .last()
                    .is_some_and(|c| c.is_ascii_digit() || c == ')')
                || next.is_empty()
                || next.starts_with(CURRENCY)
                || !next
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_digit() || c == '(')
            {
                continue;
            }
            let head = head.to_string();
            row[k + 1].text = format!("{last} {next}");
            row[k].text = head;
        }
    }

    // Column folds, repeated until no symbol-only column is left.
    loop {
        let w = rows[0].len();
        let mut target = None;
        for k in 0..w {
            let filled: Vec<&str> = rows
                .iter()
                .map(|r| r[k].text.trim())
                .filter(|t| !t.is_empty())
                .collect();
            if filled.is_empty() {
                continue;
            }
            let head_ok = header.as_ref().is_none_or(|h| h[k].text.trim().is_empty());
            if !head_ok {
                continue;
            }
            let prefix = filled.iter().all(|t| PREFIX_SYMBOLS.contains(t));
            let suffix = filled.iter().all(|t| SUFFIX_SYMBOLS.contains(t));
            if prefix && k + 1 < w {
                target = Some((k, k + 1, true));
                break;
            }
            if suffix && k > 0 {
                target = Some((k, k - 1, false));
                break;
            }
        }
        let Some((k, into, as_prefix)) = target else {
            break;
        };
        for row in rows.iter_mut() {
            let sym = row[k].clone();
            if !sym.text.trim().is_empty() {
                let dst = &mut row[into];
                // A symbol with no value beside it is dropped with its column.
                if !dst.text.trim().is_empty() {
                    let s = sym.text.trim();
                    let d = dst.text.trim();
                    dst.text = if as_prefix {
                        if s.ends_with('(') {
                            format!("{s}{d}")
                        } else {
                            format!("{s} {d}")
                        }
                    } else {
                        format!("{d}{s}")
                    };
                    extend_bbox(&mut dst.bbox, &sym.bbox);
                }
            }
            row.remove(k);
        }
        if let Some(h) = header.as_mut() {
            let removed = h.remove(k);
            let dst = &mut h[if into > k { into - 1 } else { into }];
            extend_bbox(&mut dst.bbox, &removed.bbox);
        }
        if rows[0].len() < 2 {
            break;
        }
    }
}

/// Column x-extents of a table, from the boxes of its body (and header) cells.
fn column_extents(
    header: &Option<Vec<Cell>>,
    rows: &[Vec<Cell>],
    width: usize,
) -> Vec<Option<(f32, f32)>> {
    let mut out: Vec<Option<(f32, f32)>> = vec![None; width];
    for row in header.iter().chain(rows.iter()) {
        for (k, c) in row.iter().enumerate().take(width) {
            if c.text.trim().is_empty() {
                continue;
            }
            if let Some(b) = &c.bbox {
                let (s, e) = (b.x, b.x + b.width);
                out[k] = Some(match out[k] {
                    Some((a, z)) => (a.min(s), z.max(e)),
                    None => (s, e),
                });
            }
        }
    }
    out
}

fn is_value_like(t: &str) -> bool {
    let t = t.trim();
    !t.is_empty()
        && t.chars().filter(|c| c.is_ascii_digit()).count() * 2
            >= t.chars().filter(|c| !c.is_whitespace()).count()
        && !(t.len() == 4 && t.chars().all(|c| c.is_ascii_digit()))
}

fn header_cell_plausible(text: &str) -> bool {
    text.chars().count() <= 40 && text.split_whitespace().count() <= 6
}

/// Header-line shape: short cells, not a sentence, not a unit note.
fn header_line_plausible(line: &ProjectedLine, cells: &[TableCell]) -> bool {
    let text = line.text.trim();
    if text.is_empty() || text.chars().count() > 160 || text.ends_with(['.', ';', ',']) {
        return false;
    }
    let lower = text.to_lowercase();
    if lower.contains("in millions") || lower.contains("in thousands") || lower.contains("except") {
        return false;
    }
    // Captions and section titles sit right above tables too.
    const TITLE_WORDS: &[&str] = &[
        "table", "tab", "figure", "fig", "exhibit", "schedule", "appendix", "note", "source",
    ];
    let first = lower.split_whitespace().next().unwrap_or("");
    if TITLE_WORDS
        .iter()
        .any(|w| first.trim_end_matches([':', '.']) == *w)
    {
        return false;
    }
    cells.iter().all(|c| header_cell_plausible(&c.text))
}

/// Try to absorb `cand` (the line directly above the table, with `below` the
/// table's top line) as a header row. Returns true when the block changed.
fn absorb_header_above(block: &mut Block, cand: &ProjectedLine, below: &ProjectedLine) -> bool {
    let Block::Table { header, rows } = block else {
        return false;
    };
    let Some(width) = rows.first().map(|r| r.len()) else {
        return false;
    };
    if width < 2 || rows.iter().any(|r| r.len() != width) {
        return false;
    }
    if header.as_ref().is_some_and(|h| h.len() != width) {
        return false;
    }
    if !table_rows_adjacent(cand, below) || cand.bbox.y >= below.bbox.y {
        return false;
    }
    // A larger font than the table's top line is a heading.
    if below.dominant_font_size > 0.0 && cand.dominant_font_size > below.dominant_font_size * 1.1 {
        return false;
    }
    let cells = split_cells(cand);
    if cells.is_empty() || cells.len() > width || !header_line_plausible(cand, &cells) {
        return false;
    }
    let ext = column_extents(header, rows, width);
    let Some(stub) = ext[0] else {
        return false;
    };
    let value_cols: Vec<(usize, (f32, f32))> =
        (1..width).filter_map(|k| ext[k].map(|e| (k, e))).collect();
    if value_cols.is_empty() {
        return false;
    }
    let first_value_start = value_cols[0].1.0;
    let tol = 3.0;

    // Assign each cell to the stub or to a contiguous run of value columns.
    let mut stub_text: Option<&TableCell> = None;
    let mut placed: Vec<(usize, usize, &TableCell)> = Vec::new(); // (first col, last col, cell)
    for (i, c) in cells.iter().enumerate() {
        if c.start_x < first_value_start - tol && c.end_x <= first_value_start + tol {
            // In the stub column. Only a leading cell, and only when it stays
            // left of the first value column.
            if i != 0 {
                return false;
            }
            stub_text = Some(c);
            continue;
        }
        if c.start_x < stub.1 - tol {
            // Straddles the stub and the values: a title, not a header.
            return false;
        }
        let covered: Vec<usize> = value_cols
            .iter()
            .filter(|(_, (s, e))| c.start_x <= e + tol && c.end_x >= s - tol)
            .map(|(k, _)| *k)
            .collect();
        let (lo, hi) = if covered.is_empty() {
            // In a gutter: nearest value column by center, within one gutter.
            let center = (c.start_x + c.end_x) * 0.5;
            let Some((k, d)) = value_cols
                .iter()
                .map(|(k, (s, e))| (*k, (center - (s + e) * 0.5).abs()))
                .min_by(|a, b| a.1.total_cmp(&b.1))
            else {
                return false;
            };
            if d > 40.0 {
                return false;
            }
            (k, k)
        } else {
            (covered[0], *covered.last().unwrap())
        };
        if placed.last().is_some_and(|&(_, h, _)| h >= lo) {
            return false;
        }
        placed.push((lo, hi, c));
    }
    if placed.is_empty() {
        return false;
    }
    let spans = placed.iter().any(|&(lo, hi, _)| hi > lo);
    // A single short label over one value column is a unit, a note or a
    // contact line more often than a header.
    if !spans && placed.len() < 2 {
        return false;
    }
    if spans && header.is_none() && placed.len() < 2 && rows.len() < 3 {
        return false;
    }

    let cell_bbox = |c: &TableCell| Rect {
        x: c.start_x,
        y: cand.bbox.y,
        width: (c.end_x - c.start_x).max(0.0),
        height: cand.bbox.height,
    };

    if !spans {
        // One cell per column: a plain header row, or a second header line
        // merged into the existing one top-to-bottom per column.
        let mut new_row = vec![Cell::default(); width];
        if let Some(s) = stub_text {
            new_row[0] = Cell::located(s.text.trim(), cell_bbox(s));
        }
        for &(k, _, c) in &placed {
            new_row[k] = Cell::located(c.text.trim(), cell_bbox(c));
        }
        match header {
            None => *header = Some(new_row),
            Some(h) => {
                for (k, n) in new_row.into_iter().enumerate() {
                    h[k].text = join_text(&n.text, &h[k].text);
                    extend_bbox(&mut h[k].bbox, &n.bbox);
                }
            }
        }
        return true;
    }

    // Band row with colspans above the existing header rows.
    // The band labels the header row below it: the existing header, or the
    // first body row when it holds no values. That row must read as a header,
    // with cells as short as the absorbed line's.
    let promote = header.is_none()
        && rows.len() >= 3
        && rows[0].iter().skip(1).all(|c| !is_value_like(&c.text));
    let lower = if promote {
        Some(&rows[0])
    } else {
        header.as_ref()
    };
    if lower.is_some_and(|h| !h.iter().all(|c| header_cell_plausible(&c.text))) {
        return false;
    }
    let lower_header = if promote {
        Some(rows.remove(0))
    } else {
        header.take()
    };
    let mut band: Vec<SpanCell> = Vec::new();
    let mut stub_cell = SpanCell::new(stub_text.map_or("", |s| s.text.trim()));
    if let Some(s) = stub_text {
        stub_cell = stub_cell.with_bbox(cell_bbox(s));
    }
    band.push(stub_cell);
    let mut k = 1;
    let mut pi = 0;
    while k < width {
        if pi < placed.len() && placed[pi].0 == k {
            let (lo, hi, c) = placed[pi];
            band.push(
                SpanCell::spanning(c.text.trim(), (hi - lo + 1) as u16, 1).with_bbox(cell_bbox(c)),
            );
            k = hi + 1;
            pi += 1;
        } else {
            band.push(SpanCell::new(""));
            k += 1;
        }
    }
    let to_span = |r: &[Cell]| -> Vec<SpanCell> {
        r.iter()
            .map(|c| {
                let s = SpanCell::new(c.text.clone());
                match &c.bbox {
                    Some(b) => s.with_bbox(b.clone()),
                    None => s,
                }
            })
            .collect()
    };
    let mut out: Vec<Vec<SpanCell>> = vec![band];
    let mut header_rows = 1;
    if let Some(h) = &lower_header {
        out.push(to_span(h));
        header_rows = 2;
    }
    out.extend(rows.iter().map(|r| to_span(r)));
    *block = Block::MergedTable {
        rows: out,
        header_rows,
    };
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::markdown_layout::test_helpers::{line_with_spans, page};
    use crate::markdown_layout::{
        classify_page_with_filters, render_blocks, split_repeated_header_tables,
    };

    fn render(lines: Vec<ProjectedLine>) -> String {
        let p = page(lines);
        let none = std::collections::HashSet::new();
        let blocks = classify_page_with_filters(
            &p,
            &[],
            &none,
            &[],
            crate::config::ImageMode::Off,
            &std::collections::HashSet::new(),
        );
        render_blocks(&blocks)
    }

    /// Date row wider than the right-aligned values below it, over an empty
    /// stub. The track test in `absorb_header_lines` rejects it.
    fn date_row_page() -> Vec<ProjectedLine> {
        vec![
            line_with_spans(
                &[("February 1, 2025", 270.0), ("February 3, 2024", 390.0)],
                100.0,
                10.0,
            ),
            line_with_spans(
                &[("Beauty", 50.0), ("324", 330.0), ("307", 450.0)],
                115.0,
                10.0,
            ),
            line_with_spans(
                &[("Full Assortment", 50.0), ("181", 330.0), ("156", 450.0)],
                130.0,
                10.0,
            ),
            line_with_spans(
                &[("Total", 50.0), ("505", 330.0), ("463", 450.0)],
                145.0,
                10.0,
            ),
        ]
    }

    #[test]
    fn date_row_over_empty_stub_becomes_header() {
        let on = render(date_row_page());
        assert!(
            on.starts_with(
                "|  | February 1, 2025 | February 3, 2024 |\n|---|---|---|\n| Beauty | 324 | 307 |"
            ),
            "{on}"
        );
    }

    #[test]
    fn caption_above_table_is_not_absorbed() {
        let mut lines = date_row_page();
        lines[0] = line_with_spans(
            &[("Table 1.", 270.0), ("Suggested values", 390.0)],
            100.0,
            10.0,
        );
        let on = render(lines);
        assert!(!on.contains("| Table 1."), "{on}");
        assert!(on.contains("Table 1."), "{on}");
    }

    fn spanning_band_page() -> Vec<ProjectedLine> {
        vec![
            line_with_spans(
                &[("Non-Tobacco rates", 170.0), ("Tobacco rates band", 375.0)],
                100.0,
                10.0,
            ),
            line_with_spans(
                &[
                    ("Age", 50.0),
                    ("Female", 150.0),
                    ("Male", 250.0),
                    ("Female", 350.0),
                    ("Male", 450.0),
                ],
                115.0,
                10.0,
            ),
            line_with_spans(
                &[
                    ("65", 50.0),
                    ("1,205", 150.0),
                    ("1,387", 250.0),
                    ("1,338", 350.0),
                    ("1,541", 450.0),
                ],
                130.0,
                10.0,
            ),
            line_with_spans(
                &[
                    ("66", 50.0),
                    ("1,259", 150.0),
                    ("1,449", 250.0),
                    ("1,400", 350.0),
                    ("1,608", 450.0),
                ],
                145.0,
                10.0,
            ),
            line_with_spans(
                &[
                    ("67", 50.0),
                    ("1,301", 150.0),
                    ("1,490", 250.0),
                    ("1,452", 350.0),
                    ("1,660", 450.0),
                ],
                160.0,
                10.0,
            ),
        ]
    }

    #[test]
    fn spanning_band_renders_colspan() {
        let on = render(spanning_band_page());
        assert!(
            on.contains("<th colspan=\"2\">Non-Tobacco rates</th>"),
            "{on}"
        );
        assert!(
            on.contains("<th colspan=\"2\">Tobacco rates band</th>"),
            "{on}"
        );
        assert!(on.contains("<th>Age</th>"), "{on}");
        assert!(!on.contains("rowspan"), "{on}");
    }

    #[test]
    fn abbreviated_caption_above_band_table_is_not_absorbed() {
        for caption in ["Fig. 1: Quarterly results", "Tab. 1: Quarterly results"] {
            let mut lines = spanning_band_page();
            lines[0] = line_with_spans(&[(caption, 170.0)], 100.0, 10.0);
            let on = render(lines);
            assert!(!on.contains("colspan"), "{on}");
            assert!(on.contains(caption), "{on}");
        }
    }

    #[test]
    fn spanning_band_with_repeated_lower_header_splits() {
        let mut lines = spanning_band_page();
        for mut line in spanning_band_page().into_iter().skip(1) {
            line.bbox.y += 60.0;
            for span in &mut line.spans {
                span.y += 60.0;
            }
            lines.push(line);
        }
        let p = page(lines);
        let none = std::collections::HashSet::new();
        let blocks = classify_page_with_filters(
            &p,
            &[],
            &none,
            &[],
            crate::config::ImageMode::Off,
            &std::collections::HashSet::new(),
        );
        assert_eq!(
            blocks
                .iter()
                .filter(|b| matches!(b.block, Block::MergedTable { .. }))
                .count(),
            1
        );
        let blocks = split_repeated_header_tables(blocks);
        assert_eq!(
            blocks
                .iter()
                .filter(|b| matches!(b.block, Block::Table { .. } | Block::MergedTable { .. }))
                .count(),
            2
        );
    }

    #[test]
    fn spanning_band_over_long_prose_is_not_absorbed() {
        let mut b = table(&[
            &[
                "This is a long prose stub cell",
                "Here are more than six short words",
                "Yet more than six short words here",
            ],
            &["First", "100", "200"],
            &["Second", "300", "400"],
        ]);
        let Block::Table { rows, .. } = &mut b else {
            panic!()
        };
        for (i, row) in rows.iter_mut().enumerate() {
            for (k, cell) in row.iter_mut().enumerate() {
                cell.bbox = Some(Rect {
                    x: 50.0 + k as f32 * 100.0,
                    y: 115.0 + i as f32 * 15.0,
                    width: 30.0,
                    height: 10.0,
                });
            }
        }
        let cand = line_with_spans(&[("Combined rates band", 170.0)], 100.0, 10.0);
        let below = line_with_spans(&[("First row", 50.0)], 115.0, 10.0);
        assert!(!absorb_header_above(&mut b, &cand, &below));
    }

    fn table(rows: &[&[&str]]) -> Block {
        Block::Table {
            header: None,
            rows: rows
                .iter()
                .map(|r| r.iter().map(|t| Cell::from(*t)).collect())
                .collect(),
        }
    }

    fn texts(b: &Block) -> Vec<Vec<String>> {
        let Block::Table { rows, .. } = b else {
            panic!()
        };
        rows.iter()
            .map(|r| r.iter().map(|c| c.text.clone()).collect())
            .collect()
    }

    #[test]
    fn currency_and_percent_columns_fold_into_values() {
        let mut b = table(&[
            &["Fees", "$", "3,915.6", "(4", ")%"],
            &["Other", "", "131.9", "7", "%"],
        ]);
        merge_symbol_columns(&mut b);
        assert_eq!(
            texts(&b),
            vec![
                vec!["Fees", "$ 3,915.6", "(4)%"],
                vec!["Other", "131.9", "7%"]
            ]
        );
    }

    #[test]
    fn trailing_currency_moves_to_next_cell() {
        let mut b = table(&[
            &["Fees", "$ 3,915.6 $", "4,083.6 $", "4,065.8"],
            &["Other", "131.9", "137.3", "128.2"],
        ]);
        merge_symbol_columns(&mut b);
        assert_eq!(
            texts(&b)[0],
            vec!["Fees", "$ 3,915.6", "$ 4,083.6", "$ 4,065.8"]
        );
    }

    /// The existing header is held to the same limit, and a rejected band
    /// leaves it in place.
    #[test]
    fn spanning_band_over_prose_header_is_not_absorbed() {
        let located = |rows: &[&[&str]], y0: f32| -> Vec<Vec<Cell>> {
            rows.iter()
                .enumerate()
                .map(|(i, r)| {
                    r.iter()
                        .enumerate()
                        .map(|(k, t)| {
                            let rect = Rect {
                                x: 50.0 + k as f32 * 100.0,
                                y: y0 + i as f32 * 15.0,
                                width: 30.0,
                                height: 10.0,
                            };
                            Cell::located(*t, rect)
                        })
                        .collect()
                })
                .collect()
        };
        let header = located(
            &[&["Stub", "Here are more than six short words", "Two words"]],
            115.0,
        );
        let mut b = Block::Table {
            header: header.into_iter().next(),
            rows: located(
                &[&["First", "100", "200"], &["Second", "300", "400"]],
                130.0,
            ),
        };
        let cand = line_with_spans(&[("Combined rates band", 170.0)], 100.0, 10.0);
        let below = line_with_spans(&[("Stub", 50.0)], 115.0, 10.0);
        assert!(!absorb_header_above(&mut b, &cand, &below));
        let Block::Table { header, .. } = &b else {
            panic!()
        };
        assert_eq!(
            header.as_ref().map(|h| h[1].text.as_str()),
            Some("Here are more than six short words")
        );
    }

    #[test]
    fn suffix_currency_stays_in_its_cell() {
        let mut b = table(&[&["Fee", "100 €", "200 £"], &["Other", "5 €", "6 £"]]);
        let before = texts(&b);
        merge_symbol_columns(&mut b);
        assert_eq!(texts(&b), before);
    }

    /// A band turns the run into a `MergedTable`. A line inside its span that
    /// no cell took must still come back for the classifier to emit.
    #[test]
    fn band_table_hands_back_untaken_lines() {
        let lines = vec![
            line_with_spans(&[("Rates", 150.0)], 100.0, 10.0),
            line_with_spans(&[("Age", 50.0), ("Female", 150.0)], 115.0, 10.0),
            line_with_spans(&[("see footnote", 50.0)], 130.0, 10.0),
        ];
        let run = TableRun {
            start: 0,
            end: 3,
            body_start: 1,
            block: Block::MergedTable {
                rows: vec![
                    vec![SpanCell::new(""), SpanCell::spanning("Rates", 2, 1)],
                    vec![SpanCell::new("Age"), SpanCell::new("Female")],
                ],
                header_rows: 1,
            },
            interstitials: Vec::new(),
            bbox: None,
        };
        assert_eq!(super::super::tables::lines_left_out(&run, &lines), vec![2]);
    }

    #[test]
    fn single_cell_line_over_two_column_table_is_not_a_header() {
        let mut b = Block::Table {
            header: None,
            rows: vec![
                vec![
                    Cell::located(
                        "CW6 4TR",
                        Rect {
                            x: 50.0,
                            y: 115.0,
                            width: 35.0,
                            height: 10.0,
                        },
                    ),
                    Cell::located(
                        "Payment queries - email:",
                        Rect {
                            x: 300.0,
                            y: 115.0,
                            width: 120.0,
                            height: 10.0,
                        },
                    ),
                ],
                vec![
                    Cell::located(
                        "United Kingdom",
                        Rect {
                            x: 50.0,
                            y: 130.0,
                            width: 70.0,
                            height: 10.0,
                        },
                    ),
                    Cell::located(
                        "accounts@example.com",
                        Rect {
                            x: 300.0,
                            y: 130.0,
                            width: 100.0,
                            height: 10.0,
                        },
                    ),
                ],
            ],
        };
        let cand = line_with_spans(&[("Tel: +44 000 0000000", 300.0)], 100.0, 10.0);
        let below = line_with_spans(
            &[("CW6 4TR", 50.0), ("Payment queries - email:", 300.0)],
            115.0,
            10.0,
        );
        assert!(!absorb_header_above(&mut b, &cand, &below));
    }
}
