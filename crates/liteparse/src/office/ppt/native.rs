//! DrawingML for a shape read from the binary drawing: geometry, fill, outline,
//! shadow, picture and text body, with every colour resolved against the slide's
//! colour scheme. Pictures are written as `r:embed="PIB<n>"`, `n` being the 1-based
//! picture-store index; the package writer turns those into relationships.

use crate::office::doc::docx_export::drawing;
use crate::office::ppt::props::{Props, Scheme, color, id};
use crate::office::ppt::text::{self, Body, Env};

/// MSOSPT shape types the writer treats specially.
pub const PICTURE_FRAME: u16 = 75;
const NOT_PRIMITIVE: u16 = 0;
const LINE: u16 = 20;

/// What the writer reads from one shape, after anchors are mapped to the slide.
pub struct Native<'p, 'a> {
    pub kind: u16,
    pub flip_h: bool,
    pub flip_v: bool,
    /// Clockwise rotation in degrees.
    pub rotation: f64,
    /// (x, y, width, height) in EMU.
    pub rect: [i64; 4],
    pub props: &'p Props<'a>,
    pub text: Option<&'p Body>,
    /// Character positions replaced by field text (slide numbers, footers).
    pub fields: &'p [(usize, String)],
    /// The shape shows an embedded equation: its picture is written as the
    /// preview of an OLE frame with the equation's progId.
    pub equation: bool,
}

/// Whether an OLE progId names an equation (Microsoft Equation 3.0, MathType).
pub fn is_equation_prog_id(prog_id: &str) -> bool {
    let p = prog_id.to_ascii_lowercase();
    p.starts_with("equation.") || p.starts_with("mathtype")
}

/// 16.16 fixed point to a fraction.
fn fixed(v: u32) -> f64 {
    v as i32 as f64 / 65536.0
}

fn srgb(c: u32, alpha: Option<u32>) -> String {
    match alpha.filter(|&a| a < 65536) {
        Some(a) => format!(
            "<a:srgbClr val=\"{c:06X}\"><a:alpha val=\"{}\"/></a:srgbClr>",
            (a as u64 * 100_000 / 65536)
        ),
        None => format!("<a:srgbClr val=\"{c:06X}\"/>"),
    }
}

/// The shape's own fill and line colours, which relative colours refer to.
fn base_colors(p: &Props, scheme: &Scheme) -> (u32, u32) {
    let fill = p
        .get(id::FILL_COLOR)
        .and_then(|v| color(v, scheme, 0xFFFFFF, 0))
        .unwrap_or(0xFFFFFF);
    let line = p
        .get(id::LINE_COLOR)
        .and_then(|v| color(v, scheme, fill, 0))
        .unwrap_or(0);
    (fill, line)
}

/// A picture's colour adjustments as DrawingML blip effects: its transparent
/// colour (pictureTransparent) and its contrast and brightness (pictureContrast,
/// 16.16 with 1.0 unchanged; pictureBrightness, -32768..32768 for -100%..100%).
pub fn blip_effects(p: &Props, scheme: &Scheme) -> String {
    let mut out = String::new();
    if let Some(c) = p
        .get(id::PICTURE_TRANSPARENT)
        .and_then(|v| color(v, scheme, 0xFFFFFF, 0))
    {
        out.push_str(&format!(
            "<a:clrChange><a:clrFrom><a:srgbClr val=\"{c:06X}\"/></a:clrFrom><a:clrTo><a:srgbClr val=\"{c:06X}\"><a:alpha val=\"0\"/></a:srgbClr></a:clrTo></a:clrChange>"
        ));
    }
    let contrast = p.get(id::PICTURE_CONTRAST).map(|v| v as f64 / 65536.0);
    let bright = p
        .get(id::PICTURE_BRIGHTNESS)
        .map(|v| v as i32 as f64 / 32768.0);
    if contrast.is_some() || bright.is_some() {
        let c = match contrast.unwrap_or(1.0) {
            c if c < 1.0 => c - 1.0,
            c => 1.0 - 1.0 / c,
        };
        out.push_str(&format!(
            "<a:lum bright=\"{}\" contrast=\"{}\"/>",
            (bright.unwrap_or(0.0) * 100_000.0).round() as i64,
            (c * 100_000.0).round() as i64
        ));
    }
    out
}

