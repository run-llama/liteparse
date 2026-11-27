//! Chart rendering: draws a `c:chart` graphic frame from its cached data the way
//! PowerPoint does. Covers the chart and plot areas, category/value/date axes
//! with Excel's automatic scaling, gridlines, bar/column (clustered, stacked,
//! 100%), line, area, scatter, pie, doughnut and tilted 3-D pie groups, data
//! labels, the title and the legend.
//!
//! Everything is computed in frame-local points (origin at the frame's top left)
//! and mapped to the slide with the frame transform. Text goes through the slide
//! text engine, so axis labels, legend entries, titles and data labels come out
//! as text items like the LibreOffice-rendered PDF used to produce.
//!
//! Radar charts are in `radar.rs`, surface charts (contour bands, 3-D as an
//! orthographic projection) in `surface.rs`, and chartEx charts (waterfall,
//! funnel, box & whisker, treemap, sunburst, histogram, Pareto) in `chartex.rs`.
//!
//! Not drawn: 3-D perspective (3-D bars are drawn flat), region maps (the
//! fallback picture is used), trendlines, error bars and data tables. Those
//! record a `CHART_PARTIAL` warning.

use std::rc::Rc;

use tiny_skia::{PathBuilder, Transform};

use crate::office::pptx::chart::{ChartData, Series};
use crate::office::pptx::color::{ColorScheme, Rgba};
use crate::office::pptx::geom::TextRect;
use crate::office::pptx::scene::{
    Item, LineCap, LineJoin, Paint, PathItem, Rect, ShapeInfo, Stroke,
};
use crate::office::pptx::slide::{EMU_PER_PT, SlideBuilder};
use crate::office::pptx::style::{self, Fill, StyleCtx};
use crate::office::pptx::textprops::resolve_para;
use crate::office::pptx::xml::XNode;

/// Inner margin of the chart area in automatic layout (points).
const PAD: f64 = 7.0;
/// Length of a major tick mark (points).
const TICK: f64 = 3.75;
/// Points drawn per series; longer caches are truncated.
const MAX_POINTS: usize = 4000;
/// Text labels per chart (axis, data and legend labels together).
const MAX_LABELS: usize = 1500;
/// Category labels measured per axis; skipping is estimated beyond that.
const MAX_MEASURED_CATS: usize = 300;

pub fn render(
    b: &mut SlideBuilder,
    chart: &ChartData,
    _part: &str,
    tf: Transform,
    w: f64,
    h: f64,
    idx: usize,
) {
    let name = b.scene.shapes[idx].name.clone();
    let root = chart.root.clone();
    let Some((root, c)) = root.and_then(|r| r.child("c:chart").cloned().map(|c| (r, c))) else {
        b.scene.warnings.push(("CHART_NOT_RENDERED".into(), name));
        return;
    };
    if !(w.is_finite() && h.is_finite() && w > 2.0 && h > 2.0) {
        return;
    }
    let mut r = Renderer::new(b, chart, root, c, tf, w, h, idx);
    let skipped = r.draw(chart);
    if !skipped.is_empty() {
        b.scene.warnings.push((
            "CHART_PARTIAL".into(),
            format!("{name}: {}", skipped.join(", ")),
        ));
    }
}

// ---------------------------------------------------------------------------
// Small XML helpers
// ---------------------------------------------------------------------------

fn val<'n>(n: &'n XNode, name: &str) -> Option<&'n str> {
    n.child(name).and_then(|c| c.attr("val"))
}

fn val_f(n: &XNode, name: &str) -> Option<f64> {
    val(n, name)
        .and_then(|v| v.trim().parse::<f64>().ok())
        .filter(|v| v.is_finite())
}

/// A `CT_Boolean` child: present without `val` means true.
fn flag(n: &XNode, name: &str) -> Option<bool> {
    n.child(name).map(|c| c.attr_bool("val").unwrap_or(true))
}

fn node(name: &str, attrs: Vec<(&str, String)>, children: Vec<Rc<XNode>>) -> Rc<XNode> {
    Rc::new(XNode {
        name: name.to_string(),
        attrs: attrs.into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
        children,
        text: String::new(),
    })
}

fn text_node(name: &str, text: &str) -> Rc<XNode> {
    Rc::new(XNode {
        name: name.to_string(),
        attrs: Vec::new(),
        children: Vec::new(),
        text: text.to_string(),
    })
}

/// A text body with one centered paragraph per line of `text`.
fn plain_body(text: &str) -> Rc<XNode> {
    let paras = text
        .split('\n')
        .map(|line| {
            node(
                "a:p",
                vec![],
                vec![node(
                    "a:r",
                    vec![],
                    vec![node("a:rPr", vec![], vec![]), text_node("a:t", line)],
                )],
            )
        })
        .collect();
    node("a:txBody", vec![], paras)
}

fn body_pr(wrap: bool) -> Rc<XNode> {
    let z = || "0".to_string();
    node(
        "a:bodyPr",
        vec![
            ("lIns", z()),
            ("tIns", z()),
            ("rIns", z()),
            ("bIns", z()),
            ("wrap", if wrap { "square" } else { "none" }.to_string()),
            ("anchor", "t".to_string()),
        ],
        vec![],
    )
}

/// Wraps a paragraph-properties element as a one-level list style.
fn layer_from_ppr(ppr: &XNode) -> Rc<XNode> {
    let lvl = Rc::new(XNode {
        name: "a:lvl1pPr".into(),
        attrs: ppr.attrs.clone(),
        children: ppr.children.clone(),
        text: String::new(),
    });
    node("a:lstStyle", vec![], vec![lvl])
}

/// The list-style layer of a `c:txPr` (its first paragraph's `a:pPr`).
fn txpr_layer(txpr: Option<&Rc<XNode>>) -> Option<Rc<XNode>> {
    let ppr = txpr?.child("a:p")?.child("a:pPr")?;
    Some(layer_from_ppr(ppr))
}

fn size_layer(size: f64, bold: Option<bool>) -> Rc<XNode> {
    let mut attrs = vec![("sz", format!("{}", (size * 100.0).round() as i64))];
    if let Some(b) = bold {
        attrs.push(("b", if b { "1" } else { "0" }.to_string()));
    }
    let def = node("a:defRPr", attrs, vec![]);
    node(
        "a:lstStyle",
        vec![],
        vec![node("a:lvl1pPr", vec![], vec![def])],
    )
}

/// Chart text defaults: minor theme font, 10pt, `tx1`, centered.
fn base_layer() -> Rc<XNode> {
    let clr = node("a:schemeClr", vec![("val", "tx1".into())], vec![]);
    let def = node(
        "a:defRPr",
        vec![("sz", "1000".into()), ("b", "0".into()), ("i", "0".into())],
        vec![
            node("a:solidFill", vec![], vec![clr]),
            node("a:latin", vec![("typeface", "+mn-lt".into())], vec![]),
            node("a:ea", vec![("typeface", "+mn-ea".into())], vec![]),
            node("a:cs", vec![("typeface", "+mn-cs".into())], vec![]),
        ],
    );
    node(
        "a:lstStyle",
        vec![],
        vec![node("a:lvl1pPr", vec![("algn", "ctr".into())], vec![def])],
    )
}

/// Text rotation (degrees) from a `c:txPr` body, if explicit. Office writes
/// `rot="-60000000"` for "automatic".
fn txpr_rot(txpr: Option<&Rc<XNode>>) -> Option<f64> {
    let body = txpr?.child("a:bodyPr")?;
    if let Some(v) = body.attr("vert") {
        match v {
            "vert" | "eaVert" => return Some(90.0),
            "vert270" => return Some(-90.0),
            _ => {}
        }
    }
    let r = body.attr_f64("rot")?;
    if r <= -59_000_000.0 {
        return None;
    }
    Some(r / 60_000.0)
}

/// Manual layout of a title, legend, plot area or label: `(x, y, w, h)` and
/// whether x/y are absolute (`edge`) or offsets from the default (`factor`).
#[derive(Debug, Clone, Copy, Default)]
struct Manual {
    x: Option<f64>,
    y: Option<f64>,
    w: Option<f64>,
    h: Option<f64>,
    x_edge: bool,
    y_edge: bool,
    inner: bool,
}

fn manual_layout(parent: Option<&XNode>) -> Option<Manual> {
    let ml = parent?.child("c:layout")?.child("c:manualLayout")?;
    let f = |k: &str| val_f(ml, k).filter(|v| v.abs() < 10.0);
    let m = Manual {
        x: f("c:x"),
        y: f("c:y"),
        w: f("c:w"),
        h: f("c:h"),
        x_edge: val(ml, "c:xMode") == Some("edge"),
        y_edge: val(ml, "c:yMode") == Some("edge"),
        inner: val(ml, "c:layoutTarget") == Some("inner"),
    };
    if m.x.is_none() && m.y.is_none() && m.w.is_none() && m.h.is_none() {
        return None;
    }
    Some(m)
}

// ---------------------------------------------------------------------------
// Number formatting (Excel format codes)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Lit(String),
    Digit(char),
    Point,
    Comma,
    Percent,
    Exp(bool),
    General,
    Date(String),
    At,
}

