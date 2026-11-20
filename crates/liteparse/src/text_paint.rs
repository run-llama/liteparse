//! Text whose fill never reaches the page: filled white with nothing painted
//! under it, or filled the colour of the opaque rectangle directly beneath it.
//! Only top-level text objects are judged. An unknown backdrop (image, shading,
//! form, pattern or translucent fill, anything that is not a plain rectangle)
//! preserves the text. PDFium reports pattern and shading fills as opaque white,
//! so every candidate is confirmed against a 72 dpi render of the page and kept
//! when any pixel under it differs from the expected backdrop.

use std::collections::HashSet;

use pdfium::{Color, Page, PageObject, PageObjectKind, RectF};

use crate::extract::CharView;
use crate::text_clip::rectangle;

/// A top-level object below the text being judged. Whether it is a plain opaque
/// rectangle is only worked out when a text object lands on it.
struct Painted {
    bounds: RectF,
    index: Option<usize>,
}

/// A text object whose fill looks invisible, with the colour the page should
/// show under it.
struct Candidate {
    id: usize,
    bounds: RectF,
    backdrop: Color,
}

pub(crate) struct TextPaint {
    unpainted: HashSet<usize>,
}

impl TextPaint {
    pub(crate) fn new(page: &Page, view_box: &RectF) -> Self {
        let mut candidates = Vec::new();
        let mut below: Vec<Painted> = Vec::new();
        for index in 0..page.object_count() {
            let Some(object) = page.object(index) else {
                continue;
            };
            let Some(bounds) = object.bounds() else {
                // An object with unknown extent could be anywhere.
                below.push(Painted {
                    bounds: everywhere(),
                    index: None,
                });
                continue;
            };
            if object.kind() == PageObjectKind::Text {
                if let Some(backdrop) = fill_never_shows(page, &object, &bounds, &below) {
                    candidates.push(Candidate {
                        id: object.id(),
                        bounds,
                        backdrop,
                    });
                } else {
                    below.push(Painted {
                        bounds,
                        index: None,
                    });
                }
                continue;
            }
            below.push(Painted {
                bounds,
                index: Some(index),
            });
        }
        Self {
            unpainted: confirm(page, view_box, &candidates),
        }
    }

    /// Generated separators have no reliable source object and are preserved.
    /// Only fill-only text (render mode 0) is judged, since a stroke still paints.
    pub(crate) fn hides(&self, cv: &CharView<'_, '_>) -> bool {
        !cv.is_generated()
            && cv.text_render_mode() == Some(0)
            && cv
                .text_object()
                .is_some_and(|obj| self.unpainted.contains(&(obj as usize)))
    }
}

fn everywhere() -> RectF {
    RectF {
        left: f32::NEG_INFINITY,
        right: f32::INFINITY,
        bottom: f32::NEG_INFINITY,
        top: f32::INFINITY,
    }
}

/// The backdrop colour the text is expected to vanish into, if its fill matches.
fn fill_never_shows(
    page: &Page,
    text: &PageObject,
    bounds: &RectF,
    below: &[Painted],
) -> Option<Color> {
    let fill = text.fill_color().filter(|c| c.a == 255)?;
    let Some(top) = below.iter().rev().find(|p| overlaps(&p.bounds, bounds)) else {
        // Nothing painted under it: the backdrop is the white page.
        return same_rgb(&fill, &WHITE).then_some(WHITE);
    };
    if !contains(&top.bounds, bounds) {
        return None;
    }
    let backdrop = opaque_rect_fill(&page.object(top.index?)?)?;
    same_rgb(&fill, &backdrop).then_some(backdrop)
}

/// Ids of the candidates whose whole area renders as the expected backdrop.
fn confirm(page: &Page, view_box: &RectF, candidates: &[Candidate]) -> HashSet<usize> {
    if candidates.is_empty() {
        return HashSet::new();
    }
    // Pixel boxes at 72 dpi, one pixel wider on every side to absorb rounding.
    let (view_w, view_h) = page.viewport_size(view_box);
    let boxes: Vec<_> = candidates
        .iter()
        .map(|c| {
            let b = page.bounds_to_viewport(view_box, &c.bounds);
            (
                (b.left.floor() as i32 - 1).max(0),
                (b.top.floor() as i32 - 1).max(0),
                (b.right.ceil() as i32 + 1).min(view_w.floor() as i32),
                (b.bottom.ceil() as i32 + 1).min(view_h.floor() as i32),
            )
        })
        .collect();
    let left = boxes.iter().map(|b| b.0).min().unwrap_or(0);
    let top = boxes.iter().map(|b| b.1).min().unwrap_or(0);
    let right = boxes.iter().map(|b| b.2).max().unwrap_or(0);
    let bottom = boxes.iter().map(|b| b.3).max().unwrap_or(0);
    if left >= right || top >= bottom {
        return HashSet::new();
    }
    let Ok(bitmap) = page.render_region(72.0, left, top, right - left, bottom - top) else {
        return HashSet::new();
    };
    let stride = bitmap.stride() as usize;
    let pixels = bitmap.buffer();
    candidates
        .iter()
        .zip(&boxes)
        .filter(|(c, (x0, y0, x1, y1))| {
            let (x0, y0, x1, y1) = (*x0, *y0, *x1, *y1);
            x0 < x1
                && y0 < y1
                && (y0..y1).all(|y| {
                    (x0..x1).all(|x| {
                        let at = (y - top) as usize * stride + (x - left) as usize * 4;
                        // BGRA, with a little slack for rounding in blends.
                        [(2, c.backdrop.r), (1, c.backdrop.g), (0, c.backdrop.b)]
                            .iter()
                            .all(|&(i, v)| pixels[at + i].abs_diff(v) <= 2)
                    })
                })
        })
        .map(|(c, _)| c.id)
        .collect()
}

const WHITE: Color = Color {
    r: 255,
    g: 255,
    b: 255,
    a: 255,
};

fn same_rgb(a: &Color, b: &Color) -> bool {
    (a.r, a.g, a.b) == (b.r, b.g, b.b)
}

fn overlaps(a: &RectF, b: &RectF) -> bool {
    a.left <= b.right && b.left <= a.right && a.bottom <= b.top && b.bottom <= a.top
}

fn contains(outer: &RectF, inner: &RectF) -> bool {
    outer.left <= inner.left
        && inner.right <= outer.right
        && outer.bottom <= inner.bottom
        && inner.top <= outer.top
}

/// The fill of a path that paints exactly one axis-aligned rectangle in an
/// opaque flat colour with no stroke and no clip.
fn opaque_rect_fill(object: &PageObject) -> Option<Color> {
    if object.kind() != PageObjectKind::Path {
        return None;
    }
    let mode = object.path_draw_mode()?;
    if !mode.filled || mode.stroked || !object.clip_paths()?.is_empty() {
        return None;
    }
    let fill = object.fill_color().filter(|c| c.a == 255)?;
    let matrix = object.matrix()?;
    let segments: Vec<_> = (0..object.path_segment_count()?)
        .map(|i| object.path_segment(i))
        .collect::<Option<_>>()?;
    rectangle(&segments, &matrix)?;
    Some(fill)
}
