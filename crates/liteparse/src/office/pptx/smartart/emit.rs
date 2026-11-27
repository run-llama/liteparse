//! Turns the placed layout nodes into a SmartArt drawing (`dsp:sp` shapes in a
//! `dsp:spTree`), the same form PowerPoint saves in `drawingN.xml`, so the slide
//! walker renders it like a saved drawing. Colors come from the colors part
//! (`dgm:colorsDef`) by each node's style label and index, line/fill/effect/font
//! style references from the quick style part (`dgm:styleDef`).

use std::rc::Rc;

use super::build::{LNode, ShapeDef, Tree};
use super::data::DataModel;
use super::layout::{Conn, Engine, Rect};
use crate::office::pptx::color::{self, ColorScheme, Rgba};
use crate::office::pptx::xml::XNode;

pub const EMU_PER_MM: f64 = 36_000.0;
const EMU_PER_PT: f64 = 12_700.0;

pub struct Ctx<'a> {
    pub tree: &'a Tree,
    pub dm: &'a DataModel,
    pub colors: Option<Rc<XNode>>,
    pub style: Option<Rc<XNode>>,
    pub scheme: &'a ColorScheme,
    /// Per node: (index, count) among nodes sharing its style label.
    pub style_idx: Vec<(usize, usize)>,
    /// Per node: its style label, from the layout or else the data model.
    pub labels: Vec<String>,
}

pub fn el(name: &str, attrs: Vec<(&str, String)>, children: Vec<Rc<XNode>>) -> Rc<XNode> {
    Rc::new(XNode {
        name: name.to_string(),
        attrs: attrs.into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
        children,
        text: String::new(),
    })
}

fn emu(mm: f64) -> String {
    ((mm * EMU_PER_MM).round() as i64).to_string()
}

