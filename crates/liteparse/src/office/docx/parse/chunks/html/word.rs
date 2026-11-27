//! Projection helpers for the supported computed HTML styles.
use super::style::{Computed, length};
use crate::office::pptx::xml::XNode;
use std::rc::Rc;

pub(super) fn element(name: &str, attrs: &[(&str, String)], children: Vec<Rc<XNode>>) -> Rc<XNode> {
    Rc::new(XNode {
        name: name.into(),
        attrs: attrs
            .iter()
            .map(|(k, v)| ((*k).to_string(), v.clone()))
            .collect(),
        children,
        text: String::new(),
    })
}

pub(super) fn val(name: &str, value: impl ToString) -> Rc<XNode> {
    element(name, &[("val", value.to_string())], Vec::new())
}

pub(super) fn text(value: String, style: &Computed) -> Rc<XNode> {
    let node = Rc::new(XNode {
        name: "w:t".into(),
        attrs: vec![("space".into(), "preserve".into())],
        children: Vec::new(),
        text: value,
    });
    element("w:r", &[], vec![run_properties(style), node])
}

pub(super) fn color(value: &str) -> Option<String> {
    let value = value.trim().to_ascii_lowercase();
    if let Some(hex) = value.strip_prefix('#') {
        if hex.len() == 6 && hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Some(hex.to_ascii_uppercase());
        }
        if hex.len() == 3 && hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Some(
                hex.chars()
                    .flat_map(|c| [c, c])
                    .collect::<String>()
                    .to_ascii_uppercase(),
            );
        }
    }
    if let Some(rgb) = value.strip_prefix("rgb(").and_then(|v| v.strip_suffix(')')) {
        let channels: Option<Vec<u8>> = rgb
            .split(',')
            .map(|v| {
                let v = v.trim();
                let number = if let Some(n) = v.strip_suffix('%') {
                    n.parse::<f64>().ok()? * 2.55
                } else {
                    v.parse::<f64>().ok()?
                };
                number
                    .is_finite()
                    .then_some(number.clamp(0.0, 255.0).round() as u8)
            })
            .collect();
        if let Some(channels) = channels.filter(|v| v.len() == 3) {
            return Some(format!(
                "{:02X}{:02X}{:02X}",
                channels[0], channels[1], channels[2]
            ));
        }
        return None;
    }
    Some(
        match value.as_str() {
            "black" => "000000",
            "white" => "FFFFFF",
            "red" => "FF0000",
            "green" => "008000",
            "blue" => "0000FF",
            "yellow" => "FFFF00",
            "gray" | "grey" => "808080",
            "silver" => "C0C0C0",
            "navy" => "000080",
            "purple" => "800080",
            "teal" => "008080",
            "maroon" => "800000",
            "lime" => "00FF00",
            "aqua" => "00FFFF",
            "fuchsia" => "FF00FF",
            "olive" => "808000",
            _ => return None,
        }
        .into(),
    )
}

pub(super) fn font_size(style: &Computed) -> f64 {
    style
        .get("font-size")
        .and_then(|s| length(s, 12.0))
        .filter(|n| *n > 0.0)
        .unwrap_or(12.0)
}

fn run_properties(style: &Computed) -> Rc<XNode> {
    let mut out = vec![val("w:sz", (font_size(style) * 2.0).round())];
    if let Some(font) = style.get("font-family") {
        let font = font
            .split(',')
            .next()
            .unwrap_or(font)
            .trim()
            .trim_matches(['\'', '"']);
        let font = match font {
            "serif" => "Times New Roman",
            "sans-serif" => "Arial",
            "monospace" => "Courier New",
            _ => font,
        };
        out.push(element(
            "w:rFonts",
            &[
                ("ascii", font.into()),
                ("hAnsi", font.into()),
                ("eastAsia", font.into()),
                ("cs", font.into()),
            ],
            Vec::new(),
        ));
    }
    if let Some(weight) = style.get("font-weight") {
        let bold =
            matches!(weight, "bold" | "bolder") || weight.parse::<u16>().is_ok_and(|n| n >= 600);
        out.push(val("w:b", if bold { "1" } else { "0" }));
    }
    if let Some(value) = style.get("font-style") {
        out.push(val(
            "w:i",
            if matches!(value, "italic" | "oblique") {
                "1"
            } else {
                "0"
            },
        ));
    }
    if let Some(value) = style.get("text-decoration") {
        out.push(val(
            "w:u",
            if value.split_whitespace().any(|v| v == "underline") {
                "single"
            } else {
                "none"
            },
        ));
        out.push(val(
            "w:strike",
            if value.split_whitespace().any(|v| v == "line-through") {
                "1"
            } else {
                "0"
            },
        ));
    }
    if let Some(value) = style.get("color").and_then(color) {
        out.push(val("w:color", value));
    }
    if let Some(value) = style.get("background-color").and_then(color) {
        out.push(element(
            "w:shd",
            &[("val", "clear".into()), ("fill", value)],
            Vec::new(),
        ));
    }
    element("w:rPr", &[], out)
}

