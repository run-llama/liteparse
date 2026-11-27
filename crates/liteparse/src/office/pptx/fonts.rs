//! Font discovery, substitution and glyph access.
//!
//! Families are resolved in this order: the configured substitution, the requested
//! family if installed, a face embedded in the document (`ppt/fonts`), the built-in
//! metric-compatible substitution (Calibri → Carlito, ...), then a generic sans
//! fallback. Per-character fallback (for scripts
//! the chosen face lacks) goes through `FALLBACK_FAMILIES`.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use fontdb::{Database, Family, ID, Query, Stretch, Style, Weight};
use rustybuzz::ttf_parser;

/// Metric-compatible open substitutes for common Office fonts. Line breaks only
/// match PowerPoint's when advance widths match, so these come first.
const SUBSTITUTIONS: &[(&str, &[&str])] = &[
    ("calibri", &["Carlito"]),
    ("calibri light", &["Carlito"]),
    ("cambria", &["Caladea"]),
    ("arial", &["Liberation Sans", "Arimo"]),
    ("helvetica", &["Arial", "Liberation Sans"]),
    (
        "helvetica neue",
        &["Helvetica Neue", "Arial", "Liberation Sans"],
    ),
    (
        "arial narrow",
        &["Liberation Sans Narrow", "Barlow Semi Condensed"],
    ),
    ("times new roman", &["Liberation Serif", "Tinos"]),
    ("times", &["Times New Roman", "Liberation Serif"]),
    ("courier new", &["Liberation Mono", "Cousine"]),
    ("georgia", &["Gelasio"]),
    ("segoe ui", &["Selawik", "Open Sans", "DejaVu Sans"]),
    (
        "segoe ui semibold",
        &["Selawik", "Open Sans", "DejaVu Sans"],
    ),
    ("segoe ui light", &["Selawik", "Open Sans", "DejaVu Sans"]),
    (
        "segoe ui semilight",
        &["Selawik", "Open Sans", "DejaVu Sans"],
    ),
    ("segoe ui black", &["Selawik", "Open Sans", "DejaVu Sans"]),
    ("segoe sans", &["Selawik", "Open Sans"]),
    ("segoe sans display", &["Selawik", "Open Sans"]),
    ("segoe sans text", &["Selawik", "Open Sans"]),
    ("segoe ui variable display", &["Selawik", "Open Sans"]),
    ("segoe ui variable small", &["Selawik", "Open Sans"]),
    ("segoe ui variable text", &["Selawik", "Open Sans"]),
    ("bc sans", &["Noto Sans"]),
    ("congenial", &["Figtree"]),
    ("book antiqua", &["TeX Gyre Pagella", "Liberation Serif"]),
    ("lucida bright", &["Noto Serif", "Liberation Serif"]),
    // Other faces bundled with Word for Mac (office::docx tables their metrics).
    ("baskerville old face", &["Liberation Serif"]),
    ("footlight mt light", &["Liberation Serif"]),
    ("wide latin", &["Liberation Serif"]),
    ("perpetua", &["EB Garamond", "Liberation Serif"]),
    ("lucida fax", &["Noto Serif", "Liberation Serif"]),
    (
        "lucida sans typewriter",
        &["Liberation Mono", "DejaVu Sans Mono"],
    ),
    (
        "franklin gothic heavy",
        &["Libre Franklin Black", "Libre Franklin"],
    ),
    (
        "palatino linotype",
        &["TeX Gyre Pagella", "Liberation Serif"],
    ),
    ("palatino", &["TeX Gyre Pagella", "Liberation Serif"]),
    ("bookman old style", &["TeX Gyre Bonum", "Liberation Serif"]),
    (
        "century schoolbook",
        &["TeX Gyre Schola", "Liberation Serif"],
    ),
    ("century", &["TeX Gyre Schola", "Liberation Serif"]),
    ("franklin gothic medium", &["Source Sans 3", "Carlito"]),
    ("franklin gothic demi", &["Source Sans 3", "Carlito"]),
    ("consolas", &["Inconsolata", "Liberation Mono"]),
    ("lucida sans unicode", &["DejaVu Sans", "IBM Plex Sans"]),
    ("lucida sans", &["DejaVu Sans", "IBM Plex Sans"]),
    ("lucida console", &["Liberation Mono", "DejaVu Sans Mono"]),
    ("arial nova", &["Liberation Sans", "Arimo"]),
    (
        "arial rounded mt bold",
        &["Arial Rounded MT Bold", "Nunito", "Liberation Sans"],
    ),
    ("constantia", &["Zilla Slab", "Caladea", "Liberation Serif"]),
    ("corbel", &["PT Sans", "Carlito"]),
    (
        "cambria math",
        &["Cambria Math", "STIX Two Math", "Caladea"],
    ),
    (
        "garamond",
        &["EB Garamond", "TeX Gyre Pagella", "Liberation Serif"],
    ),
    // No metric clone exists: Aptos runs ~2-5% wider than Calibri. Source Sans 3
    // matched PowerPoint renders best without flipping line wraps.
    // Thai Office fonts draw much smaller glyphs per em than modern Thai faces;
    // the scales were fitted against PowerPoint renders.
    ("th sarabunpsk", &["Sarabun*0.65", "Noto Sans Thai*0.65"]),
    ("th sarabun new", &["Sarabun*0.65", "Noto Sans Thai*0.65"]),
    ("th sarabunit๙", &["Sarabun*0.65", "Noto Sans Thai*0.65"]),
    ("cordia new", &["Noto Sans Thai*0.7"]),
    ("cordiaupc", &["Noto Sans Thai*0.7"]),
    ("angsana new", &["Noto Sans Thai*0.7"]),
    ("angsanaupc", &["Noto Sans Thai*0.7"]),
    ("browallia new", &["Noto Sans Thai*0.7"]),
    ("browalliaupc", &["Noto Sans Thai*0.7"]),
    ("eucrosiaupc", &["Noto Sans Thai*0.7"]),
    ("freesiaupc", &["Noto Sans Thai*0.7"]),
    ("dilleniaupc", &["Noto Sans Thai*0.7"]),
    ("irisupc", &["Noto Sans Thai*0.7"]),
    ("jasmineupc", &["Noto Sans Thai*0.7"]),
    ("kodchiangupc", &["Noto Sans Thai*0.7"]),
    ("lilyupc", &["Noto Sans Thai*0.7"]),
    ("aptos", &["Source Sans 3", "Carlito"]),
    ("aptos display", &["Source Sans 3", "Carlito"]),
    (
        "century gothic",
        &["URW Gothic", "TeX Gyre Adventor", "DejaVu Sans"],
    ),
    ("verdana", &["DejaVu Sans"]),
    ("tahoma", &["Selawik", "DejaVu Sans Condensed"]),
    (
        "trebuchet ms",
        &["Fira Sans", "IBM Plex Sans", "DejaVu Sans"],
    ),
    ("avenir next", &["Manrope", "Carlito"]),
    ("avenir next lt pro", &["Manrope", "Carlito"]),
    ("avenir", &["Figtree", "Carlito"]),
    ("avenir book", &["Figtree", "Carlito"]),
    ("gill sans mt", &["Gill Sans", "Carlito", "Cabin"]),
    ("gill sans", &["Carlito", "Cabin"]),
    ("comic sans ms", &["Comic Relief", "Comic Neue"]),
    ("arial black", &["Archivo Black", "Liberation Sans"]),
    ("rockwell", &["Arvo", "Liberation Serif"]),
    (
        "impact",
        &["Anton", "Oswald SemiBold", "Liberation Sans Narrow"],
    ),
    ("franklin gothic book", &["Source Sans 3", "Carlito"]),
    ("franklin gothic", &["Source Sans 3", "Carlito"]),
    (
        "arial unicode ms",
        &["Arial Unicode MS", "Liberation Sans", "Noto Sans"],
    ),
    (
        "ms pgothic",
        &["Noto Sans CJK JP", "Hiragino Sans", "Noto Sans JP"],
    ),
    (
        "ｍｓ ｐゴシック",
        &["Noto Sans CJK JP", "Hiragino Sans", "Noto Sans JP"],
    ),
    (
        "ms gothic",
        &["Noto Sans Mono CJK JP", "Noto Sans CJK JP", "Hiragino Sans"],
    ),
    (
        "ｍｓ ゴシック",
        &["Noto Sans Mono CJK JP", "Noto Sans CJK JP", "Hiragino Sans"],
    ),
    ("游ゴシック", &["Noto Sans CJK JP", "Hiragino Sans"]),
    ("游ゴシック light", &["Noto Sans CJK JP", "Hiragino Sans"]),
    ("yu gothic", &["Noto Sans CJK JP", "Hiragino Sans"]),
    ("メイリオ", &["Noto Sans CJK JP", "Hiragino Sans"]),
    ("meiryo", &["Noto Sans CJK JP", "Hiragino Sans"]),
    ("meiryo ui", &["Noto Sans CJK JP", "Hiragino Sans"]),
    (
        "simsun",
        &["Noto Serif CJK SC", "Songti SC", "Noto Sans CJK SC"],
    ),
    (
        "宋体",
        &["Noto Serif CJK SC", "Songti SC", "Noto Sans CJK SC"],
    ),
    ("microsoft yahei", &["Noto Sans CJK SC", "PingFang SC"]),
    ("微软雅黑", &["Noto Sans CJK SC", "PingFang SC"]),
    // Japanese mincho (serif) faces: the Noto Serif CJK JP design.
    ("ms mincho", &["Noto Serif CJK JP", "Hiragino Mincho ProN"]),
    ("ｍｓ 明朝", &["Noto Serif CJK JP", "Hiragino Mincho ProN"]),
    ("ms pmincho", &["Noto Serif CJK JP", "Hiragino Mincho ProN"]),
    (
        "ｍｓ ｐ明朝",
        &["Noto Serif CJK JP", "Hiragino Mincho ProN"],
    ),
    ("yu mincho", &["Noto Serif CJK JP", "Hiragino Mincho ProN"]),
    ("游明朝", &["Noto Serif CJK JP", "Hiragino Mincho ProN"]),
    // Simplified Chinese: hei is sans, kai and fangsong have open look-alikes.
    ("simhei", &["Noto Sans CJK SC", "PingFang SC"]),
    ("黑体", &["Noto Sans CJK SC", "PingFang SC"]),
    ("nsimsun", &["Noto Serif CJK SC", "Songti SC"]),
    ("新宋体", &["Noto Serif CJK SC", "Songti SC"]),
    ("kaiti", &["LXGW WenKai", "Kaiti SC", "Noto Serif CJK SC"]),
    ("楷体", &["LXGW WenKai", "Kaiti SC", "Noto Serif CJK SC"]),
    (
        "楷体_gb2312",
        &["LXGW WenKai", "Kaiti SC", "Noto Serif CJK SC"],
    ),
    (
        "kaiti_gb2312",
        &["LXGW WenKai", "Kaiti SC", "Noto Serif CJK SC"],
    ),
    (
        "fangsong",
        &[
            "Zhuque Fangsong (technical preview)",
            "STFangsong",
            "Noto Serif CJK SC",
        ],
    ),
    (
        "仿宋",
        &[
            "Zhuque Fangsong (technical preview)",
            "STFangsong",
            "Noto Serif CJK SC",
        ],
    ),
    (
        "仿宋_gb2312",
        &[
            "Zhuque Fangsong (technical preview)",
            "STFangsong",
            "Noto Serif CJK SC",
        ],
    ),
    (
        "fangsong_gb2312",
        &[
            "Zhuque Fangsong (technical preview)",
            "STFangsong",
            "Noto Serif CJK SC",
        ],
    ),
    ("方正小标宋简体", &["Noto Serif CJK SC", "Songti SC"]),
    ("方正小标宋_gbk", &["Noto Serif CJK SC", "Songti SC"]),
    ("dengxian", &["Noto Sans CJK SC", "PingFang SC"]),
    ("等线", &["Noto Sans CJK SC", "PingFang SC"]),
    // Traditional Chinese.
    ("pmingliu", &["Noto Serif CJK TC", "LiSong Pro"]),
    ("mingliu", &["Noto Serif CJK TC", "LiSong Pro"]),
    ("新細明體", &["Noto Serif CJK TC", "LiSong Pro"]),
    ("細明體", &["Noto Serif CJK TC", "LiSong Pro"]),
    ("microsoft jhenghei", &["Noto Sans CJK TC", "PingFang TC"]),
    ("微軟正黑體", &["Noto Sans CJK TC", "PingFang TC"]),
    (
        "dfkai-sb",
        &["LXGW WenKai", "Kaiti TC", "Noto Serif CJK TC"],
    ),
    ("標楷體", &["LXGW WenKai", "Kaiti TC", "Noto Serif CJK TC"]),
    // Korean: batang and gungsuh are serif, gulim and dotum sans.
    ("batang", &["Noto Serif CJK KR", "AppleMyungjo"]),
    ("바탕", &["Noto Serif CJK KR", "AppleMyungjo"]),
    ("gungsuh", &["Noto Serif CJK KR", "AppleMyungjo"]),
    ("gulim", &["Noto Sans CJK KR", "Apple SD Gothic Neo"]),
    ("굴림", &["Noto Sans CJK KR", "Apple SD Gothic Neo"]),
    ("dotum", &["Noto Sans CJK KR", "Apple SD Gothic Neo"]),
    ("돋움", &["Noto Sans CJK KR", "Apple SD Gothic Neo"]),
    // Indic.
    ("mangal", &["Noto Sans Devanagari"]),
    (
        "malgun gothic",
        &["Noto Sans CJK KR", "Apple SD Gothic Neo"],
    ),
    ("맑은 고딕", &["Noto Sans CJK KR", "Apple SD Gothic Neo"]),
    ("courier", &["Liberation Mono", "Cousine"]),
    ("source sans pro", &["Source Sans 3", "Carlito"]),
    ("microsoft sans serif", &["Liberation Sans", "Arimo"]),
    ("lucida grande", &["DejaVu Sans", "IBM Plex Sans"]),
    ("segoe", &["Selawik", "Open Sans"]),
    ("verdana pro", &["DejaVu Sans"]),
    ("muli", &["Mulish", "Carlito"]),
    // Narrow faces keep their width class (keys are normalized: "Franklin Gothic
    // Demi Cond" → "franklin gothic cond"). Bahnschrift's widths follow Barlow's.
    ("franklin gothic cond", &["Liberation Sans Narrow"]),
    (
        "aptos narrow",
        &["Barlow Semi Condensed", "Liberation Sans Narrow"],
    ),
    ("bahnschrift", &["Barlow", "Carlito"]),
    (
        "bahnschrift semicondensed",
        &["Barlow Semi Condensed", "Liberation Sans Narrow"],
    ),
    (
        "bahnschrift condensed",
        &["Barlow Condensed", "Liberation Sans Narrow"],
    ),
    (
        "univers condensed",
        &["PT Sans Narrow", "Liberation Sans Narrow"],
    ),
    // Tw Cen MT runs 0.98 (bold 0.97) of its width in Barlow Semi Condensed.
    (
        "tw cen mt",
        &[
            "Barlow Semi Condensed",
            "Source Sans 3",
            "Liberation Sans Narrow",
        ],
    ),
    (
        "tw cen mt condensed",
        &["PT Sans Narrow", "Liberation Sans Narrow"],
    ),
    (
        "agency fb",
        &["Yanone Kaffeesatz", "Liberation Sans Narrow"],
    ),
    ("univers", &["Public Sans", "Liberation Sans"]),
    ("ubuntu", &["Selawik", "Liberation Sans"]),
    ("berlin sans fb", &["Kanit", "Carlito"]),
    // Weights where Carlito fit thumbnails better than the family's substitute.
    ("berlin sans fb demi", &["Carlito"]),
    ("avenir next demi bold", &["Carlito"]),
    ("avenir next ultra light", &["Carlito"]),
    // Script faces: the closest open script by width and slide-1 scores.
    ("monotype corsiva", &["Caveat"]),
    ("brush script mt", &["Caveat"]),
    ("segoe script", &["Caveat"]),
    ("segoe print", &["Caveat"]),
    ("edwardian script itc", &["Italianno"]),
    ("mistral", &["Italianno"]),
    ("gabriola", &["Italianno"]),
    ("segoe ui emoji", &["Noto Emoji", "Selawik"]),
    ("apple color emoji", &["Noto Emoji"]),
    ("noto color emoji", &["Noto Emoji"]),
    (
        "segoe ui symbol",
        &["Noto Sans Symbols 2", "Noto Sans Symbols", "Selawik"],
    ),
];

