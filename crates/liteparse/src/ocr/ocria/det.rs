//! Text-line detector: a segmentation map from the ONNX model, then connected components
//! expanded to line quads (`lpocr_rec.Detector`).

use super::img::{Img, box_points, components4, min_area_rect, resize_area};
use super::pool::SessionPool;
use super::{OcriaError, Quad};

/// Kernel shrink ratio the detector targets were built with.
const SHRINK: f32 = 0.3;

pub struct Detector {
    pub(super) pool: SessionPool,
    pub long_side: usize,
    pub thresh: f32,
    pub min_score: f32,
    /// Join collinear boxes with gaps under this many line heights (0 = off).
    pub merge: f32,
    /// Re-detect at a larger size when lines are small (see [`Adaptive`]).
    pub adaptive: Option<Adaptive>,
    /// Also return faint short tokens next to detected lines (see [`Faint`]).
    pub faint: Option<Faint>,
}

/// Short tokens the detector scores under its threshold. A lone narrow glyph in a table cell
/// (a "1", an "l") looks like a rule fragment, which the detector learned to suppress: the
/// "1"s of issue #615 peak at 0.13 and 0.52 against 0.55, at every detection size. Regions of
/// the same probability map above `thresh` that touch no detected line become candidate boxes
/// when they sit in the middle of a detected line's row and are no taller than that line; at
/// most `max` per page, strongest first. The caller reads them and keeps confident readings.
#[derive(Clone, Copy, Debug)]
pub struct Faint {
    pub thresh: f32,
    pub max: usize,
}

/// A second detection pass for small print. The page size a PDF declares says nothing about
/// its text size (a broadsheet page saved as letter has 3 pt body text), so the trigger is
/// the measured line height: when the `quantile` of line heights at the first pass's scale,
/// weighted by line length, is under `min_h` input pixels (and the page has at least 8 lines), detect again with the
/// long side scaled to bring those lines to `target_h`, at most `max_side` (and never above
/// the image's own size). The two passes are then merged ([`merge_scales`]): the second
/// pass's lines, plus first-pass lines it missed or broke up.
#[derive(Clone, Copy, Debug)]
pub struct Adaptive {
    pub min_h: f32,
    pub target_h: f32,
    pub max_side: usize,
    /// Which line height decides: the height this share (0..1) of the page's total line
    /// length lies under. The first pass misses much of a page's small print, so the lines
    /// it does find there are a minority of what it measures.
    pub quantile: f32,
}

/// The second pass is skipped unless it enlarges the detector input by at least this much.
const ADAPTIVE_MIN_GAIN: f32 = 1.15;
/// Boxes lower than this at the first pass's scale are not counted as text lines when
/// measuring the page: dot leaders and rule fragments come out 4-5 px high, the 3 pt print of
/// a broadsheet page 10 px.
const ADAPTIVE_MIN_TEXT_H: f32 = 6.0;
/// First-pass lines this many times the small print's height are display type, which the
/// second pass may break up ([`merge_scales`]). Lower ones are left to the second pass: on the
/// broadsheet page of the measurements, keeping first-pass lines of every height lost 850
/// characters (they run across column gutters or are cut short).
const DISPLAY_RATIO: f32 = 3.0;

impl Detector {
    pub fn new(pool: SessionPool, long_side: usize, thresh: f32, merge: f32) -> Self {
        Detector {
            pool,
            long_side,
            thresh,
            min_score: 0.6,
            merge,
            adaptive: None,
            faint: None,
        }
    }

    /// Line quads (TL, TR, BR, BL along the line) in `img` pixels, in reading-ish order.
    pub fn detect(&self, img: &Img) -> Result<Vec<Quad>, OcriaError> {
        Ok(self.detect_with_faint(img)?.0)
    }

