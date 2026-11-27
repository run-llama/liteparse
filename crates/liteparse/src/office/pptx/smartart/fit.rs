//! Font fitting. Text nodes sharing a layout node name form a font group with
//! one size; groups whose sizes are derived from each other (`refType=
//! "primFontSz"`) are merged. A group starts at its constraint size (usually
//! 65 pt) and takes the largest whole size at which every member's text fits its
//! box and no stack of font-derived sizes overflows. Each node's rules, in order
//! (its own first, then its ancestors'), say how far its shape may grow at a
//! given size: a growth rule listed after a font rule only applies below that
//! rule's size (`Engine::growth`).

use super::build::{Rule, Tree};
use super::layout::{Engine, Grow, Measure, Step};

/// Rules that apply to node `n`: its own, then those of its ancestors that
/// target it through `ch`/`des` and a name or point type filter.
fn rules_for(tree: &Tree, e: &Engine, n: usize) -> Vec<Rule> {
    let node = &tree.nodes[n];
    let mut out: Vec<Rule> = node
        .rules
        .iter()
        .filter(|r| r.for_ == "self")
        .cloned()
        .collect();
    let mut cur = node.parent;
    let mut depth = 1;
    while let Some(p) = cur {
        for r in &tree.nodes[p].rules {
            let reach = match r.for_.as_str() {
                "ch" => depth == 1,
                "des" => true,
                _ => false,
            };
            let name_ok = r.for_name.is_empty() || r.for_name == node.name;
            let type_ok = r.pt_type == "all" || e.dm.pts[node.pt].ty.matches(&r.pt_type);
            if reach && name_ok && type_ok {
                out.push(r.clone());
            }
        }
        cur = tree.nodes[p].parent;
        depth += 1;
    }
    out
}

fn steps(rules: &[Rule]) -> Vec<Step> {
    let mut out = Vec::new();
    for r in rules {
        match r.ty.as_str() {
            "primFontSz" => {
                let v = if r.val.is_finite() { r.val } else { 5.0 };
                out.push(Step::Font(v.max(1.0)));
            }
            "h" | "w" => {
                let g = if r.val.is_infinite() {
                    Grow::Inf
                } else if r.fact.is_finite() {
                    Grow::Fact(r.fact)
                } else if r.val.is_finite() {
                    Grow::Upto(r.val)
                } else {
                    continue;
                };
                out.push(if r.ty == "h" {
                    Step::Grow(g)
                } else {
                    Step::GrowW(g)
                });
            }
            _ => {}
        }
    }
    out
}

/// Smallest size a group may reach: its members' lowest font rule, else 5 pt.
fn floor_size(e: &Engine, members: &[usize]) -> f64 {
    let min = members
        .iter()
        .flat_map(|&n| e.steps[n].iter())
        .filter_map(|s| match s {
            Step::Font(v) => Some(*v),
            _ => None,
        })
        .fold(f64::INFINITY, f64::min);
    if min.is_finite() { min.ceil() } else { 5.0 }
}

pub fn solve(e: &mut Engine, w: f64, h: f64, m: &mut dyn Measure, fixed: &[Option<f64>]) {
    let tree = e.tree;
    for n in 0..tree.nodes.len() {
        if e.group[n].is_some() {
            e.steps[n] = steps(&rules_for(tree, e, n));
        }
    }
    e.run(w, h, m);
    merge_groups(e, fixed);
    let groups = e.groups();
    if groups == 0 {
        return;
    }
    let members: Vec<Vec<usize>> = (0..groups)
        .map(|g| {
            (0..tree.nodes.len())
                .filter(|&n| e.group[n] == Some(g))
                .collect()
        })
        .collect();
    e.run(w, h, m);
    let start: Vec<f64> = (0..groups)
        .map(|g| {
            members[g]
                .iter()
                .map(|&n| e.font(n))
                .fold(0.0, f64::max)
                .clamp(1.0, 400.0)
                .floor()
        })
        .collect();
    e.fs.copy_from_slice(&start);
    for _pass in 0..2 {
        for g in 0..groups {
            if e.fixed[g].is_some() {
                continue;
            }
            let lo = floor_size(e, &members[g]);
            let ok = |e: &mut Engine, fs: f64, m: &mut dyn Measure| {
                e.fs[g] = fs;
                e.run(w, h, m);
                fits(e, g, &members[g], m)
            };
            let hi = start[g].max(lo);
            if ok(e, hi, m) {
                continue;
            }
            if !ok(e, lo, m) {
                e.fs[g] = lo;
                continue;
            }
            // Largest whole size in [lo, hi) that fits.
            let (mut a, mut b) = (lo, hi);
            while b - a > 1.0 {
                let mid = ((a + b) / 2.0).floor();
                if ok(e, mid, m) {
                    a = mid;
                } else {
                    b = mid;
                }
            }
            e.fs[g] = a;
        }
    }
    e.run(w, h, m);
}

/// Merges font groups whose sizes derive from another group's, and applies
/// fixed (user-set) sizes per merged group.
fn merge_groups(e: &mut Engine, fixed: &[Option<f64>]) {
    let k = e.groups();
    let mut parent: Vec<usize> = (0..k).collect();
    fn find(p: &mut [usize], x: usize) -> usize {
        let mut r = x;
        while p[r] != r {
            r = p[r];
        }
        p[x] = r;
        r
    }
    for n in 0..e.group.len() {
        let Some(g) = e.group[n] else { continue };
        let deps = e.vals[n].get("primFontSz").map(|v| v.deps).unwrap_or(0);
        for o in 0..k.min(64) {
            if o != g && deps & (1 << o) != 0 {
                let (a, b) = (find(&mut parent, g), find(&mut parent, o));
                if a != b {
                    parent[a.max(b)] = a.min(b);
                }
            }
        }
    }
    let mut ids: Vec<usize> = Vec::new();
    let mut map = vec![0; k];
    for (g, slot) in map.iter_mut().enumerate() {
        let r = find(&mut parent, g);
        *slot = match ids.iter().position(|&x| x == r) {
            Some(i) => i,
            None => {
                ids.push(r);
                ids.len() - 1
            }
        };
    }
    let mut new_fixed: Vec<Option<f64>> = vec![None; ids.len()];
    let mut all_fixed = vec![true; ids.len()];
    for (g, &id) in map.iter().enumerate() {
        match fixed.get(g).copied().flatten() {
            Some(f) => new_fixed[id] = Some(new_fixed[id].unwrap_or(0.0).max(f)),
            None => all_fixed[id] = false,
        }
    }
    for g in e.group.iter_mut().flatten() {
        *g = map[*g];
    }
    let n = ids.len();
    e.fs = vec![f64::INFINITY; n];
    e.overflow = vec![false; n];
    e.fixed = (0..n)
        .map(|i| if all_fixed[i] { new_fixed[i] } else { None })
        .collect();
}

fn fits(e: &Engine, g: usize, members: &[usize], m: &mut dyn Measure) -> bool {
    if e.overflow[g] {
        return false;
    }
    for &n in members {
        let Some(r) = e.rect[n] else { continue };
        let (need, avail) = m.text_fit(n, e.font(n), r.w, r.h, e.margins(n));
        if need > avail + 0.5 {
            return false;
        }
    }
    true
}
