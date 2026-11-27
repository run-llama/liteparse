//! From decoder output to liteparse pages.
//!
//! [`super::docx`] and [`super::pptx`] lay the document out themselves and
//! describe every page in the extract protocol (an index object plus one JSON
//! record per page). [`decode`] runs them in memory and maps each record's
//! text runs, vector graphics, picture positions and screenshot onto
//! liteparse types.
//!
//! A decoder failure is the parse's error: these formats only go through
//! LibreOffice when the caller asks for it (`use_libreoffice`).

use std::path::Path;

use serde_json::Value;

use crate::parser::ScreenshotResult;
use crate::types::{GraphicPrimitive, Page, PdfInput, Rect, ScreenshotRect, TextItem};

/// Which decoder handles an input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// `.docx` and binary `.doc` (converted to `.docx` by `office::doc`).
    Word,
    /// `.pptx` and binary `.ppt` (converted to `.pptx` by `office::ppt`).
    Presentation,
}

impl Kind {
    /// The decoder for a file extension; covers exactly
    /// [`crate::conversion::NATIVE_OFFICE_EXTENSIONS`].
    fn from_extension(ext: &str) -> Option<Kind> {
        match ext.to_ascii_lowercase().as_str() {
            "docx" | "doc" => Some(Kind::Word),
            "pptx" | "ppt" => Some(Kind::Presentation),
            _ => None,
        }
    }

    /// The decoder for `input`, judged by file extension for paths and by
    /// content sniffing for bytes.
    pub fn detect(input: &PdfInput) -> Option<Kind> {
        match input {
            PdfInput::Path(p) => Path::new(p)
                .extension()
                .and_then(|e| e.to_str())
                .and_then(Kind::from_extension),
            PdfInput::Bytes(b) => crate::conversion::guess_extension_from_data(b)
                .as_deref()
                .and_then(Kind::from_extension),
        }
    }
}

/// Decoder output mapped to liteparse types.
#[derive(Debug, Default)]
pub struct Decoded {
    pub total_pages: u32,
    pub pages: Vec<Page>,
    pub creator: Option<String>,
    pub producer: Option<String>,
    /// One per rendered page, when [`Request::screenshots`] asked for them.
    pub screenshots: Vec<ScreenshotResult>,
    /// Placed pictures per page (viewport pt), aligned with `pages`, when
    /// [`Request::image_rects`] asked for them.
    pub image_rects: Vec<Vec<Rect>>,
}

