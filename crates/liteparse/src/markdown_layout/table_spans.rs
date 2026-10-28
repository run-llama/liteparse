//! Merged-cell (rowspan / colspan) resolution for ruled tables.
//!
//! A ruled table drawn with a merged cell has no rule *inside* the merged
//! region: no horizontal rule between the rows a cell spans and no vertical
//! rule between the columns it spans. The merged label is drawn once, wherever
//! the generator aligned it, so after text is binned into the grid the label
//! sits in one cell (or, for a wrapped label, is split across several) and the
//! rest of the region is empty.
//!
//! [`resolve_merged_cells`] reads the missing rules straight off the geometry,
//! groups the cells they join into rectangles ("supercells"), joins the text of
//! each rectangle in reading order and repeats it into every cell of the
//! rectangle. Repeating (rather than blanking) is what row-oriented consumers
//! need: a row cut out of the table on its own still says which group it
//! belongs to.
//!
//! Missing rules alone are weak evidence: a sparsely ruled table (a few
//! underlines under figures) is full of cells with no rule around them. A
//! genuine merged cell is different because it is *enclosed*: the rules on the
//! far sides of the merged region are drawn. Only rectangles that are enclosed
//! along each merged axis are resolved.

use std::collections::BTreeMap;

use super::blocks::Cell;

/// Share of a merged region's edge that must be covered by drawn rules for the
/// edge to count as enclosed. Below 1.0 to absorb the padding inset many
/// generators apply to cell rules.
const ENCLOSURE_MIN_COVERAGE: f32 = 0.85;

/// Share of the table's rows (for a column boundary) or columns (for a row
/// boundary) in which a boundary must actually be drawn before a gap in it can
/// be read as a merged cell. A boundary that is drawn almost nowhere is not a
/// boundary at all - the sparsely ruled tables where every row "interrupts" it.
const MIN_BOUNDARY_SHARE: f32 = 0.15;

/// One drawn rule. For a horizontal rule `pos` is its y and `lo..hi` its x
/// extent; for a vertical rule `pos` is its x and `lo..hi` its y extent.
#[derive(Debug, Clone, Copy)]
pub(super) struct Rule {
    pub pos: f32,
    pub lo: f32,
    pub hi: f32,
}

/// The rules of one ruled table, plus the tolerances for matching them to cell
/// edges (`y_tol` for horizontal rules, `x_tol` for vertical ones).
pub(super) struct Rules<'a> {
    pub horizontal: &'a [Rule],
    pub vertical: &'a [Rule],
    pub y_tol: f32,
    pub x_tol: f32,
}

/// Plain-number copy of a cell's ruled box, so geometry can be read while the
/// cells themselves are mutated.
#[derive(Debug, Clone, Copy)]
struct Box4 {
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
}

impl Box4 {
    fn cx(&self) -> f32 {
        (self.x0 + self.x1) * 0.5
    }
    fn cy(&self) -> f32 {
        (self.y0 + self.y1) * 0.5
    }
}

/// Whether some rule at `pos` (within `tol`) covers the point `at`.
fn drawn_at(rules: &[Rule], pos: f32, at: f32, tol: f32) -> bool {
    rules
        .iter()
        .any(|r| (r.pos - pos).abs() <= tol && r.lo <= at && r.hi >= at)
}

/// Whether some rule of the perpendicular family runs *through* `pos`, i.e. the
/// grid continues on both sides of it. Distinguishes "one cell is merged" from
/// "the grid ended here".
fn grid_continues(perpendicular: &[Rule], pos: f32, tol: f32) -> bool {
    // Some rule reaches just before `pos` and some rule reaches just past it. They
    // may be one long stroke or two cell-length strokes meeting at `pos`.
    let covers = |at: f32| perpendicular.iter().any(|r| r.lo <= at && r.hi >= at);
    covers(pos - tol) && covers(pos + tol)
}

/// Fraction of `lo..hi` covered by rules at `pos` (union, so duplicate strokes
/// count once).
fn edge_coverage(rules: &[Rule], pos: f32, lo: f32, hi: f32, tol: f32) -> f32 {
    let mut spans: Vec<(f32, f32)> = rules
        .iter()
        .filter(|r| (r.pos - pos).abs() <= tol)
        .map(|r| (r.lo.max(lo), r.hi.min(hi)))
        .filter(|(a, b)| b > a)
        .collect();
    spans.sort_by(|a, b| a.0.total_cmp(&b.0));
    let (mut covered, mut end) = (0.0_f32, f32::MIN);
    for (a, b) in spans {
        let a = a.max(end);
        if b > a {
            covered += b - a;
            end = b;
        }
    }
    covered / (hi - lo).max(1.0)
}