fn split_sections(code: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut in_quote = false;
    let mut esc = false;
    for (i, c) in code.char_indices() {
        if esc {
            esc = false;
            continue;
        }
        match c {
            '\\' if !in_quote => esc = true,
            '"' => in_quote = !in_quote,
            ';' if !in_quote => {
                out.push(&code[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&code[start..]);
    out
}

fn tokenize(sec: &str) -> Vec<Tok> {
    let chars: Vec<char> = sec.chars().collect();
    let n = chars.len();
    let mut out = Vec::new();
    let mut i = 0;
    let lower_starts = |i: usize, pat: &str| -> bool {
        let s = chars[i..]
            .iter()
            .take(pat.len())
            .collect::<String>()
            .to_ascii_lowercase();
        s == pat
    };
    while i < n {
        let c = chars[i];
        match c {
            '"' => {
                let mut s = String::new();
                i += 1;
                while i < n && chars[i] != '"' {
                    s.push(chars[i]);
                    i += 1;
                }
                out.push(Tok::Lit(s));
                i += 1;
            }
            '\\' => {
                if i + 1 < n {
                    out.push(Tok::Lit(chars[i + 1].to_string()));
                }
                i += 2;
            }
            '_' => {
                out.push(Tok::Lit(" ".into()));
                i += 2;
            }
            '*' => i += 2,
            '[' => {
                let end = chars[i..]
                    .iter()
                    .position(|&c| c == ']')
                    .map(|p| i + p)
                    .unwrap_or(n);
                let content: String = chars[(i + 1).min(n)..end].iter().collect();
                if let Some(cur) = content.strip_prefix('$') {
                    let sym = cur.split('-').next().unwrap_or("");
                    if !sym.is_empty() {
                        out.push(Tok::Lit(sym.to_string()));
                    }
                }
                i = end + 1;
            }
            '0' | '#' | '?' => {
                out.push(Tok::Digit(c));
                i += 1;
            }
            '.' => {
                out.push(Tok::Point);
                i += 1;
            }
            ',' => {
                out.push(Tok::Comma);
                i += 1;
            }
            '%' => {
                out.push(Tok::Percent);
                i += 1;
            }
            'E' | 'e' if i + 1 < n && (chars[i + 1] == '+' || chars[i + 1] == '-') => {
                out.push(Tok::Exp(chars[i + 1] == '+'));
                i += 2;
            }
            '@' => {
                out.push(Tok::At);
                i += 1;
            }
            'G' | 'g' if lower_starts(i, "general") => {
                out.push(Tok::General);
                i += 7;
            }
            'A' | 'a' if lower_starts(i, "am/pm") => {
                out.push(Tok::Date("ampm".into()));
                i += 5;
            }
            'A' | 'a' if lower_starts(i, "a/p") => {
                out.push(Tok::Date("ap".into()));
                i += 3;
            }
            'y' | 'Y' | 'm' | 'M' | 'd' | 'D' | 'h' | 'H' | 's' | 'S' => {
                let lc = c.to_ascii_lowercase();
                let mut run = String::new();
                while i < n && chars[i].to_ascii_lowercase() == lc {
                    run.push(lc);
                    i += 1;
                }
                out.push(Tok::Date(run));
            }
            _ => {
                out.push(Tok::Lit(c.to_string()));
                i += 1;
            }
        }
    }
    out
}

/// Formats `v` with an Excel number format code (`General`, `0.0%`,
/// `#,##0`, `"$"#,##0.00`, `0.00E+00`, `mmm-yy`, sections `pos;neg;zero`).
pub(crate) fn format_value(v: f64, code: &str, date1904: bool) -> String {
    if !v.is_finite() {
        return String::new();
    }
    let sections = split_sections(code);
    let (sec, explicit_sign) = match sections.len() {
        0 | 1 => (sections.first().copied().unwrap_or(""), false),
        2 => {
            if v < 0.0 {
                (sections[1], true)
            } else {
                (sections[0], false)
            }
        }
        _ => {
            if v < 0.0 {
                (sections[1], true)
            } else if v == 0.0 {
                (sections[2], true)
            } else {
                (sections[0], false)
            }
        }
    };
    let toks = tokenize(sec);
    if toks.is_empty() {
        return if sections.len() > 1 {
            String::new()
        } else {
            general(v)
        };
    }
    let has_digits = toks.iter().any(|t| matches!(t, Tok::Digit(_)));
    let has_date = toks.iter().any(|t| matches!(t, Tok::Date(_)));
    if has_date && !has_digits {
        return format_date(v, &toks, date1904);
    }
    let a = if explicit_sign { v.abs() } else { v };
    let neg = a < 0.0;
    let a = a.abs();
    if !has_digits {
        // Literal-only or General sections.
        let mut s = String::new();
        let pct = toks.iter().filter(|t| matches!(t, Tok::Percent)).count() as i32;
        for t in &toks {
            match t {
                Tok::Lit(l) => s.push_str(l),
                Tok::General | Tok::At => s.push_str(&general(a * 100f64.powi(pct))),
                Tok::Percent => s.push('%'),
                Tok::Point => s.push('.'),
                Tok::Comma => s.push(','),
                Tok::Date(d) => s.push_str(d),
                _ => {}
            }
        }
        return if neg { format!("-{s}") } else { s };
    }
    let body = format_numeric(a, &toks);
    if neg && body.chars().any(|c| c.is_ascii_digit() && c != '0') {
        format!("-{body}")
    } else {
        body
    }
}

fn format_numeric(v: f64, toks: &[Tok]) -> String {
    let digit_idx: Vec<usize> = toks
        .iter()
        .enumerate()
        .filter(|(_, t)| matches!(t, Tok::Digit(_)))
        .map(|(i, _)| i)
        .collect();
    let (first, last) = (digit_idx[0], *digit_idx.last().unwrap_or(&0));
    let exp_idx = toks.iter().position(|t| matches!(t, Tok::Exp(_)));
    let point_idx = toks
        .iter()
        .position(|t| matches!(t, Tok::Point))
        .filter(|&p| p <= last && p + 1 >= first && exp_idx.is_none_or(|e| p < e));
    let int_end = point_idx.or(exp_idx).unwrap_or(last + 1);
    let digits_in = |a: usize, b: usize| -> Vec<char> {
        toks[a.min(toks.len())..b.min(toks.len())]
            .iter()
            .filter_map(|t| {
                if let Tok::Digit(c) = t {
                    Some(*c)
                } else {
                    None
                }
            })
            .collect()
    };
    let int_digits = digits_in(0, int_end);
    let dec_end = exp_idx.unwrap_or(toks.len());
    let dec_digits = point_idx
        .map(|p| digits_in(p + 1, dec_end))
        .unwrap_or_default();
    let exp_digits = exp_idx
        .map(|e| digits_in(e + 1, toks.len()))
        .unwrap_or_default();
    // Grouping: a comma between integer digits. Scaling: commas right after
    // the last integer or decimal digit (each divides by 1000).
    let int_digit_pos: Vec<usize> = digit_idx.iter().copied().filter(|&i| i < int_end).collect();
    let grouping = match (int_digit_pos.first(), int_digit_pos.last()) {
        (Some(&a), Some(&b)) => toks[a..b].iter().any(|t| matches!(t, Tok::Comma)),
        _ => false,
    };
    let mut scale = 0;
    let mut j = last + 1;
    while j < toks.len() && toks[j] == Tok::Comma {
        scale += 1;
        j += 1;
    }
    if let (Some(p), Some(&li)) = (point_idx, int_digit_pos.last()) {
        let mut k = li + 1;
        while k < p && toks[k] == Tok::Comma {
            scale += 1;
            k += 1;
        }
    }
    let pct = toks.iter().filter(|t| matches!(t, Tok::Percent)).count() as i32;
    let x = v * 100f64.powi(pct) / 1000f64.powi(scale);
    let mut prefix = String::new();
    let mut suffix = String::new();
    let lit = |t: &Tok| -> Option<String> {
        match t {
            Tok::Lit(s) => Some(s.clone()),
            Tok::Percent => Some("%".into()),
            Tok::General | Tok::At => Some(String::new()),
            Tok::Date(d) => Some(d.clone()),
            _ => None,
        }
    };
    let number_start = point_idx.map(|p| p.min(first)).unwrap_or(first);
    for (i, t) in toks.iter().enumerate() {
        if i < number_start {
            if let Some(s) = lit(t) {
                prefix.push_str(&s);
            } else if *t == Tok::Comma {
                prefix.push(',');
            }
        } else if i > last && !(i < j && *t == Tok::Comma) {
            if let Some(s) = lit(t) {
                suffix.push_str(&s);
            } else if *t == Tok::Comma {
                suffix.push(',');
            }
        }
    }
    let num = if let Some(e) = exp_idx {
        let plus = matches!(toks[e], Tok::Exp(true));
        let n_int = int_digits.len().max(1) as i32;
        let mut ex = if x == 0.0 {
            0
        } else {
            x.log10().floor() as i32 - (n_int - 1)
        };
        let mut m = x / 10f64.powi(ex);
        let p = 10f64.powi(dec_digits.len() as i32);
        if (round_half_up(m * p) / p) >= 10f64.powi(n_int) {
            ex += 1;
            m = x / 10f64.powi(ex);
        }
        let mant = fixed(m, &int_digits, &dec_digits, false, point_idx.is_some());
        let sign = if ex < 0 {
            "-"
        } else if plus {
            "+"
        } else {
            ""
        };
        let width = exp_digits.len().max(1);
        format!("{mant}E{sign}{:0width$}", ex.unsigned_abs())
    } else {
        fixed(x, &int_digits, &dec_digits, grouping, point_idx.is_some())
    };
    format!("{prefix}{num}{suffix}")
}

/// Rounds half away from zero, absorbing binary noise just below the half.
fn round_half_up(y: f64) -> f64 {
    let f = y.floor();
    let eps = (y.abs() * 1e-15 + 1e-12).min(0.1);
    if y - f >= 0.5 - eps { f + 1.0 } else { f }
}

fn fixed(x: f64, int_digits: &[char], dec_digits: &[char], grouping: bool, point: bool) -> String {
    let decs = dec_digits.len().min(15);
    let p = 10f64.powi(decs as i32);
    let scaled = round_half_up(x * p);
    if scaled.is_nan() || scaled >= 1e30 {
        return general(x);
    }
    let total = scaled as u128;
    let p_int = 10u128.pow(decs as u32);
    let int = total / p_int;
    let frac = total % p_int;
    let mut frac_s = if decs > 0 {
        format!("{frac:0decs$}")
    } else {
        String::new()
    };
    // Optional decimal places ('#', '?') drop trailing zeros.
    let mut chars: Vec<char> = frac_s.chars().collect();
    for (k, spec) in dec_digits.iter().enumerate().rev() {
        if k >= chars.len() || chars[k] != '0' {
            break;
        }
        match spec {
            '#' => {
                chars.pop();
            }
            '?' => chars[k] = ' ',
            _ => break,
        }
    }
    frac_s = chars.into_iter().collect();
    let min_int = int_digits.iter().filter(|&&c| c == '0').count();
    let mut int_s = if int == 0 && min_int == 0 {
        String::new()
    } else {
        int.to_string()
    };
    while int_s.len() < min_int {
        int_s.insert(0, '0');
    }
    if grouping && int_s.len() > 3 {
        let bytes: Vec<char> = int_s.chars().collect();
        let mut g = String::new();
        for (i, c) in bytes.iter().enumerate() {
            if i > 0 && (bytes.len() - i).is_multiple_of(3) {
                g.push(',');
            }
            g.push(*c);
        }
        int_s = g;
    }
    if point && (decs > 0 || !dec_digits.is_empty()) {
        if frac_s.is_empty() {
            format!("{int_s}.")
        } else {
            format!("{int_s}.{frac_s}")
        }
    } else if point {
        format!("{int_s}.")
    } else {
        int_s
    }
}

/// Excel's `General` format: up to 11 characters, scientific beyond.
pub(crate) fn general(v: f64) -> String {
    if !v.is_finite() {
        return String::new();
    }
    if v == 0.0 {
        return "0".into();
    }
    let a = v.abs();
    let sign = if v < 0.0 { "-" } else { "" };
    if !(1e-9..1e11).contains(&a) {
        let ex = a.log10().floor() as i32;
        let mut m = a / 10f64.powi(ex);
        let mut ex = ex;
        if round_half_up(m * 1e5) / 1e5 >= 10.0 {
            m /= 10.0;
            ex += 1;
        }
        let ms = format!("{:.5}", round_half_up(m * 1e5) / 1e5);
        let ms = ms.trim_end_matches('0').trim_end_matches('.');
        return format!(
            "{sign}{ms}E{}{:02}",
            if ex < 0 { "-" } else { "+" },
            ex.unsigned_abs()
        );
    }
    let int_digits = if a >= 1.0 {
        a.log10().floor() as i32 + 1
    } else {
        1
    };
    let decs = (10 - int_digits).clamp(0, 10);
    let p = 10f64.powi(decs);
    let r = round_half_up(a * p) / p;
    let mut s = format!("{:.*}", decs as usize, r);
    if s.contains('.') {
        s = s.trim_end_matches('0').trim_end_matches('.').to_string();
    }
    // Tiny values switch to scientific once fixed notation loses digits.
    let lossy = a < 1e-4
        && s.parse::<f64>()
            .map(|f| ((f - a) / a).abs() > 1e-6)
            .unwrap_or(true);
    if s == "0" || lossy {
        // Too small for fixed notation within 11 characters.
        let ex = a.log10().floor() as i32;
        let m = round_half_up(a / 10f64.powi(ex) * 1e5) / 1e5;
        let ms = format!("{m:.5}");
        let ms = ms.trim_end_matches('0').trim_end_matches('.');
        return format!("{sign}{ms}E-{:02}", ex.unsigned_abs());
    }
    format!("{sign}{s}")
}

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const DAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

/// Civil date from days since 1970-01-01 (proleptic Gregorian).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Excel serial → (year, month, day, weekday 0=Sunday, seconds of day).
fn excel_date(v: f64, date1904: bool) -> (i64, u32, u32, u32, u32) {
    let days = v.floor() as i64;
    let secs = ((v - v.floor()) * 86_400.0).round() as u32;
    let unix_days = if date1904 {
        days - 24_107
    } else if days >= 61 {
        days - 25_569
    } else if days == 60 {
        // Excel's fictional 1900-02-29.
        return (1900, 2, 29, 4, secs.min(86_399));
    } else {
        days - 25_568
    };
    let (y, m, d) = civil_from_days(unix_days);
    let wd = (unix_days + 4).rem_euclid(7) as u32;
    (y, m, d, wd, secs.min(86_399))
}

fn format_date(v: f64, toks: &[Tok], date1904: bool) -> String {
    let (y, mo, d, wd, secs) = excel_date(v, date1904);
    let (hh, mi, ss) = (secs / 3600, (secs / 60) % 60, secs % 60);
    let ampm = toks
        .iter()
        .any(|t| matches!(t, Tok::Date(s) if s == "ampm" || s == "ap"));
    let mut out = String::new();
    let date_tok = |i: usize| -> Option<&str> {
        if let Some(Tok::Date(s)) = toks.get(i) {
            Some(s.as_str())
        } else {
            None
        }
    };
    for (i, t) in toks.iter().enumerate() {
        match t {
            Tok::Date(s) => {
                let c = s.chars().next().unwrap_or(' ');
                let n = s.len();
                match c {
                    'y' => {
                        if n <= 2 {
                            out.push_str(&format!("{:02}", y.rem_euclid(100)));
                        } else {
                            out.push_str(&format!("{y:04}"));
                        }
                    }
                    'm' => {
                        let prev = (0..i).rev().find_map(date_tok);
                        let next = (i + 1..toks.len()).find_map(date_tok);
                        let minute = n <= 2
                            && (prev.is_some_and(|p| p.starts_with('h'))
                                || next.is_some_and(|p| p.starts_with('s')));
                        let mname = MONTHS[(mo as usize).clamp(1, 12) - 1];
                        match (minute, n) {
                            (true, 1) => out.push_str(&mi.to_string()),
                            (true, _) => out.push_str(&format!("{mi:02}")),
                            (false, 1) => out.push_str(&mo.to_string()),
                            (false, 2) => out.push_str(&format!("{mo:02}")),
                            (false, 3) => out.push_str(&mname[..3]),
                            (false, 5) => out.push_str(&mname[..1]),
                            _ => out.push_str(mname),
                        }
                    }
                    'd' => match n {
                        1 => out.push_str(&d.to_string()),
                        2 => out.push_str(&format!("{d:02}")),
                        3 => out.push_str(&DAYS[wd as usize][..3]),
                        _ => out.push_str(DAYS[wd as usize]),
                    },
                    'h' => {
                        let h = if ampm { (hh + 11) % 12 + 1 } else { hh };
                        if n == 1 {
                            out.push_str(&h.to_string());
                        } else {
                            out.push_str(&format!("{h:02}"));
                        }
                    }
                    's' => {
                        if n == 1 {
                            out.push_str(&ss.to_string());
                        } else {
                            out.push_str(&format!("{ss:02}"));
                        }
                    }
                    _ if s == "ampm" => out.push_str(if hh < 12 { "AM" } else { "PM" }),
                    _ if s == "ap" => out.push_str(if hh < 12 { "A" } else { "P" }),
                    _ => {}
                }
            }
            Tok::Lit(l) => out.push_str(l),
            Tok::Digit(c) => out.push(*c),
            Tok::Point => out.push('.'),
            Tok::Comma => out.push(','),
            Tok::Percent => out.push('%'),
            _ => {}
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Axis scaling (Excel's automatic min/max/major unit)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct AxisScale {
    pub min: f64,
    pub max: f64,
    pub major: f64,
}

/// The smallest 1-2-5 step that splits `range` into at most `max_intervals`.
pub(crate) fn nice_unit(range: f64, max_intervals: f64) -> f64 {
    let range = if range.is_finite() && range > 0.0 {
        range
    } else {
        1.0
    };
    let target = range / max_intervals.max(1.0);
    let mut e = 10f64.powf(target.log10().floor());
    loop {
        for m in [1.0, 2.0, 5.0] {
            let u = m * e;
            if u >= target * (1.0 - 1e-9) {
                return u;
            }
        }
        e *= 10.0;
    }
}

/// Excel's automatic value-axis scale for data in `[dmin, dmax]`. Fixed bounds
/// and unit (`c:min`, `c:max`, `c:majorUnit`) win. `max_intervals` limits the
/// number of major intervals for the space available.
///
/// Rules (Excel): the axis starts at zero unless all values sit within the top
/// sixth of the range; the max gets 5% headroom above the data and rounds up to
/// the next major unit.
pub(crate) fn auto_scale(
    dmin: f64,
    dmax: f64,
    fmin: Option<f64>,
    fmax: Option<f64>,
    fmajor: Option<f64>,
    max_intervals: f64,
) -> AxisScale {
    let (mut lo, mut hi) = if dmin.is_finite() && dmax.is_finite() {
        (dmin.min(dmax), dmin.max(dmax))
    } else {
        (0.0, 1.0)
    };
    if let (Some(a), Some(b)) = (fmin, fmax)
        && b > a
    {
        let major = fmajor
            .filter(|u| *u > 0.0 && (b - a) / u <= 1000.0)
            .unwrap_or_else(|| nice_unit(b - a, max_intervals));
        return AxisScale {
            min: a,
            max: b,
            major,
        };
    }
    if let Some(a) = fmin {
        lo = lo.max(a);
        hi = hi.max(a);
    }
    if let Some(b) = fmax {
        hi = hi.min(b);
        lo = lo.min(b);
    }
    if lo == hi {
        if lo == 0.0 {
            hi = 1.0;
        } else if lo > 0.0 {
            lo = 0.0;
        } else {
            hi = 0.0;
        }
    }
    // Zero rules.
    let mut lo_auto = lo;
    let mut hi_auto = hi;
    if fmin.is_none() {
        if lo >= 0.0 {
            lo_auto = if hi - lo > hi / 6.0 {
                0.0
            } else {
                lo - (hi - lo) / 2.0
            };
            if lo_auto < 0.0 {
                lo_auto = 0.0;
            }
        }
    } else {
        lo_auto = fmin.unwrap_or(lo);
    }
    if fmax.is_none() {
        if hi <= 0.0 {
            hi_auto = if hi - lo > -lo / 6.0 {
                0.0
            } else {
                hi + (hi - lo) / 2.0
            };
            if hi_auto > 0.0 {
                hi_auto = 0.0;
            }
        }
    } else {
        hi_auto = fmax.unwrap_or(hi);
    }
    // Headroom of 5% of the span on the open ends.
    let span = (hi_auto - lo_auto).abs().max(f64::MIN_POSITIVE);
    let hi_p = if fmax.is_none() && hi_auto > 0.0 {
        hi_auto + 0.05 * span
    } else {
        hi_auto
    };
    let lo_p = if fmin.is_none() && lo_auto < 0.0 {
        lo_auto - 0.05 * span
    } else {
        lo_auto
    };
    let major = match fmajor.filter(|u| *u > 0.0 && (hi_p - lo_p) / u <= 1000.0) {
        Some(u) => u,
        None => {
            let mut u = nice_unit(hi_p - lo_p, max_intervals);
            // Rounding outward can add an interval; step up when it overflows.
            for _ in 0..4 {
                let n = (hi_p / u - 1e-9).ceil() - (lo_p / u + 1e-9).floor();
                if n <= max_intervals.max(1.0) + 1e-9 {
                    break;
                }
                u = nice_unit(u * 1.01, 1.0);
            }
            u
        }
    };
    let min = match fmin {
        Some(a) => a,
        None => (lo_p / major + 1e-9).floor() * major,
    };
    // With a fixed min, ticks run from it, so the max is min + k units.
    let mut max = match (fmax, fmin) {
        (Some(b), _) => b,
        (None, Some(a)) => a + ((hi_p - a) / major - 1e-9).ceil() * major,
        (None, None) => (hi_p / major - 1e-9).ceil() * major,
    };
    if max <= min {
        max = min + major;
    }
    AxisScale {
        min: clean(min),
        max: clean(max),
        major,
    }
}

/// Removes binary noise from computed tick values (`0.30000000000000004`).
fn clean(v: f64) -> f64 {
    if v == 0.0 || !v.is_finite() {
        return v;
    }
    let mag = 10f64.powi(12 - v.abs().log10().ceil() as i32);
    (v * mag).round() / mag
}

/// Positions of the bars of one category slot for a clustered group, as
/// fractions of the slot: `(offset of series j, bar width)`.
pub(crate) fn cluster_geometry(
    n_series: usize,
    gap_width: f64,
    overlap: f64,
    j: usize,
) -> (f64, f64) {
    let k = n_series.max(1) as f64;
    let gap = (gap_width / 100.0).clamp(0.0, 5.0);
    let ov = (overlap / 100.0).clamp(-1.0, 1.0);
    let bw = 1.0 / (k - (k - 1.0) * ov + gap);
    let cluster = bw * (k - (k - 1.0) * ov);
    let start = (1.0 - cluster) / 2.0;
    (start + j as f64 * bw * (1.0 - ov), bw)
}

// ---------------------------------------------------------------------------
// Renderer state
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct Label {
    tx: Rc<XNode>,
    lists: Vec<Rc<XNode>>,
    /// Measured text extents (unrotated).
    w: f64,
    h: f64,
    /// Layout box width: the wrap width, or `w`.
    box_w: f64,
    wrap: bool,
    /// Clockwise degrees.
    rot: f64,
    size: f64,
}

impl Label {
    /// Axis-aligned extents after rotation.
    fn ext(&self) -> (f64, f64) {
        let r = self.rot.to_radians();
        let (c, s) = (r.cos().abs(), r.sin().abs());
        (self.w * c + self.h * s, self.w * s + self.h * c)
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum TextKind {
    Title,
    AxisTitle,
    Body,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    Bar { horizontal: bool },
    Line,
    Area,
    Scatter,
    Bubble,
    Pie,
    Pie3D,
    Doughnut,
    Radar { filled: bool },
    Surface { three_d: bool },
    Waterfall,
    Funnel,
    BoxWhisker,
    Treemap,
    Sunburst,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Grouping {
    Standard,
    Clustered,
    Stacked,
    Percent,
}

struct SerInfo {
    s: Series,
    node: Rc<XNode>,
    vals: Vec<Option<f64>>,
    xs: Vec<Option<f64>>,
    fmt: String,
    cat_fmt: Option<String>,
    /// Per-point text from a `c15:datalabelsRange` cache.
    range_texts: Vec<String>,
}

struct Grp {
    kind: Kind,
    node: Rc<XNode>,
    sers: Vec<SerInfo>,
    grouping: Grouping,
    vary: bool,
    ax: (Option<String>, Option<String>),
    /// Indexes into the axis list, filled once axes are known.
    cat: usize,
    val: usize,
    secondary: bool,
}

impl Grp {
    fn stacked(&self) -> bool {
        matches!(self.grouping, Grouping::Stacked | Grouping::Percent)
            || matches!(self.kind, Kind::Waterfall | Kind::BoxWhisker)
    }

    fn n_points(&self) -> usize {
        if self.kind == Kind::BoxWhisker {
            return self.box_cats().len();
        }
        self.sers
            .iter()
            .map(|s| s.vals.len().max(s.s.categories.len()))
            .max()
            .unwrap_or(0)
            .min(MAX_POINTS)
    }

    /// (base, top) of every point of series `j`, honoring stacking.
    #[expect(
        clippy::needless_range_loop,
        reason = "`i` is the point index into every series' row of `out` and its `vals`"
    )]
    fn stack(&self) -> Vec<Vec<Option<(f64, f64)>>> {
        let n = self.n_points();
        match self.kind {
            Kind::Waterfall => return self.waterfall_stack(n),
            Kind::BoxWhisker => return self.box_stack(n),
            _ => {}
        }
        let mut out = vec![vec![None; n]; self.sers.len()];
        if !self.stacked() {
            for (j, s) in self.sers.iter().enumerate() {
                for i in 0..n {
                    out[j][i] = s.vals.get(i).copied().flatten().map(|v| (0.0, v));
                }
            }
            return out;
        }
        for i in 0..n {
            let total: f64 = self
                .sers
                .iter()
                .filter_map(|s| s.vals.get(i).copied().flatten())
                .map(f64::abs)
                .sum();
            let (mut pos, mut neg) = (0.0, 0.0);
            for (j, s) in self.sers.iter().enumerate() {
                let Some(mut v) = s.vals.get(i).copied().flatten() else {
                    if self.kind == Kind::Area || self.kind == Kind::Line {
                        out[j][i] = Some((pos, pos));
                    }
                    continue;
                };
                if self.grouping == Grouping::Percent {
                    v = if total > 0.0 { v / total } else { 0.0 };
                }
                if v >= 0.0 || self.kind == Kind::Line || self.kind == Kind::Area {
                    out[j][i] = Some((pos, pos + v));
                    pos += v;
                } else {
                    out[j][i] = Some((neg, neg + v));
                    neg += v;
                }
            }
        }
        out
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum AxKind {
    Cat,
    Val,
    Date,
}

struct Axis {
    id: String,
    node: Option<Rc<XNode>>,
    kind: AxKind,
    horizontal: bool,
    pos: char,
    deleted: bool,
    reversed: bool,
    cross: String,
    between: bool,
    log: Option<f64>,
    // Value scale.
    lo: f64,
    hi: f64,
    has_data: bool,
    percent: bool,
    min: f64,
    max: f64,
    major: f64,
    // Categories.
    n: usize,
    cats: Vec<String>,
    fmt: String,
    lbl_pos: String,
    major_tick: String,
    minor_tick: String,
    rot: Option<f64>,
    skip: Option<usize>,
    lists: Vec<Rc<XNode>>,
    title: Option<Label>,
    /// (fraction along the axis, label).
    labels: Vec<(f64, Label)>,
    unwrapped: Vec<Label>,
    /// Thickness of the label band beside the axis line.
    band: f64,
    line: f64,
}

impl Axis {
    fn frac(&self, v: f64) -> f64 {
        let t = match self.log {
            Some(b) if b > 1.0 => {
                if v <= 0.0 || self.min <= 0.0 {
                    return 0.0;
                }
                (v.log(b) - self.min.log(b)) / (self.max.log(b) - self.min.log(b))
            }
            _ => (v - self.min) / (self.max - self.min),
        };
        if t.is_finite() { t } else { 0.0 }
    }

    fn cat_frac(&self, i: f64) -> f64 {
        let n = self.n.max(1) as f64;
        if self.between {
            (i + 0.5) / n
        } else if self.n > 1 {
            i / (n - 1.0)
        } else {
            0.5
        }
    }

    fn px(&self, t: f64, p: &Rect) -> f64 {
        let t = if self.reversed { 1.0 - t } else { t };
        if self.horizontal {
            p.x + t * p.w
        } else {
            p.y + p.h - t * p.h
        }
    }

    fn len(&self, p: &Rect) -> f64 {
        if self.horizontal { p.w } else { p.h }
    }

    fn visible_labels(&self) -> bool {
        !self.deleted && self.lbl_pos != "none"
    }

    fn ticks(&self) -> Vec<f64> {
        if self.kind != AxKind::Val {
            return Vec::new();
        }
        if let Some(b) = self.log.filter(|b| *b > 1.0) {
            let mut v = self.min;
            let mut out = Vec::new();
            while v <= self.max * (1.0 + 1e-9) && out.len() < 100 {
                out.push(v);
                v *= b;
            }
            return out;
        }
        let n = ((self.max - self.min) / self.major + 1e-6).floor();
        if !(n.is_finite() && n >= 0.0) || n > 500.0 {
            return vec![self.min, self.max];
        }
        (0..=n as usize)
            .map(|k| clean(self.min + k as f64 * self.major))
            .collect()
    }
}

#[derive(Clone)]
enum Palette {
    Cycle {
        colors: Vec<Rc<XNode>>,
        variations: Vec<Vec<Rc<XNode>>>,
    },
    Linear(Rc<XNode>),
    Gray,
}

fn scheme(name: &str, mods: &[(&str, i64)]) -> Rc<XNode> {
    let children = mods
        .iter()
        .map(|(k, v)| node(k, vec![("val", v.to_string())], vec![]))
        .collect();
    node("a:schemeClr", vec![("val", name.to_string())], children)
}

fn default_variations() -> Vec<Vec<Rc<XNode>>> {
    let v = |mods: &[(&str, i64)]| {
        mods.iter()
            .map(|(k, v)| node(k, vec![("val", v.to_string())], vec![]))
            .collect::<Vec<_>>()
    };
    vec![
        vec![],
        v(&[("a:lumMod", 60_000)]),
        v(&[("a:lumMod", 80_000), ("a:lumOff", 20_000)]),
        v(&[("a:lumMod", 80_000)]),
        v(&[("a:lumMod", 60_000), ("a:lumOff", 40_000)]),
        v(&[("a:lumMod", 50_000)]),
        v(&[("a:lumMod", 70_000), ("a:lumOff", 30_000)]),
        v(&[("a:lumMod", 70_000)]),
        v(&[("a:lumMod", 50_000), ("a:lumOff", 50_000)]),
    ]
}

fn mix(a: Rgba, b: Rgba, t: f32) -> Rgba {
    Rgba {
        r: a.r + (b.r - a.r) * t,
        g: a.g + (b.g - a.g) * t,
        b: a.b + (b.b - a.b) * t,
        a: a.a,
    }
}

#[derive(Clone)]
struct MarkerSpec {
    symbol: String,
    size: f64,
    fill: Option<Paint>,
    stroke: Option<Stroke>,
}

#[derive(Clone)]
#[expect(
    clippy::large_enum_variant,
    reason = "a legend holds a few keys, so the larger line variant costs little"
)]
enum Key {
    Box(Option<Paint>, Option<Stroke>),
    Line(Option<Stroke>, Option<MarkerSpec>),
}

struct LegendEntry {
    label: Label,
    key: Key,
}

struct LegendLayout {
    rect: Rect,
    /// Top-left of each entry inside the chart frame.
    pos: Vec<(f64, f64)>,
    key_w: f64,
    gap: f64,
    row_h: f64,
}

/// What a data label is attached to.
enum Anchor {
    /// A bar: its rectangle and the unit vector from base to end.
    Bar {
        r: Rect,
        dir: (f64, f64),
        pos: String,
    },
    Point {
        x: f64,
        y: f64,
        pos: String,
        marker: f64,
    },
    Slice {
        cx: f64,
        cy: f64,
        r_in: f64,
        r_out: f64,
        ry_scale: f64,
        mid: f64,
        sweep: f64,
        pos: String,
    },
}

struct DLabel {
    label: Label,
    /// Label box fill and outline (`c:dLbls/c:spPr`).
    bg: (Option<Paint>, Option<Stroke>),
    anchor: Anchor,
    offset: (f64, f64),
    manual_abs: Option<(f64, f64)>,
}

struct Renderer<'r, 'a> {
    b: &'r mut SlideBuilder<'a>,
    tf: Transform,
    idx: usize,
    w: f64,
    h: f64,
    part: String,
    root: Rc<XNode>,
    chart: Rc<XNode>,
    colors: ColorScheme,
    base_size: Option<f64>,
    base: Rc<XNode>,
    cs_layer: Option<Rc<XNode>>,
    palette: Palette,
    date1904: bool,
    labels: usize,
    modern: bool,
    /// The chart has its own theme colors (`themeOverride`); text colors are
    /// baked to RGB because the text engine resolves against the slide theme.
    theme_override: bool,
    n_series: usize,
}

impl<'r, 'a> Renderer<'r, 'a> {
    #[expect(
        clippy::too_many_arguments,
        reason = "the renderer is built once per chart from the slide builder, chart parts and frame"
    )]
    fn new(
        b: &'r mut SlideBuilder<'a>,
        data: &ChartData,
        root: Rc<XNode>,
        chart: Rc<XNode>,
        tf: Transform,
        w: f64,
        h: f64,
        idx: usize,
    ) -> Self {
        let mut colors = b
            .chain
            .colors
            .with_map(root.child("c:clrMapOvr").map(|n| n.as_ref()));
        // A chart pasted with its source formatting carries its own theme colors.
        let over = b
            .doc
            .pkg
            .rel_by_kind(&data.part, "themeOverride")
            .and_then(|r| b.doc.xml(&r.target));
        let theme_override = over.as_ref().and_then(|o| o.child("a:clrScheme")).is_some();
        if let Some(cs) = over.as_ref().and_then(|o| o.child("a:clrScheme")) {
            for slot in &cs.children {
                if let Some(c) =
                    crate::office::pptx::color::resolve(slot, &ColorScheme::default(), None)
                {
                    colors.slots.insert(slot.local().to_string(), c);
                }
            }
        }
        let cs_txpr = root.child("c:txPr").cloned();
        let base_size = cs_txpr
            .as_ref()
            .and_then(|t| t.path(&["a:p", "a:pPr", "a:defRPr"]))
            .and_then(|d| d.attr_f64("sz"))
            .map(|v| v / 100.0)
            .filter(|v| *v >= 1.0 && *v <= 400.0);
        let colors_part = b
            .doc
            .pkg
            .rel_by_kind(&data.part, "chartColorStyle")
            .and_then(|r| b.doc.xml(&r.target));
        let modern =
            colors_part.is_some() || b.doc.pkg.rel_by_kind(&data.part, "chartStyle").is_some();
        let style = val_f(&root, "c:style").map(|v| v as i64).unwrap_or(2);
        let palette = colors_part
            .as_ref()
            .and_then(|cp| parse_color_style(cp))
            .unwrap_or_else(|| style_palette(style));
        Renderer {
            b,
            tf,
            idx,
            w,
            h,
            part: data.part.clone(),
            date1904: flag(&root, "c:date1904").unwrap_or(false),
            root,
            chart,
            colors,
            base_size,
            base: base_layer(),
            cs_layer: txpr_layer(cs_txpr.as_ref()),
            palette,
            labels: 0,
            modern,
            theme_override,
            n_series: data.groups.iter().map(|g| g.series.len()).sum(),
        }
    }

    fn ctx(&self) -> StyleCtx<'_> {
        StyleCtx {
            theme: &self.b.chain.theme,
            colors: &self.colors,
            part: &self.part,
            theme_part: &self.b.chain.theme_part,
            pkg: &self.b.doc.pkg,
        }
    }

    fn resolve(&self, clr: &XNode) -> Rgba {
        crate::office::pptx::color::resolve(clr, &self.colors, None).unwrap_or(Rgba::BLACK)
    }

    /// Replaces scheme colors in a text tree with their RGB values.
    fn bake(&self, n: &Rc<XNode>) -> Rc<XNode> {
        if n.name == "a:schemeClr" && n.attr("val") != Some("phClr") {
            let c = self.resolve(n);
            let hex = c.to_argb_hex();
            let mut kids = Vec::new();
            if c.a < 1.0 {
                kids.push(node(
                    "a:alpha",
                    vec![("val", format!("{}", (c.a * 100_000.0).round() as i64))],
                    vec![],
                ));
            }
            return node("a:srgbClr", vec![("val", hex[2..].to_string())], kids);
        }
        if n.children.is_empty() {
            return n.clone();
        }
        let children = n.children.iter().map(|c| self.bake(c)).collect();
        Rc::new(XNode {
            name: n.name.clone(),
            attrs: n.attrs.clone(),
            children,
            text: n.text.clone(),
        })
    }

    fn tx1(&self) -> Rgba {
        self.colors.scheme("tx1").unwrap_or(Rgba::BLACK)
    }

    fn bg1(&self) -> Rgba {
        self.colors.scheme("bg1").unwrap_or(Rgba::WHITE)
    }

    /// Default color of axis lines and gridlines when `spPr` gives none.
    fn line_default(&self, grid: bool) -> Rgba {
        if self.modern {
            mix(self.tx1(), self.bg1(), if grid { 0.85 } else { 0.75 })
        } else {
            mix(self.tx1(), self.bg1(), 0.475)
        }
    }

    /// Automatic color of series (or point) `i` out of `n`.
    fn auto_color(&self, i: usize, n: usize) -> Rgba {
        match &self.palette {
            Palette::Cycle { colors, variations } => {
                if colors.is_empty() {
                    return self.resolve(&scheme("accent1", &[]));
                }
                let base = &colors[i % colors.len()];
                let var = if variations.is_empty() {
                    &[][..]
                } else {
                    &variations[(i / colors.len()) % variations.len()][..]
                };
                let mut n2 = (**base).clone();
                n2.children.extend(var.iter().cloned());
                self.resolve(&n2)
            }
            Palette::Linear(base) => {
                let c = self.resolve(base);
                linear_shade(c, i, n)
            }
            Palette::Gray => {
                let t = if n > 1 {
                    i as f32 / (n - 1) as f32
                } else {
                    0.0
                };
                let l = 0.3 + 0.55 * t;
                Rgba {
                    r: l,
                    g: l,
                    b: l,
                    a: 1.0,
                }
            }
        }
    }

    // ----- fills and lines -------------------------------------------------

    /// Fill from an `spPr`: explicit fill, `None` for `a:noFill`, else `default`.
    fn fill_or(&self, sp: Option<&XNode>, bounds: Rect, default: Option<Paint>) -> Option<Paint> {
        let Some(f) = sp.and_then(style::fill_child) else {
            return default;
        };
        match style::read_fill(f, &self.ctx(), None, bounds) {
            Fill::Paint(p) => Some(p),
            Fill::None => None,
            Fill::Group => default,
        }
    }

    fn has_fill(sp: Option<&XNode>) -> bool {
        sp.and_then(style::fill_child).is_some()
    }

    /// Outline from an `spPr`'s `a:ln`; `default` = (color, width) when the line
    /// is unspecified or has no color of its own.
    fn stroke_or(&self, sp: Option<&XNode>, default: Option<(Rgba, f64)>) -> Option<Stroke> {
        let ln = sp.and_then(|s| s.child("a:ln"));
        let mk = |c: Rgba, w: f64, dash: Vec<f64>| Stroke {
            paint: Paint::Solid(c),
            width: w,
            dash,
            cap: LineCap::Flat,
            join: LineJoin::Round,
            head: None,
            tail: None,
        };
        let Some(ln) = ln else {
            return default.map(|(c, w)| mk(c, w, Vec::new()));
        };
        if style::fill_child(ln).is_some() {
            let bounds = Rect {
                x: 0.0,
                y: 0.0,
                w: self.w,
                h: self.h,
            };
            let mut s = style::read_line(Some(ln), None, &self.ctx(), bounds)?;
            if ln.attr("w").is_none()
                && let Some((_, w)) = default
            {
                s.width = w;
            }
            if ln.attr("cap").is_none() {
                s.cap = LineCap::Flat;
            }
            return Some(s);
        }
        let (c, w0) = default?;
        let w = ln
            .attr_f64("w")
            .map(|v| (v / EMU_PER_PT).max(0.25))
            .unwrap_or(w0);
        let dash = ln
            .child("a:prstDash")
            .map(|d| style::preset_dash(d.attr("val").unwrap_or("solid")))
            .unwrap_or_default();
        Some(mk(c, w, dash))
    }

    // ----- emitting --------------------------------------------------------

    fn push(
        &mut self,
        path: tiny_skia::Path,
        fill: Option<Paint>,
        stroke: Option<Stroke>,
        curve: bool,
    ) {
        if fill.is_none() && stroke.is_none() {
            return;
        }
        let Some(p) = path.transform(self.tf) else {
            return;
        };
        let fill = fill.map(|f| map_paint(f, &self.tf));
        let stroke = stroke.map(|mut s| {
            s.paint = map_paint(s.paint, &self.tf);
            s
        });
        self.b.scene.items.push(Item::Path(PathItem {
            path: p,
            fill,
            stroke,
            has_curve: curve,
            shape: Some(self.idx),
            from_text: false,
        }));
    }

    fn push_rect(&mut self, r: Rect, fill: Option<Paint>, stroke: Option<Stroke>) {
        if let Some(p) = crate::office::pptx::textlayout::rect_path(r.x, r.y, r.w, r.h) {
            self.push(p, fill, stroke, false);
        }
    }

    fn push_line(&mut self, pts: &[(f64, f64)], stroke: Option<Stroke>) {
        if stroke.is_none() || pts.len() < 2 {
            return;
        }
        let mut pb = PathBuilder::new();
        pb.move_to(pts[0].0 as f32, pts[0].1 as f32);
        for p in &pts[1..] {
            pb.line_to(p.0 as f32, p.1 as f32);
        }
        if let Some(path) = pb.finish() {
            self.push(path, None, stroke, false);
        }
    }

    fn push_clip(&mut self, r: Rect) {
        if let Some(p) = crate::office::pptx::textlayout::rect_path(r.x, r.y, r.w, r.h)
            .and_then(|p| p.transform(self.tf))
        {
            self.b.scene.items.push(Item::PushClip(p));
        }
    }

    // ----- text --------------------------------------------------------------

    fn lists(&self, kind: TextKind, own: &[Option<&Rc<XNode>>]) -> Vec<Rc<XNode>> {
        let mut v = vec![self.base.clone()];
        if let Some(sz) = self.base_size {
            v.push(size_layer(sz, None));
        }
        if let Some(l) = &self.cs_layer {
            v.push(l.clone());
        }
        match kind {
            TextKind::Title => v.push(size_layer(
                self.base_size.map(|s| s * 1.2).unwrap_or(18.0),
                Some(true),
            )),
            TextKind::AxisTitle => v.push(size_layer(self.base_size.unwrap_or(10.0), Some(true))),
            TextKind::Body => {}
        }
        for t in own {
            if let Some(l) = txpr_layer(*t) {
                v.push(l);
            }
        }
        v
    }

    fn measure(&mut self, tx: &XNode, lists: &[Rc<XNode>], wrap_w: Option<f64>) -> (f64, f64) {
        let saved = std::mem::take(&mut self.b.scene);
        self.b.scene.shapes.push(ShapeInfo {
            id: String::new(),
            name: String::new(),
            parent: None,
            kind: "measure",
            placeholder: None,
            bounds: Rect {
                x: 0.0,
                y: 0.0,
                w: 0.0,
                h: 0.0,
            },
            rotation: 0.0,
            alt_text: None,
            hyperlink: None,
            paragraphs: Vec::new(),
            inherited: false,
            hidden: false,
        });
        let rect = TextRect {
            l: 0.0,
            t: 0.0,
            r: wrap_w.unwrap_or(100_000.0),
            b: 100_000.0,
        };
        let h = self.b.layout_text(
            tx,
            lists,
            &[body_pr(wrap_w.is_some())],
            &self.part,
            rect,
            Transform::identity(),
            None,
            None,
            0,
        );
        let (mut x0, mut x1) = (f64::MAX, f64::MIN);
        for it in &self.b.scene.items {
            if let Item::Text(r) = it {
                let x = r.transform.tx as f64;
                x0 = x0.min(x);
                x1 = x1.max(x + r.width);
            }
        }
        self.b.scene = saved;
        (if x1 > x0 { x1 - x0 } else { 0.0 }, h)
    }

    fn label_tx(
        &mut self,
        tx: Rc<XNode>,
        lists: Vec<Rc<XNode>>,
        rot: f64,
        wrap_w: Option<f64>,
    ) -> Option<Label> {
        if self.labels >= MAX_LABELS {
            return None;
        }
        let (tx, lists) = if self.theme_override {
            (self.bake(&tx), lists.iter().map(|l| self.bake(l)).collect())
        } else {
            (tx, lists)
        };
        let (w, h) = self.measure(&tx, &lists, wrap_w);
        if h <= 0.0 {
            return None;
        }
        self.labels += 1;
        let size = resolve_para(&lists, 0, None).def_run.size.unwrap_or(10.0);
        Some(Label {
            tx,
            lists,
            w,
            h,
            box_w: wrap_w.unwrap_or(w + 1.0),
            wrap: wrap_w.is_some(),
            rot,
            size,
        })
    }

    fn label(
        &mut self,
        text: &str,
        lists: &[Rc<XNode>],
        rot: f64,
        wrap_w: Option<f64>,
    ) -> Option<Label> {
        if text.trim().is_empty() {
            return None;
        }
        self.label_tx(plain_body(text), lists.to_vec(), rot, wrap_w)
    }

    /// Re-lays a label wrapped at `wrap_w` (or unwrapped).
    fn relabel(&mut self, lab: &Label, wrap_w: Option<f64>, rot: f64) -> Label {
        let (w, h) = self.measure(&lab.tx, &lab.lists, wrap_w);
        Label {
            w,
            h,
            box_w: wrap_w.unwrap_or(w + 1.0),
            wrap: wrap_w.is_some(),
            rot,
            ..lab.clone()
        }
    }

    /// Draws a label with its (rotated) center at `(cx, cy)`.
    fn draw_label(&mut self, lab: &Label, cx: f64, cy: f64) {
        if !(cx.is_finite() && cy.is_finite()) {
            return;
        }
        let rect = TextRect {
            l: cx - lab.box_w / 2.0,
            t: cy - lab.h / 2.0,
            r: cx + lab.box_w / 2.0,
            b: cy + lab.h / 2.0 + 1.0,
        };
        let t = if lab.rot.abs() > 1e-6 {
            Transform::from_rotate_at(lab.rot as f32, cx as f32, cy as f32).post_concat(self.tf)
        } else {
            self.tf
        };
        self.b.layout_text(
            &lab.tx,
            &lab.lists,
            &[body_pr(lab.wrap)],
            &self.part,
            rect,
            t,
            None,
            None,
            self.idx,
        );
    }

    // ----- top level -----------------------------------------------------

    fn draw(&mut self, data: &ChartData) -> Vec<String> {
        let (w, h) = (self.w, self.h);
        let frame = Rect {
            x: 0.0,
            y: 0.0,
            w,
            h,
        };
        let cs_sp = self.root.child("c:spPr").cloned();
        let fill = self.fill_or(cs_sp.as_deref(), frame, None);
        let stroke = self.stroke_or(cs_sp.as_deref(), None);
        self.push_rect(frame, fill, stroke);

        let mut skipped = Vec::new();
        let plot_area = self.chart.child("c:plotArea").cloned();
        let mut groups = Self::collect_groups(data, &mut skipped);
        // Title and legend still render for charts with nothing to plot.
        if groups.is_empty() && skipped.is_empty() {
            skipped.push("empty".to_string());
        }

        // Title.
        let title_node = self.chart.child("c:title").cloned();
        let title = self.title_label(title_node.as_deref(), data);
        let mut top = PAD;
        let mut title_pos = None;
        if let (Some(t), Some(tn)) = (&title, &title_node) {
            let (ew, eh) = t.ext();
            let mut cx = w / 2.0;
            let mut cy = PAD + eh / 2.0;
            if let Some(m) = manual_layout(Some(tn)) {
                if let Some(x) = m.x {
                    cx = if m.x_edge {
                        x * w + ew / 2.0
                    } else {
                        cx + x * w
                    };
                }
                if let Some(y) = m.y {
                    cy = if m.y_edge {
                        y * h + eh / 2.0
                    } else {
                        cy + y * h
                    };
                }
            }
            title_pos = Some((cx, cy));
            if !flag(tn, "c:overlay").unwrap_or(false) {
                top = top.max(cy + eh / 2.0 + t.size * 0.5);
            }
        }

        // Legend.
        let legend_node = self.chart.child("c:legend").cloned();
        let entries = match &legend_node {
            Some(ln) => self.legend_entries(ln, &groups),
            None => Vec::new(),
        };
        let mut outer = Rect {
            x: PAD,
            y: top,
            w: w - 2.0 * PAD,
            h: h - PAD - top,
        };
        let legend = legend_node
            .as_ref()
            .filter(|_| !entries.is_empty())
            .map(|ln| self.legend_layout(ln, &entries, top));
        if let (Some(ln), Some(lay)) = (&legend_node, &legend) {
            let overlay = flag(ln, "c:overlay").unwrap_or(false);
            if !overlay && manual_layout(Some(ln)).is_none() {
                let gap = lay.row_h * 0.5;
                match val(ln, "c:legendPos").unwrap_or("r") {
                    "b" => outer.h = (lay.rect.y - gap - outer.y).max(1.0),
                    "t" => {
                        let ny = lay.rect.y + lay.rect.h + gap;
                        outer.h = (outer.y + outer.h - ny).max(1.0);
                        outer.y = ny;
                    }
                    "l" => {
                        let nx = lay.rect.x + lay.rect.w + gap;
                        outer.w = (outer.x + outer.w - nx).max(1.0);
                        outer.x = nx;
                    }
                    _ => outer.w = (lay.rect.x - gap - outer.x).max(1.0),
                }
            }
        }

        // Plot area rectangle (manual layout wins).
        let ml = manual_layout(plot_area.as_deref());
        let mut manual_inner = None;
        if let Some(m) = ml
            && let (Some(x), Some(y), Some(mw), Some(mh)) = (m.x, m.y, m.w, m.h)
        {
            let r = Rect {
                x: if m.x_edge { x * w } else { outer.x + x * w },
                y: if m.y_edge { y * h } else { outer.y + y * h },
                w: (mw * w).max(1.0),
                h: (mh * h).max(1.0),
            };
            if m.inner {
                manual_inner = Some(r);
            } else {
                outer = r;
            }
        }

        let pie_mode = !groups.is_empty()
            && groups
                .iter()
                .all(|g| matches!(g.kind, Kind::Pie | Kind::Pie3D | Kind::Doughnut));
        let free_mode = groups.iter().any(|g| {
            matches!(
                g.kind,
                Kind::Radar { .. } | Kind::Surface { .. } | Kind::Treemap | Kind::Sunburst
            )
        });
        let plot_sp = plot_area.as_ref().and_then(|p| p.child("c:spPr").cloned());
        let mut dlabels = Vec::new();
        if free_mode {
            let rect = manual_inner.unwrap_or(outer);
            let plot_fill = self.fill_or(plot_sp.as_deref(), rect, None);
            let plot_stroke = self.stroke_or(plot_sp.as_deref(), None);
            self.push_rect(rect, plot_fill, plot_stroke);
            self.draw_free(
                &groups,
                plot_area.as_deref(),
                rect,
                manual_inner.is_some(),
                &mut dlabels,
            );
        } else if pie_mode {
            let rect = manual_inner.unwrap_or(outer);
            let plot_fill = self.fill_or(plot_sp.as_deref(), rect, None);
            let plot_stroke = self.stroke_or(plot_sp.as_deref(), None);
            self.push_rect(rect, plot_fill, plot_stroke);
            for g in &groups {
                self.draw_pie(g, rect, manual_inner.is_some(), &mut dlabels);
            }
        } else if !groups.is_empty() {
            let mut axes = self.collect_axes(plot_area.as_deref(), &mut groups);
            let inner = self.layout_axes(&mut axes, &groups, outer, manual_inner);
            let plot_fill = self.fill_or(plot_sp.as_deref(), inner, None);
            let plot_stroke = self.stroke_or(plot_sp.as_deref(), None);
            self.push_rect(inner, plot_fill, None);
            self.draw_gridlines(&axes, &inner);
            let mut order: Vec<usize> = (0..groups.len()).collect();
            let rank = |g: &Grp| match g.kind {
                Kind::Area => 0,
                Kind::Bar { .. } => 1,
                Kind::Line => 2,
                _ => 3,
            };
            order.sort_by_key(|&i| (groups[i].secondary as u8, rank(&groups[i])));
            for i in order {
                let g = &groups[i];
                match g.kind {
                    Kind::Bar { .. } => self.draw_bars(g, &axes, &inner, &mut dlabels),
                    Kind::Line => self.draw_lines(g, &axes, &inner, &mut dlabels),
                    Kind::Area => self.draw_area(g, &axes, &inner, &mut dlabels),
                    Kind::Scatter | Kind::Bubble => {
                        self.draw_scatter(g, &axes, &inner, &mut dlabels)
                    }
                    Kind::Waterfall => self.draw_waterfall(g, &axes, &inner, &mut dlabels),
                    Kind::Funnel => self.draw_funnel(g, &axes, &inner, &mut dlabels),
                    Kind::BoxWhisker => self.draw_box_whisker(g, &axes, &inner),
                    _ => {}
                }
            }
            if plot_stroke.is_some() {
                self.push_rect(inner, None, plot_stroke);
            }
            self.draw_axes(&axes, &inner);
        }
        for d in &dlabels {
            self.place_dlabel(d);
        }
        if let (Some(t), Some((cx, cy))) = (&title, title_pos) {
            if let Some(tn) = &title_node {
                let sp = tn.child("c:spPr").cloned();
                let (ew, eh) = t.ext();
                let r = Rect {
                    x: cx - ew / 2.0 - 3.0,
                    y: cy - eh / 2.0 - 2.0,
                    w: ew + 6.0,
                    h: eh + 4.0,
                };
                let f = self.fill_or(sp.as_deref(), r, None);
                let s = self.stroke_or(sp.as_deref(), None);
                self.push_rect(r, f, s);
            }
            self.draw_label(t, cx, cy);
        }
        if let (Some(ln), Some(lay)) = (&legend_node, &legend) {
            self.draw_legend(ln, &entries, lay);
        }
        self.draw_user_shapes();
        skipped
    }

    /// Shapes drawn over the chart (`c:userShapes`, a chartDrawing part):
    /// text boxes and simple shapes anchored by fractions of the chart frame.
    fn draw_user_shapes(&mut self) {
        let Some(rel) = self.b.doc.pkg.rel_by_kind(&self.part, "chartUserShapes") else {
            return;
        };
        let Some(root) = self.b.doc.xml(&rel.target) else {
            return;
        };
        for anchor in root.children.iter().take(200) {
            let (x0, y0, x1, y1) = match anchor.local() {
                "relSizeAnchor" => {
                    let pt = |n: Option<&Rc<XNode>>| {
                        let f = |k: &str| {
                            n.and_then(|n| n.child(k))
                                .and_then(|c| c.text.trim().parse::<f64>().ok())
                                .filter(|v| v.is_finite())
                                .unwrap_or(0.0)
                        };
                        (f("x:x") * self.w, f("x:y") * self.h)
                    };
                    let (a, b) = (pt(anchor.child("x:from")), pt(anchor.child("x:to")));
                    (a.0, a.1, b.0, b.1)
                }
                "absSizeAnchor" => {
                    let f = anchor.child("x:from");
                    let g = |k: &str| {
                        f.and_then(|n| n.child(k))
                            .and_then(|c| c.text.trim().parse::<f64>().ok())
                            .filter(|v| v.is_finite())
                            .unwrap_or(0.0)
                    };
                    let (x, y) = (g("x:x") * self.w, g("x:y") * self.h);
                    let ext = anchor.child("x:ext");
                    let e = |k: &str| ext.and_then(|n| n.attr_f64(k)).unwrap_or(0.0) / EMU_PER_PT;
                    (x, y, x + e("cx"), y + e("cy"))
                }
                _ => continue,
            };
            for sp in anchor
                .children
                .iter()
                .filter(|c| matches!(c.local(), "sp" | "cxnSp"))
            {
                self.user_shape(sp, rect_xyxy(x0, y0, x1, y1));
            }
        }
    }

    fn user_shape(&mut self, sp: &XNode, r: Rect) {
        if !(r.w.is_finite() && r.h.is_finite()) || r.w > self.w * 20.0 || r.h > self.h * 20.0 {
            return;
        }
        let sp_pr = sp.child("x:spPr").cloned();
        let xf = sp_pr.as_ref().and_then(|p| p.child("a:xfrm"));
        let rot = xf.and_then(|x| x.attr_f64("rot")).unwrap_or(0.0) / 60_000.0;
        let (fh, fv) = (
            xf.and_then(|x| x.attr_bool("flipH")).unwrap_or(false),
            xf.and_then(|x| x.attr_bool("flipV")).unwrap_or(false),
        );
        let (cx, cy) = ((r.w / 2.0) as f32, (r.h / 2.0) as f32);
        let local = Transform::from_translate(-cx, -cy)
            .post_concat(Transform::from_scale(
                if fh { -1.0 } else { 1.0 },
                if fv { -1.0 } else { 1.0 },
            ))
            .post_concat(Transform::from_rotate(rot as f32))
            .post_concat(Transform::from_translate(r.x as f32 + cx, r.y as f32 + cy));
        let shape_tf = local.post_concat(self.tf);
        let geom_node = sp_pr
            .as_ref()
            .and_then(|p| p.child("a:prstGeom").or(p.child("a:custGeom")).cloned());
        let geometry = crate::office::pptx::geom::build(
            geom_node.as_deref(),
            r.w * EMU_PER_PT,
            r.h * EMU_PER_PT,
        );
        let style_el = sp.child("x:style").cloned();
        let fill_ref = style_el
            .as_ref()
            .and_then(|s| s.child("a:fillRef").cloned());
        let ln_ref = style_el.as_ref().and_then(|s| s.child("a:lnRef").cloned());
        let local_bounds = Rect {
            x: 0.0,
            y: 0.0,
            w: r.w,
            h: r.h,
        };
        let (fill, stroke, font_color) = {
            let ctx = self.ctx();
            let fill = style::shape_fill(sp_pr.as_deref(), fill_ref.as_deref(), &ctx, local_bounds);
            let ln = sp_pr.as_ref().and_then(|p| p.child("a:ln").cloned());
            let stroke = style::read_line(ln.as_deref(), ln_ref.as_deref(), &ctx, local_bounds);
            let fc = style_el
                .as_ref()
                .and_then(|s| s.child("a:fontRef"))
                .and_then(|f| ctx.color(f, None));
            (fill, stroke, fc)
        };
        self.b
            .emit_geometry(&geometry, fill, stroke, shape_tf, Some(self.idx));
        let Some(tx) = sp.child("x:txBody").cloned() else {
            return;
        };
        let mut lists = Vec::new();
        if let Some(d) = self.b.doc.default_text_style() {
            lists.push(d.clone());
        }
        if let Some(ls) = tx.child("a:lstStyle") {
            lists.push(ls.clone());
        }
        let bodies: Vec<Rc<XNode>> = tx.child("a:bodyPr").cloned().into_iter().collect();
        let tr = geometry.text_rect;
        let text_tf = Transform::from_translate(-cx, -cy)
            .post_concat(Transform::from_rotate(rot as f32))
            .post_concat(Transform::from_translate(r.x as f32 + cx, r.y as f32 + cy))
            .post_concat(self.tf);
        self.b.layout_text(
            &tx, &lists, &bodies, &self.part, tr, text_tf, font_color, None, self.idx,
        );
    }

    // ----- model ---------------------------------------------------------

    fn collect_groups(data: &ChartData, skipped: &mut Vec<String>) -> Vec<Grp> {
        let mut out = Vec::new();
        for g in &data.groups {
            let kind = match g.kind.as_str() {
                "barChart" | "bar3DChart" => Kind::Bar {
                    horizontal: val(&g.node, "c:barDir") == Some("bar"),
                },
                "lineChart" | "line3DChart" | "stockChart" => Kind::Line,
                "areaChart" | "area3DChart" => Kind::Area,
                "scatterChart" => Kind::Scatter,
                "bubbleChart" => Kind::Bubble,
                "pieChart" | "ofPieChart" => Kind::Pie,
                "pie3DChart" => Kind::Pie3D,
                "doughnutChart" => Kind::Doughnut,
                "radarChart" => Kind::Radar {
                    filled: val(&g.node, "c:radarStyle") == Some("filled"),
                },
                "surfaceChart" => Kind::Surface { three_d: false },
                "surface3DChart" => Kind::Surface { three_d: true },
                "cxWaterfallChart" => Kind::Waterfall,
                "cxFunnelChart" => Kind::Funnel,
                "cxBoxWhiskerChart" => Kind::BoxWhisker,
                "cxTreemapChart" => Kind::Treemap,
                "cxSunburstChart" => Kind::Sunburst,
                other => {
                    skipped.push(other.to_string());
                    continue;
                }
            };
            let grouping = match val(&g.node, "c:grouping") {
                Some("stacked") => Grouping::Stacked,
                Some("percentStacked") => Grouping::Percent,
                Some("clustered") => Grouping::Clustered,
                Some("standard") => {
                    if matches!(kind, Kind::Bar { .. }) {
                        Grouping::Clustered
                    } else {
                        Grouping::Standard
                    }
                }
                _ => {
                    if matches!(kind, Kind::Bar { .. }) {
                        Grouping::Clustered
                    } else {
                        Grouping::Standard
                    }
                }
            };
            let ser_nodes: Vec<Rc<XNode>> = g.node.children_named("c:ser").cloned().collect();
            let mut sers: Vec<(i64, SerInfo)> = g
                .series
                .iter()
                .zip(ser_nodes.iter())
                .map(|(s, n)| {
                    let order = val_f(n, "c:order")
                        .map(|v| v as i64)
                        .unwrap_or(s.index as i64);
                    (order, Self::ser_info(s, n))
                })
                .collect();
            sers.sort_by_key(|(o, _)| *o);
            let mut ids = g
                .node
                .children_named("c:axId")
                .filter_map(|a| a.attr("val").map(str::to_string));
            let ax = (ids.next(), ids.next());
            out.push(Grp {
                kind,
                node: g.node.clone(),
                sers: sers.into_iter().map(|(_, s)| s).take(255).collect(),
                grouping,
                vary: flag(&g.node, "c:varyColors").unwrap_or(false),
                ax,
                cat: 0,
                val: 0,
                secondary: false,
            });
        }
        out
    }

    fn ser_info(s: &Series, n: &Rc<XNode>) -> SerInfo {
        let val_node = n.child("c:val").or(n.child("c:yVal"));
        let fmt = val_node
            .and_then(|v| {
                v.child("c:numRef")
                    .and_then(|r| r.child("c:numCache"))
                    .or(v.child("c:numLit"))
            })
            .and_then(|c| c.child("c:formatCode"))
            .map(|f| f.text.trim().to_string())
            .filter(|f| !f.is_empty())
            .unwrap_or_else(|| "General".to_string());
        let cat_node = n.child("c:cat").or(n.child("c:xVal"));
        let cat_fmt = cat_node
            .and_then(|v| {
                v.child("c:numRef")
                    .and_then(|r| r.child("c:numCache"))
                    .or(v.child("c:numLit"))
            })
            .and_then(|c| c.child("c:formatCode"))
            .map(|f| f.text.trim().to_string());
        let xs = s
            .x_values
            .iter()
            .map(|v| v.trim().parse::<f64>().ok().filter(|v| v.is_finite()))
            .collect();
        let mut range_texts = Vec::new();
        if let Some(dr) = n.find("x:datalabelsRange")
            && let Some(cache) = dr.child("x:dlblRangeCache")
        {
            for p in cache.children_named("c:pt") {
                let i = p.attr_i64("idx").unwrap_or(0).max(0) as usize;
                if i < MAX_POINTS {
                    if range_texts.len() <= i {
                        range_texts.resize(i + 1, String::new());
                    }
                    range_texts[i] = p.child("c:v").map(|v| v.text.clone()).unwrap_or_default();
                }
            }
        }
        SerInfo {
            vals: s
                .values
                .iter()
                .take(MAX_POINTS)
                .map(|v| v.filter(|x| x.is_finite()))
                .collect(),
            xs,
            s: s.clone(),
            node: n.clone(),
            fmt,
            cat_fmt,
            range_texts,
        }
    }

    /// Category label `i` of a series, formatted with the cache's format code.
    fn cat_text(&self, s: &SerInfo, i: usize, axis_fmt: Option<&str>) -> String {
        let raw = s.s.categories.get(i).cloned().unwrap_or_default();
        let fmt = axis_fmt.or(s.cat_fmt.as_deref());
        match (raw.trim().parse::<f64>(), fmt) {
            (Ok(v), Some(f)) if s.cat_fmt.is_some() => format_value(v, f, self.date1904),
            _ => raw,
        }
    }

    fn title_label(&mut self, tn: Option<&XNode>, data: &ChartData) -> Option<Label> {
        let tn = tn?;
        let txpr = tn.child("c:txPr").cloned();
        let mut lists = self.lists(TextKind::Title, &[txpr.as_ref()]);
        let rich = tn.path(&["c:tx", "c:rich"]).cloned();
        let max_w = self.w * 0.8;
        if let Some(rich) = rich {
            if rich.all_text().trim().is_empty() {
                return None;
            }
            if let Some(ls) = rich.child("a:lstStyle") {
                lists.push(ls.clone());
            }
            let lab = self.label_tx(rich, lists, 0.0, None)?;
            return Some(if lab.w > max_w {
                self.relabel(&lab, Some(max_w), 0.0)
            } else {
                lab
            });
        }
        let text = tn
            .path(&["c:tx", "c:strRef", "c:strCache"])
            .and_then(|c| {
                c.children_named("c:pt")
                    .next()
                    .and_then(|p| p.child("c:v"))
                    .map(|v| v.text.clone())
            })
            .or_else(|| {
                let all: Vec<&Series> = data.groups.iter().flat_map(|g| g.series.iter()).collect();
                if all.len() == 1 {
                    all[0].name.clone()
                } else {
                    Some("Chart Title".to_string())
                }
            })?;
        let lab = self.label(&text, &lists, 0.0, None)?;
        Some(if lab.w > max_w {
            self.relabel(&lab, Some(max_w), 0.0)
        } else {
            lab
        })
    }

    fn collect_axes(&mut self, plot: Option<&XNode>, groups: &mut [Grp]) -> Vec<Axis> {
        let mut axes: Vec<Axis> = Vec::new();
        if let Some(p) = plot {
            for a in &p.children {
                let kind = match a.name.as_str() {
                    "c:catAx" => AxKind::Cat,
                    "c:valAx" => AxKind::Val,
                    "c:dateAx" => AxKind::Date,
                    _ => continue,
                };
                axes.push(Self::read_axis(a, kind));
            }
        }
        // Wire groups to axes, synthesizing a pair when ids do not resolve.
        let first_ids = groups.first().map(|g| g.ax.clone());
        for g in groups.iter_mut() {
            let xy = matches!(g.kind, Kind::Scatter | Kind::Bubble);
            let hbar = matches!(g.kind, Kind::Bar { horizontal: true } | Kind::Funnel);
            let find = |axes: &[Axis], id: &Option<String>| {
                id.as_ref()
                    .and_then(|id| axes.iter().position(|a| &a.id == id))
            };
            let cat = match find(&axes, &g.ax.0) {
                Some(i) => i,
                None => {
                    let mut a = synth_axis(
                        if xy { AxKind::Val } else { AxKind::Cat },
                        if hbar { 'l' } else { 'b' },
                    );
                    a.id =
                        g.ax.0
                            .clone()
                            .unwrap_or_else(|| format!("auto-cat-{}", axes.len()));
                    axes.push(a);
                    axes.len() - 1
                }
            };
            let val_i = match find(&axes, &g.ax.1) {
                Some(i) => i,
                None => {
                    let mut a = synth_axis(AxKind::Val, if hbar { 'b' } else { 'l' });
                    a.id =
                        g.ax.1
                            .clone()
                            .unwrap_or_else(|| format!("auto-val-{}", axes.len()));
                    axes.push(a);
                    axes.len() - 1
                }
            };
            g.cat = cat;
            g.val = val_i;
            g.secondary = first_ids.as_ref().map(|f| f != &g.ax).unwrap_or(false);
            // Category-role axes: scatter X stays a value axis; others become categories.
            if !xy && axes[cat].kind == AxKind::Val {
                axes[cat].kind = AxKind::Cat;
            }
            if hbar {
                axes[cat].horizontal = false;
                axes[val_i].horizontal = true;
                if !matches!(axes[cat].pos, 'l' | 'r') {
                    axes[cat].pos = 'l';
                }
                if !matches!(axes[val_i].pos, 't' | 'b') {
                    axes[val_i].pos = 'b';
                }
            } else {
                if !matches!(axes[cat].pos, 't' | 'b') {
                    axes[cat].pos = 'b';
                }
                if !matches!(axes[val_i].pos, 'l' | 'r') {
                    axes[val_i].pos = 'l';
                }
                axes[cat].horizontal = true;
                axes[val_i].horizontal = false;
            }
        }
        // crossBetween lives on the value axis and decides category placement.
        for i in 0..axes.len() {
            if axes[i].kind == AxKind::Val {
                continue;
            }
            let cross = axes[i].cross.clone();
            let cb = axes.iter().find(|a| a.id == cross).and_then(|a| {
                a.node
                    .as_ref()
                    .and_then(|n| val(n, "c:crossBetween").map(str::to_string))
            });
            let area_only = groups
                .iter()
                .filter(|g| g.cat == i)
                .all(|g| g.kind == Kind::Area);
            axes[i].between = match cb.as_deref() {
                Some("midCat") => false,
                Some(_) => true,
                None => !area_only,
            };
        }
        // Data ranges.
        for g in groups.iter() {
            let stack = g.stack();
            let xy = matches!(g.kind, Kind::Scatter | Kind::Bubble);
            {
                let a = &mut axes[g.val];
                for col in &stack {
                    for (base, top) in col.iter().flatten() {
                        // Unstacked bases are a placeholder zero, not data.
                        let base = if g.stacked() { *base } else { *top };
                        let lo = base.min(*top);
                        let hi = base.max(*top);
                        if !a.has_data {
                            a.lo = lo;
                            a.hi = hi;
                            a.has_data = true;
                        } else {
                            a.lo = a.lo.min(lo);
                            a.hi = a.hi.max(hi);
                        }
                    }
                }
                if g.grouping == Grouping::Percent {
                    a.percent = true;
                }
            }
            let n = g.n_points();
            let a = &mut axes[g.cat];
            if xy {
                for s in &g.sers {
                    let numeric = !s.xs.is_empty() && s.xs.iter().any(|x| x.is_some());
                    for i in 0..s.vals.len() {
                        let x = if numeric {
                            s.xs.get(i).copied().flatten()
                        } else {
                            Some(i as f64 + 1.0)
                        };
                        if let Some(x) = x {
                            if !a.has_data {
                                a.lo = x;
                                a.hi = x;
                                a.has_data = true;
                            } else {
                                a.lo = a.lo.min(x);
                                a.hi = a.hi.max(x);
                            }
                        }
                    }
                }
            } else if n > a.n {
                a.n = n;
                let axis_fmt = a
                    .node
                    .as_ref()
                    .and_then(|nd| nd.child("c:numFmt").cloned())
                    .filter(|f| f.attr_bool("sourceLinked") != Some(true))
                    .and_then(|f| f.attr("formatCode").map(str::to_string));
                let src = g.sers.iter().find(|s| !s.s.categories.is_empty());
                let cats: Vec<String> = match src {
                    _ if g.kind == Kind::BoxWhisker => g.box_cats(),
                    Some(s) => (0..n)
                        .map(|i| self.cat_text(s, i, axis_fmt.as_deref()))
                        .collect(),
                    None => (1..=n).map(|i| i.to_string()).collect(),
                };
                axes[g.cat].cats = cats;
            }
        }
        // Value-axis number formats linked to the source take the series' code.
        #[expect(
            clippy::needless_range_loop,
            reason = "`i` is also compared with each group's axis indexes (`g.cat`, `g.val`)"
        )]
        for i in 0..axes.len() {
            let linked = axes[i]
                .node
                .as_ref()
                .and_then(|n| n.child("c:numFmt").cloned());
            let code = linked
                .as_ref()
                .and_then(|f| f.attr("formatCode"))
                .unwrap_or("General")
                .to_string();
            let source_linked = linked
                .as_ref()
                .map(|f| f.attr_bool("sourceLinked").unwrap_or(false))
                .unwrap_or(true);
            let fmt = if axes[i].kind == AxKind::Val && (source_linked || linked.is_none()) {
                if axes[i].percent {
                    "0%".to_string()
                } else {
                    let xy_x = groups
                        .iter()
                        .any(|g| g.cat == i && matches!(g.kind, Kind::Scatter | Kind::Bubble));
                    let g = groups.iter().find(|g| g.val == i || g.cat == i);
                    g.and_then(|g| g.sers.first())
                        .map(|s| {
                            if xy_x {
                                s.cat_fmt.clone().unwrap_or_else(|| "General".into())
                            } else {
                                s.fmt.clone()
                            }
                        })
                        .unwrap_or(code)
                }
            } else {
                code
            };
            axes[i].fmt = fmt;
        }
        // Axis text.
        for axis in &mut axes {
            let txpr = axis.node.as_ref().and_then(|n| n.child("c:txPr").cloned());
            axis.lists = self.lists(TextKind::Body, &[txpr.as_ref()]);
            axis.rot = txpr_rot(txpr.as_ref());
            if let Some(tn) = axis.node.as_ref().and_then(|n| n.child("c:title").cloned()) {
                let rot_default = if axis.horizontal { 0.0 } else { -90.0 };
                let ttx = tn.child("c:txPr").cloned();
                let rich = tn.path(&["c:tx", "c:rich"]).cloned();
                let rot = rich
                    .as_ref()
                    .and_then(|r| txpr_rot(Some(r)))
                    .or(txpr_rot(ttx.as_ref()))
                    .unwrap_or(rot_default);
                let mut lists = self.lists(TextKind::AxisTitle, &[ttx.as_ref()]);
                axis.title = match rich {
                    Some(r) if !r.all_text().trim().is_empty() => {
                        if let Some(ls) = r.child("a:lstStyle") {
                            lists.push(ls.clone());
                        }
                        self.label_tx(r, lists, rot, None)
                    }
                    _ => {
                        let t = tn
                            .path(&["c:tx", "c:strRef", "c:strCache"])
                            .and_then(|c| {
                                c.children_named("c:pt")
                                    .next()
                                    .and_then(|p| p.child("c:v"))
                                    .map(|v| v.text.clone())
                            })
                            .unwrap_or_else(|| "Axis Title".to_string());
                        self.label(&t, &lists, rot, None)
                    }
                };
            }
        }
        axes
    }

    fn read_axis(a: &Rc<XNode>, kind: AxKind) -> Axis {
        let scaling = a.child("c:scaling");
        let sf = |k: &str| scaling.and_then(|s| val_f(s, k));
        let mut ax = synth_axis(
            kind,
            val(a, "c:axPos")
                .and_then(|p| p.chars().next())
                .unwrap_or('b'),
        );
        ax.id = val(a, "c:axId").unwrap_or("").to_string();
        ax.node = Some(a.clone());
        ax.deleted = flag(a, "c:delete").unwrap_or(false);
        ax.reversed = scaling.and_then(|s| val(s, "c:orientation")) == Some("maxMin");
        ax.cross = val(a, "c:crossAx").unwrap_or("").to_string();
        ax.log = sf("c:logBase").filter(|b| *b >= 2.0 && *b <= 1000.0);
        ax.lbl_pos = val(a, "c:tickLblPos").unwrap_or("nextTo").to_string();
        ax.major_tick = val(a, "c:majorTickMark").unwrap_or("out").to_string();
        ax.minor_tick = val(a, "c:minorTickMark").unwrap_or("none").to_string();
        ax.skip = val_f(a, "c:tickLblSkip")
            .filter(|v| *v >= 1.0)
            .map(|v| v.min(10_000.0) as usize);
        ax.horizontal = matches!(ax.pos, 't' | 'b');
        ax
    }

    // ----- axis layout ---------------------------------------------------

    fn axis_fixed(a: &Axis) -> (Option<f64>, Option<f64>, Option<f64>) {
        let n = a.node.as_ref();
        let sc = n.and_then(|n| n.child("c:scaling"));
        let fmin = sc.and_then(|s| val_f(s, "c:min"));
        let fmax = sc.and_then(|s| val_f(s, "c:max"));
        let fmajor = n.and_then(|n| val_f(n, "c:majorUnit")).filter(|v| *v > 0.0);
        (fmin, fmax, fmajor)
    }

    fn scale_axis(&mut self, a: &mut Axis, len: f64) {
        let (mut fmin, mut fmax, fmajor) = Self::axis_fixed(a);
        let (lo, hi) = if a.has_data { (a.lo, a.hi) } else { (0.0, 1.0) };
        if let Some(b) = a.log {
            let lo_p = if lo > 0.0 { lo } else { 1.0 };
            let hi_p = if hi > lo_p { hi } else { lo_p * b };
            let min = fmin
                .filter(|v| *v > 0.0)
                .unwrap_or_else(|| b.powf(lo_p.log(b).floor()));
            let max = fmax
                .filter(|v| *v > min)
                .unwrap_or_else(|| b.powf(hi_p.log(b).ceil()).max(min * b));
            a.min = min;
            a.max = max;
            a.major = b;
            return;
        }
        if a.percent {
            if fmin.is_none() {
                fmin = Some(if lo < -1e-9 { -1.0 } else { 0.0 });
            }
            if fmax.is_none() {
                fmax = Some(if hi > 1e-9 || lo >= -1e-9 { 1.0 } else { 0.0 });
            }
        }
        let size = resolve_para(&a.lists, 0, None).def_run.size.unwrap_or(10.0);
        let max_int = if a.horizontal {
            // Wide labels need more room along a horizontal axis.
            let probe = [lo, hi, (lo + hi) / 2.0]
                .iter()
                .map(|v| format_value(*v, &a.fmt, self.date1904).chars().count())
                .max()
                .unwrap_or(3) as f64;
            let est = (probe * size * 0.55 + size * 1.5).max(size * 3.0);
            (len / est.max(24.0)).floor().clamp(1.0, 10.0)
        } else {
            (len / (size * 2.0).max(16.0)).floor().clamp(1.0, 10.0)
        };
        let s = auto_scale(lo, hi, fmin, fmax, fmajor, max_int);
        a.min = s.min;
        a.max = s.max;
        a.major = s.major;
    }

    /// Lays out axis labels and returns the inner plot rectangle.
    fn layout_axes(
        &mut self,
        axes: &mut [Axis],
        groups: &[Grp],
        outer: Rect,
        manual_inner: Option<Rect>,
    ) -> Rect {
        let mut inner = manual_inner.unwrap_or(outer);
        for pass in 0..3 {
            for a in axes.iter_mut() {
                if a.kind == AxKind::Val {
                    let len = a.len(&inner);
                    self.scale_axis(a, len);
                }
                self.make_axis_labels(a, &inner);
            }
            let (mut l, mut t, mut r, mut bm) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
            for a in axes.iter() {
                let used = groups.iter().any(|g| {
                    std::ptr::eq(&raw const axes[g.cat], a)
                        || std::ptr::eq(&raw const axes[g.val], a)
                });
                if !used {
                    continue;
                }
                let mut band = if a.visible_labels() { a.band } else { 0.0 };
                if let Some(tl) = &a.title {
                    let (ew, eh) = tl.ext();
                    band += if a.horizontal { eh } else { ew } + tl.size * 0.4;
                }
                match a.pos {
                    'l' => l = l.max(band),
                    'r' => r = r.max(band),
                    't' => t = t.max(band),
                    _ => bm = bm.max(band),
                }
                // Labels at the ends of a horizontal axis overhang the plot.
                if a.visible_labels()
                    && a.horizontal
                    && (a.kind == AxKind::Val || !a.between)
                    && let (Some(first), Some(last)) = (a.labels.first(), a.labels.last())
                {
                    let (f, la) = if a.reversed {
                        (last, first)
                    } else {
                        (first, last)
                    };
                    if f.0 <= 0.01 {
                        l = l.max(f.1.ext().0 / 2.0 - 2.0);
                    }
                    if la.0 >= 0.99 {
                        r = r.max(la.1.ext().0 / 2.0 - 2.0);
                    }
                }
                if a.visible_labels()
                    && !a.horizontal
                    && a.kind == AxKind::Val
                    && let Some(last) = a.labels.last()
                {
                    t = t.max(last.1.ext().1 / 2.0);
                }
            }
            let next = match manual_inner {
                // Office keeps axis labels inside the chart area: a manual
                // plot rectangle shrinks on any side whose labels would spill.
                Some(m) => {
                    let x0 = m.x.max(l + 2.0);
                    let y0 = m.y.max(t + 2.0);
                    let x1 = (m.x + m.w).min(self.w - r - 2.0);
                    let y1 = (m.y + m.h).min(self.h - bm - 2.0);
                    if x1 - x0 > m.w * 0.2 && y1 - y0 > m.h * 0.2 {
                        Rect {
                            x: x0,
                            y: y0,
                            w: x1 - x0,
                            h: y1 - y0,
                        }
                    } else {
                        m
                    }
                }
                None => Rect {
                    x: outer.x + l,
                    y: outer.y + t,
                    w: (outer.w - l - r).max(outer.w * 0.2).max(1.0),
                    h: (outer.h - t - bm).max(outer.h * 0.2).max(1.0),
                },
            };
            let stable =
                (next.w - inner.w).abs() < 0.5 && (next.h - inner.h).abs() < 0.5 && pass > 0;
            inner = next;
            if stable {
                break;
            }
        }
        // Final scale for the settled size.
        for a in axes.iter_mut() {
            if a.kind == AxKind::Val {
                let len = a.len(&inner);
                self.scale_axis(a, len);
            }
            self.make_axis_labels(a, &inner);
        }
        // Axis line positions.
        for i in 0..axes.len() {
            let cross = axes
                .iter()
                .position(|o| o.id == axes[i].cross && o.horizontal != axes[i].horizontal);
            let crosses = axes[i]
                .node
                .as_ref()
                .and_then(|n| val(n, "c:crosses").map(str::to_string));
            let crosses_at = axes[i].node.as_ref().and_then(|n| val_f(n, "c:crossesAt"));
            let line = match cross {
                Some(c) => {
                    let o = &axes[c];
                    let t = if o.kind == AxKind::Val {
                        let v = match (crosses_at, crosses.as_deref()) {
                            (Some(v), _) => v.clamp(o.min.min(o.max), o.max.max(o.min)),
                            (None, Some("min")) => o.min,
                            (None, Some("max")) => o.max,
                            _ => {
                                if o.min <= 0.0 && o.max >= 0.0 && o.log.is_none() {
                                    0.0
                                } else if o.min > 0.0 {
                                    o.min
                                } else {
                                    o.max
                                }
                            }
                        };
                        o.frac(v)
                    } else {
                        match (crosses_at, crosses.as_deref()) {
                            (_, Some("max")) => 1.0,
                            (Some(v), _) => o.cat_frac(v - 1.0).clamp(0.0, 1.0),
                            _ => 0.0,
                        }
                    };
                    o.px(t, &inner)
                }
                None => match axes[i].pos {
                    'l' => inner.x,
                    'r' => inner.x + inner.w,
                    't' => inner.y,
                    _ => inner.y + inner.h,
                },
            };
            axes[i].line = line;
        }
        inner
    }

    fn make_axis_labels(&mut self, a: &mut Axis, inner: &Rect) {
        a.labels.clear();
        a.band = 0.0;
        if !a.visible_labels() {
            return;
        }
        let lists = a.lists.clone();
        let size = resolve_para(&lists, 0, None).def_run.size.unwrap_or(10.0);
        let gap = size * 0.4
            + if matches!(a.major_tick.as_str(), "out" | "cross") {
                TICK * 0.5
            } else {
                0.0
            };
        let len = a.len(inner);
        if a.kind == AxKind::Val {
            let rot = a.rot.unwrap_or(0.0);
            for v in a.ticks() {
                let text = format_value(v, &a.fmt, self.date1904);
                if let Some(lab) = self.label(&text, &lists, rot, None) {
                    a.labels.push((a.frac(v), lab));
                }
            }
        } else {
            if a.unwrapped.is_empty() && a.n > 0 {
                let rot = a.rot.unwrap_or(0.0);
                for i in 0..a.n.min(MAX_MEASURED_CATS) {
                    let text = a.cats.get(i).cloned().unwrap_or_default();
                    let lab = self.label(&text, &lists, rot, None).unwrap_or(Label {
                        tx: plain_body(""),
                        lists: lists.clone(),
                        w: 0.0,
                        h: 0.0,
                        box_w: 0.0,
                        wrap: false,
                        rot,
                        size,
                    });
                    a.unwrapped.push(lab);
                }
            }
            let n = a.n.max(1);
            let slot = if a.between || n == 1 {
                len / n as f64
            } else {
                len / (n - 1) as f64
            };
            let mut labs: Vec<Label> = a.unwrapped.clone();
            let max_w = labs.iter().map(|l| l.w).fold(0.0, f64::max);
            let max_h = labs.iter().map(|l| l.h).fold(0.0, f64::max).max(size * 1.2);
            let mut skip = 1usize;
            if a.horizontal {
                if a.rot.is_none() && max_w > slot * 0.98 {
                    let longest_word = labs
                        .iter()
                        .map(|l| {
                            // Width of the longest word, by its share of the characters.
                            let t = l.tx.all_text();
                            let total = t.chars().count().max(1) as f64;
                            let longest = t
                                .split_whitespace()
                                .map(|w| w.chars().count())
                                .max()
                                .unwrap_or(0) as f64;
                            l.w * longest / total
                        })
                        .fold(0.0, f64::max);
                    let wrap_w = slot * 0.98;
                    let mut wrapped_ok = longest_word <= slot * 1.1 && slot > size * 2.0;
                    if wrapped_ok {
                        let mut out = Vec::with_capacity(labs.len());
                        for l in &labs {
                            if l.w > wrap_w && l.h > 0.0 {
                                out.push(self.relabel(l, Some(wrap_w), 0.0));
                            } else {
                                out.push(l.clone());
                            }
                        }
                        if out
                            .iter()
                            .any(|l| l.w > slot * 1.1 || l.h > size * 1.25 * 4.0)
                        {
                            wrapped_ok = false;
                        } else {
                            labs = out;
                        }
                    }
                    if !wrapped_ok {
                        for l in labs.iter_mut() {
                            l.rot = -45.0;
                        }
                        let pitch = max_h * std::f64::consts::SQRT_2 * 1.05;
                        skip = (pitch / slot).ceil().max(1.0) as usize;
                    }
                } else if let Some(r) = a.rot {
                    let pitch = if r.abs() > 1.0 {
                        max_h / r.to_radians().sin().abs().max(0.2)
                    } else {
                        max_w * 1.05
                    };
                    skip = (pitch / slot).ceil().max(1.0) as usize;
                }
            } else {
                // Vertical category axis (horizontal bars): wrap long labels.
                let max_allowed = (self.w * 0.33).max(size * 6.0);
                if max_w > max_allowed {
                    for l in labs.iter_mut() {
                        if l.w > max_allowed && l.h > 0.0 {
                            *l = self.relabel(l, Some(max_allowed), l.rot);
                        }
                    }
                }
                let max_h2 = labs.iter().map(|l| l.ext().1).fold(0.0, f64::max).max(size);
                skip = (max_h2 * 0.98 / slot).ceil().max(1.0) as usize;
            }
            if a.n > MAX_MEASURED_CATS {
                skip = skip.max((a.n as f64 / MAX_MEASURED_CATS as f64).ceil() as usize);
            }
            if let Some(s) = a.skip {
                skip = s;
            }
            for (i, l) in labs.into_iter().enumerate() {
                if i % skip != 0 || l.h <= 0.0 {
                    continue;
                }
                a.labels.push((a.cat_frac(i as f64), l));
            }
            if a.n > MAX_MEASURED_CATS {
                // Unmeasured categories beyond the cap reuse the last label's metrics.
                if let Some(tmpl) = a.unwrapped.last().cloned() {
                    let mut i = (MAX_MEASURED_CATS / skip + 1) * skip;
                    while i < a.n && a.labels.len() < 400 {
                        let text = a.cats.get(i).cloned().unwrap_or_default();
                        if !text.trim().is_empty() {
                            let lab = Label {
                                tx: plain_body(&text),
                                ..tmpl.clone()
                            };
                            a.labels.push((a.cat_frac(i as f64), lab));
                        }
                        i += skip;
                    }
                }
            }
        }
        let thick = a
            .labels
            .iter()
            .map(|(_, l)| {
                let (ew, eh) = l.ext();
                if a.horizontal { eh } else { ew }
            })
            .fold(0.0, f64::max);
        a.band = if a.labels.is_empty() {
            0.0
        } else {
            thick + gap + size * 0.15
        };
    }

    // ----- drawing: axes and grid ------------------------------------------

    fn draw_gridlines(&mut self, axes: &[Axis], p: &Rect) {
        for a in axes {
            let Some(n) = &a.node else { continue };
            for (name, major) in [("c:minorGridlines", false), ("c:majorGridlines", true)] {
                let Some(g) = n.child(name) else { continue };
                let sp = g.child("c:spPr").cloned();
                let stroke = self.stroke_or(sp.as_deref(), Some((self.line_default(true), 0.75)));
                if stroke.is_none() {
                    continue;
                }
                let fracs: Vec<f64> = if a.kind == AxKind::Val {
                    if major {
                        a.ticks().iter().map(|v| a.frac(*v)).collect()
                    } else {
                        let minor = val_f(n, "c:minorUnit")
                            .filter(|u| *u > 0.0)
                            .unwrap_or(a.major / 5.0);
                        let cnt = ((a.max - a.min) / minor).floor();
                        if !(cnt.is_finite() && cnt > 0.0 && cnt <= 1000.0) || a.log.is_some() {
                            Vec::new()
                        } else {
                            (0..=cnt as usize)
                                .map(|k| a.frac(a.min + k as f64 * minor))
                                .collect()
                        }
                    }
                } else if !major {
                    Vec::new()
                } else {
                    let n_cats = a.n.min(MAX_POINTS);
                    if a.between {
                        (0..=n_cats)
                            .map(|i| i as f64 / n_cats.max(1) as f64)
                            .collect()
                    } else {
                        (0..n_cats).map(|i| a.cat_frac(i as f64)).collect()
                    }
                };
                for t in fracs {
                    if !(-1e-6..=1.0 + 1e-6).contains(&t) {
                        continue;
                    }
                    let c = a.px(t, p);
                    let pts = if a.horizontal {
                        [(c, p.y), (c, p.y + p.h)]
                    } else {
                        [(p.x, c), (p.x + p.w, c)]
                    };
                    self.push_line(&pts, stroke.clone());
                }
            }
        }
    }

    fn draw_axes(&mut self, axes: &[Axis], p: &Rect) {
        for a in axes {
            if a.deleted {
                continue;
            }
            let sp = a.node.as_ref().and_then(|n| n.child("c:spPr").cloned());
            let stroke = self.stroke_or(sp.as_deref(), Some((self.line_default(false), 0.75)));
            let side = if a.horizontal {
                if a.pos == 't' { -1.0 } else { 1.0 }
            } else if a.pos == 'r' {
                1.0
            } else {
                -1.0
            };
            if stroke.is_some() {
                let pts = if a.horizontal {
                    [(p.x, a.line), (p.x + p.w, a.line)]
                } else {
                    [(a.line, p.y), (a.line, p.y + p.h)]
                };
                self.push_line(&pts, stroke.clone());
                // Major tick marks.
                let (o0, o1) = match a.major_tick.as_str() {
                    "out" => (0.0, TICK),
                    "in" => (-TICK, 0.0),
                    "cross" => (-TICK, TICK),
                    _ => (0.0, 0.0),
                };
                if o1 - o0 > 0.0 {
                    let fracs: Vec<f64> = if a.kind == AxKind::Val {
                        a.ticks().iter().map(|v| a.frac(*v)).collect()
                    } else if a.between {
                        let n = a.n.clamp(1, MAX_POINTS);
                        let step = a.skip.unwrap_or(1).max(1);
                        (0..=n).step_by(step).map(|i| i as f64 / n as f64).collect()
                    } else {
                        (0..a.n.min(MAX_POINTS))
                            .map(|i| a.cat_frac(i as f64))
                            .collect()
                    };
                    let mut pb = PathBuilder::new();
                    for t in fracs {
                        if !(-1e-6..=1.0 + 1e-6).contains(&t) {
                            continue;
                        }
                        let c = a.px(t, p);
                        if a.horizontal {
                            pb.move_to(c as f32, (a.line + side * o0) as f32);
                            pb.line_to(c as f32, (a.line + side * o1) as f32);
                        } else {
                            pb.move_to((a.line + side * o0) as f32, c as f32);
                            pb.line_to((a.line + side * o1) as f32, c as f32);
                        }
                    }
                    if let Some(path) = pb.finish() {
                        self.push(path, None, stroke.clone(), false);
                    }
                }
            }
        }
        // Labels and titles after all lines.
        for a in axes {
            if a.deleted {
                continue;
            }
            let side = if a.horizontal {
                if a.pos == 't' { -1.0 } else { 1.0 }
            } else if a.pos == 'r' {
                1.0
            } else {
                -1.0
            };
            let size = resolve_para(&a.lists, 0, None).def_run.size.unwrap_or(10.0);
            let gap = size * 0.4
                + if matches!(a.major_tick.as_str(), "out" | "cross") {
                    TICK * 0.5
                } else {
                    0.0
                };
            let anchor = match a.lbl_pos.as_str() {
                "low" => {
                    if a.horizontal {
                        p.y + p.h
                    } else {
                        p.x
                    }
                }
                "high" => {
                    if a.horizontal {
                        p.y
                    } else {
                        p.x + p.w
                    }
                }
                _ => a.line,
            };
            for (t, lab) in &a.labels {
                if !(-1e-6..=1.0 + 1e-6).contains(t) {
                    continue;
                }
                let c = a.px(*t, p);
                let (ew, eh) = lab.ext();
                let (cx, cy) = if a.horizontal {
                    if lab.rot.abs() > 1.0 && lab.rot.abs() < 80.0 {
                        // Slanted labels end at the tick.
                        let r = lab.rot.to_radians();
                        let corner = if (lab.rot < 0.0) == (side > 0.0) {
                            (lab.w / 2.0, -side * lab.h / 2.0)
                        } else {
                            (-lab.w / 2.0, -side * lab.h / 2.0)
                        };
                        let rx = corner.0 * r.cos() - corner.1 * r.sin();
                        let ry = corner.0 * r.sin() + corner.1 * r.cos();
                        (c - rx, anchor + side * gap - ry)
                    } else {
                        (c, anchor + side * (gap + eh / 2.0))
                    }
                } else {
                    (anchor + side * (gap + ew / 2.0), c)
                };
                self.draw_label(lab, cx, cy);
            }
            if let Some(tl) = &a.title {
                let (ew, eh) = tl.ext();
                let band = if a.visible_labels() { a.band } else { 0.0 };
                let base = if a.lbl_pos == "nextTo" || !a.visible_labels() {
                    if a.horizontal {
                        if side > 0.0 {
                            a.line.max(p.y + p.h)
                        } else {
                            a.line.min(p.y)
                        }
                    } else if side > 0.0 {
                        a.line.max(p.x + p.w)
                    } else {
                        a.line.min(p.x)
                    }
                } else {
                    anchor
                };
                let (cx, cy) = if a.horizontal {
                    (
                        p.x + p.w / 2.0,
                        base + side * (band + tl.size * 0.2 + eh / 2.0),
                    )
                } else {
                    (
                        base + side * (band + tl.size * 0.2 + ew / 2.0),
                        p.y + p.h / 2.0,
                    )
                };
                self.draw_label(tl, cx, cy);
            }
        }
    }

    // ----- drawing: series -------------------------------------------------

    /// Fill of point `i` of series `s` (point override, series, automatic).
    fn point_fill(
        &self,
        g: &Grp,
        s: &SerInfo,
        i: usize,
        n_points: usize,
        bounds: Rect,
    ) -> Option<Paint> {
        let auto = if g.vary
            && (g.sers.len() == 1 || matches!(g.kind, Kind::Pie | Kind::Pie3D | Kind::Doughnut))
        {
            self.auto_color(i, n_points)
        } else {
            self.auto_color(s.s.index, self.n_series)
        };
        let dpt =
            s.s.point_sp_pr
                .iter()
                .find(|(k, _)| *k == i)
                .map(|(_, n)| n.clone());
        if Self::has_fill(dpt.as_deref()) {
            return self.fill_or(dpt.as_deref(), bounds, Some(Paint::Solid(auto)));
        }
        let sp = s.s.sp_pr.clone();
        if g.vary
            && (g.sers.len() == 1 || matches!(g.kind, Kind::Pie | Kind::Pie3D | Kind::Doughnut))
            && !matches!(g.kind, Kind::Line | Kind::Scatter)
        {
            // varyColors applies when the series has no explicit fill.
            if !Self::has_fill(sp.as_deref()) {
                return Some(Paint::Solid(auto));
            }
        }
        self.fill_or(sp.as_deref(), bounds, Some(Paint::Solid(auto)))
    }

    fn point_stroke(&self, s: &SerInfo, i: usize, default: Option<(Rgba, f64)>) -> Option<Stroke> {
        let dpt =
            s.s.point_sp_pr
                .iter()
                .find(|(k, _)| *k == i)
                .map(|(_, n)| n.clone());
        if dpt.as_ref().and_then(|d| d.child("a:ln")).is_some() {
            return self.stroke_or(dpt.as_deref(), default);
        }
        self.stroke_or(s.s.sp_pr.as_deref(), default)
    }

    fn series_color(&self, s: &SerInfo, line: bool) -> Rgba {
        let auto = self.auto_color(s.s.index, self.n_series);
        let sp = s.s.sp_pr.as_deref();
        let paint = if line {
            sp.and_then(|x| x.child("a:ln"))
                .and_then(|ln| style::fill_child(ln))
                .and_then(|f| {
                    match style::read_fill(
                        f,
                        &self.ctx(),
                        None,
                        Rect {
                            x: 0.0,
                            y: 0.0,
                            w: 1.0,
                            h: 1.0,
                        },
                    ) {
                        Fill::Paint(p) => p.representative(),
                        _ => None,
                    }
                })
        } else {
            self.fill_or(
                sp,
                Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 1.0,
                    h: 1.0,
                },
                None,
            )
            .and_then(|p| p.representative())
        };
        paint.unwrap_or(auto)
    }

    /// Base value where bars start (the category axis crossing).
    fn bar_base(cat: &Axis, v: &Axis) -> f64 {
        let crosses = cat
            .node
            .as_ref()
            .and_then(|n| val(n, "c:crosses").map(str::to_string));
        let at = cat.node.as_ref().and_then(|n| val_f(n, "c:crossesAt"));
        let lo = v.min.min(v.max);
        let hi = v.max.max(v.min);
        match (at, crosses.as_deref()) {
            (Some(x), _) => x.clamp(lo, hi),
            (None, Some("max")) => v.max,
            (None, Some("min")) => v.min,
            _ => {
                if v.log.is_some() {
                    v.min
                } else {
                    0.0f64.clamp(lo, hi)
                }
            }
        }
    }

    fn draw_bars(&mut self, g: &Grp, axes: &[Axis], p: &Rect, out: &mut Vec<DLabel>) {
        let (cat, v) = (&axes[g.cat], &axes[g.val]);
        let n = g.n_points().min(cat.n.max(1));
        if n == 0 || g.sers.is_empty() {
            return;
        }
        let gap = val_f(&g.node, "c:gapWidth").unwrap_or(150.0);
        let overlap = val_f(&g.node, "c:overlap").unwrap_or(if g.stacked() { 100.0 } else { 0.0 });
        let k = if g.stacked() { 1 } else { g.sers.len() };
        let base = Self::bar_base(cat, v);
        let stack = g.stack();
        let slot_t = if cat.between {
            1.0 / cat.n.max(1) as f64
        } else {
            1.0 / (cat.n.max(2) - 1) as f64
        };
        for (j, s) in g.sers.iter().enumerate() {
            let (off, bw) = cluster_geometry(k, gap, overlap, if g.stacked() { 0 } else { j });
            let invert = flag(&s.node, "c:invertIfNegative").unwrap_or(false);
            for i in 0..n {
                let Some((b0, b1)) = stack[j].get(i).copied().flatten() else {
                    continue;
                };
                let (v0, v1) = if g.stacked() { (b0, b1) } else { (base, b1) };
                let start = if cat.between {
                    i as f64 * slot_t
                } else {
                    cat.cat_frac(i as f64) - slot_t / 2.0
                };
                let t0 = start + off * slot_t;
                let t1 = t0 + bw * slot_t;
                let c0 = cat.px(t0, p);
                let c1 = cat.px(t1, p);
                let f0 = v.frac(v0).clamp(0.0, 1.0);
                let f1 = v.frac(v1).clamp(0.0, 1.0);
                let p0 = v.px(f0, p);
                let p1 = v.px(f1, p);
                let r = if cat.horizontal {
                    rect_xyxy(c0, p0, c1, p1)
                } else {
                    rect_xyxy(p0, c0, p1, c1)
                };
                let dir = if cat.horizontal {
                    (0.0, (p1 - p0).signum())
                } else {
                    ((p1 - p0).signum(), 0.0)
                };
                let dir = if dir == (0.0, 0.0) {
                    let up = if v1 >= v0 { 1.0 } else { -1.0 };
                    let t_dir = v.px(1.0, p) - v.px(0.0, p);
                    if cat.horizontal {
                        (0.0, t_dir.signum() * up)
                    } else {
                        (t_dir.signum() * up, 0.0)
                    }
                } else {
                    dir
                };
                let mut fill = self.point_fill(g, s, i, n, r);
                if invert && b1 < 0.0 && matches!(fill, Some(Paint::Solid(_))) {
                    fill = Some(Paint::Solid(self.bg1()));
                }
                let stroke = self.point_stroke(s, i, None);
                if r.w > 0.0 || r.h > 0.0 {
                    let rr = Rect {
                        x: r.x,
                        y: r.y,
                        w: r.w.max(0.01),
                        h: r.h.max(0.01),
                    };
                    self.push_rect(rr, fill, stroke);
                }
                let default_pos = if g.stacked() { "ctr" } else { "outEnd" };
                self.collect_dlabel(
                    g,
                    s,
                    i,
                    n,
                    b1 - b0,
                    |pos| Anchor::Bar {
                        r,
                        dir,
                        pos: pos.unwrap_or(default_pos).to_string(),
                    },
                    out,
                );
            }
        }
    }

    fn marker_for(&self, g: &Grp, s: &SerInfo, color: Rgba) -> Option<MarkerSpec> {
        let m = s.node.child("c:marker");
        let symbol = m.and_then(|m| val(m, "c:symbol")).map(str::to_string);
        let show_default = match g.kind {
            Kind::Line => flag(&g.node, "c:marker").unwrap_or(false),
            Kind::Radar { filled: false } => val(&g.node, "c:radarStyle") == Some("marker"),
            Kind::Scatter => matches!(
                val(&g.node, "c:scatterStyle"),
                Some("lineMarker") | Some("marker") | Some("smoothMarker") | None
            ),
            _ => false,
        };
        let symbol = match symbol.as_deref() {
            Some("none") => return None,
            Some("auto") | None => {
                if symbol.is_none() && !show_default {
                    return None;
                }
                if self.modern {
                    "circle".to_string()
                } else {
                    const AUTO: [&str; 9] = [
                        "diamond", "square", "triangle", "x", "star", "circle", "plus", "dot",
                        "dash",
                    ];
                    AUTO[s.s.index % AUTO.len()].to_string()
                }
            }
            Some(other) => other.to_string(),
        };
        let size = m
            .and_then(|m| val_f(m, "c:size"))
            .unwrap_or(if self.modern { 5.0 } else { 7.0 })
            .clamp(2.0, 72.0);
        let sp = m.and_then(|m| m.child("c:spPr").cloned());
        let r = Rect {
            x: 0.0,
            y: 0.0,
            w: size,
            h: size,
        };
        let fill = self.fill_or(sp.as_deref(), r, Some(Paint::Solid(color)));
        let stroke = self.stroke_or(sp.as_deref(), Some((color, 0.75)));
        Some(MarkerSpec {
            symbol,
            size,
            fill,
            stroke,
        })
    }

    fn draw_marker(&mut self, m: &MarkerSpec, x: f64, y: f64) {
        let s = m.size;
        let h = s / 2.0;
        let mut pb = PathBuilder::new();
        let (x, y) = (x as f32, y as f32);
        let hf = h as f32;
        let mut filled = true;
        match m.symbol.as_str() {
            "circle" => {
                if let Some(r) = tiny_skia::Rect::from_xywh(x - hf, y - hf, s as f32, s as f32) {
                    pb.push_oval(r);
                }
            }
            "diamond" => {
                pb.move_to(x, y - hf);
                pb.line_to(x + hf, y);
                pb.line_to(x, y + hf);
                pb.line_to(x - hf, y);
                pb.close();
            }
            "triangle" => {
                pb.move_to(x, y - hf);
                pb.line_to(x + hf, y + hf);
                pb.line_to(x - hf, y + hf);
                pb.close();
            }
            "x" | "star" | "plus" => {
                filled = false;
                if m.symbol != "plus" {
                    pb.move_to(x - hf, y - hf);
                    pb.line_to(x + hf, y + hf);
                    pb.move_to(x + hf, y - hf);
                    pb.line_to(x - hf, y + hf);
                }
                if m.symbol != "x" {
                    pb.move_to(x, y - hf);
                    pb.line_to(x, y + hf);
                }
                if m.symbol == "plus" {
                    pb.move_to(x - hf, y);
                    pb.line_to(x + hf, y);
                }
            }
            "dash" => {
                if let Some(r) =
                    tiny_skia::Rect::from_xywh(x - hf, y - hf / 4.0, s as f32, (s / 4.0) as f32)
                {
                    pb.push_rect(r);
                }
            }
            "dot" => {
                if let Some(r) = tiny_skia::Rect::from_xywh(x - hf / 2.0, y - hf / 2.0, hf, hf) {
                    pb.push_rect(r);
                }
            }
            _ => {
                if let Some(r) = tiny_skia::Rect::from_xywh(x - hf, y - hf, s as f32, s as f32) {
                    pb.push_rect(r);
                }
            }
        }
        if let Some(path) = pb.finish() {
            let fill = if filled { m.fill.clone() } else { None };
            let stroke = if filled {
                m.stroke.clone()
            } else {
                m.stroke.clone().or_else(|| {
                    m.fill.clone().map(|f| Stroke {
                        paint: f,
                        width: 0.75,
                        dash: vec![],
                        cap: LineCap::Flat,
                        join: LineJoin::Round,
                        head: None,
                        tail: None,
                    })
                })
            };
            self.push(path, fill, stroke, m.symbol == "circle");
        }
    }

    fn line_stroke(&self, s: &SerInfo, color: Rgba, scatter_default: bool) -> Option<Stroke> {
        let sp = s.s.sp_pr.as_deref();
        let width = 2.25;
        let ln = sp.and_then(|x| x.child("a:ln"));
        if ln.is_none() && !scatter_default {
            return None;
        }
        let mut st = self.stroke_or(sp, Some((color, width)))?;
        if ln.and_then(|l| l.attr("cap")).is_none() {
            st.cap = LineCap::Round;
        }
        st.join = LineJoin::Round;
        Some(st)
    }

    fn draw_polyline(&mut self, pts: &[(f64, f64)], smooth: bool, stroke: Option<Stroke>) {
        if pts.len() < 2 || stroke.is_none() {
            return;
        }
        let mut pb = PathBuilder::new();
        pb.move_to(pts[0].0 as f32, pts[0].1 as f32);
        if smooth && pts.len() > 2 {
            for i in 0..pts.len() - 1 {
                let p0 = if i > 0 { pts[i - 1] } else { pts[i] };
                let p1 = pts[i];
                let p2 = pts[i + 1];
                let p3 = if i + 2 < pts.len() { pts[i + 2] } else { p2 };
                let c1 = (p1.0 + (p2.0 - p0.0) / 6.0, p1.1 + (p2.1 - p0.1) / 6.0);
                let c2 = (p2.0 - (p3.0 - p1.0) / 6.0, p2.1 - (p3.1 - p1.1) / 6.0);
                pb.cubic_to(
                    c1.0 as f32,
                    c1.1 as f32,
                    c2.0 as f32,
                    c2.1 as f32,
                    p2.0 as f32,
                    p2.1 as f32,
                );
            }
        } else {
            for q in &pts[1..] {
                pb.line_to(q.0 as f32, q.1 as f32);
            }
        }
        if let Some(path) = pb.finish() {
            self.push(path, None, stroke, smooth);
        }
    }

    fn draw_lines(&mut self, g: &Grp, axes: &[Axis], p: &Rect, out: &mut Vec<DLabel>) {
        let (cat, v) = (&axes[g.cat], &axes[g.val]);
        let n = g.n_points().min(cat.n.max(1));
        let stack = g.stack();
        let blanks = val(&self.chart, "c:dispBlanksAs")
            .unwrap_or("gap")
            .to_string();
        let clip = Rect {
            x: p.x - 2.0,
            y: p.y - 2.0,
            w: p.w + 4.0,
            h: p.h + 4.0,
        };
        for (j, s) in g.sers.iter().enumerate() {
            let color = self.series_color(s, true);
            let stroke = self.line_stroke(s, color, true);
            let smooth = flag(&s.node, "c:smooth").unwrap_or(false);
            let mut segs: Vec<Vec<(f64, f64)>> = vec![Vec::new()];
            let mut pts_all = Vec::new();
            for i in 0..n {
                let val_i = stack[j].get(i).copied().flatten().map(|(_, t)| t);
                let val_i = match (val_i, blanks.as_str()) {
                    (Some(x), _) => Some(x),
                    (None, "zero") => Some(0.0),
                    _ => None,
                };
                let Some(y) = val_i else {
                    if blanks != "span" && !segs.last().is_some_and(|s| s.is_empty()) {
                        segs.push(Vec::new());
                    }
                    continue;
                };
                let c = cat.px(cat.cat_frac(i as f64), p);
                let vv = v.px(v.frac(y), p);
                let pt = if cat.horizontal { (c, vv) } else { (vv, c) };
                if let Some(last) = segs.last_mut() {
                    last.push(pt);
                }
                pts_all.push((i, pt));
            }
            self.push_clip(clip);
            for seg in &segs {
                self.draw_polyline(seg, smooth, stroke.clone());
            }
            self.b.scene.items.push(Item::PopClip);
            let marker = self.marker_for(g, s, color);
            for &(i, (x, y)) in &pts_all {
                if !(x >= clip.x && x <= clip.x + clip.w && y >= clip.y && y <= clip.y + clip.h) {
                    continue;
                }
                let mk = self.point_marker(s, i, marker.clone(), color);
                if let Some(m) = &mk {
                    self.draw_marker(m, x, y);
                }
                let ms = mk.as_ref().map(|m| m.size).unwrap_or(0.0);
                let raw = s.vals.get(i).copied().flatten().unwrap_or(0.0);
                self.collect_dlabel(
                    g,
                    s,
                    i,
                    n,
                    raw,
                    |pos| Anchor::Point {
                        x,
                        y,
                        pos: pos.unwrap_or("r").to_string(),
                        marker: ms,
                    },
                    out,
                );
            }
        }
    }

    /// Applies a `c:dPt` marker override.
    fn point_marker(
        &self,
        s: &SerInfo,
        i: usize,
        base: Option<MarkerSpec>,
        color: Rgba,
    ) -> Option<MarkerSpec> {
        let dpt = s
            .node
            .children_named("c:dPt")
            .find(|d| val_f(d, "c:idx").map(|v| v as usize) == Some(i))
            .cloned();
        let Some(m) = dpt.as_ref().and_then(|d| d.child("c:marker").cloned()) else {
            return base;
        };
        let symbol = val(&m, "c:symbol").map(str::to_string);
        if symbol.as_deref() == Some("none") {
            return None;
        }
        let mut spec = base.unwrap_or(MarkerSpec {
            symbol: "circle".into(),
            size: 5.0,
            fill: Some(Paint::Solid(color)),
            stroke: None,
        });
        if let Some(sy) = symbol.filter(|s| s != "auto") {
            spec.symbol = sy;
        }
        if let Some(sz) = val_f(&m, "c:size") {
            spec.size = sz.clamp(2.0, 72.0);
        }
        if let Some(sp) = m.child("c:spPr").cloned() {
            let r = Rect {
                x: 0.0,
                y: 0.0,
                w: spec.size,
                h: spec.size,
            };
            spec.fill = self.fill_or(Some(&sp), r, spec.fill.clone());
            spec.stroke = self.stroke_or(
                Some(&sp),
                spec.stroke
                    .as_ref()
                    .and_then(|s| s.paint.representative().map(|c| (c, s.width))),
            );
        }
        Some(spec)
    }

    fn draw_area(&mut self, g: &Grp, axes: &[Axis], p: &Rect, out: &mut Vec<DLabel>) {
        let (cat, v) = (&axes[g.cat], &axes[g.val]);
        let n = g.n_points().min(cat.n.max(1));
        if n == 0 {
            return;
        }
        let stack = g.stack();
        let base_v = Self::bar_base(cat, v);
        self.push_clip(*p);
        let mut labels = Vec::new();
        for (j, s) in g.sers.iter().enumerate() {
            let mut top = Vec::new();
            let mut bottom = Vec::new();
            for i in 0..n {
                let (b0, b1) = stack[j].get(i).copied().flatten().unwrap_or((0.0, 0.0));
                let (b0, b1) = if g.stacked() { (b0, b1) } else { (base_v, b1) };
                let c = cat.px(cat.cat_frac(i as f64), p);
                let y1 = v.px(v.frac(b1), p);
                let y0 = v.px(v.frac(b0), p);
                if cat.horizontal {
                    top.push((c, y1));
                    bottom.push((c, y0));
                } else {
                    top.push((y1, c));
                    bottom.push((y0, c));
                }
                if s.vals.get(i).copied().flatten().is_some() {
                    labels.push((
                        j,
                        i,
                        (
                            (top[i].0 + bottom[i].0) / 2.0,
                            (top[i].1 + bottom[i].1) / 2.0,
                        ),
                    ));
                }
            }
            if top.len() == 1 {
                // A single category spans the plot width.
                let (c0, c1) = (cat.px(0.0, p), cat.px(1.0, p));
                let (t, bt) = (top[0], bottom[0]);
                top = if cat.horizontal {
                    vec![(c0, t.1), (c1, t.1)]
                } else {
                    vec![(t.0, c0), (t.0, c1)]
                };
                bottom = if cat.horizontal {
                    vec![(c0, bt.1), (c1, bt.1)]
                } else {
                    vec![(bt.0, c0), (bt.0, c1)]
                };
            }
            let mut pb = PathBuilder::new();
            pb.move_to(top[0].0 as f32, top[0].1 as f32);
            for q in &top[1..] {
                pb.line_to(q.0 as f32, q.1 as f32);
            }
            for q in bottom.iter().rev() {
                pb.line_to(q.0 as f32, q.1 as f32);
            }
            pb.close();
            let bounds = *p;
            let fill = self.point_fill(g, s, usize::MAX, n, bounds);
            let stroke = self.stroke_or(s.s.sp_pr.as_deref(), None);
            if let Some(path) = pb.finish() {
                self.push(path, fill, stroke, false);
            }
        }
        self.b.scene.items.push(Item::PopClip);
        for (j, i, (x, y)) in labels {
            let s = &g.sers[j];
            let raw = s.vals.get(i).copied().flatten().unwrap_or(0.0);
            self.collect_dlabel(
                g,
                s,
                i,
                n,
                raw,
                |pos| Anchor::Point {
                    x,
                    y,
                    pos: pos.unwrap_or("ctr").to_string(),
                    marker: 0.0,
                },
                out,
            );
        }
    }

    fn draw_scatter(&mut self, g: &Grp, axes: &[Axis], p: &Rect, out: &mut Vec<DLabel>) {
        let (xa, ya) = (&axes[g.cat], &axes[g.val]);
        let style = val(&g.node, "c:scatterStyle")
            .unwrap_or("lineMarker")
            .to_string();
        let clip = Rect {
            x: p.x - 2.0,
            y: p.y - 2.0,
            w: p.w + 4.0,
            h: p.h + 4.0,
        };
        let bubble = g.kind == Kind::Bubble;
        let max_bubble = if bubble {
            g.sers
                .iter()
                .flat_map(|s| cache_numbers(s.node.child("c:bubbleSize")))
                .fold(0.0f64, |a, b| a.max(b.abs()))
        } else {
            0.0
        };
        for s in &g.sers {
            let color = self.series_color(s, !bubble);
            let numeric = s.xs.iter().any(|x| x.is_some());
            let mut pts = Vec::new();
            for i in 0..s.vals.len().min(MAX_POINTS) {
                let Some(y) = s.vals[i] else { continue };
                let x = if numeric {
                    s.xs.get(i).copied().flatten()
                } else {
                    Some(i as f64 + 1.0)
                };
                let Some(x) = x else { continue };
                let (px, py) = (xa.px(xa.frac(x), p), ya.px(ya.frac(y), p));
                let pt = if xa.horizontal { (px, py) } else { (py, px) };
                pts.push((i, pt));
            }
            if bubble {
                let sizes = cache_numbers(s.node.child("c:bubbleSize"));
                let scale = val_f(&g.node, "c:bubbleScale").unwrap_or(100.0) / 100.0;
                let max_r = p.w.min(p.h) * 0.25 / 2.0 * scale;
                for &(i, (x, y)) in &pts {
                    let sz = sizes.get(i).copied().unwrap_or(1.0).abs();
                    let r = if max_bubble > 0.0 {
                        (sz / max_bubble).sqrt() * max_r
                    } else {
                        max_r
                    };
                    let bounds = Rect {
                        x: x - r,
                        y: y - r,
                        w: 2.0 * r,
                        h: 2.0 * r,
                    };
                    let fill = self.point_fill(g, s, i, pts.len(), bounds);
                    let stroke = self.point_stroke(s, i, None);
                    let mut pb = PathBuilder::new();
                    if let Some(rr) = tiny_skia::Rect::from_xywh(
                        bounds.x as f32,
                        bounds.y as f32,
                        bounds.w.max(0.1) as f32,
                        bounds.h.max(0.1) as f32,
                    ) {
                        pb.push_oval(rr);
                    }
                    if let Some(path) = pb.finish() {
                        self.push(path, fill, stroke, true);
                    }
                }
                continue;
            }
            let lines_default = !matches!(style.as_str(), "marker" | "none");
            let stroke = self.line_stroke(s, color, lines_default);
            let smooth = flag(&s.node, "c:smooth").unwrap_or(style.starts_with("smooth"));
            let line_pts: Vec<(f64, f64)> = pts.iter().map(|(_, q)| *q).collect();
            self.push_clip(clip);
            self.draw_polyline(&line_pts, smooth, stroke);
            self.b.scene.items.push(Item::PopClip);
            let marker = self.marker_for(g, s, color);
            for &(i, (x, y)) in &pts {
                if !(x >= clip.x && x <= clip.x + clip.w && y >= clip.y && y <= clip.y + clip.h) {
                    continue;
                }
                let mk = self.point_marker(s, i, marker.clone(), color);
                if let Some(m) = &mk {
                    self.draw_marker(m, x, y);
                }
                let ms = mk.as_ref().map(|m| m.size).unwrap_or(0.0);
                let raw = s.vals.get(i).copied().flatten().unwrap_or(0.0);
                self.collect_dlabel(
                    g,
                    s,
                    i,
                    pts.len(),
                    raw,
                    |pos| Anchor::Point {
                        x,
                        y,
                        pos: pos.unwrap_or("r").to_string(),
                        marker: ms,
                    },
                    out,
                );
            }
        }
    }

    fn draw_pie(&mut self, g: &Grp, rect: Rect, manual: bool, out: &mut Vec<DLabel>) {
        if g.sers.is_empty() {
            return;
        }
        let doughnut = g.kind == Kind::Doughnut;
        let three_d = g.kind == Kind::Pie3D;
        let sers: Vec<&SerInfo> = if doughnut {
            g.sers.iter().collect()
        } else {
            vec![&g.sers[0]]
        };
        let view = self.chart.child("c:view3D").cloned();
        let rot_x = view
            .as_ref()
            .and_then(|v| val_f(v, "c:rotX"))
            .unwrap_or(30.0)
            .clamp(0.0, 90.0);
        let tilt = if three_d {
            rot_x.to_radians().sin().clamp(0.15, 1.0)
        } else {
            1.0
        };
        let depth_pct = view
            .as_ref()
            .and_then(|v| val_f(v, "c:depthPercent"))
            .unwrap_or(100.0)
            .clamp(20.0, 2000.0);
        let first = if three_d {
            view.as_ref()
                .and_then(|v| val_f(v, "c:rotY"))
                .unwrap_or(0.0)
        } else {
            val_f(&g.node, "c:firstSliceAng").unwrap_or(0.0)
        };
        let hole = if doughnut {
            val_f(&g.node, "c:holeSize")
                .unwrap_or(10.0)
                .clamp(1.0, 90.0)
                / 100.0
        } else {
            0.0
        };
        let ser_expl = sers[0]
            .node
            .child("c:explosion")
            .and_then(|e| e.attr_f64("val"))
            .unwrap_or(0.0)
            .max(0.0);
        let point_expl = |s: &SerInfo, i: usize| -> f64 {
            s.node
                .children_named("c:dPt")
                .find(|d| val_f(d, "c:idx").map(|v| v as usize) == Some(i))
                .and_then(|d| val_f(d, "c:explosion"))
                .unwrap_or(ser_expl)
                .clamp(0.0, 400.0)
        };
        let n = sers[0].vals.len().min(MAX_POINTS);
        let max_expl = if doughnut {
            0.0
        } else {
            (0..n).map(|i| point_expl(sers[0], i)).fold(0.0, f64::max)
        };
        // Outside labels need room around the pie in automatic layout.
        let outside = !manual && !doughnut && Self::has_outside_labels(g);
        let label_room = if outside {
            self.base_size.unwrap_or(10.0) * 2.6
        } else {
            0.0
        };
        let avail_w = (rect.w - 2.0 * label_room).max(rect.w * 0.3);
        let avail_h = (rect.h - 2.0 * label_room * 0.6).max(rect.h * 0.3);
        let depth_frac = if three_d {
            0.1 * depth_pct / 100.0 * (1.0 - tilt * 0.5)
        } else {
            0.0
        };
        let mut radius = (avail_w / 2.0).min(avail_h / (2.0 * tilt + 2.0 * depth_frac));
        radius /= 1.0 + max_expl / 100.0;
        let radius = radius.max(1.0);
        let depth = radius * 2.0 * depth_frac;
        let cx = rect.x + rect.w / 2.0;
        let cy = rect.y + rect.h / 2.0 - depth / 2.0;
        let ring = if doughnut {
            radius * (1.0 - hole) / sers.len() as f64
        } else {
            radius
        };
        let mut tops = Vec::new();
        for (si, s) in sers.iter().enumerate() {
            let total: f64 = s.vals.iter().take(n).flatten().map(|v| v.abs()).sum();
            if total <= 0.0 {
                continue;
            }
            let (r_in, r_out) = if doughnut {
                (
                    radius * hole + ring * si as f64,
                    radius * hole + ring * (si + 1) as f64,
                )
            } else {
                (0.0, radius)
            };
            let mut ang = first;
            for i in 0..n {
                let Some(v) = s.vals.get(i).copied().flatten() else {
                    continue;
                };
                let sweep = v.abs() / total * 360.0;
                if sweep <= 0.0 {
                    continue;
                }
                let mid = ang + sweep / 2.0;
                let ex = if doughnut {
                    0.0
                } else {
                    point_expl(s, i) / 100.0 * radius
                };
                let (dx, dy) = (
                    mid.to_radians().sin() * ex,
                    -mid.to_radians().cos() * ex * tilt,
                );
                let bounds = Rect {
                    x: cx - radius,
                    y: cy - radius * tilt,
                    w: 2.0 * radius,
                    h: 2.0 * radius * tilt,
                };
                let fill = self.point_fill(g, s, i, n, bounds);
                let default_ln = if self.modern {
                    Some((self.bg1(), 0.75))
                } else {
                    None
                };
                let stroke = self.point_stroke(s, i, default_ln);
                tops.push((cx + dx, cy + dy, r_in, r_out, ang, sweep, fill, stroke));
                let pos_default = if doughnut { "ctr" } else { "bestFit" };
                let (ccx, ccy) = (cx + dx, cy + dy);
                self.collect_dlabel(
                    g,
                    s,
                    i,
                    n,
                    v.abs() / total,
                    |pos| Anchor::Slice {
                        cx: ccx,
                        cy: ccy,
                        r_in,
                        r_out,
                        ry_scale: tilt,
                        mid,
                        sweep,
                        pos: pos.unwrap_or(pos_default).to_string(),
                    },
                    out,
                );
                ang += sweep;
            }
        }
        if three_d && depth > 0.0 {
            for &(x, y, _, r_out, a0, sw, ref fill, _) in &tops {
                let side = fill.as_ref().map(|f| darken(f, 0.75));
                // Visible side: the part of the arc on the front half (90..270 deg).
                let lo = a0.max(90.0);
                let hi = (a0 + sw).min(270.0);
                let mut segs = vec![(lo, hi)];
                if a0 + sw > 450.0 {
                    segs.push((450.0, (a0 + sw).min(630.0)));
                }
                if a0 < -90.0 {
                    segs.push((a0.max(-270.0), (a0 + sw).min(-90.0)));
                }
                for (s0, s1) in segs {
                    if s1 <= s0 {
                        continue;
                    }
                    let mut pb = PathBuilder::new();
                    let steps = (((s1 - s0) / 4.0).ceil() as usize).clamp(2, 100);
                    for k in 0..=steps {
                        let a = (s0 + (s1 - s0) * k as f64 / steps as f64).to_radians();
                        let (px, py) = (x + r_out * a.sin(), y - r_out * tilt * a.cos());
                        if k == 0 {
                            pb.move_to(px as f32, py as f32);
                        } else {
                            pb.line_to(px as f32, py as f32);
                        }
                    }
                    for k in (0..=steps).rev() {
                        let a = (s0 + (s1 - s0) * k as f64 / steps as f64).to_radians();
                        pb.line_to(
                            (x + r_out * a.sin()) as f32,
                            (y - r_out * tilt * a.cos() + depth) as f32,
                        );
                    }
                    pb.close();
                    if let Some(path) = pb.finish() {
                        self.push(path, side.clone(), None, true);
                    }
                }
            }
        }
        for (x, y, r_in, r_out, a0, sw, fill, stroke) in tops {
            if let Some(path) = slice_path(x, y, r_in, r_out, tilt, a0, sw) {
                self.push(path, fill, stroke, true);
            }
        }
    }

    fn has_outside_labels(g: &Grp) -> bool {
        g.sers.iter().take(1).any(|s| {
            let chain = dl_chain(&g.node, &s.node, None);
            let any = [
                "c:showVal",
                "c:showPercent",
                "c:showCatName",
                "c:showSerName",
            ]
            .iter()
            .any(|k| dl_flag(&chain, k));
            any && matches!(
                dl_get(&chain, "c:dLblPos").and_then(|n| n.attr("val")),
                Some("outEnd")
            )
        })
    }

    // ----- data labels -----------------------------------------------------

    #[expect(
        clippy::too_many_arguments,
        reason = "takes the series point, its value, the anchor rule and the output list"
    )]
    fn collect_dlabel(
        &mut self,
        g: &Grp,
        s: &SerInfo,
        i: usize,
        n: usize,
        value_or_pct: f64,
        anchor: impl Fn(Option<&str>) -> Anchor,
        out: &mut Vec<DLabel>,
    ) {
        let dlbl = s.node.child("c:dLbls").and_then(|d| {
            d.children_named("c:dLbl")
                .find(|l| val_f(l, "c:idx").map(|v| v as usize) == Some(i))
                .cloned()
        });
        let chain = dl_chain(&g.node, &s.node, dlbl.clone());
        if chain.is_empty() {
            return;
        }
        if dlbl
            .as_ref()
            .is_some_and(|d| flag(d, "c:delete") == Some(true))
            || s.node
                .child("c:dLbls")
                .is_some_and(|d| flag(d, "c:delete") == Some(true))
        {
            return;
        }
        let pie = matches!(g.kind, Kind::Pie | Kind::Pie3D | Kind::Doughnut);
        let show_val = dl_flag(&chain, "c:showVal");
        let show_pct = dl_flag(&chain, "c:showPercent") && pie;
        let show_cat = dl_flag(&chain, "c:showCatName");
        let show_ser = dl_flag(&chain, "c:showSerName");
        let show_range = chain.iter().any(|c| {
            c.find("x:showDataLabelsRange")
                .is_some_and(|f| f.attr_bool("val").unwrap_or(true))
        });
        let rich = dlbl
            .as_ref()
            .and_then(|d| d.path(&["c:tx", "c:rich"]).cloned());
        if !(show_val || show_pct || show_cat || show_ser || show_range || rich.is_some()) {
            return;
        }
        let raw = s.vals.get(i).copied().flatten();
        let Some(raw) = raw else { return };
        let fmt_node = dl_get(&chain, "c:numFmt").cloned();
        let fmt = match &fmt_node {
            Some(f) if f.attr_bool("sourceLinked") != Some(true) => {
                f.attr("formatCode").unwrap_or("General").to_string()
            }
            _ => s.fmt.clone(),
        };
        let val_text = format_value(raw, &fmt, self.date1904);
        let pct = if pie { value_or_pct } else { 0.0 };
        let pct_fmt = match &fmt_node {
            Some(f)
                if f.attr_bool("sourceLinked") != Some(true)
                    && f.attr("formatCode").is_some_and(|c| c.contains('%')) =>
            {
                f.attr("formatCode").unwrap_or("0%").to_string()
            }
            _ => "0%".to_string(),
        };
        let pct_text = format_value(pct, &pct_fmt, self.date1904);
        let cat_text = self.cat_text(s, i, None);
        let ser_text = s.s.name.clone().unwrap_or_default();
        let range_text = s.range_texts.get(i).cloned().unwrap_or_default();
        let sep = dl_get(&chain, "c:separator")
            .map(|n| n.text.clone())
            .unwrap_or_else(|| {
                if pie && show_cat && (show_val || show_pct) {
                    "\n".into()
                } else {
                    ", ".into()
                }
            });
        let txpr = dl_get(&chain, "c:txPr").cloned();
        let rot = txpr_rot(txpr.as_ref()).unwrap_or(0.0);
        let mut lists = self.lists(TextKind::Body, &[txpr.as_ref()]);
        let label = if let Some(r) = rich {
            if let Some(ls) = r.child("a:lstStyle") {
                lists.push(ls.clone());
            }
            let fields = |t: &str| -> Option<String> {
                match t {
                    "VALUE" => Some(val_text.clone()),
                    "PERCENTAGE" => Some(pct_text.clone()),
                    "CATEGORYNAME" => Some(cat_text.clone()),
                    "SERIESNAME" => Some(ser_text.clone()),
                    "CELLRANGE" => Some(range_text.clone()),
                    _ => None,
                }
            };
            let tx = substitute_fields(&r, &fields);
            if tx.all_text().trim().is_empty() {
                return;
            }
            self.label_tx(tx, lists, rot, None)
        } else {
            let mut parts = Vec::new();
            if show_range && !range_text.is_empty() {
                parts.push(range_text);
            }
            if show_ser && !ser_text.is_empty() {
                parts.push(ser_text);
            }
            if show_cat && !cat_text.is_empty() {
                parts.push(cat_text);
            }
            if show_val {
                parts.push(val_text);
            }
            if show_pct {
                parts.push(pct_text);
            }
            let text = parts.join(&sep);
            let lab = self.label(&text, &lists, rot, None);
            // Office wraps long pie labels to about a fifth of the chart width.
            let max_w = (self.w * 0.2).max(lab.as_ref().map(|l| l.size * 5.0).unwrap_or(0.0));
            match lab {
                Some(l) if pie && l.w > max_w => Some(self.relabel(&l, Some(max_w), rot)),
                other => other,
            }
        };
        let Some(label) = label else { return };
        let pos = dl_get(&chain, "c:dLblPos")
            .and_then(|p| p.attr("val"))
            .map(str::to_string);
        let pos_ok = pos.as_deref().filter(|p| match g.kind {
            Kind::Bar { .. } => {
                matches!(*p, "ctr" | "inEnd" | "inBase" | "outEnd")
                    && !(g.stacked() && *p == "outEnd")
            }
            Kind::Pie | Kind::Pie3D => matches!(*p, "ctr" | "inEnd" | "outEnd" | "bestFit"),
            Kind::Doughnut => false,
            Kind::Waterfall | Kind::Funnel => matches!(*p, "ctr" | "inEnd" | "inBase" | "outEnd"),
            _ => matches!(*p, "ctr" | "l" | "r" | "t" | "b"),
        });
        let sp = dl_get(&chain, "c:spPr").cloned();
        let bg = (
            self.fill_or(
                sp.as_deref(),
                Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 1.0,
                    h: 1.0,
                },
                None,
            ),
            self.stroke_or(sp.as_deref(), None),
        );
        let mut d = DLabel {
            label,
            bg,
            anchor: anchor(pos_ok),
            offset: (0.0, 0.0),
            manual_abs: None,
        };
        if let Some(m) = manual_layout(dlbl.as_deref()) {
            if m.x_edge || m.y_edge {
                d.manual_abs = Some((m.x.unwrap_or(0.0) * self.w, m.y.unwrap_or(0.0) * self.h));
            } else {
                d.offset = (m.x.unwrap_or(0.0) * self.w, m.y.unwrap_or(0.0) * self.h);
            }
        }
        let _ = n;
        out.push(d);
    }

    fn place_dlabel(&mut self, d: &DLabel) {
        let lab = &d.label;
        let (ew, eh) = lab.ext();
        let gap = lab.size * 0.25 + 1.0;
        let (mut cx, mut cy) = match &d.anchor {
            Anchor::Bar { r, dir, pos } => {
                let (rcx, rcy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
                // End and base points along the value direction.
                let (end, base) = if dir.0 != 0.0 {
                    let e = if dir.0 > 0.0 { r.x + r.w } else { r.x };
                    let b = if dir.0 > 0.0 { r.x } else { r.x + r.w };
                    ((e, rcy), (b, rcy))
                } else {
                    let e = if dir.1 > 0.0 { r.y + r.h } else { r.y };
                    let b = if dir.1 > 0.0 { r.y } else { r.y + r.h };
                    ((rcx, e), (rcx, b))
                };
                let half = if dir.0 != 0.0 { ew / 2.0 } else { eh / 2.0 };
                match pos.as_str() {
                    "ctr" => (rcx, rcy),
                    "inEnd" => (end.0 - dir.0 * (gap + half), end.1 - dir.1 * (gap + half)),
                    "inBase" => (base.0 + dir.0 * (gap + half), base.1 + dir.1 * (gap + half)),
                    _ => (end.0 + dir.0 * (gap + half), end.1 + dir.1 * (gap + half)),
                }
            }
            Anchor::Point { x, y, pos, marker } => {
                let m = marker / 2.0 + gap;
                match pos.as_str() {
                    "ctr" => (*x, *y),
                    "l" => (x - m - ew / 2.0, *y),
                    "t" => (*x, y - m - eh / 2.0),
                    "b" => (*x, y + m + eh / 2.0),
                    _ => (x + m + ew / 2.0, *y),
                }
            }
            Anchor::Slice {
                cx,
                cy,
                r_in,
                r_out,
                ry_scale,
                mid,
                sweep,
                pos,
            } => {
                let a = mid.to_radians();
                let (ux, uy) = (a.sin(), -a.cos());
                let at = |rad: f64| (cx + ux * rad, cy + uy * rad * ry_scale);
                let inside_r = if *r_in > 0.0 {
                    (r_in + r_out) / 2.0
                } else {
                    r_out * 0.62
                };
                let outside = |gap: f64| {
                    let (ex, ey) = at(*r_out);
                    (ex + ux * (gap + ew / 2.0), ey + uy * (gap + eh / 2.0))
                };
                match pos.as_str() {
                    "ctr" => at(inside_r),
                    "inEnd" => at(r_out - gap - ew.max(eh) / 2.0),
                    "outEnd" => outside(gap),
                    _ => {
                        // Best fit: inside when the label fits in the slice.
                        let half_ang = (sweep / 2.0).min(90.0).to_radians();
                        let room_ang = inside_r * half_ang.sin() * 2.0;
                        let diag = (ew * ew + eh * eh).sqrt();
                        let ring = *r_in > 0.0;
                        // A dragged label's offset is relative to the outside spot.
                        let small = ring || diag <= r_out * 0.7;
                        let fits =
                            (small && room_ang >= diag * 0.85 && *sweep < 359.0) || *sweep >= 359.0;
                        if fits && d.offset == (0.0, 0.0) && d.manual_abs.is_none() {
                            at(inside_r)
                        } else {
                            outside(gap)
                        }
                    }
                }
            }
        };
        if let Some((x, y)) = d.manual_abs {
            cx = x + ew / 2.0;
            cy = y + eh / 2.0;
        } else {
            cx += d.offset.0;
            cy += d.offset.1;
        }
        if d.bg.0.is_some() || d.bg.1.is_some() {
            // Office's default label insets: 3pt left/right, 1.5pt top/bottom.
            let r = Rect {
                x: cx - ew / 2.0 - 3.0,
                y: cy - eh / 2.0 - 1.5,
                w: ew + 6.0,
                h: eh + 3.0,
            };
            let fill = d.bg.0.clone().map(|f| match f {
                Paint::Linear { .. }
                | Paint::Radial { .. }
                | Paint::Image { .. }
                | Paint::Path(_) => f.representative().map(Paint::Solid).unwrap_or(f),
                other => other,
            });
            self.push_rect(r, fill, d.bg.1.clone());
        }
        self.draw_label(lab, cx, cy);
    }

    // ----- legend ----------------------------------------------------------

    fn legend_entries(&mut self, ln: &XNode, groups: &[Grp]) -> Vec<LegendEntry> {
        let txpr = ln.child("c:txPr").cloned();
        let lists = self.lists(TextKind::Body, &[txpr.as_ref()]);
        let deleted: Vec<usize> = ln
            .children_named("c:legendEntry")
            .filter(|e| flag(e, "c:delete") == Some(true))
            .filter_map(|e| val_f(e, "c:idx").map(|v| v as usize))
            .collect();
        let mut raw: Vec<(String, Key)> = Vec::new();
        let per_point = groups.len() == 1
            && groups[0].vary
            && (matches!(groups[0].kind, Kind::Pie | Kind::Pie3D | Kind::Doughnut)
                || groups[0].sers.len() == 1);
        if let Some(special) = self.special_legend(groups) {
            raw = special;
        } else if per_point {
            let g = &groups[0];
            if let Some(s) = g.sers.first() {
                let n = s.vals.len().max(s.s.categories.len()).min(200);
                for i in 0..n {
                    let fill = self.point_fill(
                        g,
                        s,
                        i,
                        n,
                        Rect {
                            x: 0.0,
                            y: 0.0,
                            w: 10.0,
                            h: 10.0,
                        },
                    );
                    let default_ln = if self.modern && !matches!(g.kind, Kind::Bar { .. }) {
                        Some((self.bg1(), 0.75))
                    } else {
                        None
                    };
                    let stroke = self.point_stroke(s, i, default_ln);
                    let text = self.cat_text(s, i, None);
                    raw.push((
                        if text.is_empty() {
                            (i + 1).to_string()
                        } else {
                            text
                        },
                        Key::Box(fill, stroke),
                    ));
                }
            }
        } else {
            for g in groups {
                let mut entries: Vec<(String, Key)> = Vec::new();
                for (j, s) in g.sers.iter().enumerate() {
                    let name =
                        s.s.name
                            .clone()
                            .unwrap_or_else(|| format!("Series{}", s.s.index + 1));
                    let key = match g.kind {
                        Kind::Line | Kind::Scatter | Kind::Radar { filled: false } => {
                            let color = self.series_color(s, true);
                            let lines_default = g.kind != Kind::Scatter
                                || !matches!(
                                    val(&g.node, "c:scatterStyle"),
                                    Some("marker") | Some("none")
                                );
                            let st = self.line_stroke(s, color, lines_default);
                            Key::Line(st, self.marker_for(g, s, color))
                        }
                        _ => {
                            let fill = self.point_fill(
                                g,
                                s,
                                usize::MAX,
                                1,
                                Rect {
                                    x: 0.0,
                                    y: 0.0,
                                    w: 10.0,
                                    h: 10.0,
                                },
                            );
                            Key::Box(fill, self.stroke_or(s.s.sp_pr.as_deref(), None))
                        }
                    };
                    let _ = j;
                    entries.push((name, key));
                }
                // Stacked columns/areas list the top of the stack first in side legends.
                let side = matches!(val(ln, "c:legendPos").unwrap_or("r"), "r" | "l" | "tr");
                let hbar = matches!(g.kind, Kind::Bar { horizontal: true });
                if side
                    && ((g.stacked()
                        && matches!(g.kind, Kind::Bar { horizontal: false } | Kind::Area))
                        || (hbar && !g.stacked()))
                {
                    entries.reverse();
                }
                raw.extend(entries);
            }
        }
        let mut out = Vec::new();
        for (k, (text, key)) in raw.into_iter().enumerate() {
            if deleted.contains(&k) {
                continue;
            }
            if let Some(label) = self.label(&text, &lists, 0.0, None) {
                out.push(LegendEntry { label, key });
            }
        }
        out
    }

    fn legend_layout(&self, ln: &XNode, entries: &[LegendEntry], top: f64) -> LegendLayout {
        let size = entries
            .iter()
            .map(|e| e.label.size)
            .fold(0.0, f64::max)
            .max(1.0);
        let row_h = entries
            .iter()
            .map(|e| e.label.h)
            .fold(0.0, f64::max)
            .max(size * 1.2);
        let has_line = entries.iter().any(|e| matches!(e.key, Key::Line(..)));
        let key_w = if has_line { size * 2.0 } else { size * 0.7 };
        let gap = size * 0.35;
        let spacing = size * 0.9;
        let widths: Vec<f64> = entries.iter().map(|e| key_w + gap + e.label.w).collect();
        let pos = val(ln, "c:legendPos").unwrap_or("r");
        let manual = manual_layout(Some(ln));
        let horizontal = match manual {
            Some(Manual {
                w: Some(mw),
                h: Some(mh),
                ..
            }) => mw * self.w > mh * self.h * 1.5,
            _ => matches!(pos, "t" | "b"),
        };
        let avail_w = match manual {
            Some(Manual { w: Some(mw), .. }) => mw * self.w,
            _ => self.w - 2.0 * PAD,
        };
        let avail_h = match manual {
            Some(Manual { h: Some(mh), .. }) => mh * self.h,
            _ => self.h - top - PAD,
        };
        let mut rel = Vec::with_capacity(entries.len());
        let (lw, lh);
        if horizontal {
            let total: f64 =
                widths.iter().sum::<f64>() + spacing * (widths.len().saturating_sub(1)) as f64;
            if total <= avail_w {
                let mut x = 0.0;
                for w in &widths {
                    rel.push((x, 0.0));
                    x += w + spacing;
                }
                lw = total;
                lh = row_h;
            } else {
                let colw = widths.iter().fold(0.0f64, |a, b| a.max(*b)) + spacing;
                let cols = ((avail_w + spacing) / colw).floor().max(1.0) as usize;
                let rows = entries.len().div_ceil(cols);
                for k in 0..entries.len() {
                    rel.push(((k % cols) as f64 * colw, (k / cols) as f64 * row_h));
                }
                lw = (cols.min(entries.len()) as f64 * colw - spacing).max(0.0);
                lh = rows as f64 * row_h;
            }
        } else {
            let rows_fit = ((avail_h / row_h).floor() as usize).max(1);
            let cols = entries.len().div_ceil(rows_fit).max(1);
            let rows = entries.len().div_ceil(cols);
            let colw = widths.iter().fold(0.0f64, |a, b| a.max(*b)) + spacing;
            for k in 0..entries.len() {
                rel.push(((k / rows) as f64 * colw, (k % rows) as f64 * row_h));
            }
            lw = cols as f64 * colw - spacing;
            lh = rows as f64 * row_h;
        }
        let pad = size * 0.3;
        let (bw, bh) = (lw + 2.0 * pad, lh + 2.0 * pad);
        let (x, y) = match manual {
            Some(m) => {
                let mw = m.w.map(|v| v * self.w).unwrap_or(bw);
                let mh = m.h.map(|v| v * self.h).unwrap_or(bh);
                let default = self.legend_default_pos(pos, bw, bh, top);
                let x0 = match m.x {
                    Some(x) if m.x_edge => x * self.w,
                    Some(x) => default.0 + x * self.w,
                    None => default.0,
                };
                let y0 = match m.y {
                    Some(y) if m.y_edge => y * self.h,
                    Some(y) => default.1 + y * self.h,
                    None => default.1,
                };
                // Entries are centered in a manually sized legend box.
                let r = Rect {
                    x: x0,
                    y: y0,
                    w: mw,
                    h: mh,
                };
                let offx = ((mw - bw) / 2.0).max(0.0);
                let offy = ((mh - bh) / 2.0).max(0.0);
                let pos_abs = rel
                    .iter()
                    .map(|(rx, ry)| (x0 + offx + pad + rx, y0 + offy + pad + ry))
                    .collect();
                return LegendLayout {
                    rect: r,
                    pos: pos_abs,
                    key_w,
                    gap,
                    row_h,
                };
            }
            None => self.legend_default_pos(pos, bw, bh, top),
        };
        let pos_abs = rel
            .iter()
            .map(|(rx, ry)| (x + pad + rx, y + pad + ry))
            .collect();
        LegendLayout {
            rect: Rect { x, y, w: bw, h: bh },
            pos: pos_abs,
            key_w,
            gap,
            row_h,
        }
    }

    fn legend_default_pos(&self, pos: &str, bw: f64, bh: f64, top: f64) -> (f64, f64) {
        let (w, h) = (self.w, self.h);
        match pos {
            "b" => ((w - bw) / 2.0, h - PAD - bh),
            "t" => ((w - bw) / 2.0, top),
            "l" => (PAD, top + (h - top - PAD - bh) / 2.0),
            "tr" => (w - PAD - bw, top),
            _ => (w - PAD - bw, top + (h - top - PAD - bh).max(0.0) / 2.0),
        }
    }

    fn draw_legend(&mut self, ln: &XNode, entries: &[LegendEntry], lay: &LegendLayout) {
        let sp = ln.child("c:spPr").cloned();
        let f = self.fill_or(sp.as_deref(), lay.rect, None);
        let s = self.stroke_or(sp.as_deref(), None);
        self.push_rect(lay.rect, f, s);
        for (e, &(x, y)) in entries.iter().zip(lay.pos.iter()) {
            let cy = y + lay.row_h / 2.0;
            let size = e.label.size;
            match &e.key {
                Key::Box(fill, stroke) => {
                    let side = size * 0.6;
                    let kx = x + (lay.key_w - side) / 2.0;
                    let stroke = stroke.clone().map(|mut s| {
                        s.width = s.width.min(1.5);
                        s
                    });
                    let fill = fill.clone().map(|f| match f {
                        Paint::Image { .. }
                        | Paint::Linear { .. }
                        | Paint::Radial { .. }
                        | Paint::Path(_) => f.representative().map(Paint::Solid).unwrap_or(f),
                        other => other,
                    });
                    self.push_rect(
                        Rect {
                            x: kx,
                            y: cy - side / 2.0,
                            w: side,
                            h: side,
                        },
                        fill,
                        stroke,
                    );
                }
                Key::Line(stroke, marker) => {
                    self.push_line(
                        &[(x, cy), (x + lay.key_w, cy)],
                        stroke.clone().map(|mut s| {
                            s.cap = LineCap::Flat;
                            s
                        }),
                    );
                    if let Some(m) = marker {
                        let mut m = m.clone();
                        m.size = m.size.min(size * 0.9);
                        self.draw_marker(&m, x + lay.key_w / 2.0, cy);
                    }
                }
            }
            let tx = x + lay.key_w + lay.gap + e.label.w / 2.0;
            self.draw_label(&e.label, tx, cy);
        }
    }
}

