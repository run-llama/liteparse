//! Exact repairs for text layers stored in the wrong code page or order.
//!
//! Some PDF fonts map their glyphs to the wrong Unicode characters in a way that is mechanical
//! and reversible, so the text can be restored exactly instead of being read again by OCR:
//! - Cyrillic in the Windows-1251 code page read as Latin-1 / Windows-1252
//!   ("Ìîíòàæ êîíâåêòîðà" for "Монтаж конвектора");
//! - Greek capitals shifted by one from the unassigned U+03A2 on ("ΝΟΜΟ\u{3A2}" for "ΝΟΜΟΣ",
//!   "Σ" for "Τ", "Τ" for "Υ").
//!
//! Each repair is judged per font (the mapping is a property of the font) and kept only when
//! the repaired text passes a check the broken text fails, so clean text is never rewritten.
//! Anything these don't restore is left for the garbled-text checks in `ocr_merge`, which send
//! it to OCR. (Arabic stored in visual order is one of those: reversing each run restored the
//! words but not their order across runs, and scored under OCR on the pages tried.)

use std::collections::HashMap;

use crate::types::TextItem;

/// Windows-1251 for bytes 0x80..=0xBF (0xC0..=0xFF are U+0410..=U+044F in order).
const CP1251_HIGH: [char; 64] = [
    'Ђ', 'Ѓ', '‚', 'ѓ', '„', '…', '†', '‡', '€', '‰', 'Љ', '‹', 'Њ', 'Ќ', 'Ћ', 'Џ', 'ђ', '‘', '’',
    '“', '”', '•', '–', '—', '\u{98}', '™', 'љ', '›', 'њ', 'ќ', 'ћ', 'џ', '\u{A0}', 'Ў', 'ў', 'Ј',
    '¤', 'Ґ', '¦', '§', 'Ё', '©', 'Є', '«', '¬', '\u{AD}', '®', 'Ї', '°', '±', 'І', 'і', 'ґ', 'µ',
    '¶', '·', 'ё', '№', 'є', '»', 'ј', 'Ѕ', 'ѕ', 'ї',
];

/// Windows-1252 for bytes 0x80..=0x9F (the rest of 0x80..=0xFF is Latin-1).
const CP1252_C1: [char; 32] = [
    '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8D}', 'Ž', '\u{8F}',
    '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9D}', 'ž', 'Ÿ',
];

/// The byte a character stands for when Windows-1252 / Latin-1 text is really single-byte text
/// in another code page.
fn latin1_byte(c: char) -> Option<u8> {
    let o = c as u32;
    if (0x80..=0xFF).contains(&o) {
        return Some(o as u8);
    }
    CP1252_C1
        .iter()
        .position(|&x| x == c)
        .map(|i| 0x80 + i as u8)
}

fn cp1251_char(b: u8) -> char {
    match b {
        0x80..=0xBF => CP1251_HIGH[(b - 0x80) as usize],
        _ => char::from_u32(0x0410 + (b - 0xC0) as u32).unwrap_or('\u{FFFD}'),
    }
}

/// `text` with every high Latin-1 character re-read as Windows-1251.
fn as_cp1251(text: &str) -> String {
    text.chars()
        .map(|c| latin1_byte(c).map_or(c, cp1251_char))
        .collect()
}

fn is_cyrillic(c: char) -> bool {
    ('\u{0400}'..='\u{04FF}').contains(&c)
}

/// Fewest high Latin-1 letters in a font's text, and their least share of its letters, for the
/// Windows-1251 reading to be tried.
const MOJIBAKE_MIN: usize = 30;
const MOJIBAKE_MIN_SHARE: f32 = 0.5;
/// Share of Cyrillic letters that are vowels in Russian, Ukrainian, Bulgarian, Serbian text
/// (about 0.42). Another code page read as Windows-1251 lands far outside.
const CYRILLIC_VOWELS: &str = "аеёиоуыэюяіїєАЕЁИОУЫЭЮЯІЇЄ";
const CYRILLIC_VOWEL_RANGE: (f32, f32) = (0.33, 0.52);

