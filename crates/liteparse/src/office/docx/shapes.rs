//! DrawingML shapes in Word documents (`wps:wsp`, groups `wpg:wgp`): their
//! geometry, fill and outline become scene items at parse time, through
//! `pptx-read`'s preset geometry and fill/line readers; text box contents are
//! kept as blocks and laid out with the paragraph that anchors the shape.

use std::rc::Rc;

use crate::office::pptx::geom;
use crate::office::pptx::scene::{ImageItem, Item, PathItem, Rect};
use crate::office::pptx::style::{Fill, StyleCtx, fill_child, read_fill, read_line};
use crate::office::pptx::xml::XNode;
use tiny_skia::Transform;

use crate::office::docx::model::Block;
use crate::office::docx::parse::EMU_PER_PT;

/// A text box inside a shape: its rectangle in shape-local points, insets
/// (l, t, r, b), vertical anchoring and contents.
#[derive(Debug, Clone)]
pub struct TextBox {
    pub rect: Rect,
    pub inset: [f64; 4],
    pub v_anchor: String,
    pub blocks: Vec<Block>,
    /// The shape's fill behind the text (mean colour of a gradient).
    pub bg: Option<crate::office::pptx::color::Rgba>,
    /// Colour of automatic text: the shape style's `a:fontRef` colour.
    pub font_color: Option<crate::office::pptx::color::Rgba>,
    /// Rotation and flips of the shape and its groups, in shape-local points.
    pub transform: Option<Transform>,
    /// DrawingML text stays upright after its complete shape/group placement.
    pub upright: bool,
    /// WordArt: the text is stretched to fill the box.
    pub fit: bool,
    /// Lines that do not fit in the box are hidden (shapes that do not grow with their text).
    pub clip: bool,
    /// `spAutoFit` (VML `mso-fit-shape-to-text`): the shape takes its text's height.
    pub grow: bool,
    /// `wrap="none"` (VML `mso-wrap-style:none`): lines break only at paragraph ends.
    pub nowrap: bool,
    /// Vertical text (`vert`, `vert270`, `eaVert`...): lines run down the box.
    pub vert: Option<String>,
}

impl TextBox {
    /// Resolve upright text after all ancestor transforms have been composed.
    /// Its center follows the shape; quarter-turn sectors exchange the frame
    /// dimensions while the source writing direction remains independent.
    pub(crate) fn upright_frame(&self) -> Self {
        let mut out = self.clone();
        let Some(t) = self.transform.filter(|_| self.upright) else {
            return out;
        };
        let mut center = tiny_skia::Point::from_xy(
            (self.rect.x + self.rect.w / 2.0) as f32,
            (self.rect.y + self.rect.h / 2.0) as f32,
        );
        t.map_point(&mut center);
        let angle = (t.ky as f64)
            .atan2(t.sx as f64)
            .to_degrees()
            .rem_euclid(180.0);
        let vertical = (45.0..135.0).contains(&angle);
        if vertical {
            std::mem::swap(&mut out.rect.w, &mut out.rect.h);
        }
        out.rect.x = center.x as f64 - out.rect.w / 2.0;
        out.rect.y = center.y as f64 - out.rect.h / 2.0;
        out.transform = None;
        out
    }
}

/// A drawn shape or group, in local points with its box at (0, 0).
#[derive(Debug, Clone, Default)]
pub struct ShapeSpec {
    pub items: Vec<Item>,
    pub texts: Vec<TextBox>,
    pub graphics: Vec<PendingGraphic>,
    /// A single source raster picture, possibly accompanied by its outline.
    pub raster_picture: bool,
    /// A directly rendered ordinary chart using its source anchor extent.
    pub direct_chart: bool,
}

impl ShapeSpec {
    /// Rebase local paint and deferred payloads into an inline layout rectangle.
    pub(crate) fn translate(&mut self, x: f64, y: f64) {
        let placement = Transform::from_translate(x as f32, y as f32);
        for item in &mut self.items {
            crate::office::docx::layout::transform_item(item, placement);
        }
        for text in &mut self.texts {
            text.transform = Some(
                text.transform
                    .map_or(placement, |t| t.post_concat(placement)),
            );
        }
        for graphic in &mut self.graphics {
            graphic.transform = graphic.transform.post_concat(placement);
        }
    }
}

/// Graphics await font initialization; insertion keeps their order among shape items.
#[derive(Debug, Clone)]
pub struct PendingGraphic {
    pub data: Rc<XNode>,
    pub part: String,
    pub w: f64,
    pub h: f64,
    pub transform: Transform,
    pub insertion: usize,
}

