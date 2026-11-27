//! JSON for one page record, in the extract protocol's shapes
//! (`PdfiumExtractPage` in `llamaparse/worker/src/utils/pdfium.ts`), plus the
//! pptx `presentation` block.

use std::collections::HashMap;
use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

use crate::office::pptx::raster::{BPP, find_solid_rects, is_solid_fill, page_pixel_size};
use serde_json::{Map, Value, json};
use tiny_skia::PathSegment;

use crate::office::pptx::color::Rgba;
use crate::office::pptx::config::Config;
use crate::office::pptx::document::Document;
use crate::office::pptx::fonts::FontSystem;
use crate::office::pptx::render::{self, ImageCache, RenderTarget};
use crate::office::pptx::scene::{Item, Paint, Rect, Scene};
use crate::office::pptx::textlayout::transformed_bbox;

fn r3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

fn coords(r: &Rect, ox: f64, oy: f64) -> Value {
    json!({ "x": r3(r.x - ox), "y": r3(r.y - oy), "w": r3(r.w), "h": r3(r.h) })
}

fn argb(c: Rgba) -> String {
    c.to_argb_hex().to_lowercase()
}

// ---------------------------------------------------------------- text

/// Text items of a scene. With `clip` (the page size, `outOfBoundsContent:
/// "clip"`), items lying wholly off the page are dropped: the LibreOffice PDF
/// they replace did not carry off-slide text either.
pub fn text_json(
    fonts: &FontSystem,
    scene: &Scene,
    verbose: bool,
    (ox, oy): (f64, f64),
    clip: Option<(f64, f64)>,
) -> Value {
    let mut text = source_text_json(fonts, scene, verbose, (ox, oy), clip);
    if let Some(items) = text.get_mut("items").and_then(Value::as_array_mut) {
        for item in items {
            if let Some(item) = item.as_object_mut() {
                item.remove("nativePaintGroup");
            }
        }
    }
    text
}

