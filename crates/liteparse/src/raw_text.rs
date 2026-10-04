//! Raw text items: pdfium's text runs with fixed, heuristic-free segmentation.
//!
//! [`extract_raw_text_items`] is the low-level counterpart of the segmenter in
//! [`crate::extract`]. It applies no layout heuristics — no gap-based line or word
//! merging, no invisible-text skip, no ligature expansion, no punctuation
//! normalisation, no dedup — so its output is a stable function of what pdfium
//! reports and a consumer can build its own segmentation on top. Glyphs hidden by
//! known rectangular clips are omitted before grouping; partially clipped glyphs
//! are kept in full whenever their loose box intersects the clip. Unknown clips
//! preserve text.
//!
//! Rules:
//!
//! - An item is a run of glyphs from one text object at one angle. A glyph whose
//!   text object differs from the item's first glyph, or whose angle differs from it
//!   by more than 0.01 rad, starts a new item.
//! - A pdfium-generated `\r` or `\n` closes the open item and is dropped.
//! - Every other generated glyph, and every ASCII whitespace glyph (`\t`..`\r` and
//!   space; U+00A0, U+3000 and friends do not count), is appended and then closes the
//!   item. A pdfium-synthesised space is therefore always an item's last glyph, which
//!   is what [`RawTextItem::trailing_space_generated`] reports.
//! - Bounds are the union of the glyphs' loose boxes. Angle, colours, marked-content
//!   id, font and font metrics come from the first glyph. `text_width` sums per-glyph
//!   advance widths.
//! - The angle is the baseline direction of the glyph's text matrix (`atan2(b, a)`
//!   of its first column), not pdfium's `FPDFText_GetCharAngle`: a sheared (italic)
//!   font puts the shear in the second column, and it must not change the reading
//!   direction. Folded with the page `/Rotate` into `[0, 2π)`.
//! - [`RawTextItem::grounding_bounds`] is the tight box of the item's real glyphs:
//!   every non-generated, non-space glyph (judged after buggy-font recovery) is
//!   projected along and across the baseline and the extrema re-centred as a
//!   rectangle to rotate about its centre by the angle. `None` when no glyph
//!   qualifies. A sheared font's loose boxes overlap the next word; this box does
//!   not. Cardinal, unsheared text projects the corners of its strict glyph boxes.
//!   Rotated or sheared text projects the points of the glyph's outline instead
//!   (see [`glyph_outline_support`]): pdfium's char box already encloses a rotated
//!   glyph in page axes, and its empty corners cannot be rotated back into ink.
//!   Glyphs whose outline cannot stand in for the paint — stroked render modes,
//!   Type3 and non-`Identity-H` CID fonts, unreadable geometry — keep their strict
//!   boxes.
//! - [`RawTextItem::baseline_gap`] is measured only for an item that is a lone
//!   pdfium-generated space between two real glyphs of a horizontal font (Type1,
//!   TrueType, or a CID font with `Identity-H`) that share a baseline direction: the
//!   advance from the previous glyph's origin to the next one, minus the previous
//!   glyph's width, in page points. It reports the true separation where the
//!   envelope boxes cannot. `None` when the neighbours are ineligible or the origins
//!   run backwards, which happens when pdfium reverses object order on a rotated
//!   page.
//! - [`append_raw_widget_text_items`] adds the text painted by visible form-widget
//!   appearances, which pdfium's text API leaves out until they are flattened.
//! - A font is "buggy" when it is embedded and its name/type match the subset-font
//!   heuristic, or when any non-generated glyph decodes to a control or private-use
//!   codepoint. A glyph pdfium cannot map to Unicode decodes to 0 and therefore counts,
//!   so every Type3 item is buggy.
//! - A buggy verdict earned by a control or private-use decode is then widened to the
//!   whole font on the page when the font's map is shown wrong: an embedded font with
//!   such an item, whose other mapped glyphs the glyph recovery below identifies as
//!   something else (see [`font_map_is_contradicted`]), makes every item of the font
//!   buggy. Such a map shifts or scrambles all its codes, and only the codes that
//!   land on a control character give themselves away; the rest decode to the wrong
//!   printable characters. A font whose recovered glyphs agree with its map keeps the
//!   per-item verdict.
//! - With a [`GlyphResolver`], every non-generated glyph of a buggy item is re-decoded:
//!   Type3 glyph names first, then the resolver on the glyph outline. An unrecognised
//!   glyph becomes a space. Without a resolver the text is left as pdfium decoded it.
//! - The text is built from the glyph codepoints with C-string semantics: a 0
//!   codepoint (a glyph with no Unicode mapping) terminates the text early, and a
//!   codepoint that is not a Unicode scalar value (a surrogate, or above U+10FFFF)
//!   drops the whole item.
//!
//! Coordinates go through [`Page::bounds_to_viewport`] per glyph — the exact
//! `FPDF_PageToDevice` quantisation — rather than the affine
//! [`Page::viewport_transform`] approximation, so boxes are reproducible to the last
//! digit. The viewport is scaled by the page's `/UserUnit`, as everywhere in liteparse.

use pdfium::{Document, Font, FontType, Matrix, Page, RectF, TextPage};

use crate::GlyphResolver;
use crate::extract::{CharInfoChunks, CharView, is_buggy_codepoint, is_buggy_font};
use crate::glyph_names::resolve_glyph_name_codepoint;

/// Angle difference, in radians, at which a glyph starts a new item.
const ANGLE_SPLIT_RADIANS: f32 = 0.01;

