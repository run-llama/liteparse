//! Builds a slide's scene: background, then the master's and layout's decoration
//! shapes, then the slide's own shapes, resolving placeholder inheritance, group
//! transforms, fills, outlines, pictures, text, tables and SmartArt drawings.
//!
//! Shape trees from slides use `p:` elements and SmartArt drawings use `dsp:`; the
//! walker matches on local names so both go through the same code.

use std::rc::Rc;

use tiny_skia::Transform;

use crate::office::pptx::color::Rgba;
use crate::office::pptx::config::Config;
use crate::office::pptx::document::{Document, SlideChain};
use crate::office::pptx::effects;
use crate::office::pptx::fonts::FontSystem;
use crate::office::pptx::geom::{self, Cmd, FillMode, Geometry};
use crate::office::pptx::scene::{
    ImageItem, Item, Paint, PathItem, Rect, Scene, ShadowItem, ShadowSource, ShapeIdx, ShapeInfo,
    SoftEdge,
};
use crate::office::pptx::style::{self, Fill, StyleCtx, read_line, shape_fill};
use crate::office::pptx::textlayout::{self, LayoutEnv, ParaInput, RunInput, TextBox};
use crate::office::pptx::textprops::{self, BodyProps, resolve_para};
use crate::office::pptx::xml::XNode;

pub const EMU_PER_PT: f64 = 12_700.0;

/// Child of `n` with local name `local` under either the `p:` or `dsp:` prefix.
fn pc<'a>(n: &'a XNode, local: &str) -> Option<&'a Rc<XNode>> {
    n.children
        .iter()
        .find(|c| (c.name.starts_with("p:") || c.name.starts_with("dsp:")) && c.local() == local)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PartKind {
    Master,
    Layout,
    Slide,
}

/// Maps a group's child coordinate space into slide space.
#[derive(Clone, Copy)]
struct GroupMap {
    /// Group frame (scaled child space) → slide.
    tf: Transform,
    off: (f64, f64),
    ch_off: (f64, f64),
    scale: (f64, f64),
}

impl GroupMap {
    fn identity() -> GroupMap {
        GroupMap {
            tf: Transform::identity(),
            off: (0.0, 0.0),
            ch_off: (0.0, 0.0),
            scale: (1.0, 1.0),
        }
    }

    /// Maps a child xfrm (EMU) to a rect in the group's frame (points).
    fn map(&self, x: f64, y: f64, w: f64, h: f64) -> (f64, f64, f64, f64) {
        (
            (self.off.0 + (x - self.ch_off.0) * self.scale.0) / EMU_PER_PT,
            (self.off.1 + (y - self.ch_off.1) * self.scale.1) / EMU_PER_PT,
            w * self.scale.0 / EMU_PER_PT,
            h * self.scale.1 / EMU_PER_PT,
        )
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Xfrm {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    rot: f64,
    flip_h: bool,
    flip_v: bool,
}

fn read_xfrm(x: &XNode) -> Option<Xfrm> {
    let off = x.child("a:off");
    let ext = x.child("a:ext");
    if off.is_none() && ext.is_none() {
        return None;
    }
    Some(Xfrm {
        x: off.and_then(|o| o.attr_f64("x")).unwrap_or(0.0),
        y: off.and_then(|o| o.attr_f64("y")).unwrap_or(0.0),
        w: ext.and_then(|e| e.attr_f64("cx")).unwrap_or(0.0).max(0.0),
        h: ext.and_then(|e| e.attr_f64("cy")).unwrap_or(0.0).max(0.0),
        rot: x.attr_f64("rot").unwrap_or(0.0) / 60_000.0,
        flip_h: x.attr_bool("flipH").unwrap_or(false),
        flip_v: x.attr_bool("flipV").unwrap_or(false),
    })
}

/// Shape-local (points, origin top-left) → group frame transform for a shape
/// placed at (x, y, w, h) with rotation and flips. `text` omits the flips
/// (text is never mirrored; flipV turns it upside down instead).
#[expect(
    clippy::too_many_arguments,
    reason = "takes a shape's xfrm fields (box, rotation, flips) plus the text flag"
)]
fn local_tf(
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    rot: f64,
    flip_h: bool,
    flip_v: bool,
    text: bool,
) -> Transform {
    let (cx, cy) = ((w / 2.0) as f32, (h / 2.0) as f32);
    let mut t = Transform::from_translate(-cx, -cy);
    if text {
        if flip_v {
            t = t.post_concat(Transform::from_rotate(180.0));
        }
    } else {
        let sx = if flip_h { -1.0 } else { 1.0 };
        let sy = if flip_v { -1.0 } else { 1.0 };
        t = t.post_concat(Transform::from_scale(sx, sy));
    }
    t = t.post_concat(Transform::from_rotate(rot as f32));
    t.post_concat(Transform::from_translate(x as f32 + cx, y as f32 + cy))
}

/// `p:ph/@idx` PowerPoint writes on a slide placeholder that has no parent on its layout
/// (the 32-bit all-ones value, e.g. after a layout change dropped the parent).
const ORPHAN_PH_IDX: i64 = 4_294_967_295;

/// Placeholder identity of a shape (`p:nvPr/p:ph`).
#[derive(Clone, Debug)]
struct Ph {
    kind: String,
    idx: Option<i64>,
}

fn placeholder(sp: &XNode) -> Option<Ph> {
    let nv = sp
        .children
        .iter()
        .find(|c| c.local().starts_with("nv") && c.local().ends_with("Pr"))?;
    let ph = pc(nv, "nvPr")?.child("p:ph")?;
    Some(Ph {
        kind: ph.attr("type").unwrap_or("obj").to_string(),
        idx: ph.attr_i64("idx"),
    })
}

fn ph_family(kind: &str) -> &str {
    match kind {
        "ctrTitle" | "title" => "title",
        "dt" | "ftr" | "sldNum" | "hdr" => kind,
        _ => "body",
    }
}

fn tree_of(part: &XNode) -> Option<&Rc<XNode>> {
    part.path(&["p:cSld", "p:spTree"])
}

/// Finds the placeholder in `tree` that `ph` inherits from.
fn find_ph(tree: &XNode, ph: &Ph, by_idx: bool) -> Option<Rc<XNode>> {
    let mut all = Vec::new();
    collect_ph(tree, &mut all);
    if by_idx
        && let Some(idx) = ph.idx
        && let Some((n, _)) = all.iter().find(|(_, p)| p.idx == Some(idx))
    {
        return Some(n.clone());
    }
    if let Some((n, _)) = all.iter().find(|(_, p)| p.kind == ph.kind) {
        return Some(n.clone());
    }
    let fam = ph_family(&ph.kind);
    all.iter()
        .find(|(_, p)| ph_family(&p.kind) == fam)
        .map(|(n, _)| n.clone())
}

fn collect_ph(tree: &XNode, out: &mut Vec<(Rc<XNode>, Ph)>) {
    for c in &tree.children {
        if let Some(p) = placeholder(c) {
            out.push((c.clone(), p));
        }
        if c.local() == "grpSp" {
            collect_ph(c, out);
        }
    }
}

pub struct SlideBuilder<'a> {
    pub doc: &'a Document,
    pub fonts: &'a FontSystem,
    pub images: &'a crate::office::pptx::render::ImageCache<'a>,
    pub cfg: &'a Config,
    pub chain: &'a SlideChain,
    pub slide_number: u32,
    pub scene: Scene,
    shape_count: usize,
    /// The slide's effective background paint, in slide space (for `useBgFill`).
    background: Option<Paint>,
    /// Fill of each enclosing group, innermost last, in slide space (for `a:grpFill`).
    group_fills: Vec<Option<Paint>>,
    /// Set while a table measures a cell: a cell holding only empty paragraphs is
    /// still as tall as their end marks (see `layout_text`).
    pub(crate) measure_empty: bool,
    /// Set while an equation object's preview is drawn: PowerPoint keeps no text
    /// for equations, so their metafile text is not extracted.
    in_equation: bool,
}

/// Whether an OLE `progId` names an equation (Microsoft Equation 3.0, MathType).
fn is_equation_prog_id(prog_id: &str) -> bool {
    let p = prog_id.to_ascii_lowercase();
    p.starts_with("equation.") || p.starts_with("mathtype")
}

impl<'a> SlideBuilder<'a> {
    pub fn new(
        doc: &'a Document,
        fonts: &'a FontSystem,
        images: &'a crate::office::pptx::render::ImageCache<'a>,
        cfg: &'a Config,
        chain: &'a SlideChain,
        slide_number: u32,
    ) -> Self {
        let scene = Scene {
            width: doc.slide_w_pt(),
            height: doc.slide_h_pt(),
            ..Default::default()
        };
        SlideBuilder {
            doc,
            fonts,
            images,
            cfg,
            chain,
            slide_number,
            scene,
            shape_count: 0,
            background: None,
            group_fills: Vec::new(),
            measure_empty: false,
            in_equation: false,
        }
    }

