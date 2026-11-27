//! WMF (Windows Metafile, [MS-WMF]) records.
//!
//! Optional 22-byte placeable header (bounding box in logical units plus units
//! per inch), then a META_HEADER and 16-bit records: size in words, function,
//! parameters (coordinates are usually stored y before x). The window rectangle
//! maps onto the whole pixmap, the way a player sets the viewport to the frame.

use std::rc::Rc;

use tiny_skia::Pixmap;

use super::dc::{Brush, Font, Object, Pen, Rgb, Space};
use super::dib;
use super::player::{ArcKind, MAX_OBJECTS, Player};
use super::text::{TextRun, decode_ansi, decode_ansi_spans, merge_dx};
use super::{MAX_RECORDS, MetafileOutput, Rd, raster_size};
use crate::office::pptx::fonts::FontSystem;

const PLACEABLE_KEY: u32 = 0x9AC6_CDD7;

/// Placement of a WMF: where records start and the logical frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layout {
    pub records_at: usize,
    /// Frame in logical units (left, top, width, height); width/height may be
    /// negative when the window is flipped.
    pub frame: (f64, f64, f64, f64),
    /// Logical units per inch (placeable header), if known.
    pub inch: Option<f64>,
    pub objects: usize,
}

pub fn layout(bytes: &[u8]) -> Option<Layout> {
    let r = Rd(bytes);
    let mut off = 0usize;
    let mut placeable = None;
    if r.u32(0)? == PLACEABLE_KEY {
        let (l, t, rr, b) = (
            r.i16(6)? as f64,
            r.i16(8)? as f64,
            r.i16(10)? as f64,
            r.i16(12)? as f64,
        );
        let inch = r.u16(14)? as f64;
        placeable = Some(((l, t, rr - l, b - t), (inch > 0.0).then_some(inch)));
        off = 22;
    }
    let kind = r.u16(off)?;
    let hsize = r.u16(off + 2)? as usize;
    if !(kind == 1 || kind == 2) || hsize != 9 {
        return None;
    }
    let objects = r.u16(off + 10)? as usize;
    let records_at = off + hsize * 2;
    let (frame, inch) = match placeable {
        Some((f, inch)) if f.2 != 0.0 && f.3 != 0.0 => (f, inch),
        _ => (window_frame(bytes, records_at)?, None),
    };
    Some(Layout {
        records_at,
        frame,
        inch,
        objects,
    })
}

/// Frame of a non-placeable WMF: its window (SETWINDOWORG/EXT).
fn window_frame(bytes: &[u8], at: usize) -> Option<(f64, f64, f64, f64)> {
    let (mut org, mut ext) = ((0.0, 0.0), None);
    for (f, p) in records(bytes, at).take(10_000) {
        match f {
            0x020B => org = (p.i16(2).unwrap_or(0) as f64, p.i16(0).unwrap_or(0) as f64),
            0x020C => {
                ext = Some((p.i16(2).unwrap_or(0) as f64, p.i16(0).unwrap_or(0) as f64));
                break;
            }
            _ => {}
        }
    }
    let ext = ext.filter(|e| e.0 != 0.0 && e.1 != 0.0)?;
    Some((org.0, org.1, ext.0, ext.1))
}

/// Records as (function, parameter bytes), stopping at META_EOF, a malformed
/// size, or the record cap.
pub fn records(bytes: &[u8], start: usize) -> impl Iterator<Item = (u16, Rd<'_>)> {
    let mut off = start;
    let mut count = 0usize;
    std::iter::from_fn(move || {
        if count >= MAX_RECORDS {
            return None;
        }
        let r = Rd(bytes);
        let words = r.u32(off)? as usize;
        let f = r.u16(off + 4)?;
        if f == 0 || words < 3 {
            return None;
        }
        let end = off.checked_add(words.checked_mul(2)?)?.min(bytes.len());
        let params = Rd(bytes.get(off + 6..end)?);
        off = end;
        count += 1;
        Some((f, params))
    })
}

pub fn render(bytes: &[u8], fonts: &FontSystem) -> Option<MetafileOutput> {
    let mut p = player(bytes, fonts)?;
    let lay = layout(bytes)?;
    play(bytes, lay.records_at, &mut p);
    Some(p.finish())
}

