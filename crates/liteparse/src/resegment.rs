//! Column resegmentation from the layout model's blocks.
//!
//! On dense multi-column pages with narrow gutters the projection can build one line across two
//! columns ("caved in and signed an Halifax went hunting with ..."), and the XY-cut leaf then holds
//! both columns, so markdown reads them interleaved. The layout model (LayoutNet) sees the page's
//! text units and segments them into blocks; where a projected line's spans fall in two different
//! model blocks *and* a horizontal gap separates them, the line is split there. Every line of a leaf
//! that had such a split is then given a sub-path per block and the leaf's run is reordered block by
//! block (columns left to right, each top to bottom), so paragraph grouping and reading order follow
//! the model's columns. Leaves without a split are untouched (byte-identical output).

use crate::table_model::BLOCK_CLASSES;
use crate::types::{ParsedPage, ProjectedLine, Rect};

/// Minimum horizontal gap between two spans, in multiples of the line's font size, for a block change
/// to count as a gutter (a word space is ~0.25-0.33 em).
const GUTTER_EM: f32 = 0.5;
/// Lines a model block must hold within one leaf to count as a column for reordering.
const MIN_COLUMN_LINES: usize = 3;
/// A block at most this share of the text's width whose words are this share numbers / times is a
/// margin of line numbers, schedule times or page numbers, not a column ("19 20 21 ...", "14:00-14:10").
const MARGINAL_MAX_WIDTH: f32 = 0.15;
const MARGINAL_MIN_NUMERIC: f32 = 0.8;

/// A line number, time, time range or section number: "19", "14:00-14:10", "2pm", "(3)", "§3.1".
fn numeric_token(t: &str) -> bool {
    let t = t.trim_end_matches(['a', 'p', 'A', 'P', 'm', 'M', '.']);
    !t.is_empty()
        && t.chars().count() <= 14
        && t.chars().any(|c| c.is_ascii_digit())
        && t.chars().all(|c| {
            c.is_ascii_digit()
                || matches!(
                    c,
                    ':' | '.' | ',' | '-' | '\u{2013}' | '\u{2014}' | '(' | ')' | '/' | '\u{a7}'
                )
        })
}

fn union(a: &Rect, b: &Rect) -> Rect {
    let (x0, y0) = (a.x.min(b.x), a.y.min(b.y));
    let (x1, y1) = (
        (a.x + a.width).max(b.x + b.width),
        (a.y + a.height).max(b.y + b.height),
    );
    Rect {
        x: x0,
        y: y0,
        width: x1 - x0,
        height: y1 - y0,
    }
}

