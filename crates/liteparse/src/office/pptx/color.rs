//! DrawingML color resolution: base colors (`srgbClr`, `schemeClr`, `sysClr`,
//! `prstClr`, `hslClr`, `scrgbClr`) plus the transform children (`lumMod`, `tint`,
//! `alpha`, ...).
//!
//! Transform math follows the conventions Office uses and LibreOffice reproduces
//! (oox `Color::getColor`): `tint`/`shade` and the channel modulators work on
//! linear RGB, `lum*`/`sat*`/`hue*` on HSL of the gamma-encoded color.

use std::collections::HashMap;

use crate::office::pptx::xml::XNode;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Rgba {
    pub const BLACK: Rgba = Rgba {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 1.0,
    };
    pub const WHITE: Rgba = Rgba {
        r: 1.0,
        g: 1.0,
        b: 1.0,
        a: 1.0,
    };

    pub fn from_hex(hex: &str) -> Option<Rgba> {
        let h = hex.trim().trim_start_matches('#');
        if h.len() != 6 {
            return None;
        }
        let v = u32::from_str_radix(h, 16).ok()?;
        Some(Rgba {
            r: ((v >> 16) & 0xFF) as f32 / 255.0,
            g: ((v >> 8) & 0xFF) as f32 / 255.0,
            b: (v & 0xFF) as f32 / 255.0,
            a: 1.0,
        })
    }

    /// ARGB hex, the format of the extract protocol's color fields.
    pub fn to_argb_hex(&self) -> String {
        let c = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        format!(
            "{:02X}{:02X}{:02X}{:02X}",
            c(self.a),
            c(self.r),
            c(self.g),
            c(self.b)
        )
    }

    pub fn to_skia(&self) -> tiny_skia::Color {
        tiny_skia::Color::from_rgba(
            self.r.clamp(0.0, 1.0),
            self.g.clamp(0.0, 1.0),
            self.b.clamp(0.0, 1.0),
            self.a.clamp(0.0, 1.0),
        )
        .unwrap_or(tiny_skia::Color::BLACK)
    }
}

/// The theme color scheme plus the master/slide `clrMap`, i.e. everything needed
/// to turn a `schemeClr` into a concrete color.
#[derive(Debug, Clone, Default)]
pub struct ColorScheme {
    /// Theme slots: dk1, lt1, dk2, lt2, accent1..6, hlink, folHlink.
    pub slots: HashMap<String, Rgba>,
    /// Logical name → slot (`bg1` → `lt1`, `tx1` → `dk1`, ...).
    pub map: HashMap<String, String>,
}

impl ColorScheme {
    pub fn default_map() -> HashMap<String, String> {
        [
            ("bg1", "lt1"),
            ("tx1", "dk1"),
            ("bg2", "lt2"),
            ("tx2", "dk2"),
            ("accent1", "accent1"),
            ("accent2", "accent2"),
            ("accent3", "accent3"),
            ("accent4", "accent4"),
            ("accent5", "accent5"),
            ("accent6", "accent6"),
            ("hlink", "hlink"),
            ("folHlink", "folHlink"),
        ]
        .into_iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect()
    }

    /// Applies a `p:clrMap` or `p:clrMapOvr/a:overrideClrMapping` element.
    pub fn with_map(&self, clr_map: Option<&XNode>) -> ColorScheme {
        let mut out = self.clone();
        if let Some(m) = clr_map {
            for (k, v) in &m.attrs {
                out.map.insert(k.clone(), v.clone());
            }
        }
        out
    }

    pub fn scheme(&self, name: &str) -> Option<Rgba> {
        // Slot names can be referenced directly (`dk1`) or through the map (`tx1`).
        let slot = self.map.get(name).map(String::as_str).unwrap_or(name);
        self.slots
            .get(slot)
            .copied()
            .or_else(|| self.slots.get(name).copied())
    }
}

/// Resolves a color *container* (an element whose child is a color, e.g.
/// `a:solidFill`, `a:buClr`, `a:fontRef`) or a color element itself.
pub fn resolve(node: &XNode, scheme: &ColorScheme, ph_clr: Option<Rgba>) -> Option<Rgba> {
    if is_color_element(&node.name) {
        return resolve_color_element(node, scheme, ph_clr);
    }
    node.children
        .iter()
        .find(|c| is_color_element(&c.name))
        .and_then(|c| resolve_color_element(c, scheme, ph_clr))
}