pub(super) fn player<'f>(bytes: &[u8], fonts: &'f FontSystem) -> Option<Player<'f>> {
    let lay = layout(bytes)?;
    let (fl, ft, fw, fh) = lay.frame;
    let (pw, ph) = raster_size(fw.abs(), fh.abs())?;
    let pm = Pixmap::new(pw, ph)?;
    let px_per_unit = pw as f64 / fw.abs();
    // One 96-dpi device pixel; without a placeable header assume 1440 units/inch.
    let inch = lay.inch.unwrap_or(1440.0);
    let dev_px = (px_per_unit * inch / 96.0).clamp(0.5, pw.max(ph) as f64 / 200.0) as f32;
    let mut p = Player::new(
        pm,
        fonts,
        Space::Window {
            px_w: pw as f64,
            px_h: ph as f64,
        },
        dev_px,
    );
    p.dc.window_org = (fl, ft);
    p.dc.window_ext = (fw, fh);
    p.objects = vec![None; lay.objects.clamp(1, MAX_OBJECTS)];
    Some(p)
}

pub(super) fn play(bytes: &[u8], at: usize, p: &mut Player) {
    for (f, r) in records(bytes, at) {
        let _ = record(p, f, r);
    }
}

/// Places an object in the lowest free slot, as GDI does for WMF handles.
fn alloc(p: &mut Player, obj: Object) {
    let idx = p
        .objects
        .iter()
        .position(|o| o.is_none())
        .unwrap_or(p.objects.len());
    p.set_object(idx, obj);
}

fn pts16(r: Rd, off: usize, n: usize) -> Option<Vec<(f64, f64)>> {
    let b = r.bytes(off, n.checked_mul(4)?)?;
    Some(
        b.as_chunks::<4>()
            .0
            .iter()
            .map(|c| {
                (
                    i16::from_le_bytes([c[0], c[1]]) as f64,
                    i16::from_le_bytes([c[2], c[3]]) as f64,
                )
            })
            .collect(),
    )
}

/// Four i16 stored as bottom, right, top, left -> (l, t, r, b).
fn rect_rev(r: Rd, o: usize) -> Option<(f64, f64, f64, f64)> {
    Some((
        r.i16(o + 6)? as f64,
        r.i16(o + 4)? as f64,
        r.i16(o + 2)? as f64,
        r.i16(o)? as f64,
    ))
}

fn xy(r: Rd, o: usize) -> Option<(f64, f64)> {
    Some((r.i16(o + 2)? as f64, r.i16(o)? as f64))
}