/// What to ask the decoder for.
#[derive(Debug, Clone, Default)]
pub struct Request {
    /// 1-based page numbers, as liteparse's `target_pages` resolves them.
    pub target_pages: Option<Vec<u32>>,
    /// 0 means no limit.
    pub max_pages: usize,
    /// Text items and layout graphics. Off for screenshot-only calls.
    pub text: bool,
    /// Render PNG page screenshots.
    pub screenshots: Option<ScreenshotRequest>,
    /// Report where pictures sit on each page (no pixels).
    pub image_rects: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct ScreenshotRequest {
    pub dpi: f32,
    /// Detect solid rectangles and lines in the raster.
    pub detect_rects: bool,
}

/// Stack of the decoding thread. Parsing and layout recurse once per nesting
/// level of the document (tables in tables, groups in groups), bounded by the
/// decoders' own depth limits; the default 2 MiB of a blocking-pool thread is
/// not enough for the deepest documents they accept.
const DECODER_STACK_BYTES: usize = 256 * 1024 * 1024;

/// Run the decoder for `kind` on `input`. Blocking: call from `spawn_blocking`.
pub fn decode(kind: Kind, input: PdfInput, request: &Request) -> Result<Decoded, String> {
    let bytes = match input {
        PdfInput::Path(p) => std::fs::read(&p).map_err(|e| format!("cannot read {p}: {e}"))?,
        PdfInput::Bytes(b) => b,
    };
    // Screenshots are the one output the decoders still write as files.
    let shots_dir = request
        .screenshots
        .map(|_| {
            tempfile::Builder::new()
                .prefix("liteparse-office-shots-")
                .tempdir()
                .map_err(|e| format!("cannot create temp dir: {e}"))
        })
        .transpose()?;
    let config = decoder_config(request, shots_dir.as_ref().map(|d| d.path()));
    let run = std::thread::Builder::new()
        .name(format!("liteparse-{kind:?}").to_lowercase())
        .stack_size(DECODER_STACK_BYTES)
        .spawn(move || match kind {
            Kind::Word => super::docx::extract::read(&config, bytes),
            Kind::Presentation => super::pptx::extract::read(&config, bytes),
        })
        .map_err(|e| format!("cannot start decoder thread: {e}"))?
        .join();
    // The decoders catch their own layout panics; one that still escapes
    // becomes this document's error instead of unwinding through the caller.
    let extracted = match run {
        Ok(result) => result?,
        Err(_) => return Err("decoder panicked".into()),
    };
    let mut decoded = parse_output(&extracted.index, &extracted.pages)?;
    if let Some(dir) = &shots_dir {
        for shot in &mut decoded.screenshots {
            let path = dir.path().join(format!("page_{}.png", shot.page_num));
            shot.image_bytes = std::fs::read(&path)
                .map_err(|e| format!("cannot read screenshot {}: {e}", path.display()))?;
        }
    }
    Ok(decoded)
}

fn decoder_config(request: &Request, shots_dir: Option<&Path>) -> String {
    let mut config = serde_json::json!({
        // Verbose adds fontSize and fontAscent to every text item.
        "verbose": true,
        "text": { "enabled": request.text },
        // Table borders, rules and cell fills for the markdown layout pass.
        "path": { "enabled": request.text },
        "image": { "enabled": request.image_rects, "metadataOnly": true },
        "maxPages": request.max_pages.min(u32::MAX as usize),
    });
    if let (Some(shots), Some(dir)) = (request.screenshots, shots_dir) {
        config["screenshot"] = serde_json::json!({
            "enabled": true,
            "format": "png",
            "dpi": shots.dpi,
            // liteparse renders at the requested DPI without a size cap.
            "maxEdge": u32::MAX,
            "outputDir": dir.to_string_lossy(),
            "detectRectangles": shots.detect_rects,
            "detectLines": shots.detect_rects,
        });
    }
    if let Some(targets) = &request.target_pages {
        // The decoders take 0-based page specs.
        let spec = targets
            .iter()
            .filter(|p| **p > 0)
            .map(|p| (p - 1).to_string())
            .collect::<Vec<_>>()
            .join(",");
        config["targetPages"] = Value::String(spec);
    }
    config.to_string()
}

/// Map the extract protocol (the index object and the page records, one
/// JSON object per line) to liteparse pages.
pub fn parse_output(index: &Value, pages: &[u8]) -> Result<Decoded, String> {
    if let Some(err) = index.get("error").and_then(Value::as_str) {
        return Err(match index.get("errorDetail").and_then(Value::as_str) {
            Some(detail) if detail != err => format!("{err}: {detail}"),
            _ => err.to_string(),
        });
    }
    let string = |key: &str| index.get(key).and_then(Value::as_str).map(str::to_string);
    let mut out = Decoded {
        total_pages: index
            .get("totalPages")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            .min(u32::MAX as u64) as u32,
        creator: string("creator"),
        producer: string("producer"),
        pages: Vec::new(),
        screenshots: Vec::new(),
        image_rects: Vec::new(),
    };
    for line in pages.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
        let page: Value = serde_json::from_slice(line).map_err(|e| format!("bad page: {e}"))?;
        if let Some(err) = page.get("error").and_then(Value::as_str) {
            return Err(format!("page error: {err}"));
        }
        let page_record = page_from_json(&page);
        if let Some(shot) = page.get("screenshot") {
            out.screenshots
                .push(screenshot_from_json(shot, page_record.page_number as u32));
        }
        if page.get("images").is_some() {
            out.image_rects.push(
                array(Some(&page), "images")
                    .filter_map(|i| rect(i.get("coords")))
                    .collect(),
            );
        }
        out.pages.push(page_record);
    }
    Ok(out)
}

