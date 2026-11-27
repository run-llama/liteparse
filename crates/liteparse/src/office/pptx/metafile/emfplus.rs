//! A minimal EMF+ ([MS-EMFPLUS]) player, used only for "EMF+ only" files whose
//! plain EMF records draw nothing. Dual files are played from their EMF records.
//!
//! Supported: brush (solid; gradients as their mean color), pen (width, color,
//! dashes), path and image objects (bitmaps, compressed images, nested
//! metafiles); FillRects/DrawRects, FillPath/DrawPath, FillPolygon/DrawLines,
//! Fill/DrawEllipse, DrawImage/DrawImagePoints, Clear; world and page
//! transforms, Save/Restore/containers, SetClipRect/SetClipPath/ResetClip.
//! Text (DrawString/DrawDriverString) is not drawn.

use std::collections::HashMap;
use std::rc::Rc;

use tiny_skia::{
    Color, FillRule, FilterQuality, IntSize, LineCap, LineJoin, Paint, Path, PathBuilder, Pixmap,
    PixmapPaint, Shader, Stroke, StrokeDash, Transform,
};

use super::Rd;
use super::dc::{Clip, Space};
use super::player::Player;
use crate::office::pptx::fonts::FontSystem;
use crate::office::pptx::render::{anti_alias_fits, stroke_outset};

/// Nested metafile images rendered inside an EMF+ image object, at most.
const MAX_NESTING: u32 = 2;

#[derive(Clone)]
enum Obj {
    Brush(Color),
    Pen {
        width: f32,
        color: Color,
        dashes: Vec<f32>,
    },
    Path(Path),
    Image(Rc<Pixmap>),
}

pub struct PlusState {
    dpi: f32,
    world: Transform,
    page_scale: f32,
    page_unit: u8,
    objects: HashMap<u8, Obj>,
    saves: HashMap<u32, (Transform, Clip)>,
    /// Object id -> (declared total size, bytes so far) for continued objects.
    partial: Option<(u8, u16, Vec<u8>, usize)>,
    depth: u32,
}

/// Whether the first record after the EMF header is an EMF+ header without the
/// "dual" flag, i.e. the plain EMF records are not a usable fallback.
pub fn is_plus_only(bytes: &[u8]) -> bool {
    let r = Rd(bytes);
    let Some(off) = r.u32(4).map(|v| v as usize) else {
        return false;
    };
    r.u32(off) == Some(70)
        && r.bytes(off + 12, 4) == Some(b"EMF+")
        && r.u16(off + 16) == Some(0x4001)
        && r.u16(off + 18).is_some_and(|f| f & 1 == 0)
}

impl PlusState {
    pub fn new(depth: u32) -> PlusState {
        PlusState {
            dpi: 96.0,
            world: Transform::identity(),
            page_scale: 1.0,
            page_unit: 2,
            objects: HashMap::new(),
            saves: HashMap::new(),
            partial: None,
            depth,
        }
    }

    /// Plays the EMF+ records inside one EMR_COMMENT (`rec` is the whole record).
    pub fn comment(&mut self, p: &mut Player, rec: &[u8]) {
        let r = Rd(rec);
        let Some(n) = r.u32(8).map(|v| v as usize) else {
            return;
        };
        let end = (12 + n).min(rec.len());
        let mut off = 16;
        while off + 12 <= end {
            let (Some(t), Some(flags), Some(size), Some(dsize)) =
                (r.u16(off), r.u16(off + 2), r.u32(off + 4), r.u32(off + 8))
            else {
                return;
            };
            let size = size as usize;
            if size < 12 || off + size > end || !p.spend(1) {
                return;
            }
            let data = &rec[off + 12..off + 12 + (dsize as usize).min(size - 12)];
            let _ = self.record(p, t, flags, Rd(data));
            off += size;
        }
    }

    fn transform(&self, p: &Player) -> Transform {
        let unit = match self.page_unit {
            3 => self.dpi / 72.0,
            4 => self.dpi,
            5 => self.dpi / 300.0,
            6 => self.dpi / 25.4,
            _ => 1.0,
        };
        let page = Transform::from_scale(self.page_scale * unit, self.page_scale * unit);
        let dev = match p.space {
            Space::Device { dev_to_px, .. } => dev_to_px,
            Space::Window { .. } => Transform::identity(),
        };
        dev.pre_concat(page).pre_concat(self.world)
    }

