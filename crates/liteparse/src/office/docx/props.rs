//! Raw WordprocessingML properties (`w:rPr`, `w:pPr`) as optional fields, read
//! from XML and merged level by level. Values are converted to points on read:
//! twips (1/20 pt) for lengths, half-points for font sizes, eighths of a point
//! for border widths.

use crate::office::pptx::xml::XNode;

pub fn twips(v: f64) -> f64 {
    v / 20.0
}

/// A twips attribute in points.
pub fn tw_attr(n: &XNode, key: &str) -> Option<f64> {
    n.attr_f64(key).map(twips)
}

/// `w:val` of a child as an OOXML boolean; a bare element means true.
pub fn on_off(n: &XNode) -> bool {
    n.attr("val")
        .map(|v| crate::office::pptx::xml::parse_bool(v).unwrap_or(true))
        .unwrap_or(true)
}

/// A font slot: a family name or a theme font reference (`minorHAnsi`...).
#[derive(Debug, Clone, PartialEq)]
pub enum FontRef {
    Name(String),
    Theme(String),
}

/// Cached RGB and a theme reference belong to the same source color element.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RunColor {
    pub cached: Option<String>,
    pub theme: Option<String>,
    pub tint: Option<u8>,
    pub shade: Option<u8>,
    pub malformed_transform: bool,
}

impl RunColor {
    fn parse(n: &XNode, cached_attr: &str) -> Self {
        Self::parse_attributes(n, cached_attr, "themeColor", "themeTint", "themeShade")
    }

