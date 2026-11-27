//! The SmartArt data model (`dgm:dataModel`): its points and the tree that the
//! layout definition's axes walk.
//!
//! The tree follows the `parOf` connections from the `doc` point. A point's
//! children list holds, for each child connection in `srcOrd` order, the
//! connection's parent transition, the child node and its sibling transition, so
//! `followSib` from a node reaches its own sibling transition first.

use std::collections::HashMap;
use std::rc::Rc;

use crate::office::pptx::xml::XNode;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PtType {
    Doc,
    Node,
    Asst,
    ParTrans,
    SibTrans,
    Pres,
}

impl PtType {
    fn parse(s: &str) -> PtType {
        match s {
            "doc" => PtType::Doc,
            "asst" => PtType::Asst,
            "parTrans" => PtType::ParTrans,
            "sibTrans" => PtType::SibTrans,
            "pres" => PtType::Pres,
            _ => PtType::Node,
        }
    }

    /// Whether a point of this type passes a layout `ptType` filter value.
    pub fn matches(self, filter: &str) -> bool {
        match filter {
            "" | "all" => true,
            "doc" => self == PtType::Doc,
            "node" | "norm" => self == PtType::Node,
            // The "non" filters stay among nodes: transitions are other types.
            "nonNorm" => matches!(self, PtType::Asst | PtType::Doc),
            "asst" => self == PtType::Asst,
            "nonAsst" => matches!(self, PtType::Node | PtType::Doc),
            "parTrans" => self == PtType::ParTrans,
            "sibTrans" => self == PtType::SibTrans,
            "pres" => self == PtType::Pres,
            _ => false,
        }
    }
}

pub struct Point {
    pub ty: PtType,
    /// `dgm:t`: the point's text body.
    pub text: Option<Rc<XNode>>,
    /// `dgm:spPr`: shape properties the user set on the point.
    pub sp_pr: Option<Rc<XNode>>,
    pub pr_set: Option<Rc<XNode>>,
}

/// A `parOf` connection: (parent, srcOrd, child, parent transition, sibling
/// transition).
type ParOf = (usize, i64, usize, Option<usize>, Option<usize>);

pub struct DataModel {
    pub pts: Vec<Point>,
    pub parent: Vec<Option<usize>>,
    pub children: Vec<Vec<usize>>,
    pub depth: Vec<u32>,
    /// For a transition: the (source, destination) nodes it connects.
    pub ends: Vec<Option<(usize, usize)>>,
    pub root: usize,
    /// (presented point, layout node name) → its `pres` point.
    pub pres: HashMap<(usize, String), usize>,
}

impl DataModel {
    pub fn parse(dm: &XNode) -> Option<DataModel> {
        let mut pts = Vec::new();
        let mut by_id: HashMap<String, usize> = HashMap::new();
        let mut list = Vec::new();
        dm.find_all("dgm:pt", &mut list);
        for pt in list {
            let id = pt.attr("modelId").unwrap_or("").to_string();
            by_id.insert(id, pts.len());
            pts.push(Point {
                ty: PtType::parse(pt.attr("type").unwrap_or("node")),
                text: pt.child("dgm:t").cloned(),
                sp_pr: pt.child("dgm:spPr").cloned(),
                pr_set: pt.child("dgm:prSet").cloned(),
            });
        }
        let root = pts.iter().position(|p| p.ty == PtType::Doc)?;
        let n = pts.len();
        let mut parent = vec![None; n];
        let mut conns: Vec<ParOf> = Vec::new();
        let mut pres = HashMap::new();
        let mut cxns = Vec::new();
        dm.find_all("dgm:cxn", &mut cxns);
        for c in cxns {
            let get = |k: &str| c.attr(k).and_then(|id| by_id.get(id).copied());
            let (Some(src), Some(dst)) = (get("srcId"), get("destId")) else {
                continue;
            };
            match c.attr("type").unwrap_or("parOf") {
                "parOf" => conns.push((
                    src,
                    c.attr_i64("srcOrd").unwrap_or(0),
                    dst,
                    get("parTransId"),
                    get("sibTransId"),
                )),
                "presOf" => {
                    if let Some(name) = pts[dst].pr_set.as_ref().and_then(|p| p.attr("presName")) {
                        pres.insert((src, name.to_string()), dst);
                    }
                }
                _ => {}
            }
        }
        // Pres points name their presented point directly too (presAssocID).
        for (i, p) in pts.iter().enumerate() {
            let Some(ps) = p.pr_set.as_ref().filter(|_| p.ty == PtType::Pres) else {
                continue;
            };
            if let (Some(assoc), Some(name)) = (
                ps.attr("presAssocID").and_then(|a| by_id.get(a)),
                ps.attr("presName"),
            ) {
                pres.entry((*assoc, name.to_string())).or_insert(i);
            }
        }
        conns.sort_by_key(|c| (c.0, c.1));
        let mut children = vec![Vec::new(); n];
        let mut ends = vec![None; n];
        let mut node_children: Vec<Vec<usize>> = vec![Vec::new(); n];
        for &(src, _, dst, par_tr, sib_tr) in &conns {
            if parent[dst].is_some() || dst == root {
                continue;
            }
            parent[dst] = Some(src);
            if let Some(t) = par_tr {
                children[src].push(t);
                parent[t] = Some(src);
                ends[t] = Some((src, dst));
            }
            children[src].push(dst);
            node_children[src].push(dst);
            if let Some(t) = sib_tr {
                children[src].push(t);
                parent[t] = Some(src);
                // Destination filled in below, once all siblings are known.
                ends[t] = Some((dst, dst));
            }
        }
        // A sibling transition leads to the next sibling, the last one back to the first.
        for kids in &node_children {
            for (i, &k) in kids.iter().enumerate() {
                let next = kids[(i + 1) % kids.len()];
                for &t in &children[parent[k].unwrap_or(root)] {
                    if pts[t].ty == PtType::SibTrans && ends[t] == Some((k, k)) {
                        ends[t] = Some((k, next));
                    }
                }
            }
        }
        let mut depth = vec![0u32; n];
        let mut stack = vec![(root, 0u32)];
        let mut seen = vec![false; n];
        while let Some((p, d)) = stack.pop() {
            if seen[p] {
                continue;
            }
            seen[p] = true;
            depth[p] = d;
            for &c in &children[p] {
                stack.push((c, d + 1));
            }
        }
        Some(DataModel {
            pts,
            parent,
            children,
            depth,
            ends,
            root,
            pres,
        })
    }

