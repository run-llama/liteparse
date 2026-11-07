//! Cross-region table re-merge.
//!
//! When the XY-cut commits a V-cut *through* a table (the column gutter of the
//! table looks like a layout gutter), the table's lines land in two or more
//! sibling leaves and no per-region detector ever sees the rows whole. This
//! pass runs before region grouping: it finds side-by-side leaf groups whose
//! baselines align across the cut, fuses the same-baseline lines back into
//! single rows, and validates that the fused zone actually classifies as a
//! table. Only on successful validation are the original lines replaced — a
//! failed candidate leaves the page untouched, so the V-cut's wins on
//! multi-column prose are preserved.

use super::blocks::{Block, Cell};
use super::tables::{TableRun, detect_tables};
use crate::types::{ProjectedLine, Rect};

/// Union of the boxes of every line in `lines`, or `None` when empty.
fn union_line_rects(lines: &[&ProjectedLine]) -> Option<Rect> {
    let mut out: Option<Rect> = None;
    for l in lines {
        Rect::extend(&mut out, &l.bbox);
    }
    out
}

/// Minimum fraction of the smaller side's y-band that must overlap the other
/// side's. Side-by-side table halves overlap almost fully; a sidebar next to
/// the tail of a column does not.
const CR_MIN_Y_OVERLAP_FRAC: f32 = 0.5;
/// Minimum horizontal gap between the two sides (the sliced gutter).
const CR_MIN_COL_GAP_PT: f32 = 8.0;
/// Baseline-cluster tolerance as a fraction of the median line height.
const CR_ROW_TOL_FACTOR: f32 = 0.6;
/// At least this many baselines must align across the cut.
const CR_MIN_ALIGNED_ROWS: usize = 3;
/// ...and they must account for this fraction of the sparser side's lines.
const CR_MIN_ALIGNED_FRAC: f32 = 0.4;
/// A side "looks like prose" when it has at least this many lines...
const CR_PROSE_MIN_LINES: usize = 3;
/// ...whose median width fills this fraction of the side's width...
const CR_PROSE_WIDTH_FRAC: f32 = 0.7;
/// ...and whose summed line heights fill this fraction of the y-band.
const CR_PROSE_VFILL: f32 = 0.55;
/// A single leaf judged as prose (see the per-leaf gate) needs a median line of this many
/// characters: table cells ("1,644,143", "Rooms Occupied") stay under it, a narrow newsletter
/// column's lines ("based on generations of") run 20 and more.
const CR_PROSE_LEAF_MIN_CHARS: f32 = 16.0;
/// A prose side is dominated by single-cell lines: when this fraction or more
/// of a side's lines split into ≥2 cells, it's a sliced table half (row
/// fragments with internal column gaps), not flowing text.
const CR_PROSE_MAX_MULTI_CELL_FRAC: f32 = 0.3;
/// Validation A: the detected table run(s) must cover this fraction of the
/// fused lines.
const CR_VALIDATE_MIN_COVERAGE: f32 = 0.6;
/// Validation A: reject a detected run when more than this fraction of its
/// non-empty cells run ≥ `CR_LONG_CELL_WORDS` words. Fused newspaper /
/// reference columns align into convincing grids, but their "cells" are
/// sentence fragments; real table cells are short.
const CR_VALIDATE_MAX_LONG_CELL_FRAC: f32 = 0.3;
const CR_LONG_CELL_WORDS: usize = 5;
/// Direct 2-col path: max chars for a left-column label cell.
const CR_LABEL_MAX_CHARS: usize = 60;
/// Direct 2-col path: minimum row count.
const CR_TWO_COL_MIN_ROWS: usize = 3;
/// A left-side line continues the previous row's label when its top is within
/// this multiple of line height of the previous label line's bottom.
const CR_LABEL_WRAP_GAP_FACTOR: f32 = 1.5;

/// A validated cross-region merge: replace `lines[start..end]` with `merged`
/// (all sharing one synthetic region path) and hand `runs` (indices local to
/// `merged`) to the region classifier so the table emission is guaranteed.
pub(super) struct CrossRegionMerge {
    pub(super) start: usize,
    pub(super) end: usize,
    pub(super) merged: Vec<ProjectedLine>,
    pub(super) runs: Vec<TableRun>,
}

struct Leaf {
    start: usize,
    end: usize,
    x0: f32,
    x1: f32,
    y0: f32,
    y1: f32,
}

/// Maximum x-overlap between sibling V-cut slices, as a fraction of the
/// narrower side's width.
const CR_SIBLING_MAX_X_OVERLAP_FRAC: f32 = 0.4;

/// True when two leaves are direct children of the same region-tree node and
/// overlap only modestly in x — the signature of a V-cut whose slices were
/// widened by straddling rows.
fn sibling_v_cut_pair(a: &Leaf, b: &Leaf, lines: &[ProjectedLine]) -> bool {
    let pa = &lines[a.start].region_path;
    let pb = &lines[b.start].region_path;
    if pa.len() != pb.len() || pa.is_empty() || pa[..pa.len() - 1] != pb[..pb.len() - 1] {
        return false;
    }
    let x_overlap = a.x1.min(b.x1) - a.x0.max(b.x0);
    let narrow = (a.x1 - a.x0).min(b.x1 - b.x0).max(1.0);
    x_overlap < CR_SIBLING_MAX_X_OVERLAP_FRAC * narrow
}

fn build_leaves(lines: &[ProjectedLine]) -> Vec<Leaf> {
    let mut leaves = Vec::new();
    let mut s = 0;
    while s < lines.len() {
        let path = &lines[s].region_path;
        let mut e = s + 1;
        while e < lines.len() && lines[e].region_path == *path {
            e += 1;
        }
        let (mut x0, mut x1) = (f32::INFINITY, f32::NEG_INFINITY);
        let (mut y0, mut y1) = (f32::INFINITY, f32::NEG_INFINITY);
        for l in &lines[s..e] {
            x0 = x0.min(l.bbox.x);
            x1 = x1.max(l.bbox.x + l.bbox.width);
            y0 = y0.min(l.bbox.y);
            y1 = y1.max(l.bbox.y + l.bbox.height);
        }
        leaves.push(Leaf {
            start: s,
            end: e,
            x0,
            x1,
            y0,
            y1,
        });
        s = e;
    }
    leaves
}

fn median(mut v: Vec<f32>) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.total_cmp(b));
    v[v.len() / 2]
}

/// Longest common prefix of the region paths of all lines in the set.
fn common_path_prefix(lines: &[&ProjectedLine]) -> Vec<u16> {
    let mut prefix: Vec<u16> = lines[0].region_path.clone();
    for l in &lines[1..] {
        let n = prefix
            .iter()
            .zip(l.region_path.iter())
            .take_while(|(a, b)| a == b)
            .count();
        prefix.truncate(n);
    }
    prefix
}

/// Does this side's line set look like a column of flowing prose? Dense
/// vertical fill + lines that fill the column width. A table fragment has
/// short, sparse lines on at least one side.
fn side_prose_metrics(side: &[&ProjectedLine], side_x0: f32, side_x1: f32) -> (f32, f32) {
    let side_width = (side_x1 - side_x0).max(1.0);
    let med_width = median(side.iter().map(|l| l.bbox.width).collect());
    let (mut y0, mut y1) = (f32::INFINITY, f32::NEG_INFINITY);
    let mut sum_h = 0.0f32;
    for l in side {
        y0 = y0.min(l.bbox.y);
        y1 = y1.max(l.bbox.y + l.bbox.height);
        sum_h += l.bbox.height;
    }
    (med_width / side_width, sum_h / (y1 - y0).max(1.0))
}

