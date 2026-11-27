//! Rasterizes a scene with tiny-skia: paths (fills, gradients, strokes, arrow
//! heads), pictures (cropped, clipped, SVG through resvg) and glyph outlines.
//!
//! Output is an opaque RGBA pixmap on white; `to_bgr` converts it to the packed
//! BGR layout the shared raster scanners and encoders expect.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use tiny_skia::{
    FillRule, FilterQuality, GradientStop as SkStop, LinearGradient, Mask, Paint as SkPaint, Path,
    PathBuilder, Pattern, Pixmap, Point, RadialGradient, Shader, SpreadMode, Stroke as SkStroke,
    StrokeDash, Transform,
};

mod assets;
pub use assets::RenderAssets;

mod blur;
mod jpeg_dc;
mod resize;
mod softedge;
mod tiff_extra;

use crate::office::pptx::color::Rgba;

/// Largest device-pixel coordinate tiny-skia's anti-aliased scan converter handles.
/// It supersamples 4x and builds edges in 26.6 fixed point, so 4 * 64 * |x| must fit
/// an `i32`; past that tiny-skia 0.11 computes wrong spans and panics.
const MAX_ANTI_ALIAS_COORD: f32 = (1 << 23) as f32;

/// Whether `path` drawn through `transform` can be anti-aliased. `outset` grows the
/// path's own bounds first: half the stroke width times the miter limit for a stroke,
/// 0 for a fill. Callers draw aliased otherwise; such a path has an edge millions of
/// pixels off the canvas, so only its visible part's edges lose smoothing.
pub fn anti_alias_fits(path: &Path, transform: Transform, outset: f32) -> bool {
    let b = path.bounds();
    let grown = tiny_skia::Rect::from_ltrb(
        b.left() - outset,
        b.top() - outset,
        b.right() + outset,
        b.bottom() + outset,
    );
    grown.and_then(|r| r.transform(transform)).is_some_and(|r| {
        [r.left(), r.top(), r.right(), r.bottom()]
            .iter()
            .all(|v| v.abs() <= MAX_ANTI_ALIAS_COORD)
    })
}

/// The outset [`anti_alias_fits`] needs for a stroke.
pub fn stroke_outset(stroke: &SkStroke) -> f32 {
    stroke.width * stroke.miter_limit.max(1.0)
}
use crate::office::pptx::fonts::FontSystem;
use crate::office::pptx::scene::{
    Arrow, ArrowKind, GlyphRun, ImageItem, ImageRef, Item, LineCap, LineJoin, Paint, PathItem,
    Rect, Scene, ShadowItem, ShadowSource, Stroke,
};

/// Decoded pictures, by part name (and recolor variant). `None` = undecodable.
/// Memory is bounded: pictures are downscaled to `MAX_DECODED_EDGE` and the
/// store evicts least-recently-used entries past `CACHE_BUDGET_BYTES`.
///
/// One store serves every render thread of a document. Each entry is a slot
/// that the first thread to ask for it fills; others asking meanwhile wait for
/// that decode instead of repeating it. Decoding runs outside the lock.
#[derive(Default)]
pub struct ImageStore {
    inner: Mutex<StoreInner>,
    /// Held while decoding a large picture: decoding one takes several times
    /// its pixels in scratch memory, so threads decode large ones one at a time.
    large_decode: Mutex<()>,
}

/// Pictures above this many pixels, or files above this size (metafiles with
/// embedded bitmaps), take `ImageStore::large_decode`.
const LARGE_DECODE_PIXELS: u64 = 4 * 1024 * 1024;
const LARGE_DECODE_FILE_BYTES: usize = 16 * 1024 * 1024;

type Slot = Arc<OnceLock<Option<Arc<Pixmap>>>>;

#[derive(Default)]
struct StoreInner {
    decoded: HashMap<String, Slot>,
    /// Keys in use order (front = least recent).
    lru: std::collections::VecDeque<String>,
    /// Bytes held by the filled slots still in `decoded`.
    cached_bytes: u64,
    /// Pixel size of each picture as stored in the file, before downscaling.
    native_size: HashMap<String, (u32, u32)>,
    /// Text drawn inside metafile pictures, by part, filled when they decode.
    metafile_text: HashMap<String, Arc<Vec<crate::office::pptx::metafile::MetafileText>>>,
    /// Parts that could not be decoded, with the reason (sorted, each once).
    failures: std::collections::BTreeSet<(String, String)>,
}

impl ImageStore {
    fn lock(&self) -> std::sync::MutexGuard<'_, StoreInner> {
        // A panic elsewhere never leaves the maps half-updated: every critical
        // section is a few plain inserts or removals.
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Drops decoded pictures whose part `done` says no later page uses, so a
    /// picture-heavy deck holds only what is still ahead. Undecodable entries
    /// stay (their failure is recorded once either way).
    pub fn release(&self, done: impl Fn(&str) -> bool) {
        let mut g = self.lock();
        let inner = &mut *g;
        let mut freed = 0u64;
        inner.decoded.retain(|key, slot| {
            let part = key.split('#').next().unwrap_or(key);
            match slot.get() {
                Some(Some(pm)) if done(part) => {
                    freed += pm.data().len() as u64;
                    false
                }
                _ => true,
            }
        });
        if freed > 0 {
            let decoded = &inner.decoded;
            inner.lru.retain(|k| decoded.contains_key(k));
            inner.cached_bytes = inner.cached_bytes.saturating_sub(freed);
        }
    }

    /// Parts that could not be decoded, with the reason, sorted by part.
    pub fn failures(&self) -> Vec<(String, String)> {
        self.lock().failures.iter().cloned().collect()
    }

    /// The slot for `key` (made empty if absent), marked most recently used.
    fn slot(&self, key: &str) -> Slot {
        let mut g = self.lock();
        let inner = &mut *g;
        if let Some(slot) = inner.decoded.get(key) {
            let slot = slot.clone();
            if let Some(pos) = inner.lru.iter().position(|k| k == key) {
                let k = inner.lru.remove(pos).unwrap_or_default();
                inner.lru.push_back(k);
            }
            return slot;
        }
        let slot = Slot::default();
        inner.decoded.insert(key.to_string(), slot.clone());
        inner.lru.push_back(key.to_string());
        slot
    }