/// DOCX's intermediate text transport includes painted-run identity for its
/// deduplication pass. That pass consumes the identity before page serialization.
pub fn source_text_json(
    fonts: &FontSystem,
    scene: &Scene,
    verbose: bool,
    (ox, oy): (f64, f64),
    clip: Option<(f64, f64)>,
) -> Value {
    let mut palette: Vec<String> = Vec::new();
    let mut items = Vec::new();
    let mut standalone = Vec::new();
    for (paint_group, item) in scene.items.iter().enumerate() {
        if let Item::Bookmark(marker) = item {
            let mut value = bookmark_json(&marker.source, marker.point);
            value["textItemBoundary"] = json!(items.len());
            standalone.push(value);
            continue;
        }
        let Item::Text(run) = item else { continue };
        let bookmark_transform = run.transform;
        let fragments = source_fragments(run);
        for run in fragments.as_deref().unwrap_or(std::slice::from_ref(run)) {
            if run.text.is_empty() || (run.text.trim().is_empty() && !run.preserve_whitespace) {
                continue;
            }
            let t = &run.transform;
            // Baseline direction (1, 0) in slide space gives the clockwise angle.
            let (dx, dy) = (t.sx as f64, t.ky as f64);
            let angle = dy.atan2(dx).to_degrees().rem_euclid(360.0);
            let angle = if (angle - 360.0).abs() < 1e-6 {
                0.0
            } else {
                angle
            };
            let scale = (dx * dx + dy * dy).sqrt();
            let Some(bbox) =
                transformed_bbox(t, 0.0, -run.ascent, run.width, run.ascent + run.descent)
            else {
                continue;
            };
            if let Some((w, h)) = clip {
                let (x, y) = (bbox.x - ox, bbox.y - oy);
                if x >= w || y >= h || x + bbox.w <= 0.0 || y + bbox.h <= 0.0 {
                    continue;
                }
            }
            let mut v = Map::new();
            v.insert("unicode".into(), json!(run.text));
            if let Some(bookmarks) = &run.bookmarks {
                v.insert(
                    "nativeBookmarks".into(),
                    Value::Array(
                        bookmarks
                            .boundaries
                            .iter()
                            .map(|marker| {
                                let mut point = marker.point;
                                bookmark_transform.map_point(&mut point);
                                let mut value = bookmark_json(&marker.source, point);
                                value["utf16Offset"] = json!(marker.offset);
                                value
                            })
                            .collect(),
                    ),
                );
            }
            if let Some(source) = &run.source {
                v.insert("nativeSource".into(), json!({
                "part": source.part,
                "paragraph": source.paragraph,
                "cells": source.cells.iter().map(source_cell_json).collect::<Vec<_>>(),
                "link": source.link.as_ref().map(|link| match &link.target {
                    crate::office::pptx::scene::SourceLinkTarget::ExternalUri(uri) => json!({"id":link.id,"uri":uri}),
                    crate::office::pptx::scene::SourceLinkTarget::Bookmark { part, name } => json!({"id":link.id,"destination":{"kind":"bookmark","sourcePart":part,"bookmarkName":name}}),
                }),
            }));
            }
            v.insert("angle".into(), json!(r3(angle)));
            v.insert("textWidth".into(), json!(r3(run.width * scale)));
            v.insert("coords".into(), coords(&bbox, ox, oy));
            if let Some(g) = grounding(fonts, run) {
                // Literal box centred on the ink centre, unrotated (consumers rotate by angle).
                let mut c = [tiny_skia::Point::from_xy(
                    ((g.0 + g.2) / 2.0) as f32,
                    ((g.1 + g.3) / 2.0) as f32,
                )];
                t.map_points(&mut c);
                let (w, h) = ((g.2 - g.0) * scale, (g.3 - g.1) * scale);
                v.insert(
                "groundingCoords".into(),
                json!({ "x": r3(c[0].x as f64 - w / 2.0 - ox), "y": r3(c[0].y as f64 - h / 2.0 - oy), "w": r3(w), "h": r3(h) }),
            );
            }
            let font_name = fonts
                .postscript_name(run.face)
                .unwrap_or_else(|| run.font_name.clone());
            v.insert("fontName".into(), json!(font_name));
            if verbose {
                v.insert("fontSize".into(), json!(r3(run.size)));
                v.insert("fontAscent".into(), json!(r3(run.ascent * scale)));
            }
            v.insert("fontHeight".into(), json!(r3(run.size * scale)));
            v.insert("fontDescent".into(), json!(r3(run.descent * scale)));
            v.insert("fontIsBuggy".into(), json!(false));
            if run.color.a > 0.0 {
                let hex = argb(run.color);
                let idx = match palette.iter().position(|p| *p == hex) {
                    Some(i) => i,
                    None => {
                        palette.push(hex);
                        palette.len() - 1
                    }
                };
                v.insert("c".into(), json!(idx));
            }
            if fragments.is_some() {
                // DOCX consumes this group before export to retain distinct logical
                // fragments whose condensed painted positions share a dedupe bucket.
                v.insert("nativePaintGroup".into(), json!(paint_group));
            }
            items.push(Value::Object(v));
        }
    }
    let mut out = Map::new();
    if !palette.is_empty() {
        out.insert("colors".into(), json!(palette));
    }
    out.insert("items".into(), Value::Array(items));
    if !standalone.is_empty() {
        out.insert("nativeStandaloneBookmarks".into(), json!(standalone));
    }
    Value::Object(out)
}

