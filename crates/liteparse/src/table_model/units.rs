//! Text units from a PDFium page's characters (a port of `bench/tables/units.py: pdf_units`).
//!
//! A unit is a run of text the writer put down together, cut wherever a gap is wide enough to
//! separate two cells: wider than CELL_GAP glyph heights; a dot leader; two plain numbers apart;
//! or a space far wider than the run's own word spacing (a tight column gutter). Boxes are built
//! in the text's reading frame (sideways tables read in their own direction); `pbox` keeps the
//! rendered page's frame. Points, top-left origin.

use pdfium::{Page, TextChar};

pub const CELL_GAP: f64 = 1.8;
const LEADER: &[char] = &['.', '·', '…', '•', '\u{fffd}', '_'];
const LEADER_MIN: usize = 3;
const NUM_GAP: f64 = 0.5;
const ADAPT_X: f64 = 2.5;
const ADAPT_MIN: f64 = 0.6;

#[derive(Clone, Debug)]
pub struct Unit {
    /// reading-frame box
    pub bx: [f64; 4],
    /// rendered-page box
    pub pbox: [f64; 4],
    pub text: String,
    pub h: f64,
    pub size: f64,
    pub bold: Option<bool>,
    /// reading direction on the page: right, up, left, down = 0..3
    pub q: u8,
    /// runs against the page's main direction (a sidebar, a rotated label)
    pub minor: bool,
    /// words as (x0, x1, text) in the reading frame, for `split_aligned`
    pub words: Vec<(f64, f64, String)>,
}

/// One character: code unit(s), rendered-page box, font size, bold.
#[derive(Clone, Debug)]
struct Ch {
    c: Vec<u32>,
    pb: [f64; 4],
    size: f64,
    bold: Option<bool>,
}

impl Ch {
    fn is_space(&self) -> bool {
        self.c.len() == 1 && char::from_u32(self.c[0]).is_some_and(|c| c.is_whitespace())
    }
}

pub(super) fn to_frame(b: [f64; 4], q: u8, w: f64, h: f64) -> [f64; 4] {
    let [x0, y0, x1, y1] = b;
    match q {
        1 => [h - y1, x0, h - y0, x1],
        2 => [w - x1, h - y1, w - x0, h - y0],
        3 => [y0, w - x1, y1, w - x0],
        _ => b,
    }
}

pub(super) fn from_frame(b: [f64; 4], q: u8, w: f64, h: f64) -> [f64; 4] {
    let [x0, y0, x1, y1] = b;
    match q {
        1 => [y0, h - x1, y1, h - x0],
        2 => [w - x1, h - y1, w - x0, h - y0],
        3 => [w - y1, x0, w - y0, x1],
        _ => b,
    }
}

/// Page box (crop-relative, top-left) -> the rendered page under /Rotate `rot` quarter turns.
fn rotate_page(b: [f64; 4], rot: i32, w0: f64, h0: f64) -> [f64; 4] {
    let [x0, y0, x1, y1] = b;
    match rot {
        1 => [h0 - y1, x0, h0 - y0, x1],
        2 => [w0 - x1, h0 - y1, w0 - x0, h0 - y0],
        3 => [y0, w0 - x1, y1, w0 - x0],
        _ => b,
    }
}

fn median(v: &mut Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    }
}

/// Code units -> text, pairing UTF-16 surrogate halves and dropping lone ones.
fn decode(units: &[u32]) -> String {
    let mut s = String::new();
    let mut pending: Vec<u16> = Vec::new();
    let flush = |pending: &mut Vec<u16>, s: &mut String| {
        for c in char::decode_utf16(pending.drain(..)).flatten() {
            s.push(c);
        }
    };
    for &u in units {
        if (0xD800..=0xDFFF).contains(&u) {
            pending.push(u as u16);
        } else {
            flush(&mut pending, &mut s);
            if let Some(c) = char::from_u32(u) {
                s.push(c);
            }
        }
    }
    flush(&mut pending, &mut s);
    s
}

/// Python `\d` (a Unicode decimal digit).
fn is_dec(c: char) -> bool {
    c.is_numeric() && c.is_ascii_digit() || c.is_ascii_digit()
}