    /// Counts a slot just filled for `key` and evicts the oldest entries (never
    /// `key`) until under budget.
    fn filled(&self, key: &str, value: Option<&Arc<Pixmap>>) {
        let size = value.map(|p| p.data().len() as u64).unwrap_or(0);
        let mut g = self.lock();
        let inner = &mut *g;
        // Released or evicted while decoding: the caller keeps its copy.
        if !inner.decoded.contains_key(key) {
            return;
        }
        inner.cached_bytes += size;
        while inner.cached_bytes > CACHE_BUDGET_BYTES && inner.lru.len() > 1 {
            let Some(pos) = inner.lru.iter().position(|k| k != key) else {
                break;
            };
            let old = inner.lru.remove(pos).unwrap_or_default();
            if let Some(slot) = inner.decoded.remove(&old)
                && let Some(Some(pm)) = slot.get()
            {
                inner.cached_bytes = inner.cached_bytes.saturating_sub(pm.data().len() as u64);
            }
        }
    }
}

/// One thread's handle on an [`ImageStore`]: metafiles draw text, so their
/// decoder needs that thread's font system.
pub struct ImageCache<'f> {
    store: Arc<ImageStore>,
    fonts: &'f FontSystem,
    pub max_pixels: u64,
}

/// Raster size for SVG pictures (long edge, pixels).
const SVG_RASTER_EDGE: f32 = 2048.0;
/// Longest edge kept for decoded pictures: above any page render, below the
/// 10k+ px scans some decks embed (which would cost ~400 MB each).
const MAX_DECODED_EDGE: u32 = 4096;
const CACHE_BUDGET_BYTES: u64 = 256 * 1024 * 1024;
/// PNGs above this many pixels take the streaming, downscaling decoder.
const STREAM_DECODE_PIXELS: u64 = 32 * 1024 * 1024;
/// Hard cap even for streamed PNGs (2 G pixels).
const MAX_STREAM_PIXELS: u64 = 2 * 1024 * 1024 * 1024;

impl<'f> ImageCache<'f> {
    /// A cache with a store of its own.
    pub fn new(max_pixels: u64, fonts: &'f FontSystem) -> ImageCache<'f> {
        ImageCache::with_store(Arc::default(), max_pixels, fonts)
    }

    /// A cache over a store shared with other threads.
    pub fn with_store(
        store: Arc<ImageStore>,
        max_pixels: u64,
        fonts: &'f FontSystem,
    ) -> ImageCache<'f> {
        ImageCache {
            store,
            fonts,
            max_pixels,
        }
    }

    pub fn store(&self) -> &Arc<ImageStore> {
        &self.store
    }

    /// The decoded picture with its recolor effects applied, cached per variant.
    pub fn get(&self, doc: &dyn RenderAssets, image: &ImageRef) -> Option<Arc<Pixmap>> {
        if image.effects.is_empty() {
            return self.get_part(doc, &image.part);
        }
        let key = format!("{}#{:?}", image.part, image.effects);
        self.fill(&key, || {
            self.get_part(doc, &image.part).map(|base| {
                let mut pm = (*base).clone();
                crate::office::pptx::recolor::apply(&mut pm, &image.effects);
                Arc::new(pm)
            })
        })
    }

    /// The value of `key`'s slot, computed by `make` if this thread fills it.
    fn fill(&self, key: &str, make: impl FnOnce() -> Option<Arc<Pixmap>>) -> Option<Arc<Pixmap>> {
        let slot = self.store.slot(key);
        let mut made = false;
        let value = slot
            .get_or_init(|| {
                made = true;
                make()
            })
            .clone();
        if made {
            self.store.filled(key, value.as_ref());
        }
        value
    }

    /// Pixel size of a picture as stored (before any downscale), if decoded.
    pub fn native_size(&self, doc: &dyn RenderAssets, part: &str) -> Option<(u32, u32)> {
        let pm = self.get_part(doc, part)?;
        Some(
            self.store
                .lock()
                .native_size
                .get(part)
                .copied()
                .unwrap_or((pm.width(), pm.height())),
        )
    }

    /// Drops decoded pictures no later page uses; see [`ImageStore::release`].
    pub fn release(&self, done: impl Fn(&str) -> bool) {
        self.store.release(done);
    }

    /// Horizontal and vertical resolution recorded in a PNG (`pHYs`) or JPEG
    /// (JFIF density) picture; 96 dpi when absent.
    pub fn dpi(&self, doc: &dyn RenderAssets, part: &str) -> (f32, f32) {
        doc.read(part)
            .ok()
            .and_then(|b| image_dpi(&b))
            .unwrap_or((96.0, 96.0))
    }

    pub fn get_part(&self, doc: &dyn RenderAssets, part: &str) -> Option<Arc<Pixmap>> {
        self.fill(part, || {
            let result = match doc.read(part) {
                Ok(bytes) => {
                    let _turn = (bytes.len() > LARGE_DECODE_FILE_BYTES
                        || pixel_count(&bytes) > LARGE_DECODE_PIXELS)
                        .then(|| {
                            self.store
                                .large_decode
                                .lock()
                                .unwrap_or_else(|p| p.into_inner())
                        });
                    self.decode(&bytes, part)
                }
                Err(e) => Err(e.to_string()),
            };
            match result {
                Ok(p) => Some(Arc::new(p)),
                Err(why) => {
                    self.store.lock().failures.insert((part.to_string(), why));
                    None
                }
            }
        })
    }

    /// Runs of text drawn inside a metafile picture (empty for other images).
    pub fn metafile_text(
        &self,
        doc: &dyn RenderAssets,
        part: &str,
    ) -> Arc<Vec<crate::office::pptx::metafile::MetafileText>> {
        self.get_part(doc, part);
        self.store
            .lock()
            .metafile_text
            .get(part)
            .cloned()
            .unwrap_or_default()
    }

