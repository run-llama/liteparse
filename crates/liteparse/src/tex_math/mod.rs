//! LaTeX from the glyphs of TeX-generated PDF pages, without a model.
//!
//! Math glyphs are recognised by their font (Computer Modern / Latin Modern /
//! tx / px / AMS / Euler / rsfs math fonts); glyphs with no Unicode are read
//! through TeX's fixed encodings (OML / OMS / OMX); sub/superscripts come
//! from glyph size and baseline shift; fractions, overlines and square roots
//! come from thin rules (see [`structures`]).
//!
//! Only born-digital TeX is covered: a page needs a TeX-encoded math font
//! before anything is emitted. Ported from the `bench/math_proto` prototype;
//! comments marked "Note:" carry its traps.

mod encodings;
mod structures;

use std::collections::HashSet;

pub(crate) use encodings::Enc;
pub(crate) use structures::Rule;

/// What PDFium reported for one glyph's text.
#[derive(Debug, Clone, Copy)]
pub(crate) enum RawText {
    /// A clean Unicode mapping.
    Unicode(char),
    /// No Unicode for the glyph (PDFium's map error): the raw char code.
    Code(u32),
}

/// One page glyph, as collected by `extract.rs`. Coordinates are viewport
/// space (y down); `origin_y` is the baseline.
#[derive(Debug, Clone)]
pub(crate) struct RawChar {
    pub(crate) index: i32,
    pub(crate) text: RawText,
    /// Font base name as in the PDF (subset prefix included).
    pub(crate) font_name: std::rc::Rc<str>,
    /// Effective size (font size x text-matrix scale).
    pub(crate) size: f32,
    /// Font descent at `size` (negative, points).
    pub(crate) descent: f32,
    pub(crate) origin_y: f32,
    pub(crate) x0: f32,
    pub(crate) x1: f32,
    /// Tight glyph box (top, bottom), viewport y.
    pub(crate) ink: Option<(f32, f32)>,
    /// Font box (top, bottom), viewport y: the box text items use.
    pub(crate) loose: (f32, f32),
}

/// One math span found on a page.
#[derive(Debug, Clone)]
pub(crate) struct MathSpan {
    pub(crate) tex: String,
    /// Viewport-space box over every glyph in the span.
    pub(crate) x0: f32,
    pub(crate) x1: f32,
    pub(crate) top: f32,
    pub(crate) bottom: f32,
    /// Size of the span's base glyphs.
    pub(crate) size: f32,
    /// Char indices the span consumed.
    pub(crate) chars: Vec<i32>,
}

impl MathSpan {
    /// Whether the span carries structure plain text can't show: scripts,
    /// fractions, roots, accents, or a math alphabet. Note: a lone ± or μ
    /// (or plain Greek) stays text, so table cells like "0.11 ± 0.02" keep
    /// their form.
    pub(crate) fn has_structure(&self) -> bool {
        const MARKS: [&str; 19] = [
            r"\frac",
            r"\sqrt",
            r"\overline",
            r"\mathbb",
            r"\mathcal",
            r"\mathfrak",
            r"\mathscr",
            r"\hat",
            r"\tilde",
            r"\bar",
            r"\dot",
            r"\ddot",
            r"\acute",
            r"\grave",
            r"\check",
            r"\breve",
            r"\vec",
            r"\widehat",
            r"\widetilde",
        ];
        self.tex.contains(['_', '^']) || MARKS.iter().any(|m| self.tex.contains(m))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Role {
    Mi,
    Sy,
    Bb,
    Ex,
    Frak,
    Scr,
    Cal,
}

/// `prefix` + `rest` where the prefix is one of the tx / px font families.
fn tx_px<'a>(f: &'a str, prefixes: &[&str]) -> impl Iterator<Item = &'a str> {
    prefixes
        .iter()
        .filter_map(move |p| f.strip_prefix(p))
        .collect::<Vec<_>>()
        .into_iter()
}

/// Math font families (subset prefix and size stripped, upper case), checked
/// in order: (role, encoding). The encoding is set when the font follows
/// cmmi / cmsy / cmex (OML / OMS / OMX): a glyph without Unicode is then read
/// through that table, and a capital in an OMS font is `\mathcal`.
fn font_info(f: &str) -> (Option<Role>, Option<Enc>) {
    const TXPX: [&str; 4] = ["TX", "RTX", "PX", "RPX"];
    const TXPX_NEW: [&str; 5] = ["TX", "RTX", "PX", "RPX", "NEWTX"];
    const TXPX_MI: [&str; 7] = ["TX", "RTX", "PX", "RPX", "NEWTX", "ZX", "NTX"];
    let mi_oml = matches!(
        f,
        "CMMI" | "CMMIB" | "LMMATHITALIC" | "MLMMATHITALIC" | "CMBRMI"
    ) || tx_px(f, &TXPX_MI).any(|r| r.starts_with("MI") || r.starts_with("BMI"));
    if mi_oml {
        return (Some(Role::Mi), Some(Enc::Oml));
    }
    if matches!(f, "EURM" | "EURB" | "PAZOMATH-ITALIC" | "STIXMATH-ITALIC")
        || f.contains("MATHITALIC")
    {
        return (Some(Role::Mi), None);
    }
    if matches!(f, "CMSY" | "CMBSY" | "LMMATHSYMBOLS" | "TXBSY" | "TXBSYS")
        || tx_px(f, &TXPX_NEW).any(|r| r == "SY" || r == "SYS")
    {
        return (Some(Role::Sy), Some(Enc::Oms));
    }
    if tx_px(f, &TXPX).any(|r| r == "SYB")
        || matches!(f, "MSBM" | "BBOLD")
        || ["DSROM", "BBM", "PAZOMATHBLACKBOARDBOLD"]
            .iter()
            .any(|p| f.starts_with(p))
    {
        return (Some(Role::Bb), None);
    }
    if matches!(f, "CMEX" | "LMMATHEXTENSION" | "EUEX" | "YHCMEX")
        || f.contains("EXTENSION")
        || tx_px(f, &TXPX_NEW).any(|r| matches!(r, "EX" | "EXA" | "EXS" | "EXAS"))
    {
        return (Some(Role::Ex), Some(Enc::Omx));
    }
    if f == "MSAM" {
        return (Some(Role::Sy), Some(Enc::Msam));
    }
    if matches!(f, "EUFM" | "EUFB") {
        return (Some(Role::Frak), None);
    }
    if f.starts_with("RSFS") || f.starts_with("STIXMATHSCRIPT") {
        return (Some(Role::Scr), None);
    }
    if matches!(f, "EUSM" | "EUSB") {
        return (Some(Role::Cal), None);
    }
    let sy = tx_px(f, &TXPX).any(|r| r == "SYA" || r == "SYC")
        || matches!(
            f,
            "TEX-MATH"
                | "TEX-MATHA"
                | "TEX-MATHB"
                | "TEX-MATHC"
                | "TEX-MATHX"
                | "LASY"
                | "WASY"
                | "OAMATHSYMBOLS"
                | "STIXMATH-REGULAR"
        )
        || ["MNSYMBOL", "STANDARDSYML", "ESINT", "STMARY"]
            .iter()
            .any(|p| f.starts_with(p))
        || f.ends_with("SYMBOL")
        || f.ends_with("SYMBOL-")
        || f.strip_prefix("MATHDESIGN")
            .is_some_and(|r| r.contains("SYMBOL"));
    if sy {
        return (Some(Role::Sy), None);
    }
    (None, None)
}

