use crate::types::{Anchor, ProjectedLine};

use super::inline::{SpanStyle, line_all_bold, line_uniform_style, render_line_inline};

/// Multiplier on line height used as the paragraph-break threshold.
const PARAGRAPH_GAP_MULTIPLIER: f32 = 1.5;

/// Tolerance for treating two font sizes as "the same" when grouping
/// paragraph lines, when at least one side falls back to a bbox-height
/// estimate. Set above descender-driven jitter (~1pt) but below a
/// 12→14pt section-heading step that often follows a smaller caption
/// line with the same bold style.
const FONT_SIZE_PARAGRAPH_TOLERANCE: f32 = 1.5;
/// Tolerance when both lines report real Tf-set sizes. Tight, so a
/// 12pt-bold caption line directly above a 14pt-bold section heading
/// doesn't accidentally merge them into a single paragraph that swallows
/// the heading.
const FONT_SIZE_PARAGRAPH_TOLERANCE_REAL: f32 = 0.5;

/// Pick the right size-equality tolerance depending on whether either
/// side has an estimated (jitter-prone) font size.
fn font_size_paragraph_tolerance(prev: &ProjectedLine, cur: &ProjectedLine) -> f32 {
    if prev.font_size_is_estimated || cur.font_size_is_estimated {
        FONT_SIZE_PARAGRAPH_TOLERANCE
    } else {
        FONT_SIZE_PARAGRAPH_TOLERANCE_REAL
    }
}

/// Tolerance in points for treating two indent positions as "the same column".
const INDENT_TOLERANCE: f32 = 6.0;

/// Collapse runs of whitespace into single spaces. The projected text from
/// `projection.rs` pads with column-alignment spaces (e.g. `for    instance`)
/// which look fine as a layout grid but are wrong for prose.
pub(super) fn collapse_whitespace(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_space = false;
    for c in s.chars() {
        if c.is_whitespace() {
            if !prev_space && !out.is_empty() {
                out.push(' ');
            }
            prev_space = true;
        } else {
            out.push(c);
            prev_space = false;
        }
    }
    if out.ends_with(' ') {
        out.pop();
    }
    out
}

/// True when `text` ends with a mid-word soft hyphen — a trailing `-` whose
/// preceding character is a letter. Distinguishes a wrapped word (`architec-`)
/// from a dangling list marker (`-`) or a numeric range (`5-`), neither of
/// which should be rejoined. Unicode-aware, so accented stems (`café-`)
/// qualify. Trims trailing whitespace first.
///
/// Single source of truth for the "ends open on a hyphen" test; the heading
/// continuation guard, region stitching, and block joining all key off this.
/// (Note: `stitch_regions` has an unrelated local `ends_open` that tests
/// sentence-terminal punctuation — different concept, hence the distinct name.)
pub(super) fn ends_hyphenated(text: &str) -> bool {
    let t = text.trim_end();
    t.ends_with('-') && t.chars().rev().nth(1).is_some_and(|c| c.is_alphabetic())
}

/// True when `text` ends a sentence: its last non-space character (ignoring a
/// trailing closing quote / bracket) is `.`, `!`, or `?`. The heading
/// lowercase-continuation guard uses the negation — a previous line that does
/// *not* end a sentence is an open clause that a following lowercase line
/// continues, even when its left-edge indent defeats `continues_paragraph`
/// (centered short tail lines like "…journalistic or" → "scholarly works.").
pub(super) fn ends_sentence_final(text: &str) -> bool {
    let t = text
        .trim_end()
        .trim_end_matches(['"', '\'', ')', ']', '»', '”', '’']);
    t.chars()
        .next_back()
        .is_some_and(|c| matches!(c, '.' | '!' | '?'))
}

/// True when `prev` ends with a soft hyphen (see [`ends_hyphenated`]) and
/// `next` begins with a lowercase letter — the signal that a single word was
/// split across a line / block / region break and should be rejoined with the
/// hyphen dropped and no separator (`architec-` + `ture` → `architecture`). A
/// capitalized continuation (`well-` + `Known`) is left as a real compound.
pub(super) fn is_soft_hyphen_break(prev: &str, next: &str) -> bool {
    ends_hyphenated(prev)
        && next
            .trim_start()
            .chars()
            .next()
            .is_some_and(|c| c.is_lowercase())
}

/// Append `to_append` onto `prev`, de-hyphenating across the boundary. When
/// the boundary is a soft hyphen break (see [`is_soft_hyphen_break`], tested
/// against `check` — the plain text of the continuation), the trailing hyphen
/// is dropped and the text concatenated directly (`co-` + `operate` →
/// `cooperate`); otherwise the join is a single space.
///
/// `check` and `to_append` are separate so a caller tracking a styled `inline`
/// representation can test the condition against the raw text while appending
/// the inline-rendered chunk. Plain-text callers pass the same string twice.
pub(super) fn dehyphenate_join(prev: &mut String, check: &str, to_append: &str) {
    if check.is_empty() {
        return;
    }
    if prev.is_empty() {
        prev.push_str(to_append);
        return;
    }
    if is_soft_hyphen_break(prev, check) {
        while prev.ends_with(|c: char| c.is_whitespace()) {
            prev.pop();
        }
        prev.pop(); // the soft hyphen
        prev.push_str(to_append.trim_start());
    } else {
        prev.push(' ');
        prev.push_str(to_append);
    }
}