/// True when `text` is Windows-1251 Cyrillic read as Latin-1: mostly high Latin-1 letters, and
/// natural Cyrillic once re-read.
fn is_cp1251_mojibake(text: &str) -> bool {
    let letters = text.chars().filter(|c| c.is_alphabetic()).count();
    let high = text
        .chars()
        .filter(|&c| ('\u{C0}'..='\u{FF}').contains(&c) && c != '×' && c != '÷')
        .count();
    if high < MOJIBAKE_MIN || (high as f32) < letters as f32 * MOJIBAKE_MIN_SHARE {
        return false;
    }
    let fixed = as_cp1251(text);
    let cyr = fixed.chars().filter(|&c| is_cyrillic(c)).count();
    let vowels = fixed
        .chars()
        .filter(|&c| CYRILLIC_VOWELS.contains(c))
        .count();
    let share = vowels as f32 / cyr.max(1) as f32;
    if share < CYRILLIC_VOWEL_RANGE.0 || share > CYRILLIC_VOWEL_RANGE.1 {
        return false;
    }
    // Another 8-bit code page (Armenian ArmSCII, say) re-read as Windows-1251 can land in the
    // vowel range by leaning on one letter ("пісіНщіЫЗЭ ПійінісЩіЭ"): no Cyrillic language
    // gives one letter a fifth of its text, or capitals after lowercase inside its words.
    let mut counts: HashMap<char, usize> = HashMap::new();
    for c in fixed.chars().filter(|&c| is_cyrillic(c)) {
        *counts
            .entry(c.to_lowercase().next().unwrap_or(c))
            .or_insert(0) += 1;
    }
    let top = counts.values().copied().max().unwrap_or(0);
    let cs: Vec<char> = fixed.chars().collect();
    let humps = cs
        .windows(2)
        .filter(|w| {
            is_cyrillic(w[0]) && w[0].is_lowercase() && is_cyrillic(w[1]) && w[1].is_uppercase()
        })
        .count();
    top * 5 <= cyr && humps * 50 <= cyr
}

/// Fewest U+03A2 in a font's text for the Greek shift to be undone.
const GREEK_SHIFT_MIN: usize = 3;

/// True when `text` has Greek capitals shifted by one from sigma on: the unassigned U+03A2
/// stands for Σ, Σ for Τ and Τ for Υ, so the text has U+03A2 and no Υ at all.
fn is_greek_shifted(text: &str) -> bool {
    text.chars().filter(|&c| c == '\u{3A2}').count() >= GREEK_SHIFT_MIN && !text.contains('Υ')
}

fn unshift_greek(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '\u{3A2}' => 'Σ',
            'Σ' => 'Τ',
            'Τ' => 'Υ',
            c => c,
        })
        .collect()
}

