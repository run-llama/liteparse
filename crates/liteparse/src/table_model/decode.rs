//! Decoding TableNet logits into tables (a port of `bench/tables/tn_core.py`).
//!
//! `decode_tables`: unit groups from in_table + same_table (average linkage), then absorption of
//! stacked pieces, small groups and stray units inside a table's box, then growth over a header
//! band. `decode_grid`: one table's cells (same_cell, with stacked row-and-column pairs forced
//! together), rows and columns over cells (span-aware linkage), header rows, the near-empty column
//! fold, and the grid as HTML. Order-sensitive steps (argmax ties, stable sorts, insertion order)
//! follow the Python exactly, so parity fixtures compare byte for byte.

use super::net::Logits;

/// A text unit as the decoder sees it: box in the text's reading frame (points, top-left origin).
#[derive(Clone, Debug)]
pub struct DUnit {
    pub bx: [f64; 4],
    pub text: String,
    pub h: f64,
}

pub const CELL_MARGIN: f32 = 1.5;
pub const STACK_GAP: f64 = 0.8;
pub const STACK_GAP_HDR: f64 = 0.8;
pub const GATE: f64 = 3.0;

/// Decoder switches (options of the reference decoder; these are the shipped defaults).
#[derive(Clone, Debug)]
pub struct DecodeOptions {
    pub table_thresh: f32,
    pub head_band: bool,
    pub forced_link: bool,
    pub squeeze: bool,
    /// Fold wrapped-cell continuation rows into the row above (Rust-only; not in tn_core).
    pub fold_wraps: bool,
}

impl Default for DecodeOptions {
    fn default() -> Self {
        DecodeOptions {
            table_thresh: 0.0,
            head_band: true,
            forced_link: true,
            squeeze: true,
            fold_wraps: true,
        }
    }
}

fn median(v: &mut Vec<f64>) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    }
}

/// Median unit height, or 1 when it is 0 (`float(np.median(...)) or 1.0`).
fn median_h(units: &[DUnit], idx: impl Iterator<Item = usize>) -> f64 {
    let mut hs: Vec<f64> = idx.map(|i| units[i].h).collect();
    let m = median(&mut hs);
    if m == 0.0 || m.is_nan() { 1.0 } else { m }
}

/// Average-linkage clusters of a symmetric n x n matrix: merge while the mean logit > thresh.
/// Groups come back sorted, in the order of their surviving representative.
///
/// Each merge joins the pair with the highest mean (ties: the first in row-major order of the live
/// groups) and adds the absorbed group's sums into the survivor's row and column. Every live group
/// caches its best partner, so a merge rescans only the rows whose cached partner it touched:
/// O(n^2) on typical pages instead of rescanning every pair per merge (O(n^3); 2 s at 1,000 units).
pub fn avg_link(p: &[f64], n: usize, thresh: f64) -> Vec<Vec<usize>> {
    let mut groups: Vec<Vec<usize>> = (0..n).map(|i| vec![i]).collect();
    let mut s = p.to_vec();
    let mut cnt = vec![1f64; n * n];
    let mut alive: Vec<usize> = (0..n).collect();
    // best partner of each live group g: (mean, partner), the first maximum in `alive` order
    let row_best = |s: &[f64], cnt: &[f64], alive: &[usize], g: usize| -> (f64, usize) {
        let mut best = (f64::NEG_INFINITY, usize::MAX);
        for &h in alive {
            if h != g {
                let v = s[g * n + h] / cnt[g * n + h];
                if v > best.0 {
                    best = (v, h);
                }
            }
        }
        best
    };
    let mut best: Vec<(f64, usize)> = (0..n).map(|g| row_best(&s, &cnt, &alive, g)).collect();
    while alive.len() > 1 {
        // the first live group (in `alive` order) holding the overall maximum
        let mut ba = 0;
        for a in 1..alive.len() {
            if best[alive[a]].0 > best[alive[ba]].0 {
                ba = a;
            }
        }
        let (v, gb) = best[alive[ba]];
        if v <= thresh {
            break;
        }
        let ga = alive[ba];
        let bb = alive
            .iter()
            .position(|&g| g == gb)
            .expect("cached partner is live");
        let moved = std::mem::take(&mut groups[gb]);
        groups[ga].extend(moved);
        for k in 0..n {
            s[ga * n + k] += s[gb * n + k];
            cnt[ga * n + k] += cnt[gb * n + k];
        }
        for k in 0..n {
            s[k * n + ga] += s[k * n + gb];
            cnt[k * n + ga] += cnt[k * n + gb];
        }
        alive.remove(bb);
        // Only row ga and column ga changed (and gb is gone): a row whose partner was ga or gb is
        // rescanned; any other row compares its old best with its new entry for ga.
        best[ga] = row_best(&s, &cnt, &alive, ga);
        for &r in &alive {
            if r == ga {
                continue;
            }
            let (bv, bc) = best[r];
            if bc == ga || bc == gb {
                best[r] = row_best(&s, &cnt, &alive, r);
            } else {
                let c = s[r * n + ga] / cnt[r * n + ga];
                if c > bv || (c == bv && ga < bc) {
                    best[r] = (c, ga);
                }
            }
        }
    }
    alive
        .iter()
        .map(|&g| {
            let mut v = groups[g].clone();
            v.sort_unstable();
            v
        })
        .collect()
}

/// Units positive with two groups that are negative with each other (a spanning header).
fn bridges(p: &[f64], n: usize) -> Vec<usize> {
    let (neg, frac) = (-1.0, 0.3);
    let mut out = Vec::new();
    for i in 0..n {
        let nb: Vec<usize> = (0..n).filter(|&j| j != i && p[i * n + j] > 0.0).collect();
        if nb.len() < 2 {
            continue;
        }
        let (mut tot, mut negc) = (0usize, 0usize);
        for a in 0..nb.len() {
            for b in a + 1..nb.len() {
                tot += 1;
                if p[nb[a] * n + nb[b]] < neg {
                    negc += 1;
                }
            }
        }
        if tot > 0 && (negc as f64 / tot as f64) > frac {
            out.push(i);
        }
    }
    out
}

/// Rows or columns: linkage over the non-bridging units; each bridging unit then joins every
/// group it is positive with on average (a span), or its best group.
fn span_link(p: &[f64], n: usize) -> Vec<Vec<usize>> {
    let br = bridges(p, n);
    let core: Vec<usize> = (0..n).filter(|i| !br.contains(i)).collect();
    if core.is_empty() {
        return avg_link(p, n, 0.0);
    }
    let m = core.len();
    let mut sub = vec![0f64; m * m];
    for a in 0..m {
        for b in 0..m {
            sub[a * m + b] = p[core[a] * n + core[b]];
        }
    }
    let mut groups: Vec<Vec<usize>> = avg_link(&sub, m, 0.0)
        .into_iter()
        .map(|g| g.into_iter().map(|i| core[i]).collect())
        .collect();
    for &i in &br {
        let means: Vec<f64> = groups
            .iter()
            .map(|g| g.iter().map(|&j| p[i * n + j]).sum::<f64>() / g.len() as f64)
            .collect();
        let mut hit: Vec<usize> = (0..groups.len()).filter(|&k| means[k] > 0.0).collect();
        if hit.is_empty() {
            let mut best = 0;
            for k in 1..means.len() {
                if means[k] > means[best] {
                    best = k;
                }
            }
            hit = vec![best];
        }
        for k in hit {
            groups[k].push(i);
        }
    }
    for g in groups.iter_mut() {
        g.sort_unstable();
    }
    groups
}

/// Box indices in reading order: lines (vertical overlap over half the shorter box), top to
/// bottom, left to right within a line.
pub fn reading_order(boxes: &[[f64; 4]]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..boxes.len()).collect();
    order.sort_by(|&a, &b| boxes[a][1].partial_cmp(&boxes[b][1]).unwrap());
    let mut lines: Vec<(f64, f64, Vec<usize>)> = Vec::new();
    for k in order {
        let b = boxes[k];
        let mut placed = false;
        for ln in lines.iter_mut() {
            let ov = ln.1.min(b[3]) - ln.0.max(b[1]);
            if ov > 0.5 * (ln.1 - ln.0).min(b[3] - b[1]) {
                ln.2.push(k);
                ln.0 = ln.0.min(b[1]);
                ln.1 = ln.1.max(b[3]);
                placed = true;
                break;
            }
        }
        if !placed {
            lines.push((b[1], b[3], vec![k]));
        }
    }
    lines.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    let mut out = Vec::new();
    for ln in lines {
        let mut ks = ln.2;
        ks.sort_by(|&a, &b| boxes[a][0].partial_cmp(&boxes[b][0]).unwrap());
        out.extend(ks);
    }
    out
}

fn bbox_of(units: &[DUnit], g: &[usize]) -> [f64; 4] {
    let mut b = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for &i in g {
        let u = &units[i].bx;
        b[0] = b[0].min(u[0]);
        b[1] = b[1].min(u[1]);
        b[2] = b[2].max(u[2]);
        b[3] = b[3].max(u[3]);
    }
    b
}

/// `re.match(r"^\s*(table|tab\.|figure|fig\.|exhibit|chart|source|note)s?\b", t, re.I)`: the
/// alternatives in order, the optional "s" tried first (greedy), then a word boundary.
fn is_caption(text: &str) -> bool {
    let t: Vec<char> = text.trim_start().to_lowercase().chars().collect();
    let word = |c: char| c.is_alphanumeric() || c == '_';
    let boundary = |pos: usize| {
        let prev = pos.checked_sub(1).map(|i| word(t[i])).unwrap_or(false);
        let next = t.get(pos).map(|&c| word(c)).unwrap_or(false);
        prev != next
    };
    for w in [
        "table", "tab.", "figure", "fig.", "exhibit", "chart", "source", "note",
    ] {
        let wc: Vec<char> = w.chars().collect();
        if t.len() < wc.len() || t[..wc.len()] != wc[..] {
            continue;
        }
        let end = wc.len();
        if t.get(end) == Some(&'s') && boundary(end + 1) {
            return true;
        }
        if boundary(end) {
            return true;
        }
    }
    false
}