fn side_looks_like_prose(side: &[&ProjectedLine], side_x0: f32, side_x1: f32) -> bool {
    if side.len() < CR_PROSE_MIN_LINES {
        return false;
    }
    let (width_fill, vfill) = side_prose_metrics(side, side_x0, side_x1);
    if width_fill < CR_PROSE_WIDTH_FRAC || vfill < CR_PROSE_VFILL {
        return false;
    }
    // A sliced *wide* table's row-halves also fill their side's width and
    // y-band, but they carry internal cell gaps; flowing prose doesn't.
    let multi_cell = side
        .iter()
        .filter(|l| super::tables::split_cells(l).len() >= 2)
        .count();
    (multi_cell as f32) < CR_PROSE_MAX_MULTI_CELL_FRAC * side.len() as f32
}

struct Cluster<'a> {
    left: Vec<&'a ProjectedLine>,
    right: Vec<&'a ProjectedLine>,
}

impl<'a> Cluster<'a> {
    fn members(&self) -> impl Iterator<Item = &&'a ProjectedLine> {
        self.left.iter().chain(self.right.iter())
    }
}

/// Cluster the set's lines into baseline rows: lines whose y-centers fall
/// within `tol` of the cluster seed share a row.
fn cluster_rows<'a>(set: &[(&'a ProjectedLine, bool)], tol: f32) -> Vec<Cluster<'a>> {
    let mut sorted: Vec<&(&ProjectedLine, bool)> = set.iter().collect();
    sorted.sort_by(|a, b| {
        let ya = a.0.bbox.y + a.0.bbox.height * 0.5;
        let yb = b.0.bbox.y + b.0.bbox.height * 0.5;
        ya.total_cmp(&yb)
    });
    let mut clusters: Vec<(f32, Cluster)> = Vec::new();
    for (line, is_left) in sorted {
        let yc = line.bbox.y + line.bbox.height * 0.5;
        match clusters.last_mut() {
            Some((seed_y, c)) if (yc - *seed_y).abs() <= tol => {
                if *is_left {
                    c.left.push(line);
                } else {
                    c.right.push(line);
                }
            }
            _ => {
                let mut c = Cluster {
                    left: Vec::new(),
                    right: Vec::new(),
                };
                if *is_left {
                    c.left.push(line);
                } else {
                    c.right.push(line);
                }
                clusters.push((yc, c));
            }
        }
    }
    clusters.into_iter().map(|(_, c)| c).collect()
}

/// Fuse one baseline cluster into a single `ProjectedLine`: spans concatenate
/// (cell boundaries fall out of the span gaps in `split_cells`), text joins
/// with gap-proportional spacing, styles AND together.
fn fuse_cluster(cluster: &Cluster, path: &[u16]) -> ProjectedLine {
    let mut members: Vec<&ProjectedLine> = cluster.members().copied().collect();
    // Fuse in reading order: a right-to-left row starts at its highest x, so
    // walking left-to-right would emit its cells backwards. `fused.spans` is
    // re-sorted x-ascending at the end either way, keeping cell geometry intact.
    let rtl = crate::bidi::is_rtl_pieces(members.iter().map(|m| m.text.as_str()));
    if rtl {
        members.sort_by(|a, b| b.bbox.x.total_cmp(&a.bbox.x));
    } else {
        members.sort_by(|a, b| a.bbox.x.total_cmp(&b.bbox.x));
    }

    let first = members[0];
    let mut fused = first.clone();
    fused.rtl = rtl;
    fused.region_path = path.to_vec();
    for m in &members[1..] {
        // Gap to the previously-fused run, measured along the reading
        // direction so the space count stays proportional in both.
        let gap = if rtl {
            (fused.bbox.x - (m.bbox.x + m.bbox.width)).max(0.0)
        } else {
            (m.bbox.x - (fused.bbox.x + fused.bbox.width)).max(0.0)
        };
        let approx_char_w = (fused.dominant_font_size * 0.5).max(2.0);
        let n_spaces = ((gap / approx_char_w) as usize).clamp(2, 24);
        fused.text.push_str(&" ".repeat(n_spaces));
        fused.text.push_str(&m.text);
        fused.spans.extend(m.spans.iter().cloned());
        let x1 = (fused.bbox.x + fused.bbox.width).max(m.bbox.x + m.bbox.width);
        let y1 = (fused.bbox.y + fused.bbox.height).max(m.bbox.y + m.bbox.height);
        fused.bbox.x = fused.bbox.x.min(m.bbox.x);
        fused.bbox.y = fused.bbox.y.min(m.bbox.y);
        fused.bbox.width = x1 - fused.bbox.x;
        fused.bbox.height = y1 - fused.bbox.y;
        fused.all_bold &= m.all_bold;
        fused.all_italic &= m.all_italic;
        fused.all_mono &= m.all_mono;
        fused.all_strike &= m.all_strike;
        fused.font_size_is_estimated |= m.font_size_is_estimated;
    }
    fused.spans.sort_by(|a, b| a.x.total_cmp(&b.x));
    fused
}

/// Validation A: the fused zone re-classifies as a table via the standard
/// detectors, covering most of its lines.
fn validate_via_detectors(merged: &[ProjectedLine]) -> Option<Vec<TableRun>> {
    let mut runs = detect_tables(merged);
    super::tables::absorb_cross_region_headers(merged, &mut runs);
    let covered: usize = runs
        .iter()
        .filter(|r| match &r.block {
            Block::Table { header, rows } => {
                let cols = header
                    .as_ref()
                    .map(|h| h.len())
                    .or_else(|| rows.first().map(|row| row.len()))
                    .unwrap_or(0);
                if cols < 2 {
                    return false;
                }
                let (mut cells, mut long) = (0usize, 0usize);
                for row in rows {
                    for cell in row {
                        let t = cell.text.trim();
                        if t.is_empty() {
                            continue;
                        }
                        cells += 1;
                        if t.split_whitespace().count() >= CR_LONG_CELL_WORDS {
                            long += 1;
                        }
                    }
                }
                cells > 0 && (long as f32) <= CR_VALIDATE_MAX_LONG_CELL_FRAC * cells as f32
            }
            _ => false,
        })
        .map(|r| r.end - r.start)
        .sum();
    if covered as f32 >= CR_VALIDATE_MIN_COVERAGE * merged.len() as f32 {
        Some(runs)
    } else {
        None
    }
}

