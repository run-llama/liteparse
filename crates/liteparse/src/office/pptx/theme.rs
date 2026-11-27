//! Theme part (`ppt/theme/themeN.xml`): color scheme, font scheme and the format
//! scheme's style lists that `p:style` references index into.

use std::collections::HashMap;
use std::rc::Rc;

use crate::office::pptx::color::{ColorScheme, Rgba};
use crate::office::pptx::xml::XNode;

#[derive(Debug, Clone, Default)]
pub struct FontSet {
    pub latin: String,
    pub ea: String,
    pub cs: String,
    /// Per-script overrides (`<a:font script="Jpan" typeface="..."/>`).
    pub scripts: HashMap<String, String>,
}

#[derive(Debug, Clone, Default)]
pub struct Theme {
    pub colors: ColorScheme,
    pub major: FontSet,
    pub minor: FontSet,
    /// `a:fillStyleLst` children; `fillRef idx` 1..=3 index here.
    pub fill_styles: Vec<Rc<XNode>>,
    /// `a:bgFillStyleLst`; `bgRef`/`fillRef idx` 1001.. index here.
    pub bg_fill_styles: Vec<Rc<XNode>>,
    /// `a:lnStyleLst` children (`a:ln`).
    pub line_styles: Vec<Rc<XNode>>,
    pub effect_styles: Vec<Rc<XNode>>,
}

impl Theme {
    pub fn parse(root: &XNode) -> Theme {
        let mut theme = Theme::default();
        let Some(elements) = root.child("a:themeElements") else {
            return theme;
        };
        theme.colors.map = ColorScheme::default_map();
        if let Some(cs) = elements.child("a:clrScheme") {
            for slot in &cs.children {
                if let Some(c) =
                    crate::office::pptx::color::resolve(slot, &ColorScheme::default(), None)
                {
                    theme.colors.slots.insert(slot.local().to_string(), c);
                }
            }
        }
        if let Some(fs) = elements.child("a:fontScheme") {
            theme.major = parse_font_set(fs.child("a:majorFont").map(|n| n.as_ref()));
            theme.minor = parse_font_set(fs.child("a:minorFont").map(|n| n.as_ref()));
        }
        if let Some(fmt) = elements.child("a:fmtScheme") {
            let list = |name: &str| -> Vec<Rc<XNode>> {
                fmt.child(name)
                    .map(|l| l.children.clone())
                    .unwrap_or_default()
            };
            theme.fill_styles = list("a:fillStyleLst");
            theme.bg_fill_styles = list("a:bgFillStyleLst");
            theme.line_styles = list("a:lnStyleLst");
            theme.effect_styles = list("a:effectStyleLst");
        }
        theme
    }

    /// Resolves a theme font reference (`+mj-lt`, `+mn-ea`, ...) to a family name.
    /// Anything that is not a reference is returned unchanged.
    pub fn font_ref<'a>(&'a self, typeface: &'a str) -> &'a str {
        let set = match typeface.get(..4) {
            Some("+mj-") => &self.major,
            Some("+mn-") => &self.minor,
            _ => return typeface,
        };
        match &typeface[4..] {
            "lt" => &set.latin,
            "ea" => &set.ea,
            "cs" => &set.cs,
            _ => &set.latin,
        }
    }

    /// The fill for a style-matrix index (`fillRef`/`bgRef`): 0 is no fill,
    /// 1..999 the fill style list, 1001.. the background fill list.
    pub fn fill_style(&self, idx: i64) -> Option<&Rc<XNode>> {
        if idx >= 1001 {
            self.bg_fill_styles.get((idx - 1001) as usize)
        } else if idx >= 1 {
            self.fill_styles.get((idx - 1) as usize)
        } else {
            None
        }
    }

    pub fn line_style(&self, idx: i64) -> Option<&Rc<XNode>> {
        if idx >= 1 {
            self.line_styles.get((idx - 1) as usize)
        } else {
            None
        }
    }

    /// `tx1` through the effective color map of the part being drawn.
    pub fn default_text_color_mapped(&self, colors: &ColorScheme) -> Rgba {
        colors.scheme("tx1").unwrap_or(Rgba::BLACK)
    }
}

fn parse_font_set(node: Option<&XNode>) -> FontSet {
    let Some(node) = node else {
        return FontSet::default();
    };
    let face = |name: &str| {
        node.child(name)
            .and_then(|n| n.attr("typeface"))
            .unwrap_or("")
            .to_string()
    };
    let scripts = node
        .children_named("a:font")
        .filter_map(|f| {
            Some((
                f.attr("script")?.to_string(),
                f.attr("typeface")?.to_string(),
            ))
        })
        .collect();
    FontSet {
        latin: face("a:latin"),
        ea: face("a:ea"),
        cs: face("a:cs"),
        scripts,
    }
}
