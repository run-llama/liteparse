use crate::projection::{is_bold_item, is_italic_item, is_mono_item, is_strike_item};
use crate::types::{ProjectedLine, TextItem};

use super::paragraphs::{collapse_whitespace, dehyphenate_join};

/// Per-span style flags used by the inline-emphasis renderer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct SpanStyle {
    pub(super) bold: bool,
    pub(super) italic: bool,
    pub(super) mono: bool,
    pub(super) strike: bool,
}

impl SpanStyle {
    pub(super) fn from_item(item: &TextItem) -> Self {
        SpanStyle {
            bold: is_bold_item(item),
            italic: is_italic_item(item),
            mono: is_mono_item(item),
            strike: is_strike_item(item),
        }
    }

    pub(super) fn is_plain(self) -> bool {
        !self.bold && !self.italic && !self.mono && !self.strike
    }
}

/// Escape characters that would otherwise be interpreted as markdown emphasis.
/// Deliberately narrow: only `*`, `_`, and backslash. Aggressive escaping
/// (`#`, `[`, backticks, etc.) breaks more output than it saves in practice.
/// An `_` between two letters or digits stays bare: CommonMark never reads an
/// intraword underscore as emphasis, and `snake_case`, `first_last@host.com`
/// or a URL path should survive verbatim (`mcc\_compliance` splits the token
/// for any consumer that doesn't unescape). Inline math (`$…$`, see
/// [`math_runs`]) passes through untouched.
pub fn escape_inline(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for (is_math, run) in math_runs(s) {
        if is_math {
            out.push_str(run);
            continue;
        }
        let chars: Vec<char> = run.chars().collect();
        for (i, &c) in chars.iter().enumerate() {
            let intraword = c == '_'
                && i > 0
                && chars[i - 1].is_alphanumeric()
                && chars.get(i + 1).is_some_and(|n| n.is_alphanumeric());
            match c {
                '\\' | '*' | '_' if !intraword => {
                    out.push('\\');
                    out.push(c);
                }
                _ => out.push(c),
            }
        }
    }
    out
}

/// Split `s` into runs, flagging the `$…$` spans a renderer reads as inline
/// math under pandoc's `tex_math_dollars` rule: the opening `$` is followed
/// by a non-space, the closing `$` follows a non-space and isn't followed by
/// a digit (so "$5 to $10" stays text) unless the span is plainly LaTeX
/// (`\`, `^`, `_` or `{`: "$\mathbb{R}$3" from a split script). The math
/// pass (`tex_math`) emits its LaTeX this way, and it must reach the output
/// unescaped.
pub(crate) fn math_runs(s: &str) -> Vec<(bool, &str)> {
    let b = s.as_bytes();
    let mut runs = Vec::new();
    let (mut lit, mut i) = (0, 0);
    while i < b.len() {
        let opens = b[i] == b'$'
            && (i == 0 || b[i - 1] != b'\\')
            && b.get(i + 1)
                .is_some_and(|c| !c.is_ascii_whitespace() && *c != b'$');
        let close = opens
            .then(|| {
                let j = i + 1 + s[i + 1..].find('$')?;
                let latex = s[i + 1..j].contains(['\\', '^', '_', '{']);
                let ok = !b[j - 1].is_ascii_whitespace()
                    && b[j - 1] != b'\\'
                    && (latex || !b.get(j + 1).is_some_and(u8::is_ascii_digit));
                ok.then_some(j)
            })
            .flatten();
        match close {
            Some(j) => {
                if lit < i {
                    runs.push((false, &s[lit..i]));
                }
                runs.push((true, &s[i..=j]));
                i = j + 1;
                lit = i;
            }
            None => i += 1,
        }
    }
    if lit < s.len() {
        runs.push((false, &s[lit..]));
    }
    runs
}

/// Escape `text` for use as the body of a span with `style`. Backslash escapes
/// are inert inside code spans per CommonMark, so mono text is passed through
/// verbatim.
fn style_body(text: &str, style: SpanStyle) -> String {
    if style.mono {
        text.to_string()
    } else {
        escape_inline(text)
    }
}

/// Render `text` as a single styled markdown span: style-aware escaping
/// (`style_body`) plus marker wrapping (`apply_style`). Always use this pair
/// through here — calling `escape_inline` + `apply_style` directly would
/// backslash-escape the body of a code span, which CommonMark renders
/// literally.
fn render_span(text: &str, style: SpanStyle) -> String {
    apply_style(&style_body(text, style), style)
}

