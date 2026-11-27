//! The extract request: opens the presentation, picks the pages to emit, builds
//! and renders each slide, and returns the index and page records (screenshots,
//! embedded images and the optional screenshot PDF go to the requested paths).

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::Path;
use std::time::Instant;

use serde_json::{Map, Value, json};

use crate::office::pptx::config::{Config, OutOfBounds, PdfSource, parse_target_pages};
use crate::office::pptx::document::Document;
use crate::office::pptx::error::{DocError, DocResult};

/// A decoded document: the protocol's index object and its page records.
#[derive(Debug)]
pub struct Extracted {
    pub index: Value,
    /// One JSON object per line, in page order.
    pub pages: Vec<u8>,
}
use crate::office::pptx::fonts::FontSystem;
use crate::office::pptx::render::{self, ImageCache, RenderTarget};
use crate::office::pptx::scene::{Item, Rect, Scene};
use crate::office::pptx::slide::{SlideBuilder, rect_json};

mod parallel;

/// Decode `bytes` with the request in `config_json`: the index object and
/// the page records as JSON lines, in memory. Screenshot and image files still
/// go to the directories the request names. A document-level error (protected,
/// broken, timed out) is reported in the index, as in the file protocol.
pub fn read(config_json: &str, bytes: Vec<u8>) -> Result<Extracted, String> {
    let cfg: Config = serde_json::from_str(config_json).map_err(|e| format!("bad config: {e}"))?;
    let started = Instant::now();
    let doc = match Document::open(bytes, &cfg.limits) {
        Ok(d) => d,
        Err(e) => return Ok(error_index(&e)),
    };
    let fonts = FontSystem::new(
        &cfg.render.font_dirs,
        !cfg.render.no_system_fonts,
        &cfg.render.font_substitutions,
    );
    let embedded_failures = if cfg.render.use_embedded_fonts {
        crate::office::pptx::embedded_fonts::register(&doc, &fonts)
    } else {
        Vec::new()
    };
    let images = ImageCache::new(cfg.limits.max_image_pixels, &fonts);
    let layers = crate::office::pptx::layers::LayerCache::default();

    // Emitted pages: visible slides (or all of them), in presentation order. A
    // deck whose slides are all hidden emits them all rather than nothing.
    let all_hidden = !cfg.presentation.include_hidden_slides
        && !doc.slides.is_empty()
        && doc.slides.iter().all(|s| s.hidden);
    let emitted: Vec<usize> = doc
        .slides
        .iter()
        .enumerate()
        .filter(|(_, s)| cfg.presentation.include_hidden_slides || all_hidden || !s.hidden)
        .map(|(i, _)| i)
        .collect();
    let mut selected: Vec<u32> = match cfg.target_pages.as_deref().filter(|s| !s.trim().is_empty())
    {
        Some(spec) => parse_target_pages(spec)
            .into_iter()
            .filter(|p| (*p as usize) < emitted.len())
            .collect(),
        None => (0..emitted.len() as u32).collect(),
    };
    if cfg.max_pages > 0 {
        selected.truncate(cfg.max_pages as usize);
    }
    // Pictures leave the cache after the last page that can use them, unless the
    // PDF re-renders every page afterwards.
    let slide_order: Vec<usize> = selected.iter().map(|p| emitted[*p as usize]).collect();

    let mut out = PageWriter {
        file: Vec::new(),
        offset: 0,
        index: Vec::new(),
        pdf_pages: Vec::new(),
        warnings: Vec::new(),
        page_size: (doc.slide_w_pt(), doc.slide_h_pt()),
    };
    let workers = parallel::worker_count(cfg.workers, selected.len());
    let finished = if workers > 1 {
        let reach = (!cfg.pdf.enabled || cfg.pdf.source != PdfSource::Render)
            .then(|| slide_order.iter().map(|i| part_reach(&doc, *i)).collect());
        let pages: Vec<(u32, usize)> = selected
            .iter()
            .map(|p| (*p, emitted[*p as usize]))
            .collect();
        let weights = page_weights(&doc, &slide_order);
        let job = parallel::Job {
            doc: &doc,
            fonts: &fonts,
            images: images.store(),
            layers: &layers,
            cfg: &cfg,
            pages: &pages,
            reach,
            weights,
            started,
        };
        parallel::write_pages(job, workers, &mut out)?
    } else {
        let last_use = (!cfg.pdf.enabled || cfg.pdf.source != PdfSource::Render)
            .then(|| part_last_use(&doc, &slide_order));
        write_pages_sequential(
            &doc, &fonts, &images, &layers, &cfg, &emitted, &selected, last_use, started, &mut out,
        )?
    };
    if !finished {
        return Ok(error_index(&DocError::Timeout));
    }
    let PageWriter {
        file: pages_file,
        index,
        pdf_pages,
        mut warnings,
        ..
    } = out;

    if all_hidden {
        warnings.push(json!({
            "code": "ALL_SLIDES_HIDDEN",
            "detail": format!("all {} slides are hidden; emitted them anyway", doc.slides.len()),
        }));
    }

    for (part, why) in embedded_failures {
        warnings.push(
            json!({ "code": "FONT_EMBEDDED_UNSUPPORTED", "detail": format!("{part}: {why}") }),
        );
    }
    // Font substitutions are document-level warnings.
    for (from, to) in fonts.substituted.borrow().iter() {
        warnings.push(json!({ "code": "FONT_SUBSTITUTED", "detail": format!("{from} → {to}") }));
    }
    for (part, why) in images.store().failures() {
        let code = if why == "metafile" {
            "METAFILE_SKIPPED"
        } else {
            "IMAGE_UNDECODABLE"
        };
        let detail = if code == "IMAGE_UNDECODABLE" {
            format!("{part}: {why}")
        } else {
            part
        };
        warnings.push(json!({ "code": code, "detail": detail }));
    }

    let mut pdf_path = None;
    if cfg.pdf.enabled
        && let Some(out) = &cfg.pdf.output_path
    {
        let pages = if cfg.pdf.source == PdfSource::Render || pdf_pages.len() != selected.len() {
            render_pdf_pages(&doc, &fonts, &images, &cfg, &emitted, &selected, out)?
        } else {
            pdf_pages
        };
        crate::office::pptx::pdf::write_pdf(out, &pages).map_err(|e| format!("pdf: {e}"))?;
        pdf_path = Some(out.clone());
    }

    let slides: Vec<Value> = doc
        .slides
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let page = emitted
                .iter()
                .position(|e| *e == i)
                .filter(|p| selected.contains(&(*p as u32)));
            json!({
                "slideIndex": s.index,
                "page": page,
                "slideId": s.slide_id,
                "partName": s.part,
                "hidden": s.hidden,
                "layoutName": layout_name(&doc, &s.part),
                "sectionName": s.section_name,
            })
        })
        .collect();
    let mut idx = Map::new();
    idx.insert("totalPages".into(), json!(emitted.len()));
    if let Some(app) = &doc.meta.app {
        idx.insert("creator".into(), json!(app));
    }
    if let Some(v) = &doc.meta.app_version {
        idx.insert("producer".into(), json!(v));
    }
    idx.insert("index".into(), Value::Array(index));
    idx.insert("sourceFormat".into(), json!("pptx"));
    idx.insert(
        "slideSize".into(),
        json!({ "w": doc.slide_w_pt(), "h": doc.slide_h_pt() }),
    );
    idx.insert(
        "docMeta".into(),
        serde_json::to_value(&doc.meta).unwrap_or(Value::Null),
    );
    if let Some(p) = pdf_path {
        idx.insert("pdfPath".into(), json!(p));
    }
    idx.insert("slides".into(), Value::Array(slides));
    idx.insert("warnings".into(), Value::Array(warnings));
    if let Some(rss) = peak_rss_bytes() {
        idx.insert("peakRssBytes".into(), json!(rss));
    }
    Ok(Extracted {
        index: Value::Object(idx),
        pages: pages_file,
    })
}