/// Union-find over grid cells.
struct Dsu(Vec<usize>);

impl Dsu {
    fn new(n: usize) -> Self {
        Dsu((0..n).collect())
    }
    fn find(&mut self, mut i: usize) -> usize {
        while self.0[i] != i {
            self.0[i] = self.0[self.0[i]];
            i = self.0[i];
        }
        i
    }
    fn union(&mut self, a: usize, b: usize) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            self.0[rb.max(ra)] = rb.min(ra);
        }
    }
}

/// Resolve merged cells: join each enclosed merged rectangle's text in reading
/// order and repeat it into every cell of the rectangle.
///
/// Every cell must carry its ruled box (`Cell::bbox`); a grid with any box
/// missing is left untouched. Row 0 (the header candidate) never takes part -
/// header spans are handled by the header flattening. Returns the number of
/// merged rectangles resolved.
pub(super) fn resolve_merged_cells(
    cells: &mut [Vec<Cell>],
    has_text: &mut [Vec<bool>],
    rules: &Rules,
) -> usize {
    let n_rows = cells.len();
    let n_cols = cells.first().map_or(0, Vec::len);
    if n_rows < 2 || n_cols == 0 || cells.iter().any(|r| r.len() != n_cols) {
        return 0;
    }
    let mut boxes: Vec<Vec<Box4>> = Vec::with_capacity(n_rows);
    for row in cells.iter() {
        let mut out = Vec::with_capacity(n_cols);
        for cell in row {
            let Some(b) = cell.bbox.as_ref() else {
                return 0;
            };
            out.push(Box4 {
                x0: b.x,
                y0: b.y,
                x1: b.x + b.width,
                y1: b.y + b.height,
            });
        }
        boxes.push(out);
    }

    // How much of the table each interior boundary is actually drawn across.
    // `col_share[c]`: fraction of rows whose centre a vertical rule at column
    // boundary `c` covers. `row_share[r]`: fraction of columns whose centre a
    // horizontal rule at row boundary `r` covers.
    let col_share: Vec<f32> = (0..n_cols)
        .map(|c| {
            let drawn = (0..n_rows)
                .filter(|&r| {
                    drawn_at(
                        rules.vertical,
                        boxes[r][c].x0,
                        boxes[r][c].cy(),
                        rules.x_tol,
                    )
                })
                .count();
            drawn as f32 / n_rows as f32
        })
        .collect();
    let row_share: Vec<f32> = (0..n_rows)
        .map(|r| {
            let drawn = (0..n_cols)
                .filter(|&c| {
                    drawn_at(
                        rules.horizontal,
                        boxes[r][c].y0,
                        boxes[r][c].cx(),
                        rules.y_tol,
                    )
                })
                .count();
            drawn as f32 / n_cols as f32
        })
        .collect();
    // Join cells whose shared edge carries no rule.
    let mut dsu = Dsu::new(n_rows * n_cols);
    let id = |r: usize, c: usize| r * n_cols + c;
    for r in 1..n_rows {
        for c in 0..n_cols {
            let b = boxes[r][c];
            // Merged with the cell above: no horizontal rule on its top edge.
            // Row 0 stays out, so a span never reaches into the header.
            if r >= 2
                && row_share[r] >= MIN_BOUNDARY_SHARE
                && !drawn_at(rules.horizontal, b.y0, b.cx(), rules.y_tol)
                && grid_continues(rules.vertical, b.y0, rules.y_tol)
            {
                dsu.union(id(r - 1, c), id(r, c));
            }
            // Merged with the cell to the left: no vertical rule on its left edge.
            if c >= 1
                && col_share[c] >= MIN_BOUNDARY_SHARE
                && !drawn_at(rules.vertical, b.x0, b.cy(), rules.x_tol)
                && grid_continues(rules.horizontal, b.x0, rules.x_tol)
            {
                dsu.union(id(r, c - 1), id(r, c));
            }
        }
    }

    let mut groups: BTreeMap<usize, Vec<(usize, usize)>> = BTreeMap::new();
    for r in 1..n_rows {
        for c in 0..n_cols {
            let root = dsu.find(id(r, c));
            groups.entry(root).or_default().push((r, c));
        }
    }

    let mut resolved = 0;
    for members in groups.values().filter(|m| m.len() > 1) {
        let r0 = members.iter().map(|m| m.0).min().unwrap_or(0);
        let r1 = members.iter().map(|m| m.0).max().unwrap_or(0);
        let c0 = members.iter().map(|m| m.1).min().unwrap_or(0);
        let c1 = members.iter().map(|m| m.1).max().unwrap_or(0);
        // Only true rectangles: an L-shaped group means the rules are noise.
        if members.len() != (r1 - r0 + 1) * (c1 - c0 + 1) {
            continue;
        }
        let (multi_row, multi_col) = (r1 > r0, c1 > c0);
        let (tl, br) = (boxes[r0][c0], boxes[r1][c1]);
        if !enclosed(rules, tl, br, multi_row, multi_col) {
            continue;
        }
        // A merged cell holds one label, drawn once. Independent values in
        // several columns (a table of contents row: title, page number) are not
        // a merged cell however few rules separate them, so when a column span's
        // text sits in more than one column, leave it as read.
        if multi_col
            && (c0..=c1)
                .filter(|&c| (r0..=r1).any(|r| !cells[r][c].text.trim().is_empty()))
                .count()
                > 1
        {
            continue;
        }
        let text = cells_text(cells, r0, r1, c0, c1);
        if text.is_empty() {
            continue;
        }
        for r in r0..=r1 {
            let own = (c0..=c1).any(|c| !cells[r][c].text.trim().is_empty());
            let outside = (0..n_cols)
                .filter(|c| *c < c0 || *c > c1)
                .any(|c| !cells[r][c].text.trim().is_empty());
            // A row with no content of its own and none beside the merged region
            // is a blank spacer, not a member of the group.
            if !own && !outside {
                continue;
            }
            for c in c0..=c1 {
                cells[r][c].text = text.clone();
                has_text[r][c] = true;
            }
        }
        resolved += 1;
    }
    resolved
}