/// Wrap `inner` in a markdown inline link to `url`. Uses the angle-bracket
/// destination form when the URL contains characters that would otherwise
/// terminate or break the `(url)` form (whitespace or parentheses).
pub fn apply_link(inner: &str, url: &str) -> String {
    if url.contains([' ', '\t', '(', ')']) {
        format!("[{}](<{}>)", inner, url)
    } else {
        format!("[{}]({})", inner, url)
    }
}

/// Wrap `inner` with the markdown markers for `style`. Mono wins over bold/italic:
/// inline code (`` `…` ``) doesn't compose with emphasis in CommonMark, so when
/// a span is mono we drop the `**/*` wrap. Bold + italic → `***…***`.
fn apply_style(inner: &str, style: SpanStyle) -> String {
    let styled = if style.mono {
        // The fence must be longer than the longest backtick run inside the
        // content (a matching-length interior run would close the span early),
        // with a space buffer so an edge backtick can't merge into the fence.
        // CommonMark strips one space from each end, so the content round-trips.
        let longest_run = {
            let mut longest = 0usize;
            let mut run = 0usize;
            for c in inner.chars() {
                run = if c == '`' { run + 1 } else { 0 };
                longest = longest.max(run);
            }
            longest
        };
        if longest_run == 0 {
            format!("`{}`", inner)
        } else {
            let fence = "`".repeat(longest_run + 1);
            format!("{fence} {inner} {fence}")
        }
    } else {
        match (style.bold, style.italic) {
            (true, true) => format!("***{}***", inner),
            (true, false) => format!("**{}**", inner),
            (false, true) => format!("*{}*", inner),
            (false, false) => inner.to_string(),
        }
    };
    // Strikethrough (GFM `~~…~~`) wraps outermost so it composes with emphasis
    // and inline code (`~~**foo**~~`, `~~`bar`~~`).
    if style.strike {
        format!("~~{}~~", styled)
    } else {
        styled
    }
}

/// Render a `ProjectedLine` to markdown with per-span emphasis. Adjacent
/// same-style spans are merged into a single emphasis run; whitespace between
/// spans is preserved as one space (the underlying projection output already
/// has the right inter-word spacing baked into span text).
///
/// Per-line shortcut: when every non-whitespace span shares the same style,
/// emit one outer wrap around the collapsed line text instead of run-by-run
/// (avoids `**foo** **bar** **baz**` noise on uniformly styled lines).
///
/// Falls back to `collapse_whitespace(line.text)` when the line has no usable
/// spans (e.g. OCR-only lines where TextItem styling isn't populated).
pub(super) fn render_line_inline(line: &ProjectedLine) -> String {
    let spans: Vec<&TextItem> = line
        .spans
        .iter()
        .filter(|s| !s.text.trim().is_empty())
        .collect();
    if spans.is_empty() {
        return collapse_whitespace(&line.text);
    }

    // Sort spans into reading order regardless of extraction order. Stable so
    // equal-x spans keep their original sequence. A right-to-left line reads
    // from the highest x down, matching the join `build_one_line` used for
    // `line.text` — the uniform-style shortcut below falls back on that string,
    // so the two orderings have to agree.
    let mut spans = spans;
    if line.rtl {
        spans.sort_by(|a, b| b.x.total_cmp(&a.x));
    } else {
        spans.sort_by(|a, b| a.x.total_cmp(&b.x));
    }

    let styles: Vec<SpanStyle> = spans.iter().map(|s| SpanStyle::from_item(s)).collect();
    let links: Vec<Option<&str>> = spans.iter().map(|s| s.link.as_deref()).collect();

    // Per-line shortcut — only when the whole line shares one style AND carries
    // no hyperlinks (a link must wrap only the spans it covers, so link lines
    // always take the per-group path below).
    let uniform = styles.iter().all(|s| *s == styles[0]) && links.iter().all(|l| l.is_none());
    if uniform {
        let joined = collapse_whitespace(&line.text);
        if joined.is_empty() {
            return joined;
        }
        return render_span(&joined, styles[0]);
    }

    // Group consecutive spans by style. Within a group, span texts join with
    // a single space (we lose intra-group spacing precision; acceptable).
    let mut out = String::new();
    let mut i = 0;
    while i < spans.len() {
        let style = styles[i];
        let link = links[i];
        let mut j = i + 1;
        while j < spans.len() && styles[j] == style && links[j] == link {
            j += 1;
        }
        let mut group_text = String::new();
        for span in &spans[i..j] {
            if !group_text.is_empty() && !group_text.ends_with(' ') {
                group_text.push(' ');
            }
            group_text.push_str(span.text.trim());
        }
        let group_text = collapse_whitespace(&group_text);
        let mut rendered = render_span(&group_text, style);
        if let Some(url) = link {
            rendered = apply_link(&rendered, url);
        }
        if !out.is_empty() && !out.ends_with(' ') {
            out.push(' ');
        }
        out.push_str(&rendered);
        i = j;
    }
    out
}

