//! EMF (Enhanced Metafile, [MS-EMF]) records.
//!
//! The header gives the picture frame (`rclFrame`, 0.01 mm) and the reference
//! device size (`szlDevice` pixels over `szlMillimeters`). Device units are
//! mapped onto a pixmap that covers the frame; every record after the header is
//! replayed through [`Player`].

use tiny_skia::{Pixmap, Transform};

use super::dc::{Brush, Font, Object, Pen, Rgb, Space, modify_world};
use super::dib;
use super::emfplus::{self, PlusState};
use super::player::{ArcKind, MAX_OBJECTS, Player};
use super::text::{ETO_PDY, TextRun, decode_ansi, decode_ansi_spans, merge_dx};
use super::{MAX_RECORDS, MetafileOutput, Rd, raster_size};
use crate::office::pptx::fonts::FontSystem;

pub fn is_emf(bytes: &[u8]) -> bool {
    bytes.len() >= 88 && bytes[0..4] == [1, 0, 0, 0] && &bytes[40..44] == b" EMF"
}

/// Parsed EMR_HEADER fields that place the picture.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Header {
    /// Bounds of the drawing, device units (inclusive).
    pub bounds: (i32, i32, i32, i32),
    /// Picture frame, 0.01 mm (inclusive).
    pub frame: (i32, i32, i32, i32),
    /// Reference device size in pixels and in millimetres.
    pub device: (i32, i32),
    pub millimeters: (i32, i32),
    /// Reference device size in micrometres (EMF 1.4+), more precise than mm.
    pub micrometers: Option<(i32, i32)>,
    pub handles: u16,
}

impl Header {
    pub fn parse(bytes: &[u8]) -> Option<Header> {
        let r = Rd(bytes);
        let size = r.u32(4)? as usize;
        let rect = |o| Some((r.i32(o)?, r.i32(o + 4)?, r.i32(o + 8)?, r.i32(o + 12)?));
        // The description string may follow the header inside the record, so
        // the extension fields exist only if they end before it.
        let n_desc = r.u32(60)?;
        let header_end = if n_desc > 0 {
            (r.u32(64)? as usize).min(size)
        } else {
            size
        };
        let micrometers = if header_end >= 108 {
            Some((r.i32(100)?, r.i32(104)?))
        } else {
            None
        };
        Some(Header {
            bounds: rect(8)?,
            frame: rect(24)?,
            device: (r.i32(72)?, r.i32(76)?),
            millimeters: (r.i32(80)?, r.i32(84)?),
            micrometers: micrometers.filter(|m| m.0 > 0 && m.1 > 0),
            handles: r.u16(56)?,
        })
    }

    /// Size of one reference-device pixel, in 0.01 mm (x, y).
    pub fn hmm_per_device(&self) -> (f64, f64) {
        let (dx, dy) = (self.device.0 as f64, self.device.1 as f64);
        let per = |um: Option<f64>, mm: f64, dev: f64| -> f64 {
            let v = match um {
                Some(um) if dev > 0.0 => um / 10.0 / dev,
                _ if dev > 0.0 && mm > 0.0 => mm * 100.0 / dev,
                _ => 2540.0 / 96.0,
            };
            if v.is_finite() && v > 0.0 {
                v
            } else {
                2540.0 / 96.0
            }
        };
        (
            per(
                self.micrometers.map(|m| m.0 as f64),
                self.millimeters.0 as f64,
                dx,
            ),
            per(
                self.micrometers.map(|m| m.1 as f64),
                self.millimeters.1 as f64,
                dy,
            ),
        )
    }

    /// The frame to render, 0.01 mm (left, top, width, height). Falls back to
    /// the device bounds when the frame is empty.
    pub fn frame_hmm(&self) -> Option<(f64, f64, f64, f64)> {
        let (l, t, r, b) = self.frame;
        if r > l && b > t {
            return Some((l as f64, t as f64, (r - l) as f64, (b - t) as f64));
        }
        let (hx, hy) = self.hmm_per_device();
        let (l, t, r, b) = self.bounds;
        (r > l && b > t).then(|| {
            (
                l as f64 * hx,
                t as f64 * hy,
                (r - l + 1) as f64 * hx,
                (b - t + 1) as f64 * hy,
            )
        })
    }