/// One raw text item, in viewport space (top-left origin, 72 dpi points).
#[derive(Debug, Clone, PartialEq)]
pub struct RawTextItem {
    /// The glyph codepoints as UTF-8. Whitespace is kept, including a trailing
    /// space; see the module docs for the NUL and invalid-scalar semantics.
    pub text: String,
    /// One entry per glyph in page order: the raw char code from the content
    /// stream, 0 for a pdfium-generated glyph.
    pub char_codes: Vec<u32>,
    /// One entry per glyph, parallel to `char_codes`, for Type3 fonts only: the
    /// PostScript glyph name the font binds to that code, `""` where it binds
    /// none. `None` for every other font type.
    pub glyph_names: Option<Vec<String>>,
    /// Counter-clockwise rotation in radians with the page rotation folded in,
    /// in `[0, 2π)`. Kept in radians so a consumer can convert in the precision
    /// it needs.
    pub angle_radians: f32,
    /// Sum of the glyphs' advance widths in text space.
    pub text_width: f32,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    /// Tight box of the item's real glyphs, as a rectangle to rotate about its
    /// centre by `angle_radians`; see the module docs. Viewport space.
    pub grounding_bounds: Option<RectF>,
    /// Advance gap across a lone generated space, in page points; see the module
    /// docs. Zero or negative when the neighbours overlap.
    pub baseline_gap: Option<f64>,
    /// Marked-content id of the first glyph's text object, when it has one.
    pub mcid: Option<i32>,
    /// Base font name with pdfium's subset-tag stripping; `""` when the first
    /// glyph has no text object or its object has no font.
    pub font_name: String,
    pub font_size: f32,
    pub font_weight: i32,
    /// `font_size` scaled by the vertical scale of the text matrix; 0 when the
    /// first glyph has no text object.
    pub font_height: f32,
    /// 0 when the font exposes no metrics.
    pub font_ascent: f32,
    /// 0 when the font exposes no metrics.
    pub font_descent: f32,
    pub font_is_buggy: bool,
    /// The last glyph is a pdfium-synthesised space, not a space glyph from the
    /// content stream.
    pub trailing_space_generated: bool,
    /// First glyph's fill colour packed as ARGB, when its colour space is
    /// reportable as RGB.
    pub fill_color: Option<u32>,
    /// First glyph's stroke colour packed as ARGB, when reportable.
    pub stroke_color: Option<u32>,
}

/// Everything read per glyph before items are formed.
struct Glyph {
    index: i32,
    text_object: Option<pdfium::pdfium_sys::FPDF_PAGEOBJECT>,
    generated: bool,
    /// 0 for a generated glyph, which has no source char code.
    char_code: u32,
    /// 0 when pdfium reports a unicode-map error for the glyph.
    unicode: u32,
    /// Loose glyph box in viewport space.
    loose: RectF,
    /// Strict glyph box in viewport space; only meaningful for a non-generated
    /// glyph.
    strict: RectF,
    /// Normalised angle in radians, `[0, 2π)`.
    angle: f32,
}

/// Extract the raw text items of `text_page`. `view_box` is the page's crop box
/// ([`Page::view_box`]); `glyph_resolver` is the optional outline-based recovery
/// for buggy fonts (see the module docs), `None` to leave buggy text as decoded.
pub fn extract_raw_text_items(
    page: &Page,
    text_page: &TextPage,
    view_box: &RectF,
    glyph_resolver: Option<&dyn GlyphResolver>,
) -> Vec<RawTextItem> {
    let char_count = text_page.char_count();
    let page_rotation = page.rotation();
    let mut chunks = CharInfoChunks::new(text_page);
    let clips = crate::text_clip::TextClip::new(page);
    // Runs are collected first and turned into items afterwards, because whether a
    // run's font is buggy can depend on the font's other runs on the page.
    let mut runs: Vec<Vec<Glyph>> = Vec::new();
    let mut run: Vec<Glyph> = Vec::new();

    let flush = |run: &mut Vec<Glyph>, runs: &mut Vec<Vec<Glyph>>| {
        if !run.is_empty() {
            runs.push(std::mem::take(run));
        }
    };

    for i in 0..char_count {
        let ch = text_page.char_at_unchecked(i);
        let cv = CharView {
            ch: &ch,
            rec: chunks.as_mut().and_then(|chunks| chunks.record(i)),
        };
        if clips.hides(&cv) {
            flush(&mut run, &mut runs);
            continue;
        }
        let glyph = load_glyph(page, view_box, page_rotation, &cv, i);

        if let Some(first) = run.first()
            && (first.text_object != glyph.text_object
                || (first.angle - glyph.angle).abs() > ANGLE_SPLIT_RADIANS)
        {
            flush(&mut run, &mut runs);
        }

        if glyph.generated && matches!(glyph.unicode, 0x0A | 0x0D) {
            flush(&mut run, &mut runs);
            continue;
        }

        let closes_item = glyph.generated || is_c_locale_space(glyph.unicode);
        run.push(glyph);
        if closes_item {
            flush(&mut run, &mut runs);
        }
    }
    flush(&mut run, &mut runs);

    let mut recovery = glyph_resolver.map(GlyphRecovery::new);
    let contradicted = match &mut recovery {
        Some(recovery) => contradicted_fonts(&runs, recovery),
        None => Vec::new(),
    };
    // Outline shapes repeat across a page (the same glyph at the same size and
    // matrix), so they are computed once per page, not once per item.
    let mut outline_cache = OutlineSupportCache::default();
    runs.iter()
        .filter_map(|run| {
            let font_map_broken = run_font_key(run).is_some_and(|key| contradicted.contains(&key));
            build_item(
                page,
                text_page,
                view_box,
                run,
                recovery.as_mut(),
                font_map_broken,
                &mut outline_cache,
            )
        })
        .collect()
}

/// Glyph recovery for one page, with its answers kept per (font handle, char
/// code): the same glyph recurs across a page's items, and identifying it hashes
/// its outline.
struct GlyphRecovery<'r> {
    resolver: &'r dyn GlyphResolver,
    answers: std::collections::HashMap<(usize, u32), Option<u32>>,
}

impl<'r> GlyphRecovery<'r> {
    fn new(resolver: &'r dyn GlyphResolver) -> Self {
        Self {
            resolver,
            answers: std::collections::HashMap::new(),
        }
    }

    /// The recovered codepoint for `char_code` of `font`: the Type3 glyph name
    /// first, then the resolver on the outline. `None` when neither knows it.
    fn get(&mut self, font: &Font, char_code: u32) -> Option<u32> {
        let resolver = self.resolver;
        *self
            .answers
            .entry((font.handle() as usize, char_code))
            .or_insert_with(|| {
                (font.font_type() == FontType::Type3)
                    .then(|| type3_glyph_codepoint(font, char_code))
                    .flatten()
                    .or_else(|| identify_glyph(resolver, font, char_code))
            })
    }
}

/// Two letters of which exactly one is ASCII. An outline cannot tell a Latin
/// capital from its Greek or Cyrillic twin (A and Alpha are drawn alike), so an
/// outline database may answer either, and such a pair proves nothing about the
/// map.
fn is_cross_script_letter(a: u32, b: u32) -> bool {
    match (char::from_u32(a), char::from_u32(b)) {
        (Some(a), Some(b)) => {
            a.is_alphabetic() && b.is_alphabetic() && a.is_ascii() != b.is_ascii()
        }
        _ => false,
    }
}