fn element_transform(node: &XNode) -> Option<&Rc<XNode>> {
    node.children
        .iter()
        .find(|c| matches!(c.local(), "spPr" | "grpSpPr"))
        .and_then(|s| s.child("a:xfrm"))
        .or_else(|| {
            (node.local() == "graphicFrame")
                .then(|| node.children.iter().find(|c| c.local() == "xfrm"))
                .flatten()
        })
}

/// Maps a child's EMU coordinates into the parent's points.
#[derive(Clone, Copy)]
struct Frame {
    ox: f64,
    oy: f64,
    sx: f64,
    sy: f64,
}

impl Frame {
    fn rect(&self, x: f64, y: f64, w: f64, h: f64) -> Rect {
        Rect {
            x: self.ox + x * self.sx,
            y: self.oy + y * self.sy,
            w: w * self.sx,
            h: h * self.sy,
        }
    }
}

/// Builds a shape (`wps:wsp`), group (`wpg:wgp`) or picture (`pic:pic`)
/// of `w`×`h` points. `text_of` turns a `w:txbxContent` into blocks.
pub fn build(
    node: &XNode,
    w: f64,
    h: f64,
    ctx: &StyleCtx,
    text_of: &dyn Fn(&XNode) -> Vec<Block>,
) -> ShapeSpec {
    let mut out = ShapeSpec::default();
    let frame = Frame {
        ox: 0.0,
        oy: 0.0,
        sx: 1.0,
        sy: 1.0,
    };
    let bounds = Rect {
        x: 0.0,
        y: 0.0,
        w,
        h,
    };
    element(node, bounds, frame, ctx, text_of, &mut out, 0);
    out
}

fn element(
    node: &XNode,
    bounds: Rect,
    parent: Frame,
    ctx: &StyleCtx,
    text_of: &dyn Fn(&XNode) -> Vec<Block>,
    out: &mut ShapeSpec,
    depth: usize,
) {
    if depth > 16 {
        return;
    }
    let (items0, texts0, graphics0) = (out.items.len(), out.texts.len(), out.graphics.len());
    element_inner(node, bounds, parent, ctx, text_of, out, depth);
    // Rotation (`rot`, 60000ths of a degree) and flips turn the element about its centre.
    let xfrm = element_transform(node);
    let Some(x) = xfrm else {
        return;
    };
    let rot = x.attr_f64("rot").unwrap_or(0.0) / 60000.0;
    let flip = |k: &str| x.attr_bool(k).unwrap_or(false);
    let (fh, fv) = (flip("flipH"), flip("flipV"));
    if rot.rem_euclid(360.0) == 0.0 && !fh && !fv {
        return;
    }
    let (cx, cy) = (
        (bounds.x + bounds.w / 2.0) as f32,
        (bounds.y + bounds.h / 2.0) as f32,
    );
    let t = Transform::from_translate(cx, cy)
        .pre_rotate(rot as f32)
        .pre_scale(if fh { -1.0 } else { 1.0 }, if fv { -1.0 } else { 1.0 })
        .pre_translate(-cx, -cy);
    for it in out.items[items0..].iter_mut() {
        crate::office::docx::layout::transform_item(it, t);
    }
    for graphic in &mut out.graphics[graphics0..] {
        graphic.transform = graphic.transform.post_concat(t);
    }
    // Text keeps reading left to right under a flip: a vertical flip turns
    // it a half turn (flipH leaves it as is), so rot 180 with flipV reads upright.
    let text_rot = rot + if fv { 180.0 } else { 0.0 };
    for tb in out.texts[texts0..].iter_mut() {
        let tt = Transform::from_translate(cx, cy)
            .pre_rotate(text_rot as f32)
            .pre_translate(-cx, -cy);
        let placement = if tb.upright { t } else { tt };
        tb.transform = Some(tb.transform.map_or(placement, |o| o.post_concat(placement)));
    }
}

