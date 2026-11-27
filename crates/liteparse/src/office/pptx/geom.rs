//! Shape geometry: evaluates DrawingML preset shapes (from the ECMA-376
//! `presetShapeDefinitions.xml`, vendored under `assets/`) and custom geometry
//! (`a:custGeom`) into paths in shape-local points.
//!
//! Guide math works in EMU, as the spec defines it; angles are in 60000ths of a
//! degree. `arcTo` angles are *visual* angles on the scaled ellipse, converted to
//! parametric angles before emitting Béziers, matching Office.

use std::collections::HashMap;
use std::rc::Rc;

use crate::office::pptx::xml::{self, XNode};

const PRESETS_XML: &str = include_str!("assets/presetShapeDefinitions.xml");

thread_local! {
    static PRESETS: HashMap<String, Rc<XNode>> = load_presets();
}

fn load_presets() -> HashMap<String, Rc<XNode>> {
    // The file has no namespace on its root and per-element default namespaces,
    // so canonical names come out as `a:avLst`/`a:gdLst`/... under a bare root.
    let root = xml::parse(PRESETS_XML).expect("vendored preset definitions parse");
    root.children
        .iter()
        .map(|c| (c.name.clone(), c.clone()))
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Cmd {
    Move(f64, f64),
    Line(f64, f64),
    Quad(f64, f64, f64, f64),
    Cubic(f64, f64, f64, f64, f64, f64),
    Close,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FillMode {
    None,
    Norm,
    Lighten,
    LightenLess,
    Darken,
    DarkenLess,
}

#[derive(Debug, Clone)]
pub struct GeomPath {
    /// Commands in shape-local points (origin at the shape's top-left).
    pub cmds: Vec<Cmd>,
    pub fill: FillMode,
    pub stroke: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct TextRect {
    pub l: f64,
    pub t: f64,
    pub r: f64,
    pub b: f64,
}

#[derive(Debug, Clone)]
pub struct Geometry {
    pub paths: Vec<GeomPath>,
    /// Text rectangle in shape-local points.
    pub text_rect: TextRect,
}

const EMU_PER_PT: f64 = 12_700.0;

/// Builds the geometry of a shape of size `w`×`h` EMU from its `a:prstGeom` or
/// `a:custGeom` element. Unknown presets fall back to a rectangle.
pub fn build(geom: Option<&XNode>, w: f64, h: f64) -> Geometry {
    let rect = || rect_geometry(w, h);
    let Some(geom) = geom else {
        return rect();
    };
    match geom.name.as_str() {
        "a:prstGeom" => {
            let prst = geom.attr("prst").unwrap_or("rect");
            if prst == "rect" {
                return rect();
            }
            let def = PRESETS.with(|p| p.get(prst).cloned());
            match def {
                Some(def) => evaluate(&def, geom.child("a:avLst").map(|n| n.as_ref()), w, h),
                None => rect(),
            }
        }
        "a:custGeom" => evaluate(geom, geom.child("a:avLst").map(|n| n.as_ref()), w, h),
        _ => rect(),
    }
}

pub fn rect_geometry(w: f64, h: f64) -> Geometry {
    let (wp, hp) = (w / EMU_PER_PT, h / EMU_PER_PT);
    Geometry {
        paths: vec![GeomPath {
            cmds: vec![
                Cmd::Move(0.0, 0.0),
                Cmd::Line(wp, 0.0),
                Cmd::Line(wp, hp),
                Cmd::Line(0.0, hp),
                Cmd::Close,
            ],
            fill: FillMode::Norm,
            stroke: true,
        }],
        text_rect: TextRect {
            l: 0.0,
            t: 0.0,
            r: wp,
            b: hp,
        },
    }
}

struct Guides {
    vars: HashMap<String, f64>,
}

impl Guides {
    fn new(w: f64, h: f64) -> Guides {
        let ss = w.min(h);
        let ls = w.max(h);
        let mut vars = HashMap::new();
        let mut set = |k: &str, v: f64| {
            vars.insert(k.to_string(), v);
        };
        set("w", w);
        set("h", h);
        set("l", 0.0);
        set("t", 0.0);
        set("r", w);
        set("b", h);
        set("hc", w / 2.0);
        set("vc", h / 2.0);
        set("ss", ss);
        set("ls", ls);
        for d in [2.0, 3.0, 4.0, 5.0, 6.0, 8.0, 10.0, 12.0, 16.0, 32.0] {
            set(&format!("wd{d}"), w / d);
            set(&format!("hd{d}"), h / d);
            set(&format!("ssd{d}"), ss / d);
        }
        set("cd2", 10_800_000.0);
        set("cd4", 5_400_000.0);
        set("cd8", 2_700_000.0);
        set("3cd4", 16_200_000.0);
        set("3cd8", 8_100_000.0);
        set("5cd8", 13_500_000.0);
        set("7cd8", 18_900_000.0);
        Guides { vars }
    }

    fn get(&self, token: &str) -> f64 {
        if let Some(v) = self.vars.get(token) {
            return *v;
        }
        token.parse::<f64>().unwrap_or(0.0)
    }

    fn eval(&self, fmla: &str) -> f64 {
        let parts: Vec<&str> = fmla.split_whitespace().collect();
        let Some((op, args)) = parts.split_first() else {
            return 0.0;
        };
        let a = |i: usize| args.get(i).map(|t| self.get(t)).unwrap_or(0.0);
        let rad = |v: f64| (v / 60_000.0).to_radians();
        match *op {
            "val" => a(0),
            "*/" => {
                let c = a(2);
                if c == 0.0 { 0.0 } else { a(0) * a(1) / c }
            }
            "+-" => a(0) + a(1) - a(2),
            "+/" => {
                let c = a(2);
                if c == 0.0 { 0.0 } else { (a(0) + a(1)) / c }
            }
            "?:" => {
                if a(0) > 0.0 {
                    a(1)
                } else {
                    a(2)
                }
            }
            "abs" => a(0).abs(),
            "at2" => a(1).atan2(a(0)).to_degrees() * 60_000.0,
            "cat2" => a(0) * a(2).atan2(a(1)).cos(),
            "sat2" => a(0) * a(2).atan2(a(1)).sin(),
            "cos" => a(0) * rad(a(1)).cos(),
            "sin" => a(0) * rad(a(1)).sin(),
            "tan" => a(0) * rad(a(1)).tan(),
            "max" => a(0).max(a(1)),
            "min" => a(0).min(a(1)),
            "mod" => (a(0) * a(0) + a(1) * a(1) + a(2) * a(2)).sqrt(),
            "pin" => {
                let (lo, v, hi) = (a(0), a(1), a(2));
                if v < lo {
                    lo
                } else if v > hi {
                    hi
                } else {
                    v
                }
            }
            "sqrt" => a(0).max(0.0).sqrt(),
            _ => 0.0,
        }
    }

    fn apply_list(&mut self, list: Option<&XNode>) {
        let Some(list) = list else { return };
        for gd in list.children_named("a:gd") {
            let (Some(name), Some(fmla)) = (gd.attr("name"), gd.attr("fmla")) else {
                continue;
            };
            let v = self.eval(fmla);
            self.vars.insert(name.to_string(), v);
        }
    }
}

pub(crate) fn evaluate(def: &XNode, overrides: Option<&XNode>, w: f64, h: f64) -> Geometry {
    let mut g = Guides::new(w, h);
    g.apply_list(def.child("a:avLst").map(|n| n.as_ref()));
    // Shape-instance adjust values replace the defaults by name.
    g.apply_list(overrides);
    g.apply_list(def.child("a:gdLst").map(|n| n.as_ref()));

    let to_pt = 1.0 / EMU_PER_PT;
    let text_rect = match def.child("a:rect") {
        Some(r) => TextRect {
            l: g.get(r.attr("l").unwrap_or("l")) * to_pt,
            t: g.get(r.attr("t").unwrap_or("t")) * to_pt,
            r: g.get(r.attr("r").unwrap_or("r")) * to_pt,
            b: g.get(r.attr("b").unwrap_or("b")) * to_pt,
        },
        None => TextRect {
            l: 0.0,
            t: 0.0,
            r: w * to_pt,
            b: h * to_pt,
        },
    };

    let mut paths = Vec::new();
    if let Some(list) = def.child("a:pathLst") {
        for p in list.children_named("a:path") {
            paths.push(eval_path(p, &g, w, h));
        }
    }
    Geometry { paths, text_rect }
}

fn eval_path(p: &XNode, g: &Guides, w: f64, h: f64) -> GeomPath {
    // A path may declare its own coordinate space (w/h); points scale into the shape.
    let pw = p.attr_f64("w").filter(|v| *v > 0.0);
    let ph = p.attr_f64("h").filter(|v| *v > 0.0);
    let sx = pw.map(|pw| w / pw).unwrap_or(1.0) / EMU_PER_PT;
    let sy = ph.map(|ph| h / ph).unwrap_or(1.0) / EMU_PER_PT;
    let fill = match p.attr("fill") {
        Some("none") => FillMode::None,
        Some("lighten") => FillMode::Lighten,
        Some("lightenLess") => FillMode::LightenLess,
        Some("darken") => FillMode::Darken,
        Some("darkenLess") => FillMode::DarkenLess,
        _ => FillMode::Norm,
    };
    let stroke = p.attr_bool("stroke").unwrap_or(true);

    let pt = |n: &XNode| -> (f64, f64) {
        (
            g.get(n.attr("x").unwrap_or("0")) * sx,
            g.get(n.attr("y").unwrap_or("0")) * sy,
        )
    };
    let mut cmds = Vec::new();
    let mut cur = (0.0, 0.0);
    let mut start = (0.0, 0.0);
    for c in &p.children {
        let pts: Vec<(f64, f64)> = c.children_named("a:pt").map(|n| pt(n)).collect();
        match c.name.as_str() {
            "a:moveTo" => {
                if let Some(&p0) = pts.first() {
                    cmds.push(Cmd::Move(p0.0, p0.1));
                    cur = p0;
                    start = p0;
                }
            }
            "a:lnTo" => {
                if let Some(&p0) = pts.first() {
                    cmds.push(Cmd::Line(p0.0, p0.1));
                    cur = p0;
                }
            }
            "a:quadBezTo" if pts.len() >= 2 => {
                cmds.push(Cmd::Quad(pts[0].0, pts[0].1, pts[1].0, pts[1].1));
                cur = pts[1];
            }
            "a:cubicBezTo" if pts.len() >= 3 => {
                cmds.push(Cmd::Cubic(
                    pts[0].0, pts[0].1, pts[1].0, pts[1].1, pts[2].0, pts[2].1,
                ));
                cur = pts[2];
            }
            "a:arcTo" => {
                let wr = g.get(c.attr("wR").unwrap_or("0")) * sx;
                let hr = g.get(c.attr("hR").unwrap_or("0")) * sy;
                let st = g.get(c.attr("stAng").unwrap_or("0")) / 60_000.0;
                let sw = g.get(c.attr("swAng").unwrap_or("0")) / 60_000.0;
                if cmds.is_empty() {
                    cmds.push(Cmd::Move(cur.0, cur.1));
                    start = cur;
                }
                cur = arc_to(&mut cmds, cur, wr, hr, st, sw);
            }
            "a:close" => {
                cmds.push(Cmd::Close);
                cur = start;
            }
            _ => {}
        }
    }
    GeomPath { cmds, fill, stroke }
}

/// Converts a visual angle on an ellipse with radii (wr, hr) to its parametric angle.
fn parametric(deg: f64, wr: f64, hr: f64) -> f64 {
    let r = deg.to_radians();
    (wr * r.sin()).atan2(hr * r.cos())
}

/// Appends an elliptical arc starting at `cur`; returns the end point.
pub fn arc_to(
    cmds: &mut Vec<Cmd>,
    cur: (f64, f64),
    wr: f64,
    hr: f64,
    st_deg: f64,
    sw_deg: f64,
) -> (f64, f64) {
    if wr.abs() < 1e-9 || hr.abs() < 1e-9 || sw_deg.abs() < 1e-9 {
        return cur;
    }
    let t1 = parametric(st_deg, wr, hr);
    let t2 = parametric(st_deg + sw_deg, wr, hr);
    let full_turns = (sw_deg.abs() / 360.0).floor();
    let mut delta = t2 - t1;
    if sw_deg > 0.0 {
        while delta < 0.0 {
            delta += std::f64::consts::TAU;
        }
    } else {
        while delta > 0.0 {
            delta -= std::f64::consts::TAU;
        }
    }
    if full_turns >= 1.0 && delta.abs() < 1e-9 {
        delta = std::f64::consts::TAU * sw_deg.signum();
    }
    let cx = cur.0 - wr * t1.cos();
    let cy = cur.1 - hr * t1.sin();
    let segments = (delta.abs() / std::f64::consts::FRAC_PI_2).ceil().max(1.0) as usize;
    let step = delta / segments as f64;
    let k = 4.0 / 3.0 * (step / 4.0).tan();
    let mut a = t1;
    for _ in 0..segments {
        let b = a + step;
        let (ca, sa, cb, sb) = (a.cos(), a.sin(), b.cos(), b.sin());
        let p1 = (cx + wr * (ca - k * sa), cy + hr * (sa + k * ca));
        let p2 = (cx + wr * (cb + k * sb), cy + hr * (sb - k * cb));
        let p3 = (cx + wr * cb, cy + hr * sb);
        cmds.push(Cmd::Cubic(p1.0, p1.1, p2.0, p2.1, p3.0, p3.1));
        a = b;
    }
    (cx + wr * (t1 + delta).cos(), cy + hr * (t1 + delta).sin())
}

#[cfg(test)]
mod tests;