    fn parse_attributes(
        n: &XNode,
        cached_attr: &str,
        theme: &str,
        tint: &str,
        shade: &str,
    ) -> Self {
        let byte = |key| {
            n.attr(key)
                .filter(|v| v.len() == 2)
                .and_then(|v| u8::from_str_radix(v, 16).ok())
        };
        Self {
            cached: n.attr(cached_attr).map(str::to_string),
            theme: n.attr(theme).map(str::to_string),
            tint: byte(tint),
            shade: byte(shade),
            malformed_transform: n
                .attr(tint)
                .map(|_| byte(tint).is_none())
                .unwrap_or_else(|| n.attr(shade).is_some() && byte(shade).is_none()),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RPr {
    pub ascii: Option<FontRef>,
    pub h_ansi: Option<FontRef>,
    pub east_asia: Option<FontRef>,
    pub cs: Option<FontRef>,
    pub hint: Option<String>,
    pub b: Option<bool>,
    pub i: Option<bool>,
    pub b_cs: Option<bool>,
    pub i_cs: Option<bool>,
    pub caps: Option<bool>,
    pub small_caps: Option<bool>,
    pub strike: Option<bool>,
    pub dstrike: Option<bool>,
    pub vanish: Option<bool>,
    /// Font size in points.
    pub sz: Option<f64>,
    pub sz_cs: Option<f64>,
    /// Run color is one property across the style cascade.
    pub color: Option<RunColor>,
    pub underline: Option<String>,
    pub underline_color: Option<RunColor>,
    pub highlight: Option<String>,
    pub shading: Option<Shading>,
    /// `w:bdr`: a border around the run.
    pub bdr: Option<Border>,
    pub vert_align: Option<String>,
    /// Baseline raise in points (`w:position`, half-points in XML).
    pub position: Option<f64>,
    /// Extra letter spacing in points (`w:spacing`, twips).
    pub spacing: Option<f64>,
    /// Run character-grid exemption, inherited independently of paragraph spacing.
    pub snap_to_grid: Option<bool>,
    /// Horizontal glyph scale in percent (`w:w`).
    pub scale: Option<f64>,
    pub kern: Option<f64>,
    /// `w14:ligatures` includes standard ligatures.
    pub ligatures: Option<bool>,
    /// Extra line height in points from a `w14:shadow` text effect.
    pub shadow: Option<f64>,
    pub rtl: Option<bool>,
    pub complex: Option<bool>,
    pub lang_ea: Option<String>,
    /// `w:lang/@w:val`: language of Latin text.
    pub lang: Option<String>,
    /// `w:fitText`: width in points and group id; runs sharing the id fit together.
    pub fit_text: Option<(f64, Option<i64>)>,
    pub r_style: Option<String>,
}

/// The toggle properties: in styles they flip rather than set (ECMA-376 17.7.3).
pub const TOGGLES: usize = 8;

impl RPr {
    pub fn toggles_mut(&mut self) -> [&mut Option<bool>; TOGGLES] {
        [
            &mut self.b,
            &mut self.i,
            &mut self.b_cs,
            &mut self.i_cs,
            &mut self.caps,
            &mut self.small_caps,
            &mut self.strike,
            &mut self.dstrike,
        ]
    }

    pub fn toggles(&self) -> [Option<bool>; TOGGLES] {
        [
            self.b,
            self.i,
            self.b_cs,
            self.i_cs,
            self.caps,
            self.small_caps,
            self.strike,
            self.dstrike,
        ]
    }

    pub fn parse(n: &XNode) -> RPr {
        let mut p = RPr::default();
        for c in &n.children {
            match c.name.as_str() {
                "w:rStyle" => p.r_style = c.attr("val").map(str::to_string),
                "w:rFonts" => {
                    let slot = |name: &str, theme: &str| {
                        c.attr(theme)
                            .map(|t| FontRef::Theme(t.to_string()))
                            .or_else(|| c.attr(name).map(|v| FontRef::Name(v.to_string())))
                    };
                    p.ascii = slot("ascii", "asciiTheme");
                    p.h_ansi = slot("hAnsi", "hAnsiTheme");
                    p.east_asia = slot("eastAsia", "eastAsiaTheme");
                    p.cs = slot("cs", "cstheme");
                    p.hint = c.attr("hint").map(str::to_string);
                }
                "w:b" => p.b = Some(on_off(c)),
                "w:i" => p.i = Some(on_off(c)),
                "w:bCs" => p.b_cs = Some(on_off(c)),
                "w:iCs" => p.i_cs = Some(on_off(c)),
                "w:caps" => p.caps = Some(on_off(c)),
                "w:smallCaps" => p.small_caps = Some(on_off(c)),
                "w:strike" => p.strike = Some(on_off(c)),
                "w:dstrike" => p.dstrike = Some(on_off(c)),
                "w:vanish" => p.vanish = Some(on_off(c)),
                "w:webHidden" => {}
                "w:sz" => p.sz = c.attr_f64("val").map(|v| v / 2.0),
                "w:szCs" => p.sz_cs = c.attr_f64("val").map(|v| v / 2.0),
                "w:color" => p.color = Some(RunColor::parse(c, "val")),
                "w:u" => {
                    p.underline = c.attr("val").map(str::to_string);
                    // Pattern and color inherit independently when their attributes are absent.
                    if ["color", "themeColor"]
                        .iter()
                        .any(|key| c.attr(key).is_some())
                    {
                        p.underline_color = Some(RunColor::parse(c, "color"));
                    }
                }
                "w:highlight" => p.highlight = c.attr("val").map(str::to_string),
                "w:shd" => p.shading = Shading::parse(c),
                "w:bdr" => p.bdr = Border::parse(c).filter(|b| b.visible()),
                "w:vertAlign" => p.vert_align = c.attr("val").map(str::to_string),
                "w:position" => p.position = c.attr_f64("val").map(|v| v / 2.0),
                "w:spacing" => p.spacing = tw_attr(c, "val"),
                "w:snapToGrid" => p.snap_to_grid = Some(on_off(c)),
                "w:w" => p.scale = c.attr_f64("val"),
                "w:kern" => p.kern = c.attr_f64("val").map(|v| v / 2.0),
                "w14:ligatures" => {
                    p.ligatures = c
                        .attr("val")
                        .map(|v| v.starts_with("standard") || v == "all")
                }
                // Word for Mac probe (12 pt Arial): a shadow adds about 0.2 x its
                // distance plus 0.44 x its blur radius to the line (3 pt / 4 pt: +2.3).
                "w14:shadow" => {
                    let pt = |k: &str| c.attr_f64(k).unwrap_or(0.0).max(0.0) / 12700.0;
                    p.shadow = Some(0.2 * pt("dist") + 0.44 * pt("blurRad"));
                }
                "w:fitText" => p.fit_text = c.attr_f64("val").map(|v| (v / 20.0, c.attr_i64("id"))),
                "w:rtl" => p.rtl = Some(on_off(c)),
                "w:cs" => p.complex = Some(on_off(c)),
                "w:lang" => {
                    p.lang_ea = c.attr("eastAsia").map(str::to_string);
                    p.lang = c.attr("val").map(str::to_string);
                }
                _ => {}
            }
        }
        p
    }

    /// Overlays `o` on `self`: every property `o` sets wins. A font slot is
    /// replaced as a whole (name or theme reference).
    pub fn apply(&mut self, o: &RPr) {
        macro_rules! take {
            ($($f:ident),*) => { $( if o.$f.is_some() { self.$f = o.$f.clone(); } )* };
        }
        take!(
            ascii,
            h_ansi,
            east_asia,
            cs,
            hint,
            b,
            i,
            b_cs,
            i_cs,
            caps,
            small_caps,
            strike,
            dstrike,
            vanish,
            sz,
            sz_cs,
            color,
            underline,
            underline_color,
            highlight,
            shading,
            bdr,
            vert_align,
            position,
            spacing,
            snap_to_grid,
            scale,
            kern,
            ligatures,
            shadow,
            rtl,
            complex,
            lang_ea,
            lang,
            fit_text,
            r_style
        );
    }
}

/// One source shading element, retained intact through the style cascade.
#[derive(Debug, Clone, PartialEq)]
pub struct Shading {
    pub pattern: String,
    pub foreground: RunColor,
    pub background: RunColor,
}

impl Shading {
    pub fn parse(n: &XNode) -> Option<Self> {
        let pattern = n.attr("val").unwrap_or("clear").to_owned();
        // Preserve cached-only nil handling while its table-style semantics remain separate.
        if pattern == "nil" {
            let fill = n
                .attr("fill")
                .filter(|v| v.len() == 6 && !v.eq_ignore_ascii_case("auto"))?;
            return Some(Self {
                pattern,
                foreground: RunColor::default(),
                background: RunColor {
                    cached: Some(fill.to_owned()),
                    ..RunColor::default()
                },
            });
        }
        let mut foreground = RunColor::parse(n, "color");
        let mut background =
            RunColor::parse_attributes(n, "fill", "themeFill", "themeFillTint", "themeFillShade");
        for color in [&mut foreground, &mut background] {
            color.cached = color
                .cached
                .take()
                .filter(|v| v.len() == 6 && v.bytes().all(|b| b.is_ascii_hexdigit()));
        }
        Some(Self {
            pattern,
            foreground,
            background,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineRule {
    Auto,
    Exact,
    AtLeast,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabKind {
    Left,
    Center,
    Right,
    Decimal,
    Bar,
    Clear,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TabStop {
    pub pos: f64,
    pub kind: TabKind,
    pub leader: Option<char>,
}

/// One border line: width and spacing in points, color `RRGGBB` or `auto`.
#[derive(Debug, Clone, PartialEq)]
pub struct Border {
    pub style: String,
    pub width: f64,
    pub space: f64,
    pub color: String,
    /// Source attributes remain part of border grouping identity after paint resolution.
    pub source_color: Option<RunColor>,
}

impl Border {
    pub fn parse(n: &XNode) -> Option<Border> {
        let style = n.attr("val").unwrap_or("single").to_string();
        Some(Border {
            width: n.attr_f64("sz").map(|v| v / 8.0).unwrap_or(0.5),
            space: n.attr_f64("space").unwrap_or(0.0),
            color: n.attr("color").unwrap_or("auto").to_string(),
            source_color: Some(RunColor::parse(n, "color")),
            style,
        })
    }

    /// False for `none`/`nil` borders, which draw nothing and take no room.
    pub fn visible(&self) -> bool {
        !matches!(self.style.as_str(), "none" | "nil")
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Borders {
    pub top: Option<Border>,
    pub left: Option<Border>,
    pub bottom: Option<Border>,
    pub right: Option<Border>,
    pub between: Option<Border>,
    pub inside_h: Option<Border>,
    pub inside_v: Option<Border>,
}

impl Borders {
    pub fn parse(n: &XNode) -> Borders {
        let get = |names: &[&str]| {
            names
                .iter()
                .find_map(|name| n.child(&format!("w:{name}")))
                .and_then(|c| Border::parse(c))
        };
        Borders {
            top: get(&["top"]),
            left: get(&["left", "start"]),
            bottom: get(&["bottom"]),
            right: get(&["right", "end"]),
            between: get(&["between"]),
            inside_h: get(&["insideH"]),
            inside_v: get(&["insideV"]),
        }
    }

    pub fn apply(&mut self, o: &Borders) {
        for (mine, theirs) in [
            (&mut self.top, &o.top),
            (&mut self.left, &o.left),
            (&mut self.bottom, &o.bottom),
            (&mut self.right, &o.right),
            (&mut self.between, &o.between),
            (&mut self.inside_h, &o.inside_h),
            (&mut self.inside_v, &o.inside_v),
        ] {
            if theirs.is_some() {
                *mine = theirs.clone();
            }
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PPr {
    pub style: Option<String>,
    pub jc: Option<String>,
    pub ind_left: Option<f64>,
    pub ind_right: Option<f64>,
    /// First-line indent; negative for a hanging indent.
    pub ind_first: Option<f64>,
    pub before: Option<f64>,
    pub after: Option<f64>,
    pub before_auto: Option<bool>,
    pub after_auto: Option<bool>,
    /// `beforeLines`/`afterLines`, in hundredths of a line.
    pub before_lines: Option<f64>,
    pub after_lines: Option<f64>,
    /// `w:line`: 240ths of a line for `auto`, twips otherwise (kept raw).
    pub line: Option<f64>,
    pub line_rule: Option<LineRule>,
    pub contextual: Option<bool>,
    pub keep_next: Option<bool>,
    pub keep_lines: Option<bool>,
    pub suppress_line_numbers: Option<bool>,
    pub page_break_before: Option<bool>,
    pub widow: Option<bool>,
    /// Tab definitions in application order (clears included).
    pub tabs: Vec<TabStop>,
    pub num_id: Option<i64>,
    pub ilvl: Option<i64>,
    pub outline: Option<i64>,
    pub borders: Borders,
    pub shading: Option<Shading>,
    pub snap_to_grid: Option<bool>,
    pub bidi: Option<bool>,
    pub suppress_auto_hyphens: Option<bool>,
    /// `w:autoSpaceDE` / `w:autoSpaceDN`: extra space between East Asian text and Latin letters / digits.
    pub auto_space_de: Option<bool>,
    pub auto_space_dn: Option<bool>,
    /// `w:framePr`: the paragraph sits in a text frame.
    pub frame: Option<Frame>,
    /// Attribute-complete frame inheritance for the scoped page-frame experiment.
    pub resolved_frame: Option<Frame>,
    /// Run properties of the paragraph mark.
    pub mark: Option<RPr>,
}

/// Drop-cap placement relative to the following paragraph's text margin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropCap {
    Drop,
    Margin,
    Unsupported,
}

/// A text frame (`w:framePr`). Lengths in points; `x`/`y` are an offset or
/// an alignment keyword (`xAlign`/`yAlign`).
#[derive(Debug, Clone)]
pub struct Frame {
    pub w: Option<f64>,
    pub h: Option<f64>,
    pub h_exact: bool,
    pub x: Result<f64, String>,
    pub y: Result<f64, String>,
    pub h_anchor: String,
    pub v_anchor: String,
    pub wrap: String,
    pub h_space: f64,
    pub v_space: f64,
    pub drop_cap: Option<DropCap>,
    pub drop_lines: usize,
    /// The element's attributes, for merging over a style's frame.
    pub attrs: Vec<(String, String)>,
}

/// Frames compare by their resolved values; the raw attributes kept for
/// merging do not count.
impl PartialEq for Frame {
    fn eq(&self, o: &Frame) -> bool {
        self.w == o.w
            && self.h == o.h
            && self.h_exact == o.h_exact
            && self.x == o.x
            && self.y == o.y
            && self.h_anchor == o.h_anchor
            && self.v_anchor == o.v_anchor
            && self.wrap == o.wrap
            && self.h_space == o.h_space
            && self.v_space == o.v_space
            && self.drop_cap == o.drop_cap
            && (self.drop_cap.is_none() || self.drop_lines == o.drop_lines)
    }
}

impl Frame {
    /// This frame's attributes over `base`'s: attributes it leaves out come
    /// from `base`.
    pub fn over(&self, base: &Frame) -> Frame {
        let mut attrs = base.attrs.clone();
        for (k, v) in &self.attrs {
            attrs.retain(|(bk, _)| bk != k);
            attrs.push((k.clone(), v.clone()));
        }
        Frame::parse(&XNode {
            name: "w:framePr".into(),
            attrs,
            children: Vec::new(),
            text: String::new(),
        })
    }

    pub fn parse(c: &XNode) -> Frame {
        let pos = |off: &str, al: &str| match c.attr(al) {
            Some(a) => Err(a.to_string()),
            None => Ok(tw_attr(c, off).unwrap_or(0.0)),
        };
        Frame {
            w: tw_attr(c, "w").filter(|v| *v > 0.0),
            h: tw_attr(c, "h").filter(|v| *v > 0.0),
            h_exact: c.attr("hRule") == Some("exact"),
            x: pos("x", "xAlign"),
            y: pos("y", "yAlign"),
            // Word defaults both anchors to text (MS-OE376 2.1.48).
            h_anchor: c.attr("hAnchor").unwrap_or("text").to_string(),
            v_anchor: c.attr("vAnchor").unwrap_or("text").to_string(),
            wrap: c.attr("wrap").unwrap_or("around").to_string(),
            h_space: tw_attr(c, "hSpace").unwrap_or(0.0),
            v_space: tw_attr(c, "vSpace").unwrap_or(0.0),
            drop_cap: match c.attr("dropCap") {
                None | Some("none") => None,
                Some("drop") => Some(DropCap::Drop),
                Some("margin") => Some(DropCap::Margin),
                Some(_) => Some(DropCap::Unsupported),
            },
            drop_lines: c.attr("lines").and_then(|v| v.parse().ok()).unwrap_or(1),
            attrs: c.attrs.clone(),
        }
    }
}

impl PPr {
    pub fn parse(n: &XNode) -> PPr {
        let mut p = PPr::default();
        for c in &n.children {
            match c.name.as_str() {
                "w:pStyle" => p.style = c.attr("val").map(str::to_string),
                "w:jc" => p.jc = c.attr("val").map(str::to_string),
                "w:ind" => {
                    p.ind_left = tw_attr(c, "left").or_else(|| tw_attr(c, "start"));
                    p.ind_right = tw_attr(c, "right").or_else(|| tw_attr(c, "end"));
                    if let Some(h) = tw_attr(c, "hanging") {
                        p.ind_first = Some(-h);
                    } else if let Some(f) = tw_attr(c, "firstLine") {
                        p.ind_first = Some(f);
                    }
                }
                "w:spacing" => {
                    p.before = tw_attr(c, "before");
                    p.after = tw_attr(c, "after");
                    p.before_auto = c.attr_bool("beforeAutospacing");
                    p.after_auto = c.attr_bool("afterAutospacing");
                    p.before_lines = c.attr_f64("beforeLines");
                    p.after_lines = c.attr_f64("afterLines");
                    p.line = c.attr_f64("line");
                    p.line_rule = c.attr("lineRule").map(|r| match r {
                        "exact" => LineRule::Exact,
                        "atLeast" => LineRule::AtLeast,
                        _ => LineRule::Auto,
                    });
                    if p.line.is_some() && p.line_rule.is_none() {
                        p.line_rule = Some(LineRule::Auto);
                    }
                }
                "w:contextualSpacing" => p.contextual = Some(on_off(c)),
                "w:keepNext" => p.keep_next = Some(on_off(c)),
                "w:keepLines" => p.keep_lines = Some(on_off(c)),
                "w:suppressLineNumbers" => p.suppress_line_numbers = Some(on_off(c)),
                "w:pageBreakBefore" => p.page_break_before = Some(on_off(c)),
                "w:widowControl" => p.widow = Some(on_off(c)),
                "w:tabs" => {
                    for t in c.children_named("w:tab") {
                        let Some(pos) = tw_attr(t, "pos") else {
                            continue;
                        };
                        let kind = match t.attr("val").unwrap_or("left") {
                            "center" => TabKind::Center,
                            "right" | "end" => TabKind::Right,
                            "decimal" => TabKind::Decimal,
                            "bar" => TabKind::Bar,
                            "clear" => TabKind::Clear,
                            _ => TabKind::Left,
                        };
                        let leader = match t.attr("leader") {
                            Some("dot") => Some('.'),
                            Some("hyphen") => Some('-'),
                            Some("underscore") | Some("heavy") => Some('_'),
                            Some("middleDot") => Some('\u{b7}'),
                            _ => None,
                        };
                        p.tabs.push(TabStop { pos, kind, leader });
                    }
                }
                "w:numPr" => {
                    p.num_id = c.child("w:numId").and_then(|n| n.attr_i64("val"));
                    p.ilvl = c.child("w:ilvl").and_then(|n| n.attr_i64("val"));
                }
                "w:outlineLvl" => p.outline = c.attr_i64("val"),
                "w:pBdr" => p.borders = Borders::parse(c),
                "w:shd" => p.shading = Shading::parse(c),
                "w:snapToGrid" => p.snap_to_grid = Some(on_off(c)),
                "w:bidi" => p.bidi = Some(on_off(c)),
                "w:suppressAutoHyphens" => p.suppress_auto_hyphens = Some(on_off(c)),
                "w:autoSpaceDE" => p.auto_space_de = Some(on_off(c)),
                "w:autoSpaceDN" => p.auto_space_dn = Some(on_off(c)),
                "w:framePr" => {
                    p.frame = Some(Frame::parse(c));
                    p.resolved_frame = p.frame.clone();
                }
                "w:rPr" => p.mark = Some(RPr::parse(c)),
                _ => {}
            }
        }
        p
    }

    /// Overlays `o` on `self`. Tabs accumulate; everything else is replaced
    /// where `o` sets it. The paragraph mark properties are not merged here.
    pub fn apply(&mut self, o: &PPr) {
        macro_rules! take {
            ($($f:ident),*) => { $( if o.$f.is_some() { self.$f = o.$f.clone(); } )* };
        }
        take!(
            style,
            jc,
            ind_left,
            ind_right,
            ind_first,
            before,
            after,
            before_auto,
            after_auto,
            before_lines,
            after_lines,
            line,
            line_rule,
            contextual,
            keep_next,
            keep_lines,
            suppress_line_numbers,
            page_break_before,
            widow,
            num_id,
            ilvl,
            outline,
            shading,
            snap_to_grid,
            bidi,
            suppress_auto_hyphens,
            auto_space_de,
            auto_space_dn
        );
        self.resolved_frame = match (
            self.resolved_frame.as_ref().or(self.frame.as_ref()),
            o.resolved_frame.as_ref().or(o.frame.as_ref()),
        ) {
            (Some(base), Some(f)) => Some(f.over(base)),
            (_, Some(f)) => Some(f.clone()),
            (base, None) => base.cloned(),
        };
        self.frame = match (&self.frame, &o.frame) {
            (_, Some(f)) => Some(f.clone()),
            (base, None) => base.clone(),
        };
        self.tabs.extend(o.tabs.iter().copied());
        self.borders.apply(&o.borders);
    }

    /// The effective tab stops: later definitions replace earlier ones at the
    /// same position, and `clear` removes them. Sorted by position.
    pub fn tab_stops(&self) -> Vec<TabStop> {
        let mut out: Vec<TabStop> = Vec::new();
        for t in &self.tabs {
            out.retain(|o| (o.pos - t.pos).abs() > 0.01);
            if t.kind != TabKind::Clear {
                out.push(*t);
            }
        }
        out.sort_by(|a, b| a.pos.total_cmp(&b.pos));
        out
    }
}

#[cfg(test)]
mod tests;
