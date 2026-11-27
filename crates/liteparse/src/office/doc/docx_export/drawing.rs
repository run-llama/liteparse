//! Lenient OfficeArt walk: shape type, paint, drawing order and group membership per
//! shape id, read from the drawing containers at FIB pair 50 (OfficeArtContent) in the
//! table stream.
use super::source::{u16_le, u32_le};
use std::collections::HashMap;

/// Paint of one shape. Colors are 0xRRGGBB; `None` means no fill / no line.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShapeStyle {
    /// MSOSPT shape type (1 rectangle, 2 rounded rectangle, 3 ellipse, 20 line, 202 textbox).
    pub kind: u16,
    pub fill: Option<u32>,
    pub line: Option<u32>,
    /// Line width in EMU.
    pub line_width: u32,
    pub flip_h: bool,
    pub flip_v: bool,
    /// Position in drawing order; later shapes paint over earlier ones.
    pub z: u32,
    /// Rectangle (left, top, right, bottom) of a group member in its group's coordinates.
    pub child_anchor: Option<[i32; 4]>,
    /// Coordinate space a group shape gives its members (FSPGR).
    pub group_space: Option<[i32; 4]>,
    /// 1-based picture-store index of a picture shape (`pib`).
    pub pib: Option<u32>,
    /// fBehindDocument from the group-shape booleans (0x3BF), when set explicitly.
    pub behind: Option<bool>,
    /// Clockwise rotation in 16.16 fixed-point degrees.
    pub rotation: i32,
    /// Fill opacity, 0..=65536.
    pub fill_opacity: u32,
    /// Positioning (posh, posrelh, posv, posrelv): alignment codes and reference frames.
    pub placement: [Option<u32>; 4],
    /// lTxid: 1-based textbox story entry in the high 16 bits, chain position below.
    pub txid: Option<u32>,
    /// Drawing the shape belongs to (dgglbl: 0 main document, 1 headers).
    pub header: bool,
    /// Picture crop from top, bottom, left, right as 16.16 fractions of the picture.
    pub crop: [i32; 4],
}

/// Shape styles by id, and each group's members in drawing order (nested groups by id).
#[derive(Default)]
pub struct Drawing {
    pub styles: HashMap<u32, ShapeStyle>,
    pub groups: HashMap<u32, Vec<u32>>,
    /// DrawingML `a:path` of each freeform shape, in its geometry space.
    pub paths: HashMap<u32, String>,
    /// WordArt text, font and (bold, italic) by shape id (gtextUNICODE, gtextFont, 0xFF).
    pub wordart: HashMap<u32, (String, String, (bool, bool))>,
}

/// OfficeArt defaults: filled white, 0.75pt black line.
const DEFAULT: ShapeStyle = ShapeStyle {
    kind: 1,
    fill: Some(0xffffff),
    line: Some(0),
    line_width: 9525,
    flip_h: false,
    flip_v: false,
    z: 0,
    child_anchor: None,
    group_space: None,
    pib: None,
    behind: None,
    rotation: 0,
    fill_opacity: 65536,
    placement: [None; 4],
    txid: None,
    header: false,
    crop: [0; 4],
};

/// Shapes and groups of every drawing. Unreadable records end the walk with what was read.
pub fn read(content: &[u8]) -> Drawing {
    let mut out = Drawing::default();
    // OfficeArtContent: DggContainer, then per drawing a dgglbl byte and a DgContainer.
    let Some(len) = record_len(content, 0) else {
        return out;
    };
    let mut at = len;
    while at < content.len() {
        let Some(len) = record_len(content, at + 1) else {
            break;
        };
        let before: Vec<u32> = out.styles.keys().copied().collect();
        walk(&content[at + 1..at + 1 + len], &mut out);
        if content[at] == 1 {
            for (id, style) in out.styles.iter_mut() {
                if !before.contains(id) {
                    style.header = true;
                }
            }
        }
        at += 1 + len;
    }
    out
}

