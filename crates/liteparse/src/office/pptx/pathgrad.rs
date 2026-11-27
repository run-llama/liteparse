//! Path gradients (`a:gradFill` with `a:path path="circle|rect|shape"`), matched to
//! PowerPoint's renders.
//!
//! Gradient space is the shape's own box in points. `fillToRect` gives the focus
//! rectangle as insets from the box sides; a side pair that crosses (left inset past
//! the right edge) collapses onto its first inset, so `l=50% r=100%` focuses at 50 %.
//! `tileRect` gives the outer rectangle (the box itself when absent).
//!
//! Each contour `s` (0 = focus, 1 = outer) is a shape whose centre moves linearly from
//! the focus centre to the outer centre and whose half sizes grow linearly from the
//! focus half sizes to the outer extent: for `circle`, an ellipse reaching a circle of
//! the outer rectangle's half diagonal (so every outer corner gets the last stop); for
//! `rect` and `shape`, a rectangle reaching the outer rectangle. A point is coloured
//! by the smallest contour containing it. With a point focus at the centre this is a
//! plain radial gradient; off-centre it is a two-point (focal) gradient.
//!
//! A gradient of exactly two stops at 0 % and 100 % is drawn against `s²` instead of
//! `s` (PowerPoint 2010 and later; measured on its slide thumbnails).

use tiny_skia::{
    FilterQuality, Pattern, Pixmap, PremultipliedColorU8, Shader, SpreadMode, Transform,
};

use crate::office::pptx::scene::{GradientStop, Rect};
use crate::office::pptx::xml::XNode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathKind {
    Circle,
    Rect,
}

#[derive(Debug, Clone)]
pub struct PathGradient {
    pub kind: PathKind,
    /// Focus rectangle centre and half sizes, gradient space.
    pub focus: (f64, f64),
    pub focus_half: (f64, f64),
    /// Centre and half sizes of the last contour (equal radii for circles).
    pub outer: (f64, f64),
    pub outer_half: (f64, f64),
    /// Colour by `s²` rather than `s` (two stops at the ends).
    pub quadratic: bool,
    pub stops: Vec<GradientStop>,
    /// The region worth rasterizing (the shape box), gradient space.
    pub bounds: Rect,
    /// Gradient space → slide space.
    pub to_user: Transform,
}

/// Builds the gradient for `a:path` inside a `a:gradFill`, for a shape box `b`.
pub fn from_xml(grad: &XNode, path: &XNode, b: Rect, stops: Vec<GradientStop>) -> PathGradient {
    let kind = match path.attr("path") {
        Some("circle") => PathKind::Circle,
        _ => PathKind::Rect,
    };
    let inset = |node: Option<&XNode>, k: &str| {
        node.and_then(|n| n.attr_i64(k)).unwrap_or(0) as f64 / 100_000.0
    };
    let ftr = path.child("a:fillToRect").map(|n| &**n);
    let axis = |near: f64, far: f64| {
        let (lo, hi) = (near, 1.0 - far);
        let hi = hi.max(lo);
        ((lo + hi) / 2.0, (hi - lo) / 2.0)
    };
    let (fx, hx) = axis(inset(ftr, "l"), inset(ftr, "r"));
    let (fy, hy) = axis(inset(ftr, "t"), inset(ftr, "b"));
    let tile = grad.child("a:tileRect").map(|n| &**n);
    let (tl, tt) = (inset(tile, "l"), inset(tile, "t"));
    let (tr, tb) = (1.0 - inset(tile, "r"), 1.0 - inset(tile, "b"));
    let outer = (b.x + b.w * (tl + tr) / 2.0, b.y + b.h * (tt + tb) / 2.0);
    let (ow, oh) = ((b.w * (tr - tl)).abs() / 2.0, (b.h * (tb - tt)).abs() / 2.0);
    let outer_half = match kind {
        PathKind::Circle => {
            let r = ow.hypot(oh);
            (r, r)
        }
        PathKind::Rect => (ow, oh),
    };
    let quadratic = stops.len() == 2 && stops[0].pos == 0.0 && stops[1].pos == 1.0;
    PathGradient {
        kind,
        focus: (b.x + b.w * fx, b.y + b.h * fy),
        focus_half: (b.w * hx, b.h * hy),
        outer,
        outer_half,
        quadratic,
        stops,
        bounds: b,
        to_user: Transform::identity(),
    }
}