/// Font family: subset prefix, a `-Regular` suffix and size digits stripped,
/// upper case ("ABCDEF+CMMI10" -> "CMMI").
fn family(font_name: &str) -> String {
    let name = font_name.rsplit_once('+').map_or(font_name, |(_, n)| n);
    let mut up = name.to_uppercase();
    if up.ends_with("-REGULAR") {
        up.truncate(up.len() - "-REGULAR".len());
    }
    let trimmed = up.trim_end_matches(|c: char| c.is_ascii_digit()).len();
    up.truncate(trimmed);
    up
}

const TEXT_FAMS: [&str; 4] = ["CMR", "CMBX", "CMSL", "CMTI"];

/// Prefix marking a glyph text that is already LaTeX (decoded from a TeX
/// encoding, or a folded structure).
const PRE: char = '\0';

const OPERATORS: [&str; 35] = [
    "sin", "cos", "tan", "log", "ln", "exp", "max", "min", "arg", "lim", "det", "tr", "sup", "inf",
    "ker", "dim", "deg", "mod", "sgn", "sinh", "cosh", "tanh", "arcsin", "arccos", "arctan",
    "liminf", "limsup", "Pr", "gcd", "lcm", "Re", "Im", "diag", "rank", "span",
];

fn is_operator(w: &str) -> bool {
    OPERATORS.contains(&w)
}

fn greek(t: &str) -> Option<&'static str> {
    Some(match t {
        "α" => "alpha",
        "β" => "beta",
        "γ" => "gamma",
        "δ" => "delta",
        "ϵ" => "epsilon",
        "ε" => "varepsilon",
        "ǫ" => "epsilon",
        "ζ" => "zeta",
        "η" => "eta",
        "θ" => "theta",
        "ϑ" => "vartheta",
        "ι" => "iota",
        "κ" => "kappa",
        "λ" => "lambda",
        "µ" | "μ" => "mu",
        "ν" => "nu",
        "ξ" => "xi",
        "π" => "pi",
        "ϖ" => "varpi",
        "ρ" => "rho",
        "ϱ" => "varrho",
        "σ" => "sigma",
        "ς" => "varsigma",
        "τ" => "tau",
        "υ" => "upsilon",
        "φ" | "ϕ" => "phi",
        "χ" => "chi",
        "ψ" => "psi",
        "ω" => "omega",
        "Γ" => "Gamma",
        "∆" | "Δ" => "Delta",
        "Θ" => "Theta",
        "Λ" => "Lambda",
        "Ξ" => "Xi",
        "Π" => "Pi",
        "Σ" => "Sigma",
        "Υ" => "Upsilon",
        "Φ" => "Phi",
        "Ψ" => "Psi",
        "Ω" | "\u{2126}" => "Omega",
        _ => return None,
    })
}

fn symbol(t: &str) -> Option<&'static str> {
    Some(match t {
        "∈" => r"\in",
        "∉" => r"\notin",
        "≤" | "⩽" => r"\leq",
        "≥" | "⩾" => r"\geq",
        "≠" => r"\neq",
        "≈" => r"\approx",
        "∼" | "~" => r"\sim",
        "≃" => r"\simeq",
        "≡" => r"\equiv",
        "∝" => r"\propto",
        "≪" => r"\ll",
        "≫" => r"\gg",
        "⊂" => r"\subset",
        "⊆" => r"\subseteq",
        "⊃" => r"\supset",
        "⊇" => r"\supseteq",
        "∪" => r"\cup",
        "∩" => r"\cap",
        "∞" => r"\infty",
        "∂" => r"\partial",
        "∇" => r"\nabla",
        "±" => r"\pm",
        "∓" => r"\mp",
        "×" => r"\times",
        "·" | "⋅" => r"\cdot",
        "÷" => r"\div",
        "∗" | "˚" => r"\ast",
        "◦" | "∘" => r"\circ",
        "→" => r"\to",
        "←" => r"\leftarrow",
        "↔" => r"\leftrightarrow",
        "⇒" => r"\Rightarrow",
        "⇐" => r"\Leftarrow",
        "⇔" => r"\Leftrightarrow",
        "↦" => r"\mapsto",
        "∀" => r"\forall",
        "∃" => r"\exists",
        "¬" => r"\neg",
        "∧" => r"\wedge",
        "∨" => r"\vee",
        "∅" => r"\emptyset",
        "∑" => r"\sum",
        "∏" => r"\prod",
        "∫" => r"\int",
        "∮" => r"\oint",
        "⊗" => r"\otimes",
        "⊕" => r"\oplus",
        "∥" | "‖" => r"\|",
        "⟨" => r"\langle",
        "⟩" => r"\rangle",
        "⌊" => r"\lfloor",
        "⌋" => r"\rfloor",
        "⌈" => r"\lceil",
        "⌉" => r"\rceil",
        "…" => r"\ldots",
        "⋯" => r"\cdots",
        "−" => "-",
        "′" => "'",
        "{" => r"\{",
        "}" => r"\}",
        "\\" | "∖" => r"\setminus",
        "†" => r"\dagger",
        "⊤" => r"\top",
        "⊥" => r"\perp",
        "ℓ" => r"\ell",
        "ı" => r"\imath",
        "ȷ" => r"\jmath",
        "≺" => r"\prec",
        "≻" => r"\succ",
        "⪯" | "≼" => r"\preceq",
        "♯" | "#" => r"\#",
        "%" => r"\%",
        "&" => r"\&",
        "_" => r"\_",
        "^" => r"\hat{}",
        "⊢" => r"\vdash",
        "|" => "|",
        "√" => r"\surd",
        "ℝ" => r"\mathbb{R}",
        "ℕ" => r"\mathbb{N}",
        "ℤ" => r"\mathbb{Z}",
        "ℚ" => r"\mathbb{Q}",
        "ℂ" => r"\mathbb{C}",
        "⊊" => r"\subsetneq",
        "≜" => r"\triangleq",
        "≔" => ":=",
        "⟶" => r"\longrightarrow",
        "⟹" => r"\Longrightarrow",
        "↪" => r"\hookrightarrow",
        "⇀" => r"\rightharpoonup",
        "\u{0338}" => "NOT",
        "\u{f8eb}" => "(",
        "\u{f8ee}" => "[",
        "\u{f8f1}" => r"\{",
        "\u{f8f6}" => ")",
        "\u{f8f9}" => "]",
        "\u{f8fc}" => r"\}",
        "\u{f8ec}" | "\u{f8ed}" | "\u{f8ef}" | "\u{f8f0}" | "\u{f8f2}" | "\u{f8f3}"
        | "\u{f8f4}" | "\u{f8f5}" | "\u{f8f7}" | "\u{f8f8}" | "\u{f8fa}" | "\u{f8fb}"
        | "\u{f8fd}" | "\u{f8fe}" | "\u{f8e5}" => "",
        "⋆" => r"\star",
        "⋄" => r"\diamond",
        "□" => r"\square",
        "△" => r"\triangle",
        _ => return None,
    })
}