fn is_color_element(name: &str) -> bool {
    matches!(
        name,
        "a:srgbClr" | "a:schemeClr" | "a:sysClr" | "a:prstClr" | "a:hslClr" | "a:scrgbClr"
    )
}

fn pct(node: &XNode) -> f32 {
    node.attr("val")
        .and_then(crate::office::pptx::xml::parse_i64)
        .unwrap_or(0) as f32
        / 100_000.0
}

pub fn resolve_color_element(
    node: &XNode,
    scheme: &ColorScheme,
    ph_clr: Option<Rgba>,
) -> Option<Rgba> {
    let base =
        match node.name.as_str() {
            "a:srgbClr" => Rgba::from_hex(node.attr("val")?)?,
            "a:sysClr" => node.attr("lastClr").and_then(Rgba::from_hex).or_else(|| {
                match node.attr("val") {
                    Some("window") => Some(Rgba::WHITE),
                    Some("windowText") => Some(Rgba::BLACK),
                    _ => None,
                }
            })?,
            "a:schemeClr" => {
                let val = node.attr("val")?;
                if val == "phClr" {
                    ph_clr.unwrap_or(Rgba::BLACK)
                } else {
                    scheme.scheme(val)?
                }
            }
            "a:prstClr" => preset_color(node.attr("val")?)?,
            "a:scrgbClr" => {
                let ch = |k: &str| linear_to_srgb(node.attr_i64(k).unwrap_or(0) as f32 / 100_000.0);
                Rgba {
                    r: ch("r"),
                    g: ch("g"),
                    b: ch("b"),
                    a: 1.0,
                }
            }
            "a:hslClr" => {
                let h = node.attr_i64("hue").unwrap_or(0) as f32 / 60_000.0;
                let s = node.attr_i64("sat").unwrap_or(0) as f32 / 100_000.0;
                let l = node.attr_i64("lum").unwrap_or(0) as f32 / 100_000.0;
                let (r, g, b) = hsl_to_rgb(h, s, l);
                Rgba { r, g, b, a: 1.0 }
            }
            _ => return None,
        };
    Some(apply_transforms(base, &node.children))
}

pub fn apply_transforms(mut c: Rgba, transforms: &[std::rc::Rc<XNode>]) -> Rgba {
    for t in transforms {
        let v = pct(t);
        match t.name.as_str() {
            "a:alpha" => c.a = v.clamp(0.0, 1.0),
            "a:alphaMod" => c.a = (c.a * v).clamp(0.0, 1.0),
            "a:alphaOff" => c.a = (c.a + v).clamp(0.0, 1.0),
            "a:tint" => {
                let f =
                    |x: f32| linear_to_srgb(1.0 - (1.0 - srgb_to_linear(x)) * v.clamp(0.0, 1.0));
                c = Rgba {
                    r: f(c.r),
                    g: f(c.g),
                    b: f(c.b),
                    a: c.a,
                };
            }
            "a:shade" => {
                let f = |x: f32| linear_to_srgb(srgb_to_linear(x) * v.clamp(0.0, 1.0));
                c = Rgba {
                    r: f(c.r),
                    g: f(c.g),
                    b: f(c.b),
                    a: c.a,
                };
            }
            "a:red" | "a:redMod" | "a:redOff" | "a:green" | "a:greenMod" | "a:greenOff"
            | "a:blue" | "a:blueMod" | "a:blueOff" => {
                let local = t.local();
                let ch = match &local[..1] {
                    "r" => &mut c.r,
                    "g" => &mut c.g,
                    _ => &mut c.b,
                };
                let lin = srgb_to_linear(*ch);
                let out = if local.ends_with("Mod") {
                    lin * v
                } else if local.ends_with("Off") {
                    lin + v
                } else {
                    v
                };
                *ch = linear_to_srgb(out.clamp(0.0, 1.0));
            }
            "a:lumMod" | "a:lumOff" | "a:lum" | "a:satMod" | "a:satOff" | "a:sat" | "a:hueMod"
            | "a:hueOff" | "a:hue" => {
                let (mut h, mut s, mut l) = rgb_to_hsl(c.r, c.g, c.b);
                match t.local() {
                    "lumMod" => l *= v,
                    "lumOff" => l += v,
                    "lum" => l = v,
                    "satMod" => s *= v,
                    "satOff" => s += v,
                    "sat" => s = v,
                    // Hue values are in 60000ths of a degree; h is in sextants.
                    "hueMod" => h *= v,
                    "hueOff" => h += t.attr_i64("val").unwrap_or(0) as f32 / 60_000.0 / 60.0,
                    "hue" => h = t.attr_i64("val").unwrap_or(0) as f32 / 60_000.0 / 60.0,
                    _ => {}
                }
                let (r, g, b) = hsl_to_rgb(
                    h.rem_euclid(6.0) * 60.0,
                    s.clamp(0.0, 1.0),
                    l.clamp(0.0, 1.0),
                );
                c = Rgba { r, g, b, a: c.a };
            }
            "a:comp" => {
                let (h, s, l) = rgb_to_hsl(c.r, c.g, c.b);
                let (r, g, b) = hsl_to_rgb(((h + 3.0) % 6.0) * 60.0, s, l);
                c = Rgba { r, g, b, a: c.a };
            }
            "a:inv" => {
                c = Rgba {
                    r: 1.0 - c.r,
                    g: 1.0 - c.g,
                    b: 1.0 - c.b,
                    a: c.a,
                }
            }
            "a:gray" => {
                let y = 0.299 * c.r + 0.587 * c.g + 0.114 * c.b;
                c = Rgba {
                    r: y,
                    g: y,
                    b: y,
                    a: c.a,
                };
            }
            _ => {}
        }
    }
    c
}

