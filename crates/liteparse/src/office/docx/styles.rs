//! `word/styles.xml`: document defaults and the style cascade
//! (docDefaults → table style → paragraph style → character style → direct),
//! resolved into the concrete paragraph and run styles layout uses.

use std::collections::HashMap;
use std::rc::Rc;

use crate::office::pptx::color::Rgba;
use crate::office::pptx::xml::XNode;

use crate::office::docx::props::{Borders, FontRef, LineRule, PPr, RPr, TOGGLES, TabStop};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StyleKind {
    Paragraph,
    Character,
    Table,
    Numbering,
}

/// Conditional formatting of a table style region (`w:tblStylePr`).
#[derive(Debug, Clone, Default)]
pub struct CondFormat {
    pub ppr: PPr,
    pub rpr: RPr,
    pub tcpr: Option<Rc<XNode>>,
}

#[derive(Debug, Clone)]
pub struct Style {
    pub id: String,
    pub kind: StyleKind,
    pub based_on: Option<String>,
    pub ppr: PPr,
    pub rpr: RPr,
    pub tblpr: Option<Rc<XNode>>,
    pub trpr: Option<Rc<XNode>>,
    pub tcpr: Option<Rc<XNode>>,
    pub conditional: HashMap<String, CondFormat>,
    pub is_default: bool,
}

/// Theme fonts: (latin, east asian, complex) for major and minor.
#[derive(Debug, Clone, Default)]
pub struct ThemeFonts {
    pub major: (String, String, String),
    pub minor: (String, String, String),
}

#[derive(Debug, Default)]
pub struct StyleSheet {
    pub styles: HashMap<String, Style>,
    pub default_rpr: RPr,
    pub default_ppr: PPr,
    pub default_para: Option<String>,
    pub default_char: Option<String>,
    pub default_table: Option<String>,
    /// `w:compatibilityMode` from the settings part (12 when absent).
    pub compat_mode: u32,
    pub theme: ThemeFonts,
    pub(crate) theme_colors: crate::office::docx::theme_colors::ThemeColors,
}

impl StyleSheet {
    pub fn parse(root: Option<&XNode>, theme: ThemeFonts) -> StyleSheet {
        let mut sheet = StyleSheet {
            theme,
            ..Default::default()
        };
        let Some(root) = root else {
            return sheet;
        };
        if let Some(dd) = root.child("w:docDefaults") {
            if let Some(r) = dd.path(&["w:rPrDefault", "w:rPr"]) {
                sheet.default_rpr = RPr::parse(r);
            }
            if let Some(p) = dd.path(&["w:pPrDefault", "w:pPr"]) {
                sheet.default_ppr = PPr::parse(p);
            }
        }
        // Document defaults without a w:pPrDefault: Word applies its own, 8 pt after and
        // 1.15 lines; an empty one means none (Word for Mac probes on an 18 pt grid: lines
        // step 20.9 pt inside a paragraph and 28.8 pt between them; 18 pt once written out).
        if root
            .child("w:docDefaults")
            .is_some_and(|dd| dd.child("w:pPrDefault").is_none())
        {
            sheet.default_ppr.after = Some(8.0);
            sheet.default_ppr.line = Some(276.0);
            sheet.default_ppr.line_rule = Some(LineRule::Auto);
        }
        for s in root.children_named("w:style") {
            // A style without an id is referenced by its name (Word for Mac applies a
            // report generator's id-less 1 pt empty-cell style to its table rows).
            let Some(id) = s
                .attr("styleId")
                .or_else(|| s.child("w:name").and_then(|n| n.attr("val")))
            else {
                continue;
            };
            let kind = match s.attr("type").unwrap_or("paragraph") {
                "character" => StyleKind::Character,
                "table" => StyleKind::Table,
                "numbering" => StyleKind::Numbering,
                _ => StyleKind::Paragraph,
            };
            let is_default = s.attr_bool("default").unwrap_or(false);
            let mut conditional = HashMap::new();
            for c in s.children_named("w:tblStylePr") {
                if let Some(t) = c.attr("type") {
                    conditional.insert(
                        t.to_string(),
                        CondFormat {
                            ppr: c.child("w:pPr").map(|n| PPr::parse(n)).unwrap_or_default(),
                            rpr: c.child("w:rPr").map(|n| RPr::parse(n)).unwrap_or_default(),
                            tcpr: c.child("w:tcPr").cloned(),
                        },
                    );
                }
            }
            let style = Style {
                id: id.to_string(),
                kind,
                based_on: s
                    .child("w:basedOn")
                    .and_then(|b| b.attr("val"))
                    .map(str::to_string),
                ppr: s.child("w:pPr").map(|n| PPr::parse(n)).unwrap_or_default(),
                rpr: s.child("w:rPr").map(|n| RPr::parse(n)).unwrap_or_default(),
                tblpr: s.child("w:tblPr").cloned(),
                trpr: s.child("w:trPr").cloned(),
                tcpr: s.child("w:tcPr").cloned(),
                conditional,
                is_default,
            };
            if is_default {
                let slot = match kind {
                    StyleKind::Paragraph => &mut sheet.default_para,
                    StyleKind::Character => &mut sheet.default_char,
                    StyleKind::Table => &mut sheet.default_table,
                    StyleKind::Numbering => continue,
                };
                if slot.is_none() {
                    *slot = Some(id.to_string());
                }
            }
            sheet.styles.entry(id.to_string()).or_insert(style);
        }
        sheet
    }

