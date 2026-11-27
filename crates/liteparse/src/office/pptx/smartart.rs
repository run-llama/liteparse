//! Lays out SmartArt that was saved without its drawing part, the way PowerPoint
//! does when it opens such a file: the data model (`dgm:dataModel`) is run
//! through the layout definition (`dgm:layoutDef`), and the shapes are colored
//! from the colors and quick style parts (ECMA-376 Part 1 §21.4, DiagramML).
//!
//! `build` instantiates layout nodes over the data, `layout` evaluates
//! constraints and algorithms, `fit` picks font sizes, and `emit` writes the
//! result as a `dsp:spTree` that the slide walker draws like a saved drawing.

mod build;
mod data;
mod emit;
mod fit;
mod layout;

use std::collections::HashMap;
use std::rc::Rc;

use crate::office::pptx::scene::{Rect as PtRect, ShapeIdx};
use crate::office::pptx::slide::SlideBuilder;
use crate::office::pptx::xml::XNode;

pub(crate) const PT_PER_MM: f64 = 72.0 / 25.4;

/// The parts of one diagram, as related from the slide.
pub struct DiagramParts {
    pub data: Rc<XNode>,
    pub data_part: String,
    pub layout: Option<Rc<XNode>>,
    pub colors: Option<Rc<XNode>>,
    pub style: Option<Rc<XNode>>,
}

/// Loads the diagram parts a graphic frame's `dgm:relIds` point to.
pub fn parts(
    doc: &crate::office::pptx::document::Document,
    data: &XNode,
    part: &str,
    dm_part: &str,
) -> Option<DiagramParts> {
    let ids = data.child("dgm:relIds")?;
    let load = |attr: &str| {
        ids.attr(attr)
            .and_then(|rid| doc.pkg.rel_by_id(part, rid))
            .and_then(|r| doc.xml(&r.target))
    };
    Some(DiagramParts {
        data: doc.xml(dm_part)?,
        data_part: dm_part.to_string(),
        layout: load("r:lo"),
        colors: load("r:cs"),
        style: load("r:qs"),
    })
}

/// Lays out a diagram in a `frame` (points) and returns its drawing, with shape
/// coordinates relative to the frame's top-left in EMU. `Err` names why the
/// layout definition could not be followed.
pub fn layout(
    sb: &mut SlideBuilder,
    parts: &DiagramParts,
    frame: PtRect,
    idx: ShapeIdx,
) -> Result<Rc<XNode>, String> {
    let def = parts.layout.as_ref().ok_or("no layout part")?;
    let dm = data::DataModel::parse(&parts.data).ok_or("no data model root")?;
    let tree = build::build(def, &dm).ok_or("layout expands too far")?;
    if tree.nodes.is_empty() {
        return Err("empty layout".into());
    }
    let scheme = sb.chain.colors.clone();
    let ctx = emit::Ctx::new(
        &tree,
        &dm,
        parts.colors.clone(),
        parts.style.clone(),
        &scheme,
    );
    let mut engine = layout::Engine::new(&tree, &dm);
    let fixed: Vec<Option<f64>> = (0..engine.groups())
        .map(|g| {
            let members: Vec<usize> = (0..tree.nodes.len())
                .filter(|&n| engine.group[n] == Some(g))
                .collect();
            let all: Vec<Option<f64>> = members.iter().map(|&n| ctx.fixed_font(n)).collect();
            if all.iter().all(|f| f.is_some()) {
                all.into_iter().flatten().reduce(f64::max)
            } else {
                None
            }
        })
        .collect();
    let (w, h) = (frame.w / PT_PER_MM, frame.h / PT_PER_MM);
    let mut m = Measurer {
        sb,
        ctx: &ctx,
        part: parts.data_part.clone(),
        idx,
        cache: HashMap::new(),
    };
    fit::solve(&mut engine, w, h, &mut m, &fixed);
    if let Some(alg) = engine.unsupported.take() {
        return Err(format!("unsupported algorithm {alg}"));
    }
    // A diagram with no content (an empty placeholder) draws nothing.
    Ok(ctx.drawing(&engine))
}

/// Measures node text with the slide's own text layout.
struct Measurer<'s, 'b> {
    sb: &'s mut SlideBuilder<'b>,
    ctx: &'s emit::Ctx<'s>,
    part: String,
    idx: ShapeIdx,
    cache: HashMap<(usize, i64, i64, [i64; 4]), f64>,
}

