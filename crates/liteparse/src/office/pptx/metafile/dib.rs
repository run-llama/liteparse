//! Device-independent bitmaps: a BITMAPINFO header (+ color table) and pixel
//! bits, as embedded in EMF and WMF bitmap records. Decodes 1/4/8/16/24/32-bit
//! uncompressed, bitfields, RLE4/RLE8, and the JPEG/PNG pass-through modes into a
//! top-down premultiplied pixmap.

use tiny_skia::{IntSize, Pixmap};

use super::Rd;

/// Largest bitmap decoded, in pixels.
const MAX_DIB_PIXELS: u64 = 64 * 1024 * 1024;

const BI_RGB: u32 = 0;
const BI_RLE8: u32 = 1;
const BI_RLE4: u32 = 2;
const BI_BITFIELDS: u32 = 3;
const BI_JPEG: u32 = 4;
const BI_PNG: u32 = 5;
const BI_ALPHABITFIELDS: u32 = 6;

struct Header {
    width: u32,
    height: u32,
    bottom_up: bool,
    bpp: u16,
    compression: u32,
    /// Palette entries (RGBQUAD, or RGBTRIPLE for a core header).
    palette: Vec<[u8; 3]>,
    /// R, G, B, A masks for 16/32-bit bitfields.
    masks: Option<[u32; 4]>,
    /// Bytes of header + masks + palette: where bits start in a packed DIB.
    info_len: usize,
}

fn parse_header(bmi: &[u8]) -> Option<Header> {
    let r = Rd(bmi);
    let size = r.u32(0)? as usize;
    let (width, height, bpp, compression, clr_used, entry) = if size == 12 {
        let w = r.u16(4)? as i32;
        let h = r.i16(6)? as i32;
        (w, h, r.u16(10)?, BI_RGB, 0u32, 3usize)
    } else if size >= 40 {
        (
            r.i32(4)?,
            r.i32(8)?,
            r.u16(14)?,
            r.u32(16)?,
            r.u32(32)?,
            4usize,
        )
    } else {
        return None;
    };
    if width <= 0 || height == 0 || height == i32::MIN {
        return None;
    }
    let (w, h) = (width as u32, height.unsigned_abs());
    if w as u64 * h as u64 > MAX_DIB_PIXELS {
        return None;
    }
    let mut off = size;
    let mut masks = None;
    if compression == BI_BITFIELDS || compression == BI_ALPHABITFIELDS {
        let n = if compression == BI_ALPHABITFIELDS {
            4
        } else {
            3
        };
        // V4/V5 headers carry the masks inside the header itself.
        let base = if size >= 52 { 40 } else { size };
        let alpha = if size >= 56 || n == 4 {
            r.u32(base + 12).unwrap_or(0)
        } else {
            0
        };
        masks = Some([r.u32(base)?, r.u32(base + 4)?, r.u32(base + 8)?, alpha]);
        if size < 52 {
            off += n * 4;
        }
    }
    let count = if clr_used > 0 {
        (clr_used as usize).min(if bpp <= 8 { 1 << bpp } else { 4096 })
    } else if bpp <= 8 {
        1usize << bpp
    } else {
        0
    };
    let mut palette = Vec::with_capacity(count);
    for i in 0..count {
        let o = off + i * entry;
        match r.bytes(o, 3) {
            Some(b) => palette.push([b[2], b[1], b[0]]),
            None => break,
        }
    }
    let info_len = off + count * entry;
    Some(Header {
        width: w,
        height: h,
        bottom_up: height > 0,
        bpp,
        compression,
        palette,
        masks,
        info_len,
    })
}

/// Decodes a DIB whose header and bits are separate (EMF records).
/// `keep_alpha`: honor 32-bit alpha (AlphaBlend); GDI ignores it elsewhere.
pub fn decode(bmi: &[u8], bits: &[u8], keep_alpha: bool) -> Option<Pixmap> {
    let h = parse_header(bmi)?;
    decode_with(&h, bits, keep_alpha)
}

/// Decodes a packed DIB (header, color table and bits contiguous, as in WMF).
pub fn decode_packed(data: &[u8], keep_alpha: bool) -> Option<Pixmap> {
    let h = parse_header(data)?;
    let bits = data.get(h.info_len.min(data.len())..)?;
    decode_with(&h, bits, keep_alpha)
}