/// Grow each table up over its header band (see tn_core._head_band).
fn head_band(units: &[DUnit], l: &Logits, groups: &mut [Vec<usize>], taken: &mut Vec<bool>) {
    let hm = median_h(units, 0..units.len());
    for g in groups.iter_mut() {
        for _ in 0..3 {
            let b = bbox_of(units, g);
            let (x0, y0, x1) = (b[0], b[1], b[2]);
            let above: Vec<usize> = (0..units.len())
                .filter(|&i| {
                    let u = &units[i].bx;
                    !taken[i]
                        && l.node(i, 0) > -8.0
                        && u[3] <= y0 + 0.2 * hm
                        && y0 - u[3] < 4.0 * 1.4 * hm
                })
                .collect();
            if above.is_empty() {
                break;
            }
            // the nearest line above: units overlapping vertically with the lowest one (max()'s first)
            let mut low = above[0];
            for &i in &above[1..] {
                if units[i].bx[3] > units[low].bx[3] {
                    low = i;
                }
            }
            let lb = units[low].bx;
            let line: Vec<usize> = above
                .iter()
                .copied()
                .filter(|&i| units[i].bx[3].min(lb[3]) - units[i].bx[1].max(lb[1]) > 0.3 * hm)
                .collect();
            let w = x1 - x0;
            let mut ok = line
                .iter()
                .all(|&i| units[i].bx[0] >= x0 - 0.05 * w && units[i].bx[2] <= x1 + 0.05 * w);
            let near = y0
                - line
                    .iter()
                    .map(|&i| units[i].bx[3])
                    .fold(f64::NEG_INFINITY, f64::max)
                < 1.5 * 1.4 * hm;
            let strong = line.iter().all(|&i| l.node(i, 0) > -1.5);
            let over = line.iter().all(|&i| {
                g.iter().any(|&j| {
                    units[i].bx[2].min(units[j].bx[2]) - units[i].bx[0].max(units[j].bx[0]) > 0.0
                })
            });
            ok = ok && (strong || (near && over && line.iter().all(|&i| l.node(i, 0) > -4.0)));
            ok = ok && !line.iter().any(|&i| is_caption(&units[i].text));
            let hdr = line.iter().filter(|&&i| l.node(i, 1) > 0.0).count() as f64
                / line.len().max(1) as f64;
            ok = ok && line.len() >= 2 && hdr > 0.5;
            ok = ok
                && !line.iter().any(|&i| {
                    let t = &units[i].text;
                    t.split_whitespace().count() > 6
                        || t.trim_end().ends_with('.')
                        || t.trim_end().ends_with(':')
                });
            if !ok {
                break;
            }
            for &i in &line {
                g.push(i);
                taken[i] = true;
            }
        }
        g.sort_unstable();
    }
}

/// Tables as lists of unit indices (tn_core.decode_tables, absorb=True).
pub fn decode_tables(units: &[DUnit], l: &Logits, opt: &DecodeOptions) -> Vec<Vec<usize>> {
    let n = units.len();
    let on: Vec<usize> = (0..n)
        .filter(|&i| l.node(i, 0) > opt.table_thresh)
        .collect();
    if on.len() < 2 {
        return Vec::new();
    }
    let m = on.len();
    let mut p = vec![0f64; m * m];
    for a in 0..m {
        for b in 0..m {
            p[a * m + b] = l.pair(0, on[a], on[b]) as f64;
        }
    }
    let mut groups: Vec<Vec<usize>> = avg_link(&p, m, 0.0)
        .into_iter()
        .filter(|g| g.len() >= 2)
        .map(|g| g.into_iter().map(|i| on[i]).collect())
        .collect();
    // stacked pieces that line up across and sit within 3 lines merge back
    if groups.len() > 1 {
        let hm = median_h(units, 0..n);
        loop {
            let ext: Vec<[f64; 4]> = groups.iter().map(|g| bbox_of(units, g)).collect();
            let mut merged = None;
            'outer: for a in 0..groups.len() {
                for b in 0..groups.len() {
                    if a == b {
                        continue;
                    }
                    let (aa, bb) = (ext[a], ext[b]);
                    let gap = bb[1] - aa[3];
                    let ov = aa[2].min(bb[2]) - aa[0].max(bb[0]);
                    if gap >= 0.0
                        && gap < 3.0 * 1.4 * hm
                        && ov > 0.7 * (aa[2] - aa[0]).min(bb[2] - bb[0])
                    {
                        merged = Some((a, b));
                        break 'outer;
                    }
                }
            }
            match merged {
                Some((a, b)) => {
                    let gb = groups[b].clone();
                    groups[a].extend(gb);
                    groups[a].sort_unstable();
                    groups.remove(b);
                }
                None => break,
            }
        }
    }
    // a small group lying mostly inside a bigger table's box (grown by a line) joins it
    if groups.len() > 1 {
        let hm = median_h(units, 0..n);
        loop {
            groups.sort_by(|a, b| b.len().cmp(&a.len()));
            let ext: Vec<[f64; 4]> = groups
                .iter()
                .map(|g| {
                    let b = bbox_of(units, g);
                    [
                        b[0] - 1.4 * hm,
                        b[1] - 1.4 * hm,
                        b[2] + 1.4 * hm,
                        b[3] + 1.4 * hm,
                    ]
                })
                .collect();
            let mut merged = None;
            'outer2: for s in (1..groups.len()).rev() {
                let sb = bbox_of(units, &groups[s]);
                let area = ((sb[2] - sb[0]) * (sb[3] - sb[1])).max(1e-6);
                for b in 0..s {
                    if groups[s].len() as f64 > 0.5 * groups[b].len() as f64 {
                        continue;
                    }
                    let e = ext[b];
                    let ov = (sb[2].min(e[2]) - sb[0].max(e[0])).max(0.0)
                        * (sb[3].min(e[3]) - sb[1].max(e[1])).max(0.0);
                    if ov >= 0.5 * area {
                        merged = Some((s, b));
                        break 'outer2;
                    }
                }
            }
            match merged {
                Some((s, b)) => {
                    let gs = groups[s].clone();
                    groups[b].extend(gs);
                    groups[b].sort_unstable();
                    groups.remove(s);
                }
                None => break,
            }
        }
    }
    if !groups.is_empty() {
        let mut taken = vec![false; n];
        for g in &groups {
            for &i in g {
                taken[i] = true;
            }
        }
        for g in groups.iter_mut() {
            let b = bbox_of(units, g);
            for i in 0..n {
                if taken[i] || l.node(i, 0) < -8.0 {
                    continue;
                }
                let u = &units[i].bx;
                let (cx, cy) = ((u[0] + u[2]) / 2.0, (u[1] + u[3]) / 2.0);
                if b[0] <= cx && cx <= b[2] && b[1] <= cy && cy <= b[3] {
                    g.push(i);
                    taken[i] = true;
                }
            }
            g.sort_unstable();
        }
        if opt.head_band {
            head_band(units, l, &mut groups, &mut taken);
        }
    }
    groups
}

/// A placed grid cell (top-left slot, span ends, text, box).
#[derive(Clone, Debug)]
pub struct GridCell {
    pub r0: usize,
    pub c0: usize,
    pub r1: usize,
    pub c1: usize,
    pub text: String,
    pub bx: [f64; 4],
}

/// A decoded table: cells in placement order, the header rows, and the grid size.
#[derive(Clone, Debug)]
pub struct Grid {
    pub cells: Vec<GridCell>,
    pub head_rows: Vec<usize>,
    pub n_rows: usize,
    pub n_cols: usize,
}

