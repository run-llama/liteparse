//! Character and paragraph formatting of PPT text, written as fully resolved DrawingML.
//!
//! A text's look comes from three layers, later ones winning field by field:
//! 1. the master's TextMasterStyleAtom for the text type and indent level (with the
//!    document defaults from the Environment below it);
//! 2. the shape's TextRulerAtom (margins and indents per level);
//! 3. the StyleTextPropAtom runs: paragraph runs (character count, indent level,
//!    TextPFException) and then character runs (count, TextCFException).
//!
//! Every paragraph and run is written with explicit properties, so office::pptx needs no
//! inheritance from the generated master.

use crate::office::ppt::props::Scheme;
use crate::office::ppt::records::{self as r, Rec};

/// Master units (576 per inch) per point.
const UNITS_PER_POINT: f64 = 8.0;
const EMU_PER_MASTER: f64 = crate::office::ppt::slides::EMU_PER_MASTER;

macro_rules! merge_fields {
    ($dst:expr, $src:expr, $($f:ident),*) => { $( if $src.$f.is_some() { $dst.$f = $src.$f; } )* };
}

/// TextPFException fields; `None` where the exception leaves a value to lower layers.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pf {
    pub has_bullet: Option<bool>,
    pub bullet_has_font: Option<bool>,
    pub bullet_has_color: Option<bool>,
    pub bullet_has_size: Option<bool>,
    pub bullet_char: Option<u16>,
    pub bullet_font: Option<u16>,
    /// Percent of the text size when positive, points (negated) when negative.
    pub bullet_size: Option<i16>,
    pub bullet_color: Option<u32>,
    pub align: Option<u16>,
    /// Percent of single spacing when positive, master units (negated) when negative.
    pub line_spacing: Option<i16>,
    pub space_before: Option<i16>,
    pub space_after: Option<i16>,
    /// Where wrapped lines (and text after the bullet) start, in master units.
    pub left_margin: Option<i16>,
    /// Where the first line (or its bullet) starts, in master units.
    pub indent: Option<i16>,
    /// Automatic numbering from the PowerPoint 2000 extension: (TextAutoNumberScheme
    /// value, first number).
    pub auto_number: Option<(u16, i16)>,
}

impl Pf {
    pub fn merge(&mut self, o: &Pf) {
        merge_fields!(
            self,
            o,
            has_bullet,
            bullet_has_font,
            bullet_has_color,
            bullet_has_size,
            bullet_char,
            bullet_font,
            bullet_size,
            bullet_color,
            align,
            line_spacing,
            space_before,
            space_after,
            left_margin,
            indent,
            auto_number
        );
    }
}

/// TextCFException fields.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Cf {
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    pub shadow: Option<bool>,
    pub font: Option<u16>,
    pub ea_font: Option<u16>,
    pub symbol_font: Option<u16>,
    /// Points.
    pub size: Option<u16>,
    /// ColorIndexStruct: red, green, blue and an index (0xFE: the RGB value, 0..=7: a
    /// colour scheme entry).
    pub color: Option<u32>,
    /// Superscript (positive) or subscript offset, percent of the size.
    pub position: Option<i16>,
    /// Smallest size kerned, in hundredths of a point. The binary format has no
    /// kerning; it comes from the master's round-trip DrawingML text styles.
    pub kern: Option<u32>,
}

impl Cf {
    pub fn merge(&mut self, o: &Cf) {
        merge_fields!(
            self,
            o,
            bold,
            italic,
            underline,
            shadow,
            font,
            ea_font,
            symbol_font,
            size,
            color,
            position,
            kern
        );
    }
}

/// Sequential little-endian reader over one record body.
struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
    end: usize,
}

