//! Fill and outline resolution: turns `spPr` fill/`a:ln` elements, merged with the
//! theme style matrix (`p:style` `fillRef`/`lnRef`), into scene paints and strokes.

use std::rc::Rc;

use crate::office::pptx::color::{self, ColorScheme, Rgba};
use crate::office::pptx::scene::{
    Arrow, ArrowKind, BlipEffect, GradientStop, ImageRef, LineCap, LineJoin, Paint, Rect, Shadow,
    Stroke, Tile,
};
use crate::office::pptx::theme::Theme;
use crate::office::pptx::xml::XNode;

pub const EMU_PER_PT: f64 = 12_700.0;

/// What a part needs to resolve colors and images.
pub struct StyleCtx<'a> {
    pub theme: &'a Theme,
    pub colors: &'a ColorScheme,
    /// Part whose relationships resolve `r:embed` ids in the fills being read.
    pub part: &'a str,
    pub theme_part: &'a str,
    pub pkg: &'a crate::office::pptx::package::Package,
}

impl<'a> StyleCtx<'a> {
    /// The same context resolving relationships against the theme part, for
    /// fills and lines taken from the theme's style lists.
    pub fn for_theme(&self) -> StyleCtx<'a> {
        StyleCtx {
            theme: self.theme,
            colors: self.colors,
            part: self.theme_part,
            theme_part: self.theme_part,
            pkg: self.pkg,
        }
    }

    /// The same context resolving relationships against another part.
    pub fn for_part<'b>(&self, part: &'b str) -> StyleCtx<'b>
    where
        'a: 'b,
    {
        StyleCtx {
            theme: self.theme,
            colors: self.colors,
            part,
            theme_part: self.theme_part,
            pkg: self.pkg,
        }
    }

    pub fn color(&self, node: &XNode, ph: Option<Rgba>) -> Option<Rgba> {
        color::resolve(node, self.colors, ph)
    }

    pub fn image_ref(&self, rid: &str) -> Option<ImageRef> {
        let rel = self.pkg.rel_by_id(self.part, rid)?;
        if rel.external {
            return None;
        }
        Some(ImageRef {
            part: rel.target,
            effects: Vec::new(),
        })
    }

    /// The image of an `a:blip` with its recolor effects; `ph` resolves `phClr`.
    pub fn blip_image(&self, blip: &XNode, ph: Option<Rgba>) -> Option<ImageRef> {
        let rid = blip
            .attr("r:embed")
            .or_else(|| blip.find("asvg:svgBlip").and_then(|s| s.attr("r:embed")))?;
        let mut image = self.image_ref(rid)?;
        image.effects = blip_effects(blip, self, ph);
        Some(image)
    }
}

/// Result of reading a fill element.
#[derive(Debug, Clone)]
pub enum Fill {
    /// `a:noFill`.
    None,
    Paint(Paint),
    /// `a:grpFill`: use the enclosing group's fill.
    Group,
}

pub fn is_fill_element(name: &str) -> bool {
    matches!(
        name,
        "a:noFill" | "a:solidFill" | "a:gradFill" | "a:blipFill" | "a:pattFill" | "a:grpFill"
    )
}

/// The first fill child of `parent` (e.g. `p:spPr`, `a:ln`, `a:tcPr`).
pub fn fill_child(parent: &XNode) -> Option<&Rc<XNode>> {
    parent.children.iter().find(|c| is_fill_element(&c.name))
}