// ---------------------------------------------------------------------------
// Free helpers
// ---------------------------------------------------------------------------

fn synth_axis(kind: AxKind, pos: char) -> Axis {
    Axis {
        id: String::new(),
        node: None,
        kind,
        horizontal: matches!(pos, 't' | 'b'),
        pos,
        deleted: false,
        reversed: false,
        cross: String::new(),
        between: true,
        log: None,
        lo: 0.0,
        hi: 0.0,
        has_data: false,
        percent: false,
        min: 0.0,
        max: 1.0,
        major: 0.2,
        n: 0,
        cats: Vec::new(),
        fmt: "General".into(),
        lbl_pos: "nextTo".into(),
        major_tick: "out".into(),
        minor_tick: "none".into(),
        rot: None,
        skip: None,
        lists: Vec::new(),
        title: None,
        labels: Vec::new(),
        unwrapped: Vec::new(),
        band: 0.0,
        line: 0.0,
    }
}

fn rect_xyxy(x0: f64, y0: f64, x1: f64, y1: f64) -> Rect {
    Rect {
        x: x0.min(x1),
        y: y0.min(y1),
        w: (x1 - x0).abs(),
        h: (y1 - y0).abs(),
    }
}

fn cache_numbers(n: Option<&Rc<XNode>>) -> Vec<f64> {
    let Some(n) = n else { return Vec::new() };
    let cache = n
        .child("c:numRef")
        .and_then(|r| r.child("c:numCache"))
        .or(n.child("c:numLit"));
    let Some(cache) = cache else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for p in cache.children_named("c:pt") {
        let i = p.attr_i64("idx").unwrap_or(0).max(0) as usize;
        if i >= MAX_POINTS {
            continue;
        }
        if out.len() <= i {
            out.resize(i + 1, 0.0);
        }
        out[i] = p
            .child("c:v")
            .and_then(|v| v.text.trim().parse::<f64>().ok())
            .unwrap_or(0.0);
    }
    out
}

