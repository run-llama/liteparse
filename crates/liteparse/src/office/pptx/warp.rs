//! WordArt text warps (`a:bodyPr/a:prstTxWarp`).
//!
//! Each preset in `assets/presetTextWarpDefinitions.xml` (the ECMA-376 table) is a
//! list of open guide paths evaluated over the text box (insets removed). Two
//! families exist, told apart by the number of paths:
//!
//! - **Fit** (an even number of paths: top/bottom pairs, e.g. `textDeflate`,
//!   `textWave4`, `textArchUpPour`). Each line is first stretched to an equal band
//!   of the box, as for `textPlain` (`textlayout::layout_pass`), then a point at
//!   box fraction (u, v) is moved to `top(u) + (bottom(u) - top(u)) * v`. A curve
//!   that runs monotonically in x is sampled by x (verticals stay vertical), any
//!   other (arcs that turn back) by arc length. Several pairs split the lines
//!   between them in order.
//! - **Follow** (an odd number: `textArchUp/Down`, `textCircle`, `textButton`). The
//!   text keeps its size and runs along the path: x becomes arc length (placed by
//!   the paragraph alignment, compressed only when the line is longer than the
//!   path) and y the distance along the path normal. On an arch the last line's
//!   baseline lies on the path and earlier lines stack on the glyphs' side (text
//!   stands on the arc, outside the box ellipse for `textArchUp`); `textCircle`
//!   text hangs inside its circle; a path through the middle of the box carries
//!   its lines centred on it. Each line follows the path offset to its own
//!   baseline, so letter widths are kept on every line. These placements were
//!   matched against LibreOffice's rendering of template decks whose arched
//!   labels sit in rings drawn for them (PowerPoint references were too small).
//!
//! Glyph outlines (and underlines) are warped at draw time: [`RunWarp`] on a
//! [`GlyphRun`](crate::office::pptx::scene::GlyphRun) holds the run's real placement, while the run's `transform` is
//! replaced by an axis-aligned map onto the warped run's bounding box, so text
//! items report where the warped text actually is.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use tiny_skia::{Path, PathBuilder, PathSegment, Point, Transform};

use crate::office::pptx::geom::{self, Cmd};
use crate::office::pptx::scene::Item;
use crate::office::pptx::textprops::BodyProps;
use crate::office::pptx::xml::{self, XNode};

const DEFS_XML: &str = include_str!("assets/presetTextWarpDefinitions.xml");
const EMU_PER_PT: f64 = 12_700.0;
/// Longest piece (points, box space) a glyph outline segment is cut into before
/// its control points are moved; small enough that the warp looks smooth.
const STEP: f64 = 1.5;

thread_local! {
    static DEFS: HashMap<String, Rc<XNode>> = load_defs();
}

fn load_defs() -> HashMap<String, Rc<XNode>> {
    let root = xml::parse(DEFS_XML).expect("vendored text warp definitions parse");
    root.children
        .iter()
        .map(|c| (c.name.clone(), c.clone()))
        .collect()
}

type Pt = (f64, f64);

/// A guide path flattened to a polyline, with cumulative arc length per vertex.
#[derive(Debug, Clone)]
pub(crate) struct Curve {
    pts: Vec<Pt>,
    cum: Vec<f64>,
    /// x runs monotonically from the first point to the last.
    by_x: bool,
}

impl Curve {
    fn new(raw: Vec<Pt>) -> Option<Curve> {
        let mut pts: Vec<Pt> = Vec::with_capacity(raw.len());
        for p in raw {
            if pts
                .last()
                .is_none_or(|q| (q.0 - p.0).hypot(q.1 - p.1) > 1e-6)
            {
                pts.push(p);
            }
        }
        if pts.len() < 2 || pts.iter().any(|p| !p.0.is_finite() || !p.1.is_finite()) {
            return None;
        }
        let mut cum = Vec::with_capacity(pts.len());
        cum.push(0.0);
        for w in pts.windows(2) {
            cum.push(cum.last().copied().unwrap_or(0.0) + (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1));
        }
        let dx = pts[pts.len() - 1].0 - pts[0].0;
        let by_x = dx.abs() > 1e-6
            && pts
                .windows(2)
                .all(|w| (w[1].0 - w[0].0) * dx.signum() >= -1e-6);
        Some(Curve { pts, cum, by_x })
    }