/// Render the text portion of a list item with per-span emphasis. The marker
/// itself isn't included in the output (the renderer handles it separately).
///
/// When the line is uniformly styled we wrap the marker-stripped `rest` with
/// the line's style — this avoids the awkward emphasis-marker mismatch we'd
/// hit if we naively stripped a leading bullet out of an already-wrapped
/// rendered line (`**• item**` → `** item**`).
///
/// When the line is mixed-style we render the full line via the inline pipeline
/// and then best-effort-strip the marker prefix (with optional emphasis wrap
/// around it). On any failure we fall back to plain escaped `rest`.
pub(super) fn render_list_item_text(line: &ProjectedLine, marker: &str, rest: &str) -> String {
    if let Some(style) = line_uniform_style(line) {
        return render_span(&collapse_whitespace(rest), style);
    }
    let full = render_line_inline(line);
    if let Some(stripped) = strip_leading_marker_from_inline(&full, marker) {
        return stripped;
    }
    escape_inline(&collapse_whitespace(rest))
}

/// Try to strip a leading list marker (optionally wrapped in emphasis markers)
/// off `s`. Recognizes `***MARK*** `, `**MARK** `, `*MARK* `, `` `MARK` ``,
/// and bare `MARK `. Returns the suffix on a match.
fn strip_leading_marker_from_inline(s: &str, marker: &str) -> Option<String> {
    for wrap in ["***", "**", "*", "`"] {
        let prefix = format!("{wrap}{marker}{wrap} ");
        if let Some(rest) = s.strip_prefix(&prefix) {
            return Some(rest.to_string());
        }
    }
    let prefix = format!("{marker} ");
    s.strip_prefix(&prefix).map(|r| r.to_string())
}

/// Append an inline-rendered continuation line to an existing list-item body.
/// De-hyphenates against the raw text boundary (mirrors the paragraph rule)
/// and falls back to a space join otherwise.
pub(super) fn append_inline_continuation(
    prev_text: &mut String,
    next_raw: &str,
    next_inline: &str,
) {
    let next_raw = collapse_whitespace(next_raw);
    dehyphenate_join(prev_text, &next_raw, next_inline);
}

/// Returns the shared `SpanStyle` of `line` when every non-whitespace span on
/// the line has the same style; `None` when spans disagree. Mono is folded
/// into the style for the purpose of "uniform" — a fully-mono line is
/// uniform-mono. Used by the paragraph-level optimization to decide whether
/// to wrap once around the whole paragraph or fall back to per-line inline.
pub(super) fn line_uniform_style(line: &ProjectedLine) -> Option<SpanStyle> {
    // A line carrying any hyperlink can't use the uniform fast path: the link
    // wraps only its own spans, so such lines must render via the per-group
    // path in `render_line_inline`.
    if line
        .spans
        .iter()
        .any(|s| !s.text.trim().is_empty() && s.link.is_some())
    {
        return None;
    }
    let mut iter = line
        .spans
        .iter()
        .filter(|s| !s.text.trim().is_empty())
        .map(SpanStyle::from_item);
    let first = iter.next()?;
    for s in iter {
        if s != first {
            return None;
        }
    }
    Some(first)
}

