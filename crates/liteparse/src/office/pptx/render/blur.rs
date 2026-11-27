//! Shadow blur restricted to the part of the mask a shadow can reach.
//!
//! A three-pass box blur of radius `r` spreads ink at most `3r` pixels along
//! each axis, and pixels beyond the image edge count as zero. Blurring only the
//! inked bounding box grown by `3r` therefore gives the same bytes as blurring
//! the whole page, at a fraction of the cost for small shadows.

use tiny_skia::Mask;

/// Inclusive-exclusive pixel bounds `(x0, y0, x1, y1)` within a mask.
pub(super) type Bounds = (usize, usize, usize, usize);

/// Pixels a fill can touch: `(l, t, r, b)` device bounds of its outline grown by
/// 2 px for anti-aliasing, clamped to a `w` x `h` mask.
pub(super) fn reach(ltrb: (f32, f32, f32, f32), w: usize, h: usize) -> Bounds {
    let clamp = |v: f32, max: usize| (v.max(0.0) as usize).min(max);
    let (l, t, r, b) = ltrb;
    let x0 = clamp((l - 2.0).floor(), w);
    let y0 = clamp((t - 2.0).floor(), h);
    let x1 = clamp((r + 2.0).ceil(), w).max(x0);
    let y1 = clamp((b + 2.0).ceil(), h).max(y0);
    (x0, y0, x1, y1)
}

/// Bounds of the non-zero pixels inside `within`, or `None` if there are none.
pub(super) fn ink_bounds(data: &[u8], w: usize, within: Bounds) -> Option<Bounds> {
    let (wx0, wy0, wx1, wy1) = within;
    let row = |y: usize| &data[y * w + wx0..y * w + wx1];
    let y0 = (wy0..wy1).find(|y| row(*y).iter().any(|v| *v != 0))?;
    let y1 = (y0..wy1).rev().find(|y| row(*y).iter().any(|v| *v != 0))? + 1;
    let (mut x0, mut x1) = (wx1, wx0);
    for y in y0..y1 {
        let r = row(y);
        if let Some(first) = r.iter().position(|v| *v != 0) {
            x0 = x0.min(wx0 + first);
            x1 = x1.max(wx0 + r.iter().rposition(|v| *v != 0).unwrap_or(first) + 1);
        }
    }
    Some((x0, y0, x1, y1))
}

/// Blurs `mask` with three box passes of radius `r` per axis, given that all
/// ink lies inside `within`. Returns the region that can hold non-zero
/// coverage afterwards (`None` when there is no ink).
pub(super) fn box_blur(mask: &mut Mask, r: usize, within: Bounds) -> Option<Bounds> {
    let (w, h) = (mask.width() as usize, mask.height() as usize);
    let (x0, y0, x1, y1) = ink_bounds(mask.data(), w, within)?;
    let grow = 3 * r;
    let (bx0, by0) = (x0.saturating_sub(grow), y0.saturating_sub(grow));
    let (bx1, by1) = ((x1 + grow).min(w), (y1 + grow).min(h));
    let (bw, bh) = (bx1 - bx0, by1 - by0);
    let mut region = vec![0u8; bw * bh];
    for y in 0..bh {
        let src = (by0 + y) * w + bx0;
        region[y * bw..(y + 1) * bw].copy_from_slice(&mask.data()[src..src + bw]);
    }
    let mut tmp = vec![0u8; bw * bh];
    for _ in 0..3 {
        blur_rows(&region, &mut tmp, bw, bh, r);
        blur_cols(&tmp, &mut region, bw, bh, r);
    }
    let data = mask.data_mut();
    for y in 0..bh {
        let dst = (by0 + y) * w + bx0;
        data[dst..dst + bw].copy_from_slice(&region[y * bw..(y + 1) * bw]);
    }
    Some((bx0, by0, bx1, by1))
}

/// `sum / win` as a multiply and shift: `div(sum) == sum / win` for every
/// `sum <= 255 * win` and `win <= 129` (radius 64), checked in the tests.
#[derive(Clone, Copy)]
struct Divide {
    mul: u64,
}

impl Divide {
    const SHIFT: u32 = 32;

    fn new(win: u32) -> Divide {
        Divide {
            mul: (1u64 << Self::SHIFT).div_ceil(win as u64),
        }
    }

    #[inline]
    fn div(self, sum: u32) -> u8 {
        ((sum as u64 * self.mul) >> Self::SHIFT) as u8
    }
}

/// One running-sum box pass along each row; outside pixels count as zero.
fn blur_rows(src: &[u8], dst: &mut [u8], w: usize, h: usize, r: usize) {
    let win = Divide::new((2 * r + 1) as u32);
    for y in 0..h {
        let s = &src[y * w..(y + 1) * w];
        let d = &mut dst[y * w..(y + 1) * w];
        let mut sum: u32 = s[..=r.min(w - 1)].iter().map(|v| *v as u32).sum();
        for i in 0..w {
            d[i] = win.div(sum);
            if i + r + 1 < w {
                sum += s[i + r + 1] as u32;
            }
            if i >= r {
                sum -= s[i - r] as u32;
            }
        }
    }
}

/// The same pass down each column, swept row by row so memory is read in order.
fn blur_cols(src: &[u8], dst: &mut [u8], w: usize, h: usize, r: usize) {
    let win = Divide::new((2 * r + 1) as u32);
    let mut sums = vec![0u32; w];
    for y in 0..=r.min(h - 1) {
        for (acc, v) in sums.iter_mut().zip(&src[y * w..(y + 1) * w]) {
            *acc += *v as u32;
        }
    }
    for i in 0..h {
        for (d, acc) in dst[i * w..(i + 1) * w].iter_mut().zip(&sums) {
            *d = win.div(*acc);
        }
        if i + r + 1 < h {
            let row = &src[(i + r + 1) * w..(i + r + 2) * w];
            for (acc, v) in sums.iter_mut().zip(row) {
                *acc += *v as u32;
            }
        }
        if i >= r {
            let row = &src[(i - r) * w..(i - r + 1) * w];
            for (acc, v) in sums.iter_mut().zip(row) {
                *acc -= *v as u32;
            }
        }
    }
}

#[cfg(test)]
mod tests;
