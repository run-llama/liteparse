//! Feathered silhouette masks for `a:softEdge` and `a:glow`.
//!
//! Both need each pixel's distance to the silhouette's outline. A box filter of
//! radius r over the silhouette gives it near the outline: on a straight edge the
//! mean coverage is 1/2 on the outline and moves linearly, by 1/(2r+1) per
//! pixel, to 0 or 1 at r pixels out or in (corners come out rounded). A soft edge
//! eases from transparent to opaque over a band of depths inside the outline; a
//! glow eases from full color on the outline to nothing outside it.
//!
//! A soft-edge group is drawn in place: `begin` snapshots the page pixels under
//! the silhouette, the group's items draw normally, and `end` blends the result
//! back toward the snapshot by the feathered mask. For source-over drawing that
//! equals compositing the group as one layer whose alpha is scaled by the mask.

use tiny_skia::{FillRule, Mask, MaskType, Path, PathBuilder, Pixmap, Transform};

use super::blur::{self, Bounds};
use super::{ImageCache, RenderAssets};
use crate::office::pptx::scene::{ImageItem, SoftEdge};

/// The alpha of a picture drawn alone, as a mask the size of `pm` (with
/// `canvas` placing it), and the unit square its transform maps onto the page.
/// Only the picture's device bounds are drawn and copied.
pub(super) fn image_alpha(
    pm: &Pixmap,
    img: &ImageItem,
    canvas: Transform,
    doc: &dyn RenderAssets,
    images: &ImageCache,
) -> Option<(Mask, Path)> {
    let (w, h) = (pm.width() as usize, pm.height() as usize);
    let unit = PathBuilder::from_rect(tiny_skia::Rect::from_ltrb(0.0, 0.0, 1.0, 1.0)?);
    let b = unit.bounds().transform(img.transform.post_concat(canvas))?;
    let (x0, y0, x1, y1) = blur::reach((b.left(), b.top(), b.right(), b.bottom()), w, h);
    let mut mask = Mask::new(w as u32, h as u32)?;
    if x0 >= x1 || y0 >= y1 {
        return Some((mask, unit));
    }
    let mut layer = Pixmap::new((x1 - x0) as u32, (y1 - y0) as u32)?;
    let shift = Transform::from_translate(-(x0 as f32), -(y0 as f32));
    super::draw_image(
        &mut layer,
        img,
        canvas.post_concat(shift),
        None,
        doc,
        images,
    );
    let alpha = Mask::from_pixmap(layer.as_ref(), MaskType::Alpha);
    let data = mask.data_mut();
    for (y, row) in (y0..y1).zip(alpha.data().chunks_exact(x1 - x0)) {
        data[y * w + x0..y * w + x1].copy_from_slice(row);
    }
    Some((mask, unit))
}

/// Where the soft-edge fade starts (transparent) and ends (opaque), as depths
/// inside the outline in multiples of `softEdge rad`; measured against PowerPoint.
const FADE_START: f32 = 0.35;
const FADE_END: f32 = 1.35;
/// How far a glow reaches outside the outline, in multiples of `glow rad`.
const GLOW_REACH: f32 = 0.9;

/// `t` in 0..1 eased at both ends.
fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Turns a shadow mask holding a silhouette (ink inside `within`) into a glow
/// for `glow rad` = `rad_px`: full inside and on the outline, easing to nothing
/// `GLOW_REACH × rad` outside. Returns the region that holds the glow.
pub(super) fn glow_mask(mask: &mut Mask, rad_px: f32, within: Bounds) -> Option<Bounds> {
    let (w, h) = (mask.width() as usize, mask.height() as usize);
    let ink = blur::ink_bounds(mask.data(), w, within)?;
    let reach = rad_px * GLOW_REACH;
    let r = reach.ceil() as usize;
    if r == 0 {
        return Some(ink);
    }
    let (x0, y0) = (ink.0.saturating_sub(r), ink.1.saturating_sub(r));
    let (x1, y1) = ((ink.2 + r).min(w), (ink.3 + r).min(h));
    let bw = x1 - x0;
    let mut region = Vec::with_capacity(bw * (y1 - y0));
    for y in y0..y1 {
        region.extend_from_slice(&mask.data()[y * w + x0..y * w + x1]);
    }
    // Distance outside the outline, from the same linear box profile.
    let win = (2 * r + 1) as f32;
    let lut = table(|c| 1.0 - smoothstep((0.5 - c) * win / reach));
    let glow = box_alpha(&region, bw, y1 - y0, r, (0, 0, bw, y1 - y0), &lut);
    let data = mask.data_mut();
    for (y, row) in (y0..y1).zip(glow.chunks_exact(bw)) {
        data[y * w + x0..y * w + x1].copy_from_slice(row);
    }
    Some((x0, y0, x1, y1))
}