/// True when every non-whitespace span on the line is bold and non-mono.
/// Unlike `line_uniform_style`, this tolerates per-span *italic* variation,
/// so a heading whose spans mix bold and bold-italic (e.g.
/// "**4** ***Foo*** **Bar**") still reads as a single bold line. Returns
/// false for an empty / all-whitespace line.
pub(super) fn line_all_bold(line: &ProjectedLine) -> bool {
    let mut saw_span = false;
    for span in &line.spans {
        if span.text.trim().is_empty() {
            continue;
        }
        if is_mono_item(span) || !is_bold_item(span) {
            return false;
        }
        saw_span = true;
    }
    saw_span
}

#[cfg(test)]
mod tests {
    use super::super::test_helpers::styled_line;
    use super::*;

    #[test]
    fn render_line_inline_mid_line_bold() {
        // Plain span, then bold span: should produce a mid-line `**bold**` run.
        let l = styled_line(
            &[
                ("regular text with", 50.0, Some("Arial")),
                ("bold word", 200.0, Some("Arial-Bold")),
            ],
            100.0,
            10.0,
        );
        let out = render_line_inline(&l);
        assert!(out.contains("regular text with"), "got: {out}");
        assert!(out.contains("**bold word**"), "got: {out}");
        assert!(
            !out.starts_with("**"),
            "mid-line shouldn't open with bold: {out}"
        );
    }

    #[test]
    fn render_line_inline_uniform_bold_uses_shortcut() {
        // All spans bold → single outer wrap, no per-span noise.
        let l = styled_line(
            &[
                ("first", 50.0, Some("Arial-Bold")),
                ("second", 100.0, Some("Arial-Bold")),
            ],
            100.0,
            10.0,
        );
        let out = render_line_inline(&l);
        assert!(out.starts_with("**") && out.ends_with("**"), "got: {out}");
        // Only one bold run, not two — the shortcut should kick in.
        assert_eq!(out.matches("**").count(), 2, "got: {out}");
    }

    #[test]
    fn escape_inline_leaves_dollar_math_alone() {
        assert_eq!(
            escape_inline(r"where $x_{i}\in \mathbb{R}$ and a*b"),
            r"where $x_{i}\in \mathbb{R}$ and a\*b"
        );
        // Currency is not math: the closing `$` is followed by a digit.
        assert_eq!(
            escape_inline("costs $5 to $10 a*b"),
            r"costs $5 to $10 a\*b"
        );
        assert_eq!(escape_inline("$ x*1$"), r"$ x\*1$");
        // A script split off after the span still leaves the LaTeX intact.
        assert_eq!(escape_inline(r"M×$\mathbb{R}$3"), r"M×$\mathbb{R}$3");
    }

    #[test]
    fn escape_inline_keeps_intraword_underscores() {
        // CommonMark never reads an underscore between alphanumerics as emphasis.
        assert_eq!(
            escape_inline("mcc_compliance@masterchemical.com"),
            "mcc_compliance@masterchemical.com"
        );
        assert_eq!(escape_inline("snake_case and x_1"), "snake_case and x_1");
        // At a word edge it can open or close emphasis, so it stays escaped.
        assert_eq!(
            escape_inline("_lead and trail_ and __init__"),
            r"\_lead and trail\_ and \_\_init\_\_"
        );
    }

    #[test]
    fn render_line_inline_escapes_emphasis_chars() {
        let l = styled_line(&[("5*4=20", 50.0, Some("Arial"))], 100.0, 10.0);
        let out = render_line_inline(&l);
        assert_eq!(out, "5\\*4=20");
    }

    #[test]
    fn render_line_inline_italic_then_bold() {
        let l = styled_line(
            &[
                ("italic", 50.0, Some("Arial-Italic")),
                ("plain", 100.0, Some("Arial")),
                ("bold", 150.0, Some("Arial-Bold")),
            ],
            100.0,
            10.0,
        );
        let out = render_line_inline(&l);
        assert!(out.contains("*italic*"), "got: {out}");
        assert!(out.contains("plain"), "got: {out}");
        assert!(out.contains("**bold**"), "got: {out}");
    }

    #[test]
    fn render_line_inline_wraps_link_span() {
        // Plain span followed by a linked span → `[anchor](url)` mid-line.
        let mut l = styled_line(
            &[
                ("see", 50.0, Some("Arial")),
                ("the docs", 150.0, Some("Arial")),
            ],
            100.0,
            10.0,
        );
        l.spans[1].link = Some("https://example.com/docs".to_string());
        let out = render_line_inline(&l);
        assert!(out.contains("see"), "got: {out}");
        assert!(
            out.contains("[the docs](https://example.com/docs)"),
            "got: {out}"
        );
    }