/// The page loop on the calling thread: pages are built in order, and each
/// screenshot is encoded on one helper thread while the next page is built.
#[expect(
    clippy::too_many_arguments,
    reason = "the page loop takes the document, its caches, page selection and the writer"
)]
fn write_pages_sequential(
    doc: &Document,
    fonts: &FontSystem,
    images: &ImageCache,
    layers: &crate::office::pptx::layers::LayerCache,
    cfg: &Config,
    emitted: &[usize],
    selected: &[u32],
    last_use: Option<HashMap<String, usize>>,
    started: Instant,
    out: &mut PageWriter,
) -> Result<bool, String> {
    // Page N's screenshot is encoded on a helper thread while page N+1 is built;
    // records are still written strictly in page order.
    std::thread::scope(|scope| -> Result<bool, String> {
        let worker = crate::office::pptx::shots::ShotWorker::spawn(scope);
        let mut queue: std::collections::VecDeque<Queued> = std::collections::VecDeque::new();
        for (pos, page) in selected.iter().enumerate() {
            let slide_idx = emitted[*page as usize];
            if started.elapsed().as_millis() as u64 > cfg.render.document_budget_ms {
                return Ok(false);
            }
            // A panic while building one slide becomes a page error, not a failed document.
            let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                page_record(doc, fonts, images, layers, cfg, *page, slide_idx)
            }))
            .unwrap_or_else(|p| Err(DocError::Broken(format!("panic: {}", panic_message(&p)))));
            doc.release_slide_parts();
            if let Some(last) = &last_use {
                images.release(|part| last.get(part).is_some_and(|l| *l <= pos));
            }
            let built = built.map(|(record, pending)| {
                let shot = match pending.map(|p| worker.submit(p)) {
                    None => Shot::None,
                    Some(Ok(())) => Shot::Submitted,
                    Some(Err(p)) => Shot::Done(Ok(p.finish())),
                };
                (record, shot)
            });
            queue.push_back((*page, slide_idx, built));
            if queue.len() > 1
                && let Some(q) = queue.pop_front()
            {
                out.commit(q, Some(&worker))?;
            }
        }
        while let Some(q) = queue.pop_front() {
            out.commit(q, Some(&worker))?;
        }
        Ok(true)
    })
}