/// Families that ship without a bold face; PowerPoint (mostly) does not embolden
/// them, so bold runs keep the regular substitute's weight and widths.
// PowerPoint for Mac draws bold runs of these with their only face, at its widths
// (probe of 36 families: a bold and a regular line set the same face and width).
const NO_BOLD: &[&str] = &[
    "arial unicode ms",
    "comic sans ms",
    "calibri light",
    "franklin gothic book",
    "franklin gothic medium",
    "impact",
    "lucida sans unicode",
    "microsoft sans serif",
    "century",
    "segoe ui light",
    "baskerville old face",
    "copperplate gothic bold",
    "elephant",
];

/// (requested family, fallback face family, scale): Times New Roman draws Arabic
/// 0.89 as wide as Noto Naskh Arabic at the same size.
const FALLBACK_SCALES: &[(&str, &str, f64)] = &[("times new roman", "noto naskh arabic", 0.89)];

/// (requested family, substitute face family, bold, italic, scale): Barlow Semi
/// Condensed sets Arial Narrow text this much narrower to match its advances (summed
/// advance widths of English text; bold italic matches within 0.5%).
const STYLED_SCALES: &[(&str, &str, bool, bool, f64)] = &[
    ("arial narrow", "barlow semi condensed", false, false, 0.953),
    ("arial narrow", "barlow semi condensed", true, false, 0.976),
    ("arial narrow", "barlow semi condensed", false, true, 0.979),
];