const TEXT_MATH_OK: &str = "0123456789()[]{}=+-−,;:!/|.'<>*′";

/// One glyph in the math pass. `top` / `bottom` are viewport y (down),
/// `base` is the baseline in y-up page space (`page height - y`), as in the
/// prototype.
#[derive(Debug, Clone)]
pub(crate) struct G {
    pub(crate) t: String,
    pub(crate) role: Option<Role>,
    pub(crate) fam: String,
    pub(crate) size: f32,
    pub(crate) base: f32,
    pub(crate) x0: f32,
    pub(crate) x1: f32,
    pub(crate) top: f32,
    pub(crate) bottom: f32,
    /// Font box (top, bottom), viewport y, for the span's item box.
    pub(crate) loose: (f32, f32),
    /// Accent commands opened before this glyph, and closed after it.
    pub(crate) pre: String,
    pub(crate) post: String,
    /// Source char indices (several for a folded structure).
    pub(crate) src: Vec<i32>,
}

impl G {
    fn cx(&self) -> f32 {
        (self.x0 + self.x1) / 2.0
    }
    fn enc(&self) -> Option<Enc> {
        font_info(&self.fam).1
    }
}

/// Python `str.isalpha` on a glyph text.
fn is_alpha(t: &str) -> bool {
    !t.is_empty() && t.chars().all(char::is_alphabetic)
}

fn is_ascii_upper(t: &str) -> bool {
    !t.is_empty()
        && t.is_ascii()
        && t.chars().any(|c| c.is_ascii_uppercase())
        && !t.chars().any(|c| c.is_ascii_lowercase())
}

/// Python `statistics.median`.
pub(crate) fn median(mut v: Vec<f32>) -> f32 {
    v.sort_by(f32::total_cmp);
    let n = v.len();
    if n == 0 {
        return 0.0;
    }
    if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    }
}

fn max_size(gs: &[G], idx: &[usize]) -> f32 {
    idx.iter().map(|&i| gs[i].size).fold(f32::MIN, f32::max)
}

/// Turn collected chars into math-pass glyphs.
fn glyphs(chars: &[RawChar], page_h: f32) -> Vec<G> {
    let mut out = Vec::new();
    let mut fam_cache: std::collections::HashMap<*const u8, (String, Option<Role>, Option<Enc>)> =
        std::collections::HashMap::new();
    for c in chars {
        let (fam, role, enc) = fam_cache
            .entry(c.font_name.as_ptr())
            .or_insert_with(|| {
                let fam = family(&c.font_name);
                let (r, e) = font_info(&fam);
                (fam, r, e)
            })
            .clone();
        // Note: PDFium flags glyphs it could not map; only those are read
        // through the TeX encoding (a ToUnicode'd "{" must stay "{").
        let t = match c.text {
            RawText::Unicode(ch) => {
                if ch.is_whitespace() {
                    continue;
                }
                ch.to_string()
            }
            // Note: letters and the ASCII a cmmi font draws as itself stay
            // plain text.
            RawText::Code(code)
                if enc == Some(Enc::Oml)
                    && char::from_u32(code)
                        .is_some_and(|ch| ch.is_ascii_alphabetic() || ".,</>".contains(ch)) =>
            {
                char::from_u32(code).expect("ascii").to_string()
            }
            RawText::Code(code) => match enc.and_then(|e| encodings::decode(e, code)) {
                Some(dec) => format!("{PRE}{dec}"),
                None => match char::from_u32(code).filter(|ch| ch.is_ascii_graphic()) {
                    Some(ch) => ch.to_string(),
                    None => continue,
                },
            },
        };
        if t.is_empty() {
            continue;
        }
        let mut role = role;
        if role.is_none() && TEXT_FAMS.contains(&fam.as_str()) && greek(&t).is_some() {
            role = Some(Role::Mi); // upright capital Greek is set in the roman font
        }
        let base = page_h - c.origin_y;
        // Note: the ink box, not an em box. TeX sets \hat{x}'s accent on
        // x's own baseline, so only ink tells the accent sits above; a
        // radical's ink top is where its rule starts. (+4 points over the
        // prototype's font-descriptor boxes on the arXiv math tests.)
        let (top, bottom) = c.ink.unwrap_or_else(|| {
            let bottom_up = base + c.descent;
            (page_h - (bottom_up + c.size), page_h - bottom_up)
        });
        out.push(G {
            t,
            role,
            fam,
            size: c.size,
            base,
            x0: c.x0,
            x1: c.x1,
            top,
            bottom,
            loose: c.loose,
            pre: String::new(),
            post: String::new(),
            src: vec![c.index],
        });
    }
    out
}

