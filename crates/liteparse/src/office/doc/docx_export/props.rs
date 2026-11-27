//! Property modifiers (sprms) translated into WordprocessingML property elements.
//! Unknown modifiers are ignored; the result is the closest OOXML equivalent.
use super::source::{i16_le, sprms, u16_le, u32_le};
use std::fmt::Write;

/// The 16-entry Word color index (Ico) as RGB hex.
pub const ICO: [&str; 17] = [
    "auto", "000000", "0000FF", "00FFFF", "00FF00", "FF00FF", "FF0000", "FFFF00", "FFFFFF",
    "000080", "008080", "008000", "800080", "800000", "808000", "808080", "C0C0C0",
];
const HIGHLIGHT: [&str; 17] = [
    "none",
    "black",
    "blue",
    "cyan",
    "green",
    "magenta",
    "red",
    "yellow",
    "white",
    "darkBlue",
    "darkCyan",
    "darkGreen",
    "darkMagenta",
    "darkRed",
    "darkYellow",
    "darkGray",
    "lightGray",
];

pub fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            // XML 1.0 forbids most C0 controls even as references.
            c if (c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r') => {}
            '\u{fffe}' | '\u{ffff}' => {}
            c => out.push(c),
        }
    }
    out
}

/// COLORREF (0x00BBGGRR, high byte 0xFF = auto) as RGB hex.
pub fn colorref(v: u32) -> String {
    if v >> 24 == 0xff {
        "auto".into()
    } else {
        format!(
            "{:02X}{:02X}{:02X}",
            v & 0xff,
            (v >> 8) & 0xff,
            (v >> 16) & 0xff
        )
    }
}

/// Character state that later output needs besides the rPr XML.
#[derive(Default, Clone)]
pub struct CharFlags {
    pub deleted: bool,
    pub special: bool,
    pub ole: bool,
    pub data: bool,
    pub pic_location: Option<u32>,
    pub symbol: Option<(u16, u16)>,
    pub istd: Option<u16>,
    /// Input: toggles that are on in the governing style, so 0x81 ("opposite of the
    /// style") resolves to an explicit value.
    pub style_on: Vec<&'static str>,
}

/// Character toggle properties by sprm.
pub fn toggle_name(sprm: u16) -> Option<&'static str> {
    Some(match sprm {
        0x0835 => "b",
        0x0836 => "i",
        0x0837 => "strike",
        0x2a53 => "dstrike",
        0x0838 => "outline",
        0x0839 => "shadow",
        0x083a => "smallCaps",
        0x083b => "caps",
        0x083c => "vanish",
        0x0854 => "imprint",
        0x0858 => "emboss",
        0x085c => "bCs",
        0x085d => "iCs",
        0x085a => "rtl",
        _ => return None,
    })
}

/// Toggle operand: 0 off, 1 on, 0x80 same as the style, 0x81 opposite of the style.
pub fn toggle_value(op: u8, style_on: bool) -> Option<bool> {
    match op {
        0 => Some(false),
        1 => Some(true),
        0x80 => None,
        0x81 => Some(!style_on),
        _ => None,
    }
}

fn toggle(name: &str, op: &[u8], style_on: &[&str], out: &mut String) {
    let on = op
        .first()
        .and_then(|&v| toggle_value(v, style_on.contains(&name)));
    match on {
        Some(false) => write!(out, "<w:{name} w:val=\"0\"/>").unwrap(),
        Some(true) => write!(out, "<w:{name}/>").unwrap(),
        None => {}
    }
}