/// The font handle of a run's text object, which every glyph of the run shares.
fn run_font_key(run: &[Glyph]) -> Option<usize> {
    let obj = run.first()?.text_object?;
    unsafe { Font::from_text_object(obj) }.map(|font| font.handle() as usize)
}

/// A run is buggy on its own evidence when its font is embedded and one of its
/// real glyphs decodes to a control or private-use codepoint (0, unmapped, counts).
fn run_has_buggy_codepoint(run: &[Glyph]) -> bool {
    run.iter()
        .any(|glyph| !glyph.generated && is_buggy_codepoint(glyph.unicode))
}

/// The fonts of the page whose code-to-character map is contradicted by their
/// glyph outlines (see [`font_map_is_contradicted`]). Only embedded fonts that
/// already hold a run with a control or private-use decode are examined, and a
/// font already buggy by name needs no widening, so a page without such a run
/// costs nothing here.
fn contradicted_fonts(runs: &[Vec<Glyph>], recovery: &mut GlyphRecovery<'_>) -> Vec<usize> {
    let mut suspects: Vec<(usize, Font)> = Vec::new();
    for run in runs {
        let Some(font) = run
            .first()
            .and_then(|glyph| glyph.text_object)
            .and_then(|obj| unsafe { Font::from_text_object(obj) })
        else {
            continue;
        };
        let key = font.handle() as usize;
        if suspects.iter().any(|(k, _)| *k == key)
            || !font.is_embedded()
            || is_buggy_font(&font.base_name().unwrap_or_default(), font.font_type())
            || !run_has_buggy_codepoint(run)
        {
            continue;
        }
        suspects.push((key, font));
    }
    suspects
        .into_iter()
        .filter(|(key, font)| {
            // Each distinct code once: the map's printable answer for it, from any
            // run of the font on the page.
            let mut codes: Vec<(u32, u32)> = Vec::new();
            for run in runs.iter().filter(|run| run_font_key(run) == Some(*key)) {
                for glyph in run {
                    if glyph.generated
                        || is_buggy_codepoint(glyph.unicode)
                        || is_c_locale_space(glyph.unicode)
                        || codes.iter().any(|(code, _)| *code == glyph.char_code)
                    {
                        continue;
                    }
                    codes.push((glyph.char_code, glyph.unicode));
                }
            }
            let answers = codes
                .iter()
                .map(|&(code, mapped)| (mapped, recovery.get(font, code)));
            font_map_is_contradicted(answers)
        })
        .map(|(key, _)| key)
        .collect()
}

/// Least number of distinct codes whose recovered glyph differs from the map's
/// character before a font's map is called wrong.
const MIN_CONTRADICTED_CODES: usize = 3;

/// Whether a font's map is wrong, judged on its distinct printable codes as
/// `(map's codepoint, recovered codepoint)` pairs: at least
/// [`MIN_CONTRADICTED_CODES`] codes recover to another character, and they
/// outnumber the codes that agree two to one. Codes recovery cannot identify are
/// no evidence either way, and neither is a letter recovered in another script
/// (see [`is_cross_script_letter`]). A sound map that disagrees with recovery on
/// a few look-alike glyphs (quote and dash variants) keeps a clear majority of
/// agreeing codes; a shifted or scrambled map agrees on almost none.
fn font_map_is_contradicted(answers: impl Iterator<Item = (u32, Option<u32>)>) -> bool {
    let (mut agree, mut disagree) = (0usize, 0usize);
    for (mapped, recovered) in answers {
        match recovered {
            Some(recovered) if recovered == mapped => agree += 1,
            Some(recovered) if is_cross_script_letter(mapped, recovered) => {}
            Some(_) => disagree += 1,
            None => {}
        }
    }
    disagree >= MIN_CONTRADICTED_CODES && disagree >= 2 * agree
}

/// The per-glyph reads. A glyph whose loose box cannot be read falls back to its
/// strict box.
fn load_glyph(
    page: &Page,
    view_box: &RectF,
    page_rotation: i32,
    cv: &CharView<'_, '_>,
    index: i32,
) -> Glyph {
    let generated = cv.is_generated();
    let char_code = if generated { 0 } else { cv.char_code() };
    let unicode = if cv.has_unicode_map_error() {
        0
    } else {
        cv.unicode()
    };
    let strict = cv.strict_char_box().unwrap_or_default();
    let loose = cv.loose_char_box().unwrap_or(strict);
    // The baseline direction comes from the text matrix; pdfium's own char angle
    // is the fallback for a glyph whose matrix cannot be read.
    let angle = match cv.ch.matrix() {
        Some(matrix) => baseline_angle_radians(&matrix),
        None => f64::from(cv.ch.angle()),
    };
    Glyph {
        index,
        text_object: cv.text_object(),
        generated,
        char_code,
        unicode,
        loose: page.bounds_to_viewport(view_box, &loose),
        strict: page.bounds_to_viewport(view_box, &strict),
        angle: normalize_angle(angle, page_rotation),
    }
}

/// Counter-clockwise baseline rotation of a text matrix, in radians, from the
/// direction of its first column. Formed the way the C extractor forms it — the
/// degrees wrapped into `[0, 360)` first, then negated and converted — so an
/// angle a hair below zero lands a hair below 2π and rounds the same way.
fn baseline_angle_radians(m: &Matrix) -> f64 {
    use std::f64::consts::PI;
    let degrees = (f64::from(m.b).atan2(f64::from(m.a)) * 180.0 / PI + 360.0) % 360.0;
    -degrees * PI / 180.0
}

fn decompose_scale_single_precision_products(m: &pdfium::Matrix) -> (f32, f32) {
    let a = f64::from(m.a * m.a + m.b * m.b);
    let b = f64::from(m.a * m.c + m.b * m.d);
    let c = f64::from(m.c * m.a + m.d * m.b);
    let d = f64::from(m.c * m.c + m.d * m.d);
    let first = (a + d) / 2.0;
    let second = ((a + d) * (a + d) - 4.0 * (a * d - c * b)).sqrt() / 2.0;
    let sx = first + second;
    let sx = if sx.is_nan() { 1.0 } else { sx };
    let sy = first - second;
    let sy = if sy.is_nan() { 1.0 } else { sy };
    (sx.sqrt() as f32, sy.sqrt() as f32)
}

