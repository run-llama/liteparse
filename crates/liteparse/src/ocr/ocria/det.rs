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
}

/// A second detection pass for small print: when the median line height at the first pass's
/// scale is under `min_h` input pixels (and the page has at least 8 lines), detect again with
/// the long side scaled to bring lines to `target_h`, at most `max_side`.
#[derive(Clone, Copy, Debug)]
pub struct Adaptive {
    pub min_h: f32,
    pub target_h: f32,
    pub max_side: usize,
}

impl Detector {
    pub fn new(pool: SessionPool, long_side: usize, thresh: f32, merge: f32) -> Self {
        Detector {
            pool,
            long_side,
            thresh,
            min_score: 0.6,
            merge,
            adaptive: None,
        }
    }

    /// Line quads (TL, TR, BR, BL along the line) in `img` pixels, in reading-ish order.
    pub fn detect(&self, img: &Img) -> Result<Vec<Quad>, OcriaError> {
        let (boxes, scale) = self.detect_at(img, self.long_side)?;
        let Some(a) = self.adaptive else {
            return Ok(boxes);
        };
        if boxes.len() < 8 {
            return Ok(boxes);
        }
        let mut hs: Vec<f32> = boxes
            .iter()
            .map(|q| norm([q[3][0] - q[0][0], q[3][1] - q[0][1]]) * scale as f32)
            .collect();
        hs.sort_by(f32::total_cmp);
        let med = hs[hs.len() / 2].max(1.0);
        if med >= a.min_h {
            return Ok(boxes);
        }
        let side = ((self.long_side as f32 * a.target_h / med) as usize).min(a.max_side) / 32 * 32;
        if side <= self.long_side {
            return Ok(boxes);
        }
        Ok(self.detect_at(img, side)?.0)
    }

    /// One detection pass with the page's long side resized to `long_side` (never enlarged);
    /// also returns the resize scale.
    fn detect_at(&self, img: &Img, long_side: usize) -> Result<(Vec<Quad>, f64), OcriaError> {
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
        // (row band of 8 px, then left edge); a stable sort, as Python's
        boxes.sort_by(|a, b| {
            let ka = ((min_y(a) as f64 / 8.0).round_ties_even(), min_x(a));
            let kb = ((min_y(b) as f64 / 8.0).round_ties_even(), min_x(b));
            ka.0.total_cmp(&kb.0).then(ka.1.total_cmp(&kb.1))
        });
        Ok((boxes, s))
    }
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