    fn record(&mut self, p: &mut Player, t: u16, flags: u16, r: Rd) -> Option<()> {
        match t {
            0x4001 => {
                self.dpi = r
                    .u32(8)
                    .map(|v| v as f32)
                    .filter(|v| *v > 0.0)
                    .unwrap_or(96.0)
            }
            0x4008 => self.object(flags, r, p.fonts)?,
            0x4009 => {
                let c = argb(r.u32(0)?);
                if c.alpha() > 0.0 {
                    p.pm.fill(c);
                }
            }
            0x400A | 0x400B => {
                let n = (r.u32(4)? as usize).min(1 << 20);
                let compressed = flags & 0x4000 != 0;
                let mut pb = PathBuilder::new();
                for i in 0..n {
                    let (x, y, w, h) = rect(r, 8, i, compressed)?;
                    if let Some(rc) = tiny_skia::Rect::from_xywh(x, y, w, h) {
                        pb.push_rect(rc);
                    }
                }
                let path = pb.finish()?;
                if t == 0x400A {
                    self.fill(p, &path, flags, r.u32(0)?, FillRule::Winding);
                } else {
                    self.stroke(p, &path, (flags & 0xFF) as u8);
                }
            }
            0x400C => {
                let n = (r.u32(4)? as usize).min(1 << 20);
                let path = poly_path(&points(r, 8, n, flags)?, true)?;
                let rule = if flags & 0x2000 != 0 {
                    FillRule::Winding
                } else {
                    FillRule::EvenOdd
                };
                self.fill(p, &path, flags, r.u32(0)?, rule);
            }
            0x400D => {
                let n = (r.u32(0)? as usize).min(1 << 20);
                let path = poly_path(&points(r, 4, n, flags)?, flags & 0x2000 != 0)?;
                self.stroke(p, &path, (flags & 0xFF) as u8);
            }
            0x400E | 0x400F => {
                let off = if t == 0x400E { 4 } else { 0 };
                let (x, y, w, h) = rect(r, off, 0, flags & 0x4000 != 0)?;
                let path = PathBuilder::from_oval(tiny_skia::Rect::from_xywh(x, y, w, h)?)?;
                if t == 0x400E {
                    self.fill(p, &path, flags, r.u32(0)?, FillRule::Winding);
                } else {
                    self.stroke(p, &path, (flags & 0xFF) as u8);
                }
            }
            0x4014 => {
                if let Some(Obj::Path(path)) = self.objects.get(&((flags & 0xFF) as u8)).cloned() {
                    self.fill(p, &path, flags, r.u32(0)?, FillRule::Winding);
                }
            }
            0x4015 => {
                if let Some(Obj::Path(path)) = self.objects.get(&((flags & 0xFF) as u8)).cloned() {
                    self.stroke(p, &path, r.u32(0)? as u8);
                }
            }
            0x401A | 0x401B => {
                let Some(Obj::Image(img)) = self.objects.get(&((flags & 0xFF) as u8)).cloned()
                else {
                    return Some(());
                };
                let src = (r.f32(8)?, r.f32(12)?, r.f32(16)?, r.f32(20)?);
                let (ul, ur, ll) = if t == 0x401A {
                    let (x, y, w, h) = rect(r, 24, 0, flags & 0x4000 != 0)?;
                    ((x, y), (x + w, y), (x, y + h))
                } else {
                    let pts = points(r, 28, 3, flags)?;
                    (pts[0], pts[1], pts[2])
                };
                self.image(p, &img, src, ul, ur, ll);
            }
            0x402A => self.world = matrix(r, 0)?,
            0x402B => self.world = Transform::identity(),
            0x402C => {
                let m = matrix(r, 0)?;
                self.world = if flags & 0x2000 != 0 {
                    self.world.post_concat(m)
                } else {
                    self.world.pre_concat(m)
                };
            }
            0x402D..=0x402F => {
                let m = match t {
                    0x402D => Transform::from_translate(r.f32(0)?, r.f32(4)?),
                    0x402E => Transform::from_scale(r.f32(0)?, r.f32(4)?),
                    _ => Transform::from_rotate(r.f32(0)?),
                };
                self.world = if flags & 0x2000 != 0 {
                    self.world.post_concat(m)
                } else {
                    self.world.pre_concat(m)
                };
            }
            0x4030 => {
                self.page_unit = (flags & 0xFF) as u8;
                self.page_scale = r.f32(0).filter(|v| *v > 0.0).unwrap_or(1.0);
            }
            0x4025 | 0x4027 | 0x4028 => {
                let id = if t == 0x4027 { r.u32(36)? } else { r.u32(0)? };
                self.saves.insert(id, (self.world, p.dc.clip.clone()));
                if self.saves.len() > 4096 {
                    self.saves.clear();
                }
            }
            0x4026 | 0x4029 => {
                if let Some((w, c)) = self.saves.remove(&r.u32(0)?) {
                    self.world = w;
                    p.dc.clip = c;
                }
            }
            0x4031 => {
                let (x, y, w, h) = (r.f32(0)?, r.f32(4)?, r.f32(8)?, r.f32(12)?);
                let path = PathBuilder::from_rect(tiny_skia::Rect::from_xywh(x, y, w, h)?)
                    .transform(self.transform(p))?;
                p.combine_clip(Some(&path), FillRule::Winding, clip_mode(flags));
            }
            0x4032 => {
                if let Some(Obj::Path(path)) = self.objects.get(&((flags & 0xFF) as u8)).cloned() {
                    let path = path.transform(self.transform(p))?;
                    p.combine_clip(Some(&path), FillRule::Winding, clip_mode(flags));
                }
            }
            0x4033 if (flags >> 8) & 0xF == 0 => {
                // Region objects are not parsed; a replace still resets the clip.
                p.dc.clip = Clip::None;
            }
            _ => {}
        }
        Some(())
    }

