//! The replay engine shared by the EMF and WMF parsers: device-context stack,
//! object table, path bracket, clipping, and the drawing primitives.
//!
//! Geometry is mapped to pixmap pixels as soon as it is recorded, the way GDI
//! converts path points to device units when they are added. Pen widths use
//! the transform in effect when the stroke is drawn.

use std::rc::Rc;

use tiny_skia::{
    FillRule, FilterQuality, IntRect, LineCap, LineJoin, Mask, Paint, Path, PathBuilder, Pattern,
    Pixmap, PixmapPaint, Point, Shader, SpreadMode, Stroke, StrokeDash, Transform,
};

use super::dc::{Brush, Clip, Dc, Font, Object, PS_STYLE_MASK, PS_USERSTYLE, Pen, Rgb, Space};
use super::{MAX_POINTS, MetafileOutput, MetafileText};
use crate::office::pptx::fonts::FontSystem;
use crate::office::pptx::render::{anti_alias_fits, stroke_outset};

/// Deepest SaveDC stack kept; deeper saves are ignored.
const MAX_SAVE_DEPTH: usize = 4096;
/// Largest object table (EMF handle count / WMF object count).
pub const MAX_OBJECTS: usize = 65_536;

pub const RGN_AND: u32 = 1;
pub const RGN_OR: u32 = 2;
pub const RGN_XOR: u32 = 3;
pub const RGN_DIFF: u32 = 4;
pub const RGN_COPY: u32 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArcKind {
    Arc,
    Chord,
    Pie,
    /// ARCTO: line from the current position to the arc start, then the arc;
    /// updates the current position.
    ArcTo,
}

/// The current GDI path (BEGINPATH .. ENDPATH), in pixmap pixels.
struct Bracket {
    pb: PathBuilder,
    /// Still recording (BEGINPATH seen, ENDPATH not yet).
    recording: bool,
    /// A figure is in progress, so connected calls continue it.
    figure_open: bool,
}

pub struct Player<'f> {
    pub pm: Pixmap,
    pub fonts: &'f FontSystem,
    pub space: Space,
    pub dc: Dc,
    stack: Vec<Dc>,
    pub objects: Vec<Option<Object>>,
    bracket: Option<Bracket>,
    /// Mask built for the last rectangular clip, reused while it stays current.
    rect_mask: Option<(tiny_skia::Rect, Rc<Mask>)>,
    /// One reference-device pixel, in pixmap pixels: the cosmetic pen width.
    pub dev_px: f32,
    points: u64,
    /// Text runs drawn so far, for extraction.
    pub texts: Vec<MetafileText>,
}

/// Maps a logical point with a transform, in f64 for large coordinates.
pub fn map(t: &Transform, x: f64, y: f64) -> (f32, f32) {
    let px = t.sx as f64 * x + t.kx as f64 * y + t.tx as f64;
    let py = t.ky as f64 * x + t.sy as f64 * y + t.ty as f64;
    (px as f32, py as f32)
}

/// Length of the image of a logical unit vector.
pub fn vec_len(t: &Transform, x: f32, y: f32) -> f32 {
    let vx = t.sx * x + t.kx * y;
    let vy = t.ky * x + t.sy * y;
    (vx * vx + vy * vy).sqrt()
}

fn det(t: &Transform) -> f32 {
    t.sx * t.sy - t.kx * t.ky
}

fn finite(p: (f32, f32)) -> bool {
    p.0.is_finite() && p.1.is_finite() && p.0.abs() < 1e7 && p.1.abs() < 1e7
}

pub fn solid(c: Rgb) -> Paint<'static> {
    Paint {
        shader: Shader::SolidColor(c.to_color()),
        anti_alias: true,
        ..Default::default()
    }
}