/// Reads a fill element. `bounds` is the filled area in the local (unrotated)
/// frame used to place gradients and picture fills.
pub fn read_fill(node: &XNode, ctx: &StyleCtx, ph: Option<Rgba>, bounds: Rect) -> Fill {
    match node.name.as_str() {
        "a:noFill" => Fill::None,
        "a:grpFill" => Fill::Group,
        "a:solidFill" => match ctx.color(node, ph) {
            Some(c) => Fill::Paint(Paint::Solid(c)),
            None => Fill::None,
        },
        "a:gradFill" => read_gradient(node, ctx, ph, bounds)
            .map(Fill::Paint)
            .unwrap_or(Fill::None),
        "a:pattFill" => {
            let fg = node
                .child("a:fgClr")
                .and_then(|c| ctx.color(c, ph))
                .unwrap_or(Rgba::BLACK);
            let bg = node
                .child("a:bgClr")
                .and_then(|c| ctx.color(c, ph))
                .unwrap_or(Rgba::WHITE);
            let preset = node.attr("prst").unwrap_or("pct5").to_string();
            Fill::Paint(Paint::Pattern { preset, fg, bg })
        }
        "a:blipFill" => {
            let Some(blip) = node.child("a:blip") else {
                return Fill::None;
            };
            let Some(image) = ctx.blip_image(blip, ph) else {
                return Fill::None;
            };
            let alpha = blip_alpha(blip);
            let tile = node.child("a:tile").map(|t| {
                let pct = |k: &str| {
                    t.attr(k)
                        .and_then(crate::office::pptx::xml::parse_i64)
                        .unwrap_or(100_000) as f64
                        / 100_000.0
                };
                let off = |k: &str| t.attr_f64(k).unwrap_or(0.0) / EMU_PER_PT;
                Tile {
                    sx: pct("sx"),
                    sy: pct("sy"),
                    tx: off("tx"),
                    ty: off("ty"),
                }
            });
            // stretch/fillRect insets (negative = beyond the shape) place the
            // visible picture; srcRect crops it, so the full image maps wider still.
            let frac = |n: Option<&Rc<XNode>>, k: &str| {
                n.and_then(|n| n.attr(k))
                    .and_then(crate::office::pptx::xml::parse_i64)
                    .unwrap_or(0) as f64
                    / 100_000.0
            };
            let fr = node.path(&["a:stretch", "a:fillRect"]);
            let mut rect = Rect {
                x: bounds.x + bounds.w * frac(fr, "l"),
                y: bounds.y + bounds.h * frac(fr, "t"),
                w: bounds.w * (1.0 - frac(fr, "l") - frac(fr, "r")),
                h: bounds.h * (1.0 - frac(fr, "t") - frac(fr, "b")),
            };
            let src = node.child("a:srcRect");
            let (sl, st, sr, sb) = (
                frac(src, "l"),
                frac(src, "t"),
                frac(src, "r"),
                frac(src, "b"),
            );
            let (vw, vh) = ((1.0 - sl - sr).max(1e-6), (1.0 - st - sb).max(1e-6));
            if tile.is_none() && (sl, st, sr, sb) != (0.0, 0.0, 0.0, 0.0) {
                rect = Rect {
                    x: rect.x - rect.w * sl / vw,
                    y: rect.y - rect.h * st / vh,
                    w: rect.w / vw,
                    h: rect.h / vh,
                };
            }
            // An absent rotWithShape turns and flips the picture with its shape.
            Fill::Paint(Paint::Image {
                image,
                rect,
                tile,
                alpha,
                frame: tiny_skia::Transform::identity(),
                rot_with_shape: node.attr_bool("rotWithShape").unwrap_or(true),
            })
        }
        _ => Fill::None,
    }
}

pub fn blip_effects(blip: &XNode, ctx: &StyleCtx, ph: Option<Rgba>) -> Vec<BlipEffect> {
    let pct = |n: &XNode, k: &str| {
        n.attr(k)
            .and_then(crate::office::pptx::xml::parse_i64)
            .unwrap_or(0) as f32
            / 100_000.0
    };
    let mut out = Vec::new();
    for e in &blip.children {
        match e.name.as_str() {
            "a:duotone" => {
                let colors: Vec<Rgba> =
                    e.children.iter().filter_map(|c| ctx.color(c, ph)).collect();
                if colors.len() >= 2 {
                    out.push(BlipEffect::Duotone(colors[0], colors[1]));
                }
            }
            "a:grayscl" => out.push(BlipEffect::Grayscale),
            "a:lum" => out.push(BlipEffect::Lum {
                bright: pct(e, "bright"),
                contrast: pct(e, "contrast"),
            }),
            "a:biLevel" => out.push(BlipEffect::BiLevel(pct(e, "thresh"))),
            "a:clrChange" => {
                let from = e.child("a:clrFrom").and_then(|c| ctx.color(c, ph));
                let to = e.child("a:clrTo").and_then(|c| ctx.color(c, ph));
                if let (Some(from), Some(to)) = (from, to) {
                    out.push(BlipEffect::ClrChange { from, to });
                }
            }
            _ => {}
        }
    }
    out
}

pub fn blip_alpha(blip: &XNode) -> f32 {
    blip.child("a:alphaModFix")
        .and_then(|a| a.attr_i64("amt"))
        .map(|v| (v as f32 / 100_000.0).clamp(0.0, 1.0))
        .unwrap_or(1.0)
}

