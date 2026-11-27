//! Sizes and places the instantiated layout nodes: constraints, then each node's
//! algorithm (ECMA-376 Part 1 §21.4.4: `lin`, `snake`, `cycle`, `composite`,
//! `hierRoot`/`hierChild`, `conn`, `tx`, `sp`).
//!
//! Units: sizes and positions are millimetres, font sizes points, and text
//! margins points (`lMarg = 0.3 × primFontSz` gives a 5.4 pt inset at 18 pt).
//! Every value remembers which font groups it was derived from, so a stack whose
//! font-derived sizes overflow the diagram makes those groups' fonts smaller
//! instead of being squeezed (see `fit.rs`).

use std::collections::HashMap;

use super::build::{Constr, LNode, Tree};
use super::data::DataModel;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub fn cx(&self) -> f64 {
        self.x + self.w / 2.0
    }
    pub fn cy(&self) -> f64 {
        self.y + self.h / 2.0
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Val {
    pub v: f64,
    /// Bit set of the font groups this value was derived from.
    pub deps: u64,
}

/// One rule of a text node: lower the font towards a size, or let the shape grow.
#[derive(Clone, Copy, Debug)]
pub enum Step {
    Font(f64),
    Grow(Grow),
    /// Growth in width (text gets more room per line).
    GrowW(Grow),
}

/// How far a text node's height may grow to fit its text.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Grow {
    None,
    Fact(f64),
    Upto(f64),
    Inf,
}

/// Text measurement supplied by the renderer: the height the node's text needs
/// and the height available, both in points, for a box of `w`×`h` mm.
pub trait Measure {
    fn text_fit(&mut self, node: usize, fs: f64, w: f64, h: f64, margins: [f64; 4]) -> (f64, f64);
}

/// A connector drawn between two nodes.
#[derive(Clone, Debug)]
pub struct Conn {
    pub node: usize,
    /// Begin and end points (mm).
    pub from: (f64, f64),
    pub to: (f64, f64),
    /// Arrow thickness for 2-D connectors (mm).
    pub thick: f64,
    pub one_d: bool,
    /// Elbow routing for 1-D connectors: the bend's distance from the start along
    /// the main direction (mm), or `None` for a straight line.
    pub bend: Option<f64>,
    /// A circular connector: centre, radius (mm), start and end angles
    /// (radians, clockwise on the page).
    pub arc: Option<(f64, f64, f64, f64, f64)>,
    /// Whether the line leaves its start and enters its end vertically.
    pub vert: (bool, bool),
}

pub struct Engine<'a> {
    pub tree: &'a Tree,
    pub dm: &'a DataModel,
    pub vals: Vec<HashMap<String, Val>>,
    pub rect: Vec<Option<Rect>>,
    /// Font group of each text node (nodes sharing a layout node name).
    pub group: Vec<Option<usize>>,
    /// Per group: font size cap (points) and allowed growth.
    pub fs: Vec<f64>,
    /// Per text node: its rule steps, in order (see `growth`).
    pub steps: Vec<Vec<Step>>,
    /// Per group: fixed font size from user-set text (`custT`).
    pub fixed: Vec<Option<f64>>,
    /// Per group: a stack of its font-derived sizes did not fit.
    pub overflow: Vec<bool>,
    pub conns: Vec<Conn>,
    /// Equality constraints without a value: (members, type).
    equ: Vec<(Vec<usize>, String)>,
    /// Which node's constraint list set each value.
    owner: HashMap<(usize, String), (usize, bool)>,
    /// Centre of each cycle node's ring (mm), for curved connectors.
    arc_center: HashMap<usize, (f64, f64)>,
    /// Nodes whose children were already placed by an enclosing algorithm.
    preset: Vec<bool>,
    pub unsupported: Option<String>,
}

const POS_TYPES: [&str; 8] = ["l", "t", "r", "b", "ctrX", "ctrY", "w", "h"];

fn bit(g: Option<usize>) -> u64 {
    match g {
        Some(g) if g < 64 => 1 << g,
        _ => 0,
    }
}