impl<'f> Player<'f> {
    pub fn new(pm: Pixmap, fonts: &'f FontSystem, space: Space, dev_px: f32) -> Player<'f> {
        Player {
            pm,
            fonts,
            space,
            dc: Dc::default(),
            stack: Vec::new(),
            objects: Vec::new(),
            bracket: None,
            rect_mask: None,
            dev_px,
            points: 0,
            texts: Vec::new(),
        }
    }

    pub fn finish(self) -> MetafileOutput {
        MetafileOutput {
            pixmap: self.pm,
            text: self.texts,
        }
    }

    /// Logical-to-pixmap transform for the current DC.
    pub fn t(&self) -> Transform {
        self.dc.transform(&self.space)
    }

    /// Charges `n` points to the work budget; false once it is exhausted.
    pub fn spend(&mut self, n: usize) -> bool {
        self.points = self.points.saturating_add(n as u64 + 1);
        self.points <= MAX_POINTS
    }

    // ---- objects and state -------------------------------------------------

    pub fn save_dc(&mut self) {
        if self.stack.len() < MAX_SAVE_DEPTH {
            self.stack.push(self.dc.clone());
        }
    }

    /// RESTOREDC: negative = relative to the top of the stack, positive = absolute
    /// (1-based) save level.
    pub fn restore_dc(&mut self, n: i32) {
        let len = self.stack.len() as i64;
        let idx = if n < 0 { len + n as i64 } else { n as i64 - 1 };
        if idx >= 0 && idx < len {
            let idx = idx as usize;
            self.dc = self.stack[idx].clone();
            self.stack.truncate(idx);
        }
    }

    pub fn set_object(&mut self, index: usize, obj: Object) {
        if index >= MAX_OBJECTS {
            return;
        }
        if self.objects.len() <= index {
            self.objects.resize(index + 1, None);
        }
        self.objects[index] = Some(obj);
    }

    pub fn delete_object(&mut self, index: usize) {
        if let Some(slot) = self.objects.get_mut(index) {
            *slot = None;
        }
    }

    pub fn select(&mut self, obj: &Object) {
        match obj {
            Object::Pen(p) => self.dc.pen = p.clone(),
            Object::Brush(b) => self.dc.brush = b.clone(),
            Object::Font(f) => self.dc.font = f.clone(),
            Object::Other => {}
        }
    }

    pub fn select_index(&mut self, index: usize) {
        if let Some(Some(obj)) = self.objects.get(index) {
            let obj = obj.clone();
            self.select(&obj);
        }
    }

    /// GetStockObject equivalents (EMF stock handles have the high bit set).
    pub fn stock(index: u32) -> Option<Object> {
        let gray = |v: u8| Object::Brush(Brush::Solid(Rgb { r: v, g: v, b: v }));
        Some(match index {
            0 | 18 => gray(255),
            1 => gray(0xC0),
            2 => gray(0x80),
            3 => gray(0x40),
            4 => gray(0),
            5 => Object::Brush(Brush::Null),
            6 => Object::Pen(Pen::solid(Rgb::WHITE)),
            7 | 19 => Object::Pen(Pen::solid(Rgb::BLACK)),
            8 => Object::Pen(Pen::null()),
            10 | 11 | 16 => Object::Font(Font {
                face: "Courier New".into(),
                height: 12,
                ..Font::default()
            }),
            12..=14 | 17 => Object::Font(Font {
                height: 12,
                ..Font::default()
            }),
            _ => return None,
        })
    }

    // ---- path bracket --------------------------------------------------------

    pub fn begin_path(&mut self) {
        self.bracket = Some(Bracket {
            pb: PathBuilder::new(),
            recording: true,
            figure_open: false,
        });
    }

    pub fn end_path(&mut self) {
        if let Some(b) = &mut self.bracket {
            b.recording = false;
        }
    }

    pub fn abort_path(&mut self) {
        self.bracket = None;
    }

    pub fn close_figure(&mut self) {
        if let Some(b) = self.recording() {
            if b.figure_open {
                b.pb.close();
            }
            b.figure_open = false;
        }
    }

    fn recording(&mut self) -> Option<&mut Bracket> {
        self.bracket.as_mut().filter(|b| b.recording)
    }

    fn take_path(&mut self) -> Option<Path> {
        self.bracket.take().and_then(|b| b.pb.finish())
    }

    pub fn fill_path(&mut self) {
        if let Some(p) = self.take_path() {
            self.draw(&p, true, false);
        }
    }

    pub fn stroke_path(&mut self) {
        if let Some(p) = self.take_path() {
            self.draw(&p, false, true);
        }
    }

    pub fn stroke_and_fill_path(&mut self) {
        if let Some(p) = self.take_path() {
            self.draw(&p, true, true);
        }
    }

    pub fn select_clip_path(&mut self, mode: u32) {
        if let Some(p) = self.take_path() {
            let rule = if self.dc.winding {
                FillRule::Winding
            } else {
                FillRule::EvenOdd
            };
            self.combine_clip(Some(&p), rule, mode);
        }
    }

    // ---- shapes ----------------------------------------------------------------

    /// Records or draws a finished shape. Inside a path bracket it is appended
    /// as new figures; otherwise it is drawn (filled only when `closed`).
    fn emit(&mut self, pb: PathBuilder, closed: bool) {
        let Some(path) = pb.finish() else { return };
        if let Some(b) = self.recording() {
            b.pb.push_path(&path);
            b.figure_open = !closed;
            return;
        }
        self.draw(&path, closed, true);
    }

    pub fn move_to(&mut self, x: f64, y: f64) {
        self.dc.cur = (x, y);
        if let Some(b) = self.recording() {
            b.figure_open = false;
        }
    }

    pub fn line_to(&mut self, x: f64, y: f64) {
        let (cx, cy) = self.dc.cur;
        self.polyline_to(&[(x, y)], &[cx, cy]);
    }

    /// Connected segments from the current position (LINETO, POLYLINETO);
    /// `from` is the starting logical point. Updates the current position.
    pub fn polyline_to(&mut self, pts: &[(f64, f64)], from: &[f64; 2]) {
        let Some(&last) = pts.last() else { return };
        if !self.spend(pts.len()) {
            return;
        }
        let t = self.t();
        self.dc.cur = last;
        let start = map(&t, from[0], from[1]);
        if let Some(b) = self.recording() {
            if !b.figure_open {
                b.pb.move_to(start.0, start.1);
                b.figure_open = true;
            }
            for &(x, y) in pts {
                let p = map(&t, x, y);
                b.pb.line_to(p.0, p.1);
            }
            return;
        }
        let mut pb = PathBuilder::new();
        pb.move_to(start.0, start.1);
        for &(x, y) in pts {
            let p = map(&t, x, y);
            pb.line_to(p.0, p.1);
        }
        if let Some(path) = pb.finish() {
            self.draw(&path, false, true);
        }
    }

    /// Cubic Béziers continuing from the current position (POLYBEZIERTO).
    pub fn bezier_to(&mut self, pts: &[(f64, f64)]) {
        if pts.len() < 3 || !self.spend(pts.len()) {
            return;
        }
        let t = self.t();
        let (cx, cy) = self.dc.cur;
        let start = map(&t, cx, cy);
        let whole = pts.len() / 3 * 3;
        self.dc.cur = pts[whole - 1];
        if let Some(b) = self.recording() {
            if !b.figure_open {
                b.pb.move_to(start.0, start.1);
                b.figure_open = true;
            }
            for c in pts.as_chunks::<3>().0 {
                let (p1, p2, e) = (
                    map(&t, c[0].0, c[0].1),
                    map(&t, c[1].0, c[1].1),
                    map(&t, c[2].0, c[2].1),
                );
                b.pb.cubic_to(p1.0, p1.1, p2.0, p2.1, e.0, e.1);
            }
            return;
        }
        let mut pb = PathBuilder::new();
        pb.move_to(start.0, start.1);
        for c in pts.as_chunks::<3>().0 {
            let (p1, p2, e) = (
                map(&t, c[0].0, c[0].1),
                map(&t, c[1].0, c[1].1),
                map(&t, c[2].0, c[2].1),
            );
            pb.cubic_to(p1.0, p1.1, p2.0, p2.1, e.0, e.1);
        }
        if let Some(path) = pb.finish() {
            self.draw(&path, false, true);
        }
    }

    /// POLYBEZIER: first point, then (control, control, end) triples. Open.
    pub fn poly_bezier(&mut self, pts: &[(f64, f64)]) {
        if pts.len() < 4 || !self.spend(pts.len()) {
            return;
        }
        let t = self.t();
        let mut pb = PathBuilder::new();
        let s = map(&t, pts[0].0, pts[0].1);
        pb.move_to(s.0, s.1);
        for c in pts[1..].as_chunks::<3>().0 {
            let (p1, p2, e) = (
                map(&t, c[0].0, c[0].1),
                map(&t, c[1].0, c[1].1),
                map(&t, c[2].0, c[2].1),
            );
            pb.cubic_to(p1.0, p1.1, p2.0, p2.1, e.0, e.1);
        }
        self.emit(pb, false);
    }

    /// One or more polylines/polygons drawn as a single path.
    pub fn poly(&mut self, polys: &[&[(f64, f64)]], closed: bool) {
        let n: usize = polys.iter().map(|p| p.len()).sum();
        if n == 0 || !self.spend(n) {
            return;
        }
        let t = self.t();
        let mut pb = PathBuilder::new();
        for pts in polys {
            if pts.len() < 2 {
                continue;
            }
            for (i, &(x, y)) in pts.iter().enumerate() {
                let p = map(&t, x, y);
                if i == 0 {
                    pb.move_to(p.0, p.1);
                } else {
                    pb.line_to(p.0, p.1);
                }
            }
            if closed {
                pb.close();
            }
        }
        self.emit(pb, closed);
    }

    pub fn rectangle(&mut self, l: f64, t_: f64, r: f64, b: f64) {
        let pts = [(l, t_), (r, t_), (r, b), (l, b)];
        self.poly(&[&pts], true);
    }

    /// ROUNDRECT: `w`/`h` are the corner ellipse's full width and height.
    pub fn round_rect(&mut self, l: f64, t_: f64, r: f64, b: f64, w: f64, h: f64) {
        let (l, r) = (l.min(r), l.max(r));
        let (t_, b) = (t_.min(b), t_.max(b));
        let rx = (w.abs() / 2.0).min((r - l) / 2.0);
        let ry = (h.abs() / 2.0).min((b - t_) / 2.0);
        if rx <= 0.0 || ry <= 0.0 {
            return self.rectangle(l, t_, r, b);
        }
        let t = self.t();
        let mut pb = PathBuilder::new();
        let s = map(&t, l + rx, t_);
        pb.move_to(s.0, s.1);
        use std::f64::consts::{FRAC_PI_2, PI};
        let corners = [
            (r - rx, t_ + ry, -FRAC_PI_2),
            (r - rx, b - ry, 0.0),
            (l + rx, b - ry, FRAC_PI_2),
            (l + rx, t_ + ry, PI),
        ];
        for (cx, cy, a0) in corners {
            let p = (cx + rx * a0.cos(), cy + ry * a0.sin());
            let m = map(&t, p.0, p.1);
            pb.line_to(m.0, m.1);
            arc_into(&mut pb, &t, (cx, cy), (rx, ry), a0, FRAC_PI_2);
        }
        pb.close();
        self.emit(pb, true);
    }

    pub fn ellipse(&mut self, l: f64, t_: f64, r: f64, b: f64) {
        let (cx, cy) = ((l + r) / 2.0, (t_ + b) / 2.0);
        let (rx, ry) = ((r - l).abs() / 2.0, (b - t_).abs() / 2.0);
        if rx <= 0.0 || ry <= 0.0 {
            return;
        }
        let t = self.t();
        let mut pb = PathBuilder::new();
        let s = map(&t, cx + rx, cy);
        pb.move_to(s.0, s.1);
        arc_into(&mut pb, &t, (cx, cy), (rx, ry), 0.0, std::f64::consts::TAU);
        pb.close();
        self.emit(pb, true);
    }

    /// ARC / CHORD / PIE / ARCTO over the ellipse in box (l,t,r,b), from the
    /// radial through (xs,ys) to the radial through (xe,ye), in the DC's arc
    /// direction as seen on the device.
    #[expect(
        clippy::too_many_arguments,
        reason = "mirrors the EMF/WMF arc records: bounding box plus start and end radials"
    )]
    pub fn arc(
        &mut self,
        kind: ArcKind,
        l: f64,
        t_: f64,
        r: f64,
        b: f64,
        xs: f64,
        ys: f64,
        xe: f64,
        ye: f64,
    ) {
        let (cx, cy) = ((l + r) / 2.0, (t_ + b) / 2.0);
        let (rx, ry) = ((r - l).abs() / 2.0, (b - t_).abs() / 2.0);
        if rx <= 0.0 || ry <= 0.0 {
            return;
        }
        let t = self.t();
        let a0 = ((ys - cy) / ry).atan2((xs - cx) / rx);
        let a1 = ((ye - cy) / ry).atan2((xe - cx) / rx);
        // Parameter angles grow clockwise on a y-down device; a mirroring
        // transform reverses that.
        let ccw_is_negative = det(&t) > 0.0;
        let negative = ccw_is_negative != self.dc.arc_clockwise;
        let tau = std::f64::consts::TAU;
        let mut sweep = if negative {
            (a1 - a0).rem_euclid(tau) - tau
        } else {
            (a1 - a0).rem_euclid(tau)
        };
        if sweep.abs() < 1e-9 {
            sweep = if negative { -tau } else { tau };
        }
        let start = (cx + rx * a0.cos(), cy + ry * a0.sin());
        let end = (cx + rx * (a0 + sweep).cos(), cy + ry * (a0 + sweep).sin());
        if kind == ArcKind::ArcTo {
            let from = [self.dc.cur.0, self.dc.cur.1];
            self.polyline_to(&[start], &from);
            let t2 = self.t();
            if let Some(b) = self.recording() {
                arc_into(&mut b.pb, &t2, (cx, cy), (rx, ry), a0, sweep);
            } else {
                let mut pb = PathBuilder::new();
                let s = map(&t2, start.0, start.1);
                pb.move_to(s.0, s.1);
                arc_into(&mut pb, &t2, (cx, cy), (rx, ry), a0, sweep);
                if let Some(p) = pb.finish() {
                    self.draw(&p, false, true);
                }
            }
            self.dc.cur = end;
            return;
        }
        let mut pb = PathBuilder::new();
        if kind == ArcKind::Pie {
            let c = map(&t, cx, cy);
            pb.move_to(c.0, c.1);
            let s = map(&t, start.0, start.1);
            pb.line_to(s.0, s.1);
        } else {
            let s = map(&t, start.0, start.1);
            pb.move_to(s.0, s.1);
        }
        arc_into(&mut pb, &t, (cx, cy), (rx, ry), a0, sweep);
        let closed = kind != ArcKind::Arc;
        if closed {
            pb.close();
        }
        self.emit(pb, closed);
    }

    /// ANGLEARC: line from the current position to the arc start, then a circular
    /// arc; angles in degrees, counterclockwise.
    pub fn angle_arc(&mut self, cx: f64, cy: f64, radius: f64, start_deg: f64, sweep_deg: f64) {
        let (a0, sw) = (-start_deg.to_radians(), -sweep_deg.to_radians());
        let start = (cx + radius * a0.cos(), cy + radius * a0.sin());
        let from = [self.dc.cur.0, self.dc.cur.1];
        self.polyline_to(&[start], &from);
        let t = self.t();
        if let Some(b) = self.recording() {
            arc_into(&mut b.pb, &t, (cx, cy), (radius, radius), a0, sw);
        } else {
            let mut pb = PathBuilder::new();
            let s = map(&t, start.0, start.1);
            pb.move_to(s.0, s.1);
            arc_into(&mut pb, &t, (cx, cy), (radius, radius), a0, sw);
            if let Some(p) = pb.finish() {
                self.draw(&p, false, true);
            }
        }
        self.dc.cur = (cx + radius * (a0 + sw).cos(), cy + radius * (a0 + sw).sin());
    }

    /// Fills a logical rectangle with a brush, ignoring the pen (PATBLT, region fills).
    pub fn fill_rect_with(&mut self, l: f64, t_: f64, r: f64, b: f64, brush: Brush) {
        let t = self.t();
        let mut pb = PathBuilder::new();
        for (i, (x, y)) in [(l, t_), (r, t_), (r, b), (l, b)].into_iter().enumerate() {
            let p = map(&t, x, y);
            if i == 0 {
                pb.move_to(p.0, p.1);
            } else {
                pb.line_to(p.0, p.1);
            }
        }
        pb.close();
        let Some(path) = pb.finish() else { return };
        let saved = std::mem::replace(&mut self.dc.brush, brush);
        self.draw(&path, true, false);
        self.dc.brush = saved;
    }

    /// GRADIENTFILL rectangle: a linear gradient from `c0` at the left (or top)
    /// edge to `c1` at the right (or bottom) edge of the logical rectangle.
    pub fn gradient_rect(
        &mut self,
        (l, t_, r, b): (f64, f64, f64, f64),
        c0: Rgb,
        c1: Rgb,
        vertical: bool,
    ) {
        let t = self.t();
        let mut pb = PathBuilder::new();
        for (i, (x, y)) in [(l, t_), (r, t_), (r, b), (l, b)].into_iter().enumerate() {
            let p = map(&t, x, y);
            if i == 0 {
                pb.move_to(p.0, p.1);
            } else {
                pb.line_to(p.0, p.1);
            }
        }
        pb.close();
        let Some(path) = pb.finish() else { return };
        let (p0, p1) = (
            map(&t, l, t_),
            if vertical {
                map(&t, l, b)
            } else {
                map(&t, r, t_)
            },
        );
        let stops = vec![
            tiny_skia::GradientStop::new(0.0, c0.to_color()),
            tiny_skia::GradientStop::new(1.0, c1.to_color()),
        ];
        let shader = tiny_skia::LinearGradient::new(
            Point::from_xy(p0.0, p0.1),
            Point::from_xy(p1.0, p1.1),
            stops,
            SpreadMode::Pad,
            Transform::identity(),
        )
        .unwrap_or(Shader::SolidColor(c0.to_color()));
        // GDI fills gradients without anti-aliasing (bands abut seamlessly).
        let paint = Paint {
            shader,
            anti_alias: false,
            ..Default::default()
        };
        let mask = self.clip_mask();
        self.pm.fill_path(
            &path,
            &paint,
            FillRule::Winding,
            Transform::identity(),
            mask.as_deref(),
        );
    }

    /// GRADIENTFILL triangle: colours interpolated between the vertices
    /// (Gouraud). GDI does not anti-alias these: a pixel is painted when its centre
    /// is inside, so abutting triangles and sub-pixel bands leave no seams.
    pub fn gradient_triangle(&mut self, pts: [(f64, f64); 3], cols: [Rgb; 3]) {
        let t = self.t();
        let d = pts.map(|(x, y)| {
            let p = map(&t, x, y);
            (p.0 as f64, p.1 as f64)
        });
        let area = (d[1].0 - d[0].0) * (d[2].1 - d[0].1) - (d[2].0 - d[0].0) * (d[1].1 - d[0].1);
        if area.abs() < 1e-9 || !area.is_finite() {
            return;
        }
        let (w, h) = (self.pm.width() as i64, self.pm.height() as i64);
        let (xs, ys) = (d.map(|p| p.0), d.map(|p| p.1));
        let x0 = (xs.iter().copied().fold(f64::MAX, f64::min).floor() as i64).clamp(0, w);
        let x1 = (xs.iter().copied().fold(f64::MIN, f64::max).ceil() as i64).clamp(0, w);
        let y0 = (ys.iter().copied().fold(f64::MAX, f64::min).floor() as i64).clamp(0, h);
        let y1 = (ys.iter().copied().fold(f64::MIN, f64::max).ceil() as i64).clamp(0, h);
        let mask = self.clip_mask();
        let width = w as usize;
        let data = self.pm.data_mut();
        // Barycentric weight of vertex `i` at (x, y): edge function of the opposite edge.
        let edge = |i: usize, x: f64, y: f64| {
            let (a, b) = (d[(i + 1) % 3], d[(i + 2) % 3]);
            ((b.0 - a.0) * (y - a.1) - (b.1 - a.1) * (x - a.0)) / area
        };
        for py in y0..y1 {
            for px in x0..x1 {
                let (cx, cy) = (px as f64 + 0.5, py as f64 + 0.5);
                let wts = [edge(0, cx, cy), edge(1, cx, cy), edge(2, cx, cy)];
                if wts.iter().any(|v| *v < -1e-9) {
                    continue;
                }
                let k = py as usize * width + px as usize;
                let cov = mask.as_ref().map_or(255u32, |m| m.data()[k] as u32);
                if cov == 0 {
                    continue;
                }
                let ch = |f: fn(&Rgb) -> u8| {
                    (wts[0] * f(&cols[0]) as f64
                        + wts[1] * f(&cols[1]) as f64
                        + wts[2] * f(&cols[2]) as f64)
                        .round()
                        .clamp(0.0, 255.0) as u32
                };
                let src = [ch(|c| c.r), ch(|c| c.g), ch(|c| c.b), 255];
                let px = &mut data[k * 4..k * 4 + 4];
                for (dst, s) in px.iter_mut().zip(src) {
                    *dst = ((s * cov + *dst as u32 * (255 - cov) + 127) / 255) as u8;
                }
            }
        }
    }

    /// Fills device-unit rectangles (EMF region data) with a brush.
    pub fn fill_device_rects(&mut self, rects: &[(f64, f64, f64, f64)], brush: Brush) {
        let Some(path) = self.device_rects_path(rects) else {
            return;
        };
        let saved = std::mem::replace(&mut self.dc.brush, brush);
        let winding = std::mem::replace(&mut self.dc.winding, true);
        self.draw(&path, true, false);
        self.dc.brush = saved;
        self.dc.winding = winding;
    }

    // ---- painting ----------------------------------------------------------------

    /// Fills (brush) and/or strokes (pen) a pixel-space path with the current DC.
    pub fn draw(&mut self, path: &Path, fill: bool, stroke: bool) {
        if self.dc.rop2 == 11 {
            return; // R2_NOP
        }
        let mask = self.clip_mask();
        let mask = mask.as_deref();
        if fill {
            let brush = self.dc.brush.clone();
            let hatch;
            let paint = match &brush {
                Brush::Null => None,
                Brush::Solid(c) => Some(solid(self.rop_color(*c))),
                Brush::Pattern(pm) => Some(Paint {
                    shader: Pattern::new(
                        (**pm).as_ref(),
                        SpreadMode::Repeat,
                        FilterQuality::Nearest,
                        1.0,
                        Transform::from_scale(self.dev_px, self.dev_px),
                    ),
                    anti_alias: true,
                    ..Default::default()
                }),
                Brush::Hatched { color, style } => {
                    hatch = hatch_pixmap(
                        *style,
                        *color,
                        self.dc.bk_opaque.then_some(self.dc.bk_color),
                    );
                    hatch.as_ref().map(|h| Paint {
                        shader: Pattern::new(
                            h.as_ref(),
                            SpreadMode::Repeat,
                            FilterQuality::Nearest,
                            1.0,
                            Transform::from_scale(self.dev_px, self.dev_px),
                        ),
                        anti_alias: true,
                        ..Default::default()
                    })
                }
            };
            if let Some(mut paint) = paint {
                paint.anti_alias &= anti_alias_fits(path, Transform::identity(), 0.0);
                let rule = if self.dc.winding {
                    FillRule::Winding
                } else {
                    FillRule::EvenOdd
                };
                self.pm
                    .fill_path(path, &paint, rule, Transform::identity(), mask);
            }
        }
        if stroke && let Some((mut paint, st)) = self.pen_stroke() {
            paint.anti_alias &= anti_alias_fits(path, Transform::identity(), stroke_outset(&st));
            self.pm
                .stroke_path(path, &paint, &st, Transform::identity(), mask);
        }
    }

    /// Applies the binary raster operations that force a color.
    fn rop_color(&self, c: Rgb) -> Rgb {
        match self.dc.rop2 {
            1 => Rgb::BLACK,
            16 => Rgb::WHITE,
            _ => c,
        }
    }

    /// Paint and stroke for the current pen, in pixmap pixels.
    fn pen_stroke(&self) -> Option<(Paint<'static>, Stroke)> {
        let pen = &self.dc.pen;
        if pen.is_null() {
            return None;
        }
        let t = self.t();
        let scale = det(&t).abs().sqrt();
        let geometric = !pen.cosmetic && pen.width > 0.0;
        let width = if geometric {
            (pen.width * scale).max(self.dev_px)
        } else {
            self.dev_px
        };
        let width = width.max(0.75);
        let mut st = Stroke {
            width,
            miter_limit: self.dc.miter_limit.max(1.0),
            ..Default::default()
        };
        let (cap, join) = if geometric {
            let cap = match pen.style & 0xF00 {
                0x100 => LineCap::Square,
                0x200 => LineCap::Butt,
                _ => LineCap::Round,
            };
            let join = match pen.style & 0xF000 {
                0x1000 => LineJoin::Bevel,
                0x2000 => LineJoin::Miter,
                _ => LineJoin::Round,
            };
            (cap, join)
        } else {
            (LineCap::Round, LineJoin::Round)
        };
        st.line_cap = cap;
        st.line_join = join;
        let unit = if geometric { width } else { self.dev_px * 3.0 };
        let pattern: Vec<f32> = match pen.style & PS_STYLE_MASK {
            1 => vec![3.0, 1.0],
            2 => vec![1.0, 1.0],
            3 => vec![3.0, 1.0, 1.0, 1.0],
            4 => vec![3.0, 1.0, 1.0, 1.0, 1.0, 1.0],
            PS_USERSTYLE if !pen.user_dashes.is_empty() => {
                let s = if geometric { scale } else { self.dev_px };
                let mut v: Vec<f32> = pen
                    .user_dashes
                    .iter()
                    .map(|d| (d * s / unit).max(0.01))
                    .collect();
                if v.len() % 2 == 1 {
                    v.extend_from_within(..);
                }
                v
            }
            _ => Vec::new(),
        };
        if !pattern.is_empty() {
            st.line_cap = LineCap::Butt;
            st.dash = StrokeDash::new(pattern.iter().map(|v| v * unit).collect(), 0.0);
        }
        Some((solid(self.rop_color(pen.color)), st))
    }

    // ---- clipping ----------------------------------------------------------------

    fn full_rect(&self) -> tiny_skia::Rect {
        tiny_skia::Rect::from_xywh(0.0, 0.0, self.pm.width() as f32, self.pm.height() as f32)
            .unwrap()
    }

    /// The clip as a mask, built lazily for rectangles and cached.
    pub(super) fn clip_mask(&mut self) -> Option<Rc<Mask>> {
        match &self.dc.clip {
            Clip::None => None,
            Clip::Mask(m) => Some(m.clone()),
            Clip::Rect(r) => {
                if let Some((cr, m)) = &self.rect_mask
                    && cr == r
                {
                    return Some(m.clone());
                }
                let m = Rc::new(self.rect_to_mask(*r));
                self.rect_mask = Some((*r, m.clone()));
                Some(m)
            }
        }
    }

    fn rect_to_mask(&self, r: tiny_skia::Rect) -> Mask {
        let mut m = Mask::new(self.pm.width(), self.pm.height()).expect("pixmap-sized mask");
        if r.width() > 0.0 && r.height() > 0.0 {
            let path = PathBuilder::from_rect(r);
            let aa = anti_alias_fits(&path, Transform::identity(), 0.0);
            m.fill_path(&path, FillRule::Winding, aa, Transform::identity());
        }
        m
    }

    fn current_mask(&self) -> Mask {
        match &self.dc.clip {
            Clip::None => {
                let mut m =
                    Mask::new(self.pm.width(), self.pm.height()).expect("pixmap-sized mask");
                m.data_mut().fill(255);
                m
            }
            Clip::Rect(r) => self.rect_to_mask(*r),
            Clip::Mask(m) => (**m).clone(),
        }
    }

    fn set_clip_rect(&mut self, r: tiny_skia::Rect) {
        let full = self.full_rect();
        self.dc.clip = if r.left() <= 0.0
            && r.top() <= 0.0
            && r.right() >= full.right()
            && r.bottom() >= full.bottom()
        {
            Clip::None
        } else {
            Clip::Rect(r)
        };
    }

    /// Combines a pixel-space region with the clip (RGN_* mode). `None`
    /// region with RGN_COPY resets the clip to the whole picture.
    pub fn combine_clip(&mut self, region: Option<&Path>, rule: FillRule, mode: u32) {
        let Some(region) = region else {
            if mode == RGN_COPY {
                self.dc.clip = Clip::None;
            }
            return;
        };
        let (w, h) = (self.pm.width(), self.pm.height());
        if let Some(r) = axis_rect(region) {
            match (&self.dc.clip, mode) {
                (_, RGN_COPY) | (Clip::None, RGN_AND) => return self.set_clip_rect(r),
                (Clip::Rect(cur), RGN_AND) => {
                    let i = intersect(*cur, r);
                    return self.set_clip_rect(i);
                }
                _ => {}
            }
        }
        let mut sub = Mask::new(w, h).expect("pixmap-sized mask");
        let aa = anti_alias_fits(region, Transform::identity(), 0.0);
        sub.fill_path(region, rule, aa, Transform::identity());
        let result = if mode == RGN_COPY {
            sub
        } else {
            let mut cur = self.current_mask();
            for (a, b) in cur.data_mut().iter_mut().zip(sub.data()) {
                let (x, y) = (*a as u32, *b as u32);
                *a = match mode {
                    RGN_AND => (x * y + 127) / 255,
                    RGN_OR => x.max(y),
                    RGN_XOR => x.max(y) - x.min(y),
                    RGN_DIFF => (x * (255 - y) + 127) / 255,
                    _ => x,
                } as u8;
            }
            cur
        };
        self.dc.clip = Clip::Mask(Rc::new(result));
    }

    /// INTERSECTCLIPRECT / EXCLUDECLIPRECT with a logical rectangle.
    pub fn clip_logical_rect(&mut self, l: f64, t_: f64, r: f64, b: f64, exclude: bool) {
        let t = self.t();
        let mut pb = PathBuilder::new();
        for (i, (x, y)) in [(l, t_), (r, t_), (r, b), (l, b)].into_iter().enumerate() {
            let p = map(&t, x, y);
            if !finite(p) {
                return;
            }
            if i == 0 {
                pb.move_to(p.0, p.1);
            } else {
                pb.line_to(p.0, p.1);
            }
        }
        pb.close();
        match pb.finish() {
            Some(p) => self.combine_clip(
                Some(&p),
                FillRule::Winding,
                if exclude { RGN_DIFF } else { RGN_AND },
            ),
            // Degenerate rectangle: intersecting leaves nothing visible.
            None if !exclude => {
                self.dc.clip = Clip::Rect(tiny_skia::Rect::from_xywh(0.0, 0.0, 0.0, 0.0).unwrap())
            }
            None => {}
        }
    }

    /// Pixel-space path of device-unit rectangles (EMF region data).
    fn device_rects_path(&self, rects: &[(f64, f64, f64, f64)]) -> Option<Path> {
        let t = match self.space {
            Space::Device { dev_to_px, .. } => dev_to_px,
            Space::Window { .. } => self.t(),
        };
        let mut pb = PathBuilder::new();
        for &(l, tp, r, b) in rects {
            let (a, c) = (map(&t, l, tp), map(&t, r, b));
            if !finite(a) || !finite(c) {
                continue;
            }
            if let Some(rect) =
                tiny_skia::Rect::from_ltrb(a.0.min(c.0), a.1.min(c.1), a.0.max(c.0), a.1.max(c.1))
            {
                pb.push_rect(rect);
            }
        }
        pb.finish()
    }

    /// EXTSELECTCLIPRGN: device-unit rectangles combined with the clip.
    pub fn clip_device_rects(&mut self, rects: &[(f64, f64, f64, f64)], mode: u32) {
        if rects.is_empty() {
            if mode == RGN_COPY {
                self.dc.clip = Clip::None;
            }
            return;
        }
        match self.device_rects_path(rects) {
            Some(p) => self.combine_clip(Some(&p), FillRule::Winding, mode),
            None if mode == RGN_COPY || mode == RGN_AND => {
                self.dc.clip = Clip::Rect(tiny_skia::Rect::from_xywh(0.0, 0.0, 0.0, 0.0).unwrap())
            }
            None => {}
        }
    }

    // ---- bitmaps ----------------------------------------------------------------

    /// Draws `src` (x, y, w, h in top-down bitmap pixels) of `img` into the
    /// logical destination rectangle (x, y, w, h; negative extents mirror),
    /// applying the raster operation `rop` approximately (see [`rop_source`]).
    pub fn draw_bitmap(
        &mut self,
        img: &Pixmap,
        src: (f64, f64, f64, f64),
        dst: (f64, f64, f64, f64),
        rop: u32,
    ) {
        let (sx, sy, sw, sh) = src;
        let (dx, dy, dw, dh) = dst;
        if sw <= 0.0 || sh <= 0.0 || dw == 0.0 || dh == 0.0 || !self.spend(1) {
            return;
        }
        // Crop to the source rectangle (clamped to the bitmap).
        let x0 = sx.max(0.0).floor() as i64;
        let y0 = sy.max(0.0).floor() as i64;
        let x1 = ((sx + sw).ceil() as i64).min(img.width() as i64);
        let y1 = ((sy + sh).ceil() as i64).min(img.height() as i64);
        if x1 <= x0 || y1 <= y0 {
            return;
        }
        let full = x0 == 0 && y0 == 0 && x1 == img.width() as i64 && y1 == img.height() as i64;
        let cropped = if full {
            None
        } else {
            let Some(r) =
                IntRect::from_xywh(x0 as i32, y0 as i32, (x1 - x0) as u32, (y1 - y0) as u32)
            else {
                return;
            };
            let Some(c) = img.clone_rect(r) else { return };
            Some(c)
        };
        let base = cropped.as_ref().unwrap_or(img);
        let Some(processed) = rop_source(base, rop, &self.dc) else {
            return;
        };
        let pm_ref = processed.as_ref().unwrap_or(base).as_ref();
        let t = self.t();
        // Bitmap pixel (x0,y0) lands on (dx,dy) scaled by dw/sw, dh/sh.
        let (kx, ky) = (dw / sw, dh / sh);
        let place = Transform::from_row(
            kx as f32,
            0.0,
            0.0,
            ky as f32,
            (dx + (x0 as f64 - sx) * kx) as f32,
            (dy + (y0 as f64 - sy) * ky) as f32,
        );
        let full_t = t.pre_concat(place);
        let px_scale = det(&full_t).abs().sqrt();
        let quality = if pm_ref.width().min(pm_ref.height()) <= 4 || px_scale > 6.0 {
            FilterQuality::Nearest
        } else if px_scale < 0.9 {
            FilterQuality::Bicubic
        } else {
            FilterQuality::Bilinear
        };
        let paint = PixmapPaint {
            quality,
            ..Default::default()
        };
        let mask = self.clip_mask();
        self.pm
            .draw_pixmap(0, 0, pm_ref, &paint, full_t, mask.as_deref());
    }
}