/// Data-label settings, most specific first: the point's `c:dLbl`, then the
/// series `c:dLbls` (or the group's when the series has none).
fn dl_chain(group: &XNode, ser: &XNode, dlbl: Option<Rc<XNode>>) -> Vec<Rc<XNode>> {
    let mut v = Vec::new();
    if let Some(d) = dlbl {
        v.push(d);
    }
    match ser.child("c:dLbls") {
        Some(sd) => v.push(sd.clone()),
        None => {
            if let Some(gd) = group.child("c:dLbls") {
                v.push(gd.clone());
            }
        }
    }
    v
}

fn dl_get<'n>(chain: &'n [Rc<XNode>], name: &str) -> Option<&'n Rc<XNode>> {
    chain.iter().find_map(|n| n.child(name))
}

fn dl_flag(chain: &[Rc<XNode>], name: &str) -> bool {
    dl_get(chain, name)
        .map(|c| c.attr_bool("val").unwrap_or(true))
        .unwrap_or(false)
}

/// Replaces `a:fld` data-label fields (`VALUE`, `CATEGORYNAME`...) with runs.
fn substitute_fields(n: &XNode, f: &dyn Fn(&str) -> Option<String>) -> Rc<XNode> {
    let children = n
        .children
        .iter()
        .map(|c| {
            if c.name == "a:fld"
                && let Some(text) = c.attr("type").and_then(f)
            {
                let mut kids: Vec<Rc<XNode>> = c
                    .children
                    .iter()
                    .filter(|k| k.name == "a:rPr")
                    .cloned()
                    .collect();
                kids.push(text_node("a:t", &text));
                return node("a:r", vec![], kids);
            }
            substitute_fields(c, f)
        })
        .collect();
    Rc::new(XNode {
        name: n.name.clone(),
        attrs: n.attrs.clone(),
        children,
        text: n.text.clone(),
    })
}

