//! Surface charts (`c:surfaceChart`, `c:surface3DChart`).
//!
//! The grid is categories (columns) by series (rows). Each grid cell is split
//! into two triangles and every triangle is clipped against the value bands
//! (one band per major unit of the value axis), so band edges are true linear
//! contours. A 2-D surface is that contour map seen from the top; a 3-D one is
//! drawn as an orthographic projection (`view3D` rotX/rotY), painted back to
//! front, with the walls' value gridlines and the three axes' labels.

use super::*;

/// A clipped polygon vertex: grid position (x along categories, z along
/// series, both 0..1) and value.
#[derive(Clone, Copy, Debug)]
struct Vtx {
    x: f64,
    z: f64,
    v: f64,
}

fn lerp(a: Vtx, b: Vtx, t: f64) -> Vtx {
    Vtx {
        x: a.x + (b.x - a.x) * t,
        z: a.z + (b.z - a.z) * t,
        v: a.v + (b.v - a.v) * t,
    }
}

/// Keeps the part of a convex polygon where `keep(v)` holds; `edge` is the
/// band boundary value (Sutherland–Hodgman against a scalar threshold).
fn clip(poly: &[Vtx], edge: f64, above: bool) -> Vec<Vtx> {
    let inside = |p: &Vtx| if above { p.v >= edge } else { p.v <= edge };
    let mut out = Vec::new();
    for i in 0..poly.len() {
        let a = poly[i];
        let b = poly[(i + 1) % poly.len()];
        let (ia, ib) = (inside(&a), inside(&b));
        if ia {
            out.push(a);
        }
        if ia != ib && (b.v - a.v).abs() > 1e-300 {
            out.push(lerp(a, b, (edge - a.v) / (b.v - a.v)));
        }
    }
    out
}

/// A grid point or clipped vertex: (x, z, value).
pub(super) type Pt3 = (f64, f64, f64);

/// Splits a triangle into per-band pieces: `(band index, polygon)`.
pub(super) fn band_pieces(tri: [Pt3; 3], edges: &[f64]) -> Vec<(usize, Vec<Pt3>)> {
    let poly: Vec<Vtx> = tri.iter().map(|&(x, z, v)| Vtx { x, z, v }).collect();
    let lo = poly.iter().map(|p| p.v).fold(f64::INFINITY, f64::min);
    let hi = poly.iter().map(|p| p.v).fold(f64::NEG_INFINITY, f64::max);
    let nb = edges.len().saturating_sub(1);
    let mut out = Vec::new();
    for k in 0..nb {
        let (a, b) = (edges[k], edges[k + 1]);
        // Values outside the axis range fall into the first or last band.
        let a_eff = if k == 0 { f64::NEG_INFINITY } else { a };
        let b_eff = if k + 1 == nb { f64::INFINITY } else { b };
        if hi < a_eff || lo > b_eff {
            continue;
        }
        let mut p = poly.clone();
        if a_eff.is_finite() {
            p = clip(&p, a_eff, true);
        }
        if b_eff.is_finite() && p.len() >= 3 {
            p = clip(&p, b_eff, false);
        }
        if p.len() >= 3 {
            out.push((k, p.iter().map(|q| (q.x, q.z, q.v)).collect()));
        }
    }
    out
}