/// Appends an elliptical arc (logical center/radii, parameter angles in
/// radians) to a pixel-space path as cubic Béziers of at most 90 degrees.
pub fn arc_into(
    pb: &mut PathBuilder,
    t: &Transform,
    c: (f64, f64),
    r: (f64, f64),
    a0: f64,
    sweep: f64,
) {
    let n = (sweep.abs() / std::f64::consts::FRAC_PI_2)
        .ceil()
        .clamp(1.0, 16.0) as usize;
    let step = sweep / n as f64;
    let k = 4.0 / 3.0 * (step / 4.0).tan();
    let pt = |a: f64| (c.0 + r.0 * a.cos(), c.1 + r.1 * a.sin());
    let d = |a: f64| (-r.0 * a.sin(), r.1 * a.cos());
    for i in 0..n {
        let (s, e) = (a0 + step * i as f64, a0 + step * (i + 1) as f64);
        let (p0, p3) = (pt(s), pt(e));
        let (d0, d3) = (d(s), d(e));
        let c1 = map(t, p0.0 + k * d0.0, p0.1 + k * d0.1);
        let c2 = map(t, p3.0 - k * d3.0, p3.1 - k * d3.1);
        let e = map(t, p3.0, p3.1);
        pb.cubic_to(c1.0, c1.1, c2.0, c2.1, e.0, e.1);
    }
}

