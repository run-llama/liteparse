use std::collections::HashMap;

use crate::types::{OutlineTarget, ParsedPage, ProjectedLine, StructNode};

use super::classify::is_rotated_line;
use super::inline::line_all_bold;
use super::paragraphs::continues_paragraph;
use super::tables::{TABLE_MIN_COLUMNS, split_cells};

/// Tolerance in points for "is this size larger than the body size".
pub(super) const HEADING_SIZE_EPSILON: f32 = 0.5;

/// Margin (points) a *height-estimated* size must clear over body before it
/// counts as a heading size. When PDFium bakes the font size into the text
/// matrix, `dominant_font_size` falls back to bbox height, which jitters ±1pt
/// line-to-line (descenders, parens, capitals). The small `HEADING_SIZE_EPSILON`
/// then admits jittered body lines as a bogus heading level (e.g. 10pt over a
/// 9pt body), promoting every tall-glyph body line to a heading. Real headings
/// in these docs are ≥2pt over body, so a wider margin filters the jitter while
/// keeping genuine headings.
pub(super) const ESTIMATED_HEADING_SIZE_MARGIN: f32 = 1.5;

/// Cap on heading levels (matches plan: H1..H6).
pub(super) const MAX_HEADING_LEVELS: usize = 6;

/// A normalized line text that recurs at least this many times among
/// heading-candidate lines (size > body) is treated as chart/figure label
/// spam, not a heading, and is excluded from the heading-size map. Example:
/// the "Input-Input Layer5" attention-plot labels in academic papers render
/// at 23–28pt (far above body) and would otherwise hijack H1..H3, demoting the
/// real title and section headings. Genuine headings are not byte-identical
/// across the document, so this never drops a real heading size *unless* that
/// size is occupied solely by the repeated string — and the exclusion is
/// per-line, so a repeated string sharing a font size with real headings
/// leaves that level intact.
pub(super) const REPEATED_HEADING_LINE_MIN: usize = 3;