/// Fold the page's `/Rotate` into a counter-clockwise glyph angle and wrap it
/// into `[0, 2π)`. Computed in f64 so the wrap does not lose precision; the
/// final narrowing to f32 can round a value just under 2π up to it.
fn normalize_angle(angle_radians: f64, page_rotation: i32) -> f32 {
    use std::f64::consts::PI;
    let mut angle = angle_radians;
    match page_rotation {
        1 => angle -= 3.0 * PI / 2.0,
        2 => angle -= PI,
        3 => angle -= PI / 2.0,
        _ => {}
    }
    angle %= 2.0 * PI;
    if angle < 0.0 {
        angle += 2.0 * PI;
    }
    angle as f32
}

/// ASCII whitespace only — deliberately not Unicode `White_Space`, so non-breaking
/// and ideographic spaces stay inside their items.
fn is_c_locale_space(unicode: u32) -> bool {
    matches!(unicode, 0x09..=0x0D | 0x20)
}

fn pack_argb(color: pdfium::Color) -> u32 {
    u32::from_be_bytes([color.a, color.r, color.g, color.b])
}

/// Turn a run of glyphs into an item. `None` when a codepoint is not a Unicode
/// scalar value (see [`c_string_from_utf32`]).
fn build_item(
    page: &Page,
    text_page: &TextPage,
    view_box: &RectF,
    glyphs: &[Glyph],
    recovery: Option<&mut GlyphRecovery<'_>>,
    font_map_broken: bool,
    outline_cache: &mut OutlineSupportCache,
) -> Option<RawTextItem> {
    let first = glyphs.first()?;
    let first_char = text_page.char_at_unchecked(first.index);
    let font_size = first_char.font_size() as f32;
    let font = first
        .text_object
        .and_then(|obj| unsafe { Font::from_text_object(obj) });

    let mut bounds = first.loose;
    let mut text_width = 0.0f32;
    let mut font_name = String::new();
    let mut font_ascent = 0.0f32;
    let mut font_descent = 0.0f32;
    let mut font_is_embedded = false;
    let mut font_is_buggy = false;
    if let Some(font) = &font {
        font_name = font.base_name().unwrap_or_default();
        font_ascent = font.ascent(font_size).unwrap_or(0.0);
        font_descent = font.descent(font_size).unwrap_or(0.0);
        font_is_embedded = font.is_embedded();
        font_is_buggy = font_is_embedded && is_buggy_font(&font_name, font.font_type());
        if let Some(width) = font.glyph_width_from_char_code(first.char_code, font_size) {
            text_width += width;
        }
    }
    let font_height = if first.text_object.is_some() {
        let scale_y = first_char
            .matrix()
            .map(|matrix| decompose_scale_single_precision_products(&matrix).1)
            .unwrap_or(1.0);
        font_size * scale_y
    } else {
        0.0
    };

    for glyph in &glyphs[1..] {
        bounds.left = bounds.left.min(glyph.loose.left);
        bounds.top = bounds.top.min(glyph.loose.top);
        bounds.right = bounds.right.max(glyph.loose.right);
        bounds.bottom = bounds.bottom.max(glyph.loose.bottom);
        if let Some(font) = &font {
            let width = if glyph.generated {
                font.glyph_width(glyph.unicode, font_size)
            } else {
                font.glyph_width_from_char_code(glyph.char_code, font_size)
            };
            if let Some(width) = width {
                text_width += width;
            }
        }
    }
    // A control / private-use decode in an embedded font flags the whole item
    // (`unicode` is 0 for an unmapped glyph, so those count too), and so does a
    // font whose map the page's other items show wrong (see the module docs).
    if font_is_embedded && !font_is_buggy {
        font_is_buggy = font_map_broken || run_has_buggy_codepoint(glyphs);
    }

    let mut codepoints: Vec<u32> = glyphs.iter().map(|glyph| glyph.unicode).collect();
    if font_is_buggy && let (Some(font), Some(recovery)) = (&font, recovery) {
        for (glyph, codepoint) in glyphs.iter().zip(codepoints.iter_mut()) {
            if glyph.generated {
                continue;
            }
            *codepoint = recovery
                .get(font, glyph.char_code)
                .unwrap_or(u32::from(' '));
        }
    }
    let text = c_string_from_utf32(&codepoints)?;
    // Judged after recovery: a glyph's original codepoint can be missing or end up
    // replaced by a space.
    let grounding_bounds = grounding_bounds(glyphs, &codepoints, first.angle, |glyph, c, s| {
        glyph_outline_support(page, text_page, view_box, glyph, c, s, outline_cache)
    });
    let baseline_gap = if glyphs.len() == 1 && first.generated && first.unicode == u32::from(' ') {
        measure_baseline_gap(text_page, first.index)
    } else {
        None
    };

    let glyph_names = font
        .as_ref()
        .filter(|font| font.font_type() == FontType::Type3)
        .map(|font| {
            glyphs
                .iter()
                .map(|glyph| font.char_glyph_name(glyph.char_code).unwrap_or_default())
                .collect()
        });

    Some(RawTextItem {
        text,
        char_codes: glyphs.iter().map(|glyph| glyph.char_code).collect(),
        glyph_names,
        angle_radians: first.angle,
        text_width,
        x: bounds.left,
        y: bounds.top,
        width: bounds.right - bounds.left,
        height: bounds.bottom - bounds.top,
        grounding_bounds,
        baseline_gap,
        mcid: first_char.marked_content_id(),
        font_name,
        font_size,
        font_weight: first_char.font_weight(),
        font_height,
        font_ascent,
        font_descent,
        font_is_buggy,
        trailing_space_generated: glyphs.last().is_some_and(|glyph| glyph.generated),
        fill_color: first_char.fill_color().map(pack_argb),
        stroke_color: first_char.stroke_color().map(pack_argb),
    })
}

/// Extrema of a glyph's ink in an item's baseline frame, viewport units: `along`
/// is the projection onto the baseline direction, `across` onto its normal.
#[derive(Debug, Clone, Copy, PartialEq)]
struct BaselineSupport {
    min_along: f64,
    max_along: f64,
    min_across: f64,
    max_across: f64,
}

impl BaselineSupport {
    const EMPTY: Self = Self {
        min_along: f64::INFINITY,
        max_along: f64::NEG_INFINITY,
        min_across: f64::INFINITY,
        max_across: f64::NEG_INFINITY,
    };

    fn include(&mut self, along: f64, across: f64) {
        self.min_along = self.min_along.min(along);
        self.max_along = self.max_along.max(along);
        self.min_across = self.min_across.min(across);
        self.max_across = self.max_across.max(across);
    }

    fn translate(self, along: f64, across: f64) -> Self {
        Self {
            min_along: self.min_along + along,
            max_along: self.max_along + along,
            min_across: self.min_across + across,
            max_across: self.max_across + across,
        }
    }
}