impl<'a> Ctx<'a> {
    pub fn new(
        tree: &'a Tree,
        dm: &'a DataModel,
        colors: Option<Rc<XNode>>,
        style: Option<Rc<XNode>>,
        scheme: &'a ColorScheme,
    ) -> Ctx<'a> {
        let n = tree.nodes.len();
        let labels: Vec<String> = tree
            .nodes
            .iter()
            .map(|node| {
                if !node.style_lbl.is_empty() {
                    return node.style_lbl.clone();
                }
                let pres = dm
                    .pres_point(node.pt, &node.name)
                    .and_then(|p| p.pr_set.as_ref())
                    .and_then(|ps| ps.attr("presStyleLbl"))
                    .filter(|l| !l.is_empty());
                match pres {
                    Some(l) => l.to_string(),
                    None if node.alg.kind == "conn" => {
                        if node.alg.param("dim") == Some("1D") {
                            "parChTrans1D1".into()
                        } else {
                            "sibTrans2D1".into()
                        }
                    }
                    None => "node1".into(),
                }
            })
            .collect();
        let mut style_idx = vec![(0, 1); n];
        let mut counts: std::collections::HashMap<&str, Vec<usize>> = Default::default();
        for (i, l) in labels.iter().enumerate() {
            counts.entry(l.as_str()).or_default().push(i);
        }
        for members in counts.values() {
            for (k, &i) in members.iter().enumerate() {
                style_idx[i] = (k, members.len());
            }
        }
        // PowerPoint's own numbering, when the data model kept it.
        for (i, node) in tree.nodes.iter().enumerate() {
            if let Some(ps) = dm
                .pres_point(node.pt, &node.name)
                .and_then(|p| p.pr_set.as_ref())
                && let (Some(idx), Some(cnt)) =
                    (ps.attr_i64("presStyleIdx"), ps.attr_i64("presStyleCnt"))
                && cnt > 0
                && idx >= 0
            {
                style_idx[i] = (idx as usize, cnt as usize);
            }
        }
        Ctx {
            tree,
            dm,
            colors,
            style,
            scheme,
            style_idx,
            labels,
        }
    }

    fn node(&self, n: usize) -> &LNode {
        &self.tree.nodes[n]
    }

    /// Color `kind` (`fillClrLst`, `linClrLst`, `txFillClrLst`) of a node.
    fn color(&self, n: usize, kind: &str) -> Option<Rc<XNode>> {
        let lbl = &self.labels[n];
        let list = self
            .colors
            .as_ref()?
            .children_named("dgm:styleLbl")
            .find(|s| s.attr("name") == Some(lbl.as_str()))?
            .child(&format!("dgm:{kind}"))?
            .clone();
        let colors: Vec<&Rc<XNode>> = list
            .children
            .iter()
            .filter(|c| c.name.starts_with("a:") && c.name.ends_with("Clr"))
            .collect();
        if colors.is_empty() {
            return None;
        }
        let (idx, cnt) = self.style_idx[n];
        match list.attr("meth").unwrap_or("repeat") {
            "span" if colors.len() > 1 && cnt > 1 => {
                let t = idx.min(cnt - 1) as f64 / (cnt - 1) as f64 * (colors.len() - 1) as f64;
                let i = (t.floor() as usize).min(colors.len() - 2);
                let f = (t - i as f64) as f32;
                let a = color::resolve(colors[i], self.scheme, None)?;
                let b = color::resolve(colors[i + 1], self.scheme, None)?;
                let mix = |x: f32, y: f32| x + (y - x) * f;
                Some(srgb(Rgba {
                    r: mix(a.r, b.r),
                    g: mix(a.g, b.g),
                    b: mix(a.b, b.b),
                    a: mix(a.a, b.a),
                }))
            }
            _ => Some(colors[idx % colors.len()].clone()),
        }
    }

    /// The quick style's `dgm:style` for the node's label.
    fn style_refs(&self, n: usize) -> Option<Rc<XNode>> {
        let lbl = &self.labels[n];
        self.style
            .as_ref()?
            .children_named("dgm:styleLbl")
            .find(|s| s.attr("name") == Some(lbl.as_str()))?
            .child("dgm:style")
            .cloned()
    }

    /// The user's shape properties for a node: its pres point's, else (for a node
    /// that shows its own point) the data point's.
    fn user_sp_pr(&self, n: usize) -> Option<Rc<XNode>> {
        let node = self.node(n);
        let pres = self
            .dm
            .pres_point(node.pt, &node.name)
            .and_then(|p| p.sp_pr.clone())
            .filter(|s| !s.children.is_empty());
        pres.or_else(|| {
            node.text
                .iter()
                .any(|&(p, _)| p == node.pt)
                .then(|| self.dm.pts[node.pt].sp_pr.clone())
                .flatten()
                .filter(|s| !s.children.is_empty())
        })
    }

    fn custom_text(&self, pt: usize) -> bool {
        self.dm.pts[pt]
            .pr_set
            .as_ref()
            .and_then(|p| p.attr_bool("custT"))
            .unwrap_or(false)
    }

    /// Fixed font size of a node whose text sizes the user set, if all its text is.
    pub fn fixed_font(&self, n: usize) -> Option<f64> {
        let node = self.node(n);
        let pts: Vec<usize> = node
            .text
            .iter()
            .filter(|&&(p, _)| self.dm.has_text(p))
            .map(|&(p, _)| p)
            .collect();
        if pts.is_empty() || !pts.iter().all(|&p| self.custom_text(p)) {
            return None;
        }
        let mut best: Option<f64> = None;
        for &(p, lvl) in &node.text {
            if lvl > 0 {
                continue;
            }
            let mut rprs = Vec::new();
            if let Some(t) = &self.dm.pts[p].text {
                t.find_all("a:rPr", &mut rprs);
            }
            for r in rprs {
                if let Some(sz) = r.attr_f64("sz") {
                    best = Some(best.unwrap_or(0.0).max(sz / 100.0));
                }
            }
        }
        best.or(Some(18.0))
    }

    /// Diagram text uses the theme font its quick style names (minor unless
    /// `major`), not the presentation's default text style.
    fn font_list(&self, n: usize) -> Rc<XNode> {
        let major = self
            .style_refs(n)
            .and_then(|s| {
                s.child("a:fontRef")
                    .and_then(|f| f.attr("idx").map(str::to_string))
            })
            .is_some_and(|i| i == "major");
        let k = if major { "mj" } else { "mn" };
        let levels = (1..=9)
            .map(|l| {
                let fonts = ["latin", "ea", "cs"]
                    .iter()
                    .map(|f| {
                        let t = if *f == "latin" { "lt" } else { f };
                        el(
                            &format!("a:{f}"),
                            vec![("typeface", format!("+{k}-{t}"))],
                            vec![],
                        )
                    })
                    .collect();
                el(
                    &format!("a:lvl{l}pPr"),
                    vec![],
                    vec![el("a:defRPr", vec![], fonts)],
                )
            })
            .collect();
        el("a:lstStyle", vec![], levels)
    }

    /// The text body a node shows at primary font size `fs` with its margins.
    pub fn tx_body(&self, n: usize, fs: f64, margins: [f64; 4]) -> Option<Rc<XNode>> {
        let node = self.node(n);
        let alg = &node.alg;
        let pnum = |k: &str, d: f64| {
            alg.param(k)
                .and_then(|v| v.parse::<f64>().ok())
                .unwrap_or(d)
        };
        let st_bullet = pnum("stBulletLvl", 1.0) as u32;
        let has_child_text = node.text.iter().any(|&(p, l)| l > 0 && self.dm.has_text(p));
        let sec = (fs * 0.78).round().max(1.0);
        let mut paras = Vec::new();
        for &(p, lvl) in &node.text {
            if !self.dm.has_text(p) {
                continue;
            }
            let Some(t) = &self.dm.pts[p].text else {
                continue;
            };
            let cust = self.custom_text(p);
            for para in t.children_named("a:p") {
                let lvl = lvl.min(8);
                let size = if lvl == 0 { fs } else { sec };
                let algn = if lvl == 0 {
                    alg.param("parTxLTRAlign").unwrap_or("ctr")
                } else {
                    alg.param("shpTxLTRAlignCh").unwrap_or("l")
                };
                let (sp_ln, sp_af) = if lvl == 0 {
                    (pnum("lnSpPar", 90.0), pnum("lnSpAfParP", 35.0))
                } else {
                    (pnum("lnSpCh", 90.0), pnum("lnSpAfChP", 15.0))
                };
                let bullet = lvl >= st_bullet && lvl > 0;
                paras.push(paragraph(para, lvl, size, algn, sp_ln, sp_af, bullet, cust));
            }
        }
        if paras.is_empty() {
            return None;
        }
        let anchor = if has_child_text {
            alg.param("txAnchorVertCh").unwrap_or("mid")
        } else {
            alg.param("txAnchorVert").unwrap_or("mid")
        };
        let anchor = match anchor {
            "t" => "t",
            "b" => "b",
            _ => "ctr",
        };
        let ins = |pt: f64| ((pt * EMU_PER_PT).round() as i64).to_string();
        let body_pr = el(
            "a:bodyPr",
            vec![
                ("wrap", "square".into()),
                ("lIns", ins(margins[0])),
                ("tIns", ins(margins[1])),
                ("rIns", ins(margins[2])),
                ("bIns", ins(margins[3])),
                ("anchor", anchor.into()),
                ("anchorCtr", "0".into()),
            ],
            vec![el("a:noAutofit", vec![], vec![])],
        );
        let mut children = vec![body_pr, self.font_list(n)];
        children.extend(paras);
        Some(el("dsp:txBody", vec![], children))
    }

    /// The drawing for a laid-out diagram, in frame coordinates.
    pub fn drawing(&self, e: &Engine) -> Rc<XNode> {
        let mut shapes: Vec<(f64, usize, Vec<Rc<XNode>>)> = Vec::new();
        let mut seq = 0usize;
        for n in 0..self.tree.nodes.len() {
            let Some(r) = e.rect[n] else { continue };
            let node = self.node(n);
            let z = node.shape.z_off as f64;
            let mut out = Vec::new();
            if node.alg.kind == "conn" || node.shape.ty == "conn" {
                if let Some(c) = e.conns.iter().find(|c| c.node == n)
                    && node.shape.ty == "conn"
                    && !node.shape.hide_geom
                {
                    out.extend(self.connector(n, c));
                }
            } else {
                let text = e.group[n].and_then(|_| {
                    let fs = e.font(n);
                    self.tx_body(n, fs, e.margins(n))
                });
                out.extend(self.shape(n, r, text));
            }
            if !out.is_empty() {
                shapes.push((seq as f64 + z, seq, out));
                seq += 1;
            }
        }
        shapes.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        let children = shapes.into_iter().flat_map(|s| s.2).collect();
        el("dsp:spTree", vec![], children)
    }

    fn style_node(&self, n: usize) -> Rc<XNode> {
        let refs = self.style_refs(n);
        let get = |name: &str, idx: &str| -> (String, Option<Rc<XNode>>) {
            let r = refs.as_ref().and_then(|s| s.child(name).cloned());
            let i = r
                .as_ref()
                .and_then(|r| r.attr("idx"))
                .unwrap_or(idx)
                .to_string();
            (i, r.and_then(|r| r.children.first().cloned()))
        };
        let (ln_i, _) = get("a:lnRef", "2");
        let (fill_i, _) = get("a:fillRef", "1");
        let (eff_i, _) = get("a:effectRef", "0");
        let (font_i, font_c) = get("a:fontRef", "minor");
        let fill = self.color(n, "fillClrLst");
        let line = self.color(n, "linClrLst");
        let text = self.color(n, "txFillClrLst").or(font_c);
        let with = |name: &str, idx: String, c: Option<Rc<XNode>>| {
            el(name, vec![("idx", idx)], c.into_iter().collect())
        };
        let fill_i = if fill.is_none() {
            "0".to_string()
        } else {
            fill_i
        };
        let ln_i = if line.is_none() {
            "0".to_string()
        } else {
            ln_i
        };
        el(
            "dsp:style",
            vec![],
            vec![
                with("a:lnRef", ln_i, line),
                with("a:fillRef", fill_i, fill.clone()),
                with("a:effectRef", eff_i, fill),
                with("a:fontRef", font_i, text),
            ],
        )
    }

    fn sp(name: &str, sp_pr: Vec<Rc<XNode>>, style: Rc<XNode>, tx: Option<Rc<XNode>>) -> Rc<XNode> {
        let nv = el(
            "dsp:nvSpPr",
            vec![],
            vec![
                el(
                    "dsp:cNvPr",
                    vec![("id", "0".into()), ("name", name.to_string())],
                    vec![],
                ),
                el("dsp:cNvSpPr", vec![], vec![]),
            ],
        );
        let mut children = vec![nv, el("dsp:spPr", vec![], sp_pr), style];
        children.extend(tx);
        el("dsp:sp", vec![], children)
    }

    fn shape(&self, n: usize, r: Rect, text: Option<Rc<XNode>>) -> Vec<Rc<XNode>> {
        let node = self.node(n);
        let ty = node.shape.ty.as_str();
        let draw_geom = !ty.is_empty() && ty != "none" && !node.shape.hide_geom;
        if !draw_geom && text.is_none() {
            return Vec::new();
        }
        let rot = norm_rot(node.shape.rot);
        let quarter = (rot - 90.0).abs() < 1e-6 || (rot - 270.0).abs() < 1e-6;
        // The layout box is the shape's footprint; a quarter-turned shape's own
        // box has its width and height swapped.
        let (gw, gh) = if quarter { (r.h, r.w) } else { (r.w, r.h) };
        let gx = r.cx() - gw / 2.0;
        let gy = r.cy() - gh / 2.0;
        let style = self.style_node(n);
        let mut out = Vec::new();
        let user = self.user_sp_pr(n);
        let geom = preset(&node.shape);
        if draw_geom {
            let mut sp_pr = vec![xfrm(gx, gy, gw, gh, rot), geom];
            if let Some(u) = &user {
                sp_pr.extend(
                    u.children
                        .iter()
                        .filter(|c| {
                            crate::office::pptx::style::is_fill_element(&c.name) || c.name == "a:ln"
                        })
                        .cloned(),
                );
            }
            let upright = rot.abs() < 1e-6;
            out.push(Self::sp(
                &node.name,
                sp_pr,
                style.clone(),
                if upright { text.clone() } else { None },
            ));
            if upright {
                return out;
            }
        }
        if let Some(tx) = text {
            // Text stays upright over a turned shape, in the turned text box.
            let tr = text_rect_mm(&node.shape, r);
            let sp_pr = vec![
                xfrm(tr.x, tr.y, tr.w, tr.h, 0.0),
                el(
                    "a:prstGeom",
                    vec![("prst", "rect".into())],
                    vec![el("a:avLst", vec![], vec![])],
                ),
                el("a:noFill", vec![], vec![]),
                el("a:ln", vec![], vec![el("a:noFill", vec![], vec![])]),
            ];
            out.push(Self::sp(&node.name, sp_pr, style, Some(tx)));
        }
        out
    }

    fn connector(&self, n: usize, c: &Conn) -> Vec<Rc<XNode>> {
        if let Some(arc) = c.arc {
            return self.arc_shape(n, c, arc);
        }
        let style = self.style_node(n);
        let (dx, dy) = (c.to.0 - c.from.0, c.to.1 - c.from.1);
        let len = (dx * dx + dy * dy).sqrt();
        if len < 0.05 {
            return Vec::new();
        }
        let node = self.node(n);
        if !c.one_d {
            let ang = dy.atan2(dx).to_degrees();
            let (mx, my) = ((c.from.0 + c.to.0) / 2.0, (c.from.1 + c.to.1) / 2.0);
            let geom = el(
                "a:prstGeom",
                vec![("prst", "rightArrow".into())],
                vec![el(
                    "a:avLst",
                    vec![],
                    vec![
                        el(
                            "a:gd",
                            vec![("name", "adj1".into()), ("fmla", "val 60000".into())],
                            vec![],
                        ),
                        el(
                            "a:gd",
                            vec![("name", "adj2".into()), ("fmla", "val 50000".into())],
                            vec![],
                        ),
                    ],
                )],
            );
            let sp_pr = vec![
                xfrm(
                    mx - len / 2.0,
                    my - c.thick / 2.0,
                    len,
                    c.thick,
                    norm_rot(ang),
                ),
                geom,
            ];
            return vec![Self::sp(&node.name, sp_pr, style, None)];
        }
        // 1-D: a polyline (straight, or an elbow) drawn with the line style.
        let mut pts = vec![c.from];
        if let Some(b) = c.bend {
            // An elbow: leave and enter on the sides the end points face.
            match c.vert {
                (true, false) => pts.push((c.from.0, c.to.1)),
                (false, true) => pts.push((c.to.0, c.from.1)),
                (true, true) => {
                    let by = c.from.1 + b.min(dy.abs()) * dy.signum();
                    pts.push((c.from.0, by));
                    pts.push((c.to.0, by));
                }
                (false, false) => {
                    let bx = c.from.0 + b.min(dx.abs()) * dx.signum();
                    pts.push((bx, c.from.1));
                    pts.push((bx, c.to.1));
                }
            }
        }
        pts.push(c.to);
        vec![self.line_shape(n, &pts, style, None)]
    }

    /// An open polyline drawn with the node's line style and its arrowheads
    /// (`begSty`/`endSty`; 1-D connectors end in an arrow unless told not to).
    fn line_shape(
        &self,
        n: usize,
        pts: &[(f64, f64)],
        style: Rc<XNode>,
        frame: Option<Rect>,
    ) -> Rc<XNode> {
        let node = self.node(n);
        let (xf, geom) = poly_geom(pts, false, frame);
        let mut ln_kids = Vec::new();
        let arrow = |k: &str| node.alg.param(k).map(|v| v == "arr");
        if arrow("begSty") == Some(true) {
            ln_kids.push(el("a:headEnd", vec![("type", "triangle".into())], vec![]));
        }
        if arrow("endSty").unwrap_or(true) {
            ln_kids.push(el("a:tailEnd", vec![("type", "triangle".into())], vec![]));
        }
        let mut sp_pr = vec![xf, geom, el("a:noFill", vec![], vec![])];
        if !ln_kids.is_empty() {
            sp_pr.push(el("a:ln", vec![], ln_kids));
        }
        Self::sp(&node.name, sp_pr, style, None)
    }

    /// A connector along a circle: a line arc (1-D) or a band with an arrowhead
    /// at each end whose style asks for one (2-D).
    fn arc_shape(&self, n: usize, c: &Conn, arc: (f64, f64, f64, f64, f64)) -> Vec<Rc<XNode>> {
        let style = self.style_node(n);
        let (cx, cy, r, a0, a1) = arc;
        let at = |rad: f64, a: f64| (cx + rad * a.cos(), cy + rad * a.sin());
        let arc_pts = |rad: f64, from: f64, to: f64| -> Vec<(f64, f64)> {
            let steps = (((to - from).abs() * rad).max(1.0).sqrt() * 4.0).clamp(8.0, 96.0) as usize;
            (0..=steps)
                .map(|i| at(rad, from + (to - from) * i as f64 / steps as f64))
                .collect()
        };
        // The shape box is the whole circle, as PowerPoint's arc shapes have.
        let circle = |rad: f64| Rect {
            x: cx - rad,
            y: cy - rad,
            w: 2.0 * rad,
            h: 2.0 * rad,
        };
        if c.one_d {
            return vec![self.line_shape(n, &arc_pts(r, a0, a1), style, Some(circle(r)))];
        }
        let node = self.node(n);
        let arrow = |k: &str, d: bool| node.alg.param(k).map(|v| v == "arr").unwrap_or(d);
        let t = c.thick.min(r);
        let head = (t / r).min((a1 - a0) / 2.0);
        let (s0, s1) = (
            if arrow("begSty", false) {
                a0 + head
            } else {
                a0
            },
            if arrow("endSty", true) { a1 - head } else { a1 },
        );
        let mut pts = arc_pts(r + t / 2.0, s0, s1);
        if s1 < a1 {
            pts.push(at(r + t, s1));
            pts.push(at(r, a1));
            pts.push(at(r - t, s1));
        }
        pts.extend(arc_pts(r - t / 2.0, s1, s0));
        if s0 > a0 {
            pts.push(at(r - t, s0));
            pts.push(at(r, a0));
            pts.push(at(r + t, s0));
        }
        let (xf, geom) = poly_geom(&pts, true, Some(circle(r + t)));
        vec![Self::sp(&node.name, vec![xf, geom], style, None)]
    }
}