/// Faces tried first for a script, ahead of [`FALLBACK_FAMILIES`]: broad fonts there
/// (DejaVu Sans) also cover these scripts, but in a much wider style than the Office
/// fonts' own glyphs (Times New Roman and Arial draw Arabic in compact Naskh).
pub fn script_fallbacks(ch: char) -> &'static [&'static str] {
    match ch as u32 {
        0x0600..=0x06FF | 0x0750..=0x077F | 0x08A0..=0x08FF | 0xFB50..=0xFDFF | 0xFE70..=0xFEFF => {
            &["Noto Naskh Arabic", "Noto Sans Arabic"]
        }
        0x0590..=0x05FF | 0xFB1D..=0xFB4F => &["Noto Sans Hebrew"],
        0x0E00..=0x0E7F => &["Noto Sans Thai"],
        0x0900..=0x097F => &["Noto Sans Devanagari"],
        0x0980..=0x09FF => &["Noto Sans Bengali"],
        0x0B80..=0x0BFF => &["Noto Sans Tamil"],
        0x0530..=0x058F => &["Noto Sans Armenian"],
        0x10A0..=0x10FF => &["Noto Sans Georgian"],
        _ => &[],
    }
}

/// Tried in order for characters the run's face cannot map.
const FALLBACK_FAMILIES: &[&str] = &[
    "Carlito",
    "Liberation Sans",
    "Arial",
    "DejaVu Sans",
    "Noto Sans",
    "Noto Sans Symbols",
    "Noto Sans Symbols 2",
    "Noto Sans Thai",
    "Noto Sans Arabic",
    "Noto Sans Hebrew",
    "Noto Sans Devanagari",
    "Noto Sans Bengali",
    "Noto Sans Tamil",
    "Noto Sans Telugu",
    "Noto Sans Gujarati",
    "Noto Sans Gurmukhi",
    "Noto Sans Kannada",
    "Noto Sans Malayalam",
    "Noto Sans Sinhala",
    "Noto Sans Khmer",
    "Noto Sans Lao",
    "Noto Sans Myanmar",
    "Noto Sans Ethiopic",
    "Noto Sans Armenian",
    "Noto Sans Georgian",
    "Noto Sans CJK JP",
    "Hiragino Sans",
    "PingFang SC",
    "Apple SD Gothic Neo",
    "Arial Unicode MS",
    "Apple Symbols",
    "Noto Sans Math",
    "STIX Two Math",
    // Monochrome outlines: colour emoji fonts are bitmaps or layered glyphs,
    // which the rasterizer does not draw.
    "Noto Emoji",
    "Apple Color Emoji",
];

/// Tried first for characters a black or heavy (800+) run's face lacks, as their
/// regular-and-bold-only peers would draw it visibly lighter. Montserrat Black is
/// within 1% of Arial Black's widths (Latin and Cyrillic); Noto Sans adds Greek.
const HEAVY_FALLBACKS: &[&str] = &["Montserrat", "Noto Sans"];

/// Prefix of private family names: marks embedded faces in the database and
/// embedded-font candidates in [`FontSystem::resolve`].
const EMBEDDED_MARK: &str = "\u{1}embedded:";