/// units.py _NUM: `^[-–−]?\$?\d[\d,.]*%?(?:[-–−]\d[\d,.]*%?)?$`
fn is_num(t: &str) -> bool {
    let cs: Vec<char> = t.chars().collect();
    let mut i = 0;
    let dash = |c: char| c == '-' || c == '–' || c == '−';
    if i < cs.len() && dash(cs[i]) {
        i += 1;
    }
    if i < cs.len() && cs[i] == '$' {
        i += 1;
    }
    let number = |i: &mut usize| -> bool {
        if *i >= cs.len() || !is_dec(cs[*i]) {
            return false;
        }
        *i += 1;
        while *i < cs.len() && (is_dec(cs[*i]) || cs[*i] == ',' || cs[*i] == '.') {
            *i += 1;
        }
        if *i < cs.len() && cs[*i] == '%' {
            *i += 1;
        }
        true
    };
    if !number(&mut i) {
        return false;
    }
    if i == cs.len() {
        return true;
    }
    // optional range part; `[\d,.]*` is greedy but can't swallow the dash, so no backtracking
    if dash(cs[i]) {
        i += 1;
        if number(&mut i) && i == cs.len() {
            return true;
        }
    }
    false
}

/// True for characters in a dot-leader run (LEADER_MIN+ leader characters, spaces between).
fn leader_mask(chars: &[(Ch, [f64; 4])]) -> Vec<bool> {
    let mut mask = vec![false; chars.len()];
    let mut i = 0;
    let is_lead =
        |c: &Ch| c.c.len() == 1 && char::from_u32(c.c[0]).is_some_and(|ch| LEADER.contains(&ch));
    while i < chars.len() {
        if is_lead(&chars[i].0) {
            let (mut j, mut k) = (i, 0);
            while j < chars.len() && (is_lead(&chars[j].0) || chars[j].0.is_space()) {
                k += is_lead(&chars[j].0) as usize;
                j += 1;
            }
            if k >= LEADER_MIN {
                for m in mask.iter_mut().take(j).skip(i) {
                    *m = true;
                }
            }
            i = j;
        } else {
            i += 1;
        }
    }
    mask
}

/// The gap above which a space in this run is a column gutter (None: too few word gaps).
fn gutter_gap(chars: &[(Ch, [f64; 4])], h: f64) -> Option<f64> {
    let mut gaps = Vec::new();
    let (mut prev, mut spaced) = (None::<f64>, false);
    for (c, b) in chars {
        if c.is_space() {
            spaced = true;
            continue;
        }
        if let Some(p) = prev
            && spaced
        {
            gaps.push(b[0] - p);
        }
        prev = Some(b[2]);
        spaced = false;
    }
    if gaps.len() < 3 {
        return None;
    }
    Some((ADAPT_X * median(&mut gaps)).max(ADAPT_MIN * h))
}