/// DrawingML fill of a shape or background (`a:noFill` when unfilled).
pub fn fill_xml(p: &Props, scheme: &Scheme) -> String {
    if p.flag(id::FILL_BOOLS, 4) == Some(false) {
        return "<a:noFill/>".into();
    }
    let (fill, line) = base_colors(p, scheme);
    let back = p
        .get(id::FILL_BACK_COLOR)
        .and_then(|v| color(v, scheme, fill, line))
        .unwrap_or(0xFFFFFF);
    let opacity = p.get(id::FILL_OPACITY);
    match p.get(id::FILL_TYPE).unwrap_or(0) {
        2 | 3 => match p.get(id::FILL_BLIP) {
            Some(pib) if pib > 0 => {
                let mode = if p.get(id::FILL_TYPE) == Some(2) {
                    "<a:tile tx=\"0\" ty=\"0\" sx=\"100000\" sy=\"100000\" flip=\"none\" algn=\"tl\"/>"
                } else {
                    "<a:stretch><a:fillRect/></a:stretch>"
                };
                format!(
                    "<a:blipFill dpi=\"0\" rotWithShape=\"1\"><a:blip r:embed=\"PIB{pib}\"/>{mode}</a:blipFill>"
                )
            }
            _ => format!("<a:solidFill>{}</a:solidFill>", srgb(fill, opacity)),
        },
        4..=8 => {
            let back_opacity = p.get(id::FILL_BACK_OPACITY);
            let focus = p.get(id::FILL_FOCUS).map_or(0, |v| v as i32);
            let (a, b) = ((fill, opacity), (back, back_opacity));
            let stops = match focus {
                // The fill colour starts the gradient and the back colour sits at the
                // focus (percent along it): 100 runs fill to back, 0 back to fill.
                f if f.abs() >= 75 => shade_stops(a, b, &[(0, 100_000)]),
                // Positive focus puts the back colour at both ends and the fill colour in
                // the middle (a +50 master background in PowerPoint); negative the reverse.
                f if f >= 25 => shade_stops(a, b, &[(50_000, 0), (50_000, 100_000)]),
                f if f.abs() >= 25 => shade_stops(b, a, &[(50_000, 0), (50_000, 100_000)]),
                _ => shade_stops(a, b, &[(100_000, 0)]),
            };
            // fillAngle turns the gradient counter-clockwise from top-to-bottom.
            let angle = fixed(p.get(id::FILL_ANGLE).unwrap_or(0));
            let dml = (90.0 - angle).rem_euclid(360.0);
            let path = match p.get(id::FILL_TYPE) {
                Some(5) | Some(6) => "<a:path path=\"rect\"><a:fillToRect l=\"50000\" t=\"50000\" r=\"50000\" b=\"50000\"/></a:path>".to_string(),
                _ => format!("<a:lin ang=\"{}\" scaled=\"0\"/>", (dml * 60000.0).round() as i64),
            };
            format!("<a:gradFill rotWithShape=\"1\"><a:gsLst>{stops}</a:gsLst>{path}</a:gradFill>")
        }
        9 => "<a:noFill/>".into(),
        _ => format!("<a:solidFill>{}</a:solidFill>", srgb(fill, opacity)),
    }
}

/// Gradient stops for runs from colour `from` (at the first position of each
/// pair) to `to` (at the second), positions in 1/1000 percent. PowerPoint shades a
/// legacy gradient along a curve: at a fraction `t` of the way from `from`, the
/// colour is `t^1.87` of the way to `to` (a +50 master background: 0.73 of the
/// way to the fill colour at the run's midpoint). Eight stops per run follow it.
fn shade_stops(from: (u32, Option<u32>), to: (u32, Option<u32>), runs: &[(i64, i64)]) -> String {
    const STEPS: i64 = 8;
    let mix = |t: f64| {
        let k = t.powf(1.87);
        let channel = |shift: u32| {
            let (x, y) = (
                f64::from((from.0 >> shift) & 0xFF),
                f64::from((to.0 >> shift) & 0xFF),
            );
            ((x + (y - x) * k).round() as u32).min(255) << shift
        };
        let alpha = match (from.1, to.1) {
            (None, None) => None,
            (x, y) => {
                let (x, y) = (f64::from(x.unwrap_or(65536)), f64::from(y.unwrap_or(65536)));
                Some((x + (y - x) * k).round() as u32)
            }
        };
        srgb(channel(16) | channel(8) | channel(0), alpha)
    };
    let mut stops: Vec<(i64, String)> = Vec::new();
    for &(p0, p1) in runs {
        for i in 0..=STEPS {
            let pos = p0 + (p1 - p0) * i / STEPS;
            if !stops.iter().any(|(p, _)| *p == pos) {
                stops.push((pos, mix(i as f64 / STEPS as f64)));
            }
        }
    }
    stops.sort_by_key(|(p, _)| *p);
    stops
        .into_iter()
        .map(|(p, c)| format!("<a:gs pos=\"{p}\">{c}</a:gs>"))
        .collect()
}