const GENERIC_SANS: &[&str] = &[
    "Carlito",
    "Liberation Sans",
    "Arial",
    "DejaVu Sans",
    "Helvetica",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FaceKey {
    pub id: ID,
}

/// Vertical metrics in font units, chosen the way Windows GDI (and therefore
/// PowerPoint) does: OS/2 winAscent/winDescent when present, else hhea.
#[derive(Debug, Clone, Copy)]
pub struct Metrics {
    pub units_per_em: f32,
    pub ascent: f32,
    pub descent: f32,
    pub underline_pos: f32,
    pub underline_thickness: f32,
    pub strikeout_pos: f32,
}

pub struct FaceData {
    /// The shaping face, built once (see [`FaceData::shaper`]). It borrows the
    /// bytes of `data`, which the same value owns and never replaces.
    shaper: std::sync::OnceLock<Option<rustybuzz::Face<'static>>>,
    pub data: Arc<dyn AsRef<[u8]> + Send + Sync>,
    pub index: u32,
    pub metrics: Metrics,
    pub family: String,
    /// `wght` axis value for an instance of a variable font.
    pub wght: Option<f32>,
    /// `wdth` axis value (percent of normal) for a narrowed instance.
    pub wdth: Option<f32>,
    /// Embedded in the document (`ppt/fonts`); `family` is the deck's typeface.
    pub embedded: bool,
}

impl FaceData {
    /// The shaping face of [`FaceData::shaping_face`], built on first use and
    /// kept: building one parses the layout tables again each time.
    pub fn shaper(&self) -> Option<&rustybuzz::Face<'_>> {
        self.shaper
            .get_or_init(|| {
                let face = self.shaping_face()?;
                // SAFETY: the face borrows the bytes behind `self.data`, an
                // `Arc` owned by `self` and never replaced, so they outlive the
                // face, which is only handed out borrowed from `self`.
                Some(unsafe {
                    std::mem::transmute::<rustybuzz::Face<'_>, rustybuzz::Face<'static>>(face)
                })
            })
            .as_ref()
    }

    /// A shaping face with the instance's variation applied.
    pub fn shaping_face(&self) -> Option<rustybuzz::Face<'_>> {
        let mut face = rustybuzz::Face::from_slice((*self.data).as_ref(), self.index)?;
        let vars: Vec<rustybuzz::Variation> = [(b"wght", self.wght), (b"wdth", self.wdth)]
            .into_iter()
            .filter_map(|(tag, v)| {
                Some(rustybuzz::Variation {
                    tag: ttf_parser::Tag::from_bytes(tag),
                    value: v?,
                })
            })
            .collect();
        if !vars.is_empty() {
            face.set_variations(&vars);
        }
        Some(face)
    }
}

/// CSS weight a run asks for: a weight word in the family name ("Arial Black" → 900),
/// raised to bold and beyond by the bold flag.
/// Whether PowerPoint draws a bold run of `family` with its only (regular) face,
/// emboldened by a stroke at the regular widths ([`NO_BOLD`]).
pub fn bold_from_regular(family: &str) -> bool {
    let full = family.trim().to_lowercase();
    let base = normalize_family(family).0.to_lowercase();
    NO_BOLD.contains(&full.as_str()) || NO_BOLD.contains(&base.as_str())
}

pub fn requested_weight(family: &str, bold: bool) -> u16 {
    match (normalize_family(family).1, bold) {
        (Some(w), true) => (w + 200).clamp(700, 900),
        (Some(w), false) => w,
        (None, true) => 700,
        (None, false) => 400,
    }
}

/// Splits a requested family into its base name and a weight implied by the name:
/// "Arial,Sans-Serif" → Arial; "Segoe UI Semibold" → (Segoe UI, 600).
pub fn normalize_family(name: &str) -> (String, Option<u16>) {
    let first = name
        .split(',')
        .next()
        .unwrap_or(name)
        .trim()
        .trim_matches('"')
        .trim();
    // PDF subset prefixes ("SVYILS+ArialMT") and PostScript "MT" suffixes.
    let first = match first.split_once('+') {
        Some((tag, rest)) if tag.len() == 6 && tag.chars().all(|c| c.is_ascii_uppercase()) => rest,
        _ => first,
    };
    // UI labels such as "Calibri (Body)" name the theme font they resolved to.
    let first = match first.split_once(" (") {
        Some((f, _)) if !f.trim().is_empty() => f.trim(),
        _ => first,
    };
    let first = first
        .strip_suffix("MT")
        .filter(|f| f.len() > 2)
        .unwrap_or(first);
    let first = postscript_family(first);
    let first = first.as_str();
    let lower = first.to_lowercase();
    // Suffixes are ASCII and cut by byte length, which needs `lower` aligned with
    // `first` (lowercasing can change the length of a few non-ASCII letters).
    if lower.len() != first.len() {
        return (first.to_string(), None);
    }
    // A width word after the weight ("Bahnschrift SemiBold Condensed") stays in the base.
    let width = WIDTH_WORDS
        .iter()
        .find(|w| lower.len() > w.len() && lower.ends_with(*w))
        .map_or("", |w| &first[first.len() - w.len()..]);
    let mut end = first.len() - width.len();
    const WEIGHTS: &[(&str, u16)] = &[
        (" extralight", 200),
        (" extra light", 200),
        (" ultralight", 200),
        (" ultra light", 200),
        (" semilight", 350),
        (" semibold", 600),
        (" semi bold", 600),
        (" demibold", 600),
        (" demi bold", 600),
        (" extrabold", 800),
        (" extra bold", 800),
        (" ultrabold", 800),
        (" light", 300),
        (" medium", 500),
        (" regular", 400),
        (" bold", 700),
        (" black", 900),
        (" heavy", 900),
        (" thin", 100),
        (" demi", 600),
    ];
    // "Italic(s)" after a space or weight ("BoldItalic") is a style, not the family.
    for s in ["italics", "italic", "oblique"] {
        if let Some(r) = lower[..end].strip_suffix(s) {
            let styled = r.ends_with(' ') || WEIGHTS.iter().any(|(w, _)| r.ends_with(&w[1..]));
            if styled && r.trim_end().contains(' ') {
                end = r.trim_end().len();
            }
            break;
        }
    }
    let lower = &lower[..end];
    for (suffix, w) in WEIGHTS {
        if lower.ends_with(suffix) {
            let base = first[..end - suffix.len()].trim();
            return (format!("{base}{width}"), Some(*w));
        }
    }
    (format!("{}{width}", first[..end].trim_end()), None)
}

/// Width words that may follow a weight word in a family name, lowercase.
const WIDTH_WORDS: &[&str] = &[
    " semi condensed",
    " semicondensed",
    " extra condensed",
    " condensed",
    " compressed",
    " narrow",
    " cond",
];

/// Family words from a PostScript-style name: "TimesNewRomanPS-BoldMT" →
/// "Times New Roman Bold", "OpenSans-Light" → "Open Sans Light". A name that
/// already has spaces only gets its hyphens replaced.
fn postscript_family(name: &str) -> String {
    if name.contains(' ') {
        return name.replace('-', " ");
    }
    let (fam, style) = match name.rsplit_once('-') {
        Some((f, s)) if !f.is_empty() => (f, s.strip_suffix("MT").unwrap_or(s)),
        _ => (name, ""),
    };
    let fam = fam.strip_suffix("MT").unwrap_or(fam);
    let fam = fam
        .strip_suffix("PS")
        .filter(|f| f.len() > 2)
        .unwrap_or(fam);
    // Split camel case at lower → upper transitions only ("IBMPlex" stays whole).
    let mut out = String::new();
    let mut prev_lower = false;
    for c in fam.chars() {
        if c.is_uppercase() && prev_lower {
            out.push(' ');
        }
        prev_lower = c.is_lowercase();
        out.push(c);
    }
    if !style.is_empty() {
        out.push(' ');
        out.push_str(style);
    }
    out
}

/// A resolved face and the substitute family it came from, if any.
type Resolved = (Option<ID>, Option<String>);