/// The soft-edge fade for `softEdge rad` = `rad_px`, as a coverage table for
/// a box of radius `rb` (`None` when the fade is under half a pixel).
fn fade_table(rad_px: f32) -> Option<(usize, Vec<u8>)> {
    let (d0, d1) = (rad_px * FADE_START, rad_px * FADE_END);
    if d1 < 0.5 {
        return None;
    }
    // One box pass is linear across a straight edge: coverage c lies 2r+1 px of
    // depth per unit from 1/2, so `d` is the depth inside the outline in pixels.
    let rb = d1.ceil() as usize;
    let win = (2 * rb + 1) as f32;
    let lut = table(|c| smoothstep(((c - 0.5) * win - d0) / (d1 - d0).max(1e-3)));
    Some((rb, lut))
}

/// Fades a silhouette mask (ink inside `within`) in place as a soft edge of
/// `rad_px` would fade the shape, so its shadow or glow follows the faded shape.
pub(super) fn feather_mask(mask: &mut Mask, rad_px: f32, within: Bounds) {
    let Some((rb, lut)) = fade_table(rad_px) else {
        return;
    };
    let (w, h) = (mask.width() as usize, mask.height() as usize);
    let (x0, y0, x1, y1) = within;
    let (gx0, gy0) = (x0.saturating_sub(rb), y0.saturating_sub(rb));
    let (gx1, gy1) = ((x1 + rb).min(w), (y1 + rb).min(h));
    if gx0 >= gx1 || gy0 >= gy1 {
        return;
    }
    let gw = gx1 - gx0;
    let mut region = Vec::with_capacity(gw * (gy1 - gy0));
    for y in gy0..gy1 {
        region.extend_from_slice(&mask.data()[y * w + gx0..y * w + gx1]);
    }
    let out = (x0 - gx0, y0 - gy0, x1 - gx0, y1 - gy0);
    let faded = box_alpha(&region, gw, gy1 - gy0, rb, out, &lut);
    let data = mask.data_mut();
    for (y, row) in (y0..y1).zip(faded.chunks_exact(x1 - x0)) {
        data[y * w + x0..y * w + x1].copy_from_slice(row);
    }
}

/// A soft-edge group being drawn: the page pixels under it before its items
/// and the fade mask, both over `bounds`.
pub(super) struct Group {
    bounds: Bounds,
    before: Vec<u8>,
    alpha: Vec<u8>,
}

/// Starts a soft-edge group; `None` when the silhouette is off the page.
pub(super) fn begin(pm: &Pixmap, s: &SoftEdge, canvas: Transform, scale: f32) -> Option<Group> {
    let (w, h) = (pm.width() as usize, pm.height() as usize);
    let fade = fade_table(s.radius as f32 * scale);
    let rb = fade.as_ref().map_or(0, |f| f.0);
    let m = rb + 1;
    // The blended region covers outlines drawn just outside the silhouette (they
    // fade out with it); the mask extends past it, and past the page, by the
    // box's reach, so a shape cut by the page edge does not fade there.
    let b = s.path.bounds().transform(canvas)?;
    let mf = m as f32;
    let bounds = blur::reach(
        (b.left() - mf, b.top() - mf, b.right() + mf, b.bottom() + mf),
        w,
        h,
    );
    let (x0, y0, x1, y1) = bounds;
    if x0 >= x1 || y0 >= y1 {
        return None;
    }
    let (mw, mh) = (x1 - x0 + 2 * m, y1 - y0 + 2 * m);
    let mut mask = Mask::new(mw as u32, mh as u32)?;
    let shift = Transform::from_translate(m as f32 - x0 as f32, m as f32 - y0 as f32);
    let t = canvas.post_concat(shift);
    let aa = super::anti_alias_fits(&s.path, t, 0.0);
    mask.fill_path(&s.path, FillRule::Winding, aa, t);
    let (gw, gh) = (x1 - x0, y1 - y0);
    let alpha = match &fade {
        Some((_, lut)) => box_alpha(mask.data(), mw, mh, rb, (m, m, m + gw, m + gh), lut),
        None => (0..gh)
            .flat_map(|y| &mask.data()[(y + m) * mw + m..(y + m) * mw + m + gw])
            .copied()
            .collect(),
    };
    let mut before = Vec::with_capacity(gw * gh * 4);
    for y in y0..y1 {
        before.extend_from_slice(&pm.data()[(y * w + x0) * 4..(y * w + x1) * 4]);
    }
    Some(Group {
        bounds,
        before,
        alpha,
    })
}