/// `a:xfrm` and `a:custGeom` for a polyline given in frame millimetres.
/// The shape box is the points' bounds, or `frame` (x, y, w, h) when given.
fn poly_geom(pts: &[(f64, f64)], closed: bool, frame: Option<Rect>) -> (Rc<XNode>, Rc<XNode>) {
    let (x0, y0, w, h) = match frame {
        Some(f) => (f.x, f.y, f.w.max(0.01), f.h.max(0.01)),
        None => {
            let x0 = pts.iter().map(|p| p.0).fold(f64::MAX, f64::min);
            let y0 = pts.iter().map(|p| p.1).fold(f64::MAX, f64::min);
            let x1 = pts.iter().map(|p| p.0).fold(f64::MIN, f64::max);
            let y1 = pts.iter().map(|p| p.1).fold(f64::MIN, f64::max);
            (x0, y0, (x1 - x0).max(0.01), (y1 - y0).max(0.01))
        }
    };
    let mut cmds = Vec::new();
    for (i, p) in pts.iter().enumerate() {
        let pt = el(
            "a:pt",
            vec![("x", emu(p.0 - x0)), ("y", emu(p.1 - y0))],
            vec![],
        );
        cmds.push(el(
            if i == 0 { "a:moveTo" } else { "a:lnTo" },
            vec![],
            vec![pt],
        ));
    }
    let mut attrs = vec![("w", emu(w)), ("h", emu(h))];
    if closed {
        cmds.push(el("a:close", vec![], vec![]));
    } else {
        attrs.push(("fill", "none".into()));
    }
    let path = el("a:path", attrs, cmds);
    let geom = el(
        "a:custGeom",
        vec![],
        vec![
            el("a:avLst", vec![], vec![]),
            el("a:gdLst", vec![], vec![]),
            el("a:pathLst", vec![], vec![path]),
        ],
    );
    (xfrm(x0, y0, w, h, 0.0), geom)
}

