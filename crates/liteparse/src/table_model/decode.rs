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