impl<'a> Engine<'a> {
    pub fn new(tree: &'a Tree, dm: &'a DataModel) -> Engine<'a> {
        let n = tree.nodes.len();
        let mut group = vec![None; n];
        let mut names: Vec<String> = Vec::new();
        for (i, node) in tree.nodes.iter().enumerate() {
            if node.alg.kind == "tx" && node.text.iter().any(|&(p, _)| dm.has_text(p)) {
                let g = match names.iter().position(|nm| *nm == node.name) {
                    Some(g) => g,
                    None => {
                        names.push(node.name.clone());
                        names.len() - 1
                    }
                };
                group[i] = Some(g);
            }
        }
        let k = names.len();
        Engine {
            tree,
            dm,
            vals: vec![HashMap::new(); n],
            rect: vec![None; n],
            group,
            fs: vec![f64::INFINITY; k],
            steps: vec![Vec::new(); n],
            fixed: vec![None; k],
            overflow: vec![false; k],
            conns: Vec::new(),
            equ: Vec::new(),
            owner: HashMap::new(),
            arc_center: HashMap::new(),
            preset: vec![false; n],
            unsupported: None,
        }
    }

    pub fn groups(&self) -> usize {
        self.fs.len()
    }

    fn node(&self, n: usize) -> &'a LNode {
        &self.tree.nodes[n]
    }

    /// Runs the whole layout for a `w`×`h` mm frame.
    pub fn run(&mut self, w: f64, h: f64, m: &mut dyn Measure) {
        for v in &mut self.vals {
            v.clear();
        }
        self.rect.iter_mut().for_each(|r| *r = None);
        self.preset.iter_mut().for_each(|p| *p = false);
        self.overflow.iter_mut().for_each(|o| *o = false);
        self.conns.clear();
        self.equ.clear();
        self.owner.clear();
        self.arc_center.clear();
        self.rect[0] = Some(Rect {
            x: 0.0,
            y: 0.0,
            w,
            h,
        });
        self.eval_self(0);
        self.layout(0, m);
    }

    /// Font size of a text node (points), capped by its group.
    pub fn font(&self, n: usize) -> f64 {
        let g = self.group[n];
        if let Some(f) = g.and_then(|g| self.fixed[g]) {
            return f;
        }
        let v = self.vals[n].get("primFontSz").map(|v| v.v).unwrap_or(65.0);
        let cap = g.map(|g| self.fs[g]).unwrap_or(f64::INFINITY);
        v.min(cap)
    }

    pub fn margins(&self, n: usize) -> [f64; 4] {
        let get = |k: &str| {
            self.vals[n]
                .get(k)
                .map(|v| v.v)
                .filter(|v| v.is_finite())
                .unwrap_or(0.0)
                .max(0.0)
        };
        [get("lMarg"), get("tMarg"), get("rMarg"), get("bMarg")]
    }

    // ---- constraints -------------------------------------------------------

    fn targets(&self, n: usize, rel: &str, name: &str, pt_type: &str) -> Vec<usize> {
        let mut out = Vec::new();
        match rel {
            "ch" => out.extend(self.node(n).children.iter().copied()),
            "des" => self.descendants(n, &mut out),
            _ => out.push(n),
        }
        out.retain(|&t| {
            let node = self.node(t);
            (name.is_empty() || node.name == name)
                && (pt_type == "all" || self.dm.pts[node.pt].ty.matches(pt_type))
        });
        out
    }

    fn descendants(&self, n: usize, out: &mut Vec<usize>) {
        for &c in &self.node(n).children {
            out.push(c);
            self.descendants(c, out);
        }
    }

    /// Current value of `ty` on node `n`: its placed rectangle for geometry,
    /// else the constraint value.
    pub fn get(&self, n: usize, ty: &str) -> Option<Val> {
        if let (Some(r), true) = (self.rect[n], POS_TYPES.contains(&ty)) {
            let (ox, oy) = self
                .node(n)
                .parent
                .and_then(|p| self.rect[p])
                .map(|p| (p.x, p.y))
                .unwrap_or((0.0, 0.0));
            let deps = self.vals[n]
                .get(if matches!(ty, "w" | "l" | "r" | "ctrX") {
                    "w"
                } else {
                    "h"
                })
                .map(|v| v.deps)
                .unwrap_or(0);
            let v = match ty {
                "l" => r.x - ox,
                "t" => r.y - oy,
                "r" => r.x + r.w - ox,
                "b" => r.y + r.h - oy,
                "ctrX" => r.cx() - ox,
                "ctrY" => r.cy() - oy,
                "w" => r.w,
                _ => r.h,
            };
            return Some(Val { v, deps });
        }
        if ty == "primFontSz" {
            let g = self.group[n];
            let v = self.vals[n].get(ty).copied();
            if let Some(f) = g.and_then(|g| self.fixed[g]) {
                return Some(Val { v: f, deps: 0 });
            }
            return v.map(|v| Val {
                v: v.v.min(g.map(|g| self.fs[g]).unwrap_or(f64::INFINITY)),
                deps: v.deps | bit(g),
            });
        }
        if let Some(v) = self.vals[n].get(ty).copied() {
            return Some(v);
        }
        if POS_TYPES.contains(&ty) {
            return self.derived(n, ty);
        }
        None
    }

    /// A position or size of an unplaced node implied by its other geometry
    /// constraints (`r` from `l` and `w`, and so on).
    fn derived(&self, n: usize, ty: &str) -> Option<Val> {
        let horizontal = matches!(ty, "l" | "r" | "w" | "ctrX");
        let keys = if horizontal {
            ["l", "r", "w", "ctrX", "lOff", "rOff", "wOff", "ctrXOff"]
        } else {
            ["t", "b", "h", "ctrY", "tOff", "bOff", "hOff", "ctrYOff"]
        };
        let g = |k: &str| self.vals[n].get(k).map(|v| v.v).filter(|v| v.is_finite());
        let off = |k: &str| g(k).unwrap_or(0.0);
        let known = keys[..4].iter().filter(|k| g(k).is_some()).count();
        if known < 2 {
            return None;
        }
        let parent = self
            .node(n)
            .parent
            .and_then(|p| self.rect[p])
            .map(|r| if horizontal { r.w } else { r.h })
            .unwrap_or(0.0);
        let (x, w) = Self::span1(
            g(keys[0]).map(|v| v + off(keys[4])),
            g(keys[1]).map(|v| v + off(keys[5])),
            g(keys[2]).map(|v| v + off(keys[6])),
            g(keys[3]).map(|v| v + off(keys[7])),
            parent,
        );
        let deps = keys[..4]
            .iter()
            .filter_map(|k| self.vals[n].get(*k))
            .fold(0, |a, v| a | v.deps);
        let v = match ty {
            "l" | "t" => x,
            "r" | "b" => x + w,
            "ctrX" | "ctrY" => x + w / 2.0,
            _ => w,
        };
        Some(Val { v, deps })
    }

    fn set(&mut self, n: usize, ty: &str, v: Val) {
        self.vals[n].insert(ty.to_string(), v);
    }

    fn apply(&mut self, n: usize, c: &Constr) {
        let targets = self.targets(n, &c.for_, &c.for_name, &c.pt_type);
        if targets.is_empty() {
            return;
        }
        let value = if c.ref_ty != "none" && !c.ref_ty.is_empty() {
            let refs = self.targets(n, &c.ref_for, &c.ref_for_name, &c.ref_pt_type);
            let Some(r) = refs.first().and_then(|&r| self.get(r, &c.ref_ty)) else {
                return;
            };
            let fact = if c.fact.is_nan() { 1.0 } else { c.fact };
            Some(Val {
                v: r.v * fact,
                deps: r.deps,
            })
        } else if c.op == "none" || c.val.is_some() {
            Some(Val {
                v: c.val.filter(|v| !v.is_nan()).unwrap_or(0.0),
                deps: 0,
            })
        } else {
            None
        };
        // A bare `<constr type="h"/>` only declares a zero default: it never
        // replaces a value another constraint set.
        let decl = c.val.is_none() && (c.ref_ty == "none" || c.ref_ty.is_empty());
        match c.op.as_str() {
            "gte" | "lte" => {
                let Some(v) = value else { return };
                for t in targets {
                    if let Some(cur) = self.vals[t].get(&c.ty).copied() {
                        let keep = if c.op == "gte" {
                            cur.v >= v.v
                        } else {
                            cur.v <= v.v
                        };
                        if !keep {
                            self.set(t, &c.ty, v);
                        }
                    }
                }
            }
            _ => {
                if c.op == "equ" && value.is_none() {
                    self.equ.push((targets.clone(), c.ty.clone()));
                }
                if let Some(v) = value {
                    for t in targets {
                        // A declaration only resets values from its own list, and
                        // a node's own constraints yield to its ancestors' values.
                        let key = (t, c.ty.clone());
                        let theirs = if self.vals[t].contains_key(&c.ty) {
                            self.owner.get(&key).copied().filter(|&(o, _)| o != n)
                        } else {
                            None
                        };
                        let keep = match theirs {
                            Some((_, their_decl)) => decl || (t == n && !their_decl),
                            None => false,
                        };
                        if !keep {
                            self.set(t, &c.ty, v);
                            self.owner.insert(key, (n, decl));
                        }
                    }
                }
            }
        }
    }

    /// Evaluates every constraint of `n` (it is placed: its own size is known).
    /// Constraints may refer to values set further down their list, so the list
    /// is applied twice.
    fn eval_all(&mut self, n: usize) {
        for _ in 0..2 {
            for c in &self.node(n).constrs {
                if c.ty == "connDist" && c.val.is_none() && c.ref_ty == "none" {
                    continue;
                }
                self.apply(n, c);
            }
        }
    }

    /// Evaluates the constraints `n` sets on itself, before it is placed.
    fn eval_self(&mut self, n: usize) {
        for _ in 0..2 {
            for c in &self.node(n).constrs {
                if c.for_ == "self" && !(c.ty == "connDist" && c.ref_ty == "none") {
                    self.apply(n, c);
                }
            }
        }
    }

    // ---- placement ---------------------------------------------------------

    fn layout(&mut self, n: usize, m: &mut dyn Measure) {
        let Some(mut r) = self.rect[n] else { return };
        // A composite with a fixed aspect ratio (`ar`) keeps it inside its box.
        if let Some(ar) = (self.node(n).alg.kind == "composite")
            .then(|| {
                self.node(n)
                    .alg
                    .param("ar")
                    .and_then(|v| v.parse::<f64>().ok())
            })
            .flatten()
            .filter(|a| *a > 0.0 && r.h > 0.0)
        {
            let alg = &self.node(n).alg;
            if r.w / r.h > ar {
                let w = r.h * ar;
                r.x += match alg.param("horzAlign").unwrap_or("ctr") {
                    "l" => 0.0,
                    "r" => r.w - w,
                    _ => (r.w - w) / 2.0,
                };
                r.w = w;
            } else {
                let h = r.w / ar;
                r.y += match alg.param("vertAlign").unwrap_or("mid") {
                    "t" => 0.0,
                    "b" => r.h - h,
                    _ => (r.h - h) / 2.0,
                };
                r.h = h;
            }
            self.rect[n] = Some(r);
        }
        self.eval_all(n);
        let kids = self.node(n).children.clone();
        if !self.preset[n] {
            for &c in &kids {
                self.eval_self(c);
                self.cust_scale(c);
            }
            let alg = self.node(n).alg.kind.clone();
            match alg.as_str() {
                "lin" => self.lin(n, r, &kids, m),
                "snake" => self.snake(n, r, &kids, m),
                "cycle" => self.cycle(n, r, &kids, m),
                "hierChild" | "hierRoot" => self.hier(n, r),
                "pyra" => self.unsupported = Some(alg),
                // composite, tx, sp, conn and unknown: children by constraints.
                _ => self.composite(n, r, &kids, m),
            }
        }
        for &c in &kids {
            if self.node(c).alg.kind != "conn" {
                self.layout(c, m);
            }
        }
        for &c in &kids {
            if self.node(c).alg.kind == "conn" {
                self.connector(c);
                self.layout(c, m);
            }
        }
    }

    /// The user's per-shape resize (`custScaleX/Y` on the pres point).
    /// The user's per-shape resize (`custScaleX/Y` on the pres point), as
    /// factors on the requested width and height.
    fn cust_factors(&self, c: usize) -> (f64, f64) {
        let node = self.node(c);
        let Some(ps) = self
            .dm
            .pres_point(node.pt, &node.name)
            .and_then(|p| p.pr_set.as_ref())
        else {
            return (1.0, 1.0);
        };
        let f = |a: &str| {
            ps.attr_f64(a)
                .filter(|s| *s > 0.0)
                .map(|s| s / 100_000.0)
                .unwrap_or(1.0)
        };
        (f("custScaleX"), f("custScaleY"))
    }

    fn cust_scale(&mut self, c: usize) {
        let (sx, sy) = self.cust_factors(c);
        for (ty, f) in [("w", sx), ("h", sy)] {
            if f != 1.0
                && let Some(v) = self.vals[c].get_mut(ty)
            {
                v.v *= f;
            }
        }
    }

    /// The fixed aspect ratio (w / h) of a composite, if it has one.
    fn aspect(&self, c: usize) -> Option<f64> {
        let alg = &self.node(c).alg;
        (alg.kind == "composite")
            .then(|| alg.param("ar").and_then(|v| v.parse::<f64>().ok()))
            .flatten()
            .filter(|a| *a > 0.0)
    }

    fn req(&self, c: usize, ty: &str) -> Option<Val> {
        self.vals[c].get(ty).copied().filter(|v| !v.v.is_nan())
    }

    fn is_conn(&self, c: usize) -> bool {
        self.node(c).alg.kind == "conn"
    }

    /// A spacing node: no shape, no algorithm of its own.
    fn is_space(&self, c: usize) -> bool {
        let n = self.node(c);
        n.alg.kind == "sp" && n.shape.ty.is_empty() && n.children.is_empty()
    }

    /// Requested size of a child that lays out its own children when it has no
    /// explicit (or an unbounded) size: the extent of those children.
    fn natural(&mut self, c: usize, horizontal: bool) -> Val {
        let ty = if horizontal { "w" } else { "h" };
        let kids = self.node(c).children.clone();
        let along = match self.node(c).alg.param("linDir").unwrap_or("fromL") {
            "fromT" | "fromB" => !horizontal,
            _ => horizontal,
        };
        let lin = self.node(c).alg.kind == "lin";
        let mut total = Val::default();
        for &k in &kids {
            self.eval_self(k);
            let Some(mut v) = self.req(k, ty).filter(|v| v.v.is_finite()) else {
                continue;
            };
            let (sx, sy) = self.cust_factors(k);
            v.v *= if horizontal { sx } else { sy };
            total.deps |= v.deps;
            if lin && along {
                total.v += v.v;
            } else {
                total.v = total.v.max(v.v);
            }
        }
        total
    }

    fn deps_of(&self, c: usize, ty: &str) -> u64 {
        self.vals[c].get(ty).map(|v| v.deps).unwrap_or(0)
    }

    fn flag_overflow(&mut self, deps: u64) {
        for g in 0..self.overflow.len().min(64) {
            if deps & (1 << g) != 0 {
                self.overflow[g] = true;
            }
        }
    }

    /// The growth (height, width) a node's rules allow at its current font size:
    /// rules apply in order, so a growth rule listed after a font rule only
    /// applies once the font has come down to that rule's size.
    pub fn growth(&self, n: usize) -> (Grow, Grow) {
        let fs = self.font(n);
        // A size the user fixed skips the font rules.
        let fixed = self.group[n].is_some_and(|g| self.fixed[g].is_some());
        let mut cur = (Grow::None, Grow::None);
        for step in &self.steps[n] {
            match *step {
                Step::Font(min) if fs > min + 1e-9 && !fixed => return cur,
                Step::Font(_) => {}
                Step::Grow(g) => cur.0 = g,
                Step::GrowW(g) => cur.1 = g,
            }
        }
        cur
    }

    /// Width a text child may take, at most `room`: widened until its text fits
    /// as far as its rules allow.
    fn grown_w(&mut self, c: usize, w: f64, h: f64, room: f64, m: &mut dyn Measure) -> f64 {
        if self.group[c].is_none() || w <= 0.0 {
            return w;
        }
        let cap = match self.growth(c).1 {
            Grow::None => return w,
            Grow::Fact(f) => w * f,
            Grow::Upto(v) => v.max(w),
            Grow::Inf => f64::INFINITY,
        }
        .min(room.max(w));
        let (fs, marg) = (self.font(c), self.margins(c));
        let fits = |m: &mut dyn Measure, ww: f64| {
            let (need, avail) = m.text_fit(c, fs, ww, h, marg);
            need <= avail + 0.5
        };
        if cap <= w || fits(m, w) {
            return w;
        }
        if !fits(m, cap) {
            return cap;
        }
        let (mut lo, mut hi) = (w, cap);
        for _ in 0..8 {
            let mid = (lo + hi) / 2.0;
            if fits(m, mid) {
                hi = mid;
            } else {
                lo = mid;
            }
        }
        hi
    }

    /// Height a text child may take: its requested height, grown to fit its text
    /// as far as its rules allow.
    fn grown(&mut self, c: usize, w: f64, h: f64, m: &mut dyn Measure) -> f64 {
        if self.group[c].is_none() {
            return h;
        }
        let allow = self.growth(c).0;
        if allow == Grow::None || w <= 0.0 {
            return h;
        }
        let cap = match allow {
            Grow::Fact(f) => h * f,
            Grow::Upto(v) => v.max(h),
            _ => f64::INFINITY,
        };
        let fs = self.font(c);
        let marg = self.margins(c);
        let mut hh = h.max(0.1);
        for _ in 0..6 {
            let (need, avail) = m.text_fit(c, fs, w, hh, marg);
            if need <= avail + 0.5 || hh >= cap || !need.is_finite() {
                break;
            }
            hh = (hh + (need - avail) / super::PT_PER_MM + 0.05).min(cap);
        }
        hh
    }

    /// Equalizes sizes that `op="equ"` constraints tie together, to the largest.
    fn equalize(&self, kids: &[usize], sizes: &mut [(f64, f64)]) {
        for (members, ty) in &self.equ {
            let axis = match ty.as_str() {
                "w" => 0,
                "h" => 1,
                _ => continue,
            };
            let idx: Vec<usize> = kids
                .iter()
                .enumerate()
                .filter(|(_, k)| members.contains(k))
                .map(|(i, _)| i)
                .collect();
            if idx.len() < 2 {
                continue;
            }
            let max = idx
                .iter()
                .map(|&i| if axis == 0 { sizes[i].0 } else { sizes[i].1 })
                .fold(f64::MIN, f64::max);
            for &i in &idx {
                if axis == 0 {
                    sizes[i].0 = max;
                } else {
                    sizes[i].1 = max;
                }
            }
        }
    }

    fn lin(&mut self, n: usize, r: Rect, kids: &[usize], m: &mut dyn Measure) {
        let alg = &self.node(n).alg;
        let dir = alg.param("linDir").unwrap_or("fromL").to_string();
        let horizontal = matches!(dir.as_str(), "fromL" | "fromR");
        let (aty, cty) = if horizontal { ("w", "h") } else { ("h", "w") };
        let (avail_a, avail_c) = if horizontal { (r.w, r.h) } else { (r.h, r.w) };
        let node_align = if horizontal {
            alg.param("nodeVertAlign").unwrap_or("mid")
        } else {
            alg.param("nodeHorzAlign").unwrap_or("ctr")
        }
        .to_string();
        let stack_align = if horizontal {
            alg.param("horzAlign").unwrap_or("ctr")
        } else {
            alg.param("vertAlign").unwrap_or("mid")
        }
        .to_string();
        // Linear stacks space their children with spacer nodes; `sp` is unused.
        // Requested (axis, cross) sizes.
        let mut sizes: Vec<(f64, f64)> = Vec::new();
        let mut deps = 0u64;
        for &c in kids {
            // Unbounded or unset sizes of containers come from their content.
            let a = match self.req(c, aty) {
                Some(v) if v.v.is_finite() => v.v,
                _ => {
                    let v = self.natural(c, horizontal);
                    self.set(c, aty, v);
                    v.v
                }
            };
            deps |= self.deps_of(c, aty);
            let cr = match self.req(c, cty) {
                Some(v) if v.v.is_finite() => v.v,
                None if self.node(c).children.is_empty() || self.is_conn(c) => avail_c,
                _ => {
                    let v = self.natural(c, !horizontal);
                    self.set(c, cty, v);
                    v.v
                }
            };
            // A composite with a fixed aspect ratio asks for the box it will use.
            let (a, cr) = match self.aspect(c) {
                Some(ar) => {
                    let (w, h) = if horizontal { (a, cr) } else { (cr, a) };
                    let (w, h) = if w / h.max(1e-9) > ar {
                        (h * ar, h)
                    } else {
                        (w, w / ar)
                    };
                    if horizontal { (w, h) } else { (h, w) }
                }
                None => (a, cr),
            };
            sizes.push((a, cr));
        }
        let mut total: f64 = sizes.iter().map(|s| s.0).sum::<f64>();
        // Scale along the axis; sizes tied to the axis size are re-derived.
        if total > avail_a + 1e-6 && total > 0.0 {
            let s = avail_a / total;
            self.flag_overflow(deps);
            for (i, &c) in kids.iter().enumerate() {
                let a = sizes[i].0 * s;
                let d = self.deps_of(c, aty);
                // The child's subtree shrinks with it along the axis.
                let mut des = Vec::new();
                self.descendants(c, &mut des);
                for k in des {
                    if let Some(v) = self.vals[k].get_mut(aty) {
                        v.v *= s;
                    }
                }
                self.set(c, aty, Val { v: a, deps: d });
                let before = self.req(c, cty).map(|v| v.v);
                self.eval_self(c);
                self.set(c, aty, Val { v: a, deps: d });
                let after = self.req(c, cty).map(|v| v.v);
                if after != before {
                    // Re-derived from the new axis size: the user's resize again.
                    let (sx, sy) = self.cust_factors(c);
                    if let Some(v) = self.vals[c].get_mut(cty) {
                        v.v *= if horizontal { sy } else { sx };
                    }
                }
                sizes[i].0 = a;
                if self.aspect(c).is_some() {
                    sizes[i].1 *= s;
                } else if let Some(v) = self.req(c, cty).filter(|v| v.v.is_finite()) {
                    sizes[i].1 = v.v;
                }
            }
            total = avail_a;
        }
        // Text growth, then equal sizes.
        for (i, &c) in kids.iter().enumerate() {
            if !horizontal {
                sizes[i].1 = self.grown_w(c, sizes[i].1, sizes[i].0, avail_c, m);
            }
            let (w, h) = if horizontal {
                (sizes[i].0, sizes[i].1)
            } else {
                (sizes[i].1, sizes[i].0)
            };
            let gh = self.grown(c, w, h, m);
            if horizontal {
                sizes[i].1 = gh;
            } else {
                sizes[i].0 = gh;
            }
        }
        self.equalize_dirs(kids, &mut sizes, horizontal);
        if !horizontal {
            let t2: f64 = sizes.iter().map(|s| s.0).sum::<f64>();
            if t2 > avail_a + 1e-6 && t2 > 0.0 {
                let mut gdeps = deps;
                for &c in kids {
                    gdeps |= bit(self.group[c]);
                }
                self.flag_overflow(gdeps);
                let s = avail_a / t2;
                sizes.iter_mut().for_each(|x| x.0 *= s);
                total = avail_a;
            } else {
                total = t2;
            }
        }
        // Cross overflow: shrink everything uniformly. A user's resize does not
        // count: such a shape may stick out of the stack.
        let cross_cust = |e: &Self, c: usize| {
            let (sx, sy) = e.cust_factors(c);
            if horizontal { sy } else { sx }
        };
        let max_c = kids
            .iter()
            .enumerate()
            .map(|(i, &c)| sizes[i].1 / cross_cust(self, c).max(1.0))
            .fold(0.0, f64::max);
        let mut s2 = 1.0;
        if max_c > avail_c + 1e-6 && max_c > 0.0 {
            s2 = avail_c / max_c;
            let mut cdeps = 0;
            for &c in kids {
                cdeps |= self.deps_of(c, cty);
            }
            self.flag_overflow(cdeps);
        }
        let total = total * s2;
        let lead = match stack_align.as_str() {
            "l" | "t" | "none" => 0.0,
            "r" | "b" => avail_a - total,
            _ => (avail_a - total) / 2.0,
        };
        let rev = matches!(dir.as_str(), "fromR" | "fromB");
        let mut pos = lead;
        for (i, &c) in kids.iter().enumerate() {
            let (a, cr) = (sizes[i].0 * s2, sizes[i].1 * s2);
            let cpos = match node_align.as_str() {
                "l" | "t" => 0.0,
                "r" | "b" => avail_c - cr,
                _ => (avail_c - cr) / 2.0,
            };
            let start = if rev { avail_a - pos - a } else { pos };
            let rc = if horizontal {
                Rect {
                    x: r.x + start,
                    y: r.y + cpos,
                    w: a,
                    h: cr,
                }
            } else {
                Rect {
                    x: r.x + cpos,
                    y: r.y + start,
                    w: cr,
                    h: a,
                }
            };
            self.rect[c] = Some(rc);
            pos += a;
        }
    }

    fn equalize_dirs(&self, kids: &[usize], sizes: &mut [(f64, f64)], horizontal: bool) {
        // `sizes` is (axis, cross); equalize works on (w, h).
        let mut wh: Vec<(f64, f64)> = sizes
            .iter()
            .map(|&(a, c)| if horizontal { (a, c) } else { (c, a) })
            .collect();
        self.equalize(kids, &mut wh);
        for (i, s) in sizes.iter_mut().enumerate() {
            *s = if horizontal {
                wh[i]
            } else {
                (wh[i].1, wh[i].0)
            };
        }
    }

    fn snake(&mut self, n: usize, r: Rect, kids: &[usize], m: &mut dyn Measure) {
        let alg = self.node(n).alg.clone();
        let items: Vec<usize> = kids
            .iter()
            .copied()
            .filter(|&c| !self.is_conn(c) && !self.is_space(c))
            .collect();
        if items.is_empty() {
            return;
        }
        let sp = self.req(n, "sp").map(|v| v.v).unwrap_or(0.0).max(0.0);
        let mut w = 0.0f64;
        let mut h = 0.0f64;
        let mut deps = 0;
        for &c in &items {
            w = w.max(
                self.req(c, "w")
                    .map(|v| v.v)
                    .filter(|v| v.is_finite())
                    .unwrap_or(r.w),
            );
            h = h.max(
                self.req(c, "h")
                    .map(|v| v.v)
                    .filter(|v| v.is_finite())
                    .unwrap_or(r.h),
            );
            deps |= self.deps_of(c, "w") | self.deps_of(c, "h");
        }
        let k = items.len();
        let by_col = alg.param("flowDir") == Some("col");
        // A rule letting the items narrow to a minimum width: as many per row
        // as fit at that width, at their own height, the gaps taking the rest.
        let name = self.node(items[0]).name.clone();
        let min_w = self
            .node(n)
            .rules
            .iter()
            .find(|r| {
                r.ty == "w" && r.for_ == "ch" && (r.for_name.is_empty() || r.for_name == name)
            })
            .map(|r| r.val)
            .filter(|v| v.is_finite() && *v > 0.0 && *v < w);
        let (per, iw, ih0, isp) = if let (Some(min_w), false) = (min_w, by_col) {
            let per = ((r.w / min_w).floor() as usize).clamp(1, k);
            let ratio = if w > 0.0 { sp / w } else { 0.0 };
            let mut iw = (r.w / (per as f64 + (per - 1) as f64 * ratio)).min(w);
            let mut isp = iw * ratio;
            if iw < min_w {
                iw = min_w.min(r.w);
                isp = if per > 1 {
                    ((r.w - per as f64 * iw) / (per - 1) as f64).max(0.0)
                } else {
                    0.0
                };
            }
            let rows = k.div_ceil(per) as f64;
            let ih = h.min((r.h - (rows - 1.0) * isp) / rows);
            if ih < h {
                self.flag_overflow(deps);
            }
            (per, iw, ih, isp)
        } else {
            // Pick the number of items per row that makes them largest.
            let mut best = (0.0, 1usize);
            for per in 1..=k {
                let lines = k.div_ceil(per);
                let (cols, rows) = if by_col { (lines, per) } else { (per, lines) };
                let tw = cols as f64 * w + (cols - 1) as f64 * sp;
                let th = rows as f64 * h + (rows - 1) as f64 * sp;
                let s = (r.w / tw).min(r.h / th);
                if s > best.0 + 1e-9 {
                    best = (s, per);
                }
            }
            let (s, per) = best;
            let s = s.min(1.0);
            if s < 1.0 {
                self.flag_overflow(deps);
            }
            (per, w * s, h * s, sp * s)
        };
        let mut ih = ih0;
        for &c in &items {
            ih = ih.max(self.grown(c, iw, ih0, m));
        }
        let lines = k.div_ceil(per);
        let (cols, rows) = if by_col { (lines, per) } else { (per, lines) };
        let tw = cols as f64 * iw + (cols - 1) as f64 * isp;
        let th = rows as f64 * ih + (rows - 1) as f64 * isp;
        let ox = r.x + (r.w - tw) / 2.0;
        let oy = r.y + (r.h - th) / 2.0;
        let gr = alg.param("grDir").unwrap_or("tL").to_string();
        let rev_dir = alg.param("contDir") == Some("revDir");
        let center = alg.param("off").unwrap_or("ctr") == "ctr";
        for (i, &c) in items.iter().enumerate() {
            let line = i / per;
            let mut j = i % per;
            let in_line = if line == lines - 1 {
                k - line * per
            } else {
                per
            };
            if rev_dir && line % 2 == 1 {
                j = in_line - 1 - j;
            }
            let shift = if center && in_line < per {
                (per - in_line) as f64 / 2.0
            } else {
                0.0
            };
            let (mut col, mut row) = if by_col {
                (line as f64, j as f64 + shift)
            } else {
                (j as f64 + shift, line as f64)
            };
            if gr.ends_with('R') {
                col = cols as f64 - 1.0 - col;
            }
            if gr.starts_with('b') {
                row = rows as f64 - 1.0 - row;
            }
            self.rect[c] = Some(Rect {
                x: ox + col * (iw + isp),
                y: oy + row * (ih + isp),
                w: iw,
                h: ih,
            });
        }
    }

    fn cycle(&mut self, n: usize, r: Rect, kids: &[usize], m: &mut dyn Measure) {
        let alg = self.node(n).alg.clone();
        let size = |e: &Self, c: usize| -> (f64, f64) {
            let w = e
                .req(c, "w")
                .map(|v| v.v)
                .filter(|v| v.is_finite())
                .unwrap_or(0.0);
            let h = e
                .req(c, "h")
                .map(|v| v.v)
                .filter(|v| v.is_finite())
                .unwrap_or(w);
            (w.max(0.0), h.max(0.0))
        };
        // Spacers with a size take their own slot on the ring.
        let items: Vec<usize> = kids
            .iter()
            .copied()
            .filter(|&c| !self.is_conn(c) && (!self.is_space(c) || size(self, c).0 > 0.0))
            .collect();
        if items.is_empty() {
            return;
        }
        let st = alg
            .param("stAng")
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(0.0);
        let span = alg
            .param("spanAng")
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(360.0);
        let center_first = alg.param("ctrShpMap") == Some("fNode") && items.len() > 1;
        let ring: Vec<usize> = if center_first {
            items[1..].to_vec()
        } else {
            items.clone()
        };
        let k = ring.len();
        let full = span.abs() >= 359.9;
        let step = if full {
            span / k as f64
        } else if k > 1 {
            span / (k - 1) as f64
        } else {
            0.0
        };
        let sib_sp = self.req(n, "sibSp").map(|v| v.v).unwrap_or(0.0);
        let diam_of = |e: &Self, c: usize| {
            let (w, h) = size(e, c);
            if e.is_space(c) { w } else { w.max(h) }
        };
        // Radius where neighbours on the ring are `sibSp` apart.
        let mut radius: f64 = 0.0;
        if k > 1 {
            let half = (step.abs().min(180.0) / 2.0).to_radians().sin().max(1e-3);
            let pairs = if full { k } else { k - 1 };
            for i in 0..pairs {
                let (a, b) = (ring[i], ring[(i + 1) % k]);
                let need = diam_of(self, a) / 2.0 + diam_of(self, b) / 2.0 + sib_sp;
                radius = radius.max(need / (2.0 * half));
            }
        }
        if let Some(d) = self.req(n, "diam").filter(|v| v.v.is_finite() && v.v > 0.0) {
            radius = d.v / 2.0;
        }
        let pts: Vec<(f64, f64)> = (0..k)
            .map(|i| {
                let a = (st + step * i as f64).to_radians();
                (radius * a.sin(), -radius * a.cos())
            })
            .collect();
        let (cw, ch) = if center_first {
            size(self, items[0])
        } else {
            (0.0, 0.0)
        };
        let (mut x0, mut x1, mut y0, mut y1) = (-cw / 2.0, cw / 2.0, -ch / 2.0, ch / 2.0);
        for (i, &(x, y)) in pts.iter().enumerate() {
            let (w, h) = size(self, ring[i]);
            x0 = x0.min(x - w / 2.0);
            x1 = x1.max(x + w / 2.0);
            y0 = y0.min(y - h / 2.0);
            y1 = y1.max(y + h / 2.0);
        }
        let s = (r.w / (x1 - x0).max(1e-6))
            .min(r.h / (y1 - y0).max(1e-6))
            .min(1.0);
        if s < 1.0 {
            let mut deps = 0;
            for &c in &items {
                deps |= self.deps_of(c, "w") | self.deps_of(c, "h");
            }
            self.flag_overflow(deps);
            // Everything on the ring, connectors included, shrinks together.
            for &c in kids {
                let mut sub = vec![c];
                self.descendants(c, &mut sub);
                for d in sub {
                    for ty in ["w", "h"] {
                        if let Some(v) = self.vals[d].get_mut(ty) {
                            v.v *= s;
                        }
                    }
                }
            }
        }
        let ox = r.x + r.w / 2.0 - (x0 + x1) / 2.0 * s;
        let oy = r.y + r.h / 2.0 - (y0 + y1) / 2.0 * s;
        let deps = self.deps_of(n, "w");
        self.set(
            n,
            "diam",
            Val {
                v: 2.0 * radius * s,
                deps,
            },
        );
        self.arc_center.insert(n, (ox, oy));
        if center_first {
            let (gw, gh) = size(self, items[0]);
            let gh = self.grown(items[0], gw, gh, m).max(gh);
            self.rect[items[0]] = Some(Rect {
                x: ox - gw / 2.0,
                y: oy - gh / 2.0,
                w: gw,
                h: gh,
            });
        }
        for (i, &c) in ring.iter().enumerate() {
            let (cw, ch) = size(self, c);
            let ch = self.grown(c, cw, ch, m).max(ch);
            let (px, py) = pts[i];
            self.rect[c] = Some(Rect {
                x: ox + px * s - cw / 2.0,
                y: oy + py * s - ch / 2.0,
                w: cw,
                h: ch,
            });
        }
    }

    /// Places children from their position and size constraints, relative to
    /// the node's own rectangle.
    fn composite(&mut self, n: usize, r: Rect, kids: &[usize], m: &mut dyn Measure) {
        // Children that grow to fit their text move the siblings placed from
        // their size, so the constraints are evaluated again after growth.
        for pass in 0..3 {
            let mut grew = false;
            for &c in kids {
                if self.is_conn(c) {
                    // Connectors are placed between their end nodes later; until
                    // then their children get the whole box.
                    self.rect[c] = Some(r);
                    continue;
                }
                let g = |e: &Self, k: &str| e.req(c, k).map(|v| v.v).filter(|v| v.is_finite());
                let off = |e: &Self, k: &str| g(e, k).unwrap_or(0.0);
                let (x, w) = Self::span1(
                    g(self, "l").map(|v| v + off(self, "lOff")),
                    g(self, "r").map(|v| v + off(self, "rOff")),
                    g(self, "w").map(|v| v + off(self, "wOff")),
                    g(self, "ctrX").map(|v| v + off(self, "ctrXOff")),
                    r.w,
                );
                let (y, h) = Self::span1(
                    g(self, "t").map(|v| v + off(self, "tOff")),
                    g(self, "b").map(|v| v + off(self, "bOff")),
                    g(self, "h").map(|v| v + off(self, "hOff")),
                    g(self, "ctrY").map(|v| v + off(self, "ctrYOff")),
                    r.h,
                );
                let h2 = self.grown(c, w, h, m);
                let y = if h2 > h && g(self, "b").is_some() && g(self, "t").is_none() {
                    y - (h2 - h)
                } else if h2 > h && g(self, "ctrY").is_some() {
                    y - (h2 - h) / 2.0
                } else {
                    y
                };
                if h2 > h + 0.01 {
                    grew = true;
                    let deps = self.deps_of(c, "h") | bit(self.group[c]);
                    self.set(c, "h", Val { v: h2, deps });
                }
                self.rect[c] = Some(Rect {
                    x: r.x + x,
                    y: r.y + y,
                    w,
                    h: h2,
                });
            }
            if !grew || pass == 2 {
                break;
            }
            for &c in kids {
                self.rect[c] = None;
            }
            self.eval_all(n);
        }
        // Font-derived children that spill out of the box: fonts must shrink.
        for &c in kids {
            let Some(cr) = self.rect[c].filter(|_| !self.is_conn(c)) else {
                continue;
            };
            if cr.y + cr.h > r.y + r.h + 0.5 || cr.x + cr.w > r.x + r.w + 0.5 {
                // Only sizes derived from fonts (or grown for text) can shrink.
                let deps = self.deps_of(c, "h") | self.deps_of(c, "t") | self.deps_of(c, "b");
                self.flag_overflow(deps);
            }
        }
    }

    /// One dimension of a composite child: (start, size) from its start, end,
    /// size and centre constraints, with the parent extent as default size.
    fn span1(
        start: Option<f64>,
        end: Option<f64>,
        size: Option<f64>,
        ctr: Option<f64>,
        parent: f64,
    ) -> (f64, f64) {
        match (start, end, size, ctr) {
            (Some(s), _, Some(w), _) => (s, w),
            (None, Some(e), Some(w), _) => (e - w, w),
            (None, None, Some(w), Some(c)) => (c - w / 2.0, w),
            (None, None, Some(w), None) => (0.0, w),
            (Some(s), Some(e), None, _) => (s, e - s),
            (Some(s), None, None, Some(c)) => (s, 2.0 * (c - s)),
            (None, Some(e), None, Some(c)) => (2.0 * c - e, 2.0 * (e - c)),
            (Some(s), None, None, None) => (s, (parent - s).max(0.0)),
            (None, Some(e), None, None) => (0.0, e),
            (None, None, None, Some(c)) => (c - parent / 2.0, parent),
            (None, None, None, None) => (0.0, parent),
        }
    }

    // ---- hierarchy ---------------------------------------------------------

    fn is_hier(&self, n: usize) -> bool {
        matches!(self.node(n).alg.kind.as_str(), "hierRoot" | "hierChild")
    }

    /// Lays out a whole hierarchy below `n` (the outermost hier node): measures
    /// every subtree, scales the tree to fit and places all hier nodes and the
    /// shapes they hold. A `hierRoot` holds its own shapes and the `hierChild`
    /// nodes of its children; a `hierChild` lines up `hierRoot` subtrees.
    fn hier(&mut self, n: usize, r: Rect) {
        // `layout` prepared n's own children; the deeper levels are done here.
        for c in self.node(n).children.clone() {
            if self.is_hier(c) {
                self.hier_prepare(c);
            }
        }
        let (w, h) = self.hier_measure(n);
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        let s = (r.w / w).min(r.h / h).min(1.0);
        let x = r.x + (r.w - w * s) / 2.0;
        let y = r.y + (r.h - h * s) / 2.0;
        self.hier_place(n, x, y, s);
    }

    fn hier_prepare(&mut self, n: usize) {
        for c in self.node(n).children.clone() {
            self.eval_self(c);
            self.cust_scale(c);
            if self.is_hier(c) {
                self.hier_prepare(c);
            }
        }
    }

    fn hier_vertical(&self, n: usize) -> bool {
        let alg = &self.node(n).alg;
        matches!(alg.param("linDir"), Some("fromT") | Some("fromB"))
            || matches!(alg.param("secLinDir"), Some("fromT") | Some("fromB"))
    }

    fn hier_box(&self, c: usize) -> (f64, f64) {
        let w = self
            .req(c, "w")
            .map(|v| v.v)
            .filter(|v| v.is_finite())
            .unwrap_or(0.0);
        let h = self
            .req(c, "h")
            .map(|v| v.v)
            .filter(|v| v.is_finite())
            .unwrap_or(0.0);
        (w.max(0.0), h.max(0.0))
    }

    fn hier_size(&self, c: usize) -> (f64, f64) {
        if self.is_hier(c) {
            self.hier_measure(c)
        } else {
            self.hier_box(c)
        }
    }

    fn hier_gap(&self, n: usize) -> f64 {
        let v = if self.hier_vertical(n) {
            self.req(n, "sibSp").or(self.req(n, "secSibSp"))
        } else {
            self.req(n, "sibSp")
        };
        v.map(|v| v.v).unwrap_or(0.0).max(0.0)
    }

    /// The parts of a `hierRoot`: its own shapes' box, and its child block
    /// (the `hierChild` nodes stacked).
    fn hier_parts(&self, n: usize) -> ((f64, f64), (f64, f64)) {
        let (mut rw, mut rh, mut cw, mut ch) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
        for &c in &self.node(n).children {
            if self.is_conn(c) {
                continue;
            }
            let (w, h) = self.hier_size(c);
            if self.is_hier(c) {
                cw = cw.max(w);
                ch += h;
            } else {
                rw = rw.max(w);
                rh = rh.max(h);
            }
        }
        ((rw, rh), (cw, ch))
    }

    fn hier_align(&self, n: usize) -> String {
        self.node(n)
            .alg
            .param("hierAlign")
            .unwrap_or("tCtr")
            .to_string()
    }

    /// Unscaled (w, h) of a hier node's subtree.
    fn hier_measure(&self, n: usize) -> (f64, f64) {
        if self.node(n).alg.kind == "hierChild" {
            let parts: Vec<(f64, f64)> = self
                .node(n)
                .children
                .iter()
                .filter(|&&c| !self.is_conn(c))
                .map(|&c| self.hier_size(c))
                .filter(|p| p.0 > 0.0 || p.1 > 0.0)
                .collect();
            if parts.is_empty() {
                return (0.0, 0.0);
            }
            let gaps = self.hier_gap(n) * (parts.len() - 1) as f64;
            let max_w = parts.iter().map(|p| p.0).fold(0.0, f64::max);
            let max_h = parts.iter().map(|p| p.1).fold(0.0, f64::max);
            return if self.hier_vertical(n) {
                (max_w, parts.iter().map(|p| p.1).sum::<f64>() + gaps)
            } else {
                (parts.iter().map(|p| p.0).sum::<f64>() + gaps, max_h)
            };
        }
        let ((rw, rh), (cw, ch)) = self.hier_parts(n);
        let has_kids = cw > 0.0 || ch > 0.0;
        let sp = if has_kids {
            self.req(n, "sp").map(|v| v.v).unwrap_or(0.0).max(0.0)
        } else {
            0.0
        };
        let align = self.hier_align(n);
        let off = self.req(n, "alignOff").map(|v| v.v).unwrap_or(0.0).max(0.0) * rw;
        match align.chars().next() {
            Some('l') | Some('r') => (rw + sp + cw, rh.max(ch)),
            _ if align.ends_with('L') || align.ends_with('R') => (rw.max(off + cw), rh + sp + ch),
            _ => (rw.max(cw), rh + sp + ch),
        }
    }

    fn hier_place(&mut self, n: usize, x: f64, y: f64, s: f64) {
        let (w, h) = self.hier_measure(n);
        let (w, h) = (w * s, h * s);
        self.rect[n] = Some(Rect { x, y, w, h });
        self.preset[n] = true;
        let kids = self.node(n).children.clone();
        for &c in &kids {
            if self.is_conn(c) {
                self.rect[c] = Some(Rect { x, y, w, h });
            }
        }
        if self.node(n).alg.kind == "hierChild" {
            let vertical = self.hier_vertical(n);
            let gap = self.hier_gap(n) * s;
            let alg = &self.node(n).alg;
            let rev = matches!(alg.param("linDir"), Some("fromR") | Some("fromB"));
            let align = alg
                .param("chAlign")
                .unwrap_or(if vertical { "l" } else { "t" })
                .to_string();
            let mut pos = 0.0;
            for c in kids {
                if self.is_conn(c) {
                    continue;
                }
                let (cw, ch) = self.hier_size(c);
                let (cw, ch) = (cw * s, ch * s);
                if cw <= 0.0 && ch <= 0.0 {
                    continue;
                }
                let (cx, cy) = if vertical {
                    let ax = match align.as_str() {
                        "r" => w - cw,
                        "ctr" => (w - cw) / 2.0,
                        _ => 0.0,
                    };
                    let py = if rev { h - pos - ch } else { pos };
                    (x + ax, y + py)
                } else {
                    let ay = match align.as_str() {
                        "b" => h - ch,
                        "ctr" | "mid" => (h - ch) / 2.0,
                        _ => 0.0,
                    };
                    let px = if rev { w - pos - cw } else { pos };
                    (x + px, y + ay)
                };
                if self.is_hier(c) {
                    self.hier_place(c, cx, cy, s);
                } else {
                    self.rect[c] = Some(Rect {
                        x: cx,
                        y: cy,
                        w: cw,
                        h: ch,
                    });
                }
                pos += if vertical { ch } else { cw } + gap;
            }
            return;
        }
        let ((rw, rh), (cw, ch)) = self.hier_parts(n);
        let (rw, rh, cw, ch) = (rw * s, rh * s, cw * s, ch * s);
        let sp = self.req(n, "sp").map(|v| v.v).unwrap_or(0.0).max(0.0) * s;
        let off = self.req(n, "alignOff").map(|v| v.v).unwrap_or(0.0).max(0.0) * rw;
        let align = self.hier_align(n);
        // Where the root's shapes and the child block go inside (w, h).
        let (root, block) = match align.chars().next() {
            Some('l') | Some('r') => {
                let left = align.starts_with('l');
                let ry = if align.contains("Ctr") {
                    (h - rh) / 2.0
                } else if align.ends_with('B') {
                    h - rh
                } else {
                    0.0
                };
                let by = if align.contains("Ctr") {
                    (h - ch) / 2.0
                } else {
                    0.0
                };
                if left {
                    ((0.0, ry), (rw + sp, by))
                } else {
                    ((w - rw, ry), (w - rw - sp - cw, by))
                }
            }
            Some('b') => {
                let rx = (w - rw) / 2.0;
                ((rx, h - rh), ((w - cw) / 2.0, 0.0))
            }
            _ if align.ends_with('L') => ((0.0, 0.0), (off, rh + sp)),
            _ if align.ends_with('R') => ((w - rw, 0.0), (w - off - cw, rh + sp)),
            _ => (((w - rw) / 2.0, 0.0), ((w - cw) / 2.0, rh + sp)),
        };
        let mut by = y + block.1;
        for c in kids {
            if self.is_conn(c) {
                continue;
            }
            if self.is_hier(c) {
                let (kw, kh) = self.hier_measure(c);
                let bx = x + block.0 + (cw - kw * s) / 2.0 * (!align.ends_with('L') as u8 as f64);
                self.hier_place(c, bx, by, s);
                by += kh * s;
            } else {
                let (bw, bh) = self.hier_box(c);
                self.rect[c] = Some(Rect {
                    x: x + root.0 + (rw - bw * s) / 2.0,
                    y: y + root.1 + (rh - bh * s) / 2.0,
                    w: bw * s,
                    h: bh * s,
                });
            }
        }
    }

    // ---- connectors --------------------------------------------------------

    /// The node a connector starts or ends at: the layout node named by the
    /// algorithm (`srcNode`/`dstNode`) or else a drawn node presenting that data
    /// point, the nearest in the layout tree.
    fn conn_end(&self, c: usize, pt: usize, name: Option<&str>) -> Option<usize> {
        let mut best: Option<(usize, usize)> = None;
        let anc = self.ancestors(c);
        for (i, node) in self.tree.nodes.iter().enumerate() {
            if i == c || self.rect[i].is_none() {
                continue;
            }
            let ok = match name {
                Some(nm) => node.name == nm,
                None => {
                    node.pt == pt
                        && !node.shape.ty.is_empty()
                        && node.shape.ty != "conn"
                        && node.alg.kind != "conn"
                }
            };
            if !ok {
                continue;
            }
            // Distance through the nearest common ancestor; a named node on
            // the connector's own point wins.
            let mine = self.ancestors(i);
            let common = anc
                .iter()
                .position(|a| mine.contains(a))
                .unwrap_or(anc.len());
            let mut d = common
                + mine
                    .iter()
                    .position(|a| anc.contains(a))
                    .unwrap_or(mine.len());
            if node.pt != pt {
                d += 1000;
            }
            if best.map(|b| d < b.0).unwrap_or(true) {
                best = Some((d, i));
            }
        }
        best.map(|b| b.1)
    }

    fn ancestors(&self, n: usize) -> Vec<usize> {
        let mut out = Vec::new();
        let mut cur = self.node(n).parent;
        while let Some(p) = cur {
            out.push(p);
            cur = self.node(p).parent;
        }
        out
    }

    fn connector(&mut self, c: usize) {
        let node = self.node(c);
        let Some((src_pt, dst_pt)) = self.dm.ends[node.pt].or_else(|| {
            // A connector bound to a node joins it to its parent.
            let p = self.dm.parent[node.pt]?;
            Some((p, node.pt))
        }) else {
            return;
        };
        let alg = node.alg.clone();
        let (Some(a), Some(b)) = (
            self.conn_end(c, src_pt, alg.param("srcNode")),
            self.conn_end(c, dst_pt, alg.param("dstNode")),
        ) else {
            self.rect[c] = None;
            return;
        };
        let (Some(ra), Some(rb)) = (self.rect[a], self.rect[b]) else {
            return;
        };
        let one_d = alg.param("dim") == Some("1D");
        let curved = matches!(alg.param("connRout"), Some("curve") | Some("longCurve"));
        if curved {
            self.arc_connector(c, ra, rb, one_d);
            return;
        }
        let ellipse = |i: usize| self.node(i).shape.ty == "ellipse";
        let beg = alg.param("begPts").unwrap_or("auto");
        let end = alg.param("endPts").unwrap_or("auto");
        let p0 = conn_point(
            ra,
            rb,
            beg.split_whitespace().next().unwrap_or("auto"),
            ellipse(a),
        );
        let p1 = conn_point(rb, ra, pick_end(end, ra, rb), ellipse(b));
        let dist = ((p1.0 - p0.0).powi(2) + (p1.1 - p0.1).powi(2)).sqrt();
        self.set(c, "connDist", Val { v: dist, deps: 0 });
        self.eval_all(c);
        let beg_pad = self.req(c, "begPad").map(|v| v.v).unwrap_or(0.0);
        let end_pad = self.req(c, "endPad").map(|v| v.v).unwrap_or(0.0);
        let (ux, uy) = if dist > 1e-9 {
            ((p1.0 - p0.0) / dist, (p1.1 - p0.1) / dist)
        } else {
            (1.0, 0.0)
        };
        let mut q0 = (p0.0 + ux * beg_pad, p0.1 + uy * beg_pad);
        let mut q1 = (p1.0 - ux * end_pad, p1.1 - uy * end_pad);
        let thick = self
            .req(c, "h")
            .map(|v| v.v)
            .filter(|v| v.is_finite() && *v > 0.0)
            .unwrap_or(dist * 0.3);
        if !one_d {
            // A 2-D arrow is at most as long as the connector's own width.
            let len = (dist - beg_pad - end_pad).max(0.0);
            let max_len = self
                .req(c, "w")
                .map(|v| v.v)
                .filter(|v| v.is_finite() && *v > 0.0)
                .unwrap_or(len);
            if len > max_len {
                let mid = ((q0.0 + q1.0) / 2.0, (q0.1 + q1.1) / 2.0);
                q0 = (mid.0 - ux * max_len / 2.0, mid.1 - uy * max_len / 2.0);
                q1 = (mid.0 + ux * max_len / 2.0, mid.1 + uy * max_len / 2.0);
            }
        }
        let bend = if one_d && alg.param("connRout") == Some("bend") {
            let bd = self
                .req(c, "bendDist")
                .map(|v| v.v)
                .filter(|v| v.is_finite());
            Some(bd.unwrap_or(((q1.1 - q0.1).abs()) / 2.0))
        } else {
            None
        };
        let len = ((q1.0 - q0.0).powi(2) + (q1.1 - q0.1).powi(2)).sqrt();
        let mid = ((q0.0 + q1.0) / 2.0, (q0.1 + q1.1) / 2.0);
        self.rect[c] = Some(Rect {
            x: mid.0 - len / 2.0,
            y: mid.1 - thick / 2.0,
            w: len,
            h: thick,
        });
        let side = |spec: &str, d: (f64, f64)| match spec {
            "bCtr" | "tCtr" => true,
            "midL" | "midR" => false,
            _ => d.1.abs() >= d.0.abs(),
        };
        let d = (p1.0 - p0.0, p1.1 - p0.1);
        let vert = (
            side(beg.split_whitespace().next().unwrap_or("auto"), d),
            side(pick_end(end, ra, rb), d),
        );
        self.conns.push(Conn {
            node: c,
            from: q0,
            to: q1,
            thick,
            one_d,
            bend,
            arc: None,
            vert,
        });
    }

    /// A connector that follows a circle: around a cycle's ring, or the circle
    /// its own `diam` and position give. It runs clockwise from the source to
    /// the destination, clear of both nodes, less its padding.
    fn arc_connector(&mut self, c: usize, ra: Rect, rb: Rect, one_d: bool) {
        let parent = self.node(c).parent.unwrap_or(0);
        let pr = self.rect[parent].unwrap_or(ra);
        let g = |e: &Self, k: &str| e.req(c, k).map(|v| v.v).filter(|v| v.is_finite());
        let center = match (g(self, "l"), g(self, "t")) {
            (Some(l), Some(t)) => (pr.x + l, pr.y + t),
            _ => self
                .arc_center
                .get(&parent)
                .copied()
                .unwrap_or((pr.cx(), pr.cy())),
        };
        let pa = (ra.cx(), ra.cy());
        let pb = (rb.cx(), rb.cy());
        let dist = |p: (f64, f64)| ((p.0 - center.0).powi(2) + (p.1 - center.1).powi(2)).sqrt();
        let radius = g(self, "diam")
            .or_else(|| self.req(parent, "diam").map(|v| v.v))
            .filter(|d| *d > 0.0)
            .map(|d| d / 2.0)
            .unwrap_or_else(|| (dist(pa) + dist(pb)) / 2.0);
        if radius <= 0.0 {
            return;
        }
        let ang = |p: (f64, f64)| (p.1 - center.1).atan2(p.0 - center.0);
        let a0 = ang(pa);
        let mut a1 = ang(pb);
        while a1 <= a0 {
            a1 += std::f64::consts::TAU;
        }
        // Clear the end nodes: half their size seen from the centre.
        let clear = |r: Rect| ((r.w.max(r.h) / 2.0) / radius).min(1.0).asin();
        let (c0, c1) = (clear(ra), clear(rb));
        let gap = ((a1 - a0) - c0 - c1).max(0.0);
        self.set(
            c,
            "connDist",
            Val {
                v: gap * radius,
                deps: 0,
            },
        );
        self.eval_all(c);
        let pad = |e: &Self, k: &str| e.req(c, k).map(|v| v.v).unwrap_or(0.0) / radius;
        let (b0, b1) = (a0 + c0 + pad(self, "begPad"), a1 - c1 - pad(self, "endPad"));
        if b1 <= b0 {
            self.rect[c] = None;
            return;
        }
        let thick = g(self, "h").filter(|v| *v > 0.0).unwrap_or(radius * 0.1);
        self.rect[c] = Some(Rect {
            x: center.0 - radius,
            y: center.1 - radius,
            w: 2.0 * radius,
            h: 2.0 * radius,
        });
        let at = |a: f64| (center.0 + radius * a.cos(), center.1 + radius * a.sin());
        self.conns.push(Conn {
            node: c,
            from: at(b0),
            to: at(b1),
            thick,
            one_d,
            bend: None,
            arc: Some((center.0, center.1, radius, b0, b1)),
            vert: (false, false),
        });
    }
}