    fn decode(&self, bytes: &[u8], part: &str) -> Result<Pixmap, String> {
        if is_metafile(bytes) {
            let out = crate::office::pptx::metafile::render_with_text(bytes, self.fonts)
                .ok_or_else(|| "metafile".to_string())?;
            self.store
                .lock()
                .metafile_text
                .insert(part.to_string(), Arc::new(out.text));
            return Ok(out.pixmap);
        }
        let head = &bytes[..bytes.len().min(512)];
        let text_head = String::from_utf8_lossy(head);
        // Only data that is not a known raster format is sniffed for SVG: PNGs can
        // carry XML metadata (C2PA credentials) that mentions "svg".
        let raster = image::guess_format(bytes).is_ok();
        if !raster
            && (text_head.contains("<svg")
                || (text_head.trim_start().starts_with("<?xml") && text_head.contains("svg")))
        {
            return render_svg(bytes);
        }
        // Oversized PNGs (charts exported at 18000 px) are decoded row by row
        // straight into a downscaled buffer instead of being rejected.
        if bytes.starts_with(b"\x89PNG")
            && let Some((nw, nh)) = png_size(bytes)
        {
            // Interlaced files cannot be streamed row by row; they take the
            // normal decoder below (within `max_pixels`).
            let interlaced = bytes.get(28) == Some(&1);
            if nw as u64 * nh as u64 > STREAM_DECODE_PIXELS && !interlaced {
                self.store
                    .lock()
                    .native_size
                    .insert(part.to_string(), (nw, nh));
                return decode_png_downscaled(bytes, MAX_DECODED_EDGE);
            }
        }
        // JPEGs above the pixel limit are read at 1/8 scale from their DC terms.
        let huge_jpeg = jpeg_dc::size(bytes).filter(|&(w, h)| {
            w as u64 * h as u64 > self.max_pixels && w as u64 * h as u64 <= MAX_STREAM_PIXELS
        });
        let mut img = if huge_jpeg.is_some() {
            jpeg_dc::decode(bytes)?
        } else {
            match image::load_from_memory(bytes) {
                Ok(img) => img,
                // Palette and CMYK+alpha TIFFs; any other failure keeps its message.
                Err(e) => tiff_extra::decode(bytes, self.max_pixels).map_err(|_| e.to_string())?,
            }
        };
        let (nw, nh) = huge_jpeg.unwrap_or((img.width(), img.height()));
        if huge_jpeg.is_none() && nw as u64 * nh as u64 > self.max_pixels {
            return Err(format!("{nw}x{nh} exceeds the pixel limit"));
        }
        self.store
            .lock()
            .native_size
            .insert(part.to_string(), (nw, nh));
        if nw.max(nh) > MAX_DECODED_EDGE {
            // Same pixels as `img.resize`, without its full-height f32 copy.
            img = resize::fit_triangle(&img, MAX_DECODED_EDGE).unwrap_or_else(|| {
                img.resize(
                    MAX_DECODED_EDGE,
                    MAX_DECODED_EDGE,
                    image::imageops::FilterType::Triangle,
                )
            });
        }
        let (w, h) = (img.width(), img.height());
        let rgba = img.into_rgba8();
        let mut data = rgba.into_raw();
        // tiny-skia stores premultiplied alpha.
        for px in data.as_chunks_mut::<4>().0 {
            let a = px[3] as u32;
            if a < 255 {
                px[0] = ((px[0] as u32 * a + 127) / 255) as u8;
                px[1] = ((px[1] as u32 * a + 127) / 255) as u8;
                px[2] = ((px[2] as u32 * a + 127) / 255) as u8;
            }
        }
        let size = tiny_skia::IntSize::from_wh(w, h).ok_or("empty image")?;
        Pixmap::from_vec(data, size).ok_or_else(|| "pixmap".to_string())
    }
}

/// Bytes a picture takes once decoded (RGBA, long edge capped as decoding
/// does), from its header alone. Other pictures (metafiles, SVG) count twice
/// their file size: their bytes plus the bitmaps they carry.
pub fn decoded_bytes_estimate(doc: &dyn RenderAssets, part: &str) -> u64 {
    let Ok(head) = doc.read_head(part, 64 * 1024) else {
        return 0;
    };
    let Some((w, h)) = image::ImageReader::new(std::io::Cursor::new(&head))
        .with_guessed_format()
        .ok()
        .and_then(|r| r.into_dimensions().ok())
    else {
        return doc.part_size(part).unwrap_or(0).saturating_mul(2);
    };
    let scale = (MAX_DECODED_EDGE as f64 / w.max(h).max(1) as f64).min(1.0);
    ((w as f64 * scale) * (h as f64 * scale) * 4.0) as u64
}

/// Pixel count a raster picture declares in its header (0 when unknown).
fn pixel_count(bytes: &[u8]) -> u64 {
    raster_dimensions(bytes).map_or(0, |(w, h)| w as u64 * h as u64)
}

/// Header dimensions for imported raster assets, without decoding their pixel buffers.
pub fn raster_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()
        .and_then(|r| r.into_dimensions().ok())
}

fn png_size(b: &[u8]) -> Option<(u32, u32)> {
    // IHDR is the first chunk: width and height at bytes 16..24.
    if b.len() < 24 || &b[12..16] != b"IHDR" {
        return None;
    }
    Some((
        u32::from_be_bytes(b[16..20].try_into().ok()?),
        u32::from_be_bytes(b[20..24].try_into().ok()?),
    ))
}

/// Decodes a PNG one row at a time, box-averaging into a pixmap whose long edge
/// is at most `max_edge`. Memory is one source row plus the output.
fn decode_png_downscaled(bytes: &[u8], max_edge: u32) -> Result<Pixmap, String> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let (w, h) = (reader.info().width, reader.info().height);
    if w as u64 * h as u64 > MAX_STREAM_PIXELS || reader.info().interlaced {
        return Err(format!("{w}x{h} exceeds the pixel limit"));
    }
    let k = w.max(h).div_ceil(max_edge).max(1);
    let (ow, oh) = (w.div_ceil(k), h.div_ceil(k));
    let (color, _) = reader.output_color_type();
    let channels = match color {
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        png::ColorType::Indexed => return Err("unexpanded palette".into()),
    };
    // Running sums per output pixel for the current band of k source rows.
    let mut acc = vec![0u32; ow as usize * 4];
    let mut out = vec![0u8; ow as usize * oh as usize * 4];
    let mut rows_in_band = 0u32;
    let mut oy = 0usize;
    let flush = |acc: &mut [u32], rows: u32, oy: usize, out: &mut [u8]| {
        for ox in 0..ow as usize {
            let cols = (w - ox as u32 * k).min(k);
            let n = (rows * cols).max(1);
            let a = acc[ox * 4 + 3] / n;
            for c in 0..3 {
                // Premultiply as tiny-skia expects.
                let v = acc[ox * 4 + c] / n;
                out[(oy * ow as usize + ox) * 4 + c] = ((v * a + 127) / 255) as u8;
            }
            out[(oy * ow as usize + ox) * 4 + 3] = a as u8;
        }
        acc.fill(0);
    };
    while let Some(row) = reader.next_row().map_err(|e| e.to_string())? {
        let data = &row.data()[..w as usize * channels];
        match channels {
            1 => accumulate_row::<1>(data, k as usize, &mut acc),
            2 => accumulate_row::<2>(data, k as usize, &mut acc),
            3 => accumulate_row::<3>(data, k as usize, &mut acc),
            _ => accumulate_row::<4>(data, k as usize, &mut acc),
        }
        rows_in_band += 1;
        if rows_in_band == k {
            flush(&mut acc, rows_in_band, oy, &mut out);
            rows_in_band = 0;
            oy += 1;
        }
    }
    if rows_in_band > 0 && oy < oh as usize {
        flush(&mut acc, rows_in_band, oy, &mut out);
    }
    let size = tiny_skia::IntSize::from_wh(ow, oh).ok_or("empty image")?;
    Pixmap::from_vec(out, size).ok_or_else(|| "pixmap".to_string())
}