    /// Line quads as [`Detector::detect`], and the faint short-token candidates of the same pass
    /// (empty without [`Detector::faint`]).
    pub fn detect_with_faint(&self, img: &Img) -> Result<(Vec<Quad>, Vec<Quad>), OcriaError> {
        let (boxes, scale, faint) = self.detect_at(img, self.long_side)?;
        let Some(a) = self.adaptive else {
            return Ok((boxes, faint));
        };
        let Some((side, small_h)) = self.adaptive_side(&a, &boxes, scale, img) else {
            return Ok((boxes, faint));
        };
        let (fine, _, faint) = self.detect_at(img, side)?;
        let mut merged = merge_scales(&boxes, &fine, DISPLAY_RATIO * small_h);
        sort_reading(&mut merged);
        // a faint candidate inside a line kept from the first pass is part of that line
        let faint = faint
            .into_iter()
            .filter(|f| {
                let c = centre(f);
                !merged.iter().any(|q| inside(q, c))
            })
            .collect();
        Ok((merged, faint))
    }

    /// The long side for [`Adaptive`]'s second pass and the small print's line height in image
    /// pixels, or `None` when the page needs no second pass.
    fn adaptive_side(
        &self,
        a: &Adaptive,
        boxes: &[Quad],
        scale: f64,
        img: &Img,
    ) -> Option<(usize, f32)> {
        if boxes.len() < 8 {
            return None;
        }
        // (height, length) per line at the pass's scale, without rules, dot leaders and specks
        let mut hs: Vec<(f32, f32)> = boxes
            .iter()
            .map(|q| {
                let len = norm([q[1][0] - q[0][0], q[1][1] - q[0][1]]);
                (quad_h(q) * scale as f32, len)
            })
            .filter(|&(h, _)| h >= ADAPTIVE_MIN_TEXT_H)
            .collect();
        hs.sort_by(|a, b| a.0.total_cmp(&b.0));
        // the height `quantile` of the page's line length lies under: a count of boxes
        // would let a poster's specks or a contents page's fragments outvote its text
        let total: f32 = hs.iter().map(|x| x.1).sum();
        let mut acc = 0f32;
        let mut h = hs.last()?.0;
        for &(hk, len) in &hs {
            acc += len;
            if acc >= a.quantile.clamp(0.0, 1.0) * total {
                h = hk;
                break;
            }
        }
        if h >= a.min_h {
            return None;
        }
        let side = ((self.long_side as f32 * a.target_h / h) as usize).min(a.max_side) / 32 * 32;
        // a pass never enlarges the page: at the image's own size it runs unscaled, which
        // reads small print better than a resize that is a few percent short of it
        let side = side.min(img.w.max(img.h));
        (side as f32 >= self.long_side as f32 * ADAPTIVE_MIN_GAIN)
            .then_some((side, h / scale as f32))
    }