/// A built page waiting for its screenshot: (page, slide index, record).
type Queued = (u32, usize, DocResult<(Record, Shot)>);

/// A page record. `paths` holds the pre-serialized value of the record's `"paths"`
/// key (a `null` placeholder in `value` keeps its position).
struct Record {
    value: Value,
    paths: Option<Vec<u8>>,
}

impl Record {
    /// Compact JSON, identical to `serde_json::to_vec` of the record with the
    /// paths value in place.
    fn to_bytes(&self) -> serde_json::Result<Vec<u8>> {
        let (Some(raw), Some(obj)) = (&self.paths, self.value.as_object()) else {
            return serde_json::to_vec(&self.value);
        };
        let mut out = Vec::with_capacity(raw.len() + 4096);
        out.push(b'{');
        for (i, (k, v)) in obj.iter().enumerate() {
            if i > 0 {
                out.push(b',');
            }
            serde_json::to_writer(&mut out, k)?;
            out.push(b':');
            if k == "paths" {
                out.extend_from_slice(raw);
            } else {
                serde_json::to_writer(&mut out, v)?;
            }
        }
        out.push(b'}');
        Ok(out)
    }
}

enum Shot {
    None,
    Submitted,
    Done(crate::office::pptx::shots::Finished),
}

/// Writes page records in order, with their warnings and screenshot paths.
struct PageWriter {
    /// The page records, one JSON line each.
    file: Vec<u8>,
    offset: u64,
    index: Vec<Value>,
    pdf_pages: Vec<crate::office::pptx::pdf::PdfPage>,
    warnings: Vec<Value>,
    page_size: (f64, f64),
}