    /// The style chain from the root ancestor down to `id` (cycles cut).
    pub fn chain(&self, id: &str) -> Vec<&Style> {
        let mut out = Vec::new();
        let mut cur = self.styles.get(id);
        while let Some(s) = cur {
            if out.iter().any(|o: &&Style| o.id == s.id) || out.len() > 32 {
                break;
            }
            out.push(s);
            cur = s.based_on.as_deref().and_then(|b| self.styles.get(b));
        }
        out.reverse();
        out
    }

    /// Paragraph properties of a style with its ancestors merged.
    pub fn style_ppr(&self, id: &str) -> PPr {
        let mut p = PPr::default();
        for s in self.chain(id) {
            p.apply(&s.ppr);
        }
        p
    }

    pub fn style_rpr(&self, id: &str) -> RPr {
        let mut r = RPr::default();
        for s in self.chain(id) {
            r.apply(&s.rpr);
        }
        r
    }

    /// Word for Mac: a table style's paragraph and run properties sit between the
    /// document defaults and the paragraph style chain, Normal included. Below
    /// compatibility mode 15, a 12 pt font size inherited from the default
    /// paragraph style yields to the table style's size (probe renders).
    fn table_size_wins(&self, para_style: Option<&str>) -> bool {
        if self.compat_mode >= 15 {
            return false;
        }
        let Some(id) = para_style else { return false };
        let sized = self
            .chain(id)
            .into_iter()
            .rev()
            .find(|s| s.rpr.sz.is_some());
        sized.is_some_and(|s| {
            Some(s.id.as_str()) == self.default_para.as_deref() && s.rpr.sz == Some(12.0)
        })
    }

    /// Paragraph style id to use for a paragraph (its own, else the default).
    pub fn para_style_id<'a>(&'a self, own: Option<&'a str>) -> Option<&'a str> {
        own.filter(|s| self.styles.contains_key(*s))
            .or(self.default_para.as_deref())
    }

    /// Resolves the effective paragraph properties: defaults, table style,
    /// paragraph style chain, then direct properties.
    pub fn resolve_ppr(&self, direct: &PPr, table: Option<&PPr>) -> PPr {
        let mut p = self.default_ppr.clone();
        let id = self.para_style_id(direct.style.as_deref());
        if let Some(t) = table {
            p.apply(t);
        }
        if let Some(id) = id {
            p.apply(&self.style_ppr(id));
        }
        p.apply(direct);
        p
    }