/// Decide whether `cur` is a wrapped continuation of a heading line `prev`.
/// Looser than `continues_paragraph`: drops the indent-shift check, because
/// a centered wrapped heading has shifting left edges by line (the second
/// line is shorter, so its left edge is further right than the first), and
/// the projection's anchor classifier often labels short heading lines as
/// `Anchor::Left` rather than `Anchor::Center` since they don't anchor to a
/// column-center cluster. Without this relaxation, multi-line wrapped
/// headings like `# Author's Note to the` + `# 2021 Edition` fail the
/// merge check and emit as two separate `#` blocks.
pub(super) fn continues_heading(prev: &ProjectedLine, cur: &ProjectedLine) -> bool {
    let centered_mismatch = (prev.anchor == Anchor::Center) ^ (cur.anchor == Anchor::Center);
    if centered_mismatch {
        return false;
    }
    if (prev.dominant_font_size - cur.dominant_font_size).abs()
        > font_size_paragraph_tolerance(prev, cur)
    {
        return false;
    }
    if let (Some(p), Some(c)) = (line_uniform_style(prev), line_uniform_style(cur))
        && p.bold != c.bold
    {
        return false;
    }
    if line_all_bold(prev) != line_all_bold(cur) {
        return false;
    }
    if prev.region_path != cur.region_path {
        return false;
    }
    let prev_bottom = prev.bbox.y + prev.bbox.height;
    let gap = cur.bbox.y - prev_bottom;
    let line_height = prev.bbox.height.max(cur.bbox.height).max(1.0);
    gap <= line_height * PARAGRAPH_GAP_MULTIPLIER
}

/// Decide whether `cur` continues the paragraph started by `prev`.
pub(super) fn continues_paragraph(prev: &ProjectedLine, cur: &ProjectedLine) -> bool {
    paragraph_flow(prev, cur, false)
}

/// List-item continuation: like `continues_paragraph`, but tolerant of a
/// hanging indent. A wrapped list-item body commonly aligns under the marker's
/// *text* — i.e. indented to the right of the marker line — which
/// `continues_paragraph` would read as a new indented block.
pub(super) fn continues_list_item(prev: &ProjectedLine, cur: &ProjectedLine) -> bool {
    paragraph_flow(prev, cur, true)
}

fn paragraph_flow(prev: &ProjectedLine, cur: &ProjectedLine, allow_hanging_indent: bool) -> bool {
    // Anchor only signals a paragraph break when one of the lines is clearly
    // centered while the other isn't — justified prose routinely alternates
    // between Left / Right / Floating dominant anchors as line widths flex,
    // and treating those as paragraph breaks shreds normal text.
    let centered_mismatch = (prev.anchor == Anchor::Center) ^ (cur.anchor == Anchor::Center);
    if centered_mismatch {
        return false;
    }
    if (prev.dominant_font_size - cur.dominant_font_size).abs()
        > font_size_paragraph_tolerance(prev, cur)
    {
        return false;
    }
    // Uniform-bold ↔ non-bold transition is a paragraph break. Catches
    // body-size headings that share font size with the surrounding prose but
    // are emitted in a bold variant (e.g. Brill-Bold over Brill-Roman). Both
    // sides must have a uniform style for this to fire; mid-line emphasis
    // (None style) falls through to the gap/indent checks so prose with
    // mid-paragraph bold spans doesn't get fragmented.
    if let (Some(p), Some(c)) = (line_uniform_style(prev), line_uniform_style(cur))
        && p.bold != c.bold
    {
        return false;
    }
    // Same intent, but italic-tolerant: an all-bold line (which may mix bold
    // and bold-italic spans, so `line_uniform_style` yields None) adjacent to
    // a non-all-bold line is a paragraph break. Catches a bold section heading
    // hugging the body paragraph below it.
    if line_all_bold(prev) != line_all_bold(cur) {
        return false;
    }
    if prev.region_path != cur.region_path {
        // Cross-region continuation: the same paragraph can wrap from the
        // bottom of one column into the top of the next. Only bridge regions
        // when the previous line clearly breaks mid-sentence (no terminal
        // punctuation) AND the next line starts with a lowercase letter — a
        // strict signal that catches the column-wrap case while rejecting
        // unrelated paragraphs that happen to sit in adjacent leaves.
        let prev_trim = prev.text.trim_end();
        let ends_open = !prev_trim.ends_with(|c: char| {
            matches!(
                c,
                '.' | '!' | '?' | ':' | ';' | '”' | '"' | ')' | ']' | '。' | '』' | '」'
            )
        });
        let starts_lower = cur
            .text
            .trim_start()
            .chars()
            .next()
            .is_some_and(|c| c.is_lowercase());
        return ends_open && starts_lower;
    }
    if !allow_hanging_indent
        && (prev.indent_x - cur.indent_x).abs() > INDENT_TOLERANCE
        && cur.anchor == Anchor::Left
    {
        // Indent change on a left-aligned block usually means a new paragraph
        // (block-quote, list, indented passage, etc.). Allow first-line indent
        // by checking only when the *next* line shifts right relative to prev.
        if cur.indent_x > prev.indent_x + INDENT_TOLERANCE {
            return false;
        }
    }
    // Vertical gap check.
    let prev_bottom = prev.bbox.y + prev.bbox.height;
    let gap = cur.bbox.y - prev_bottom;
    let line_height = prev.bbox.height.max(cur.bbox.height).max(1.0);
    gap <= line_height * PARAGRAPH_GAP_MULTIPLIER
}