/// Split one run (chars with reading-frame boxes) into units.
fn cut(chars: &[(Ch, [f64; 4])], h: f64) -> Vec<Unit> {
    let lead = leader_mask(chars);
    let gut = gutter_gap(chars, h);
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut cur: Vec<usize> = Vec::new();
    let mut prev: Option<f64> = None;
    let (mut word, mut last_word, mut spaced) = (String::new(), String::new(), false);
    for k in 0..chars.len() {
        let (c, b) = &chars[k];
        if lead[k] {
            if !cur.is_empty() {
                groups.push(std::mem::take(&mut cur));
                prev = None;
                word.clear();
                last_word.clear();
                spaced = false;
            }
            continue;
        }
        if c.is_space() {
            if !cur.is_empty() {
                cur.push(k);
                if !word.is_empty() {
                    last_word = std::mem::take(&mut word);
                }
                spaced = true;
            }
            continue;
        }
        let mut is_cut = prev.is_some_and(|p| b[0] - p > CELL_GAP * h);
        if !is_cut
            && spaced
            && let (Some(g), Some(p)) = (gut, prev)
            && b[0] - p > g
        {
            is_cut = true;
        }
        if !is_cut && spaced && prev.is_some_and(|p| b[0] - p > NUM_GAP * h) && is_num(&last_word) {
            let mut nxt = decode(&c.c);
            for (x, _) in chars.iter().skip(k + 1).take(23) {
                if x.is_space() {
                    break;
                }
                nxt.push_str(&decode(&x.c));
            }
            is_cut = is_num(&nxt);
        }
        if is_cut {
            groups.push(std::mem::take(&mut cur));
            last_word.clear();
        }
        cur.push(k);
        word.push_str(&decode(&c.c));
        spaced = false;
        prev = Some(b[2]);
    }
    if !cur.is_empty() {
        groups.push(cur);
    }
    let mut out = Vec::new();
    for g in groups {
        let ink: Vec<usize> = g
            .iter()
            .copied()
            .filter(|&k| !chars[k].0.is_space())
            .collect();
        if ink.is_empty() {
            continue;
        }
        let raw: Vec<u32> = g
            .iter()
            .flat_map(|&k| chars[k].0.c.iter().copied())
            .collect();
        let text = decode(&raw)
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if text.is_empty() {
            continue;
        }
        let mut bx = [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ];
        for &k in &ink {
            let b = chars[k].1;
            bx[0] = bx[0].min(b[0]);
            bx[1] = bx[1].min(b[1]);
            bx[2] = bx[2].max(b[2]);
            bx[3] = bx[3].max(b[3]);
        }
        let mut hs: Vec<f64> = ink.iter().map(|&k| chars[k].1[3] - chars[k].1[1]).collect();
        let mut sizes: Vec<f64> = ink.iter().map(|&k| chars[k].0.size).collect();
        let bolds: Vec<bool> = ink.iter().filter_map(|&k| chars[k].0.bold).collect();
        let bold = if bolds.is_empty() {
            None
        } else {
            Some(bolds.iter().filter(|&&b| b).count() as f64 > bolds.len() as f64 / 2.0)
        };
        let mut words = Vec::new();
        let mut w: Vec<usize> = Vec::new();
        for k in g.iter().copied().map(Some).chain([None]) {
            match k {
                Some(k) if !chars[k].0.is_space() => w.push(k),
                _ => {
                    if !w.is_empty() {
                        let x0 = w
                            .iter()
                            .map(|&k| chars[k].1[0])
                            .fold(f64::INFINITY, f64::min);
                        let x1 = w
                            .iter()
                            .map(|&k| chars[k].1[2])
                            .fold(f64::NEG_INFINITY, f64::max);
                        let raw: Vec<u32> = w
                            .iter()
                            .flat_map(|&k| chars[k].0.c.iter().copied())
                            .collect();
                        words.push((x0, x1, decode(&raw)));
                        w.clear();
                    }
                }
            }
        }
        out.push(Unit {
            bx,
            pbox: bx,
            text,
            h: median(&mut hs),
            size: median(&mut sizes),
            bold,
            q: 0,
            minor: false,
            words,
        });
    }
    out
}