fn line_xml(p: &Props, scheme: &Scheme) -> String {
    if p.flag(id::LINE_BOOLS, 3) == Some(false) {
        return "<a:ln><a:noFill/></a:ln>".into();
    }
    let (_, line) = base_colors(p, scheme);
    let w = p.get(id::LINE_WIDTH).unwrap_or(9525);
    let dash = match p.get(id::LINE_DASHING).unwrap_or(0) {
        1 => "sysDash",
        2 => "sysDot",
        3 => "sysDashDot",
        4 => "sysDashDotDot",
        5 => "dot",
        6 => "dash",
        7 => "lgDash",
        8 => "dashDot",
        9 => "lgDashDot",
        10 => "lgDashDotDot",
        _ => "solid",
    };
    let end = |tag: &str, kind: u16, wid: u16, len: u16| -> String {
        let t = match p.get(kind).unwrap_or(0) {
            1 => "triangle",
            2 => "stealth",
            3 => "diamond",
            4 => "oval",
            5 => "arrow",
            _ => return String::new(),
        };
        let size = |v: Option<u32>| match v.unwrap_or(1) {
            0 => "sm",
            2 => "lg",
            _ => "med",
        };
        format!(
            "<a:{tag} type=\"{t}\" w=\"{}\" len=\"{}\"/>",
            size(p.get(wid)),
            size(p.get(len))
        )
    };
    format!(
        "<a:ln w=\"{w}\"><a:solidFill>{}</a:solidFill><a:prstDash val=\"{dash}\"/>{}{}</a:ln>",
        srgb(line, p.get(id::LINE_OPACITY)),
        end(
            "headEnd",
            id::LINE_START_ARROW,
            id::LINE_START_WIDTH,
            id::LINE_START_LENGTH
        ),
        end(
            "tailEnd",
            id::LINE_END_ARROW,
            id::LINE_END_WIDTH,
            id::LINE_END_LENGTH
        ),
    )
}

fn effects_xml(p: &Props, scheme: &Scheme) -> String {
    if p.flag(id::SHADOW_BOOLS, 1) != Some(true) {
        return String::new();
    }
    let (fill, line) = base_colors(p, scheme);
    let c = p
        .get(id::SHADOW_COLOR)
        .and_then(|v| color(v, scheme, fill, line))
        .unwrap_or(0x808080);
    let dx = p
        .get(id::SHADOW_OFFSET_X)
        .map_or(25400.0, |v| v as i32 as f64);
    let dy = p
        .get(id::SHADOW_OFFSET_Y)
        .map_or(25400.0, |v| v as i32 as f64);
    let dist = (dx * dx + dy * dy).sqrt().round() as i64;
    let dir = (dy.atan2(dx).to_degrees().rem_euclid(360.0) * 60000.0).round() as i64;
    format!(
        "<a:effectLst><a:outerShdw dist=\"{dist}\" dir=\"{dir}\" algn=\"ctr\" rotWithShape=\"0\">{}</a:outerShdw></a:effectLst>",
        srgb(c, p.get(id::SHADOW_OPACITY))
    )
}

fn geometry_xml(n: &Native) -> String {
    let p = n.props;
    if let (Some(v), Some(s)) = (
        p.complex(id::VERTICES),
        p.complex(id::SEGMENTS).or(Some(&[])),
    ) {
        let geo = [0, 1, 2, 3].map(|k| {
            p.get(id::GEO_LEFT + k)
                .map_or(if k >= 2 { 21600 } else { 0 }, |v| v as i32)
        });
        let segs = drawing::array(s);
        if let Some(path) = drawing::path_xml(&drawing::array(v), &segs, geo) {
            return format!(
                "<a:custGeom><a:avLst/><a:gdLst/><a:ahLst/><a:cxnLst/><a:rect l=\"l\" t=\"t\" r=\"r\" b=\"b\"/><a:pathLst>{path}</a:pathLst></a:custGeom>"
            );
        }
    }
    let prst = match n.kind {
        NOT_PRIMITIVE | PICTURE_FRAME => "rect",
        k => drawing::preset(k).unwrap_or("rect"),
    };
    let av = match (prst, p.get(id::ADJUST)) {
        ("roundRect", Some(a)) => format!(
            "<a:gd name=\"adj\" fmla=\"val {}\"/>",
            (a as i32 as i64 * 100_000 / 21600).clamp(0, 50_000)
        ),
        _ => String::new(),
    };
    format!("<a:prstGeom prst=\"{prst}\"><a:avLst>{av}</a:avLst></a:prstGeom>")
}