fn norm_rot(r: f64) -> f64 {
    let r = r % 360.0;
    if r < 0.0 { r + 360.0 } else { r }
}

fn srgb(c: Rgba) -> Rc<XNode> {
    let h = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    el(
        "a:srgbClr",
        vec![("val", format!("{:02X}{:02X}{:02X}", h(c.r), h(c.g), h(c.b)))],
        vec![el(
            "a:alpha",
            vec![(
                "val",
                ((c.a.clamp(0.0, 1.0) * 100000.0).round() as i64).to_string(),
            )],
            vec![],
        )],
    )
}

fn xfrm(x: f64, y: f64, w: f64, h: f64, rot: f64) -> Rc<XNode> {
    let mut attrs = Vec::new();
    if rot.abs() > 1e-6 {
        attrs.push(("rot", ((rot * 60_000.0).round() as i64).to_string()));
    }
    el(
        "a:xfrm",
        attrs,
        vec![
            el("a:off", vec![("x", emu(x)), ("y", emu(y))], vec![]),
            el(
                "a:ext",
                vec![("cx", emu(w.max(0.0))), ("cy", emu(h.max(0.0)))],
                vec![],
            ),
        ],
    )
}

/// `a:prstGeom` for a layout shape, with its adjust values. Layout adjust values
/// are fractions (×100000 in DrawingML) or, for angles, degrees.
pub fn preset(shape: &ShapeDef) -> Rc<XNode> {
    let ty = if shape.ty.is_empty() || shape.ty == "none" {
        "rect"
    } else {
        shape.ty.as_str()
    };
    let mut gds = Vec::new();
    for &(idx, v) in &shape.adj {
        let val = if v.abs() > 4.0 {
            (v * 60_000.0).round() as i64
        } else {
            (v * 100_000.0).round() as i64
        };
        let mut names = vec![format!("adj{idx}")];
        if idx == 1 {
            names.push("adj".into());
        }
        for name in names {
            gds.push(el(
                "a:gd",
                vec![("name", name), ("fmla", format!("val {val}"))],
                vec![],
            ));
        }
    }
    el(
        "a:prstGeom",
        vec![("prst", ty.to_string())],
        vec![el("a:avLst", vec![], gds)],
    )
}