/// Paragraph accumulator state. We track two parallel representations of the
/// running paragraph text:
///
/// - `raw` — plain text (no emphasis markers). Used for the paragraph-uniform
///   shortcut: if every contributing line had the same uniform style, we wrap
///   the whole paragraph once with `wrap_emphasis(raw, …)` to avoid the
///   `**foo** **bar** **baz**` per-line noise.
/// - `inline` — per-line markdown with emphasis baked in via
///   `render_line_inline`. Used when the paragraph contains mid-line emphasis
///   shifts or lines with differing uniform styles.
///
/// `uniform` is `Some((bold, italic))` while every line so far has been a
/// uniformly-styled line sharing the same (bold, italic) flags, and `None` as
/// soon as that invariant breaks.
pub(super) struct ParaAccum {
    pub(super) raw: String,
    pub(super) inline: String,
    pub(super) last: ProjectedLine,
    pub(super) uniform: Option<(bool, bool)>,
    /// Union of every line folded into this paragraph, so the emitted block
    /// reports the whole band it covers rather than its last line.
    pub(super) bbox: Option<crate::types::Rect>,
    /// Source-item indices of every line folded into this paragraph, in
    /// insertion (reading) order — the returned-text-items contract of
    /// `ProjectedLine::span_item_indices`. Grown exactly where `bbox` grows.
    pub(super) item_indices: Vec<usize>,
}

/// Append `next_line` to a paragraph accumulator. Maintains both the `raw` and
/// `inline` text representations and updates the running `uniform` flag.
/// De-hyphenation runs on the `raw` boundary; the `inline` boundary mirrors it
/// when the trailing char is still a literal `-` (i.e. the hyphen sits outside
/// any emphasis wrap — the common case).
pub(super) fn append_to_paragraph(accum: &mut ParaAccum, next_line: &ProjectedLine) {
    let next_raw = collapse_whitespace(next_line.text.trim());
    if next_raw.is_empty() {
        return;
    }
    let next_inline = render_line_inline(next_line);
    // A struck line has no block-level flag, so it can't use the uniform
    // raw-text fast path — drop it to `None` to force the `inline` rendering.
    let next_uniform: Option<SpanStyle> = line_uniform_style(next_line).filter(|s| !s.strike);

    crate::types::Rect::extend(&mut accum.bbox, &next_line.bbox);
    accum
        .item_indices
        .extend(next_line.span_item_indices.iter().copied());

    if accum.raw.is_empty() {
        accum.raw.push_str(&next_raw);
        accum.inline.push_str(&next_inline);
        accum.uniform = next_uniform.map(|s| (s.bold, s.italic));
        accum.last = next_line.clone();
        return;
    }

    // Raw side de-hyphenates against its own boundary. The inline side keys off
    // the same raw lowercase test but checks *its own* trailing char: a hyphen
    // tucked inside an emphasis wrap ends in `*`/`` ` `` rather than `-`, so it
    // won't strip and falls through to a space join — exactly the prior
    // behavior, now via one helper.
    dehyphenate_join(&mut accum.raw, &next_raw, &next_raw);
    dehyphenate_join(&mut accum.inline, &next_raw, &next_inline);

    // Maintain the running uniform-style flag.
    accum.uniform = match (accum.uniform, next_uniform) {
        (Some(cur), Some(s)) if cur == (s.bold, s.italic) => Some(cur),
        _ => None,
    };
    accum.last = next_line.clone();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dehyphenate_join_only_strips_before_lowercase() {
        let mut s = String::from("co-");
        dehyphenate_join(&mut s, "operate", "operate");
        assert_eq!(s, "cooperate");

        let mut s = String::from("Vitamin-");
        dehyphenate_join(&mut s, "A", "A");
        assert_eq!(s, "Vitamin- A");
    }
}
