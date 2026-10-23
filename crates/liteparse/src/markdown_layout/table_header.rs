//! Header lookback for ruled tables.
//!
//! A ruled grid often stops at the body: the header rows above it are plain
//! text with no rules around them (a multi-level header over a bordered body).
//! The header text is then emitted as loose headings and the table starts at
//! its first data row, so the columns have no names.
//!
//! [`ruled_header_lookback`] walks upward from the grid, one text line at a
//! time, and decides which body columns each header text spans:
//!
//! * a line with several cells: each cell owns the columns whose centre falls
//!   inside the cell's extent widened by half the gap to its neighbours, so a
//!   group label centred over three columns claims all three;
//! * a lone cell that is not a banner: the columns it overlaps.
//!
//! It stops at anything that is not header text: a title centred over the whole
//! table (a banner), a line wider than the table, long prose, or a gap larger
//! than a line. Every decision is appended to `log` so a wrong absorption can
//! be traced instead of guessed at.

const MAX_HEADER_LINES: usize = 4;
const MAX_CELL_CHARS: usize = 40;
const TOL: f32 = 2.0;

/// How many lines above the grid the caller should offer, nearest first.
pub(super) const MAX_LOOKBACK_LINES: usize = MAX_HEADER_LINES;

#[derive(Debug, Clone)]
pub(super) struct HdrCell {
    pub start_x: f32,
    pub end_x: f32,
    pub text: String,
}

#[derive(Debug, Clone)]
pub(super) struct HdrLine {
    pub top: f32,
    pub bottom: f32,
    pub cells: Vec<HdrCell>,
}

#[derive(Debug)]
pub(super) struct Lookback {
    /// One entry per body column; empty when no absorbed line covers it.
    pub header: Vec<String>,
    /// How many of the offered lines were absorbed (nearest first).
    pub lines_used: usize,
}

/// Text that reads as a fragment of a sentence rather than a column label: it
/// starts in lowercase ("were as follows (in millions):" is the tail of a wrapped
/// sentence) or ends in a colon (a lead-in such as "Right-of-use assets:").
fn looks_like_prose(text: &str) -> bool {
    let t = text.trim();
    t.ends_with(':')
        || t.chars()
            .find(|c| c.is_alphabetic())
            .is_some_and(char::is_lowercase)
}

/// Which body columns each cell of a header line spans (`result[i]` for cell `i`).
fn cover_columns(cells: &[HdrCell], bands: &[(f32, f32)], centres: &[f32]) -> Vec<Vec<usize>> {
    let mut out = vec![Vec::new(); cells.len()];
    if cells.len() == 1 {
        let c = &cells[0];
        let w = (c.end_x - c.start_x).max(1.0);
        for (k, b) in bands.iter().enumerate() {
            let overlap = (c.end_x.min(b.1) - c.start_x.max(b.0)).max(0.0);
            if overlap >= 0.5 * w.min(b.1 - b.0) {
                out[0].push(k);
            }
        }
        return out;
    }
    let mut order: Vec<usize> = (0..cells.len()).collect();
    order.sort_by(|&a, &b| cells[a].start_x.total_cmp(&cells[b].start_x));
    for (pos, &ci) in order.iter().enumerate() {
        let c = &cells[ci];
        let gap_right = order
            .get(pos + 1)
            .map(|&n| (cells[n].start_x - c.end_x).max(0.0));
        let gap_left = (pos > 0).then(|| (c.start_x - cells[order[pos - 1]].end_x).max(0.0));
        let right_ext = gap_right.or(gap_left).unwrap_or(0.0) / 2.0;
        let left_ext = gap_left.or(gap_right).unwrap_or(0.0) / 2.0;
        let (lo, hi) = (c.start_x - left_ext, c.end_x + right_ext);
        for (k, &cx) in centres.iter().enumerate() {
            if cx >= lo && cx < hi {
                out[ci].push(k);
            }
        }
    }
    out
}

