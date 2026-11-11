//! Conservative glyph visibility under rectangular PDF graphics-state clips.
//! Clip points already include the object's CTM; only ancestor form matrices
//! remain. Unknown paths/transforms preserve text, including OCR text layers.

use std::collections::HashMap;

use pdfium::{Matrix, Page, PageObject, PageObjectKind, RawPathSegment, RectF, SegmentKind};

use crate::extract::CharView;

const IDENTITY: Matrix = Matrix {
    a: 1.0,
    b: 0.0,
    c: 0.0,
    d: 1.0,
    e: 0.0,
    f: 0.0,
};

/// Only objects whose complete path stack is understood have an entry.
/// Bounds are y-up page coordinates; inverted bounds mean an empty intersection.
pub(crate) struct TextClip {
    objects: HashMap<usize, RectF>,
}

impl TextClip {
    pub(crate) fn new(page: &Page) -> Self {
        let mut clips = Self {
            objects: HashMap::new(),
        };
        for index in 0..page.object_count() {
            if let Some(object) = page.object(index) {
                clips.walk(&object, &IDENTITY, None, 0);
            }
        }
        clips
    }

    fn walk(
        &mut self,
        object: &PageObject,
        parent: &Matrix,
        inherited: Option<RectF>,
        depth: usize,
    ) {
        if depth > 64 {
            return;
        }
        let Some(paths) = object.clip_paths() else {
            return;
        };
        let mut bounds = inherited;
        for path in paths {
            let Some(rect) = rectangle(&path, parent) else {
                return;
            };
            bounds = Some(match bounds {
                None => rect,
                Some(old) => RectF {
                    left: old.left.max(rect.left),
                    right: old.right.min(rect.right),
                    bottom: old.bottom.max(rect.bottom),
                    top: old.top.min(rect.top),
                },
            });
        }
        match object.kind() {
            PageObjectKind::Text => {
                if let Some(bounds) = bounds {
                    self.objects.insert(object.id(), bounds);
                }
            }
            PageObjectKind::Form => {
                let Some(matrix) = object.matrix() else {
                    return;
                };
                let matrix = compose(parent, &matrix);
                if !valid_matrix(&matrix) {
                    return;
                }
                let Some(count) = object.form_object_count() else {
                    return;
                };
                for index in 0..count {
                    if let Some(child) = object.form_object(index) {
                        self.walk(&child, &matrix, bounds, depth + 1);
                    }
                }
            }
            _ => {}
        }
    }

    /// Generated separators have no reliable source object and are preserved.
    /// A glyph is kept whole when its ink box, widened by [`INK_MARGIN`] of its em
    /// extent, meets the clip; glyphs without ink are judged by their loose box.
    pub(crate) fn hides(&self, cv: &CharView<'_, '_>) -> bool {
        if cv.is_generated() {
            return false;
        }
        let Some(bounds) = cv
            .text_object()
            .and_then(|obj| self.objects.get(&(obj as usize)))
        else {
            return false;
        };
        // The ink (strict) box decides: a loose box carries the font's side bearings and
        // ascent, so it can graze a clip edge by a fraction of a point while every pixel of
        // the glyph lies outside. Whitespace and inkless glyphs keep the loose box.
        let inked = !char::from_u32(cv.unicode()).is_some_and(char::is_whitespace);
        let loose = cv.loose_char_box();
        let strict = cv
            .strict_char_box()
            .filter(|b| inked && b.right > b.left && b.top > b.bottom);
        let (glyph, margin) = match (strict, loose) {
            (Some(ink), _) => (ink, INK_MARGIN * loose.map_or(0.0, em_extent)),
            (None, Some(loose)) => (loose, 0.0),
            (None, None) => return false,
        };
        if ![glyph.left, glyph.right, glyph.bottom, glyph.top]
            .iter()
            .all(|v| v.is_finite())
            || glyph.left > glyph.right
            || glyph.bottom > glyph.top
        {
            return false;
        }
        // A tiny tolerance prevents float rounding at a clip edge losing a glyph.
        const EPS: f64 = 0.001;
        let tol = EPS + margin;
        bounds.left > bounds.right
            || bounds.bottom > bounds.top
            || f64::from(glyph.right) < f64::from(bounds.left) - tol
            || f64::from(glyph.left) > f64::from(bounds.right) + tol
            || f64::from(glyph.top) < f64::from(bounds.bottom) - tol
            || f64::from(glyph.bottom) > f64::from(bounds.top) + tol
    }
}