/// Validation B: direct 2-column construction for label/description shapes
/// the standard detectors reject (e.g. labels with list-marker prefixes —
/// `is_label_like` is intentionally strict in normal flow, but here the V-cut
/// geometry has already established two columns). Rows are anchored on
/// left-side lines; a left line within wrap distance of the previous one
/// continues the label cell, right-side lines append to the current row's
/// description cell.
fn try_two_col_direct(clusters: &[Cluster], merged_len: usize, tol: f32) -> Option<Vec<TableRun>> {
    // (left, right, bold). Each side's cell box is the union of the lines that
    // fed it, so wrapped labels and multi-line descriptions report the full band
    // they occupy rather than just their first line.
    let mut rows: Vec<(Cell, Cell, bool)> = Vec::new();
    let mut last_label_bottom: Option<f32> = None;

    for cluster in clusters {
        if !cluster.left.is_empty() {
            let text = cluster
                .left
                .iter()
                .map(|l| l.text.trim())
                .collect::<Vec<_>>()
                .join(" ");
            let top = cluster
                .left
                .iter()
                .map(|l| l.bbox.y)
                .fold(f32::INFINITY, f32::min);
            let bottom = cluster
                .left
                .iter()
                .map(|l| l.bbox.y + l.bbox.height)
                .fold(f32::NEG_INFINITY, f32::max);
            let h = (bottom - top).max(1.0);
            let wraps = match (last_label_bottom, rows.last()) {
                (Some(lb), Some(_)) => top - lb <= CR_LABEL_WRAP_GAP_FACTOR * h.min(tol * 2.0),
                _ => false,
            };
            let left_rect = union_line_rects(&cluster.left);
            if wraps {
                let row = rows.last_mut().unwrap();
                if !row.0.text.is_empty() {
                    row.0.text.push(' ');
                }
                row.0.text.push_str(&text);
                if let Some(r) = &left_rect {
                    Rect::extend(&mut row.0.bbox, r);
                }
            } else {
                let bold = cluster.left.iter().all(|l| l.all_bold);
                rows.push((
                    Cell {
                        text,
                        bbox: left_rect,
                    },
                    Cell::default(),
                    bold,
                ));
            }
            last_label_bottom = Some(bottom);
        }
        if !cluster.right.is_empty() {
            let text = cluster
                .right
                .iter()
                .map(|l| l.text.trim())
                .collect::<Vec<_>>()
                .join(" ");
            if rows.is_empty() {
                rows.push((Cell::default(), Cell::default(), false));
            }
            let row = rows.last_mut().unwrap();
            if !row.1.text.is_empty() {
                row.1.text.push(' ');
            }
            row.1.text.push_str(&text);
            if let Some(r) = &union_line_rects(&cluster.right) {
                Rect::extend(&mut row.1.bbox, r);
            }
        }
    }

    if rows.len() < CR_TWO_COL_MIN_ROWS {
        return None;
    }
    // Labels must stay label-shaped after wrap joins; a left side of long
    // sentences is prose the anti-prose gate missed.
    if rows
        .iter()
        .any(|(l, _, _)| l.text.chars().count() > CR_LABEL_MAX_CHARS)
    {
        return None;
    }
    let both = rows
        .iter()
        .filter(|(l, r, _)| !l.text.is_empty() && !r.text.is_empty())
        .count();
    if both < 2 {
        return None;
    }

    // First row promotes to header when bold or when both cells are short
    // header-ish words ("Area" / "Competence").
    let header = {
        let (l, r, bold) = &rows[0];
        if !l.text.is_empty()
            && !r.text.is_empty()
            && (*bold || (l.text.chars().count() <= 20 && r.text.chars().count() <= 20))
        {
            Some(vec![l.clone(), r.clone()])
        } else {
            None
        }
    };
    let body: Vec<Vec<Cell>> = rows
        .iter()
        .skip(if header.is_some() { 1 } else { 0 })
        .map(|(l, r, _)| vec![l.clone(), r.clone()])
        .collect();
    if body.len() < 2 {
        return None;
    }
    // The run's line indices address the synthetic fused rows, not the page, so
    // its region comes from the clusters the rows were built from.
    let bbox = clusters
        .iter()
        .flat_map(|c| c.members())
        .fold(None, |mut acc: Option<Rect>, l| {
            Rect::extend(&mut acc, &l.bbox);
            acc
        });
    Some(vec![TableRun {
        start: 0,
        end: merged_len,
        body_start: if header.is_some() { 1 } else { 0 },
        bbox,
        block: Block::Table { header, rows: body },
        interstitials: Vec::new(),
    }])
}