/// Units of one raw run, built in the reading frame of direction q.
fn run_units(chars: &[Ch], q: u8, w: f64, h: f64, minor: bool) -> Vec<Unit> {
    let mut runs: Vec<Vec<(Ch, [f64; 4])>> = Vec::new();
    let mut cur: Vec<(Ch, [f64; 4])> = Vec::new();
    // apostrophes and quotes sit high, commas and periods low, combining marks above or below:
    // one right after a glyph rides with it, and the glyph after one is placed against the last
    // other glyph ("Treasurer's" stayed three units, "Treasurer ' s", and the "'s" could land in
    // another table row; Thai vowel and tone marks split "วันที่" into fake cells)
    let is_mark = |x: &Ch| {
        x.c.len() == 1
            && char::from_u32(x.c[0]).is_some_and(|c| {
                matches!(c, '\'' | '’' | '‘' | 'ʼ' | '`' | '´' | '"' | '“' | '”' | ',' | '.')
                    // Thai / Arabic / Indic marks stacked over or under their base
                    || crate::extract::is_combining_mark(c)
            })
    };
    for c in chars {
        let bx = to_frame(c.pb, q, w, h);
        if !cur.is_empty() && !c.is_space() {
            let prev = cur.iter().rev().find(|(x, _)| !x.is_space());
            let anchor = cur
                .iter()
                .rev()
                .find(|(x, _)| !x.is_space() && !is_mark(x))
                .or(prev);
            if let (Some((_, pb)), Some((_, bb))) = (prev, anchor) {
                let hh = (bb[3] - bb[1]).max(bx[3] - bx[1]).max(1.0);
                let rides = is_mark(c) && bx[0] >= pb[0] && bx[0] - pb[2] < 0.5 * hh;
                // the anchor only corrects the height a mark sits at; a step back is measured from
                // the glyph right before (in a right-to-left run "3.000" read "000 3." when the
                // digits after the point were measured from the "3")
                if !rides
                    && (((bx[1] + bx[3]) / 2.0 - (bb[1] + bb[3]) / 2.0).abs() > 0.6 * hh
                        || bx[0] < pb[0] - hh)
                {
                    runs.push(std::mem::take(&mut cur));
                }
            }
        }
        cur.push((c.clone(), bx));
    }
    if !cur.is_empty() {
        runs.push(cur);
    }
    let mut units = Vec::new();
    for run in runs {
        // the gap scale is a glyph height, from letters and digits only (flat "–" / "." markers
        // would pull the median to ~0 and cut every character apart)
        let alnum: Vec<f64> = run
            .iter()
            .filter(|(c, _)| {
                c.c.len() == 1 && char::from_u32(c.c[0]).is_some_and(|x| x.is_alphanumeric())
            })
            .map(|(_, b)| b[3] - b[1])
            .collect();
        let mut hs = if alnum.is_empty() {
            run.iter()
                .filter(|(c, _)| !c.is_space())
                .map(|(_, b)| b[3] - b[1])
                .collect()
        } else {
            alnum
        };
        if hs.is_empty() {
            continue;
        }
        let m = median(&mut hs);
        units.extend(cut(&run, m));
    }
    for u in units.iter_mut() {
        u.pbox = from_frame(u.bx, q, w, h);
        u.q = q;
        u.minor = minor;
    }
    units
}

/// Re-join pieces the baseline rule split but that touch (a superscript, a symbol in another font).
fn join_touching(units: Vec<Unit>) -> Vec<Unit> {
    let mut order: Vec<usize> = (0..units.len()).collect();
    order.sort_by(|&a, &b| units[a].bx[0].partial_cmp(&units[b].bx[0]).unwrap());
    let mut used = vec![false; units.len()];
    let mut out = Vec::new();
    for &i in &order {
        if used[i] {
            continue;
        }
        let u = &units[i];
        let (mut bx, mut text, mut h) = (u.bx, u.text.clone(), u.h);
        let mut words = u.words.clone();
        let mut grew = true;
        while grew {
            grew = false;
            for &j in &order {
                if used[j] || j == i {
                    continue;
                }
                let v = &units[j];
                if v.q != u.q {
                    continue;
                }
                let hh = h.min(v.h).max(1.0);
                let gap = v.bx[0] - bx[2];
                if -0.5 * hh <= gap
                    && gap < 0.1 * hh
                    && bx[3].min(v.bx[3]) - bx[1].max(v.bx[1]) > 0.0
                {
                    bx = [
                        bx[0].min(v.bx[0]),
                        bx[1].min(v.bx[1]),
                        bx[2].max(v.bx[2]),
                        bx[3].max(v.bx[3]),
                    ];
                    text.push_str(&v.text);
                    h = h.max(v.h);
                    // touching: v's first word continues u's last
                    let mut vw = v.words.iter().cloned();
                    if let (Some(a), false) = (words.last_mut(), v.words.is_empty()) {
                        let b = vw.next().unwrap();
                        *a = (a.0.min(b.0), a.1.max(b.1), format!("{}{}", a.2, b.2));
                    }
                    words.extend(vw);
                    used[j] = true;
                    grew = true;
                }
            }
        }
        used[i] = true;
        out.push(Unit {
            bx,
            pbox: bx,
            text,
            h,
            size: u.size,
            bold: u.bold,
            q: u.q,
            minor: u.minor,
            words,
        });
    }
    out.sort_by(|a, b| {
        (a.bx[1].round_ties_even() as i64, a.bx[0])
            .partial_cmp(&(b.bx[1].round_ties_even() as i64, b.bx[0]))
            .unwrap()
    });
    out
}