pub struct FontSystem {
    db: RefCell<Database>,
    faces: RefCell<HashMap<ID, Arc<FaceData>>>,
    /// Resolved face per (lowercase family, bold, italic), with the substitute
    /// family when one was used.
    resolve_cache: RefCell<HashMap<(String, bool, bool), Resolved>>,
    /// Glyph scale of a substitute drawn for a requested family, keyed by
    /// (requested family, lowercase; resolved face). Absent means 1.0.
    em_scales: RefCell<HashMap<(String, ID), f64>>,
    /// Last-resort fallback per character: any installed face that maps it.
    scan_cache: RefCell<HashMap<char, Option<ID>>>,
    /// Whether a face maps a character (memo of [`FontSystem::has_glyph`]).
    glyph_cache: RefCell<HashMap<(ID, char), bool>>,
    outlines: RefCell<HashMap<(ID, u16), Option<tiny_skia::Path>>>,
    /// Instances of variable fonts: (base face, weight) → registered face, and
    /// each instance's `wght` value.
    /// Keyed by (base face, weight, width in tenths of a percent; 0 = default).
    instances: RefCell<HashMap<(ID, u16, u16), ID>>,
    instance_wght: RefCell<HashMap<ID, f32>>,
    instance_wdth: RefCell<HashMap<ID, f32>>,
    user_subs: HashMap<String, String>,
    /// Lowercase family → its installed spelling, `None` when not installed.
    families: RefCell<HashMap<String, Option<String>>>,
    /// Requested families that were not installed, with what replaced them
    /// (sorted, so the warnings they become are in a stable order).
    pub substituted: RefCell<BTreeSet<(String, String)>>,
    /// Faces embedded in the document: lowercase typeface → regular, bold,
    /// italic and bold italic slots.
    embedded: RefCell<HashMap<String, [Option<ID>; 4]>>,
    /// Typeface each embedded face was registered for.
    embedded_names: RefCell<HashMap<ID, String>>,
}

/// Re-registers "Semilight" faces that declare weight 300 (Selawik Semilight) at
/// 350, so a light request takes the family's narrower Light face.
fn semilight_weights(db: &mut Database) {
    let semilight: Vec<fontdb::FaceInfo> = db
        .faces()
        .filter(|f| {
            f.weight.0 == 300
                && (f.post_script_name.to_lowercase().contains("semilight")
                    || f.families
                        .iter()
                        .any(|(n, _)| n.to_lowercase().contains("semilight")))
        })
        .cloned()
        .collect();
    for mut face in semilight {
        db.remove_face(face.id);
        face.weight = fontdb::Weight(350);
        db.push_face_info(face);
    }
}

impl FontSystem {
    pub fn new(
        font_dirs: &[String],
        system: bool,
        user_subs: &HashMap<String, String>,
    ) -> FontSystem {
        let mut db = Database::new();
        for dir in font_dirs {
            db.load_fonts_dir(dir);
        }
        if system {
            db.load_system_fonts();
            // Development convenience: LibreOffice bundles the metric-compatible set.
            for dir in [
                "/Applications/LibreOffice.app/Contents/Resources/fonts/truetype",
                "/usr/lib/libreoffice/share/fonts/truetype",
                "/opt/libreoffice/share/fonts/truetype",
            ] {
                if std::path::Path::new(dir).is_dir() {
                    db.load_fonts_dir(dir);
                }
            }
        }
        semilight_weights(&mut db);
        FontSystem {
            db: RefCell::new(db),
            faces: RefCell::new(HashMap::new()),
            resolve_cache: RefCell::new(HashMap::new()),
            scan_cache: RefCell::new(HashMap::new()),
            glyph_cache: RefCell::new(HashMap::new()),
            em_scales: RefCell::new(HashMap::new()),
            outlines: RefCell::new(HashMap::new()),
            instances: RefCell::new(HashMap::new()),
            instance_wght: RefCell::new(HashMap::new()),
            instance_wdth: RefCell::new(HashMap::new()),
            user_subs: user_subs
                .iter()
                .map(|(k, v)| (k.to_lowercase(), v.clone()))
                .collect(),
            families: RefCell::new(HashMap::new()),
            substituted: RefCell::new(BTreeSet::new()),
            embedded: RefCell::new(HashMap::new()),
            embedded_names: RefCell::new(HashMap::new()),
        }
    }

    /// A copy for another render thread: same faces (installed and embedded)
    /// and caches. Call [`FontSystem::share_files`] first so that the copies
    /// read one mapping of each font file rather than one each.
    pub fn fork(&self) -> FontSystem {
        FontSystem {
            db: RefCell::new(self.db.borrow().clone()),
            faces: self.faces.clone(),
            resolve_cache: self.resolve_cache.clone(),
            em_scales: self.em_scales.clone(),
            scan_cache: self.scan_cache.clone(),
            glyph_cache: self.glyph_cache.clone(),
            outlines: self.outlines.clone(),
            instances: self.instances.clone(),
            instance_wght: self.instance_wght.clone(),
            instance_wdth: self.instance_wdth.clone(),
            user_subs: self.user_subs.clone(),
            families: self.families.clone(),
            substituted: self.substituted.clone(),
            embedded: self.embedded.clone(),
            embedded_names: self.embedded_names.clone(),
        }
    }

    /// Memory-maps every font file now (pages load only when read), so forks
    /// made afterwards share each mapping.
    pub fn share_files(&self) {
        let mut db = self.db.borrow_mut();
        let ids: Vec<ID> = db.faces().map(|f| f.id).collect();
        for id in ids {
            // SAFETY: as in `face`: font files are not modified while the process runs.
            drop(unsafe { db.make_shared_face_data(id) });
        }
    }

    /// True when `name` is installed or embedded in the document (either as is
    /// or as the base of an embedded weight-named typeface).
    pub fn has_family(&self, name: &str) -> bool {
        if self.installed(name) {
            return true;
        }
        let key = name.trim().to_lowercase();
        self.embedded
            .borrow()
            .keys()
            .any(|k| *k == key || normalize_family(k).0 == key)
    }

    fn installed(&self, name: &str) -> bool {
        self.canonical(name).is_some()
    }

    /// The installed family name matching `name` regardless of case ("lato" → "Lato");
    /// font queries compare names exactly.
    fn canonical(&self, name: &str) -> Option<String> {
        let key = name.to_lowercase();
        if let Some(v) = self.families.borrow().get(&key) {
            return v.clone();
        }
        let found = self.db.borrow().faces().find_map(|f| {
            f.families
                .iter()
                .find(|(n, _)| n.eq_ignore_ascii_case(name))
                .map(|(n, _)| n.clone())
        });
        self.families.borrow_mut().insert(key, found.clone());
        found
    }

    fn query_weight(&self, family: &str, weight: u16, italic: bool) -> Option<ID> {
        self.query_stretch(family, weight, italic, Stretch::Normal)
    }

    fn query_stretch(
        &self,
        family: &str,
        weight: u16,
        italic: bool,
        stretch: Stretch,
    ) -> Option<ID> {
        let weight = Weight(weight);
        let style = if italic { Style::Italic } else { Style::Normal };
        let name = self.canonical(family);
        let families = [Family::Name(name.as_deref().unwrap_or(family))];
        let q = Query {
            families: &families,
            weight,
            stretch,
            style,
        };
        self.db.borrow().query(&q)
    }

    /// Resolves an exact source family, including document-local aliases.
    /// Configured substitutions use the ordinary resolution path instead.
    pub fn source_face(&self, family: &str, bold: bool, italic: bool) -> Option<ID> {
        let full = family.trim();
        let (base, _) = normalize_family(full);
        if self.has_user_substitute(full) || self.has_user_substitute(&base) {
            return None;
        }
        let weight = requested_weight(full, bold);
        self.query_weight(full, weight, italic)
            .map(|id| self.instance_width(id, weight, None))
            .or_else(|| self.embedded_face(full, bold, italic))
    }