    fn from_cmds(cmds: &[Cmd]) -> Option<Curve> {
        const N: usize = 24;
        let mut pts = Vec::new();
        let mut start = (0.0, 0.0);
        let mut cur = (0.0, 0.0);
        for c in cmds {
            match *c {
                Cmd::Move(x, y) => {
                    pts.push((x, y));
                    start = (x, y);
                    cur = (x, y);
                }
                Cmd::Line(x, y) => {
                    pts.push((x, y));
                    cur = (x, y);
                }
                Cmd::Quad(x1, y1, x, y) => {
                    for k in 1..=N {
                        let t = k as f64 / N as f64;
                        let m = 1.0 - t;
                        pts.push((
                            m * m * cur.0 + 2.0 * m * t * x1 + t * t * x,
                            m * m * cur.1 + 2.0 * m * t * y1 + t * t * y,
                        ));
                    }
                    cur = (x, y);
                }
                Cmd::Cubic(x1, y1, x2, y2, x, y) => {
                    for k in 1..=N {
                        let t = k as f64 / N as f64;
                        let m = 1.0 - t;
                        let (a, b, c, d) = (m * m * m, 3.0 * m * m * t, 3.0 * m * t * t, t * t * t);
                        pts.push((
                            a * cur.0 + b * x1 + c * x2 + d * x,
                            a * cur.1 + b * y1 + c * y2 + d * y,
                        ));
                    }
                    cur = (x, y);
                }
                Cmd::Close => {
                    pts.push(start);
                    cur = start;
                }
            }
        }
        Curve::new(pts)
    }

    fn len(&self) -> f64 {
        self.cum.last().copied().unwrap_or(0.0)
    }

    /// Point and unit tangent at arc length `s`; past either end the end segment
    /// extends in a straight line.
    fn at(&self, s: f64) -> (Pt, Pt) {
        let n = self.pts.len();
        let i = self
            .cum
            .partition_point(|&c| c <= s)
            .saturating_sub(1)
            .min(n - 2);
        let (a, b) = (self.pts[i], self.pts[i + 1]);
        let l = (self.cum[i + 1] - self.cum[i]).max(1e-12);
        let t = ((b.0 - a.0) / l, (b.1 - a.1) / l);
        let d = s - self.cum[i];
        ((a.0 + t.0 * d, a.1 + t.1 * d), t)
    }

    /// Point at fraction `u` of the curve: by x when x is monotonic, else by length.
    fn at_frac(&self, u: f64) -> Pt {
        if !self.by_x {
            return self.at(u * self.len()).0;
        }
        let n = self.pts.len();
        let (x0, x1) = (self.pts[0].0, self.pts[n - 1].0);
        let sgn = (x1 - x0).signum();
        let x = x0 + u * (x1 - x0);
        let i = self
            .pts
            .partition_point(|p| (p.0 - x) * sgn <= 0.0)
            .saturating_sub(1)
            .min(n - 2);
        let (a, b) = (self.pts[i], self.pts[i + 1]);
        let dx = b.0 - a.0;
        let f = if dx.abs() > 1e-12 {
            (x - a.0) / dx
        } else {
            0.0
        };
        (a.0 + f * (b.0 - a.0), a.1 + f * (b.1 - a.1))
    }