/// A header row PDFium emits as one run ("Saccharometer DI Water Glucose Solution ...") splits
/// where its words line up with the table body: cut at every gap no nearby table cell crosses (a
/// gutter), then fold back every piece whose left, centre or right edge matches no column edge
/// shared by two body cells. Kept only when it leaves 2+ aligned pieces of at most 5 words
/// (units.py `_split_aligned`).
fn split_aligned(units: Vec<Unit>) -> Vec<Unit> {
    let mut out = Vec::with_capacity(units.len());
    for (i, u) in units.iter().enumerate() {
        let pieces = if !u.minor && (2..=16).contains(&u.words.len()) {
            aligned_pieces(i, &units)
        } else {
            None
        };
        match pieces {
            Some(p) => out.extend(p),
            None => out.push(u.clone()),
        }
    }
    out
}

fn aligned_pieces(i: usize, units: &[Unit]) -> Option<Vec<Unit>> {
    let u = &units[i];
    let [x0, y0, x1, y1] = u.bx;
    let h = u.h.max(1.0);
    let tol = (0.25 * h).max(1.5);
    let cy = |v: &Unit| (v.bx[1] + v.bx[3]) / 2.0;
    // the body must look tabular: 2+ rows holding 2+ units each, or one row of 3+; its cells are
    // short units (4 words at most) within the run's span (a page header, caption or prose line of
    // a neighbouring column is no cell)
    let mut rows: Vec<(i64, Vec<&Unit>)> = Vec::new();
    for (j, v) in units.iter().enumerate() {
        let near = j != i
            && v.q == u.q
            && !v.minor
            && v.bx[0] < x1 + 2.0 * h
            && v.bx[2] > x0 - 2.0 * h
            && ((y1 - 0.2 * h <= v.bx[1] && v.bx[1] <= y1 + 6.0 * h)
                || (y0 - 6.0 * h <= v.bx[3] && v.bx[3] <= y0 + 0.2 * h));
        let nw = if v.words.is_empty() { 1 } else { v.words.len() };
        if near
            && nw <= 4
            && v.bx[2] - v.bx[0] < 0.6 * (x1 - x0)
            && v.bx[0] >= x0 - tol
            && v.bx[2] <= x1 + 2.0 * h
        {
            let key = (cy(v) / h).round_ties_even() as i64;
            match rows.iter_mut().find(|(k, _)| *k == key) {
                Some(r) => r.1.push(v),
                None => rows.push((key, vec![v])),
            }
        }
    }
    let tab: Vec<&Vec<&Unit>> = rows.iter().map(|r| &r.1).filter(|r| r.len() >= 2).collect();
    // a lone body row must be 3+ cells, mostly numbers or short marks ("N/A", "-")
    let datum = |v: &&&Unit| v.text.chars().any(|c| c.is_numeric()) || v.text.chars().count() <= 3;
    let lone_ok = tab.len() == 1
        && tab[0].len() >= 3
        && 2 * tab[0].iter().filter(datum).count() > tab[0].len();
    if !(tab.len() >= 2 || lone_ok) {
        return None;
    }
    let cells: Vec<&Unit> = tab.iter().flat_map(|r| r.iter().copied()).collect();
    // a column edge: shared by cells of two rows (any cell's edge, when one row is the body)
    let anchors = |f: &dyn Fn(&Unit) -> f64| -> Vec<f64> {
        let vals: Vec<(f64, f64)> = cells.iter().map(|v| (f(v), cy(v))).collect();
        if tab.len() == 1 {
            return vals.iter().map(|x| x.0).collect();
        }
        vals.iter()
            .filter(|&&(a, ya)| {
                vals.iter()
                    .any(|&(b, yb)| (a - b).abs() <= tol && (ya - yb).abs() > 0.5 * h)
            })
            .map(|x| x.0)
            .collect()
    };
    let lefts = anchors(&|v| v.bx[0]);
    let rights = anchors(&|v| v.bx[2]);
    let mids = anchors(&|v| (v.bx[0] + v.bx[2]) / 2.0);
    let words = &u.words;
    // a piece is a word range [a, b)
    let aligned = |a: usize, b: usize| {
        let (l, r) = (words[a].0, words[b - 1].1);
        lefts.iter().any(|&x| (l - x).abs() <= tol)
            || rights.iter().any(|&x| (r - x).abs() <= tol)
            || mids.iter().any(|&x| ((l + r) / 2.0 - x).abs() <= tol)
    };
    // no cell crosses mid, and every body row holding cells on both sides leaves a wide (2+ glyph
    // heights) gap there, in two rows at least: a gutter runs down the table, while justified
    // prose cut into pieces has its wide gaps strewn
    let gutter = |mid: f64| {
        if cells.iter().any(|v| v.bx[0] < mid && mid < v.bx[2]) {
            return false;
        }
        let mut spans = 0;
        for r in &tab {
            let left = r
                .iter()
                .filter(|v| v.bx[2] <= mid)
                .map(|v| v.bx[2])
                .fold(f64::NEG_INFINITY, f64::max);
            let right = r
                .iter()
                .filter(|v| v.bx[0] >= mid)
                .map(|v| v.bx[0])
                .fold(f64::INFINITY, f64::min);
            if left.is_finite() && right.is_finite() {
                if right - left < 2.0 * h {
                    return false;
                }
                spans += 1;
            }
        }
        spans >= tab.len().min(2)
    };
    let mut cuts: Vec<usize> = vec![0];
    for k in 1..words.len() {
        if gutter((words[k - 1].1 + words[k].0) / 2.0) {
            cuts.push(k);
        }
    }
    cuts.push(words.len());
    // pieces as consecutive cut pairs; fold unaligned pieces into an aligned merge
    while cuts.len() >= 3 {
        let n = cuts.len() - 1;
        let Some(bad) = (0..n).find(|&p| !aligned(cuts[p], cuts[p + 1])) else {
            break;
        };
        if bad > 0 && aligned(cuts[bad - 1], cuts[bad + 1]) {
            cuts.remove(bad);
        } else if bad + 1 < n && aligned(cuts[bad], cuts[bad + 2]) {
            cuts.remove(bad + 1);
        } else {
            return None;
        }
    }
    let n = cuts.len() - 1;
    if n < 2 || (0..n).any(|p| cuts[p + 1] - cuts[p] > 5) {
        return None;
    }
    // a piece opening in lower case continues a sentence ("kind of information | than we do")
    if (1..n).any(|p| {
        words[cuts[p]]
            .2
            .chars()
            .next()
            .is_some_and(|c| c.is_lowercase())
    }) {
        return None;
    }
    // each piece over its own column: no cell lies under two pieces
    for v in &cells {
        let over = (0..n)
            .filter(|&p| v.bx[2].min(words[cuts[p + 1] - 1].1) > v.bx[0].max(words[cuts[p]].0))
            .count();
        if over > 1 {
            return None;
        }
    }
    Some(
        (0..n)
            .map(|p| {
                let ws = words[cuts[p]..cuts[p + 1]].to_vec();
                Unit {
                    bx: [ws[0].0, y0, ws[ws.len() - 1].1, y1],
                    pbox: [ws[0].0, y0, ws[ws.len() - 1].1, y1],
                    text: ws
                        .iter()
                        .map(|w| w.2.as_str())
                        .collect::<Vec<_>>()
                        .join(" "),
                    h: u.h,
                    size: u.size,
                    bold: u.bold,
                    q: u.q,
                    minor: u.minor,
                    words: ws,
                }
            })
            .collect(),
    )
}