    /// Resolves a requested family + style to an installed face. Weight comes from
    /// the bold flag and any weight word in the name; variable fonts get an
    /// instance at that weight.
    pub fn resolve(&self, family: &str, bold: bool, italic: bool) -> Option<ID> {
        let key = (family.to_lowercase(), bold, italic);
        let full = family
            .split(',')
            .next()
            .unwrap_or(family)
            .trim()
            .to_string();
        if let Some((hit, sub)) = self.resolve_cache.borrow().get(&key) {
            // Each spelling met is reported, so the warnings do not depend on
            // which page (or render thread) asked first.
            if let Some(sub) = sub {
                self.substituted.borrow_mut().insert((full, sub.clone()));
            }
            return *hit;
        }
        let (base, name_weight) = normalize_family(family);
        let weight = requested_weight(family, bold);
        let mut candidates: Vec<String> = Vec::new();
        for name in [full.to_lowercase(), base.to_lowercase()] {
            if let Some(u) = self.user_subs.get(&name) {
                candidates.push(u.clone());
            }
        }
        candidates.push(full.clone());
        candidates.push(format!("{EMBEDDED_MARK}{full}"));
        candidates.push(base.clone());
        candidates.push(format!("{EMBEDDED_MARK}{base}"));
        for name in [full.to_lowercase(), base.to_lowercase()] {
            if let Some((_, subs)) = SUBSTITUTIONS.iter().find(|(k, _)| *k == name) {
                candidates.extend(subs.iter().map(|s| (*s).to_string()));
            }
        }
        candidates.extend(GENERIC_SANS.iter().map(|s| (*s).to_string()));
        // A deck's embedded copy of "Montserrat Extra Bold" comes before installed
        // Montserrat drawn at 800: the requested name is then only taken as is.
        let derived_later = [&full, &base]
            .iter()
            .any(|n| self.embedded_face(n, bold, italic).is_some());
        let mut found = None;
        let mut substitute = None;
        for c in &candidates {
            // An embedded copy stands in for a family that is not installed.
            if let Some(name) = c.strip_prefix(EMBEDDED_MARK) {
                if let Some(id) = self.embedded_face(name, bold, italic) {
                    found = Some(id);
                    break;
                }
                continue;
            }
            // "Family*0.8": the substitute's glyphs are drawn at 0.8 × the size, for
            // fonts whose glyphs are much smaller per em than the substitute's.
            let (c, em) = match c.rsplit_once('*') {
                Some((n, f)) => (n.trim().to_string(), f.trim().parse::<f64>().unwrap_or(1.0)),
                None => (c.clone(), 1.0),
            };
            let c = &c;
            // Width variants such as "DejaVu Sans Condensed" are registered under
            // their typographic family ("DejaVu Sans") with a narrower stretch.
            // A substitute named with a weight ("Oswald SemiBold") sets that weight.
            let (sub_base, sub_weight) = normalize_family(c);
            let semi = c
                .strip_suffix(" SemiCondensed")
                .or(c.strip_suffix(" Semi Condensed"));
            let (family_name, stretch, sub_weight) = match c.strip_suffix(" Condensed") {
                _ if derived_later && c.eq_ignore_ascii_case(&full) => {
                    (c.as_str(), Stretch::Normal, None)
                }
                _ if semi.is_some_and(|b| !self.installed(c) && self.installed(b)) => {
                    (semi.unwrap_or_default(), Stretch::SemiCondensed, None)
                }
                Some(b) if !self.installed(c) && self.installed(b) => (b, Stretch::Condensed, None),
                _ if !self.installed(c) && sub_weight.is_some() && self.installed(&sub_base) => {
                    (sub_base.as_str(), Stretch::Normal, sub_weight)
                }
                _ => (c.as_str(), Stretch::Normal, None),
            };
            if c.is_empty() || !self.installed(family_name) {
                continue;
            }
            // An installed weight-named family ("Montserrat Light") is used as is.
            let w = if let Some(sw) = sub_weight {
                if bold { (sw + 200).clamp(700, 900) } else { sw }
            } else if c.eq_ignore_ascii_case(&full) && name_weight.is_some() {
                if bold { 700 } else { 400 }
            } else {
                weight
            };
            // Fonts with no bold face are drawn regular, as PowerPoint does.
            let no_bold = [&base, &full].map(|f| f.to_lowercase());
            // (A weight named in the family, as Segoe UI Light's 300, still holds.)
            let w = if no_bold.iter().any(|f| NO_BOLD.contains(&f.as_str())) {
                if bold {
                    sub_weight.or(name_weight).unwrap_or(400)
                } else {
                    w
                }
            } else {
                w
            };
            if let Some(id) = self.query_stretch(family_name, w, italic, stretch) {
                if !family_name.eq_ignore_ascii_case(&full) && !full.is_empty() {
                    self.substituted
                        .borrow_mut()
                        .insert((full, family_name.to_string()));
                    substitute = Some(family_name.to_string());
                }
                // A variable font with a `wdth` axis narrows itself (Roboto, Open Sans).
                let wdth = match stretch {
                    Stretch::Condensed => Some(75.0),
                    Stretch::SemiCondensed => Some(87.5),
                    _ => None,
                };
                let inst = self.instance_width(id, w, wdth);
                if em != 1.0 {
                    self.em_scales
                        .borrow_mut()
                        .insert((family.to_lowercase(), inst), em);
                }
                found = Some(inst);
                break;
            }
        }
        self.resolve_cache
            .borrow_mut()
            .insert(key, (found, substitute));
        found
    }

    /// Glyph scale for text in `family` drawn with face `id` (see "Family*0.8").
    pub fn em_scale(&self, family: &str, id: ID) -> f64 {
        let family = family.to_lowercase();
        if let Some(scale) = self.em_scales.borrow().get(&(family.clone(), id)) {
            return *scale;
        }
        // A script fallback drawing for a font that has that script itself: sized to
        // the requested font's glyphs.
        let face = self
            .face(id)
            .map(|f| f.family.to_lowercase())
            .unwrap_or_default();
        let styled = self.db.borrow().face(id).and_then(|info| {
            let bold = info.weight.0 >= 600;
            let italic = info.style != fontdb::Style::Normal;
            STYLED_SCALES
                .iter()
                .find(|(req, fb, b, i, _)| {
                    *req == family && *fb == face && *b == bold && *i == italic
                })
                .map(|s| s.4)
        });
        styled.unwrap_or_else(|| {
            FALLBACK_SCALES
                .iter()
                .find(|(req, fb, _)| *req == family && *fb == face)
                .map_or(1.0, |(_, _, scale)| *scale)
        })
    }

    /// The face to use for `weight`: `id` itself, or for a variable font with a
    /// `wght` axis, a registered instance at that weight.
    fn instance(&self, id: ID, weight: u16) -> ID {
        self.instance_width(id, weight, None)
    }