/// Whole record length (8-byte header plus body), if it fits.
fn record_len(data: &[u8], at: usize) -> Option<usize> {
    let len = 8 + u32_le(data, at + 4)? as usize;
    (at + len <= data.len()).then_some(len)
}

fn walk(data: &[u8], out: &mut Drawing) {
    let mut at = 0;
    while let Some(len) = record_len(data, at) {
        let head = u16_le(data, at).unwrap_or(0);
        let kind = u16_le(data, at + 2).unwrap_or(0);
        let body = &data[at + 8..at + len];
        match kind {
            0xf004 => {
                shape(body, out);
            }
            0xf003 => {
                group(body, out);
            }
            _ if head & 0xf == 0xf => walk(body, out),
            _ => {}
        }
        at += len;
    }
}

/// SpgrContainer: the first SpContainer is the group shape itself, then its members
/// (shapes or nested groups) in drawing order. Returns the group shape's id.
fn group(data: &[u8], out: &mut Drawing) -> Option<u32> {
    let mut at = 0;
    let mut id = None;
    let mut members = Vec::new();
    while let Some(len) = record_len(data, at) {
        let kind = u16_le(data, at + 2).unwrap_or(0);
        let body = &data[at + 8..at + len];
        // Only a leading SpContainer names the group; nested groups are always members.
        match kind {
            0xf004 if id.is_none() && members.is_empty() => id = shape(body, out),
            0xf004 => members.extend(shape(body, out)),
            0xf003 => members.extend(group(body, out)),
            _ => {}
        }
        at += len;
    }
    if let Some(id) = id {
        out.groups.insert(id, members);
    }
    id
}

/// One SpContainer: FSP (0xF00A) gives the id and type; OPT records the paint; FSPGR
/// (0xF009) and ChildAnchor (0xF00F) place group members.
fn shape(data: &[u8], out: &mut Drawing) -> Option<u32> {
    let mut style = DEFAULT;
    let mut id = None;
    let (mut filled, mut lined) = (true, true);
    let (mut vertices, mut segments) = (Vec::new(), Vec::new());
    let (mut gtext, mut gfont) = (None, String::new());
    let mut gstyle = (false, false);
    // geoLeft, geoTop, geoRight, geoBottom.
    let mut geo = [0, 0, 21600, 21600];
    let mut at = 0;
    while let Some(len) = record_len(data, at) {
        let head = u16_le(data, at).unwrap_or(0);
        let kind = u16_le(data, at + 2).unwrap_or(0);
        let body = &data[at + 8..at + len];
        match kind {
            0xf00a => {
                style.kind = head >> 4;
                id = u32_le(body, 0);
                let flags = u32_le(body, 4).unwrap_or(0);
                style.flip_h = flags & 0x40 != 0;
                style.flip_v = flags & 0x80 != 0;
            }
            0xf009 | 0xf00f => {
                let rect = [0, 4, 8, 12].map(|o| u32_le(body, o).unwrap_or(0) as i32);
                if kind == 0xf009 {
                    style.group_space = Some(rect);
                } else {
                    style.child_anchor = Some(rect);
                }
            }
            0xf00b | 0xf122 => {
                let count = (head >> 4) as usize;
                // Complex property data follows the fixed table, in property order.
                let mut complex = count * 6;
                for k in 0..count {
                    let Some(op) = u16_le(body, k * 6) else { break };
                    let value = u32_le(body, k * 6 + 2).unwrap_or(0);
                    if op & 0x8000 != 0 {
                        let mut len = value as usize;
                        // IMsoArray lengths sometimes leave out the 6-byte array header.
                        if matches!(op & 0x3fff, 0x145 | 0x146 | 0x151 | 0x152) {
                            let n = u16_le(body, complex).unwrap_or(0) as usize;
                            let size = match u16_le(body, complex + 4).unwrap_or(0) {
                                0xfff0 => 4,
                                c => c as usize,
                            };
                            if len + 6 == 6 + n * size {
                                len += 6;
                            }
                        }
                        let data = body.get(complex..complex + len).unwrap_or(&[]);
                        complex += len;
                        match op & 0x3fff {
                            0x145 => vertices = array(data),
                            0x146 => segments = array(data),
                            0xc0 => gtext = Some(utf16z(data)),
                            0xc5 => gfont = utf16z(data),
                            _ => {}
                        }
                        continue;
                    }
                    match op & 0x3fff {
                        0x140..=0x143 => geo[(op & 0x3fff) as usize - 0x140] = value as i32,
                        0x181 => style.fill = rgb(value).or(style.fill),
                        0x1c0 => style.line = rgb(value).or(style.line),
                        0x1cb => style.line_width = value,
                        0x104 => style.pib = Some(value),
                        0x080 => style.txid = Some(value),
                        0x100..=0x103 => style.crop[(op & 0x3fff) as usize - 0x100] = value as i32,
                        0x004 => style.rotation = value as i32,
                        0x38f..=0x392 => {
                            style.placement[(op & 0x3fff) as usize - 0x38f] = Some(value)
                        }
                        0x182 => style.fill_opacity = value.min(65536),
                        // Geometry-text booleans: bold 0x20 and italic 0x10, each with a use bit 16 higher.
                        0xff => {
                            gstyle = (
                                value & 0x20_0020 == 0x20_0020,
                                value & 0x10_0010 == 0x10_0010,
                            )
                        }
                        0x3bf if value & 0x20_0000 != 0 => style.behind = Some(value & 0x20 != 0),
                        // Boolean groups: bit n is the value, bit n+16 says it is set.
                        0x1bf if value & 0x10_0000 != 0 => filled = value & 0x10 != 0,
                        0x1ff if value & 0x8_0000 != 0 => lined = value & 0x8 != 0,
                        _ => {}
                    }
                }
            }
            _ => {}
        }
        at += len;
    }
    if !filled {
        style.fill = None;
    }
    if !lined {
        style.line = None;
    }
    let id = id?;
    style.z = out.styles.len() as u32;
    out.styles.insert(id, style);
    if let Some(path) = path_xml(&vertices, &segments, geo) {
        out.paths.insert(id, path);
    }
    if let Some(text) = gtext.filter(|t| !t.trim().is_empty()) {
        out.wordart.insert(id, (text, gfont, gstyle));
    }
    Some(id)
}