/// Export logical ownership without changing the single shaped and painted run.
/// Boundaries inside a glyph cluster share that cluster's geometry.
fn source_fragments(
    run: &crate::office::pptx::scene::GlyphRun,
) -> Option<Vec<crate::office::pptx::scene::GlyphRun>> {
    let spans = &run.bookmarks.as_ref()?.source_spans;
    if spans.is_empty() {
        return None;
    }
    let mut grouped: Vec<crate::office::pptx::scene::RunSourceSpan> = Vec::new();
    for span in spans.iter() {
        if let Some(last) = grouped
            .last_mut()
            .filter(|last| last.source == span.source && last.text.end == span.text.start)
        {
            last.text.end = span.text.end;
            last.glyphs.end = last.glyphs.end.max(span.glyphs.end);
            let right = (last.x + last.width).max(span.x + span.width);
            last.x = last.x.min(span.x);
            last.width = right - last.x;
        } else {
            grouped.push(span.clone());
        }
    }
    let mut template = run.clone();
    template.glyphs.clear();
    template.text.clear();
    template.bookmarks = None;
    let mut markers = run
        .bookmarks
        .as_ref()
        .map(|markers| markers.boundaries.clone())
        .unwrap_or_default();
    markers.sort_by_key(|marker| marker.offset);
    let mut utf16_start = 0;
    Some(
        grouped
            .into_iter()
            .map(|span| {
                let mut fragment = template.clone();
                fragment.source = span.source;
                fragment.text = run.text[span.text.clone()].to_owned();
                fragment.preserve_whitespace = true;
                fragment.glyphs = run.glyphs[span.glyphs].to_vec();
                for glyph in &mut fragment.glyphs {
                    glyph.x -= span.x;
                }
                fragment.transform = run.transform.pre_translate(span.x as f32, 0.0);
                fragment.width = span.width;
                let start = utf16_start;
                let end = start + fragment.text.encode_utf16().count();
                utf16_start = end;
                let first = markers.partition_point(|marker| marker.offset < start);
                let last = markers.partition_point(|marker| {
                    marker.offset < end || (marker.offset == end && span.text.end == run.text.len())
                });
                if first < last {
                    let boundaries = markers[first..last]
                        .iter()
                        .cloned()
                        .map(|mut marker| {
                            marker.offset -= start;
                            marker
                        })
                        .collect();
                    fragment.bookmarks = Some(Box::new(crate::office::pptx::scene::RunBookmarks {
                        boundaries,
                        ..Default::default()
                    }));
                }
                fragment
            })
            .collect(),
    )
}

/// Ink bounds of a run in run-local space (x0, y0, x1, y1; y down).
fn grounding(
    fonts: &FontSystem,
    run: &crate::office::pptx::scene::GlyphRun,
) -> Option<(f64, f64, f64, f64)> {
    let fd = fonts.face(run.face)?;
    let scale = run.size / fd.metrics.units_per_em as f64;
    let mut b: Option<(f64, f64, f64, f64)> = None;
    for g in &run.glyphs {
        let Some(path) = fonts.outline(run.face, g.id) else {
            continue;
        };
        let pb = path.bounds();
        let x0 = g.x + pb.left() as f64 * scale;
        let x1 = g.x + pb.right() as f64 * scale;
        // Font units are y-up.
        let y0 = g.y - pb.bottom() as f64 * scale;
        let y1 = g.y - pb.top() as f64 * scale;
        b = Some(match b {
            Some(a) => (a.0.min(x0), a.1.min(y0), a.2.max(x1), a.3.max(y1)),
            None => (x0, y0, x1, y1),
        });
    }
    b
}

// ---------------------------------------------------------------- images

/// Images too small to matter, matching `llamaparse-read/src/image.rs`.
const MIN_PIXEL_EDGE: u32 = 12;
const MIN_PIXEL_AREA: u64 = 200;
const MIN_PLACED_EDGE: f64 = 6.0;
const MIN_PLACED_AREA: f64 = 200.0;