/// One glyph outline's shape in the baseline frame, before the glyph's own origin
/// is added, with everything the shape depends on as its key.
struct OutlineShape {
    /// Address of the `FPDF_FONT` handle; pdfium hands the same handle back for
    /// the same font object while the page is loaded.
    font: usize,
    char_code: u32,
    size_bits: u32,
    matrix_bits: [u32; 4],
    cos_bits: u64,
    sin_bits: u64,
    support: BaselineSupport,
}

/// Per-page cache of outline shapes: 256 direct-mapped slots keyed by
/// `char_code % 256`, a collision recomputing the shape rather than evicting
/// anything, as in the C extractor. Bounds the memory held per page without
/// retaining font handles beyond it.
pub(crate) struct OutlineSupportCache {
    slots: Vec<Option<OutlineShape>>,
}

impl Default for OutlineSupportCache {
    fn default() -> Self {
        Self {
            slots: (0..256).map(|_| None).collect(),
        }
    }
}

/// The extrema of `glyph`'s outline in the item's baseline frame (`c`, `s` are
/// the cosine and sine of the item angle), or `None` when the glyph keeps its
/// strict box. `None` for cardinal, unsheared text, whose char box is already
/// axis-aligned (shear alone needs outlines even at a cardinal angle: judging the
/// baseline only made bounds jump when a sheared baseline crossed the threshold);
/// for stroked render modes, whose paint the fill outline does not describe; for
/// fonts other than Type1, TrueType and `Identity-H` CID (a Type3 outline may be
/// a substitute face rather than the drawn CharProc, and vertical CID placement is
/// not exposed by the glyph-path API); and for any geometry that cannot be read
/// or is degenerate.
///
/// Glyph paths are in em units scaled by the font size, so the shape is built
/// with the linear part of the text matrix at a fixed origin — projecting each
/// glyph's own origin made the 0.001 pt viewport rounding change an otherwise
/// identical shape — and each occurrence adds its own origin afterwards. Bézier
/// control points are included as a conservative hull of the curve. Point
/// products are formed in f32 before the projection, as the C extractor forms
/// them, so the two agree to the last digit.
fn glyph_outline_support(
    page: &Page,
    text_page: &TextPage,
    view_box: &RectF,
    glyph: &Glyph,
    c: f64,
    s: f64,
    cache: &mut OutlineSupportCache,
) -> Option<BaselineSupport> {
    let font = unsafe { Font::from_text_object(glyph.text_object?) }?;
    let ch = text_page.char_at_unchecked(glyph.index);
    let matrix = ch.matrix()?;
    let linear = [matrix.a, matrix.b, matrix.c, matrix.d];
    if !linear.iter().all(|v| v.is_finite()) {
        return None;
    }
    let axes = f64::from(matrix.a).hypot(f64::from(matrix.b))
        * f64::from(matrix.c).hypot(f64::from(matrix.d));
    let shear = f64::from(matrix.a * matrix.c + matrix.b * matrix.d);
    if (c * s).abs() <= 1e-6 && shear.abs() <= 1e-6 * axes {
        return None;
    }
    // Fill, invisible, fill-clip and clip modes paint (or clip to) the outline
    // itself; unknown and every stroking mode fall back to the char box.
    match ch.text_render_mode() {
        Some(0 | 3 | 4 | 7) => {}
        _ => return None,
    }
    match font.font_type() {
        FontType::Type1 | FontType::TrueType => {}
        FontType::CidType0 | FontType::CidType2 => {
            if font.encoding().as_deref() != Some("Identity-H") {
                return None;
            }
        }
        _ => return None,
    }
    let size = ch.font_size() as f32;
    if !size.is_finite() || size <= 0.0 {
        return None;
    }

    let font_address = font.handle() as usize;
    let matrix_bits = linear.map(f32::to_bits);
    let slot = &mut cache.slots[(glyph.char_code % 256) as usize];
    let cached = slot.as_ref().filter(|shape| {
        shape.font == font_address
            && shape.char_code == glyph.char_code
            && shape.size_bits == size.to_bits()
            && shape.matrix_bits == matrix_bits
            && shape.cos_bits == c.to_bits()
            && shape.sin_bits == s.to_bits()
    });
    let support = match cached {
        Some(shape) => shape.support,
        None => {
            let segments = font.glyph_path_segments(glyph.char_code, size)?;
            if segments.is_empty() {
                return None;
            }
            let (vx, vy) = page.page_to_viewport(view_box, 0.0, 0.0);
            let (ux, uy) = page.page_to_viewport(view_box, size * matrix.a, size * matrix.b);
            let (wx, wy) = page.page_to_viewport(view_box, size * matrix.c, size * matrix.d);
            let mut support = BaselineSupport::EMPTY;
            for &(_, x, y) in &segments {
                if !x.is_finite() || !y.is_finite() {
                    return None;
                }
                let px = f64::from(x * (ux - vx) + y * (wx - vx));
                let py = f64::from(x * (uy - vy) + y * (wy - vy));
                support.include(px * c + py * s, py * c - px * s);
            }
            if !(support.min_along < support.max_along && support.min_across < support.max_across) {
                return None;
            }
            *slot = Some(OutlineShape {
                font: font_address,
                char_code: glyph.char_code,
                size_bits: size.to_bits(),
                matrix_bits,
                cos_bits: c.to_bits(),
                sin_bits: s.to_bits(),
                support,
            });
            support
        }
    };

    // Only the shape is shared; each occurrence keeps its own advance and rise.
    let (ox, oy) = ch.origin()?;
    if !ox.is_finite() || !oy.is_finite() {
        return None;
    }
    let (x, y) = page.page_to_viewport(view_box, ox as f32, oy as f32);
    let (x, y) = (f64::from(x), f64::from(y));
    Some(support.translate(x * c + y * s, y * c - x * s))
}