/// Translate character modifiers. `fonts` maps font indexes to names.
pub fn rpr(grpprl: &[u8], fonts: &[String], flags: &mut CharFlags) -> String {
    let mut fonts_xml = [None, None, None, None];
    let mut out = String::new();
    let mut size = None;
    let mut size_cs = None;
    let mut color = None;
    let mut underline = None;
    let mut vert = None;
    let mut highlight = None;
    let mut shading = None;
    let mut toggles: Vec<(&str, Vec<u8>)> = Vec::new();
    let mut spacing = None;
    let mut position = None;
    let mut scale = None;
    let mut lang = None;
    for (sprm, op) in sprms(grpprl) {
        let font = |op: &[u8]| u16_le(op, 0).and_then(|i| fonts.get(i as usize)).cloned();
        match sprm {
            // 0x81 is the opposite of the style's value; styles carry no revision marks.
            0x0800 => flags.deleted = matches!(op.first(), Some(1 | 0x81)),
            0x0806 => flags.data = op.first() == Some(&1),
            0x080a => flags.ole = op.first() == Some(&1),
            0x0855 => flags.special = op.first() == Some(&1),
            0x6a03 => {
                flags.pic_location = u32_le(op, 0);
                flags.special = true;
            }
            0x6a09 => {
                flags.symbol = Some((u16_le(op, 0).unwrap_or(0), u16_le(op, 2).unwrap_or(0)));
                flags.special = true;
            }
            0x4a30 => flags.istd = u16_le(op, 0),
            t if toggle_name(t).is_some() => toggles.push((toggle_name(t).unwrap(), op.to_vec())),
            0x4a43 => size = u16_le(op, 0),
            0x4a61 => size_cs = u16_le(op, 0),
            0x2a42 => {
                color = op
                    .first()
                    .and_then(|&i| ICO.get(i as usize))
                    .map(|s| s.to_string())
            }
            0x6870 => color = u32_le(op, 0).map(colorref),
            0x2a3e => underline = op.first().copied(),
            0x2a48 => vert = op.first().copied(),
            0x2a0c => highlight = op.first().copied(),
            0x4a4f => fonts_xml[0] = font(op),
            0x4a50 => fonts_xml[1] = font(op),
            0x4a51 => fonts_xml[2] = font(op),
            0x4a5e => fonts_xml[3] = font(op),
            0x8840 => spacing = i16_le(op, 0),
            0x4845 => position = i16_le(op, 0),
            0x4852 => scale = u16_le(op, 0),
            0x486d | 0x4873 => lang = u16_le(op, 0),
            0xca71 => {
                // Shd (cvFore, cvBack, ipat); the operand's cb byte is already stripped.
                if let Some(back) = u32_le(op, 4)
                    && back >> 24 != 0xff
                {
                    shading = Some(colorref(back));
                }
            }
            0x4866 => {
                // Shd80: icoFore 5 bits, icoBack 5 bits, ipat 6 bits.
                if let Some(v) = u16_le(op, 0) {
                    let back = ((v >> 5) & 0x1f) as usize;
                    if back != 0 {
                        shading = ICO.get(back).map(|s| s.to_string());
                    }
                }
            }
            _ => {}
        }
    }
    if fonts_xml.iter().any(Option::is_some) {
        out.push_str("<w:rFonts");
        for (attr, value) in [
            ("w:ascii", &fonts_xml[0]),
            ("w:hAnsi", &fonts_xml[2].clone().or(fonts_xml[0].clone())),
            ("w:eastAsia", &fonts_xml[1]),
            ("w:cs", &fonts_xml[3]),
        ] {
            if let Some(v) = value {
                write!(out, " {attr}=\"{}\"", esc(v)).unwrap();
            }
        }
        out.push_str("/>");
    }
    for (name, op) in &toggles {
        if !matches!(
            *name,
            "rtl" | "vanish" | "emboss" | "imprint" | "outline" | "shadow"
        ) {
            toggle(name, op, &flags.style_on, &mut out);
        }
    }
    for name in ["outline", "shadow", "emboss", "imprint"] {
        if let Some((_, op)) = toggles.iter().rev().find(|t| t.0 == name) {
            toggle(name, op, &flags.style_on, &mut out);
        }
    }
    if let Some((_, op)) = toggles.iter().rev().find(|t| t.0 == "vanish") {
        toggle("vanish", op, &flags.style_on, &mut out);
    }
    if let Some(c) = color {
        write!(out, "<w:color w:val=\"{c}\"/>").unwrap();
    }
    if let Some(s) = spacing {
        write!(out, "<w:spacing w:val=\"{s}\"/>").unwrap();
    }
    if let Some(s) = scale
        && s != 100
        && (1..=600).contains(&s)
    {
        write!(out, "<w:w w:val=\"{s}\"/>").unwrap();
    }
    if let Some(p) = position {
        write!(out, "<w:position w:val=\"{p}\"/>").unwrap();
    }
    if let Some(s) = size {
        write!(out, "<w:sz w:val=\"{s}\"/>").unwrap();
    }
    if let Some(s) = size_cs.or(size) {
        write!(out, "<w:szCs w:val=\"{s}\"/>").unwrap();
    }
    if let Some(h) = highlight
        && h != 0
        && let Some(name) = HIGHLIGHT.get(h as usize)
    {
        write!(out, "<w:highlight w:val=\"{name}\"/>").unwrap();
    }
    if let Some(u) = underline {
        let val = match u {
            0 => "none",
            1 => "single",
            2 => "words",
            3 => "double",
            4 => "dotted",
            6 => "thick",
            7 => "dash",
            9 => "dotDash",
            10 => "dotDotDash",
            11 => "wave",
            20 => "dottedHeavy",
            23 => "dashedHeavy",
            25 => "dashDotHeavy",
            26 => "dashDotDotHeavy",
            27 => "wavyHeavy",
            39 => "dashLong",
            43 => "wavyDouble",
            55 => "dashLongHeavy",
            _ => "single",
        };
        write!(out, "<w:u w:val=\"{val}\"/>").unwrap();
    }
    if let Some(s) = shading {
        write!(
            out,
            "<w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"{s}\"/>"
        )
        .unwrap();
    }
    if let Some(v) = vert {
        let val = match v {
            1 => "superscript",
            2 => "subscript",
            _ => "baseline",
        };
        write!(out, "<w:vertAlign w:val=\"{val}\"/>").unwrap();
    }
    if let Some((_, op)) = toggles.iter().rev().find(|t| t.0 == "rtl") {
        toggle("rtl", op, &flags.style_on, &mut out);
    }
    let _ = lang;
    out
}