fn xfrm(n: &Native) -> String {
    let [x, y, w, h] = n.rect;
    let mut attrs = String::new();
    let rot = (n.rotation.rem_euclid(360.0) * 60000.0).round() as i64;
    if rot != 0 {
        attrs.push_str(&format!(" rot=\"{rot}\""));
    }
    if n.flip_h {
        attrs.push_str(" flipH=\"1\"");
    }
    if n.flip_v {
        attrs.push_str(" flipV=\"1\"");
    }
    format!("<a:xfrm{attrs}><a:off x=\"{x}\" y=\"{y}\"/><a:ext cx=\"{w}\" cy=\"{h}\"/></a:xfrm>")
}

fn body_pr(p: &Props) -> String {
    let ins = |k: u16, d: u32| p.get(k).unwrap_or(d) as i32;
    let anchor = match p.get(id::ANCHOR_TEXT).unwrap_or(0) {
        1 | 4 | 7 => "ctr",
        2 | 5 | 8 => "b",
        _ => "t",
    };
    let ctr = matches!(p.get(id::ANCHOR_TEXT), Some(3..=5) | Some(9));
    let wrap = if p.get(id::WRAP_TEXT) == Some(2) {
        "none"
    } else {
        "square"
    };
    let vert = match p.get(id::TEXT_FLOW).unwrap_or(0) {
        1 | 3 | 5 => " vert=\"vert\"",
        2 => " vert=\"vert270\"",
        _ => "",
    };
    format!(
        "<a:bodyPr wrap=\"{wrap}\" lIns=\"{}\" tIns=\"{}\" rIns=\"{}\" bIns=\"{}\" anchor=\"{anchor}\" anchorCtr=\"{}\" rtlCol=\"0\"{vert}><a:noAutofit/></a:bodyPr>",
        ins(id::TEXT_LEFT, 91440),
        ins(id::TEXT_TOP, 45720),
        ins(id::TEXT_RIGHT, 91440),
        ins(id::TEXT_BOTTOM, 45720),
        u8::from(ctr)
    )
}

/// The `p:sp` or `p:pic` element for one shape; `None` for a shape that draws nothing.
pub fn shape(n: &Native, env: &Env, shape_id: u32) -> Option<String> {
    let p = n.props;
    let scheme = env.scheme;
    let pib = p.get(id::PIB).filter(|&v| v > 0);
    let geom = geometry_xml(n);
    let line = line_xml(p, scheme);
    let effects = effects_xml(p, scheme);
    if let Some(pib) = pib {
        let crop = [
            id::CROP_TOP,
            id::CROP_TOP + 1,
            id::CROP_TOP + 2,
            id::CROP_TOP + 3,
        ]
        .map(|k| (fixed(p.get(k).unwrap_or(0)) * 100_000.0).round() as i64);
        // Crops are stored top, bottom, left, right.
        let src = if crop.iter().any(|&c| c != 0) {
            format!(
                "<a:srcRect l=\"{}\" t=\"{}\" r=\"{}\" b=\"{}\"/>",
                crop[2], crop[0], crop[3], crop[1]
            )
        } else {
            String::new()
        };
        let pic = format!(
            "<p:pic><p:nvPicPr><p:cNvPr id=\"{shape_id}\" name=\"Picture {shape_id}\"/><p:cNvPicPr/><p:nvPr/></p:nvPicPr>\
<p:blipFill><a:blip r:embed=\"PIB{pib}\">{}</a:blip>{src}<a:stretch><a:fillRect/></a:stretch></p:blipFill>\
<p:spPr>{}{geom}{line}{effects}</p:spPr></p:pic>",
            blip_effects(p, scheme),
            xfrm(n)
        );
        if n.equation {
            let frame = xfrm(n)
                .replace("<a:xfrm", "<p:xfrm")
                .replace("</a:xfrm>", "</p:xfrm>");
            return Some(format!(
                "<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id=\"{shape_id}\" name=\"Equation {shape_id}\"/>\
<p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr>{frame}<a:graphic><a:graphicData \
uri=\"http://schemas.openxmlformats.org/presentationml/2006/ole\"><p:oleObj progId=\"Equation.3\">\
<p:embed/>{pic}</p:oleObj></a:graphicData></a:graphic></p:graphicFrame>"
            ));
        }
        return Some(pic);
    }
    if let Some(xml) = wordart(n, env, shape_id) {
        return Some(xml);
    }
    let fill = if n.kind == LINE {
        "<a:noFill/>".to_string()
    } else {
        fill_xml(p, scheme)
    };
    let has_text = n.text.is_some_and(|b| !b.units.is_empty());
    if fill == "<a:noFill/>" && line.contains("<a:noFill/>") && !has_text && effects.is_empty() {
        return None;
    }
    let body = match n.text {
        Some(b) => format!(
            "<p:txBody>{}<a:lstStyle/>{}</p:txBody>",
            body_pr(p),
            text::paragraphs(b, env, n.fields)
        ),
        None => String::new(),
    };
    Some(format!(
        "<p:sp><p:nvSpPr><p:cNvPr id=\"{shape_id}\" name=\"Shape {shape_id}\"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>\
<p:spPr>{}{geom}{fill}{line}{effects}</p:spPr>{body}</p:sp>",
        xfrm(n)
    ))
}

