//! Structural clean-up of emitted tables, applied to every table block (model or heuristic)
//! before rendering. Each rule fixes a pattern that costs whole columns or rows in table scoring
//! even when every value was read correctly:
//!
//! - currency symbols in their own cell (`$ | 14,046`) or stuck to the previous value
//!   (`8.8 % $ | 13,917`) move onto the number they belong to; columns left empty are removed;
//! - a wrapped cell's continuation line emitted as its own row is folded into the row above;
//! - a group header in a multi-row header ("Fiscal 2024" over "Net Sales | % of Net Sales") spans
//!   the sub-columns it is centred over instead of the first one only;
//! - dot / underscore leaders are stripped from cell text.

use super::blocks::{Block, Cell, SpanCell};
use crate::types::Rect;

/// A cell on the dense grid: top-left slot, spans, text, box.
#[derive(Clone, Debug)]
struct DCell {
    r0: usize,
    c0: usize,
    rs: usize,
    cs: usize,
    text: String,
    bbox: Option<Rect>,
}

struct Dense {
    cells: Vec<DCell>,
    n_rows: usize,
    n_cols: usize,
    header_rows: usize,
}

/// Apply the clean-up to a table block in place (other blocks are untouched).
pub(super) fn fix_block(block: &mut Block) {
    match block {
        Block::MergedTable { rows, header_rows } => {
            let Some(mut d) = from_span_rows(rows, *header_rows) else {
                return;
            };
            run(&mut d);
            let (r, h) = to_span_rows(&d);
            *rows = r;
            *header_rows = h;
        }
        Block::Table { header, rows } => {
            let mut all: Vec<Vec<SpanCell>> = Vec::new();
            let hr = usize::from(header.is_some());
            if let Some(h) = header.as_ref() {
                all.push(h.iter().map(span_of).collect());
            }
            all.extend(rows.iter().map(|r| r.iter().map(span_of).collect()));
            let Some(mut d) = from_span_rows(&all, hr) else {
                return;
            };
            run(&mut d);
            // a plain table stays plain: the rules here never create spans in a one-row header
            let (out, h) = to_span_rows(&d);
            if out.iter().flatten().any(|c| c.colspan > 1 || c.rowspan > 1) {
                return;
            }
            let mut plain: Vec<Vec<Cell>> = out
                .into_iter()
                .map(|r| {
                    r.into_iter()
                        .map(|c| Cell {
                            text: c.text,
                            bbox: c.bbox,
                            text_item_indices: c.text_item_indices,
                        })
                        .collect()
                })
                .collect();
            if h > 0 && !plain.is_empty() {
                *header = Some(plain.remove(0));
            } else {
                *header = None;
            }
            *rows = plain;
        }
        _ => {}
    }
}

fn span_of(c: &Cell) -> SpanCell {
    SpanCell {
        text: c.text.clone(),
        colspan: 1,
        rowspan: 1,
        bbox: c.bbox.clone(),
        text_item_indices: c.text_item_indices.clone(),
    }
}

fn run(d: &mut Dense) {
    strip_leaders(d);
    let emptied = move_currency(d);
    drop_empty_columns(d, &emptied);
    fold_continuation_rows(d);
    span_group_headers(d);
}

// ---------------------------------------------------------------------------------------------
// dense grid <-> span rows

#[allow(clippy::needless_range_loop)]
fn from_span_rows(rows: &[Vec<SpanCell>], header_rows: usize) -> Option<Dense> {
    let n_rows = rows.len();
    if n_rows == 0 {
        return None;
    }
    // place cells HTML-style: each row's cells fill the next free slots left to right
    let mut occ: Vec<Vec<bool>> = vec![Vec::new(); n_rows];
    let mut cells = Vec::new();
    for (r, row) in rows.iter().enumerate() {
        let mut c = 0;
        for sc in row {
            while occ[r].get(c).copied().unwrap_or(false) {
                c += 1;
            }
            let (rs, cs) = (sc.rowspan.max(1) as usize, sc.colspan.max(1) as usize);
            for rr in r..(r + rs).min(n_rows) {
                if occ[rr].len() < c + cs {
                    occ[rr].resize(c + cs, false);
                }
                for cc in c..c + cs {
                    occ[rr][cc] = true;
                }
            }
            cells.push(DCell {
                r0: r,
                c0: c,
                rs: rs.min(n_rows - r),
                cs,
                text: sc.text.clone(),
                bbox: sc.bbox.clone(),
            });
            c += cs;
        }
    }
    let n_cols = occ.iter().map(|o| o.len()).max().unwrap_or(0);
    if n_cols == 0 {
        return None;
    }
    // ragged rows: fill missing slots with empty cells so every slot has an owner
    for (r, o) in occ.iter().enumerate() {
        for c in 0..n_cols {
            if !o.get(c).copied().unwrap_or(false) {
                cells.push(DCell {
                    r0: r,
                    c0: c,
                    rs: 1,
                    cs: 1,
                    text: String::new(),
                    bbox: None,
                });
            }
        }
    }
    Some(Dense {
        cells,
        n_rows,
        n_cols,
        header_rows: header_rows.min(n_rows),
    })
}

