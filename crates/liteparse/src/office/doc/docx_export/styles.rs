//! STSH decoding into styles.xml. Each style keeps its base chain through w:basedOn,
//! so style UPX modifiers translate directly as differences from the base.
use super::props::{CharFlags, ParaFlags, ppr, rpr};
use super::source::{Source, u16_le};
use std::fmt::Write;

pub struct Style {
    pub name: String,
    /// 1 paragraph, 2 character, 3 table, 4 numbering.
    pub kind: u8,
    pub base: Option<u16>,
    pub next: u16,
    pub papx: Vec<u8>,
    pub chpx: Vec<u8>,
}

pub struct Styles {
    pub styles: Vec<Option<Style>>,
    /// Default font indexes from Stshi.rgftcStandardChpStsh (ascii, east asian, other).
    pub default_fonts: [u16; 3],
}

impl Styles {
    pub fn read(source: &Source) -> Styles {
        let mut out = Styles {
            styles: Vec::new(),
            default_fonts: [0, 0, 0],
        };
        let Some(stsh) = source.pair(super::source::pair::STSHF) else {
            return out;
        };
        let Some(cb_stshi) = u16_le(stsh, 0) else {
            return out;
        };
        let stshi = &stsh[2..(2 + cb_stshi as usize).min(stsh.len())];
        let count = u16_le(stshi, 0).unwrap_or(0) as usize;
        let base_len = u16_le(stshi, 2).unwrap_or(10) as usize;
        for (i, f) in out.default_fonts.iter_mut().enumerate() {
            *f = u16_le(stshi, 12 + i * 2).unwrap_or(0);
        }
        let mut at = 2 + cb_stshi as usize;
        for _ in 0..count {
            let Some(len) = u16_le(stsh, at) else { break };
            let len = len as usize;
            let std = stsh.get(at + 2..at + 2 + len).unwrap_or(&[]);
            at += 2 + len;
            out.styles.push(if len == 0 {
                None
            } else {
                decode_std(std, base_len)
            });
        }
        out
    }

    pub fn get(&self, istd: u16) -> Option<&Style> {
        self.styles.get(istd as usize)?.as_ref()
    }

    /// The frame modifiers `istd` and its basedOn chain set, root style first, as raw
    /// sprms. A paragraph's w:framePr replaces the style's whole frame, so a paragraph
    /// that adjusts only part of it carries these ahead of its own modifiers.
    pub fn frame_sprms(&self, istd: u16) -> Vec<u8> {
        let mut chain = Vec::new();
        let mut cur = Some(istd);
        while let Some(i) = cur {
            if chain.contains(&i) || chain.len() > 16 {
                break;
            }
            chain.push(i);
            cur = self.get(i).and_then(|s| s.base);
        }
        let mut out = Vec::new();
        for i in chain.into_iter().rev() {
            let Some(s) = self.get(i) else { continue };
            for (sprm, op) in super::source::sprms(&s.papx) {
                if super::props::is_frame_sprm(sprm) {
                    out.extend_from_slice(&sprm.to_le_bytes());
                    out.extend_from_slice(op);
                }
            }
        }
        out
    }

    /// Paragraph flags a style contributes (table depth, list membership).
    pub fn para_flags(&self, istd: u16) -> ParaFlags {
        let mut chain = Vec::new();
        let mut cur = Some(istd);
        while let Some(i) = cur {
            if chain.contains(&i) || chain.len() > 16 {
                break;
            }
            chain.push(i);
            cur = self.get(i).and_then(|s| s.base);
        }
        let mut flags = ParaFlags::default();
        for i in chain.into_iter().rev() {
            if let Some(s) = self.get(i) {
                ppr(&s.papx, &mut flags);
            }
        }
        flags
    }