/// Find one validated cross-region table merge on this page, or `None`.
/// Callers may re-invoke on the spliced result to catch multiple sliced
/// tables per page.
pub(super) fn find_cross_region_table_merge(lines: &[ProjectedLine]) -> Option<CrossRegionMerge> {
    let debug = *super::flags::DEBUG_CROSS_REGION;
    let leaves = build_leaves(lines);
    if debug {
        for (k, l) in leaves.iter().enumerate() {
            eprintln!(
                "[cross-region] leaf {k}: lines [{},{}) x=[{:.1},{:.1}] y=[{:.1},{:.1}] path={:?}",
                l.start, l.end, l.x0, l.x1, l.y0, l.y1, lines[l.start].region_path
            );
        }
    }
    if leaves.len() < 2 {
        return None;
    }

    for i in 0..leaves.len() {
        for j in (i + 1)..leaves.len() {
            let (a, b) = (&leaves[i], &leaves[j]);
            if is_layout_region(&lines[a.start]) || is_layout_region(&lines[b.start]) {
                continue;
            }
            let (left, right) = if a.x1 <= b.x0 - CR_MIN_COL_GAP_PT {
                (a, b)
            } else if b.x1 <= a.x0 - CR_MIN_COL_GAP_PT {
                (b, a)
            } else if sibling_v_cut_pair(a, b, lines) {
                // Direct siblings of one region-tree node that overlap in y
                // can only come from a V-cut. Their bbox x-extents may overlap
                // (table rows straddling the cut widen the leaf), so the
                // disjointness test above misses them.
                if a.x0 <= b.x0 { (a, b) } else { (b, a) }
            } else {
                continue;
            };
            let ov = left.y1.min(right.y1) - left.y0.max(right.y0);
            let min_h = (left.y1 - left.y0).min(right.y1 - right.y0).max(1.0);
            if ov < CR_MIN_Y_OVERLAP_FRAC * min_h {
                continue;
            }
            // For disjoint sides this is the gutter midpoint; for overlapping
            // sibling slices it's the midpoint of the contested band.
            let cut_x = (left.x1.min(right.x1) + right.x0.max(left.x0)) * 0.5;
            // Union band: a tall side may pair with the first of several
            // stacked leaves on the other side; the extension below pulls the
            // rest in, so the band must cover both sides fully.
            let band = (left.y0.min(right.y0) - 2.0, left.y1.max(right.y1) + 2.0);
            let stays_on_one_side = |leaf: &Leaf| {
                let width = (leaf.x1 - leaf.x0).max(1.0);
                let center = (leaf.x0 + leaf.x1) * 0.5;
                let overhang = if center < cut_x {
                    (leaf.x1 - cut_x).max(0.0)
                } else {
                    (cut_x - leaf.x0).max(0.0)
                };
                overhang <= CR_SIBLING_MAX_X_OVERLAP_FRAC * width
            };

            // Extend the pair to every leaf living inside the overlap band and
            // entirely on one side of the cut (a side may itself be H-cut into
            // multiple leaves).
            let mut set_leaves: Vec<usize> = Vec::new();
            for (k, leaf) in leaves.iter().enumerate() {
                let leaf_h = (leaf.y1 - leaf.y0).max(1.0);
                let leaf_ov = leaf.y1.min(band.1) - leaf.y0.max(band.0);
                if leaf_ov < 0.5 * leaf_h || is_layout_region(&lines[leaf.start]) {
                    continue;
                }
                // The leaf must sit predominantly on one side of the cut:
                // its overhang across the cut stays within the straddle
                // allowance (widened slices), measured against its own width.
                if stays_on_one_side(leaf) {
                    set_leaves.push(k);
                }
            }
            if set_leaves.len() < 2 {
                continue;
            }

            // Close over interleaved leaves inside the selected line range.
            // Do not extend the y-band here: only the original candidate set
            // uses that test, while closure admits intervening leaves that
            // remain predominantly on one side of the same cut.
            let mut lo = set_leaves.iter().map(|&k| leaves[k].start).min().unwrap();
            let mut hi = set_leaves.iter().map(|&k| leaves[k].end).max().unwrap();
            loop {
                let covered: usize = set_leaves
                    .iter()
                    .map(|&k| leaves[k].end - leaves[k].start)
                    .sum();
                if covered == hi - lo {
                    break;
                }
                let additions: Vec<usize> = leaves
                    .iter()
                    .enumerate()
                    .filter(|(k, leaf)| {
                        !set_leaves.contains(k)
                            && !is_layout_region(&lines[leaf.start])
                            && leaf.start >= lo
                            && leaf.end <= hi
                            && stays_on_one_side(leaf)
                    })
                    .map(|(k, _)| k)
                    .collect();
                if additions.is_empty() {
                    break;
                }
                if debug {
                    for &k in &additions {
                        eprintln!(
                            "[cross-region] close cut@{cut_x:.1}: add leaf {k} lines [{},{})",
                            leaves[k].start, leaves[k].end
                        );
                    }
                }
                set_leaves.extend(additions);
                lo = set_leaves.iter().map(|&k| leaves[k].start).min().unwrap();
                hi = set_leaves.iter().map(|&k| leaves[k].end).max().unwrap();
            }
            let covered: usize = set_leaves
                .iter()
                .map(|&k| leaves[k].end - leaves[k].start)
                .sum();
            if covered != hi - lo {
                continue;
            }

            // Split the set's lines into sides of the cut.
            let mut set: Vec<(&ProjectedLine, bool)> = Vec::new();
            for &k in &set_leaves {
                let leaf = &leaves[k];
                let is_left = (leaf.x0 + leaf.x1) * 0.5 < cut_x;
                for l in &lines[leaf.start..leaf.end] {
                    set.push((l, is_left));
                }
            }
            let left_lines: Vec<&ProjectedLine> =
                set.iter().filter(|(_, s)| *s).map(|(l, _)| *l).collect();
            let right_lines: Vec<&ProjectedLine> =
                set.iter().filter(|(_, s)| !*s).map(|(l, _)| *l).collect();
            if left_lines.is_empty() || right_lines.is_empty() {
                continue;
            }

            // Anti-prose gate: two side-by-side prose columns must stay split.
            let lx0 = left_lines
                .iter()
                .map(|l| l.bbox.x)
                .fold(f32::INFINITY, f32::min);
            let rx0 = right_lines
                .iter()
                .map(|l| l.bbox.x)
                .fold(f32::INFINITY, f32::min);
            let lx1 = left_lines
                .iter()
                .map(|l| l.bbox.x + l.bbox.width)
                .fold(f32::NEG_INFINITY, f32::max);
            let rx1 = right_lines
                .iter()
                .map(|l| l.bbox.x + l.bbox.width)
                .fold(f32::NEG_INFINITY, f32::max);
            if debug {
                let (lw, lv) = side_prose_metrics(&left_lines, lx0, lx1);
                let (rw, rv) = side_prose_metrics(&right_lines, rx0, rx1);
                eprintln!(
                    "[cross-region] candidate cut@{cut_x:.1}: left n={} wfill={lw:.2} vfill={lv:.2} | right n={} wfill={rw:.2} vfill={rv:.2}",
                    left_lines.len(),
                    right_lines.len()
                );
            }
            // Either side reading as flowing prose means the V-cut was right:
            // a body column next to a sidebar / second column must stay split.
            // A side made of several leaves (two columns left of a third) is judged leaf by
            // leaf as well: lumped together its lines fill only part of the side's width. A
            // leaf holding one table column fills its own width too, so a prose leaf must also
            // run to sentence-length lines.
            let leaf_is_prose = || {
                set_leaves.len() > 2
                    && set_leaves.iter().any(|&k| {
                        let leaf = &leaves[k];
                        let ls: Vec<&ProjectedLine> = lines[leaf.start..leaf.end].iter().collect();
                        let chars = median(
                            ls.iter()
                                .map(|l| l.text.trim().chars().count() as f32)
                                .collect(),
                        );
                        chars >= CR_PROSE_LEAF_MIN_CHARS
                            && side_looks_like_prose(&ls, leaf.x0, leaf.x1)
                    })
            };
            if side_looks_like_prose(&left_lines, lx0, lx1)
                || side_looks_like_prose(&right_lines, rx0, rx1)
                || leaf_is_prose()
            {
                if debug {
                    eprintln!("[cross-region] reject cut@{cut_x:.1}: a side is prose-like");
                }
                continue;
            }

            // Row alignment across the cut.
            let tol = CR_ROW_TOL_FACTOR
                * median(set.iter().map(|(l, _)| l.bbox.height).collect()).max(1.0);
            let clusters = cluster_rows(&set, tol);
            let aligned = clusters
                .iter()
                .filter(|c| !c.left.is_empty() && !c.right.is_empty())
                .count();
            let min_side = left_lines.len().min(right_lines.len());
            if aligned < CR_MIN_ALIGNED_ROWS
                || (aligned as f32) < CR_MIN_ALIGNED_FRAC * min_side as f32
            {
                if debug {
                    eprintln!(
                        "[cross-region] reject cut@{cut_x:.1}: aligned={aligned} of min_side={min_side}"
                    );
                }
                continue;
            }

            // Fuse and validate.
            let all: Vec<&ProjectedLine> = set.iter().map(|(l, _)| *l).collect();
            let path = common_path_prefix(&all);
            let merged: Vec<ProjectedLine> =
                clusters.iter().map(|c| fuse_cluster(c, &path)).collect();

            let runs = validate_via_detectors(&merged)
                .or_else(|| try_two_col_direct(&clusters, merged.len(), tol));
            let Some(runs) = runs else {
                if debug {
                    eprintln!(
                        "[cross-region] reject cut@{cut_x:.1}: fused zone failed table validation"
                    );
                }
                continue;
            };
            if debug {
                eprintln!(
                    "[cross-region] MERGE cut@{cut_x:.1}: lines [{lo},{hi}) -> {} fused rows, {} run(s)",
                    merged.len(),
                    runs.len()
                );
            }
            return Some(CrossRegionMerge {
                start: lo,
                end: hi,
                merged,
                runs,
            });
        }
    }
    None
}

/// First element of the synthetic region path given to the lines of one layout
/// table box. Real xy_cut paths start at 0, so these never collide.
pub(super) const LAYOUT_REGION_MARK: u16 = u16::MAX;

pub(super) fn is_layout_region(line: &ProjectedLine) -> bool {
    line.region_path.first() == Some(&LAYOUT_REGION_MARK)
}