    /// One detection pass with the page's long side resized to `long_side` (never enlarged);
    /// also returns the resize scale and the faint candidates.
    fn detect_at(
        &self,
        img: &Img,
        long_side: usize,
    ) -> Result<(Vec<Quad>, f64, Vec<Quad>), OcriaError> {
        let (big_h, big_w) = (img.h as f64, img.w as f64);
        let s = (long_side as f64 / big_h.max(big_w)).min(1.0);
        let h = 32usize.max(((big_h * s / 32.0).round_ties_even() as usize) * 32);
        let w = 32usize.max(((big_w * s / 32.0).round_ties_even() as usize) * 32);
        let (sx, sy) = (w as f64 / big_w, h as f64 / big_h);
        let small = resize_area(img, w, h);
        let mut x = vec![0f32; 3 * h * w];
        for c in 0..3 {
            for i in 0..h * w {
                x[c * h * w + i] = small.data[i * 3 + c] as f32 / 127.5 - 1.0;
            }
        }
        let (shape, prob) = self.pool.infer([1, 3, h, w], x, &["prob"])?.f32("prob")?;
        let (ph, pw) = (shape[2], shape[3]);
        let stride = (h / ph) as f32;
        let mask: Vec<bool> = prob.iter().map(|&p| p > self.thresh).collect();
        let mut boxes = Vec::new();
        for comp in components4(&mask, pw, ph) {
            if comp.pixels.len() < 2 {
                continue;
            }
            let mean = comp
                .pixels
                .iter()
                .map(|&(x, y)| prob[y * pw + x] as f64)
                .sum::<f64>()
                / comp.pixels.len() as f64;
            if (mean as f32) < self.min_score {
                continue;
            }
            // the hull of the component's pixel cells: each row's leftmost and rightmost cell
            let rows = comp.y1 - comp.y0 + 1;
            let mut span = vec![(usize::MAX, 0usize); rows];
            for &(x, y) in &comp.pixels {
                let r = &mut span[y - comp.y0];
                r.0 = r.0.min(x);
                r.1 = r.1.max(x);
            }
            let mut pts = Vec::with_capacity(rows * 4);
            for (i, &(x0, x1)) in span.iter().enumerate() {
                if x0 == usize::MAX {
                    continue;
                }
                let y = (comp.y0 + i) as f32;
                for (px, py) in [
                    (x0 as f32, y),
                    (x0 as f32, y + 1.0),
                    (x1 as f32 + 1.0, y),
                    (x1 as f32 + 1.0, y + 1.0),
                ] {
                    pts.push((px * stride, py * stride));
                }
            }
            let mut r = min_area_rect(&pts);
            // line height is the side nearer vertical, not the shorter side: a single digit's
            // kernel is taller than it is wide
            let w_is_horizontal = ((r.angle + 90.0).rem_euclid(180.0) - 90.0).abs() < 45.0;
            let kernel_h = if w_is_horizontal { r.h } else { r.w };
            let line_h = kernel_h / (1.0 - 2.0 * SHRINK);
            let d = SHRINK * line_h;
            r.w += 2.0 * d;
            r.h += 2.0 * d;
            let mut quad = box_points(&r);
            for p in quad.iter_mut() {
                p[0] = ((p[0] as f64 / sx) as f32).clamp(0.0, img.w as f32);
                p[1] = ((p[1] as f64 / sy) as f32).clamp(0.0, img.h as f32);
            }
            boxes.push(order_quad(&quad));
        }
        if self.merge > 0.0 {
            boxes = merge_collinear(&boxes, self.merge);
        }
        sort_reading(&mut boxes);
        let faint = match self.faint {
            Some(f) => {
                let to_img = |x: usize, y: usize| {
                    (
                        ((x as f32 * stride) as f64 / sx) as f32,
                        ((y as f32 * stride) as f64 / sy) as f32,
                    )
                };
                faint_boxes(&prob, &mask, pw, ph, to_img, &boxes, f, img)
            }
            None => Vec::new(),
        };
        Ok((boxes, s, faint))
    }
}