    fn object(&mut self, flags: u16, r: Rd, fonts: &FontSystem) -> Option<()> {
        let id = (flags & 0xFF) as u8;
        let kind = (flags >> 8) & 0x7F;
        let data: Vec<u8>;
        let body = if flags & 0x8000 != 0 {
            // Continued object: u32 total size, then a chunk of it.
            let total = r.u32(0)? as usize;
            let chunk = r.bytes(4, r.len().saturating_sub(4))?;
            let mut st = match self.partial.take() {
                Some(st) if st.0 == id && st.1 == kind => st,
                _ => (id, kind, Vec::new(), total.min(256 << 20)),
            };
            st.2.extend_from_slice(chunk);
            if st.2.len() < st.3 {
                self.partial = Some(st);
                return Some(());
            }
            data = st.2;
            Rd(&data)
        } else {
            r
        };
        let obj = match kind {
            1 => Obj::Brush(brush(body)?),
            2 => pen(body)?,
            3 => Obj::Path(path_object(body)?),
            5 => Obj::Image(Rc::new(self.image_object(body, fonts)?)),
            _ => return Some(()),
        };
        self.objects.insert(id, obj);
        Some(())
    }

    fn image_object(&self, r: Rd, fonts: &FontSystem) -> Option<Pixmap> {
        match r.u32(4)? {
            1 => {
                let (w, h, stride, fmt, kind) =
                    (r.i32(8)?, r.i32(12)?, r.i32(16)?, r.u32(20)?, r.u32(24)?);
                let data = r.bytes(28, r.len().saturating_sub(28))?;
                if kind == 1 {
                    return decode_compressed(data);
                }
                raw_bitmap(data, w, h, stride, fmt)
            }
            2 => {
                if self.depth >= MAX_NESTING {
                    return None;
                }
                let size = r.u32(12)? as usize;
                let data = r.bytes(16, size.min(r.len().saturating_sub(16)))?;
                super::render_nested(data, fonts, self.depth + 1)
            }
            _ => None,
        }
    }