    /// [`Self::instance`], also narrowed to `wdth` (percent) when the font has that axis.
    fn instance_width(&self, id: ID, weight: u16, wdth: Option<f32>) -> ID {
        let key = (id, weight, wdth.map_or(0, |w| (w * 10.0) as u16));
        if let Some(hit) = self.instances.borrow().get(&key) {
            return *hit;
        }
        let Some(fd) = self.face(id) else { return id };
        let axes: Vec<ttf_parser::VariationAxis> =
            ttf_parser::Face::parse((*fd.data).as_ref(), fd.index)
                .map(|f| f.variation_axes().into_iter().collect())
                .unwrap_or_default();
        // Axis value for a requested setting, or None when it is the default.
        let value = |tag: &[u8; 4], want: Option<f32>| {
            let a = axes
                .iter()
                .find(|a| a.tag == ttf_parser::Tag::from_bytes(tag))?;
            let v = want?.clamp(a.min_value, a.max_value);
            ((v - a.def_value).abs() >= 0.5).then_some(v)
        };
        let wght = value(b"wght", Some(weight as f32));
        let wdth = value(b"wdth", wdth);
        if wght.is_none() && wdth.is_none() {
            self.instances.borrow_mut().insert(key, id);
            return id;
        }
        // Register the same bytes again: a new ID that carries the variation.
        let source = fontdb::Source::Binary(fd.data.clone());
        let ids = self.db.borrow_mut().load_font_source(source);
        let new_id = ids
            .iter()
            .copied()
            .find(|i| self.db.borrow().face(*i).map(|f| f.index) == Some(fd.index))
            .unwrap_or(id);
        if new_id != id {
            // An instance at the default weight still names it, for `postscript_name`.
            let default_wght = axes
                .iter()
                .find(|a| a.tag == ttf_parser::Tag::from_bytes(b"wght"))
                .map(|a| a.def_value);
            if let Some(w) = wght.or(default_wght) {
                self.instance_wght.borrow_mut().insert(new_id, w);
            }
            if let Some(w) = wdth {
                self.instance_wdth.borrow_mut().insert(new_id, w);
            }
        }
        self.instances.borrow_mut().insert(key, new_id);
        new_id
    }

    pub fn face(&self, id: ID) -> Option<Arc<FaceData>> {
        if let Some(f) = self.faces.borrow().get(&id) {
            return Some(f.clone());
        }
        // SAFETY: fontdb memory-maps font files; they are not modified while the
        // process runs (fonts ship read-only in the image).
        let (data, index) = unsafe { self.db.borrow_mut().make_shared_face_data(id) }?;
        let embedded_name = self.embedded_names.borrow().get(&id).cloned();
        let embedded = embedded_name.is_some();
        let family = embedded_name.unwrap_or_else(|| {
            self.db
                .borrow()
                .face(id)
                .and_then(|f| f.families.first().map(|(n, _)| n.clone()))
                .unwrap_or_default()
        });
        let face = ttf_parser::Face::parse((*data).as_ref(), index).ok()?;
        let metrics = metrics_of(&face);
        let wght = self.instance_wght.borrow().get(&id).copied();
        let wdth = self.instance_wdth.borrow().get(&id).copied();
        let fd = Arc::new(FaceData {
            shaper: std::sync::OnceLock::new(),
            data: data.clone(),
            index,
            metrics,
            family,
            wght,
            wdth,
            embedded,
        });
        self.faces.borrow_mut().insert(id, fd.clone());
        Some(fd)
    }

    /// PostScript name of a face (`Carlito-Bold`), the closest analogue of a PDF
    /// font name.
    /// An instance of a variable font is named for its weight ("Raleway-SemiBold"):
    /// the file's own name is its default instance's ("Raleway-Thin").
    pub fn postscript_name(&self, id: ID) -> Option<String> {
        let db = self.db.borrow();
        let face = db.face(id)?;
        if let Some(w) = self.instance_wght.borrow().get(&id) {
            let family: String = face
                .families
                .first()
                .map(|(n, _)| n.chars().filter(|c| !c.is_whitespace()).collect())
                .unwrap_or_default();
            if !family.is_empty() {
                let italic = face.style != Style::Normal;
                let width = match self.instance_wdth.borrow().get(&id) {
                    Some(w) if *w < 80.0 => "Condensed",
                    Some(_) => "SemiCondensed",
                    None => "",
                };
                return Some(format!("{family}{width}-{}", weight_style_name(*w, italic)));
            }
        }
        Some(face.post_script_name.clone()).filter(|n| !n.is_empty())
    }

    pub fn has_glyph(&self, id: ID, ch: char) -> bool {
        if let Some(hit) = self.glyph_cache.borrow().get(&(id, ch)) {
            return *hit;
        }
        let Some(fd) = self.face(id) else {
            return false;
        };
        let has = ttf_parser::Face::parse((*fd.data).as_ref(), fd.index)
            .ok()
            .and_then(|f| f.glyph_index(ch))
            .is_some();
        self.glyph_cache.borrow_mut().insert((id, ch), has);
        has
    }

    /// A face that maps `ch`, preferring one close to `bold`/`italic`.
    pub fn fallback_for(&self, ch: char, bold: bool, italic: bool) -> Option<ID> {
        self.fallback_weighted(ch, if bold { 700 } else { 400 }, italic)
    }

    /// [`Self::fallback_for`] at a CSS weight (see [`requested_weight`]): a run in a
    /// heavy face whose substitute lacks the script ("Arial Black" → Archivo Black has
    /// no Cyrillic) keeps its weight instead of dropping to the regular fallback.
    pub fn fallback_weighted(&self, ch: char, weight: u16, italic: bool) -> Option<ID> {
        let heavy: &[&str] = if weight >= 800 { HEAVY_FALLBACKS } else { &[] };
        for fam in script_fallbacks(ch)
            .iter()
            .chain(heavy)
            .chain(FALLBACK_FAMILIES)
        {
            if !self.installed(fam) {
                continue;
            }
            if let Some(id) = self.query_weight(fam, weight, italic) {
                if !self.has_glyph(id, ch) {
                    continue;
                }
                let inst = self.instance(id, weight);
                // A heavy-list family installed without a black face is no better
                // than the general list.
                if heavy.contains(fam) && self.weight_of(inst) < 800 {
                    continue;
                }
                return Some(inst);
            }
        }
        self.scan_for(ch)
    }

    /// Drawn weight of a face: its variable-font instance's `wght`, else its OS/2 class.
    fn weight_of(&self, id: ID) -> u16 {
        if let Some(w) = self.instance_wght.borrow().get(&id) {
            return *w as u16;
        }
        self.db.borrow().face(id).map_or(400, |f| f.weight.0)
    }

    /// Any installed face with a glyph for `ch` (the first in database order).
    fn scan_for(&self, ch: char) -> Option<ID> {
        if let Some(hit) = self.scan_cache.borrow().get(&ch) {
            return *hit;
        }
        // Private-use and control characters have no meaningful cross-font match.
        let found = if ch.is_control() || ('\u{E000}'..='\u{F8FF}').contains(&ch) {
            None
        } else {
            let db = self.db.borrow();
            let embedded = self.embedded_names.borrow();
            // Read each face transiently; only the winner gets loaded and cached.

            db.faces()
                .map(|f| f.id)
                .filter(|id| !embedded.contains_key(id))
                .find(|id| {
                    db.with_face_data(*id, |data, index| {
                        ttf_parser::Face::parse(data, index)
                            .ok()
                            .and_then(|f| f.glyph_index(ch))
                            .is_some()
                    })
                    .unwrap_or(false)
                })
        };
        self.scan_cache.borrow_mut().insert(ch, found);
        found
    }