/// Candidate boxes for [`Faint`] tokens, in `img` pixels. `to_img` maps a probability-map cell
/// corner to image pixels; `boxes` are the detected lines.
#[allow(clippy::too_many_arguments)]
fn faint_boxes(
    prob: &[f32],
    mask: &[bool],
    pw: usize,
    ph: usize,
    to_img: impl Fn(usize, usize) -> (f32, f32),
    boxes: &[Quad],
    f: Faint,
    img: &Img,
) -> Vec<Quad> {
    // a candidate keeps the side margin a detected line has around its text
    const PAD: f32 = 0.35;
    let bbox = |q: &Quad| {
        let xs = q.iter().map(|p| p[0]);
        let ys = q.iter().map(|p| p[1]);
        (
            xs.clone().fold(f32::INFINITY, f32::min),
            ys.clone().fold(f32::INFINITY, f32::min),
            xs.fold(f32::NEG_INFINITY, f32::max),
            ys.fold(f32::NEG_INFINITY, f32::max),
        )
    };
    let lines: Vec<(f32, f32, f32, f32)> = boxes.iter().map(bbox).collect();
    // rows: lines that run level, so their box is the row's band
    let rows: Vec<(f32, f32, f32, f32)> = boxes
        .iter()
        .zip(&lines)
        .filter(|(q, l)| (q[1][1] - q[0][1]).abs() <= 0.1 * (l.2 - l.0))
        .map(|(_, &l)| l)
        .collect();
    if rows.is_empty() {
        return Vec::new();
    }
    let weak: Vec<bool> = prob.iter().map(|&p| p > f.thresh).collect();
    let mut cands: Vec<(f32, Quad)> = Vec::new();
    for comp in components4(&weak, pw, ph) {
        if comp.pixels.iter().any(|&(x, y)| mask[y * pw + x]) {
            continue;
        }
        let (x0, y0) = to_img(comp.x0, comp.y0);
        let (x1, y1) = to_img(comp.x1 + 1, comp.y1 + 1);
        let cy = (y0 + y1) / 2.0;
        // the row: a level line whose middle half holds the region's centre
        let Some(&(_, ry0, _, ry1)) = rows
            .iter()
            .filter(|r| {
                let h = r.3 - r.1;
                cy >= r.1 + 0.25 * h && cy <= r.3 - 0.25 * h
            })
            .min_by(|a, b| {
                (a.1 + a.3 - 2.0 * cy)
                    .abs()
                    .total_cmp(&(b.1 + b.3 - 2.0 * cy).abs())
            })
        else {
            continue;
        };
        let h = ry1 - ry0;
        // a short token's core is lower than its row's line box and at most a few glyphs wide
        if y1 - y0 > 0.8 * h || x1 - x0 > 1.5 * h {
            continue;
        }
        let (bx0, bx1) = ((x0 - PAD * h).max(0.0), (x1 + PAD * h).min(img.w as f32));
        let (by0, by1) = (ry0.max(0.0), ry1.min(img.h as f32));
        let area = (bx1 - bx0) * (by1 - by0);
        // not part of a detected line (a gap between words, an accent, a descender)
        let covered = lines.iter().any(|l| {
            let ix = (bx1.min(l.2) - bx0.max(l.0)).max(0.0);
            let iy = (by1.min(l.3) - by0.max(l.1)).max(0.0);
            ix * iy > 0.2 * area
        });
        if covered {
            continue;
        }
        let peak = comp
            .pixels
            .iter()
            .map(|&(x, y)| prob[y * pw + x])
            .fold(0f32, f32::max);
        cands.push((peak, [[bx0, by0], [bx1, by0], [bx1, by1], [bx0, by1]]));
    }
    cands.sort_by(|a, b| b.0.total_cmp(&a.0));
    cands.truncate(f.max);
    cands.into_iter().map(|(_, q)| q).collect()
}

/// Reading-ish order: row band of 8 px, then left edge (a stable sort, as Python's).
fn sort_reading(boxes: &mut [Quad]) {
    boxes.sort_by(|a, b| {
        let ka = ((min_y(a) as f64 / 8.0).round_ties_even(), min_x(a));
        let kb = ((min_y(b) as f64 / 8.0).round_ties_even(), min_x(b));
        ka.0.total_cmp(&kb.0).then(ka.1.total_cmp(&kb.1))
    });
}

/// A line quad's height: its left edge.
fn quad_h(q: &Quad) -> f32 {
    norm([q[3][0] - q[0][0], q[3][1] - q[0][1]])
}

fn centre(q: &Quad) -> [f32; 2] {
    [
        (q[0][0] + q[1][0] + q[2][0] + q[3][0]) / 4.0,
        (q[0][1] + q[1][1] + q[2][1] + q[3][1]) / 4.0,
    ]
}

/// (x0, y0, x1, y1) of a quad.
fn extent(q: &Quad) -> (f32, f32, f32, f32) {
    let xs = q.iter().map(|p| p[0]);
    let ys = q.iter().map(|p| p[1]);
    (
        xs.clone().fold(f32::INFINITY, f32::min),
        ys.clone().fold(f32::INFINITY, f32::min),
        xs.fold(f32::NEG_INFINITY, f32::max),
        ys.fold(f32::NEG_INFINITY, f32::max),
    )
}