/// Paragraph state that structure building needs besides the pPr XML.
#[derive(Default, Clone, Debug)]
pub struct ParaFlags {
    pub in_table: bool,
    pub ttp: bool,
    pub itap: Option<u32>,
    pub inner_cell: bool,
    pub inner_ttp: bool,
    pub ilfo: Option<u16>,
    pub ilvl: Option<u8>,
}

impl ParaFlags {
    pub fn depth(&self) -> u32 {
        self.itap.unwrap_or(u32::from(self.in_table))
    }
}

/// Brc80 (4 bytes) or Brc (8 bytes) as an OOXML border element.
pub fn border(name: &str, op: &[u8]) -> Option<String> {
    let (width_eighths, kind, color, space) = if op.len() >= 8 {
        let cv = u32_le(op, 0)?;
        (op[4], op[5], colorref(cv), op[6] & 0x1f)
    } else if op.len() >= 4 {
        if op[..4] == [0xff; 4] {
            return None;
        }
        (
            op[0],
            op[1],
            ICO.get(op[2] as usize).unwrap_or(&"auto").to_string(),
            op[3] & 0x1f,
        )
    } else {
        return None;
    };
    if kind == 0 || kind == 0xff {
        return Some(format!("<w:{name} w:val=\"nil\"/>"));
    }
    let val = match kind {
        1 => "single",
        2 => "thick",
        3 => "double",
        5 => "single",
        6 => "dotted",
        7 => "dashed",
        8 => "dotDash",
        9 => "dotDotDash",
        10 => "triple",
        11..=19 => "thinThickSmallGap",
        20 => "wave",
        21 => "doubleWave",
        22 => "dashSmallGap",
        23 => "dashDotStroked",
        24 => "threeDEmboss",
        25 => "threeDEngrave",
        26 => "outset",
        27 => "inset",
        _ => "single",
    };
    Some(format!(
        "<w:{name} w:val=\"{val}\" w:sz=\"{}\" w:space=\"{space}\" w:color=\"{color}\"/>",
        width_eighths.max(2)
    ))
}

/// Whether `sprm` is one of the fixed-size frame modifiers `frame_xml` reads.
pub fn is_frame_sprm(sprm: u16) -> bool {
    matches!(
        sprm,
        0x261b | 0x8418 | 0x8419 | 0x841a | 0x442b | 0x2423 | 0x842f | 0x842e
    )
}