/// Whether the rules on the far sides of the merged rectangle are drawn along
/// each merged axis: top and bottom for a row span, left and right for a column
/// span.
fn enclosed(rules: &Rules, tl: Box4, br: Box4, multi_row: bool, multi_col: bool) -> bool {
    let ok = |cov: f32| cov >= ENCLOSURE_MIN_COVERAGE;
    let rows_ok = !multi_row
        || (ok(edge_coverage(
            rules.horizontal,
            tl.y0,
            tl.x0,
            br.x1,
            rules.y_tol,
        )) && ok(edge_coverage(
            rules.horizontal,
            br.y1,
            tl.x0,
            br.x1,
            rules.y_tol,
        )));
    let cols_ok = !multi_col
        || (ok(edge_coverage(
            rules.vertical,
            tl.x0,
            tl.y0,
            br.y1,
            rules.x_tol,
        )) && ok(edge_coverage(
            rules.vertical,
            br.x1,
            tl.y0,
            br.y1,
            rules.x_tol,
        )));
    rows_ok && cols_ok
}

/// Text of the rectangle in reading order (top-to-bottom, left-to-right).
fn cells_text(cells: &[Vec<Cell>], r0: usize, r1: usize, c0: usize, c1: usize) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for row in &cells[r0..=r1] {
        for cell in &row[c0..=c1] {
            let t = cell.text.trim();
            if !t.is_empty() {
                parts.push(t);
            }
        }
    }
    parts.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Rect;

    const CW: f32 = 10.0; // column width
    const RH: f32 = 10.0; // row height

    /// Grid of `rows x cols` cells, each carrying its ruled box.
    fn grid(texts: &[&[&str]]) -> (Vec<Vec<Cell>>, Vec<Vec<bool>>) {
        let cells: Vec<Vec<Cell>> = texts
            .iter()
            .enumerate()
            .map(|(r, row)| {
                row.iter()
                    .enumerate()
                    .map(|(c, t)| Cell {
                        text: (*t).to_string(),
                        bbox: Some(Rect {
                            x: c as f32 * CW,
                            y: r as f32 * RH,
                            width: CW,
                            height: RH,
                        }),
                    })
                    .collect()
            })
            .collect();
        let has = texts
            .iter()
            .map(|r| r.iter().map(|t| !t.is_empty()).collect())
            .collect();
        (cells, has)
    }

    /// Full grid of rules for `rows x cols`, then drop the ones in `skip_h`
    /// `(row_boundary, col)` and `skip_v` `(col_boundary, row)`. Adjacent
    /// surviving segments are coalesced into one long stroke, as a real drawing
    /// has them.
    fn rules(
        rows: usize,
        cols: usize,
        skip_h: &[(usize, usize)],
        skip_v: &[(usize, usize)],
    ) -> (Vec<Rule>, Vec<Rule>) {
        fn runs(n: usize, step: f32, pos: f32, skip: impl Fn(usize) -> bool) -> Vec<Rule> {
            let mut out = Vec::new();
            let mut start: Option<usize> = None;
            for i in 0..=n {
                let present = i < n && !skip(i);
                match (present, start) {
                    (true, None) => start = Some(i),
                    (false, Some(s)) => {
                        out.push(Rule {
                            pos,
                            lo: s as f32 * step,
                            hi: i as f32 * step,
                        });
                        start = None;
                    }
                    _ => {}
                }
            }
            out
        }
        let mut h = Vec::new();
        for b in 0..=rows {
            h.extend(runs(cols, CW, b as f32 * RH, |c| skip_h.contains(&(b, c))));
        }
        let mut v = Vec::new();
        for b in 0..=cols {
            v.extend(runs(rows, RH, b as f32 * CW, |r| skip_v.contains(&(b, r))));
        }
        (h, v)
    }

    fn run(cells: &mut [Vec<Cell>], has: &mut [Vec<bool>], h: &[Rule], v: &[Rule]) -> usize {
        let rules = Rules {
            horizontal: h,
            vertical: v,
            y_tol: 2.0,
            x_tol: 2.0,
        };
        resolve_merged_cells(cells, has, &rules)
    }

    fn col(cells: &[Vec<Cell>], c: usize) -> Vec<String> {
        cells.iter().map(|r| r[c].text.clone()).collect()
    }

    #[test]
    fn label_column_rowspan_is_repeated_whatever_row_holds_the_label() {
        // Header + one 3-row group in column 0; label sits on the middle row.
        let (mut cells, mut has) = grid(&[
            &["Batch", "Stage"],
            &["", "Prep"],
            &["B1", "Assembly"],
            &["", "QA"],
        ]);
        // Column 0 has no rule between rows 1|2 and 2|3 (boundaries 2 and 3).
        let (h, v) = rules(4, 2, &[(2, 0), (3, 0)], &[]);
        assert_eq!(run(&mut cells, &mut has, &h, &v), 1);
        assert_eq!(col(&cells, 0), ["Batch", "B1", "B1", "B1"]);
        assert_eq!(col(&cells, 1), ["Stage", "Prep", "Assembly", "QA"]);
        assert!(has.iter().all(|r| r[0]));
    }

    #[test]
    fn two_by_two_block_with_a_wrapped_label_is_joined_and_filled() {
        // 4 rows x 3 cols; the block covers rows 1-2, cols 1-2; its two-line
        // label landed in (1,1) and (2,1).
        let (mut cells, mut has) = grid(&[
            &["", "A", "B"],
            &["1", "BLOCK", ""],
            &["2", "B2:C3", ""],
            &["3", "y", "z"],
        ]);
        // Interior of the block: boundary 2 (between rows 1|2) for cols 1,2 and
        // the vertical boundary 2 (between cols 1|2) for rows 1,2.
        let (h, v) = rules(4, 3, &[(2, 1), (2, 2)], &[(2, 1), (2, 2)]);
        assert_eq!(run(&mut cells, &mut has, &h, &v), 1);
        for (r, c) in [(1, 1), (1, 2), (2, 1), (2, 2)] {
            assert_eq!(cells[r][c].text, "BLOCK B2:C3", "cell ({r},{c})");
            assert!(has[r][c]);
        }
        assert_eq!(cells[3][1].text, "y");
        assert_eq!(cells[1][0].text, "1");
    }

    #[test]
    fn column_span_in_a_data_row_is_repeated() {
        let (mut cells, mut has) = grid(&[&["H1", "H2", "H3"], &["Total", "", "5"]]);
        // Row 1: no vertical rule between cols 0|1.
        let (h, v) = rules(2, 3, &[], &[(1, 1)]);
        assert_eq!(run(&mut cells, &mut has, &h, &v), 1);
        assert_eq!(cells[1][0].text, "Total");
        assert_eq!(cells[1][1].text, "Total");
        assert_eq!(cells[1][2].text, "5");
    }

    #[test]
    fn column_span_with_independent_values_in_several_columns_is_left_alone() {
        // A table-of-contents row: no interior rules, three independent values.
        let (mut cells, mut has) = grid(&[&["H1", "H2", "H3"], &["Item 1A.", "Risk Factors", "5"]]);
        let (h, v) = rules(2, 3, &[], &[(1, 1), (2, 1)]);
        assert_eq!(run(&mut cells, &mut has, &h, &v), 0);
        assert_eq!(cells[1][0].text, "Item 1A.");
        assert_eq!(cells[1][1].text, "Risk Factors");
        assert_eq!(cells[1][2].text, "5");
    }

    #[test]
    fn unenclosed_span_is_left_alone() {
        // Same shape as the label-column case, but the rule closing the group at
        // the top (boundary 1) is missing too: the region is not boxed in, so
        // this is a sparsely ruled table, not a merged cell.
        let (mut cells, mut has) = grid(&[
            &["Batch", "Stage"],
            &["", "Prep"],
            &["B1", "Assembly"],
            &["", "QA"],
        ]);
        let (h, v) = rules(4, 2, &[(1, 0), (2, 0), (3, 0)], &[]);
        assert_eq!(run(&mut cells, &mut has, &h, &v), 0);
        assert_eq!(col(&cells, 0), ["Batch", "", "B1", ""]);
    }

    #[test]
    fn header_row_never_joins_a_span() {
        let (mut cells, mut has) = grid(&[&["", "Cost"], &["Cash", "10"], &["Loan", "20"]]);
        // No rule between header and first body row in column 0.
        let (h, v) = rules(3, 2, &[(1, 0)], &[]);
        assert_eq!(run(&mut cells, &mut has, &h, &v), 0);
        assert_eq!(col(&cells, 0), ["", "Cash", "Loan"]);
    }

    #[test]
    fn non_rectangular_group_is_ignored() {
        // (1,0)-(2,0) joined vertically and (2,0)-(2,1) joined horizontally:
        // an L-shape, which is rule noise rather than a merged cell.
        let (mut cells, mut has) = grid(&[&["H", "H"], &["a", "x"], &["b", ""], &["c", "z"]]);
        let (h, v) = rules(4, 2, &[(2, 0)], &[(1, 2)]);
        assert_eq!(run(&mut cells, &mut has, &h, &v), 0);
        assert_eq!(col(&cells, 0), ["H", "a", "b", "c"]);
    }

    #[test]
    fn blank_spacer_row_inside_a_span_is_not_filled() {
        let (mut cells, mut has) =
            grid(&[&["Label", "Val"], &["Group", "1"], &["", ""], &["", "3"]]);
        let (h, v) = rules(4, 2, &[(2, 0), (3, 0)], &[]);
        assert_eq!(run(&mut cells, &mut has, &h, &v), 1);
        assert_eq!(col(&cells, 0), ["Label", "Group", "", "Group"]);
    }

    #[test]
    fn fully_ruled_grid_is_untouched() {
        let (mut cells, mut has) = grid(&[&["a", "b"], &["", "c"], &["d", ""]]);
        let (h, v) = rules(3, 2, &[], &[]);
        assert_eq!(run(&mut cells, &mut has, &h, &v), 0);
        assert_eq!(col(&cells, 0), ["a", "", "d"]);
    }

    #[test]
    fn missing_cell_boxes_skip_the_table() {
        let (mut cells, mut has) = grid(&[&["a", "b"], &["", "c"], &["d", ""]]);
        cells[1][0].bbox = None;
        let (h, v) = rules(3, 2, &[(2, 0)], &[]);
        assert_eq!(run(&mut cells, &mut has, &h, &v), 0);
    }
}
