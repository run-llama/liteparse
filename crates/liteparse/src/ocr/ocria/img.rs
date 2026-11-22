//! Image operations for the ocria-nano pipeline, written to OpenCV's definitions (pixel-centre
//! mapping, fixed-point interpolation weights, border rules) so the models see the inputs they
//! were trained and validated on. OpenCV's own results differ by platform (NEON / KleidiCV on
//! Arm, IPP on x86); these follow its portable scalar code.

/// An 8-bit image, `ch` interleaved channels per pixel (1 = grey, 3 = RGB).
#[derive(Clone, Debug)]
pub struct Img {
    pub w: usize,
    pub h: usize,
    pub ch: usize,
    pub data: Vec<u8>,
}

impl Img {
    pub fn new(w: usize, h: usize, ch: usize) -> Self {
        Img {
            w,
            h,
            ch,
            data: vec![0; w * h * ch],
        }
    }

    #[inline]
    pub fn at(&self, x: usize, y: usize, c: usize) -> u8 {
        self.data[(y * self.w + x) * self.ch + c]
    }

    /// `cv2.cvtColor(.., COLOR_RGB2GRAY)`: 14-bit fixed-point BT.601 weights.
    pub fn to_gray(&self) -> Img {
        if self.ch == 1 {
            return self.clone();
        }
        let mut out = Img::new(self.w, self.h, 1);
        for (o, p) in out.data.iter_mut().zip(self.data.chunks_exact(self.ch)) {
            let v = p[0] as u32 * 4899 + p[1] as u32 * 9617 + p[2] as u32 * 1868 + (1 << 13);
            *o = (v >> 14) as u8;
        }
        out
    }

    /// `img.mean(axis=2).astype(np.uint8)`: the channel mean, truncated.
    pub fn mean_gray(&self) -> Img {
        if self.ch == 1 {
            return self.clone();
        }
        let mut out = Img::new(self.w, self.h, 1);
        for (o, p) in out.data.iter_mut().zip(self.data.chunks_exact(self.ch)) {
            *o = (p.iter().map(|&v| v as u32).sum::<u32>() / self.ch as u32) as u8;
        }
        out
    }

    /// `np.rot90(img, k)`: k quarter turns counter-clockwise.
    pub fn rot90(&self, k: u32) -> Img {
        match k % 4 {
            0 => self.clone(),
            1 => {
                // out[i][j] = in[j][w - 1 - i]; out is h x w turned
                let mut out = Img::new(self.h, self.w, self.ch);
                for i in 0..out.h {
                    for j in 0..out.w {
                        let (sx, sy) = (self.w - 1 - i, j);
                        for c in 0..self.ch {
                            out.data[(i * out.w + j) * self.ch + c] = self.at(sx, sy, c);
                        }
                    }
                }
                out
            }
            2 => {
                let mut out = Img::new(self.w, self.h, self.ch);
                for y in 0..self.h {
                    for x in 0..self.w {
                        for c in 0..self.ch {
                            out.data[(y * self.w + x) * self.ch + c] =
                                self.at(self.w - 1 - x, self.h - 1 - y, c);
                        }
                    }
                }
                out
            }
            _ => self.rot90(1).rot90(2),
        }
    }
}

/// OpenCV's `INTER_RESIZE_COEF_BITS`.
const RESIZE_BITS: i32 = 11;
const RESIZE_ONE: i32 = 1 << RESIZE_BITS;

/// cvRound: round half to even, as `saturate_cast` does in the default FP mode.
#[inline]
fn cv_round(v: f64) -> i64 {
    v.round_ties_even() as i64
}

#[inline]
fn sat_short(v: f32) -> i32 {
    cv_round(v as f64).clamp(i16::MIN as i64, i16::MAX as i64) as i32
}

/// Two-tap source index + fixed-point weights per output index, for `cv2.resize`'s linear
/// path. `area` selects INTER_AREA's upscaling coefficients.
fn linear_taps(src: usize, dst: usize, area: bool) -> Vec<(usize, i32, i32)> {
    let inv_scale = dst as f64 / src as f64;
    let scale = 1.0 / inv_scale;
    (0..dst)
        .map(|d| {
            let (mut sx, mut f) = if area {
                let sx = (d as f64 * scale).floor() as i64;
                let f = ((d + 1) as f64 - (sx + 1) as f64 * inv_scale) as f32;
                (sx, if f <= 0.0 { 0.0 } else { f - f.floor() })
            } else {
                let fx = ((d as f64 + 0.5) * scale - 0.5) as f32;
                let sx = fx.floor();
                (sx as i64, fx - sx)
            };
            if sx < 0 {
                sx = 0;
                f = 0.0;
            }
            if sx >= src as i64 - 1 {
                sx = src as i64 - 1;
                f = 0.0;
            }
            let a0 = sat_short((1.0 - f) * RESIZE_ONE as f32);
            let a1 = sat_short(f * RESIZE_ONE as f32);
            (sx as usize, a0, a1)
        })
        .collect()
}

