//! `word/numbering.xml`: list definitions and the running counters that turn a
//! paragraph's `w:numPr` into its label ("1.", "a)", "•").

use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

use crate::office::pptx::xml::XNode;

use crate::office::docx::model::Suffix;
use crate::office::docx::props::{PPr, RPr};

#[derive(Debug, Clone)]
pub struct Level {
    pub picture_id: Option<i64>,
    pub start: i64,
    pub fmt: String,
    pub text: String,
    /// The source explicitly sets an empty string without a null-character override.
    pub explicit_empty_text: bool,
    pub ppr: PPr,
    pub rpr: RPr,
    pub suffix: Suffix,
    /// Restart after a use of this level index (0-based); `None` = any higher.
    pub restart: Option<i64>,
    pub legal: bool,
    pub style: Option<String>,
    pub jc: String,
}

impl Level {
    fn parse(n: &XNode) -> Level {
        let val = |name: &str| {
            n.child(name)
                .and_then(|c| c.attr("val"))
                .map(str::to_string)
        };
        Level {
            picture_id: n.child("w:lvlPicBulletId").and_then(|c| c.attr_i64("val")),
            start: n
                .child("w:start")
                .and_then(|c| c.attr_i64("val"))
                .unwrap_or(0),
            fmt: val("w:numFmt").unwrap_or_else(|| "decimal".into()),
            text: val("w:lvlText").unwrap_or_default(),
            explicit_empty_text: n
                .child("w:lvlText")
                .is_some_and(|t| t.attr("val") == Some("") && t.attr_bool("null") != Some(true)),
            ppr: n.child("w:pPr").map(|c| PPr::parse(c)).unwrap_or_default(),
            rpr: n.child("w:rPr").map(|c| RPr::parse(c)).unwrap_or_default(),
            suffix: match val("w:suff").as_deref() {
                Some("space") => Suffix::Space,
                Some("nothing") => Suffix::Nothing,
                _ => Suffix::Tab,
            },
            restart: n
                .child("w:lvlRestart")
                .and_then(|c| c.attr_i64("val"))
                .map(|v| v - 1),
            legal: n.child("w:isLgl").is_some(),
            style: val("w:pStyle"),
            jc: val("w:lvlJc").unwrap_or_else(|| "left".into()),
        }
    }
}

#[derive(Debug, Clone, Default)]
struct Abstract {
    levels: Vec<Option<Level>>,
    style_link: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct Num {
    abstract_id: i64,
    /// Per level: start override and replacement level definition.
    overrides: BTreeMap<i64, (Option<i64>, Option<Level>)>,
}

#[derive(Debug, Default)]
pub struct Numbering {
    pub picture_part: Option<String>,
    pub pictures: HashMap<i64, Rc<XNode>>,
    abstracts: HashMap<i64, Abstract>,
    nums: HashMap<i64, Num>,
    /// Running counters per abstract list, per level; `None` = not started.
    counters: HashMap<i64, [Option<i64>; 9]>,
    /// Nums whose start overrides have been applied.
    started: std::collections::HashSet<i64>,
}

impl Numbering {
    pub fn parse_in_part(root: Option<&XNode>, part: Option<String>) -> Numbering {
        let mut numbering = Self::parse(root);
        numbering.picture_part = part;
        numbering
    }