fn element_inner(
    node: &XNode,
    bounds: Rect,
    parent: Frame,
    ctx: &StyleCtx,
    text_of: &dyn Fn(&XNode) -> Vec<Block>,
    out: &mut ShapeSpec,
    depth: usize,
) {
    match node.local() {
        "wsp" => shape(node, bounds, ctx, text_of, out),
        // A drawing canvas places its shapes like a group in EMU from its corner.
        "wgp" | "grpSp" | "wpc" => {
            let gpr = node
                .children
                .iter()
                .find(|c| c.local() == "grpSpPr")
                .and_then(|g| g.child("a:xfrm"));
            // Children are placed in the group's child coordinate space.
            let (cox, coy, cw, ch) = gpr
                .map(|x| {
                    let p =
                        |n: &str, a: &str| x.child(n).and_then(|c| c.attr_f64(a)).unwrap_or(0.0);
                    (
                        p("a:chOff", "x"),
                        p("a:chOff", "y"),
                        p("a:chExt", "cx"),
                        p("a:chExt", "cy"),
                    )
                })
                .unwrap_or((0.0, 0.0, 0.0, 0.0));
            let sx = if cw > 0.0 {
                bounds.w / cw
            } else {
                1.0 / EMU_PER_PT
            };
            let sy = if ch > 0.0 {
                bounds.h / ch
            } else {
                1.0 / EMU_PER_PT
            };
            let frame = Frame {
                ox: bounds.x - cox * sx,
                oy: bounds.y - coy * sy,
                sx,
                sy,
            };
            let _ = parent;
            for c in &node.children {
                let Some(r) = child_rect(c, frame) else {
                    continue;
                };
                element(c, r, frame, ctx, text_of, out, depth + 1);
            }
        }
        "graphicFrame" => {
            if let Some(data) = node
                .child("a:graphic")
                .and_then(|g| g.child("a:graphicData"))
            {
                let uri = data.attr("uri").unwrap_or("");
                if uri.ends_with("/chart") || uri.ends_with("/chartex") || uri.ends_with("/diagram")
                {
                    out.graphics.push(PendingGraphic {
                        data: data.clone(),
                        part: ctx.part.to_string(),
                        w: bounds.w,
                        h: bounds.h,
                        transform: Transform::from_translate(bounds.x as f32, bounds.y as f32),
                        insertion: out.items.len(),
                    });
                }
            }
        }
        "pic" => picture(node, bounds, ctx, out),
        _ => {}
    }
}

/// A group child's rectangle from its own `a:xfrm`.
fn child_rect(c: &XNode, frame: Frame) -> Option<Rect> {
    let x = element_transform(c)?;
    let off = x.child("a:off");
    let ext = x.child("a:ext");
    let v = |n: Option<&Rc<XNode>>, a: &str| n.and_then(|n| n.attr_f64(a)).unwrap_or(0.0);
    Some(frame.rect(v(off, "x"), v(off, "y"), v(ext, "cx"), v(ext, "cy")))
}

fn shape(
    node: &XNode,
    b: Rect,
    ctx: &StyleCtx,
    text_of: &dyn Fn(&XNode) -> Vec<Block>,
    out: &mut ShapeSpec,
) {
    let sp_pr = node.children.iter().find(|c| c.local() == "spPr");
    let style = node.children.iter().find(|c| c.local() == "style");
    let geom_node = sp_pr.and_then(|s| {
        s.children
            .iter()
            .find(|c| matches!(c.name.as_str(), "a:prstGeom" | "a:custGeom"))
    });
    let g = geom::build(
        geom_node.map(|n| n.as_ref()),
        b.w * EMU_PER_PT,
        b.h * EMU_PER_PT,
    );
    let local = Rect {
        x: 0.0,
        y: 0.0,
        w: b.w,
        h: b.h,
    };
    // Own fill, else the theme fill the style references.
    let fill = match sp_pr.and_then(|s| fill_child(s)) {
        Some(f) => read_fill(f, ctx, None, local),
        None => style
            .and_then(|s| s.child("a:fillRef"))
            .and_then(|r| {
                let idx = r.attr_i64("idx")?;
                let ph = ctx.color(r, None);
                let f = ctx.theme.fill_style(idx)?;
                Some(read_fill(f, ctx, ph, local))
            })
            .unwrap_or(Fill::None),
    };
    let line = read_line(
        sp_pr.and_then(|s| s.child("a:ln")).map(|n| n.as_ref()),
        style.and_then(|s| s.child("a:lnRef")).map(|n| n.as_ref()),
        ctx,
        local,
    );
    let to_page = Transform::from_translate(b.x as f32, b.y as f32);
    for gp in &g.paths {
        let Some((path, curve)) = crate::office::pptx::slide::to_skia_path(&gp.cmds) else {
            continue;
        };
        let Some(path) = path.transform(to_page) else {
            continue;
        };
        let paint = match (&fill, gp.fill) {
            (Fill::Paint(p), m) if m != geom::FillMode::None => Some(p.clone()),
            _ => None,
        };
        let stroke = line.clone().filter(|_| gp.stroke);
        if paint.is_none() && stroke.is_none() {
            continue;
        }
        out.items.push(Item::Path(PathItem {
            path,
            fill: paint,
            stroke,
            has_curve: curve,
            shape: None,
            from_text: false,
        }));
    }
    let body = node.children.iter().find(|c| c.local() == "bodyPr");
    if let Some(content) = node.find("w:txbxContent") {
        let ins = |k: &str, d: f64| {
            body.and_then(|b| b.attr_f64(k))
                .map(|v| v / EMU_PER_PT)
                .unwrap_or(d)
        };
        let blocks = text_of(content);
        let bg = match &fill {
            Fill::Paint(crate::office::pptx::scene::Paint::Solid(c)) => Some(*c),
            Fill::Paint(crate::office::pptx::scene::Paint::Linear { stops, .. })
            | Fill::Paint(crate::office::pptx::scene::Paint::Radial { stops, .. })
                if !stops.is_empty() =>
            {
                let n = stops.len() as f32;
                let mean = |f: fn(&crate::office::pptx::color::Rgba) -> f32| {
                    stops.iter().map(|s| f(&s.color)).sum::<f32>() / n
                };
                Some(crate::office::pptx::color::Rgba {
                    r: mean(|c| c.r),
                    g: mean(|c| c.g),
                    b: mean(|c| c.b),
                    a: mean(|c| c.a),
                })
            }
            _ => None,
        };
        let font_color = style
            .and_then(|s| s.child("a:fontRef"))
            .and_then(|r| ctx.color(r, None));
        if !blocks.is_empty() {
            out.texts.push(TextBox {
                rect: b,
                inset: [
                    ins("lIns", 7.2),
                    ins("tIns", 3.6),
                    ins("rIns", 7.2),
                    ins("bIns", 3.6),
                ],
                v_anchor: body
                    .and_then(|b| b.attr("anchor"))
                    .unwrap_or("t")
                    .to_string(),
                blocks,
                bg,
                font_color,
                transform: None,
                upright: body.and_then(|b| b.attr_bool("upright")).unwrap_or(false),
                fit: false,
                clip: body.is_some_and(|b| b.child("a:spAutoFit").is_none()),
                grow: body.is_some_and(|b| b.child("a:spAutoFit").is_some()),
                nowrap: body.and_then(|b| b.attr("wrap")) == Some("none"),
                vert: body
                    .and_then(|b| b.attr("vert"))
                    .filter(|v| *v != "horz")
                    .map(str::to_string),
            });
        }
    }
}