/// Steps of the coverage → alpha table.
const LUT_STEPS: usize = 2048;

/// `f` (coverage 0..1 → alpha 0..1) tabulated at `LUT_STEPS + 1` points.
fn table(f: impl Fn(f32) -> f32) -> Vec<u8> {
    (0..=LUT_STEPS)
        .map(|i| (f(i as f32 / LUT_STEPS as f32).clamp(0.0, 1.0) * 255.0).round() as u8)
        .collect()
}

/// For each pixel of `out` (a window of the `w` x `h` mask `data`), the mean
/// coverage of the `(2r+1)²` box around it (outside counts as empty), mapped
/// through `lut` (from `table`). Two running-sum passes, so any `r` costs the same.
fn box_alpha(data: &[u8], w: usize, h: usize, r: usize, out: Bounds, lut: &[u8]) -> Vec<u8> {
    let (ox0, oy0, ox1, oy1) = out;
    let mut rows = vec![0u32; w * h];
    for y in oy0.saturating_sub(r)..(oy1 + r).min(h) {
        let (src, dst) = (&data[y * w..(y + 1) * w], &mut rows[y * w..(y + 1) * w]);
        let mut sum: u32 = src[..=r.min(w - 1)].iter().map(|v| *v as u32).sum();
        for x in 0..w {
            dst[x] = sum;
            if x + r + 1 < w {
                sum += src[x + r + 1] as u32;
            }
            if x >= r {
                sum -= src[x - r] as u32;
            }
        }
    }
    let full = 255 * ((2 * r + 1) as u64).pow(2);
    let mut res = Vec::with_capacity((ox1 - ox0) * (oy1 - oy0));
    let mut sums = vec![0u64; ox1 - ox0];
    let cols = |y: usize| &rows[y * w + ox0..y * w + ox1];
    for y in oy0.saturating_sub(r)..=(oy0 + r).min(h - 1) {
        for (acc, v) in sums.iter_mut().zip(cols(y)) {
            *acc += *v as u64;
        }
    }
    for y in oy0..oy1 {
        res.extend(
            sums.iter()
                .map(|acc| lut[((acc * LUT_STEPS as u64 + full / 2) / full) as usize]),
        );
        if y + r + 1 < h {
            for (acc, v) in sums.iter_mut().zip(cols(y + r + 1)) {
                *acc += *v as u64;
            }
        }
        if y >= r {
            for (acc, v) in sums.iter_mut().zip(cols(y - r)) {
                *acc -= *v as u64;
            }
        }
    }
    res
}

/// Ends a group: each pixel moves from its pre-group value toward what the
/// group drew by the mask (premultiplied channels, so the lerp stays valid).
pub(super) fn end(pm: &mut Pixmap, g: Group) {
    let w = pm.width() as usize;
    let (x0, y0, x1, y1) = g.bounds;
    let gw = x1 - x0;
    let data = pm.data_mut();
    for y in y0..y1 {
        let px = &mut data[(y * w + x0) * 4..(y * w + x1) * 4];
        let old = &g.before[(y - y0) * gw * 4..(y - y0 + 1) * gw * 4];
        let alpha = &g.alpha[(y - y0) * gw..(y - y0 + 1) * gw];
        for ((p, o), a) in px
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip(old.as_chunks::<4>().0)
            .zip(alpha)
        {
            if *a == 255 {
                continue;
            }
            let a = *a as i32;
            for (c, oc) in p.iter_mut().zip(o) {
                let (n, oc) = (*c as i32, *oc as i32);
                *c = (oc + ((n - oc) * a + 127) / 255) as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests;