fn to_span_rows(d: &Dense) -> (Vec<Vec<SpanCell>>, usize) {
    let mut rows: Vec<Vec<(usize, SpanCell)>> = vec![Vec::new(); d.n_rows];
    for c in &d.cells {
        rows[c.r0].push((
            c.c0,
            SpanCell {
                text: c.text.clone(),
                colspan: c.cs as u16,
                rowspan: c.rs as u16,
                bbox: c.bbox.clone(),
                text_item_indices: Vec::new(),
            },
        ));
    }
    let out = rows
        .into_iter()
        .map(|mut r| {
            r.sort_by_key(|(c, _)| *c);
            r.into_iter().map(|(_, s)| s).collect()
        })
        .collect();
    (out, d.header_rows)
}

/// Index of the cell covering slot (r, c).
fn at(d: &Dense, r: usize, c: usize) -> Option<usize> {
    d.cells
        .iter()
        .position(|x| x.r0 <= r && r < x.r0 + x.rs && x.c0 <= c && c < x.c0 + x.cs)
}

/// Index of the 1x1 cell at (r, c), if the slot holds one.
fn single(d: &Dense, r: usize, c: usize) -> Option<usize> {
    at(d, r, c).filter(|&k| d.cells[k].rs == 1 && d.cells[k].cs == 1)
}

// ---------------------------------------------------------------------------------------------
// rules

const CURRENCY: [&str; 6] = ["$", "€", "£", "¥", "US$", "R$"];

fn starts_numeric(t: &str) -> bool {
    let t = t.trim_start();
    t.starts_with(|ch: char| ch.is_ascii_digit())
        || (t.starts_with(['(', '-', '−', '–', '.'])
            && t.chars().nth(1).is_some_and(|ch| ch.is_ascii_digit()))
}

fn has_digit(t: &str) -> bool {
    t.chars().any(|ch| ch.is_ascii_digit())
}