/// Share of a glyph's em extent by which its ink box may miss a clip and still count
/// as visible. PDFium's strict box can understate the painted outline (a substituted
/// font, quantised glyph bounds) by about 1% of the font size; that edge of ink is drawn.
const INK_MARGIN: f64 = 0.02;

/// The larger side of the loose box, about one em (ascent to descent) at any rotation.
fn em_extent(loose: RectF) -> f64 {
    let em = f64::from(
        (loose.right - loose.left)
            .abs()
            .max((loose.top - loose.bottom).abs()),
    );
    if em.is_finite() { em } else { 0.0 }
}

fn compose(p: &Matrix, c: &Matrix) -> Matrix {
    Matrix {
        a: p.a * c.a + p.c * c.b,
        b: p.b * c.a + p.d * c.b,
        c: p.a * c.c + p.c * c.d,
        d: p.b * c.c + p.d * c.d,
        e: p.a * c.e + p.c * c.f + p.e,
        f: p.b * c.e + p.d * c.f + p.f,
    }
}

fn valid_matrix(m: &Matrix) -> bool {
    [m.a, m.b, m.c, m.d, m.e, m.f].iter().all(|v| v.is_finite())
        && (f64::from(m.a) * f64::from(m.d) - f64::from(m.b) * f64::from(m.c)).abs() > 1e-12
}

/// Accept exactly one closed rectangle, never the bounding box of a compound
/// path (which can contain holes). Rotated/sheared non-axis-aligned clips fail open.
pub(crate) fn rectangle(path: &[RawPathSegment], parent: &Matrix) -> Option<RectF> {
    if !matches!(path.len(), 4 | 5) || !path.last()?.close || !valid_matrix(parent) {
        return None;
    }
    let mut points = Vec::with_capacity(path.len());
    for (i, segment) in path.iter().enumerate() {
        let expected = if i == 0 {
            SegmentKind::MoveTo
        } else {
            SegmentKind::LineTo
        };
        if segment.kind != Some(expected) || (i + 1 != path.len() && segment.close) {
            return None;
        }
        let (x, y) = segment.point?;
        let point = (
            parent.a * x + parent.c * y + parent.e,
            parent.b * x + parent.d * y + parent.f,
        );
        if !point.0.is_finite() || !point.1.is_finite() {
            return None;
        }
        points.push(point);
    }
    if points.len() == 5 && points.pop()? != points[0] {
        return None;
    }
    let mut horizontal = [false; 4];
    for i in 0..4 {
        let a = points[i];
        let b = points[(i + 1) % 4];
        horizontal[i] = a.1 == b.1 && a.0 != b.0;
        if !(horizontal[i] || a.0 == b.0 && a.1 != b.1) {
            return None;
        }
    }
    if (0..4).any(|i| horizontal[i] == horizontal[(i + 1) % 4]) {
        return None;
    }
    Some(RectF {
        left: points.iter().map(|p| p.0).fold(f32::INFINITY, f32::min),
        right: points.iter().map(|p| p.0).fold(f32::NEG_INFINITY, f32::max),
        bottom: points.iter().map(|p| p.1).fold(f32::INFINITY, f32::min),
        top: points.iter().map(|p| p.1).fold(f32::NEG_INFINITY, f32::max),
    })
}