fn num(v: &Value, key: &str) -> Option<f32> {
    v.get(key).and_then(Value::as_f64).map(|f| f as f32)
}

fn rect(v: Option<&Value>) -> Option<Rect> {
    let v = v?;
    Some(Rect {
        x: num(v, "x")?,
        y: num(v, "y")?,
        width: num(v, "w")?,
        height: num(v, "h")?,
    })
}

/// Screenshot metadata; `image_bytes` is filled from the PNG file afterwards.
fn screenshot_from_json(shot: &Value, page_num: u32) -> ScreenshotResult {
    let px = |key: &str| shot.get(key).and_then(Value::as_u64).unwrap_or(0) as u32;
    ScreenshotResult {
        page_num,
        width: px("width"),
        height: px("height"),
        image_bytes: Vec::new(),
        is_solid_fill: flag(shot, "isSolidFill"),
        rects: array(Some(shot), "rects")
            .filter_map(|r| {
                let b = rect(Some(r))?;
                Some(ScreenshotRect {
                    x: b.x,
                    y: b.y,
                    width: b.width,
                    height: b.height,
                    color: argb(r.get("color").and_then(Value::as_str).unwrap_or("")),
                    is_line: flag(r, "isLine"),
                })
            })
            .collect(),
    }
}

/// The protocol writes raster colors as `0x00RRGGBB` with an unused alpha;
/// liteparse reports opaque lowercase ARGB hex (`ffrrggbb`).
fn argb(c: &str) -> String {
    let hex = c.trim_start_matches("0x").to_ascii_lowercase();
    match hex.len() {
        8 => format!("ff{}", &hex[2..]),
        6 => format!("ff{hex}"),
        _ => hex,
    }
}

// Every coordinate the decoders write is already relative to the page's
// top-left corner (the viewport origin is reported as 0,0).
fn page_from_json(page: &Value) -> Page {
    let viewport = page.get("viewport");
    let text = page.get("text");
    let colors: Vec<&str> = text
        .and_then(|t| t.get("colors"))
        .and_then(Value::as_array)
        .map(|c| c.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let text_items = array(text, "items")
        .filter_map(|item| text_item(item, &colors))
        .collect();
    Page {
        page_number: page.get("page").and_then(Value::as_u64).unwrap_or(0) as usize + 1,
        page_width: viewport.and_then(|v| num(v, "w")).unwrap_or(0.0),
        page_height: viewport.and_then(|v| num(v, "h")).unwrap_or(0.0),
        content_bounds: rect(page.get("contentBounds")),
        text_items,
        graphics: graphics(page.get("paths")),
        ..Default::default()
    }
}

fn array<'a>(parent: Option<&'a Value>, key: &str) -> impl Iterator<Item = &'a Value> {
    parent
        .and_then(|p| p.get(key))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
}

fn color(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_string)
}

fn flag(v: &Value, key: &str) -> bool {
    v.get(key).and_then(Value::as_bool).unwrap_or(false)
}

/// The layout graphics PDF extraction would build from the same drawing
/// (`extract_layout_graphics`): one `Rect` per filled shape, one `Stroke`
/// per stroked straight segment. The protocol only reports axis-aligned
/// segments, which are the ones table and rule detection use.
fn graphics(paths: Option<&Value>) -> Vec<GraphicPrimitive> {
    let rects = array(paths, "shapes")
        .filter(|s| flag(s, "fill"))
        .filter_map(|s| {
            let bbox = rect(s.get("coords"))?;
            let fill = color(s, "fillColor");
            if !flag(s, "hasCurve")
                && let Some(rule) = hairline(&bbox, fill.clone())
            {
                return Some(rule);
            }
            Some(GraphicPrimitive::Rect {
                bbox,
                fill,
                stroke: flag(s, "stroke").then(|| color(s, "strokeColor")).flatten(),
            })
        });
    let strokes = array(paths, "lines")
        .filter(|l| flag(l, "stroke"))
        .filter_map(|l| {
            Some(GraphicPrimitive::Stroke {
                x1: num(l, "x1")?,
                y1: num(l, "y1")?,
                x2: num(l, "x2")?,
                y2: num(l, "y2")?,
                color: color(l, "strokeColor"),
                width: num(l, "strokeWidth").unwrap_or(1.0),
            })
        });
    rects.chain(strokes).collect()
}