pub(super) fn paragraph(style: &Computed, mut runs: Vec<Rc<XNode>>) -> Rc<XNode> {
    let size = font_size(style);
    let mut props = Vec::new();
    if let Some(align) = style.get("text-align") {
        props.push(val(
            "w:jc",
            match align {
                "justify" => "both",
                "end" => {
                    if style.get("direction") == Some("rtl") {
                        "left"
                    } else {
                        "right"
                    }
                }
                "start" => {
                    if style.get("direction") == Some("rtl") {
                        "right"
                    } else {
                        "left"
                    }
                }
                v => v,
            },
        ));
    }
    let mut spacing = vec![("before", "0".into()), ("after", "0".into())];
    for (css, word) in [("margin-top", "before"), ("margin-bottom", "after")] {
        if let Some(value) = style.get(css).and_then(|v| length(v, size)) {
            spacing.retain(|(key, _)| *key != word);
            spacing.push((word, (value.max(0.0) * 20.0).round().to_string()));
        }
    }
    if let Some(line) = style.get("line-height") {
        if let Ok(multiplier) = line.parse::<f64>() {
            if multiplier.is_finite() && multiplier > 0.0 {
                spacing.push(("line", (multiplier * 240.0).round().to_string()));
                spacing.push(("lineRule", "auto".into()));
            }
        } else if let Some(points) = length(line, size).filter(|n| *n > 0.0) {
            spacing.push(("line", (points * 20.0).round().to_string()));
            spacing.push(("lineRule", "exact".into()));
        }
    }
    props.push(element("w:spacing", &spacing, Vec::new()));
    let mut indent = Vec::new();
    for (css, word) in [
        ("margin-left", "left"),
        ("margin-right", "right"),
        ("text-indent", "firstLine"),
    ] {
        if let Some(value) = style.get(css).and_then(|v| length(v, size)) {
            indent.push((word, (value * 20.0).round().to_string()));
        }
    }
    if !indent.is_empty() {
        props.push(element("w:ind", &indent, Vec::new()));
    }
    if style.get("direction") == Some("rtl") {
        props.push(val("w:bidi", "1"));
    }
    if style.get("page-break-before") == Some("always") {
        props.push(val("w:pageBreakBefore", "1"));
    }
    props.push(run_properties(style));
    runs.insert(0, element("w:pPr", &[], props));
    if style.get("page-break-after") == Some("always") {
        runs.push(element(
            "w:r",
            &[],
            vec![element("w:br", &[("type", "page".into())], Vec::new())],
        ));
    }
    element("w:p", &[], runs)
}

pub(super) fn border(value: &str) -> Option<(f64, &'static str, String)> {
    if matches!(value, "none" | "0") {
        return Some((0.0, "nil", "000000".into()));
    }
    let mut width = 0.75;
    let mut pattern = None;
    let mut paint = "000000".to_string();
    for word in value.split_whitespace() {
        if let Some(points) = length(word, 12.0).filter(|v| *v >= 0.0) {
            width = points;
        } else if let Some(value) = color(word) {
            paint = value;
        } else {
            pattern = Some(match word {
                "solid" => "single",
                "dashed" => "dashed",
                "dotted" => "dotted",
                "double" => "double",
                "none" => "nil",
                _ => return None,
            });
        }
    }
    Some((width, pattern?, paint))
}

pub(super) fn cell_properties(style: &Computed) -> Vec<Rc<XNode>> {
    let mut out = Vec::new();
    let size = font_size(style);
    let mut padding = [0.0; 4];
    if let Some(value) = style.get("padding") {
        let values: Vec<_> = value
            .split_whitespace()
            .filter_map(|v| length(v, size))
            .collect();
        padding = match values.as_slice() {
            [a] => [*a; 4],
            [a, b] => [*a, *b, *a, *b],
            [a, b, c] => [*a, *b, *c, *b],
            [a, b, c, d] => [*a, *b, *c, *d],
            _ => padding,
        };
    }
    for (i, name) in [
        "padding-top",
        "padding-right",
        "padding-bottom",
        "padding-left",
    ]
    .iter()
    .enumerate()
    {
        if let Some(value) = style.get(name).and_then(|v| length(v, size)) {
            padding[i] = value;
        }
    }
    out.push(element(
        "w:tcMar",
        &[],
        ["top", "right", "bottom", "left"]
            .iter()
            .enumerate()
            .map(|(i, name)| {
                element(
                    &format!("w:{name}"),
                    &[
                        ("type", "dxa".into()),
                        ("w", (padding[i].max(0.0) * 20.0).round().to_string()),
                    ],
                    Vec::new(),
                )
            })
            .collect(),
    ));
    let mut borders = Vec::new();
    for name in ["top", "right", "bottom", "left"] {
        if let Some((width, pattern, paint)) = style
            .get(&format!("border-{name}"))
            .or(style.get("border"))
            .and_then(border)
        {
            borders.push(element(
                &format!("w:{name}"),
                &[
                    ("val", pattern.into()),
                    ("sz", (width * 8.0).round().to_string()),
                    ("color", paint),
                ],
                Vec::new(),
            ));
        }
    }
    if !borders.is_empty() {
        out.push(element("w:tcBorders", &[], borders));
    }
    out
}

pub(super) fn width(value: &str, em: f64) -> Option<(String, String)> {
    if let Some(percent) = value
        .trim()
        .strip_suffix('%')
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|n| n.is_finite() && *n >= 0.0)
    {
        Some(("pct".into(), (percent * 50.0).round().to_string()))
    } else {
        length(value, em)
            .filter(|n| *n >= 0.0)
            .map(|n| ("dxa".into(), (n * 20.0).round().to_string()))
    }
}
