//! Automatic hyphenation (`w:autoHyphenation`) with Liang's pattern algorithm.
//!
//! Patterns are the hyph-utf8 English files in `src/hyph/` (licences in
//! `src/hyph/LICENSE.md`). Word languages map to them explicitly: British
//! variants (en-GB, en-ZA, en-IN, en-IE, en-NZ, ...) to `en-gb`, every other
//! English variant to `en-us`; other languages are not hyphenated.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

static AUTO: AtomicBool = AtomicBool::new(false);
static CAPS: AtomicBool = AtomicBool::new(true);
/// Hyphenation zone in twips.
static ZONE: AtomicU32 = AtomicU32::new(360);
static LIMIT: AtomicU32 = AtomicU32::new(0);

/// Document hyphenation settings from `settings.xml`.
pub fn set_settings(auto: bool, zone_twips: Option<f64>, limit: Option<i64>, no_caps: bool) {
    let on = auto;
    AUTO.store(on, Ordering::Relaxed);
    CAPS.store(!no_caps, Ordering::Relaxed);
    ZONE.store(
        zone_twips.unwrap_or(360.0).max(0.0) as u32,
        Ordering::Relaxed,
    );
    LIMIT.store(limit.unwrap_or(0).max(0) as u32, Ordering::Relaxed);
}

pub fn enabled() -> bool {
    AUTO.load(Ordering::Relaxed)
}

/// Hyphenation zone in points: a word is hyphenated only when the line would
/// otherwise end further than this from the right edge.
pub fn zone() -> f64 {
    ZONE.load(Ordering::Relaxed) as f64 / 20.0
}

/// Most consecutive lines that may end in a hyphen (0 = no limit).
pub fn consecutive_limit() -> usize {
    LIMIT.load(Ordering::Relaxed) as usize
}

pub fn hyphenate_caps() -> bool {
    CAPS.load(Ordering::Relaxed)
}

struct Patterns {
    /// Letter string to its inter-letter values (one more than the letters).
    rules: HashMap<String, Vec<u8>>,
    exceptions: HashMap<String, Vec<usize>>,
    max_len: usize,
}

const LEFT_MIN: usize = 2;
const RIGHT_MIN: usize = 3;

fn parse(src: &str) -> Patterns {
    let body: String = src
        .lines()
        .map(|l| l.split('%').next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");
    let block = |name: &str| -> String {
        body.find(name)
            .and_then(|i| {
                let rest = &body[i + name.len()..];
                rest.find('}').map(|e| rest[..e].to_string())
            })
            .unwrap_or_default()
    };
    let mut rules = HashMap::new();
    let mut max_len = 0;
    for tok in block("\\patterns{").split_whitespace() {
        let mut letters = String::new();
        let mut vals = vec![0u8];
        for c in tok.chars() {
            if let Some(d) = c.to_digit(10) {
                *vals.last_mut().unwrap() = d as u8;
            } else {
                letters.push(c);
                vals.push(0);
            }
        }
        max_len = max_len.max(letters.chars().count());
        rules.insert(letters, vals);
    }
    let mut exceptions = HashMap::new();
    for tok in block("\\hyphenation{").split_whitespace() {
        let mut word = String::new();
        let mut cuts = Vec::new();
        for c in tok.chars() {
            if c == '-' {
                cuts.push(word.chars().count());
            } else {
                word.push(c);
            }
        }
        exceptions.insert(word.to_lowercase(), cuts);
    }
    Patterns {
        rules,
        exceptions,
        max_len,
    }
}

fn patterns(lang: &str) -> Option<&'static Patterns> {
    static US: OnceLock<Patterns> = OnceLock::new();
    static GB: OnceLock<Patterns> = OnceLock::new();
    let mut tags = lang.split('-');
    if !tags
        .next()
        .is_some_and(|language| language.eq_ignore_ascii_case("en"))
    {
        return None;
    }
    // Region follows the optional script; extension/private-use payloads do not
    // select dictionaries. Existing English regional mappings remain unchanged.
    let region = tags
        .take_while(|tag| tag.len() != 1)
        .find(|tag| tag.len() == 2 && tag.bytes().all(|b| b.is_ascii_alphabetic()));
    let british = [
        "gb", "za", "in", "ie", "nz", "sg", "my", "hk", "jm", "tt", "bz", "zw",
    ];
    if region.is_some_and(|region| british.iter().any(|b| region.eq_ignore_ascii_case(b))) {
        Some(GB.get_or_init(|| parse(include_str!("hyph/hyph-en-gb.tex"))))
    } else {
        Some(US.get_or_init(|| parse(include_str!("hyph/hyph-en-us.tex"))))
    }
}

/// Character offsets inside `word` (all letters) where a hyphen may go, in
/// increasing order; empty when the language has no patterns.
pub fn points(word: &str, lang: &str) -> Vec<usize> {
    let Some(p) = patterns(lang) else {
        return Vec::new();
    };
    let lower: Vec<char> = word.to_lowercase().chars().collect();
    let n = lower.len();
    if n < LEFT_MIN + RIGHT_MIN {
        return Vec::new();
    }
    let key: String = lower.iter().collect();
    if let Some(cuts) = p.exceptions.get(&key) {
        return cuts.clone();
    }
    let dotted: Vec<char> = std::iter::once('.')
        .chain(lower.iter().copied())
        .chain(std::iter::once('.'))
        .collect();
    let mut vals = vec![0u8; dotted.len() + 1];
    for i in 0..dotted.len() {
        let mut s = String::new();
        for &c in &dotted[i..dotted.len().min(i + p.max_len)] {
            s.push(c);
            if let Some(v) = p.rules.get(&s) {
                for (k, &d) in v.iter().enumerate() {
                    vals[i + k] = vals[i + k].max(d);
                }
            }
        }
    }
    // vals[t + 1] sits between word letters t - 1 and t.
    (LEFT_MIN..=n - RIGHT_MIN)
        .filter(|&t| vals[t + 1] % 2 == 1)
        .collect()
}

#[cfg(test)]
mod tests;