/// UTF-16LE text up to its terminating NUL.
fn utf16z(data: &[u8]) -> String {
    let units: Vec<u16> = data
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .take_while(|&u| u != 0)
        .collect();
    String::from_utf16_lossy(&units)
}

/// IMsoArray elements as (x, y) pairs or u16 values: nElems, nElemsAlloc, cbElem, then
/// data. cbElem 0xFFF0 means two 16-bit halves; 8 means two 32-bit values.
pub fn array(data: &[u8]) -> Vec<(i32, i32)> {
    let n = u16_le(data, 0).unwrap_or(0) as usize;
    let cb = u16_le(data, 4).unwrap_or(0);
    let size = match cb {
        0xfff0 => 4,
        c => c as usize,
    };
    (0..n)
        .map_while(|k| {
            let at = 6 + k * size;
            match size {
                2 => Some((u16_le(data, at)? as i32, 0)),
                4 => Some((
                    u16_le(data, at)? as i16 as i32,
                    u16_le(data, at + 2)? as i16 as i32,
                )),
                8 => Some((u32_le(data, at)? as i32, u32_le(data, at + 4)? as i32)),
                _ => None,
            }
        })
        .collect()
}

/// Freeform outline as a DrawingML path. Segment entries carry a type in the top 3 bits
/// (0 line, 1 cubic curve, 2 move, 3 close, 4 end) and a count; without them the
/// vertices form one polyline.
pub fn path_xml(vertices: &[(i32, i32)], segments: &[(i32, i32)], geo: [i32; 4]) -> Option<String> {
    if vertices.len() < 2 {
        return None;
    }
    let [l, t, r, b] = geo;
    let pt = |&(x, y): &(i32, i32)| format!("<a:pt x=\"{}\" y=\"{}\"/>", x - l, y - t);
    let mut out = format!("<a:path w=\"{}\" h=\"{}\">", (r - l).max(1), (b - t).max(1));
    let mut points = vertices.iter();
    let mut started = false;
    let plan: Vec<(u16, usize)> = if segments.is_empty() {
        vec![(2, 1), (0, vertices.len() - 1)]
    } else {
        segments
            .iter()
            .map(|&(v, _)| ((v as u16) >> 13, (v as u16 & 0x1fff) as usize))
            .collect()
    };
    for (kind, count) in plan {
        match kind {
            0 | 1 if !started => return None,
            0 => {
                for p in points.by_ref().take(count.max(1)) {
                    out.push_str(&format!("<a:lnTo>{}</a:lnTo>", pt(p)));
                }
            }
            1 => {
                for _ in 0..count.max(1) {
                    let three: Vec<String> = points.by_ref().take(3).map(pt).collect();
                    if three.len() < 3 {
                        break;
                    }
                    out.push_str(&format!("<a:cubicBezTo>{}</a:cubicBezTo>", three.concat()));
                }
            }
            2 => {
                let p = points.next()?;
                out.push_str(&format!("<a:moveTo>{}</a:moveTo>", pt(p)));
                started = true;
            }
            3 => out.push_str("<a:close/>"),
            4 => break,
            _ => {}
        }
    }
    started.then(|| out + "</a:path>")
}