fn record(p: &mut Player, f: u16, r: Rd) -> Option<()> {
    match f {
        0x001E => p.save_dc(),
        0x0127 => p.restore_dc(r.i16(0)? as i32),
        0x0102 => p.dc.bk_opaque = r.u16(0)? == 2,
        0x0103 => p.dc.map_mode = r.u16(0)? as u32,
        0x0104 => p.dc.rop2 = r.u16(0)? as u32,
        0x0106 => p.dc.winding = r.u16(0)? == 2,
        0x012E => p.dc.text_align = r.u16(0)? as u32,
        0x0201 => p.dc.bk_color = Rgb::from_colorref(r.u32(0)?),
        0x0209 => p.dc.text_color = Rgb::from_colorref(r.u32(0)?),
        0x020B => p.dc.window_org = xy(r, 0)?,
        0x020C => p.dc.window_ext = xy(r, 0)?,
        0x020F => {
            let (dx, dy) = xy(r, 0)?;
            p.dc.window_org = (p.dc.window_org.0 + dx, p.dc.window_org.1 + dy);
        }
        0x0410 => {
            // SCALEWINDOWEXT: yDenom, yNum, xDenom, xNum.
            let v = [r.i16(0)?, r.i16(2)?, r.i16(4)?, r.i16(6)?].map(|v| v as f64);
            if v[0] != 0.0 && v[2] != 0.0 {
                let e = p.dc.window_ext;
                p.dc.window_ext = (e.0 * v[3] / v[2], e.1 * v[1] / v[0]);
            }
        }
        0x0213 => {
            let (x, y) = xy(r, 0)?;
            p.line_to(x, y);
        }
        0x0214 => {
            let (x, y) = xy(r, 0)?;
            p.move_to(x, y);
        }
        0x0415 | 0x0416 => {
            let (l, t, rr, b) = rect_rev(r, 0)?;
            p.clip_logical_rect(l, t, rr, b, f == 0x0415);
        }
        0x012C => p.combine_clip(None, tiny_skia::FillRule::Winding, super::player::RGN_COPY),
        0x0418 => {
            let (l, t, rr, b) = rect_rev(r, 0)?;
            p.ellipse(l, t, rr, b);
        }
        0x041B => {
            let (l, t, rr, b) = rect_rev(r, 0)?;
            p.rectangle(l, t, rr, b);
        }
        0x061C => {
            let (h, w) = (r.i16(0)? as f64, r.i16(2)? as f64);
            let (l, t, rr, b) = rect_rev(r, 4)?;
            p.round_rect(l, t, rr, b, w, h);
        }
        0x0817 | 0x081A | 0x0830 => {
            let (xe, ye) = xy(r, 0)?;
            let (xs, ys) = xy(r, 4)?;
            let (l, t, rr, b) = rect_rev(r, 8)?;
            let kind = match f {
                0x0817 => ArcKind::Arc,
                0x081A => ArcKind::Pie,
                _ => ArcKind::Chord,
            };
            p.arc(kind, l, t, rr, b, xs, ys, xe, ye);
        }
        0x0324 | 0x0325 => {
            let n = r.u16(0)? as usize;
            let pts = pts16(r, 2, n)?;
            p.poly(&[&pts], f == 0x0324);
        }
        0x0538 => {
            let np = r.u16(0)? as usize;
            let counts: Vec<usize> = (0..np)
                .map(|i| r.u16(2 + i * 2).map(|c| c as usize))
                .collect::<Option<_>>()?;
            let total: usize = counts.iter().sum();
            let pts = pts16(r, 2 + np * 2, total)?;
            let mut polys: Vec<&[(f64, f64)]> = Vec::with_capacity(np);
            let mut i = 0;
            for c in counts {
                polys.push(&pts[i..i + c]);
                i += c;
            }
            p.poly(&polys, true);
        }
        0x061D => {
            // PATBLT: rop, height, width, y, x.
            let rop = r.u32(0)?;
            let (h, w) = (r.i16(4)? as f64, r.i16(6)? as f64);
            let (x, y) = xy(r, 8)?;
            pattern_blit(p, rop, (x, y, w, h));
        }
        0x0521 => {
            let n = r.u16(0)? as usize;
            let s = r.bytes(2, n)?;
            let o = 2 + n + (n & 1);
            let (x, y) = xy(r, o)?;
            let text = decode_ansi(s, p.dc.font.charset);
            p.text_out(TextRun {
                x,
                y,
                text,
                options: 0,
                dx: Vec::new(),
                dy: Vec::new(),
                rect: None,
            });
        }
        0x0A32 => {
            let (x, y) = xy(r, 0)?;
            let n = r.u16(4)? as usize;
            let options = r.u16(6)? as u32;
            let mut o = 8;
            let mut rect = None;
            if options & 0x6 != 0 {
                let v = [r.i16(8)?, r.i16(10)?, r.i16(12)?, r.i16(14)?].map(|v| v as f64);
                rect = Some((v[0], v[1], v[2], v[3]));
                o += 8;
            }
            let s = r.bytes(o, n)?;
            o += n + (n & 1);
            let dx: Vec<f64> = (0..n)
                .map_while(|i| r.i16(o + i * 2).map(|v| v as f64))
                .collect();
            let spans = decode_ansi_spans(s, p.dc.font.charset);
            let dx = if dx.len() == n {
                merge_dx(&spans, &dx)
            } else {
                Vec::new()
            };
            let text = spans.into_iter().map(|(c, _)| c).collect();
            p.text_out(TextRun {
                x,
                y,
                text,
                options: options & !0x10,
                dx,
                dy: Vec::new(),
                rect,
            });
        }
        0x02FA => {
            let style = r.u16(0)? as u32;
            let width = r.i16(2)? as f32;
            let color = Rgb::from_colorref(r.u32(6)?);
            let pen = Pen {
                style,
                width,
                color,
                cosmetic: width <= 0.0,
                user_dashes: Vec::new(),
            };
            alloc(p, Object::Pen(pen));
        }
        0x02FC => {
            let style = r.u16(0)? as u32;
            let c = Rgb::from_colorref(r.u32(2)?);
            let b = match style {
                1 => Brush::Null,
                2 => Brush::Hatched {
                    color: c,
                    style: r.u16(6).unwrap_or(0) as u32,
                },
                _ => Brush::Solid(c),
            };
            alloc(p, Object::Brush(b));
        }
        0x02FB => {
            let face: Vec<u8> = r
                .bytes(18, r.len().saturating_sub(18).min(32))
                .unwrap_or(&[])
                .iter()
                .copied()
                .take_while(|&c| c != 0)
                .collect();
            let font = Font {
                height: r.i16(0)? as i32,
                width: r.i16(2)? as i32,
                escapement: r.i16(4)? as i32,
                weight: r.i16(8)? as i32,
                italic: r.u8(10)? != 0,
                underline: r.u8(11)? != 0,
                strikeout: r.u8(12)? != 0,
                charset: r.u8(13)?,
                face: decode_ansi(&face, r.u8(13).filter(|&c| c != 2).unwrap_or(0))
                    .into_iter()
                    .filter_map(char::from_u32)
                    .collect(),
            };
            alloc(p, Object::Font(font));
        }
        0x0142 => {
            let b = match dib::decode_packed(r.bytes(4, r.len().saturating_sub(4))?, false) {
                Some(img) => Brush::Pattern(Rc::new(img)),
                None => Brush::Solid(Rgb {
                    r: 128,
                    g: 128,
                    b: 128,
                }),
            };
            alloc(p, Object::Brush(b));
        }
        0x01F9 => alloc(
            p,
            Object::Brush(Brush::Solid(Rgb {
                r: 128,
                g: 128,
                b: 128,
            })),
        ),
        0x00F7 | 0x06FF => alloc(p, Object::Other),
        0x012D => p.select_index(r.u16(0)? as usize),
        0x01F0 => p.delete_object(r.u16(0)? as usize),
        0x0940 | 0x0B41 | 0x0F43 => blit(p, f, r)?,
        _ => {}
    }
    Some(())
}

