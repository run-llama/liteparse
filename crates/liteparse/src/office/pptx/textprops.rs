//! Text property inheritance.
//!
//! A paragraph's effective properties fold a chain of list styles from general to
//! specific: presentation `defaultTextStyle` → master `txStyles` (title/body/other)
//! → master placeholder `lstStyle` → layout placeholder `lstStyle` → the shape's own
//! `lstStyle` → the paragraph's `a:pPr`. Run properties fold each level's `defRPr`
//! the same way, then the run's `a:rPr`. Each layer only overrides what it sets.

use std::rc::Rc;

use crate::office::pptx::xml::XNode;

pub const EMU_PER_PT: f64 = 12_700.0;

#[derive(Debug, Clone, Default)]
pub struct RunProps {
    /// Points.
    pub size: Option<f64>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<String>,
    pub strike: Option<String>,
    /// Fraction of the font size (0.3 = superscript 30%).
    pub baseline: Option<f64>,
    pub cap: Option<String>,
    /// Extra spacing after each character, points.
    pub spacing: Option<f64>,
    pub latin: Option<String>,
    /// PANOSE of the latin typeface (20 hex digits), used to pick a fallback
    /// weight and serif style when the font is not installed.
    pub latin_panose: Option<String>,
    /// Whether the latin / cs typeface is declared with the Thai character set
    /// (`charset="-34"`, 222), which picks PowerPoint's Thai fallback when it is missing.
    pub latin_thai: Option<bool>,
    pub ea: Option<String>,
    pub cs: Option<String>,
    pub cs_thai: Option<bool>,
    pub sym: Option<String>,
    /// Fill element (`a:solidFill`, `a:noFill`, `a:gradFill`...).
    pub fill: Option<Rc<XNode>>,
    pub highlight: Option<Rc<XNode>>,
    pub hlink_rid: Option<String>,
    /// `a:effectLst` of the run (text shadows).
    pub effects: Option<Rc<XNode>>,
    /// `a:ln`: the glyph outline (stroked around each glyph when it has a fill).
    pub outline: Option<Rc<XNode>>,
    /// Underline fill (the fill inside `a:uFill`) and line (`a:uLn`); `None`
    /// follows the text's own fill and outline (`a:uFillTx`, `a:uLnTx`).
    pub u_fill: Option<Rc<XNode>>,
    pub u_ln: Option<Rc<XNode>>,
    /// Kerning applies at and above this size (points); `None` = no kerning.
    pub kern: Option<f64>,
}