/// Padding (points) around a layout box when testing whether a line's center
/// falls inside it. Layout boxes hug the ink; line boxes include leading.
const LT_BOX_PAD_PT: f32 = 3.0;
/// Box lines above the first detected run that may join the header band.
const LT_MAX_HEADER_LINES: usize = 3;
/// A header-band cell longer than this is a caption or prose, not a header.
const LT_HEADER_MAX_CELL_CHARS: usize = 40;
/// The detected runs must cover this share of a box's rows, or the box falls
/// back to the direct 2-column and whitespace-grid builders.
const LT_MIN_COVERAGE: f32 = 0.5;

/// Classify each layout table box as its own table region.
///
/// For every box (in the order given, which callers sort by confidence), the
/// unclaimed lines whose centers fall inside it are fused into baseline rows
/// under one synthetic region path, so no xy_cut V-cut splits them and no
/// merge pass reaches past the box edge. Rows are classified with 2-column
/// runs allowed; box lines above the first run join its header row. When the
/// detectors cover too little of the box, the direct 2-column builder and then
/// a whitespace-column grid are tried. A box that yields no table is left
/// alone, as is a box crossed by an interior vertical rule (a ruled grid the
/// ruled detectors already handle). Returns the rewritten line list and the
/// runs per synthetic path, in the shape `classify_region` takes as
/// `precomputed_tables`.
pub(super) fn apply_layout_table_regions(
    lines: &[ProjectedLine],
    boxes: &[Rect],
    rule_segments: &super::tables::RuleSegments,
) -> Option<(Vec<ProjectedLine>, Vec<(Vec<u16>, Vec<TableRun>)>)> {
    let mut claimed = vec![false; lines.len()];
    // (first original index, fused rows) per accepted box.
    let mut accepted: Vec<(usize, Vec<ProjectedLine>)> = Vec::new();
    let mut runs_by_path: Vec<(Vec<u16>, Vec<TableRun>)> = Vec::new();
    for (k, b) in boxes.iter().enumerate() {
        if super::tables::has_interior_vertical_rule(rule_segments, b) {
            continue;
        }
        let inside: Vec<usize> = (0..lines.len())
            .filter(|&i| {
                let l = &lines[i];
                let cx = l.bbox.x + l.bbox.width * 0.5;
                let cy = l.bbox.y + l.bbox.height * 0.5;
                !claimed[i]
                    && !l.text.trim().is_empty()
                    && !super::classify::is_rotated_line(l)
                    && cx >= b.x - LT_BOX_PAD_PT
                    && cx <= b.x + b.width + LT_BOX_PAD_PT
                    && cy >= b.y - LT_BOX_PAD_PT
                    && cy <= b.y + b.height + LT_BOX_PAD_PT
            })
            .collect();
        if inside.len() < 2 {
            continue;
        }
        let mid_x = b.x + b.width * 0.4;
        let set: Vec<(&ProjectedLine, bool)> = inside
            .iter()
            .map(|&i| (&lines[i], lines[i].bbox.x < mid_x))
            .collect();
        let tol =
            CR_ROW_TOL_FACTOR * median(set.iter().map(|(l, _)| l.bbox.height).collect()).max(1.0);
        let clusters = cluster_rows(&set, tol);
        let path = vec![LAYOUT_REGION_MARK, k as u16];
        let merged: Vec<ProjectedLine> = clusters.iter().map(|c| fuse_cluster(c, &path)).collect();
        let Some(runs) = layout_box_runs(&merged, &clusters, tol) else {
            continue;
        };
        for &i in &inside {
            claimed[i] = true;
        }
        accepted.push((inside[0], merged));
        runs_by_path.push((path, runs));
    }
    if accepted.is_empty() {
        return None;
    }
    // Each box's rows replace its lines at the position of its first line.
    let mut out: Vec<ProjectedLine> = Vec::with_capacity(lines.len());
    let mut at: std::collections::HashMap<usize, Vec<ProjectedLine>> =
        accepted.into_iter().collect();
    for (i, line) in lines.iter().enumerate() {
        if let Some(rows) = at.remove(&i) {
            out.extend(rows);
        }
        if !claimed[i] {
            out.push(line.clone());
        }
    }
    Some((out, runs_by_path))
}

/// Table runs for one box's fused rows, or `None` when nothing table-shaped
/// is found. See `apply_layout_table_regions`.
///
/// Detection is seeded at each of the box's first `LT_MAX_HEADER_LINES + 1`
/// rows: a header row with fewer cells than the body (years over value
/// columns, nothing over the label column) would otherwise set the column
/// tracks and push the label cells out of the table. Skipped rows join the
/// first run's header. The seed that puts the most box text into table cells
/// wins; on a tie, the one with fewer runs (a header split off as its own
/// table), then the earliest.
fn layout_box_runs(
    merged: &[ProjectedLine],
    clusters: &[Cluster],
    tol: f32,
) -> Option<Vec<TableRun>> {
    let mut best: Option<(usize, Vec<TableRun>)> = None;
    for skip in 0..=LT_MAX_HEADER_LINES.min(merged.len().saturating_sub(2)) {
        let mut runs = super::tables::detect_tables_in_box(&merged[skip..]);
        runs.retain(|r| run_columns(r) >= 2);
        for r in &mut runs {
            r.start += skip;
            r.end += skip;
            r.body_start += skip;
        }
        if let Some(first) = runs.first_mut() {
            absorb_box_header(merged, first);
        }
        let score: usize = runs.iter().map(run_text_chars).sum();
        let better = best
            .as_ref()
            .is_none_or(|(b, prev)| score > *b || (score == *b && runs.len() < prev.len()));
        if score > 0 && better {
            best = Some((score, runs));
        }
    }
    let runs = best.map(|(_, r)| r).unwrap_or_default();
    let covered: usize = runs.iter().map(|r| r.end - r.start).sum();
    if (covered as f32) < LT_MIN_COVERAGE * merged.len() as f32
        && let Some(r) =
            try_two_col_direct(clusters, merged.len(), tol).or_else(|| whitespace_grid(merged))
    {
        return Some(r);
    }
    (!runs.is_empty()).then_some(runs)
}

/// Non-whitespace characters in a run's header and body cells.
fn run_text_chars(run: &TableRun) -> usize {
    match &run.block {
        Block::Table { header, rows } => header
            .iter()
            .flatten()
            .chain(rows.iter().flatten())
            .map(|c| c.text.chars().filter(|ch| !ch.is_whitespace()).count())
            .sum(),
        _ => 0,
    }
}

fn run_columns(run: &TableRun) -> usize {
    match &run.block {
        Block::Table { header, rows } => header
            .as_ref()
            .map(|h| h.len())
            .into_iter()
            .chain(rows.iter().map(|r| r.len()))
            .max()
            .unwrap_or(0),
        _ => 0,
    }
}