/// The text box of a laid-out shape, in mm: its geometry's text rectangle,
/// turned with the shape (text itself stays upright).
pub fn text_rect_mm(shape: &ShapeDef, r: Rect) -> Rect {
    let rot = norm_rot(shape.rot);
    let quarter = (rot - 90.0).abs() < 1e-6 || (rot - 270.0).abs() < 1e-6;
    let (gw, gh) = if quarter { (r.h, r.w) } else { (r.w, r.h) };
    if shape.hide_geom || shape.ty.is_empty() || shape.ty == "conn" {
        return r;
    }
    let g =
        crate::office::pptx::geom::build(Some(&preset(shape)), gw * EMU_PER_MM, gh * EMU_PER_MM);
    let to_mm = EMU_PER_PT / EMU_PER_MM;
    let (l, t, rr, b) = (
        g.text_rect.l * to_mm,
        g.text_rect.t * to_mm,
        g.text_rect.r * to_mm,
        g.text_rect.b * to_mm,
    );
    // Corners of the text rect around the shape centre, turned by `rot`.
    let (cx, cy) = (gw / 2.0, gh / 2.0);
    let (s, c) = rot.to_radians().sin_cos();
    let mut x0 = f64::MAX;
    let mut y0 = f64::MAX;
    let mut x1 = f64::MIN;
    let mut y1 = f64::MIN;
    for (px, py) in [(l, t), (rr, t), (rr, b), (l, b)] {
        let (dx, dy) = (px - cx, py - cy);
        let (qx, qy) = (dx * c - dy * s, dx * s + dy * c);
        x0 = x0.min(qx);
        y0 = y0.min(qy);
        x1 = x1.max(qx);
        y1 = y1.max(qy);
    }
    Rect {
        x: r.cx() + x0,
        y: r.cy() + y0,
        w: x1 - x0,
        h: y1 - y0,
    }
}