impl Cursor<'_> {
    fn u16(&mut self) -> Option<u16> {
        if self.pos + 2 > self.end {
            return None;
        }
        let v = r::u16_at(self.data, self.pos)?;
        self.pos += 2;
        Some(v)
    }
    fn i16(&mut self) -> Option<i16> {
        self.u16().map(|v| v as i16)
    }
    fn u32(&mut self) -> Option<u32> {
        if self.pos + 4 > self.end {
            return None;
        }
        let v = r::u32_at(self.data, self.pos)?;
        self.pos += 4;
        Some(v)
    }
    /// `read` when `bit` of `masks` is set.
    fn opt<T>(
        &mut self,
        masks: u32,
        bit: u32,
        read: fn(&mut Self) -> Option<T>,
    ) -> Option<Option<T>> {
        if masks & (1 << bit) != 0 {
            read(self).map(Some)
        } else {
            Some(None)
        }
    }
}

/// TextPFException: a mask and the fields it selects, in the order [MS-PPT] lists them.
fn read_pf(c: &mut Cursor) -> Option<Pf> {
    let m = c.u32()?;
    let mut pf = Pf::default();
    if m & 0xF != 0 {
        let flags = c.u16()?;
        let pick = |bit: u32| (m & (1 << bit) != 0).then_some(flags & (1 << bit) != 0);
        pf.has_bullet = pick(0);
        pf.bullet_has_font = pick(1);
        pf.bullet_has_color = pick(2);
        pf.bullet_has_size = pick(3);
    }
    pf.bullet_char = c.opt(m, 7, Cursor::u16)?;
    pf.bullet_font = c.opt(m, 4, Cursor::u16)?;
    pf.bullet_size = c.opt(m, 6, Cursor::i16)?;
    pf.bullet_color = c.opt(m, 5, Cursor::u32)?;
    pf.align = c.opt(m, 11, Cursor::u16)?;
    pf.line_spacing = c.opt(m, 12, Cursor::i16)?;
    pf.space_before = c.opt(m, 13, Cursor::i16)?;
    pf.space_after = c.opt(m, 14, Cursor::i16)?;
    pf.left_margin = c.opt(m, 8, Cursor::i16)?;
    pf.indent = c.opt(m, 10, Cursor::i16)?;
    c.opt(m, 15, Cursor::u16)?; // defaultTabSize
    if m & (1 << 20) != 0 {
        let n = c.u16()? as usize;
        c.pos += 4 * n;
    }
    c.opt(m, 16, Cursor::u16)?; // fontAlign
    if m & (0x7 << 17) != 0 {
        c.u16()?; // wrapFlags
    }
    c.opt(m, 21, Cursor::u16)?; // textDirection
    Some(pf)
}

/// TextCFException.
fn read_cf(c: &mut Cursor) -> Option<Cf> {
    let m = c.u32()?;
    let mut cf = Cf::default();
    if m & 0x3EB7 != 0 {
        let style = c.u16()?;
        let pick = |bit: u32| (m & (1 << bit) != 0).then_some(style & (1 << bit) != 0);
        cf.bold = pick(0);
        cf.italic = pick(1);
        cf.underline = pick(2);
        cf.shadow = pick(4);
    }
    cf.font = c.opt(m, 16, Cursor::u16)?;
    cf.ea_font = c.opt(m, 21, Cursor::u16)?;
    c.opt(m, 22, Cursor::u16)?; // ansiFontRef
    cf.symbol_font = c.opt(m, 23, Cursor::u16)?;
    cf.size = c.opt(m, 17, Cursor::u16)?;
    cf.color = c.opt(m, 18, Cursor::u32)?;
    cf.position = c.opt(m, 19, Cursor::i16)?;
    Some(cf)
}

fn cursor(data: &[u8], rec: Rec) -> Cursor<'_> {
    Cursor {
        data,
        pos: rec.start,
        end: rec.end,
    }
}

/// TextPFExceptionAtom (a reserved u16, then the exception) or TextCFExceptionAtom.
pub fn read_default_pf(data: &[u8], rec: Rec) -> Option<Pf> {
    let mut c = cursor(data, rec);
    c.u16()?;
    read_pf(&mut c)
}

pub fn read_default_cf(data: &[u8], rec: Rec) -> Option<Cf> {
    read_cf(&mut cursor(data, rec))
}

/// Paragraph runs and character runs of a StyleTextPropAtom. Its counts cover the
/// text plus one character, the paragraph mark that ends the last paragraph.
#[derive(Clone, Debug, Default)]
pub struct Runs {
    pub paras: Vec<(usize, u16, Pf)>,
    pub chars: Vec<(usize, Cf)>,
}