fn resize_linear_taps(img: &Img, w: usize, h: usize, area: bool) -> Img {
    let xt = linear_taps(img.w, w, area);
    let yt = linear_taps(img.h, h, area);
    let ch = img.ch;
    // horizontal pass, one row of i32 per source row
    let mut rows: Vec<Vec<i32>> = Vec::with_capacity(img.h);
    for y in 0..img.h {
        let mut r = vec![0i32; w * ch];
        for (dx, &(sx, a0, a1)) in xt.iter().enumerate() {
            for c in 0..ch {
                let s0 = img.at(sx, y, c) as i32;
                r[dx * ch + c] = if sx + 1 < img.w {
                    s0 * a0 + img.at(sx + 1, y, c) as i32 * a1
                } else {
                    s0 * RESIZE_ONE
                };
            }
        }
        rows.push(r);
    }
    let mut out = Img::new(w, h, ch);
    for (dy, &(sy, b0, b1)) in yt.iter().enumerate() {
        let r0 = &rows[sy];
        let r1 = &rows[(sy + 1).min(img.h - 1)];
        let o = &mut out.data[dy * w * ch..(dy + 1) * w * ch];
        for i in 0..w * ch {
            // FixedPtCast<int, uchar, 22>
            let v = (r0[i] as i64 * b0 as i64 + r1[i] as i64 * b1 as i64 + (1 << 21)) >> 22;
            o[i] = v.clamp(0, 255) as u8;
        }
    }
    out
}

/// `cv2.resize(.., interpolation=INTER_LINEAR)`.
pub fn resize_linear(img: &Img, w: usize, h: usize) -> Img {
    if w == img.w && h == img.h {
        return img.clone();
    }
    resize_linear_taps(img, w, h, false)
}

/// OpenCV `computeResizeAreaTab`: (source index, weight) runs per output index.
fn area_tab(src: usize, dst: usize, scale: f64) -> Vec<Vec<(usize, f32)>> {
    (0..dst)
        .map(|d| {
            let fsx1 = d as f64 * scale;
            let fsx2 = fsx1 + scale;
            let cell = scale.min(src as f64 - fsx1);
            let mut sx2 = fsx2.floor() as i64;
            sx2 = sx2.min(src as i64 - 1);
            let sx1 = (fsx1.ceil() as i64).min(sx2);
            let mut t = Vec::new();
            if sx1 as f64 - fsx1 > 1e-3 {
                t.push(((sx1 - 1) as usize, ((sx1 as f64 - fsx1) / cell) as f32));
            }
            for sx in sx1..sx2 {
                t.push((sx as usize, (1.0 / cell) as f32));
            }
            if fsx2 - sx2 as f64 > 1e-3 {
                t.push((
                    sx2 as usize,
                    ((fsx2 - sx2 as f64).min(1.0).min(cell) / cell) as f32,
                ));
            }
            t
        })
        .collect()
}