/// Single characters stacked in a column (vertical text emitted one glyph per run) are minor -
/// unless most of them have row-mates (another unit centred on the same line): that is a table
/// column of one-glyph cells (digits, check marks), not vertical text.
fn flag_letter_stacks(units: &mut [Unit]) {
    let mut singles: Vec<usize> = (0..units.len())
        .filter(|&i| units[i].text.chars().count() == 1 && !units[i].minor)
        .collect();
    let cx = |u: &Unit| (u.pbox[0] + u.pbox[2]) / 2.0;
    singles.sort_by(|&a, &b| {
        (cx(&units[a]).round_ties_even() as i64, units[a].pbox[1])
            .partial_cmp(&(cx(&units[b]).round_ties_even() as i64, units[b].pbox[1]))
            .unwrap()
    });
    let mut used = vec![false; units.len()];
    for s in 0..singles.len() {
        let u = singles[s];
        if used[u] {
            continue;
        }
        let mut chain = vec![u];
        let mut last = u;
        for &v in &singles[s + 1..] {
            let (cl, cv) = (cx(&units[last]), cx(&units[v]));
            let lb = units[last].pbox;
            let hh = (lb[3] - lb[1]).max(lb[2] - lb[0]).max(1.0);
            let dy = units[v].pbox[1] - lb[3];
            if (cv - cl).abs() < 0.6 * hh && 0.0 <= dy && dy < 1.5 * hh {
                chain.push(v);
                last = v;
            }
        }
        if chain.len() >= 3 && !row_mates(units, &chain) {
            for v in chain {
                units[v].minor = true;
                used[v] = true;
            }
        }
    }
}