/// OfficeArtCOLORREF is 0x00BBGGRR. Scheme and system color indexes (high byte set)
/// return `None`, so the shape keeps its default color.
fn rgb(v: u32) -> Option<u32> {
    (v >> 24 == 0).then_some(((v & 0xff) << 16) | (v & 0xff00) | ((v >> 16) & 0xff))
}

impl ShapeStyle {
    /// Rotation in whole degrees, normalized to 0..360.
    pub fn degrees(&self) -> f64 {
        (self.rotation as f64 / 65536.0).rem_euclid(360.0)
    }

    /// DrawingML fill and outline for wps:spPr.
    pub fn paint_xml(&self) -> String {
        let fill = match self.fill {
            Some(c) => format!("<a:solidFill><a:srgbClr val=\"{c:06X}\"/></a:solidFill>"),
            None => "<a:noFill/>".into(),
        };
        let line = match self.line {
            Some(c) => format!(
                "<a:ln w=\"{}\"><a:solidFill><a:srgbClr val=\"{c:06X}\"/></a:solidFill></a:ln>",
                self.line_width
            ),
            None => "<a:ln><a:noFill/></a:ln>".into(),
        };
        format!("{fill}{line}")
    }

    /// Preset geometry for a plain shape, if it has a DrawingML equivalent (MSOSPT number
    /// to preset name; shapes Word only draws through custom geometry return `None`).
    pub fn preset(&self) -> Option<&'static str> {
        preset(self.kind)
    }
}