/// `cv2.resize(.., interpolation=INTER_AREA)`: box averaging when shrinking both axes,
/// OpenCV's area-weighted linear taps otherwise.
pub fn resize_area(img: &Img, w: usize, h: usize) -> Img {
    if w == img.w && h == img.h {
        return img.clone();
    }
    let (scale_x, scale_y) = (img.w as f64 / w as f64, img.h as f64 / h as f64);
    if scale_x < 1.0 || scale_y < 1.0 {
        return resize_linear_taps(img, w, h, true);
    }
    let ch = img.ch;
    let (isx, isy) = (scale_x.round(), scale_y.round());
    if (scale_x - isx).abs() < f64::EPSILON && (scale_y - isy).abs() < f64::EPSILON {
        // integer factors: plain block mean (resizeAreaFast)
        let (kx, ky) = (isx as usize, isy as usize);
        let area = (kx * ky) as u32;
        let mut out = Img::new(w, h, ch);
        for dy in 0..h {
            for dx in 0..w {
                for c in 0..ch {
                    let mut s = 0u32;
                    for yy in 0..ky {
                        for xx in 0..kx {
                            s += img.at(dx * kx + xx, dy * ky + yy, c) as u32;
                        }
                    }
                    let v = if area == 4 {
                        (s + 2) >> 2
                    } else {
                        cv_round(s as f64 * (1.0f32 / area as f32) as f64) as u32
                    };
                    out.data[(dy * w + dx) * ch + c] = v.min(255) as u8;
                }
            }
        }
        return out;
    }
    // flat horizontal taps: (output offset, source offset, weight), in OpenCV's order
    let xt: Vec<(usize, usize, f32)> = area_tab(img.w, w, scale_x)
        .into_iter()
        .enumerate()
        .flat_map(|(dx, taps)| taps.into_iter().map(move |(sx, a)| (dx * ch, sx * ch, a)))
        .collect();
    let yt = area_tab(img.h, h, scale_y);
    let mut out = Img::new(w, h, ch);
    let row_len = img.w * ch;
    // horizontal sums of one source row; adjacent output rows share boundary rows, so the
    // last row's sums are kept (the same values OpenCV computes, in the same order)
    let hsum = |sy: usize, buf: &mut [f32]| {
        buf.fill(0.0);
        let row = &img.data[sy * row_len..(sy + 1) * row_len];
        for &(d, s, alpha) in &xt {
            for c in 0..ch {
                buf[d + c] += row[s + c] as f32 * alpha;
            }
        }
    };
    let (mut buf, mut buf_row) = (vec![0f32; w * ch], usize::MAX);
    let mut sum = vec![0f32; w * ch];
    for (dy, ytaps) in yt.iter().enumerate() {
        sum.fill(0.0);
        for &(sy, beta) in ytaps {
            if buf_row != sy {
                hsum(sy, &mut buf);
                buf_row = sy;
            }
            for (s, b) in sum.iter_mut().zip(&buf) {
                *s += b * beta;
            }
        }
        for (o, s) in out.data[dy * w * ch..(dy + 1) * w * ch]
            .iter_mut()
            .zip(&sum)
        {
            *o = cv_round(*s as f64).clamp(0, 255) as u8;
        }
    }
    out
}

/// `cv2.getPerspectiveTransform(src, dst)`: the 3x3 homography taking `src` corners to `dst`.
pub fn perspective_transform(src: &[[f32; 2]; 4], dst: &[[f32; 2]; 4]) -> [f64; 9] {
    let mut a = [[0f64; 9]; 8];
    for i in 0..4 {
        let (x, y) = (src[i][0] as f64, src[i][1] as f64);
        let (u, v) = (dst[i][0] as f64, dst[i][1] as f64);
        a[i] = [x, y, 1.0, 0.0, 0.0, 0.0, -x * u, -y * u, u];
        a[i + 4] = [0.0, 0.0, 0.0, x, y, 1.0, -x * v, -y * v, v];
    }
    // Gaussian elimination with partial pivoting (OpenCV solves with LU)
    for col in 0..8 {
        let piv = (col..8)
            .max_by(|&p, &q| a[p][col].abs().total_cmp(&a[q][col].abs()))
            .unwrap();
        a.swap(col, piv);
        let d = a[col][col];
        if d.abs() < 1e-300 {
            continue;
        }
        for r in 0..8 {
            if r != col {
                let f = a[r][col] / d;
                if f != 0.0 {
                    for k in col..9 {
                        a[r][k] -= f * a[col][k];
                    }
                }
            }
        }
    }
    let mut m = [0f64; 9];
    for i in 0..8 {
        m[i] = if a[i][i] != 0.0 {
            a[i][8] / a[i][i]
        } else {
            0.0
        };
    }
    m[8] = 1.0;
    m
}

fn invert3(m: &[f64; 9]) -> [f64; 9] {
    let det = m[0] * (m[4] * m[8] - m[5] * m[7]) - m[1] * (m[3] * m[8] - m[5] * m[6])
        + m[2] * (m[3] * m[7] - m[4] * m[6]);
    if det == 0.0 {
        return [0.0; 9];
    }
    let d = 1.0 / det;
    [
        (m[4] * m[8] - m[5] * m[7]) * d,
        (m[2] * m[7] - m[1] * m[8]) * d,
        (m[1] * m[5] - m[2] * m[4]) * d,
        (m[5] * m[6] - m[3] * m[8]) * d,
        (m[0] * m[8] - m[2] * m[6]) * d,
        (m[2] * m[3] - m[0] * m[5]) * d,
        (m[3] * m[7] - m[4] * m[6]) * d,
        (m[1] * m[6] - m[0] * m[7]) * d,
        (m[0] * m[4] - m[1] * m[3]) * d,
    ]
}