fn pattern_blit(p: &mut Player, rop: u32, (x, y, w, h): (f64, f64, f64, f64)) {
    let brush = match (rop >> 16) & 0xFF {
        0xF0 => p.dc.brush.clone(),
        0x00 => Brush::Solid(Rgb::BLACK),
        0xFF => Brush::Solid(Rgb::WHITE),
        _ => return,
    };
    p.fill_rect_with(x, y, x + w, y + h, brush);
}

/// DIBBITBLT (0x0940), DIBSTRETCHBLT (0x0B41), STRETCHDIB (0x0F43).
fn blit(p: &mut Player, f: u16, r: Rd) -> Option<()> {
    let rop = r.u32(0)?;
    // Variants without a bitmap carry an extra reserved word and are pattern ops.
    let no_dib = (f == 0x0940 && r.len() == 18) || (f == 0x0B41 && r.len() == 22);
    if no_dib {
        let (x, y) = xy(r, r.len() - 4)?;
        let (h, w) = (r.i16(r.len() - 8)? as f64, r.i16(r.len() - 6)? as f64);
        pattern_blit(p, rop, (x, y, w, h));
        return Some(());
    }
    // Fields after rop (and STRETCHDIB's color usage), each i16.
    let base = if f == 0x0F43 { 6 } else { 4 };
    let (src, dst, dib_at) = if f == 0x0940 {
        let (ys, xs) = (r.i16(base)? as f64, r.i16(base + 2)? as f64);
        let (h, w) = (r.i16(base + 4)? as f64, r.i16(base + 6)? as f64);
        let (yd, xd) = (r.i16(base + 8)? as f64, r.i16(base + 10)? as f64);
        ((xs, ys, w, h), (xd, yd, w, h), base + 12)
    } else {
        let (hs, ws) = (r.i16(base)? as f64, r.i16(base + 2)? as f64);
        let (ys, xs) = (r.i16(base + 4)? as f64, r.i16(base + 6)? as f64);
        let (hd, wd) = (r.i16(base + 8)? as f64, r.i16(base + 10)? as f64);
        let (yd, xd) = (r.i16(base + 12)? as f64, r.i16(base + 14)? as f64);
        ((xs, ys, ws, hs), (xd, yd, wd, hd), base + 16)
    };
    let data = r.bytes(dib_at, r.len().saturating_sub(dib_at))?;
    let img = dib::decode_packed(data, false)?;
    let bottom_up = Rd(data).u32(0) == Some(12) || Rd(data).i32(8).is_some_and(|h| h > 0);
    let (xs, mut ys, ws, hs) = src;
    if bottom_up && f == 0x0F43 {
        ys = img.height() as f64 - ys - hs.abs();
    }
    p.draw_bitmap(&img, (xs, ys, ws.abs(), hs.abs()), dst, rop);
    Some(())
}

#[cfg(test)]
mod tests;