    /// The `pres` point that customizes layout node `name` for point `pt`.
    pub fn pres_point(&self, pt: usize, name: &str) -> Option<&Point> {
        self.pres
            .get(&(pt, name.to_string()))
            .map(|&i| &self.pts[i])
    }

    /// Text of a point, if it has any visible characters.
    pub fn has_text(&self, pt: usize) -> bool {
        self.pts[pt]
            .text
            .as_ref()
            .map(|t| !t.all_text().trim().is_empty())
            .unwrap_or(false)
    }

    /// Points along one axis step from `p` (before type filtering).
    pub fn axis(&self, p: usize, axis: &str) -> Vec<usize> {
        let mut out = Vec::new();
        match axis {
            "self" | "" => out.push(p),
            "ch" => out.extend(self.children[p].iter().copied()),
            "des" => self.descendants(p, &mut out),
            "desOrSelf" => {
                out.push(p);
                self.descendants(p, &mut out);
            }
            "par" => out.extend(self.parent[p]),
            "ancst" | "ancstOrSelf" => {
                if axis == "ancstOrSelf" {
                    out.push(p);
                }
                let mut cur = self.parent[p];
                while let Some(c) = cur {
                    if out.contains(&c) {
                        break;
                    }
                    out.push(c);
                    cur = self.parent[c];
                }
            }
            "root" => out.push(self.root),
            "followSib" | "precedSib" => {
                if let Some(par) = self.parent[p] {
                    let sibs = &self.children[par];
                    if let Some(i) = sibs.iter().position(|&s| s == p) {
                        if axis == "followSib" {
                            out.extend(sibs[i + 1..].iter().copied());
                        } else {
                            out.extend(sibs[..i].iter().copied());
                        }
                    }
                }
            }
            _ => {}
        }
        out
    }

    fn descendants(&self, p: usize, out: &mut Vec<usize>) {
        for &c in &self.children[p] {
            if out.len() > 10_000 || out.contains(&c) {
                return;
            }
            out.push(c);
            self.descendants(c, out);
        }
    }

    /// The last sibling transition among `p`'s parent's children: hidden by
    /// `hideLastTrans` (the default) since it would lead back to the first node.
    pub fn last_sib_trans(&self, p: usize) -> Option<usize> {
        let par = self.parent[p]?;
        self.children[par]
            .iter()
            .rev()
            .find(|&&c| self.pts[c].ty == PtType::SibTrans)
            .copied()
    }

    /// 1-based position of `p` among its parent's children of the same type.
    pub fn position(&self, p: usize) -> (usize, usize) {
        let Some(par) = self.parent[p] else {
            return (1, 1);
        };
        let ty = self.pts[p].ty;
        let same: Vec<usize> = self.children[par]
            .iter()
            .copied()
            .filter(|&c| self.pts[c].ty == ty)
            .collect();
        let pos = same
            .iter()
            .position(|&c| c == p)
            .map(|i| i + 1)
            .unwrap_or(1);
        (pos, same.len())
    }
}
