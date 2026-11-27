//! Picture recolor effects (`a:duotone`, `a:grayscl`, `a:lum`, `a:clrChange`,
//! `a:biLevel`) applied to decoded, premultiplied pixmaps.

use tiny_skia::Pixmap;

use crate::office::pptx::color::Rgba;
use crate::office::pptx::scene::BlipEffect;

/// Rec. 601 luma of straight (unpremultiplied) RGB in 0..1.
fn luma(r: f32, g: f32, b: f32) -> f32 {
    0.299 * r + 0.587 * g + 0.114 * b
}

pub fn apply(pm: &mut Pixmap, effects: &[BlipEffect]) {
    for px in pm.data_mut().as_chunks_mut::<4>().0 {
        let a = px[3] as f32 / 255.0;
        if a <= 0.0 {
            continue;
        }
        // Work on straight color, then premultiply again.
        let (mut r, mut g, mut b) = (
            px[0] as f32 / 255.0 / a,
            px[1] as f32 / 255.0 / a,
            px[2] as f32 / 255.0 / a,
        );
        let mut alpha = a;
        for e in effects {
            match e {
                BlipEffect::Grayscale => {
                    let y = luma(r, g, b);
                    (r, g, b) = (y, y, y);
                }
                BlipEffect::Duotone(c1, c2) => {
                    let t = luma(r, g, b);
                    let mix = |x: f32, y: f32| x + (y - x) * t;
                    (r, g, b) = (mix(c1.r, c2.r), mix(c1.g, c2.g), mix(c1.b, c2.b));
                }
                BlipEffect::Lum { bright, contrast } => {
                    let (k, shift) = lum_slope_shift(*bright, *contrast);
                    let f = |v: f32| ((v - 0.5) * k + 0.5 + shift).clamp(0.0, 1.0);
                    (r, g, b) = (f(r), f(g), f(b));
                }
                BlipEffect::BiLevel(t) => {
                    let v = if luma(r, g, b) >= *t { 1.0 } else { 0.0 };
                    (r, g, b) = (v, v, v);
                }
                BlipEffect::ClrChange { from, to } => {
                    if close(r, g, b, from) {
                        (r, g, b, alpha) = (to.r, to.g, to.b, to.a * alpha);
                    }
                }
            }
        }
        px[0] = (r.clamp(0.0, 1.0) * alpha * 255.0).round() as u8;
        px[1] = (g.clamp(0.0, 1.0) * alpha * 255.0).round() as u8;
        px[2] = (b.clamp(0.0, 1.0) * alpha * 255.0).round() as u8;
        px[3] = (alpha * 255.0).round() as u8;
    }
}

/// `a:lum` as a linear map around mid-gray: returns the contrast slope and the
/// brightness shift, both fitted to PowerPoint renders. Positive contrast scales
/// the brightened value (shift = bright × slope); negative contrast damps the
/// brightness to bright × (1 + 0.55 × contrast), so "Washout" (+70 %/−70 %) stays visible.
fn lum_slope_shift(bright: f32, contrast: f32) -> (f32, f32) {
    if contrast > 0.0 {
        let k = 1.0 / (1.0 - contrast.min(0.99));
        (k, bright * k)
    } else {
        (1.0 + contrast, bright * (1.0 + 0.55 * contrast))
    }
}

/// Office matches `clrFrom` exactly; allow a little slack for JPEG noise.
fn close(r: f32, g: f32, b: f32, c: &Rgba) -> bool {
    const TOL: f32 = 3.0 / 255.0;
    (r - c.r).abs() <= TOL && (g - c.g).abs() <= TOL && (b - c.b).abs() <= TOL
}

#[cfg(test)]
mod tests;