impl Renderer<'_, '_> {
    /// The value axis of a surface group, scaled; its ticks are the band edges.
    fn surface_axis(&mut self, g: &Grp) -> Axis {
        let plot = self.chart.child("c:plotArea").cloned();
        let node = plot.as_ref().and_then(|p| {
            p.children_named("c:valAx")
                .find(|a| g.ax.1.is_none() || val(a, "c:axId") == g.ax.1.as_deref())
                .cloned()
        });
        let mut a = match &node {
            Some(n) => Self::read_axis(n, AxKind::Val),
            None => synth_axis(AxKind::Val, 'l'),
        };
        a.horizontal = false;
        let txpr = a.node.as_ref().and_then(|n| n.child("c:txPr").cloned());
        a.lists = self.lists(TextKind::Body, &[txpr.as_ref()]);
        for s in &g.sers {
            for v in s.vals.iter().flatten() {
                if !a.has_data {
                    (a.lo, a.hi, a.has_data) = (*v, *v, true);
                } else {
                    a.lo = a.lo.min(*v);
                    a.hi = a.hi.max(*v);
                }
            }
        }
        let nf = a.node.as_ref().and_then(|n| n.child("c:numFmt").cloned());
        a.fmt = match &nf {
            Some(f) if f.attr_bool("sourceLinked") != Some(true) => {
                f.attr("formatCode").unwrap_or("General").to_string()
            }
            _ => g
                .sers
                .first()
                .map(|s| s.fmt.clone())
                .unwrap_or_else(|| "General".into()),
        };
        // Band count follows the axis scale, which depends on its length.
        let len = self.h * 0.6;
        self.scale_axis(&mut a, len);
        a
    }

    fn band_paint(&self, g: &Grp, k: usize, nb: usize) -> Option<Paint> {
        let own = g.node.child("c:bandFmts").and_then(|b| {
            b.children_named("c:bandFmt")
                .find(|f| val_f(f, "c:idx").map(|v| v as usize) == Some(k))
                .and_then(|f| f.child("c:spPr").cloned())
        });
        let auto = Some(Paint::Solid(self.auto_color(k, nb)));
        self.fill_or(
            own.as_deref(),
            Rect {
                x: 0.0,
                y: 0.0,
                w: 1.0,
                h: 1.0,
            },
            auto,
        )
    }

    pub(super) fn surface_legend(&mut self, g: &Grp) -> Vec<(String, Key)> {
        let a = self.surface_axis(g);
        let edges = a.ticks();
        let nb = edges.len().saturating_sub(1);
        (0..nb)
            .map(|k| {
                let f = |v: f64| format_value(v, &a.fmt, self.date1904);
                let text = format!("{}-{}", f(edges[k]), f(edges[k + 1]));
                (text, Key::Box(self.band_paint(g, k, nb), None))
            })
            .collect()
    }

    /// An axis title label (rich text or cached text), rotated as stored.
    fn surface_title(&mut self, axis: Option<&Rc<XNode>>, default_rot: f64) -> Option<Label> {
        let tn = axis?.child("c:title")?.clone();
        let ttx = tn.child("c:txPr").cloned();
        let mut lists = self.lists(TextKind::AxisTitle, &[ttx.as_ref()]);
        let rich = tn.path(&["c:tx", "c:rich"]).cloned();
        let rot = rich
            .as_ref()
            .and_then(|r| txpr_rot(Some(r)))
            .or(txpr_rot(ttx.as_ref()))
            .unwrap_or(default_rot);
        match rich {
            Some(r) if !r.all_text().trim().is_empty() => {
                if let Some(ls) = r.child("a:lstStyle") {
                    lists.push(ls.clone());
                }
                self.label_tx(r, lists, rot, None)
            }
            _ => {
                let t = tn
                    .path(&["c:tx", "c:strRef", "c:strCache"])
                    .and_then(|c| c.children_named("c:pt").next().cloned())
                    .and_then(|p| p.child("c:v").map(|v| v.text.clone()))
                    .unwrap_or_else(|| "Axis Title".into());
                self.label(&t, &lists, rot, None)
            }
        }
    }