/// Adds one source row of `C`-channel pixels into the RGBA sums of its output
/// pixels, `k` source pixels per output pixel (the last one may get fewer).
fn accumulate_row<const C: usize>(data: &[u8], k: usize, acc: &mut [u32]) {
    for (src, o) in data.chunks(k * C).zip(acc.as_chunks_mut::<4>().0) {
        let (mut r, mut g, mut b, mut a) = (0u32, 0u32, 0u32, 0u32);
        for px in src.as_chunks::<C>().0 {
            let (pr, pg, pb, pa) = match C {
                1 => (px[0], px[0], px[0], 255),
                2 => (px[0], px[0], px[0], px[1]),
                3 => (px[0], px[1], px[2], 255),
                _ => (px[0], px[1], px[2], px[3]),
            };
            r += pr as u32;
            g += pg as u32;
            b += pb as u32;
            a += pa as u32;
        }
        o[0] += r;
        o[1] += g;
        o[2] += b;
        o[3] += a;
    }
}

pub fn image_dpi(b: &[u8]) -> Option<(f32, f32)> {
    if b.starts_with(b"\x89PNG") {
        // Walk chunks for pHYs: x/y pixels per unit, unit 1 = metre.
        let mut i = 8;
        while i + 8 <= b.len() {
            let len = u32::from_be_bytes(b[i..i + 4].try_into().ok()?) as usize;
            let kind = &b[i + 4..i + 8];
            if kind == b"pHYs" && i + 8 + 9 <= b.len() && b[i + 16] == 1 {
                let x = u32::from_be_bytes(b[i + 8..i + 12].try_into().ok()?) as f32 * 0.0254;
                let y = u32::from_be_bytes(b[i + 12..i + 16].try_into().ok()?) as f32 * 0.0254;
                return (x > 1.0 && y > 1.0).then_some((x, y));
            }
            if kind == b"IDAT" || kind == b"IEND" {
                return None;
            }
            i += 12 + len;
        }
        return None;
    }
    if b.len() > 18 && b.starts_with(&[0xFF, 0xD8, 0xFF, 0xE0]) && &b[6..11] == b"JFIF\0" {
        // JFIF density: units 1 = dots per inch, 2 = per cm.
        let unit = b[13];
        let x = u16::from_be_bytes([b[14], b[15]]) as f32;
        let y = u16::from_be_bytes([b[16], b[17]]) as f32;
        let k = match unit {
            1 => 1.0,
            2 => 2.54,
            _ => return None,
        };
        return (x > 1.0 && y > 1.0).then_some((x * k, y * k));
    }
    None
}

pub fn is_metafile(bytes: &[u8]) -> bool {
    // EMF: EMR_HEADER record type 1 with " EMF" signature at offset 40.
    let emf = bytes.len() > 44 && bytes[0..4] == [1, 0, 0, 0] && &bytes[40..44] == b" EMF";
    // WMF: placeable header magic, or a standard header (type 1/2, header size 9).
    let wmf = bytes.starts_with(&[0xD7, 0xCD, 0xC6, 0x9A])
        || (bytes.len() > 4
            && (bytes[0..2] == [1, 0] || bytes[0..2] == [2, 0])
            && bytes[2..4] == [9, 0]);
    emf || wmf
}

fn render_svg(bytes: &[u8]) -> Result<Pixmap, String> {
    let opt = usvg::Options::default();
    let tree = usvg::Tree::from_data(bytes, &opt).map_err(|e| e.to_string())?;
    let size = tree.size();
    let scale = (SVG_RASTER_EDGE / size.width().max(size.height()).max(1.0)).min(8.0);
    let w = (size.width() * scale).ceil().max(1.0) as u32;
    let h = (size.height() * scale).ceil().max(1.0) as u32;
    // resvg draws with its own tiny-skia release; both store premultiplied RGBA,
    // so the pixels move over as is. Text inside SVGs is not drawn (no font stack).
    let mut svg_pm = resvg::tiny_skia::Pixmap::new(w, h).ok_or("svg size")?;
    let ts = resvg::tiny_skia::Transform::from_scale(scale, scale);
    resvg::render(&tree, ts, &mut svg_pm.as_mut());
    Pixmap::from_vec(
        svg_pm.take(),
        tiny_skia::IntSize::from_wh(w, h).ok_or("svg size")?,
    )
    .ok_or_else(|| "svg pixmap".to_string())
}

/// Output region and size of one render.
pub struct RenderTarget {
    /// Slide-space region to render (points).
    pub region: Rect,
    pub width: u32,
    pub height: u32,
}

pub fn render(
    scene: &Scene,
    target: &RenderTarget,
    doc: &dyn RenderAssets,
    fonts: &FontSystem,
    images: &ImageCache,
) -> Option<Pixmap> {
    render_layered(scene, target, doc, fonts, images, None)
}