/// Tight rotated box of the item's real glyphs (see the module docs). Each
/// qualifying glyph's support — its outline extrema from `outline` when that
/// returns one, otherwise the four corners of its strict box — is projected onto
/// the baseline direction (`along`) and its normal (`across`); the extrema over all
/// glyphs give the box, which is then rotated back to viewport axes at its centre.
/// Everything in f64; the coordinates narrow to f32 at the end.
fn grounding_bounds(
    glyphs: &[Glyph],
    codepoints: &[u32],
    angle_radians: f32,
    mut outline: impl FnMut(&Glyph, f64, f64) -> Option<BaselineSupport>,
) -> Option<RectF> {
    let (c, s) = (
        f64::from(angle_radians).cos(),
        f64::from(angle_radians).sin(),
    );
    let (mut min_along, mut max_along) = (f64::INFINITY, f64::NEG_INFINITY);
    let (mut min_across, mut max_across) = (f64::INFINITY, f64::NEG_INFINITY);
    for (glyph, &codepoint) in glyphs.iter().zip(codepoints) {
        let b = glyph.strict;
        if glyph.generated
            || codepoint == 0
            || is_c_locale_space(codepoint)
            || !(b.left.is_finite()
                && b.right.is_finite()
                && b.top.is_finite()
                && b.bottom.is_finite())
            || b.right <= b.left
            || b.bottom <= b.top
        {
            continue;
        }
        if let Some(support) = outline(glyph, c, s) {
            min_along = min_along.min(support.min_along);
            max_along = max_along.max(support.max_along);
            min_across = min_across.min(support.min_across);
            max_across = max_across.max(support.max_across);
            continue;
        }
        for corner in 0..4 {
            let x = f64::from(if corner & 1 != 0 { b.right } else { b.left });
            let y = f64::from(if corner & 2 != 0 { b.bottom } else { b.top });
            let along = x * c + y * s;
            let across = y * c - x * s;
            min_along = min_along.min(along);
            max_along = max_along.max(along);
            min_across = min_across.min(across);
            max_across = max_across.max(across);
        }
    }
    if min_along > max_along {
        return None;
    }
    let along = (min_along + max_along) / 2.0;
    let across = (min_across + max_across) / 2.0;
    let cx = along * c - across * s;
    let cy = along * s + across * c;
    let w = max_along - min_along;
    let h = max_across - min_across;
    Some(RectF {
        left: (cx - w / 2.0) as f32,
        right: (cx + w / 2.0) as f32,
        top: (cy - h / 2.0) as f32,
        bottom: (cy + h / 2.0) as f32,
    })
}

/// What one neighbour of a generated space contributes to the gap measurement.
struct GapNeighbour {
    font: Font,
    char_code: u32,
    matrix: Matrix,
    /// `hypot(a, b)` of the matrix: the baseline scale.
    scale: f64,
    x: f64,
    y: f64,
}

impl GapNeighbour {
    /// `None` when the glyph is generated, unmapped, whitespace, or in a font
    /// whose advances do not describe a horizontal baseline (only Type1, TrueType
    /// and CID fonts with `Identity-H` qualify), or when its geometry is unreadable
    /// or degenerate.
    fn read(text_page: &TextPage, index: i32) -> Option<Self> {
        let ch = text_page.char_at_unchecked(index);
        let codepoint = ch.unicode();
        if ch.is_generated() || codepoint == 0 || is_c_locale_space(codepoint) {
            return None;
        }
        let font = unsafe { Font::from_text_object(ch.text_object()?) }?;
        match font.font_type() {
            FontType::Type1 | FontType::TrueType => {}
            FontType::CidType0 | FontType::CidType2 => {
                if font.encoding().as_deref() != Some("Identity-H") {
                    return None;
                }
            }
            _ => return None,
        }
        let matrix = ch.matrix()?;
        let (x, y) = ch.origin()?;
        let scale = f64::from(matrix.a).hypot(f64::from(matrix.b));
        if !scale.is_finite() || scale <= 0.0 || !x.is_finite() || !y.is_finite() {
            return None;
        }
        Some(Self {
            font,
            char_code: ch.char_code(),
            matrix,
            scale,
            x,
            y,
        })
    }
}

/// The advance gap across the generated space at `index` (see the module docs):
/// the displacement from the previous glyph's origin to the next one, projected
/// onto the previous glyph's baseline, minus the previous glyph's advance. Page
/// points, f64 as the C extractor computes it.
fn measure_baseline_gap(text_page: &TextPage, index: i32) -> Option<f64> {
    let (previous, next) = (index - 1, index + 1);
    if previous < 0 || next >= text_page.char_count() {
        return None;
    }
    let p = GapNeighbour::read(text_page, previous)?;
    let n = GapNeighbour::read(text_page, next)?;
    let same_direction = (f64::from(p.matrix.a) / p.scale - f64::from(n.matrix.a) / n.scale).abs()
        < 1e-6
        && (f64::from(p.matrix.b) / p.scale - f64::from(n.matrix.b) / n.scale).abs() < 1e-6;
    if !same_direction {
        return None;
    }
    let size = text_page.char_at_unchecked(previous).font_size() as f32;
    let width = p.font.glyph_width_from_char_code(p.char_code, size)?;
    let displacement =
        ((n.x - p.x) * f64::from(p.matrix.a) + (n.y - p.y) * f64::from(p.matrix.b)) / p.scale;
    let gap = displacement - f64::from(width) * p.scale;
    // Backwards origins do not measure a forward separator; overlapping forward
    // advances still supply their authoritative zero or negative gap.
    (displacement >= 0.0 && size.is_finite() && width.is_finite() && gap.is_finite()).then_some(gap)
}

/// Append the text that page `page_index`'s visible form-widget appearances paint,
/// which [`extract_raw_text_items`] cannot see: pdfium's text API only reports
/// appearance glyphs once they are flattened into page content. The appearances
/// are flattened on a private copy of the page ([`Document::widget_appearance_copy`])
/// and extracted with the same rules, then appended to `items` — except a widget
/// item that repeats an existing item's text at the same geometry, which some
/// producers emit in both places. Appended items carry no marked-content id: an
/// appearance's MCIDs do not belong to the page's structure tree.
///
/// Returns `false` when the copy could not be built or read; `items` is then
/// unchanged and the caller keeps the page text it has. A page without such a
/// widget is a no-op `true`.
pub fn append_raw_widget_text_items(
    doc: &Document<'_>,
    page: &Page<'_, '_>,
    page_index: i32,
    glyph_resolver: Option<&dyn GlyphResolver>,
    items: &mut Vec<RawTextItem>,
) -> bool {
    if !page.has_form_widget_text() {
        return true;
    }
    let Some(copy) = doc.widget_appearance_copy(page_index) else {
        return false;
    };
    let Ok(copy_page) = copy.page(0) else {
        return false;
    };
    let Some(view_box) = copy_page.view_box() else {
        return false;
    };
    let Ok(text_page) = copy_page.text() else {
        return false;
    };
    let widgets = extract_raw_text_items(&copy_page, &text_page, &view_box, glyph_resolver);
    let original_count = items.len();
    for mut item in widgets {
        if items[..original_count]
            .iter()
            .any(|existing| same_text_item(&item, existing))
        {
            continue;
        }
        item.mcid = None;
        items.push(item);
    }
    true
}