impl Grid {
    /// The grid of a fully ruled table, taken from its ruling lines ("lattice"): rows and columns
    /// are the bands between consecutive horizontal / vertical rules over the table, a cell
    /// boundary whose rule is missing joins the two cells (a spanned cell), and each unit goes to
    /// the cell holding its centre. The model reads rules only as soft features, so it can still
    /// split a wrapped cell into rows or fuse two cells a vertical rule separates; when the rules
    /// enclose the table, they decide. `None` when they don't: under 2x2 bands, units straddling a
    /// rule, a mostly empty lattice (graph paper, form boxes), or a band holding several rows of
    /// its own (see below).
    /// Header rows are the lattice rows inside the model's header band.
    pub fn lattice(
        &self,
        h_rules: &[[f32; 3]],
        v_rules: &[[f32; 3]],
        units: &[DUnit],
        words: &[&[(f64, f64, String)]],
        g: &[usize],
    ) -> Option<Grid> {
        if g.len() < 4 {
            return None;
        }
        let mut t = [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ];
        for &i in g {
            let b = units[i].bx;
            t = [
                t[0].min(b[0]),
                t[1].min(b[1]),
                t[2].max(b[2]),
                t[3].max(b[3]),
            ];
        }
        let lh = {
            let mut hs: Vec<f64> = g.iter().map(|&i| units[i].bx[3] - units[i].bx[1]).collect();
            median(&mut hs).max(4.0)
        };
        let pad = 1.5 * lh;
        let tw = t[2] - t[0];
        // rule positions over the table, clustered (a rule drawn as two strokes, a thick border
        // read as two edges) with their covered intervals
        let lines = |rules: &[[f32; 3]], lo: f64, hi: f64, a0: f64, a1: f64, min_len: f64| {
            let mut rs: Vec<(f64, f64, f64)> = rules
                .iter()
                .map(|r| (r[0] as f64, r[1].min(r[2]) as f64, r[1].max(r[2]) as f64))
                .filter(|&(p, s, e)| {
                    p >= lo - pad
                        && p <= hi + pad
                        && e - s >= min_len
                        && e > a0 - pad
                        && s < a1 + pad
                })
                .collect();
            rs.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut out: Vec<(f64, Vec<(f64, f64)>)> = Vec::new();
            for (p, s, e) in rs {
                match out.last_mut() {
                    Some((q, iv)) if p - *q < 3.0 => {
                        *q = (*q + p) / 2.0;
                        iv.push((s, e));
                    }
                    _ => out.push((p, vec![(s, e)])),
                }
            }
            out
        };
        let mut ys = lines(h_rules, t[1], t[3], t[0], t[2], (0.1 * tw).max(2.0 * lh));
        let mut xs = lines(v_rules, t[0], t[2], t[1], t[3], (0.5 * lh).max(4.0));
        let covers = |iv: &[(f64, f64)], a: f64, b: f64| -> f64 {
            let mut iv: Vec<(f64, f64)> = iv
                .iter()
                .map(|&(s, e)| (s.max(a), e.min(b)))
                .filter(|x| x.1 > x.0)
                .collect();
            iv.sort_by(|x, y| x.0.total_cmp(&y.0));
            let (mut sum, mut end) = (0.0, a);
            for (s, e) in iv {
                if e > end {
                    sum += e - s.max(end);
                    end = e;
                }
            }
            sum / (b - a).max(1e-6)
        };
        // a rule over under 30% of the table's height (width) is a stroke inside one cell, not a
        // grid line (decoration in a header, a box around a figure)
        xs.retain(|(_, iv)| covers(iv, t[1], t[3]) >= 0.3);
        ys.retain(|(_, iv)| covers(iv, t[0], t[2]) >= 0.3);
        // a side with no rule (open-sided tables, a header above the box) is closed at the text
        let near =
            |v: &[(f64, Vec<(f64, f64)>)], p: f64| v.iter().any(|(q, _)| (q - p).abs() < 0.5 * lh);
        let inner = |v: &[(f64, Vec<(f64, f64)>)], lo: f64, hi: f64| {
            v.iter()
                .filter(|(q, _)| *q > lo + 0.5 * lh && *q < hi - 0.5 * lh)
                .count()
        };
        let (ys_in, xs_in) = (inner(&ys, t[1], t[3]), inner(&xs, t[0], t[2]));
        ys.retain(|(q, _)| *q >= t[1] - pad && *q <= t[3] + pad);
        xs.retain(|(q, _)| *q >= t[0] - pad && *q <= t[2] + pad);
        if ys.first().is_none_or(|(q, _)| *q > t[1] + 0.5 * lh) && !near(&ys, t[1]) {
            ys.insert(0, (t[1] - 1.0, vec![(f64::NEG_INFINITY, f64::INFINITY)]));
        }
        if ys.last().is_none_or(|(q, _)| *q < t[3] - 0.5 * lh) && !near(&ys, t[3]) {
            ys.push((t[3] + 1.0, vec![(f64::NEG_INFINITY, f64::INFINITY)]));
        }
        if xs.first().is_none_or(|(q, _)| *q > t[0] + 0.5 * lh) && !near(&xs, t[0]) {
            xs.insert(0, (t[0] - 1.0, vec![(f64::NEG_INFINITY, f64::INFINITY)]));
        }
        if xs.last().is_none_or(|(q, _)| *q < t[2] - 0.5 * lh) && !near(&xs, t[2]) {
            xs.push((t[2] + 1.0, vec![(f64::NEG_INFINITY, f64::INFINITY)]));
        }
        if ys_in < 1 || xs_in < 1 || ys.len() < 3 || xs.len() < 3 {
            return None;
        }
        // the frame: the outermost rules must enclose the text, spanning most of the table
        let (fx0, fx1) = (xs[0].0, xs[xs.len() - 1].0);
        let (fy0, fy1) = (ys[0].0, ys[ys.len() - 1].0);
        if fx0 > t[0] + 0.5 * lh
            || fx1 < t[2] - 0.5 * lh
            || fy0 > t[1] + 0.5 * lh
            || fy1 < t[3] - 0.5 * lh
        {
            return None;
        }
        if covers(&ys[0].1, fx0, fx1) < 0.8 || covers(&ys[ys.len() - 1].1, fx0, fx1) < 0.8 {
            return None;
        }
        let (nr, nc) = (ys.len() - 1, xs.len() - 1);
        if nr * nc > 4 * g.len() + 16 {
            return None;
        }
        // a unit crossing a vertical rule (text set flush against it fuses two cells) is cut there
        // by its words; units by centre after that. One still straddling a rule (no word boxes, a
        // word across the rule) means the rules don't explain the text: left to the model
        let mut pieces: Vec<([f64; 4], String)> = Vec::new();
        for &i in g {
            let b = units[i].bx;
            let cuts: Vec<f64> = xs
                .iter()
                .filter(|(x, iv)| {
                    b[0] + 0.3 * lh < *x && *x < b[2] - 0.3 * lh && covers(iv, b[1], b[3]) > 0.5
                })
                .map(|(x, _)| *x)
                .collect();
            if cuts.is_empty() || words[i].len() < 2 {
                pieces.push((b, units[i].text.clone()));
                continue;
            }
            let mut groups: Vec<Vec<&(f64, f64, String)>> = vec![Vec::new(); cuts.len() + 1];
            for w in words[i] {
                let k = cuts.iter().filter(|&&x| (w.0 + w.1) / 2.0 > x).count();
                groups[k].push(w);
            }
            for ws in groups.into_iter().filter(|ws| !ws.is_empty()) {
                let (x0, x1) = ws.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |a, w| {
                    (a.0.min(w.0), a.1.max(w.1))
                });
                let text = ws
                    .iter()
                    .map(|w| w.2.as_str())
                    .collect::<Vec<_>>()
                    .join(" ");
                pieces.push(([x0, b[1], x1, b[3]], text));
            }
        }
        // which slots hold text of their own, and which vertical boundaries text runs across: a
        // missing vertical rule between two slots that both hold their own text is a rule the
        // render didn't catch (coloured, faint), not a spanned cell
        let slot_of = |b: &[f64; 4]| -> Option<(usize, usize)> {
            let (cx, cy) = ((b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0);
            let r = (0..nr).find(|&r| ys[r].0 <= cy && cy <= ys[r + 1].0)?;
            let c = (0..nc).find(|&c| xs[c].0 <= cx && cx <= xs[c + 1].0)?;
            Some((r, c))
        };
        let mut occupied = vec![false; nr * nc];
        let mut crossed = vec![false; nr * nc]; // boundary right of slot (r, c) has text across it
        let mut crossed_h = vec![false; nr * nc]; // boundary below slot (r, c) has text across it
        for (b, _) in &pieces {
            if let Some((r, c)) = slot_of(b) {
                occupied[r * nc + c] = true;
                for (k, (x, _)) in xs.iter().enumerate().skip(1).take(nc - 1) {
                    if b[0] < *x && *x < b[2] {
                        crossed[r * nc + k - 1] = true;
                    }
                }
                for (k, (y, _)) in ys.iter().enumerate().skip(1).take(nr - 1) {
                    if b[1] < *y && *y < b[3] {
                        crossed_h[(k - 1) * nc + c] = true;
                    }
                }
            }
        }
        let box_col: Vec<bool> = (0..nc)
            .map(|c| (1..nr).all(|k| covers(&ys[k].1, xs[c].0, xs[c + 1].0) < 0.5))
            .collect();
        // slots joined where the rule between them is (mostly) missing
        let mut parent: Vec<usize> = (0..nr * nc).collect();
        fn root(p: &mut [usize], mut k: usize) -> usize {
            while p[k] != k {
                p[k] = p[p[k]];
                k = p[k];
            }
            k
        }
        let join = |p: &mut Vec<usize>, a: usize, b: usize| {
            let (ra, rb) = (root(p, a), root(p, b));
            if ra != rb {
                p[ra.max(rb)] = ra.min(rb);
            }
        };
        for r in 0..nr {
            for c in 0..nc {
                if c + 1 < nc
                    && !(occupied[r * nc + c] && occupied[r * nc + c + 1] && !crossed[r * nc + c])
                    && covers(&xs[c + 1].1, ys[r].0, ys[r + 1].0) < 0.5
                {
                    join(&mut parent, r * nc + c, r * nc + c + 1);
                }
                // likewise a missing horizontal segment between two slots each holding their own
                // text, in a column with no row line at all (a column drawn as one box, the row
                // lines stopping at its edge); elsewhere a missing segment is a spanned cell
                if r + 1 < nr
                    && !(box_col[c]
                        && occupied[r * nc + c]
                        && occupied[(r + 1) * nc + c]
                        && !crossed_h[r * nc + c])
                    && covers(&ys[r + 1].1, xs[c].0, xs[c + 1].0) < 0.5
                {
                    join(&mut parent, r * nc + c, (r + 1) * nc + c);
                }
            }
        }
        // each component must be a rectangle of slots
        let mut span: std::collections::HashMap<usize, (usize, usize, usize, usize)> =
            std::collections::HashMap::new();
        for r in 0..nr {
            for c in 0..nc {
                let k = root(&mut parent, r * nc + c);
                let e = span.entry(k).or_insert((r, c, r, c));
                *e = (e.0.min(r), e.1.min(c), e.2.max(r), e.3.max(c));
            }
        }
        for (&k, &(r0, c0, r1, c1)) in &span {
            for r in r0..=r1 {
                for c in c0..=c1 {
                    if root(&mut parent, r * nc + c) != k {
                        return None;
                    }
                }
            }
        }
        let mut members: std::collections::HashMap<usize, Vec<usize>> =
            std::collections::HashMap::new();
        let mut straddle = 0;
        for (i, (b, _)) in pieces.iter().enumerate() {
            let b = *b;
            let (cx, cy) = ((b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0);
            let Some(r) = (0..nr).find(|&r| ys[r].0 <= cy && cy <= ys[r + 1].0) else {
                return None;
            };
            let Some(c) = (0..nc).find(|&c| xs[c].0 <= cx && cx <= xs[c + 1].0) else {
                return None;
            };
            let k = root(&mut parent, r * nc + c);
            let (_, c0, _, c1) = span[&k];
            let cut = xs[c0 + 1..=c1]
                .iter()
                .chain(xs[..=c0].iter().rev().take(1))
                .chain(xs[c1 + 1..].iter().take(1));
            if cut.into_iter().any(|(x, iv)| {
                b[0] + 0.3 * lh < *x && *x < b[2] - 0.3 * lh && covers(iv, b[1], b[3]) > 0.5
            }) {
                straddle += 1;
            }
            members.entry(k).or_default().push(i);
        }
        if straddle * 10 > pieces.len() {
            return None;
        }
        // rows the rules don't draw: a cell stacking two or more lines that are each a number
        // (numbers don't wrap: those are rows), or a band whose filled cells all hold the same
        // number of lines, line for line (a header over the first data row with no rule between)
        let lines_of = |ms: &[usize]| -> Vec<(f64, String)> {
            let mut ls: Vec<(f64, String)> = Vec::new();
            let mut ps: Vec<usize> = ms.to_vec();
            ps.sort_by(|&a, &b| pieces[a].0[1].total_cmp(&pieces[b].0[1]));
            for i in ps {
                let (b, t) = &pieces[i];
                let cy = (b[1] + b[3]) / 2.0;
                match ls.last_mut() {
                    Some((y, s)) if (cy - *y).abs() < 0.5 * lh => {
                        s.push(' ');
                        s.push_str(t.trim());
                    }
                    _ => ls.push((cy, t.trim().to_string())),
                }
            }
            ls
        };
        let mut band_lines: std::collections::HashMap<(usize, usize), Vec<Vec<f64>>> =
            std::collections::HashMap::new();
        for (k, ms) in &members {
            let ls = lines_of(ms);
            if ls.len() >= 2 && ls.iter().filter(|(_, t)| is_number(t)).count() >= 2 {
                return None;
            }
            let (r0, _, r1, _) = span[k];
            band_lines
                .entry((r0, r1))
                .or_default()
                .push(ls.iter().map(|l| l.0).collect());
        }
        for cells in band_lines.values() {
            // most of the band's cells (a marker column of one glyph per row aside): the
            // multi-line cell holding the most lines sets the pattern
            let Some(lead) = cells
                .iter()
                .filter(|ys| ys.len() >= 2)
                .max_by_key(|ys| ys.len())
            else {
                continue;
            };
            let k = lead.len();
            let lockstep = cells
                .iter()
                .filter(|ys| {
                    ys.len() == k && ys.iter().zip(lead).all(|(a, b)| (a - b).abs() < 0.5 * lh)
                })
                .count();
            if cells.len() >= 3 && lockstep >= 3 && lockstep * 4 >= cells.len() * 3 {
                return None;
            }
        }
        // keep rows / columns holding text
        let used_r: Vec<bool> = (0..nr)
            .map(|r| {
                span.iter()
                    .any(|(k, s)| s.0 <= r && r <= s.2 && members.contains_key(k))
            })
            .collect();
        let used_c: Vec<bool> = (0..nc)
            .map(|c| {
                span.iter()
                    .any(|(k, s)| s.1 <= c && c <= s.3 && members.contains_key(k))
            })
            .collect();
        let rmap: Vec<usize> = used_r
            .iter()
            .scan(0, |n, &u| {
                let v = *n;
                *n += u as usize;
                Some(v)
            })
            .collect();
        let cmap: Vec<usize> = used_c
            .iter()
            .scan(0, |n, &u| {
                let v = *n;
                *n += u as usize;
                Some(v)
            })
            .collect();
        let (n_rows, n_cols) = (
            used_r.iter().filter(|&&u| u).count(),
            used_c.iter().filter(|&&u| u).count(),
        );
        // rules that explain fewer columns than the text shows aren't the table's grid
        if n_rows < 2 || n_cols < 2 || n_cols + 1 < self.n_cols {
            return None;
        }
        if members.len() * 10 < 3 * span.len().min(n_rows * n_cols) {
            return None;
        }
        let first_used =
            |a: usize, b: usize, used: &[bool], map: &[usize]| -> Option<(usize, usize)> {
                let us: Vec<usize> = (a..=b).filter(|&k| used[k]).collect();
                Some((map[*us.first()?], map[*us.last()?]))
            };
        // header band: the model's header rows
        let head_band = self
            .cells
            .iter()
            .filter(|c| self.head_rows.contains(&c.r0) && !c.text.trim().is_empty())
            .fold(None, |acc: Option<(f64, f64)>, c| {
                Some(acc.map_or((c.bx[1], c.bx[3]), |a| (a.0.min(c.bx[1]), a.1.max(c.bx[3]))))
            });
        let mut cells = Vec::new();
        let mut head = Vec::new();
        let mut keys: Vec<&usize> = span.keys().collect();
        keys.sort_by_key(|k| span[k]);
        for k in keys {
            let (r0, c0, r1, c1) = span[k];
            let Some((nr0, nr1)) = first_used(r0, r1, &used_r, &rmap) else {
                continue;
            };
            let Some((nc0, nc1)) = first_used(c0, c1, &used_c, &cmap) else {
                continue;
            };
            let (text, bx) = match members.get(k) {
                Some(ms) => {
                    let boxes: Vec<[f64; 4]> = ms.iter().map(|&i| pieces[i].0).collect();
                    // lines top to bottom, pieces left to right within a line (a raised "(*)" is
                    // part of its line); a CJK line wrapped inside the cell joins without a space
                    let mut order: Vec<usize> = (0..ms.len()).collect();
                    order.sort_by(|&a, &b| {
                        let (pa, pb) = (boxes[a], boxes[b]);
                        let same = pa[1].max(pb[1])
                            < pa[3].min(pb[3]) - 0.3 * (pa[3] - pa[1]).min(pb[3] - pb[1]);
                        if same {
                            pa[0].total_cmp(&pb[0])
                        } else {
                            pa[1].total_cmp(&pb[1])
                        }
                    });
                    let cjk = |c: char| matches!(c, '\u{3040}'..='\u{30FF}' | '\u{3400}'..='\u{4DBF}' | '\u{4E00}'..='\u{9FFF}' | '\u{AC00}'..='\u{D7AF}' | '\u{FF00}'..='\u{FFEF}');
                    let mut text = String::new();
                    for j in order {
                        let t = pieces[ms[j]].1.trim();
                        if !text.is_empty()
                            && !(text.chars().last().is_some_and(cjk)
                                && t.chars().next().is_some_and(cjk))
                        {
                            text.push(' ');
                        }
                        text.push_str(t);
                    }
                    let bx = boxes.iter().fold(
                        [
                            f64::INFINITY,
                            f64::INFINITY,
                            f64::NEG_INFINITY,
                            f64::NEG_INFINITY,
                        ],
                        |a, b| {
                            [
                                a[0].min(b[0]),
                                a[1].min(b[1]),
                                a[2].max(b[2]),
                                a[3].max(b[3]),
                            ]
                        },
                    );
                    (text, bx)
                }
                None => (
                    String::new(),
                    [xs[c0].0, ys[r0].0, xs[c1 + 1].0, ys[r1 + 1].0],
                ),
            };
            if let Some((h0, h1)) = head_band {
                let cy = (ys[r0].0 + ys[r1 + 1].0) / 2.0;
                if h0 <= cy && cy <= h1 && !head.contains(&nr0) {
                    head.push(nr0);
                }
            }
            cells.push(GridCell {
                r0: nr0,
                c0: nc0,
                r1: nr1,
                c1: nc1,
                text,
                bx,
            });
        }
        // header rows run from the top
        head.sort_unstable();
        let head_rows: Vec<usize> = head
            .iter()
            .enumerate()
            .take_while(|(k, r)| *k == **r)
            .map(|(_, r)| *r)
            .collect();
        Some(Grid {
            cells,
            head_rows,
            n_rows,
            n_cols,
        })
    }

    /// Rotated text inside an upright table (column headers set sideways, "Ezzitounna" /
    /// "La Manouba" over narrow columns) is left out of the model's units; each such unit
    /// fills the cell under it if the model left that cell empty ([`Grid::place_items`]), its
    /// lines in reading order (text running up stacks left to right). Returns the units placed.
    pub fn place_rotated(&mut self, units: &[super::units::Unit], minor: &[bool]) -> Vec<usize> {
        let items: Vec<(usize, [f64; 4], String, f64)> = units
            .iter()
            .enumerate()
            // header words, not stray sideways digits and punctuation
            .filter(|(i, u)| {
                minor[*i] && u.q != 0 && u.text.chars().filter(|c| c.is_alphabetic()).count() >= 3
            })
            .map(|(i, u)| {
                let cx = (u.pbox[0] + u.pbox[2]) / 2.0;
                (i, u.pbox, u.text.clone(), if u.q == 3 { -cx } else { cx })
            })
            .collect();
        self.place_items(&items)
    }

    /// OCR text inside a table on a page with a text layer (a column drawn as outlines or
    /// pictures: "10 kW" beside native "5 | YES | 3977") is not among the model's units; each
    /// item fills the empty cell under it ([`Grid::place_items`]). Returns the items placed.
    pub fn place_ocr(&mut self, items: &[([f64; 4], String)]) -> Vec<usize> {
        // only a column the text layer leaves (almost) empty below its header: OCR text over a
        // table whose box takes in a chart (axis ticks, a legend) must not scatter into the gaps
        // of columns that hold native text
        let body: Vec<usize> = (0..self.n_rows)
            .filter(|r| !self.head_rows.contains(r))
            .collect();
        let empty_col: Vec<bool> = (0..self.n_cols)
            .map(|c| {
                let filled = body
                    .iter()
                    .filter(|&&r| {
                        self.cells.iter().any(|x| {
                            x.r0 <= r
                                && r <= x.r1
                                && x.c0 <= c
                                && c <= x.c1
                                && !x.text.trim().is_empty()
                        })
                    })
                    .count();
                !body.is_empty() && filled * 5 <= body.len()
            })
            .collect();
        if !empty_col.iter().any(|&e| e) {
            return Vec::new();
        }
        let before: Vec<bool> = self
            .cells
            .iter()
            .map(|c| !c.text.trim().is_empty())
            .collect();
        let items: Vec<(usize, [f64; 4], String, f64)> = items
            .iter()
            .enumerate()
            .map(|(i, (b, t))| (i, *b, t.clone(), b[0]))
            .collect();
        let placed = self.place_items(&items);
        // undo what landed outside the empty columns
        let mut keep = Vec::new();
        for (k, c) in self.cells.iter_mut().enumerate() {
            let was = before.get(k).copied().unwrap_or(false);
            if !was && !c.text.trim().is_empty() && !(c.c0..=c.c1).any(|q| empty_col[q]) {
                c.text.clear();
            }
        }
        for i in placed {
            let b = items[i].1;
            let cx = (b[0] + b[2]) / 2.0;
            if self.cells.iter().any(|c| {
                !c.text.trim().is_empty()
                    && c.bx[0] <= cx
                    && cx <= c.bx[2]
                    && (c.c0..=c.c1).any(|q| empty_col[q])
            }) {
                keep.push(i);
            }
        }
        keep
    }

    /// Place `(id, page box, text, order)` items into the cells under them: the column whose span
    /// (from its one-column filled cells) holds the item's centre, the row whose band it overlaps
    /// most; only cells the model left empty (text over a filled cell belongs to a column the grid
    /// lost), several items in one cell joined by `order`. Returns the ids placed.
    fn place_items(&mut self, items: &[(usize, [f64; 4], String, f64)]) -> Vec<usize> {
        let span = |pick: &dyn Fn(&GridCell) -> Option<usize>, n: usize, lo: usize, hi: usize| {
            let mut v = vec![(f64::INFINITY, f64::NEG_INFINITY); n];
            for c in &self.cells {
                if let Some(k) = pick(c).filter(|&k| k < n && !c.text.trim().is_empty()) {
                    v[k] = (v[k].0.min(c.bx[lo]), v[k].1.max(c.bx[hi]));
                }
            }
            v
        };
        let cols = span(&|c| (c.c0 == c.c1).then_some(c.c0), self.n_cols, 0, 2);
        let rows = span(&|c| (c.r0 == c.r1).then_some(c.r0), self.n_rows, 1, 3);
        let mut slots: Vec<((usize, usize), f64, usize)> = Vec::new();
        for (k, (_, p, _, order)) in items.iter().enumerate() {
            let p = *p;
            let cx = (p[0] + p[2]) / 2.0;
            let Some(c) = cols.iter().position(|&(a, b)| a <= cx && cx <= b) else {
                continue;
            };
            let overlap = |&(a, b): &(f64, f64)| (b.min(p[3]) - a.max(p[1])).max(0.0);
            let Some(r) = (0..rows.len())
                .filter(|&r| overlap(&rows[r]) > 0.0)
                .max_by(|&x, &y| overlap(&rows[x]).total_cmp(&overlap(&rows[y])))
            else {
                continue;
            };
            slots.push(((r, c), *order, k));
        }
        slots.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
        // only into cells the model left empty: rotated text over a cell that holds text belongs
        // to a column the grid lost, and stays out of the table
        let filled: Vec<bool> = self
            .cells
            .iter()
            .map(|c| !c.text.trim().is_empty())
            .collect();
        let mut placed = Vec::new();
        for ((r, c), _, k) in slots {
            let (i, p, t, _) = &items[k];
            let (i, p, t) = (*i, *p, t.trim());
            if self
                .cells
                .iter()
                .zip(&filled)
                .any(|(x, &f)| f && x.r0 <= r && r <= x.r1 && x.c0 <= c && c <= x.c1)
            {
                continue;
            }
            let k = match self
                .cells
                .iter()
                .position(|x| x.r0 <= r && r <= x.r1 && x.c0 <= c && c <= x.c1)
            {
                Some(k) => k,
                None => {
                    self.cells.push(GridCell {
                        r0: r,
                        c0: c,
                        r1: r,
                        c1: c,
                        text: String::new(),
                        bx: p,
                    });
                    self.cells.len() - 1
                }
            };
            let cell = &mut self.cells[k];
            if cell.text.trim().is_empty() {
                cell.text = t.to_string();
                cell.bx = p;
            } else {
                cell.text = format!("{} {}", cell.text.trim(), t);
                cell.bx = [
                    cell.bx[0].min(p[0]),
                    cell.bx[1].min(p[1]),
                    cell.bx[2].max(p[2]),
                    cell.bx[3].max(p[3]),
                ];
            }
            placed.push(i);
        }
        placed
    }

    /// Merge rows a ruled table draws as one: when most row boundaries carry a horizontal rule
    /// across the table, a boundary without one is a wrapped cell's next line (a tall SWOT cell
    /// read as 35 rows, "подметание территории ... 1 раз в" / "сутки" split in two). `h_rules`
    /// are `[y, x0, x1]` in the frame of the cell boxes.
    pub fn merge_unruled_rows(&mut self, h_rules: &[[f32; 3]]) {
        if self.n_rows < 3 || h_rules.is_empty() {
            return;
        }
        let x0 = self
            .cells
            .iter()
            .map(|c| c.bx[0])
            .fold(f64::INFINITY, f64::min);
        let x1 = self
            .cells
            .iter()
            .map(|c| c.bx[2])
            .fold(f64::NEG_INFINITY, f64::max);
        let width = (x1 - x0).max(1.0);
        // each row's ink band, from its one-row cells
        let mut band = vec![(f64::INFINITY, f64::NEG_INFINITY); self.n_rows];
        for c in self
            .cells
            .iter()
            .filter(|c| c.r0 == c.r1 && !c.text.is_empty())
        {
            band[c.r0].0 = band[c.r0].0.min(c.bx[1]);
            band[c.r0].1 = band[c.r0].1.max(c.bx[3]);
        }
        if band.iter().any(|b| !b.0.is_finite()) {
            return;
        }
        let ruled: Vec<bool> = (0..self.n_rows - 1)
            .map(|r| {
                let (lo, hi) = (band[r].1 - 2.0, band[r + 1].0 + 2.0);
                h_rules.iter().any(|&[y, a, b]| {
                    let y = y as f64;
                    let cover = (x1.min(b as f64) - x0.max(a as f64)).max(0.0);
                    y >= lo && y <= hi && cover >= 0.5 * width
                })
            })
            .collect();
        let n_ruled = ruled.iter().filter(|&&r| r).count();
        if n_ruled < 2 || n_ruled * 5 < ruled.len() * 3 || n_ruled == ruled.len() {
            return;
        }
        // a rule-less boundary folds only a continuation line: the row below starts with an empty
        // first column, fills at most half the columns, or fills columns the row above left
        // empty; two complete records (a table ruled every other row) stay apart, as do header rows
        let filled = |r: usize| -> Vec<usize> {
            let mut cols: Vec<usize> = self
                .cells
                .iter()
                .filter(|c| c.r0 <= r && r <= c.r1 && !c.text.is_empty())
                .flat_map(|c| c.c0..=c.c1)
                .collect();
            cols.sort_unstable();
            cols.dedup();
            cols
        };
        let fold: Vec<bool> = (0..self.n_rows - 1)
            .map(|r| {
                if ruled[r] || self.head_rows.contains(&r) || self.head_rows.contains(&(r + 1)) {
                    return false;
                }
                let (a, b) = (filled(r), filled(r + 1));
                let overlap = b.iter().filter(|q| a.contains(q)).count();
                !b.contains(&0) || b.len() * 2 <= self.n_cols || overlap <= 1
            })
            .collect();
        if !fold.iter().any(|&f| f) {
            return;
        }
        let mut map = vec![0usize; self.n_rows];
        for r in 1..self.n_rows {
            map[r] = map[r - 1] + !fold[r - 1] as usize;
        }
        let mut merged: Vec<GridCell> = Vec::new();
        let mut cells = std::mem::take(&mut self.cells);
        cells.sort_by(|a, b| (a.c0, a.bx[1]).partial_cmp(&(b.c0, b.bx[1])).unwrap());
        for mut c in cells {
            c.r0 = map[c.r0];
            c.r1 = map[c.r1];
            if let Some(m) = merged
                .iter_mut()
                .find(|m| m.c0 == c.c0 && m.r0 <= c.r1 && c.r0 <= m.r1)
            {
                if !c.text.is_empty() {
                    if !m.text.is_empty() {
                        m.text.push(' ');
                    }
                    m.text.push_str(&c.text);
                }
                m.r0 = m.r0.min(c.r0);
                m.r1 = m.r1.max(c.r1);
                m.c1 = m.c1.max(c.c1);
                m.bx = [
                    m.bx[0].min(c.bx[0]),
                    m.bx[1].min(c.bx[1]),
                    m.bx[2].max(c.bx[2]),
                    m.bx[3].max(c.bx[3]),
                ];
            } else {
                merged.push(c);
            }
        }
        merged.sort_by_key(|c| (c.r0, c.c0));
        self.cells = merged;
        let mut head: Vec<usize> = self.head_rows.iter().map(|&r| map[r]).collect();
        head.dedup();
        self.head_rows = head;
        self.n_rows = map[self.n_rows - 1] + 1;
    }
}

/// Fold near-empty columns into their nearest neighbour (tn_core._squeeze, column rule).
fn squeeze_cols(cells: &mut Vec<GridCell>, head: &[usize], n_cols: &mut usize, hh: f64) {
    let mut changed = true;
    while changed && *n_cols >= 3 {
        changed = false;
        for q in 0..*n_cols {
            let col: Vec<usize> = (0..cells.len())
                .filter(|&k| cells[k].c0 == q && cells[k].c1 == q && !cells[k].text.is_empty())
                .collect();
            if col
                .iter()
                .filter(|&&k| !head.contains(&cells[k].r0))
                .count()
                > 1
            {
                continue;
            }
            let boxes: Vec<[f64; 4]> = col.iter().map(|&k| cells[k].bx).collect();
            let mut best: Option<(f64, usize)> = None;
            for t in [q as isize - 1, q as isize + 1] {
                if t < 0 || t as usize >= *n_cols {
                    continue;
                }
                let t = t as usize;
                let nb: Vec<[f64; 4]> = cells
                    .iter()
                    .filter(|x| x.c0 <= t && t <= x.c1 && x.c0 == x.c1 && !x.text.is_empty())
                    .map(|x| x.bx)
                    .collect();
                if boxes.is_empty() || nb.is_empty() {
                    continue;
                }
                let mut gap = f64::INFINITY;
                for b in &boxes {
                    for nn in &nb {
                        gap = gap.min((b[0] - nn[2]).max(nn[0] - b[2]).max(0.0));
                    }
                }
                if gap < 1.5 * hh && best.is_none_or(|(g, _)| gap < g) {
                    best = Some((gap, t));
                }
            }
            if let Some((_, t)) = best {
                fold_col(cells, q, t);
                *n_cols -= 1;
                changed = true;
                break;
            }
        }
    }
}

fn fold_col(cells: &mut Vec<GridCell>, k: usize, t: usize) {
    let movers: Vec<usize> = (0..cells.len())
        .filter(|&i| cells[i].c0 == k && cells[i].c1 == k)
        .collect();
    let mut remove = Vec::new();
    for &xi in &movers {
        let r = cells[xi].r0;
        let tgt = (0..cells.len()).find(|&j| {
            cells[j].r0 <= r && r <= cells[j].r1 && cells[j].c0 <= t && t <= cells[j].c1
        });
        match tgt {
            Some(j) if j != xi => {
                if !cells[xi].text.is_empty() {
                    let joined = if t < k {
                        format!("{} {}", cells[j].text, cells[xi].text)
                    } else {
                        format!("{} {}", cells[xi].text, cells[j].text)
                    };
                    cells[j].text = joined.trim().to_string();
                }
                remove.push(xi);
            }
            _ => {
                cells[xi].c0 = t;
                cells[xi].c1 = t;
            }
        }
    }
    remove.sort_unstable();
    for &i in remove.iter().rev() {
        cells.remove(i);
    }
    for x in cells.iter_mut() {
        if x.c0 <= k && k <= x.c1 && x.c0 < x.c1 {
            x.c1 -= 1;
        } else if x.c0 > k {
            x.c0 -= 1;
            x.c1 -= 1;
        }
    }
}

/// One table's grid from its units (global indices `idx`) and the row / col / cell heads.
pub fn decode_grid(units: &[DUnit], idx: &[usize], l: &Logits, opt: &DecodeOptions) -> Grid {
    let m = idx.len();
    if m == 0 {
        return Grid {
            cells: Vec::new(),
            head_rows: Vec::new(),
            n_rows: 0,
            n_cols: 0,
        };
    }
    let bxs: Vec<[f32; 4]> = idx
        .iter()
        .map(|&i| {
            let b = units[i].bx;
            [b[0] as f32, b[1] as f32, b[2] as f32, b[3] as f32]
        })
        .collect();
    let hh: Vec<f32> = bxs.iter().map(|b| (b[3] - b[1]).max(1e-3)).collect();
    let pr = |k: usize, a: usize, b: usize| l.pair(k, idx[a], idx[b]);
    // cell logits: same_cell, or for stacked neighbours the weaker of same_row / same_col - margin
    let mut cell_p = vec![0f64; m * m];
    let mut forced = vec![false; m * m];
    for a in 0..m {
        for b in 0..m {
            let vgap = (bxs[b][1] - bxs[a][3]).max(bxs[a][1] - bxs[b][3]);
            let xov = bxs[a][2].min(bxs[b][2]) - bxs[a][0].max(bxs[b][0]);
            let both_hdr = l.node(idx[a], 1) > 0.0 && l.node(idx[b], 1) > 0.0;
            let lim = if both_hdr { STACK_GAP_HDR } else { STACK_GAP } * hh[a].min(hh[b]) as f64;
            let stacked = (vgap as f64) < lim && vgap > -0.5 * hh[a] && xov > 0.0;
            let f = if stacked {
                pr(1, a, b).min(pr(2, a, b)) - CELL_MARGIN
            } else {
                f32::NEG_INFINITY
            };
            forced[a * m + b] = f > 0.0;
            cell_p[a * m + b] = pr(3, a, b).max(f) as f64;
        }
    }
    let mut cells = avg_link(&cell_p, m, 0.0);
    if opt.forced_link && cells.len() > 1 {
        let mut of = vec![0usize; m];
        for (k, g) in cells.iter().enumerate() {
            for &i in g {
                of[i] = k;
            }
        }
        let mut parent: Vec<usize> = (0..cells.len()).collect();
        fn root(parent: &mut [usize], mut k: usize) -> usize {
            while parent[k] != k {
                parent[k] = parent[parent[k]];
                k = parent[k];
            }
            k
        }
        for a in 0..m {
            for b in 0..m {
                if forced[a * m + b] {
                    let ra = root(&mut parent, of[a]);
                    let rb = root(&mut parent, of[b]);
                    if ra != rb {
                        parent[ra] = rb;
                    }
                }
            }
        }
        let mut keys: Vec<usize> = Vec::new();
        let mut merged: Vec<Vec<usize>> = Vec::new();
        for (k, g) in cells.iter().enumerate() {
            let r = root(&mut parent, k);
            match keys.iter().position(|&x| x == r) {
                Some(p) => merged[p].extend(g.iter().copied()),
                None => {
                    keys.push(r);
                    merged.push(g.clone());
                }
            }
        }
        for g in merged.iter_mut() {
            g.sort_unstable();
        }
        cells = merged;
    }
    let nc = cells.len();
    let cb: Vec<[f64; 4]> = cells
        .iter()
        .map(|g| {
            let mut b = [
                f64::INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::NEG_INFINITY,
            ];
            for &i in g {
                let u = units[idx[i]].bx;
                b[0] = b[0].min(u[0]);
                b[1] = b[1].min(u[1]);
                b[2] = b[2].max(u[2]);
                b[3] = b[3].max(u[3]);
            }
            b
        })
        .collect();
    // cell-pair logits on an axis: the mean of their unit pairs, minus GATE without overlap
    let cell_logits = |k: usize, lo: usize, hi: usize| -> Vec<f64> {
        let mut out = vec![0f64; nc * nc];
        for a in 0..nc {
            for b in a + 1..nc {
                let mut s = 0f64;
                for &i in &cells[a] {
                    for &j in &cells[b] {
                        s += pr(k, i, j) as f64;
                    }
                }
                let mut v = s / (cells[a].len() * cells[b].len()) as f64;
                if cb[a][hi].min(cb[b][hi]) - cb[a][lo].max(cb[b][lo]) <= 0.0 {
                    v -= GATE;
                }
                out[a * nc + b] = v;
                out[b * nc + a] = v;
            }
        }
        out
    };
    let (mut rows, mut cols) = if nc > 1 {
        (
            span_link(&cell_logits(1, 1, 3), nc),
            span_link(&cell_logits(2, 0, 2), nc),
        )
    } else {
        (vec![vec![0]], vec![vec![0]])
    };
    let mean_of = |g: &Vec<usize>, f: &dyn Fn(usize) -> f64| {
        g.iter().map(|&k| f(k)).sum::<f64>() / g.len() as f64
    };
    let ycen = |k: usize| (cb[k][1] + cb[k][3]) / 2.0;
    let xcen = |k: usize| (cb[k][0] + cb[k][2]) / 2.0;
    rows.sort_by(|a, b| mean_of(a, &ycen).partial_cmp(&mean_of(b, &ycen)).unwrap());
    cols.sort_by(|a, b| mean_of(a, &xcen).partial_cmp(&mean_of(b, &xcen)).unwrap());
    let mut r_in = vec![Vec::new(); nc];
    let mut c_in = vec![Vec::new(); nc];
    for (k, g) in rows.iter().enumerate() {
        for &c in g {
            r_in[c].push(k);
        }
    }
    for (k, g) in cols.iter().enumerate() {
        for &c in g {
            c_in[c].push(k);
        }
    }
    let n_rows0 = rows.len();
    let n_cols0 = cols.len();
    let mut placed: Vec<GridCell> = Vec::new();
    let mut owner = vec![usize::MAX; n_rows0 * n_cols0]; // slot -> index in `placed`
    let mut order: Vec<usize> = (0..nc).collect();
    let rmin = |c: usize| *r_in[c].iter().min().unwrap();
    let cmin = |c: usize| *c_in[c].iter().min().unwrap();
    order.sort_by_key(|&a| (rmin(a), cmin(a)));
    for c in order {
        let (mut r0, mut r1) = (rmin(c), *r_in[c].iter().max().unwrap());
        let (mut c0, mut c1) = (cmin(c), *c_in[c].iter().max().unwrap());
        let boxes: Vec<[f64; 4]> = cells[c].iter().map(|&i| units[idx[i]].bx).collect();
        let text = reading_order(&boxes)
            .into_iter()
            .map(|k| units[idx[cells[c][k]]].text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let mut span = Vec::new();
        for r in r0..=r1 {
            for q in c0..=c1 {
                span.push((r, q));
            }
        }
        if span
            .iter()
            .any(|&(r, q)| owner[r * n_cols0 + q] != usize::MAX)
        {
            match span
                .iter()
                .find(|&&(r, q)| owner[r * n_cols0 + q] == usize::MAX)
            {
                Some(&(r, q)) => {
                    r0 = r;
                    r1 = r;
                    c0 = q;
                    c1 = q;
                }
                None => {
                    let o = owner[r0 * n_cols0 + c0];
                    placed[o].text = format!("{} {}", placed[o].text, text);
                    continue;
                }
            }
        }
        let pi = placed.len();
        placed.push(GridCell {
            r0,
            c0,
            r1,
            c1,
            text,
            bx: cb[c],
        });
        for r in r0..=r1 {
            for q in c0..=c1 {
                owner[r * n_cols0 + q] = pi;
            }
        }
    }
    let mut head_rows = Vec::new();
    for (k, g) in rows.iter().enumerate() {
        let us: Vec<usize> = g.iter().flat_map(|&c| cells[c].iter().copied()).collect();
        let pos = us.iter().filter(|&&i| l.node(idx[i], 1) > 0.0).count() as f64;
        if pos / us.len() as f64 > 0.5 {
            head_rows.push(k);
        }
    }
    let mut n_cols = n_cols0;
    if opt.squeeze {
        let hh = median_h(units, idx.iter().copied());
        squeeze_cols(&mut placed, &head_rows, &mut n_cols, hh);
    }
    let mut head_rows = head_rows;
    let mut n_rows = n_rows0;
    if opt.fold_wraps {
        fold_wrapped_rows(&mut placed, &mut head_rows, &mut n_rows);
        fold_lead_wraps(&mut placed, &mut head_rows, &mut n_rows);
        fold_header_wraps(&mut placed, &mut head_rows, &mut n_rows);
    }
    Grid {
        cells: placed,
        head_rows,
        n_rows,
        n_cols,
    }
}

/// A lone number (thousands separators, sign, decimal, percent): values don't wrap.
fn is_number(t: &str) -> bool {
    let t = t.trim().trim_end_matches('%').replace([',', ' '], "");
    !t.is_empty()
        && t.chars().any(|c| c.is_ascii_digit())
        && t.chars()
            .all(|c| c.is_ascii_digit() || "+-.()$€£".contains(c))
}

/// Text opening a new list item ("1.2 ...", "(a) ...", "• ..."): a sub-row, never a wrapped line.
fn starts_item(t: &str) -> bool {
    let t = t.trim_start();
    let Some(first) = t.split_whitespace().next() else {
        return false;
    };
    if first.chars().count() == 1 && "•·▪◦‣-–—*●○■□".contains(first) {
        return true;
    }
    let core = first
        .trim_start_matches('(')
        .trim_end_matches([')', '.', ':']);
    let enumerator = !core.is_empty()
        && (core
            .split('.')
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
            || (core.chars().count() == 1
                && core.chars().all(|c| c.is_ascii_alphabetic())
                && first != core));
    enumerator && first != t
}

/// Rows that are a wrapped cell's continuation lines fold into the row above: a row with fewer
/// filled cells than the one above, each under a filled cell, none a number, sitting clearly
/// tighter (under 0.8x) than the table's own row pitch (measured between rows of 2+ filled cells,
/// so a dense table with uniform pitch never folds).
fn fold_wrapped_rows(cells: &mut Vec<GridCell>, head: &mut Vec<usize>, n_rows: &mut usize) {
    let filled = |cells: &[GridCell], r: usize| -> Vec<usize> {
        (0..cells.len())
            .filter(|&k| cells[k].r0 == r && cells[k].r1 == r && !cells[k].text.trim().is_empty())
            .collect()
    };
    let top = |cells: &[GridCell], ks: &[usize]| {
        ks.iter()
            .map(|&k| cells[k].bx[1])
            .fold(f64::INFINITY, f64::min)
    };
    let mut pitches = Vec::new();
    for r in 1..*n_rows {
        let (a, b) = (filled(cells, r - 1), filled(cells, r));
        if a.len() >= 2 && b.len() >= 2 {
            pitches.push(top(cells, &b) - top(cells, &a));
        }
    }
    if pitches.len() < 2 {
        return;
    }
    let pitch = median(&mut pitches);
    let mut r = 1;
    while r < *n_rows {
        let (above, here) = (filled(cells, r - 1), filled(cells, r));
        let same_kind = head.contains(&r) == head.contains(&(r - 1));
        let mut targets = Vec::new();
        let span_starts = cells.iter().any(|c| c.r0 == r && c.r1 > r);
        let ok = same_kind
            && !span_starts
            && !here.is_empty()
            && here.len() < above.len()
            && here.iter().all(|&k| {
                let c = &cells[k];
                // the filled cell covering this column in the row above
                let t = (0..cells.len()).find(|&j| {
                    cells[j].r0 < r
                        && r - 1 <= cells[j].r1
                        && cells[j].c0 <= c.c0
                        && c.c1 <= cells[j].c1
                });
                match t {
                    Some(j)
                        if !cells[j].text.trim().is_empty()
                            && !is_number(&c.text)
                            && !starts_item(&c.text) =>
                    {
                        let ok = c.bx[1] - cells[j].bx[1] < 0.8 * pitch
                            && c.bx[1] - cells[j].bx[3] < 0.5 * pitch;
                        targets.push(j);
                        ok
                    }
                    _ => false,
                }
            });
        if !ok {
            r += 1;
            continue;
        }
        for (&k, &j) in here.iter().zip(&targets) {
            let (t, b) = (cells[k].text.trim().to_string(), cells[k].bx);
            let c = &mut cells[j];
            c.text = format!("{} {}", c.text.trim(), t);
            c.bx = [
                c.bx[0].min(b[0]),
                c.bx[1].min(b[1]),
                c.bx[2].max(b[2]),
                c.bx[3].max(b[3]),
            ];
        }
        remove_row(cells, head, n_rows, r);
    }
}

/// A wrapped row label whose values sit on its last line ("Unrealized appreciation on" / "Total
/// Return Swap" / "contracts  $ (67,658)") comes out as value-less rows above the value row. A body
/// row with no number folds down into the row below when that row has a number, every filled cell
/// has a filled, non-number cell directly below it that reads as its next line ([`continues`]),
/// and each pair sits line-tight (gap under 0.6x a line's height). Bottom-up, so a label
/// wrapped over three lines folds twice.
fn fold_lead_wraps(cells: &mut Vec<GridCell>, head: &mut Vec<usize>, n_rows: &mut usize) {
    // a line's height: the median filled cell (most cells hold one line); a short lowercase
    // line's own box ("contracts") lacks the ascender height
    let mut hs: Vec<f64> = cells
        .iter()
        .filter(|c| !c.text.trim().is_empty())
        .map(|c| c.bx[3] - c.bx[1])
        .collect();
    if hs.is_empty() {
        return;
    }
    let lh = median(&mut hs);
    let mut r = n_rows.saturating_sub(1);
    while r > 0 {
        r -= 1;
        if head.contains(&r) || head.contains(&(r + 1)) {
            continue;
        }
        if cells
            .iter()
            .any(|c| c.r0 != c.r1 && c.r0 <= r + 1 && r <= c.r1)
        {
            continue; // spans touching either row
        }
        let row = |cells: &[GridCell], r: usize| -> Vec<usize> {
            (0..cells.len())
                .filter(|&k| cells[k].r0 == r && !cells[k].text.trim().is_empty())
                .collect()
        };
        let (here, below) = (row(cells, r), row(cells, r + 1));
        if here.is_empty()
            || here.iter().any(|&k| is_number(&cells[k].text))
            || !below.iter().any(|&k| is_number(&cells[k].text))
        {
            continue;
        }
        let mut pairs = Vec::new();
        let ok = here.iter().all(|&k| {
            let a = &cells[k];
            let Some(&j) = below
                .iter()
                .find(|&&j| cells[j].c0 == a.c0 && cells[j].c1 == a.c1)
            else {
                return false;
            };
            let b = &cells[j];
            let line_h = (b.bx[3] - b.bx[1]).max(lh);
            pairs.push((k, j));
            // a bracket opening the lower line marks a sub-item ("(ii) Income tax ...") here, not
            // a continuation, unless the upper line left one open
            let bracketed = b.text.trim_start().starts_with(['(', '[', '{'])
                && a.text.matches('(').count() <= a.text.matches(')').count();
            !is_number(&b.text)
                && !starts_item(&b.text)
                && !bracketed
                && continues(&a.text, &b.text)
                && line_h > 0.0
                && b.bx[1] - a.bx[3] < 0.6 * line_h
        });
        if !ok {
            continue;
        }
        for (k, j) in pairs {
            let (t, bx) = (cells[k].text.trim().to_string(), cells[k].bx);
            let c = &mut cells[j];
            c.text = format!("{} {}", t, c.text.trim());
            c.bx = [
                c.bx[0].min(bx[0]),
                c.bx[1].min(bx[1]),
                c.bx[2].max(bx[2]),
                c.bx[3].max(bx[3]),
            ];
            cells[k].text.clear();
        }
        remove_row(cells, head, n_rows, r);
    }
}

/// Drop row `r` after its text moved up: its (empty) cells go, spans through it shrink, rows
/// below move up.
fn remove_row(cells: &mut Vec<GridCell>, head: &mut Vec<usize>, n_rows: &mut usize, r: usize) {
    cells.retain(|c| !(c.r0 == r && c.r1 == r));
    for c in cells.iter_mut() {
        if c.r0 > r {
            c.r0 -= 1;
            c.r1 -= 1;
        } else if c.r1 >= r {
            c.r1 -= 1;
        }
    }
    head.retain(|&h| h != r);
    for h in head.iter_mut() {
        if *h > r {
            *h -= 1;
        }
    }
    *n_rows -= 1;
}

/// Whether `below` reads as the next line of the same phrase as `above`: it opens lowercase or
/// with a bracket, or `above` breaks off mid-phrase (a hyphen, slash, comma, open bracket, `&`,
/// or a connective). A list of names ("Buick Reatta" under "Alfa Romeo") gives no such cue.
fn continues(above: &str, below: &str) -> bool {
    let (a, b) = (above.trim(), below.trim());
    let opens = b
        .chars()
        .next()
        .is_some_and(|ch| ch.is_lowercase() || "([{".contains(ch));
    let open_brackets = a.matches('(').count() > a.matches(')').count();
    let last_word = a
        .rsplit(char::is_whitespace)
        .next()
        .unwrap_or("")
        .to_lowercase();
    let breaks = a.ends_with(['-', '/', ',', '(', '&'])
        || open_brackets
        || [
            "and", "of", "the", "for", "to", "in", "on", "or", "per", "by", "with", "from",
        ]
        .contains(&last_word.as_str());
    opens || breaks
}

/// Header cells wrapped over several lines ("Product" / "(active substance," / "strains)")
/// come out as one header row per line, each with as many filled cells as the line above, which
/// [`fold_wrapped_rows`] (fewer cells, looser than the body pitch) leaves alone. In the header
/// area (the leading rows without numbers, and any marked header rows; at most four), a row
/// folds into the one above when every filled cell sits under a filled cell, none is a number,
/// each starts within 0.6x its own line height of the cell above's bottom, and each reads as a
/// continuation ([`continues`]): wrapped lines, not a second header level (that one adds cells
/// under empty slots) nor a text-only table's next row.
fn fold_header_wraps(cells: &mut Vec<GridCell>, head: &mut Vec<usize>, n_rows: &mut usize) {
    let filled = |cells: &[GridCell], r: usize| -> Vec<usize> {
        (0..cells.len())
            .filter(|&k| cells[k].r0 == r && cells[k].r1 == r && !cells[k].text.trim().is_empty())
            .collect()
    };
    // the header area: the leading rows without numbers (at most four), and any marked header
    // rows at the top
    let zone = |cells: &[GridCell], head: &[usize], n_rows: usize| -> usize {
        let mut marked = 0;
        while marked < n_rows && head.contains(&marked) {
            marked += 1;
        }
        let mut z = 0;
        while z < n_rows.min(4) {
            let f = filled(cells, z);
            if f.is_empty() || f.iter().any(|&k| is_number(&cells[k].text)) {
                break;
            }
            z += 1;
        }
        z.max(marked).min(4)
    };
    let mut r = 1;
    while r < zone(cells, head, *n_rows) {
        let here = filled(cells, r);
        let span_starts = cells.iter().any(|c| c.r0 == r && c.r1 > r);
        let mut pairs = Vec::new();
        // one lone filled cell in a wider row is a body cell's wrapped tail more often than a
        // header line (a header wraps in several columns at once)
        let n_cols = cells.iter().map(|c| c.c1 + 1).max().unwrap_or(0);
        let ok = (here.len() >= 2 || n_cols <= 2)
            && !span_starts
            && here.iter().all(|&k| {
                let c = &cells[k];
                let above = (0..cells.len()).find(|&j| {
                    cells[j].r0 < r
                        && r - 1 <= cells[j].r1
                        && cells[j].c0 <= c.c0
                        && c.c1 <= cells[j].c1
                        && !cells[j].text.trim().is_empty()
                });
                match above {
                    Some(j) if !is_number(&c.text) && !starts_item(&c.text) => {
                        let line_h = (c.bx[3] - c.bx[1]).max(1e-6);
                        pairs.push((k, j));
                        c.bx[1] - cells[j].bx[3] < 0.6 * line_h
                            && continues(&cells[j].text, &c.text)
                    }
                    _ => false,
                }
            });
        if !ok {
            r += 1;
            continue;
        }
        for (k, j) in pairs {
            let (t, b) = (cells[k].text.trim().to_string(), cells[k].bx);
            let c = &mut cells[j];
            c.text = format!("{} {}", c.text.trim(), t);
            c.bx = [
                c.bx[0].min(b[0]),
                c.bx[1].min(b[1]),
                c.bx[2].max(b[2]),
                c.bx[3].max(b[3]),
            ];
        }
        remove_row(cells, head, n_rows, r);
    }
}

/// Python `html.escape(text, quote=True)`.
fn escape(t: &str) -> String {
    let mut s = String::with_capacity(t.len());
    for ch in t.chars() {
        match ch {
            '&' => s.push_str("&amp;"),
            '<' => s.push_str("&lt;"),
            '>' => s.push_str("&gt;"),
            '"' => s.push_str("&quot;"),
            '\'' => s.push_str("&#x27;"),
            c => s.push(c),
        }
    }
    s
}

impl Grid {
    /// The grid as HTML, exactly as tn_core.decode_grid writes it.
    pub fn to_html(&self) -> String {
        let mut at = std::collections::HashMap::new();
        let mut taken = std::collections::HashSet::new();
        for (k, c) in self.cells.iter().enumerate() {
            at.insert((c.r0, c.c0), k);
            for r in c.r0..=c.r1 {
                for q in c.c0..=c.c1 {
                    taken.insert((r, q));
                }
            }
        }
        let mut out = vec!["<table>".to_string()];
        for r in 0..self.n_rows {
            let tag = if self.head_rows.contains(&r) {
                "th"
            } else {
                "td"
            };
            let mut tds = String::new();
            let mut q = 0;
            while q < self.n_cols {
                if let Some(&k) = at.get(&(r, q)) {
                    let c = &self.cells[k];
                    let mut attr = String::new();
                    if c.r1 > r {
                        attr.push_str(&format!(" rowspan=\"{}\"", c.r1 - r + 1));
                    }
                    if c.c1 > q {
                        attr.push_str(&format!(" colspan=\"{}\"", c.c1 - q + 1));
                    }
                    tds.push_str(&format!("<{tag}{attr}>{}</{tag}>", escape(&c.text)));
                    q = c.c1 + 1;
                } else if taken.contains(&(r, q)) {
                    q += 1;
                } else {
                    tds.push_str(&format!("<{tag}></{tag}>"));
                    q += 1;
                }
            }
            out.push(format!("<tr>{tds}</tr>"));
        }
        out.push("</table>".to_string());
        out.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Four ruled rows, the second's label wrapping onto a fifth line with no rule above it:
    /// the wrap folds into its row; a table with no interior rules is left alone.
    #[test]
    fn merge_unruled_rows_folds_wrapped_lines() {
        let cell = |r: usize, c: usize, t: &str, y: f64| GridCell {
            r0: r,
            c0: c,
            r1: r,
            c1: c,
            text: t.into(),
            bx: [
                10.0 + 100.0 * c as f64,
                y,
                90.0 + 100.0 * c as f64,
                y + 10.0,
            ],
        };
        let rows = [
            ("a", "1", 0.0),
            ("b first", "2", 20.0),
            ("b second", "", 32.0),
            ("c", "3", 50.0),
            ("d", "4", 70.0),
        ];
        let mk = || Grid {
            cells: rows
                .iter()
                .enumerate()
                .flat_map(|(r, &(l, v, y))| [cell(r, 0, l, y), cell(r, 1, v, y)])
                .collect(),
            head_rows: vec![0],
            n_rows: 5,
            n_cols: 2,
        };
        let rules: Vec<[f32; 3]> = [15.0, 45.0, 65.0]
            .iter()
            .map(|&y| [y, 10.0, 190.0])
            .collect();
        let mut g = mk();
        g.merge_unruled_rows(&rules);
        assert_eq!(g.n_rows, 4);
        let b: Vec<&str> = g
            .cells
            .iter()
            .filter(|c| c.r0 == 1)
            .map(|c| c.text.as_str())
            .collect();
        assert_eq!(b, ["b first b second", "2"]);
        let mut g = mk();
        g.merge_unruled_rows(&[]);
        assert_eq!(g.n_rows, 5);
    }

    /// The exhaustive average linkage `avg_link` replaced (rescans every live pair per merge):
    /// the reference its merges, order and sums must match exactly.
    fn avg_link_reference(p: &[f64], n: usize, thresh: f64) -> Vec<Vec<usize>> {
        let mut groups: Vec<Vec<usize>> = (0..n).map(|i| vec![i]).collect();
        let mut s = p.to_vec();
        let mut cnt = vec![1f64; n * n];
        let mut alive: Vec<usize> = (0..n).collect();
        while alive.len() > 1 {
            let m = alive.len();
            let (mut best, mut ba, mut bb) = (f64::NEG_INFINITY, 0, 0);
            for a in 0..m {
                for b in 0..m {
                    if a == b {
                        continue;
                    }
                    let (ga, gb) = (alive[a], alive[b]);
                    let v = s[ga * n + gb] / cnt[ga * n + gb];
                    if v > best {
                        best = v;
                        ba = a;
                        bb = b;
                    }
                }
            }
            if best <= thresh {
                break;
            }
            let (ga, gb) = (alive[ba], alive[bb]);
            let moved = std::mem::take(&mut groups[gb]);
            groups[ga].extend(moved);
            for k in 0..n {
                s[ga * n + k] += s[gb * n + k];
                cnt[ga * n + k] += cnt[gb * n + k];
            }
            for k in 0..n {
                s[k * n + ga] += s[k * n + gb];
                cnt[k * n + ga] += cnt[k * n + gb];
            }
            alive.remove(bb);
        }
        alive
            .iter()
            .map(|&g| {
                let mut v = groups[g].clone();
                v.sort_unstable();
                v
            })
            .collect()
    }

    #[test]
    fn avg_link_matches_reference() {
        // xorshift: deterministic matrices without a rand dependency
        let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next = move || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        let levels = [-2.0, -1.0, -0.5, 0.0, 0.25, 0.5, 1.0, 2.0];
        for case in 0..400 {
            let n = 1 + (next() % 80) as usize;
            let symmetric = case % 4 != 3;
            let discrete = case % 2 == 0; // small value set: many exact ties
            let mut p = vec![0f64; n * n];
            for i in 0..n {
                for j in 0..n {
                    if symmetric && j < i {
                        p[i * n + j] = p[j * n + i];
                        continue;
                    }
                    p[i * n + j] = if discrete {
                        levels[(next() % levels.len() as u64) as usize]
                    } else {
                        (next() % 20_001) as f64 / 5_000.0 - 2.0
                    };
                }
            }
            for thresh in [0.0, -0.75, 0.6] {
                assert_eq!(
                    avg_link(&p, n, thresh),
                    avg_link_reference(&p, n, thresh),
                    "case {case} n {n} symmetric {symmetric} discrete {discrete} thresh {thresh}"
                );
            }
        }
    }

    fn hcell(r: usize, c: usize, text: &str, y0: f64, y1: f64) -> GridCell {
        GridCell {
            r0: r,
            c0: c,
            r1: r,
            c1: c,
            text: text.into(),
            bx: [c as f64 * 100.0, y0, c as f64 * 100.0 + 90.0, y1],
        }
    }

    #[test]
    fn wrapped_header_lines_fold() {
        // "Product" / "(active substance(s))" and "Recommendation" / "(additions to text)"
        let mut cells = vec![
            hcell(0, 0, "Product", 76.6, 83.6),
            hcell(0, 1, "Date", 76.6, 83.6),
            hcell(0, 2, "Recommendation", 76.6, 83.6),
            hcell(1, 0, "(active substance(s))", 87.5, 96.2),
            hcell(1, 2, "(additions to text)", 87.5, 96.2),
            hcell(2, 0, "Purevax", 110.0, 118.0),
            hcell(2, 1, "5-7 October 2021", 110.0, 118.0),
            hcell(2, 2, "Section 4.6", 110.0, 118.0),
        ];
        let (mut head, mut n) = (vec![0], 3);
        fold_header_wraps(&mut cells, &mut head, &mut n);
        assert_eq!(n, 2);
        assert!(
            cells
                .iter()
                .any(|c| c.text == "Product (active substance(s))")
        );
    }

    #[test]
    fn name_lists_and_lone_tails_do_not_fold() {
        // a text-only list: no continuation cue
        let mut list = vec![
            hcell(0, 0, "Alfa Romeo", 10.0, 18.0),
            hcell(0, 1, "Audi", 10.0, 18.0),
            hcell(1, 0, "Buick Reatta", 20.0, 28.0),
            hcell(1, 1, "Cadillac", 20.0, 28.0),
        ];
        let (mut head, mut n) = (vec![], 2);
        fold_header_wraps(&mut list, &mut head, &mut n);
        assert_eq!(n, 2);
        // one lowercase tail in a wide row: a body cell's wrap, not a header line
        let mut tail = vec![
            hcell(0, 0, "Country", 10.0, 18.0),
            hcell(0, 1, "Sector", 10.0, 18.0),
            hcell(0, 2, "Restrictions", 10.0, 18.0),
            hcell(1, 2, "right required to acquire land", 20.0, 28.0),
        ];
        let (mut head, mut n) = (vec![0], 2);
        fold_header_wraps(&mut tail, &mut head, &mut n);
        assert_eq!(n, 2);
    }
}