    pub(super) fn draw_surface(
        &mut self,
        g: &Grp,
        plot: Option<&XNode>,
        rect: Rect,
        three_d: bool,
    ) {
        let m = g.sers.len().min(500);
        let n = g.n_points().min(1000);
        if m == 0 || n == 0 {
            return;
        }
        let vax = self.surface_axis(g);
        let edges = vax.ticks();
        if edges.len() < 2 {
            return;
        }
        let nb = edges.len() - 1;
        let grid: Vec<Vec<f64>> = g
            .sers
            .iter()
            .take(m)
            .map(|s| {
                (0..n)
                    .map(|i| s.vals.get(i).copied().flatten().unwrap_or(0.0))
                    .collect()
            })
            .collect();
        let find = |name: &str| plot.and_then(|p| p.child(name).cloned());
        let (cat_node, ser_node) = (find("c:catAx"), find("c:serAx"));
        let lists_of = |r: &mut Self, a: Option<&Rc<XNode>>| {
            let t = a.and_then(|a| a.child("c:txPr").cloned());
            r.lists(TextKind::Body, &[t.as_ref()])
        };
        let cat_lists = lists_of(self, cat_node.as_ref());
        let ser_lists = lists_of(self, ser_node.as_ref());
        let shown = |a: &Option<Rc<XNode>>| {
            a.as_ref().is_none_or(|a| {
                flag(a, "c:delete") != Some(true) && val(a, "c:tickLblPos") != Some("none")
            })
        };
        let cat_texts: Vec<String> = match g.sers.iter().find(|s| !s.s.categories.is_empty()) {
            Some(s) => (0..n).map(|i| self.cat_text(s, i, None)).collect(),
            None => (1..=n).map(|i| i.to_string()).collect(),
        };
        let ser_texts: Vec<String> = g
            .sers
            .iter()
            .take(m)
            .map(|s| {
                s.s.name
                    .clone()
                    .unwrap_or_else(|| format!("Series{}", s.s.index + 1))
            })
            .collect();
        let size = resolve_para(&cat_lists, 0, None)
            .def_run
            .size
            .unwrap_or(10.0);
        let titles = (
            self.surface_title(cat_node.as_ref(), 0.0),
            self.surface_title(vax.node.as_ref(), -90.0),
            self.surface_title(ser_node.as_ref(), -90.0),
        );
        // Room for labels and titles around the drawing.
        let (tb, tl, tr) = (
            titles.0.as_ref().map(|t| t.ext().1 + 4.0).unwrap_or(0.0),
            titles.1.as_ref().map(|t| t.ext().0 + 4.0).unwrap_or(0.0),
            titles.2.as_ref().map(|t| t.ext().0 + 4.0).unwrap_or(0.0),
        );
        let lab_h = size * 1.6;
        let lab_w = size * 3.0;
        let inner = Rect {
            x: rect.x + tl + lab_w,
            y: rect.y + size * 0.5,
            w: (rect.w - tl - tr - 2.0 * lab_w).max(rect.w * 0.3),
            h: (rect.h - tb - lab_h - size * 0.5).max(rect.h * 0.3),
        };
        let view = self.chart.child("c:view3D").cloned();
        let rot_x = view
            .as_ref()
            .and_then(|v| val_f(v, "c:rotX"))
            .unwrap_or(15.0)
            .clamp(-90.0, 90.0)
            .to_radians();
        let rot_y = view
            .as_ref()
            .and_then(|v| val_f(v, "c:rotY"))
            .unwrap_or(20.0)
            .to_radians();
        // World box: x, z in [-1, 1] (depth scaled), height `hgt`.
        let (depth, hgt) = (0.8, 0.9);
        let world = |x: f64, z: f64, v: f64| {
            let wx = x * 2.0 - 1.0;
            let wz = (z * 2.0 - 1.0) * depth;
            let wy = vax.frac(v).clamp(0.0, 1.0) * hgt;
            let x1 = wx * rot_y.cos() + wz * rot_y.sin();
            let z1 = -wx * rot_y.sin() + wz * rot_y.cos();
            let sy = -wy * rot_x.cos() - z1 * rot_x.sin();
            let d = z1 * rot_x.cos() - wy * rot_x.sin();
            (x1, sy, d)
        };
        let (vmin, vmax) = (vax.min, vax.max);
        // Fit the projected box into `inner`.
        let mut bb = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for &x in &[0.0, 1.0] {
            for &z in &[0.0, 1.0] {
                for &v in &[vmin, vmax] {
                    let (px, py, _) = if three_d {
                        world(x, z, v)
                    } else {
                        (x, -z, 0.0)
                    };
                    bb = (bb.0.min(px), bb.1.min(py), bb.2.max(px), bb.3.max(py));
                }
            }
        }
        let sc = if three_d {
            (inner.w / (bb.2 - bb.0)).min(inner.h / (bb.3 - bb.1))
        } else {
            1.0
        };
        let ox = inner.x + (inner.w - (bb.2 - bb.0) * sc) / 2.0 - bb.0 * sc;
        let oy = inner.y + (inner.h - (bb.3 - bb.1) * sc) / 2.0 - bb.1 * sc;
        let proj = |x: f64, z: f64, v: f64| -> (f64, f64, f64) {
            if three_d {
                let (px, py, d) = world(x, z, v);
                (ox + px * sc, oy + py * sc, d)
            } else {
                (inner.x + x * inner.w, inner.y + (1.0 - z) * inner.h, 0.0)
            }
        };
        let gx = |i: usize| {
            if n > 1 {
                i as f64 / (n - 1) as f64
            } else {
                0.5
            }
        };
        let gz = |j: usize| {
            if m > 1 {
                j as f64 / (m - 1) as f64
            } else {
                0.5
            }
        };
        let grid_stroke = self.stroke_or(
            vax.node
                .as_ref()
                .and_then(|a| a.child("c:majorGridlines"))
                .and_then(|gl| gl.child("c:spPr"))
                .map(|s| s.as_ref()),
            Some((self.line_default(true), 0.75)),
        );
        if three_d {
            // Back and side walls with the value gridlines, then the floor edges.
            for t in &edges {
                let a = proj(0.0, 1.0, *t);
                let b = proj(1.0, 1.0, *t);
                let c = proj(0.0, 0.0, *t);
                self.push_line(&[(c.0, c.1), (a.0, a.1), (b.0, b.1)], grid_stroke.clone());
            }
            for (x, z) in [(0.0, 1.0), (1.0, 1.0), (0.0, 0.0)] {
                let a = proj(x, z, vmin);
                let b = proj(x, z, vmax);
                self.push_line(&[(a.0, a.1), (b.0, b.1)], grid_stroke.clone());
            }
            let f: Vec<(f64, f64)> = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0)]
                .iter()
                .map(|&(x, z)| {
                    let p = proj(x, z, vmin);
                    (p.0, p.1)
                })
                .collect();
            self.push_line(&f, grid_stroke);
        }
        // Band polygons, painted back to front in 3-D.
        let wire = flag(&g.node, "c:wireframe").unwrap_or(false);
        // (depth, band, screen polygon)
        type Piece = (f64, usize, Vec<(f64, f64)>);
        let mut polys: Vec<Piece> = Vec::new();
        for j in 0..m.saturating_sub(1) {
            for i in 0..n.saturating_sub(1) {
                let c = |ii: usize, jj: usize| (gx(ii), gz(jj), grid[jj][ii]);
                let quads = [
                    [c(i, j), c(i + 1, j), c(i + 1, j + 1)],
                    [c(i, j), c(i + 1, j + 1), c(i, j + 1)],
                ];
                for tri in quads {
                    for (k, poly) in band_pieces(tri, &edges) {
                        let pts: Vec<(f64, f64, f64)> =
                            poly.iter().map(|&(x, z, v)| proj(x, z, v)).collect();
                        let d = pts.iter().map(|p| p.2).sum::<f64>() / pts.len() as f64;
                        polys.push((d, k, pts.iter().map(|p| (p.0, p.1)).collect()));
                    }
                }
            }
        }
        if three_d {
            polys.sort_by(|a, b| b.0.total_cmp(&a.0));
        } else {
            polys.sort_by_key(|p| p.1);
        }
        let paints: Vec<Option<Paint>> = (0..nb).map(|k| self.band_paint(g, k, nb)).collect();
        let mut start = 0;
        while start < polys.len() {
            let k = polys[start].1;
            let mut end = start;
            let mut pb = PathBuilder::new();
            while end < polys.len() && polys[end].1 == k {
                let pts = &polys[end].2;
                pb.move_to(pts[0].0 as f32, pts[0].1 as f32);
                for p in &pts[1..] {
                    pb.line_to(p.0 as f32, p.1 as f32);
                }
                pb.close();
                end += 1;
            }
            if let Some(path) = pb.finish() {
                let paint = paints[k].clone();
                // A same-color hairline hides anti-aliasing seams between pieces.
                let seam = paint
                    .as_ref()
                    .and_then(|p| p.representative())
                    .map(|c| Stroke {
                        paint: Paint::Solid(c),
                        width: if wire { 0.75 } else { 0.4 },
                        dash: Vec::new(),
                        cap: LineCap::Flat,
                        join: LineJoin::Round,
                        head: None,
                        tail: None,
                    });
                let fill = if wire { None } else { paint };
                self.push(path, fill, seam, false);
            }
            start = end;
        }
        // Labels: values on the left edge, categories along the front, series
        // along the right side (or the left edge in 2-D).
        if vax.visible_labels() && three_d {
            let lists = vax.lists.clone();
            for t in &edges {
                let text = format_value(*t, &vax.fmt, self.date1904);
                if let Some(l) = self.label(&text, &lists, 0.0, None) {
                    let p = proj(0.0, 0.0, *t);
                    self.draw_label(&l, p.0 - 3.0 - l.w / 2.0, p.1);
                }
            }
        }
        let skip_of = |a: &Option<Rc<XNode>>| {
            a.as_ref()
                .and_then(|a| val_f(a, "c:tickLblSkip"))
                .filter(|v| *v >= 1.0)
                .map(|v| v as usize)
        };
        if shown(&cat_node) {
            let a = proj(0.0, 0.0, vmin);
            let b = proj(1.0, 0.0, vmin);
            let edge_len = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
            let probe = cat_texts
                .iter()
                .map(|t| t.chars().count())
                .max()
                .unwrap_or(1) as f64;
            let auto = ((probe * size * 0.6 + size) * n as f64 / edge_len.max(1.0)).ceil() as usize;
            let step = skip_of(&cat_node).unwrap_or(auto.max(1));
            for i in (0..n).step_by(step.max(1)) {
                if let Some(l) = self.label(&cat_texts[i], &cat_lists, 0.0, None) {
                    let p = proj(gx(i), 0.0, vmin);
                    self.draw_label(&l, p.0, p.1 + 3.0 + l.h / 2.0);
                }
            }
        }
        if shown(&ser_node) || !three_d {
            let a = proj(1.0, 0.0, vmin);
            let b = proj(1.0, 1.0, vmin);
            let edge_len = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
            let ssize = resolve_para(&ser_lists, 0, None)
                .def_run
                .size
                .unwrap_or(10.0);
            let auto = (ssize * 1.3 * m as f64 / edge_len.max(1.0)).ceil() as usize;
            let step = skip_of(&ser_node).unwrap_or(auto.max(1));
            for j in (0..m).step_by(step.max(1)) {
                if let Some(l) = self.label(&ser_texts[j], &ser_lists, 0.0, None) {
                    if three_d {
                        let p = proj(1.0, gz(j), vmin);
                        self.draw_label(&l, p.0 + 3.0 + l.w / 2.0, p.1);
                    } else {
                        let p = proj(0.0, gz(j), vmin);
                        self.draw_label(&l, p.0 - 3.0 - l.w / 2.0, p.1);
                    }
                }
            }
        }
        if let Some(t) = &titles.0 {
            let (_, eh) = t.ext();
            self.draw_label(t, inner.x + inner.w / 2.0, rect.y + rect.h - eh / 2.0);
        }
        if let Some(t) = &titles.1 {
            let (ew, _) = t.ext();
            self.draw_label(t, rect.x + ew / 2.0, inner.y + inner.h / 2.0);
        }
        if let Some(t) = &titles.2 {
            let (ew, _) = t.ext();
            self.draw_label(t, rect.x + rect.w - ew / 2.0, inner.y + inner.h / 2.0);
        }
    }
}