    /// Effective run properties. Toggle properties set by styles flip each other
    /// across the table, paragraph and character style levels; direct
    /// formatting sets them outright.
    pub fn resolve_rpr(
        &self,
        para_style: Option<&str>,
        direct: &RPr,
        table: Option<&RPr>,
        extra: Option<&RPr>,
    ) -> RPr {
        let mut out = self.default_rpr.clone();
        let mut levels: Vec<RPr> = Vec::new();
        let id = self.para_style_id(para_style);
        if let Some(t) = table {
            levels.push(t.clone());
        }
        if let Some(id) = id {
            let mut chain = self.style_rpr(id);
            if let Some(sz) = table
                .and_then(|t| t.sz)
                .filter(|_| self.table_size_wins(Some(id)))
            {
                chain.sz = Some(sz);
            }
            levels.push(chain);
        }
        let char_style = direct
            .r_style
            .as_deref()
            .or(self.default_char.as_deref())
            .filter(|s| self.styles.contains_key(*s));
        if let Some(id) = char_style {
            levels.push(self.style_rpr(id));
        }
        let mut toggled: [Option<bool>; TOGGLES] = [None; TOGGLES];
        for level in &levels {
            for (k, v) in level.toggles().iter().enumerate() {
                if let Some(v) = v {
                    toggled[k] = Some(toggled[k].unwrap_or(false) ^ v);
                }
            }
            out.apply(level);
        }
        for (slot, t) in out.toggles_mut().into_iter().zip(toggled) {
            if let Some(t) = t {
                *slot = Some(t);
            }
        }
        if let Some(e) = extra {
            out.apply(e);
        }
        out.apply(direct);
        out
    }

    /// A theme font reference (`minorHAnsi`, `majorEastAsia`, ...) as a family.
    pub fn theme_font(&self, r: &str) -> String {
        let set = if r.starts_with("major") {
            &self.theme.major
        } else {
            &self.theme.minor
        };
        let name = if r.ends_with("EastAsia") {
            &set.1
        } else if r.ends_with("Bidi") {
            &set.2
        } else {
            &set.0
        };
        name.clone()
    }

    pub fn font_name(&self, f: &Option<FontRef>) -> Option<String> {
        match f {
            Some(FontRef::Name(n)) => Some(n.clone()),
            Some(FontRef::Theme(t)) => Some(self.theme_font(t)).filter(|s| !s.is_empty()),
            None => None,
        }
    }

    /// Concrete paragraph style with document theme shading.
    pub fn para_style(&self, p: &PPr, line_unit: f64) -> ParaStyle {
        let mut style = ParaStyle::from_ppr(p, line_unit);
        style.borders = self.theme_colors.borders(&p.borders);
        style.shading = p
            .shading
            .as_ref()
            .and_then(|s| self.theme_colors.shading(s));
        style
    }