/// `render`, resuming from (and feeding) the master/layout layer cache.
pub fn render_layered(
    scene: &Scene,
    target: &RenderTarget,
    doc: &dyn RenderAssets,
    fonts: &FontSystem,
    images: &ImageCache,
    layers: Option<&crate::office::pptx::layers::LayerCache>,
) -> Option<Pixmap> {
    let (width, height) = (target.width.max(1), target.height.max(1));
    let r = &target.region;
    let key = (
        target.width,
        target.height,
        [r.x, r.y, r.w, r.h].map(f64::to_bits),
    );
    let mut plan = layers.map(|l| l.plan(scene, key));
    let mut pm = match plan.as_mut().and_then(|p| p.base.take()) {
        Some(base) => base,
        None => {
            let mut pm = Pixmap::new(width, height)?;
            pm.fill(tiny_skia::Color::WHITE);
            pm
        }
    };
    let start = plan.as_ref().map_or(0, |p| p.start);
    let snapshot_at = plan.as_ref().and_then(|p| p.snapshot_at);
    let sx = target.width as f32 / target.region.w.max(1e-6) as f32;
    let sy = target.height as f32 / target.region.h.max(1e-6) as f32;
    let canvas = Transform::from_translate(-target.region.x as f32, -target.region.y as f32)
        .post_scale(sx, sy);
    let mut clips: Vec<Mask> = Vec::new();
    let mut soft: Vec<Option<softedge::Group>> = Vec::new();
    for (i, item) in scene.items.iter().enumerate().skip(start) {
        if snapshot_at == Some(i)
            && let (Some(l), Some(p)) = (layers, &plan)
        {
            l.store(p, &pm);
        }
        let mask = clips.last();
        match item {
            Item::Bookmark(_) => {}
            Item::Shadow(s) => draw_shadow(&mut pm, s, canvas, mask, fonts, sx, (doc, images)),
            Item::Path(p) => draw_path(&mut pm, p, canvas, mask, doc, images),
            Item::Multiply(p) => draw_multiply(&mut pm, p, canvas, mask, doc, images),
            Item::Image(img) => draw_image(&mut pm, img, canvas, mask, doc, images),
            Item::Text(run) => draw_text(&mut pm, run, canvas, mask, fonts),
            Item::PushClip(path) => {
                if let Some(mut m) = Mask::new(pm.width(), pm.height()) {
                    let aa = anti_alias_fits(path, canvas, 0.0);
                    m.fill_path(path, FillRule::Winding, aa, canvas);
                    clips.push(m);
                }
            }
            Item::PopClip => {
                clips.pop();
            }
            Item::PushSoftEdge(se) => soft.push(softedge::begin(&pm, se, canvas, sx)),
            Item::PopSoftEdge => {
                if let Some(Some(g)) = soft.pop() {
                    softedge::end(&mut pm, g);
                }
            }
        }
    }
    if snapshot_at == Some(scene.items.len())
        && let (Some(l), Some(p)) = (layers, &plan)
    {
        l.store(p, &pm);
    }
    Some(pm)
}

/// Packs an opaque pixmap into BGR bytes (the layout of pdfium's 24-bit target).
pub fn to_bgr(pm: &Pixmap) -> Vec<u8> {
    let src = pm.data();
    // Indexed writes into a sized buffer: no per-pixel capacity checks.
    let mut out = vec![0u8; src.len() / 4 * 3];
    for (o, px) in out
        .as_chunks_mut::<3>()
        .0
        .iter_mut()
        .zip(src.as_chunks::<4>().0)
    {
        o[0] = px[2];
        o[1] = px[1];
        o[2] = px[0];
    }
    out
}

fn sk_color(c: Rgba) -> tiny_skia::Color {
    c.to_skia()
}

fn stops(stops: &[crate::office::pptx::scene::GradientStop]) -> Vec<SkStop> {
    stops
        .iter()
        .map(|s| SkStop::new(s.pos, sk_color(s.color)))
        .collect()
}

/// Shader in the coordinate space of the path being filled: tiny-skia applies the
/// fill transform to the shader as well, so `canvas` must not be folded in here.
fn shader<'a>(
    p: &Paint,
    doc: &dyn RenderAssets,
    images: &'a ImageCache,
    holder: &'a mut Option<Arc<Pixmap>>,
) -> Option<Shader<'a>> {
    match p {
        Paint::Solid(c) => Some(Shader::SolidColor(sk_color(*c))),
        Paint::Linear {
            start,
            end,
            stops: st,
        } => {
            if st.len() == 1 {
                return Some(Shader::SolidColor(sk_color(st[0].color)));
            }
            LinearGradient::new(
                Point::from_xy(start.0 as f32, start.1 as f32),
                Point::from_xy(end.0 as f32, end.1 as f32),
                stops(st),
                SpreadMode::Pad,
                Transform::identity(),
            )
            .or_else(|| st.first().map(|s| Shader::SolidColor(sk_color(s.color))))
        }
        Paint::Radial {
            center,
            radius,
            stops: st,
        } => {
            if st.len() == 1 {
                return Some(Shader::SolidColor(sk_color(st[0].color)));
            }
            let c = Point::from_xy(center.0 as f32, center.1 as f32);
            RadialGradient::new(
                c,
                0.0,
                c,
                *radius as f32,
                stops(st),
                SpreadMode::Pad,
                Transform::identity(),
            )
            .or_else(|| st.last().map(|s| Shader::SolidColor(sk_color(s.color))))
        }
        Paint::Path(g) => {
            if g.stops.len() == 1 {
                return Some(Shader::SolidColor(sk_color(g.stops[0].color)));
            }
            crate::office::pptx::pathgrad::shader(g, holder)
        }
        Paint::Pattern { preset, fg, bg } => {
            // The 8×8 preset tile, one pattern pixel per screen pixel at 96 dpi,
            // repeated from the slide origin.
            let rows = crate::office::pptx::patterns::tile(preset);
            let mut tile = Pixmap::new(8, 8)?;
            let (on, off) = (sk_color(*fg).premultiply(), sk_color(*bg).premultiply());
            for (y, row) in rows.iter().enumerate() {
                for x in 0..8 {
                    let c = if row & (0x80 >> x) != 0 { on } else { off };
                    tile.pixels_mut()[y * 8 + x] = c.to_color_u8();
                }
            }
            *holder = Some(Arc::new(tile));
            let pm = holder.as_ref()?;
            let k = crate::office::pptx::patterns::PIXEL_PT;
            Some(Pattern::new(
                Pixmap::as_ref(pm),
                SpreadMode::Repeat,
                FilterQuality::Nearest,
                1.0,
                Transform::from_scale(k, k),
            ))
        }
        Paint::Image {
            image,
            rect,
            tile,
            alpha,
            frame,
            ..
        } => {
            *holder = images.get(doc, image);
            let pm = holder.as_ref()?;
            let (iw, ih) = (pm.width() as f32, pm.height() as f32);
            let t = if let Some(tl) = tile {
                // Tiles at the picture's natural size (its DPI, 96 if unknown).
                let (dx, dy) = images.dpi(doc, &image.part);
                let (kx, ky) = ((72.0 / dx) as f64 * tl.sx, (72.0 / dy) as f64 * tl.sy);
                Transform::from_scale(kx as f32, ky as f32)
                    .post_translate((rect.x + tl.tx) as f32, (rect.y + tl.ty) as f32)
            } else {
                Transform::from_scale(rect.w as f32 / iw, rect.h as f32 / ih)
                    .post_translate(rect.x as f32, rect.y as f32)
            }
            .post_concat(*frame);
            Some(Pattern::new(
                Pixmap::as_ref(pm),
                if tile.is_some() {
                    SpreadMode::Repeat
                } else {
                    SpreadMode::Pad
                },
                FilterQuality::Bicubic,
                *alpha,
                t,
            ))
        }
    }
}