/// Restore the text of fonts whose mapping is one of the reversible mistakes above. Returns how
/// many items were rewritten.
pub(crate) fn repair_text_items(items: &mut [TextItem]) -> usize {
    let mut by_font: HashMap<Option<&str>, String> = HashMap::new();
    for it in items.iter() {
        let t = by_font.entry(it.font_name.as_deref()).or_default();
        t.push_str(&it.text);
        t.push('\n');
    }
    let mut fixes: HashMap<Option<String>, fn(&str) -> String> = HashMap::new();
    for (font, text) in by_font {
        let fix: Option<fn(&str) -> String> = if is_cp1251_mojibake(&text) {
            Some(as_cp1251)
        } else if is_greek_shifted(&text) {
            Some(unshift_greek)
        } else {
            None
        };
        if let Some(f) = fix {
            fixes.insert(font.map(str::to_string), f);
        }
    }
    if fixes.is_empty() {
        return 0;
    }
    let mut n = 0;
    for it in items.iter_mut() {
        if let Some(f) = fixes.get(&it.font_name) {
            let fixed = f(&it.text);
            if fixed != it.text {
                it.text = fixed;
                n += 1;
            }
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    const RUSSIAN: &str = "Монтаж конвектора. Этот конвектор может быть установлен в любом \
        помещении: в ванных комнатах при условии, что человек не прикасается к нему.";
    const UKRAINIAN: &str = "Ідеологічне підґрунтя інформаційного суспільства: вплив, тому \
        гроші, титули, слава тут не мають значення. До речі, думка її є цікавою.";

    /// Windows-1251 bytes of `s`, read as Windows-1252.
    fn mojibake(s: &str) -> String {
        s.chars()
            .map(|c| {
                let b = match c as u32 {
                    0x0410..=0x044F => Some((c as u32 - 0x0410 + 0xC0) as u8),
                    _ => CP1251_HIGH
                        .iter()
                        .position(|&x| x == c)
                        .map(|i| 0x80 + i as u8),
                };
                match b {
                    Some(b @ 0x80..=0x9F) => CP1252_C1[(b - 0x80) as usize],
                    Some(b) => char::from_u32(b as u32).unwrap(),
                    None => c,
                }
            })
            .collect()
    }

    #[test]
    fn cp1251_mojibake_round_trips() {
        for s in [RUSSIAN, UKRAINIAN] {
            let bad = mojibake(s);
            assert_ne!(bad, s);
            assert!(is_cp1251_mojibake(&bad), "{bad}");
            assert_eq!(as_cp1251(&bad), s);
        }
    }

    #[test]
    fn greek_shift_round_trips() {
        let s = "ΕΛΛΗΝΙΚΗ ΔΗΜΟΚΡΑΤΙΑ ΝΟΜΟΣ ΗΜΑΘΙΑΣ ΔΗΜΟΣ ΗΡΩΙΚΗΣ ΠΟΛΕΩΣ ΝΑΟΥΣΑΣ ΦΥΣΙΟΛΟΓΙΑ";
        let bad: String = s
            .chars()
            .map(|c| match c {
                'Σ' => '\u{3A2}',
                'Τ' => 'Σ',
                'Υ' => 'Τ',
                c => c,
            })
            .collect();
        assert!(is_greek_shifted(&bad));
        assert_eq!(unshift_greek(&bad), s);
        assert!(!is_greek_shifted(s));
    }

    /// Clean text in languages dense in the characters each repair looks at is left alone.
    #[test]
    fn clean_text_is_not_repaired() {
        let clean = [
            "Événement créé à l'été : élève, théâtre, déjà réglé, frère aîné, où ça ? Très âgé.",
            "Tiếng Việt có rất nhiều dấu: được, đã, đến, để, là, và, các, những, một, người.",
            "Þetta er íslenska: það, ég, þú, hún, við, þið, þeir, þær, þau, áður, síðan, núna.",
            "ÀÉÎÕÜ ÇÑ ÆØÅ ÀÉÎÕÜ ÇÑ ÆØÅ ÀÉÎÕÜ ÇÑ ÆØÅ ÀÉÎÕÜ ÇÑ ÆØÅ ÀÉÎÕÜ ÇÑ ÆØÅ ÀÉÎÕÜ",
            // Armenian (ArmSCII) read as Latin-1: garbled, but not Windows-1251
            "ÐÐ ï³ñ³Íù³ÛÇÝ Ï³é³í³ñÙ³Ý ¨ ½³ñ·³óÙ³Ý Ý³Ë³ñ³ñáõÃÛáõÝ",
            RUSSIAN,
            "ΕΛΛΗΝΙΚΗ ΔΗΜΟΚΡΑΤΙΑ ΝΟΜΟΣ ΗΜΑΘΙΑΣ ΥΠΟΥΡΓΕΙΟ",
            "وزيادة على هذا، لم يعالج الفضيل قضايا وطنه فحسب، فقد كان غيره متأثرا بالمدرسة",
        ];
        for c in clean {
            let text = [c; 8].join("\n");
            assert!(!is_cp1251_mojibake(&text), "{c}");
            assert!(!is_greek_shifted(&text), "{c}");
        }
    }

    #[test]
    fn repairs_only_the_broken_font() {
        let item = |font: &str, text: &str| TextItem {
            text: text.to_string(),
            font_name: Some(font.to_string()),
            ..Default::default()
        };
        let mut items = vec![
            item("Helvetica", "Résumé of the café menu, déjà vu."),
            item("Pragmatica", &mojibake(RUSSIAN)),
        ];
        assert_eq!(repair_text_items(&mut items), 1);
        assert_eq!(items[0].text, "Résumé of the café menu, déjà vu.");
        assert_eq!(items[1].text, RUSSIAN);
    }
}