    fn fill(&mut self, p: &mut Player, path: &Path, flags: u16, brush_id: u32, rule: FillRule) {
        let color = if flags & 0x8000 != 0 {
            argb(brush_id)
        } else {
            match self.objects.get(&(brush_id as u8)) {
                Some(Obj::Brush(c)) => *c,
                _ => return,
            }
        };
        let t = self.transform(p);
        let mask = p.clip_mask();
        let paint = Paint {
            shader: Shader::SolidColor(color),
            anti_alias: anti_alias_fits(path, t, 0.0),
            ..Default::default()
        };
        p.pm.fill_path(path, &paint, rule, t, mask.as_deref());
    }

    fn stroke(&mut self, p: &mut Player, path: &Path, pen_id: u8) {
        let Some(Obj::Pen {
            width,
            color,
            dashes,
        }) = self.objects.get(&pen_id).cloned()
        else {
            return;
        };
        let t = self.transform(p);
        let Some(px_path) = path.clone().transform(t) else {
            return;
        };
        let scale = (t.sx * t.sy - t.kx * t.ky).abs().sqrt();
        let w = (width * scale).max(p.dev_px.min(1.0));
        let mut st = Stroke {
            width: w,
            line_cap: LineCap::Butt,
            line_join: LineJoin::Miter,
            ..Default::default()
        };
        if !dashes.is_empty() {
            st.dash = StrokeDash::new(dashes.iter().map(|d| (d * w).max(0.1)).collect(), 0.0);
        }
        let mask = p.clip_mask();
        let paint = Paint {
            shader: Shader::SolidColor(color),
            anti_alias: anti_alias_fits(&px_path, Transform::identity(), stroke_outset(&st)),
            ..Default::default()
        };
        p.pm.stroke_path(
            &px_path,
            &paint,
            &st,
            Transform::identity(),
            mask.as_deref(),
        );
    }

    fn image(
        &mut self,
        p: &mut Player,
        img: &Pixmap,
        src: (f32, f32, f32, f32),
        ul: (f32, f32),
        ur: (f32, f32),
        ll: (f32, f32),
    ) {
        let (sx, sy, sw, sh) = src;
        if sw <= 0.0 || sh <= 0.0 {
            return;
        }
        // Source rectangle (bitmap pixels) onto the destination parallelogram.
        let place = Transform::from_row(
            (ur.0 - ul.0) / sw,
            (ur.1 - ul.1) / sw,
            (ll.0 - ul.0) / sh,
            (ll.1 - ul.1) / sh,
            ul.0,
            ul.1,
        )
        .pre_translate(-sx, -sy);
        let t = self.transform(p).pre_concat(place);
        let mask = p.clip_mask();
        let paint = PixmapPaint {
            quality: FilterQuality::Bilinear,
            ..Default::default()
        };
        p.pm.draw_pixmap(0, 0, img.as_ref(), &paint, t, mask.as_deref());
    }
}

fn clip_mode(flags: u16) -> u32 {
    // EMF+ CombineMode: 0 replace, 1 intersect, 2 union, 3 xor, 4 exclude.
    match (flags >> 8) & 0xF {
        0 => super::player::RGN_COPY,
        1 => super::player::RGN_AND,
        2 => super::player::RGN_OR,
        3 => super::player::RGN_XOR,
        4 => super::player::RGN_DIFF,
        _ => super::player::RGN_AND,
    }
}

fn argb(v: u32) -> Color {
    Color::from_rgba8((v >> 16) as u8, (v >> 8) as u8, v as u8, (v >> 24) as u8)
}

fn matrix(r: Rd, o: usize) -> Option<Transform> {
    Some(Transform::from_row(
        r.f32(o)?,
        r.f32(o + 4)?,
        r.f32(o + 8)?,
        r.f32(o + 12)?,
        r.f32(o + 16)?,
        r.f32(o + 20)?,
    ))
}

/// Rectangle `i` of an array at `o`: four f32, or four i16 when compressed.
fn rect(r: Rd, o: usize, i: usize, compressed: bool) -> Option<(f32, f32, f32, f32)> {
    if compressed {
        let b = o + i * 8;
        Some((
            r.i16(b)? as f32,
            r.i16(b + 2)? as f32,
            r.i16(b + 4)? as f32,
            r.i16(b + 6)? as f32,
        ))
    } else {
        let b = o + i * 16;
        Some((r.f32(b)?, r.f32(b + 4)?, r.f32(b + 8)?, r.f32(b + 12)?))
    }
}

