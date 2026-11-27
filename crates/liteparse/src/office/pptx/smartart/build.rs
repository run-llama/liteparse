//! Instantiates the layout definition (`dgm:layoutDef`) over the data model: every
//! `dgm:layoutNode` reached through `forEach` and `choose` becomes one layout node
//! bound to the data point that was current there, with its algorithm, shape,
//! presented text, constraints and rules.

use std::collections::HashMap;
use std::rc::Rc;

use super::data::{DataModel, PtType};
use crate::office::pptx::xml::XNode;

/// Guard against layout definitions that expand without bound.
const MAX_NODES: usize = 3000;

#[derive(Clone, Debug, Default)]
pub struct Alg {
    pub kind: String,
    pub params: HashMap<String, String>,
}

impl Alg {
    pub fn param(&self, k: &str) -> Option<&str> {
        self.params.get(k).map(String::as_str)
    }
}

#[derive(Clone, Debug, Default)]
pub struct ShapeDef {
    /// Preset geometry name, `conn` (drawn by the connector algorithm) or empty.
    pub ty: String,
    pub rot: f64,
    pub hide_geom: bool,
    pub z_off: i32,
    pub adj: Vec<(u32, f64)>,
}

#[derive(Clone, Debug)]
pub struct Constr {
    pub ty: String,
    pub for_: String,
    pub for_name: String,
    pub pt_type: String,
    pub ref_ty: String,
    pub ref_for: String,
    pub ref_for_name: String,
    pub ref_pt_type: String,
    pub op: String,
    pub fact: f64,
    pub val: Option<f64>,
}

#[derive(Clone, Debug)]
pub struct Rule {
    pub ty: String,
    pub for_: String,
    pub for_name: String,
    pub pt_type: String,
    /// Limit the value may move to (NaN: none, INF: unbounded).
    pub val: f64,
    /// Factor limit relative to the original value (NaN: none).
    pub fact: f64,
}

#[derive(Clone, Debug, Default)]
pub struct LNode {
    pub name: String,
    pub style_lbl: String,
    pub alg: Alg,
    pub shape: ShapeDef,
    /// The data point current where the node was instantiated.
    pub pt: usize,
    /// Points whose text the node shows, with their level (0 = first).
    pub text: Vec<(usize, u32)>,
    pub constrs: Vec<Constr>,
    pub rules: Vec<Rule>,
    pub vars: Vec<(String, String)>,
    pub children: Vec<usize>,
    pub parent: Option<usize>,
}

pub struct Tree {
    pub nodes: Vec<LNode>,
}

pub fn build(def: &XNode, dm: &DataModel) -> Option<Tree> {
    let root = def.child("dgm:layoutNode")?;
    let mut named = HashMap::new();
    collect_for_each(def, &mut named);
    let mut b = Builder {
        dm,
        named,
        nodes: Vec::new(),
        depth: 0,
    };
    b.layout_node(root, dm.root, None);
    if b.nodes.len() >= MAX_NODES {
        return None;
    }
    Some(Tree { nodes: b.nodes })
}

fn collect_for_each(el: &XNode, out: &mut HashMap<String, Rc<XNode>>) {
    for c in &el.children {
        if c.name == "dgm:forEach"
            && let Some(n) = c.attr("name")
        {
            out.entry(n.to_string()).or_insert_with(|| c.clone());
        }
        collect_for_each(c, out);
    }
}

struct Builder<'a> {
    dm: &'a DataModel,
    named: HashMap<String, Rc<XNode>>,
    nodes: Vec<LNode>,
    depth: u32,
}