/// The rectangle a path describes, if it is a single axis-aligned rectangle.
fn axis_rect(path: &Path) -> Option<tiny_skia::Rect> {
    let pts = path.points();
    if pts.len() < 4 || pts.len() > 5 {
        return None;
    }
    let b = path.bounds();
    let on_edge = |p: &Point| {
        let ex = (p.x - b.left()).abs() < 1e-3 || (p.x - b.right()).abs() < 1e-3;
        let ey = (p.y - b.top()).abs() < 1e-3 || (p.y - b.bottom()).abs() < 1e-3;
        ex && ey
    };
    let segs = path
        .segments()
        .filter(|s| {
            matches!(
                s,
                tiny_skia::PathSegment::CubicTo(..) | tiny_skia::PathSegment::QuadTo(..)
            )
        })
        .count();
    (segs == 0 && pts.iter().all(on_edge)).then_some(b)
}

fn intersect(a: tiny_skia::Rect, b: tiny_skia::Rect) -> tiny_skia::Rect {
    let l = a.left().max(b.left());
    let t = a.top().max(b.top());
    let r = a.right().min(b.right()).max(l);
    let bt = a.bottom().min(b.bottom()).max(t);
    tiny_skia::Rect::from_ltrb(l, t, r, bt).unwrap_or(a)
}