/// Whether a DIB header describes a 1-bit (monochrome) bitmap.
pub fn is_monochrome(bmi: &[u8]) -> bool {
    parse_header(bmi).is_some_and(|h| h.bpp == 1)
}

fn decode_with(h: &Header, bits: &[u8], keep_alpha: bool) -> Option<Pixmap> {
    if h.compression == BI_JPEG || h.compression == BI_PNG {
        return decode_embedded(bits);
    }
    let (w, ht) = (h.width as usize, h.height as usize);
    let mut out = vec![0u8; w * ht * 4];
    match h.compression {
        BI_RLE8 | BI_RLE4 => {
            let indices = decode_rle(bits, w, ht, h.compression == BI_RLE4);
            for (row, chunk) in indices.chunks_exact(w).enumerate() {
                let y = if h.bottom_up { ht - 1 - row } else { row };
                for (x, &(set, idx)) in chunk.iter().enumerate() {
                    if set {
                        let c = h.palette.get(idx as usize).copied().unwrap_or([0, 0, 0]);
                        put(&mut out, (y * w + x) * 4, c, 255);
                    }
                }
            }
        }
        BI_RGB | BI_BITFIELDS | BI_ALPHABITFIELDS => {
            let stride = (w * h.bpp as usize).div_ceil(32) * 4;
            if stride == 0 {
                return None;
            }
            let any_alpha = keep_alpha && h.bpp == 32 && has_alpha(bits, stride, w, ht, h.masks);
            for row in 0..ht {
                let y = if h.bottom_up { ht - 1 - row } else { row };
                let Some(line) = bits.get(row * stride..) else {
                    break;
                };
                let line = &line[..stride.min(line.len())];
                for x in 0..w {
                    let o = (y * w + x) * 4;
                    let Some((c, a)) = pixel(h, line, x, any_alpha) else {
                        break;
                    };
                    put(&mut out, o, c, a);
                }
            }
        }
        _ => return None,
    }
    let size = IntSize::from_wh(h.width, h.height)?;
    Pixmap::from_vec(out, size)
}

fn put(out: &mut [u8], o: usize, c: [u8; 3], a: u8) {
    let pm = |v: u8| ((v as u32 * a as u32 + 127) / 255) as u8;
    out[o] = pm(c[0]);
    out[o + 1] = pm(c[1]);
    out[o + 2] = pm(c[2]);
    out[o + 3] = a;
}