pub fn images_json(
    doc: &Document,
    images: &ImageCache,
    cfg: &Config,
    scene: &Scene,
    page: u32,
    ox: f64,
    oy: f64,
) -> (Vec<Value>, u32) {
    let mut out = Vec::new();
    let mut errors = 0;
    let mut written: HashMap<String, (String, String)> = HashMap::new();
    let page_label = if cfg.image.one_index_pages {
        page + 1
    } else {
        page
    };
    let view = Rect {
        x: ox,
        y: oy,
        w: scene.width,
        h: scene.height,
    };
    for item in &scene.items {
        if out.len() >= cfg.image.max_per_page as usize {
            break;
        }
        let Item::Image(img) = item else { continue };
        let Some(bbox) = transformed_bbox(&img.transform, 0.0, 0.0, 1.0, 1.0) else {
            continue;
        };
        if bbox.w < MIN_PLACED_EDGE
            || bbox.h < MIN_PLACED_EDGE
            || bbox.w * bbox.h < MIN_PLACED_AREA
            || !bbox.intersects(&view)
        {
            continue;
        }
        let Some(pm) = images.get_part(doc, &img.image.part) else {
            errors += 1;
            continue;
        };
        let (w, h) = images
            .native_size(doc, &img.image.part)
            .unwrap_or((pm.width(), pm.height()));
        if w < MIN_PIXEL_EDGE || h < MIN_PIXEL_EDGE || (w as u64 * h as u64) < MIN_PIXEL_AREA {
            continue;
        }
        let rotation = (img.transform.ky as f64)
            .atan2(img.transform.sx as f64)
            .to_degrees();
        let mut v = Map::new();
        let n = out.len();
        if cfg.image.metadata_only {
            v.insert("name".into(), json!(format!("img_p{page_label}_{}", n + 1)));
        } else {
            let (name, path) = match written.get(&img.image.part) {
                Some(hit) => hit.clone(),
                None => {
                    let Some(dir) = &cfg.image.output_dir else {
                        break;
                    };
                    let bytes = doc.pkg.read(&img.image.part).unwrap_or_default();
                    let native = native_ext(&bytes);
                    let ext = native.unwrap_or("png");
                    let name = format!("img_p{page_label}_{}.{ext}", n + 1);
                    let path = Path::new(dir).join(&name);
                    let ok = match native {
                        Some(_) => std::fs::write(&path, &bytes).is_ok(),
                        None => write_png_rgba(&path, &pm, cfg.png_level()),
                    };
                    if !ok {
                        errors += 1;
                        continue;
                    }
                    let entry = (name, path.to_string_lossy().into_owned());
                    written.insert(img.image.part.clone(), entry.clone());
                    entry
                }
            };
            v.insert("name".into(), json!(name));
            v.insert("path".into(), json!(path));
        }
        v.insert("width".into(), json!(w));
        v.insert("height".into(), json!(h));
        v.insert("coords".into(), coords(&bbox, ox, oy));
        v.insert("rotation".into(), json!(r3(-rotation)));
        if let Some(alt) = &img.alt_text {
            v.insert("altText".into(), json!(alt));
        }
        out.push(Value::Object(v));
    }
    (out, errors)
}

/// JPEG and PNG pictures are copied byte for byte; anything else is re-encoded.
fn native_ext(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("jpg")
    } else if bytes.starts_with(b"\x89PNG") {
        Some("png")
    } else {
        None
    }
}