    /// Registers a font embedded in the document for `typeface` in the slot given
    /// by `bold`/`italic`, whatever the font's own name table says (embedded
    /// subsets often carry odd names). The face gets a private family name so it
    /// never shadows an installed font. Returns false when `sfnt` is not a font.
    pub fn add_embedded(&self, typeface: &str, bold: bool, italic: bool, sfnt: Vec<u8>) -> bool {
        let typeface = typeface.trim();
        let key = typeface.to_lowercase();
        let slot = bold as usize | (italic as usize) << 1;
        if self
            .embedded
            .borrow()
            .get(&key)
            .is_some_and(|s| s[slot].is_some())
        {
            return true;
        }
        // Built by hand rather than by fontdb's loader, which rejects faces
        // without a name table (common in embedded subsets).
        let post_script_name = match ttf_parser::Face::parse(&sfnt, 0) {
            Ok(face) if !typeface.is_empty() => face
                .names()
                .into_iter()
                .find(|n| n.name_id == ttf_parser::name_id::POST_SCRIPT_NAME)
                .and_then(|n| n.to_string())
                .unwrap_or_default(),
            _ => return false,
        };
        let info = fontdb::FaceInfo {
            id: ID::dummy(),
            source: fontdb::Source::Binary(Arc::new(sfnt)),
            index: 0,
            families: vec![(
                format!("{EMBEDDED_MARK}{key}#{slot}"),
                fontdb::Language::English_UnitedStates,
            )],
            post_script_name,
            style: if italic { Style::Italic } else { Style::Normal },
            weight: Weight(if bold { 700 } else { 400 }),
            stretch: Stretch::Normal,
            monospaced: false,
        };
        let id = self.db.borrow_mut().push_face_info(info);
        self.embedded.borrow_mut().entry(key).or_default()[slot] = Some(id);
        self.resolve_cache.borrow_mut().clear();
        self.embedded_names
            .borrow_mut()
            .insert(id, typeface.to_string());
        true
    }

    /// Whether `render.fontSubstitutions` names a replacement for `family`.
    pub fn has_user_substitute(&self, family: &str) -> bool {
        self.user_subs.contains_key(&family.trim().to_lowercase())
    }

    /// For an embedded face: whether its slot is (bold, italic). PowerPoint draws a
    /// style the deck did not embed by emboldening/slanting the face it has.
    pub fn embedded_style(&self, id: ID) -> Option<(bool, bool)> {
        self.embedded.borrow().values().find_map(|slots| {
            let s = slots.iter().position(|f| *f == Some(id))?;
            Some((s & 1 == 1, s & 2 == 2))
        })
    }

    /// Whether a face draws slanted glyphs itself: an embedded face by its slot,
    /// an installed one by its italic/oblique style.
    pub fn is_italic(&self, id: ID) -> bool {
        if let Some((_, italic)) = self.embedded_style(id) {
            return italic;
        }
        self.db
            .borrow()
            .face(id)
            .is_some_and(|f| f.style != Style::Normal)
    }

    /// The embedded face for `typeface`, closest to the requested style: the
    /// exact slot, then same boldness, then same slant, then any.
    fn embedded_face(&self, typeface: &str, bold: bool, italic: bool) -> Option<ID> {
        let map = self.embedded.borrow();
        let slots = map.get(&typeface.trim().to_lowercase())?;
        let want = bold as usize | (italic as usize) << 1;
        [want, want & 1, want & 2, 0, 1, 2, 3]
            .iter()
            .find_map(|s| slots[*s])
    }

    /// Glyph outline in font units (y up), cached.
    pub fn outline(&self, id: ID, gid: u16) -> Option<tiny_skia::Path> {
        if let Some(p) = self.outlines.borrow().get(&(id, gid)) {
            return p.clone();
        }
        let fd = self.face(id)?;
        let mut face = ttf_parser::Face::parse((*fd.data).as_ref(), fd.index).ok()?;
        if let Some(w) = fd.wght {
            face.set_variation(ttf_parser::Tag::from_bytes(b"wght"), w);
        }
        if let Some(w) = fd.wdth {
            face.set_variation(ttf_parser::Tag::from_bytes(b"wdth"), w);
        }
        let mut builder = OutlineBuilder(tiny_skia::PathBuilder::new());
        let path = face
            .outline_glyph(ttf_parser::GlyphId(gid), &mut builder)
            .and_then(|_| builder.0.finish());
        self.outlines.borrow_mut().insert((id, gid), path.clone());
        path
    }
}

/// PostScript style suffix for a CSS weight: 600 → "SemiBold", 400 italic → "Italic".
fn weight_style_name(wght: f32, italic: bool) -> String {
    const NAMES: [&str; 9] = [
        "Thin",
        "ExtraLight",
        "Light",
        "Regular",
        "Medium",
        "SemiBold",
        "Bold",
        "ExtraBold",
        "Black",
    ];
    let i = ((wght / 100.0).round() as usize).clamp(1, 9) - 1;
    match (NAMES[i], italic) {
        ("Regular", true) => "Italic".to_string(),
        (n, true) => format!("{n}Italic"),
        (n, false) => n.to_string(),
    }
}

fn metrics_of(face: &ttf_parser::Face) -> Metrics {
    let upem = face.units_per_em() as f32;
    let (mut ascent, mut descent) = (face.ascender() as f32, -(face.descender() as f32));
    if let Some(os2) = face.tables().os2 {
        let wa = os2.windows_ascender() as f32;
        let wd = os2.windows_descender() as f32;
        if wa > 0.0 && wd.abs() > 0.0 {
            ascent = wa;
            descent = wd.abs();
        }
    }
    let ul = face.underline_metrics();
    let so = face.strikeout_metrics();
    Metrics {
        units_per_em: upem,
        ascent,
        descent,
        underline_pos: ul.map(|m| m.position as f32).unwrap_or(-0.1 * upem),
        underline_thickness: ul
            .map(|m| m.thickness as f32)
            .unwrap_or(0.05 * upem)
            .max(1.0),
        strikeout_pos: so.map(|m| m.position as f32).unwrap_or(0.3 * upem),
    }
}

struct OutlineBuilder(tiny_skia::PathBuilder);

impl ttf_parser::OutlineBuilder for OutlineBuilder {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to(x, y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.line_to(x, y);
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.0.quad_to(x1, y1, x, y);
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.0.cubic_to(x1, y1, x2, y2, x, y);
    }
    fn close(&mut self) {
        self.0.close();
    }
}

/// True when the built-in table knows a metric-compatible replacement.
pub fn has_substitute(family: &str) -> bool {
    let lower = family.trim().to_lowercase();
    SUBSTITUTIONS.iter().any(|(k, _)| *k == lower)
}

/// Fallback hints from a PANOSE classification: (bold, serif). Byte 3 is the
/// weight (7+ = demi/bold and heavier); byte 2 is the serif style (11-13 are
/// sans-serif, 2-10 serif) for Latin text faces (byte 1 = 2).
pub fn panose_hints(panose: &str) -> (bool, bool) {
    let bytes: Vec<u8> = (0..panose.len().min(20))
        .step_by(2)
        .filter_map(|i| u8::from_str_radix(panose.get(i..i + 2)?, 16).ok())
        .collect();
    if bytes.len() < 3 || bytes[0] != 2 {
        return (false, false);
    }
    (bytes[2] >= 7, (2..=10).contains(&bytes[1]))
}

/// Maps a character in a symbol font (Wingdings, Symbol, ...) to a Unicode
/// equivalent. Symbol fonts address glyphs by byte value, often written as
/// U+F0xx private-use code points. Returns `None` for non-symbol fonts.
pub fn map_symbol_char(font: &str, ch: char) -> Option<char> {
    crate::office::pptx::symbols::map_symbol_char(font, ch)
}

#[cfg(test)]
mod tests;