/// Pull the box lines above `run` into its header row. Each band cell goes to
/// the column whose body cells sit nearest its center; several band lines and
/// an existing header join top to bottom per column. Lines with a long cell
/// are prose and stop the pull; a one-cell title line bounds it from above.
pub(super) fn absorb_box_header(merged: &[ProjectedLine], run: &mut TableRun) {
    if run.start == 0 || run.start > LT_MAX_HEADER_LINES {
        return;
    }
    let Block::Table { header, rows } = &mut run.block else {
        return;
    };
    let ncols = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    if ncols < 2 {
        return;
    }
    // Column centers: median x-center of the located body cells per column.
    let centers: Vec<Option<f32>> = (0..ncols)
        .map(|c| {
            let xs: Vec<f32> = rows
                .iter()
                .filter_map(|r| r.get(c))
                .filter(|cell| !cell.text.trim().is_empty())
                .filter_map(|cell| cell.bbox.as_ref().map(|b| b.x + b.width * 0.5))
                .collect();
            (!xs.is_empty()).then(|| median(xs))
        })
        .collect();
    if centers.iter().all(|c| c.is_none()) {
        return;
    }
    // A one-cell line with no digits ("Standard Group") titles the table; it
    // and everything above it stay outside. Year rows keep their digits.
    let band_start = merged[..run.start]
        .iter()
        .rposition(|line| {
            let cells = super::tables::split_cells(line);
            cells.len() == 1 && !cells[0].text.chars().any(|c| c.is_ascii_digit())
        })
        .map_or(0, |j| j + 1);
    if band_start == run.start {
        return;
    }
    let mut band: Vec<Vec<String>> = vec![Vec::new(); ncols];
    let mut band_rect: Option<Rect> = None;
    for line in &merged[band_start..run.start] {
        let cells = super::tables::split_cells(line);
        if cells.is_empty()
            || cells
                .iter()
                .any(|c| c.text.trim().chars().count() > LT_HEADER_MAX_CELL_CHARS)
        {
            return;
        }
        for cell in &cells {
            let cx = (cell.start_x + cell.end_x) * 0.5;
            let col = centers
                .iter()
                .enumerate()
                .filter_map(|(i, c)| c.map(|c| (i, (c - cx).abs())))
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(i, _)| i)
                .unwrap_or(0);
            band[col].push(cell.text.trim().to_string());
        }
        Rect::extend(&mut band_rect, &line.bbox);
    }
    let mut new_header: Vec<Cell> = band
        .into_iter()
        .map(|parts| Cell {
            text: parts.join(" "),
            bbox: None,
        })
        .collect();
    match header.take() {
        Some(old) => {
            for (i, cell) in old.into_iter().enumerate() {
                if i >= new_header.len() {
                    new_header.push(cell);
                } else if !cell.text.trim().is_empty() {
                    let t = &mut new_header[i].text;
                    if !t.is_empty() {
                        t.push(' ');
                    }
                    t.push_str(cell.text.trim());
                    new_header[i].bbox = cell.bbox;
                }
            }
        }
        None => {}
    }
    *header = Some(new_header);
    if let Some(r) = &band_rect {
        Rect::extend(&mut run.bbox, r);
    }
    run.start = band_start;
}