fn read_gradient(node: &XNode, ctx: &StyleCtx, ph: Option<Rgba>, b: Rect) -> Option<Paint> {
    let mut stops: Vec<GradientStop> = node
        .child("a:gsLst")?
        .children_named("a:gs")
        .filter_map(|gs| {
            let pos = gs.attr_i64("pos").unwrap_or(0) as f32 / 100_000.0;
            Some(GradientStop {
                pos: pos.clamp(0.0, 1.0),
                color: ctx.color(gs, ph)?,
            })
        })
        .collect();
    if stops.is_empty() {
        return None;
    }
    stops.sort_by(|a, b| a.pos.total_cmp(&b.pos));
    if let Some(path) = node.child("a:path") {
        let g = crate::office::pptx::pathgrad::from_xml(node, path, b, stops);
        return Some(Paint::Path(Box::new(g)));
    }
    let (cx, cy) = (b.x + b.w / 2.0, b.y + b.h / 2.0);
    let ang = node
        .child("a:lin")
        .and_then(|l| l.attr_i64("ang"))
        .map(|a| a as f64 / 60_000.0)
        .unwrap_or(0.0);
    let scaled = node
        .child("a:lin")
        .and_then(|l| l.attr_bool("scaled"))
        .unwrap_or(false);
    let rad = ang.to_radians();
    let (mut dx, mut dy) = (rad.cos(), rad.sin());
    if scaled && b.w > 0.0 && b.h > 0.0 {
        // A scaled angle is defined on the unit square and stretched with the shape.
        let (sx, sy) = (dx * b.w, dy * b.h);
        let n = (sx * sx + sy * sy).sqrt().max(1e-9);
        dx = sx / n;
        dy = sy / n;
    }
    // Project the box corners on the direction to get the covering segment.
    let half = (b.w * dx.abs() + b.h * dy.abs()) / 2.0;
    let start = (cx - dx * half, cy - dy * half);
    let end = (cx + dx * half, cy + dy * half);
    Some(Paint::Linear { start, end, stops })
}

/// Resolves the outline of a shape: the theme line from `lnRef` (if any) with
/// the shape's own `a:ln` merged on top. Returns `None` for no visible line.
pub fn read_line(
    own: Option<&XNode>,
    style_ref: Option<&XNode>,
    ctx: &StyleCtx,
    bounds: Rect,
) -> Option<Stroke> {
    let ref_color = style_ref.and_then(|r| ctx.color(r, None));
    let theme_ln = style_ref
        .and_then(|r| r.attr_i64("idx"))
        .and_then(|i| ctx.theme.line_style(i).cloned());
    let layers: Vec<&XNode> = [theme_ln.as_deref(), own].into_iter().flatten().collect();
    if layers.is_empty() {
        return None;
    }
    let mut width_emu: Option<f64> = None;
    let mut fill: Option<Fill> = None;
    let mut dash: Vec<f64> = Vec::new();
    let mut cap = LineCap::Flat;
    let mut join = LineJoin::Round;
    let mut head = None;
    let mut tail = None;
    for ln in layers {
        if let Some(w) = ln.attr_f64("w") {
            width_emu = Some(w);
        }
        match ln.attr("cap") {
            Some("rnd") => cap = LineCap::Round,
            Some("sq") => cap = LineCap::Square,
            Some("flat") => cap = LineCap::Flat,
            _ => {}
        }
        if let Some(f) = fill_child(ln) {
            fill = Some(read_fill(f, ctx, ref_color, bounds));
        }
        if let Some(d) = ln.child("a:prstDash") {
            dash = preset_dash(d.attr("val").unwrap_or("solid"));
        }
        if let Some(cd) = ln.child("a:custDash") {
            dash = cd
                .children_named("a:ds")
                .flat_map(|ds| {
                    let d = ds.attr_i64("d").unwrap_or(100_000) as f64 / 100_000.0;
                    let sp = ds.attr_i64("sp").unwrap_or(100_000) as f64 / 100_000.0;
                    [d, sp]
                })
                .collect();
        }
        if ln.child("a:round").is_some() {
            join = LineJoin::Round;
        } else if ln.child("a:bevel").is_some() {
            join = LineJoin::Bevel;
        } else if ln.child("a:miter").is_some() {
            join = LineJoin::Miter;
        }
        if let Some(h) = ln.child("a:headEnd") {
            head = read_arrow(h);
        }
        if let Some(t) = ln.child("a:tailEnd") {
            tail = read_arrow(t);
        }
    }
    let Fill::Paint(paint) = fill? else {
        return None;
    };
    // w="0" (or absent) renders as a hairline in Office: one device pixel.
    let width = width_emu.unwrap_or(9_525.0).max(0.0) / EMU_PER_PT;
    Some(Stroke {
        paint,
        width: width.max(0.25),
        dash,
        cap,
        join,
        head,
        tail,
    })
}