pub fn srgb_to_linear(x: f32) -> f32 {
    if x <= 0.04045 {
        x / 12.92
    } else {
        ((x + 0.055) / 1.055).powf(2.4)
    }
}

pub fn linear_to_srgb(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    if x <= 0.003_130_8 {
        x * 12.92
    } else {
        1.055 * x.powf(1.0 / 2.4) - 0.055
    }
}

/// Returns hue in sextants (0..6), saturation and lightness in 0..1.
fn rgb_to_hsl(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    let d = max - min;
    if d <= f32::EPSILON {
        return (0.0, 0.0, l);
    }
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };
    let h = if max == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h, s, l)
}

/// Hue in degrees.
fn hsl_to_rgb(h_deg: f32, s: f32, l: f32) -> (f32, f32, f32) {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = (h_deg.rem_euclid(360.0)) / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r1, g1, b1) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    (r1 + m, g1 + m, b1 + m)
}

fn preset_color(name: &str) -> Option<Rgba> {
    let hex = match name {
        "black" => "000000",
        "white" => "FFFFFF",
        "red" => "FF0000",
        "green" => "008000",
        "lime" => "00FF00",
        "blue" => "0000FF",
        "yellow" => "FFFF00",
        "cyan" | "aqua" => "00FFFF",
        "magenta" | "fuchsia" => "FF00FF",
        "gray" | "grey" => "808080",
        "dkGray" | "darkGray" => "A9A9A9",
        "ltGray" | "lightGray" => "D3D3D3",
        "silver" => "C0C0C0",
        "maroon" => "800000",
        "navy" => "000080",
        "olive" => "808000",
        "purple" => "800080",
        "teal" => "008080",
        "orange" => "FFA500",
        "pink" => "FFC0CB",
        "brown" => "A52A2A",
        "gold" => "FFD700",
        "dkBlue" | "darkBlue" => "00008B",
        "dkRed" | "darkRed" => "8B0000",
        "dkGreen" | "darkGreen" => "006400",
        "ltBlue" | "lightBlue" => "ADD8E6",
        "ltGreen" | "lightGreen" => "90EE90",
        "ltYellow" | "lightYellow" => "FFFFE0",
        "violet" => "EE82EE",
        "indigo" => "4B0082",
        "coral" => "FF7F50",
        "salmon" => "FA8072",
        "tan" => "D2B48C",
        "beige" => "F5F5DC",
        "khaki" => "F0E68C",
        "crimson" => "DC143C",
        "skyBlue" => "87CEEB",
        "steelBlue" => "4682B4",
        "royalBlue" => "4169E1",
        "slateGray" => "708090",
        "dimGray" => "696969",
        "whiteSmoke" => "F5F5F5",
        "ivory" => "FFFFF0",
        "lavender" => "E6E6FA",
        "turquoise" => "40E0D0",
        "orchid" => "DA70D6",
        "plum" => "DDA0DD",
        "chocolate" => "D2691E",
        "firebrick" => "B22222",
        "forestGreen" => "228B22",
        "seaGreen" => "2E8B57",
        "tomato" => "FF6347",
        _ => return None,
    };
    Rgba::from_hex(hex)
}

#[cfg(test)]
mod tests;