impl PageWriter {
    fn commit(
        &mut self,
        (page, slide_idx, built): Queued,
        worker: Option<&crate::office::pptx::shots::ShotWorker>,
    ) -> Result<(), String> {
        let built = built.and_then(|(mut record, shot)| {
            let finished = match shot {
                Shot::None => return Ok((record, None)),
                Shot::Submitted => worker
                    .and_then(|w| w.next())
                    .unwrap_or_else(|| Err(Box::new("screenshot worker stopped"))),
                Shot::Done(f) => f,
            };
            match finished {
                Ok(Some((shot, path))) => {
                    record.value["screenshot"] = shot;
                    Ok((record, Some(path)))
                }
                Ok(None) => {
                    if let Some(obj) = record.value.as_object_mut() {
                        obj.shift_remove("screenshot");
                    }
                    Ok((record, None))
                }
                Err(p) => Err(DocError::Broken(format!("panic: {}", panic_message(&p)))),
            }
        });
        let (record, shot) = match built {
            Ok(r) => r,
            Err(e) => {
                let detail = e.to_string();
                let code = if detail.contains("panic:") {
                    "SLIDE_RENDER_FAILED"
                } else {
                    "SLIDE_PART_UNREADABLE"
                };
                self.warnings
                    .push(json!({ "code": code, "slideIndex": slide_idx, "detail": detail }));
                let value = json!({ "page": page, "error": "PDF_PAGE_ERROR" });
                (Record { value, paths: None }, None)
            }
        };
        if let Some(w) = record.value.get("_warnings").and_then(Value::as_array) {
            for item in w {
                let mut item = item.clone();
                item["slideIndex"] = json!(slide_idx);
                self.warnings.push(item);
            }
        }
        let mut record = record;
        if let Some(obj) = record.value.as_object_mut() {
            obj.remove("_warnings");
        }
        let line = record.to_bytes().map_err(|e| e.to_string())?;
        drop(record);
        self.index
            .push(json!({ "page": page, "offset": self.offset }));
        self.file.write_all(&line).map_err(|e| e.to_string())?;
        self.file.write_all(b"\n").map_err(|e| e.to_string())?;
        self.offset += line.len() as u64 + 1;
        if let Some(path) = shot {
            self.pdf_pages.push(crate::office::pptx::pdf::PdfPage {
                width_pt: self.page_size.0,
                height_pt: self.page_size.1,
                image_path: path,
            });
        }
        Ok(())
    }
}