/// Translate paragraph modifiers into pPr children (without pStyle/numPr).
/// w:framePr from the DOC frame modifiers, or "" when the paragraph has none. pc holds
/// the vertical anchor in bits 4-5 (margin, page, paragraph) and the horizontal one in
/// bits 6-7 (column, margin, page); dxaAbs/dyaAbs are twips or negative alignment codes.
fn frame_xml(f: &[Option<i32>; 8]) -> String {
    let [pc, x, y, w, h, wrap, h_space, v_space] = *f;
    if pc.is_none() && x.is_none() && y.is_none() && w.is_none() {
        return String::new();
    }
    let pc = pc.unwrap_or(0);
    let v_anchor = ["margin", "page", "text", "text"][((pc >> 4) & 3) as usize];
    let h_anchor = ["text", "margin", "page", "page"][((pc >> 6) & 3) as usize];
    let mut out = String::from("<w:framePr");
    if let Some(w) = w.filter(|&w| w > 0) {
        write!(out, " w:w=\"{w}\"").unwrap();
    }
    if let Some(h) = h.filter(|&h| h & 0x7fff > 0) {
        // Bit 15 set: at least this height; clear: exact.
        let rule = if h & 0x8000 != 0 { "atLeast" } else { "exact" };
        write!(out, " w:h=\"{}\" w:hRule=\"{rule}\"", h & 0x7fff).unwrap();
    }
    let wrap = match wrap {
        Some(1) => "notBeside",
        Some(3) => "none",
        Some(4) => "tight",
        Some(5) => "through",
        _ => "around",
    };
    write!(
        out,
        " w:wrap=\"{wrap}\" w:hAnchor=\"{h_anchor}\" w:vAnchor=\"{v_anchor}\""
    )
    .unwrap();
    match x.unwrap_or(0) {
        0 => out.push_str(" w:xAlign=\"left\""),
        -4 => out.push_str(" w:xAlign=\"center\""),
        -8 => out.push_str(" w:xAlign=\"right\""),
        -12 => out.push_str(" w:xAlign=\"inside\""),
        -16 => out.push_str(" w:xAlign=\"outside\""),
        v => write!(out, " w:x=\"{v}\"").unwrap(),
    }
    match y.unwrap_or(0) {
        -4 => out.push_str(" w:yAlign=\"top\""),
        -8 => out.push_str(" w:yAlign=\"center\""),
        -12 => out.push_str(" w:yAlign=\"bottom\""),
        -16 => out.push_str(" w:yAlign=\"inside\""),
        -20 => out.push_str(" w:yAlign=\"outside\""),
        v => write!(out, " w:y=\"{v}\"").unwrap(),
    }
    if let Some(s) = h_space.filter(|&s| s > 0) {
        write!(out, " w:hSpace=\"{s}\"").unwrap();
    }
    if let Some(s) = v_space.filter(|&s| s > 0) {
        write!(out, " w:vSpace=\"{s}\"").unwrap();
    }
    out.push_str("/>");
    out
}