fn draw_path(
    pm: &mut Pixmap,
    p: &PathItem,
    canvas: Transform,
    mask: Option<&Mask>,
    doc: &dyn RenderAssets,
    images: &ImageCache,
) {
    if let Some(fill) = &p.fill {
        let mut holder = None;
        if let Some(sh) = shader(fill, doc, images, &mut holder) {
            let paint = SkPaint {
                shader: sh,
                anti_alias: anti_alias_fits(&p.path, canvas, 0.0),
                ..Default::default()
            };
            match thin_rule(&p.path, canvas) {
                Some(rule) => pm.fill_rect(rule, &paint, Transform::identity(), mask),
                None => pm.fill_path(&p.path, &paint, FillRule::Winding, canvas, mask),
            }
        }
    }
    if let Some(s) = &p.stroke {
        stroke_path(pm, &p.path, s, canvas, mask, doc, images);
    }
}

/// Fills `p` with its paint multiplied over the pixels beneath (`a:fillOverlay`).
fn draw_multiply(
    pm: &mut Pixmap,
    p: &PathItem,
    canvas: Transform,
    mask: Option<&Mask>,
    doc: &dyn RenderAssets,
    images: &ImageCache,
) {
    let Some(fill) = &p.fill else {
        return;
    };
    let mut holder = None;
    if let Some(sh) = shader(fill, doc, images, &mut holder) {
        let paint = SkPaint {
            shader: sh,
            blend_mode: tiny_skia::BlendMode::Multiply,
            anti_alias: anti_alias_fits(&p.path, canvas, 0.0),
            ..Default::default()
        };
        pm.fill_path(&p.path, &paint, FillRule::Winding, canvas, mask);
    }
}

/// A filled axis-aligned rectangle thinner than one device pixel (a hairline table
/// border or rule), widened to one pixel about its centre in device space, as PDF
/// renderers draw thin lines. Anti-aliasing would otherwise paint it as faint grey.
fn thin_rule(path: &Path, canvas: Transform) -> Option<tiny_skia::Rect> {
    if canvas.kx != 0.0 || canvas.ky != 0.0 {
        return None;
    }
    let b = path.bounds();
    let points = path.points();
    let on_edge = |v: f32, a: f32, z: f32| (v - a).abs() < 1e-3 || (v - z).abs() < 1e-3;
    if points.len() > 5
        || !points
            .iter()
            .all(|q| on_edge(q.x, b.left(), b.right()) && on_edge(q.y, b.top(), b.bottom()))
    {
        return None;
    }
    let d = b.transform(canvas)?;
    let (w, h) = (d.width(), d.height());
    if w >= 1.0 && h >= 1.0 {
        return None;
    }
    let (cx, cy) = (d.left() + w / 2.0, d.top() + h / 2.0);
    let (w, h) = (w.max(1.0), h.max(1.0));
    tiny_skia::Rect::from_xywh(cx - w / 2.0, cy - h / 2.0, w, h)
}

fn stroke_path(
    pm: &mut Pixmap,
    path: &Path,
    s: &Stroke,
    canvas: Transform,
    mask: Option<&Mask>,
    doc: &dyn RenderAssets,
    images: &ImageCache,
) {
    let mut holder = None;
    let Some(sh) = shader(&s.paint, doc, images, &mut holder) else {
        return;
    };
    let mut paint = SkPaint {
        shader: sh,
        ..Default::default()
    };
    let width = s.width as f32;
    let mut stroke = SkStroke {
        width,
        line_cap: match s.cap {
            LineCap::Flat => tiny_skia::LineCap::Butt,
            LineCap::Round => tiny_skia::LineCap::Round,
            LineCap::Square => tiny_skia::LineCap::Square,
        },
        line_join: match s.join {
            LineJoin::Miter => tiny_skia::LineJoin::Miter,
            LineJoin::Round => tiny_skia::LineJoin::Round,
            LineJoin::Bevel => tiny_skia::LineJoin::Bevel,
        },
        ..Default::default()
    };
    if !s.dash.is_empty() && s.dash.len().is_multiple_of(2) {
        let intervals: Vec<f32> = s
            .dash
            .iter()
            .map(|d| (*d as f32 * width).max(0.1))
            .collect();
        stroke.dash = StrokeDash::new(intervals, 0.0);
    }
    paint.anti_alias = anti_alias_fits(path, canvas, stroke_outset(&stroke));
    pm.stroke_path(path, &paint, &stroke, canvas, mask);
    if s.head.is_some() || s.tail.is_some() {
        let (head, tail) = endpoints(path);
        let color_paint = paint.clone();
        if let (Some(a), Some((pt, dir))) = (s.head, head) {
            draw_arrow(pm, a, pt, dir, width, &color_paint, canvas, mask);
        }
        if let (Some(a), Some((pt, dir))) = (s.tail, tail) {
            draw_arrow(pm, a, pt, dir, width, &color_paint, canvas, mask);
        }
    }
}

type End = Option<(Point, Point)>;

/// The first and last points of a path with the outward direction at each.
fn endpoints(path: &Path) -> (End, End) {
    use tiny_skia::PathSegment as S;
    let mut first: Option<Point> = None;
    let mut first_next: Option<Point> = None;
    let mut last = Point::zero();
    let mut prev = Point::zero();
    for seg in path.segments() {
        match seg {
            S::MoveTo(p) => {
                if first.is_none() {
                    first = Some(p);
                }
                last = p;
            }
            S::LineTo(p) => {
                if first_next.is_none() {
                    first_next = Some(p);
                }
                prev = last;
                last = p;
            }
            S::QuadTo(c, p) => {
                if first_next.is_none() {
                    first_next = Some(c);
                }
                prev = c;
                last = p;
            }
            S::CubicTo(c1, c2, p) => {
                if first_next.is_none() {
                    first_next = Some(c1);
                }
                prev = c2;
                last = p;
            }
            S::Close => return (None, None),
        }
    }
    let head = match (first, first_next) {
        (Some(a), Some(b)) => Some((a, Point::from_xy(a.x - b.x, a.y - b.y))),
        _ => None,
    };
    let tail = Some((last, Point::from_xy(last.x - prev.x, last.y - prev.y)));
    (head, tail)
}