/// Last-resort grid for a layout box: columns are the x-intervals occupied by
/// the cells of multi-cell rows, unioned across rows; every cell goes to the
/// interval holding its center. The first row is the header. Rejects shapes
/// with fewer than two columns or two rows, and prose-like cell text.
fn whitespace_grid(merged: &[ProjectedLine]) -> Option<Vec<TableRun>> {
    let rows: Vec<Vec<super::tables::TableCell>> =
        merged.iter().map(super::tables::split_cells).collect();
    let mut spans: Vec<(f32, f32)> = rows
        .iter()
        .filter(|r| r.len() >= 2)
        .flat_map(|r| r.iter().map(|c| (c.start_x, c.end_x)))
        .collect();
    if spans.len() < 4 {
        return None;
    }
    spans.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut cols: Vec<(f32, f32)> = Vec::new();
    for (x0, x1) in spans {
        match cols.last_mut() {
            Some(last) if x0 <= last.1 => last.1 = last.1.max(x1),
            _ => cols.push((x0, x1)),
        }
    }
    if cols.len() < 2 {
        return None;
    }
    let col_of = |cx: f32| {
        cols.iter()
            .enumerate()
            .min_by(|a, b| {
                let da = if cx < a.1.0 {
                    a.1.0 - cx
                } else {
                    (cx - a.1.1).max(0.0)
                };
                let db = if cx < b.1.0 {
                    b.1.0 - cx
                } else {
                    (cx - b.1.1).max(0.0)
                };
                da.total_cmp(&db)
            })
            .map(|(i, _)| i)
            .unwrap_or(0)
    };
    let (mut cells, mut long) = (0usize, 0usize);
    let mut grid: Vec<Vec<Cell>> = Vec::new();
    for (line, row) in merged.iter().zip(&rows) {
        if row.is_empty() {
            continue;
        }
        let mut out = vec![Cell::default(); cols.len()];
        for c in row {
            let t = c.text.trim();
            cells += 1;
            if t.split_whitespace().count() >= CR_LONG_CELL_WORDS {
                long += 1;
            }
            let slot = &mut out[col_of((c.start_x + c.end_x) * 0.5)];
            if !slot.text.is_empty() {
                slot.text.push(' ');
            }
            slot.text.push_str(t);
            let r = Rect {
                x: c.start_x,
                y: line.bbox.y,
                width: (c.end_x - c.start_x).max(0.0),
                height: line.bbox.height,
            };
            Rect::extend(&mut slot.bbox, &r);
        }
        grid.push(out);
    }
    if grid.len() < 2 || (long as f32) > CR_VALIDATE_MAX_LONG_CELL_FRAC * cells as f32 {
        return None;
    }
    let header = grid.remove(0);
    let bbox = merged.iter().fold(None, |mut acc: Option<Rect>, l| {
        Rect::extend(&mut acc, &l.bbox);
        acc
    });
    Some(vec![TableRun {
        start: 0,
        end: merged.len(),
        body_start: 1.min(merged.len()),
        bbox,
        block: Block::Table {
            header: Some(header),
            rows: grid,
        },
        interstitials: Vec::new(),
    }])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::markdown_layout::test_helpers::{line, line_with_spans};

    fn render_with_regions(lines: Vec<ProjectedLine>, regions: &[Rect]) -> String {
        let page = crate::markdown_layout::test_helpers::page(lines);
        let blocks = crate::markdown_layout::classify_page_with_filters(
            &page,
            &[],
            &std::collections::HashSet::new(),
            &[],
            crate::config::ImageMode::Placeholder,
            &std::collections::HashSet::new(),
            regions,
        );
        crate::markdown_layout::blocks::render_blocks(&blocks)
    }

    /// A label column in its own xy_cut leaf, values and a year header row in
    /// another. The layout box puts labels, values and years in one table.
    fn label_column_lines() -> Vec<ProjectedLine> {
        let mut lines = Vec::new();
        for (i, label) in ["Net sales", "Cost of sales", "Gross margin"]
            .iter()
            .enumerate()
        {
            lines.push(at_region(
                line_with_spans(&[(label, 72.0)], 130.0 + 15.0 * i as f32, 10.0),
                &[0, 0],
            ));
        }
        lines.push(at_region(
            line_with_spans(&[("2025", 300.0), ("2024", 380.0)], 112.0, 10.0),
            &[0, 1],
        ));
        for (i, (a, b)) in [("100", "90"), ("60", "55"), ("40", "35")]
            .iter()
            .enumerate()
        {
            lines.push(at_region(
                line_with_spans(&[(a, 300.0), (b, 380.0)], 130.0 + 15.0 * i as f32, 10.0),
                &[0, 1],
            ));
        }
        lines
    }

    #[test]
    fn layout_box_joins_label_column_and_year_header() {
        let boxed = Rect {
            x: 70.0,
            y: 108.0,
            width: 340.0,
            height: 70.0,
        };
        let md = render_with_regions(label_column_lines(), &[boxed]);
        assert!(md.contains("| Cost of sales | 60 | 55 |"), "{md}");
        assert!(
            md.contains("2025") && md.lines().any(|l| l.starts_with('|') && l.contains("2025")),
            "{md}"
        );
        assert_eq!(md.matches("Net sales").count(), 1, "{md}");
    }

    #[test]
    fn without_layout_box_label_column_stays_outside() {
        let md = render_with_regions(label_column_lines(), &[]);
        assert!(!md.contains("| Cost of sales | 60 | 55 |"), "{md}");
    }

    #[test]
    fn layout_box_title_line_stays_outside_header() {
        let mut lines = vec![line_with_spans(&[("Standard Group", 200.0)], 96.0, 10.0)];
        lines.extend(label_column_lines());
        let boxed = Rect {
            x: 70.0,
            y: 94.0,
            width: 340.0,
            height: 84.0,
        };
        let md = render_with_regions(lines, &[boxed]);
        assert!(md.starts_with("Standard Group"), "{md}");
        assert!(md.contains("| Cost of sales | 60 | 55 |"), "{md}");
    }

    #[test]
    fn layout_box_two_by_two_becomes_table() {
        let lines = vec![
            line_with_spans(&[("Age", 72.0), ("Rate", 200.0)], 100.0, 10.0),
            line_with_spans(&[("35", 72.0), ("1.25", 200.0)], 115.0, 10.0),
        ];
        let boxed = Rect {
            x: 70.0,
            y: 98.0,
            width: 160.0,
            height: 30.0,
        };
        let md = render_with_regions(lines, &[boxed]);
        assert!(
            md.contains("| Age | Rate |") && md.contains("| 35 | 1.25 |"),
            "{md}"
        );
    }

    #[test]
    fn prose_in_a_layout_box_stays_prose() {
        let lines = vec![
            line(
                "The company recognized revenue when control of the goods passed to",
                72.0,
                100.0,
                10.0,
                10.0,
            ),
            line(
                "the customer, which generally occurs upon shipment of the product.",
                72.0,
                113.0,
                10.0,
                10.0,
            ),
        ];
        let boxed = Rect {
            x: 70.0,
            y: 98.0,
            width: 400.0,
            height: 30.0,
        };
        let md = render_with_regions(lines, &[boxed]);
        assert!(!md.contains('|'), "{md}");
    }

    fn at_region(mut l: ProjectedLine, path: &[u16]) -> ProjectedLine {
        l.region_path = path.to_vec();
        l
    }

    /// A 3-col numeric table sliced by a V-cut: label column in one leaf,
    /// two numeric columns in the other. Validation A (standard detectors on
    /// the fused rows) should accept.
    #[test]
    fn sliced_numeric_table_is_merged() {
        let mut lines = Vec::new();
        // Left leaf: row labels.
        for (i, label) in ["MANILA", "CEBU", "CAGAYAN DE ORO", "SUBIC"]
            .iter()
            .enumerate()
        {
            let y = 100.0 + i as f32 * 20.0;
            lines.push(at_region(
                line_with_spans(&[(label, 50.0)], y, 10.0),
                &[0, 0],
            ));
        }
        // Right leaf: two numeric columns per row.
        for (i, (a, b)) in [
            ("2454", "6125"),
            ("1138", "79500"),
            ("958", "13196"),
            ("313", "136"),
        ]
        .iter()
        .enumerate()
        {
            let y = 100.0 + i as f32 * 20.0;
            lines.push(at_region(
                line_with_spans(&[(a, 200.0), (b, 280.0)], y, 10.0),
                &[0, 1],
            ));
        }
        let m = find_cross_region_table_merge(&lines).expect("should merge");
        assert_eq!((m.start, m.end), (0, 8));
        assert_eq!(m.merged.len(), 4);
        assert!(!m.runs.is_empty());
        // Every fused row carries the label and both numbers.
        assert!(m.merged[0].text.contains("MANILA"));
        assert!(m.merged[0].text.contains("2454"));
        assert!(m.merged[0].text.contains("6125"));
    }

    fn interleaved_three_column_table(wide_top_prose: bool) -> Vec<ProjectedLine> {
        let columns = [
            (
                50.0,
                ["Trading", "Projections"],
                ["Rooms Revenue", "Rooms Occupied", "Average Rate", "RevPAR"],
            ),
            (
                200.0,
                ["Year One", "Year Two"],
                ["1,644,143", "1,650,000", "123.45", "87.65"],
            ),
            (
                280.0,
                ["% of TR", "% of TR"],
                ["71.0%", "70.5%", "—", "50.2%"],
            ),
        ];
        let mut lines = Vec::new();
        for (column, (x, top, body)) in columns.iter().enumerate() {
            for (row, text) in top.iter().enumerate() {
                let line = if wide_top_prose && column == 2 && row == 0 {
                    line(&"a ".repeat(80), -200.0, 100.0, 10.0, 10.0)
                } else {
                    line_with_spans(&[(text, *x)], 100.0 + row as f32 * 12.0, 10.0)
                };
                lines.push(at_region(line, &[column as u16, 0]));
            }
            for (row, text) in body.iter().enumerate() {
                lines.push(at_region(
                    line_with_spans(&[(text, *x)], 200.0 + row as f32 * 20.0, 10.0),
                    &[column as u16, 1],
                ));
            }
        }
        lines
    }

    #[test]
    fn interleaved_h_cut_leaves_close_into_a_table() {
        let lines = interleaved_three_column_table(false);
        let m =
            find_cross_region_table_merge(&lines).expect("should close over interleaved leaves");
        assert_eq!(m.merged.len(), 6);
        assert!(m.merged[2].text.contains("Rooms Revenue"));
        assert!(m.merged[2].text.contains("1,644,143"));
        assert!(m.merged[2].text.contains("71.0%"));
        let has_complete_row = m.runs.iter().any(|run| match &run.block {
            Block::Table { header, rows } => header.iter().chain(rows.iter()).any(|row| {
                let text = row
                    .iter()
                    .map(|cell| cell.text.as_str())
                    .collect::<String>();
                text.contains("Rooms Revenue")
                    && text.contains("1,644,143")
                    && text.contains("71.0%")
            }),
            _ => false,
        });
        assert!(
            has_complete_row,
            "table rows should retain labels and both values"
        );
    }

    #[test]
    fn interleaved_table_headers_align_with_merged_body_columns() {
        let columns = [
            (
                50.0,
                ["Trading", "Projections"],
                ["Rooms Revenue", "Rooms Occupied", "Average Rate", "RevPAR"],
            ),
            (
                200.0,
                ["Year One", "Year Two"],
                ["1,644,143", "1,650,000", "123.45", "87.65"],
            ),
            (
                280.0,
                ["% of TR", "% of TR"],
                ["71.0%", "70.5%", "—", "50.2%"],
            ),
        ];
        let mut lines = Vec::new();
        for (column, (x, top, body)) in columns.iter().enumerate() {
            for (row, text) in top.iter().enumerate() {
                lines.push(at_region(
                    line_with_spans(&[(*text, *x)], 160.0 + row as f32 * 12.0, 10.0),
                    &[column as u16, 0],
                ));
            }
            if column == 0 {
                lines.push(at_region(
                    line_with_spans(&[("Revenue", *x)], 188.0, 10.0),
                    &[column as u16, 1],
                ));
            }
            for (row, text) in body.iter().enumerate() {
                lines.push(at_region(
                    line_with_spans(&[(*text, *x)], 200.0 + row as f32 * 20.0, 10.0),
                    &[column as u16, 1],
                ));
            }
        }

        let m = find_cross_region_table_merge(&lines).expect("should merge");
        let header = m
            .runs
            .iter()
            .find_map(|run| match &run.block {
                Block::Table {
                    header: Some(header),
                    ..
                } => Some(header),
                _ => None,
            })
            .expect("aligned year labels should be absorbed as the table header");
        let header_text = header
            .iter()
            .map(|cell| cell.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(header_text.contains("Year One"));
        assert!(header_text.contains("Year Two"));
        let body_text = m
            .runs
            .iter()
            .flat_map(|run| match &run.block {
                Block::Table { rows, .. } => rows
                    .iter()
                    .flat_map(|row| row.iter().map(|cell| cell.text.as_str()))
                    .collect::<Vec<_>>(),
                _ => Vec::new(),
            })
            .collect::<Vec<_>>()
            .join(" ");
        assert!(body_text.contains("1,644,143"));
        assert!(body_text.contains("71.0%"));
    }

    #[test]
    fn cross_region_header_absorption_skips_one_section_label() {
        let lines = vec![
            line_with_spans(&[("Year One", 794.4), ("Total", 875.6)], 100.0, 10.0),
            line_with_spans(&[("Year Two", 794.4), ("Total", 875.6)], 110.0, 10.0),
            line_with_spans(&[("Revenue", 50.0)], 120.0, 10.0),
            line_with_spans(
                &[("Rooms Revenue", 50.0), ("1,644,143", 794.4)],
                140.0,
                10.0,
            ),
            line_with_spans(
                &[("Rooms Occupied", 50.0), ("1,650,000", 794.4)],
                160.0,
                10.0,
            ),
        ];
        let located_cell = |text: &str, x: f32| {
            Cell::located(
                text,
                Rect {
                    x,
                    y: 140.0,
                    width: 20.0,
                    height: 10.0,
                },
            )
        };
        let mut runs = vec![TableRun {
            start: 3,
            end: 5,
            body_start: 3,
            bbox: None,
            block: Block::Table {
                header: None,
                rows: vec![
                    vec![
                        located_cell("Rooms Revenue", 50.0),
                        located_cell("1,644,143", 794.4),
                        Cell::default(),
                        located_cell("2,000,000", 853.8),
                    ],
                    vec![
                        located_cell("Rooms Occupied", 50.0),
                        located_cell("1,650,000", 794.4),
                        Cell::default(),
                        located_cell("2,100,000", 853.8),
                    ],
                ],
            },
            interstitials: Vec::new(),
        }];

        super::super::tables::absorb_cross_region_headers(&lines, &mut runs);

        let run = &runs[0];
        assert_eq!(run.start, 0);
        assert_eq!(run.interstitials[0].text, "Revenue");
        let Block::Table {
            header: Some(header),
            ..
        } = &run.block
        else {
            panic!("aligned year labels should become the table header");
        };
        assert!(header[1].text.contains("Year One"));
        assert!(header[1].text.contains("Year Two"));
        assert!(header[2].text.is_empty());
        assert!(header[3].text.contains("Total"));
    }

    #[test]
    fn interleaved_wide_prose_leaf_prevents_closure() {
        let lines = interleaved_three_column_table(true);
        assert!(find_cross_region_table_merge(&lines).is_none());
    }

    /// Two side-by-side prose columns with coincidentally aligned baselines
    /// must NOT merge — that's the layout the V-cut exists to protect.
    #[test]
    fn two_column_prose_is_not_merged() {
        let mut lines = Vec::new();
        for i in 0..8 {
            let y = 100.0 + i as f32 * 12.0;
            lines.push(at_region(
                line("the quick brown fox jumps over it", 50.0, y, 10.0, 10.0),
                &[0, 0],
            ));
        }
        for i in 0..8 {
            let y = 100.0 + i as f32 * 12.0;
            lines.push(at_region(
                line("a second column of body text here", 250.0, y, 10.0, 10.0),
                &[0, 1],
            ));
        }
        assert!(find_cross_region_table_merge(&lines).is_none());
    }

    /// Label / description shape with list-marker labels (the doc-146 shape):
    /// standard detectors reject it, the direct 2-col path accepts.
    #[test]
    fn sliced_label_description_table_uses_direct_path() {
        let mut lines = Vec::new();
        lines.push(at_region(line("Area", 50.0, 100.0, 10.0, 10.0), &[0, 0]));
        for (i, label) in [
            "1. Embodying values",
            "2. Embracing complexity",
            "3. Envisioning futures",
        ]
        .iter()
        .enumerate()
        {
            let y = 150.0 + i as f32 * 50.0;
            lines.push(at_region(line(label, 50.0, y, 10.0, 10.0), &[0, 0]));
        }
        lines.push(at_region(
            line("Competence", 250.0, 100.0, 10.0, 10.0),
            &[0, 1],
        ));
        for (i, desc) in [
            "1.1 Valuing sustainability 1.2 Supporting fairness",
            "2.1 Systems thinking 2.2 Critical thinking",
            "3.1 Futures literacy 3.2 Adaptability",
        ]
        .iter()
        .enumerate()
        {
            let y = 150.0 + i as f32 * 50.0;
            lines.push(at_region(line(desc, 250.0, y, 10.0, 10.0), &[0, 1]));
        }
        let m = find_cross_region_table_merge(&lines).expect("should merge via direct path");
        assert_eq!(m.runs.len(), 1);
        match &m.runs[0].block {
            Block::Table { header, rows } => {
                let header_texts: Vec<&str> = header
                    .as_ref()
                    .expect("header should be present")
                    .iter()
                    .map(|c| c.as_str())
                    .collect();
                assert_eq!(header_texts, ["Area", "Competence"]);
                assert_eq!(rows.len(), 3);
                assert!(rows[0][1].text.contains("1.1 Valuing sustainability"));
            }
            other => panic!("expected table, got {other:?}"),
        }
    }

    /// A single region (no V-cut) never merges.
    #[test]
    fn single_leaf_is_untouched() {
        let lines: Vec<ProjectedLine> = (0..6)
            .map(|i| {
                at_region(
                    line("text", 50.0, 100.0 + i as f32 * 12.0, 10.0, 10.0),
                    &[0],
                )
            })
            .collect();
        assert!(find_cross_region_table_merge(&lines).is_none());
    }

    /// Vertically stacked leaves (H-cut siblings) never pair: no y-overlap.
    #[test]
    fn stacked_leaves_do_not_pair() {
        let mut lines = Vec::new();
        for i in 0..4 {
            lines.push(at_region(
                line("alpha beta", 50.0, 100.0 + i as f32 * 12.0, 10.0, 10.0),
                &[0],
            ));
        }
        for i in 0..4 {
            lines.push(at_region(
                line("gamma delta", 50.0, 300.0 + i as f32 * 12.0, 10.0, 10.0),
                &[1],
            ));
        }
        assert!(find_cross_region_table_merge(&lines).is_none());
    }
}