    /// Character toggles (bold, italic, ...) that are on for `istd` after its basedOn
    /// chain, applying 0, 1, 0x80 (same as base) and 0x81 (opposite of base) in order.
    pub fn toggles_on(&self, istd: u16) -> Vec<&'static str> {
        let mut chain = Vec::new();
        let mut cur = Some(istd);
        while let Some(i) = cur {
            if chain.contains(&i) || chain.len() > 16 {
                break;
            }
            chain.push(i);
            cur = self.get(i).and_then(|s| s.base);
        }
        let mut on: Vec<&'static str> = Vec::new();
        for i in chain.into_iter().rev() {
            let Some(style) = self.get(i) else { continue };
            for (sprm, op) in super::source::sprms(&style.chpx) {
                let Some(name) = super::props::toggle_name(sprm) else {
                    continue;
                };
                let base = on.contains(&name);
                match op
                    .first()
                    .and_then(|&v| super::props::toggle_value(v, base))
                {
                    Some(true) if !base => on.push(name),
                    Some(false) => on.retain(|&n| n != name),
                    _ => {}
                }
            }
        }
        on
    }

    /// `lists` is the number of list overrides (valid numIds are 1..=lists).
    pub fn xml(&self, fonts: &[String], lists: usize) -> String {
        let font = |i: u16| fonts.get(i as usize).cloned().unwrap_or_default();
        let mut out = String::from(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
             <w:styles xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">",
        );
        write!(
            out,
            "<w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii=\"{a}\" w:hAnsi=\"{o}\" w:eastAsia=\"{e}\" w:cs=\"{o}\"/>\
             <w:sz w:val=\"20\"/><w:szCs w:val=\"20\"/></w:rPr></w:rPrDefault>\
             <w:pPrDefault><w:pPr><w:spacing w:after=\"0\" w:line=\"240\" w:lineRule=\"auto\"/></w:pPr></w:pPrDefault></w:docDefaults>",
            a = super::props::esc(&font(self.default_fonts[0])),
            e = super::props::esc(&font(self.default_fonts[1])),
            o = super::props::esc(&font(self.default_fonts[2])),
        )
        .unwrap();
        for (i, style) in self.styles.iter().enumerate() {
            let Some(style) = style else { continue };
            let kind = match style.kind {
                1 => "paragraph",
                2 => "character",
                3 => "table",
                4 => "numbering",
                _ => continue,
            };
            let default = matches!((i, style.kind), (0, 1) | (10, 2) | (11, 3));
            write!(
                out,
                "<w:style w:type=\"{kind}\" w:styleId=\"s{i}\"{}><w:name w:val=\"{}\"/>",
                if default { " w:default=\"1\"" } else { "" },
                super::props::esc(&style.name)
            )
            .unwrap();
            if let Some(b) = style.base.filter(|&b| self.get(b).is_some()) {
                write!(out, "<w:basedOn w:val=\"s{b}\"/>").unwrap();
            }
            if style.kind == 1 && self.get(style.next).is_some() {
                write!(out, "<w:next w:val=\"s{}\"/>", style.next).unwrap();
            }
            if style.kind == 1 || style.kind == 3 {
                let mut flags = ParaFlags::default();
                let p = ppr(&style.papx, &mut flags);
                let num = match flags.ilfo {
                    Some(ilfo) if ilfo > 0 && (ilfo as usize) <= lists => format!(
                        "<w:numPr><w:ilvl w:val=\"{}\"/><w:numId w:val=\"{ilfo}\"/></w:numPr>",
                        flags.ilvl.unwrap_or(0).min(8)
                    ),
                    Some(0) => "<w:numPr><w:numId w:val=\"0\"/></w:numPr>".into(),
                    _ => String::new(),
                };
                if !p.is_empty() || !num.is_empty() {
                    write!(out, "<w:pPr>{num}{p}</w:pPr>").unwrap();
                }
            }
            // A style's 0x81 toggles are relative to its base style.
            let mut flags = CharFlags {
                style_on: style.base.map_or(Vec::new(), |b| self.toggles_on(b)),
                ..CharFlags::default()
            };
            let r = rpr(&style.chpx, fonts, &mut flags);
            if !r.is_empty() {
                write!(out, "<w:rPr>{r}</w:rPr>").unwrap();
            }
            out.push_str("</w:style>");
        }
        out.push_str("</w:styles>");
        out
    }
}

/// STD: Stdf (base 10 bytes, optional StdfPost2000), xstzName, then UPXs on even offsets.
fn decode_std(std: &[u8], base_len: usize) -> Option<Style> {
    let w1 = u16_le(std, 2)?;
    let w2 = u16_le(std, 4)?;
    let kind = (w1 & 0xf) as u8;
    let base = w1 >> 4;
    let cupx = (w2 & 0xf) as usize;
    let next = w2 >> 4;
    let mut at = base_len;
    let name_len = u16_le(std, at)? as usize;
    let name: Vec<u16> = (0..name_len)
        .filter_map(|k| u16_le(std, at + 2 + k * 2))
        .collect();
    at += 2 + name_len * 2 + 2;
    let mut upx = Vec::new();
    for _ in 0..cupx {
        if at % 2 == 1 {
            at += 1;
        }
        let Some(cb) = u16_le(std, at) else { break };
        let body = std
            .get(at + 2..at + 2 + cb as usize)
            .unwrap_or(&[])
            .to_vec();
        at += 2 + cb as usize;
        upx.push(body);
    }
    let (papx, chpx) = match kind {
        1 | 3 => (
            upx.first().and_then(|p| p.get(2..)).unwrap_or(&[]).to_vec(),
            upx.get(1).cloned().unwrap_or_default(),
        ),
        2 => (Vec::new(), upx.first().cloned().unwrap_or_default()),
        _ => (Vec::new(), Vec::new()),
    };
    Some(Style {
        name: String::from_utf16_lossy(&name),
        kind,
        base: (base != 0xfff).then_some(base),
        next,
        papx,
        chpx,
    })
}