struct Base {
    y: f32,
    size: f32,
    gs: Vec<usize>,
    ext: Option<(f32, f32)>,
}

/// Group glyphs into lines: full-size glyphs set the line baselines, smaller
/// ones (scripts) attach to the nearest line whose band they fall in.
fn lines(all: &mut [G], live: &[usize]) -> Vec<Vec<usize>> {
    if live.is_empty() {
        return Vec::new();
    }
    let body = median(live.iter().map(|&i| all[i].size).collect());
    let mut big: Vec<usize> = live
        .iter()
        .copied()
        .filter(|&i| all[i].size >= 0.85 * body)
        .collect();
    big.sort_by(|&a, &b| all[b].base.total_cmp(&all[a].base));
    let mut bases: Vec<Base> = Vec::new();
    for &g in &big {
        let gb = all[g].base;
        let gsz = all[g].size;
        match bases.iter_mut().find(|b| (b.y - gb).abs() < 0.25 * gsz) {
            Some(b) => b.gs.push(g),
            None => bases.push(Base {
                y: gb,
                size: gsz,
                gs: vec![g],
                ext: None,
            }),
        }
    }
    // Scripts attach to the nearest baseline among the rows whose x-extent
    // holds them (one baseline can hold a row of each column on a
    // two-column page).
    for b in &mut bases {
        let x0 = b.gs.iter().map(|&i| all[i].x0).fold(f32::MAX, f32::min);
        let x1 = b.gs.iter().map(|&i| all[i].x1).fold(f32::MIN, f32::max);
        b.ext = Some((x0 - 1.0 * b.size, x1 + 1.5 * b.size));
    }
    for &g in live {
        if all[g].role != Some(Role::Ex) {
            continue;
        }
        // Note: a cmex glyph hangs from its origin; its line is the nearest
        // baseline below that.
        let xc = all[g].cx();
        let (gb, gs) = (all[g].base, all[g].size);
        let mut row: Option<usize> = None;
        for (k, b) in bases.iter().enumerate() {
            let Some((e0, e1)) = b.ext else { continue };
            let ok = e0 <= xc
                && xc <= e1
                && gb - 3.0 * gs <= b.y
                && b.y <= gb + 0.3 * gs
                && b.gs.first().is_some_and(|&f| all[f].role != Some(Role::Ex));
            if ok && row.is_none_or(|r| b.y > bases[r].y) {
                row = Some(k);
            }
        }
        if let Some(r) = row {
            for b in &mut bases {
                if let Some(p) = b.gs.iter().position(|&x| x == g) {
                    b.gs.remove(p);
                }
            }
            all[g].base = bases[r].y;
            all[g].size = bases[r].size;
            bases[r].gs.push(g);
        }
    }
    bases.retain(|b| !b.gs.is_empty());
    for &g in live {
        if all[g].size >= 0.85 * body || all[g].role == Some(Role::Ex) {
            continue;
        }
        let xc = all[g].cx();
        let gb = all[g].base;
        let mut best: Option<usize> = None;
        for (k, b) in bases.iter().enumerate() {
            let Some((e0, e1)) = b.ext else { continue };
            let d = gb - b.y;
            if !(-0.6 * b.size < d && d < 0.8 * b.size && e0 <= xc && xc <= e1) {
                continue;
            }
            if best.is_none_or(|r| (gb - b.y).abs() < (gb - bases[r].y).abs()) {
                best = Some(k);
            }
        }
        match best {
            Some(k) => bases[k].gs.push(g),
            None => bases.push(Base {
                y: gb,
                size: all[g].size,
                gs: vec![g],
                ext: None,
            }),
        }
    }
    // Note: then split each row at wide gaps nothing fills: the gutter
    // between two columns.
    let mut split: Vec<(f32, Vec<usize>)> = Vec::new();
    for b in &bases {
        let mut row = b.gs.clone();
        row.sort_by(|&a, &c| all[a].x0.total_cmp(&all[c].x0));
        let mut cur = vec![row[0]];
        let mut right = all[row[0]].x1;
        for &g in &row[1..] {
            if all[g].x0 - right > 1.5 * b.size {
                split.push((b.y, std::mem::take(&mut cur)));
            }
            cur.push(g);
            right = right.max(all[g].x1);
        }
        split.push((b.y, cur));
    }
    split.sort_by(|a, b| {
        b.0.total_cmp(&a.0)
            .then(all[a.1[0]].x0.total_cmp(&all[b.1[0]].x0))
    });
    split
        .into_iter()
        .map(|(_, mut gs)| {
            gs.sort_by(|&a, &b| all[a].x0.total_cmp(&all[b].x0));
            gs
        })
        .collect()
}

fn accent_name(t: &str) -> Option<&'static str> {
    Some(match t {
        "ˆ" | "^" => "hat",
        "˜" | "~" => "tilde",
        "¯" => "bar",
        "˙" => "dot",
        "¨" => "ddot",
        "´" => "acute",
        "`" => "grave",
        "ˇ" => "check",
        "˘" => "breve",
        "\u{20d7}" => "vec",
        _ => return None,
    })
}

fn accent_of(g: &G) -> Option<String> {
    if let Some(rest) = g.t.strip_prefix("\0ACC:") {
        return Some(rest.to_string());
    }
    if (g.role.is_none() || g.role == Some(Role::Mi))
        && let Some(a) = accent_name(&g.t)
        && (g.role.is_none() || "˜¯˙¨ˆ\u{20d7}".contains(g.t.as_str()))
    {
        return Some(a.to_string());
    }
    None
}