/// Whether `p` lies in the quad's extent.
fn inside(q: &Quad, p: [f32; 2]) -> bool {
    let (x0, y0, x1, y1) = extent(q);
    p[0] >= x0 && p[0] <= x1 && p[1] >= y0 && p[1] <= y1
}

/// The lines of a page detected at two scales: every `fine` (enlarged pass) line, except that
/// a `coarse` (first pass) line at least `display_h` tall replaces the fine lines inside it
/// when those are pieces of the same line, and is kept when the fine pass has nothing under it.
///
/// Enlarged, small print separates into its lines, but display type can outgrow the detector
/// and come back in pieces ("Grand" for a 79 px "Grand Opening!") or not at all. The first
/// pass has those lines whole. Its boxes over small print are the opposite case, one blob
/// over several stacked lines, and are dropped: the fine lines under a blob are much lower
/// than it, the pieces of a headline as tall as it.
fn merge_scales(coarse: &[Quad], fine: &[Quad], display_h: f32) -> Vec<Quad> {
    // a fine line this share of the coarse line's height is the same line, not one stacked in it
    const SAME_LINE_H: f32 = 0.6;
    // the fine pass covers less than this share of a coarse line: it missed the line
    const MISSED: f32 = 0.2;
    // lower fine lines fill this share of a coarse box: it is a blob over stacked lines
    const BLOB: f32 = 0.4;
    // one fine line spanning this share of the coarse line's width already is that line
    const WHOLE: f32 = 0.8;
    let mut drop_fine = vec![false; fine.len()];
    let mut kept = Vec::new();
    for b in coarse.iter().filter(|b| quad_h(b) >= display_h) {
        let (bx0, by0, bx1, by1) = extent(b);
        let area = ((bx1 - bx0) * (by1 - by0)).max(1.0);
        let mut covered = 0f32;
        for f in fine {
            let (x0, y0, x1, y1) = extent(f);
            covered += (x1.min(bx1) - x0.max(bx0)).max(0.0) * (y1.min(by1) - y0.max(by0)).max(0.0);
        }
        if covered < MISSED * area {
            kept.push(*b);
            continue;
        }
        let within: Vec<usize> = (0..fine.len())
            .filter(|&i| !drop_fine[i] && inside(b, centre(&fine[i])))
            .collect();
        let (pieces, lower): (Vec<usize>, Vec<usize>) = within
            .iter()
            .partition(|&&i| quad_h(&fine[i]) >= SAME_LINE_H * quad_h(b));
        let lower_area: f32 = lower
            .iter()
            .map(|&i| {
                let e = extent(&fine[i]);
                (e.2 - e.0) * (e.3 - e.1)
            })
            .sum();
        if pieces.is_empty() || lower_area >= BLOB * area {
            continue;
        }
        let span = pieces
            .iter()
            .map(|&i| {
                let e = extent(&fine[i]);
                e.2 - e.0
            })
            .fold(0f32, f32::max);
        if pieces.len() >= 2 || span < WHOLE * (bx1 - bx0) {
            kept.push(*b);
            for i in pieces {
                drop_fine[i] = true;
            }
        }
    }
    kept.extend(
        fine.iter()
            .zip(&drop_fine)
            .filter(|(_, d)| !**d)
            .map(|(f, _)| *f),
    );
    kept
}

fn min_y(q: &Quad) -> f32 {
    q.iter().map(|p| p[1]).fold(f32::INFINITY, f32::min)
}

fn min_x(q: &Quad) -> f32 {
    q.iter().map(|p| p[0]).fold(f32::INFINITY, f32::min)
}

fn norm(v: [f32; 2]) -> f32 {
    (v[0] * v[0] + v[1] * v[1]).sqrt()
}