/// WordArt (text-effect presets 136-175, text in gtextUNICODE): drawn as centred,
/// unwrapped text in the shape's fill colour, sized to fill the box. The warp itself
/// is not reproduced.
fn wordart(n: &Native, env: &Env, shape_id: u32) -> Option<String> {
    if !(136..=175).contains(&n.kind) {
        return None;
    }
    let p = n.props;
    let utf16z = |d: &[u8]| {
        let units: Vec<u16> = d
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .take_while(|&u| u != 0)
            .collect();
        String::from_utf16_lossy(&units)
    };
    let text = p.complex(0xC0).map(utf16z)?;
    let lines: Vec<&str> = text.split(['\r', '\n']).filter(|l| !l.is_empty()).collect();
    if lines.iter().all(|l| l.trim().is_empty()) {
        return None;
    }
    let font = p
        .complex(0xC5)
        .map(utf16z)
        .filter(|f| !f.is_empty())
        .unwrap_or_else(|| "Arial".into());
    let style = p.get(0xFF).unwrap_or(0);
    let (bold, italic) = (
        style & 0x20_0020 == 0x20_0020,
        style & 0x10_0010 == 0x10_0010,
    );
    // Points per line from the box height, capped so the longest line fits its width
    // at about 0.55 em per character.
    let (w, h) = (n.rect[2] as f64 / 12_700.0, n.rect[3] as f64 / 12_700.0);
    let longest = lines
        .iter()
        .map(|l| l.chars().count())
        .max()
        .unwrap_or(1)
        .max(1) as f64;
    let size = (h / lines.len() as f64 * 0.8)
        .min(w / (longest * 0.55))
        .clamp(4.0, 400.0);
    let sz = ((size * 2.0).round() * 50.0) as i64;
    let fill = match fill_xml(p, env.scheme) {
        f if f == "<a:noFill/>" => "<a:solidFill><a:srgbClr val=\"000000\"/></a:solidFill>".into(),
        f => f,
    };
    let attrs = format!(
        " sz=\"{sz}\"{}{}",
        if bold { " b=\"1\"" } else { "" },
        if italic { " i=\"1\"" } else { "" }
    );
    let face = crate::office::ppt::pptx::escape(&font);
    let paras: String = lines
        .iter()
        .map(|l| {
            format!(
                "<a:p><a:pPr algn=\"ctr\"/><a:r><a:rPr lang=\"en-US\"{attrs}>{fill}<a:latin typeface=\"{face}\"/></a:rPr><a:t>{}</a:t></a:r></a:p>",
                crate::office::ppt::pptx::escape(l)
            )
        })
        .collect();
    Some(format!(
        "<p:sp><p:nvSpPr><p:cNvPr id=\"{shape_id}\" name=\"WordArt {shape_id}\"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>\
<p:spPr>{}<a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom><a:noFill/><a:ln><a:noFill/></a:ln></p:spPr>\
<p:txBody><a:bodyPr wrap=\"none\" lIns=\"0\" tIns=\"0\" rIns=\"0\" bIns=\"0\" anchor=\"ctr\"/><a:lstStyle/>{paras}</p:txBody></p:sp>",
        xfrm(n)
    ))
}