    /// The returned context borrows the document and chain, not the builder, so
    /// the builder can keep emitting while it is alive.
    pub(crate) fn style_ctx<'p>(&self, part: &'p str) -> StyleCtx<'p>
    where
        'a: 'p,
    {
        let chain: &'a SlideChain = self.chain;
        let doc: &'a Document = self.doc;
        StyleCtx {
            theme: &chain.theme,
            colors: &chain.colors,
            part,
            theme_part: &chain.theme_part,
            pkg: &doc.pkg,
        }
    }

    pub fn build(mut self) -> Scene {
        self.background();
        let chain = self.chain;
        let show_master = |n: &Option<Rc<XNode>>| {
            n.as_ref()
                .and_then(|x| x.attr_bool("showMasterSp"))
                .unwrap_or(true)
        };
        let slide_shows = show_master(&Some(chain.slide.clone()));
        let layout_shows_master = show_master(&chain.layout);
        if slide_shows {
            if layout_shows_master
                && let (Some(m), Some(mp)) = (&chain.master, &chain.master_part)
                && let Some(tree) = tree_of(m)
            {
                self.walk(tree, mp, PartKind::Master, GroupMap::identity(), None);
            }
            if let (Some(l), Some(lp)) = (&chain.layout, &chain.layout_part)
                && let Some(tree) = tree_of(l)
            {
                self.walk(tree, lp, PartKind::Layout, GroupMap::identity(), None);
            }
        }
        if let Some(tree) = tree_of(&chain.slide) {
            let sp = chain.slide_part.as_str();
            self.walk(tree, sp, PartKind::Slide, GroupMap::identity(), None);
        }
        self.scene
    }

    fn background(&mut self) {
        let chain = self.chain;
        let candidates: [(Option<&Rc<XNode>>, Option<&String>); 3] = [
            (Some(&chain.slide), Some(&chain.slide_part)),
            (chain.layout.as_ref(), chain.layout_part.as_ref()),
            (chain.master.as_ref(), chain.master_part.as_ref()),
        ];
        let full = Rect {
            x: 0.0,
            y: 0.0,
            w: self.scene.width,
            h: self.scene.height,
        };
        for (node, part) in candidates {
            let (Some(node), Some(part)) = (node, part) else {
                continue;
            };
            let Some(bg) = node.path(&["p:cSld", "p:bg"]) else {
                continue;
            };
            let ctx = self.style_ctx(part);
            let fill = if let Some(bg_pr) = bg.child("p:bgPr") {
                style::fill_child(bg_pr).map(|f| style::read_fill(f, &ctx, None, full))
            } else if let Some(bg_ref) = bg.child("p:bgRef") {
                let idx = bg_ref.attr_i64("idx").unwrap_or(0);
                let ref_color = ctx.color(bg_ref, None);
                match chain.theme.fill_style(idx) {
                    Some(f) => Some(style::read_fill(f, &ctx.for_theme(), ref_color, full)),
                    None => ref_color.map(|c| Fill::Paint(Paint::Solid(c))),
                }
            } else {
                None
            };
            if let Some(Fill::Paint(p)) = fill {
                self.background = Some(p.clone());
                if let Some(path) = textlayout::rect_path(0.0, 0.0, full.w, full.h) {
                    self.scene.items.push(Item::Path(PathItem {
                        path,
                        fill: Some(p),
                        stroke: None,
                        has_curve: false,
                        shape: None,
                        from_text: false,
                    }));
                }
            }
            return;
        }
    }

    fn walk(
        &mut self,
        tree: &XNode,
        part: &str,
        kind: PartKind,
        gm: GroupMap,
        parent: Option<ShapeIdx>,
    ) {
        for el in &tree.children {
            if self.shape_count >= self.cfg.limits.max_shapes_per_slide as usize {
                self.scene
                    .warnings
                    .push(("SHAPE_LIMIT".into(), "shape limit reached".into()));
                return;
            }
            // Hidden shapes draw nothing, but their text is still extracted (as
            // invisible items), matching what the LibreOffice PDF path exposed.
            let hidden = is_hidden(el);
            if hidden && kind != PartKind::Slide {
                continue;
            }
            let (items_before, shapes_before) = (self.scene.items.len(), self.scene.shapes.len());
            if el.local() == "textOnly" {
                // The modern version of content drawn by its fallback picture: only
                // its text is kept, as items that are reported but not drawn.
                self.walk(el, part, kind, gm, parent);
                let tail: Vec<Item> = self.scene.items.drain(items_before..).collect();
                for item in tail {
                    if let Item::Text(mut run) = item {
                        run.drawn = false;
                        self.scene.items.push(Item::Text(run));
                    }
                }
                for sh in &mut self.scene.shapes[shapes_before..] {
                    sh.hidden = true;
                }
                continue;
            }
            self.walk_one(el, part, kind, gm, parent);
            if hidden {
                let tail: Vec<Item> = self.scene.items.drain(items_before..).collect();
                for item in tail {
                    if let Item::Text(mut run) = item {
                        run.color.a = 0.0;
                        self.scene.items.push(Item::Text(run));
                    }
                }
                for sh in &mut self.scene.shapes[shapes_before..] {
                    sh.hidden = true;
                }
            }
        }
    }

    fn walk_one(
        &mut self,
        el: &XNode,
        part: &str,
        kind: PartKind,
        gm: GroupMap,
        parent: Option<ShapeIdx>,
    ) {
        match el.local() {
            "sp" | "cxnSp" => {
                // Master/layout placeholders are templates, not drawn content.
                if kind != PartKind::Slide && placeholder(el).is_some() {
                    return;
                }
                self.shape_count += 1;
                self.shape(el, part, kind, gm, parent);
            }
            "pic" => {
                if kind != PartKind::Slide && placeholder(el).is_some() {
                    return;
                }
                self.shape_count += 1;
                self.picture(el, part, kind, gm, parent);
            }
            "grpSp" => {
                self.shape_count += 1;
                self.group(el, part, kind, gm, parent);
            }
            "graphicFrame" => {
                if kind != PartKind::Slide && placeholder(el).is_some() {
                    return;
                }
                self.shape_count += 1;
                self.graphic_frame(el, part, kind, gm, parent);
            }
            _ => {}
        }
    }

    fn register(
        &mut self,
        el: &XNode,
        kind_name: &'static str,
        bounds: Rect,
        rot: f64,
        parent: Option<ShapeIdx>,
        inherited: bool,
    ) -> ShapeIdx {
        let nv = el.children.iter().find(|c| c.local().starts_with("nv"));
        let c_nv = nv.and_then(|n| pc(n, "cNvPr"));
        let alt = c_nv
            .and_then(|c| c.attr("descr").or(c.attr("title")))
            .filter(|s| !s.trim().is_empty())
            .map(str::to_string);
        let ph = placeholder(el).map(|p| p.kind);
        self.scene.shapes.push(ShapeInfo {
            id: c_nv.and_then(|c| c.attr("id")).unwrap_or("").to_string(),
            name: c_nv.and_then(|c| c.attr("name")).unwrap_or("").to_string(),
            parent,
            kind: kind_name,
            placeholder: ph,
            bounds,
            rotation: rot,
            alt_text: alt,
            hyperlink: None,
            paragraphs: Vec::new(),
            inherited,
            hidden: false,
        });
        self.scene.shapes.len() - 1
    }

    /// Resolves the inheritance sources of a slide placeholder: (layout ph, master ph).
    fn ph_sources(&self, el: &XNode, kind: PartKind) -> (Option<Rc<XNode>>, Option<Rc<XNode>>) {
        let Some(ph) = placeholder(el) else {
            return (None, None);
        };
        let chain = self.chain;
        let master_tree = chain.master.as_ref().and_then(|m| tree_of(m).cloned());
        match kind {
            PartKind::Slide => {
                // A slide placeholder saved with the orphan idx has no layout counterpart,
                // even when the layout has one of its type: it inherits from the master only.
                let orphan = ph.idx == Some(ORPHAN_PH_IDX);
                let layout_ph = chain
                    .layout
                    .as_ref()
                    .and_then(|l| tree_of(l))
                    .filter(|_| !orphan)
                    .and_then(|t| find_ph(t, &ph, true));
                let master_key = layout_ph
                    .as_ref()
                    .and_then(|l| placeholder(l))
                    .unwrap_or(ph);
                let master_ph = master_tree.and_then(|t| find_ph(&t, &master_key, false));
                (layout_ph, master_ph)
            }
            PartKind::Layout => (None, master_tree.and_then(|t| find_ph(&t, &ph, false))),
            PartKind::Master => (None, None),
        }
    }

    /// The part each inheritance source lives in, parallel to `[el, layout_ph, master_ph]`
    /// with the missing placeholders left out (inherited fills resolve images there).
    fn source_parts(
        &self,
        part: &str,
        layout_ph: Option<&Rc<XNode>>,
        master_ph: Option<&Rc<XNode>>,
    ) -> Vec<String> {
        let chain = self.chain;
        [
            Some(part.to_string()),
            layout_ph.map(|_| chain.layout_part.clone().unwrap_or_default()),
            master_ph.map(|_| chain.master_part.clone().unwrap_or_default()),
        ]
        .into_iter()
        .flatten()
        .collect()
    }

    fn shape(
        &mut self,
        el: &XNode,
        part: &str,
        kind: PartKind,
        gm: GroupMap,
        parent: Option<ShapeIdx>,
    ) {
        let (layout_ph, master_ph) = self.ph_sources(el, kind);
        let sources: Vec<&XNode> = [Some(el), layout_ph.as_deref(), master_ph.as_deref()]
            .into_iter()
            .flatten()
            .collect();
        let sp_pr = |n: &XNode| pc(n, "spPr").cloned();
        let Some(xf) = sources
            .iter()
            .find_map(|s| sp_pr(s).and_then(|p| p.child("a:xfrm").and_then(|x| read_xfrm(x))))
        else {
            return;
        };
        let (x, y, w, h) = gm.map(xf.x, xf.y, xf.w, xf.h);
        let shape_tf = local_tf(x, y, w, h, xf.rot, xf.flip_h, xf.flip_v, false).post_concat(gm.tf);
        let text_tf = local_tf(x, y, w, h, xf.rot, xf.flip_h, xf.flip_v, true).post_concat(gm.tf);
        let bounds =
            textlayout::transformed_bbox(&shape_tf, 0.0, 0.0, w, h).unwrap_or(Rect { x, y, w, h });
        let idx = self.register(
            el,
            if el.local() == "cxnSp" { "cxnSp" } else { "sp" },
            bounds,
            xf.rot,
            parent,
            kind != PartKind::Slide,
        );

        let geom_node = sources.iter().find_map(|s| {
            sp_pr(s).and_then(|p| p.child("a:prstGeom").or(p.child("a:custGeom")).cloned())
        });
        let geometry = geom::build(geom_node.as_deref(), w * EMU_PER_PT, h * EMU_PER_PT);
        let style_el = sources.iter().find_map(|s| pc(s, "style").cloned());
        let fill_ref = style_el
            .as_ref()
            .and_then(|s| s.child("a:fillRef").cloned());
        let ln_ref = style_el.as_ref().and_then(|s| s.child("a:lnRef").cloned());
        let font_ref = style_el
            .as_ref()
            .and_then(|s| s.child("a:fontRef").cloned());

        let ctx = self.style_ctx(part);
        let local_bounds = Rect {
            x: 0.0,
            y: 0.0,
            w,
            h,
        };
        // Fill: own spPr, then inherited placeholder spPr, then the style matrix.
        // An inherited fill resolves its picture relationships in its own part.
        let source_parts = self.source_parts(part, layout_ph.as_ref(), master_ph.as_ref());
        let fill = sources
            .iter()
            .zip(source_parts.iter())
            .find_map(|(s, p)| {
                sp_pr(s)
                    .and_then(|sp| style::fill_child(&sp).cloned())
                    .map(|f| (f, p.as_str()))
            })
            .map(|(f, p)| style::read_fill(&f, &ctx.for_part(p), None, local_bounds))
            .or_else(|| shape_fill(None, fill_ref.as_deref(), &ctx, local_bounds));
        // useBgFill: the shape shows the slide background, placed as on the slide,
        // unless its own spPr sets a fill (PowerPoint draws the explicit fill).
        let own_fill = sp_pr(el).as_deref().and_then(style::fill_child).is_some();
        let fill = if el.attr_bool("useBgFill") == Some(true) && !own_fill {
            let inv = shape_tf.invert().unwrap_or_default();
            Some(match &self.background {
                Some(p) => Fill::Paint(transform_paint(p.clone(), &inv)),
                None => Fill::Paint(Paint::Solid(Rgba::WHITE)),
            })
        } else {
            fill
        };
        let fill = self.resolve_group_fill(fill, &shape_tf);
        let own_ln = sources
            .iter()
            .find_map(|s| sp_pr(s).and_then(|p| p.child("a:ln").cloned()));
        let stroke = read_line(own_ln.as_deref(), ln_ref.as_deref(), &ctx, local_bounds);
        let own_effects = sources
            .iter()
            .find_map(|s| sp_pr(s).and_then(|p| p.child("a:effectLst").cloned()));
        let effect_ref = style_el
            .as_ref()
            .and_then(|s| s.child("a:effectRef").cloned());
        let (own_fx, fx_ref) = (own_effects.as_deref(), effect_ref.as_deref());
        let filled = matches!(fill, Some(Fill::Paint(_)));
        let soft_rad = effects::read_soft_edge(own_fx, fx_ref, &ctx);
        // Shadow and glow come from the soft-edged silhouette.
        let fx = [
            style::read_shadow(own_fx, fx_ref, &ctx),
            effects::read_glow(own_fx, fx_ref, &ctx),
        ];
        for mut shadow in fx.into_iter().flatten() {
            shadow.soft = soft_rad.unwrap_or(0.0);
            self.push_shadow(&geometry, shadow, shape_tf, filled, stroke.as_ref());
        }
        // Soft edges fade the fill and outline, not the text.
        let soft = soft_rad.and_then(|radius| {
            let path = effects::silhouette(&geometry, shape_tf, filled, stroke.as_ref())?;
            Some(SoftEdge { path, radius })
        });
        let soft = soft.map(|s| self.scene.items.push(Item::PushSoftEdge(s)));
        self.emit_geometry(&geometry, fill, stroke, shape_tf, Some(idx));
        if filled
            && let Some(overlay) = effects::read_fill_overlay(own_fx, fx_ref, &ctx, local_bounds)
        {
            self.push_multiply(&geometry, overlay, shape_tf, idx);
        }
        if soft.is_some() {
            self.scene.items.push(Item::PopSoftEdge);
        }

        // Text.
        let tx = pc(el, "txBody").cloned();
        if let Some(tx) = tx {
            let text_rect = match pc(el, "txXfrm").and_then(|t| read_xfrm(t)) {
                // SmartArt shapes carry their own text box, in the frame's space.
                Some(t) => {
                    let (tx0, ty0, tw, th) = gm.map(t.x, t.y, t.w, t.h);
                    geom::TextRect {
                        l: tx0 - x,
                        t: ty0 - y,
                        r: tx0 - x + tw,
                        b: ty0 - y + th,
                    }
                }
                None => geometry.text_rect,
            };
            let font_ref_family = font_ref.as_ref().and_then(|f| match f.attr("idx") {
                Some("major") => Some("+mj-lt".to_string()),
                Some("minor") => Some("+mn-lt".to_string()),
                _ => None,
            });
            let font_ref_color = font_ref.as_ref().and_then(|f| ctx.color(f, None));
            self.text_body(
                &tx,
                el,
                part,
                kind,
                layout_ph.as_ref(),
                master_ph.as_ref(),
                text_rect,
                text_tf,
                font_ref_color,
                font_ref_family,
                idx,
            );
        }
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "takes the shape's placeholder chain, text frame and inherited text defaults"
    )]
    fn text_body(
        &mut self,
        tx: &XNode,
        el: &XNode,
        part: &str,
        kind: PartKind,
        layout_ph: Option<&Rc<XNode>>,
        master_ph: Option<&Rc<XNode>>,
        rect: geom::TextRect,
        transform: Transform,
        default_color: Option<Rgba>,
        default_font: Option<String>,
        idx: ShapeIdx,
    ) {
        let chain = self.chain;
        let ph = placeholder(el);
        // List style chain, general → specific. The presentation's default text
        // style is for ordinary text boxes; placeholders start from the master's
        // text styles (and the built-in 18pt default when those set no size).
        let mut lists: Vec<Rc<XNode>> = Vec::new();
        if ph.is_none()
            && let Some(d) = self.doc.default_text_style()
        {
            lists.push(d.clone());
        }
        let master_styles = chain
            .master
            .as_ref()
            .and_then(|m| m.child("p:txStyles").cloned());
        let style_name = match ph.as_ref().map(|p| ph_family(&p.kind)) {
            Some("title") => Some("p:titleStyle"),
            Some("body") => Some("p:bodyStyle"),
            Some(_) => Some("p:otherStyle"),
            None => None,
        };
        // Text boxes on a master use its otherStyle; those on layouts, like slides,
        // only the presentation defaults.
        let style_name = style_name.or(if kind == PartKind::Master {
            Some("p:otherStyle")
        } else {
            None
        });
        if let (Some(ms), Some(name)) = (&master_styles, style_name)
            && let Some(s) = ms.child(name)
        {
            lists.push(s.clone());
        }
        let tx_of = |n: Option<&Rc<XNode>>| n.and_then(|n| pc(n, "txBody").cloned());
        let mut bodies: Vec<Rc<XNode>> = Vec::new();
        for src in [tx_of(master_ph), tx_of(layout_ph)].into_iter().flatten() {
            if let Some(ls) = src.child("a:lstStyle") {
                lists.push(ls.clone());
            }
            if let Some(b) = src.child("a:bodyPr") {
                bodies.push(b.clone());
            }
        }
        // The shape style's fontRef color beats the inherited list styles (a default
        // rectangle's text is lt1 although the text defaults say tx1), but not the
        // shape's own lstStyle or run properties.
        if let Some(c) = default_color {
            lists.extend(textprops::uniform_list_style(
                "",
                &textprops::solid_fill_xml(c),
            ));
        }
        if let Some(ls) = tx.child("a:lstStyle") {
            lists.push(ls.clone());
        }
        if let Some(b) = tx.child("a:bodyPr") {
            bodies.push(b.clone());
        }
        self.layout_text(
            tx,
            &lists,
            &bodies,
            part,
            rect,
            transform,
            default_color,
            default_font,
            idx,
        );
    }

    /// Lays out a text body given its list-style chain and bodyPr chain (both
    /// general → specific). Shared by shapes and table cells.
    #[expect(
        clippy::too_many_arguments,
        reason = "takes the style chains, text frame and inherited defaults for shapes and cells"
    )]
    pub(crate) fn layout_text(
        &mut self,
        tx: &XNode,
        lists: &[Rc<XNode>],
        bodies: &[Rc<XNode>],
        part: &str,
        rect: geom::TextRect,
        transform: Transform,
        default_color: Option<Rgba>,
        default_font: Option<String>,
        idx: ShapeIdx,
    ) -> f64 {
        let chain = self.chain;
        let mut body = BodyProps::default();
        for b in bodies {
            body.merge(b);
        }
        let mut paras = Vec::new();
        for p in tx.children_named("a:p") {
            let ppr = p.child("a:pPr");
            let level = ppr.and_then(|x| x.attr_i64("lvl")).unwrap_or(0).clamp(0, 8) as u32;
            let props = resolve_para(lists, level, ppr.map(|n| n.as_ref()));
            let mut runs = Vec::new();
            for r in &p.children {
                match r.name.as_str() {
                    "a:r" | "a:fld" => {
                        let mut rp = props.def_run.clone();
                        if let Some(rpr) = r.child("a:rPr") {
                            rp.merge(rpr);
                        }
                        let mut text = r.child("a:t").map(|t| t.text.clone()).unwrap_or_default();
                        if r.name == "a:fld" && r.attr("type") == Some("slidenum") {
                            text = self.slide_number.to_string();
                        }
                        // A vertical tab is a line break inside the run.
                        for (k, piece) in text.split('\u{b}').enumerate() {
                            if k > 0 {
                                runs.push(RunInput {
                                    text: String::new(),
                                    props: rp.clone(),
                                    is_break: true,
                                });
                            }
                            runs.push(RunInput {
                                text: piece.to_string(),
                                props: rp.clone(),
                                is_break: false,
                            });
                        }
                    }
                    "a14:m" => {
                        // Inline equation: linear text, styled like its first math run.
                        let mut rp = props.def_run.clone();
                        if let Some(rpr) = r.find("a:rPr") {
                            rp.merge(rpr);
                        }
                        let text = crate::office::pptx::math::linearize(r);
                        if !text.is_empty() {
                            runs.push(RunInput {
                                text,
                                props: rp,
                                is_break: false,
                            });
                        }
                    }
                    "a:br" => {
                        let mut rp = props.def_run.clone();
                        if let Some(rpr) = r.child("a:rPr") {
                            rp.merge(rpr);
                        }
                        runs.push(RunInput {
                            text: String::new(),
                            props: rp,
                            is_break: true,
                        });
                    }
                    _ => {}
                }
            }
            // Without endParaRPr the paragraph mark takes the last run's formatting,
            // which sizes the paragraph's blank lines.
            let end_props = match p.child("a:endParaRPr") {
                Some(e) => {
                    let mut ep = props.def_run.clone();
                    ep.merge(e);
                    ep
                }
                None => runs
                    .last()
                    .map(|r| r.props.clone())
                    .unwrap_or_else(|| props.def_run.clone()),
            };
            // A newline inside run text starts a new paragraph, as in PowerPoint:
            // each piece before it closes a paragraph with the same properties.
            let mut pieces: Vec<Vec<RunInput>> = vec![Vec::new()];
            for run in runs {
                if run.is_break || !run.text.contains('\n') {
                    pieces.last_mut().unwrap().push(run);
                    continue;
                }
                for (k, text) in run.text.split('\n').enumerate() {
                    if k > 0 {
                        pieces.push(Vec::new());
                    }
                    pieces.last_mut().unwrap().push(RunInput {
                        text: text.to_string(),
                        props: run.props.clone(),
                        is_break: false,
                    });
                }
            }
            let last = pieces.len() - 1;
            for (k, runs) in pieces.into_iter().enumerate() {
                let end_props = if k == last {
                    end_props.clone()
                } else {
                    runs.last()
                        .map_or_else(|| props.def_run.clone(), |r| r.props.clone())
                };
                paras.push(ParaInput {
                    level,
                    props: props.clone(),
                    runs,
                    end_props,
                });
            }
        }
        if !self.measure_empty
            && paras
                .iter()
                .all(|p| p.runs.iter().all(|r| r.text.is_empty() && !r.is_break))
        {
            return 0.0;
        }
        let tb = TextBox {
            rect,
            transform,
            body,
            paras,
            default_color,
            default_font,
            shape: Some(idx),
            unknown_font: Default::default(),
            narrow: Default::default(),
        };
        let mut scene = std::mem::take(&mut self.scene);
        let res = {
            let ctx = self.style_ctx(part);
            let env = LayoutEnv {
                fonts: self.fonts,
                theme: &chain.theme,
                style: &ctx,
                slide_number: self.slide_number,
            };
            textlayout::layout(&tb, &env, &mut scene)
        };
        self.scene = scene;
        self.scene.shapes[idx].paragraphs.extend(res.paragraphs);
        res.height
    }

    /// Text drawn inside a metafile picture, as text items placed over the
    /// picture (not drawn again: the pixels are in the picture). `unit` maps the
    /// visible picture's unit square to slide space; `crop` is its srcRect.
    fn metafile_text(
        &mut self,
        part: &str,
        unit: Transform,
        crop: (f64, f64, f64, f64),
        idx: ShapeIdx,
    ) {
        if self.in_equation {
            return;
        }
        let runs = metafile_text_runs(
            self.images,
            self.doc,
            self.fonts,
            part,
            unit,
            crop,
            Some(idx),
        );
        self.scene.items.extend(runs.into_iter().map(Item::Text));
    }

    /// Fallback for SmartArt saved without its drawing: the node text as an
    /// indented list centred in the frame, sized to fit, so it is visible and
    /// extracted. PowerPoint itself would lay the diagram out from its layout part.
    fn smart_art_list(
        &mut self,
        nodes: &[(u32, String)],
        part: &str,
        frame: Rect,
        tf: Transform,
        idx: ShapeIdx,
    ) {
        if nodes.is_empty() {
            return;
        }
        let lines: f64 = nodes
            .iter()
            .map(|(_, t)| 1.0 + t.matches('\n').count() as f64)
            .sum();
        let size = (frame.h / (lines * 1.5)).clamp(8.0, 24.0);
        let paras: String = nodes
            .iter()
            .flat_map(|(level, t)| t.split('\n').map(move |line| (*level, line.to_string())))
            .map(|(level, line)| {
                format!(
                    r#"<a:p><a:pPr marL="{}" lvl="{}"/><a:r><a:rPr sz="{}"/><a:t>{}</a:t></a:r></a:p>"#,
                    (level.min(8) as i64) * 228_600,
                    level.min(8),
                    (size * 100.0).round() as i64,
                    xml_escape(&line)
                )
            })
            .collect();
        let ns = r#"xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main""#;
        let tx_xml = format!(
            r#"<p:txBody {ns}><a:bodyPr anchor="ctr" wrap="square"/><a:lstStyle/>{paras}</p:txBody>"#
        );
        let Ok(tx) = crate::office::pptx::xml::parse(&tx_xml) else {
            return;
        };
        let mut lists: Vec<Rc<XNode>> = Vec::new();
        if let Some(d) = self.doc.default_text_style() {
            lists.push(d.clone());
        }
        let bodies: Vec<Rc<XNode>> = tx.child("a:bodyPr").cloned().into_iter().collect();
        let rect = geom::TextRect {
            l: 0.0,
            t: 0.0,
            r: frame.w,
            b: frame.h,
        };
        self.layout_text(&tx, &lists, &bodies, part, rect, tf, None, None, idx);
    }

    /// Shadow of a shape: its filled outline, or the stroked outline of an
    /// unfilled shape.
    fn push_shadow(
        &mut self,
        g: &Geometry,
        shadow: crate::office::pptx::scene::Shadow,
        tf: Transform,
        filled: bool,
        stroke: Option<&crate::office::pptx::scene::Stroke>,
    ) {
        for gp in &g.paths {
            let Some((path, _)) = to_skia_path(&gp.cmds) else {
                continue;
            };
            let Some(path) = path.transform(tf) else {
                continue;
            };
            let outline = if filled && gp.fill != FillMode::None {
                Some(path)
            } else {
                stroke.and_then(|s| {
                    let st = tiny_skia::Stroke {
                        width: s.width as f32,
                        ..Default::default()
                    };
                    path.stroke(&st, 1.0)
                })
            };
            if let Some(path) = outline {
                self.scene.items.push(Item::Shadow(ShadowItem {
                    source: ShadowSource::Path(path),
                    shadow,
                }));
            }
        }
    }

    /// `a:fillOverlay`: the overlay paint multiplied over the shape's filled paths.
    fn push_multiply(&mut self, g: &Geometry, paint: Paint, tf: Transform, idx: ShapeIdx) {
        let paint = transform_paint(paint, &tf);
        for gp in g.paths.iter().filter(|gp| gp.fill != FillMode::None) {
            let Some((path, has_curve)) = to_skia_path(&gp.cmds) else {
                continue;
            };
            let Some(path) = path.transform(tf) else {
                continue;
            };
            self.scene.items.push(Item::Multiply(PathItem {
                path,
                fill: Some(paint.clone()),
                stroke: None,
                has_curve,
                shape: Some(idx),
                from_text: false,
            }));
        }
    }

    pub(crate) fn emit_geometry(
        &mut self,
        g: &Geometry,
        fill: Option<Fill>,
        stroke: Option<crate::office::pptx::scene::Stroke>,
        tf: Transform,
        idx: Option<ShapeIdx>,
    ) {
        let fill_paint = match fill {
            Some(Fill::Paint(p)) => Some(p),
            _ => None,
        };
        for gp in &g.paths {
            let Some((path, has_curve)) = to_skia_path(&gp.cmds) else {
                continue;
            };
            let fill = if gp.fill == FillMode::None {
                None
            } else {
                fill_paint.as_ref().map(|p| shade_paint(p, gp.fill))
            };
            let stroke = if gp.stroke { stroke.clone() } else { None };
            if fill.is_none() && stroke.is_none() {
                continue;
            }
            // Paint geometry lives in shape-local space; map it with the shape.
            let fill = fill.map(|p| transform_paint(p, &tf));
            let stroke = stroke.map(|mut s| {
                s.paint = transform_paint(s.paint, &tf);
                s
            });
            let Some(path) = path.transform(tf) else {
                continue;
            };
            self.scene.items.push(Item::Path(PathItem {
                path,
                fill,
                stroke,
                has_curve,
                shape: idx,
                from_text: false,
            }));
        }
    }

    fn picture(
        &mut self,
        el: &XNode,
        part: &str,
        kind: PartKind,
        gm: GroupMap,
        parent: Option<ShapeIdx>,
    ) {
        let (layout_ph, master_ph) = self.ph_sources(el, kind);
        let sources: Vec<&XNode> = [Some(el), layout_ph.as_deref(), master_ph.as_deref()]
            .into_iter()
            .flatten()
            .collect();
        let Some(xf) = sources
            .iter()
            .find_map(|s| pc(s, "spPr").and_then(|p| p.child("a:xfrm").and_then(|x| read_xfrm(x))))
        else {
            return;
        };
        let (x, y, w, h) = gm.map(xf.x, xf.y, xf.w, xf.h);
        let shape_tf = local_tf(x, y, w, h, xf.rot, xf.flip_h, xf.flip_v, false).post_concat(gm.tf);
        let bounds =
            textlayout::transformed_bbox(&shape_tf, 0.0, 0.0, w, h).unwrap_or(Rect { x, y, w, h });
        let idx = self.register(el, "pic", bounds, xf.rot, parent, kind != PartKind::Slide);
        let ctx = self.style_ctx(part);
        // A picture in a placeholder inherits geometry, fill, outline, effects and
        // style from the layout/master placeholder, like any other shape.
        let parts = self.source_parts(part, layout_ph.as_ref(), master_ph.as_ref());
        let sp_pr_child = |pick: &dyn Fn(&XNode) -> Option<Rc<XNode>>| {
            sources.iter().zip(parts.iter()).find_map(|(s, p)| {
                pc(s, "spPr")
                    .and_then(|sp| pick(sp))
                    .map(|n| (n, p.clone()))
            })
        };
        let geom_node = sp_pr_child(&|p| p.child("a:prstGeom").or(p.child("a:custGeom")).cloned())
            .map(|(n, _)| n);
        let geometry = geom::build(geom_node.as_deref(), w * EMU_PER_PT, h * EMU_PER_PT);
        let style_el = sources.iter().find_map(|s| pc(s, "style").cloned());

        // Fill behind the picture (rare), then the picture, then the outline.
        if let Some((f, fill_part)) = sp_pr_child(&|p| style::fill_child(p).cloned()) {
            let fill = style::read_fill(
                &f,
                &ctx.for_part(&fill_part),
                None,
                Rect {
                    x: 0.0,
                    y: 0.0,
                    w,
                    h,
                },
            );
            let fill = self.resolve_group_fill(Some(fill), &shape_tf);
            self.emit_geometry(&geometry, fill, None, shape_tf, Some(idx));
        }
        let mut soft_open = false;
        let blip_fill = pc(el, "blipFill").cloned();
        if let Some(bf) = blip_fill
            && let Some(blip) = bf.child("a:blip")
        {
            if let Some(image) = ctx.blip_image(blip, None) {
                let crop = bf
                    .child("a:srcRect")
                    .map(|s| {
                        let f = |k: &str| s.attr_i64(k).unwrap_or(0) as f64 / 100_000.0;
                        (f("l"), f("t"), f("r"), f("b"))
                    })
                    .unwrap_or((0.0, 0.0, 0.0, 0.0));
                // fillRect insets the stretched picture inside the shape.
                let (fl, ft, fr, fb) = bf
                    .path(&["a:stretch", "a:fillRect"])
                    .map(|s| {
                        let f = |k: &str| s.attr_i64(k).unwrap_or(0) as f64 / 100_000.0;
                        (f("l"), f("t"), f("r"), f("b"))
                    })
                    .unwrap_or((0.0, 0.0, 0.0, 0.0));
                let px = w * fl;
                let py = h * ft;
                let pw = w * (1.0 - fl - fr);
                let ph = h * (1.0 - ft - fb);
                let unit =
                    Transform::from_row(pw as f32, 0.0, 0.0, ph as f32, px as f32, py as f32)
                        .post_concat(shape_tf);
                let clip = if geom_node
                    .as_ref()
                    .and_then(|g| g.attr("prst"))
                    .map(|p| p != "rect")
                    .unwrap_or(geom_node.is_some())
                {
                    geometry
                        .paths
                        .first()
                        .and_then(|gp| to_skia_path(&gp.cmds))
                        .and_then(|(p, _)| p.transform(shape_tf))
                } else {
                    None
                };
                let alt = self.scene.shapes[idx].alt_text.clone();
                let own_effects = sp_pr_child(&|p| p.child("a:effectLst").cloned()).map(|(n, _)| n);
                let effect_ref = style_el
                    .as_ref()
                    .and_then(|s| s.child("a:effectRef").cloned());
                let outline = clip.clone().or_else(|| {
                    textlayout::rect_path(0.0, 0.0, 1.0, 1.0).and_then(|p| p.transform(unit))
                });
                let (own_fx, fx_ref) = (own_effects.as_deref(), effect_ref.as_deref());
                let metafile_part = is_metafile_part(&image.part).then(|| image.part.clone());
                let item = ImageItem {
                    image,
                    transform: unit,
                    crop,
                    alpha: style::blip_alpha(blip),
                    clip,
                    shape: Some(idx),
                    alt_text: alt,
                };
                // Shadow and glow come from the soft-edged picture; a glow
                // follows the picture's own transparency.
                let soft_rad = effects::read_soft_edge(own_fx, fx_ref, &ctx);
                if let (Some(mut shadow), Some(path)) =
                    (style::read_shadow(own_fx, fx_ref, &ctx), outline.clone())
                {
                    shadow.soft = soft_rad.unwrap_or(0.0);
                    self.scene.items.push(Item::Shadow(ShadowItem {
                        source: ShadowSource::Path(path),
                        shadow,
                    }));
                }
                if let Some(mut glow) = effects::read_glow(own_fx, fx_ref, &ctx) {
                    glow.soft = soft_rad.unwrap_or(0.0);
                    self.scene.items.push(Item::Shadow(ShadowItem {
                        source: ShadowSource::Image(Box::new(item.clone())),
                        shadow: glow,
                    }));
                }
                // The soft edge fades the picture and its outline.
                if let (Some(radius), Some(path)) = (soft_rad, outline) {
                    self.scene
                        .items
                        .push(Item::PushSoftEdge(SoftEdge { path, radius }));
                    soft_open = true;
                }
                self.scene.items.push(Item::Image(item));
                if let Some(mp) = metafile_part {
                    self.metafile_text(&mp, unit, crop, idx);
                }
            } else if blip.attr("r:embed").is_some() && blip.attr("r:link").is_none() {
                // A picture whose data is missing: PowerPoint draws its "can't be
                // displayed" box, white with a 1 pt black outline, over what lies beneath.
                let outline = crate::office::pptx::scene::Stroke {
                    paint: Paint::Solid(Rgba::BLACK),
                    width: 1.0,
                    dash: Vec::new(),
                    cap: crate::office::pptx::scene::LineCap::Flat,
                    join: crate::office::pptx::scene::LineJoin::Miter,
                    head: None,
                    tail: None,
                };
                let white = Some(Fill::Paint(Paint::Solid(Rgba::WHITE)));
                self.emit_geometry(&geometry, white, Some(outline), shape_tf, Some(idx));
            }
        }
        let ln = sp_pr_child(&|p| p.child("a:ln").cloned()).map(|(n, _)| n);
        let ln_ref = style_el.as_ref().and_then(|s| s.child("a:lnRef").cloned());
        if let Some(stroke) = read_line(
            ln.as_deref(),
            ln_ref.as_deref(),
            &ctx,
            Rect {
                x: 0.0,
                y: 0.0,
                w,
                h,
            },
        ) {
            self.emit_geometry(&geometry, None, Some(stroke), shape_tf, Some(idx));
        }
        if soft_open {
            self.scene.items.push(Item::PopSoftEdge);
        }
        // Media: a picture carrying a video or audio file is its poster frame.
        let media = pc(el, "nvPicPr").and_then(|n| pc(n, "nvPr")).and_then(|n| {
            n.children
                .iter()
                .find(|c| matches!(c.local(), "videoFile" | "audioFile"))
                .cloned()
        });
        if let Some(m) = media {
            let kind = if m.local() == "videoFile" {
                "video"
            } else {
                "audio"
            };
            self.scene.media.push(serde_json::json!({
                "kind": kind,
                "coords": rect_json(&bounds),
                "shapeIndex": idx,
            }));
        }
    }

    fn group(
        &mut self,
        el: &XNode,
        part: &str,
        kind: PartKind,
        gm: GroupMap,
        parent: Option<ShapeIdx>,
    ) {
        let Some(gp) = pc(el, "grpSpPr") else { return };
        let xfrm = gp.child("a:xfrm");
        let xf = xfrm.and_then(|x| read_xfrm(x)).unwrap_or_default();
        let (ch_off, ch_ext) = match xfrm {
            Some(x) => (
                x.child("a:chOff")
                    .map(|o| {
                        (
                            o.attr_f64("x").unwrap_or(0.0),
                            o.attr_f64("y").unwrap_or(0.0),
                        )
                    })
                    .unwrap_or((xf.x, xf.y)),
                x.child("a:chExt")
                    .map(|e| {
                        (
                            e.attr_f64("cx").unwrap_or(0.0),
                            e.attr_f64("cy").unwrap_or(0.0),
                        )
                    })
                    .unwrap_or((xf.w, xf.h)),
            ),
            None => ((0.0, 0.0), (0.0, 0.0)),
        };
        let (x, y, w, h) = gm.map(xf.x, xf.y, xf.w, xf.h);
        // Child EMU → frame EMU; the frame is the group's mapped extent.
        let scale = (
            if ch_ext.0 > 0.0 {
                w * EMU_PER_PT / ch_ext.0
            } else {
                gm.scale.0
            },
            if ch_ext.1 > 0.0 {
                h * EMU_PER_PT / ch_ext.1
            } else {
                gm.scale.1
            },
        );
        // Children are placed in the group's frame, then rotated/flipped with it.
        let rot_tf = local_tf(x, y, w, h, xf.rot, xf.flip_h, xf.flip_v, false);
        let frame = Transform::from_translate(-x as f32, -y as f32)
            .post_concat(rot_tf)
            .post_concat(gm.tf);
        let child = GroupMap {
            tf: frame,
            off: (x * EMU_PER_PT, y * EMU_PER_PT),
            ch_off,
            scale,
        };
        let bounds = textlayout::transformed_bbox(&rot_tf.post_concat(gm.tf), 0.0, 0.0, w, h)
            .unwrap_or(Rect { x, y, w, h });
        let idx = self.register(el, "grpSp", bounds, xf.rot, parent, kind != PartKind::Slide);
        // The group's own fill spans its box and turns with it; `grpFill` passes
        // the enclosing group's fill on, and a group without a fill has none.
        let group_fill = match style::fill_child(gp) {
            Some(f) => {
                let local = Rect {
                    x: 0.0,
                    y: 0.0,
                    w,
                    h,
                };
                match style::read_fill(f, &self.style_ctx(part), None, local) {
                    Fill::Paint(p) => Some(transform_paint(p, &rot_tf.post_concat(gm.tf))),
                    Fill::Group => self.group_fills.last().cloned().flatten(),
                    Fill::None => None,
                }
            }
            None => None,
        };
        self.group_fills.push(group_fill);
        self.walk(el, part, kind, child, Some(idx));
        self.group_fills.pop();
    }

    /// Replaces `a:grpFill` with the enclosing group's fill, moved into the frame
    /// of the shape whose local-to-slide transform is `tf`.
    fn resolve_group_fill(&self, fill: Option<Fill>, tf: &Transform) -> Option<Fill> {
        match fill {
            Some(Fill::Group) => {
                let p = self.group_fills.last().cloned().flatten()?;
                let inv = tf.invert()?;
                Some(Fill::Paint(transform_paint(p, &inv)))
            }
            other => other,
        }
    }

    fn graphic_frame(
        &mut self,
        el: &XNode,
        part: &str,
        kind: PartKind,
        gm: GroupMap,
        parent: Option<ShapeIdx>,
    ) {
        let Some(xf) = pc(el, "xfrm").and_then(|x| read_xfrm(x)) else {
            return;
        };
        let (x, y, w, h) = gm.map(xf.x, xf.y, xf.w, xf.h);
        let tf = local_tf(x, y, w, h, xf.rot, xf.flip_h, xf.flip_v, false).post_concat(gm.tf);
        let bounds =
            textlayout::transformed_bbox(&tf, 0.0, 0.0, w, h).unwrap_or(Rect { x, y, w, h });
        let idx = self.register(
            el,
            "graphicFrame",
            bounds,
            xf.rot,
            parent,
            kind != PartKind::Slide,
        );
        let Some(data) = el.path(&["a:graphic", "a:graphicData"]).cloned() else {
            return;
        };
        let uri = data.attr("uri").unwrap_or("");
        if uri.ends_with("/table") {
            if let Some(tbl) = data.child("a:tbl") {
                crate::office::pptx::table::build_table(self, tbl, part, tf, w, h, idx);
            }
        } else if uri.ends_with("/diagram") {
            self.smart_art(&data, part, gm, Rect { x, y, w, h }, tf, idx);
        } else if uri.ends_with("/chart") {
            crate::office::pptx::chart::build_chart(
                self,
                &data,
                part,
                tf,
                Rect { x, y, w, h },
                bounds,
                idx,
            );
        } else if uri.ends_with("/chartex") {
            let frame = Rect { x, y, w, h };
            if !crate::office::pptx::chart::build_chartex(self, &data, part, tf, frame, bounds, idx)
            {
                // Region maps and unreadable parts: PowerPoint's own fallback picture.
                for pic in el
                    .child("x:fallback")
                    .into_iter()
                    .flat_map(|f| f.children.iter())
                {
                    if pic.name == "p:pic" {
                        self.picture(pic, part, kind, gm, Some(idx));
                    }
                }
            }
        } else if uri.ends_with("/ole") {
            // OLE objects carry a preview picture (p:oleObj/p:pic, or the mc:Fallback pic).
            self.in_equation = data
                .find("p:oleObj")
                .and_then(|o| o.attr("progId"))
                .is_some_and(is_equation_prog_id);
            let mut pics = Vec::new();
            data.find_all("p:pic", &mut pics);
            if let Some(pic) = pics.first() {
                let mut pic: XNode = (***pic).clone();
                // The preview spans the frame regardless of the pic's own xfrm.
                pic.children.retain(|c| c.local() != "spPr");
                let sp_pr = format!(
                    r#"<p:spPr xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:xfrm><a:off x="{}" y="{}"/><a:ext cx="{}" cy="{}"/></a:xfrm></p:spPr>"#,
                    xf.x, xf.y, xf.w, xf.h
                );
                if let Ok(node) = crate::office::pptx::xml::parse(&sp_pr) {
                    pic.children.push(node);
                    self.picture(&pic, part, kind, gm, Some(idx));
                }
            } else if let Some(image) = data
                .find("p:oleObj")
                .and_then(|o| o.attr("spid"))
                .and_then(|spid| self.vml_preview(part, spid))
            {
                // Legacy objects keep their preview in the slide's VML drawing.
                let unit =
                    Transform::from_row(w as f32, 0.0, 0.0, h as f32, 0.0, 0.0).post_concat(tf);
                let metafile_part = is_metafile_part(&image.part).then(|| image.part.clone());
                self.scene.items.push(Item::Image(ImageItem {
                    image,
                    transform: unit,
                    crop: (0.0, 0.0, 0.0, 0.0),
                    alpha: 1.0,
                    clip: None,
                    shape: Some(idx),
                    alt_text: None,
                }));
                if let Some(mp) = metafile_part {
                    self.metafile_text(&mp, unit, (0.0, 0.0, 0.0, 0.0), idx);
                }
            } else {
                self.scene.warnings.push((
                    "OLE_OBJECT".into(),
                    format!("no preview for {}", self.scene.shapes[idx].name),
                ));
            }
            self.in_equation = false;
        }
    }

    /// Preview image of a legacy OLE object: the VML shape whose id is the
    /// object's `spid`, through its `v:imagedata o:relid`.
    fn vml_preview(&self, part: &str, spid: &str) -> Option<crate::office::pptx::scene::ImageRef> {
        for rel in self
            .doc
            .pkg
            .rels(part)
            .iter()
            .filter(|r| r.kind() == "vmlDrawing")
        {
            let Some(vml) = self.doc.xml(&rel.target) else {
                continue;
            };
            let mut shapes = Vec::new();
            vml.find_all("v:shape", &mut shapes);
            let Some(shape) = shapes
                .into_iter()
                .find(|s| s.attr("id") == Some(spid) || s.attr("o:spid") == Some(spid))
            else {
                continue;
            };
            let rid = shape
                .child("v:imagedata")
                .and_then(|i| i.attr("o:relid").or(i.attr("r:id")))?;
            let target = self.doc.pkg.rel_by_id(&rel.target, rid)?;
            if !target.external {
                return Some(crate::office::pptx::scene::ImageRef {
                    part: target.target,
                    effects: Vec::new(),
                });
            }
        }
        None
    }

    fn smart_art(
        &mut self,
        data: &XNode,
        part: &str,
        gm: GroupMap,
        frame: Rect,
        tf: Transform,
        idx: ShapeIdx,
    ) {
        let (x, y) = (frame.x, frame.y);
        // The data part's extension names the slide relationship of the drawing.
        let dm_rid = data
            .child("dgm:relIds")
            .and_then(|r| r.attr("r:dm"))
            .map(str::to_string);
        let dm_part = dm_rid
            .and_then(|rid| self.doc.pkg.rel_by_id(part, &rid))
            .map(|r| r.target);
        let drawing_rid = dm_part
            .as_deref()
            .and_then(|p| self.doc.xml(p))
            .and_then(|dm| {
                dm.find("dsp:dataModelExt")
                    .and_then(|e| e.attr("relId").map(str::to_string))
            });
        let drawing_part = drawing_rid
            .and_then(|rid| self.doc.pkg.rel_by_id(part, &rid))
            .map(|r| r.target);
        // Text of the data model in tree order, for the presentation block and
        // for the fallback layout when no drawing was saved.
        let nodes = dm_part
            .as_deref()
            .and_then(|p| self.doc.xml(p))
            .map(|dm| smart_art_nodes(&dm))
            .unwrap_or_default();
        let b = self.scene.shapes[idx].bounds;
        let json_nodes: Vec<serde_json::Value> = nodes
            .iter()
            .map(|(level, text)| serde_json::json!({ "text": text, "level": level }))
            .collect();
        self.scene
            .smart_art
            .push(serde_json::json!({ "coords": rect_json(&b), "nodes": json_nodes }));
        let Some(dp) = drawing_part else {
            self.smart_art_fallback(data, part, dm_part.as_deref(), &nodes, gm, frame, tf, idx);
            return;
        };
        let tree = self
            .doc
            .xml(&dp)
            .and_then(|d| d.child("dsp:spTree").cloned());
        // A stale or empty drawing cache: fall back like a missing one.
        let has_shapes = tree
            .as_ref()
            .map(|t| {
                t.children
                    .iter()
                    .any(|c| matches!(c.local(), "sp" | "grpSp" | "pic"))
            })
            .unwrap_or(false);
        let Some(tree) = tree.filter(|_| has_shapes) else {
            self.smart_art_fallback(data, part, dm_part.as_deref(), &nodes, gm, frame, tf, idx);
            return;
        };
        // Drawing coordinates are relative to the frame's top-left.
        let child = GroupMap {
            tf: gm.tf,
            off: (x * EMU_PER_PT, y * EMU_PER_PT),
            ch_off: (0.0, 0.0),
            scale: (1.0, 1.0),
        };
        self.walk(&tree, &dp, PartKind::Slide, child, Some(idx));
    }

    /// SmartArt without a usable drawing: laid out from its data model and layout
    /// definition (`crate::office::pptx::smartart`), else listed as indented text.
    #[expect(
        clippy::too_many_arguments,
        reason = "takes the SmartArt parts plus the frame, transform and shape it replaces"
    )]
    fn smart_art_fallback(
        &mut self,
        data: &XNode,
        part: &str,
        dm_part: Option<&str>,
        nodes: &[(u32, String)],
        gm: GroupMap,
        frame: Rect,
        tf: Transform,
        idx: ShapeIdx,
    ) {
        let name = self.scene.shapes[idx].name.clone();
        let parts =
            dm_part.and_then(|dp| crate::office::pptx::smartart::parts(self.doc, data, part, dp));
        let laid_out = parts
            .as_ref()
            .map(|p| crate::office::pptx::smartart::layout(self, p, frame, idx));
        match (laid_out, parts) {
            (Some(Ok(tree)), Some(p)) => {
                self.scene.warnings.push(("SMARTART_LAID_OUT".into(), name));
                let child = GroupMap {
                    tf: gm.tf,
                    off: (frame.x * EMU_PER_PT, frame.y * EMU_PER_PT),
                    ch_off: (0.0, 0.0),
                    scale: (1.0, 1.0),
                };
                self.walk(&tree, &p.data_part, PartKind::Slide, child, Some(idx));
            }
            (res, _) => {
                let why = match res {
                    Some(Err(e)) => format!("{name}: {e}"),
                    _ => name,
                };
                self.scene
                    .warnings
                    .push(("SMARTART_NO_FALLBACK".into(), why));
                self.smart_art_list(nodes, part, frame, tf, idx);
            }
        }
    }
}