pub fn read_runs(data: &[u8], rec: Rec, text_len: usize) -> Runs {
    let mut c = cursor(data, rec);
    let mut out = Runs::default();
    let total = text_len + 1;
    let mut seen = 0;
    while seen < total {
        let (Some(n), Some(level)) = (c.u32(), c.u16()) else {
            return out;
        };
        let Some(pf) = read_pf(&mut c) else {
            return out;
        };
        let n = (n as usize).max(1);
        out.paras.push((n, level, pf));
        seen += n;
    }
    seen = 0;
    while seen < total {
        let Some(n) = c.u32() else { return out };
        let Some(cf) = read_cf(&mut c) else {
            return out;
        };
        let n = (n as usize).max(1);
        out.chars.push((n, cf));
        seen += n;
    }
    out
}

/// TextRulerAtom: per-level left margins and indents (levels 1..=5).
#[derive(Clone, Copy, Debug, Default)]
pub struct Ruler {
    pub left_margin: [Option<i16>; 5],
    pub indent: [Option<i16>; 5],
}

pub fn read_ruler(data: &[u8], rec: Rec) -> Ruler {
    let mut c = cursor(data, rec);
    let mut ruler = Ruler::default();
    let Some(m) = c.u32() else { return ruler };
    let mut step = || -> Option<()> {
        if m & 0x2 != 0 {
            c.u16()?; // cLevels
        }
        if m & 0x1 != 0 {
            c.u16()?; // defaultTabSize
        }
        if m & 0x4 != 0 {
            let n = c.u16()? as usize;
            c.pos += 4 * n;
        }
        for lvl in 0..5 {
            if m & (0x8 << lvl) != 0 {
                ruler.left_margin[lvl] = Some(c.i16()?);
            }
            if m & (0x100 << lvl) != 0 {
                ruler.indent[lvl] = Some(c.i16()?);
            }
        }
        Some(())
    };
    step();
    ruler
}

/// Text types (TextHeaderAtom) that have a master style, and the type each inherits
/// from: center body, half body and quarter body build on body; center title on title.
const STYLE_TYPES: usize = 9;

fn parent(t: usize) -> Option<usize> {
    match t {
        5 | 7 | 8 => Some(1),
        6 => Some(0),
        _ => None,
    }
}

/// Master text styles by text type, each up to five levels.
#[derive(Clone, Debug, Default)]
pub struct Styles {
    pub levels: Vec<Vec<(Pf, Cf)>>,
    /// `kern` of the title, body and other DrawingML styles by level.
    pub kern: [[Option<u32>; 5]; 3],
}

impl Styles {
    pub fn empty() -> Styles {
        Styles {
            levels: vec![Vec::new(); STYLE_TYPES],
            kern: [[None; 5]; 3],
        }
    }

    /// Reads a TextMasterStyleAtom into the slot for its instance (the text type).
    pub fn add(&mut self, data: &[u8], rec: Rec) {
        let t = rec.inst as usize;
        if t >= STYLE_TYPES {
            return;
        }
        let mut c = cursor(data, rec);
        let Some(n) = c.u16() else { return };
        let mut levels = Vec::new();
        for _ in 0..n.min(5) {
            if t >= 5 && c.u16().is_none() {
                break;
            }
            let Some(pf) = read_pf(&mut c) else { break };
            let Some(cf) = read_cf(&mut c) else { break };
            levels.push((pf, cf));
        }
        self.levels[t] = levels;
    }