    #[test]
    fn render_line_inline_link_wraps_outside_emphasis() {
        // An italic linked span → `[*anchor*](url)` (link outside emphasis).
        let mut l = styled_line(&[("cite", 50.0, Some("Arial-Italic"))], 100.0, 10.0);
        l.spans[0].link = Some("https://example.com/p.pdf".to_string());
        let out = render_line_inline(&l);
        assert_eq!(out, "[*cite*](https://example.com/p.pdf)");
    }

    #[test]
    fn render_line_inline_link_url_with_space_uses_angle_brackets() {
        let mut l = styled_line(&[("link", 50.0, Some("Arial"))], 100.0, 10.0);
        l.spans[0].link = Some("https://example.com/a b".to_string());
        let out = render_line_inline(&l);
        assert_eq!(out, "[link](<https://example.com/a b>)");
    }

    #[test]
    fn render_line_inline_strike_span() {
        // A struck-through span mid-line → `~~word~~`, plain neighbors untouched.
        let mut l = styled_line(
            &[
                ("keep this", 50.0, Some("Arial")),
                ("removed", 150.0, Some("Arial")),
            ],
            100.0,
            10.0,
        );
        l.spans[1].strike = true;
        let out = render_line_inline(&l);
        assert!(out.contains("keep this"), "got: {out}");
        assert!(out.contains("~~removed~~"), "got: {out}");
    }

    #[test]
    fn render_line_inline_strike_composes_with_bold() {
        // Strike wraps outside emphasis: `~~**word**~~`.
        let mut l = styled_line(&[("gone", 50.0, Some("Arial-Bold"))], 100.0, 10.0);
        l.spans[0].strike = true;
        let out = render_line_inline(&l);
        assert_eq!(out, "~~**gone**~~");
    }

    #[test]
    fn render_line_inline_mono_span() {
        let l = styled_line(
            &[
                ("call", 50.0, Some("Arial")),
                ("foo()", 100.0, Some("Courier")),
                ("on it", 150.0, Some("Arial")),
            ],
            100.0,
            10.0,
        );
        let out = render_line_inline(&l);
        assert!(out.contains("`foo()`"), "got: {out}");
        // Plain spans stay unwrapped.
        assert!(out.contains("call"));
        assert!(out.contains("on it"));
    }

    #[test]
    fn render_line_inline_mono_span_is_not_escaped() {
        let l = styled_line(
            &[
                ("call", 50.0, Some("Arial")),
                ("get_user_id", 100.0, Some("Courier")),
                ("now", 200.0, Some("Arial")),
            ],
            100.0,
            10.0,
        );
        let out = render_line_inline(&l);
        assert!(out.contains("`get_user_id`"), "got: {out}");
        assert!(!out.contains('\\'), "got: {out}");
    }

    #[test]
    fn render_line_inline_mono_span_keeps_backslashes() {
        let l = styled_line(
            &[
                ("open", 50.0, Some("Arial")),
                (r"C:\Program\bin", 100.0, Some("Courier")),
                ("next", 200.0, Some("Arial")),
            ],
            100.0,
            10.0,
        );
        let out = render_line_inline(&l);
        assert!(out.contains(r"`C:\Program\bin`"), "got: {out}");
    }

    #[test]
    fn render_line_inline_uniform_mono_line_is_not_escaped() {
        let l = styled_line(&[("a_b*c", 50.0, Some("Courier"))], 100.0, 10.0);
        let out = render_line_inline(&l);
        assert_eq!(out, "`a_b*c`");
    }

    #[test]
    fn mono_span_with_single_backtick_uses_double_fence() {
        let l = styled_line(&[("a`b", 50.0, Some("Courier"))], 100.0, 10.0);
        let out = render_line_inline(&l);
        assert_eq!(out, "`` a`b ``");
    }

    #[test]
    fn mono_span_with_double_backtick_run_uses_longer_fence() {
        // A 2-backtick run inside the content would close a 2-backtick fence
        // early; the fence must be one longer than the longest interior run.
        let l = styled_line(&[("a``b", 50.0, Some("Courier"))], 100.0, 10.0);
        let out = render_line_inline(&l);
        assert_eq!(out, "``` a``b ```");
    }
}