#[expect(
    clippy::too_many_arguments,
    reason = "an arrowhead needs its tip, direction, width, paint, transform and clip mask"
)]
fn draw_arrow(
    pm: &mut Pixmap,
    a: Arrow,
    tip: Point,
    dir: Point,
    width: f32,
    paint: &SkPaint,
    canvas: Transform,
    mask: Option<&Mask>,
) {
    let len = (dir.x * dir.x + dir.y * dir.y).sqrt();
    if len < 1e-6 || a.kind == ArrowKind::None {
        return;
    }
    let (ux, uy) = (dir.x / len, dir.y / len);
    let (nx, ny) = (-uy, ux);
    let w = (a.w as f32 * width.max(1.0)) / 2.0;
    let l = a.len as f32 * width.max(1.0);
    let base = Point::from_xy(tip.x - ux * l, tip.y - uy * l);
    let mut pb = PathBuilder::new();
    match a.kind {
        ArrowKind::Oval => {
            let c = Point::from_xy(tip.x - ux * l / 2.0, tip.y - uy * l / 2.0);
            if let Some(r) = tiny_skia::Rect::from_xywh(c.x - w, c.y - w, 2.0 * w, 2.0 * w) {
                pb.push_oval(r);
            }
        }
        ArrowKind::Diamond => {
            let c = Point::from_xy(tip.x - ux * l / 2.0, tip.y - uy * l / 2.0);
            pb.move_to(tip.x, tip.y);
            pb.line_to(c.x + nx * w, c.y + ny * w);
            pb.line_to(base.x, base.y);
            pb.line_to(c.x - nx * w, c.y - ny * w);
            pb.close();
        }
        ArrowKind::Stealth => {
            let notch = Point::from_xy(tip.x - ux * l * 0.6, tip.y - uy * l * 0.6);
            pb.move_to(tip.x, tip.y);
            pb.line_to(base.x + nx * w, base.y + ny * w);
            pb.line_to(notch.x, notch.y);
            pb.line_to(base.x - nx * w, base.y - ny * w);
            pb.close();
        }
        ArrowKind::Arrow => {
            // Open arrow: two strokes.
            let mut sb = PathBuilder::new();
            sb.move_to(base.x + nx * w, base.y + ny * w);
            sb.line_to(tip.x, tip.y);
            sb.line_to(base.x - nx * w, base.y - ny * w);
            if let Some(p) = sb.finish() {
                let stroke = SkStroke {
                    width,
                    line_join: tiny_skia::LineJoin::Round,
                    ..Default::default()
                };
                let mut paint = paint.clone();
                paint.anti_alias = anti_alias_fits(&p, canvas, stroke_outset(&stroke));
                pm.stroke_path(&p, &paint, &stroke, canvas, mask);
            }
            return;
        }
        _ => {
            pb.move_to(tip.x, tip.y);
            pb.line_to(base.x + nx * w, base.y + ny * w);
            pb.line_to(base.x - nx * w, base.y - ny * w);
            pb.close();
        }
    }
    if let Some(p) = pb.finish() {
        let mut paint = paint.clone();
        paint.anti_alias = anti_alias_fits(&p, canvas, 0.0);
        pm.fill_path(&p, &paint, FillRule::Winding, canvas, mask);
    }
}

fn draw_image(
    pm: &mut Pixmap,
    img: &ImageItem,
    canvas: Transform,
    mask: Option<&Mask>,
    doc: &dyn RenderAssets,
    images: &ImageCache,
) {
    let Some(src) = images.get(doc, &img.image) else {
        return;
    };
    let (iw, ih) = (src.width() as f32, src.height() as f32);
    let (l, t, r, b) = (
        img.crop.0 as f32,
        img.crop.1 as f32,
        img.crop.2 as f32,
        img.crop.3 as f32,
    );
    let vis_w = (1.0 - l - r).max(1e-6);
    let vis_h = (1.0 - t - b).max(1e-6);
    // Image pixel → unit square of the placed picture.
    let px_to_unit = Transform::from_translate(-l * iw, -t * ih)
        .post_scale(1.0 / (vis_w * iw), 1.0 / (vis_h * ih));
    // The fill transform (unit square → device) also applies to the shader.
    let pattern = Pattern::new(
        src.as_ref().as_ref(),
        SpreadMode::Pad,
        FilterQuality::Bicubic,
        img.alpha,
        px_to_unit,
    );
    let mut paint = SkPaint {
        shader: pattern,
        ..Default::default()
    };
    // The visible area is the unit square, minus anything outside the source
    // (negative crops extend the frame with transparency).
    let ux0 = (-l / vis_w).max(0.0);
    let uy0 = (-t / vis_h).max(0.0);
    let ux1 = ((1.0 - l) / vis_w).min(1.0);
    let uy1 = ((1.0 - t) / vis_h).min(1.0);
    let Some(rect) = tiny_skia::Rect::from_ltrb(ux0, uy0, ux1.max(ux0 + 1e-6), uy1.max(uy0 + 1e-6))
    else {
        return;
    };
    let unit = PathBuilder::from_rect(rect);
    let tf = img.transform.post_concat(canvas);
    paint.anti_alias = anti_alias_fits(&unit, tf, 0.0);
    match &img.clip {
        Some(clip) => {
            let Some(mut m) = Mask::new(pm.width(), pm.height()) else {
                return;
            };
            let aa = anti_alias_fits(clip, canvas, 0.0);
            m.fill_path(clip, FillRule::Winding, aa, canvas);
            if let Some(outer) = mask {
                intersect_masks(&mut m, outer);
            }
            pm.fill_path(&unit, &paint, FillRule::Winding, tf, Some(&m));
        }
        None => pm.fill_path(&unit, &paint, FillRule::Winding, tf, mask),
    }
}

fn intersect_masks(m: &mut Mask, other: &Mask) {
    for (a, b) in m.data_mut().iter_mut().zip(other.data()) {
        *a = ((*a as u16 * *b as u16) / 255) as u8;
    }
}