    /// Reads `kern` from a `p:txStyles` element (RoundTripOArtTextStyles12Atom).
    pub fn add_kerning(&mut self, tx_styles: &str) {
        let Ok(doc) = roxmltree::Document::parse(tx_styles) else {
            return;
        };
        for (g, name) in ["titleStyle", "bodyStyle", "otherStyle"].iter().enumerate() {
            let Some(style) = doc.descendants().find(|n| n.tag_name().name() == *name) else {
                continue;
            };
            for lvl in style.children().filter(|n| n.is_element()) {
                let tag = lvl.tag_name().name();
                let Some(i) = tag
                    .strip_prefix("lvl")
                    .and_then(|r| r.strip_suffix("pPr"))
                    .and_then(|n| n.parse::<usize>().ok())
                    .filter(|i| (1..=5).contains(i))
                else {
                    continue;
                };
                let kern = lvl
                    .children()
                    .find(|n| n.tag_name().name() == "defRPr")
                    .and_then(|r| r.attribute("kern"))
                    .and_then(|k| k.parse().ok());
                self.kern[g][i - 1] = kern;
            }
        }
    }

    /// Style of text type `t` at indent `level`, over `base` (document defaults). Each
    /// level builds on the levels below it.
    pub fn resolve(&self, t: usize, level: usize, base: &(Pf, Cf)) -> (Pf, Cf) {
        let t = if t < STYLE_TYPES { t } else { 4 };
        let mut out = match parent(t) {
            Some(p) => self.resolve(p, level, base),
            None => *base,
        };
        for (pf, cf) in self.levels[t].iter().take(level + 1) {
            out.0.merge(pf);
            out.1.merge(cf);
        }
        let group = match t {
            0 | 6 => 0,
            1 | 5 | 7 | 8 => 1,
            _ => 2,
        };
        out.1.kern = self.kern[group][level.min(4)];
        out
    }
}

/// What the writer needs to resolve fonts and colours.
pub struct Env<'a> {
    pub fonts: &'a [String],
    pub scheme: &'a Scheme,
    pub styles: &'a Styles,
    pub base: &'a (Pf, Cf),
}

/// One text body: the raw text (UTF-16 units; `\r` ends a paragraph, 0x0B is a line
/// break) with its formatting records.
#[derive(Clone, Debug, Default)]
pub struct Body {
    pub text_type: u32,
    pub units: Vec<u16>,
    pub runs: Option<Runs>,
    pub ruler: Option<Ruler>,
    /// Hyperlinked character ranges (TextInteractiveInfoAtom), start inclusive.
    pub links: Vec<(usize, usize)>,
}

impl Body {
    pub fn text(&self) -> String {
        String::from_utf16_lossy(&self.units)
    }
}

/// Font face for a FontCollection index.
fn face<'a>(env: &'a Env, i: Option<u16>) -> Option<&'a str> {
    env.fonts
        .get(i? as usize)
        .map(String::as_str)
        .filter(|f| !f.is_empty())
}

/// ColorIndexStruct to 0xRRGGBB.
fn text_color(v: u32, scheme: &Scheme) -> Option<u32> {
    let index = v >> 24;
    match index {
        0xFE => Some(((v & 0xFF) << 16) | (v & 0xFF00) | ((v >> 16) & 0xFF)),
        0..=7 => Some(scheme.0[index as usize]),
        _ => None,
    }
}

fn spacing(v: i16) -> String {
    if v >= 0 {
        format!("<a:spcPct val=\"{}\"/>", v as i64 * 1000)
    } else {
        format!(
            "<a:spcPts val=\"{}\"/>",
            ((-(v as f64)) * 100.0 / UNITS_PER_POINT).round() as i64
        )
    }
}

fn emu(v: i16) -> i64 {
    (v as f64 * EMU_PER_MASTER).round() as i64
}