/// Corners as [top-left, top-right, bottom-right, bottom-left] along the line's long axis.
pub fn order_quad(q: &Quad) -> Quad {
    let c = [
        (q[0][0] + q[1][0] + q[2][0] + q[3][0]) / 4.0,
        (q[0][1] + q[1][1] + q[2][1] + q[3][1]) / 4.0,
    ];
    let e1 = [q[1][0] - q[0][0], q[1][1] - q[0][1]];
    let e2 = [q[2][0] - q[1][0], q[2][1] - q[1][1]];
    let mut u = if norm(e1) >= norm(e2) { e1 } else { e2 };
    let n = norm(u) + 1e-6;
    u = [u[0] / n, u[1] / n];
    if u[0] < 0.0 {
        u = [-u[0], -u[1]];
    }
    let v = [-u[1], u[0]];
    let a: Vec<f32> = q
        .iter()
        .map(|p| (p[0] - c[0]) * u[0] + (p[1] - c[1]) * u[1])
        .collect();
    let b: Vec<f32> = q
        .iter()
        .map(|p| (p[0] - c[0]) * v[0] + (p[1] - c[1]) * v[1])
        .collect();
    // np.argmin / argmax: the first extreme index
    let pick = |f: &dyn Fn(usize) -> f32, max: bool| -> usize {
        let mut best = 0;
        for i in 1..4 {
            if (max && f(i) > f(best)) || (!max && f(i) < f(best)) {
                best = i;
            }
        }
        best
    };
    let tl = pick(&|i| a[i] + b[i], false);
    let tr = pick(&|i| a[i] - b[i], true);
    let br = pick(&|i| a[i] + b[i], true);
    let bl = pick(&|i| a[i] - b[i], false);
    [q[tl], q[tr], q[br], q[bl]]
}