const INTER_BITS: i64 = 5;
const INTER_TAB: i64 = 1 << INTER_BITS;
const REMAP_SCALE: i32 = 1 << 15;

/// OpenCV `interpolateCubic` (A = -0.75).
fn cubic_coeffs(x: f32) -> [f32; 4] {
    const A: f32 = -0.75;
    let c0 = ((A * (x + 1.0) - 5.0 * A) * (x + 1.0) + 8.0 * A) * (x + 1.0) - 4.0 * A;
    let c1 = ((A + 2.0) * x - (A + 3.0)) * x * x + 1.0;
    let c2 = ((A + 2.0) * (1.0 - x) - (A + 3.0)) * (1.0 - x) * (1.0 - x) + 1.0;
    [c0, c1, c2, 1.0 - c0 - c1 - c2]
}

/// OpenCV `initInterTab2D(INTER_CUBIC)` fixed-point table: 32 x 32 sub-pixel positions, 16
/// weights each, corrected to sum to exactly 2^15.
fn bicubic_tab() -> &'static [[i32; 16]] {
    static TAB: std::sync::OnceLock<Vec<[i32; 16]>> = std::sync::OnceLock::new();
    TAB.get_or_init(|| {
        let one_d: Vec<[f32; 4]> = (0..INTER_TAB)
            .map(|i| cubic_coeffs(i as f32 / INTER_TAB as f32))
            .collect();
        let mut tab = Vec::with_capacity((INTER_TAB * INTER_TAB) as usize);
        for iy in 0..INTER_TAB as usize {
            for ix in 0..INTER_TAB as usize {
                let (vy, vx) = (one_d[iy], one_d[ix]);
                let mut t = [0i32; 16];
                let mut isum = 0;
                for i in 0..4 {
                    for j in 0..4 {
                        let v = vy[i] * vx[j];
                        t[i * 4 + j] = sat_short(v * REMAP_SCALE as f32);
                        isum += t[i * 4 + j];
                    }
                }
                if isum != REMAP_SCALE {
                    let diff = isum - REMAP_SCALE;
                    let (mut mk, mut big) = ((2, 2), (2, 2));
                    for k1 in 2..4 {
                        for k2 in 2..4 {
                            if t[k1 * 4 + k2] < t[mk.0 * 4 + mk.1] {
                                mk = (k1, k2);
                            } else if t[k1 * 4 + k2] > t[big.0 * 4 + big.1] {
                                big = (k1, k2);
                            }
                        }
                    }
                    if diff < 0 {
                        t[big.0 * 4 + big.1] -= diff;
                    } else {
                        t[mk.0 * 4 + mk.1] -= diff;
                    }
                }
                tab.push(t);
            }
        }
        tab
    })
}

/// `cv2.warpPerspective(img, m, (w, h), flags=INTER_CUBIC, borderMode=BORDER_REPLICATE)`.
pub fn warp_perspective_cubic(img: &Img, m: &[f64; 9], w: usize, h: usize) -> Img {
    let inv = invert3(m);
    let tab = bicubic_tab();
    let ch = img.ch;
    let mut out = Img::new(w, h, ch);
    let (sw, sh) = (img.w as i64, img.h as i64);
    for y in 0..h {
        for x in 0..w {
            let (xf, yf) = (x as f64, y as f64);
            let x0 = inv[0] * xf + inv[1] * yf + inv[2];
            let y0 = inv[3] * xf + inv[4] * yf + inv[5];
            let mut ww = inv[6] * xf + inv[7] * yf + inv[8];
            ww = if ww != 0.0 {
                INTER_TAB as f64 / ww
            } else {
                0.0
            };
            let fx = (x0 * ww).clamp(i32::MIN as f64, i32::MAX as f64);
            let fy = (y0 * ww).clamp(i32::MIN as f64, i32::MAX as f64);
            let (xi, yi) = (cv_round(fx), cv_round(fy));
            let (sx, sy) = (xi >> INTER_BITS, yi >> INTER_BITS);
            let a = ((yi & (INTER_TAB - 1)) * INTER_TAB + (xi & (INTER_TAB - 1))) as usize;
            let wt = &tab[a];
            // the 4 x 4 source window, clamped to the image (BORDER_REPLICATE)
            let inside = sx >= 1 && sy >= 1 && sx + 2 < sw && sy + 2 < sh;
            let col = |j: usize| -> usize {
                if inside {
                    (sx - 1) as usize + j
                } else {
                    (sx - 1 + j as i64).clamp(0, sw - 1) as usize
                }
            };
            let cols = [col(0), col(1), col(2), col(3)];
            let mut sum = [0i64; 4];
            for i in 0..4 {
                let yy = if inside {
                    (sy - 1) as usize + i
                } else {
                    (sy - 1 + i as i64).clamp(0, sh - 1) as usize
                };
                let row = &img.data[yy * img.w * ch..(yy + 1) * img.w * ch];
                for (j, &xx) in cols.iter().enumerate() {
                    let wv = wt[i * 4 + j] as i64;
                    let px = &row[xx * ch..xx * ch + ch];
                    for c in 0..ch {
                        sum[c] += px[c] as i64 * wv;
                    }
                }
            }
            let o = &mut out.data[(y * w + x) * ch..(y * w + x) * ch + ch];
            for c in 0..ch {
                o[c] = ((sum[c] + (1 << 14)) >> 15).clamp(0, 255) as u8;
            }
        }
    }
    out
}