fn is_hidden(el: &XNode) -> bool {
    el.children
        .iter()
        .find(|c| c.local().starts_with("nv"))
        .and_then(|nv| pc(nv, "cNvPr"))
        .and_then(|c| c.attr_bool("hidden"))
        .unwrap_or(false)
}

/// Text of a `dgm:t`/`a:txBody`-like element: paragraphs joined by newlines.
pub fn para_texts(t: &XNode) -> String {
    t.children_named("a:p")
        .map(|p| p.all_text())
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn rect_json(r: &Rect) -> serde_json::Value {
    serde_json::json!({ "x": r.x, "y": r.y, "w": r.w, "h": r.h })
}

pub fn to_skia_path(cmds: &[Cmd]) -> Option<(tiny_skia::Path, bool)> {
    let mut pb = tiny_skia::PathBuilder::new();
    let mut curve = false;
    let mut open = false;
    for c in cmds {
        match *c {
            Cmd::Move(x, y) => {
                pb.move_to(x as f32, y as f32);
                open = true;
            }
            Cmd::Line(x, y) => {
                if !open {
                    pb.move_to(x as f32, y as f32);
                    open = true;
                } else {
                    pb.line_to(x as f32, y as f32);
                }
            }
            Cmd::Quad(a, b, x, y) => {
                if !open {
                    pb.move_to(a as f32, b as f32);
                    open = true;
                }
                pb.quad_to(a as f32, b as f32, x as f32, y as f32);
                curve = true;
            }
            Cmd::Cubic(a, b, c2, d, x, y) => {
                if !open {
                    pb.move_to(a as f32, b as f32);
                    open = true;
                }
                pb.cubic_to(a as f32, b as f32, c2 as f32, d as f32, x as f32, y as f32);
                curve = true;
            }
            Cmd::Close => pb.close(),
        }
    }
    pb.finish().map(|p| (p, curve))
}

/// Applies a path's `fill` shading mode (lighten/darken) to a paint.
fn shade_paint(p: &Paint, mode: FillMode) -> Paint {
    let f = |c: Rgba| -> Rgba {
        let k = match mode {
            FillMode::Lighten => 0.6,
            FillMode::LightenLess => 0.8,
            FillMode::Darken => -0.6,
            FillMode::DarkenLess => -0.8,
            _ => return c,
        };
        if k > 0.0 {
            // Lighten: blend toward white by (1 - k).
            let t = 1.0 - k as f32;
            Rgba {
                r: c.r + (1.0 - c.r) * t,
                g: c.g + (1.0 - c.g) * t,
                b: c.b + (1.0 - c.b) * t,
                a: c.a,
            }
        } else {
            let t = (-k) as f32;
            Rgba {
                r: c.r * t,
                g: c.g * t,
                b: c.b * t,
                a: c.a,
            }
        }
    };
    match p {
        Paint::Solid(c) => Paint::Solid(f(*c)),
        other => other.clone(),
    }
}

/// Moves a paint defined in shape-local space into slide space.
pub fn transform_paint(p: Paint, tf: &Transform) -> Paint {
    let map = |(x, y): (f64, f64)| {
        let mut pt = [tiny_skia::Point::from_xy(x as f32, y as f32)];
        tf.map_points(&mut pt);
        (pt[0].x as f64, pt[0].y as f64)
    };
    match p {
        Paint::Linear { start, end, stops } => Paint::Linear {
            start: map(start),
            end: map(end),
            stops,
        },
        Paint::Radial {
            center,
            radius,
            stops,
        } => {
            let scale = ((tf.sx * tf.sx + tf.ky * tf.ky) as f64).sqrt().max(1e-6);
            Paint::Radial {
                center: map(center),
                radius: radius * scale,
                stops,
            }
        }
        Paint::Path(g) => Paint::Path(Box::new((*g).transformed(tf))),
        Paint::Image {
            image,
            rect,
            tile,
            alpha,
            frame,
            rot_with_shape,
        } => {
            let full = frame.post_concat(*tf);
            if rot_with_shape {
                return Paint::Image {
                    image,
                    rect,
                    tile,
                    alpha,
                    frame: full,
                    rot_with_shape,
                };
            }
            // rotWithShape="0": the picture stays upright over the turned box's bounds.
            let r =
                textlayout::transformed_bbox(&full, rect.x, rect.y, rect.w, rect.h).unwrap_or(rect);
            Paint::Image {
                image,
                rect: r,
                tile,
                alpha,
                frame: Transform::identity(),
                rot_with_shape,
            }
        }
        other => other,
    }
}

#[cfg(test)]
mod tests;

/// SmartArt data-model text as (depth, text) in document order: the `parOf`
/// connections form a tree under the `doc` point, ordered by `srcOrd`.
fn smart_art_nodes(dm: &XNode) -> Vec<(u32, String)> {
    use std::collections::HashMap;
    let mut text: HashMap<String, String> = HashMap::new();
    let mut root = None;
    let mut pts = Vec::new();
    dm.find_all("dgm:pt", &mut pts);
    for pt in pts {
        let id = pt.attr("modelId").unwrap_or("").to_string();
        match pt.attr("type").unwrap_or("node") {
            "doc" => root = Some(id),
            "node" | "asst" => {
                let t = pt.child("dgm:t").map(|t| para_texts(t)).unwrap_or_default();
                text.insert(id, t);
            }
            _ => {}
        }
    }
    let mut children: HashMap<String, Vec<(i64, String)>> = HashMap::new();
    let mut cxns = Vec::new();
    dm.find_all("dgm:cxn", &mut cxns);
    for c in cxns {
        if c.attr("type").unwrap_or("parOf") != "parOf" {
            continue;
        }
        let (Some(src), Some(dst)) = (c.attr("srcId"), c.attr("destId")) else {
            continue;
        };
        children
            .entry(src.to_string())
            .or_default()
            .push((c.attr_i64("srcOrd").unwrap_or(0), dst.to_string()));
    }
    for v in children.values_mut() {
        v.sort();
    }
    let mut out = Vec::new();
    let mut stack: Vec<(String, u32)> = Vec::new();
    if let Some(r) = root {
        for (_, c) in children.get(&r).into_iter().flatten().rev() {
            stack.push((c.clone(), 0));
        }
    }
    let mut seen = std::collections::HashSet::new();
    while let Some((id, depth)) = stack.pop() {
        if !seen.insert(id.clone()) || seen.len() > 10_000 {
            continue;
        }
        if let Some(t) = text.get(&id).filter(|t| !t.trim().is_empty()) {
            out.push((depth, t.clone()));
        }
        for (_, c) in children.get(&id).into_iter().flatten().rev() {
            stack.push((c.clone(), depth + 1));
        }
    }
    out
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Places a metafile text run over its slide-space box `(x, y, w, h)` at
/// `angle` (degrees clockwise): a near-vertical run runs along the box height.
/// The run spans ascent to descent, baseline at 80 %, centred on the box.
/// Text runs drawn inside a metafile picture, placed over the picture and marked not
/// drawn (the pixels are in the picture). `unit` maps the visible picture's unit
/// square to page space; `crop` is its srcRect.
pub fn metafile_text_runs(
    images: &crate::office::pptx::render::ImageCache,
    doc: &dyn crate::office::pptx::render::RenderAssets,
    fonts: &FontSystem,
    part: &str,
    unit: Transform,
    crop: (f64, f64, f64, f64),
    shape: Option<ShapeIdx>,
) -> Vec<crate::office::pptx::scene::GlyphRun> {
    let mut out = Vec::new();
    let runs = images.metafile_text(doc, part);
    let (l, t, r, b) = crop;
    let (vw, vh) = ((1.0 - l - r).max(1e-6), (1.0 - t - b).max(1e-6));
    let map = |x: f64, y: f64| {
        let mut p = [tiny_skia::Point::from_xy(
            ((x - l) / vw) as f32,
            ((y - t) / vh) as f32,
        )];
        unit.map_points(&mut p);
        (p[0].x as f64, p[0].y as f64)
    };
    // The picture's own rotation and height on the slide: the mapped unit axes.
    let (x0, y0) = map(0.0, 0.0);
    let (x1, y1) = map(0.0, 1.0);
    let (xr, yr) = map(1.0, 0.0);
    let frame_h = (x1 - x0).hypot(y1 - y0);
    let pic_angle = (yr - y0).atan2(xr - x0).to_degrees();
    for m in runs.iter() {
        if m.text.trim().is_empty()
            || m.x + m.w < l
            || m.x > 1.0 - r
            || m.y + m.h < t
            || m.y > 1.0 - b
        {
            continue;
        }
        let (ax, ay) = map(m.x, m.y);
        let (bx, by) = map(m.x + m.w, m.y + m.h);
        let (x, y, w, h) = (ax.min(bx), ay.min(by), (bx - ax).abs(), (by - ay).abs());
        if w <= 0.0 || h <= 0.0 {
            continue;
        }
        let Some(face) = fonts
            .resolve(&m.font_family, m.bold, m.italic)
            .or_else(|| fonts.resolve("Carlito", m.bold, m.italic))
        else {
            continue;
        };
        let (transform, len, ascent, descent) =
            metafile_run_placement((x, y, w, h), m.angle + pic_angle);
        out.push(crate::office::pptx::scene::GlyphRun {
            bookmarks: None,
            source: None,
            preserve_whitespace: false,
            face,
            font_name: m.font_family.clone(),
            size: (m.size * frame_h).max(0.1),
            color: m.color,
            glyphs: Vec::new(),
            transform,
            text: m.text.clone(),
            width: len,
            ascent,
            descent,
            outline: None,
            fake_bold: false,
            fake_italic: false,
            shape,
            paragraph: 0,
            hyperlink: None,
            space_before: false,
            drawn: false,
            warp: None,
        });
    }
    out
}

/// Returns the run transform, its length, ascent and descent (points).
fn metafile_run_placement(
    (x, y, w, h): (f64, f64, f64, f64),
    angle: f64,
) -> (Transform, f64, f64, f64) {
    let vertical = angle.to_radians().sin().abs() > std::f64::consts::FRAC_1_SQRT_2;
    let (len, thick) = if vertical { (h, w) } else { (w, h) };
    let transform = Transform::from_translate(-(len / 2.0) as f32, (0.3 * thick) as f32)
        .post_rotate(angle as f32)
        .post_translate((x + w / 2.0) as f32, (y + h / 2.0) as f32);
    (transform, len, thick * 0.8, thick * 0.2)
}

fn is_metafile_part(part: &str) -> bool {
    let lower = part.to_ascii_lowercase();
    lower.ends_with(".emf") || lower.ends_with(".wmf")
}