/// Join near-horizontal line quads that continue one another (`lpocr_rec._merge_collinear`):
/// each box links to its nearest qualifying right neighbour when that neighbour's nearest left
/// neighbour is the same box, and each chain becomes the rectangle spanning it along its
/// longest member's axis.
pub fn merge_collinear(boxes: &[Quad], gap: f32) -> Vec<Quad> {
    const MAX_TILT: f32 = 0.35;
    const MAX_PERP: f32 = 0.35;
    const MIN_H_RATIO: f32 = 0.6;
    const MAX_DANGLE: f32 = 0.07;
    let n = boxes.len();
    if n < 2 {
        return boxes.to_vec();
    }
    let c: Vec<[f32; 2]> = boxes
        .iter()
        .map(|q| {
            [
                (q[0][0] + q[1][0] + q[2][0] + q[3][0]) / 4.0,
                (q[0][1] + q[1][1] + q[2][1] + q[3][1]) / 4.0,
            ]
        })
        .collect();
    let mut u: Vec<[f32; 2]> = Vec::with_capacity(n);
    let mut len: Vec<f32> = Vec::with_capacity(n);
    let mut hgt: Vec<f32> = Vec::with_capacity(n);
    for q in boxes {
        let d = [q[1][0] - q[0][0], q[1][1] - q[0][1]];
        let l = norm(d).max(1e-3);
        u.push([d[0] / l, d[1] / l]);
        len.push(l);
        hgt.push(norm([q[3][0] - q[0][0], q[3][1] - q[0][1]]).max(1e-3));
    }
    let v: Vec<[f32; 2]> = u.iter().map(|u| [-u[1], u[0]]).collect();
    let flat: Vec<bool> = (0..n)
        .map(|i| u[i][1].abs() <= MAX_TILT && len[i] >= 0.5 * hgt[i])
        .collect();
    let cos_min = 1.0 - MAX_DANGLE * MAX_DANGLE / 2.0;
    // g[a][b] where the link a -> b qualifies, else +inf
    let mut right = vec![-1isize; n];
    let mut left_best = vec![(f32::INFINITY, -1isize); n];
    for a in 0..n {
        let mut best = (f32::INFINITY, -1isize);
        if flat[a] {
            for b in 0..n {
                if a == b || !flat[b] {
                    continue;
                }
                let d = [c[b][0] - c[a][0], c[b][1] - c[a][1]];
                let along = d[0] * u[a][0] + d[1] * u[a][1];
                let perp = (d[0] * v[a][0] + d[1] * v[a][1]).abs();
                let hm = (hgt[a] + hgt[b]) / 2.0;
                let g = along - (len[a] + len[b]) / 2.0;
                let ok = along > 0.0
                    && hgt[a].min(hgt[b]) / hgt[a].max(hgt[b]) >= MIN_H_RATIO
                    && (u[a][0] * u[b][0] + u[a][1] * u[b][1]).abs() >= cos_min
                    && perp <= MAX_PERP * hm
                    && g <= gap * hm
                    && g >= -0.5 * len[a].min(len[b]);
                if ok {
                    if g < best.0 {
                        best = (g, b as isize);
                    }
                    // column-wise argmin for b: first a reaching the minimum
                    if g < left_best[b].0 {
                        left_best[b] = (g, a as isize);
                    }
                }
            }
        }
        right[a] = best.1;
    }
    let mut nxt = vec![-1isize; n];
    for a in 0..n {
        let b = right[a];
        if b >= 0 && left_best[b as usize].1 == a as isize {
            nxt[a] = b;
        }
    }
    let mut has_prev = vec![false; n];
    for &b in &nxt {
        if b >= 0 {
            has_prev[b as usize] = true;
        }
    }
    let mut out = Vec::new();
    let mut seen = vec![false; n];
    for a in 0..n {
        if has_prev[a] || seen[a] {
            continue;
        }
        let mut chain = vec![a];
        seen[a] = true;
        while let Some(&last) = chain.last() {
            let nx = nxt[last];
            if nx < 0 || seen[nx as usize] {
                break;
            }
            chain.push(nx as usize);
            seen[nx as usize] = true;
        }
        if chain.len() == 1 {
            out.push(boxes[a]);
            continue;
        }
        let pts: Vec<[f32; 2]> = chain.iter().flat_map(|&i| boxes[i]).collect();
        // the chain's direction is its longest member's (first one on ties, as max())
        let mut k = chain[0];
        for &i in &chain[1..] {
            if len[i] > len[k] {
                k = i;
            }
        }
        let np = pts.len() as f32;
        let ca = [
            pts.iter().map(|p| p[0]).sum::<f32>() / np,
            pts.iter().map(|p| p[1]).sum::<f32>() / np,
        ];
        let (ua, va) = (u[k], v[k]);
        let (mut a0, mut a1, mut b0, mut b1) = (
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::INFINITY,
            f32::NEG_INFINITY,
        );
        for p in &pts {
            let (dx, dy) = (p[0] - ca[0], p[1] - ca[1]);
            let pa = dx * ua[0] + dy * ua[1];
            let pb = dx * va[0] + dy * va[1];
            a0 = a0.min(pa);
            a1 = a1.max(pa);
            b0 = b0.min(pb);
            b1 = b1.max(pb);
        }
        let at = |s: f32, t: f32| [ca[0] + ua[0] * s + va[0] * t, ca[1] + ua[1] * s + va[1] * t];
        out.push(order_quad(&[
            at(a0, b0),
            at(a1, b0),
            at(a1, b1),
            at(a0, b1),
        ]));
    }
    for i in 0..n {
        if !seen[i] {
            out.push(boxes[i]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 60 x 20 probability map at one image pixel per cell, with one detected line (cells
    /// x 2..20, y 8..12) whose box is 20 px tall.
    fn page() -> (Vec<f32>, Vec<bool>, Vec<Quad>, Img) {
        let (pw, ph) = (60, 20);
        let mut prob = vec![0f32; pw * ph];
        for y in 8..12 {
            for x in 2..20 {
                prob[y * pw + x] = 0.9;
            }
        }
        let mask = prob.iter().map(|&p| p > 0.55).collect();
        let line = [[0.0, 0.0], [22.0, 0.0], [22.0, 20.0], [0.0, 20.0]];
        let img = Img {
            w: pw,
            h: ph,
            ch: 3,
            data: vec![255; pw * ph * 3],
        };
        (prob, mask, vec![line], img)
    }

    fn faint(prob: &[f32], mask: &[bool], boxes: &[Quad], img: &Img) -> Vec<Quad> {
        let f = Faint {
            thresh: 0.1,
            max: 24,
        };
        faint_boxes(
            prob,
            mask,
            60,
            20,
            |x, y| (x as f32, y as f32),
            boxes,
            f,
            img,
        )
    }

    fn rect(x0: f32, y0: f32, x1: f32, y1: f32) -> Quad {
        [[x0, y0], [x1, y0], [x1, y1], [x0, y1]]
    }

    #[test]
    fn merge_scales_drops_a_blob_over_stacked_lines() {
        // first pass: one 30 px box over three 10 px lines
        let coarse = [rect(0.0, 0.0, 200.0, 30.0)];
        let fine = [
            rect(0.0, 0.0, 200.0, 10.0),
            rect(0.0, 10.0, 200.0, 20.0),
            rect(0.0, 20.0, 180.0, 30.0),
        ];
        assert_eq!(merge_scales(&coarse, &fine, 30.0), fine.to_vec());
    }

    #[test]
    fn merge_scales_keeps_a_headline_whole() {
        // the enlarged pass found the headline in two pieces
        let coarse = [rect(0.0, 0.0, 400.0, 80.0)];
        let fine = [
            rect(0.0, 5.0, 150.0, 75.0),
            rect(170.0, 5.0, 400.0, 75.0),
            rect(0.0, 100.0, 300.0, 112.0),
        ];
        assert_eq!(merge_scales(&coarse, &fine, 30.0), vec![coarse[0], fine[2]]);
        // ... or only its first word
        assert_eq!(merge_scales(&coarse, &fine[..1], 30.0), vec![coarse[0]]);
    }

    #[test]
    fn merge_scales_leaves_small_lines_to_the_fine_pass() {
        // a first-pass line under the display height, though the fine pass has it in two pieces
        let coarse = [rect(0.0, 0.0, 400.0, 20.0)];
        let fine = [rect(0.0, 2.0, 150.0, 18.0), rect(170.0, 2.0, 400.0, 18.0)];
        assert_eq!(merge_scales(&coarse, &fine, 30.0), fine.to_vec());
    }

    #[test]
    fn merge_scales_keeps_missed_lines_and_prefers_whole_fine_lines() {
        let coarse = [rect(0.0, 0.0, 400.0, 80.0), rect(0.0, 200.0, 300.0, 240.0)];
        // the fine pass has the first line whole and nothing under the second
        let fine = [rect(2.0, 3.0, 398.0, 78.0)];
        assert_eq!(merge_scales(&coarse, &fine, 30.0), vec![coarse[1], fine[0]]);
    }

    #[test]
    fn faint_token_in_a_row_is_a_candidate() {
        let (mut prob, mask, boxes, img) = page();
        // a weak "1" in the same row, well right of the line
        for y in 8..12 {
            prob[y * 60 + 40] = 0.13;
        }
        let c = faint(&prob, &mask, &boxes, &img);
        assert_eq!(c.len(), 1);
        // the row's band, with a side margin around the region
        assert_eq!((c[0][0][1], c[0][2][1]), (0.0, 20.0));
        assert!(c[0][0][0] < 40.0 && c[0][1][0] > 41.0);
    }

    #[test]
    fn faint_regions_off_row_touching_or_tall_are_not() {
        let (mut prob, mask, boxes, img) = page();
        // off the row: centred below the line's middle half
        prob[18 * 60 + 40] = 0.2;
        prob[19 * 60 + 40] = 0.2;
        // touching the detected line
        prob[10 * 60 + 20] = 0.2;
        // taller than 0.8 of the row
        for y in 0..20 {
            prob[y * 60 + 50] = 0.2;
        }
        assert!(faint(&prob, &mask, &boxes, &img).is_empty());
    }
}