    /// True when the recorded drawing bounds are empty (GDI writes
    /// `(0, 0, -1, -1)` for a metafile that drew nothing).
    pub fn draws_nothing(&self) -> bool {
        let (l, t, r, b) = self.bounds;
        r < l || b < t
    }
}

pub fn render(bytes: &[u8], fonts: &FontSystem) -> Option<MetafileOutput> {
    render_depth(bytes, fonts, 0)
}

/// [`render`] at a metafile nesting depth (for EMF+ embedded images).
pub fn render_depth(bytes: &[u8], fonts: &FontSystem, depth: u32) -> Option<MetafileOutput> {
    let hdr = Header::parse(bytes)?;
    let Some((fl, ft, fw, fh)) = hdr.frame_hmm() else {
        // No frame and empty bounds: the recorder saw nothing drawn (Office
        // writes such stubs for blank pictures), so the picture is blank.
        if !hdr.draws_nothing() {
            return None;
        }
        return Some(MetafileOutput {
            pixmap: Pixmap::new(1, 1)?,
            text: Vec::new(),
        });
    };
    let (pw, ph) = raster_size(fw, fh)?;
    let pm = Pixmap::new(pw, ph)?;
    let (sx, sy) = (pw as f64 / fw, ph as f64 / fh);
    let (hx, hy) = hdr.hmm_per_device();
    let dev_to_px = Transform::from_row(
        (hx * sx) as f32,
        0.0,
        0.0,
        (hy * sy) as f32,
        (-fl * sx) as f32,
        (-ft * sy) as f32,
    );
    let dev_per_mm = (100.0 / hx, 100.0 / hy);
    // One device pixel, but never so wide that hairlines swamp the picture.
    let dev_px = ((hx * sx + hy * sy) / 2.0).clamp(0.5, pw.max(ph) as f64 / 200.0) as f32;
    let mut p = Player::new(
        pm,
        fonts,
        Space::Device {
            dev_to_px,
            dev_per_mm,
        },
        dev_px,
    );
    p.objects = vec![None; (hdr.handles as usize).clamp(1, MAX_OBJECTS)];
    let mut plus = emfplus::is_plus_only(bytes).then(|| PlusState::new(depth));
    for (t, rec) in records(bytes) {
        match (t, plus.as_mut()) {
            (1, _) => {}
            (70, Some(st)) if rec.get(12..16) == Some(b"EMF+") => st.comment(&mut p, rec),
            _ => {
                let _ = record(&mut p, t, Rd(rec));
            }
        }
    }
    Some(p.finish())
}

/// Iterates records (type, whole record bytes), stopping at EMR_EOF, a
/// malformed size, or the record cap.
pub fn records(bytes: &[u8]) -> impl Iterator<Item = (u32, &[u8])> {
    let mut off = 0usize;
    let mut count = 0usize;
    std::iter::from_fn(move || {
        if count >= MAX_RECORDS {
            return None;
        }
        let r = Rd(bytes);
        let t = r.u32(off)?;
        let size = r.u32(off + 4)? as usize;
        if size < 8 || off + size > bytes.len() {
            return None;
        }
        let rec = &bytes[off..off + size];
        off += size;
        count += 1;
        if t == 14 {
            count = MAX_RECORDS; // EMR_EOF: this is the last record
        }
        Some((t, rec))
    })
}

fn points32(r: Rd, off: usize, n: usize) -> Option<Vec<(f64, f64)>> {
    let bytes = r.bytes(off, n.checked_mul(8)?)?;
    Some(
        bytes
            .as_chunks::<8>()
            .0
            .iter()
            .map(|c| (le_i32(&c[0..4]) as f64, le_i32(&c[4..8]) as f64))
            .collect(),
    )
}