fn pick_end(spec: &str, from: Rect, to: Rect) -> &str {
    // "midL midR": the side facing the source.
    let opts: Vec<&str> = spec.split_whitespace().collect();
    if opts.len() > 1 {
        return if from.cx() < to.cx() {
            opts[0]
        } else {
            opts[1]
        };
    }
    opts.first().copied().unwrap_or("auto")
}

/// A connection point on rectangle `r` (facing `other` for `auto`).
fn conn_point(r: Rect, other: Rect, spec: &str, ellipse: bool) -> (f64, f64) {
    match spec {
        "ctr" => (r.cx(), r.cy()),
        "tCtr" => (r.cx(), r.y),
        "bCtr" => (r.cx(), r.y + r.h),
        "midL" => (r.x, r.cy()),
        "midR" => (r.x + r.w, r.cy()),
        "tL" => (r.x, r.y),
        "tR" => (r.x + r.w, r.y),
        "bL" => (r.x, r.y + r.h),
        "bR" => (r.x + r.w, r.y + r.h),
        _ => {
            let (dx, dy) = (other.cx() - r.cx(), other.cy() - r.cy());
            let d = (dx * dx + dy * dy).sqrt();
            if d < 1e-9 {
                return (r.cx(), r.cy());
            }
            let (ux, uy) = (dx / d, dy / d);
            let (a, b) = (r.w / 2.0, r.h / 2.0);
            let t = if ellipse {
                1.0 / ((ux / a.max(1e-9)).powi(2) + (uy / b.max(1e-9)).powi(2)).sqrt()
            } else {
                let tx = if ux.abs() > 1e-9 {
                    a / ux.abs()
                } else {
                    f64::INFINITY
                };
                let ty = if uy.abs() > 1e-9 {
                    b / uy.abs()
                } else {
                    f64::INFINITY
                };
                tx.min(ty)
            };
            (r.cx() + ux * t, r.cy() + uy * t)
        }
    }
}