/// Drop accent glyphs from a span, wrapping the glyphs under each in
/// `\hat{..}` etc.
fn attach_accents(all: &mut [G], sp: &[usize]) -> Vec<usize> {
    let accs: Vec<usize> = sp
        .iter()
        .copied()
        .filter(|&i| accent_of(&all[i]).is_some())
        .collect();
    if accs.is_empty() {
        return sp.to_vec();
    }
    let rest: Vec<usize> = sp
        .iter()
        .copied()
        .filter(|&i| accent_of(&all[i]).is_none())
        .collect();
    for &a in &accs {
        let (ax0, ax1, atop, asz) = (all[a].x0, all[a].x1, all[a].top, all[a].size);
        let cx = (ax0 + ax1) / 2.0;
        let name = accent_of(&all[a]).unwrap_or_default();
        let wide = name.starts_with("wide");
        let mut under: Vec<usize> = if wide {
            let rest_max = max_size(all, &rest);
            rest.iter()
                .copied()
                .filter(|&g| {
                    let gg = &all[g];
                    gg.top > atop
                        && ax0 - 0.1 * asz <= gg.cx()
                        && gg.cx() <= ax1 + 0.1 * asz
                        && gg.size >= 0.9 * rest_max
                })
                .collect()
        } else {
            rest.iter()
                .copied()
                .filter(|&g| {
                    let gg = &all[g];
                    gg.top > atop + 0.1 * asz
                        && gg.x0 - 0.15 * asz <= cx
                        && cx <= gg.x1 + 0.15 * asz
                })
                .collect()
        };
        if under.is_empty() {
            continue;
        }
        under.sort_by(|&a, &b| all[a].x0.total_cmp(&all[b].x0));
        if !wide {
            let pick = under
                .iter()
                .copied()
                .reduce(|best, g| {
                    if (all[g].cx() - cx).abs() < (all[best].cx() - cx).abs() {
                        g
                    } else {
                        best
                    }
                })
                .expect("non-empty");
            under = vec![pick];
        }
        let first = under[0];
        let last = *under.last().expect("non-empty");
        all[first].pre.push_str(&format!("\\{name}{{"));
        all[last].post = format!("}}{}", all[last].post);
    }
    rest
}

fn tex_char(g: &G) -> String {
    format!("{}{}{}", g.pre, tex_char_inner(g), g.post)
}

fn cmd_space(s: &str) -> String {
    if s.starts_with('\\') && s.chars().last().is_some_and(char::is_alphabetic) {
        format!("{s} ")
    } else {
        s.to_string()
    }
}

fn tex_char_inner(g: &G) -> String {
    let t = g.t.as_str();
    if let Some(s) = t.strip_prefix(PRE) {
        return cmd_space(s);
    }
    if TEXT_FAMS.contains(&g.fam.as_str())
        && let Some(n) = greek(t)
    {
        return format!("\\{n} ");
    }
    if g.role == Some(Role::Bb) && is_alpha(t) {
        return format!(r"\mathbb{{{}}}", t.to_uppercase());
    }
    if g.role == Some(Role::Frak) && is_alpha(t) {
        return format!(r"\mathfrak{{{t}}}");
    }
    if g.role == Some(Role::Scr) && is_alpha(t) {
        return format!(r"\mathscr{{{t}}}");
    }
    if ((g.role == Some(Role::Sy) && g.enc() == Some(Enc::Oms)) || g.role == Some(Role::Cal))
        && is_ascii_upper(t)
    {
        return format!(r"\mathcal{{{t}}}");
    }
    if let Some(n) = greek(t) {
        return format!("\\{n} ");
    }
    if let Some(s) = symbol(t) {
        return cmd_space(s);
    }
    t.to_string()
}

fn in_text_math_ok(t: &str) -> bool {
    let mut cs = t.chars();
    matches!((cs.next(), cs.next()), (Some(c), None) if TEXT_MATH_OK.contains(c))
}

/// Glyphs a footnote marker is made of: digits and the reference symbols;
/// commas only between them.
fn is_marker(t: &str, first: bool) -> bool {
    let mut cs = t.chars();
    matches!((cs.next(), cs.next()), (Some(c), None)
        if c.is_ascii_digit() || "∗*†‡§¶⋆".contains(c) || (!first && c == ','))
}

/// Drops a text-font bracket at either end of a span that has no partner
/// inside it (the `)` of "(q and q̇)" belongs to the sentence).
fn trim_unmatched_brackets(all: &[G], sp: &mut Vec<usize>) {
    let mut unmatched = vec![false; sp.len()];
    // Note: any opener closes with any closer: half-open intervals ([0, ∞)).
    let mut stack: Vec<usize> = Vec::new();
    for (k, &x) in sp.iter().enumerate() {
        if all[x].role.is_some() {
            continue;
        }
        match all[x].t.as_str() {
            "(" | "[" => stack.push(k),
            ")" | "]" => {
                if stack.pop().is_none() {
                    unmatched[k] = true;
                }
            }
            _ => {}
        }
    }
    for k in stack {
        unmatched[k] = true;
    }
    let lo = unmatched.iter().take_while(|&&u| u).count();
    let hi = sp.len() - unmatched[lo..].iter().rev().take_while(|&&u| u).count();
    sp.truncate(hi);
    sp.drain(..lo);
}