/// Build a header row for a ruled table from the text lines above its grid.
///
/// `bands` are the body columns' `(left, right)` edges, `grid_top` the top of the
/// drawn grid, and `up` the candidate lines ordered nearest-first. Returns
/// `None` when the nearest line is not a header row.
pub(super) fn ruled_header_lookback(
    bands: &[(f32, f32)],
    grid_top: f32,
    up: &[HdrLine],
    log: &mut Vec<String>,
) -> Option<Lookback> {
    let n = bands.len();
    if n < 2 {
        return None;
    }
    let (tx0, tx1) = (bands[0].0, bands[n - 1].1);
    let tw = tx1 - tx0;
    let avg_w = tw / n as f32;
    let centres: Vec<f32> = bands.iter().map(|b| (b.0 + b.1) * 0.5).collect();
    let mut per_col: Vec<Vec<String>> = vec![Vec::new(); n];
    let mut prev_top = grid_top;
    let mut used = 0usize;

    for (i, line) in up.iter().take(MAX_HEADER_LINES).enumerate() {
        let label = line
            .cells
            .iter()
            .map(|c| c.text.as_str())
            .collect::<Vec<_>>()
            .join(" | ");
        let h = (line.bottom - line.top).max(8.0);
        let gap = prev_top - line.bottom;
        if line.cells.is_empty() {
            log.push(format!("[ruled-header] stop: empty line #{i}"));
            break;
        }
        if gap > 1.5 * h {
            log.push(format!(
                "[ruled-header] stop @{i} \"{label}\": gap {gap:.1} > 1.5 x line height {h:.1}"
            ));
            break;
        }
        if line
            .cells
            .iter()
            .any(|c| c.text.chars().count() > MAX_CELL_CHARS)
        {
            log.push(format!(
                "[ruled-header] stop @{i} \"{label}\": cell longer than {MAX_CELL_CHARS} chars (prose)"
            ));
            break;
        }
        if let Some(c) = line.cells.iter().find(|c| looks_like_prose(&c.text)) {
            log.push(format!(
                "[ruled-header] stop @{i} \"{label}\": \"{}\" reads as a sentence fragment, not a column label",
                c.text
            ));
            break;
        }
        let x_lo = line
            .cells
            .iter()
            .map(|c| c.start_x)
            .fold(f32::INFINITY, f32::min);
        let x_hi = line
            .cells
            .iter()
            .map(|c| c.end_x)
            .fold(f32::NEG_INFINITY, f32::max);
        if x_lo < tx0 - TOL || x_hi > tx1 + TOL {
            log.push(format!(
                "[ruled-header] stop @{i} \"{label}\": x {x_lo:.0}..{x_hi:.0} outside table {tx0:.0}..{tx1:.0}"
            ));
            break;
        }
        if line.cells.len() == 1 {
            let c = &line.cells[0];
            let cx = (c.start_x + c.end_x) * 0.5;
            if (cx - (tx0 + tx1) * 0.5).abs() <= 0.5 * avg_w && (c.end_x - c.start_x) >= 0.3 * tw {
                log.push(format!(
                    "[ruled-header] stop @{i} \"{label}\": banner centred over the whole table (kept as a heading)"
                ));
                break;
            }
        }
        let cover = cover_columns(&line.cells, bands, &centres);
        // Never drop header text: a cell that lands in no column would vanish
        // from the output, so the whole absorption is refused instead.
        if let Some(k) = cover.iter().position(Vec::is_empty) {
            log.push(format!(
                "[ruled-header] reject @{i} \"{label}\": cell \"{}\" maps to no body column, refusing to drop it",
                line.cells[k].text
            ));
            return None;
        }
        if i == 0 {
            let covered: usize = cover.iter().map(Vec::len).sum();
            if line.cells.len() < 2 || covered * 2 < n {
                log.push(format!(
                    "[ruled-header] reject @0 \"{label}\": nearest line is not a header row ({} cells cover {covered}/{n} columns)",
                    line.cells.len()
                ));
                return None;
            }
        }
        for (cell, cols) in line.cells.iter().zip(&cover) {
            for &c in cols {
                per_col[c].push(cell.text.clone());
            }
        }
        log.push(format!(
            "[ruled-header] absorb @{i} \"{label}\" -> columns {:?}",
            cover
        ));
        used += 1;
        prev_top = line.top;
    }
    if used == 0 {
        return None;
    }
    let header: Vec<String> = per_col
        .into_iter()
        .map(|mut v| {
            v.reverse(); // collected nearest-first; header reads top-to-bottom
            v.join(" ")
        })
        .collect();
    log.push(format!(
        "[ruled-header] header = {header:?} ({used} line(s))"
    ));
    Some(Lookback {
        header,
        lines_used: used,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(s: f32, e: f32, t: &str) -> HdrCell {
        HdrCell {
            start_x: s,
            end_x: e,
            text: t.to_string(),
        }
    }
    fn line(top: f32, cells: Vec<HdrCell>) -> HdrLine {
        HdrLine {
            top,
            bottom: top + 10.0,
            cells,
        }
    }
    /// Seven body columns, 86.4..525.6 (the nested-header table of the hard PDF).
    fn bands() -> Vec<(f32, f32)> {
        (0..7)
            .map(|i| (86.4 + 61.2 * i as f32, 86.4 + 61.2 * (i + 1) as f32))
            .collect()
    }
    fn nested_header_lines() -> Vec<HdrLine> {
        vec![
            line(
                139.3,
                vec![
                    cell(176.8, 201.2, "Shift 1"),
                    cell(238.0, 262.4, "Shift 2"),
                    cell(299.2, 323.6, "Shift 3"),
                    cell(360.4, 384.8, "Shift 1"),
                    cell(421.6, 446.0, "Shift 2"),
                    cell(482.8, 507.2, "Shift 3"),
                ],
            ),
            line(128.3, vec![cell(112.6, 132.2, "Plant")]),
            line(
                117.3,
                vec![
                    cell(206.4, 294.0, "Throughput (units/day)"),
                    cell(404.5, 463.2, "Defect Rate (%)"),
                ],
            ),
            line(
                95.3,
                vec![cell(234.6, 377.3, "Global Manufacturing KPIs - FY2026")],
            ),
        ]
    }

    #[test]
    fn multi_level_header_spans_are_resolved_and_the_title_is_left_out() {
        let mut log = Vec::new();
        let r = ruled_header_lookback(&bands(), 156.0, &nested_header_lines(), &mut log).unwrap();
        assert_eq!(r.lines_used, 3);
        assert_eq!(
            r.header,
            [
                "Plant",
                "Throughput (units/day) Shift 1",
                "Throughput (units/day) Shift 2",
                "Throughput (units/day) Shift 3",
                "Defect Rate (%) Shift 1",
                "Defect Rate (%) Shift 2",
                "Defect Rate (%) Shift 3",
            ]
        );
        assert!(log.iter().any(|l| l.contains("banner")));
    }

    #[test]
    fn caption_directly_above_the_grid_is_not_a_header() {
        let up = vec![line(140.0, vec![cell(90.0, 150.0, "Table 3")])];
        let mut log = Vec::new();
        assert!(ruled_header_lookback(&bands(), 156.0, &up, &mut log).is_none());
        assert!(log[0].contains("not a header row"));
    }

    #[test]
    fn prose_wider_than_the_table_stops_absorption() {
        let mut up = vec![nested_header_lines().remove(0)];
        up.push(line(
            128.0,
            vec![
                cell(60.0, 300.0, "Wider Than The Table"),
                cell(310.0, 560.0, "And So Is This"),
            ],
        ));
        let mut log = Vec::new();
        let r = ruled_header_lookback(&bands(), 156.0, &up, &mut log).unwrap();
        assert_eq!(r.lines_used, 1);
        assert!(log.iter().any(|l| l.contains("outside table")));
    }

    #[test]
    fn a_large_gap_stops_absorption() {
        let mut up = vec![nested_header_lines().remove(0)];
        up.push(line(
            60.0,
            vec![cell(176.8, 201.2, "Far"), cell(238.0, 262.4, "Away")],
        ));
        let mut log = Vec::new();
        let r = ruled_header_lookback(&bands(), 156.0, &up, &mut log).unwrap();
        assert_eq!(r.lines_used, 1);
        assert!(log.iter().any(|l| l.contains("gap")));
    }

    #[test]
    fn a_cell_that_maps_to_no_column_rejects_the_whole_lookback() {
        // Four cells over a three-column grid: the first text lands in no column.
        let three: Vec<(f32, f32)> = vec![(50.0, 250.0), (250.0, 350.0), (350.0, 450.0)];
        let up = vec![line(
            140.0,
            vec![
                cell(50.0, 120.0, "Lease-Related"),
                cell(300.0, 340.0, "Line Items"),
                cell(360.0, 380.0, "2024"),
                cell(420.0, 440.0, "2023"),
            ],
        )];
        let mut log = Vec::new();
        assert!(ruled_header_lookback(&three, 156.0, &up, &mut log).is_none());
        assert!(log.iter().any(|l| l.contains("refusing to drop")));
    }

    #[test]
    fn the_tail_of_a_wrapped_sentence_above_the_header_is_not_absorbed() {
        let mut up = vec![nested_header_lines().remove(0)];
        up.push(line(
            128.3,
            vec![cell(100.0, 180.0, "were as follows (in millions):")],
        ));
        let mut log = Vec::new();
        let r = ruled_header_lookback(&bands(), 156.0, &up, &mut log).unwrap();
        assert_eq!(r.lines_used, 1);
        assert_eq!(r.header[0], "");
        assert!(log.iter().any(|l| l.contains("sentence fragment")));
    }

    #[test]
    fn no_candidate_lines_gives_none() {
        let mut log = Vec::new();
        assert!(ruled_header_lookback(&bands(), 156.0, &[], &mut log).is_none());
    }

    #[test]
    fn a_nearest_line_covering_too_few_columns_is_rejected() {
        let up = vec![line(
            140.0,
            vec![cell(100.0, 130.0, "A"), cell(170.0, 200.0, "B")],
        )];
        let mut log = Vec::new();
        assert!(ruled_header_lookback(&bands(), 156.0, &up, &mut log).is_none());
    }
}