fn points16(r: Rd, off: usize, n: usize) -> Option<Vec<(f64, f64)>> {
    let bytes = r.bytes(off, n.checked_mul(4)?)?;
    Some(
        bytes
            .as_chunks::<4>()
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

fn le_i32(b: &[u8]) -> i32 {
    i32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

fn rect(r: Rd, o: usize) -> Option<(f64, f64, f64, f64)> {
    Some((
        r.i32(o)? as f64,
        r.i32(o + 4)? as f64,
        r.i32(o + 8)? as f64,
        r.i32(o + 12)? as f64,
    ))
}

fn xform(r: Rd, o: usize) -> Option<Transform> {
    let v = [
        r.f32(o)?,
        r.f32(o + 4)?,
        r.f32(o + 8)?,
        r.f32(o + 12)?,
        r.f32(o + 16)?,
        r.f32(o + 20)?,
    ];
    Some(Transform::from_row(v[0], v[1], v[2], v[3], v[4], v[5]))
}

/// A DIB referenced by (offBmi, cbBmi, offBits, cbBits) fields at `o`.
fn dib_at(r: Rd, o: usize, keep_alpha: bool) -> Option<(Pixmap, bool)> {
    let (ob, cb, obits, cbits) = (
        r.u32(o)? as usize,
        r.u32(o + 4)? as usize,
        r.u32(o + 8)? as usize,
        r.u32(o + 12)? as usize,
    );
    if cb == 0 {
        return None;
    }
    let bmi = r.bytes(ob, cb)?;
    let bits = r.bytes(obits, cbits).unwrap_or(&[]);
    let mono = dib::is_monochrome(bmi);
    dib::decode(bmi, bits, keep_alpha).map(|pm| (pm, mono))
}

/// Whether the DIB at (offBmi..) is stored bottom-up (positive height).
fn bottom_up(r: Rd, o: usize) -> bool {
    let Some(ob) = r.u32(o) else { return true };
    let ob = ob as usize;
    match r.u32(ob) {
        Some(12) => true,
        _ => r.i32(ob + 8).is_some_and(|h| h > 0),
    }
}

fn logfont(r: Rd, o: usize) -> Option<Font> {
    let face: Vec<u16> = (0..32)
        .map_while(|i| r.u16(o + 28 + i * 2).filter(|&c| c != 0))
        .collect();
    Some(Font {
        height: r.i32(o)?,
        width: r.i32(o + 4)?,
        escapement: r.i32(o + 8)?,
        weight: r.i32(o + 16)?,
        italic: r.u8(o + 20)? != 0,
        underline: r.u8(o + 21)? != 0,
        strikeout: r.u8(o + 22)? != 0,
        charset: r.u8(o + 23)?,
        face: String::from_utf16_lossy(&face),
    })
}

/// Rectangles of an RGNDATA structure at `o`, device units.
fn region_rects(r: Rd, o: usize, len: usize) -> Vec<(f64, f64, f64, f64)> {
    let Some(n) = r.u32(o + 8) else {
        return Vec::new();
    };
    let hdr = r.u32(o).unwrap_or(32).clamp(32, 1024) as usize;
    let max = len.saturating_sub(hdr) / 16;
    (0..(n as usize).min(max))
        .filter_map(|i| rect(r, o + hdr + i * 16))
        .collect()
}

fn select(p: &mut Player, ih: u32) {
    if ih & 0x8000_0000 != 0 {
        if let Some(obj) = Player::stock(ih & 0x7FFF_FFFF) {
            p.select(&obj);
        }
    } else {
        p.select_index(ih as usize);
    }
}

fn record(p: &mut Player, t: u32, r: Rd) -> Option<()> {
    match t {
        // Polylines and polygons, 32-bit points: bounds, count, points.
        2..=6 => {
            let n = r.u32(24)? as usize;
            let pts = points32(r, 28, n)?;
            shape(p, t, &pts);
        }
        85..=89 => {
            let n = r.u32(24)? as usize;
            let pts = points16(r, 28, n)?;
            shape(p, t - 83, &pts);
        }
        // POLYPOLYLINE / POLYPOLYGON (32 and 16-bit points).
        7 | 8 | 90 | 91 => {
            let np = r.u32(24)? as usize;
            let total = r.u32(28)? as usize;
            let counts: Vec<usize> = (0..np)
                .map(|i| r.u32(32 + i * 4).map(|c| c as usize))
                .collect::<Option<_>>()?;
            let off = 32 + np * 4;
            let pts = if t <= 8 {
                points32(r, off, total)?
            } else {
                points16(r, off, total)?
            };
            let mut polys: Vec<&[(f64, f64)]> = Vec::with_capacity(np);
            let mut i = 0usize;
            for c in counts {
                let end = i.checked_add(c)?.min(pts.len());
                polys.push(&pts[i..end]);
                i = end;
            }
            p.poly(&polys, t == 8 || t == 91);
        }
        9 => p.dc.window_ext = (r.i32(8)? as f64, r.i32(12)? as f64),
        10 => p.dc.window_org = (r.i32(8)? as f64, r.i32(12)? as f64),
        11 => p.dc.viewport_ext = (r.i32(8)? as f64, r.i32(12)? as f64),
        12 => p.dc.viewport_org = (r.i32(8)? as f64, r.i32(12)? as f64),
        17 => p.dc.map_mode = r.u32(8)?,
        18 => p.dc.bk_opaque = r.u32(8)? == 2,
        19 => p.dc.winding = r.u32(8)? == 2,
        20 => p.dc.rop2 = r.u32(8)?,
        22 => p.dc.text_align = r.u32(8)?,
        24 => p.dc.text_color = Rgb::from_colorref(r.u32(8)?),
        25 => p.dc.bk_color = Rgb::from_colorref(r.u32(8)?),
        27 => p.move_to(r.i32(8)? as f64, r.i32(12)? as f64),
        29 | 30 => {
            let (l, tp, rr, b) = rect(r, 8)?;
            p.clip_logical_rect(l, tp, rr, b, t == 29);
        }
        31 | 32 => {
            // SCALEVIEWPORTEXTEX / SCALEWINDOWEXTEX: xNum, xDenom, yNum, yDenom.
            let v = [r.i32(8)?, r.i32(12)?, r.i32(16)?, r.i32(20)?].map(|v| v as f64);
            if v[1] != 0.0 && v[3] != 0.0 {
                let ext = if t == 31 {
                    &mut p.dc.viewport_ext
                } else {
                    &mut p.dc.window_ext
                };
                *ext = (ext.0 * v[0] / v[1], ext.1 * v[2] / v[3]);
            }
        }
        33 => p.save_dc(),
        34 => p.restore_dc(r.i32(8)?),
        35 => p.dc.world = xform(r, 8)?,
        36 => p.dc.world = modify_world(p.dc.world, xform(r, 8)?, r.u32(32)?),
        37 => select(p, r.u32(8)?),
        38 => {
            let style = r.u32(12)?;
            let width = r.i32(16)? as f32;
            let pen = Pen {
                style,
                width,
                color: Rgb::from_colorref(r.u32(24)?),
                cosmetic: width <= 1.0 && style & 0x10000 == 0,
                user_dashes: Vec::new(),
            };
            p.set_object(r.u32(8)? as usize, Object::Pen(pen));
        }
        39 => {
            let brush = brush(r.u32(12)?, r.u32(16)?, r.u32(20)?);
            p.set_object(r.u32(8)? as usize, Object::Brush(brush));
        }
        40 => p.delete_object(r.u32(8)? as usize),
        41 => {
            let (x, y, rad) = (r.i32(8)? as f64, r.i32(12)? as f64, r.u32(16)? as f64);
            p.angle_arc(x, y, rad, r.f32(20)? as f64, r.f32(24)? as f64);
        }
        42 => {
            let (l, tp, rr, b) = rect(r, 8)?;
            p.ellipse(l, tp, rr, b);
        }
        43 => {
            let (l, tp, rr, b) = rect(r, 8)?;
            p.rectangle(l, tp, rr, b);
        }
        44 => {
            let (l, tp, rr, b) = rect(r, 8)?;
            p.round_rect(l, tp, rr, b, r.i32(24)? as f64, r.i32(28)? as f64);
        }
        45..=47 | 55 => {
            let (l, tp, rr, b) = rect(r, 8)?;
            let (xs, ys, xe, ye) = rect(r, 24)?;
            let kind = match t {
                45 => ArcKind::Arc,
                46 => ArcKind::Chord,
                47 => ArcKind::Pie,
                _ => ArcKind::ArcTo,
            };
            p.arc(kind, l, tp, rr, b, xs, ys, xe, ye);
        }
        49 => p.set_object(r.u32(8)? as usize, Object::Other),
        54 => p.line_to(r.i32(8)? as f64, r.i32(12)? as f64),
        57 => p.dc.arc_clockwise = r.u32(8)? == 2,
        58 => p.dc.miter_limit = r.u32(8)?.min(1000) as f32,
        59 => p.begin_path(),
        60 => p.end_path(),
        61 => p.close_figure(),
        62 => p.fill_path(),
        63 => p.stroke_and_fill_path(),
        64 => p.stroke_path(),
        67 => p.select_clip_path(r.u32(8)?),
        68 => p.abort_path(),
        71 | 74 => {
            // FILLRGN (with a brush object) / PAINTRGN (current brush).
            let cb = r.u32(24)? as usize;
            let (brush_obj, off) = if t == 71 { (r.u32(28), 32) } else { (None, 28) };
            let rects = region_rects(r, off, cb);
            let b = match brush_obj {
                Some(ih) if ih & 0x8000_0000 != 0 => match Player::stock(ih & 0x7FFF_FFFF) {
                    Some(Object::Brush(b)) => b,
                    _ => return None,
                },
                Some(ih) => match p.objects.get(ih as usize) {
                    Some(Some(Object::Brush(b))) => b.clone(),
                    _ => return None,
                },
                None => p.dc.brush.clone(),
            };
            p.fill_device_rects(&rects, b);
        }
        75 => {
            let cb = r.u32(8)? as usize;
            let mode = r.u32(12)?;
            let rects = if cb >= 32 {
                region_rects(r, 16, cb)
            } else {
                Vec::new()
            };
            p.clip_device_rects(&rects, mode);
        }
        76 | 77 | 114 | 116 => blit(p, t, r)?,
        80 => {
            // SETDIBITSTODEVICE: 1:1 copy of (part of) a DIB.
            let (xd, yd) = (r.i32(24)? as f64, r.i32(28)? as f64);
            let (xs, ys, cx, cy) = (
                r.i32(32)? as f64,
                r.i32(36)? as f64,
                r.i32(40)? as f64,
                r.i32(44)? as f64,
            );
            let (img, _) = dib_at(r, 48, false)?;
            let ys = if bottom_up(r, 48) {
                img.height() as f64 - ys - cy
            } else {
                ys
            };
            p.draw_bitmap(&img, (xs, ys, cx, cy), (xd, yd, cx, cy), 0x00CC_0020);
        }
        81 => {
            let (xd, yd) = (r.i32(24)? as f64, r.i32(28)? as f64);
            let (xs, ys, cxs, cys) = (
                r.i32(32)? as f64,
                r.i32(36)? as f64,
                r.i32(40)? as f64,
                r.i32(44)? as f64,
            );
            let rop = r.u32(68)?;
            let (cxd, cyd) = (r.i32(72)? as f64, r.i32(76)? as f64);
            match dib_at(r, 48, false) {
                Some((img, _)) => {
                    // StretchDIBits measures ySrc from the bottom of a bottom-up DIB.
                    let ys = if bottom_up(r, 48) {
                        img.height() as f64 - ys - cys
                    } else {
                        ys
                    };
                    p.draw_bitmap(&img, (xs, ys, cxs, cys), (xd, yd, cxd, cyd), rop);
                }
                None => pattern_blit(p, rop, (xd, yd, cxd, cyd)),
            }
        }
        82 => {
            let font = logfont(r, 12)?;
            p.set_object(r.u32(8)? as usize, Object::Font(font));
        }
        83 | 84 => text(p, r, t == 84)?,
        93 | 94 => {
            // CREATEMONOBRUSH / CREATEDIBPATTERNBRUSHPT: ih, usage, DIB.
            let ih = r.u32(8)? as usize;
            let b = match dib_at(r, 16, false) {
                Some((img, _)) => Brush::Pattern(std::rc::Rc::new(img)),
                None => Brush::Solid(Rgb {
                    r: 128,
                    g: 128,
                    b: 128,
                }),
            };
            p.set_object(ih, Object::Brush(b));
        }
        95 => {
            let ih = r.u32(8)? as usize;
            let style = r.u32(28)?;
            let width = r.u32(32)? as f32;
            let brush_style = r.u32(36)?;
            let n = r.u32(48)? as usize;
            let user_dashes = (0..n.min(64))
                .filter_map(|i| r.u32(52 + i * 4).map(|v| v as f32))
                .collect();
            let pen = Pen {
                style: if brush_style == 1 { 5 } else { style },
                width,
                color: Rgb::from_colorref(r.u32(40)?),
                cosmetic: style & 0x10000 == 0,
                user_dashes,
            };
            p.set_object(ih, Object::Pen(pen));
        }
        108 => small_text(p, r)?,
        118 => gradient_fill(p, r)?,
        _ => {}
    }
    Some(())
}

/// Polyline-family records by their 32-bit type: 2 POLYBEZIER, 3 POLYGON,
/// 4 POLYLINE, 5 POLYBEZIERTO, 6 POLYLINETO.
fn shape(p: &mut Player, t: u32, pts: &[(f64, f64)]) {
    match t {
        2 => p.poly_bezier(pts),
        3 => p.poly(&[pts], true),
        4 => p.poly(&[pts], false),
        5 => p.bezier_to(pts),
        6 => {
            let from = [p.dc.cur.0, p.dc.cur.1];
            p.polyline_to(pts, &from);
        }
        _ => {}
    }
}

fn brush(style: u32, color: u32, hatch: u32) -> Brush {
    let c = Rgb::from_colorref(color);
    match style {
        1 => Brush::Null,
        2 => Brush::Hatched {
            color: c,
            style: hatch,
        },
        _ => Brush::Solid(c),
    }
}

/// Raster operations that need no source bitmap.
fn pattern_blit(p: &mut Player, rop: u32, (x, y, w, h): (f64, f64, f64, f64)) {
    let brush = match (rop >> 16) & 0xFF {
        0xF0 => p.dc.brush.clone(),
        0x00 => Brush::Solid(Rgb::BLACK),
        0xFF => Brush::Solid(Rgb::WHITE),
        _ => return,
    };
    p.fill_rect_with(x, y, x + w, y + h, brush);
}

/// BITBLT (76), STRETCHBLT (77), ALPHABLEND (114), TRANSPARENTBLT (116).
fn blit(p: &mut Player, t: u32, r: Rd) -> Option<()> {
    let (xd, yd, cxd, cyd) = (
        r.i32(24)? as f64,
        r.i32(28)? as f64,
        r.i32(32)? as f64,
        r.i32(36)? as f64,
    );
    let rop = r.u32(40)?;
    let (xs, ys) = (r.i32(44)? as f64, r.i32(48)? as f64);
    let src_xf = xform(r, 52).unwrap_or_default();
    let has_src = r.u32(88).unwrap_or(0) > 0;
    if !has_src {
        if t == 76 || t == 77 {
            pattern_blit(p, rop, (xd, yd, cxd, cyd));
        }
        return Some(());
    }
    // ALPHABLEND's BLENDFUNCTION: AC_SRC_ALPHA in byte 3, constant alpha in byte 2.
    let keep_alpha = t == 114 && (rop >> 24) & 1 == 1;
    let (mut img, _) = dib_at(r, 84, keep_alpha)?;
    let (cxs, cys) = if t == 76 {
        (cxd, cyd)
    } else {
        (r.i32(100)? as f64, r.i32(104)? as f64)
    };
    // The source transform maps source DC units to bitmap pixels; usually a scale.
    let (xs, ys) = (
        xs * src_xf.sx as f64 + src_xf.tx as f64,
        ys * src_xf.sy as f64 + src_xf.ty as f64,
    );
    let (cxs, cys) = if t == 76 {
        (cxs * src_xf.sx as f64, cys * src_xf.sy as f64)
    } else {
        (cxs, cys)
    };
    let rop = match t {
        114 => {
            let alpha = ((rop >> 16) & 0xFF) as f32 / 255.0;
            if alpha < 1.0 {
                scale_alpha(&mut img, alpha);
            }
            0x00CC_0020
        }
        116 => {
            let key = Rgb::from_colorref(rop);
            color_key(&mut img, key);
            0x00CC_0020
        }
        _ => rop,
    };
    p.draw_bitmap(
        &img,
        (xs, ys, cxs.abs(), cys.abs()),
        (xd, yd, cxd, cyd),
        rop,
    );
    Some(())
}

/// EMR_GRADIENTFILL: bounds, nVer, nTri, mode, `TRIVERTEX[nVer]` (x, y, and
/// 16-bit r, g, b, a), then rectangles (2 indices) or triangles (3 indices).
fn gradient_fill(p: &mut Player, r: Rd) -> Option<()> {
    let nv = (r.u32(24)? as usize).min(1 << 20);
    let no = (r.u32(28)? as usize).min(1 << 20);
    let mode = r.u32(32)?;
    let vert = |i: usize| -> Option<((f64, f64), Rgb)> {
        let o = 36 + i.checked_mul(16)?;
        let c = Rgb {
            r: (r.u16(o + 8)? >> 8) as u8,
            g: (r.u16(o + 10)? >> 8) as u8,
            b: (r.u16(o + 12)? >> 8) as u8,
        };
        Some(((r.i32(o)? as f64, r.i32(o + 4)? as f64), c))
    };
    let objs = 36 + nv * 16;
    for k in 0..no {
        if !p.spend(4) {
            break;
        }
        if mode == 2 {
            let o = objs + k * 12;
            let (a, b, c) = (
                vert(r.u32(o)? as usize)?,
                vert(r.u32(o + 4)? as usize)?,
                vert(r.u32(o + 8)? as usize)?,
            );
            p.gradient_triangle([a.0, b.0, c.0], [a.1, b.1, c.1]);
        } else {
            let o = objs + k * 8;
            let (a, b) = (vert(r.u32(o)? as usize)?, vert(r.u32(o + 4)? as usize)?);
            p.gradient_rect((a.0.0, a.0.1, b.0.0, b.0.1), a.1, b.1, mode == 1);
        }
    }
    Some(())
}

fn scale_alpha(img: &mut Pixmap, a: f32) {
    for v in img.data_mut() {
        *v = (*v as f32 * a).round() as u8;
    }
}

fn color_key(img: &mut Pixmap, key: Rgb) {
    for px in img.data_mut().as_chunks_mut::<4>().0 {
        if px[3] == 255 && px[0] == key.r && px[1] == key.g && px[2] == key.b {
            px.copy_from_slice(&[0, 0, 0, 0]);
        }
    }
}

/// EXTTEXTOUTA/W: bounds, graphics mode, scales, then EMRTEXT at 36.
fn text(p: &mut Player, r: Rd, wide: bool) -> Option<()> {
    let (x, y) = (r.i32(36)? as f64, r.i32(40)? as f64);
    let n = (r.u32(44)? as usize).min(1 << 16);
    let off_string = r.u32(48)? as usize;
    let options = r.u32(52)?;
    let rc = rect(r, 56)?;
    let off_dx = r.u32(72)? as usize;
    let mut spans = None;
    let text: Vec<u32> = if wide {
        let b = r.bytes(off_string, n * 2)?;
        let units: Vec<u16> = b
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        if options & super::text::ETO_GLYPH_INDEX != 0 {
            units.iter().map(|&u| u as u32).collect()
        } else {
            char::decode_utf16(units.iter().copied())
                .map(|c| c.map(|c| c as u32).unwrap_or(0xFFFD))
                .collect()
        }
    } else {
        let sp = decode_ansi_spans(r.bytes(off_string, n)?, p.dc.font.charset);
        let text = sp.iter().map(|&(c, _)| c).collect();
        spans = Some(sp);
        text
    };
    let per = if options & ETO_PDY != 0 { 2 } else { 1 };
    let mut dx = Vec::new();
    let mut dy = Vec::new();
    if off_dx > 0 {
        for i in 0..n {
            let Some(v) = r.i32(off_dx + i * 4 * per) else {
                break;
            };
            dx.push(v as f64);
            if per == 2 {
                dy.push(r.i32(off_dx + i * 8 + 4).unwrap_or(0) as f64);
            }
        }
    }
    // Surrogate pairs decode to one char but carry two dx entries.
    if wide && dx.len() > text.len() && options & super::text::ETO_GLYPH_INDEX == 0 {
        dx = merge_surrogate_dx(r.bytes(off_string, n * 2)?, &dx);
    }
    if let Some(sp) = spans.filter(|sp| sp.len() < n) {
        dx = merge_dx(&sp, &dx);
        dy.clear();
    }
    let has_rect = rc.2 > rc.0 && rc.3 > rc.1;
    p.text_out(TextRun {
        x,
        y,
        text,
        options,
        dx,
        dy,
        rect: has_rect.then_some(rc),
    });
    Some(())
}

fn merge_surrogate_dx(bytes: &[u8], dx: &[f64]) -> Vec<f64> {
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    let mut out = Vec::with_capacity(dx.len());
    let mut i = 0;
    while i < units.len() && i < dx.len() {
        if (0xD800..0xDC00).contains(&units[i]) && i + 1 < dx.len() {
            out.push(dx[i] + dx[i + 1]);
            i += 2;
        } else {
            out.push(dx[i]);
            i += 1;
        }
    }
    out
}

/// EMR_SMALLTEXTOUT: x, y, count, options, graphics mode, scales, optional
/// clip rectangle, then 8-bit (ETO_SMALL_CHARS) or 16-bit text.
fn small_text(p: &mut Player, r: Rd) -> Option<()> {
    let (x, y) = (r.i32(8)? as f64, r.i32(12)? as f64);
    let n = (r.u32(16)? as usize).min(1 << 16);
    let options = r.u32(20)?;
    let (rc, off) = if options & 0x100 == 0 {
        (rect(r, 36), 52)
    } else {
        (None, 36)
    };
    let text = if options & 0x200 != 0 {
        decode_ansi(r.bytes(off, n)?, p.dc.font.charset)
    } else {
        let b = r.bytes(off, n * 2)?;
        char::decode_utf16(
            b.as_chunks::<2>()
                .0
                .iter()
                .map(|c| u16::from_le_bytes([c[0], c[1]])),
        )
        .map(|c| c.map(|c| c as u32).unwrap_or(0xFFFD))
        .collect()
    };
    p.text_out(TextRun {
        x,
        y,
        text,
        options: options & !0x300,
        dx: Vec::new(),
        dy: Vec::new(),
        rect: rc,
    });
    Some(())
}

#[cfg(test)]
mod tests;