/// One 4-connected component of a binary mask: its bounding box and pixels.
pub struct Component {
    pub x0: usize,
    pub y0: usize,
    pub x1: usize,
    pub y1: usize,
    pub pixels: Vec<(usize, usize)>,
}

/// 4-connected components of `mask` (row-major, `w` x `h`), labelled in raster order of each
/// component's first pixel, as `cv2.connectedComponentsWithStats` numbers them.
pub fn components4(mask: &[bool], w: usize, h: usize) -> Vec<Component> {
    let mut seen = vec![false; w * h];
    let mut out = Vec::new();
    let mut stack = Vec::new();
    for start in 0..w * h {
        if !mask[start] || seen[start] {
            continue;
        }
        seen[start] = true;
        stack.push(start);
        let mut comp = Component {
            x0: usize::MAX,
            y0: usize::MAX,
            x1: 0,
            y1: 0,
            pixels: Vec::new(),
        };
        while let Some(p) = stack.pop() {
            let (x, y) = (p % w, p / w);
            comp.x0 = comp.x0.min(x);
            comp.y0 = comp.y0.min(y);
            comp.x1 = comp.x1.max(x);
            comp.y1 = comp.y1.max(y);
            comp.pixels.push((x, y));
            let mut visit = |q: usize| {
                if mask[q] && !seen[q] {
                    seen[q] = true;
                    stack.push(q);
                }
            };
            if x > 0 {
                visit(p - 1);
            }
            if x + 1 < w {
                visit(p + 1);
            }
            if y > 0 {
                visit(p - w);
            }
            if y + 1 < h {
                visit(p + w);
            }
        }
        out.push(comp);
    }
    out
}

/// A rotated rectangle: centre, side lengths along `u` (w) and its normal (h), and `u`'s
/// angle in degrees.
#[derive(Clone, Copy, Debug)]
pub struct RotRect {
    pub cx: f32,
    pub cy: f32,
    pub w: f32,
    pub h: f32,
    pub angle: f32,
}

fn cross(o: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0)
}

