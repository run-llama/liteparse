//! Shape effects besides the outer shadow (`style::read_shadow`): `a:glow`, a
//! blurred halo of a color around the silhouette, and `a:softEdge`, which fades
//! the shape's own edges to transparent. Both live in the shape's `a:effectLst`
//! or in the theme effect style its `effectRef` picks; the rasterizer draws them
//! (`render::draw_shadow` for glow, `render/softedge.rs` for soft edges).

use tiny_skia::{Path, PathBuilder, Transform};

use crate::office::pptx::color::Rgba;
use crate::office::pptx::geom::{FillMode, Geometry};
use crate::office::pptx::scene::{Paint, Shadow, Stroke};
use crate::office::pptx::style::{EMU_PER_PT, StyleCtx};
use crate::office::pptx::xml::XNode;

/// The effect list that applies: the shape's own `a:effectLst` when present (an
/// empty list means no effects), else the theme effect style from `effectRef`,
/// whose color stands in for `phClr`.
fn effect_list<'n>(
    own: Option<&'n XNode>,
    effect_ref: Option<&XNode>,
    ctx: &'n StyleCtx,
) -> Option<(&'n XNode, Option<Rgba>)> {
    if let Some(lst) = own {
        return Some((lst, None));
    }
    let r = effect_ref?;
    let idx = r.attr_i64("idx").filter(|i| *i >= 1)?;
    let style = ctx.theme.effect_styles.get((idx - 1) as usize)?;
    Some((style.child("a:effectLst")?, ctx.color(r, None)))
}

/// `a:fillOverlay blend="mult"`: the fill multiplied over the shape's own fill
/// (in the shape's local frame, as `bounds`). Other blend modes are not drawn.
pub fn read_fill_overlay(
    own: Option<&XNode>,
    effect_ref: Option<&XNode>,
    ctx: &StyleCtx,
    bounds: crate::office::pptx::scene::Rect,
) -> Option<Paint> {
    let (lst, ph) = effect_list(own, effect_ref, ctx)?;
    let overlay = lst.child("a:fillOverlay")?;
    if overlay.attr("blend") != Some("mult") {
        return None;
    }
    let fill = crate::office::pptx::style::fill_child(overlay)?;
    match crate::office::pptx::style::read_fill(fill, ctx, ph, bounds) {
        crate::office::pptx::style::Fill::Paint(p) => Some(p),
        _ => None,
    }
}

/// `a:glow`: color and radius (points) as a glow-flavoured [`Shadow`].
pub fn read_glow(
    own: Option<&XNode>,
    effect_ref: Option<&XNode>,
    ctx: &StyleCtx,
) -> Option<Shadow> {
    let (lst, ph) = effect_list(own, effect_ref, ctx)?;
    let g = lst.child("a:glow")?;
    let rad = g.attr_f64("rad").unwrap_or(0.0) / EMU_PER_PT;
    let color = ctx.color(g, ph)?;
    (rad > 0.0 && color.a > 0.0).then_some(Shadow {
        color,
        blur: rad,
        dx: 0.0,
        dy: 0.0,
        glow: true,
        soft: 0.0,
    })
}

/// `a:softEdge` radius in points.
pub fn read_soft_edge(
    own: Option<&XNode>,
    effect_ref: Option<&XNode>,
    ctx: &StyleCtx,
) -> Option<f64> {
    let (lst, _) = effect_list(own, effect_ref, ctx)?;
    let rad = lst.child("a:softEdge")?.attr_f64("rad")? / EMU_PER_PT;
    (rad > 0.0).then_some(rad)
}

/// One path covering a shape in slide space: its filled subpaths, or, for an
/// unfilled shape, its stroked outlines (as shadows use).
pub fn silhouette(
    g: &Geometry,
    tf: Transform,
    filled: bool,
    stroke: Option<&Stroke>,
) -> Option<Path> {
    let mut pb = PathBuilder::new();
    for gp in &g.paths {
        let Some(path) =
            crate::office::pptx::slide::to_skia_path(&gp.cmds).and_then(|(p, _)| p.transform(tf))
        else {
            continue;
        };
        let outline = if filled && gp.fill != FillMode::None {
            Some(path)
        } else if gp.stroke {
            stroke.and_then(|s| {
                let st = tiny_skia::Stroke {
                    width: s.width as f32,
                    ..Default::default()
                };
                path.stroke(&st, 1.0)
            })
        } else {
            None
        };
        if let Some(p) = outline {
            pb.push_path(&p);
        }
    }
    pb.finish()
}

#[cfg(test)]
mod tests;