    /// The curve moved by `d` along its text-down normal (tangent turned 90°
    /// clockwise on screen, i.e. (−ty, tx) with y down).
    fn offset(&self, d: f64) -> Option<Curve> {
        if d.abs() < 1e-9 {
            return Some(self.clone());
        }
        let n = self.pts.len();
        let seg = |i: usize| -> Pt {
            let (a, b) = (self.pts[i], self.pts[i + 1]);
            let l = (b.0 - a.0).hypot(b.1 - a.1).max(1e-12);
            ((b.0 - a.0) / l, (b.1 - a.1) / l)
        };
        let pts = (0..n)
            .map(|i| {
                let t0 = seg(i.saturating_sub(1).min(n - 2));
                let t1 = seg(i.min(n - 2));
                let (tx, ty) = (t0.0 + t1.0, t0.1 + t1.1);
                let l = tx.hypot(ty);
                let (tx, ty) = if l > 1e-9 { (tx / l, ty / l) } else { t1 };
                let p = self.pts[i];
                (p.0 - ty * d, p.1 + tx * d)
            })
            .collect();
        Curve::new(pts)
    }
}

/// Paragraph alignment of a line, as far as placing it along a path goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

impl Align {
    pub fn from_algn(a: &str) -> Align {
        match a {
            "ctr" => Align::Center,
            "r" => Align::Right,
            _ => Align::Left,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Anchor {
    /// The block's top on the path, glyphs hanging on its inner side (textCircle).
    Top,
    /// The last line's baseline on the path, glyphs standing on it (arches).
    Baseline,
    /// The block centred on the path (a path through the middle of the box).
    Center,
}

#[derive(Debug)]
enum Kind {
    Fit(Vec<(Curve, Curve)>),
    Follow(Vec<(Curve, Anchor)>),
}

/// A text box's warp, evaluated over its box (points, origin at the text area's
/// top-left after insets).
#[derive(Debug)]
pub struct TextWarp {
    kind: Kind,
    box_w: f64,
    box_h: f64,
}

/// Vertical metrics of one laid-out line in box space: (baseline, ascent, descent).
pub type LineMetrics = (f64, f64, f64);

impl TextWarp {
    /// The warp of `body`, or None for no warp, `textPlain` (handled by the
    /// layout itself), an unknown preset or a degenerate box.
    pub fn new(body: &BodyProps, box_w: f64, box_h: f64) -> Option<TextWarp> {
        let prst = body.warp.as_deref()?;
        if prst == "textPlain" || box_w <= 0.0 || box_h <= 0.0 {
            return None;
        }
        Self::from_preset(prst, &body.warp_adj, box_w, box_h)
    }

    pub(crate) fn from_preset(
        prst: &str,
        adj: &[(String, String)],
        box_w: f64,
        box_h: f64,
    ) -> Option<TextWarp> {
        let def = DEFS.with(|d| d.get(prst).cloned())?;
        let av = XNode {
            name: "a:avLst".into(),
            attrs: Vec::new(),
            children: adj
                .iter()
                .map(|(n, f)| {
                    Rc::new(XNode {
                        name: "a:gd".into(),
                        attrs: vec![("name".into(), n.clone()), ("fmla".into(), f.clone())],
                        children: Vec::new(),
                        text: String::new(),
                    })
                })
                .collect(),
            text: String::new(),
        };
        let g = geom::evaluate(&def, Some(&av), box_w * EMU_PER_PT, box_h * EMU_PER_PT);
        let curves: Vec<Curve> = g
            .paths
            .iter()
            .map(|p| Curve::from_cmds(&p.cmds))
            .collect::<Option<_>>()?;
        if curves.is_empty() {
            return None;
        }
        let kind = if curves.len().is_multiple_of(2) {
            let mut it = curves.into_iter();
            let mut pairs = Vec::new();
            while let (Some(a), Some(b)) = (it.next(), it.next()) {
                pairs.push((a, b));
            }
            Kind::Fit(pairs)
        } else {
            Kind::Follow(
                curves
                    .into_iter()
                    .map(|c| {
                        let mid = c.at(c.len() / 2.0).0;
                        let anchor = if prst == "textCircle" {
                            Anchor::Top
                        } else if mid.1 < box_h * 0.4 || mid.1 > box_h * 0.6 {
                            Anchor::Baseline
                        } else {
                            Anchor::Center
                        };
                        (c, anchor)
                    })
                    .collect(),
            )
        };
        Some(TextWarp { kind, box_w, box_h })
    }

    /// True for the fit family: lines are stretched to bands of the box first.
    pub fn fits(&self) -> bool {
        matches!(self.kind, Kind::Fit(_))
    }

    /// Path index of each line for the follow family: the first line on the
    /// first path, the last on the last, the rest on the middle one.
    fn follow_path(line: usize, n_lines: usize, n_paths: usize) -> usize {
        if n_paths == 1 || line == 0 {
            0
        } else if line + 1 == n_lines {
            n_paths - 1
        } else {
            n_paths / 2
        }
    }

    /// The warp of line `i` of `lines`. `x` and `width` are the line's start and
    /// content width (box space); for the fit family, the line must already be
    /// stretched to its band of the box. `to_slide` maps box space to the slide.
    pub fn line(
        &self,
        i: usize,
        lines: &[LineMetrics],
        x: f64,
        width: f64,
        align: Align,
        to_slide: Transform,
    ) -> Option<Arc<LineWarp>> {
        let from_slide = to_slide.invert()?;
        let n = lines.len();
        let map = match &self.kind {
            Kind::Fit(pairs) => {
                let p = pairs.len();
                let pair_of = |l: usize| if n >= p { l * p / n } else { l };
                let k = pair_of(i);
                let first = (0..n).find(|&l| pair_of(l) == k)?;
                let end = (0..n).rposition(|l| pair_of(l) == k)? + 1;
                let (top, bot) = pairs.get(k)?.clone();
                LineMap::Fit {
                    top,
                    bot,
                    v0: first as f64 / n as f64,
                    v1: end as f64 / n as f64,
                    w: self.box_w,
                    h: self.box_h,
                }
            }
            Kind::Follow(paths) => {
                let pi = Self::follow_path(i, n, paths.len());
                let (curve, anchor) = &paths[pi];
                let group: Vec<&LineMetrics> = (0..n)
                    .filter(|&l| Self::follow_path(l, n, paths.len()) == pi)
                    .map(|l| &lines[l])
                    .collect();
                let top = group.iter().map(|m| m.0 - m.1).fold(f64::MAX, f64::min);
                let bottom = group.iter().map(|m| m.0 + m.2).fold(f64::MIN, f64::max);
                let anchor_y = match anchor {
                    Anchor::Top => top,
                    Anchor::Baseline => group.last().map_or(bottom, |m| m.0),
                    Anchor::Center => (top + bottom) / 2.0,
                };
                let baseline = lines.get(i)?.0;
                let curve = curve.offset(baseline - anchor_y)?;
                let len = curve.len();
                let (s0, k) = if width > len {
                    (0.0, len / width)
                } else {
                    let s0 = match align {
                        Align::Left => 0.0,
                        Align::Center => (len - width) / 2.0,
                        Align::Right => len - width,
                    };
                    (s0, 1.0)
                };
                LineMap::Follow {
                    curve,
                    s0,
                    k,
                    x0: x,
                    baseline,
                }
            }
        };
        Some(Arc::new(LineWarp {
            map,
            to_slide,
            from_slide,
        }))
    }
}

#[derive(Debug)]
enum LineMap {
    Fit {
        top: Curve,
        bot: Curve,
        /// The pair's share of the box height (fractions) that this line's group fills.
        v0: f64,
        v1: f64,
        w: f64,
        h: f64,
    },
    Follow {
        /// The path offset to this line's baseline.
        curve: Curve,
        /// Arc length where the line's start lands, and the x → length scale.
        s0: f64,
        k: f64,
        x0: f64,
        baseline: f64,
    },
}

/// The warp of one line: a point map from slide space to slide space.
#[derive(Debug)]
pub struct LineWarp {
    map: LineMap,
    to_slide: Transform,
    from_slide: Transform,
}

impl LineWarp {
    fn map_box(&self, p: Pt) -> Pt {
        match &self.map {
            LineMap::Fit {
                top,
                bot,
                v0,
                v1,
                w,
                h,
            } => {
                let u = p.0 / w;
                let v = (p.1 / h - v0) / (v1 - v0).max(1e-9);
                let a = top.at_frac(u);
                let b = bot.at_frac(u);
                (a.0 + (b.0 - a.0) * v, a.1 + (b.1 - a.1) * v)
            }
            LineMap::Follow {
                curve,
                s0,
                k,
                x0,
                baseline,
            } => {
                let (q, t) = curve.at(s0 + (p.0 - x0) * k);
                let d = p.1 - baseline;
                (q.0 - t.1 * d, q.1 + t.0 * d)
            }
        }
    }

    fn to_box(&self, p: Point) -> Pt {
        let mut q = [p];
        self.from_slide.map_points(&mut q);
        (q[0].x as f64, q[0].y as f64)
    }

    fn to_slide(&self, p: Pt) -> Point {
        let mut q = [Point::from_xy(p.0 as f32, p.1 as f32)];
        self.to_slide.map_points(&mut q);
        q[0]
    }

    /// Warps a slide-space point.
    pub fn map_point(&self, p: Point) -> Point {
        self.to_slide(self.map_box(self.to_box(p)))
    }

    /// `path` placed by `t` (into slide space), warped; the result is in slide space.
    /// Segments are cut into short pieces whose control points are moved.
    pub fn warp_path(&self, path: &Path, t: Transform) -> Option<Path> {
        let to_box = |p: Point| {
            let mut q = [p];
            t.map_points(&mut q);
            self.to_box(q[0])
        };
        let out = |p: Pt| self.to_slide(self.map_box(p));
        let pieces = |a: Pt, b: Pt, extra: f64| -> usize {
            (((b.0 - a.0).hypot(b.1 - a.1) + extra) / STEP)
                .ceil()
                .clamp(1.0, 64.0) as usize
        };
        let mut pb = PathBuilder::new();
        let mut cur: Pt = (0.0, 0.0);
        let mut start: Pt = (0.0, 0.0);
        for seg in path.segments() {
            match seg {
                PathSegment::MoveTo(p) => {
                    cur = to_box(p);
                    start = cur;
                    let o = out(cur);
                    pb.move_to(o.x, o.y);
                }
                PathSegment::LineTo(p) => {
                    let b = to_box(p);
                    line_pieces(&mut pb, cur, b, pieces(cur, b, 0.0), &out);
                    cur = b;
                }
                PathSegment::QuadTo(p1, p) => {
                    let (q1, b) = (to_box(p1), to_box(p));
                    // Degree elevation: the same curve as a cubic.
                    let c1 = (
                        cur.0 + 2.0 / 3.0 * (q1.0 - cur.0),
                        cur.1 + 2.0 / 3.0 * (q1.1 - cur.1),
                    );
                    let c2 = (
                        b.0 + 2.0 / 3.0 * (q1.0 - b.0),
                        b.1 + 2.0 / 3.0 * (q1.1 - b.1),
                    );
                    let n = pieces(cur, b, (q1.0 - cur.0).hypot(q1.1 - cur.1));
                    cubic_pieces(&mut pb, [cur, c1, c2, b], n, &out);
                    cur = b;
                }
                PathSegment::CubicTo(p1, p2, p) => {
                    let (c1, c2, b) = (to_box(p1), to_box(p2), to_box(p));
                    let n = pieces(cur, b, (c1.0 - cur.0).hypot(c1.1 - cur.1));
                    cubic_pieces(&mut pb, [cur, c1, c2, b], n, &out);
                    cur = b;
                }
                PathSegment::Close => {
                    line_pieces(&mut pb, cur, start, pieces(cur, start, 0.0), &out);
                    pb.close();
                    cur = start;
                }
            }
        }
        pb.finish()
    }
}

fn line_pieces(pb: &mut PathBuilder, a: Pt, b: Pt, n: usize, out: &impl Fn(Pt) -> Point) {
    for k in 1..=n {
        let t = k as f64 / n as f64;
        let o = out((a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t));
        pb.line_to(o.x, o.y);
    }
}

fn cubic_pieces(pb: &mut PathBuilder, c: [Pt; 4], n: usize, out: &impl Fn(Pt) -> Point) {
    let at = |t: f64| -> (Pt, Pt) {
        let m = 1.0 - t;
        let p = (
            m * m * m * c[0].0
                + 3.0 * m * m * t * c[1].0
                + 3.0 * m * t * t * c[2].0
                + t * t * t * c[3].0,
            m * m * m * c[0].1
                + 3.0 * m * m * t * c[1].1
                + 3.0 * m * t * t * c[2].1
                + t * t * t * c[3].1,
        );
        let d = (
            3.0 * (m * m * (c[1].0 - c[0].0)
                + 2.0 * m * t * (c[2].0 - c[1].0)
                + t * t * (c[3].0 - c[2].0)),
            3.0 * (m * m * (c[1].1 - c[0].1)
                + 2.0 * m * t * (c[2].1 - c[1].1)
                + t * t * (c[3].1 - c[2].1)),
        );
        (p, d)
    };
    for k in 0..n {
        let (t0, t1) = (k as f64 / n as f64, (k + 1) as f64 / n as f64);
        let h = (t1 - t0) / 3.0;
        let ((p0, d0), (p3, d3)) = (at(t0), at(t1));
        let p1 = out((p0.0 + d0.0 * h, p0.1 + d0.1 * h));
        let p2 = out((p3.0 - d3.0 * h, p3.1 - d3.1 * h));
        let p3 = out(p3);
        pb.cubic_to(p1.x, p1.y, p2.x, p2.y, p3.x, p3.y);
    }
}

/// A warped glyph run: its real run-local → slide placement and the line's warp.
#[derive(Debug, Clone)]
pub struct RunWarp {
    pub warp: Arc<LineWarp>,
    pub local: Transform,
}

impl RunWarp {
    /// A glyph outline placed by `t` (glyph → run-local), warped into slide space.
    pub fn warp_path(&self, path: &Path, t: Transform) -> Option<Path> {
        self.warp.warp_path(path, t.post_concat(self.local))
    }
}

/// Applies a line's warp to the items emitted for it: glyph runs (and their
/// shadows) get a [`RunWarp`] and a transform onto their warped bounding box;
/// text decorations (underline, highlight) are warped in place.
pub fn apply(items: &mut [Item], warp: &Arc<LineWarp>) {
    for item in items {
        match item {
            Item::Text(run) => warp_run(run, warp),
            Item::Shadow(s) => {
                if let crate::office::pptx::scene::ShadowSource::Glyphs(run) = &mut s.source {
                    warp_run(run, warp);
                }
            }
            Item::Path(p) if p.from_text => {
                if let Some(path) = warp.warp_path(&p.path, Transform::identity()) {
                    p.path = path;
                    p.has_curve = true;
                }
            }
            _ => {}
        }
    }
}

fn warp_run(run: &mut crate::office::pptx::scene::GlyphRun, warp: &Arc<LineWarp>) {
    let local = run.transform;
    let (w, top, bottom) = (run.width, -run.ascent, run.descent);
    // Bounding box of the warped line box, sampled along its edges.
    const N: usize = 16;
    let mut b = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for k in 0..=N {
        let f = k as f64 / N as f64;
        for p in [
            (w * f, top),
            (w * f, bottom),
            (0.0, top + (bottom - top) * f),
            (w, top + (bottom - top) * f),
        ] {
            let mut q = [Point::from_xy(p.0 as f32, p.1 as f32)];
            local.map_points(&mut q);
            let q = warp.map_point(q[0]);
            b = (b.0.min(q.x), b.1.min(q.y), b.2.max(q.x), b.3.max(q.y));
        }
    }
    let h = bottom - top;
    if w > 1e-6 && h > 1e-6 && b.0.is_finite() && b.2 > b.0 {
        let sx = (b.2 - b.0) / w as f32;
        let sy = (b.3 - b.1) / h as f32;
        run.transform = Transform::from_row(sx, 0.0, 0.0, sy, b.0, b.1 - top as f32 * sy);
    }
    run.warp = Some(Arc::new(RunWarp {
        warp: warp.clone(),
        local,
    }));
}

#[cfg(test)]
mod tests;
