//! TIFF variants the `image` crate rejects: palette colour (`RGBPalette`) and
//! CMYK with an extra alpha sample. Both are read with the `tiff` crate, which
//! handles the compression and predictors; this module maps the samples to RGBA.

use image::{DynamicImage, RgbaImage};
use std::io::Cursor;
use tiff::ColorType;
use tiff::decoder::{Decoder, DecodingResult};
use tiff::tags::Tag;

/// One entry of the first image directory: (value offset in the file, type, count).
fn find_entry(b: &[u8], tag: u16) -> Option<(usize, u16, u32)> {
    let le = b.starts_with(b"II*\0");
    if !le && !b.starts_with(b"MM\0*") {
        return None;
    }
    let u16_at = |i: usize| -> Option<u16> {
        let x = [*b.get(i)?, *b.get(i + 1)?];
        Some(if le {
            u16::from_le_bytes(x)
        } else {
            u16::from_be_bytes(x)
        })
    };
    let u32_at = |i: usize| -> Option<u32> {
        let x = [*b.get(i)?, *b.get(i + 1)?, *b.get(i + 2)?, *b.get(i + 3)?];
        Some(if le {
            u32::from_le_bytes(x)
        } else {
            u32::from_be_bytes(x)
        })
    };
    let ifd = u32_at(4)? as usize;
    for k in 0..u16_at(ifd)? as usize {
        let e = ifd + 2 + 12 * k;
        if u16_at(e)? == tag {
            return Some((e + 8, u16_at(e + 2)?, u32_at(e + 4)?));
        }
    }
    None
}

pub(super) fn decode(b: &[u8], max_pixels: u64) -> Result<DynamicImage, String> {
    let (photo_at, ty, _) = find_entry(b, 262).ok_or("not a TIFF")?;
    let le = b.starts_with(b"II");
    let photometric = if le { b[photo_at] } else { b[photo_at + 1] };
    if ty != 3 {
        return Err("unsupported TIFF".into());
    }
    match photometric {
        3 => decode_palette(b, photo_at, le, max_pixels),
        5 => decode_cmyk(b, max_pixels),
        _ => Err("unsupported TIFF".into()),
    }
}

/// A decoder over the file and the picture's width and height.
type Opened<'a> = (Decoder<Cursor<&'a [u8]>>, u32, u32);

fn open(b: &[u8], max_pixels: u64) -> Result<Opened<'_>, String> {
    let mut d = Decoder::new(Cursor::new(b)).map_err(|e| e.to_string())?;
    let (w, h) = d.dimensions().map_err(|e| e.to_string())?;
    if w as u64 * h as u64 > max_pixels {
        return Err(format!("{w}x{h} exceeds the pixel limit"));
    }
    Ok((d, w, h))
}

/// Palette TIFF: decoded as grayscale indices (the photometric tag patched to
/// BlackIsZero), then looked up in the 16-bit `ColorMap`.
fn decode_palette(
    b: &[u8],
    photo_at: usize,
    le: bool,
    max_pixels: u64,
) -> Result<DynamicImage, String> {
    let mut patched = b.to_vec();
    patched[photo_at + usize::from(!le)] = 1;
    let (mut d, w, h) = open(&patched, max_pixels)?;
    let ColorType::Gray(bits @ (1 | 2 | 4 | 8)) = d.colortype().map_err(|e| e.to_string())? else {
        return Err("unsupported palette TIFF".into());
    };
    let map = d
        .get_tag_u16_vec(Tag::ColorMap)
        .map_err(|e| e.to_string())?;
    let n = 1usize << bits;
    if map.len() < 3 * n {
        return Err("short TIFF colour map".into());
    }
    let DecodingResult::U8(idx) = d.read_image().map_err(|e| e.to_string())? else {
        return Err("unsupported palette TIFF".into());
    };
    let row_bytes = (w as usize * bits as usize).div_ceil(8);
    let mut out = RgbaImage::new(w, h);
    for (y, row) in idx.chunks(row_bytes).take(h as usize).enumerate() {
        for x in 0..w as usize {
            let bit = x * bits as usize;
            let byte = row.get(bit / 8).copied().unwrap_or(0);
            let i = ((byte as usize) >> (8 - bits as usize - bit % 8)) & (n - 1);
            let px = [
                (map[i] >> 8) as u8,
                (map[n + i] >> 8) as u8,
                (map[2 * n + i] >> 8) as u8,
                255,
            ];
            out.put_pixel(x as u32, y as u32, image::Rgba(px));
        }
    }
    Ok(DynamicImage::ImageRgba8(out))
}

/// 8-bit CMYK, with or without an alpha sample. Inks convert as
/// `R = (255 − C)(255 − K) / 255`, the conversion `image` uses for plain CMYK;
/// associated (premultiplied) alpha is divided out first.
fn decode_cmyk(b: &[u8], max_pixels: u64) -> Result<DynamicImage, String> {
    let (mut d, w, h) = open(b, max_pixels)?;
    let ct = d.colortype().map_err(|e| e.to_string())?;
    let spp = match ct {
        ColorType::CMYK(8) => 4,
        ColorType::CMYKA(8) => 5,
        _ => return Err("unsupported CMYK TIFF".into()),
    };
    let associated = spp == 5
        && d.find_tag_unsigned_vec::<u16>(Tag::ExtraSamples)
            .ok()
            .flatten()
            .is_some_and(|v| v.first() == Some(&1));
    let DecodingResult::U8(px) = d.read_image().map_err(|e| e.to_string())? else {
        return Err("unsupported CMYK TIFF".into());
    };
    let mut out = RgbaImage::new(w, h);
    for (i, s) in px
        .chunks_exact(spp)
        .take(w as usize * h as usize)
        .enumerate()
    {
        let a = if spp == 5 { s[4] } else { 255 };
        let ink = |v: u8| -> u32 {
            if associated && a > 0 && a < 255 {
                (v as u32 * 255 / a as u32).min(255)
            } else {
                v as u32
            }
        };
        let k = 255 - ink(s[3]);
        let ch = |v: u8| ((255 - ink(v)) * k / 255) as u8;
        let (x, y) = ((i % w as usize) as u32, (i / w as usize) as u32);
        out.put_pixel(x, y, image::Rgba([ch(s[0]), ch(s[1]), ch(s[2]), a]));
    }
    Ok(DynamicImage::ImageRgba8(out))
}