impl PathGradient {
    /// The same gradient drawn through `tf` (gradient space stays the shape's box).
    pub fn transformed(mut self, tf: &Transform) -> PathGradient {
        self.to_user = self.to_user.post_concat(*tf);
        self
    }

    /// Contour index `s` of a gradient-space point, clamped to `[0, 1]`.
    pub fn contour(&self, x: f64, y: f64) -> f64 {
        let s = match self.kind {
            PathKind::Circle if self.focus_half == (0.0, 0.0) => self.focal(x, y),
            PathKind::Circle => self.ellipse_bisect(x, y),
            PathKind::Rect => self.rect(x, y),
        };
        s.clamp(0.0, 1.0)
    }

    /// Stop parameter `t` of a point: the contour index, squared for two-stop gradients.
    pub fn param(&self, x: f64, y: f64) -> f64 {
        let s = self.contour(x, y);
        if self.quadratic { s * s } else { s }
    }

    /// Point focus: smallest `s ≥ 0` with |q − s·d| ≤ s·r, q = p − focus,
    /// d = outer − focus; i.e. (d² − r²)s² − 2(q·d)s + q² ≤ 0. Uncovered points
    /// (a focus outside the outer circle leaves a cone) take the last stop.
    fn focal(&self, x: f64, y: f64) -> f64 {
        let (qx, qy) = (x - self.focus.0, y - self.focus.1);
        let (dx, dy) = (self.outer.0 - self.focus.0, self.outer.1 - self.focus.1);
        let r = self.outer_half.0;
        let a = dx * dx + dy * dy - r * r;
        let b = qx * dx + qy * dy;
        let c = qx * qx + qy * qy;
        if c <= 0.0 {
            return 0.0;
        }
        let disc = b * b - a * c;
        if disc < 0.0 {
            return 1.0;
        }
        // c / (b + √disc) is the smaller root of either sign of `a`, without cancellation.
        let den = b + disc.sqrt();
        if den <= 0.0 {
            return 1.0;
        }
        c / den
    }

    fn lerp(&self, s: f64) -> (f64, f64, f64, f64) {
        let l = |a: f64, b: f64| a + (b - a) * s;
        (
            l(self.focus.0, self.outer.0),
            l(self.focus.1, self.outer.1),
            l(self.focus_half.0, self.outer_half.0),
            l(self.focus_half.1, self.outer_half.1),
        )
    }

    /// Focus with an extent: the contour family has no closed form, so bisect on
    /// containment (monotonic over [0, 1] whenever the last contour holds the point).
    fn ellipse_bisect(&self, x: f64, y: f64) -> f64 {
        let inside = |s: f64| {
            let (cx, cy, rx, ry) = self.lerp(s);
            let (dx, dy) = (x - cx, y - cy);
            dx * dx * ry * ry + dy * dy * rx * rx <= rx * rx * ry * ry
        };
        if !inside(1.0) {
            return 1.0;
        }
        let (mut lo, mut hi) = (0.0, 1.0);
        if inside(0.0) {
            return 0.0;
        }
        for _ in 0..16 {
            let mid = (lo + hi) / 2.0;
            if inside(mid) {
                hi = mid;
            } else {
                lo = mid;
            }
        }
        hi
    }

    /// Rectangles: each side condition is linear in `s` (α + β·s ≥ 0), so the
    /// answer is the largest lower bound if it stays under every upper bound.
    fn rect(&self, x: f64, y: f64) -> f64 {
        let (dcx, dcy) = (self.outer.0 - self.focus.0, self.outer.1 - self.focus.1);
        let (drx, dry) = (
            self.outer_half.0 - self.focus_half.0,
            self.outer_half.1 - self.focus_half.1,
        );
        let (px, py) = (x - self.focus.0, y - self.focus.1);
        let conds = [
            (self.focus_half.0 - px, drx + dcx),
            (self.focus_half.0 + px, drx - dcx),
            (self.focus_half.1 - py, dry + dcy),
            (self.focus_half.1 + py, dry - dcy),
        ];
        let (mut lo, mut hi) = (0.0f64, f64::INFINITY);
        for (alpha, beta) in conds {
            if beta.abs() < 1e-12 {
                if alpha < 0.0 {
                    return 1.0;
                }
            } else if beta > 0.0 {
                lo = lo.max(-alpha / beta);
            } else {
                hi = hi.min(-alpha / beta);
            }
        }
        if lo <= hi { lo } else { 1.0 }
    }

