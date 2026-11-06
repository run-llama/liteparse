//! Minimal bidirectional-text helpers.
//!
//! PDF content streams store right-to-left text in *logical* order already —
//! a correctly generated Hebrew/Arabic PDF places the first logical character
//! at the highest x and advances leftward, and PDFium hands the characters back
//! in that same stream order. So liteparse never needs to reverse characters;
//! what it does need is to stop assuming that "left to right on the page" means
//! "first to last in reading order".
//!
//! Two places care:
//!   * `extract` measures inter-character gaps along the writing direction
//!     (see `SegmentBuilder::gap_to`), so RTL runs are not sheared into
//!     fragments by the line-change heuristics.
//!   * line assembly concatenates a line's items in reading order, which for an
//!     RTL line runs right-to-left across the page.

/// Strong right-to-left character (Unicode bidi class R or AL). Covers Hebrew,
/// Arabic, Syriac, Thaana, N'Ko, Samaritan, Mandaic, Adlam and the Arabic
/// presentation-form blocks that subset fonts commonly map to.
pub(crate) fn is_rtl_char(c: char) -> bool {
    matches!(c as u32,
        0x0590..=0x05FF   // Hebrew
        | 0x0600..=0x07BF // Arabic, Syriac, Thaana, N'Ko
        | 0x0800..=0x085F // Samaritan, Mandaic
        | 0x08A0..=0x08FF // Arabic Extended-A
        | 0xFB1D..=0xFDFF // Hebrew/Arabic presentation forms A
        | 0xFE70..=0xFEFF // Arabic presentation forms B
        | 0x10800..=0x10FFF // Cypriot..Old Hungarian etc.
        | 0x1E800..=0x1EFFF // Mende Kikakui, Adlam, Arabic Math
    )
}

/// Strong left-to-right character. Deliberately *not* the complement of
/// [`is_rtl_char`]: digits, punctuation and whitespace are bidi-neutral and
/// must not vote on a line's base direction, or every "12.34 SAR" amount on an
/// Arabic invoice would drag its line back to LTR.
fn is_strong_ltr_char(c: char) -> bool {
    c.is_alphabetic() && !is_rtl_char(c)
}

/// Base direction of a run of characters, decided by majority of
/// strong-direction characters. Neutral-only text (pure numbers, punctuation)
/// is LTR, which keeps every left-to-right document on exactly the path it took
/// before.
fn is_rtl_chars(chars: impl Iterator<Item = char>) -> bool {
    let mut rtl = 0usize;
    let mut ltr = 0usize;
    for c in chars {
        if is_rtl_char(c) {
            rtl += 1;
        } else if is_strong_ltr_char(c) {
            ltr += 1;
        }
    }
    rtl > ltr
}

/// Base direction of a line of text. See [`is_rtl_chars`].
pub(crate) fn is_rtl_text(s: &str) -> bool {
    !s.is_ascii() && is_rtl_chars(s.chars())
}

/// Base direction of a line still held as separate pieces, without joining
/// them first — line assembly needs the direction *before* it picks a join
/// order, and the pieces can be numerous.
///
/// Every RTL character is non-ASCII, so a line whose pieces are all ASCII has
/// an RTL count of zero and cannot be RTL whatever its LTR count. `is_ascii` is
/// a byte-wise scan rather than per-`char` classification, which keeps this off
/// the hot path for left-to-right documents — the overwhelmingly common case.
pub(crate) fn is_rtl_pieces<'a>(pieces: impl IntoIterator<Item = &'a str> + Clone) -> bool {
    if pieces.clone().into_iter().all(|p| p.is_ascii()) {
        return false;
    }
    is_rtl_chars(pieces.into_iter().flat_map(str::chars))
}

/// Reorder a text item's words from visual (left-to-right) order into logical
/// reading order. A left-to-right item is left alone. A right-to-left item is
/// walked right-to-left, except that each run of strong-LTR words, together
/// with the neutral words between them, keeps its left-to-right order.
pub(crate) fn words_to_logical_order(words: &mut [crate::types::WordBox]) {
    if !is_rtl_pieces(words.iter().map(|w| w.text.as_str())) {
        return;
    }
    words.sort_by(|a, b| b.x.total_cmp(&a.x));
    let ltr: Vec<bool> = words
        .iter()
        .map(|w| w.text.chars().any(is_strong_ltr_char))
        .collect();
    let mut i = 0;
    while i < words.len() {
        if !ltr[i] {
            i += 1;
            continue;
        }
        let mut end = i;
        for j in i + 1..words.len() {
            if ltr[j] {
                end = j;
            } else if words[j].text.chars().any(is_rtl_char) {
                break;
            }
        }
        words[i..=end].reverse();
        i = end + 1;
    }
}