/// DrawingML preset geometry name of an MSOSPT shape type, if it has one.
pub fn preset(kind: u16) -> Option<&'static str> {
    Some(match kind {
        1 | 202 => "rect",
        2 => "roundRect",
        3 => "ellipse",
        4 => "diamond",
        5 => "triangle",
        6 => "rtTriangle",
        7 => "parallelogram",
        8 => "trapezoid",
        9 => "hexagon",
        10 => "octagon",
        11 => "plus",
        12 => "star5",
        13 => "rightArrow",
        15 => "homePlate",
        16 => "cube",
        19 => "arc",
        20 | 32 => "line",
        21 => "plaque",
        22 => "can",
        23 => "donut",
        33 => "bentConnector2",
        34 => "bentConnector3",
        35 => "bentConnector4",
        36 => "bentConnector5",
        37 => "curvedConnector2",
        38 => "curvedConnector3",
        39 => "curvedConnector4",
        40 => "curvedConnector5",
        55 => "chevron",
        56 => "pentagon",
        57 => "noSmoking",
        58 => "star8",
        59 => "star16",
        60 => "star32",
        61 => "wedgeRectCallout",
        62 => "wedgeRoundRectCallout",
        63 => "wedgeEllipseCallout",
        64 => "wave",
        65 => "foldedCorner",
        66 => "leftArrow",
        67 => "downArrow",
        68 => "upArrow",
        69 => "leftRightArrow",
        70 => "upDownArrow",
        71 => "irregularSeal1",
        72 => "irregularSeal2",
        73 => "lightningBolt",
        74 => "heart",
        76 => "quadArrow",
        84 => "bevel",
        85 => "leftBracket",
        86 => "rightBracket",
        87 => "leftBrace",
        88 => "rightBrace",
        92 => "star24",
        96 => "smileyFace",
        97 => "verticalScroll",
        98 => "horizontalScroll",
        99 => "circularArrow",
        // 100 is the notched circular arrow, which DrawingML draws as a circular arrow.
        94 => "notchedRightArrow",
        100 => "circularArrow",
        109 => "flowChartProcess",
        110 => "flowChartDecision",
        111 => "flowChartInputOutput",
        112 => "flowChartPredefinedProcess",
        113 => "flowChartInternalStorage",
        114 => "flowChartDocument",
        115 => "flowChartMultidocument",
        116 => "flowChartTerminator",
        117 => "flowChartPreparation",
        118 => "flowChartManualInput",
        119 => "flowChartManualOperation",
        120 => "flowChartConnector",
        121 => "flowChartPunchedCard",
        122 => "flowChartPunchedTape",
        123 => "flowChartSummingJunction",
        124 => "flowChartOr",
        125 => "flowChartCollate",
        126 => "flowChartSort",
        127 => "flowChartExtract",
        128 => "flowChartMerge",
        129 => "flowChartOfflineStorage",
        130 => "flowChartOnlineStorage",
        131 => "flowChartMagneticTape",
        132 => "flowChartMagneticDisk",
        133 => "flowChartMagneticDrum",
        134 => "flowChartDisplay",
        135 => "flowChartDelay",
        176 => "flowChartAlternateProcess",
        177 => "flowChartOffpageConnector",
        183 => "sun",
        184 => "moon",
        185 => "bracketPair",
        186 => "bracePair",
        187 => "star4",
        14 => "rightArrow",
        41 => "callout1",
        42 => "callout2",
        43 => "callout3",
        44 => "accentCallout1",
        45 => "accentCallout2",
        46 => "accentCallout3",
        47 => "borderCallout1",
        48 => "borderCallout2",
        49 => "borderCallout3",
        50 => "accentBorderCallout1",
        51 => "accentBorderCallout2",
        52 => "accentBorderCallout3",
        53 => "ribbon",
        54 => "ribbon2",
        77 => "leftArrowCallout",
        78 => "rightArrowCallout",
        79 => "upArrowCallout",
        80 => "downArrowCallout",
        81 => "leftRightArrowCallout",
        82 => "upDownArrowCallout",
        83 => "quadArrowCallout",
        89 => "leftUpArrow",
        90 => "bentUpArrow",
        91 => "bentArrow",
        93 => "stripedRightArrow",
        95 => "blockArc",
        101 => "uturnArrow",
        102 => "curvedRightArrow",
        103 => "curvedLeftArrow",
        104 => "curvedUpArrow",
        105 => "curvedDownArrow",
        106 => "cloudCallout",
        107 => "ellipseRibbon",
        108 => "ellipseRibbon2",
        182 => "leftRightUpArrow",
        188 => "doubleWave",
        189 => "actionButtonBlank",
        190 => "actionButtonHome",
        191 => "actionButtonHelp",
        192 => "actionButtonInformation",
        193 => "actionButtonForwardNext",
        194 => "actionButtonBackPrevious",
        195 => "actionButtonEnd",
        196 => "actionButtonBeginning",
        197 => "actionButtonReturn",
        198 => "actionButtonDocument",
        199 => "actionButtonSound",
        200 => "actionButtonMovie",
        _ => return None,
    })
}