/// One paragraph of node text, restyled as PowerPoint draws diagram text: level,
/// alignment, 90% line spacing, space after, bullets below the first level, and
/// the fitted font size unless the user sized the text (`custT`).
#[expect(
    clippy::too_many_arguments,
    reason = "each argument is one paragraph property the diagram restyles"
)]
fn paragraph(
    p: &XNode,
    lvl: u32,
    size: f64,
    algn: &str,
    ln_sp: f64,
    sp_af: f64,
    bullet: bool,
    cust: bool,
) -> Rc<XNode> {
    let user = p.child("a:pPr");
    let algn = user
        .and_then(|u| u.attr("algn"))
        .unwrap_or(algn)
        .to_string();
    let indent = 114_300 * lvl.min(8) as i64;
    let mut attrs = vec![("lvl", lvl.to_string()), ("algn", algn)];
    if bullet {
        attrs.push(("marL", indent.to_string()));
        attrs.push(("indent", (-114_300).to_string()));
    } else {
        attrs.push(("marL", "0".into()));
        attrs.push(("indent", "0".into()));
    }
    let pct = |name: &str, v: f64| {
        el(
            name,
            vec![],
            vec![el(
                "a:spcPct",
                vec![("val", ((v * 1000.0).round() as i64).to_string())],
                vec![],
            )],
        )
    };
    let mut kids = vec![
        pct("a:lnSpc", ln_sp),
        pct("a:spcBef", 0.0),
        pct("a:spcAft", sp_af),
    ];
    kids.push(if bullet {
        el("a:buChar", vec![("char", "•".into())], vec![])
    } else {
        el("a:buNone", vec![], vec![])
    });
    let ppr = el("a:pPr", attrs, kids);
    let sz = ((size * 100.0).round() as i64).max(100).to_string();
    let mut children = vec![ppr];
    for c in &p.children {
        match c.name.as_str() {
            "a:pPr" => {}
            "a:r" | "a:fld" | "a:br" | "a:endParaRPr" => {
                let mut node = (**c).clone();
                let set_sz = |rpr: &mut XNode| {
                    if !cust || rpr.attr("sz").is_none() {
                        rpr.attrs.retain(|(k, _)| k != "sz");
                        rpr.attrs.push(("sz".into(), sz.clone()));
                    }
                };
                if node.name == "a:endParaRPr" {
                    set_sz(&mut node);
                } else {
                    let pos = node.children.iter().position(|k| k.name == "a:rPr");
                    match pos {
                        Some(i) => {
                            let mut rpr = (*node.children[i]).clone();
                            set_sz(&mut rpr);
                            node.children[i] = Rc::new(rpr);
                        }
                        None => {
                            let mut rpr = XNode {
                                name: "a:rPr".into(),
                                attrs: Vec::new(),
                                children: Vec::new(),
                                text: String::new(),
                            };
                            set_sz(&mut rpr);
                            node.children.insert(0, Rc::new(rpr));
                        }
                    }
                }
                children.push(Rc::new(node));
            }
            _ => children.push(c.clone()),
        }
    }
    el("a:p", vec![], children)
}