fn panic_message(p: &Box<dyn std::any::Any + Send>) -> String {
    p.downcast_ref::<&str>()
        .map(|s| (*s).to_string())
        .or_else(|| p.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown".into())
}

fn error_index(e: &DocError) -> Extracted {
    Extracted {
        index: json!({ "error": e.code(), "errorDetail": e.to_string(), "totalPages": 0, "index": [] }),
        pages: Vec::new(),
    }
}

/// The file-protocol form of [`read`], for the tests: reads `input`, writes
/// the index and the page lines to the given paths.
#[cfg(test)]
pub fn run_read(
    config_json: &str,
    input: &str,
    index_path: &str,
    pages_path: &str,
) -> Result<(), String> {
    let bytes = std::fs::read(input).map_err(|e| format!("cannot read {input}: {e}"))?;
    let out = read(config_json, bytes)?;
    std::fs::write(pages_path, &out.pages)
        .map_err(|e| format!("cannot write {pages_path}: {e}"))?;
    let index = serde_json::to_vec(&out.index).map_err(|e| e.to_string())?;
    std::fs::write(index_path, index).map_err(|e| format!("cannot write {index_path}: {e}"))
}

/// For every part a slide reaches through relationships (its layout, master,
/// theme, pictures, charts, diagrams...), the last position in `slides` that
/// reaches it.
fn part_last_use(doc: &Document, slides: &[usize]) -> HashMap<String, usize> {
    let mut last = HashMap::new();
    for (pos, idx) in slides.iter().enumerate() {
        for part in part_reach(doc, *idx) {
            last.insert(part, pos);
        }
    }
    last
}

/// The parts slide `idx` reaches through relationships. Links to other slides,
/// and a master's links to all its layouts, are not followed.
fn part_reach(doc: &Document, idx: usize) -> HashSet<String> {
    let mut seen = HashSet::new();
    let mut todo = vec![doc.slides[idx].part.clone()];
    while let Some(part) = todo.pop() {
        if seen.contains(&part) {
            continue;
        }
        let master = part.contains("slideMasters/");
        for rel in doc.pkg.rels(&part).iter() {
            let kind = rel.kind();
            if rel.external || kind == "slide" || (master && kind == "slideLayout") {
                continue;
            }
            todo.push(rel.target.clone());
        }
        seen.insert(part);
    }
    seen
}

/// Parsed trees take about 12 times their XML (measured on 10 MB slides).
const TREE_BYTES_PER_XML_BYTE: u64 = 12;

/// Rough memory each page needs while it is built, for the parallel loop's
/// in-flight bound: its trees (the slide and the charts, diagrams and drawings
/// it reaches, the parts `release_slide_parts` drops) plus the decoded size of
/// the pictures it is the first page to reach (later pages find them decoded).
fn page_weights(doc: &Document, slides: &[usize]) -> Vec<u64> {
    let mut seen = HashSet::new();
    slides
        .iter()
        .map(|idx| {
            let mut weight = 0;
            for part in part_reach(doc, *idx) {
                if crate::office::pptx::document::SLIDE_SCOPED
                    .iter()
                    .any(|s| part.starts_with(s))
                {
                    weight += doc.pkg.part_size(&part).unwrap_or(0) * TREE_BYTES_PER_XML_BYTE;
                } else if part.starts_with("ppt/media/") && seen.insert(part.clone()) {
                    weight += crate::office::pptx::render::decoded_bytes_estimate(doc, &part);
                }
            }
            weight
        })
        .collect()
}

fn layout_name(doc: &Document, slide_part: &str) -> Option<String> {
    let layout = doc.pkg.rel_by_kind(slide_part, "slideLayout")?.target;
    let x = doc.xml(&layout)?;
    x.child("p:cSld")
        .and_then(|c| c.attr("name"))
        .map(str::to_string)
}

/// Builds the scene of one slide (honouring `outOfBoundsContent`).
pub fn build_scene(
    doc: &Document,
    fonts: &FontSystem,
    images: &ImageCache,
    cfg: &Config,
    slide_idx: usize,
) -> DocResult<Scene> {
    let s = &doc.slides[slide_idx];
    let chain = doc.chain(&s.part)?;
    let scene = SlideBuilder::new(doc, fonts, images, cfg, &chain, s.index + 1).build();
    Ok(scene)
}

/// The slide region the page covers: the slide frame, or the union with all
/// content when `outOfBoundsContent` is `expand`.
fn page_frame(doc: &Document, cfg: &Config, scene: &Scene) -> Rect {
    let slide = Rect {
        x: 0.0,
        y: 0.0,
        w: doc.slide_w_pt(),
        h: doc.slide_h_pt(),
    };
    if cfg.presentation.out_of_bounds_content != OutOfBounds::Expand {
        return slide;
    }
    scene
        .shapes
        .iter()
        .filter(|s| !s.inherited)
        .fold(slide, |acc, s| acc.union(&s.bounds))
}

fn page_record(
    doc: &Document,
    fonts: &FontSystem,
    images: &ImageCache,
    layers: &crate::office::pptx::layers::LayerCache,
    cfg: &Config,
    page: u32,
    slide_idx: usize,
) -> DocResult<(Record, Option<crate::office::pptx::output::PendingShot>)> {
    let scene = build_scene(doc, fonts, images, cfg, slide_idx)?;
    let frame = page_frame(doc, cfg, &scene);
    // Everything is reported relative to the page frame's top-left.
    let (ox, oy) = (frame.x, frame.y);
    let shift = |r: &Rect| Rect {
        x: r.x - ox,
        y: r.y - oy,
        w: r.w,
        h: r.h,
    };
    let mut rec = Map::new();
    rec.insert("page".into(), json!(page));
    rec.insert(
        "viewport".into(),
        json!({ "x": 0.0, "y": 0.0, "w": frame.w, "h": frame.h }),
    );
    rec.insert(
        "contentBounds".into(),
        rect_json(&shift(&content_bounds(&scene, &frame))),
    );

    if cfg.annotation.enabled {
        let mut annots: Vec<Value> = scene
            .links
            .iter()
            .map(|l| json!({ "subtype": "Link", "rect": rect_json(&shift(&l.rect)), "uri": l.uri }))
            .collect();
        if cfg.presentation.comments {
            annots.extend(crate::office::pptx::notes::comments(
                doc,
                &doc.slides[slide_idx].part,
            ));
        }
        rec.insert("annotations".into(), Value::Array(annots));
    }

    if cfg.image.enabled {
        let (imgs, errors) =
            crate::office::pptx::output::images_json(doc, images, cfg, &scene, page, ox, oy);
        rec.insert("images".into(), Value::Array(imgs));
        if errors > 0 {
            rec.insert("imageErrorCount".into(), json!(errors));
        }
    }
    let mut paths = None;
    if cfg.path.enabled {
        rec.insert("paths".into(), Value::Null);
        paths = Some(crate::office::pptx::output::paths_json(
            &scene,
            cfg.path.max_per_page as usize,
            ox,
            oy,
        ));
    }
    let mut pending = None;
    if cfg.screenshot.enabled {
        pending = crate::office::pptx::output::render_screenshot(
            doc, fonts, images, cfg, &scene, &frame, page, layers,
        );
        if pending.is_some() {
            // Filled in (or removed) once the screenshot is encoded.
            rec.insert("screenshot".into(), Value::Null);
        }
    }
    if cfg.text.enabled {
        rec.insert(
            "text".into(),
            crate::office::pptx::output::text_json(
                fonts,
                &scene,
                cfg.verbose,
                (ox, oy),
                (cfg.presentation.out_of_bounds_content == OutOfBounds::Clip)
                    .then_some((frame.w, frame.h)),
            ),
        );
    }
    rec.insert(
        "presentation".into(),
        crate::office::pptx::output::presentation_json(doc, cfg, &scene, slide_idx, &frame),
    );
    if !scene.warnings.is_empty() {
        let w: Vec<Value> = scene
            .warnings
            .iter()
            .map(|(c, d)| json!({ "code": c, "detail": d }))
            .collect();
        rec.insert("_warnings".into(), Value::Array(w));
    }
    let value = Value::Object(rec);
    Ok((Record { value, paths }, pending))
}

fn content_bounds(scene: &Scene, frame: &Rect) -> Rect {
    let mut acc: Option<Rect> = None;
    let mut add = |r: Rect| {
        acc = Some(match acc {
            Some(a) => a.union(&r),
            None => r,
        })
    };
    for item in &scene.items {
        match item {
            Item::Path(p) => {
                // The full-slide background does not count as content.
                let b = p.path.bounds();
                let r = Rect {
                    x: b.x() as f64,
                    y: b.y() as f64,
                    w: b.width() as f64,
                    h: b.height() as f64,
                };
                if p.shape.is_some() || r.w < frame.w * 0.99 || r.h < frame.h * 0.99 {
                    add(r);
                }
            }
            Item::Image(i) => {
                if let Some(r) = crate::office::pptx::textlayout::transformed_bbox(
                    &i.transform,
                    0.0,
                    0.0,
                    1.0,
                    1.0,
                ) {
                    add(r);
                }
            }
            Item::Text(t) => {
                if let Some(r) = crate::office::pptx::textlayout::transformed_bbox(
                    &t.transform,
                    0.0,
                    -t.ascent,
                    t.width,
                    t.ascent + t.descent,
                ) {
                    add(r);
                }
            }
            _ => {}
        }
    }
    let r = acc.unwrap_or(Rect {
        x: frame.x,
        y: frame.y,
        w: 0.0,
        h: 0.0,
    });
    // Clip to the frame.
    let x0 = r.x.max(frame.x);
    let y0 = r.y.max(frame.y);
    let x1 = (r.x + r.w).min(frame.x + frame.w);
    let y1 = (r.y + r.h).min(frame.y + frame.h);
    Rect {
        x: x0,
        y: y0,
        w: (x1 - x0).max(0.0),
        h: (y1 - y0).max(0.0),
    }
}

fn render_pdf_pages(
    doc: &Document,
    fonts: &FontSystem,
    images: &ImageCache,
    cfg: &Config,
    emitted: &[usize],
    selected: &[u32],
    out: &str,
) -> Result<Vec<crate::office::pptx::pdf::PdfPage>, String> {
    let dir = Path::new(out)
        .parent()
        .unwrap_or(Path::new("."))
        .join(format!(".pptx-pdf-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mut pages = Vec::new();
    for page in selected {
        let slide_idx = emitted[*page as usize];
        let scene = build_scene(doc, fonts, images, cfg, slide_idx).map_err(|e| e.to_string())?;
        doc.release_slide_parts();
        let frame = page_frame(doc, cfg, &scene);
        let scale = cfg.pdf.dpi.max(36.0) as f64 / 72.0;
        let target = RenderTarget {
            region: frame,
            width: (frame.w * scale).round() as u32,
            height: (frame.h * scale).round() as u32,
        };
        let pm = render::render(&scene, &target, doc, fonts, images).ok_or("render failed")?;
        let path = dir.join(format!("page_{}.jpg", page + 1));
        let bgr = render::to_bgr(&pm);
        crate::office::pptx::output::write_jpeg(
            &path,
            &bgr,
            pm.width() as usize,
            pm.height() as usize,
            cfg.pdf.dpi as i32,
            cfg.jpeg_quality(),
        );
        pages.push(crate::office::pptx::pdf::PdfPage {
            width_pt: frame.w,
            height_pt: frame.h,
            image_path: path.to_string_lossy().into_owned(),
        });
    }
    Ok(pages)
}

/// This process's peak RSS in bytes: VmHWM, since Linux `ru_maxrss` carries the
/// parent's high-water across fork+exec.
#[cfg(unix)]
fn peak_rss_bytes() -> Option<u64> {
    // ponytail: the getrusage fallback only fires off Linux (local dev).
    if let Some(bytes) = crate::office::pptx::peak_rss::own_peak_rss_bytes() {
        return Some(bytes);
    }
    // SAFETY: `rusage` is plain C data, so all-zero bytes are a valid value.
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    // SAFETY: getrusage writes into the zeroed struct we own.
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, &raw mut usage) } != 0 {
        return None;
    }
    let max = usage.ru_maxrss as u64;
    // macOS reports bytes, Linux kilobytes.
    Some(if cfg!(target_os = "macos") {
        max
    } else {
        max * 1024
    })
}

#[cfg(not(unix))]
fn peak_rss_bytes() -> Option<u64> {
    None
}

#[cfg(test)]
mod tests;