/// A pie slice (or ring segment when `r_in > 0`), angles in degrees clockwise
/// from 12 o'clock, vertically squashed by `tilt` for 3-D pies.
fn slice_path(
    cx: f64,
    cy: f64,
    r_in: f64,
    r_out: f64,
    tilt: f64,
    a0: f64,
    sweep: f64,
) -> Option<tiny_skia::Path> {
    let mut pb = PathBuilder::new();
    let pt = |r: f64, a: f64| {
        let a = a.to_radians();
        ((cx + r * a.sin()) as f32, (cy - r * tilt * a.cos()) as f32)
    };
    let steps = ((sweep / 3.0).ceil() as usize).clamp(2, 240);
    if sweep >= 359.999 {
        // Full circle (or ring): two closed contours.
        for (k, r) in [r_out, r_in].iter().enumerate() {
            if *r <= 0.0 {
                continue;
            }
            for s in 0..=steps {
                let a = if k == 0 {
                    a0 + 360.0 * s as f64 / steps as f64
                } else {
                    a0 - 360.0 * s as f64 / steps as f64
                };
                let (x, y) = pt(*r, a);
                if s == 0 {
                    pb.move_to(x, y);
                } else {
                    pb.line_to(x, y);
                }
            }
            pb.close();
        }
        return pb.finish();
    }
    let (sx, sy) = pt(r_out, a0);
    if r_in > 0.0 {
        let (ix, iy) = pt(r_in, a0);
        pb.move_to(ix, iy);
        pb.line_to(sx, sy);
    } else {
        let (x, y) = (cx as f32, cy as f32);
        pb.move_to(x, y);
        pb.line_to(sx, sy);
    }
    for s in 1..=steps {
        let (x, y) = pt(r_out, a0 + sweep * s as f64 / steps as f64);
        pb.line_to(x, y);
    }
    if r_in > 0.0 {
        for s in (0..=steps).rev() {
            let (x, y) = pt(r_in, a0 + sweep * s as f64 / steps as f64);
            pb.line_to(x, y);
        }
    }
    pb.close();
    pb.finish()
}