impl Builder<'_> {
    fn layout_node(&mut self, el: &XNode, pt: usize, parent: Option<usize>) -> Option<usize> {
        if self.nodes.len() >= MAX_NODES || self.depth > 64 {
            return None;
        }
        let id = self.nodes.len();
        self.nodes.push(LNode {
            name: el.attr("name").unwrap_or("").to_string(),
            style_lbl: el.attr("styleLbl").unwrap_or("").to_string(),
            pt,
            parent,
            ..Default::default()
        });
        self.depth += 1;
        self.content(el, id, pt);
        self.depth -= 1;
        Some(id)
    }

    fn content(&mut self, el: &XNode, id: usize, pt: usize) {
        for c in &el.children {
            if self.nodes.len() >= MAX_NODES {
                return;
            }
            match c.local() {
                "alg" => self.nodes[id].alg = parse_alg(c),
                "shape" => self.nodes[id].shape = parse_shape(c),
                "presOf" => {
                    let sel = select(self.dm, pt, c);
                    let min = sel
                        .iter()
                        .map(|&(p, _)| self.dm.depth[p])
                        .min()
                        .unwrap_or(0);
                    // Text reached through `des` (not `desOrSelf`) is child-level text.
                    let base = match c.attr("axis").and_then(|a| a.split_whitespace().last()) {
                        Some("des") => 1,
                        _ => 0,
                    };
                    self.nodes[id].text = sel
                        .iter()
                        .map(|&(p, _)| (p, self.dm.depth[p] - min + base))
                        .collect();
                }
                "constrLst" => {
                    let list = c.children_named("dgm:constr").map(|k| parse_constr(k));
                    self.nodes[id].constrs.extend(list);
                }
                "ruleLst" => {
                    let list = c.children_named("dgm:rule").map(|k| parse_rule(k));
                    self.nodes[id].rules.extend(list);
                }
                "varLst" => {
                    for v in &c.children {
                        let val = v
                            .attr("val")
                            .map(str::to_string)
                            .unwrap_or_else(|| var_default(v.local()).to_string());
                        self.nodes[id].vars.push((v.local().to_string(), val));
                    }
                }
                "layoutNode" => {
                    if let Some(child) = self.layout_node(c, pt, Some(id)) {
                        self.nodes[id].children.push(child);
                    }
                }
                "forEach" => {
                    let fe = match c.attr("ref") {
                        Some(r) => match self.named.get(r) {
                            Some(f) => f.clone(),
                            None => continue,
                        },
                        None => c.clone(),
                    };
                    if self.depth > 64 {
                        continue;
                    }
                    self.depth += 1;
                    for (p, _) in select(self.dm, pt, &fe) {
                        self.content(&fe, id, p);
                    }
                    self.depth -= 1;
                }
                "choose" => {
                    for branch in &c.children {
                        let taken = match branch.local() {
                            "if" => self.condition(branch, id, pt),
                            "else" => true,
                            _ => false,
                        };
                        if taken {
                            self.content(branch, id, pt);
                            break;
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn condition(&self, el: &XNode, id: usize, pt: usize) -> bool {
        let func = el.attr("func").unwrap_or("");
        let op = el.attr("op").unwrap_or("equ");
        let want = el.attr("val").unwrap_or("");
        if func == "var" {
            let got = self.var(id, pt, el.attr("arg").unwrap_or(""));
            return match op {
                "equ" => got == want,
                "neq" => got != want,
                _ => false,
            };
        }
        let dm = self.dm;
        let has_axis = el.attr("axis").is_some();
        let sel = if has_axis {
            select(dm, pt, el)
        } else {
            vec![(pt, pt)]
        };
        let got = match func {
            "cnt" => sel.len() as f64,
            "pos" | "revPos" | "posEven" | "posOdd" => {
                let Some(&(p, _)) = sel.last() else {
                    return false;
                };
                let (pos, n) = dm.position(p);
                match func {
                    "pos" => pos as f64,
                    "revPos" => (n + 1 - pos) as f64,
                    "posEven" => (pos % 2 == 0) as u8 as f64,
                    _ => (pos % 2 == 1) as u8 as f64,
                }
            }
            "depth" => sel.last().map(|&(p, _)| dm.depth[p]).unwrap_or(0) as f64,
            "maxDepth" => sel
                .iter()
                .map(|&(p, o)| dm.depth[p].saturating_sub(dm.depth[o]))
                .max()
                .unwrap_or(0) as f64,
            _ => return false,
        };
        let Ok(want) = want.trim().parse::<f64>() else {
            return false;
        };
        compare(got, op, want)
    }

    /// A layout variable: the user's override on the node's `pres` point, then the
    /// definition's `varLst` defaults, nearest layout node first.
    fn var(&self, id: usize, pt: usize, name: &str) -> String {
        let mut cur = Some(id);
        let mut p = pt;
        while let Some(i) = cur {
            let n = &self.nodes[i];
            if let Some(v) = self
                .dm
                .pres_point(p, &n.name)
                .and_then(|pp| pp.pr_set.as_ref())
                .and_then(|ps| ps.child("dgm:presLayoutVars"))
                .and_then(|lv| lv.children.iter().find(|c| c.local() == name).cloned())
            {
                return v
                    .attr("val")
                    .unwrap_or_else(|| var_default(name))
                    .to_string();
            }
            if let Some((_, v)) = n.vars.iter().find(|(k, _)| k == name) {
                return v.clone();
            }
            cur = n.parent;
            if let Some(par) = cur {
                p = self.nodes[par].pt;
            }
        }
        var_default(name).to_string()
    }
}

fn var_default(name: &str) -> &'static str {
    match name {
        "dir" => "norm",
        "hierBranch" => "std",
        "animLvl" | "animOne" => "none",
        "resizeHandles" => "rel",
        "bulletEnabled" | "orgChart" => "0",
        _ => "",
    }
}

pub fn compare(got: f64, op: &str, want: f64) -> bool {
    match op {
        "equ" => (got - want).abs() < 1e-9,
        "neq" => (got - want).abs() >= 1e-9,
        "gt" => got > want,
        "lt" => got < want,
        "gte" => got >= want,
        "lte" => got <= want,
        _ => false,
    }
}

/// Points selected by an element's `axis`/`ptType`/`st`/`cnt`/`step` lists, each
/// with the point its last axis step started from.
pub fn select(dm: &DataModel, p: usize, el: &XNode) -> Vec<(usize, usize)> {
    let split = |k: &str| -> Vec<String> {
        el.attr(k)
            .map(|v| v.split_whitespace().map(str::to_string).collect())
            .unwrap_or_default()
    };
    let axes = split("axis");
    let types = split("ptType");
    let nums = |k: &str, d: i64| -> Vec<i64> {
        split(k)
            .iter()
            .map(|v| v.parse::<i64>().unwrap_or(d))
            .collect()
    };
    let (sts, cnts, steps) = (nums("st", 1), nums("cnt", 0), nums("step", 1));
    let hide_last = el.attr_bool("hideLastTrans").unwrap_or(true);
    let n_steps = axes.len().max(1);
    let mut cur = vec![(p, p)];
    for i in 0..n_steps {
        let axis = axes.get(i).map(String::as_str).unwrap_or("");
        let ty = types.get(i).map(String::as_str).unwrap_or("all");
        let st = sts.get(i).copied().unwrap_or(1);
        let cnt = cnts.get(i).copied().unwrap_or(0);
        let step = steps.get(i).copied().unwrap_or(1);
        let mut next = Vec::new();
        for &(q, _) in &cur {
            let mut list: Vec<usize> = dm
                .axis(q, axis)
                .into_iter()
                .filter(|&x| dm.pts[x].ty != PtType::Pres && dm.pts[x].ty.matches(ty))
                .collect();
            if hide_last {
                list.retain(|&x| {
                    dm.pts[x].ty != PtType::SibTrans || dm.last_sib_trans(x) != Some(x)
                });
            }
            let len = list.len() as i64;
            if len == 0 {
                continue;
            }
            let mut idx = if st > 0 { st - 1 } else { len + st };
            let step = if step == 0 { 1 } else { step };
            let mut taken = 0;
            while idx >= 0 && idx < len && (cnt <= 0 || taken < cnt) {
                next.push((list[idx as usize], q));
                taken += 1;
                idx += step;
            }
        }
        cur = next;
        if cur.len() > 5000 {
            break;
        }
    }
    cur
}

fn parse_alg(el: &XNode) -> Alg {
    Alg {
        kind: el.attr("type").unwrap_or("").to_string(),
        params: el
            .children_named("dgm:param")
            .filter_map(|p| Some((p.attr("type")?.to_string(), p.attr("val")?.to_string())))
            .collect(),
    }
}

fn parse_shape(el: &XNode) -> ShapeDef {
    ShapeDef {
        ty: el.attr("type").unwrap_or("").to_string(),
        rot: el.attr_f64("rot").unwrap_or(0.0),
        hide_geom: el.attr_bool("hideGeom").unwrap_or(false),
        z_off: el.attr_i64("zOrderOff").unwrap_or(0) as i32,
        adj: el
            .child("dgm:adjLst")
            .map(|l| {
                l.children_named("dgm:adj")
                    .filter_map(|a| Some((a.attr_i64("idx")? as u32, a.attr_f64("val")?)))
                    .collect()
            })
            .unwrap_or_default(),
    }
}

fn s(el: &XNode, k: &str, d: &str) -> String {
    el.attr(k).unwrap_or(d).to_string()
}

fn parse_num(v: Option<&str>) -> Option<f64> {
    let v = v?.trim();
    match v {
        "INF" => Some(f64::INFINITY),
        "-INF" => Some(f64::NEG_INFINITY),
        "NaN" => Some(f64::NAN),
        _ => v.parse().ok(),
    }
}

fn parse_constr(el: &XNode) -> Constr {
    Constr {
        ty: s(el, "type", ""),
        for_: s(el, "for", "self"),
        for_name: s(el, "forName", ""),
        pt_type: s(el, "ptType", "all"),
        ref_ty: s(el, "refType", "none"),
        ref_for: s(el, "refFor", "self"),
        ref_for_name: s(el, "refForName", ""),
        ref_pt_type: s(el, "refPtType", "all"),
        op: s(el, "op", "none"),
        fact: parse_num(el.attr("fact")).unwrap_or(1.0),
        val: parse_num(el.attr("val")),
    }
}

fn parse_rule(el: &XNode) -> Rule {
    Rule {
        ty: s(el, "type", ""),
        for_: s(el, "for", "self"),
        for_name: s(el, "forName", ""),
        pt_type: s(el, "ptType", "all"),
        val: parse_num(el.attr("val")).unwrap_or(f64::NAN),
        fact: parse_num(el.attr("fact")).unwrap_or(f64::NAN),
    }
}