    pub fn parse(root: Option<&XNode>) -> Numbering {
        let mut out = Numbering::default();
        let Some(root) = root else {
            return out;
        };
        for picture in root.children_named("w:numPicBullet") {
            if let Some(id) = picture.attr_i64("numPicBulletId") {
                out.pictures.insert(id, picture.clone());
            }
        }
        for a in root.children_named("w:abstractNum") {
            let Some(id) = a.attr_i64("abstractNumId") else {
                continue;
            };
            let mut abs = Abstract {
                levels: vec![None; 9],
                style_link: a
                    .child("w:numStyleLink")
                    .and_then(|c| c.attr("val"))
                    .map(str::to_string),
            };
            for l in a.children_named("w:lvl") {
                let i = l.attr_i64("ilvl").unwrap_or(0);
                if (0..9).contains(&i) {
                    abs.levels[i as usize] = Some(Level::parse(l));
                }
            }
            out.abstracts.insert(id, abs);
        }
        for n in root.children_named("w:num") {
            let Some(id) = n.attr_i64("numId") else {
                continue;
            };
            let abstract_id = n
                .child("w:abstractNumId")
                .and_then(|c| c.attr_i64("val"))
                .unwrap_or(-1);
            let mut overrides = BTreeMap::new();
            for o in n.children_named("w:lvlOverride") {
                let i = o.attr_i64("ilvl").unwrap_or(0);
                let start = o.child("w:startOverride").and_then(|c| c.attr_i64("val"));
                let lvl = o.child("w:lvl").map(|l| Level::parse(l));
                overrides.insert(i, (start, lvl));
            }
            out.nums.insert(
                id,
                Num {
                    abstract_id,
                    overrides,
                },
            );
        }
        out
    }

    /// The abstract list a num uses, following `numStyleLink` through the
    /// numbering style's own `numPr` (resolved by `style_num`).
    fn abstract_for(&self, num_id: i64, style_num: &dyn Fn(&str) -> Option<i64>) -> Option<i64> {
        let mut aid = self.nums.get(&num_id)?.abstract_id;
        for _ in 0..4 {
            let link = self.abstracts.get(&aid)?.style_link.clone();
            let Some(link) = link else { break };
            let next = style_num(&link).and_then(|n| self.nums.get(&n))?;
            aid = next.abstract_id;
        }
        Some(aid)
    }

    /// The level definition a paragraph uses, without advancing counters.
    pub fn level(
        &self,
        num_id: i64,
        ilvl: i64,
        style_num: &dyn Fn(&str) -> Option<i64>,
    ) -> Option<Level> {
        let num = self.nums.get(&num_id)?;
        if let Some((_, Some(l))) = num.overrides.get(&ilvl) {
            return Some(l.clone());
        }
        let aid = self.abstract_for(num_id, style_num)?;
        self.abstracts.get(&aid)?.levels.get(ilvl as usize)?.clone()
    }

    /// The level whose `w:pStyle` is `style` (lists attached to heading styles).
    pub fn level_for_style(
        &self,
        num_id: i64,
        style: &str,
        style_num: &dyn Fn(&str) -> Option<i64>,
    ) -> Option<i64> {
        let aid = self.abstract_for(num_id, style_num)?;
        self.abstracts
            .get(&aid)?
            .levels
            .iter()
            .position(|l| l.as_ref().and_then(|l| l.style.as_deref()) == Some(style))
            .map(|p| p as i64)
    }

