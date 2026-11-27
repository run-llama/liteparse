//! Page decoration drawn outside the text flow: page borders (`w:pgBorders`)
//! and the stripes of multi-line border styles.

use crate::office::pptx::scene::{Item, Paint, PathItem};

use crate::office::docx::layout::border_color;
use crate::office::docx::props::{Border, Borders};

/// `w:pgBorders`: the section's page borders.
#[derive(Debug, Clone, Default)]
pub struct PageBorders {
    pub borders: Borders,
    /// `offsetFrom="page"`: `space` is measured from the page edge, else from the text.
    pub from_page: bool,
    /// `display`: `allPages`, `firstPage` or `notFirstPage`.
    pub display: String,
}

/// Stripes of a border style as (offset from the outer edge, width), in points,
/// for a border whose line width is `w`. Dotted and dashed styles are drawn solid.
pub fn stripes(style: &str, w: f64) -> Vec<(f64, f64)> {
    let w = w.max(0.25);
    match style {
        "double" => vec![(0.0, w), (2.0 * w, w)],
        "triple" => vec![(0.0, w), (2.0 * w, w), (4.0 * w, w)],
        s if s.starts_with("thinThick") => vec![(0.0, 0.75), (0.75 + w * 0.5, w)],
        s if s.starts_with("thickThin") => vec![(0.0, w), (w * 1.5, 0.75)],
        _ => vec![(0.0, w)],
    }
}

/// Whether a style is a line style; art borders (pictures) are not drawn.
fn line_style(style: &str) -> bool {
    matches!(
        style,
        "single"
            | "thick"
            | "double"
            | "dotted"
            | "dashed"
            | "dotDash"
            | "dotDotDash"
            | "triple"
            | "wave"
            | "doubleWave"
            | "dashSmallGap"
            | "dashDotStroked"
            | "threeDEmboss"
            | "threeDEngrave"
            | "outset"
            | "inset"
    ) || style.starts_with("thinThick")
        || style.starts_with("thickThin")
}

fn fill(x: f64, y: f64, w: f64, h: f64, b: &Border, out: &mut Vec<Item>) {
    if let Some(path) = crate::office::pptx::textlayout::rect_path(x, y, w.max(0.0), h.max(0.0)) {
        out.push(Item::Path(PathItem {
            path,
            fill: Some(Paint::Solid(border_color(b))),
            stroke: None,
            has_curve: false,
            shape: None,
            from_text: false,
        }));
    }
}

/// Total width of a border's stripes.
fn extent(b: &Border) -> f64 {
    stripes(&b.style, b.width)
        .iter()
        .map(|(o, w)| o + w)
        .fold(0.0, f64::max)
}

/// Draws the page border rectangle of a page. `body` is the text area
/// (left, top, right, bottom) in page points.
pub fn page_border(
    pb: &PageBorders,
    page_w: f64,
    page_h: f64,
    body: (f64, f64, f64, f64),
    first_of_section: bool,
    out: &mut Vec<Item>,
) {
    match pb.display.as_str() {
        "firstPage" if !first_of_section => return,
        "notFirstPage" if first_of_section => return,
        _ => {}
    }
    let b = &pb.borders;
    let side = |s: &Option<Border>| s.clone().filter(|s| s.visible() && line_style(&s.style));
    let (top, left, bottom, right) = (side(&b.top), side(&b.left), side(&b.bottom), side(&b.right));
    let ext = |s: &Option<Border>| s.as_ref().map_or(0.0, extent);
    // Outer edges of the rectangle: from the page edge, or outside the text.
    let edge = |s: &Option<Border>, page_edge: f64, text_edge: f64, sign: f64| match s {
        Some(s) if pb.from_page => page_edge + sign * s.space,
        Some(s) => text_edge - sign * (s.space + extent(s)),
        None => text_edge,
    };
    let x0 = edge(&left, 0.0, body.0, 1.0);
    let y0 = edge(&top, 0.0, body.1, 1.0);
    let x1 = edge(&right, page_w, body.2, -1.0);
    let y1 = edge(&bottom, page_h, body.3, -1.0);
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    if let Some(s) = &top {
        for (o, w) in stripes(&s.style, s.width) {
            fill(x0, y0 + o, x1 - x0, w, s, out);
        }
    }
    if let Some(s) = &bottom {
        for (o, w) in stripes(&s.style, s.width) {
            fill(x0, y1 - o - w, x1 - x0, w, s, out);
        }
    }
    let (t, bm) = (ext(&top), ext(&bottom));
    if let Some(s) = &left {
        for (o, w) in stripes(&s.style, s.width) {
            fill(x0 + o, y0 + t, w, y1 - y0 - t - bm, s, out);
        }
    }
    if let Some(s) = &right {
        for (o, w) in stripes(&s.style, s.width) {
            fill(x1 - o - w, y0 + t, w, y1 - y0 - t - bm, s, out);
        }
    }
}

/// `w:lnNumType`: line numbers in the margin beside the body text.
#[derive(Debug, Clone)]
pub struct LineNumbers {
    pub count_by: i64,
    pub start: i64,
    /// Gap between the number and the text (points); Word's automatic gap is 0.25 in.
    pub distance: Option<f64>,
    /// `newPage`, `newSection` or `continuous`.
    pub restart: String,
    pub style: std::rc::Rc<crate::office::docx::styles::RunStyle>,
}