/// A run of 4+ leader characters (dots / underscores / middle dots), spaces allowed between.
fn strip_leader_runs(t: &str) -> String {
    let chars: Vec<char> = t.chars().collect();
    let leader = |ch: char| matches!(ch, '.' | '_' | '·' | '…');
    let mut out = String::with_capacity(t.len());
    let mut i = 0;
    while i < chars.len() {
        if leader(chars[i]) {
            let mut j = i;
            let mut n = 0;
            while j < chars.len() && (leader(chars[j]) || chars[j] == ' ') {
                if leader(chars[j]) {
                    n += 1;
                }
                j += 1;
            }
            if n >= 4 {
                out.push(' ');
                i = j;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn strip_leaders(d: &mut Dense) {
    for c in d.cells.iter_mut() {
        if c.text.contains("....")
            || c.text.contains("____")
            || c.text.contains(". . . .")
            || c.text.contains('…')
        {
            c.text = strip_leader_runs(&c.text);
        }
    }
}

/// `$ | 14,046` -> `| $ 14,046`; `8.8 % $ | 13,917` -> `8.8 % | $ 13,917`.
fn move_currency(d: &mut Dense) -> Vec<usize> {
    let mut emptied = Vec::new();
    for r in 0..d.n_rows {
        for c in 0..d.n_cols.saturating_sub(1) {
            let (Some(a), Some(b)) = (single(d, r, c), single(d, r, c + 1)) else {
                continue;
            };
            if !starts_numeric(&d.cells[b].text) {
                continue;
            }
            let ta = d.cells[a].text.trim().to_string();
            if CURRENCY.contains(&ta.as_str()) {
                d.cells[b].text = format!("{ta} {}", d.cells[b].text.trim());
                d.cells[a].text.clear();
                if !emptied.contains(&c) {
                    emptied.push(c);
                }
                continue;
            }
            // a value with the next column's symbol glued to its end
            if let Some((head, sym)) = ta.rsplit_once(' ')
                && CURRENCY.contains(&sym)
                && has_digit(head)
            {
                d.cells[a].text = head.trim_end().to_string();
                d.cells[b].text = format!("{sym} {}", d.cells[b].text.trim());
            }
        }
    }
    emptied
}

/// Remove a column that moving currency symbols out of left with only empty 1x1 cells or slots
/// covered by spans that also cover a neighbouring column (spans shrink by one). Columns that
/// were empty to begin with are left alone.
fn drop_empty_columns(d: &mut Dense, emptied: &[usize]) {
    let mut emptied: Vec<usize> = emptied.to_vec();
    emptied.sort_unstable();
    let mut c = 0;
    while c < d.n_cols && d.n_cols > 1 {
        if !emptied.contains(&c) {
            c += 1;
            continue;
        }
        let removable = (0..d.n_rows).all(|r| match at(d, r, c) {
            Some(k) => {
                let x = &d.cells[k];
                if x.cs == 1 {
                    x.text.trim().is_empty() && x.rs == 1
                } else {
                    true
                }
            }
            None => true,
        });
        // keep a column that only spans cover (a structural slot) unless it has no own cell
        let owns_any = d.cells.iter().any(|x| x.cs == 1 && x.c0 == c);
        if !removable || !owns_any {
            c += 1;
            continue;
        }
        d.cells.retain(|x| !(x.cs == 1 && x.c0 == c));
        for x in d.cells.iter_mut() {
            if x.c0 > c {
                x.c0 -= 1;
            } else if x.c0 + x.cs > c {
                x.cs -= 1;
            }
        }
        d.n_cols -= 1;
        // later emptied columns moved left with the removal
        emptied.retain(|&e| e != c);
        for e in emptied.iter_mut() {
            if *e > c {
                *e -= 1;
            }
        }
    }
}

/// Fold a body row that continues the cells of the row above (a wrapped cell emitted as a row).
fn fold_continuation_rows(d: &mut Dense) {
    if d.n_cols < 2 {
        return;
    }
    let body0 = d.header_rows;
    let text = |d: &Dense, r: usize, c: usize| -> Option<String> {
        single(d, r, c).map(|k| d.cells[k].text.trim().to_string())
    };
    // tables whose records carry a label in the first column
    let body = d.n_rows.saturating_sub(body0);
    let led = body >= 3
        && (body0..d.n_rows)
            .filter(|&r| text(d, r, 0).is_some_and(|t| !t.is_empty()))
            .count() as f64
            >= 0.6 * body as f64;
    let mut r = body0 + 1;
    while r < d.n_rows {
        let row: Vec<Option<String>> = (0..d.n_cols).map(|c| text(d, r, c)).collect();
        let above: Vec<Option<String>> = (0..d.n_cols).map(|c| text(d, r - 1, c)).collect();
        if row.iter().chain(&above).any(|t| t.is_none()) {
            // the same wrap written with spans: the record's other cells span both rows and only
            // the wrapped column has a cell of its own in row r
            let mut own = Vec::new();
            let mut spanned = 0;
            let ok = (0..d.n_cols).all(|c| match at(d, r, c) {
                Some(k) if d.cells[k].r0 < r => {
                    spanned += 1;
                    true
                }
                Some(k) if d.cells[k].rs == 1 && d.cells[k].cs == 1 => {
                    let t = d.cells[k].text.trim();
                    if t.is_empty() {
                        return true;
                    }
                    match single(d, r - 1, c) {
                        Some(j) => {
                            let u = d.cells[j].text.trim();
                            let fits = !u.is_empty() && (u.ends_with(['-', '/']) || !is_value(t));
                            if fits {
                                own.push((j, k));
                            }
                            fits
                        }
                        None => false,
                    }
                }
                _ => false,
            });
            // a wrap continues one or two cells of a record whose other cells span it; a row with
            // more cells of its own than spanned slots is a new record under row-group labels
            let own_filled = (0..d.n_cols)
                .filter(|&c| {
                    at(d, r, c)
                        .is_some_and(|k| d.cells[k].r0 == r && !d.cells[k].text.trim().is_empty())
                })
                .count();
            if ok && spanned > 0 && !own.is_empty() && own_filled <= 2 && own_filled < spanned {
                for (j, k) in own {
                    let t = d.cells[k].text.trim().to_string();
                    let up = d.cells[j].text.trim_end().to_string();
                    d.cells[j].text = if up.ends_with(['-', '/']) {
                        format!("{up}{t}")
                    } else {
                        format!("{up} {t}")
                    };
                    if let (Some(x), Some(y)) = (d.cells[j].bbox.clone(), d.cells[k].bbox.clone()) {
                        d.cells[j].bbox = Some(union(x, y));
                    }
                }
                remove_row(d, r);
                continue;
            }
            r += 1;
            continue;
        }
        let row: Vec<String> = row.into_iter().flatten().collect();
        let above: Vec<String> = above.into_iter().flatten().collect();
        let filled: Vec<usize> = (0..d.n_cols).filter(|&c| !row[c].is_empty()).collect();
        let filled_above = above.iter().filter(|t| !t.is_empty()).count();
        // each filled cell continues the cell above it on the page: the one its box overlaps
        // most horizontally (a renderer may have put a lone continuation line in the first slot),
        // else the one in its own column
        let target = |c: usize| -> usize {
            let Some(b) = single(d, r, c).and_then(|k| d.cells[k].bbox.clone()) else {
                return c;
            };
            let ov = |a: &Rect| (b.x + b.width).min(a.x + a.width) - b.x.max(a.x);
            (0..d.n_cols)
                .filter_map(|cc| {
                    let a = single(d, r - 1, cc).and_then(|k| d.cells[k].bbox.clone())?;
                    let o = ov(&a);
                    (o > 0.5 * b.width.min(a.width)).then_some((o, cc))
                })
                .max_by(|x, y| x.0.total_cmp(&y.0))
                .map_or(c, |(_, cc)| cc)
        };
        let targets: Vec<usize> = filled.iter().map(|&c| target(c)).collect();
        let distinct = {
            let mut t = targets.clone();
            t.sort_unstable();
            t.dedup();
            t.len() == targets.len()
        };
        let continues = !filled.is_empty()
            && distinct
            && filled.len() < filled_above
            && filled.iter().zip(&targets).all(|(&c, &tc)| {
                let (t, u) = (&row[c], &above[tc]);
                let marker = t.starts_with(['•', '-', '*', '–'])
                    || t.split('.').next().is_some_and(|p| {
                        !p.is_empty()
                            && p.len() <= 3
                            && p.chars().all(|ch| ch.is_ascii_digit())
                            && t.len() > p.len()
                            && t[p.len()..].starts_with('.')
                    });
                !u.is_empty()
                    && !marker
                    && (u.ends_with(['-', '/'])
                        || (!has_digit(t) && led && tc != 0 && !above[0].is_empty()))
            });
        if !continues {
            r += 1;
            continue;
        }
        for (&c, &tc) in filled.iter().zip(&targets) {
            let (Some(a), Some(b)) = (single(d, r - 1, tc), single(d, r, c)) else {
                continue;
            };
            let t = d.cells[b].text.trim().to_string();
            let up = d.cells[a].text.trim_end().to_string();
            d.cells[a].text = if up.ends_with(['-', '/']) {
                format!("{up}{t}")
            } else {
                format!("{up} {t}")
            };
            if let (Some(x), Some(y)) = (d.cells[a].bbox.clone(), d.cells[b].bbox.clone()) {
                d.cells[a].bbox = Some(union(x, y));
            }
        }
        remove_row(d, r);
    }
}

/// A plain number / amount / percent / date-like value (not wrapped label text).
fn is_value(t: &str) -> bool {
    let digits = t.chars().filter(|ch| ch.is_ascii_digit()).count();
    let letters = t.chars().filter(|ch| ch.is_alphabetic()).count();
    digits > 0 && digits >= letters
}

/// Delete row r (its own cells go; spans through it shrink; rows below move up).
fn remove_row(d: &mut Dense, r: usize) {
    d.cells.retain(|x| !(x.r0 == r && x.rs == 1));
    for x in d.cells.iter_mut() {
        if x.r0 > r {
            x.r0 -= 1;
        } else if x.r0 + x.rs > r {
            x.rs -= 1;
        }
    }
    d.n_rows -= 1;
}

fn union(a: Rect, b: Rect) -> Rect {
    let x0 = a.x.min(b.x);
    let y0 = a.y.min(b.y);
    let x1 = (a.x + a.width).max(b.x + b.width);
    let y1 = (a.y + a.height).max(b.y + b.height);
    Rect {
        x: x0,
        y: y0,
        width: x1 - x0,
        height: y1 - y0,
    }
}

/// In a multi-row header, a cell followed by empty slots that the next header row fills spans
/// those columns when its box is centred over them (a group header over its sub-headers).
fn span_group_headers(d: &mut Dense) {
    if d.header_rows < 2 {
        return;
    }
    for h in 0..d.header_rows - 1 {
        let mut c = 0;
        while c < d.n_cols {
            let Some(k) = single(d, h, c) else {
                c += 1;
                continue;
            };
            if d.cells[k].text.trim().is_empty() {
                c += 1;
                continue;
            }
            let mut end = c;
            while end + 1 < d.n_cols
                && single(d, h, end + 1).is_some_and(|j| d.cells[j].text.trim().is_empty())
                && at(d, h + 1, end + 1).is_some_and(|j| !d.cells[j].text.trim().is_empty())
            {
                end += 1;
            }
            if end == c {
                c += 1;
                continue;
            }
            // centred over the group: closer to the group's centre than to its first sub-column's
            let centre = |b: &Rect| b.x + b.width / 2.0;
            let subs: Vec<Rect> = (c..=end)
                .filter_map(|cc| at(d, h + 1, cc).and_then(|j| d.cells[j].bbox.clone()))
                .collect();
            let ok = match (d.cells[k].bbox.clone(), subs.first().cloned()) {
                (Some(hb), Some(first)) => {
                    let g = subs.iter().cloned().reduce(union).unwrap();
                    (centre(&hb) - centre(&g)).abs() < (centre(&hb) - centre(&first)).abs()
                }
                _ => false,
            };
            if !ok {
                c = end + 1;
                continue;
            }
            d.cells
                .retain(|x| !(x.r0 == h && x.rs == 1 && x.cs == 1 && x.c0 > c && x.c0 <= end));
            let k = single(d, h, c).unwrap();
            d.cells[k].cs = end - c + 1;
            c = end + 1;
        }
    }
}

/// Header band left above a model table: when the table's top row has empty slots, the text
/// lines just above it whose runs fall over its columns (two or more columns, separated by
/// column-sized gaps, not prose) become new header rows. Returns the indices of the absorbed
/// lines (the caller drops them from the text flow). Common in practice: a table's header row
/// printed as a paragraph above an empty-headed table.
/// A header band cell: column, text, box.
type BandCell = (usize, String, [f64; 4]);

pub(super) fn absorb_header_band(
    g: &mut crate::table_model::decode::Grid,
    lines: &[crate::types::ProjectedLine],
    pb: &[Rect],
    taken: &std::collections::HashMap<usize, Option<crate::types::ProjectedLine>>,
) -> Vec<usize> {
    use crate::table_model::decode::GridCell;
    if g.n_cols < 2 || g.n_rows < 2 {
        return Vec::new();
    }
    let filled = |r: usize, c: usize| {
        g.cells
            .iter()
            .any(|x| x.r0 <= r && r <= x.r1 && x.c0 <= c && c <= x.c1 && !x.text.trim().is_empty())
    };
    let empty0: Vec<usize> = (0..g.n_cols).filter(|&c| !filled(0, c)).collect();
    if empty0.is_empty() {
        return Vec::new();
    }
    // column extents from the single-column cells
    let mut rng: Vec<Option<(f64, f64)>> = vec![None; g.n_cols];
    let mut hs = Vec::new();
    for x in &g.cells {
        if x.text.trim().is_empty() {
            continue;
        }
        hs.push(x.bx[3] - x.bx[1]);
        if x.c0 == x.c1 {
            let e = rng[x.c0].get_or_insert((x.bx[0], x.bx[2]));
            e.0 = e.0.min(x.bx[0]);
            e.1 = e.1.max(x.bx[2]);
        }
    }
    if rng.iter().any(|r| r.is_none()) || hs.is_empty() {
        return Vec::new();
    }
    let rng: Vec<(f64, f64)> = rng.into_iter().flatten().collect();
    hs.sort_by(|a, b| a.total_cmp(b));
    let hh = hs[hs.len() / 2].max(1.0);
    let top = g
        .cells
        .iter()
        .map(|x| x.bx[1])
        .fold(f64::INFINITY, f64::min);
    let (tx0, tx1) = (rng[0].0, rng[g.n_cols - 1].1);
    let column = |cx: f64| -> Option<usize> {
        let (mut best, mut bd) = (None, f64::INFINITY);
        for (c, &(a, b)) in rng.iter().enumerate() {
            let dist = if cx < a {
                a - cx
            } else if cx > b {
                cx - b
            } else {
                0.0
            };
            if dist < bd {
                bd = dist;
                best = Some(c);
            }
        }
        best.filter(|_| bd <= 2.0 * hh)
    };
    // candidate lines above the table, closest first
    let mut cand: Vec<usize> = (0..lines.len())
        .filter(|&i| {
            let b = &pb[i];
            let (bot, cx) = ((b.y + b.height) as f64, (b.x + b.width / 2.0) as f64);
            !taken.contains_key(&i)
                && lines[i].spans.len() >= 2
                && bot <= top + 0.3 * hh
                && top - bot <= 2.5 * hh
                && tx0 - hh <= cx
                && cx <= tx1 + hh
        })
        .collect();
    cand.sort_by(|&a, &b| pb[b].y.total_cmp(&pb[a].y));
    let mut rows: Vec<(usize, Vec<BandCell>)> = Vec::new();
    let mut edge = top;
    for i in cand {
        let l = &lines[i];
        let bot = (pb[i].y + pb[i].height) as f64;
        if edge - bot > 2.0 * hh || rows.len() >= 2 {
            break;
        }
        let text = l.text.trim();
        if text.split_whitespace().count() > 16 || (text.ends_with('.') && text.len() > 40) {
            break;
        }
        let mut spans: Vec<&crate::types::TextItem> = l
            .spans
            .iter()
            .filter(|s| !s.text.trim().is_empty())
            .collect();
        spans.sort_by(|a, b| a.x.total_cmp(&b.x));
        let wide_gap = spans
            .windows(2)
            .any(|w| (w[1].x - (w[0].x + w[0].width)) as f64 >= 1.5 * hh);
        let mut cells: Vec<BandCell> = Vec::new();
        let mut ok = wide_gap;
        for s in &spans {
            let cx = (s.x + s.width / 2.0) as f64;
            let Some(c) = column(cx) else {
                ok = false;
                break;
            };
            let bx = [
                s.x as f64,
                s.y as f64,
                (s.x + s.width) as f64,
                (s.y + s.height) as f64,
            ];
            match cells.iter_mut().find(|(cc, _, _)| *cc == c) {
                Some(e) => {
                    e.1 = format!("{} {}", e.1, s.text.trim());
                    e.2 = [
                        e.2[0].min(bx[0]),
                        e.2[1].min(bx[1]),
                        e.2[2].max(bx[2]),
                        e.2[3].max(bx[3]),
                    ];
                }
                None => cells.push((c, s.text.trim().to_string(), bx)),
            }
        }
        if !ok || cells.len() < 2 || !cells.iter().any(|(c, _, _)| empty0.contains(c)) {
            break;
        }
        rows.push((i, cells));
        edge = pb[i].y as f64;
    }
    if rows.is_empty() {
        return Vec::new();
    }
    // top-most line first; every existing row moves down
    rows.reverse();
    let k = rows.len();
    for x in g.cells.iter_mut() {
        x.r0 += k;
        x.r1 += k;
    }
    for h in g.head_rows.iter_mut() {
        *h += k;
    }
    for (r, (_, cells)) in rows.iter().enumerate() {
        g.head_rows.push(r);
        for c in 0..g.n_cols {
            let (text, bx) = cells
                .iter()
                .find(|(cc, _, _)| *cc == c)
                .map_or((String::new(), [0.0; 4]), |(_, t, b)| (t.clone(), *b));
            g.cells.push(GridCell {
                r0: r,
                c0: c,
                r1: r,
                c1: c,
                text,
                bx,
            });
        }
    }
    // the band's rows lead the header; rows that were header stay header
    g.head_rows.sort_unstable();
    g.head_rows.dedup();
    g.n_rows += k;
    rows.into_iter().map(|(i, _)| i).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sc(t: &str) -> SpanCell {
        SpanCell {
            text: t.into(),
            colspan: 1,
            rowspan: 1,
            bbox: None,
            text_item_indices: Vec::new(),
        }
    }

    fn texts(rows: &[Vec<SpanCell>]) -> Vec<Vec<String>> {
        rows.iter()
            .map(|r| r.iter().map(|c| c.text.clone()).collect())
            .collect()
    }

    #[test]
    fn currency_cells_move_and_empty_column_goes() {
        let mut b = Block::MergedTable {
            rows: vec![
                vec![sc("Item"), sc(""), sc("2024"), sc("Pct")],
                vec![sc("Appliances"), sc("$"), sc("14,046"), sc("8.8 %")],
                vec![sc("Bath"), sc(""), sc("6,485"), sc("4.1")],
            ],
            header_rows: 1,
        };
        fix_block(&mut b);
        let Block::MergedTable { rows, .. } = b else {
            panic!()
        };
        assert_eq!(
            texts(&rows),
            vec![
                vec!["Item", "2024", "Pct"],
                vec!["Appliances", "$ 14,046", "8.8 %"],
                vec!["Bath", "6,485", "4.1"]
            ]
        );
    }

    #[test]
    fn trailing_symbol_moves_right() {
        let mut b = Block::MergedTable {
            rows: vec![
                vec![sc("A"), sc("8.8 % $"), sc("13,917")],
                vec![sc("B"), sc("4.3"), sc("6,630")],
            ],
            header_rows: 0,
        };
        fix_block(&mut b);
        let Block::MergedTable { rows, .. } = b else {
            panic!()
        };
        assert_eq!(texts(&rows)[0], vec!["A", "8.8 %", "$ 13,917"]);
    }

    #[test]
    fn wrapped_cell_row_folds() {
        let mut b = Block::MergedTable {
            rows: vec![
                vec![sc("Title"), sc("Page"), sc("Dated")],
                vec![sc("Independent Distributors"), sc("CW-CGL"), sc("06/01/18")],
                vec![sc(""), sc("FOOD.EXCLPRODUCTS"), sc("")],
                vec![sc("Independent Distributors"), sc("CW-CGL"), sc("06/01/18")],
                vec![sc("Other"), sc("CW-X"), sc("06/01/18")],
            ],
            header_rows: 1,
        };
        fix_block(&mut b);
        let Block::MergedTable { rows, .. } = b else {
            panic!()
        };
        let t = texts(&rows);
        assert_eq!(t.len(), 4);
        assert_eq!(t[1][1], "CW-CGL FOOD.EXCLPRODUCTS");
    }

    #[test]
    fn numeric_sub_rows_do_not_fold() {
        let mut b = Block::MergedTable {
            rows: vec![
                vec![sc("Year"), sc("Q"), sc("Value")],
                vec![sc("2022"), sc("Q1"), sc("5")],
                vec![sc(""), sc("Q2"), sc("6")],
                vec![sc("2023"), sc("Q1"), sc("7")],
            ],
            header_rows: 1,
        };
        fix_block(&mut b);
        let Block::MergedTable { rows, .. } = b else {
            panic!()
        };
        assert_eq!(rows.len(), 4);
    }

    #[test]
    fn leaders_stripped() {
        assert_eq!(strip_leader_runs("2022 ......................"), "2022");
        assert_eq!(strip_leader_runs("Total . . . . . ."), "Total");
        assert_eq!(strip_leader_runs("v1.2.3"), "v1.2.3");
    }

    #[test]
    fn group_header_spans_when_centred() {
        let r = |x: f32, w: f32| {
            Some(Rect {
                x,
                y: 0.0,
                width: w,
                height: 10.0,
            })
        };
        let cell = |t: &str, b: Option<Rect>| SpanCell {
            text: t.into(),
            colspan: 1,
            rowspan: 1,
            bbox: b,
            text_item_indices: Vec::new(),
        };
        let mut b = Block::MergedTable {
            rows: vec![
                vec![
                    cell("", None),
                    cell("Fiscal 2024", r(120.0, 60.0)),
                    cell("", None),
                ],
                vec![
                    cell("Item", r(0.0, 50.0)),
                    cell("Net Sales", r(100.0, 40.0)),
                    cell("% of Net Sales", r(150.0, 50.0)),
                ],
                vec![cell("Bath", None), cell("6,485", None), cell("4.1", None)],
            ],
            header_rows: 2,
        };
        fix_block(&mut b);
        let Block::MergedTable { rows, .. } = b else {
            panic!()
        };
        assert_eq!(rows[0].len(), 2);
        assert_eq!(rows[0][1].colspan, 2);
    }
}