    /// Concrete run style from merged properties.
    pub fn run_style(&self, r: &RPr) -> RunStyle {
        let ascii = self
            .font_name(&r.ascii)
            .or_else(|| self.font_name(&r.h_ansi))
            .unwrap_or_else(|| "Times New Roman".into());
        let east_asia = self
            .font_name(&r.east_asia)
            .unwrap_or_else(|| ascii.clone());
        let cs = self.font_name(&r.cs).unwrap_or_else(|| ascii.clone());
        let color = r.color.as_ref().and_then(|c| self.theme_colors.resolve(c));
        let rtl = r.rtl;
        let complex = rtl == Some(true);
        RunStyle {
            source_style: None,
            source: None,
            symbol: None,
            rtl,
            font: ascii,
            font_ea: east_asia,
            font_cs: cs,
            hint_ea: r.hint.as_deref() == Some("eastAsia"),
            size: (if complex { r.sz_cs.or(r.sz) } else { r.sz })
                .unwrap_or(10.0)
                .clamp(0.5, 1638.0),
            bold: (if complex { r.b_cs.or(r.b) } else { r.b }).unwrap_or(false),
            italic: (if complex { r.i_cs.or(r.i) } else { r.i }).unwrap_or(false),
            caps: r.caps.unwrap_or(false),
            small_caps: r.small_caps.unwrap_or(false),
            strike: r.strike.unwrap_or(false) || r.dstrike.unwrap_or(false),
            hidden: r.vanish.unwrap_or(false),
            underline: r
                .underline
                .clone()
                .filter(|u| u != "none" && u != "0" && u != "false"),
            underline_color: r
                .underline_color
                .as_ref()
                .and_then(|c| self.theme_colors.resolve(c)),
            color,
            highlight: r.highlight.as_deref().and_then(highlight_color),
            shading: r
                .shading
                .as_ref()
                .and_then(|s| self.theme_colors.shading(s)),
            bdr: r.bdr.as_ref().map(|b| self.theme_colors.border(b)),
            vert_align: match r.vert_align.as_deref() {
                Some("superscript") => VertAlign::Super,
                Some("subscript") => VertAlign::Sub,
                _ => VertAlign::Baseline,
            },
            position: r.position.unwrap_or(0.0),
            spacing: r.spacing.unwrap_or(0.0),
            // Keep run coalescing unchanged while character-grid tracking is off.
            snap_to_grid: r.snap_to_grid.unwrap_or(true),
            scale: r.scale.unwrap_or(100.0).clamp(1.0, 600.0) / 100.0,
            kern: r.kern,
            ligatures: r.ligatures.unwrap_or(false),
            shadow_extra: r.shadow.unwrap_or(0.0),
            lang: r.lang.clone(),
            lang_ea: r.lang_ea.clone(),
            fit_text: r.fit_text,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VertAlign {
    Baseline,
    Super,
    Sub,
}

/// Concrete character formatting of one run.
#[derive(Debug, Clone, PartialEq)]
pub struct RunStyle {
    /// Retained pre-internal-link style identity controls text coalescence.
    pub source_style: Option<std::rc::Rc<RunStyle>>,
    pub source: Option<std::sync::Arc<crate::office::pptx::scene::TextSource>>,
    /// Explicit symbol family and code, separate from accessible text.
    pub symbol: Option<(String, char)>,
    /// Inherited run direction override; retained only by the bidi experiment.
    pub rtl: Option<bool>,
    pub font: String,
    pub font_ea: String,
    pub font_cs: String,
    pub hint_ea: bool,
    pub size: f64,
    pub bold: bool,
    pub italic: bool,
    pub caps: bool,
    pub small_caps: bool,
    pub strike: bool,
    pub hidden: bool,
    pub underline: Option<String>,
    /// `w:u w:color`; the text colour when absent.
    pub underline_color: Option<Rgba>,
    pub color: Option<Rgba>,
    pub highlight: Option<Rgba>,
    pub shading: Option<Rgba>,
    pub bdr: Option<crate::office::docx::props::Border>,
    pub vert_align: VertAlign,
    pub position: f64,
    pub spacing: f64,
    pub snap_to_grid: bool,
    pub scale: f64,
    pub kern: Option<f64>,
    /// Standard ligatures are drawn (`w14:ligatures`).
    pub ligatures: bool,
    /// Extra descent in points that a `w14:shadow` text effect adds to its line.
    pub shadow_extra: f64,
    /// Language tag of Latin text (`w:lang/@w:val`).
    pub lang: Option<String>,
    /// East Asian language tag (`w:lang/@w:eastAsia`).
    pub lang_ea: Option<String>,
    /// `w:fitText` width (points) and group id.
    pub fit_text: Option<(f64, Option<i64>)>,
}

fn highlight_color(name: &str) -> Option<Rgba> {
    let hex = match name {
        "yellow" => "FFFF00",
        "green" => "00FF00",
        "cyan" => "00FFFF",
        "magenta" => "FF00FF",
        "blue" => "0000FF",
        "red" => "FF0000",
        "darkBlue" => "000080",
        "darkCyan" => "008080",
        "darkGreen" => "008000",
        "darkMagenta" => "800080",
        "darkRed" => "800000",
        "darkYellow" => "808000",
        "darkGray" => "808080",
        "lightGray" => "C0C0C0",
        "black" => "000000",
        "white" => "FFFFFF",
        _ => return None,
    };
    Rgba::from_hex(hex)
}

/// Concrete paragraph formatting (points).
#[derive(Debug, Clone, PartialEq)]
pub struct ParaStyle {
    pub style_id: Option<String>,
    pub jc: Jc,
    pub left: f64,
    pub right: f64,
    pub first: f64,
    pub before: f64,
    pub after: f64,
    pub before_auto: bool,
    pub after_auto: bool,
    pub line: Option<(f64, LineRule)>,
    pub contextual: bool,
    pub keep_next: bool,
    pub keep_lines: bool,
    /// `w:suppressLineNumbers`: the paragraph's lines are not numbered.
    pub suppress_line_numbers: bool,
    pub page_break_before: bool,
    /// A `w:br w:clear` in the paragraph: what follows starts below floating objects.
    pub clear_break: bool,
    pub widow: bool,
    pub tabs: Vec<TabStop>,
    pub borders: Borders,
    pub shading: Option<Rgba>,
    pub snap_to_grid: bool,
    pub auto_space_de: bool,
    pub auto_space_dn: bool,
    pub suppress_auto_hyphens: bool,
    pub bidi: bool,
    pub outline: Option<i64>,
    pub frame: Option<crate::office::docx::props::Frame>,
    pub resolved_frame: Option<crate::office::docx::props::Frame>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Jc {
    Left,
    Center,
    Right,
    Both,
    Distribute,
}

impl ParaStyle {
    /// `before_lines`/`after_lines` (hundredths of a line) need the line unit:
    /// 12 pt, or the pitch of a line grid.
    pub fn from_ppr(p: &PPr, line_unit: f64) -> ParaStyle {
        let bidi = p.bidi.unwrap_or(false);
        let jc = match p.jc.as_deref() {
            Some("center") => Jc::Center,
            Some("right") => {
                if bidi {
                    Jc::Left
                } else {
                    Jc::Right
                }
            }
            Some("end") => Jc::Right,
            Some("both")
            | Some("lowKashida")
            | Some("mediumKashida")
            | Some("highKashida")
            | Some("thaiDistribute") => Jc::Both,
            Some("distribute") => Jc::Distribute,
            Some("left") | Some("start") => {
                if bidi {
                    Jc::Right
                } else {
                    Jc::Left
                }
            }
            _ => {
                if bidi {
                    Jc::Right
                } else {
                    Jc::Left
                }
            }
        };
        // Legacy left/right are logical start/end aliases. Resolve physical sides
        // after style inheritance has supplied the paragraph's final direction.
        let (left, right) = if bidi {
            (p.ind_right.unwrap_or(0.0), p.ind_left.unwrap_or(0.0))
        } else {
            (p.ind_left.unwrap_or(0.0), p.ind_right.unwrap_or(0.0))
        };
        let before = match p.before_lines {
            Some(l) if l > 0.0 => l / 100.0 * line_unit,
            _ => p.before.unwrap_or(0.0),
        };
        let after = match p.after_lines {
            Some(l) if l > 0.0 => l / 100.0 * line_unit,
            _ => p.after.unwrap_or(0.0),
        };
        ParaStyle {
            style_id: p.style.clone(),
            jc,
            left,
            right,
            first: p.ind_first.unwrap_or(0.0),
            before,
            after,
            before_auto: p.before_auto.unwrap_or(false),
            after_auto: p.after_auto.unwrap_or(false),
            line: p
                .line
                .map(|l| (l, p.line_rule.unwrap_or(LineRule::Auto)))
                .filter(|(l, r)| *l > 0.0 || *r == LineRule::AtLeast),
            contextual: p.contextual.unwrap_or(false),
            keep_next: p.keep_next.unwrap_or(false),
            keep_lines: p.keep_lines.unwrap_or(false),
            suppress_line_numbers: p.suppress_line_numbers.unwrap_or(false),
            page_break_before: p.page_break_before.unwrap_or(false),
            clear_break: false,
            // Word turns widow control on unless a style or the paragraph turns
            // it off (`w:widowControl w:val="0"`).
            widow: p.widow.unwrap_or(true),
            tabs: p.tab_stops(),
            borders: p.borders.clone(),
            shading: p
                .shading
                .as_ref()
                .and_then(|s| crate::office::docx::theme_colors::ThemeColors::default().shading(s)),
            snap_to_grid: p.snap_to_grid.unwrap_or(true),
            auto_space_de: p.auto_space_de.unwrap_or(true),
            auto_space_dn: p.auto_space_dn.unwrap_or(true),
            suppress_auto_hyphens: p.suppress_auto_hyphens.unwrap_or(false),
            bidi,
            outline: p.outline,
            frame: p.frame.clone(),
            resolved_frame: p.resolved_frame.clone(),
        }
    }
}

/// Reads the theme's major/minor font scheme. An empty east asian slot takes
/// the script font for the document's east asian language, and an empty complex
/// script slot the one for its bidi language (`he` → the `Hebr` font).
pub fn theme_fonts(
    theme: Option<&XNode>,
    lang_ea: Option<&str>,
    lang_bidi: Option<&str>,
) -> ThemeFonts {
    let bidi_script = match lang_bidi.unwrap_or("").split('-').next().unwrap_or("") {
        "he" | "yi" => "Hebr",
        "ar" | "fa" | "ur" | "ps" | "sd" | "ug" | "ku" => "Arab",
        "syr" => "Syrc",
        "dv" => "Thaa",
        _ => "",
    };
    let script = match lang_ea.unwrap_or("") {
        l if l.starts_with("ja") => "Jpan",
        l if l.eq_ignore_ascii_case("zh-TW") || l.eq_ignore_ascii_case("zh-HK") => "Hant",
        l if l.starts_with("zh") => "Hans",
        l if l.starts_with("ko") => "Hang",
        _ => "",
    };
    let read = |n: Option<&std::rc::Rc<XNode>>| -> (String, String, String) {
        let Some(n) = n else {
            return Default::default();
        };
        let face = |name: &str| {
            n.child(name)
                .and_then(|c| c.attr("typeface"))
                .unwrap_or("")
                .to_string()
        };
        let mut ea = face("a:ea");
        if ea.is_empty() && !script.is_empty() {
            ea = n
                .children_named("a:font")
                .find(|f| f.attr("script") == Some(script))
                .and_then(|f| f.attr("typeface"))
                .unwrap_or("")
                .to_string();
        }
        let mut cs = face("a:cs");
        if cs.is_empty() && !bidi_script.is_empty() {
            cs = n
                .children_named("a:font")
                .find(|f| f.attr("script") == Some(bidi_script))
                .and_then(|f| f.attr("typeface"))
                .unwrap_or("")
                .to_string();
        }
        (face("a:latin"), ea, cs)
    };
    let Some(fs) = theme.and_then(|t| t.path(&["a:themeElements", "a:fontScheme"])) else {
        return ThemeFonts::default();
    };
    ThemeFonts {
        major: read(fs.child("a:majorFont")),
        minor: read(fs.child("a:minorFont")),
    }
}

#[cfg(test)]
mod tests;