/// `a:rPr` (or `a:endParaRPr` with `tag`) attributes and children for a resolved run.
fn run_props(tag: &str, cf: &Cf, env: &Env) -> String {
    let mut attrs = String::from(" lang=\"en-US\"");
    if let Some(k) = cf.kern {
        attrs.push_str(&format!(" kern=\"{k}\""));
    }
    if let Some(sz) = cf.size {
        attrs.push_str(&format!(" sz=\"{}\"", sz as u32 * 100));
    }
    attrs.push_str(if cf.bold == Some(true) {
        " b=\"1\""
    } else {
        " b=\"0\""
    });
    attrs.push_str(if cf.italic == Some(true) {
        " i=\"1\""
    } else {
        " i=\"0\""
    });
    if cf.underline == Some(true) {
        attrs.push_str(" u=\"sng\"");
    }
    if let Some(p) = cf.position.filter(|&p| p != 0) {
        attrs.push_str(&format!(" baseline=\"{}\"", p as i64 * 1000));
    }
    let mut kids = String::new();
    if let Some(c) = cf.color.and_then(|c| text_color(c, env.scheme)) {
        kids.push_str(&format!(
            "<a:solidFill><a:srgbClr val=\"{c:06X}\"/></a:solidFill>"
        ));
    }
    if cf.shadow == Some(true) {
        kids.push_str(
            "<a:effectLst><a:outerShdw dist=\"38100\" dir=\"2700000\" algn=\"tl\" rotWithShape=\"0\">\
<a:srgbClr val=\"000000\"><a:alpha val=\"50000\"/></a:srgbClr></a:outerShdw></a:effectLst>",
        );
    }
    if let Some(f) = face(env, cf.font) {
        let f = crate::office::ppt::pptx::escape(f);
        kids.push_str(&format!("<a:latin typeface=\"{f}\"/>"));
        match face(env, cf.ea_font) {
            Some(ea) => kids.push_str(&format!(
                "<a:ea typeface=\"{}\"/>",
                crate::office::ppt::pptx::escape(ea)
            )),
            None => kids.push_str(&format!("<a:ea typeface=\"{f}\"/>")),
        }
        kids.push_str(&format!("<a:cs typeface=\"{f}\"/>"));
    }
    format!("<a:{tag}{attrs}>{kids}</a:{tag}>")
}

fn para_props(pf: &Pf, first: &Cf, level: u16, env: &Env) -> String {
    let mut attrs = String::new();
    let lm = pf.left_margin.unwrap_or(0);
    let ind = pf.indent.unwrap_or(0);
    attrs.push_str(&format!(
        " marL=\"{}\" indent=\"{}\"",
        emu(lm),
        emu(ind) - emu(lm)
    ));
    if level > 0 {
        attrs.push_str(&format!(" lvl=\"{}\"", level.min(8)));
    }
    let algn = match pf.align.unwrap_or(0) {
        1 => "ctr",
        2 => "r",
        3 => "just",
        4 => "dist",
        5 => "thaiDist",
        6 => "justLow",
        _ => "l",
    };
    attrs.push_str(&format!(" algn=\"{algn}\""));
    let mut kids = String::new();
    kids.push_str(&format!(
        "<a:lnSpc>{}</a:lnSpc>",
        spacing(pf.line_spacing.unwrap_or(100))
    ));
    kids.push_str(&format!(
        "<a:spcBef>{}</a:spcBef>",
        spacing(pf.space_before.unwrap_or(0))
    ));
    kids.push_str(&format!(
        "<a:spcAft>{}</a:spcAft>",
        spacing(pf.space_after.unwrap_or(0))
    ));
    if pf.has_bullet == Some(true) {
        let color = if pf.bullet_has_color == Some(true) {
            pf.bullet_color
        } else {
            first.color
        };
        if let Some(c) = color.and_then(|c| text_color(c, env.scheme)) {
            kids.push_str(&format!("<a:buClr><a:srgbClr val=\"{c:06X}\"/></a:buClr>"));
        }
        let size = if pf.bullet_has_size == Some(true) {
            pf.bullet_size.unwrap_or(100)
        } else {
            100
        };
        if size > 0 {
            kids.push_str(&format!("<a:buSzPct val=\"{}\"/>", size as i64 * 1000));
        } else if size < 0 {
            kids.push_str(&format!("<a:buSzPts val=\"{}\"/>", -(size as i64) * 100));
        }
        let font = if pf.bullet_has_font == Some(true) {
            face(env, pf.bullet_font)
        } else {
            face(env, first.font)
        };
        if let Some(f) = font {
            kids.push_str(&format!(
                "<a:buFont typeface=\"{}\"/>",
                crate::office::ppt::pptx::escape(f)
            ));
        }
        if let Some((scheme, start)) = pf.auto_number {
            kids.push_str(&format!(
                "<a:buAutoNum type=\"{}\" startAt=\"{}\"/>",
                auto_number_type(scheme),
                start.max(1)
            ));
        } else {
            let ch = match pf.bullet_char {
                Some(c) if c >= 0x20 => char::from_u32(c as u32).unwrap_or('\u{2022}'),
                _ => '\u{2022}',
            };
            kids.push_str(&format!(
                "<a:buChar char=\"{}\"/>",
                crate::office::ppt::pptx::escape(&ch.to_string())
            ));
        }
    } else {
        kids.push_str("<a:buNone/>");
    }
    format!("<a:pPr{attrs}>{kids}</a:pPr>")
}