pub fn ppr(grpprl: &[u8], flags: &mut ParaFlags) -> String {
    let mut jc = None;
    let mut jc_logical = None;
    let (mut left, mut right, mut first) = (None, None, None);
    let (mut before, mut after) = (None, None);
    let (mut before_auto, mut after_auto) = (None, None);
    let mut line = None;
    let mut keep = None;
    let mut keep_next = None;
    let mut page_break = None;
    let mut widow = None;
    let mut outline = None;
    let mut bidi = None;
    let mut snap = None;
    let mut tabs: Vec<(i16, Option<u8>)> = Vec::new();
    let mut borders: [Option<String>; 6] = Default::default();
    let mut shading = None;
    let mut contextual = None;
    // Frame: pc, dxaAbs, dyaAbs, dxaWidth, dyaHeight, wr, dxaFromText, dyaFromText.
    let mut frame: [Option<i32>; 8] = [None; 8];
    for (sprm, op) in sprms(grpprl) {
        let word = || i16_le(op, 0).map(i32::from);
        match sprm {
            0x261b => frame[0] = op.first().map(|&v| v as i32),
            0x8418 => frame[1] = word(),
            0x8419 => frame[2] = word(),
            0x841a => frame[3] = word(),
            0x442b => frame[4] = u16_le(op, 0).map(i32::from),
            0x2423 => frame[5] = op.first().map(|&v| v as i32),
            0x842f => frame[6] = word(),
            0x842e => frame[7] = word(),
            0x2403 => jc = op.first().copied(),
            0x2461 => jc_logical = op.first().copied(),
            0x840f | 0x845e => left = i16_le(op, 0),
            0x840e | 0x845d => right = i16_le(op, 0),
            0x8411 | 0x8460 => first = i16_le(op, 0),
            0xa413 => before = u16_le(op, 0),
            0xa414 => after = u16_le(op, 0),
            0x245b => before_auto = op.first().copied(),
            0x245c => after_auto = op.first().copied(),
            0x6412 => line = Some((i16_le(op, 0).unwrap_or(240), i16_le(op, 2).unwrap_or(1))),
            0x2405 => keep = op.first().copied(),
            0x2406 => keep_next = op.first().copied(),
            0x2407 => page_break = op.first().copied(),
            0x2431 => widow = op.first().copied(),
            0x2640 => outline = op.first().copied(),
            0x2441 => bidi = op.first().copied(),
            // sprmPFUsePgsuSettings: lines follow the section's document grid.
            0x2447 => snap = op.first().copied(),
            0x246d => contextual = op.first().copied(),
            0x2416 => flags.in_table = op.first() == Some(&1),
            0x2417 => flags.ttp = op.first() == Some(&1),
            0x6649 => flags.itap = u32_le(op, 0),
            0x664a => {
                // sprmPDtap adds to the current depth.
                let d = u32_le(op, 0).unwrap_or(0) as i32;
                flags.itap = Some((flags.depth() as i32 + d).max(0) as u32);
            }
            0x244b => flags.inner_cell = op.first() == Some(&1),
            0x244c => flags.inner_ttp = op.first() == Some(&1),
            0x260a => flags.ilvl = op.first().copied(),
            0x460b => flags.ilfo = u16_le(op, 0),
            0x6424 | 0xc64e => borders[0] = border("top", op),
            0x6425 | 0xc64f => borders[1] = border("left", op),
            0x6426 | 0xc650 => borders[2] = border("bottom", op),
            0x6427 | 0xc651 => borders[3] = border("right", op),
            0x6428 | 0xc652 => borders[4] = border("between", op),
            0x442d => {
                if let Some(v) = u16_le(op, 0) {
                    let back = ((v >> 5) & 0x1f) as usize;
                    if back != 0 {
                        shading = ICO.get(back).map(|s| s.to_string());
                    }
                }
            }
            0xc64d => {
                if let Some(back) = u32_le(op, 4)
                    && back >> 24 != 0xff
                {
                    shading = Some(colorref(back));
                }
            }
            0xc60d | 0xc615 => apply_tabs(sprm, op, &mut tabs),
            _ => {}
        }
    }
    let mut out = frame_xml(&frame);
    if keep_next == Some(1) {
        out.push_str("<w:keepNext/>");
    }
    if keep == Some(1) {
        out.push_str("<w:keepLines/>");
    }
    if page_break == Some(1) {
        out.push_str("<w:pageBreakBefore/>");
    }
    if let Some(w) = widow {
        out.push_str(if w == 0 {
            "<w:widowControl w:val=\"0\"/>"
        } else {
            "<w:widowControl/>"
        });
    }
    if borders.iter().any(Option::is_some) {
        out.push_str("<w:pBdr>");
        for b in borders.iter().flatten() {
            out.push_str(b);
        }
        out.push_str("</w:pBdr>");
    }
    if let Some(s) = shading {
        write!(
            out,
            "<w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"{s}\"/>"
        )
        .unwrap();
    }
    if !tabs.is_empty() {
        out.push_str("<w:tabs>");
        for (pos, kind) in &tabs {
            match kind {
                None => write!(out, "<w:tab w:val=\"clear\" w:pos=\"{pos}\"/>").unwrap(),
                Some(tbd) => {
                    let val = match tbd & 7 {
                        1 => "center",
                        2 => "right",
                        3 => "decimal",
                        4 => "bar",
                        _ => "left",
                    };
                    let leader = match (tbd >> 3) & 7 {
                        1 => " w:leader=\"dot\"",
                        2 => " w:leader=\"hyphen\"",
                        3 => " w:leader=\"underscore\"",
                        4 => " w:leader=\"heavy\"",
                        5 => " w:leader=\"middleDot\"",
                        _ => "",
                    };
                    write!(out, "<w:tab w:val=\"{val}\"{leader} w:pos=\"{pos}\"/>").unwrap();
                }
            }
        }
        out.push_str("</w:tabs>");
    }
    if let Some(v) = snap {
        write!(out, "<w:snapToGrid w:val=\"{}\"/>", u8::from(v != 0)).unwrap();
    }
    let any_auto = before_auto.is_some() || after_auto.is_some();
    if before.is_some() || after.is_some() || line.is_some() || any_auto {
        out.push_str("<w:spacing");
        if let Some(b) = before {
            write!(out, " w:before=\"{b}\"").unwrap();
        }
        // An explicit 0 matters: it switches off auto spacing set by the style.
        if let Some(v) = before_auto {
            write!(out, " w:beforeAutospacing=\"{}\"", u8::from(v != 0)).unwrap();
        }
        if let Some(a) = after {
            write!(out, " w:after=\"{a}\"").unwrap();
        }
        if let Some(v) = after_auto {
            write!(out, " w:afterAutospacing=\"{}\"", u8::from(v != 0)).unwrap();
        }
        if let Some((dya, mult)) = line {
            // A negative dyaLine is exact spacing whatever fMultLinespace says.
            if dya < 0 {
                write!(out, " w:line=\"{}\" w:lineRule=\"exact\"", -(dya as i32)).unwrap();
            } else if mult != 0 {
                write!(out, " w:line=\"{}\" w:lineRule=\"auto\"", dya.max(1)).unwrap();
            } else {
                write!(out, " w:line=\"{dya}\" w:lineRule=\"atLeast\"").unwrap();
            }
        }
        out.push_str("/>");
    }
    if left.is_some() || right.is_some() || first.is_some() {
        out.push_str("<w:ind");
        if let Some(l) = left {
            write!(out, " w:left=\"{l}\"").unwrap();
        }
        if let Some(r) = right {
            write!(out, " w:right=\"{r}\"").unwrap();
        }
        match first {
            Some(f) if f < 0 => write!(out, " w:hanging=\"{}\"", -(f as i32)).unwrap(),
            Some(f) => write!(out, " w:firstLine=\"{f}\"").unwrap(),
            None => {}
        }
        out.push_str("/>");
    }
    if contextual == Some(1) {
        out.push_str("<w:contextualSpacing/>");
    }
    if bidi == Some(1) {
        out.push_str("<w:bidi/>");
    }
    if let Some(j) = jc_logical.or(jc) {
        let val = match j {
            1 => "center",
            2 => "right",
            3 => "both",
            4 => "distribute",
            _ => "left",
        };
        write!(out, "<w:jc w:val=\"{val}\"/>").unwrap();
    }
    if let Some(o) = outline
        && o < 9
    {
        write!(out, "<w:outlineLvl w:val=\"{o}\"/>").unwrap();
    }
    out
}