/// Two items are the same when their text agrees up to trailing ASCII whitespace
/// and is non-empty, and their loose boxes and angles coincide. Equal text
/// elsewhere on the page is a distinct item.
fn same_text_item(a: &RawTextItem, b: &RawTextItem) -> bool {
    let a_text = a.text.trim_end_matches(is_c_locale_space_char);
    let b_text = b.text.trim_end_matches(is_c_locale_space_char);
    !a_text.is_empty()
        && a_text == b_text
        && (a.x - b.x).abs() < 0.01
        && (a.y - b.y).abs() < 0.01
        && ((a.x + a.width) - (b.x + b.width)).abs() < 0.01
        && ((a.y + a.height) - (b.y + b.height)).abs() < 0.01
        && (a.angle_radians - b.angle_radians).abs() < 0.001
}

fn is_c_locale_space_char(c: char) -> bool {
    is_c_locale_space(u32::from(c))
}

/// The codepoint a Type3 font's own `/Encoding` `/Differences` name resolves to.
/// A Type3 font has no font program, so this name is the only statement the
/// document makes about what the glyph means.
fn type3_glyph_codepoint(font: &Font, char_code: u32) -> Option<u32> {
    let name = font.char_glyph_name(char_code)?;
    resolve_glyph_name_codepoint(&name)
}

/// The resolver's answer for the glyph outline. A glyph with no outline
/// (whitespace, non-rendered) is unidentified.
fn identify_glyph(resolver: &dyn GlyphResolver, font: &Font, char_code: u32) -> Option<u32> {
    let segments = font.glyph_path_segments(char_code, crate::GLYPH_RESOLVER_FONT_SIZE)?;
    resolver.resolve_codepoint(&segments)
}