    /// Advances the counters for a numbered paragraph and returns its label.
    pub fn next_label(
        &mut self,
        num_id: i64,
        ilvl: i64,
        style_num: &dyn Fn(&str) -> Option<i64>,
    ) -> Option<(String, Level)> {
        let ilvl = ilvl.clamp(0, 8);
        let level = self.level(num_id, ilvl, style_num)?;
        let aid = self.abstract_for(num_id, style_num)?;
        let levels: Vec<Option<Level>> = (0..9).map(|i| self.level(num_id, i, style_num)).collect();
        let counters = self.counters.entry(aid).or_insert([None; 9]);
        // A num's start overrides restart the shared list the first time it is used.
        // Apply ancestors first so each deeper override survives its ancestors' reset.
        if self.started.insert(num_id)
            && let Some(num) = self.nums.get(&num_id)
        {
            for (i, (start, _)) in &num.overrides {
                if let (Some(s), true) = (start, (0..9).contains(i)) {
                    counters[*i as usize] = Some(s - 1);
                    for c in counters.iter_mut().skip(*i as usize + 1) {
                        *c = None;
                    }
                }
            }
        }
        let l = ilvl as usize;
        counters[l] = Some(match counters[l] {
            Some(v) => v + 1,
            None => level.start,
        });
        for (deeper, c) in counters.iter_mut().enumerate().skip(l + 1) {
            let restart = levels[deeper]
                .as_ref()
                .and_then(|lv| lv.restart)
                .unwrap_or(deeper as i64 - 1);
            if restart >= 0 && restart >= ilvl {
                *c = None;
            }
        }
        let values: Vec<i64> = (0..9)
            .map(|i| {
                counters[i].unwrap_or_else(|| levels[i].as_ref().map(|l| l.start).unwrap_or(1))
            })
            .collect();
        let mut text = String::new();
        let mut chars = level.text.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '%'
                && let Some(d) = chars.peek().and_then(|d| d.to_digit(10))
            {
                chars.next();
                let i = (d as usize).saturating_sub(1).min(8);
                let fmt = if level.legal && i != l {
                    "decimal"
                } else {
                    levels[i]
                        .as_ref()
                        .map(|lv| lv.fmt.as_str())
                        .unwrap_or("decimal")
                };
                text.push_str(&format_number(fmt, values[i]));
                continue;
            }
            text.push(c);
        }
        if level.fmt == "none" {
            text = level.text.chars().filter(|c| *c != '%').collect();
            text.retain(|c| !c.is_ascii_digit());
        }
        Some((text, level))
    }
}

/// Formats a list counter.
pub fn format_number(fmt: &str, n: i64) -> String {
    let n = n.max(0);
    match fmt {
        "lowerLetter" => letters(n, false),
        "upperLetter" => letters(n, true),
        "lowerRoman" => crate::office::pptx::textlayout::format_autonum("romanLcPlain", n as u32)
            .trim_end_matches('.')
            .to_string(),
        "upperRoman" => crate::office::pptx::textlayout::format_autonum("romanUcPlain", n as u32)
            .trim_end_matches('.')
            .to_string(),
        "decimalZero" => format!("{n:02}"),
        // Footnote symbols: *, †, ‡, §, then each doubled, tripled...
        "chicago" if n > 0 => {
            let sym = ['*', '\u{2020}', '\u{2021}', '\u{a7}'][((n - 1) % 4) as usize];
            std::iter::repeat_n(sym, ((n - 1) / 4 + 1) as usize).collect()
        }
        "bullet" | "none" => String::new(),
        "ordinal" => format!("{n}{}", ordinal_suffix(n)),
        "numberInDash" => format!("- {n} -"),
        "decimalFullWidth" | "decimalFullWidth2" => n
            .to_string()
            .chars()
            .map(|c| char::from_u32(c as u32 - '0' as u32 + 0xFF10).unwrap_or(c))
            .collect(),
        "decimalEnclosedCircle" | "decimalEnclosedCircleChinese" if (1..=20).contains(&n) => {
            char::from_u32(0x2460 + n as u32 - 1).map_or_else(|| n.to_string(), String::from)
        }
        "chineseCounting" | "chineseCountingThousand" | "japaneseCounting" | "ideographDigital"
            if (1..10000).contains(&n) =>
        {
            cjk_counting(n, fmt == "ideographDigital")
        }
        "cardinalText" if (1..1_000_000).contains(&n) => capitalize(&english_words(n)),
        "ordinalText" if (1..1_000_000).contains(&n) => capitalize(&english_ordinal(n)),
        // Field switches `\* CardText` / `\* OrdText` stay lower case.
        "cardinalTextLower" if (1..1_000_000).contains(&n) => english_words(n),
        "ordinalTextLower" if (1..1_000_000).contains(&n) => english_ordinal(n),
        _ => n.to_string(),
    }
}