/// Apply sprmPChgTabsPapx / sprmPChgTabs: deletions then additions, sorted by position.
fn apply_tabs(sprm: u16, op: &[u8], tabs: &mut Vec<(i16, Option<u8>)>) {
    let mut at = 0;
    let Some(&del) = op.get(at) else { return };
    at += 1;
    let mut deleted = Vec::new();
    for i in 0..del as usize {
        if let Some(p) = i16_le(op, at + i * 2) {
            deleted.push(p);
        }
    }
    at += del as usize * 2;
    if sprm == 0xc615 {
        at += del as usize * 2;
    }
    let Some(&add) = op.get(at) else { return };
    at += 1;
    tabs.retain(|t| !deleted.contains(&t.0));
    for p in &deleted {
        tabs.push((*p, None));
    }
    for i in 0..add as usize {
        let Some(p) = i16_le(op, at + i * 2) else {
            break;
        };
        let tbd = op.get(at + add as usize * 2 + i).copied().unwrap_or(0);
        tabs.retain(|t| t.0 != p);
        tabs.push((p, Some(tbd)));
    }
    tabs.sort_by_key(|t| t.0);
}

/// Section geometry and options read from SEPX modifiers.
#[derive(Clone)]
pub struct Section {
    pub width: u16,
    pub height: u16,
    pub landscape: bool,
    pub margins: [i32; 4],
    pub header: u16,
    pub footer: u16,
    pub gutter: u16,
    pub columns: u16,
    pub column_space: u16,
    pub break_kind: u8,
    pub title_page: bool,
    pub page_start: Option<u16>,
    pub page_restart: bool,
    /// sprmSNfcPgn: the page number format (MSONFC), 0 decimal.
    pub page_format: u8,
    pub vertical_align: u8,
    /// Line numbering: count-by, distance (twips), start and sprmSLnc restart mode.
    pub line_numbers: Option<(u16, i16, u16, u8)>,
    /// Document grid: sprmSClm type, line pitch and character spacing.
    pub grid: (u16, Option<u16>, Option<i32>),
    /// Unequal columns (sprmSFEvenlySpaced off): width and following space per column.
    pub column_widths: Option<Vec<(u16, u16)>>,
    /// Page border elements (top, left, bottom, right) and sprmSPgbProp options.
    pub page_borders: [Option<String>; 4],
    pub page_border_options: u16,
}