fn darken(p: &Paint, k: f32) -> Paint {
    let c = p.representative().unwrap_or(Rgba::BLACK);
    Paint::Solid(Rgba {
        r: c.r * k,
        g: c.g * k,
        b: c.b * k,
        a: c.a,
    })
}

/// Shade `i` of `n` for single-hue palettes: darker first, lighter last.
fn linear_shade(c: Rgba, i: usize, n: usize) -> Rgba {
    if n <= 1 {
        return c;
    }
    let t = i as f32 / (n - 1) as f32;
    let f = -0.5 + t;
    if f < 0.0 {
        mix(c, Rgba::BLACK, -f * 0.8)
    } else {
        mix(c, Rgba::WHITE, f * 1.2)
    }
}

/// Series palette of a legacy `c:style` (1..48): column 1 of each row is
/// grayscale, column 2 the accent cycle, columns 3..8 one accent each.
fn style_palette(style: i64) -> Palette {
    let col = (style.clamp(1, 48) - 1) % 8;
    match col {
        0 => Palette::Gray,
        1 => Palette::Cycle {
            colors: (1..=6)
                .map(|i| scheme(&format!("accent{i}"), &[]))
                .collect(),
            variations: default_variations(),
        },
        k => Palette::Linear(scheme(&format!("accent{}", k - 1), &[])),
    }
}

