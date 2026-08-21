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
use crate::types::{ProjectedLine, Rect, TextItem};

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
        fused
            .span_item_indices
            .extend(m.span_item_indices.iter().copied());
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
    // Sort spans by x together with their source indices — sorting the two
    // vectors independently would silently break the positional alignment.
    let mut zipped: Vec<(TextItem, usize)> = fused
        .spans
        .drain(..)
        .zip(fused.span_item_indices.drain(..))
        .collect();
    zipped.sort_by(|a, b| a.0.x.total_cmp(&b.0.x));
    (fused.spans, fused.span_item_indices) = zipped.into_iter().unzip();
    fused
}

/// Validation A: the fused zone re-classifies as a table via the standard
/// detectors, covering most of its lines.
fn validate_via_detectors(merged: &[ProjectedLine]) -> Option<Vec<TableRun>> {
    let runs = detect_tables(merged);
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
                        text_item_indices: Vec::new(),
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
        block_indices: Vec::new(),
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

            // Extend the pair to every leaf living inside the overlap band and
            // entirely on one side of the cut (a side may itself be H-cut into
            // multiple leaves).
            let mut set_leaves: Vec<usize> = Vec::new();
            for (k, leaf) in leaves.iter().enumerate() {
                let leaf_h = (leaf.y1 - leaf.y0).max(1.0);
                let leaf_ov = leaf.y1.min(band.1) - leaf.y0.max(band.0);
                if leaf_ov < 0.5 * leaf_h {
                    continue;
                }
                // The leaf must sit predominantly on one side of the cut:
                // its overhang across the cut stays within the straddle
                // allowance (widened slices), measured against its own width.
                let width = (leaf.x1 - leaf.x0).max(1.0);
                let center = (leaf.x0 + leaf.x1) * 0.5;
                let overhang = if center < cut_x {
                    (leaf.x1 - cut_x).max(0.0)
                } else {
                    (cut_x - leaf.x0).max(0.0)
                };
                if overhang <= CR_SIBLING_MAX_X_OVERLAP_FRAC * width {
                    set_leaves.push(k);
                }
            }
            if set_leaves.len() < 2 {
                continue;
            }

            // The set's line ranges must tile a contiguous range — a foreign
            // leaf interleaved in reading order means this isn't a clean
            // sliced zone.
            let lo = set_leaves.iter().map(|&k| leaves[k].start).min().unwrap();
            let hi = set_leaves.iter().map(|&k| leaves[k].end).max().unwrap();
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
            if side_looks_like_prose(&left_lines, lx0, lx1)
                || side_looks_like_prose(&right_lines, rx0, rx1)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::markdown_layout::test_helpers::{line, line_with_spans};

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

    /// Fusion must keep `span_item_indices` positionally aligned with
    /// `spans` — the fused spans are re-sorted by x, so sorting the two
    /// vectors independently would scramble the pairing. Give every span a
    /// globally distinct source index and check each fused span still sits
    /// beside the index of its own source item.
    #[test]
    fn fused_lines_keep_span_item_indices_aligned() {
        let mut lines = Vec::new();
        let mut source_texts: Vec<String> = Vec::new();
        // Left leaf: row labels (one span each).
        for (i, label) in ["MANILA", "CEBU", "CAGAYAN DE ORO", "SUBIC"]
            .iter()
            .enumerate()
        {
            let y = 100.0 + i as f32 * 20.0;
            let mut l = at_region(line_with_spans(&[(label, 50.0)], y, 10.0), &[0, 0]);
            l.span_item_indices = vec![source_texts.len()];
            source_texts.push((*label).into());
            lines.push(l);
        }
        // Right leaf: two numeric columns per row (two spans each).
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
            let mut l = at_region(line_with_spans(&[(a, 200.0), (b, 280.0)], y, 10.0), &[0, 1]);
            l.span_item_indices = vec![source_texts.len(), source_texts.len() + 1];
            source_texts.push((*a).into());
            source_texts.push((*b).into());
            lines.push(l);
        }
        let m = find_cross_region_table_merge(&lines).expect("should merge");
        assert_eq!(m.merged.len(), 4);
        for fused in &m.merged {
            assert_eq!(fused.spans.len(), fused.span_item_indices.len());
            for (span, &idx) in fused.spans.iter().zip(&fused.span_item_indices) {
                assert_eq!(source_texts[idx], span.text);
            }
        }
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