/// `cv2.minAreaRect`: the smallest-area rectangle around `pts` (convex hull + one candidate
/// per hull edge).
pub fn min_area_rect(pts: &[(f32, f32)]) -> RotRect {
    let mut p: Vec<(f64, f64)> = pts.iter().map(|&(x, y)| (x as f64, y as f64)).collect();
    p.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    p.dedup();
    if p.len() == 1 {
        return RotRect {
            cx: p[0].0 as f32,
            cy: p[0].1 as f32,
            w: 0.0,
            h: 0.0,
            angle: 0.0,
        };
    }
    // monotone chain
    let mut hull: Vec<(f64, f64)> = Vec::with_capacity(p.len() * 2);
    for &q in &p {
        while hull.len() >= 2 && cross(hull[hull.len() - 2], hull[hull.len() - 1], q) <= 0.0 {
            hull.pop();
        }
        hull.push(q);
    }
    let lower = hull.len() + 1;
    for &q in p.iter().rev().skip(1) {
        while hull.len() >= lower && cross(hull[hull.len() - 2], hull[hull.len() - 1], q) <= 0.0 {
            hull.pop();
        }
        hull.push(q);
    }
    hull.pop();
    let mut best: Option<(f64, RotRect)> = None;
    for i in 0..hull.len() {
        let a = hull[i];
        let b = hull[(i + 1) % hull.len()];
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let len = dx.hypot(dy);
        if len == 0.0 {
            continue;
        }
        let (ux, uy) = (dx / len, dy / len);
        let (vx, vy) = (-uy, ux);
        let (mut u0, mut u1, mut v0, mut v1) = (
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
        );
        for &q in &hull {
            let pu = q.0 * ux + q.1 * uy;
            let pv = q.0 * vx + q.1 * vy;
            u0 = u0.min(pu);
            u1 = u1.max(pu);
            v0 = v0.min(pv);
            v1 = v1.max(pv);
        }
        let area = (u1 - u0) * (v1 - v0);
        if best.as_ref().is_none_or(|(ba, _)| area < *ba) {
            let (cu, cv) = ((u0 + u1) / 2.0, (v0 + v1) / 2.0);
            best = Some((
                area,
                RotRect {
                    cx: (cu * ux + cv * vx) as f32,
                    cy: (cu * uy + cv * vy) as f32,
                    w: (u1 - u0) as f32,
                    h: (v1 - v0) as f32,
                    angle: uy.atan2(ux).to_degrees() as f32,
                },
            ));
        }
    }
    best.map(|b| b.1).unwrap_or(RotRect {
        cx: p[0].0 as f32,
        cy: p[0].1 as f32,
        w: 0.0,
        h: 0.0,
        angle: 0.0,
    })
}

/// `cv2.boxPoints`: the rectangle's four corners.
pub fn box_points(r: &RotRect) -> [[f32; 2]; 4] {
    let a = (r.angle as f64).to_radians();
    let (b, c) = ((a.cos() * 0.5) as f32, (a.sin() * 0.5) as f32);
    let p0 = [r.cx - c * r.h - b * r.w, r.cy + b * r.h - c * r.w];
    let p1 = [r.cx + c * r.h - b * r.w, r.cy - b * r.h - c * r.w];
    let p2 = [2.0 * r.cx - p0[0], 2.0 * r.cy - p0[1]];
    let p3 = [2.0 * r.cx - p1[0], 2.0 * r.cy - p1[1]];
    [p0, p1, p2, p3]
}

/// Otsu's threshold over 8-bit values (`server._otsu`): the first level maximising the
/// between-class variance.
pub fn otsu(values: impl Iterator<Item = u8>) -> u8 {
    let mut hist = [0f64; 256];
    let mut n = 0f64;
    for v in values {
        hist[v as usize] += 1.0;
        n += 1.0;
    }
    let n = n.max(1.0);
    let (mut w, mut mu) = (0f64, 0f64);
    let mut cum = [(0f64, 0f64); 256];
    for i in 0..256 {
        w += hist[i] / n;
        mu += hist[i] / n * i as f64;
        cum[i] = (w, mu);
    }
    let mu_t = cum[255].1;
    let mut best = (f64::NEG_INFINITY, 0u8);
    for (i, &(w, mu)) in cum.iter().enumerate() {
        let s = (mu_t * w - mu).powi(2) / (w * (1.0 - w));
        if s.is_finite() && s > best.0 {
            best = (s, i as u8);
        }
    }
    best.1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rot90_matches_numpy() {
        // [[1,2,3],[4,5,6]] -> np.rot90 -> [[3,6],[2,5],[1,4]]
        let img = Img {
            w: 3,
            h: 2,
            ch: 1,
            data: vec![1, 2, 3, 4, 5, 6],
        };
        assert_eq!(img.rot90(1).data, vec![3, 6, 2, 5, 1, 4]);
        assert_eq!(img.rot90(2).data, vec![6, 5, 4, 3, 2, 1]);
    }

    #[test]
    fn bicubic_weights_sum_to_one() {
        for t in bicubic_tab() {
            assert_eq!(t.iter().sum::<i32>(), REMAP_SCALE);
        }
    }

    #[test]
    fn min_area_rect_of_axis_box() {
        let r = min_area_rect(&[(0.0, 0.0), (10.0, 0.0), (10.0, 4.0), (0.0, 4.0)]);
        assert!((r.cx - 5.0).abs() < 1e-4 && (r.cy - 2.0).abs() < 1e-4);
        let (a, b) = (r.w.max(r.h), r.w.min(r.h));
        assert!((a - 10.0).abs() < 1e-4 && (b - 4.0).abs() < 1e-4);
    }
}