/// Runs of math glyphs in a line, extended over text-font digits /
/// punctuation / operator names that sit between or next to math glyphs.
fn spans_in_line(all: &[G], ln: &[usize]) -> Vec<Vec<usize>> {
    let n = ln.len();
    if n == 0 {
        return Vec::new();
    }
    let g = |k: usize| &all[ln[k]];
    let body = max_size(all, ln);
    let mut math: Vec<bool> = (0..n)
        .map(|k| g(k).role.is_some() || g(k).size < 0.85 * body)
        .collect();
    // Note: a small text-font glyph is a script only when the line also has
    // math-font glyphs.
    if !(0..n).any(|k| g(k).role.is_some()) {
        return Vec::new();
    }
    // Note: a raised small run set tight after a text-font word or sentence
    // punctuation is a footnote marker (Kim∗, Lee∗†, b.³), not a superscript.
    // Operator names keep their scripts (sin²).
    let mut k = 1;
    while k < n {
        let prev = g(k - 1);
        let after_text = prev.role.is_none()
            && prev.size >= 0.85 * body
            && (is_alpha(&prev.t) || ".,;:".contains(prev.t.as_str()))
            && g(k).x0 - prev.x1 < 0.3 * prev.size;
        let raised = |m: usize| g(m).size < 0.85 * body && g(m).base > prev.base + 0.2 * prev.size;
        if !(after_text && raised(k) && is_marker(&g(k).t, true)) {
            k += 1;
            continue;
        }
        let mut w = k - 1;
        while w > 0 && g(w - 1).role.is_none() && is_alpha(&g(w - 1).t) {
            w -= 1;
        }
        let word: String = (w..k).map(|m| g(m).t.as_str()).collect();
        let mut j = k;
        while j < n && raised(j) && is_marker(&g(j).t, false) {
            j += 1;
        }
        // Note: a word whose script is set tight against a bracket or math is
        // a function name (Aut⁰(X)).
        let tight_after =
            j < n && g(j).x0 - g(j - 1).x1 < 0.15 * g(j).size && (math[j] || g(j).t == "(");
        if !is_operator(&word) && !tight_after {
            for m in math.iter_mut().take(j).skip(k) {
                *m = false;
            }
        }
        k = j;
    }
    // Operator names in text font: a run of letters matching OPERATORS next
    // to math.
    let mut i = 0;
    while i < n {
        if g(i).role.is_none() && is_alpha(&g(i).t) {
            let mut j = i;
            while j < n
                && g(j).role.is_none()
                && is_alpha(&g(j).t)
                && (j == i || g(j).x0 - g(j - 1).x1 < 0.15 * g(j).size)
            {
                j += 1;
            }
            let word: String = (i..j).map(|k| g(k).t.as_str()).collect();
            let near = (i > 0 && math[i - 1]) || (j < n && math[j]);
            let em = g(i).size;
            let tight_l = i > 0 && math[i - 1] && g(i).x0 - g(i - 1).x1 < 0.2 * em;
            let tight_r = j < n && g(j).x0 - g(j - 1).x1 < 0.12 * em && (math[j] || g(j).t == "(");
            // Note: a bold letter against math is a vector (\mathbf{x}); a
            // word set tight against a bracket or math is a function name
            // (Hom(A, B)).
            let bold_letter =
                j - i == 1 && (g(i).fam == "CMBX" || g(i).fam == "CMB") && (tight_l || tight_r);
            if (is_operator(&word) && near) || bold_letter || (j - i >= 2 && tight_r && near) {
                for m in math.iter_mut().take(j).skip(i) {
                    *m = true;
                }
            }
            i = j;
        } else {
            i += 1;
        }
    }
    for k in 0..n {
        if !math[k] && accent_of(g(k)).is_some() {
            let cx = g(k).cx();
            math[k] = (k.saturating_sub(3)..(k + 4).min(n))
                .any(|m| math[m] && g(m).x0 - 1.0 <= cx && cx <= g(m).x1 + 1.0);
        }
    }
    // Grow over text-font digits / punctuation adjacent to math, both
    // directions, until stable.
    let mut changed = true;
    while changed {
        changed = false;
        for k in 0..n {
            if math[k] || !in_text_math_ok(&g(k).t) {
                continue;
            }
            let sz = g(k).size;
            let left = k > 0 && math[k - 1] && g(k).x0 - g(k - 1).x1 < 0.6 * sz;
            let right = k + 1 < n && math[k + 1] && g(k + 1).x0 - g(k).x1 < 0.6 * sz;
            if left || right {
                // Note: a sentence's full stop or comma after math stays text.
                if ".,;:".contains(g(k).t.as_str()) && !right {
                    continue;
                }
                math[k] = true;
                changed = true;
            }
        }
    }
    let mut out: Vec<Vec<usize>> = Vec::new();
    let mut cur: Vec<usize> = Vec::new();
    for k in 0..n {
        if math[k] {
            // Note: TeX's widest math space is ~0.3em; a wider gap before a
            // text-font number is a table column (3.6→ | 18.7→), not math.
            let gap = cur.last().map(|&last| g(k).x0 - all[last].x1);
            let numeral = g(k).role.is_none() && g(k).t.chars().all(|c| c.is_ascii_digit());
            if gap.is_some_and(|d| d > 1.5 * body || (numeral && d > 0.6 * body)) {
                out.push(std::mem::take(&mut cur));
            }
            cur.push(ln[k]);
        } else if !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    for sp in out.iter_mut() {
        trim_unmatched_brackets(all, sp);
    }
    out.retain(|s| s.iter().any(|&x| all[x].role.is_some()) || s.len() > 1);
    out
}

/// Note: a small glyph back on the baseline is a script's script (the l of
/// `\|g\|_{C^l}`).
fn on_base(g: &G, base: f32, size: f32) -> bool {
    g.size >= 0.92 * size || ((g.base - base).abs() <= 0.12 * size && g.size >= 0.8 * size)
}

const OPERATORNAME: [&str; 6] = ["diag", "rank", "span", "sgn", "Re", "Im"];

/// LaTeX for glyphs on baseline `base` at `size`, with sub/superscripts
/// recursively.
fn build(all: &[G], gs: &[usize], base: f32, size: f32) -> String {
    let mut out = String::new();
    let n = gs.len();
    let mut i = 0;
    while i < n {
        let g = &all[gs[i]];
        if on_base(g, base, size) {
            if g.role.is_none() && is_alpha(&g.t) {
                let mut j = i;
                while j < n
                    && all[gs[j]].role.is_none()
                    && is_alpha(&all[gs[j]].t)
                    && on_base(&all[gs[j]], base, size)
                {
                    j += 1;
                }
                let word: String = gs[i..j].iter().map(|&x| all[x].t.as_str()).collect();
                if is_operator(&word) {
                    if OPERATORNAME.contains(&word.as_str()) {
                        out.push_str(&format!(r"\operatorname{{{word}}}"));
                    } else {
                        out.push_str(&format!("\\{word} "));
                    }
                    i = j;
                    continue;
                }
            }
            out.push_str(&tex_char(g));
            i += 1;
            continue;
        }
        // A script run: consecutive glyphs off the baseline.
        let mut j = i;
        while j < n && !on_base(&all[gs[j]], base, size) {
            j += 1;
        }
        let run = &gs[i..j];
        let top = max_size(all, run);
        // Note: first-level script glyphs pick sub or sup by their baseline;
        // smaller (second-level) glyphs go with the first-level glyph before
        // them (E_{g^{-1}}: the -1 rides on the g).
        let mut sub: Vec<usize> = Vec::new();
        let mut sup: Vec<usize> = Vec::new();
        let mut last_sup: Option<bool> = None;
        // Baselines of the latest first-level sub and sup glyphs.
        let (mut sub_base, mut sup_base): (Option<f32>, Option<f32>) = (None, None);
        for &x in run {
            if all[x].size >= 0.9 * top || last_sup.is_none() {
                let up = all[x].base > base;
                last_sup = Some(up);
                if up {
                    sup_base = Some(all[x].base);
                } else {
                    sub_base = Some(all[x].base);
                }
            } else if let (Some(lo), Some(hi)) = (sub_base, sup_base) {
                // Note: with both scripts open, a smaller glyph rides the one
                // whose baseline is nearer (\Phi_{B_s}^{ren}: the s is B's).
                last_sup = Some((all[x].base - hi).abs() < (all[x].base - lo).abs());
            }
            if last_sup == Some(true) {
                sup.push(x);
            } else {
                sub.push(x);
            }
        }
        for (grp, mark, is_sup) in [(&sub, '_', false), (&sup, '^', true)] {
            if grp.is_empty() {
                continue;
            }
            let mut lvl1: Vec<usize> = grp
                .iter()
                .copied()
                .filter(|&x| all[x].size >= 0.9 * top)
                .collect();
            if lvl1.is_empty() {
                lvl1 = grp.clone();
            }
            let b = median(lvl1.iter().map(|&x| all[x].base).collect());
            let mut body = build(all, grp, b, max_size(all, &lvl1));
            if is_sup {
                // Note: primes are written f', not f^{'} (a double
                // superscript in KaTeX).
                let k = body.len() - body.trim_start_matches('\'').len();
                if k > 0 {
                    out.push_str(&"'".repeat(k));
                    body = body[k..].trim().to_string();
                }
            }
            if !body.is_empty() {
                out.push(mark);
                out.push('{');
                out.push_str(&body);
                out.push('}');
            }
        }
        i = j;
    }
    tidy(out.trim())
}

fn tidy(s: &str) -> String {
    let mut s = s
        .replace("...", r"\ldots ")
        .replace(r"\cdot \cdot \cdot ", r"\cdots ");
    s = s
        .replace("NOT=", r"\neq ")
        .replace("NOT", r"\not ")
        .replace(r"MAPSTO\to ", r"\mapsto ")
        .replace("MAPSTO", "");
    for tok in ["HAT", "TILDE", "SQRT"] {
        s = s.replace(tok, "");
    }
    s
}

/// Whether a font can make its page a TeX page (see [`page_math`]): a math
/// font in a TeX encoding, or an AMS / Euler / rsfs alphabet. A cheap gate
/// that lets callers skip collecting glyphs on every other page.
pub(crate) fn is_tex_math_font(font_name: &str) -> bool {
    let (role, enc) = font_info(&family(font_name));
    enc.is_some()
        || matches!(
            role,
            Some(Role::Bb) | Some(Role::Frak) | Some(Role::Cal) | Some(Role::Scr)
        )
}

/// The item box for a span on baseline `base`: the font box of the real
/// glyphs on that baseline (the span's own, else its line's), so the item sits
/// in its line like text does.
///
/// Note: neither scripts' ink nor a fraction's stacked parts may set it: an
/// item that rises above its line drops out of the line's band in layout,
/// and the text after it then reads as a new column start (a two-column page
/// turned "tabular").
fn line_box(
    all: &[G],
    ln: &[usize],
    kept: &[usize],
    base: f32,
    size: f32,
    page_h: f32,
) -> (f32, f32) {
    let real = |x: &&usize| {
        let g = &all[**x];
        g.fam != structures::STAND_IN_FAM
            && (g.base - base).abs() <= 0.1 * size
            && g.size >= 0.8 * size
    };
    let boxes: Vec<(f32, f32)> = match kept
        .iter()
        .filter(real)
        .map(|&x| all[x].loose)
        .collect::<Vec<_>>()
    {
        b if !b.is_empty() => b,
        _ => ln.iter().filter(real).map(|&x| all[x].loose).collect(),
    };
    if boxes.is_empty() {
        // Typical font box: 0.78 em above the baseline, 0.22 em below.
        let y = page_h - base;
        return (y - 0.78 * size, y + 0.22 * size);
    }
    (
        boxes.iter().map(|b| b.0).fold(f32::MAX, f32::min),
        boxes.iter().map(|b| b.1).fold(f32::MIN, f32::max),
    )
}

/// Math spans of one page, or None when the page isn't set in TeX.
pub(crate) fn page_math(chars: &[RawChar], rules: &[Rule], page_h: f32) -> Option<Vec<MathSpan>> {
    let mut all = glyphs(chars, page_h);
    // Note: only pages set by TeX (a math font in a TeX encoding, or an AMS /
    // Euler / rsfs font) get math: Word's Symbol font and table rules
    // elsewhere would otherwise read as math.
    let tex_page = all.iter().any(|g| {
        g.enc().is_some()
            || matches!(
                g.role,
                Some(Role::Bb) | Some(Role::Frak) | Some(Role::Cal) | Some(Role::Scr)
            )
    });
    if !tex_page {
        return None;
    }
    let mut live: Vec<usize> = (0..all.len()).collect();
    live = structures::fold(&mut all, live, rules, page_h);
    let mut res = Vec::new();
    for ln in lines(&mut all, &live) {
        for sp in spans_in_line(&all, &ln) {
            let kept = attach_accents(&mut all, &sp);
            if kept.is_empty() {
                continue;
            }
            let big = max_size(&all, &kept);
            let base = median(
                kept.iter()
                    .filter(|&&x| all[x].size >= 0.92 * big)
                    .map(|&x| all[x].base)
                    .collect(),
            );
            let tex = build(&all, &kept, base, big);
            if tex.is_empty() {
                continue;
            }
            let (top, bottom) = line_box(&all, &ln, &kept, base, big, page_h);
            let mut chars: Vec<i32> = sp
                .iter()
                .flat_map(|&x| all[x].src.iter().copied())
                .collect();
            chars.sort_unstable();
            chars.dedup();
            res.push(MathSpan {
                tex,
                x0: sp.iter().map(|&x| all[x].x0).fold(f32::MAX, f32::min),
                x1: sp.iter().map(|&x| all[x].x1).fold(f32::MIN, f32::max),
                top,
                bottom,
                size: big,
                chars,
            });
        }
    }
    Some(res)
}

/// The spans to splice into the page text, and the char indices they
/// replace.
pub(crate) fn emitted(spans: &[MathSpan]) -> (Vec<&MathSpan>, HashSet<i32>) {
    let keep: Vec<&MathSpan> = spans.iter().filter(|s| s.has_structure()).collect();
    let consumed = keep.iter().flat_map(|s| s.chars.iter().copied()).collect();
    (keep, consumed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn family_strips_prefix_size_and_regular() {
        assert_eq!(family("ABCDEF+CMMI10"), "CMMI");
        assert_eq!(family("LMMathItalic10-Regular"), "LMMATHITALIC");
        assert_eq!(family("MSBM10"), "MSBM");
    }

    #[test]
    fn font_rules_match_prototype_roles() {
        assert_eq!(font_info("CMMI"), (Some(Role::Mi), Some(Enc::Oml)));
        assert_eq!(font_info("CMSY"), (Some(Role::Sy), Some(Enc::Oms)));
        assert_eq!(font_info("CMEX"), (Some(Role::Ex), Some(Enc::Omx)));
        assert_eq!(font_info("MSBM"), (Some(Role::Bb), None));
        assert_eq!(font_info("MSAM"), (Some(Role::Sy), Some(Enc::Msam)));
        assert_eq!(font_info("CMR"), (None, None));
        assert_eq!(font_info("TIMES-ROMAN"), (None, None));
    }

    fn glyph(t: &str, role: Option<Role>, fam: &str, size: f32, base: f32, x0: f32, x1: f32) -> G {
        G {
            t: t.to_string(),
            role,
            fam: fam.to_string(),
            size,
            base,
            x0,
            x1,
            top: 800.0 - (base + size * 0.75),
            bottom: 800.0 - (base - size * 0.25),
            loose: (800.0 - (base + size * 0.75), 800.0 - (base - size * 0.25)),
            pre: String::new(),
            post: String::new(),
            src: vec![],
        }
    }

    #[test]
    fn build_writes_sub_and_superscripts() {
        let all = vec![
            glyph("x", Some(Role::Mi), "CMMI", 10.0, 100.0, 0.0, 5.0),
            glyph("i", Some(Role::Mi), "CMMI", 7.0, 98.0, 5.0, 8.0),
            glyph("2", None, "CMR", 7.0, 104.0, 8.0, 11.0),
        ];
        // The 2 is a fresh first-level glyph above the baseline.
        assert_eq!(build(&all, &[0, 1, 2], 100.0, 10.0), "x_{i}^{2}");
    }

    #[test]
    fn primes_stay_outside_superscript_braces() {
        let all = vec![
            glyph("f", Some(Role::Mi), "CMMI", 10.0, 100.0, 0.0, 5.0),
            glyph("\0'", Some(Role::Sy), "CMSY", 7.0, 104.0, 5.0, 7.0),
        ];
        assert_eq!(build(&all, &[0, 1], 100.0, 10.0), "f'");
    }

    #[test]
    fn footnote_markers_after_words_stay_text() {
        // "Kim∗": a raised CMSY star tight after a roman word.
        let all = vec![
            glyph("K", None, "CMR", 10.0, 100.0, 0.0, 7.0),
            glyph("i", None, "CMR", 10.0, 100.0, 7.0, 10.0),
            glyph("m", None, "CMR", 10.0, 100.0, 10.0, 18.0),
            glyph("∗", Some(Role::Sy), "CMSY", 7.0, 104.0, 18.0, 22.0),
        ];
        assert!(spans_in_line(&all, &[0, 1, 2, 3]).is_empty());
        // "Aut⁰(X)": the same shape tight against a bracket is a function.
        let all = vec![
            glyph("A", None, "CMR", 10.0, 100.0, 0.0, 7.0),
            glyph("u", None, "CMR", 10.0, 100.0, 7.0, 12.0),
            glyph("t", None, "CMR", 10.0, 100.0, 12.0, 16.0),
            glyph("0", None, "CMR", 7.0, 104.0, 16.0, 20.0),
            glyph("(", None, "CMR", 10.0, 100.0, 20.0, 23.0),
            glyph("X", Some(Role::Mi), "CMMI", 10.0, 100.0, 23.0, 31.0),
            glyph(")", None, "CMR", 10.0, 100.0, 31.0, 34.0),
        ];
        assert_eq!(
            spans_in_line(&all, &[0, 1, 2, 3, 4, 5, 6]),
            vec![vec![0, 1, 2, 3, 4, 5, 6]]
        );
    }

    #[test]
    fn unmatched_end_brackets_are_trimmed() {
        let g = |t: &str, x: f32| glyph(t, None, "CMR", 10.0, 100.0, x, x + 3.0);
        let all = vec![
            g("q", 0.0),
            g(")", 3.0),
            g("[", 6.0),
            g("0", 9.0),
            g(")", 12.0),
        ];
        let mut sp = vec![0, 1];
        trim_unmatched_brackets(&all, &mut sp);
        assert_eq!(sp, vec![0]);
        // A half-open interval [0) is matched.
        let mut sp = vec![2, 3, 4];
        trim_unmatched_brackets(&all, &mut sp);
        assert_eq!(sp, vec![2, 3, 4]);
    }

    #[test]
    fn structure_rule_keeps_plain_symbols_as_text() {
        let span = |tex: &str| MathSpan {
            tex: tex.to_string(),
            x0: 0.0,
            x1: 0.0,
            top: 0.0,
            bottom: 0.0,
            size: 0.0,
            chars: vec![],
        };
        assert!(!span(r"\pm ").has_structure());
        assert!(!span(r"\alpha ").has_structure());
        assert!(span("x_{i}").has_structure());
        assert!(span(r"\mathcal{O}(2^{M}\cdot K)").has_structure());
    }
}