impl layout::Measure for Measurer<'_, '_> {
    fn text_fit(&mut self, node: usize, fs: f64, w: f64, h: f64, margins: [f64; 4]) -> (f64, f64) {
        let shape = &self.ctx.tree.nodes[node].shape;
        let tr = emit::text_rect_mm(
            shape,
            layout::Rect {
                x: 0.0,
                y: 0.0,
                w,
                h,
            },
        );
        let (tw, th) = (tr.w * PT_PER_MM, tr.h * PT_PER_MM);
        let avail = th - margins[1] - margins[3];
        let key = (
            node,
            (fs * 10.0).round() as i64,
            (tw * 10.0).round() as i64,
            margins.map(|m| (m * 10.0).round() as i64),
        );
        if let Some(&need) = self.cache.get(&key) {
            return (need, avail);
        }
        let need = match self.ctx.tx_body(node, fs, margins) {
            Some(tx) => {
                let (h, _) = self.measure(&tx, tw);
                // PowerPoint shrinks text rather than break a word across lines.
                let (_, widths) = self.measure(&words_body(&tx), 100_000.0);
                let widest = widths.into_iter().fold(0.0, f64::max);
                if widest > tw - margins[0] - margins[2] + 0.5 {
                    f64::INFINITY
                } else {
                    h
                }
            }
            None => 0.0,
        };
        self.cache.insert(key, need);
        (need, avail)
    }
}

/// The words of a text body, one per paragraph and unwrapped, to measure the
/// widest. East Asian characters count as words of their own.
fn words_body(tx: &XNode) -> Rc<XNode> {
    let mut paras = Vec::new();
    for p in tx.children_named("a:p") {
        for r in p.children_named("a:r") {
            let rpr: Vec<Rc<XNode>> = r.child("a:rPr").cloned().into_iter().collect();
            let text = r.child("a:t").map(|t| t.text.as_str()).unwrap_or("");
            for word in split_words(text) {
                let t = Rc::new(XNode {
                    name: "a:t".into(),
                    attrs: Vec::new(),
                    children: Vec::new(),
                    text: word,
                });
                let mut kids = rpr.clone();
                kids.push(t);
                let run = emit::el("a:r", vec![], kids);
                paras.push(emit::el("a:p", vec![], vec![run]));
            }
        }
    }
    let body = emit::el(
        "a:bodyPr",
        vec![
            ("wrap", "none".into()),
            ("lIns", "0".into()),
            ("rIns", "0".into()),
        ],
        vec![],
    );
    let mut kids = vec![body];
    kids.extend(paras);
    emit::el("dsp:txBody", vec![], kids)
}

fn split_words(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for ch in text.chars() {
        if ch.is_whitespace() || ch as u32 >= 0x2E80 {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            if !ch.is_whitespace() {
                out.push(ch.to_string());
            }
        } else {
            cur.push(ch);
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

impl Measurer<'_, '_> {
    /// Height (points) of a text body laid out `w` points wide and the widths of
    /// the text runs it emitted, without keeping anything.
    fn measure(&mut self, tx: &XNode, w: f64) -> (f64, Vec<f64>) {
        let sb = &mut *self.sb;
        let mut lists: Vec<Rc<XNode>> = Vec::new();
        if let Some(d) = sb.doc.default_text_style() {
            lists.push(d.clone());
        }
        lists.extend(tx.child("a:lstStyle").cloned());
        let bodies: Vec<Rc<XNode>> = tx.child("a:bodyPr").cloned().into_iter().collect();
        let marks = (
            sb.scene.items.len(),
            sb.scene.links.len(),
            sb.scene.warnings.len(),
            sb.scene.shapes[self.idx].paragraphs.len(),
        );
        // A tall box: only the text height matters.
        let rect = crate::office::pptx::geom::TextRect {
            l: 0.0,
            t: 0.0,
            r: w.max(1.0),
            b: 100_000.0,
        };
        let h = sb.layout_text(
            tx,
            &lists,
            &bodies,
            &self.part,
            rect,
            tiny_skia::Transform::identity(),
            None,
            Some("+mn-lt".to_string()),
            self.idx,
        );
        let widths = sb.scene.items[marks.0..]
            .iter()
            .filter_map(|i| match i {
                crate::office::pptx::scene::Item::Text(g) => Some(g.width),
                _ => None,
            })
            .collect();
        sb.scene.items.truncate(marks.0);
        sb.scene.links.truncate(marks.1);
        sb.scene.warnings.truncate(marks.2);
        sb.scene.shapes[self.idx].paragraphs.truncate(marks.3);
        (h, widths)
    }
}

#[cfg(test)]
mod tests;