/// One pixel of an uncompressed row: color and straight alpha.
fn pixel(h: &Header, line: &[u8], x: usize, use_alpha: bool) -> Option<([u8; 3], u8)> {
    let pal = |i: usize| {
        h.palette
            .get(i)
            .copied()
            .unwrap_or(if i == 0 { [0, 0, 0] } else { [255, 255, 255] })
    };
    match h.bpp {
        1 => {
            let b = *line.get(x / 8)?;
            Some((pal(((b >> (7 - (x % 8))) & 1) as usize), 255))
        }
        2 => {
            let b = *line.get(x / 4)?;
            Some((pal(((b >> (6 - 2 * (x % 4))) & 3) as usize), 255))
        }
        4 => {
            let b = *line.get(x / 2)?;
            let i = if x.is_multiple_of(2) { b >> 4 } else { b & 0xF };
            Some((pal(i as usize), 255))
        }
        8 => Some((pal(*line.get(x)? as usize), 255)),
        16 => {
            let v = u16::from_le_bytes([*line.get(x * 2)?, *line.get(x * 2 + 1)?]) as u32;
            let m = h.masks.unwrap_or([0x7C00, 0x03E0, 0x001F, 0]);
            Some(([field(v, m[0]), field(v, m[1]), field(v, m[2])], 255))
        }
        24 => {
            let p = line.get(x * 3..x * 3 + 3)?;
            Some(([p[2], p[1], p[0]], 255))
        }
        32 => {
            let p = line.get(x * 4..x * 4 + 4)?;
            let v = u32::from_le_bytes([p[0], p[1], p[2], p[3]]);
            let m = h.masks.unwrap_or([0xFF_0000, 0xFF00, 0xFF, 0xFF00_0000]);
            let am = if m[3] == 0 { 0xFF00_0000 } else { m[3] };
            let a = if use_alpha { field(v, am) } else { 255 };
            let mut c = [field(v, m[0]), field(v, m[1]), field(v, m[2])];
            if use_alpha && a > 0 {
                // AlphaBlend sources are premultiplied; `put` premultiplies again.
                for ch in &mut c {
                    *ch = ((*ch as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8;
                }
            }
            Some((c, a))
        }
        _ => None,
    }
}

/// Extracts a bitfield and scales it to 0..=255.
fn field(v: u32, mask: u32) -> u8 {
    if mask == 0 {
        return 0;
    }
    let shift = mask.trailing_zeros();
    let max = (mask >> shift) as u64;
    let x = ((v & mask) >> shift) as u64;
    ((x * 255 + max / 2) / max) as u8
}

/// True when any pixel of a 32-bit bitmap has a non-zero alpha byte. Bitmaps
/// written by GDI leave alpha at 0, which means opaque.
fn has_alpha(bits: &[u8], stride: usize, w: usize, h: usize, masks: Option<[u32; 4]>) -> bool {
    let am = masks
        .map(|m| m[3])
        .filter(|&m| m != 0)
        .unwrap_or(0xFF00_0000);
    (0..h).any(|row| {
        bits.get(row * stride..row * stride + w * 4)
            .is_some_and(|l| {
                l.as_chunks::<4>()
                    .0
                    .iter()
                    .any(|p| u32::from_le_bytes([p[0], p[1], p[2], p[3]]) & am != 0)
            })
    })
}

/// RLE4/RLE8 into (written, palette index) per pixel, in stored row order
/// (bottom row first for bottom-up bitmaps). Skipped pixels stay unwritten,
/// which GDI leaves transparent.
fn decode_rle(data: &[u8], w: usize, h: usize, four: bool) -> Vec<(bool, u8)> {
    let mut out = vec![(false, 0u8); w * h];
    let (mut x, mut y, mut i) = (0usize, 0usize, 0usize);
    let set = |x: usize, y: usize, v: u8, out: &mut Vec<(bool, u8)>| {
        if x < w && y < h {
            out[y * w + x] = (true, v);
        }
    };
    while i + 1 < data.len() && y < h {
        let (n, v) = (data[i] as usize, data[i + 1]);
        i += 2;
        if n > 0 {
            for k in 0..n {
                let idx = if four {
                    if k % 2 == 0 { v >> 4 } else { v & 0xF }
                } else {
                    v
                };
                set(x, y, idx, &mut out);
                x += 1;
            }
            continue;
        }
        match v {
            0 => {
                x = 0;
                y += 1;
            }
            1 => break,
            2 => {
                let (Some(&dx), Some(&dy)) = (data.get(i), data.get(i + 1)) else {
                    break;
                };
                x += dx as usize;
                y += dy as usize;
                i += 2;
            }
            count => {
                let count = count as usize;
                let bytes = if four { count.div_ceil(2) } else { count };
                for k in 0..count {
                    let Some(&b) = data.get(i + if four { k / 2 } else { k }) else {
                        break;
                    };
                    let idx = if four {
                        if k % 2 == 0 { b >> 4 } else { b & 0xF }
                    } else {
                        b
                    };
                    set(x, y, idx, &mut out);
                    x += 1;
                }
                // Absolute runs are padded to a 16-bit boundary.
                i += bytes + (bytes & 1);
            }
        }
    }
    out
}

fn decode_embedded(bits: &[u8]) -> Option<Pixmap> {
    let img = image::load_from_memory(bits).ok()?;
    if img.width() as u64 * img.height() as u64 > MAX_DIB_PIXELS {
        return None;
    }
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    let mut data = rgba.into_raw();
    for px in data.as_chunks_mut::<4>().0 {
        let a = px[3] as u32;
        if a < 255 {
            for c in &mut px[..3] {
                *c = ((*c as u32 * a + 127) / 255) as u8;
            }
        }
    }
    Pixmap::from_vec(data, IntSize::from_wh(w, h)?)
}

#[cfg(test)]
mod tests;