impl Default for Section {
    fn default() -> Self {
        Section {
            width: 12240,
            height: 15840,
            landscape: false,
            margins: [1440, 1800, 1440, 1800],
            header: 720,
            footer: 720,
            gutter: 0,
            columns: 1,
            column_space: 720,
            break_kind: 2,
            title_page: false,
            page_start: None,
            page_restart: false,
            page_format: 0,
            vertical_align: 0,
            line_numbers: None,
            grid: (0, None, None),
            column_widths: None,
            page_borders: [None, None, None, None],
            page_border_options: 0,
        }
    }
}

pub fn section(grpprl: &[u8]) -> Section {
    let mut s = Section::default();
    let mut ln_mod = 0;
    let mut ln_start = 0;
    let mut ln_dist = 0;
    let mut ln_restart = 0;
    // Development dump of section modifiers (see `doc-to-docx`).
    if std::env::var("DOC_DUMP_SEPX").is_ok() {
        let all: Vec<String> = sprms(grpprl)
            .map(|(s, o)| format!("{s:04x}:{o:02x?}"))
            .collect();
        eprintln!("sepx [{}]", all.join(" "));
    }
    for (sprm, op) in sprms(grpprl) {
        let w = || u16_le(op, 0).unwrap_or(0);
        match sprm {
            // sprmSBrcTop80..Right80 (Brc80) and sprmSBrcTop..Right (Brc).
            0x702b..=0x702e | 0xd234..=0xd237 => {
                let side = (sprm & 0xf) as usize - if sprm >> 12 == 7 { 0xb } else { 4 };
                let name = ["top", "left", "bottom", "right"][side];
                s.page_borders[side] = border(name, op);
            }
            0x522f => s.page_border_options = w(),
            0x3005 if op.first() == Some(&0) => {
                s.column_widths.get_or_insert_with(Vec::new);
            }
            // sprmSDxaColWidth / sprmSDxaColSpacing: column index, then twips.
            0xf203 | 0xf204 => {
                let (Some(&i), Some(v)) = (op.first(), u16_le(op, 1)) else {
                    continue;
                };
                let cols = s.column_widths.get_or_insert_with(Vec::new);
                if (i as usize) < 45 {
                    if cols.len() <= i as usize {
                        cols.resize(i as usize + 1, (0, 0));
                    }
                    if sprm == 0xf203 {
                        cols[i as usize].0 = v;
                    } else {
                        cols[i as usize].1 = v;
                    }
                }
            }
            0xb01f => s.width = w(),
            0xb020 => s.height = w(),
            0x301d => s.landscape = op.first() == Some(&2),
            0x9023 => s.margins[0] = i16_le(op, 0).unwrap_or(1440) as i32,
            0xb021 => s.margins[1] = w() as i32,
            0x9024 => s.margins[2] = i16_le(op, 0).unwrap_or(1440) as i32,
            0xb022 => s.margins[3] = w() as i32,
            0xb017 => s.header = w(),
            0xb018 => s.footer = w(),
            0xb025 => s.gutter = w(),
            0x500b => s.columns = w().saturating_add(1),
            0x900c => s.column_space = w(),
            0x3009 => s.break_kind = op.first().copied().unwrap_or(2),
            0x300a => s.title_page = op.first() == Some(&1),
            0x501c => s.page_start = Some(w()),
            0x3011 => s.page_restart = op.first() == Some(&1),
            0x300e => s.page_format = op.first().copied().unwrap_or(0),
            0x301a => s.vertical_align = op.first().copied().unwrap_or(0),
            0x5015 => ln_mod = w(),
            0x9016 => ln_dist = i16_le(op, 0).unwrap_or(0),
            0x501b => ln_start = w(),
            0x3013 => ln_restart = op.first().copied().unwrap_or(0),
            0x5032 => s.grid.0 = w(),
            0x9031 => s.grid.1 = Some(w()),
            0x7030 => s.grid.2 = u32_le(op, 0).map(|v| v as i32),
            _ => {}
        }
    }
    if ln_mod > 0 {
        s.line_numbers = Some((ln_mod, ln_dist, ln_start, ln_restart));
    }
    s
}