/// 8x8 hatch tile (HS_* style) in `color` over `bk` (transparent when None).
fn hatch_pixmap(style: u32, color: Rgb, bk: Option<Rgb>) -> Option<Pixmap> {
    let mut pm = Pixmap::new(8, 8)?;
    if let Some(bk) = bk {
        pm.fill(bk.to_color());
    }
    let c = tiny_skia::ColorU8::from_rgba(color.r, color.g, color.b, 255).premultiply();
    for y in 0..8usize {
        for x in 0..8usize {
            let on = match style {
                0 => y == 0,
                1 => x == 0,
                2 => x == y,
                3 => x + y == 7,
                4 => x == 0 || y == 0,
                5 => x == y || x + y == 7,
                _ => false,
            };
            if on {
                pm.pixels_mut()[y * 8 + x] = c;
            }
        }
    }
    Some(pm)
}

/// Approximates a ternary raster operation on a transparent canvas by turning
/// the source into an image to composite. Returns `None` to draw nothing,
/// `Some(None)` to draw the source as is, `Some(Some(pm))` to draw `pm`.
///
/// The classic transparency idiom draws a mask with SRCAND (white = keep the
/// background) and then the image with SRCPAINT/SRCINVERT (black = keep), so
/// those make white respectively black transparent.
fn rop_source(src: &Pixmap, rop: u32, dc: &Dc) -> Option<Option<Pixmap>> {
    let code = (rop >> 16) & 0xFF;
    let recolor = |f: &dyn Fn(u8, u8, u8, u8) -> [u8; 4]| {
        let mut out = src.clone();
        for px in out.data_mut().as_chunks_mut::<4>().0 {
            let v = f(px[0], px[1], px[2], px[3]);
            px.copy_from_slice(&v);
        }
        Some(Some(out))
    };
    let brush_color = match &dc.brush {
        Brush::Solid(c) => Some(*c),
        Brush::Hatched { color, .. } => Some(*color),
        _ => None,
    };
    match code {
        0xCC | 0xC0 => Some(None),
        0x33 => recolor(&|r, g, b, a| [a - r.min(a), a - g.min(a), a - b.min(a), a]),
        0x88 => recolor(&|r, g, b, a| {
            if r >= 250 && g >= 250 && b >= 250 {
                [0; 4]
            } else {
                [r, g, b, a]
            }
        }),
        0xEE | 0x66 => recolor(&|r, g, b, a| {
            if r <= 5 && g <= 5 && b <= 5 {
                [0; 4]
            } else {
                [r, g, b, a]
            }
        }),
        // Monochrome-mask brush painting: brush where the source is black (B8)
        // or white (E2), background elsewhere.
        0xB8 | 0xE2 => {
            let c = brush_color?;
            let want_dark = code == 0xB8;
            recolor(&|r, g, b, _| {
                let dark = (r as u32 + g as u32 + b as u32) < 384;
                if dark == want_dark {
                    [c.r, c.g, c.b, 255]
                } else {
                    [0; 4]
                }
            })
        }
        // Operations that ignore the source are handled by the caller.
        _ if (code >> 2) & 0x33 == code & 0x33 => None,
        _ => Some(None),
    }
}