fn write_png_rgba(path: &Path, pm: &tiny_skia::Pixmap, level: u8) -> bool {
    let Ok(file) = File::create(path) else {
        return false;
    };
    let mut enc = png::Encoder::new(BufWriter::new(file), pm.width(), pm.height());
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_compression(if level >= 7 {
        png::Compression::High
    } else {
        png::Compression::Fast
    });
    let Ok(mut w) = enc.write_header() else {
        return false;
    };
    // Un-premultiply.
    let mut data = pm.data().to_vec();
    for px in data.as_chunks_mut::<4>().0 {
        let a = px[3] as u32;
        if a > 0 && a < 255 {
            for c in &mut px[..3] {
                *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
    w.write_image_data(&data).is_ok() && w.finish().is_ok()
}

// ---------------------------------------------------------------- paths

/// The page's `paths` value as compact JSON bytes, `{"shapes":[...],"lines":[...]}`.
/// Each entry is serialized as soon as it is built: a vector-heavy slide reports
/// 100k paths, whose `Value` tree would cost hundreds of MB.
pub fn paths_json(scene: &Scene, max: usize, ox: f64, oy: f64) -> Vec<u8> {
    let mut shapes = JsonArray::default();
    let mut lines = JsonArray::default();
    for item in &scene.items {
        let Item::Path(p) = item else { continue };
        if shapes.len() >= max {
            break;
        }
        let fill = p.fill.as_ref().filter(|f| f.is_visible());
        let stroke = p.stroke.as_ref().filter(|s| s.paint.is_visible());
        if fill.is_none() && stroke.is_none() {
            continue;
        }
        let b = p.path.bounds();
        let r = Rect {
            x: b.x() as f64,
            y: b.y() as f64,
            w: b.width() as f64,
            h: b.height() as f64,
        };
        let mut v = Map::new();
        v.insert("coords".into(), coords(&r, ox, oy));
        v.insert("stroke".into(), json!(stroke.is_some()));
        if let Some(c) = stroke.and_then(|s| s.paint.representative()) {
            v.insert("strokeColor".into(), json!(argb(c)));
        }
        v.insert("fill".into(), json!(fill.is_some()));
        if let Some(c) = fill.and_then(solid_color) {
            v.insert("fillColor".into(), json!(argb(c)));
        }
        v.insert("hasCurve".into(), json!(p.has_curve));
        shapes.push(&Value::Object(v));

        // Axis-aligned straight segments become `lines`.
        let mut start = tiny_skia::Point::zero();
        let mut cur = tiny_skia::Point::zero();
        for seg in p.path.segments() {
            let next = match seg {
                PathSegment::MoveTo(pt) => {
                    start = pt;
                    cur = pt;
                    continue;
                }
                PathSegment::LineTo(pt) => pt,
                PathSegment::Close => start,
                PathSegment::QuadTo(_, pt) | PathSegment::CubicTo(_, _, pt) => {
                    cur = pt;
                    continue;
                }
            };
            let horizontal = (next.y - cur.y).abs() < 0.01;
            let vertical = (next.x - cur.x).abs() < 0.01;
            if (horizontal || vertical) && (next.x - cur.x).abs() + (next.y - cur.y).abs() > 0.01 {
                let mut l = Map::new();
                l.insert("x1".into(), json!(r3(cur.x as f64 - ox)));
                l.insert("y1".into(), json!(r3(cur.y as f64 - oy)));
                l.insert("x2".into(), json!(r3(next.x as f64 - ox)));
                l.insert("y2".into(), json!(r3(next.y as f64 - oy)));
                l.insert("stroke".into(), json!(stroke.is_some()));
                if let Some(s) = stroke {
                    l.insert("strokeWidth".into(), json!(r3(s.width)));
                    if let Some(c) = s.paint.representative() {
                        l.insert("strokeColor".into(), json!(argb(c)));
                    }
                }
                l.insert("fill".into(), json!(fill.is_some()));
                if let Some(c) = fill.and_then(solid_color) {
                    l.insert("fillColor".into(), json!(argb(c)));
                }
                lines.push(&Value::Object(l));
            }
            cur = next;
        }
    }
    let mut out = Vec::with_capacity(shapes.0.len() + lines.0.len() + 24);
    out.extend_from_slice(b"{\"shapes\":");
    out.extend_from_slice(&shapes.finish());
    out.extend_from_slice(b",\"lines\":");
    out.extend_from_slice(&lines.finish());
    out.push(b'}');
    out
}

/// A JSON array written element by element, byte-identical to serializing a
/// `Value::Array` of the same elements.
#[derive(Default)]
struct JsonArray(Vec<u8>, usize);

impl JsonArray {
    fn push(&mut self, v: &Value) {
        self.0.push(if self.0.is_empty() { b'[' } else { b',' });
        // Writing a `Value` into a Vec cannot fail.
        drop(serde_json::to_writer(&mut self.0, v));
        self.1 += 1;
    }

    fn len(&self) -> usize {
        self.1
    }

    fn finish(mut self) -> Vec<u8> {
        if self.0.is_empty() {
            self.0.push(b'[');
        }
        self.0.push(b']');
        self.0
    }
}

/// Only flat fills report a color, as pdfium does for pattern/gradient colorspaces.
fn solid_color(p: &Paint) -> Option<Rgba> {
    match p {
        Paint::Solid(c) => Some(*c),
        _ => None,
    }
}

// ---------------------------------------------------------------- screenshot

/// A rendered page screenshot whose encoding and rectangle detection are still
/// to do. It owns everything it needs, so `finish` can run on another thread.
pub struct PendingShot {
    pm: tiny_skia::Pixmap,
    width: i32,
    height: i32,
    dpi: i32,
    png: bool,
    png_level: u8,
    jpeg_quality: u8,
    name: String,
    path: std::path::PathBuf,
    region: Rect,
    frame: Rect,
    detect_rectangles: bool,
    detect_lines: bool,
}

/// Renders the page screenshot (the part that needs the document and caches).
#[expect(
    clippy::too_many_arguments,
    reason = "the screenshot needs the document, its caches, the scene and the page frame"
)]
pub fn render_screenshot(
    doc: &Document,
    fonts: &FontSystem,
    images: &ImageCache,
    cfg: &Config,
    scene: &Scene,
    frame: &Rect,
    page: u32,
    layers: &crate::office::pptx::layers::LayerCache,
) -> Option<PendingShot> {
    let sc = &cfg.screenshot;
    let dir = sc.output_dir.as_ref()?;
    // Crop fractions cut each side of the page.
    let region = match sc.bbox {
        Some(b) if b.top != 0.0 || b.bottom != 0.0 || b.left != 0.0 || b.right != 0.0 => Rect {
            x: frame.x + frame.w * b.left as f64,
            y: frame.y + frame.h * b.top as f64,
            w: frame.w * (1.0 - b.left as f64 - b.right as f64),
            h: frame.h * (1.0 - b.top as f64 - b.bottom as f64),
        },
        _ => *frame,
    };
    if region.w <= 0.0 || region.h <= 0.0 {
        return None;
    }
    let (dpi, w, h) = page_pixel_size(
        region.w as f32,
        region.h as f32,
        sc.dpi.round() as i32,
        sc.max_edge as i32,
    );
    if w <= 0 || h <= 0 {
        return None;
    }
    let target = RenderTarget {
        region,
        width: w as u32,
        height: h as u32,
    };
    let pm = render::render_layered(scene, &target, doc, fonts, images, Some(layers))?;
    let png = sc.format.eq_ignore_ascii_case("png");
    let name = format!("page_{}.{}", page + 1, if png { "png" } else { "jpg" });
    Some(PendingShot {
        pm,
        width: w,
        height: h,
        dpi,
        png,
        png_level: cfg.png_level(),
        jpeg_quality: cfg.jpeg_quality(),
        path: Path::new(dir).join(&name),
        name,
        region,
        frame: *frame,
        detect_rectangles: sc.detect_rectangles,
        detect_lines: sc.detect_lines,
    })
}

impl PendingShot {
    /// Encodes the raster and builds the `screenshot` record (with its solid
    /// rectangles); `None` when the file cannot be written.
    pub fn finish(self) -> Option<(Value, String)> {
        let (w, h, region, frame) = (self.width, self.height, self.region, self.frame);
        let bgr = render::to_bgr(&self.pm);
        drop(self.pm);
        let path = &self.path;
        let (wu, hu) = (w as usize, h as usize);
        let ok = if self.png {
            write_png_bgr(path, &bgr, wu, hu, self.png_level)
        } else {
            write_jpeg(path, &bgr, wu, hu, self.dpi, self.jpeg_quality)
        };
        if !ok {
            return None;
        }
        let mut shot = Map::new();
        shot.insert("width".into(), json!(w));
        shot.insert("height".into(), json!(h));
        shot.insert("renderDPI".into(), json!(self.dpi));
        shot.insert("name".into(), json!(self.name));
        shot.insert("path".into(), json!(path.to_string_lossy()));
        shot.insert("coords".into(), coords(&region, frame.x, frame.y));
        let solid = is_solid_fill(&bgr, wu, hu, wu * BPP);
        shot.insert("isSolidFill".into(), json!(solid));
        if !solid && (self.detect_rectangles || self.detect_lines) {
            let rects: Vec<Value> = find_solid_rects(
                &bgr,
                wu,
                hu,
                wu * BPP,
                self.detect_rectangles,
                self.detect_lines,
            )
            .into_iter()
            .map(|r| {
                json!({
                    "x": r3(r.left as f64 / w as f64 * region.w + region.x - frame.x),
                    "y": r3(r.top as f64 / h as f64 * region.h + region.y - frame.y),
                    "w": r3(r.width as f64 / w as f64 * region.w),
                    "h": r3(r.height as f64 / h as f64 * region.h),
                    "color": format!("0x{:08X}", r.color),
                })
            })
            .collect();
            shot.insert("rects".into(), Value::Array(rects));
        }
        Some((Value::Object(shot), path.to_string_lossy().into_owned()))
    }
}

pub fn write_jpeg(
    path: &Path,
    bgr: &[u8],
    width: usize,
    height: usize,
    dpi: i32,
    quality: u8,
) -> bool {
    let (Ok(w), Ok(h)) = (u16::try_from(width), u16::try_from(height)) else {
        return false;
    };
    let Ok(mut encoder) = jpeg_encoder::Encoder::new_file(path, quality) else {
        return false;
    };
    // Same encoder settings as llamaparse-read: 4:2:0 and the DPI in the JFIF header.
    encoder.set_sampling_factor(jpeg_encoder::SamplingFactor::F_2_2);
    if let Ok(d) = u16::try_from(dpi) {
        encoder.set_density(jpeg_encoder::PixelDensity {
            unit: jpeg_encoder::PixelDensityUnit::Inches,
            density: (d, d),
        });
    }
    encoder
        .encode(bgr, w, h, jpeg_encoder::ColorType::Bgr)
        .is_ok()
}

fn write_png_bgr(path: &Path, bgr: &[u8], width: usize, height: usize, level: u8) -> bool {
    let Ok(file) = File::create(path) else {
        return false;
    };
    let mut enc = png::Encoder::new(BufWriter::new(file), width as u32, height as u32);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_compression(if level >= 7 {
        png::Compression::High
    } else {
        png::Compression::Fast
    });
    let Ok(mut w) = enc.write_header() else {
        return false;
    };
    let mut rgb = Vec::with_capacity(bgr.len());
    for px in bgr.as_chunks::<3>().0 {
        rgb.extend_from_slice(&[px[2], px[1], px[0]]);
    }
    w.write_image_data(&rgb).is_ok() && w.finish().is_ok()
}

// ---------------------------------------------------------------- presentation

pub fn presentation_json(
    doc: &Document,
    cfg: &Config,
    scene: &Scene,
    slide_idx: usize,
    frame: &Rect,
) -> Value {
    let pc = &cfg.presentation;
    let s = &doc.slides[slide_idx];
    let (ox, oy) = (frame.x, frame.y);
    let mut v = Map::new();
    v.insert("slideIndex".into(), json!(s.index));
    v.insert("slideId".into(), json!(s.slide_id));
    if frame.x != 0.0
        || frame.y != 0.0
        || (frame.w - doc.slide_w_pt()).abs() > 1e-6
        || (frame.h - doc.slide_h_pt()).abs() > 1e-6
    {
        v.insert(
            "originalSlideFrame".into(),
            json!({ "x": r3(-ox), "y": r3(-oy), "w": r3(doc.slide_w_pt()), "h": r3(doc.slide_h_pt()) }),
        );
    }
    if pc.sections
        && let Some(name) = &s.section_name
    {
        v.insert("sectionName".into(), json!(name));
    }
    if pc.speaker_notes
        && let Some(n) = crate::office::pptx::notes::speaker_notes(doc, &s.part)
    {
        v.insert("speakerNotes".into(), n);
    }
    if pc.charts && !scene.charts.is_empty() {
        let charts: Vec<Value> = scene
            .charts
            .iter()
            .map(|c| {
                let mut c = c.clone();
                if let Some(b) = c.get("coords").cloned() {
                    let r = rect_from_json(&b);
                    c["coords"] = coords(&r, ox, oy);
                    c["hasOverlappingContent"] =
                        json!(overlaps_later_shape(scene, &r, c.get("shapeIndex")));
                }
                c
            })
            .collect();
        v.insert("charts".into(), Value::Array(charts));
    }
    if pc.tables && !scene.tables.is_empty() {
        let tables: Vec<Value> = scene
            .tables
            .iter()
            .map(|t| {
                json!({
                    "shapeId": scene.shapes[t.shape].id,
                    "coords": coords(&t.bounds, ox, oy),
                    "rows": t.rows.iter().map(|row| row.iter().map(|c| json!({
                        "text": c.text, "coords": coords(&c.bounds, ox, oy), "rowSpan": c.row_span, "colSpan": c.col_span,
                    })).collect::<Vec<_>>()).collect::<Vec<_>>(),
                })
            })
            .collect();
        v.insert("tables".into(), Value::Array(tables));
    }
    if pc.shapes {
        // Paragraph → text item ranges, counted the way text_json emits items.
        let mut item_idx = 0usize;
        let mut ranges: HashMap<(usize, usize), [usize; 2]> = HashMap::new();
        for item in &scene.items {
            let Item::Text(run) = item else { continue };
            if run.text.is_empty() || (run.text.trim().is_empty() && !run.preserve_whitespace) {
                continue;
            }
            if let Some(sh) = run.shape {
                let e = ranges
                    .entry((sh, run.paragraph))
                    .or_insert([item_idx, item_idx]);
                e[1] = item_idx + 1;
            }
            item_idx += 1;
        }
        let shapes: Vec<Value> = scene
            .shapes
            .iter()
            .enumerate()
            .filter(|(_, sh)| !sh.inherited || sh.placeholder.is_none())
            .map(|(i, sh)| {
                let mut o = json!({
                    "id": sh.id, "name": sh.name, "kind": sh.kind,
                    "coords": coords(&sh.bounds, ox, oy), "rotation": r3(sh.rotation),
                });
                if let Some(p) = sh.parent {
                    o["parentIndex"] = json!(p);
                }
                if let Some(p) = &sh.placeholder {
                    o["placeholder"] = json!(p);
                }
                if sh.inherited {
                    o["inherited"] = json!(true);
                }
                if sh.hidden {
                    o["hidden"] = json!(true);
                }
                if let Some(a) = &sh.alt_text {
                    o["altText"] = json!(a);
                }
                if !sh.paragraphs.is_empty() {
                    let paras: Vec<Value> = sh
                        .paragraphs
                        .iter()
                        .enumerate()
                        .map(|(pi, (level, bullet))| {
                            let mut p = json!({ "level": level, "textItemRange": ranges.get(&(i, pi)).copied().unwrap_or([0, 0]) });
                            if let Some(b) = bullet {
                                p["bullet"] = json!(b);
                            }
                            p
                        })
                        .collect();
                    o["paragraphs"] = Value::Array(paras);
                }
                o["index"] = json!(i);
                o
            })
            .collect();
        v.insert("shapes".into(), Value::Array(shapes));
    }
    if !scene.smart_art.is_empty() {
        v.insert("smartArt".into(), Value::Array(scene.smart_art.clone()));
    }
    if !scene.media.is_empty() {
        v.insert("media".into(), Value::Array(scene.media.clone()));
    }
    Value::Object(v)
}

fn rect_from_json(v: &Value) -> Rect {
    let f = |k: &str| v.get(k).and_then(Value::as_f64).unwrap_or(0.0);
    Rect {
        x: f("x"),
        y: f("y"),
        w: f("w"),
        h: f("h"),
    }
}

/// True when a shape drawn after the chart's frame overlaps it.
fn overlaps_later_shape(scene: &Scene, r: &Rect, _shape: Option<&Value>) -> bool {
    let Some(chart_idx) = scene.shapes.iter().position(|s| {
        s.kind == "graphicFrame" && (s.bounds.x - r.x).abs() < 0.5 && (s.bounds.y - r.y).abs() < 0.5
    }) else {
        return false;
    };
    scene
        .shapes
        .iter()
        .skip(chart_idx + 1)
        .any(|s| s.parent != Some(chart_idx) && s.kind == "sp" && s.bounds.intersects(r))
}

pub fn source_cell_json(cell: &crate::office::pptx::scene::SourceCell) -> Value {
    let mut value = json!({ "table": cell.table, "row": cell.row, "cell": cell.cell, "span": cell.span, "cellCount": cell.cell_count, "cellSpans": cell.cell_spans, "textOnly": cell.text_only });
    if cell.grid_before > 0 {
        value["gridBefore"] = json!(cell.grid_before);
    }
    if cell.grid_after > 0 {
        value["gridAfter"] = json!(cell.grid_after);
    }
    if !cell.vertical_merges.is_empty() {
        value["verticalMerges"] = json!(
            cell.vertical_merges
                .iter()
                .map(|merge| json!({
                    "cell":merge.cell, "gridStart":merge.grid_start, "span":merge.span,
                    "startRow":merge.start_row, "endRow":merge.end_row,
                }))
                .collect::<Vec<_>>()
        );
    }
    if let Some(fragment) = &cell.fragment {
        value["fragment"] = source_cell_json(fragment);
    }
    value
}

/// Native source identity and transformed point of a nonpainting bookmark boundary.
pub fn bookmark_json(
    marker: &crate::office::pptx::scene::SourceBookmark,
    point: tiny_skia::Point,
) -> Value {
    let mut value = json!({"id":marker.id,"sourcePart":marker.source.part,
        "sourceBookmarkId":marker.xml_id,"sourceBookmarkName":marker.name,
        "sourceParagraphId":marker.source.paragraph,
        "cells":marker.source.cells.iter().map(source_cell_json).collect::<Vec<_>>(),
        "point":{"x":r3(point.x as f64),"y":r3(point.y as f64)}});
    if marker.block_boundary {
        value.as_object_mut().unwrap().remove("sourceParagraphId");
    }
    value
}
