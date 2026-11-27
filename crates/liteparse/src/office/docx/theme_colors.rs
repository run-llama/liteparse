//! Document theme run colors, resolved after Word's style cascade.

use std::collections::HashMap;

use crate::office::docx::props::RunColor;
use crate::office::pptx::{color::Rgba, xml::XNode};

#[derive(Debug)]
enum ThemeColor {
    Rgb(Rgba),
    System { name: String, last: Option<Rgba> },
}

#[derive(Debug, Default)]
pub(crate) struct ThemeColors {
    slots: HashMap<String, ThemeColor>,
    mapping: HashMap<String, String>,
}

impl ThemeColors {
    pub(crate) fn parse(theme: Option<&XNode>, settings: Option<&XNode>) -> Self {
        let mut colors = Self::default();
        if let Some(scheme) = theme.and_then(|t| t.path(&["a:themeElements", "a:clrScheme"])) {
            for slot in &scheme.children {
                let value = if let Some(system) = slot.child("a:sysClr") {
                    system.attr("val").map(|name| ThemeColor::System {
                        name: name.to_owned(),
                        last: system.attr("lastClr").and_then(rgb),
                    })
                } else {
                    slot.child("a:srgbClr")
                        .and_then(|n| n.attr("val"))
                        .and_then(rgb)
                        .map(ThemeColor::Rgb)
                };
                if let Some(value) = value {
                    colors.slots.insert(slot.local().to_owned(), value);
                }
            }
        }
        if let Some(mapping) = settings.and_then(|s| s.child("w:clrSchemeMapping")) {
            for key in [
                "bg1",
                "t1",
                "bg2",
                "t2",
                "accent1",
                "accent2",
                "accent3",
                "accent4",
                "accent5",
                "accent6",
                "hyperlink",
                "followedHyperlink",
            ] {
                if let Some(value) = mapping.attr(key).and_then(slot_name) {
                    colors.mapping.insert(key.to_owned(), value.to_owned());
                }
            }
        }
        colors
    }

    pub(crate) fn border(
        &self,
        source: &crate::office::docx::props::Border,
    ) -> crate::office::docx::props::Border {
        let mut border = source.clone();
        if let Some(color) = border.source_color.as_ref()
            && let Some(resolved) = self.resolve(color)
        {
            border.color = resolved.to_argb_hex()[2..].to_owned();
        }
        border
    }

    pub(crate) fn borders(
        &self,
        source: &crate::office::docx::props::Borders,
    ) -> crate::office::docx::props::Borders {
        let mut borders = source.clone();
        for side in [
            &mut borders.top,
            &mut borders.left,
            &mut borders.bottom,
            &mut borders.right,
            &mut borders.between,
            &mut borders.inside_h,
            &mut borders.inside_v,
        ] {
            if let Some(border) = side.as_mut() {
                *border = self.border(border);
            }
        }
        borders
    }

    pub(crate) fn shading(&self, source: &crate::office::docx::props::Shading) -> Option<Rgba> {
        let background = self.resolve(&source.background);
        // Pattern paint retains the existing averaged-color approximation.
        let share = match source.pattern.as_str() {
            "solid" => 1.0,
            v if v.starts_with("pct") => v[3..].parse::<f64>().unwrap_or(0.0) / 100.0,
            "horzStripe" | "vertStripe" | "diagStripe" | "reverseDiagStripe" | "horzCross"
            | "diagCross" => 0.5,
            v if v.starts_with("thin") => 0.25,
            _ => 0.0,
        };
        if share <= 0.0 {
            return background;
        }
        let foreground = self.resolve(&source.foreground).unwrap_or(Rgba::BLACK);
        let background = background.unwrap_or(Rgba::WHITE);
        let mix = |a: f32, b: f32| {
            ((a as f64 * 255.0).round() * (1.0 - share) + (b as f64 * 255.0).round() * share)
                .round() as u8
        };
        Rgba::from_hex(&format!(
            "{:02X}{:02X}{:02X}",
            mix(background.r, foreground.r),
            mix(background.g, foreground.g),
            mix(background.b, foreground.b)
        ))
    }

    pub(crate) fn resolve(&self, color: &RunColor) -> Option<Rgba> {
        let cached = color.cached.as_deref().and_then(Rgba::from_hex);
        if color.malformed_transform {
            return cached;
        }
        let Some(reference) = color.theme.as_deref() else {
            return cached;
        };
        let key = match reference {
            "text1" => "t1",
            "text2" => "t2",
            "background1" => "bg1",
            "background2" => "bg2",
            other => other,
        };
        let slot = self
            .mapping
            .get(key)
            .map(String::as_str)
            .or_else(|| slot_name(reference));
        let Some(value) = slot.and_then(|s| self.slots.get(s)) else {
            return cached;
        };
        let base = match value {
            ThemeColor::Rgb(rgb) => *rgb,
            // A cached run RGB already includes its source transforms. Known
            // system colors without a cache use the headless page palette.
            ThemeColor::System { name, last } => {
                if cached.is_some() {
                    return cached;
                }
                match name.as_str() {
                    "windowText" => Rgba::BLACK,
                    "window" => Rgba::WHITE,
                    _ => (*last)?,
                }
            }
        };
        Some(luminance(base, color.tint, color.shade))
    }
}

fn rgb(value: &str) -> Option<Rgba> {
    (value.len() == 6 && value.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| Rgba::from_hex(value))
        .flatten()
}

fn slot_name(reference: &str) -> Option<&str> {
    Some(match reference {
        "text1" | "dark1" => "dk1",
        "text2" | "dark2" => "dk2",
        "background1" | "light1" => "lt1",
        "background2" | "light2" => "lt2",
        "hyperlink" => "hlink",
        "followedHyperlink" => "folHlink",
        "accent1" | "accent2" | "accent3" | "accent4" | "accent5" | "accent6" => reference,
        _ => return None,
    })
}

/// Word run tint/shade changes HSL luminance; tint wins when both are present.
/// Scaling chroma around the new luminance preserves the source hue/saturation.
fn luminance(color: Rgba, tint: Option<u8>, shade: Option<u8>) -> Rgba {
    if tint.is_none() && shade.is_none() {
        return color;
    }
    // Scheme channels are source bytes. Recover them before HSL arithmetic so
    // their f32 storage does not move exact integer results across a floor.
    let channels =
        [color.r, color.g, color.b].map(|channel| (f64::from(channel) * 255.0).round() / 255.0);
    let min = channels.iter().copied().fold(f64::INFINITY, f64::min);
    let max = channels.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let light = (min + max) / 2.0;
    let next = if let Some(tint) = tint {
        1.0 - (1.0 - light) * f64::from(tint) / 255.0
    } else {
        light * f64::from(shade.unwrap_or(255)) / 255.0
    };
    let chroma = max - min;
    let new_chroma = if chroma > 0.0 {
        chroma / (1.0 - (2.0 * light - 1.0).abs()) * (1.0 - (2.0 * next - 1.0).abs())
    } else {
        0.0
    };
    let change = |channel: f64| {
        let value = if chroma > 0.0 {
            (channel - min) * new_chroma / chroma + next - new_chroma / 2.0
        } else {
            next
        };
        // The epsilon covers f64 arithmetic at an exact byte boundary.
        ((value.clamp(0.0, 1.0) * 255.0 + 1.0e-9).floor() / 255.0) as f32
    };
    Rgba {
        r: change(channels[0]),
        g: change(channels[1]),
        b: change(channels[2]),
        a: color.a,
    }
}

#[cfg(test)]
mod tests;