/// Draws line number `n` right-aligned `distance` left of `text_x`, on the baseline `base_y`.
pub fn line_number(
    ln: &LineNumbers,
    n: i64,
    text_x: f64,
    base_y: f64,
    env: &crate::office::docx::text::TextEnv,
    out: &mut Vec<Item>,
) {
    let gap = ln.distance.unwrap_or(18.0);
    let para = crate::office::docx::model::Paragraph {
        style: crate::office::docx::styles::ParaStyle::from_ppr(
            &crate::office::docx::props::PPr::default(),
            12.0,
        )
        .into(),
        inlines: vec![crate::office::docx::model::Inline::Text(
            n.to_string(),
            ln.style.clone(),
        )],
        mark: ln.style.clone(),
        num: None,
    };
    let lines = crate::office::docx::text::layout_paragraph(&para, 1.0e4, env);
    let Some(l) = lines.first() else {
        return;
    };
    let w = l
        .items
        .iter()
        .filter_map(|it| match it {
            Item::Text(r) => Some(r.transform.tx as f64 + r.width),
            _ => None,
        })
        .fold(0.0_f64, f64::max);
    for it in &l.items {
        let mut it = it.clone();
        crate::office::docx::text::translate_item(&mut it, text_x - gap - w, base_y - l.baseline);
        out.push(it);
    }
}

#[cfg(test)]
mod tests;

/// A border band along one edge: `horizontal` from (`x`, `y`) rightwards over
/// `len`, else downwards. (`x`, `y`) is the band's top-left corner; the band is
/// `band_width(b)` thick. Multi-line styles draw their stripes, dotted and
/// dashed styles their dashes (dot = line width, dash = 3 widths).
pub fn edge(b: &Border, x: f64, y: f64, len: f64, horizontal: bool, out: &mut Vec<Item>) {
    let w = b.width.max(0.25);
    let stripes = stripes(&b.style, b.width);
    // Dash pattern (on, off) in line widths.
    let pattern = match b.style.as_str() {
        "dotted" => Some((1.0, 1.0)),
        "dashed" => Some((3.0, 3.0)),
        "dashSmallGap" => Some((3.0, 1.0)),
        "dotDash" | "dotDotDash" => Some((2.0, 1.5)),
        _ => None,
    };
    for (off, sw) in stripes {
        let mut seg = |a: f64, l: f64| {
            if horizontal {
                fill(x + a, y + off, l, sw, b, out)
            } else {
                fill(x + off, y + a, sw, l, b, out)
            }
        };
        match pattern {
            Some((on, gap)) => {
                let (on, gap) = (on * w, gap * w);
                let mut a = 0.0;
                while a < len {
                    seg(a, on.min(len - a));
                    a += on + gap;
                }
            }
            None => seg(0.0, len),
        }
    }
}

/// A run border (`w:bdr`) around the text box (`x`, `y`, `w`, `h`): `space`
/// pads it left and right; `open`/`close` draw the left/right sides.
#[expect(
    clippy::too_many_arguments,
    reason = "a border side needs the run box, both side flags and the output list"
)]
pub fn run_border(
    b: &Border,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    open: bool,
    close: bool,
    out: &mut Vec<Item>,
) {
    let t = band_width(b);
    let pad = b.space.min(31.0);
    let x0 = if open { x - pad - t } else { x };
    let x1 = if close { x + w + pad + t } else { x + w };
    edge(b, x0, y - t, x1 - x0, true, out);
    edge(b, x0, y + h, x1 - x0, true, out);
    if open {
        edge(b, x0, y - t, h + 2.0 * t, false, out);
    }
    if close {
        edge(b, x1 - t, y - t, h + 2.0 * t, false, out);
    }
}

/// A straight border line between two points (diagonal cell borders).
pub fn line(b: &Border, a: (f64, f64), z: (f64, f64)) -> Option<Item> {
    let mut pb = tiny_skia::PathBuilder::new();
    pb.move_to(a.0 as f32, a.1 as f32);
    pb.line_to(z.0 as f32, z.1 as f32);
    Some(Item::Path(PathItem {
        path: pb.finish()?,
        fill: None,
        stroke: Some(crate::office::pptx::scene::Stroke {
            paint: Paint::Solid(border_color(b)),
            width: b.width.max(0.25),
            dash: Vec::new(),
            cap: crate::office::pptx::scene::LineCap::Flat,
            join: crate::office::pptx::scene::LineJoin::Miter,
            head: None,
            tail: None,
        }),
        has_curve: false,
        shape: None,
        from_text: false,
    }))
}

/// The shadow of a picture placed at (`x`, `y`), from its rectangle.
pub fn picture_shadow(pic: &crate::office::docx::model::Picture, x: f64, y: f64) -> Option<Item> {
    let shadow = pic.shadow?;
    let path = crate::office::pptx::textlayout::rect_path(x, y, pic.w, pic.h)?;
    Some(Item::Shadow(crate::office::pptx::scene::ShadowItem {
        source: crate::office::pptx::scene::ShadowSource::Path(path),
        shadow,
    }))
}

/// Thickness of a border band (all its stripes).
pub fn band_width(b: &Border) -> f64 {
    extent(b)
}