impl RunProps {
    pub fn merge(&mut self, rpr: &XNode) {
        if let Some(v) = rpr.attr_f64("sz") {
            self.size = Some(v / 100.0);
        }
        if let Some(v) = rpr.attr_bool("b") {
            self.bold = Some(v);
        }
        if let Some(v) = rpr.attr_bool("i") {
            self.italic = Some(v);
        }
        if let Some(v) = rpr.attr("u") {
            self.underline = Some(v.to_string());
        }
        if let Some(v) = rpr.attr("strike") {
            self.strike = Some(v.to_string());
        }
        if let Some(v) = rpr.attr_i64("baseline") {
            self.baseline = Some(v as f64 / 100_000.0);
        }
        if let Some(v) = rpr.attr("cap") {
            self.cap = Some(v.to_string());
        }
        if let Some(v) = rpr.attr_f64("kern") {
            self.kern = Some(v / 100.0);
        }
        if let Some(v) = rpr.attr_f64("spc") {
            self.spacing = Some(v / 100.0);
        }
        for c in &rpr.children {
            match c.name.as_str() {
                "a:latin" => {
                    self.latin = c.attr("typeface").map(str::to_string);
                    self.latin_panose = c.attr("panose").map(str::to_string);
                    self.latin_thai = Some(thai_charset(c));
                }
                "a:ea" => self.ea = c.attr("typeface").map(str::to_string),
                "a:cs" => {
                    self.cs = c.attr("typeface").map(str::to_string);
                    self.cs_thai = Some(thai_charset(c));
                }
                "a:sym" => self.sym = c.attr("typeface").map(str::to_string),
                "a:noFill" | "a:solidFill" | "a:gradFill" | "a:pattFill" | "a:blipFill" => {
                    self.fill = Some(c.clone())
                }
                "a:highlight" => self.highlight = Some(c.clone()),
                "a:effectLst" => self.effects = Some(c.clone()),
                "a:ln" => self.outline = Some(c.clone()),
                "a:uFill" => self.u_fill = c.children.first().cloned(),
                "a:uFillTx" => self.u_fill = None,
                "a:uLn" => self.u_ln = Some(c.clone()),
                "a:uLnTx" => self.u_ln = None,
                "a:hlinkClick" => {
                    self.hlink_rid = c.attr("r:id").filter(|s| !s.is_empty()).map(str::to_string)
                }
                _ => {}
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Spacing {
    /// Fraction of single line spacing.
    Pct(f64),
    Pts(f64),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Bullet {
    None,
    Char(String),
    AutoNum { scheme: String, start: u32 },
    Blip(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum BuSize {
    FollowText,
    Pct(f64),
    Pts(f64),
}

#[derive(Debug, Clone, Default)]
pub struct ParaProps {
    pub align: Option<String>,
    pub mar_l: Option<f64>,
    pub indent: Option<f64>,
    pub def_tab: Option<f64>,
    pub rtl: Option<bool>,
    pub line_spacing: Option<Spacing>,
    pub space_before: Option<Spacing>,
    pub space_after: Option<Spacing>,
    pub bullet: Option<Bullet>,
    /// `None` inside = follow text font.
    pub bu_font: Option<Option<String>>,
    pub bu_color: Option<Option<Rc<XNode>>>,
    pub bu_size: Option<BuSize>,
    pub tabs: Option<Vec<(f64, String)>>,
    pub def_run: RunProps,
}

fn read_spacing(node: &XNode) -> Option<Spacing> {
    if let Some(p) = node.child("a:spcPct") {
        return p
            .attr("val")
            .and_then(crate::office::pptx::xml::parse_i64)
            .map(|v| Spacing::Pct(v as f64 / 100_000.0));
    }
    if let Some(p) = node.child("a:spcPts") {
        return p.attr_f64("val").map(|v| Spacing::Pts(v / 100.0));
    }
    None
}

impl ParaProps {
    pub fn merge(&mut self, ppr: &XNode) {
        if let Some(v) = ppr.attr("algn") {
            self.align = Some(v.to_string());
        }
        if let Some(v) = ppr.attr_f64("marL") {
            self.mar_l = Some(v / EMU_PER_PT);
        }
        if let Some(v) = ppr.attr_f64("indent") {
            self.indent = Some(v / EMU_PER_PT);
        }
        if let Some(v) = ppr.attr_f64("defTabSz") {
            self.def_tab = Some(v / EMU_PER_PT);
        }
        if let Some(v) = ppr.attr_bool("rtl") {
            self.rtl = Some(v);
        }
        for c in &ppr.children {
            match c.name.as_str() {
                "a:lnSpc" => self.line_spacing = read_spacing(c).or(self.line_spacing),
                "a:spcBef" => self.space_before = read_spacing(c).or(self.space_before),
                "a:spcAft" => self.space_after = read_spacing(c).or(self.space_after),
                "a:buNone" => self.bullet = Some(Bullet::None),
                "a:buChar" => {
                    self.bullet = Some(Bullet::Char(c.attr("char").unwrap_or("•").to_string()))
                }
                "a:buAutoNum" => {
                    self.bullet = Some(Bullet::AutoNum {
                        scheme: c.attr("type").unwrap_or("arabicPeriod").to_string(),
                        start: c.attr_i64("startAt").unwrap_or(1).max(1) as u32,
                    })
                }
                "a:buBlip" => {
                    if let Some(rid) = c.child("a:blip").and_then(|b| b.attr("r:embed")) {
                        self.bullet = Some(Bullet::Blip(rid.to_string()));
                    }
                }
                "a:buFontTx" => self.bu_font = Some(None),
                "a:buFont" => self.bu_font = Some(c.attr("typeface").map(str::to_string)),
                "a:buClrTx" => self.bu_color = Some(None),
                "a:buClr" => self.bu_color = Some(Some(c.clone())),
                "a:buSzTx" => self.bu_size = Some(BuSize::FollowText),
                "a:buSzPct" => {
                    if let Some(v) = c.attr("val").and_then(crate::office::pptx::xml::parse_i64) {
                        self.bu_size = Some(BuSize::Pct(v as f64 / 100_000.0));
                    }
                }
                "a:buSzPts" => {
                    if let Some(v) = c.attr_f64("val") {
                        self.bu_size = Some(BuSize::Pts(v / 100.0));
                    }
                }
                "a:tabLst" => {
                    let tabs = c
                        .children_named("a:tab")
                        .filter_map(|t| {
                            Some((
                                t.attr_f64("pos")? / EMU_PER_PT,
                                t.attr("algn").unwrap_or("l").to_string(),
                            ))
                        })
                        .collect();
                    self.tabs = Some(tabs);
                }
                "a:defRPr" => self.def_run.merge(c),
                _ => {}
            }
        }
    }
}

/// A list style layer: `a:lstStyle`, `p:titleStyle`, `p:bodyStyle`, `p:otherStyle`,
/// or `p:defaultTextStyle`. All share the `a:defPPr`/`a:lvlNpPr` children.
/// The `a:solidFill` XML for a concrete color, alpha included.
pub fn solid_fill_xml(c: crate::office::pptx::color::Rgba) -> String {
    let h = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!(
        r#"<a:solidFill><a:srgbClr val="{:02X}{:02X}{:02X}"><a:alpha val="{}"/></a:srgbClr></a:solidFill>"#,
        h(c.r),
        h(c.g),
        h(c.b),
        (c.a.clamp(0.0, 1.0) * 100000.0).round() as i64
    )
}

/// A synthetic `a:lstStyle` whose nine levels all carry `def_rpr_inner` in `a:defRPr`
/// (with `def_rpr_attrs` on the element), for slotting into a list-style chain.
pub fn uniform_list_style(def_rpr_attrs: &str, def_rpr_inner: &str) -> Option<Rc<XNode>> {
    let levels: String = (1..=9)
        .map(|l| {
            format!(
                "<a:lvl{l}pPr><a:defRPr{def_rpr_attrs}>{def_rpr_inner}</a:defRPr></a:lvl{l}pPr>"
            )
        })
        .collect();
    crate::office::pptx::xml::parse(&format!(
        r#"<a:lstStyle xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">{levels}</a:lstStyle>"#
    ))
    .ok()
}

pub fn level_ppr(list: &XNode, level: u32) -> Vec<&Rc<XNode>> {
    let name = format!("a:lvl{}pPr", level.min(8) + 1);
    [list.child("a:defPPr"), list.child(&name)]
        .into_iter()
        .flatten()
        .collect()
}

/// Folds the chain (general → specific) for one paragraph level.
pub fn resolve_para(chain: &[Rc<XNode>], level: u32, own: Option<&XNode>) -> ParaProps {
    let mut p = ParaProps::default();
    for list in chain {
        for ppr in level_ppr(list, level) {
            p.merge(ppr);
        }
    }
    if let Some(own) = own {
        p.merge(own);
    }
    p
}

#[derive(Debug, Clone, Default)]
pub struct BodyProps {
    pub l_ins: Option<f64>,
    pub t_ins: Option<f64>,
    pub r_ins: Option<f64>,
    pub b_ins: Option<f64>,
    pub anchor: Option<String>,
    pub anchor_ctr: Option<bool>,
    /// `spcFirstLastPara`: the first paragraph's space before and the last one's
    /// space after count too.
    pub spc_first_last: Option<bool>,
    pub wrap: Option<String>,
    pub vert: Option<String>,
    pub rot: Option<f64>,
    pub upright: Option<bool>,
    pub num_col: Option<u32>,
    pub spc_col: Option<f64>,
    pub font_scale: Option<f64>,
    pub ln_spc_reduction: Option<f64>,
    pub autofit: Option<String>,
    /// WordArt text warp preset (`a:prstTxWarp prst`), when not `textNoShape`.
    pub warp: Option<String>,
    /// `fromWordArt="1"`: legacy WordArt, whose plain warp stretches text to the box.
    pub from_word_art: Option<bool>,
    /// The warp's adjust values (`a:avLst/a:gd`) as (name, formula).
    pub warp_adj: Vec<(String, String)>,
}

impl BodyProps {
    pub fn merge(&mut self, b: &XNode) {
        if let Some(v) = b.attr_bool("fromWordArt") {
            self.from_word_art = Some(v);
        }
        if let Some(w) = b.child("a:prstTxWarp") {
            self.warp = w
                .attr("prst")
                .filter(|p| *p != "textNoShape")
                .map(str::to_string);
            self.warp_adj = w
                .child("a:avLst")
                .map(|l| {
                    l.children_named("a:gd")
                        .filter_map(|g| Some((g.attr("name")?.into(), g.attr("fmla")?.into())))
                        .collect()
                })
                .unwrap_or_default();
        }
        let ins = |k: &str| b.attr_f64(k).map(|v| v / EMU_PER_PT);
        if let Some(v) = ins("lIns") {
            self.l_ins = Some(v);
        }
        if let Some(v) = ins("tIns") {
            self.t_ins = Some(v);
        }
        if let Some(v) = ins("rIns") {
            self.r_ins = Some(v);
        }
        if let Some(v) = ins("bIns") {
            self.b_ins = Some(v);
        }
        if let Some(v) = b.attr("anchor") {
            self.anchor = Some(v.to_string());
        }
        if let Some(v) = b.attr_bool("spcFirstLastPara") {
            self.spc_first_last = Some(v);
        }
        if let Some(v) = b.attr_bool("anchorCtr") {
            self.anchor_ctr = Some(v);
        }
        if let Some(v) = b.attr("wrap") {
            self.wrap = Some(v.to_string());
        }
        if let Some(v) = b.attr("vert") {
            self.vert = Some(v.to_string());
        }
        if let Some(v) = b.attr_f64("rot") {
            self.rot = Some(v / 60_000.0);
        }
        if let Some(v) = b.attr_bool("upright") {
            self.upright = Some(v);
        }
        if let Some(v) = b.attr_i64("numCol") {
            self.num_col = Some(v.max(1) as u32);
        }
        if let Some(v) = ins("spcCol") {
            self.spc_col = Some(v);
        }
        if let Some(n) = b.child("a:normAutofit") {
            self.autofit = Some("norm".into());
            self.font_scale = n
                .attr("fontScale")
                .and_then(crate::office::pptx::xml::parse_i64)
                .map(|v| v as f64 / 100_000.0);
            self.ln_spc_reduction = n
                .attr("lnSpcReduction")
                .and_then(crate::office::pptx::xml::parse_i64)
                .map(|v| v as f64 / 100_000.0);
        } else if b.child("a:spAutoFit").is_some() {
            self.autofit = Some("shape".into());
            self.font_scale = None;
            self.ln_spc_reduction = None;
        } else if b.child("a:noAutofit").is_some() {
            self.autofit = Some("none".into());
            self.font_scale = None;
            self.ln_spc_reduction = None;
        }
    }
}

#[cfg(test)]
mod tests;

/// Whether a typeface element declares the Thai character set (222, written signed).
fn thai_charset(face: &XNode) -> bool {
    matches!(face.attr("charset"), Some("-34" | "222"))
}