/// Point array at `o`: f32 pairs, i16 pairs (flag 0x4000), or relative
/// run-length-encoded offsets (flag 0x0800).
fn points(r: Rd, o: usize, n: usize, flags: u16) -> Option<Vec<(f32, f32)>> {
    let mut out = Vec::with_capacity(n.min(1 << 16));
    if flags & 0x0800 != 0 {
        let (mut x, mut y, mut off) = (0.0f32, 0.0f32, o);
        for _ in 0..n {
            let mut read = || -> Option<f32> {
                let b = r.u8(off)?;
                if b & 0x80 != 0 {
                    let v = (((b as u16 & 0x7F) << 8) | r.u8(off + 1)? as u16) as i16;
                    off += 2;
                    // 15-bit signed value.
                    Some(((v << 1) >> 1) as f32)
                } else {
                    off += 1;
                    Some((((b << 1) as i8) >> 1) as f32)
                }
            };
            x += read()?;
            y += read()?;
            out.push((x, y));
        }
    } else if flags & 0x4000 != 0 {
        for i in 0..n {
            out.push((r.i16(o + i * 4)? as f32, r.i16(o + i * 4 + 2)? as f32));
        }
    } else {
        for i in 0..n {
            out.push((r.f32(o + i * 8)?, r.f32(o + i * 8 + 4)?));
        }
    }
    Some(out)
}

fn poly_path(pts: &[(f32, f32)], closed: bool) -> Option<Path> {
    let mut pb = PathBuilder::new();
    for (i, &(x, y)) in pts.iter().enumerate() {
        if i == 0 {
            pb.move_to(x, y);
        } else {
            pb.line_to(x, y);
        }
    }
    if closed {
        pb.close();
    }
    pb.finish()
}

/// EmfPlusBrush: version, type, data. Solid = one ARGB; gradients use the
/// mean of their two end colors.
fn brush(r: Rd) -> Option<Color> {
    match r.u32(4)? {
        0 => Some(argb(r.u32(8)?)),
        // Linear gradient: flags, wrap, rect (16), start ARGB, end ARGB.
        4 => {
            let (a, b) = (argb(r.u32(32)?), argb(r.u32(36)?));
            Some(Color::from_rgba(
                (a.red() + b.red()) / 2.0,
                (a.green() + b.green()) / 2.0,
                (a.blue() + b.blue()) / 2.0,
                (a.alpha() + b.alpha()) / 2.0,
            )?)
        }
        // Hatch: style, fore color, back color.
        1 => Some(argb(r.u32(12)?)),
        _ => Some(Color::from_rgba8(128, 128, 128, 255)),
    }
}

/// EmfPlusPen: version, type, flags, unit, width, optional data (by flags),
/// then an EmfPlusBrush for the color.
fn pen(r: Rd) -> Option<Obj> {
    let flags = r.u32(8)?;
    let width = r.f32(16)?;
    let mut o = 20usize;
    let mut dashes = Vec::new();
    let mut style = 0;
    for (bit, size) in [
        (0x01u32, 24usize),
        (0x02, 4),
        (0x04, 4),
        (0x08, 4),
        (0x10, 4),
        (0x20, 4),
        (0x40, 4),
        (0x80, 4),
    ] {
        if flags & bit != 0 {
            if bit == 0x20 {
                style = r.u32(o)?;
            }
            o += size;
        }
    }
    if flags & 0x100 != 0 {
        let n = (r.u32(o)? as usize).min(64);
        dashes = (0..n).filter_map(|i| r.f32(o + 4 + i * 4)).collect();
        o += 4 + n * 4;
    }
    if flags & 0x200 != 0 {
        o += 4;
    }
    if flags & 0x400 != 0 {
        o += 4 + r.u32(o)? as usize * 4;
    }
    for bit in [0x800u32, 0x1000] {
        if flags & bit != 0 {
            o += 4 + r.u32(o)? as usize;
        }
    }
    if dashes.is_empty() {
        dashes = match style {
            1 => vec![3.0, 1.0],
            2 => vec![1.0, 1.0],
            3 => vec![3.0, 1.0, 1.0, 1.0],
            4 => vec![3.0, 1.0, 1.0, 1.0, 1.0, 1.0],
            _ => Vec::new(),
        };
    }
    if dashes.len() % 2 == 1 {
        dashes.extend_from_within(..);
    }
    let color = r
        .bytes(o, r.len().saturating_sub(o))
        .and_then(|b| brush(Rd(b)))
        .unwrap_or(Color::BLACK);
    Some(Obj::Pen {
        width,
        color,
        dashes,
    })
}