/// Field text standing for the slide number on a master: written as an `a:fld` of type
/// `slidenum`, which each slide fills with its own number.
pub const SLIDE_NUMBER_FIELD: &str = "\u{FDD0}";

/// The `a:p` elements of a text body. `fields` replaces the characters at the given
/// positions (a slide number's `*`) with text.
pub fn paragraphs(body: &Body, env: &Env, fields: &[(usize, String)]) -> String {
    let units = &body.units;
    let runs = body.runs.clone().unwrap_or_default();
    // Per-unit paragraph run and character run, the last run extending to the end.
    let expand = |counts: Vec<usize>| -> Vec<usize> {
        let mut out = Vec::with_capacity(units.len() + 1);
        for (i, n) in counts.iter().enumerate() {
            out.extend(std::iter::repeat_n(i, *n));
        }
        let last = counts.len().saturating_sub(1);
        out.resize(units.len() + 1, last);
        out
    };
    let para_of = expand(runs.paras.iter().map(|p| p.0).collect());
    let char_of = expand(runs.chars.iter().map(|c| c.0).collect());
    let mut out = String::new();
    let mut start = 0;
    let n = if units.last() == Some(&0x0D) {
        units.len() - 1
    } else {
        units.len()
    };
    loop {
        let end = units[start..n]
            .iter()
            .position(|&u| u == 0x0D)
            .map_or(n, |p| start + p);
        let (level, run_pf) = runs
            .paras
            .get(para_of[start])
            .map_or((0, Pf::default()), |p| (p.1, p.2));
        let lvl = (level as usize).min(4);
        let (mut pf, master_cf) = env.styles.resolve(body.text_type as usize, lvl, env.base);
        if let Some(ruler) = &body.ruler {
            if let Some(v) = ruler.left_margin[lvl] {
                pf.left_margin = Some(v);
            }
            if let Some(v) = ruler.indent[lvl] {
                pf.indent = Some(v);
            }
        }
        pf.merge(&run_pf);
        let cf_at = |i: usize| -> Cf {
            let mut cf = master_cf;
            if let Some((_, c)) = runs.chars.get(char_of[i]) {
                cf.merge(c);
            }
            // PowerPoint draws hyperlinks underlined in the scheme's hyperlink colour.
            if body.links.iter().any(|&(a, b)| (a..b).contains(&i)) {
                cf.color = Some(6 << 24);
                cf.underline = Some(true);
            }
            cf
        };
        let first = cf_at(start);
        out.push_str("<a:p>");
        out.push_str(&para_props(&pf, &first, level, env));
        let mut i = start;
        while i < end {
            if units[i] == 0x0B {
                out.push_str(&format!(
                    "<a:br>{}</a:br>",
                    run_props("rPr", &cf_at(i), env)
                ));
                i += 1;
                continue;
            }
            if fields.iter().any(|f| f.0 == i && f.1 == SLIDE_NUMBER_FIELD) {
                out.push_str(&format!(
                    "<a:fld id=\"{{B6F15528-21DE-4FAA-801E-634DDDAF4B2B}}\" type=\"slidenum\">{}<a:t>‹#›</a:t></a:fld>",
                    run_props("rPr", &cf_at(i), env)
                ));
                i += 1;
                continue;
            }
            let in_link = |k: usize| body.links.iter().any(|&(a, b)| (a..b).contains(&k));
            let run = (char_of[i], in_link(i));
            let mut j = i;
            let mut s = String::new();
            while j < end
                && units[j] != 0x0B
                && (char_of[j], in_link(j)) == run
                && !fields.iter().any(|f| f.0 == j && f.1 == SLIDE_NUMBER_FIELD)
            {
                match fields.iter().find(|f| f.0 == j) {
                    Some((_, t)) => s.push_str(t),
                    None => s.push_str(&String::from_utf16_lossy(&units[j..j + 1])),
                }
                j += 1;
            }
            // A surrogate pair split across two pushes decodes lossily; rejoin it.
            let s = rejoin(&units[i..j], &s, fields, i);
            out.push_str(&format!(
                "<a:r>{}<a:t>{}</a:t></a:r>",
                run_props("rPr", &cf_at(i), env),
                crate::office::ppt::pptx::escape(&s)
            ));
            i = j;
        }
        out.push_str(&run_props("endParaRPr", &cf_at(end.min(units.len())), env));
        out.push_str("</a:p>");
        if end >= n {
            break;
        }
        start = end + 1;
    }
    out
}

