//! `DynamicImage::resize(edge, edge, Triangle)` for 8-bit images, computed in
//! bands of output rows.
//!
//! The `image` crate resamples columns into a full-height `f32` RGBA copy
//! (16 bytes per pixel: 340 MB for a 5000 px picture) before resampling rows.
//! This module runs the same two passes, with the same weights and the same
//! float operations in the same order, over a few output rows at a time, so
//! the result is identical while the scratch memory stays a few megabytes.

use image::{DynamicImage, ImageBuffer, Pixel};

/// Output rows resampled per band.
const BAND_ROWS: usize = 32;

/// Source index range start and normalized weights of one output pixel, as
/// `image::imageops::sample` computes them for the triangle filter.
fn weights(src: u32, dst: u32) -> Vec<(usize, Vec<f32>)> {
    let ratio = src as f32 / dst as f32;
    let sratio = if ratio < 1.0 { 1.0 } else { ratio };
    let support = 1.0 * sratio;
    (0..dst)
        .map(|out| {
            let input = (out as f32 + 0.5) * ratio;
            let left = ((input - support).floor() as i64).clamp(0, src as i64 - 1);
            let right = ((input + support).ceil() as i64).clamp(left + 1, src as i64);
            let input = input - 0.5;
            let mut ws: Vec<f32> = (left..right)
                .map(|i| {
                    let x = (i as f32 - input) / sratio;
                    if x.abs() < 1.0 { 1.0 - x.abs() } else { 0.0 }
                })
                .collect();
            let sum = ws.iter().fold(0.0f32, |acc, w| acc + w);
            for w in ws.iter_mut() {
                *w /= sum;
            }
            (left as usize, ws)
        })
        .collect()
}

/// `image`'s `resize_dimensions` (fit inside `edge` x `edge`, keep aspect).
fn fit(width: u32, height: u32, edge: u32) -> (u32, u32) {
    let ratio = f64::min(edge as f64 / width as f64, edge as f64 / height as f64);
    let nw = ((width as f64 * ratio).round() as u64).max(1);
    let nh = ((height as f64 * ratio).round() as u64).max(1);
    (
        nw.min(u32::MAX as u64) as u32,
        nh.min(u32::MAX as u64) as u32,
    )
}

fn resize_buffer<P: Pixel<Subpixel = u8>>(
    src: &ImageBuffer<P, Vec<u8>>,
    nw: u32,
    nh: u32,
) -> ImageBuffer<P, Vec<u8>> {
    let n = P::CHANNEL_COUNT as usize;
    let (w, h) = (src.width() as usize, src.height() as usize);
    let data = src.as_raw();
    let rows = weights(h as u32, nh);
    let cols = weights(w as u32, nw);
    let mut out = vec![0u8; nw as usize * nh as usize * n];
    let mut band = vec![0f32; BAND_ROWS * w * n];
    for (b, chunk) in rows.chunks(BAND_ROWS).enumerate() {
        // Columns pass: each band row is a weighted sum of source rows.
        for (r, (top, ws)) in chunk.iter().enumerate() {
            let row = &mut band[r * w * n..(r + 1) * w * n];
            row.fill(0.0);
            for (i, wt) in ws.iter().enumerate() {
                let line = &data[(top + i) * w * n..(top + i + 1) * w * n];
                for (acc, v) in row.iter_mut().zip(line) {
                    *acc += *v as f32 * wt;
                }
            }
        }
        // Rows pass, rounded and clamped to 0..=255 like `image`.
        for r in 0..chunk.len() {
            let row = &band[r * w * n..(r + 1) * w * n];
            let oy = b * BAND_ROWS + r;
            let dst = &mut out[oy * nw as usize * n..(oy + 1) * nw as usize * n];
            for (ox, (left, ws)) in cols.iter().enumerate() {
                for c in 0..n {
                    let mut t = 0.0f32;
                    for (i, wt) in ws.iter().enumerate() {
                        t += row[(left + i) * n + c] * wt;
                    }
                    dst[ox * n + c] = t.clamp(0.0, 255.0).round() as u8;
                }
            }
        }
    }
    let mut buf = ImageBuffer::from_raw(nw, nh, out).expect("sized above");
    drop(buf.set_color_space(src.color_space()));
    buf
}

/// The image resized to fit `edge` with the triangle filter, or `None` for
/// pixel formats this module does not handle (the caller then uses `image`).
pub(super) fn fit_triangle(img: &DynamicImage, edge: u32) -> Option<DynamicImage> {
    let (nw, nh) = fit(img.width(), img.height(), edge);
    if (nw, nh) == (img.width(), img.height()) || img.width() == 0 || img.height() == 0 {
        return None;
    }
    Some(match img {
        DynamicImage::ImageLuma8(b) => DynamicImage::ImageLuma8(resize_buffer(b, nw, nh)),
        DynamicImage::ImageLumaA8(b) => DynamicImage::ImageLumaA8(resize_buffer(b, nw, nh)),
        DynamicImage::ImageRgb8(b) => DynamicImage::ImageRgb8(resize_buffer(b, nw, nh)),
        DynamicImage::ImageRgba8(b) => DynamicImage::ImageRgba8(resize_buffer(b, nw, nh)),
        _ => return None,
    })
}

#[cfg(test)]
mod tests;
