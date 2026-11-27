//! Radar charts (`c:radarChart`: standard, marker and filled styles).
//!
//! Categories are spokes starting straight up and running clockwise; the value
//! axis lies along the first spoke with its labels to its left. Value
//! gridlines are polygons through every major tick; category gridlines (and a
//! visible category axis) are the spokes. Category labels sit just outside the
//! outer polygon, pushed away from the center by half their size.

use super::*;

impl Renderer<'_, '_> {
    pub(super) fn draw_radar(
        &mut self,
        groups: &[&Grp],
        plot: Option<&XNode>,
        rect: Rect,
        manual: bool,
        out: &mut Vec<DLabel>,
    ) {
        let Some(g0) = groups.first() else { return };
        let find = |name: &str, id: &Option<String>| {
            plot.and_then(|p| {
                p.children_named(name)
                    .find(|a| id.is_none() || val(a, "c:axId") == id.as_deref())
                    .cloned()
            })
        };
        let cat_node = find("c:catAx", &g0.ax.0).or_else(|| find("c:dateAx", &g0.ax.0));
        let val_node = find("c:valAx", &g0.ax.1);
        let mut cat = match &cat_node {
            Some(n) => Self::read_axis(n, AxKind::Cat),
            None => synth_axis(AxKind::Cat, 'b'),
        };
        let mut vax = match &val_node {
            Some(n) => Self::read_axis(n, AxKind::Val),
            None => synth_axis(AxKind::Val, 'l'),
        };
        vax.horizontal = false;
        for a in [&mut cat, &mut vax] {
            let txpr = a.node.as_ref().and_then(|n| n.child("c:txPr").cloned());
            a.lists = self.lists(TextKind::Body, &[txpr.as_ref()]);
        }
        let n = groups
            .iter()
            .map(|g| g.n_points())
            .max()
            .unwrap_or(0)
            .min(MAX_POINTS);
        if n == 0 {
            return;
        }
        // Value range and number format.
        for g in groups {
            for s in &g.sers {
                for v in s.vals.iter().take(n).flatten() {
                    if !vax.has_data {
                        (vax.lo, vax.hi, vax.has_data) = (*v, *v, true);
                    } else {
                        vax.lo = vax.lo.min(*v);
                        vax.hi = vax.hi.max(*v);
                    }
                }
            }
        }
        let nf = vax.node.as_ref().and_then(|a| a.child("c:numFmt").cloned());
        vax.fmt = match &nf {
            Some(f) if f.attr_bool("sourceLinked") != Some(true) => {
                f.attr("formatCode").unwrap_or("General").to_string()
            }
            _ => g0
                .sers
                .first()
                .map(|s| s.fmt.clone())
                .unwrap_or_else(|| "General".into()),
        };
        // Category labels, measured before sizing the web.
        let src = groups
            .iter()
            .flat_map(|g| g.sers.iter())
            .find(|s| !s.s.categories.is_empty());
        let mut cat_labels: Vec<Option<Label>> = Vec::new();
        if cat.visible_labels() {
            let lists = &cat.lists;
            for i in 0..n {
                let text = match src {
                    Some(s) => self.cat_text(s, i, None),
                    None => (i + 1).to_string(),
                };
                cat_labels.push(self.label(text.trim_end(), lists, 0.0, None));
            }
        }
        let size = resolve_para(&cat.lists, 0, None)
            .def_run
            .size
            .unwrap_or(10.0);
        let gap = size * 0.4;
        let (cx, cy) = (rect.x + rect.w / 2.0, rect.y + rect.h / 2.0);
        let radius = if manual {
            rect.w.min(rect.h) / 2.0
        } else {
            let mw = cat_labels
                .iter()
                .flatten()
                .map(|l| l.w)
                .fold(0.0f64, f64::max);
            let mh = cat_labels
                .iter()
                .flatten()
                .map(|l| l.h)
                .fold(0.0f64, f64::max);
            ((rect.w - 2.0 * (mw + gap)) / 2.0)
                .min((rect.h - 2.0 * (mh + gap)) / 2.0)
                .max(rect.w.min(rect.h) * 0.15)
        };
        if !(radius.is_finite() && radius > 1.0) {
            return;
        }
        self.scale_axis(&mut vax, radius);
        let reversed = cat.reversed;
        let angle = |i: usize| {
            let a = std::f64::consts::TAU * i as f64 / n as f64;
            if reversed { -a } else { a }
        };
        let rad = |v: f64| vax.frac(v).clamp(0.0, 1.0) * radius;
        let at = |i: usize, r: f64| {
            let a = angle(i);
            (cx + r * a.sin(), cy - r * a.cos())
        };
        let closed = |pts: &[(f64, f64)]| {
            let mut pb = PathBuilder::new();
            pb.move_to(pts[0].0 as f32, pts[0].1 as f32);
            for p in &pts[1..] {
                pb.line_to(p.0 as f32, p.1 as f32);
            }
            pb.close();
            pb.finish()
        };
        // Value gridlines (polygons) and spokes.
        if let Some(gl) = vax
            .node
            .as_ref()
            .and_then(|a| a.child("c:majorGridlines").cloned())
        {
            let st = self.stroke_or(
                gl.child("c:spPr").map(|s| s.as_ref()),
                Some((self.line_default(true), 0.75)),
            );
            for t in vax.ticks() {
                let r = rad(t);
                if r <= 0.01 {
                    continue;
                }
                let pts: Vec<(f64, f64)> = (0..n).map(|i| at(i, r)).collect();
                if let Some(p) = closed(&pts) {
                    self.push(p, None, st.clone(), false);
                }
            }
        }
        let cat_grid = cat
            .node
            .as_ref()
            .and_then(|a| a.child("c:majorGridlines").cloned());
        let spoke = match &cat_grid {
            Some(gl) => self.stroke_or(
                gl.child("c:spPr").map(|s| s.as_ref()),
                Some((self.line_default(true), 0.75)),
            ),
            None if !cat.deleted => {
                let sp = cat.node.as_ref().and_then(|a| a.child("c:spPr").cloned());
                self.stroke_or(sp.as_deref(), Some((self.line_default(false), 0.75)))
            }
            None => None,
        };
        if spoke.is_some() {
            for i in 0..n {
                self.push_line(&[(cx, cy), at(i, radius)], spoke.clone());
            }
        }
        // Series.
        for g in groups {
            let filled = matches!(g.kind, Kind::Radar { filled: true });
            for s in &g.sers {
                let pts: Vec<Option<(f64, f64)>> = (0..n)
                    .map(|i| s.vals.get(i).copied().flatten().map(|v| at(i, rad(v))))
                    .collect();
                let color = self.series_color(s, !filled);
                if filled {
                    let poly: Vec<(f64, f64)> = pts.iter().flatten().copied().collect();
                    if poly.len() >= 3 {
                        let fill = self.point_fill(g, s, usize::MAX, 1, rect);
                        let stroke = self.stroke_or(s.s.sp_pr.as_deref(), None);
                        if let Some(p) = closed(&poly) {
                            self.push(p, fill, stroke, false);
                        }
                    }
                } else {
                    let stroke = self.line_stroke(s, color, true);
                    if pts.iter().all(Option::is_some) && n >= 2 {
                        let poly: Vec<(f64, f64)> = pts.iter().flatten().copied().collect();
                        if let Some(p) = closed(&poly) {
                            self.push(p, None, stroke, false);
                        }
                    } else {
                        // Blank points break the outline.
                        for i in 0..n {
                            if let (Some(a), Some(b)) = (pts[i], pts[(i + 1) % n]) {
                                self.push_line(&[a, b], stroke.clone());
                            }
                        }
                    }
                }
                let marker = if filled {
                    None
                } else {
                    self.marker_for(g, s, color)
                };
                for (i, p) in pts.iter().enumerate() {
                    let Some((x, y)) = *p else { continue };
                    let mk = self.point_marker(s, i, marker.clone(), color);
                    if let Some(m) = &mk {
                        self.draw_marker(m, x, y);
                    }
                    let ms = mk.as_ref().map(|m| m.size).unwrap_or(0.0);
                    let raw = s.vals.get(i).copied().flatten().unwrap_or(0.0);
                    let a = angle(i);
                    let outward = if a.sin().abs() > a.cos().abs() {
                        if a.sin() > 0.0 { "r" } else { "l" }
                    } else if a.cos() > 0.0 {
                        "t"
                    } else {
                        "b"
                    };
                    self.collect_dlabel(
                        g,
                        s,
                        i,
                        n,
                        raw,
                        |pos| Anchor::Point {
                            x,
                            y,
                            pos: pos.unwrap_or(outward).to_string(),
                            marker: ms,
                        },
                        out,
                    );
                }
            }
        }
        // Value axis line, tick marks and labels along the first spoke.
        if !vax.deleted {
            let sp = vax.node.as_ref().and_then(|a| a.child("c:spPr").cloned());
            let st = self.stroke_or(sp.as_deref(), Some((self.line_default(false), 0.75)));
            self.push_line(&[(cx, cy), (cx, cy - radius)], st.clone());
            let tick = match vax.major_tick.as_str() {
                "cross" => Some((-TICK, TICK)),
                "out" => Some((-TICK, 0.0)),
                "in" => Some((0.0, TICK)),
                _ => None,
            };
            if let Some((t0, t1)) = tick {
                for t in vax.ticks() {
                    let y = cy - rad(t);
                    self.push_line(&[(cx + t0, y), (cx + t1, y)], st.clone());
                }
            }
            if vax.visible_labels() {
                let lists = vax.lists.clone();
                let vsize = resolve_para(&lists, 0, None).def_run.size.unwrap_or(10.0);
                for t in vax.ticks() {
                    let text = format_value(t, &vax.fmt, self.date1904);
                    if let Some(l) = self.label(&text, &lists, 0.0, None) {
                        let y = cy - rad(t);
                        self.draw_label(&l, cx - vsize * 0.4 - TICK - l.w / 2.0, y);
                    }
                }
            }
        }
        for (i, l) in cat_labels.iter().enumerate() {
            let Some(l) = l else { continue };
            let a = angle(i);
            let (ux, uy) = (a.sin(), -a.cos());
            let (ex, ey) = at(i, radius + gap);
            self.draw_label(l, ex + ux * l.w / 2.0, ey + uy * l.h / 2.0);
        }
    }
}