/// Build the item text from raw codepoints with C-string semantics: `None` when
/// any value is not a Unicode scalar value (the whole item is dropped), otherwise
/// the text up to the first 0 — a 0 is an unmapped glyph, and it terminates the
/// string rather than being replaced.
fn c_string_from_utf32(codepoints: &[u32]) -> Option<String> {
    let mut chars = Vec::with_capacity(codepoints.len());
    for &codepoint in codepoints {
        match char::from_u32(codepoint) {
            Some(c) => chars.push(c),
            None => return None,
        }
    }
    Some(chars.into_iter().take_while(|&c| c != '\0').collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(list: &[(char, Option<char>)]) -> impl Iterator<Item = (u32, Option<u32>)> + '_ {
        list.iter()
            .map(|&(mapped, recovered)| (u32::from(mapped), recovered.map(u32::from)))
    }

    #[test]
    fn map_is_contradicted_by_a_two_to_one_majority_of_three_or_more() {
        // Every code recovered as the character a constant offset away.
        let shifted = [
            ('*', Some('2')),
            ('(', Some('0')),
            (',', Some('4')),
            ('A', Some('I')),
        ];
        assert!(font_map_is_contradicted(pairs(&shifted)));
        // Two contradictions are too few to judge.
        assert!(!font_map_is_contradicted(pairs(&shifted[..2])));
        // Unidentified codes are no evidence.
        let unknown = [('*', Some('2')), ('(', Some('0')), ('x', None), ('y', None)];
        assert!(!font_map_is_contradicted(pairs(&unknown)));
        // A sound map with a few variant glyphs keeps its majority.
        let sound = [
            ('a', Some('a')),
            ('b', Some('b')),
            ('c', Some('c')),
            ('\'', Some('\u{2019}')),
            ('-', Some('\u{2013}')),
            ('"', Some('\u{201D}')),
        ];
        assert!(!font_map_is_contradicted(pairs(&sound)));
    }

    #[test]
    fn letters_recovered_in_another_script_are_no_evidence() {
        // Latin capitals answered with their Greek and Cyrillic twins.
        let twins = [
            ('A', Some('\u{391}')),
            ('N', Some('\u{39D}')),
            ('E', Some('\u{415}')),
            ('O', Some('\u{41E}')),
        ];
        assert!(!font_map_is_contradicted(pairs(&twins)));
        assert!(is_cross_script_letter(u32::from('P'), 0x420));
        assert!(!is_cross_script_letter(u32::from('P'), u32::from('R')));
        assert!(!is_cross_script_letter(u32::from('1'), 0x391));
    }

    #[test]
    fn c_locale_space_is_ascii_only() {
        for space in [0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x20] {
            assert!(is_c_locale_space(space), "{space:#x}");
        }
        for not_space in [
            0x00, 0x08, 0x0E, 0x1F, 0x21, 0x85, 0xA0, 0x2003, 0x202F, 0x3000,
        ] {
            assert!(!is_c_locale_space(not_space), "{not_space:#x}");
        }
    }

    #[test]
    fn utf32_conversion_keeps_c_string_semantics() {
        assert_eq!(
            c_string_from_utf32(&[0x48, 0x69, 0x20]).as_deref(),
            Some("Hi ")
        );
        // A 0 terminates the string; what follows is dropped, not the item.
        assert_eq!(
            c_string_from_utf32(&[0x48, 0x00, 0x69]).as_deref(),
            Some("H")
        );
        assert_eq!(c_string_from_utf32(&[0x00]).as_deref(), Some(""));
        assert_eq!(c_string_from_utf32(&[]).as_deref(), Some(""));
        // A surrogate or out-of-range value drops the item, even after a 0.
        assert_eq!(c_string_from_utf32(&[0x48, 0xD800]), None);
        assert_eq!(c_string_from_utf32(&[0x48, 0x00, 0x110000]), None);
        // Noncharacters are valid scalars and pass through.
        assert_eq!(c_string_from_utf32(&[0xFFFE]).as_deref(), Some("\u{FFFE}"));
    }

    #[test]
    fn angle_folds_page_rotation_and_wraps() {
        use std::f64::consts::PI;
        let close = |a: f32, b: f64| (f64::from(a) - b).abs() < 1e-5;
        assert!(close(normalize_angle(0.0, 0), 0.0));
        assert!(close(normalize_angle(PI / 2.0, 0), PI / 2.0));
        // /Rotate 90: the angle minus 3π/2, wrapped up into [0, 2π).
        assert!(close(normalize_angle(0.0, 1), PI / 2.0));
        assert!(close(normalize_angle(0.0, 2), PI));
        assert!(close(normalize_angle(0.0, 3), 3.0 * PI / 2.0));
        assert!(close(normalize_angle(-0.5, 0), 2.0 * PI - 0.5));
        assert!(close(normalize_angle(2.0 * PI + 0.25, 0), 0.25));
    }

    fn matrix(a: f32, b: f32, c: f32, d: f32) -> Matrix {
        Matrix {
            a,
            b,
            c,
            d,
            e: 0.0,
            f: 0.0,
        }
    }

    #[test]
    fn baseline_angle_ignores_shear_and_keeps_the_c_wrap() {
        use std::f64::consts::PI;
        let close = |a: f64, b: f64| (a - b).abs() < 1e-9;
        // Upright and italic (sheared second column) share a baseline.
        assert!(close(
            baseline_angle_radians(&matrix(12.0, 0.0, 0.0, 12.0)),
            0.0
        ));
        assert!(close(
            baseline_angle_radians(&matrix(12.0, 0.0, 3.0, 12.0)),
            0.0
        ));
        // A counter-clockwise quarter turn.
        assert!(close(
            baseline_angle_radians(&matrix(0.0, 12.0, -12.0, 0.0)),
            -PI / 2.0
        ));
        // A baseline a hair below horizontal wraps to just under 360°, which the
        // fold brings back to a hair above zero.
        let radians = baseline_angle_radians(&matrix(12.0, -1e-6, 0.0, 12.0));
        assert!(radians < -2.0 * PI + 1e-6 && radians > -2.0 * PI);
        assert!(normalize_angle(radians, 0) > 0.0 && normalize_angle(radians, 0) < 1e-6);
        // On a /Rotate 90 page a baseline a hair past the quarter turn folds to a
        // hair under 2π, and the narrowing to f32 leaves it there — a consumer
        // converting to degrees at three decimals reads 360, as with the C extractor.
        let past_quarter = baseline_angle_radians(&matrix(-1e-6, 12.0, -12.0, 0.0));
        let folded = normalize_angle(past_quarter, 1);
        assert!(f64::from(folded) > 2.0 * PI - 1e-6 && f64::from(folded) <= 2.0 * PI + 1e-6);
    }

    fn glyph(strict: RectF, generated: bool) -> Glyph {
        Glyph {
            index: 0,
            text_object: None,
            generated,
            char_code: 0,
            unicode: 0,
            loose: strict,
            strict,
            angle: 0.0,
        }
    }

    fn rect(left: f32, top: f32, right: f32, bottom: f32) -> RectF {
        RectF {
            left,
            top,
            right,
            bottom,
        }
    }

    #[test]
    fn grounding_bounds_skip_spaces_and_generated_glyphs() {
        let glyphs = [
            glyph(rect(10.0, 20.0, 15.0, 30.0), false),
            glyph(rect(15.0, 20.0, 20.0, 30.0), false),
            glyph(rect(20.0, 20.0, 25.0, 30.0), true),
            glyph(rect(25.0, 20.0, 30.0, 30.0), false),
            glyph(rect(30.0, 20.0, 35.0, 30.0), false),
            glyph(rect(35.0, 0.0, 40.0, 40.0), false),
        ];
        // Space glyph, a generated glyph and an unmapped (0) glyph all drop out.
        let codepoints = [b'H', b'i', b' ', b' ', b'!', 0].map(u32::from);
        let bounds = grounding_bounds(&glyphs, &codepoints, 0.0, |_, _, _| None).unwrap();
        assert_eq!(bounds, rect(10.0, 20.0, 35.0, 30.0));
        assert!(grounding_bounds(&glyphs[2..3], &codepoints[2..3], 0.0, |_, _, _| None).is_none());
        // A degenerate strict box contributes nothing.
        let flat = [glyph(rect(1.0, 1.0, 1.0, 5.0), false)];
        assert!(grounding_bounds(&flat, &[u32::from(b'x')], 0.0, |_, _, _| None).is_none());
    }

    #[test]
    fn grounding_bounds_rotate_about_the_centre() {
        use std::f32::consts::FRAC_PI_2;
        // One 10×4 box at a quarter turn: along the baseline it measures the box's
        // height, across it the width, centred where the box is centred.
        let glyphs = [glyph(rect(0.0, 0.0, 10.0, 4.0), false)];
        let bounds =
            grounding_bounds(&glyphs, &[u32::from(b'x')], FRAC_PI_2, |_, _, _| None).unwrap();
        let close = |a: f32, b: f32| (a - b).abs() < 1e-5;
        assert!(close(bounds.left, 3.0) && close(bounds.right, 7.0));
        assert!(close(bounds.top, -3.0) && close(bounds.bottom, 7.0));
    }

    fn item(text: &str, x: f32, y: f32, width: f32, height: f32, angle: f32) -> RawTextItem {
        RawTextItem {
            text: text.into(),
            char_codes: Vec::new(),
            glyph_names: None,
            angle_radians: angle,
            text_width: 0.0,
            x,
            y,
            width,
            height,
            grounding_bounds: None,
            baseline_gap: None,
            mcid: Some(3),
            font_name: String::new(),
            font_size: 0.0,
            font_weight: 0,
            font_height: 0.0,
            font_ascent: 0.0,
            font_descent: 0.0,
            font_is_buggy: false,
            trailing_space_generated: false,
            fill_color: None,
            stroke_color: None,
        }
    }

    #[test]
    fn same_text_item_needs_equal_text_and_geometry() {
        let a = item("Total ", 10.0, 20.0, 30.0, 8.0, 0.0);
        assert!(same_text_item(
            &a,
            &item("Total", 10.005, 20.0, 29.995, 8.0, 0.0005)
        ));
        assert!(!same_text_item(
            &a,
            &item("Total", 10.02, 20.0, 29.98, 8.0, 0.0)
        ));
        assert!(!same_text_item(
            &a,
            &item("Total", 10.0, 20.0, 30.0, 8.0, 0.002)
        ));
        assert!(!same_text_item(
            &a,
            &item("Totals", 10.0, 20.0, 30.0, 8.0, 0.0)
        ));
        // Whitespace-only text never matches, so blank widgets are always kept.
        let blank = item("  ", 10.0, 20.0, 30.0, 8.0, 0.0);
        assert!(!same_text_item(&blank, &blank));
    }

    #[test]
    fn argb_packing_matches_fpdf_argb() {
        let color = pdfium::Color {
            r: 0x12,
            g: 0x34,
            b: 0x56,
            a: 0xFF,
        };
        assert_eq!(pack_argb(color), 0xFF12_3456);
    }
}