/// Glue back words that pdfium delivered as separate visual runs with no
/// space between them, such as an RTL word split around "/" or ":". Two
/// consecutive words whose boxes are closer than `max_gap` become one word.
pub(crate) fn join_touching_words(words: &mut Vec<crate::types::WordBox>, max_gap: f32) {
    let mut i = 1;
    while i < words.len() {
        let (a, b) = (&words[i - 1], &words[i]);
        let gap = (b.x - (a.x + a.width)).max(a.x - (b.x + b.width));
        if gap >= max_gap {
            i += 1;
            continue;
        }
        let b = words.remove(i);
        let a = &mut words[i - 1];
        let right = (a.x + a.width).max(b.x + b.width);
        let bottom = (a.y + a.height).max(b.y + b.height);
        a.x = a.x.min(b.x);
        a.y = a.y.min(b.y);
        a.width = right - a.x;
        a.height = bottom - a.y;
        a.text.push_str(&b.text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::WordBox;

    fn words(texts: &[&str]) -> Vec<WordBox> {
        texts
            .iter()
            .enumerate()
            .map(|(i, t)| WordBox {
                text: t.to_string(),
                x: i as f32 * 10.0,
                ..Default::default()
            })
            .collect()
    }

    fn texts(words: &[WordBox]) -> Vec<&str> {
        words.iter().map(|w| w.text.as_str()).collect()
    }

    /// Catches a plain reversal (which would read "Excel Microsoft") and a
    /// reversal that drags the digits "2020" into the LTR run. Also guards
    /// against touching a left-to-right line at all.
    #[test]
    fn rtl_words_reorder_logically_keeping_ltr_runs() {
        let mut w = words(&["2020", "سنة", "Microsoft", "Excel", "النظام", "استعمل"]);
        words_to_logical_order(&mut w);
        assert_eq!(
            texts(&w),
            ["استعمل", "النظام", "Microsoft", "Excel", "سنة", "2020"]
        );

        let mut w = words(&["Total", "12.34", "ب"]);
        words_to_logical_order(&mut w);
        assert_eq!(texts(&w), ["Total", "12.34", "ب"]);
    }

    /// pdfium hands "أنشطة/روتينات" over as three touching runs. After the
    /// logical reorder they must fuse into one word spanning all three boxes,
    /// while the space-separated neighbour "ما" stays apart.
    #[test]
    fn touching_words_fuse_after_reorder() {
        let mk = |t: &str, x: f32, w: f32| WordBox {
            text: t.to_string(),
            x,
            width: w,
            height: 7.0,
            ..Default::default()
        };
        let mut w = vec![
            mk("روتينات", 525.0, 19.2),
            mk("/", 544.3, 2.1),
            mk("أنشطة", 546.3, 14.3),
            mk("ما", 573.0, 4.6),
        ];
        words_to_logical_order(&mut w);
        join_touching_words(&mut w, 0.15 * 7.5);
        assert_eq!(texts(&w), ["ما", "أنشطة/روتينات"]);
        assert!((w[1].x - 525.0).abs() < 1e-3 && (w[1].width - 35.6).abs() < 1e-3);
    }

    #[test]
    fn strong_direction_classification() {
        assert!(is_rtl_char('\u{05DE}')); // Hebrew mem
        assert!(is_rtl_char('\u{0642}')); // Arabic qaf
        assert!(!is_rtl_char('A'));
        assert!(!is_rtl_char('7'));
        // Digits and punctuation are neutral, not strong LTR.
        assert!(!is_strong_ltr_char('7'));
        assert!(!is_strong_ltr_char('.'));
        assert!(is_strong_ltr_char('A'));
    }

    #[test]
    fn line_direction_ignores_neutrals() {
        // An Arabic label with a Latin currency code and an amount stays RTL:
        // the digits must not outvote the letters.
        assert!(is_rtl_text("المجموع الفرعي: 75.00"));
        assert!(is_rtl_text("סך הכל 12.34"));
        // A Latin line with a stray Hebrew word stays LTR.
        assert!(!is_rtl_text("Total due 87.75 ILS"));
        // Neutral-only lines stay LTR so LTR documents are untouched.
        assert!(!is_rtl_text("2026-07-27 10:30"));
        assert!(!is_rtl_text(""));
    }

    #[test]
    fn ascii_fast_path_agrees_with_full_scan() {
        // The `is_ascii` short-circuit must be an optimization only: for any
        // input it has to return exactly what the per-char scan would.
        for s in [
            "",
            "Total due 87.75 ILS",
            "2026-07-27",
            "!@#$%^&*()",
            "\u{05DE}\u{05E1}",
            "mixed \u{0642} latin",
            "caf\u{e9} na\u{ef}ve",
        ] {
            assert_eq!(is_rtl_text(s), is_rtl_chars(s.chars()), "mismatch on {s:?}");
            assert_eq!(is_rtl_pieces([s]), is_rtl_chars(s.chars()));
        }
        // Split across pieces, where no single piece is decisive.
        assert_eq!(
            is_rtl_pieces(["12.34 ", "\u{05DE}\u{05E1}\u{05E2}"]),
            is_rtl_chars("12.34 \u{05DE}\u{05E1}\u{05E2}".chars())
        );
    }

    #[test]
    fn currency_code_does_not_flip_short_rtl_line() {
        // "USD" is 3 strong LTR chars; the Hebrew must still win.
        assert!(is_rtl_text("סך הכל לתשלום 87.75 ILS"));
    }
}