/// Normalize a line's text for repeat-detection: collapse internal whitespace
/// and lowercase, so trivially-different renderings of the same label string
/// still hash together.
fn normalize_repeat_key(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Tighter tolerance for matching against the heading-size map. Keeps the
/// heading detector strict so descender-induced height jitter doesn't promote
/// regular body lines to headings.
pub(super) const FONT_SIZE_HEADING_TOLERANCE: f32 = 0.6;

/// Maximum characters in a "bold body-size heading" candidate. Section
/// headings like "Abstract", "1 Introduction", "2.1 Related Work" are short;
/// a bold body-size line longer than this is almost always a bold *sentence*
/// inside a paragraph, not a heading.
pub(super) const BOLD_HEADING_MAX_CHARS: usize = 80;
/// Maximum length for a font-size-promoted heading. Looser than
/// `BOLD_HEADING_MAX_CHARS` (which gates the bold-detection path) because
/// genuine title lines can run long. Tight enough to reject footnotes
/// and citations that score slightly above body size — those are
/// multi-clause sentences that exceed any reasonable label length.
pub(super) const HEADING_MAX_TEXT_CHARS: usize = 140;

/// Recognize a section-number prefix like "1", "1.5", "A.2", "Sec. 2",
/// "Ch. 3", "§4". Used to exempt numbered subsection headings from the
/// bold-heading run-in guard (which would otherwise reject "1.5. Migrant
/// Workers..." because of the embedded ". " after the section number).
pub(super) fn is_section_number_prefix(s: &str) -> bool {
    let t = s.trim();
    if t.is_empty() {
        return false;
    }
    // Strip optional "Sec", "Ch", "Chapter", "§" lead-in.
    let stripped = t
        .strip_prefix('§')
        .or_else(|| {
            for lead in [
                "Sec.", "Sec", "Ch.", "Ch", "Chapter", "Chap.", "Chap", "Part",
            ] {
                if let Some(rest) = t.strip_prefix(lead) {
                    return Some(rest.trim_start());
                }
            }
            None
        })
        .unwrap_or(t);
    // Remaining must be a dotted numeric / alphanumeric section identifier.
    // Examples accepted: "1", "1.5", "1.5.2", "A.2", "IV".
    if stripped.is_empty() {
        // "§" alone counts as a section marker context.
        return true;
    }
    let mut saw_digit = false;
    let mut prev_dot = true;
    for c in stripped.chars() {
        if c.is_ascii_digit() {
            saw_digit = true;
            prev_dot = false;
        } else if c.is_ascii_uppercase() && !prev_dot {
            // Allow a leading letter ("A.2") but not letters mid-segment.
            return false;
        } else if c.is_ascii_uppercase() {
            prev_dot = false;
        } else if c == '.' {
            if prev_dot {
                return false;
            }
            prev_dot = true;
        } else {
            return false;
        }
    }
    saw_digit
}

/// Recognize an attribution / annotation prefix like "Source:",
/// "Note:", "Adapted from", "Reproduced from", "Image:" — these
/// commonly appear as isolated bold-styled lines beneath charts and
/// figures, but they are never section headings.
pub(super) fn is_attribution_line(text: &str) -> bool {
    let t = text.trim_start();
    const PREFIXES: &[&str] = &[
        "Source:",
        "Sources:",
        "Note:",
        "Notes:",
        "Adapted from",
        "Reproduced from",
        "Reprinted from",
        "Image:",
        "Image source:",
        "Photo:",
        "Photo credit:",
        "Credit:",
        "Caption:",
    ];
    for p in PREFIXES {
        if t.len() >= p.len() && t.is_char_boundary(p.len()) && t[..p.len()].eq_ignore_ascii_case(p)
        {
            return true;
        }
    }
    false
}

/// Recognize caption-prefix lines like "Figure 7", "Fig. 12.", "Table 3:",
/// "Tab. 5", "Equation (4)" — these routinely render in a slightly distinct
/// font/size that lands them in the heading_map and gets them promoted to a
/// document-level heading. We want to keep them as plain paragraphs.
pub(super) fn is_caption_line(text: &str) -> bool {
    let t = text.trim_start();
    // Try each known prefix: must be followed by a number (optionally with
    // separators) within the first ~20 chars.
    const PREFIXES: &[&str] = &[
        "Figure",
        "Figures",
        "Fig.",
        "Fig ",
        "Table",
        "Tables",
        "Tab.",
        "Tab ",
        "Equation",
        "Eq.",
        "Eq ",
        "Scheme",
        "Chart",
        "Plate",
        "Photo",
        "Algorithm",
        "Listing",
    ];
    let lower_t_first_word: String = t
        .chars()
        .take_while(|c| c.is_alphabetic() || *c == '.')
        .collect();
    for p in PREFIXES {
        let p_trim = p.trim_end();
        if lower_t_first_word.eq_ignore_ascii_case(p_trim) {
            // Look at what follows the prefix word.
            let rest = t[lower_t_first_word.len()..].trim_start();
            // Allow a leading "(" then digit, or directly a digit / roman numeral.
            let mut chars = rest.chars();
            if let Some(c0) = chars.next()
                && (c0.is_ascii_digit()
                    || (c0 == '(' && chars.next().is_some_and(|c| c.is_ascii_digit()))
                    || matches!(c0, 'I' | 'V' | 'X' | 'L' | 'C'))
            {
                return true;
            }
        }
    }
    false
}

/// Trailing page-number extractor for TOC-entry shaped lines. Returns the
/// numeric value of the trailing page number when the line looks like a TOC
/// entry: alphabetic body, separator that *includes whitespace* (to reject
/// decimals like "94.2"), then a trailing arabic 1–4 digit number. Roman
/// numerals are accepted in `looks_like_toc_entry` but not returned for the
/// monotonic-sequence check.
pub(super) fn toc_entry_arabic_number(text: &str) -> Option<i32> {
    let s = text.trim();
    if s.is_empty() {
        return None;
    }
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    let mut tail_start = n;
    while tail_start > 0 && chars[tail_start - 1].is_ascii_digit() {
        tail_start -= 1;
    }
    let tail_len = n - tail_start;
    if tail_len == 0 || tail_len > 4 {
        return None;
    }
    // Separator: ≥1 whitespace + optional '.' leaders. The mandatory
    // whitespace rules out decimals ("94.2") and inline number suffixes
    // ("Recall3 7 94.2") that aren't TOC entries.
    let mut sep_end = tail_start;
    let mut saw_ws = false;
    while sep_end > 0 {
        let c = chars[sep_end - 1];
        if c.is_whitespace() {
            sep_end -= 1;
            saw_ws = true;
        } else if c == '.' {
            sep_end -= 1;
        } else {
            break;
        }
    }
    if !saw_ws {
        return None;
    }
    // Body must (a) carry meaningful alpha content and (b) NOT end with a
    // digit — that would mean we sliced a multi-part number ("vol 5 12").
    let body = &chars[..sep_end];
    let alpha = body.iter().filter(|c| c.is_alphabetic()).count();
    // Require ≥8 alpha chars in the body. This keeps real one-word headings
    // like "Chapter 7" / "Section 4" out of the TOC bucket while still
    // accepting typical TOC entries ("Introduction 7", "Conclusion 127", ...).
    if alpha < 8 {
        return None;
    }
    if body
        .iter()
        .rev()
        .find(|c| !c.is_whitespace())
        .is_some_and(|c| c.is_ascii_digit())
    {
        return None;
    }
    let tail: String = chars[tail_start..].iter().collect();
    tail.parse::<i32>().ok()
}

/// Roman-or-arabic TOC-entry detector — used by tests and the TOC-page
/// detector to count TOC-shaped lines that the strict arabic extractor
/// above misses (e.g. "Author's Note ... ix").
pub(super) fn looks_like_toc_entry(text: &str) -> bool {
    if toc_entry_arabic_number(text).is_some() {
        return true;
    }
    // Try Roman numerals at the tail.
    let s = text.trim();
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    let mut tail_start = n;
    while tail_start > 0
        && matches!(
            chars[tail_start - 1],
            'i' | 'v' | 'x' | 'l' | 'c' | 'd' | 'm' | 'I' | 'V' | 'X' | 'L' | 'C' | 'D' | 'M'
        )
    {
        tail_start -= 1;
    }
    let tail_len = n - tail_start;
    if !(2..=6).contains(&tail_len) {
        return false;
    }
    let mut sep_end = tail_start;
    let mut saw_ws = false;
    while sep_end > 0 {
        let c = chars[sep_end - 1];
        if c.is_whitespace() {
            sep_end -= 1;
            saw_ws = true;
        } else if c == '.' {
            sep_end -= 1;
        } else {
            break;
        }
    }
    if !saw_ws {
        return false;
    }
    let alpha = chars[..sep_end]
        .iter()
        .filter(|c| c.is_alphabetic())
        .count();
    alpha >= 5
}

/// Returns true if `text` is the canonical title of a TOC page: "Contents",
/// "Table of Contents", "Index", etc. We use this to let the TOC's own title
/// through the heading promotion that's otherwise suppressed on TOC pages.
pub(super) fn is_toc_title(text: &str) -> bool {
    let t = text.trim().trim_end_matches(':').to_ascii_lowercase();
    matches!(
        t.as_str(),
        "contents"
            | "table of contents"
            | "table of content"
            | "index"
            | "list of figures"
            | "list of tables"
            | "table of figures"
            | "toc"
    )
}

/// TOC-page detection. A page is a TOC iff it carries ≥4 arabic-trailing
/// TOC-entry lines whose page numbers form a *mostly non-decreasing*
/// sequence (≥70% of adjacent pairs satisfy `next >= prev`). The
/// monotonicity check is what separates real TOCs from chart/graph pages
/// with random axis-value tails. Lines whose tail isn't arabic but matches
/// the looser `looks_like_toc_entry` (e.g. roman numerals "ix", "xi") still
/// count toward the row floor — they just don't participate in the
/// monotonicity check.
pub(super) fn page_is_toc(page: &ParsedPage) -> bool {
    let mut nums: Vec<i32> = Vec::new();
    let mut total_toc_like = 0usize;
    for line in &page.projected_lines {
        if is_rotated_line(line) {
            continue;
        }
        if let Some(n) = toc_entry_arabic_number(&line.text) {
            nums.push(n);
            total_toc_like += 1;
        } else if looks_like_toc_entry(&line.text) {
            total_toc_like += 1;
        }
    }
    if total_toc_like < 4 || nums.len() < 3 {
        return false;
    }
    let mut nondec = 0usize;
    for w in nums.windows(2) {
        if w[1] >= w[0] {
            nondec += 1;
        }
    }
    let frac = nondec as f32 / (nums.len() - 1) as f32;
    frac >= 0.7
}

/// Returns true if `line` looks like a section heading rendered in body-size
/// bold text (a very common style for academic / technical PDFs where every
/// "real" heading uses the same font size as body, distinguished only by
/// weight). Requires:
///   - uniformly bold across all spans
///   - short (≤ `BOLD_HEADING_MAX_CHARS`)
///   - paragraph-break gap above (or first line on the page)
///   - paragraph-break gap below (or last line on the page)
pub(super) fn looks_like_bold_heading(
    line: &ProjectedLine,
    prev: Option<&ProjectedLine>,
    next: Option<&ProjectedLine>,
) -> bool {
    let text = line.text.trim();
    if text.is_empty() || text.chars().count() > BOLD_HEADING_MAX_CHARS {
        return false;
    }
    // Captions ("Figure 7", "Table 3.") are commonly bold body-sized lines
    // that would otherwise satisfy every other rule here. Keep them as
    // paragraphs so they don't appear in the heading hierarchy.
    if is_caption_line(text) {
        return false;
    }
    // Attribution lines ("Source: …", "Note: …", "Adapted from …") commonly
    // appear as isolated bold lines beneath charts/figures. Never headings.
    if is_attribution_line(text) {
        return false;
    }
    // Accept a line whose spans are all bold (italic may vary) and non-mono.
    // A strict single-style check would reject headings that mix bold and
    // bold-italic spans (e.g. "**4** ***Foo*** **Bar**"), which are common
    // for numbered section headings.
    if !line_all_bold(line) {
        return false;
    }
    // Run-in labels — a bold lead-in that ends in a period and flows straight
    // into the body sentence ("United Kingdom.", "Model merging.") — read as
    // emphasis, not block headings. A trailing '.' is the giveaway; real
    // section headings (numbered or titled) don't terminate in a period.
    // ':' is deliberately allowed ("Reference frameworks:").
    if text.ends_with('.') {
        return false;
    }
    // Run-in label: a bold line whose first sentence ends in ". " followed
    // by body prose (multi-word continuation that doesn't look like a
    // sub-numbered heading) is a paragraph lead, not a section heading.
    // Tight constraints to avoid rejecting numbered subsection headings like
    // "1.5. Migrant Workers" or "Sec. 2. Method":
    //   - ≥3 space-separated words after the break
    //   - line ends with mid-word "-" (wrap continuation) OR is >50 chars
    //   - first char after the break is uppercase ASCII letter
    let run_in_break = text
        .find(". ")
        .map(|p| (p, 2))
        .or_else(|| text.find(": ").map(|p| (p, 2)));
    if let Some((pos, sep_len)) = run_in_break {
        let before = &text[..pos];
        // Section-number prefix exemption: when the segment before ". " is a
        // numbered section identifier (e.g. "1", "1.5", "A.2", "Sec. 2",
        // "Ch. 3", "§4"), this is a numbered subsection heading like
        // "1.5. Migrant Workers..." — the period is part of the section
        // number, not a sentence terminator. Skip the run-in rejection.
        let is_section_number = is_section_number_prefix(before);
        let after = text[pos + sep_len..].trim();
        let starts_upper = after.chars().next().is_some_and(|c| c.is_ascii_uppercase());
        let word_count = after.split_whitespace().count();
        let ends_hyphen = text.trim_end().ends_with('-');
        if !is_section_number
            && starts_upper
            && ((word_count >= 2 && ends_hyphen) || (word_count >= 3 && text.chars().count() > 50))
        {
            if *super::flags::DEBUG_MD {
                eprintln!(
                    "[MD bold-heading REJECT run-in] '{}' (pos={} word_count={} ends_hyphen={} len={})",
                    text.chars().take(80).collect::<String>(),
                    pos,
                    word_count,
                    ends_hyphen,
                    text.chars().count()
                );
            }
            return false;
        }
    }
    // Block headings are capitalized. A bold line starting lowercase is almost
    // always a stray bold word or a borderless table cell, not a heading.
    if text.chars().next().is_some_and(|c| c.is_lowercase()) {
        return false;
    }
    // Reject bold-uniform lines dominated by digits / punctuation — these are
    // almost always cells inside a tabular layout the table detector didn't
    // pick up (results tables, scoreboards, math display). A real section
    // heading is mostly letters: "1 Introduction" passes (~92% alpha across
    // non-whitespace chars), "47.5 14" doesn't (0%), "BLEU-1 25.87" doesn't.
    if alpha_ratio(text) < 0.5 {
        return false;
    }
    // Reject when the line itself looks tabular: ≥3 cells separated by font-size
    // gaps. A bold body-size line with that many cell tracks is almost always a
    // table header row, not a section heading. Without this guard, multi-line
    // table headers ("Model Method ... | F1 BLEU-1 F1 BLEU-1 ...") get promoted
    // to H3 instead of being absorbed by the table detector.
    // Tabular shape rejection. Two passes because the projection sometimes
    // collapses a wide multi-column line into a single span with column-
    // padding spaces — span-based detection misses those.
    if split_cells(line).len() >= TABLE_MIN_COLUMNS {
        return false;
    }
    // Text-based fallback: ≥3 tokens separated by runs of 2+ spaces (the
    // projection inserts alignment padding between cells) → table header row
    // collapsed into one span, not a section heading.
    let multi_space_tokens = text
        .split("  ")
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .count();
    if multi_space_tokens >= TABLE_MIN_COLUMNS {
        return false;
    }
    let gap_above_ok = match prev {
        None => true,
        Some(p) => !continues_paragraph(p, line),
    };
    if !gap_above_ok {
        return false;
    }
    match next {
        None => true,
        // A wrapped multi-line bold heading ("Cellular Cycle\nand Replication")
        // would otherwise be rejected here because line 1's next line continues
        // the paragraph. Accept the case where `next` is itself an all-bold
        // line — the `heading_run` merge in classify.rs will absorb the
        // continuation. Without this, wrapped bold-only headings silently emit
        // as bold paragraphs and don't reach the heading hierarchy at all.
        Some(n) => !continues_paragraph(line, n) || line_all_bold(n),
    }
}

/// Returns true if `line` is a numbered section heading like "1. **Foo**" —
/// `parse_list_marker` already matched the "N." / "N)" prefix; this checks
/// that the body after the marker is uniformly bold body-size text and that
/// the line has a paragraph break above it. When true the caller should emit a
/// Heading at `heading_map.len()+1` rather than a ListItem. Mirrors
/// `looks_like_bold_heading`'s gating modulo the marker.
pub(super) fn looks_like_numbered_bold_heading(
    line: &ProjectedLine,
    rest: &str,
    prev: Option<&ProjectedLine>,
) -> bool {
    let rest_trim = rest.trim();
    if rest_trim.is_empty() || rest_trim.chars().count() > BOLD_HEADING_MAX_CHARS {
        return false;
    }
    if is_caption_line(&line.text) {
        return false;
    }
    if rest_trim.ends_with('.')
        && rest_trim
            .chars()
            .filter(|c| *c == '.' || *c == '?' || *c == '!')
            .count()
            >= 2
    {
        // "1. Sentence one. Sentence two." → not a heading.
        return false;
    }
    // The spans after the marker must all be bold and non-mono. Marker
    // characters are typically `'0'..='9'`, `'.'`, `')'`, plus whitespace —
    // identify and skip them at the front of the span list.
    let mut saw_bold_body = false;
    let mut saw_non_bold_body = false;
    for span in &line.spans {
        let text = span.text.trim();
        if text.is_empty() {
            continue;
        }
        let is_marker = text
            .chars()
            .all(|c| c.is_ascii_digit() || c == '.' || c == ')' || c == '(');
        if is_marker {
            continue;
        }
        if crate::projection::is_mono_item(span) {
            return false;
        }
        if crate::projection::is_bold_item(span) {
            saw_bold_body = true;
        } else {
            saw_non_bold_body = true;
        }
    }
    if !saw_bold_body || saw_non_bold_body {
        return false;
    }
    // Mostly alphabetic — same intuition as `looks_like_bold_heading`'s
    // alpha-ratio filter: rejects tabular bold rows of digits.
    if alpha_ratio(rest_trim) < 0.5 {
        return false;
    }
    // Paragraph-break gap above. We deliberately don't require gap_below:
    // a numbered section heading is often followed by another bold body
    // line (a sub-heading or a multi-line title continuation) which would
    // satisfy `continues_paragraph`. The numbered+bold combination is
    // distinctive enough that the false-positive risk is small.
    match prev {
        None => true,
        Some(p) => !continues_paragraph(p, line),
    }
}

/// Compute the body font size as the char-weighted mode across all lines in
/// all pages. Rotated lines are excluded so a long rotated sidebar can't
/// pull the body estimate. Falls back to `0.0` when no font-size info is
/// available.
/// A size whose char-weight is at least this fraction of the top size's weight
/// is considered a co-dominant body block. When two sizes are co-dominant, the
/// larger one is the body: dense small-print blocks (references,
/// acknowledgments, footnotes) routinely out-weigh the main narrative by raw
/// char count, but the body is never *smaller* than its own footnotes.
const BODY_CODOMINANT_FRACTION: f32 = 0.5;

pub fn compute_body_size(pages: &[ParsedPage]) -> f32 {
    let mut acc = BodySizeAccumulator::default();
    for page in pages {
        acc.add_page(page);
    }
    acc.finish()
}

/// The char-weighted font-size histogram behind [`compute_body_size`], fed
/// one page at a time so a caller parsing a document in page batches can
/// compute the document's body size without holding every page in memory.
/// Feeding every page in order and calling [`finish`](Self::finish) gives
/// exactly what [`compute_body_size`] returns for the same pages.
#[derive(Debug, Clone, Default)]
pub struct BodySizeAccumulator {
    /// `round(size * 100)` → (first size seen at that key, char weight).
    weights: HashMap<u32, (f32, usize)>,
}

impl BodySizeAccumulator {
    pub fn add_page(&mut self, page: &ParsedPage) {
        for line in &page.projected_lines {
            if is_rotated_line(line) {
                continue;
            }
            let size = heading_size_of(line);
            if size <= 0.0 {
                continue;
            }
            let chars = line.text.chars().count().max(1);
            let key = (size * 100.0).round() as u32;
            let entry = self.weights.entry(key).or_insert((size, 0));
            entry.1 += chars;
        }
    }

    pub fn finish(&self) -> f32 {
        let weights = &self.weights;
        let max_weight = weights.values().map(|(_, n)| *n).max().unwrap_or(0);
        if max_weight == 0 {
            return 0.0;
        }
        let threshold = (max_weight as f32 * BODY_CODOMINANT_FRACTION) as usize;
        // Among sizes that are co-dominant with the heaviest size, pick the
        // largest. This rescues the true body when a dense references/footnote
        // block at a smaller size would otherwise win the raw char-weight vote.
        let body = weights
            .values()
            .filter(|(_, n)| *n >= threshold)
            .map(|(s, _)| *s)
            .fold(0.0_f32, f32::max);
        if std::env::var_os("LITEPARSE_DEBUG_MD").is_some() {
            let mut w: Vec<_> = weights.values().copied().collect();
            w.sort_by(|a, b| b.1.cmp(&a.1));
            eprintln!(
                "[MD body-size] body={body:.2} top weights={:?}",
                &w[..w.len().min(6)]
            );
        }
        body
    }
}

/// Minimum total non-whitespace characters across all occurrences at a font
/// size for it to qualify as a heading level. Low floor — just a noise guard
/// against 1-2 char artifacts. The real legend-token discriminator is
/// `MIN_HEADING_AVG_LINE_CHARS` below.
const MIN_HEADING_TOTAL_CHARS: usize = 10;

/// Minimum average non-whitespace characters *per line* at a font size for it
/// to qualify as a heading. This is what separates a real heading (a coherent
/// line like "Annual Events" = 12 chars/line, or "A-MEM: Agentic Memory for
/// LLM Agents" = 31) from scattered chart-legend tokens at a one-off display
/// size (paper.pdf's "A-mem"/"Base" ~ 4-5 chars/line). A per-line average is
/// robust where a total-char floor was not: a single short-but-real heading
/// (14pt "Aligning with Your Identity" = 24 non-ws chars, over 10.5pt body)
/// clears it, while many tiny repeated tokens do not no matter how many.
const MIN_HEADING_AVG_LINE_CHARS: f32 = 8.0;

/// Maximum average characters per line for a size to qualify as a heading.
/// A "size larger than body" with very long lines is almost always a
/// large-print body block (callouts, footnotes-as-display, intro paragraph),
/// not a real heading.
const MAX_HEADING_AVG_LINE_CHARS: f32 = 200.0;

/// Minimum fraction of non-whitespace chars at a size that must be alphabetic
/// for it to qualify as a heading. Drops sizes dominated by digits (graph
/// axes, results tables, math display) which otherwise pollute the top
/// heading slots.
const MIN_HEADING_ALPHA_RATIO: f32 = 0.5;

/// Build a heading-size → level map: sizes strictly larger than `body_size`,
/// filtered by the quality guards `MIN_HEADING_TOTAL_CHARS`,
/// `MIN_HEADING_AVG_LINE_CHARS`, `MAX_HEADING_AVG_LINE_CHARS`, and
/// `MIN_HEADING_ALPHA_RATIO` (drop one-off equation / figure-label / legend
/// artifacts), sorted descending, mapped to levels 1..=MAX_HEADING_LEVELS.
pub fn build_heading_map(pages: &[ParsedPage], body_size: f32) -> Vec<(f32, u8)> {
    let mut acc = HeadingMapAccumulator::default();
    for page in pages {
        acc.add_page(page);
    }
    acc.finish(body_size)
}

/// The per-line statistics behind [`build_heading_map`], fed one page at a
/// time so a caller parsing a document in page batches can rank heading sizes
/// against the whole document without holding every page in memory.
///
/// Which lines count depends on the body size, which is only known once every
/// page has been seen, so lines are kept as compact groups — one per distinct
/// `(size, size-is-estimated, normalized text)`, with the text reduced to a
/// hash — and the map is built in [`finish`](Self::finish). Feeding every page
/// in order gives exactly what [`build_heading_map`] returns for the same
/// pages and body size.
#[derive(Debug, Clone, Default)]
pub struct HeadingMapAccumulator {
    /// `(size bits, size is estimated, hash of normalize_repeat_key)` →
    /// that group's counts.
    groups: HashMap<(u32, bool, u64), HeadingLineGroup>,
}

/// Counts for one group of lines. `u32` keeps a group at 16 bytes, which
/// matters because body lines are mostly distinct, so there is roughly one
/// group per line of the document.
#[derive(Debug, Clone, Copy)]
struct HeadingLineGroup {
    /// Creation order of the group, so the size that represents a rounded
    /// size key is the first one seen in reading order, as in a single pass.
    first_seen: u32,
    lines: u32,
    /// Non-whitespace characters across the group's lines.
    chars: u32,
    /// Alphabetic characters across the group's lines.
    alpha: u32,
}

impl HeadingMapAccumulator {
    pub fn add_page(&mut self, page: &ParsedPage) {
        use std::hash::{DefaultHasher, Hash, Hasher};
        for line in &page.projected_lines {
            // Captions ("Figure 7", "Table 3.") often render slightly larger
            // than body and would otherwise inflate / hijack the heading map.
            if is_rotated_line(line) || line.in_figure || is_caption_line(&line.text) {
                continue;
            }
            let mut hasher = DefaultHasher::new();
            normalize_repeat_key(&line.text).hash(&mut hasher);
            let key = (
                heading_size_of(line).to_bits(),
                line.font_size_is_estimated,
                hasher.finish(),
            );
            let first_seen = self.groups.len() as u32;
            let group = self.groups.entry(key).or_insert(HeadingLineGroup {
                first_seen,
                lines: 0,
                chars: 0,
                alpha: 0,
            });
            group.lines += 1;
            for c in line.text.chars() {
                if c.is_whitespace() {
                    continue;
                }
                group.chars += 1;
                if c.is_alphabetic() {
                    group.alpha += 1;
                }
            }
        }
    }

    pub fn finish(&self, body_size: f32) -> Vec<(f32, u8)> {
        // Only lines larger than the body can be headings; that is a small
        // fraction of the groups, so filter before sorting.
        let mut candidates: Vec<(f32, u64, HeadingLineGroup)> = self
            .groups
            .iter()
            .filter_map(|(&(size_bits, estimated, repeat_key), group)| {
                let size = f32::from_bits(size_bits);
                let margin = if estimated {
                    ESTIMATED_HEADING_SIZE_MARGIN
                } else {
                    HEADING_SIZE_EPSILON
                };
                (size > body_size + margin).then_some((size, repeat_key, *group))
            })
            .collect();
        candidates.sort_by_key(|(_, _, group)| group.first_seen);
        // Frequency of each normalized candidate-line text. A string that
        // recurs ≥ REPEATED_HEADING_LINE_MIN times is chart/figure label spam
        // (e.g. repeated attention-plot labels at 23–28pt), not a heading.
        let mut text_freq: HashMap<u64, usize> = HashMap::new();
        for (_, repeat_key, group) in &candidates {
            *text_freq.entry(*repeat_key).or_insert(0) += group.lines as usize;
        }
        // (size_key → (size, line_count, total_chars, alpha_chars))
        let mut sizes: HashMap<u32, (f32, usize, usize, usize)> = HashMap::new();
        for (size, repeat_key, group) in &candidates {
            // Exclude repeated chart/figure labels (per-line, so a repeated
            // string sharing a size with real headings still leaves that level).
            if text_freq.get(repeat_key).copied().unwrap_or(0) >= REPEATED_HEADING_LINE_MIN {
                continue;
            }
            let key = (size * 100.0).round() as u32;
            let entry = sizes.entry(key).or_insert((*size, 0, 0, 0));
            entry.1 += group.lines as usize;
            entry.2 += group.chars as usize;
            entry.3 += group.alpha as usize;
        }
        let all: Vec<(f32, usize, usize, usize)> = sizes.into_values().collect();
        // Always apply quality filters: total-char floor, average-line cap, and
        // alpha-ratio floor. The total-char floor (rather than a line-count one)
        // lets one-off titles survive — a 31-char title on a single line passes —
        // while still rejecting chart-legend tokens like "A-mem" + "Base" that
        // total fewer chars across 4 occurrences than a single heading line does.
        let mut kept: Vec<f32> = all
            .iter()
            .filter(|(_, lines, chars, alpha)| {
                let alpha_ratio = if *chars == 0 {
                    0.0
                } else {
                    (*alpha as f32) / (*chars as f32)
                };
                let avg_line_chars = *chars as f32 / (*lines).max(1) as f32;
                *chars >= MIN_HEADING_TOTAL_CHARS
                    && (MIN_HEADING_AVG_LINE_CHARS..=MAX_HEADING_AVG_LINE_CHARS)
                        .contains(&avg_line_chars)
                    && alpha_ratio >= MIN_HEADING_ALPHA_RATIO
            })
            .map(|(s, _, _, _)| *s)
            .collect();
        kept.sort_by(|a, b| b.total_cmp(a));
        kept.truncate(MAX_HEADING_LEVELS);
        kept.into_iter()
            .enumerate()
            .map(|(i, s)| (s, (i + 1) as u8))
            .collect()
    }
}

/// Fraction of non-whitespace characters in `text` that are alphabetic.
/// Returns 0.0 for an empty/all-whitespace string. The heading heuristics use
/// this to reject digit-dominated tabular rows ("47.5 14", "BLEU-1 25.87")
/// that would otherwise be promoted to headings.
fn alpha_ratio(text: &str) -> f32 {
    let mut alpha = 0usize;
    let mut total = 0usize;
    for c in text.chars() {
        if c.is_whitespace() {
            continue;
        }
        total += 1;
        if c.is_alphabetic() {
            alpha += 1;
        }
    }
    if total == 0 {
        return 0.0;
    }
    alpha as f32 / total as f32
}

/// Size a line presents to heading detection: the precise matrix-derived
/// `heading_font_size` when available (matrix-baked-size lines), else the
/// `dominant_font_size`. Used by `compute_body_size`, `build_heading_map`, and
/// the per-line heading-level lookup — NOT by table/paragraph grouping.
pub(super) fn heading_size_of(line: &ProjectedLine) -> f32 {
    line.heading_font_size.unwrap_or(line.dominant_font_size)
}

pub(super) fn heading_level_for(size: f32, heading_map: &[(f32, u8)]) -> Option<u8> {
    for (s, level) in heading_map {
        if (size - *s).abs() < FONT_SIZE_HEADING_TOLERANCE {
            return Some(*level);
        }
    }
    None
}

/// Highest-priority heading source: a struct-tree node `H1`..`H6` directly
/// tagging this line via its `mcid`. Available only for tagged PDFs.
pub(super) fn struct_heading_level(
    line: &ProjectedLine,
    struct_nodes: &[StructNode],
) -> Option<u8> {
    let mcid = line.mcid?;
    for node in struct_nodes {
        if !node.mcids.contains(&mcid) {
            continue;
        }
        if let Some(level) = parse_heading_role(&node.role) {
            return Some(level);
        }
    }
    None
}

/// Parse a struct-tree role string like "H1" or "H3" into a heading level.
/// Returns None for non-heading roles (P, L, Figure, Table, ...).
fn parse_heading_role(role: &str) -> Option<u8> {
    let trimmed = role.trim();
    if !trimmed.starts_with('H') && !trimmed.starts_with('h') {
        return None;
    }
    let digits = &trimmed[1..];
    let n: u8 = digits.parse().ok()?;
    if (1..=6).contains(&n) { Some(n) } else { None }
}

/// Second-priority heading source: a document outline (bookmark) entry that
/// points at this page near this line's y coordinate, with a title that
/// prefix-matches the line text. Used on untagged PDFs that ship a TOC.
pub(super) fn outline_heading_level(
    line: &ProjectedLine,
    page_height: f32,
    outline: &[OutlineTarget],
    line_text: &str,
) -> Option<u8> {
    if outline.is_empty() {
        return None;
    }
    let normalized_line = normalize_outline_text(line_text);
    if normalized_line.is_empty() {
        return None;
    }
    let row_h = line.bbox.height.max(super::MIN_ROW_HEIGHT_PT);
    let y_tolerance = row_h * 1.5;
    for entry in outline {
        let normalized_title = normalize_outline_text(&entry.title);
        if normalized_title.is_empty() {
            continue;
        }
        // Spatial check is *strict* only when the entry actually carries a
        // usable y (in-page). Many bookmarks point at "top of page" with a
        // Y outside the MediaBox or no Y at all — in that case we still
        // accept any line on the page that prefix-matches the title.
        let y_ok = match entry.y_pdf {
            Some(y) => {
                let y_view = page_height - y;
                if y_view < 0.0 || y_view > page_height {
                    true
                } else {
                    (y_view - line.bbox.y).abs() <= y_tolerance
                }
            }
            None => true,
        };
        if !y_ok {
            continue;
        }
        // Short outline titles ("Nutrition") would otherwise false-match any
        // paragraph that happens to start with them. Require the matched line
        // to be heading-shaped: not much longer than the title itself.
        let line_len = normalized_line.chars().count();
        let title_len = normalized_title.chars().count();
        let max_line_len = (title_len * 3).max(120);
        // Multiple sentences → almost certainly prose, not a heading line.
        let sentence_breaks = normalized_line.matches(". ").count();
        if line_len > max_line_len || sentence_breaks >= 2 {
            continue;
        }
        // Run-in label guard: a single ". " sentence break is OK on a heading
        // only when the line is roughly title-shaped. When the line carries
        // substantial body text past the title (e.g. "Base model. Any n-layer
        // transformer architec-"), it's a paragraph leading with a bold
        // run-in label, not a section heading. Title is the *prefix* of the
        // matched line, so excess body length = line_len - title_len.
        if sentence_breaks >= 1 && line_len > title_len + 15 {
            continue;
        }
        if normalized_line.starts_with(&normalized_title)
            || normalized_title.starts_with(&normalized_line)
        {
            return Some(entry.level.min(MAX_HEADING_LEVELS as u8));
        }
    }
    None
}

/// Lowercase + collapse whitespace, for forgiving outline-title vs line-text
/// comparison. Outline titles often have trailing numbering or punctuation
/// not present on the rendered line (and vice versa).
fn normalize_outline_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_space = true;
    for c in s.chars() {
        if c.is_whitespace() {
            if !prev_space {
                out.push(' ');
                prev_space = true;
            }
        } else {
            for lc in c.to_lowercase() {
                out.push(lc);
            }
            prev_space = false;
        }
    }
    if out.ends_with(' ') {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::test_helpers::{line, page};
    use super::*;

    #[test]
    fn toc_entry_arabic_extracts_trailing_page_number() {
        assert_eq!(toc_entry_arabic_number("Introduction 7"), Some(7));
        assert_eq!(
            toc_entry_arabic_number("1. A Fountain in the Square 1"),
            Some(1)
        );
        assert_eq!(
            toc_entry_arabic_number("6. For the Love of Iran . . . 41"),
            Some(41)
        );
    }

    #[test]
    fn toc_entry_arabic_rejects_decimals_and_axis_labels() {
        // "94.2" → decimal, not a TOC entry
        assert_eq!(toc_entry_arabic_number("OCR-Recall3 7 94.2"), None);
        // Chapter 7 — too short an alpha body to be a TOC entry
        assert_eq!(toc_entry_arabic_number("Chapter 7"), None);
        // No separator
        assert_eq!(toc_entry_arabic_number("Section1"), None);
    }

    #[test]
    fn is_toc_title_matches_common_variants() {
        assert!(is_toc_title("Contents"));
        assert!(is_toc_title("Table of Contents"));
        assert!(is_toc_title("table of contents"));
        assert!(is_toc_title("Index"));
        assert!(!is_toc_title("Introduction"));
    }

    #[test]
    fn page_is_toc_requires_monotonic_page_numbers() {
        // Real TOC: monotonically increasing page numbers.
        let pages_toc = page(vec![
            line("Table of contents", 50.0, 30.0, 18.0, 18.0),
            line("Introduction 7", 50.0, 60.0, 12.0, 12.0),
            line("Part I: New Children 21", 50.0, 72.0, 12.0, 12.0),
            line("Part II: From Solitary 45", 50.0, 84.0, 12.0, 12.0),
            line("Part III: Commercial 71", 50.0, 96.0, 12.0, 12.0),
            line("Conclusion 127", 50.0, 108.0, 12.0, 12.0),
        ]);
        assert!(page_is_toc(&pages_toc));

        // Chart page: trailing numbers but NOT monotonic and many fail the
        // separator/alpha rules anyway.
        let pages_chart = page(vec![
            line("OCR-Recall is 94.2", 50.0, 60.0, 9.0, 9.0),
            line("Precision rate 89.0", 50.0, 72.0, 9.0, 9.0),
            line("F1 score 80.4", 50.0, 84.0, 9.0, 9.0),
        ]);
        assert!(!page_is_toc(&pages_chart));
    }

    #[test]
    fn body_size_picks_most_common() {
        let pages = vec![page(vec![
            line("Title", 50.0, 50.0, 18.0, 18.0),
            line("body line one", 50.0, 80.0, 10.0, 10.0),
            line("body line two", 50.0, 92.0, 10.0, 10.0),
            line("body line three", 50.0, 104.0, 10.0, 10.0),
        ])];
        let body = compute_body_size(&pages);
        assert!((body - 10.0).abs() < 0.01, "body size = {body}");
    }

    #[test]
    fn heading_map_descending_levels() {
        // Heading text needs to clear `MIN_HEADING_TOTAL_CHARS` (10) so the
        // size qualifies as a real heading rather than chart-legend noise.
        let pages = vec![page(vec![
            line("The largest heading on the page", 50.0, 50.0, 24.0, 24.0),
            line("A smaller heading right below it", 50.0, 80.0, 18.0, 18.0),
            // Several lines of body so it beats the heading text in the
            // char-weighted body-size mode.
            line(
                "body text line one with plenty of content",
                50.0,
                110.0,
                10.0,
                10.0,
            ),
            line(
                "body text line two with plenty of content",
                50.0,
                122.0,
                10.0,
                10.0,
            ),
            line(
                "body text line three with even more content",
                50.0,
                134.0,
                10.0,
                10.0,
            ),
            line(
                "body text line four with even more content",
                50.0,
                146.0,
                10.0,
                10.0,
            ),
        ])];
        let body = compute_body_size(&pages);
        let map = build_heading_map(&pages, body);
        assert_eq!(map.len(), 2);
        assert_eq!(map[0].1, 1);
        assert_eq!(map[1].1, 2);
        assert!(map[0].0 > map[1].0);
    }
}