/// Whether at least 80% of `chain` have another unit (not in the chain) centred on their line.
fn row_mates(units: &[Unit], chain: &[usize]) -> bool {
    let cy = |u: &Unit| (u.pbox[1] + u.pbox[3]) / 2.0;
    let mated = chain
        .iter()
        .filter(|&&v| {
            let b = units[v].pbox;
            let tol = 0.3 * (b[3] - b[1]).max(1.0);
            units.iter().enumerate().any(|(w, u)| {
                !chain.contains(&w) && !u.minor && (cy(u) - cy(&units[v])).abs() < tol
            })
        })
        .count();
    mated * 5 >= chain.len() * 4
}

/// Bold from the character's font: weight >= 600, or a bold-ish font name.
fn char_bold(c: &TextChar) -> Option<bool> {
    let w = c.font_weight();
    let name = c.font_name();
    if w < 0 && name.is_none() {
        return None;
    }
    let name = name.unwrap_or_default();
    Some(
        w >= 600
            || ["Bold", "Black", "Heavy", "Semibold", ",B"]
                .iter()
                .any(|k| name.contains(k)),
    )
}

/// Units of a page's text layer (empty when it has none). Also returns the rendered page size.
pub fn pdf_units(page: &Page) -> (Vec<Unit>, (f64, f64)) {
    let Some(vb) = page.view_box() else {
        return (Vec::new(), (0.0, 0.0));
    };
    let (cl, cb, cr, ct) = (
        vb.left as f64,
        vb.bottom as f64,
        vb.right as f64,
        vb.top as f64,
    );
    let (w0, h0) = (cr - cl, ct - cb);
    let rot = page.rotation().rem_euclid(4);
    let (w, h) = if rot % 2 == 1 { (h0, w0) } else { (w0, h0) };
    let Ok(tp) = page.text() else {
        return (Vec::new(), (w, h));
    };
    // Some(char) or None for a generated line break
    let mut chars: Vec<Option<Ch>> = Vec::new();
    for c in tp.chars() {
        let u = c.unicode();
        let ch = char::from_u32(u);
        if c.is_generated() && matches!(ch, Some('\r') | Some('\n')) {
            chars.push(None);
            continue;
        }
        if u == 0 || matches!(ch, Some('\r') | Some('\n') | Some('\u{fffe}')) {
            continue;
        }
        // PDFium reports U+0002 both for a hyphen it found at a line end and for a font that
        // encodes a drawn '-' (often a minus) as code 0x02 with no ToUnicode map. The line-end
        // hyphen is a wrapped cell's word break, kept as a SOFT HYPHEN so markdown can rejoin the
        // word (or print the hyphen before a capital); the real code reads as '-', since
        // dropping it flips the sign of negative table values.
        let (u, ch) = match u {
            0x02 if c.is_hyphen() => (0x00AD, Some('\u{00AD}')),
            0x02 => (u32::from('-'), Some('-')),
            _ => (u, ch),
        };
        let Some(b) = c.char_box() else { continue };
        let pb = rotate_page(
            [b.left - cl, ct - b.top, b.right - cl, ct - b.bottom],
            rot,
            w0,
            h0,
        );
        let space = ch.is_some_and(|x| x.is_whitespace());
        if pb[2] <= pb[0] && pb[3] <= pb[1] && !space {
            continue;
        }
        chars.push(Some(Ch {
            c: vec![u],
            pb,
            size: c.font_size(),
            bold: char_bold(&c),
        }));
    }
    // raw runs between PDFium's breaks
    let mut raw_runs: Vec<Vec<Ch>> = Vec::new();
    let mut cur: Vec<Ch> = Vec::new();
    for c in chars {
        match c {
            None => {
                if !cur.is_empty() {
                    raw_runs.push(std::mem::take(&mut cur));
                }
            }
            Some(c) => cur.push(c),
        }
    }
    if !cur.is_empty() {
        raw_runs.push(cur);
    }
    // reading direction per run by how consecutive glyphs advance; votes kept in first-seen order
    // (Counter.most_common's tie order)
    let direction = |run: &[Ch]| -> Vec<(u8, usize)> {
        let mut v: Vec<(u8, usize)> = Vec::new();
        let mut prev: Option<&Ch> = None;
        for c in run {
            if c.is_space() {
                continue;
            }
            if let Some(p) = prev {
                let dx = (c.pb[0] + c.pb[2] - p.pb[0] - p.pb[2]) / 2.0;
                let dy = (c.pb[1] + c.pb[3] - p.pb[1] - p.pb[3]) / 2.0;
                let sz = (c.pb[2] - c.pb[0]).max(c.pb[3] - c.pb[1]).max(1.0);
                let d = dx.abs() + dy.abs();
                if 0.0 < d && d < 3.0 * sz {
                    let q = if dx.abs() >= dy.abs() {
                        if dx > 0.0 { 0 } else { 2 }
                    } else if dy < 0.0 {
                        1
                    } else {
                        3
                    };
                    match v.iter_mut().find(|(k, _)| *k == q) {
                        Some(e) => e.1 += 1,
                        None => v.push((q, 1)),
                    }
                }
            }
            prev = Some(c);
        }
        v
    };
    let most = |v: &[(u8, usize)]| -> Option<u8> {
        let mut best: Option<(u8, usize)> = None;
        for &(k, n) in v {
            if best.is_none_or(|(_, bn)| n > bn) {
                best = Some((k, n));
            }
        }
        best.map(|b| b.0)
    };
    let dirs: Vec<Vec<(u8, usize)>> = raw_runs.iter().map(|r| direction(r)).collect();
    let mut page_votes: Vec<(u8, usize)> = Vec::new();
    for d in &dirs {
        for &(k, n) in d {
            match page_votes.iter_mut().find(|(q, _)| *q == k) {
                Some(e) => e.1 += n,
                None => page_votes.push((k, n)),
            }
        }
    }
    let q = most(&page_votes).unwrap_or(0);
    let mut units = Vec::new();
    for (run, v) in raw_runs.iter().zip(&dirs) {
        let total: usize = v.iter().map(|x| x.1).sum();
        let rq = if !v.is_empty() && total >= 2 {
            most(v).unwrap()
        } else {
            q
        };
        units.extend(run_units(run, rq, w, h, rq != q));
    }
    let mut units = join_touching(units);
    units = split_aligned(units);
    for u in units.iter_mut() {
        u.pbox = from_frame(u.bx, u.q, w, h);
    }
    flag_letter_stacks(&mut units);
    (units, (w, h))
}