impl Section {
    /// sectPr children after the header/footer references.
    pub fn xml(&self) -> String {
        let mut out = String::new();
        let kind = match self.break_kind {
            0 => "continuous",
            1 => "nextColumn",
            3 => "evenPage",
            4 => "oddPage",
            _ => "nextPage",
        };
        write!(out, "<w:type w:val=\"{kind}\"/>").unwrap();
        write!(
            out,
            "<w:pgSz w:w=\"{}\" w:h=\"{}\"{}/>",
            self.width,
            self.height,
            if self.landscape {
                " w:orient=\"landscape\""
            } else {
                ""
            }
        )
        .unwrap();
        write!(
            out,
            "<w:pgMar w:top=\"{}\" w:right=\"{}\" w:bottom=\"{}\" w:left=\"{}\" w:header=\"{}\" w:footer=\"{}\" w:gutter=\"{}\"/>",
            self.margins[0], self.margins[3], self.margins[2], self.margins[1], self.header,
            self.footer, self.gutter
        )
        .unwrap();
        let sides: String = self
            .page_borders
            .iter()
            .flatten()
            .filter(|b| !b.contains("\"nil\""))
            .cloned()
            .collect();
        if !sides.is_empty() {
            // sprmSPgbProp: pgbApplyTo in bits 0-2, pgbOffsetFrom (1 = page edge) in bits 5-7.
            let display = match self.page_border_options & 7 {
                1 => "firstPage",
                2 => "notFirstPage",
                _ => "allPages",
            };
            let from = if (self.page_border_options >> 5) & 7 == 1 {
                "page"
            } else {
                "text"
            };
            write!(
                out,
                "<w:pgBorders w:offsetFrom=\"{from}\" w:display=\"{display}\">{sides}</w:pgBorders>"
            )
            .unwrap();
        }
        if let Some((count_by, distance, start, restart)) = self.line_numbers {
            // sprmSLnc: 0 restarts each page, 1 each section, 2 continues through the document.
            let restart = match restart {
                1 => "newSection",
                2 => "continuous",
                _ => "newPage",
            };
            write!(
                out,
                "<w:lnNumType w:countBy=\"{count_by}\" w:distance=\"{}\" w:start=\"{start}\" w:restart=\"{restart}\"/>",
                distance.max(0)
            )
            .unwrap();
        }
        let fmt = match self.page_format {
            1 => Some("upperRoman"),
            2 => Some("lowerRoman"),
            3 => Some("upperLetter"),
            4 => Some("lowerLetter"),
            22 => Some("decimalZero"),
            57 => Some("numberInDash"),
            _ => None,
        };
        if self.page_restart || fmt.is_some() {
            out.push_str("<w:pgNumType");
            if let Some(fmt) = fmt {
                write!(out, " w:fmt=\"{fmt}\"").unwrap();
            }
            if self.page_restart {
                write!(out, " w:start=\"{}\"", self.page_start.unwrap_or(1)).unwrap();
            }
            out.push_str("/>");
        }
        let widths = self
            .column_widths
            .as_ref()
            .filter(|w| w.len() == self.columns as usize && w.iter().all(|c| c.0 > 0));
        match widths {
            Some(widths) if self.columns > 1 => {
                write!(
                    out,
                    "<w:cols w:num=\"{}\" w:space=\"{}\" w:equalWidth=\"0\">",
                    self.columns, self.column_space
                )
                .unwrap();
                for &(w, space) in widths {
                    write!(out, "<w:col w:w=\"{w}\" w:space=\"{space}\"/>").unwrap();
                }
                out.push_str("</w:cols>");
            }
            _ if self.columns > 1 => {
                write!(
                    out,
                    "<w:cols w:num=\"{}\" w:space=\"{}\"/>",
                    self.columns, self.column_space
                )
                .unwrap();
            }
            _ => {}
        }
        if self.vertical_align != 0 {
            let v = match self.vertical_align {
                1 => "center",
                2 => "both",
                3 => "bottom",
                _ => "top",
            };
            write!(out, "<w:vAlign w:val=\"{v}\"/>").unwrap();
        }
        if self.title_page {
            out.push_str("<w:titlePg/>");
        }
        let kind = match self.grid.0 {
            1 => Some("linesAndChars"),
            2 => Some("lines"),
            3 => Some("snapToChars"),
            _ => None,
        };
        if let Some(pitch) = self.grid.1 {
            out.push_str("<w:docGrid");
            if let Some(k) = kind {
                write!(out, " w:type=\"{k}\"").unwrap();
            }
            write!(out, " w:linePitch=\"{pitch}\"").unwrap();
            if let Some(c) = self.grid.2.filter(|_| self.grid.0 == 1 || self.grid.0 == 3) {
                write!(out, " w:charSpace=\"{c}\"").unwrap();
            }
            out.push_str("/>");
        }
        out
    }
}