    /// Premultiplied colour at stop parameter `t`, interpolated in sRGB.
    fn color_at(&self, t: f64) -> PremultipliedColorU8 {
        let st = &self.stops;
        let t = t as f32;
        let c = if t <= st[0].pos {
            st[0].color
        } else if t >= st[st.len() - 1].pos {
            st[st.len() - 1].color
        } else {
            let i = st.iter().position(|s| s.pos >= t).unwrap_or(st.len() - 1);
            let (a, b) = (&st[i - 1], &st[i]);
            let span = b.pos - a.pos;
            let k = if span > 0.0 { (t - a.pos) / span } else { 1.0 };
            let m = |x: f32, y: f32| x + (y - x) * k;
            crate::office::pptx::color::Rgba {
                r: m(a.color.r, b.color.r),
                g: m(a.color.g, b.color.g),
                b: m(a.color.b, b.color.b),
                a: m(a.color.a, b.color.a),
            }
        };
        c.to_skia().premultiply().to_color_u8()
    }

    /// Rasterizes the gradient over its bounds at about `px_per_pt` device pixels
    /// per gradient-space point (long side capped at `MAX_SIDE`); the result is a
    /// smooth image, so bilinear sampling hides the reduced resolution.
    pub fn raster(&self, px_per_pt: f64) -> Option<Pixmap> {
        const MAX_SIDE: f64 = 512.0;
        let b = self.bounds;
        if !(b.w > 0.0 && b.h > 0.0) || self.stops.is_empty() {
            return None;
        }
        let k = px_per_pt.min(MAX_SIDE / b.w.max(b.h)).max(1e-3);
        let (w, h) = (
            (b.w * k).ceil().clamp(1.0, MAX_SIDE) as u32,
            (b.h * k).ceil().clamp(1.0, MAX_SIDE) as u32,
        );
        let mut pm = Pixmap::new(w, h)?;
        let (sx, sy) = (b.w / w as f64, b.h / h as f64);
        let px = pm.pixels_mut();
        for j in 0..h {
            let y = b.y + (j as f64 + 0.5) * sy;
            for i in 0..w {
                let x = b.x + (i as f64 + 0.5) * sx;
                px[(j * w + i) as usize] = self.color_at(self.param(x, y));
            }
        }
        Some(pm)
    }

    /// Pixmap → slide-space transform for `raster`'s output.
    pub fn pattern_transform(&self, pm: &Pixmap) -> Transform {
        let b = self.bounds;
        Transform::from_scale(
            (b.w / pm.width() as f64) as f32,
            (b.h / pm.height() as f64) as f32,
        )
        .post_translate(b.x as f32, b.y as f32)
        .post_concat(self.to_user)
    }

    /// Device pixels per gradient-space point for a canvas scale of `canvas_scale`
    /// pixels per slide point.
    pub fn density(&self, canvas_scale: f64) -> f64 {
        let t = self.to_user;
        let s = ((t.sx * t.sx + t.ky * t.ky).sqrt()).max((t.kx * t.kx + t.sy * t.sy).sqrt());
        canvas_scale * s as f64
    }
}

/// Raster density in pixels per slide point: 3 covers the 200 dpi page renders
/// (the long side is capped anyway), and bilinear sampling smooths the rest.
const PX_PER_PT: f64 = 3.0;

/// A pattern shader drawing `g`, keeping its pixmap alive in `holder`.
pub fn shader<'a>(
    g: &PathGradient,
    holder: &'a mut Option<std::sync::Arc<Pixmap>>,
) -> Option<Shader<'a>> {
    let pm = g.raster(g.density(PX_PER_PT))?;
    let t = g.pattern_transform(&pm);
    *holder = Some(std::sync::Arc::new(pm));
    let pm = holder.as_ref()?;
    Some(Pattern::new(
        Pixmap::as_ref(pm),
        SpreadMode::Pad,
        FilterQuality::Bilinear,
        1.0,
        t,
    ))
}

#[cfg(test)]
mod tests;
