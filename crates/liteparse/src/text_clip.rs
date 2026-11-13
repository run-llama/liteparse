//! Conservative glyph visibility under rectangular PDF graphics-state clips.
//! Clip points already include the object's CTM; only ancestor form matrices
//! remain. Unknown paths/transforms preserve text, including OCR text layers.
//!
//! Visibility is decided per word: a word with any glyph inside its clip is kept
//! whole, so a mostly hidden line never loses just its low punctuation
//! (`1234.56` -> `1234 56`). Only leader runs (`......`) are trimmed per glyph.

use std::collections::{HashMap, HashSet};

use pdfium::{
    Matrix, Page, PageObject, PageObjectKind, RawPathSegment, RectF, SegmentKind, TextPage,
};

use crate::extract::{CharInfoChunks, CharView};

const IDENTITY: Matrix = Matrix {
    a: 1.0,
    b: 0.0,
    c: 0.0,
    d: 1.0,
    e: 0.0,
    f: 0.0,
};

/// Char indices of a text page hidden by rectangular clips.
pub(crate) struct TextClip {
    hidden: HashSet<i32>,
}

impl TextClip {
    /// `keep` (the `keep_clipped_text` option) disables the filter.
    pub(crate) fn new(page: &Page, text_page: &TextPage, keep: bool) -> Self {
        let mut hidden = HashSet::new();
        if keep {
            return Self { hidden };
        }
        let clips = ObjectClips::new(page);
        // Most pages clip no text: skip the char pass entirely.
        if clips.objects.is_empty() {
            return Self { hidden };
        }
        let mut chunks = CharInfoChunks::new(text_page);
        let mut word = Word::default();
        // The loose box and clip of the word's last glyph.
        let mut last: Option<RectF> = None;
        let mut last_clip: Option<RectF> = None;
        for i in 0..text_page.char_count() {
            let ch = text_page.char_at_unchecked(i);
            let cv = CharView {
                ch: &ch,
                rec: chunks.as_mut().and_then(|chunks| chunks.record(i)),
            };
            let c = char::from_u32(cv.unicode());
            if cv.is_generated() || c.is_some_and(char::is_whitespace) {
                word.close(&mut hidden);
                last = None;
                // A space is judged on its own loose box.
                if clips.hides(&cv) {
                    hidden.insert(i);
                }
                continue;
            }
            // pdfium emits no separator at every line break or between objects drawn
            // over each other, so a word also ends where its glyphs stop touching or
            // its clip changes (overflow from one cell running into the next).
            let loose = cv.loose_char_box();
            let clip = clips.bounds(&cv);
            if !word.glyphs.is_empty()
                && (clip != last_clip
                    || matches!((last, loose), (Some(prev), Some(cur)) if !adjacent(prev, cur)))
            {
                word.close(&mut hidden);
            }
            last = loose;
            last_clip = clip;
            word.glyphs.push((i, c, clips.hides(&cv)));
        }
        word.close(&mut hidden);
        Self { hidden }
    }

    pub(crate) fn hides(&self, index: i32) -> bool {
        self.hidden.contains(&index)
    }
}

/// A run of touching glyphs under one clip: each glyph's char index, char and own verdict.
#[derive(Default)]
struct Word {
    glyphs: Vec<(i32, Option<char>, bool)>,
}

impl Word {
    /// A word with any visible glyph is kept whole, except a leader run inside it
    /// (`775……………`): leaders carry no value to corrupt, so their dots keep their own
    /// verdicts and a leader that overflows its cell is trimmed at the clip.
    fn close(&mut self, hidden: &mut HashSet<i32>) {
        let visible = self.glyphs.iter().any(|&(_, _, h)| !h);
        let leaders = leader_runs(&self.glyphs);
        for (&(i, _, h), leader) in self.glyphs.iter().zip(leaders) {
            if h && (!visible || leader) {
                hidden.insert(i);
            }
        }
        self.glyphs.clear();
    }
}

/// Leader characters in a row that make a leader (`...` and longer).
const MIN_LEADER_RUN: usize = 3;

/// Marks the glyphs in runs of [`MIN_LEADER_RUN`] or more leader characters.
fn leader_runs(glyphs: &[(i32, Option<char>, bool)]) -> Vec<bool> {
    let is_leader = |c: Option<char>| {
        matches!(
            c,
            Some(
                '.' | '\u{2026}'
                    | '_'
                    | '-'
                    | '\u{2013}'
                    | '\u{2014}'
                    | '\u{00B7}'
                    | '\u{2025}'
                    | '\u{22EF}'
            )
        )
    };
    let mut marks = vec![false; glyphs.len()];
    let mut start = 0;
    while start < glyphs.len() {
        let end = start
            + glyphs[start..]
                .iter()
                .take_while(|&&(_, c, _)| is_leader(c))
                .count();
        if end - start >= MIN_LEADER_RUN {
            marks[start..end].fill(true);
        }
        start = end + 1;
    }
    marks
}

/// Only objects whose complete path stack is understood have an entry.
/// Bounds are y-up page coordinates; inverted bounds mean an empty intersection.
struct ObjectClips {
    objects: HashMap<usize, RectF>,
}

impl ObjectClips {
    fn new(page: &Page) -> Self {
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

    /// The clip bounds of the glyph's text object, `None` when it is unclipped.
    fn bounds(&self, cv: &CharView<'_, '_>) -> Option<RectF> {
        cv.text_object()
            .and_then(|obj| self.objects.get(&(obj as usize)))
            .copied()
    }

    /// Generated separators have no reliable source object and are preserved.
    /// A glyph is visible when its ink box, widened by [`INK_MARGIN`] of its em
    /// extent, meets the clip; glyphs without ink are judged by their loose box.
    fn hides(&self, cv: &CharView<'_, '_>) -> bool {
        if cv.is_generated() {
            return false;
        }
        let Some(bounds) = self.bounds(cv) else {
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

/// Whether two loose glyph boxes touch, within a quarter em, at any rotation.
fn adjacent(a: RectF, b: RectF) -> bool {
    let gap_x = (b.left - a.right).max(a.left - b.right);
    let gap_y = (b.bottom - a.top).max(a.bottom - b.top);
    f64::from(gap_x.max(gap_y)) <= 0.25 * em_extent(a).max(em_extent(b))
}

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
fn rectangle(path: &[RawPathSegment], parent: &Matrix) -> Option<RectF> {
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