fn draw_text(
    pm: &mut Pixmap,
    run: &GlyphRun,
    canvas: Transform,
    mask: Option<&Mask>,
    fonts: &FontSystem,
) {
    if (run.color.a <= 0.0 && run.outline.is_none()) || !run.drawn {
        return;
    }
    let Some(fd) = fonts.face(run.face) else {
        return;
    };
    let scale = (run.size / fd.metrics.units_per_em as f64) as f32;
    let mut paint = SkPaint {
        shader: Shader::SolidColor(sk_color(run.color)),
        ..Default::default()
    };
    let base = run.transform.post_concat(canvas);
    for g in &run.glyphs {
        let Some(outline) = fonts.outline(run.face, g.id) else {
            continue;
        };
        let mut t = Transform::from_scale(scale, -scale);
        if run.fake_italic {
            t = t.post_concat(Transform::from_row(1.0, 0.0, -0.2, 1.0, 0.0, 0.0));
        }
        let t = t.post_translate(g.x as f32, g.y as f32);
        // A warped glyph comes back in slide space, so widths are in points (unit 1).
        let (outline, t, scale) = match &run.warp {
            Some(w) => match w.warp_path(&outline, t) {
                Some(p) => (p, canvas, 1.0),
                None => continue,
            },
            None => (outline, t.post_concat(base), scale),
        };
        if run.color.a > 0.0 {
            paint.anti_alias = anti_alias_fits(&outline, t, 0.0);
            pm.fill_path(&outline, &paint, FillRule::Winding, t, mask);
        }
        if let Some((color, width)) = run.outline {
            // Centred on the glyph edge, as PowerPoint strokes text outlines.
            let stroke = SkStroke {
                width: (width as f32 / scale).max(0.0),
                line_join: tiny_skia::LineJoin::Round,
                ..Default::default()
            };
            let line = SkPaint {
                shader: Shader::SolidColor(sk_color(color)),
                anti_alias: anti_alias_fits(&outline, t, stroke_outset(&stroke)),
                ..Default::default()
            };
            pm.stroke_path(&outline, &line, &stroke, t, mask);
        }
        if run.color.a > 0.0 && run.fake_bold {
            let stroke = SkStroke {
                width: (run.size as f32 * 0.03) / scale,
                ..Default::default()
            };
            paint.anti_alias = anti_alias_fits(&outline, t, stroke_outset(&stroke));
            pm.stroke_path(&outline, &paint, &stroke, t, mask);
        }
    }
}

/// Draws a shadow: the silhouette rasterized into a mask at the shadow offset,
/// box-blurred three times (a Gaussian approximation), then filled with the
/// shadow color through the mask.
fn draw_shadow(
    pm: &mut Pixmap,
    s: &ShadowItem,
    canvas: Transform,
    clip: Option<&Mask>,
    fonts: &FontSystem,
    scale: f32,
    (doc, images): (&dyn RenderAssets, &ImageCache),
) {
    let Some(mut mask) = Mask::new(pm.width(), pm.height()) else {
        return;
    };
    let offset =
        Transform::from_translate(s.shadow.dx as f32, s.shadow.dy as f32).post_concat(canvas);
    // Device bounds of everything filled, so later steps skip the empty page.
    let mut ink: Option<(f32, f32, f32, f32)> = None;
    let mut add = |path: &Path, t: Transform| match path.bounds().transform(t) {
        Some(b) => {
            let (l, t, r, bt) = ink.unwrap_or((b.left(), b.top(), b.right(), b.bottom()));
            ink = Some((
                l.min(b.left()),
                t.min(b.top()),
                r.max(b.right()),
                bt.max(b.bottom()),
            ));
        }
        None => ink = Some((f32::MIN, f32::MIN, f32::MAX, f32::MAX)),
    };
    match &s.source {
        ShadowSource::Path(p) => {
            let aa = anti_alias_fits(p, offset, 0.0);
            mask.fill_path(p, FillRule::Winding, aa, offset);
            add(p, offset);
        }
        ShadowSource::Image(img) => {
            let Some((m, unit)) = softedge::image_alpha(pm, img, offset, doc, images) else {
                return;
            };
            mask = m;
            add(&unit, img.transform.post_concat(offset));
        }
        ShadowSource::Glyphs(run) => {
            let Some(fd) = fonts.face(run.face) else {
                return;
            };
            let k = (run.size / fd.metrics.units_per_em as f64) as f32;
            let base = run.transform.post_concat(offset);
            for g in &run.glyphs {
                if let Some(outline) = fonts.outline(run.face, g.id) {
                    let mut t = Transform::from_scale(k, -k);
                    if run.fake_italic {
                        t = t.post_concat(Transform::from_row(1.0, 0.0, -0.2, 1.0, 0.0, 0.0));
                    }
                    let t = t.post_translate(g.x as f32, g.y as f32);
                    let (outline, t) = match &run.warp {
                        Some(w) => match w.warp_path(&outline, t) {
                            Some(p) => (p, offset),
                            None => continue,
                        },
                        None => (outline, t.post_concat(base)),
                    };
                    let aa = anti_alias_fits(&outline, t, 0.0);
                    mask.fill_path(&outline, FillRule::Winding, aa, t);
                    add(&outline, t);
                }
            }
        }
    }
    // blurRad is the full blur extent; three box passes of r ≈ blur/3 approximate it.
    // Blur and composite only the region the shadow can cover.
    let (w, h) = (pm.width() as usize, pm.height() as usize);
    let Some(ink) = ink else {
        return;
    };
    let within = blur::reach(ink, w, h);
    if s.shadow.soft > 0.0 {
        softedge::feather_mask(&mut mask, s.shadow.soft as f32 * scale, within);
    }
    let radius = ((s.shadow.blur as f32 * scale) / 3.0).round() as usize;
    let region = if s.shadow.glow {
        softedge::glow_mask(&mut mask, s.shadow.blur as f32 * scale, within)
    } else if radius > 0 {
        blur::box_blur(&mut mask, radius.min(64), within)
    } else {
        blur::ink_bounds(mask.data(), w, within)
    };
    let Some((x0, y0, x1, y1)) = region else {
        return;
    };
    if let Some(c) = clip {
        for y in y0..y1 {
            let row = y * w + x0..y * w + x1;
            for (a, b) in mask.data_mut()[row.clone()].iter_mut().zip(&c.data()[row]) {
                *a = ((*a as u16 * *b as u16) / 255) as u8;
            }
        }
    }
    let paint = SkPaint {
        shader: Shader::SolidColor(sk_color(s.shadow.color)),
        anti_alias: true,
        ..Default::default()
    };
    if let Some(rect) = tiny_skia::Rect::from_ltrb(x0 as f32, y0 as f32, x1 as f32, y1 as f32) {
        pm.fill_rect(rect, &paint, Transform::identity(), Some(&mask));
    }
}

#[cfg(test)]
mod tests;