/// The run text with surrogate pairs decoded together, unless a field replaced part.
fn rejoin(units: &[u16], built: &str, fields: &[(usize, String)], at: usize) -> String {
    let touched = fields.iter().any(|f| f.0 >= at && f.0 < at + units.len());
    if touched {
        built.to_string()
    } else {
        String::from_utf16_lossy(units)
    }
}

/// DrawingML `buAutoNum` type of an [MS-PPT] TextAutoNumberScheme value.
fn auto_number_type(scheme: u16) -> &'static str {
    const TYPES: [&str; 16] = [
        "alphaLcPeriod",
        "alphaUcPeriod",
        "arabicParenR",
        "arabicPeriod",
        "romanLcParenBoth",
        "romanLcParenR",
        "romanLcPeriod",
        "romanUcPeriod",
        "alphaLcParenBoth",
        "alphaLcParenR",
        "alphaUcParenBoth",
        "alphaUcParenR",
        "arabicParenBoth",
        "arabicPlain",
        "romanUcParenBoth",
        "romanUcParenR",
    ];
    TYPES
        .get(scheme as usize)
        .copied()
        .unwrap_or("arabicPeriod")
}

/// Automatic numbering per paragraph run from a StyleTextProp9Atom: entry `i` extends
/// paragraph run `i`. Each StyleTextProp9 holds a TextPFException9, a
/// TextCFException9 and a TextSIException, whose fields their masks select.
pub fn read_auto_numbers(data: &[u8], rec: Rec) -> Vec<Option<(u16, i16)>> {
    let mut out = Vec::new();
    let mut p = rec.start;
    let u16_at = |p: usize| crate::office::ppt::records::u16_at(data, p);
    let u32_at = |p: usize| crate::office::ppt::records::u32_at(data, p);
    while p + 4 <= rec.end {
        let Some(pf) = u32_at(p) else { break };
        p += 4;
        if pf & 0x0080_0000 != 0 {
            p += 2;
        }
        let mut has = None;
        if pf & 0x0200_0000 != 0 {
            has = u16_at(p);
            p += 2;
        }
        let mut scheme = None;
        if pf & 0x0100_0000 != 0 {
            scheme = u16_at(p).zip(u16_at(p + 2).map(|v| v as i16));
            p += 4;
        }
        let Some(cf) = u32_at(p) else { break };
        p += 4;
        for (bit, size) in [
            (0x10_0000, 4),
            (0x40_0000, 2),
            (0x80_0000, 2),
            (0x100_0000, 4),
        ] {
            if cf & bit != 0 {
                p += size;
            }
        }
        let Some(si) = u32_at(p) else { break };
        p += 4;
        for (bit, size) in [(0x1, 2), (0x2, 2), (0x4, 2), (0x40, 2), (0x20, 4)] {
            if si & bit != 0 {
                p += size;
            }
        }
        if si & 0x200 != 0 {
            let n = u32_at(p).unwrap_or(0) as usize;
            p += 4 + 4 * n.min(rec.end);
        }
        if p > rec.end {
            break;
        }
        out.push(match (has, scheme) {
            (Some(1), Some(s)) => Some(s),
            (Some(1), None) => Some((3, 1)),
            _ => None,
        });
    }
    out
}