/// EmfPlusPath: version, count, flags, points, point types (1 byte each:
/// 0 start, 1 line, 3 Bézier; 0x80 closes the subpath).
fn path_object(r: Rd) -> Option<Path> {
    let n = (r.u32(4)? as usize).min(1 << 20);
    let flags = r.u32(8)? as u16;
    let pts = points(r, 12, n, flags)?;
    let pts_len = if flags & 0x0800 != 0 {
        // Relative points have variable size; find types by scanning backwards is
        // not possible, so only absolute formats are supported here.
        return None;
    } else if flags & 0x4000 != 0 {
        n * 4
    } else {
        n * 8
    };
    let types = r.bytes(12 + pts_len, n)?;
    let mut pb = PathBuilder::new();
    let mut i = 0;
    while i < n {
        let ty = types[i];
        let (x, y) = pts[i];
        match ty & 0x7 {
            0 => pb.move_to(x, y),
            3 if i + 2 < n => {
                let (a, b) = (pts[i + 1], pts[i + 2]);
                pb.cubic_to(x, y, a.0, a.1, b.0, b.1);
                i += 2;
            }
            _ => pb.line_to(x, y),
        }
        if types[i] & 0x80 != 0 {
            pb.close();
        }
        i += 1;
    }
    pb.finish()
}

fn decode_compressed(data: &[u8]) -> Option<Pixmap> {
    let img = image::load_from_memory(data).ok()?;
    if img.width() as u64 * img.height() as u64 > 64 << 20 {
        return None;
    }
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    let mut v = rgba.into_raw();
    for px in v.as_chunks_mut::<4>().0 {
        let a = px[3] as u32;
        for c in &mut px[..3] {
            *c = ((*c as u32 * a + 127) / 255) as u8;
        }
    }
    Pixmap::from_vec(v, IntSize::from_wh(w, h)?)
}

/// Uncompressed GDI+ bitmap rows (top-down). Pixel formats: 32bpp ARGB /
/// PARGB / RGB and 24bpp RGB.
fn raw_bitmap(data: &[u8], w: i32, h: i32, stride: i32, fmt: u32) -> Option<Pixmap> {
    if w <= 0 || h <= 0 || stride <= 0 || (w as u64 * h as u64) > (64 << 20) {
        return None;
    }
    let (w, h, stride) = (w as usize, h as usize, stride as usize);
    let bpp = match fmt {
        0x0026_200A | 0x000E_200B | 0x0002_2009 => 4,
        0x0002_1808 => 3,
        _ => return None,
    };
    let mut out = vec![0u8; w * h * 4];
    for y in 0..h {
        let Some(row) = data.get(y * stride..y * stride + w * bpp) else {
            break;
        };
        for x in 0..w {
            let p = &row[x * bpp..x * bpp + bpp];
            let (b, g, r) = (p[0] as u32, p[1] as u32, p[2] as u32);
            let a = if bpp == 4 && fmt != 0x0002_2009 {
                p[3] as u32
            } else {
                255
            };
            let pm = |c: u32| {
                if fmt == 0x000E_200B {
                    c.min(a)
                } else {
                    (c * a + 127) / 255
                }
            };
            out[(y * w + x) * 4..(y * w + x) * 4 + 4].copy_from_slice(&[
                pm(r) as u8,
                pm(g) as u8,
                pm(b) as u8,
                a as u8,
            ]);
        }
    }
    Pixmap::from_vec(out, IntSize::from_wh(w as u32, h as u32)?)
}