/// Split and reorder `page.projected_lines` along the layout model's blocks. Returns the number of
/// lines split. No-op without at least two model blocks.
pub(crate) fn resegment_by_model_blocks(page: &mut ParsedPage) -> usize {
    let blocks = &page.model_blocks;
    if blocks.len() < 2 || page.projected_lines.is_empty() {
        return 0;
    }
    let is_table = |c: usize| matches!(BLOCK_CLASSES[c], "Table" | "FigureText");
    // the smallest model block containing a point (nested blocks: the inner one)
    let block_at = |x: f32, y: f32| -> Option<usize> {
        blocks
            .iter()
            .enumerate()
            .filter(|(_, b)| {
                b.bbox.x <= x
                    && x <= b.bbox.x + b.bbox.width
                    && b.bbox.y <= y
                    && y <= b.bbox.y + b.bbox.height
            })
            .min_by(|a, b| {
                (a.1.bbox.width * a.1.bbox.height).total_cmp(&(b.1.bbox.width * b.1.bbox.height))
            })
            .map(|(i, _)| i)
    };

    // each span's model block
    let assigns: Vec<Vec<Option<usize>>> = page
        .projected_lines
        .iter()
        .map(|l| {
            l.spans
                .iter()
                .map(|s| block_at(s.x + s.width / 2.0, s.y + s.height / 2.0))
                .collect()
        })
        .collect();
    // marginal blocks: narrow runs of line numbers / times / page numbers beside the text. Splitting them off
    // as a column moved "19 20 21 ... 37" or a schedule's time column ahead of the text they number.
    let (tx0, tx1) = page
        .projected_lines
        .iter()
        .fold((f32::MAX, f32::MIN), |(a, b), l| {
            (a.min(l.bbox.x), b.max(l.bbox.x + l.bbox.width))
        });
    let mut words = vec![(0usize, 0usize); blocks.len()];
    for (line, assign) in page.projected_lines.iter().zip(&assigns) {
        for (sp, a) in line.spans.iter().zip(assign) {
            if let Some(k) = a {
                for w in sp.text.split_whitespace() {
                    words[*k].0 += 1;
                    words[*k].1 += numeric_token(w) as usize;
                }
            }
        }
    }
    let marginal: Vec<bool> = blocks
        .iter()
        .zip(&words)
        .map(|(b, &(n, num))| {
            n > 0
                && b.bbox.width <= MARGINAL_MAX_WIDTH * (tx1 - tx0).max(1.0)
                && num as f32 >= MARGINAL_MIN_NUMERIC * n as f32
        })
        .collect();

    // 1. split lines that cross a block boundary at a gutter. Candidate cuts first, per line (index, gap centre).
    let lines = std::mem::take(&mut page.projected_lines);
    let mut cand: Vec<Vec<(usize, f32)>> = vec![Vec::new(); lines.len()];
    for (li, (line, assign)) in lines.iter().zip(&assigns).enumerate() {
        let rotated = line.spans.iter().any(|s| s.rotation.abs() > 1.0);
        let touches_table = assign.iter().flatten().any(|&b| is_table(blocks[b].class));
        if line.spans.len() < 2 || rotated || touches_table || line.rtl || line.all_mono {
            continue;
        }
        let gap_min = GUTTER_EM * line.dominant_font_size.max(4.0);
        let mut prev_block = assign[0];
        for i in 1..line.spans.len() {
            let (a, b) = (&line.spans[i - 1], &line.spans[i]);
            if let (Some(pb), Some(cb)) = (prev_block, assign[i])
                && pb != cb
                && !marginal[pb]
                && !marginal[cb]
                && b.x - (a.x + a.width) >= gap_min
            {
                cand[li].push((i, (a.x + a.width + b.x) / 2.0));
            }
            if assign[i].is_some() {
                prev_block = assign[i];
            }
        }
    }
    // A column gutter is a channel: the same gap on several lines of the leaf. A lone wide word gap where a
    // model block edge cuts through justified text ("... access to | it.") is not, and splitting there moved
    // line endings into a column of their own.
    let mut keep: Vec<Vec<usize>> = vec![Vec::new(); lines.len()];
    for li in 0..lines.len() {
        let em = lines[li].dominant_font_size.max(4.0);
        for &(i, x) in &cand[li] {
            let support = (0..lines.len())
                .filter(|&lj| {
                    lines[lj].region_path == lines[li].region_path
                        && cand[lj].iter().any(|&(_, y)| (y - x).abs() <= em)
                })
                .count();
            if support >= MIN_COLUMN_LINES {
                keep[li].push(i);
            }
        }
    }
    let mut out: Vec<(ProjectedLine, Option<usize>)> = Vec::with_capacity(lines.len());
    let mut split_paths: Vec<Vec<u16>> = Vec::new();
    let mut n_split = 0;
    for ((line, assign), cuts) in lines.into_iter().zip(assigns).zip(keep) {
        let majority = {
            let mut best: Option<(usize, usize)> = None;
            for b in assign.iter().flatten() {
                let n = assign.iter().filter(|a| **a == Some(*b)).count();
                if best.is_none_or(|(_, m)| n > m) {
                    best = Some((*b, n));
                }
            }
            best.map(|(b, _)| b)
        };
        // cuts: before span i where its block differs from the previous one across a gutter that lines up on
        // other lines of the leaf (monospace, rotated, RTL and table lines have none)
        if cuts.is_empty() {
            out.push((line, majority));
            continue;
        }
        n_split += 1;
        if !split_paths.contains(&line.region_path) {
            split_paths.push(line.region_path.clone());
        }
        let mut bounds = vec![0];
        bounds.extend(cuts);
        bounds.push(line.spans.len());
        for w in bounds.windows(2) {
            let spans = line.spans[w[0]..w[1]].to_vec();
            let mut bbox = Rect {
                x: spans[0].x,
                y: spans[0].y,
                width: spans[0].width,
                height: spans[0].height,
            };
            for s in &spans[1..] {
                bbox = union(
                    &bbox,
                    &Rect {
                        x: s.x,
                        y: s.y,
                        width: s.width,
                        height: s.height,
                    },
                );
            }
            let text = spans
                .iter()
                .map(|s| s.text.trim())
                .filter(|t| !t.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            let block = assign[w[0]..w[1]].iter().flatten().next().copied();
            let mut part = line.clone();
            part.indent_x = line.indent_x + (bbox.x - line.bbox.x);
            part.text = text;
            part.bbox = bbox;
            part.spans = spans;
            out.push((part, block));
        }
    }
    // Leaves whose whole lines fall in side-by-side model blocks (columns the XY-cut missed: separate lines
    // read row by row across the page) are reordered too. Each such block must hold >= MIN_COLUMN_LINES of
    // the leaf's lines, so forms / key-value rows (one-line blocks side by side) keep their row order.
    {
        let mut i = 0;
        while i < out.len() {
            let path = out[i].0.region_path.clone();
            let mut j = i + 1;
            while j < out.len() && out[j].0.region_path == path {
                j += 1;
            }
            let mono = out[i..j].iter().filter(|(l, _)| l.all_mono).count() * 2 > j - i;
            if !split_paths.contains(&path) && !mono {
                let mut count: Vec<(usize, usize)> = Vec::new();
                for (_, b) in &out[i..j] {
                    if let Some(b) = b {
                        match count.iter_mut().find(|(k, _)| k == b) {
                            Some(c) => c.1 += 1,
                            None => count.push((*b, 1)),
                        }
                    }
                }
                let cols: Vec<usize> = count
                    .iter()
                    .filter(|(k, n)| *n >= MIN_COLUMN_LINES && !marginal[*k])
                    .map(|(k, _)| *k)
                    .collect();
                let side_by_side = cols.iter().any(|&a| {
                    cols.iter().any(|&b| {
                        let (ra, rb) = (&blocks[a].bbox, &blocks[b].bbox);
                        a != b
                            && ra.x + ra.width <= rb.x
                            && ra.y < rb.y + rb.height
                            && rb.y < ra.y + ra.height
                    })
                });
                if side_by_side {
                    split_paths.push(path);
                }
            }
            i = j;
        }
    }
    if split_paths.is_empty() {
        page.projected_lines = out.into_iter().map(|(l, _)| l).collect();
        return 0;
    }

    // 2. in each leaf that had a split, a sub-path per column and a column-by-column order (each top to bottom).
    let mut result: Vec<ProjectedLine> = Vec::with_capacity(out.len());
    let mut i = 0;
    while i < out.len() {
        let path = out[i].0.region_path.clone();
        let mut j = i + 1;
        while j < out.len() && out[j].0.region_path == path {
            j += 1;
        }
        if !split_paths.contains(&path) {
            result.extend(out[i..j].iter().map(|(l, _)| l.clone()));
            i = j;
            continue;
        }
        // a marginal block is no column: its lines join the nearest column in y order, beside the lines they number
        let mut present: Vec<usize> = out[i..j]
            .iter()
            .filter_map(|(_, b)| *b)
            .filter(|&k| !marginal[k])
            .collect();
        present.sort_unstable();
        present.dedup();
        present.sort_by(|&a, &b| blocks[a].bbox.x.total_cmp(&blocks[b].bbox.x));
        // Columns: blocks whose x-ranges overlap share one (an indented quote or list stays in its column instead
        // of sorting after every block at the column's left edge). Blocks spanning most of the leaf (a full-width
        // heading) would fuse the columns, so they are left out of the clustering and join the nearest column.
        let (lx0, lx1) = out[i..j]
            .iter()
            .fold((f32::MAX, f32::MIN), |(a, b), (l, _)| {
                (a.min(l.bbox.x), b.max(l.bbox.x + l.bbox.width))
            });
        let wide = |k: usize| blocks[k].bbox.width > 0.6 * (lx1 - lx0).max(1.0);
        let mut col_right: Vec<f32> = Vec::new();
        let mut col_of: Vec<(usize, usize)> = Vec::new();
        for &k in present.iter().filter(|&&k| !wide(k)) {
            let r = &blocks[k].bbox;
            match col_right.last_mut() {
                Some(right) if r.x < *right => *right = right.max(r.x + r.width),
                _ => col_right.push(r.x + r.width),
            }
            col_of.push((k, col_right.len() - 1));
        }
        let nearest = |l: &ProjectedLine| -> usize {
            let cx = l.bbox.x + l.bbox.width / 2.0;
            *present
                .iter()
                .min_by(|&&a, &&b| {
                    let d = |k: usize| {
                        let r = &blocks[k].bbox;
                        if cx < r.x {
                            r.x - cx
                        } else if cx > r.x + r.width {
                            cx - r.x - r.width
                        } else {
                            0.0
                        }
                    };
                    d(a).total_cmp(&d(b))
                })
                .unwrap()
        };
        // a line's column: its block's, or (wide block / no block) the column whose x-range is nearest its centre
        let col_left: Vec<f32> = (0..col_right.len())
            .map(|c| {
                col_of
                    .iter()
                    .filter(|(_, cc)| *cc == c)
                    .map(|(k, _)| blocks[*k].bbox.x)
                    .fold(f32::MAX, f32::min)
            })
            .collect();
        let column_at = |l: &ProjectedLine| -> usize {
            let cx = l.bbox.x + l.bbox.width / 2.0;
            (0..col_right.len())
                .min_by(|&a, &b| {
                    let d = |c: usize| {
                        if cx < col_left[c] {
                            col_left[c] - cx
                        } else if cx > col_right[c] {
                            cx - col_right[c]
                        } else {
                            0.0
                        }
                    };
                    d(a).total_cmp(&d(b))
                })
                .unwrap_or(0)
        };
        let _ = &nearest;
        let mut leaf: Vec<(usize, ProjectedLine)> = out[i..j]
            .iter()
            .map(|(l, b)| {
                let col = b
                    .and_then(|k| col_of.iter().find(|(kk, _)| *kk == k).map(|(_, c)| *c))
                    .unwrap_or_else(|| column_at(l));
                (col, l.clone())
            })
            .collect();
        leaf.sort_by(|(ra, a), (rb, b)| {
            ra.cmp(rb)
                .then(a.bbox.y.total_cmp(&b.bbox.y))
                .then(a.bbox.x.total_cmp(&b.bbox.x))
        });
        for (rank, mut l) in leaf {
            l.region_path.push(rank as u16);
            result.push(l);
        }
        i = j;
    }
    page.projected_lines = result;
    n_split
}

#[cfg(test)]
mod tests {
    use super::numeric_token;

    #[test]
    fn numeric_tokens_are_margins() {
        for t in [
            "19",
            "14:00-14:10",
            "2pm",
            "5:35pm",
            "(3)",
            "§3.1",
            "2025",
            "1.2.3.",
        ] {
            assert!(numeric_token(t), "{t}");
        }
        for t in ["Abstract", "a", "pm", "Room", "7F", "COVID-19", "-", "..."] {
            assert!(!numeric_token(t), "{t}");
        }
    }
}