/// Outline paths of preset `prst` in a `w`×`h` box (points): (path, filled,
/// stroked, has curves).
pub fn preset_paths(prst: &str, w: f64, h: f64) -> Vec<(tiny_skia::Path, bool, bool, bool)> {
    let node = XNode {
        name: "a:prstGeom".into(),
        attrs: vec![("prst".into(), prst.into())],
        children: Vec::new(),
        text: String::new(),
    };
    let g = geom::build(Some(&node), w * EMU_PER_PT, h * EMU_PER_PT);
    g.paths
        .iter()
        .filter_map(|gp| {
            let (path, curve) = crate::office::pptx::slide::to_skia_path(&gp.cmds)?;
            Some((path, gp.fill != geom::FillMode::None, gp.stroke, curve))
        })
        .collect()
}

/// A picture's outline (`pic:spPr/a:ln`) around its `b` rectangle; `None`
/// without a visible line.
pub fn picture_outline(pic: &XNode, b: Rect, ctx: &StyleCtx) -> Option<Item> {
    let ln = pic
        .children
        .iter()
        .find(|c| c.local() == "spPr")
        .and_then(|s| s.child("a:ln"))?;
    let local = Rect {
        x: 0.0,
        y: 0.0,
        w: b.w,
        h: b.h,
    };
    let stroke = read_line(Some(ln.as_ref()), None, ctx, local)?;
    let path = crate::office::pptx::textlayout::rect_path(b.x, b.y, b.w, b.h)?;
    Some(Item::Path(PathItem {
        path,
        fill: None,
        stroke: Some(stroke),
        has_curve: false,
        shape: None,
        from_text: false,
    }))
}

fn picture(node: &XNode, b: Rect, ctx: &StyleCtx, out: &mut ShapeSpec) {
    let Some(blip) = node.find("a:blip") else {
        return;
    };
    if let (Some(shadow), Some(path)) = (
        None,
        crate::office::pptx::textlayout::rect_path(b.x, b.y, b.w, b.h),
    ) {
        out.items
            .push(Item::Shadow(crate::office::pptx::scene::ShadowItem {
                source: crate::office::pptx::scene::ShadowSource::Path(path),
                shadow,
            }));
    }
    let Some(image) = ctx.blip_image(blip, None) else {
        return;
    };
    out.items.push(Item::Image(ImageItem {
        image,
        transform: Transform::from_row(b.w as f32, 0.0, 0.0, b.h as f32, b.x as f32, b.y as f32),
        crop: (0.0, 0.0, 0.0, 0.0),
        alpha: 1.0,
        clip: None,
        shape: None,
        alt_text: None,
    }));
    out.items.extend(picture_outline(node, b, ctx));
}

#[cfg(test)]
mod tests;