fn read_arrow(node: &XNode) -> Option<Arrow> {
    let kind = match node.attr("type")? {
        "triangle" => ArrowKind::Triangle,
        "stealth" => ArrowKind::Stealth,
        "diamond" => ArrowKind::Diamond,
        "oval" => ArrowKind::Oval,
        "arrow" => ArrowKind::Arrow,
        _ => return None,
    };
    let size = |v: Option<&str>| match v {
        Some("sm") => 2.0,
        Some("lg") => 5.0,
        _ => 3.0,
    };
    Some(Arrow {
        kind,
        w: size(node.attr("w")),
        len: size(node.attr("len")),
    })
}

/// Dash patterns in multiples of the line width (dash, gap, dash, gap...).
pub fn preset_dash(val: &str) -> Vec<f64> {
    match val {
        "sysDot" => vec![1.0, 1.0],
        "sysDash" => vec![3.0, 1.0],
        "dash" => vec![4.0, 3.0],
        "dot" => vec![1.0, 3.0],
        "dashDot" => vec![4.0, 3.0, 1.0, 3.0],
        "lgDash" => vec![8.0, 3.0],
        "lgDashDot" => vec![8.0, 3.0, 1.0, 3.0],
        "lgDashDotDot" => vec![8.0, 3.0, 1.0, 3.0, 1.0, 3.0],
        "sysDashDot" => vec![3.0, 1.0, 1.0, 1.0],
        "sysDashDotDot" => vec![3.0, 1.0, 1.0, 1.0, 1.0, 1.0],
        _ => Vec::new(),
    }
}

/// Resolves the fill of a shape: its own `spPr` fill, else the theme fill from
/// the `fillRef` style reference.
pub fn shape_fill(
    sp_pr: Option<&XNode>,
    style_ref: Option<&XNode>,
    ctx: &StyleCtx,
    bounds: Rect,
) -> Option<Fill> {
    if let Some(f) = sp_pr.and_then(fill_child) {
        return Some(read_fill(f, ctx, None, bounds));
    }
    let r = style_ref?;
    let idx = r.attr_i64("idx")?;
    let ref_color = ctx.color(r, None);
    let theme_fill = ctx.theme.fill_style(idx)?;
    Some(read_fill(theme_fill, &ctx.for_theme(), ref_color, bounds))
}

/// The outer shadow of a shape: its own `a:effectLst` when present (an empty
/// list means no effects), else the theme effect style from `effectRef`.
pub fn read_shadow(
    own: Option<&XNode>,
    effect_ref: Option<&XNode>,
    ctx: &StyleCtx,
) -> Option<Shadow> {
    if let Some(lst) = own {
        return lst
            .child("a:outerShdw")
            .and_then(|s| shadow_of(s, ctx, None));
    }
    let r = effect_ref?;
    let idx = r.attr_i64("idx").filter(|i| *i >= 1)?;
    let style = ctx.theme.effect_styles.get((idx - 1) as usize)?;
    let ref_color = ctx.color(r, None);
    style
        .child("a:effectLst")?
        .child("a:outerShdw")
        .and_then(|s| shadow_of(s, ctx, ref_color))
}

fn shadow_of(node: &XNode, ctx: &StyleCtx, ph: Option<Rgba>) -> Option<Shadow> {
    let color = ctx.color(node, ph)?;
    let emu = |k: &str| node.attr_f64(k).unwrap_or(0.0) / EMU_PER_PT;
    let dir = (node.attr_f64("dir").unwrap_or(0.0) / 60_000.0).to_radians();
    let dist = emu("dist");
    Some(Shadow {
        color,
        blur: emu("blurRad"),
        dx: dist * dir.cos(),
        dy: dist * dir.sin(),
        glow: false,
        soft: 0.0,
    })
}