/// CJK counting numerals: 一 … 九, 十 一 = 11, 二十 = 20, 一百 …; `digital`
/// writes one numeral per decimal digit (〇 for zero).
fn cjk_counting(n: i64, digital: bool) -> String {
    const D: [char; 10] = ['〇', '一', '二', '三', '四', '五', '六', '七', '八', '九'];
    if digital {
        return n
            .to_string()
            .chars()
            .map(|c| D[(c as u8 - b'0') as usize])
            .collect();
    }
    let units = [(1000, '千'), (100, '百'), (10, '十')];
    let mut out = String::new();
    let mut rest = n;
    let mut zero = false;
    for (u, name) in units {
        let d = rest / u;
        rest %= u;
        if d > 0 {
            if zero {
                out.push('〇');
                zero = false;
            }
            // 十 alone stands for 10..19 at the start.
            if !(u == 10 && d == 1 && out.is_empty()) {
                out.push(D[d as usize]);
            }
            out.push(name);
        } else if !out.is_empty() {
            zero = true;
        }
    }
    if rest > 0 {
        if zero {
            out.push('〇');
        }
        out.push(D[rest as usize]);
    }
    out
}

/// Word's letter numbering repeats the letter: a..z, aa..zz, aaa...
fn letters(n: i64, upper: bool) -> String {
    if n <= 0 {
        return String::new();
    }
    // Word cycles after 780, 30 repeats of "z" (Word for Mac probes: 40000 is nine
    // "l", 134954608 twenty-two "v").
    let n = (n - 1) % 780;
    let base = if upper { b'A' } else { b'a' };
    let c = (base + (n % 26) as u8) as char;
    std::iter::repeat_n(c, (n / 26 + 1) as usize).collect()
}

/// English number words: 21 is "twenty-one", 105 "one hundred five" (Word's style).
fn english_words(n: i64) -> String {
    const ONES: [&str; 20] = [
        "zero",
        "one",
        "two",
        "three",
        "four",
        "five",
        "six",
        "seven",
        "eight",
        "nine",
        "ten",
        "eleven",
        "twelve",
        "thirteen",
        "fourteen",
        "fifteen",
        "sixteen",
        "seventeen",
        "eighteen",
        "nineteen",
    ];
    const TENS: [&str; 10] = [
        "", "", "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety",
    ];
    let below_100 = |n: i64| -> String {
        match n {
            0..=19 => ONES[n as usize].to_string(),
            _ if n % 10 == 0 => TENS[(n / 10) as usize].to_string(),
            _ => format!("{}-{}", TENS[(n / 10) as usize], ONES[(n % 10) as usize]),
        }
    };
    let below_1000 = |n: i64| -> String {
        match (n / 100, n % 100) {
            (0, r) => below_100(r),
            (h, 0) => format!("{} hundred", ONES[h as usize]),
            (h, r) => format!("{} hundred {}", ONES[h as usize], below_100(r)),
        }
    };
    match (n / 1000, n % 1000) {
        (0, r) => below_1000(r),
        (t, 0) => format!("{} thousand", below_1000(t)),
        (t, r) => format!("{} thousand {}", below_1000(t), below_1000(r)),
    }
}

/// English ordinal words: "first", "twenty-second", "one hundredth".
fn english_ordinal(n: i64) -> String {
    let words = english_words(n);
    let (head, last) = match words.rfind([' ', '-']) {
        Some(i) => words.split_at(i + 1),
        None => ("", words.as_str()),
    };
    let last = match last {
        "one" => "first".to_string(),
        "two" => "second".to_string(),
        "three" => "third".to_string(),
        "five" => "fifth".to_string(),
        "eight" => "eighth".to_string(),
        "nine" => "ninth".to_string(),
        "twelve" => "twelfth".to_string(),
        w if w.ends_with('y') => format!("{}ieth", &w[..w.len() - 1]),
        w => format!("{w}th"),
    };
    format!("{head}{last}")
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map_or_else(String::new, |f| f.to_uppercase().chain(c).collect())
}

fn ordinal_suffix(n: i64) -> &'static str {
    match (n % 10, n % 100) {
        (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    }
}

#[cfg(test)]
mod tests;