/// Parses `ppt/charts/colorsN.xml` (`cs:colorStyle`).
fn parse_color_style(root: &XNode) -> Option<Palette> {
    let is_color = |n: &XNode| {
        matches!(
            n.name.as_str(),
            "a:srgbClr" | "a:schemeClr" | "a:sysClr" | "a:prstClr" | "a:hslClr" | "a:scrgbClr"
        )
    };
    let colors: Vec<Rc<XNode>> = root
        .children
        .iter()
        .filter(|c| is_color(c))
        .cloned()
        .collect();
    if colors.is_empty() {
        return None;
    }
    let variations: Vec<Vec<Rc<XNode>>> = root
        .children
        .iter()
        .filter(|c| c.local() == "variation")
        .map(|v| v.children.clone())
        .collect();
    match root.attr("meth") {
        Some("withinLinear") | Some("acrossLinear") if colors.len() == 1 => {
            Some(Palette::Linear(colors[0].clone()))
        }
        _ => Some(Palette::Cycle { colors, variations }),
    }
}

/// Moves a paint defined in frame-local space into slide space.
fn map_paint(p: Paint, tf: &Transform) -> Paint {
    let map = |(x, y): (f64, f64)| {
        let mut pt = [tiny_skia::Point::from_xy(x as f32, y as f32)];
        tf.map_points(&mut pt);
        (pt[0].x as f64, pt[0].y as f64)
    };
    match p {
        Paint::Linear { start, end, stops } => Paint::Linear {
            start: map(start),
            end: map(end),
            stops,
        },
        Paint::Radial {
            center,
            radius,
            stops,
        } => {
            let scale = ((tf.sx * tf.sx + tf.ky * tf.ky) as f64).sqrt().max(1e-6);
            Paint::Radial {
                center: map(center),
                radius: radius * scale,
                stops,
            }
        }
        Paint::Image {
            image,
            rect,
            tile,
            alpha,
            frame,
            rot_with_shape,
        } => {
            let full = frame.post_concat(*tf);
            let r = crate::office::pptx::textlayout::transformed_bbox(
                &full, rect.x, rect.y, rect.w, rect.h,
            )
            .unwrap_or(rect);
            Paint::Image {
                image,
                rect: r,
                tile,
                alpha,
                frame: tiny_skia::Transform::identity(),
                rot_with_shape,
            }
        }
        Paint::Path(g) => Paint::Path(Box::new((*g).transformed(tf))),
        other => other,
    }
}

mod chartex;
mod radar;
mod surface;

pub(crate) use chartex::translate as translate_chartex;

#[cfg(test)]
mod tests;
