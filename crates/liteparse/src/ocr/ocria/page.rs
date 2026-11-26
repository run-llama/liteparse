//! Page-level steps around the models (`server.py`): line crops, column-gutter splitting,
//! orientation, reading-order quads, and the reading filters.

use super::Quad;
use super::img::{Img, otsu, perspective_transform, warp_perspective_cubic};

fn dist(a: [f32; 2], b: [f32; 2]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

/// Perspective-crop a (possibly rotated) quad, as PaddleOCR does (`lpocr_rec._crop`).
pub fn crop(img: &Img, q: &Quad) -> Img {
    let w = (dist(q[0], q[1]).max(dist(q[2], q[3])) as usize).max(2);
    let h = (dist(q[0], q[3]).max(dist(q[1], q[2])) as usize).max(2);
    let dst = [
        [0.0, 0.0],
        [w as f32, 0.0],
        [w as f32, h as f32],
        [0.0, h as f32],
    ];
    let m = perspective_transform(q, &dst);
    let c = warp_perspective_cubic(img, &m, w, h);
    if h as f64 / w as f64 >= 1.5 {
        // vertical box: turn it horizontal
        c.rot90(1)
    } else {
        c
    }
}

fn extent(q: &Quad) -> (f64, f64, f64, f64) {
    let xs = q.iter().map(|p| p[0] as f64);
    let ys = q.iter().map(|p| p[1] as f64);
    (
        xs.clone().fold(f64::INFINITY, f64::min),
        ys.clone().fold(f64::INFINITY, f64::min),
        xs.fold(f64::NEG_INFINITY, f64::max),
        ys.fold(f64::NEG_INFINITY, f64::max),
    )
}

/// Boxes that are clearly horizontal text lines (at least 3x wider than tall, >= 30 px).
pub fn long_lines(boxes: &[Quad]) -> usize {
    boxes
        .iter()
        .filter(|q| {
            let (x0, y0, x1, y1) = extent(q);
            let (dx, dy) = (x1 - x0, y1 - y0);
            dx >= 30.0 && dx >= 3.0 * dy
        })
        .count()
}

/// Detector boxes of a page scanned on its side: many boxes, almost none horizontal lines.
pub fn looks_sideways(boxes: &[Quad]) -> bool {
    boxes.len() >= 15 && (long_lines(boxes) as f64) < 0.25 * boxes.len() as f64
}

/// Most boxes standing upright (taller than 1.5x their width): the lines of a page turned a
/// quarter, as the detector finds them; a sparse page (a plot's axis labels, a short form) has
/// too few boxes for [`looks_sideways`].
pub fn mostly_tall(boxes: &[Quad]) -> bool {
    let tall = boxes
        .iter()
        .filter(|q| {
            let (xs, ys) = (q.iter().map(|p| p[0]), q.iter().map(|p| p[1]));
            let w = xs.clone().fold(f32::NEG_INFINITY, f32::max) - xs.fold(f32::INFINITY, f32::min);
            let h = ys.clone().fold(f32::NEG_INFINITY, f32::max) - ys.fold(f32::INFINITY, f32::min);
            h > 1.5 * w
        })
        .count();
    boxes.len() >= 4 && tall * 10 >= boxes.len() * 4
}

/// A box quad (screen order in the image it was read in) as a quad in the original image
/// whose corners run in reading order: upside-down pages swap corners; pages read after a
/// quarter turn map back with x = w0 - 1 - y', y = x'.
pub fn reading_frame(q: &Quad, flip: bool, sideways: bool, w0: usize) -> Quad {
    let mut p = *q;
    if flip {
        p = [p[2], p[3], p[0], p[1]];
    }
    if sideways {
        for pt in p.iter_mut() {
            *pt = [(w0 as f64 - 1.0 - pt[1] as f64) as f32, pt[0]];
        }
    }
    p
}

/// Minimum blank-run width, in line heights, for a column gutter.
const GUTTER_MIN: f64 = 0.4;

/// Cut near-horizontal boxes at blank vertical runs that line up with other boxes' edges and
/// stay clear in the neighbouring lines (`server.split_gutters`).
pub fn split_gutters(img: &Img, boxes: &[Quad]) -> Vec<Quad> {
    if boxes.len() < 3 {
        return boxes.to_vec();
    }
    let gray = img.mean_gray();
    let (gw, gh) = (gray.w as i64, gray.h as i64);
    let px = |x: i64, y: i64| gray.data[(y * gw + x) as usize];
    let ext: Vec<(f64, f64, f64, f64)> = boxes.iter().map(extent).collect();
    let mut out = Vec::with_capacity(boxes.len());
    for (i, q) in boxes.iter().enumerate() {
        let (x0, y0, x1, y1) = ext[i];
        let h = y1 - y0;
        let tilt = (q[1][1] as f64 - q[0][1] as f64).abs();
        if h < 6.0 || x1 - x0 < 4.0 * h || tilt > 0.25 * h {
            out.push(*q);
            continue;
        }
        let ya = ((y0 + 0.15 * h) as i64).max(0).min(gh);
        let yb = ((y1 - 0.15 * h).ceil() as i64).clamp(0, gh);
        let xa = (x0 as i64).max(0);
        let xb = (x1.ceil() as i64).min(gw);
        if yb <= ya || xb <= xa {
            out.push(*q);
            continue;
        }
        let (mut lo, mut hi) = (255u8, 0u8);
        for y in ya..yb {
            for x in xa..xb {
                let v = px(x, y);
                lo = lo.min(v);
                hi = hi.max(v);
            }
        }
        if (hi as i32) - (lo as i32) < 60 {
            out.push(*q);
            continue;
        }
        let thr = otsu(
            (ya..yb)
                .flat_map(|y| (xa..xb).map(move |x| (x, y)))
                .map(|(x, y)| px(x, y)),
        );
        // A printed column rule (a thin solid stroke that carries on above and below the line, through the
        // space between lines) belongs to the gutter, not the text: glyph stems stop at the line.
        let rule = rule_columns(&px, (xa, xb), (y0, y1), (gw, gh), thr);
        // ink columns (relative to xa)
        let idx: Vec<i64> = (xa..xb)
            .filter(|&x| !rule.contains(&x) && (ya..yb).any(|y| px(x, y) < thr))
            .map(|x| x - xa)
            .collect();
        if idx.len() < 2 {
            out.push(*q);
            continue;
        }
        let (cy, tol) = ((y0 + y1) / 2.0, 0.6 * h);
        let mut cuts: Vec<(f64, f64)> = Vec::new();
        for g in 0..idx.len() - 1 {
            if ((idx[g + 1] - idx[g]) as f64) <= GUTTER_MIN * h {
                continue;
            }
            // blank columns [a, b)
            let (a, b) = ((xa + idx[g] + 1) as f64, (xa + idx[g + 1]) as f64);
            let near = ext.iter().enumerate().filter(|&(j, e)| {
                j != i
                    && ((e.1 + e.3) / 2.0 - cy).abs() < 25.0 * h
                    && 0.5 * h < e.3 - e.1
                    && e.3 - e.1 < 2.0 * h
            });
            let (mut left, mut right) = (0, 0);
            for (_, e) in near {
                left += ((e.0 - b).abs() < tol) as usize;
                right += ((e.2 - a).abs() < tol) as usize;
            }
            // both sides need a column of edges; a hanging-indent marker has only one
            if left < 1 || right < 1 || left + right < 3 || a - x0 < 3.0 * h {
                continue;
            }
            // the gutter stays clear in the neighbouring lines, judged per line
            let m = (((b - a) * 0.25) as i64).max(1) as f64;
            let (sa, sb) = ((a + m) as i64, (b - m) as i64);
            if sb <= sa {
                continue;
            }
            let (mut ok, mut bad) = (0, 0);
            for (j, e) in ext.iter().enumerate() {
                if j == i
                    || ((e.1 + e.3) / 2.0 - cy).abs() > 4.0 * h
                    || !(0.5 * h < e.3 - e.1 && e.3 - e.1 < 2.0 * h)
                {
                    continue;
                }
                if e.2 <= sa as f64 || e.0 >= sb as f64 {
                    ok += 1;
                    continue;
                }
                let eh = e.3 - e.1;
                let (ra, rb) = (
                    ((e.1 + 0.15 * eh) as i64).clamp(0, gh),
                    ((e.3 - 0.15 * eh).ceil() as i64).clamp(0, gh),
                );
                let (ca, cb) = (sa.clamp(0, gw), sb.clamp(0, gw));
                let inked =
                    (ra..rb).any(|y| (ca..cb).any(|x| !rule.contains(&x) && px(x, y) < thr));
                if inked {
                    bad += 1;
                } else {
                    ok += 1;
                }
            }
            if !(ok >= 2 && (bad == 0 || (bad == 1 && ok >= 3))) {
                continue;
            }
            cuts.push((a, b));
        }
        if cuts.is_empty() {
            out.push(*q);
            continue;
        }
        let pad = 0.15 * h;
        let mut edges = vec![x0];
        for &(a, b) in &cuts {
            edges.push(a + pad);
            edges.push(b - pad);
        }
        edges.push(x1);
        for pair in edges.chunks(2) {
            let (s, e) = (pair[0], pair[1]);
            if e - s > 0.5 * h {
                let (s, e, ya, yb) = (s as f32, e as f32, y0 as f32, y1 as f32);
                out.push([[s, ya], [e, ya], [e, yb], [s, yb]]);
            }
        }
    }
    out
}

/// Columns of a printed vertical rule crossing the line box `y0..y1`: runs at most ~0.15 line heights wide
/// that are inked down the whole line and through bands of 0.35 line heights above and below it, with blank
/// columns on both sides, widened by their anti-aliased edges.
fn rule_columns(
    px: &impl Fn(i64, i64) -> u8,
    (xa, xb): (i64, i64),
    (y0, y1): (f64, f64),
    (gw, gh): (i64, i64),
    thr: u8,
) -> std::collections::HashSet<i64> {
    let h = y1 - y0;
    let band = (0.35 * h).ceil() as i64;
    let (top, bot) = (y0 as i64, y1.ceil() as i64);
    let inked_share = |x: i64, ya: i64, yb: i64| -> f64 {
        let (ya, yb) = (ya.clamp(0, gh), yb.clamp(0, gh));
        if yb <= ya || x < 0 || x >= gw {
            return 0.0;
        }
        (ya..yb).filter(|&y| px(x, y) < thr).count() as f64 / (yb - ya) as f64
    };
    let solid: Vec<bool> = (xa..xb)
        .map(|x| {
            inked_share(x, top, bot) >= 0.9
                && inked_share(x, top - band, top) >= 0.8
                && inked_share(x, bot, bot + band) >= 0.8
        })
        .collect();
    let max_w = ((0.15 * h).ceil() as i64).max(3);
    let mut out = std::collections::HashSet::new();
    let mut i = 0usize;
    while i < solid.len() {
        if !solid[i] {
            i += 1;
            continue;
        }
        let start = i;
        while i < solid.len() && solid[i] {
            i += 1;
        }
        // a stroke between blank columns (a light ground): on a dark panel every gap between light letters is
        // "inked" top to bottom too
        let flank = ((0.25 * h).ceil() as i64).max(3);
        let blank_side = |from: i64, to: i64| {
            (from..to)
                .filter(|&x| inked_share(x, top, bot) < 0.1)
                .count() as i64
                * 2
                >= to - from
        };
        let (rs, re) = (xa + start as i64, xa + i as i64);
        if (i - start) as i64 <= max_w && blank_side(rs - flank, rs) && blank_side(re, re + flank) {
            // with its anti-aliased edges, inked in only part of the line
            let pad = ((0.06 * h).ceil() as i64).max(2);
            out.extend((xa + start as i64 - pad)..(xa + i as i64 + pad));
        }
    }
    out
}

/// The most common group (np.bincount(..).argmax(): the lowest index on ties).
pub fn majority(groups: &[usize]) -> usize {
    let mut n = [0usize; 4];
    for &g in groups {
        n[g] += 1;
    }
    let mut best = 0;
    for i in 1..4 {
        if n[i] > n[best] {
            best = i;
        }
    }
    best
}

/// Characters that show a reading really is in `group`'s script (`server.SCRIPT_CHARS`).
pub fn script_chars(group: usize, text: &str) -> usize {
    text.chars()
        .filter(|&c| {
            let u = c as u32;
            match group {
                0 => {
                    c.is_ascii_alphabetic()
                        || (0x00c0..=0x024f).contains(&u)
                        || (0x0370..=0x04ff).contains(&u)
                }
                1 => [
                    (0x3040, 0x30ff),
                    (0x3400, 0x9fff),
                    (0xac00, 0xd7af),
                    (0xf900, 0xfaff),
                ]
                .iter()
                .any(|&(a, b)| (a..=b).contains(&u)),
                2 => [(0x0590, 0x08ff), (0xfb1d, 0xfdff), (0xfe70, 0xfeff)]
                    .iter()
                    .any(|&(a, b)| (a..=b).contains(&u)),
                _ => [
                    (0x0900, 0x0dff),
                    (0x0e00, 0x0eff),
                    (0x0f00, 0x109f),
                    (0x10a0, 0x10ff),
                    (0x0530, 0x058f),
                    (0x1200, 0x139f),
                    (0x1780, 0x17ff),
                ]
                .iter()
                .any(|&(a, b)| (a..=b).contains(&u)),
            }
        })
        .count()
}

/// Cyrillic / Greek lookalikes of Latin letters (`server._HOMO`).
const HOMO_FROM: &str = "аеорсухкіјѕԁԛԝАВЕКМНОРСТХІЈЅΑΒΕΖΗΙΚΜΝΟΡΤΥΧοαν";
const HOMO_TO: &str = "aeopcyxkijsdqwABEKMHOPCTXIJSABEZHIKMNOPTYXoav";
/// Lookalikes from the other scripts a Latin line picks up: the Latin recognizer's own foreign
/// classes (Thai น ท ร, Armenian ա) and the world recognizer reading a Latin line ("REPՍBLQՍE",
/// "C๐mmunicatoทร"). Zero digits that look like "o" count here: inside a Latin word they are
/// the letter.
const HOMO_EXTRA_FROM: &str = "ՍսՕօոհցզաԼՏนทรอบ๐ບນຣ໐၀ဝ";
const HOMO_EXTRA_TO: &str = "UuOonhgqwLSunsououusooo";

/// First code point of each run of decimal digits the recognizers can write besides ASCII
/// (Arabic-Indic, Extended Arabic-Indic, the Indic scripts, Thai, Lao, Tibetan, Myanmar, Khmer).
const DIGIT_ZEROS: [u32; 18] = [
    0x0660, 0x06f0, 0x0966, 0x09e6, 0x0a66, 0x0ae6, 0x0b66, 0x0be6, 0x0c66, 0x0ce6, 0x0d66, 0x0de6,
    0x0e50, 0x0ed0, 0x0f20, 0x1040, 0x1090, 0x17e0,
];

fn is_latin_letter(c: char) -> bool {
    c.is_ascii_alphabetic() || (c.is_alphabetic() && ('\u{00c0}'..='\u{024f}').contains(&c))
}

/// Latin letters in `text` (ASCII and Latin-1 / Extended-A/B; not Greek or Cyrillic).
pub fn latin_letters(text: &str) -> usize {
    text.chars().filter(|&c| is_latin_letter(c)).count()
}

/// Latin and Greek lookalikes of Cyrillic letters, for a Cyrillic word the Latin recognizer wrote
/// partly in Latin ("aрганізацый", "ФOPYMA", "ПяТAГΑ").
const CYR_FROM: &str = "aceopxyiABCEHKMOPTXYIΑΒΓΕΗΚΜΟΠΡΤΧαο";
const CYR_TO: &str = "асеорхуіАВСЕНКМОРТХУІАВГЕНКМОПРТХао";

/// The Latin letter a Cyrillic or other-script lookalike stands for in a Latin word. Small-cap and
/// italic Cyrillic forms (т н м п г л) only look Latin in context, so they are mapped here and
/// not in `server._HOMO`; т н м take the word's case ("ZQX272003тOKEN", "OнMS").
fn latin_lookalike(c: char, upper: bool) -> Option<char> {
    homoglyph(c).or(match c {
        'т' => Some(if upper { 'T' } else { 't' }),
        'н' => Some(if upper { 'H' } else { 'h' }),
        'м' => Some(if upper { 'M' } else { 'm' }),
        'п' | 'л' => Some('n'),
        'г' => Some('r'),
        _ => None,
    })
}

fn homoglyph(c: char) -> Option<char> {
    HOMO_FROM
        .chars()
        .zip(HOMO_TO.chars())
        .chain(HOMO_EXTRA_FROM.chars().zip(HOMO_EXTRA_TO.chars()))
        .find(|m| m.0 == c)
        .map(|m| m.1)
}

/// Clean a reading of other scripts' lookalike characters. In a line that is mostly Latin, inside a
/// word with at least as many Latin letters as other letters, lookalikes become the Latin letter
/// ("REPՍBLQՍE" -> "REPUBLQUE"), a word of lookalikes alone does too ("В - Linear"), and other
/// scripts' digits become ASCII digits ("(UA०८" -> "(UA08"). Lines mostly in another script
/// (a Cyrillic page read by the Latin recognizer, a bilingual Gurmukhi line) are left alone.
pub fn clean_latin(text: &str) -> String {
    let lat = latin_letters(text);
    let oth = text
        .chars()
        .filter(|&c| c.is_alphabetic() && !is_latin_letter(c))
        .count();
    let latin_line = lat > 0 && lat >= 2 * oth;
    let cyr_line = text
        .chars()
        .filter(|&c| ('\u{0400}'..='\u{04ff}').contains(&c))
        .count()
        > lat;
    let mut out = String::with_capacity(text.len());
    for (k, word) in text.split(char::is_whitespace).enumerate() {
        if k > 0 {
            // split() drops the separator; every separator here is one whitespace char
            out.push(' ');
        }
        let wl = latin_letters(word);
        let wo = word
            .chars()
            .filter(|&c| c.is_alphabetic() && !is_latin_letter(c))
            .count();
        let upper = word.chars().filter(|c| c.is_uppercase()).count()
            > word.chars().filter(|c| c.is_lowercase()).count();
        // Greek letters standing alone are real ("α = 0.05"), never lookalikes
        let all_lookalike = wo > 0
            && word.chars().all(|c| {
                !c.is_alphabetic()
                    || is_latin_letter(c)
                    || (latin_lookalike(c, upper).is_some()
                        && !('\u{0370}'..='\u{03ff}').contains(&c))
            });
        // Cyrillic / Greek inside a Latin word fold anywhere (`server._HOMO`); the other scripts'
        // lookalikes only in a Latin line, since Thai and Lao glue Latin words to their own
        // without a space ("Windowsนี้")
        let cg = word
            .chars()
            .filter(|&c| ('\u{0370}'..='\u{04ff}').contains(&c))
            .count();
        let fold_cg = cg > 0 && wl >= cg;
        let fold = latin_line && wo > 0 && (wl >= wo || all_lookalike);
        // the other way: a Cyrillic word with Latin / Greek lookalikes inside
        let cyr = word
            .chars()
            .filter(|&c| ('\u{0400}'..='\u{04ff}').contains(&c))
            .count();
        let lookalikes_only = word
            .chars()
            .filter(|&c| c.is_alphabetic() && !('\u{0400}'..='\u{04ff}').contains(&c))
            .all(|c| CYR_FROM.contains(c));
        let to_cyr =
            (cyr >= 2 && cyr > wl + (cg - cyr)) || (cyr >= 1 && cyr_line && lookalikes_only);
        for c in word.chars() {
            let c = if to_cyr {
                CYR_FROM
                    .chars()
                    .zip(CYR_TO.chars())
                    .find(|m| m.0 == c)
                    .map_or(c, |m| m.1)
            } else if fold || (fold_cg && ('\u{0370}'..='\u{04ff}').contains(&c)) {
                latin_lookalike(c, upper).unwrap_or(c)
            } else {
                c
            };
            let u = c as u32;
            let digit = DIGIT_ZEROS.iter().find(|&&z| (z..z + 10).contains(&u));
            match digit {
                Some(&z) if latin_line => out.push(char::from(b'0' + (u - z) as u8)),
                _ => out.push(c),
            }
        }
    }
    out
}

/// A single short token ("1", "12", "x3"): no space, at least one letter or digit, at most six
/// characters. Faint detector candidates are only kept as one.
pub fn is_short_token(text: &str) -> bool {
    let t = text.trim();
    !t.is_empty()
        && t.chars().count() <= 6
        && !t.chars().any(char::is_whitespace)
        && t.chars().any(char::is_alphanumeric)
}

/// A detector false positive on a picture: no letter or digit, or 1-2 unsure characters.
pub fn is_junk(text: &str, conf: f32) -> bool {
    const JUNK_CONF: f32 = 0.8;
    let word = text.chars().filter(|c| c.is_alphanumeric()).count();
    word == 0 || (word <= 2 && conf < JUNK_CONF)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_tokens() {
        for t in ["1", "12", " 7 ", "x3", "€1", "1,0"] {
            assert!(is_short_token(t), "{t:?}");
        }
        for t in ["", "|", "--", "1 2", "1234567"] {
            assert!(!is_short_token(t), "{t:?}");
        }
    }

    #[test]
    fn homoglyphs_fold_only_inside_latin_words() {
        assert_eq!(clean_latin("pt6а-42 Привет"), "pt6a-42 Привет");
    }

    #[test]
    fn clean_latin_folds_other_scripts_in_latin_lines() {
        // world-recognizer readings of Latin words
        assert_eq!(clean_latin("REPՍBLQՍE FRANÇAISE"), "REPUBLQUE FRANÇAISE");
        assert_eq!(
            clean_latin("C๐mmunicatoทร Security"),
            "Communicatons Security"
        );
        // a lone lookalike word in a Latin line
        assert_eq!(clean_latin("В - Linear 1- Center"), "B - Linear 1- Center");
        // other scripts' digits in a Latin line
        assert_eq!(
            clean_latin("KLAR 08 (UA०८ SHIP TERM"),
            "KLAR 08 (UA08 SHIP TERM"
        );
        assert_eq!(clean_latin("GSTIN: 27ADC300६/१ZK"), "GSTIN: 27ADC3006/1ZK");
        // small-cap / italic Cyrillic forms in Latin words
        assert_eq!(clean_latin("ZQX272003тOKEN OнMS"), "ZQX272003TOKEN OHMS");
        assert_eq!(
            clean_latin("airborлe пoise to гeduce"),
            "airborne noise to reduce"
        );
        // Latin / Greek lookalikes inside Cyrillic words
        assert_eq!(
            clean_latin("aрганізацый ФOPYMA ПяТAГΑ"),
            "арганізацый ФОРУМА ПяТАГА"
        );
        // a Greek letter in an English line stays Greek
        assert_eq!(
            clean_latin("significant at α = 0.05 level"),
            "significant at α = 0.05 level"
        );
    }

    #[test]
    fn clean_latin_leaves_other_script_lines() {
        for s in [
            "Привет мир, как дела",
            "ReturnToWorkSA ਦਾ ਨਿਯਮ ਸਮੂਹ (Regulation Group) ਰਿਟਰਨ ਟੂ ਵਰਕ",
            "३ मार्च २०२४ को",
            "การบริหารความเสี่ยง ๒๕๖๗",
            "Հայաստանի Հանրապետություն",
            "ติดตั้ง Windowsนี้ ได้ที่ร้าน",
        ] {
            assert_eq!(clean_latin(s), s);
        }
    }

    #[test]
    fn reading_frame_maps_quarter_turn_back() {
        let q = [[0.0, 0.0], [10.0, 0.0], [10.0, 2.0], [0.0, 2.0]];
        let r = reading_frame(&q, false, true, 100);
        assert_eq!(r[0], [99.0, 0.0]);
        assert_eq!(r[1], [99.0, 10.0]);
    }
}