/// Thickest filled bar read as a line, the same limit horizontal-rule
/// detection puts on strokes.
const HAIRLINE_MAX_PT: f32 = 2.0;

/// Word draws paragraph and table-cell borders as thin filled bars where a
/// PDF producer strokes a line. Report such a bar as the stroke along its
/// centre line, so rule and ruled-table detection see what they see in PDFs.
fn hairline(b: &Rect, color: Option<String>) -> Option<GraphicPrimitive> {
    let (thin, long) = (b.width.min(b.height), b.width.max(b.height));
    if thin > HAIRLINE_MAX_PT || long < 4.0 * thin.max(0.25) {
        return None;
    }
    let (x1, y1, x2, y2) = if b.width >= b.height {
        let y = b.y + b.height / 2.0;
        (b.x, y, b.x + b.width, y)
    } else {
        let x = b.x + b.width / 2.0;
        (x, b.y, x, b.y + b.height)
    };
    Some(GraphicPrimitive::Stroke {
        x1,
        y1,
        x2,
        y2,
        color,
        width: thin,
    })
}

fn text_item(item: &Value, colors: &[&str]) -> Option<TextItem> {
    let text = item.get("unicode").and_then(Value::as_str)?.to_string();
    let coords = rect(item.get("coords"))?;
    // The protocol's angle is clockwise; liteparse's rotation is counter-clockwise.
    let angle = num(item, "angle").unwrap_or(0.0);
    let rotation = (360.0 - angle).rem_euclid(360.0);
    let font_height = num(item, "fontHeight");
    Some(TextItem {
        text,
        x: coords.x,
        y: coords.y,
        width: coords.width,
        height: coords.height,
        rotation: if rotation.abs() < 1e-3 || rotation > 360.0 - 1e-3 {
            0.0
        } else {
            rotation
        },
        font_name: color(item, "fontName"),
        font_size: num(item, "fontSize").or(font_height),
        font_height,
        font_ascent: num(item, "fontAscent"),
        font_descent: num(item, "fontDescent"),
        text_width: num(item, "textWidth"),
        font_is_buggy: flag(item, "fontIsBuggy"),
        fill_color: item
            .get("c")
            .and_then(Value::as_u64)
            .and_then(|i| colors.get(i as usize))
            .map(|c| c.to_string()),
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_kind_by_extension() {
        let kind = |p: &str| Kind::detect(&PdfInput::Path(p.into()));
        assert_eq!(kind("a.DOCX"), Some(Kind::Word));
        assert_eq!(kind("a.doc"), Some(Kind::Word));
        assert_eq!(kind("a.pptx"), Some(Kind::Presentation));
        assert_eq!(kind("a.ppt"), Some(Kind::Presentation));
        assert_eq!(kind("a.pdf"), None);
        assert_eq!(kind("a.odt"), None);
        for ext in crate::conversion::NATIVE_OFFICE_EXTENSIONS {
            assert!(Kind::from_extension(ext).is_some(), "{ext}");
        }
    }

    #[test]
    fn target_pages_become_zero_based() {
        let cfg = decoder_config(
            &Request {
                target_pages: Some(vec![1, 3]),
                max_pages: 5,
                text: true,
                ..Default::default()
            },
            None,
        );
        let v: Value = serde_json::from_str(&cfg).unwrap();
        assert_eq!(v["targetPages"], "0,2");
        assert_eq!(v["maxPages"], 5);
        assert!(v.get("screenshot").is_none());
    }

    #[test]
    fn maps_protocol_items() {
        let index = r#"{"totalPages":3,"creator":"Word","index":[]}"#;
        let pages = r#"{"page":1,"viewport":{"x":0,"y":0,"w":612,"h":792},"text":{"colors":["ff000000"],"items":[{"unicode":"Hi","angle":90.0,"textWidth":10,"coords":{"x":5,"y":6,"w":10,"h":12},"fontName":"Carlito","fontHeight":11,"fontDescent":2,"fontIsBuggy":false,"c":0}]}}"#;
        let d = parse_output(&serde_json::from_str(index).unwrap(), pages.as_bytes()).unwrap();
        assert_eq!(d.total_pages, 3);
        assert_eq!(d.creator.as_deref(), Some("Word"));
        let p = &d.pages[0];
        assert_eq!(p.page_number, 2);
        assert_eq!((p.page_width, p.page_height), (612.0, 792.0));
        let t = &p.text_items[0];
        assert_eq!(t.text, "Hi");
        assert_eq!((t.x, t.y, t.width, t.height), (5.0, 6.0, 10.0, 12.0));
        assert_eq!(t.rotation, 270.0);
        assert_eq!(t.font_size, Some(11.0));
        assert_eq!(t.fill_color.as_deref(), Some("ff000000"));
    }

    #[test]
    fn maps_paths_to_layout_graphics() {
        let paths: Value = serde_json::from_str(
            r#"{"shapes":[{"coords":{"x":1,"y":2,"w":3,"h":4},"stroke":false,"fill":true,"fillColor":"ffff0000"},{"coords":{"x":0,"y":0,"w":9,"h":0},"stroke":true,"fill":false}],
               "lines":[{"x1":0,"y1":0,"x2":9,"y2":0,"stroke":true,"strokeWidth":0.5,"strokeColor":"ff000000","fill":false},{"x1":1,"y1":2,"x2":4,"y2":2,"stroke":false,"fill":true}]}"#,
        )
        .unwrap();
        let g = graphics(Some(&paths));
        assert_eq!(g.len(), 2);
        assert!(
            matches!(&g[0], GraphicPrimitive::Rect { fill: Some(f), stroke: None, .. } if f == "ffff0000")
        );
        assert!(
            matches!(&g[1], GraphicPrimitive::Stroke { x2, width, .. } if *x2 == 9.0 && *width == 0.5)
        );
    }

    #[test]
    fn thin_filled_bars_become_strokes() {
        let bar = Rect {
            x: 90.0,
            y: 100.0,
            width: 432.0,
            height: 1.0,
        };
        assert!(matches!(
            hairline(&bar, None),
            Some(GraphicPrimitive::Stroke { y1, y2, width, .. }) if y1 == 100.5 && y2 == 100.5 && width == 1.0
        ));
        let col = Rect {
            x: 10.0,
            y: 0.0,
            width: 0.5,
            height: 13.0,
        };
        assert!(
            matches!(hairline(&col, None), Some(GraphicPrimitive::Stroke { x1, x2, .. }) if x1 == 10.25 && x2 == 10.25)
        );
        let cell = Rect {
            x: 0.0,
            y: 0.0,
            width: 40.0,
            height: 12.0,
        };
        assert!(hairline(&cell, None).is_none());
        let dot = Rect {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        };
        assert!(hairline(&dot, None).is_none());
    }

    #[test]
    fn maps_screenshot_metadata() {
        let shot: Value = serde_json::from_str(
            r#"{"width":10,"height":20,"isSolidFill":true,"rects":[{"x":1,"y":2,"w":3,"h":0.5,"color":"0x00808080","isLine":true}]}"#,
        )
        .unwrap();
        let s = screenshot_from_json(&shot, 4);
        assert_eq!(
            (s.page_num, s.width, s.height, s.is_solid_fill),
            (4, 10, 20, true)
        );
        assert_eq!(s.rects[0].color, "ff808080");
        assert!(s.rects[0].is_line);
    }

    #[test]
    fn document_error_is_reported() {
        let index = serde_json::json!({"totalPages": 0, "error": "protected"});
        let err = parse_output(&index, b"").unwrap_err();
        assert_eq!(err, "protected");
    }
}
