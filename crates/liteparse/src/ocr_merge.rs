use std::sync::Arc;

use crate::error::LiteParseError;
use crate::ocr::{OcrEngine, OcrOptions, OcrResult};
use crate::types::{CutAxis, Page, ParsedPage, Region, RegionKind, TextItem};
use pdfium::{Document, ImageBounds};
use serde::Serialize;

/// Minimum dark filled-path area (pt², ~72 DPI page space) not covered by
/// native text before a page is sent to OCR. 400 pt² is roughly one word at
/// a 10–12pt size; anything smaller is likely a bullet, icon, dot leader, or
/// decoration whose loss is acceptable. A false trigger only costs an extra
/// OCR pass (the overlap filter discards OCR results that duplicate native
/// text); measured on a 121-page financial report, the trigger fires on ~3
/// pages at this threshold.
const UNCOVERED_VECTOR_AREA_THRESHOLD: f32 = 400.0;

/// Minimum side length (pt) for a raster image object to count toward image
/// coverage; smaller objects (rule lines, bullets, icons) are ignored.
const MIN_IMAGE_SIZE_PT: f32 = 25.0;

/// A single image at or above this fraction of the page is treated as a
/// full-page background and ignored.
const MAX_IMAGE_PAGE_COVERAGE: f32 = 0.9;

/// An XY-cut subtree must hold at least this many text items to count as a
/// text column, regardless of page size.
const MIN_COLUMN_ITEMS: usize = 8;

/// ...and at least this fraction of the page's text items. Together with
/// `MIN_COLUMN_ITEMS` this keeps thin sidebars, figure captions, and margin
/// notes from flagging a page as multi-column.
const MIN_COLUMN_ITEM_FRACTION: f32 = 0.15;

/// Figure coverage at or above this fraction of the page area flags
/// `DenseGraphics`.
const DENSE_GRAPHICS_MIN_COVERAGE: f32 = 0.2;

/// Owned page bitmap prepared for OCR. Indices refer to positions in the `pages` slice.
pub(crate) struct RenderedPage {
    pub idx: usize,
    /// Tightly-packed pixels: 1 byte/px (grayscale) or 3 (RGB).
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// Why a page was flagged as needing more than the cheap text-only path.
/// Multiple reasons can apply to one page (e.g. a sparse page whose little
/// text is also garbled). Empty exactly when `needs_ocr` is false.
///
/// This is the discriminator a caller routes on: a scan goes to OCR, dense
/// vector text to a vector-aware pass, and so on. New variants will be added
/// as the routing function learns to recommend heavier pipelines (tables,
/// charts, LLM passes), so callers should treat unknown variants leniently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ComplexityReason {
    /// A single raster covers essentially the whole page and there is little or
    /// no extractable text behind it — a scanned/photographed page.
    Scanned,
    /// Almost no extractable native text, and no full-page raster behind it
    /// (a blank page, or a near-empty cover/divider).
    NoText,
    /// Some real text, but it covers very little of the page — typically a
    /// figure-heavy page with only thin captions.
    SparseText,
    /// Substantial embedded raster figures sit alongside the native text.
    EmbeddedImages,
    /// The native text decodes to garbage (broken cmap / Type3 char-code
    /// fallback), so the visible glyphs and the extracted text disagree.
    Garbled,
    /// Text is painted as filled vector outlines, outside the text layer, so
    /// no native text items represent it.
    VectorText,
}

impl ComplexityReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            ComplexityReason::Scanned => "scanned",
            ComplexityReason::NoText => "no-text",
            ComplexityReason::SparseText => "sparse-text",
            ComplexityReason::EmbeddedImages => "embedded-images",
            ComplexityReason::Garbled => "garbled",
            ComplexityReason::VectorText => "vector-text",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct PageComplexityStats {
    pub page_number: usize,
    pub text_length: usize,
    pub text_coverage: f32,
    pub has_substantial_images: bool,
    /// Number of raster image objects counted on the page, after the size and
    /// single-image coverage filters.
    pub image_block_count: usize,
    /// Combined area of the counted images over the page area, clamped to 1.0.
    /// Stacked or overlapping images can inflate the raw sum past the truly
    /// covered fraction, hence the clamp — read it as "summed image-bbox area,"
    /// not unique covered area.
    pub image_coverage: f32,
    /// Area of the single largest counted image over the page area, clamped to
    /// 1.0. Useful for telling a single full-bleed scan apart from many small
    /// inline figures that sum to a similar `image_coverage`.
    pub largest_image_coverage: f32,
    /// A single raster covering ≥90% of the page is present. Such full-page
    /// backgrounds are excluded from `image_coverage`/`largest_image_coverage`
    /// (they're not inline figures), so this flag is the only signal that
    /// distinguishes a scan from a genuinely blank page — both otherwise report
    /// no text and no counted images.
    pub full_page_image: bool,
    /// Filled vector-outline area not covered by native text, in pt². `None`
    /// when a cheaper predicate already flagged the page for OCR, so this
    /// expensive page-object walk was skipped (it wasn't the deciding signal).
    pub uncovered_vector_area: Option<f32>,
    pub is_garbled: bool,
    pub page_area: f32,
    /// Whether the page needs more than the cheap text-only path. Equivalent to
    /// `!reasons.is_empty()`; kept as a flat bool for the common predicate case
    /// and as the internal per-page OCR gate.
    pub needs_ocr: bool,
    /// Every reason the page was flagged, in no particular priority order.
    pub reasons: Vec<ComplexityReason>,
    /// Layout-difficulty signals (columns, tables, dense graphics), computed
    /// from the post-projection page. Orthogonal to `needs_ocr`/`reasons`:
    /// none of these imply OCR. `None` in the internal per-page OCR gate
    /// (`render_pages_for_ocr`), which never runs projection — only
    /// `is_complex()` and `include_complexity` parses populate it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layout: Option<LayoutComplexityStats>,
}

pub(crate) fn calculate_page_complexity(
    page: &Page,
    page_obj: &pdfium::Page,
) -> Result<PageComplexityStats, LiteParseError> {
    // Count only usable native text. Substitution-cipher-style corrupt
    // encodings (e.g. PDFs with a broken cmap) produce long "text" that looks
    // populated but is unreadable — without this, such pages bypass OCR
    // because text_length >= 20 and coverage looks fine. The same applies to
    // unmappable items (Type3 fonts with no ToUnicode), whose text is a
    // char-code fallback and whose bounding boxes come from deceptive
    // declared metrics.
    let text_length: usize = page
        .text_items
        .iter()
        .filter(|item| !is_unusable_native(item))
        .map(|item| item.text.len())
        .sum();
    // Collect every raster ≥ MIN_IMAGE_SIZE_PT, including full-page ones, so a
    // scan can be told apart from a blank page. The "counted" subset below then
    // drops full-page backgrounds, matching the old
    // `image_bounds(.., MAX_IMAGE_PAGE_COVERAGE)` behaviour for inline figures.
    let pw = page.page_width;
    let ph = page.page_height;
    let all_images = page_obj.image_bounds(MIN_IMAGE_SIZE_PT, f32::INFINITY);
    let is_full_page = |b: &ImageBounds| {
        b.width > pw * MAX_IMAGE_PAGE_COVERAGE && b.height > ph * MAX_IMAGE_PAGE_COVERAGE
    };
    let full_page_image = all_images.iter().any(is_full_page);
    let image_bounds: Vec<&ImageBounds> = all_images.iter().filter(|b| !is_full_page(b)).collect();
    let has_images = !image_bounds.is_empty();

    let page_area = pw * ph;

    let (image_area_sum, largest_image_area) =
        image_bounds
            .iter()
            .fold((0.0_f32, 0.0_f32), |(sum, max), b| {
                let area = b.width.max(0.0) * b.height.max(0.0);
                (sum + area, max.max(area))
            });
    let (image_coverage, largest_image_coverage) = if page_area > 0.0 {
        (
            (image_area_sum / page_area).min(1.0),
            (largest_image_area / page_area).min(1.0),
        )
    } else {
        (0.0, 0.0)
    };
    let text_bbox_area: f32 = page
        .text_items
        .iter()
        .filter(|item| !is_unusable_native(item))
        .map(|item| item.width * item.height)
        .sum();
    let text_coverage = if page_area > 0.0 {
        text_bbox_area / page_area
    } else {
        0.0
    };

    // Low spatial coverage only signals a scan/sparse page when there also
    // isn't much native text. A text-dense page (e.g. a ruled table with
    // wide intra-cell whitespace) is spatially sparse but needs no OCR.
    let sparse_text = text_length < 2000 && text_coverage < 0.15;
    let is_garbled = page_is_garbled(page);

    let mut reasons = Vec::new();
    if text_length < 20 {
        // Too little text to be the page's content. A full-page raster behind
        // it means a scan; otherwise it's effectively blank.
        reasons.push(if full_page_image {
            ComplexityReason::Scanned
        } else {
            ComplexityReason::NoText
        });
    } else if sparse_text {
        // There is real text, but it's too thin to be the whole page.
        reasons.push(ComplexityReason::SparseText);
    }
    if has_images {
        reasons.push(ComplexityReason::EmbeddedImages);
    }
    if is_garbled {
        reasons.push(ComplexityReason::Garbled);
    }
    let mut needs_ocr = !reasons.is_empty();

    // Text drawn as filled vector outlines lives outside the text layer
    // entirely: no text items, no image XObjects, so none of the cheap
    // predicates fire on such a text-dense page. Detect it by measuring filled
    // path area that native text doesn't account for. Checked last so this
    // relatively expensive page-object walk only runs when the cheap predicates
    // all pass; when they don't, the area is left unmeasured (`None`).
    let uncovered_vector_area = if !needs_ocr {
        let path_bounds = page_obj.filled_path_bounds(3.0, 0.9);
        let uncovered = uncovered_path_area(&path_bounds, &page.text_items);
        if uncovered >= UNCOVERED_VECTOR_AREA_THRESHOLD {
            needs_ocr = true;
            reasons.push(ComplexityReason::VectorText);
        }
        Some(uncovered)
    } else {
        None
    };

    Ok(PageComplexityStats {
        page_number: page.page_number,
        text_length,
        text_coverage,
        has_substantial_images: has_images,
        image_block_count: image_bounds.len(),
        image_coverage,
        largest_image_coverage,
        full_page_image,
        uncovered_vector_area,
        is_garbled,
        page_area,
        needs_ocr,
        reasons,
        layout: None,
    })
}

/// Why a page's layout is expected to be hard to reconstruct faithfully.
/// Orthogonal to `ComplexityReason`: none of these imply OCR — they signal
/// that the text-only path may mangle reading order or structure, so a caller
/// can route the page to a higher-accuracy pipeline. Same leniency contract
/// as `ComplexityReason`: new variants will be added, treat unknowns kindly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum LayoutComplexityReason {
    /// The XY-cut layout tree splits the page into two or more side-by-side
    /// regions that each hold a substantial share of the text.
    MultiColumn,
    /// Table structure detected: ruled-line grid(s) and/or borderless
    /// column-aligned text runs — the same detectors the markdown emitter
    /// uses. Description lists (label/value pairs) do not count.
    TableLikely,
    /// Figure regions (charts, diagrams, decorated boxes) cover a large
    /// fraction of the page; their vector content is invisible to text
    /// extraction.
    DenseGraphics,
}

impl LayoutComplexityReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            LayoutComplexityReason::MultiColumn => "multi-column",
            LayoutComplexityReason::TableLikely => "table-likely",
            LayoutComplexityReason::DenseGraphics => "dense-graphics",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct LayoutComplexityStats {
    /// Side-by-side text columns found by the XY-cut layout pass (1 = single
    /// column). Only subtrees holding a substantial share of the page's text
    /// count, so a thin sidebar or caption does not inflate this.
    pub column_count: usize,
    /// Validated ruled-table runs on the page: grid components confirmed
    /// against the projected text, so a boxed callout or a chart's axis
    /// gridlines — lines without cell structure — don't count.
    pub ruled_table_count: usize,
    /// Combined validated ruled-table area (text extent) over page area,
    /// clamped to 1.0.
    pub ruled_table_coverage: f32,
    /// Borderless table runs found by the emitter's track-alignment detector
    /// over the projected lines (description lists excluded). Ruled tables'
    /// text rows also align to tracks, so this overlaps `ruled_table_count`
    /// and must not be summed with it.
    pub text_table_run_count: usize,
    /// Figure regions clustered from vector graphics.
    pub figure_count: usize,
    /// Combined figure area over page area, clamped to 1.0. Overlapping rects
    /// can inflate the sum past the truly covered fraction — same caveat as
    /// `image_coverage`.
    pub figure_coverage: f32,
    /// Whether any layout reason fired. Equivalent to `!reasons.is_empty()`,
    /// kept flat for the common predicate case (mirrors `needs_ocr`).
    pub is_complex: bool,
    pub reasons: Vec<LayoutComplexityReason>,
}

/// Compute layout-difficulty signals from a projected page. Read-only over
/// structures projection already built: the XY-cut region tree for columns,
/// `figures` for graphics density, and the emitter's own table detectors —
/// validated ruled grids and borderless track-aligned runs — over the
/// projected lines and vector primitives.
pub(crate) fn calculate_layout_complexity(page: &ParsedPage) -> LayoutComplexityStats {
    let page_area = page.page_width * page.page_height;
    let total_items = page.text_items.len();

    let column_count = count_columns(&page.regions, total_items).max(1);

    let table_rects = crate::markdown_layout::validated_ruled_table_rects(
        &page.projected_lines,
        &page.graphics,
        page.page_width,
        page.page_height,
    );
    let text_table_run_count = crate::markdown_layout::count_text_table_runs(&page.projected_lines);
    let table_area: f32 = table_rects
        .iter()
        .map(|r| r.width.max(0.0) * r.height.max(0.0))
        .sum();
    let figure_area: f32 = page
        .figures
        .iter()
        .map(|r| r.width.max(0.0) * r.height.max(0.0))
        .sum();
    // The `area > 0.0` guard also normalizes the `-0.0` an empty `f32` sum
    // yields, which would otherwise serialize as `-0.0` in JSON.
    let coverage = |area: f32| {
        if page_area > 0.0 && area > 0.0 {
            (area / page_area).min(1.0)
        } else {
            0.0
        }
    };
    let ruled_table_coverage = coverage(table_area);
    let figure_coverage = coverage(figure_area);

    let mut reasons = Vec::new();
    if column_count >= 2 {
        reasons.push(LayoutComplexityReason::MultiColumn);
    }
    if !table_rects.is_empty() || text_table_run_count > 0 {
        reasons.push(LayoutComplexityReason::TableLikely);
    }
    if figure_coverage >= DENSE_GRAPHICS_MIN_COVERAGE {
        reasons.push(LayoutComplexityReason::DenseGraphics);
    }

    LayoutComplexityStats {
        column_count,
        ruled_table_count: table_rects.len(),
        ruled_table_coverage,
        text_table_run_count,
        figure_count: page.figures.len(),
        figure_coverage,
        is_complex: !reasons.is_empty(),
        reasons,
    }
}

fn region_item_count(region: &Region) -> usize {
    match &region.kind {
        RegionKind::Leaf { item_indices } => item_indices.len(),
        RegionKind::Split { children, .. } => children.iter().map(region_item_count).sum(),
    }
}

/// Count side-by-side text columns in an XY-cut subtree. Subtrees below the
/// substantiality bar contribute zero. Vertical splits sum their children
/// (side-by-side regions), horizontal splits take the max of theirs (stacked
/// bands — a full-width title above a two-column body is a two-column page).
/// Nested vertical splits therefore count correctly: a three-column page cut
/// as `V(a, V(b, c))` yields 3.
fn count_columns(region: &Region, total_items: usize) -> usize {
    let min_items =
        MIN_COLUMN_ITEMS.max((total_items as f32 * MIN_COLUMN_ITEM_FRACTION).ceil() as usize);
    if region_item_count(region) < min_items {
        return 0;
    }
    match &region.kind {
        RegionKind::Leaf { .. } => 1,
        RegionKind::Split { axis, children } => {
            let counts = children.iter().map(|c| count_columns(c, total_items));
            match axis {
                CutAxis::Vertical => counts.sum(),
                CutAxis::Horizontal => counts.max().unwrap_or(0),
            }
        }
    }
}

/// Render pages that need OCR from an already-open document.
///
/// The pdfium `Document` holds raw pointers that are not `Send`, so callers must
/// drop it before awaiting the OCR engine.
pub(crate) fn render_pages_for_ocr(
    document: &Document,
    pages: &[Page],
    dpi: f32,
    grayscale: bool,
) -> Result<Vec<RenderedPage>, LiteParseError> {
    let mut rendered = Vec::new();
    for (idx, page) in pages.iter().enumerate() {
        let page_obj = document.page((page.page_number - 1) as i32)?;
        let page_complexity = calculate_page_complexity(page, &page_obj)?;

        if !page_complexity.needs_ocr {
            continue;
        }

        let bitmap = page_obj.render(dpi)?;
        let width = bitmap.width() as u32;
        let height = bitmap.height() as u32;
        // Grayscale or RGB per the engine; see `OcrEngine::prefers_grayscale`.
        let pixels = if grayscale {
            bitmap.to_luma()
        } else {
            bitmap.to_rgb()
        };

        rendered.push(RenderedPage {
            idx,
            pixels,
            width,
            height,
        });
    }
    Ok(rendered)
}

/// Run OCR on pre-rendered page bitmaps and merge results into `pages`.
pub(crate) async fn ocr_and_merge_rendered(
    pages: &mut [Page],
    rendered: Vec<RenderedPage>,
    dpi: f32,
    ocr_engine: Arc<dyn OcrEngine>,
    ocr_language: &str,
    num_workers: usize,
    ocr_failure_fatal: bool,
) -> Result<(), LiteParseError> {
    // Phase 1: spawn one async task per page. A semaphore limits how many run
    // `recognize` concurrently to `num_workers`.
    //
    // The permit MUST be acquired in async context (`acquire_owned().await`),
    // not inside `spawn_blocking` via `block_on`. Acquiring it on a blocking
    // thread parks that OS thread until a permit is free; with more pages than
    // tokio's blocking pool (default `max_blocking_threads = 512`), every pool
    // thread ends up parked waiting on the semaphore. The single task holding
    // the permit then calls `recognize`, whose HTTP client resolves DNS via its
    // own internal `spawn_blocking` — which can never get a thread, so the
    // request never goes out, the permit is never released, and the whole OCR
    // pass deadlocks. Acquiring the permit asynchronously parks the lightweight
    // task instead, so only `num_workers` blocking threads are ever consumed.
    let num_workers = num_workers.max(1);
    let semaphore = Arc::new(tokio::sync::Semaphore::new(num_workers));
    let mut handles = Vec::with_capacity(rendered.len());

    let handle = tokio::runtime::Handle::current();

    for r in rendered {
        let engine = ocr_engine.clone();
        let sem = semaphore.clone();
        let language = ocr_language.to_string();
        let page_number = pages[r.idx].page_number;
        let rt_handle = handle.clone();

        handles.push((
            r.idx,
            page_number,
            tokio::spawn(async move {
                // Park the task (not an OS thread) until a permit is available.
                let _permit = sem.acquire_owned().await.expect("semaphore closed");
                let options = OcrOptions { language, dpi };
                // Offload the (possibly CPU-blocking, e.g. Tesseract) recognize
                // onto a blocking thread. Because the permit is already held,
                // at most `num_workers` blocking threads are in use at once,
                // leaving the rest of the pool free for the HTTP client's
                // internal DNS resolution.
                match tokio::task::spawn_blocking(move || {
                    rt_handle.block_on(engine.recognize(&r.pixels, r.width, r.height, &options))
                })
                .await
                {
                    Ok(result) => result,
                    Err(join_err) => {
                        Err(Box::new(join_err) as Box<dyn std::error::Error + Send + Sync>)
                    }
                }
            }),
        ));
    }

    // Phase 3: collect results and merge into pages.
    let scale_factor = 72.0 / dpi;

    // Track OCR task outcomes so we can distinguish a systemic failure (e.g.
    // missing Tesseract language data, which fails identically on every page)
    // from incidental per-page failures. Without this, every page logs the same
    // error and `parse()` still returns "success" with no OCR text.
    //
    // We additionally track whether any *sparse-text* page failed: a page is
    // rendered for OCR if it has sparse native text OR merely contains an image
    // (`needs_ocr = text_length < 20 || text_coverage < 0.15 || has_images`).
    // A native-text PDF with a logo on every page is rendered for OCR
    // enrichment but already has all its text. We must only fail loud when OCR
    // failure destroyed a sparse page's likely primary text source — otherwise
    // a broken OCR setup would abort perfectly good native-text documents.
    let total_tasks = handles.len();
    let mut failed_tasks = 0usize;
    let mut failed_sparse_text_page = false;
    let mut first_error: Option<String> = None;

    for (idx, page_number, handle) in handles {
        let ocr_results: Vec<OcrResult> = match handle.await {
            Ok(Ok(results)) => results,
            Ok(Err(e)) => {
                failed_tasks += 1;
                failed_sparse_text_page |= page_has_sparse_native_text(&pages[idx]);
                // Only log the first failure to avoid flooding stderr with an
                // identical message for every page.
                if first_error.is_none() {
                    let msg = e.to_string();
                    eprintln!("[ocr] failed for page {}: {}", page_number, msg);
                    first_error = Some(msg);
                }
                continue;
            }
            Err(e) => {
                failed_tasks += 1;
                failed_sparse_text_page |= page_has_sparse_native_text(&pages[idx]);
                if first_error.is_none() {
                    let msg = e.to_string();
                    eprintln!("[ocr] task panicked for page {}: {}", page_number, msg);
                    first_error = Some(msg);
                }
                continue;
            }
        };

        if ocr_results.is_empty() {
            continue;
        }

        let page = &mut pages[idx];
        // Drop unusable native items (substitution-cipher cmap corruption, or
        // unmappable Type3 text) so OCR can replace them. Without this,
        // garbled-but-spatially-present native text suppresses every OCR
        // result that overlaps it via the overlap check below, leaving the
        // output stuck with unreadable text. We apply both per-item and
        // per-page checks: short garbled labels ("GDWH", "XVG") can't be
        // flagged alone, but their host page can.
        if page_is_garbled(page) {
            page.text_items.clear();
        } else {
            page.text_items.retain(|item| !is_unusable_native(item));
        }

        // Only check overlap against native (already-extracted) PDF text. Comparing
        // each OCR result against previously-accepted OCR results caused adjacent
        // OCR lines whose bounding boxes touched within tolerance to suppress each
        // other, dropping every second line on scanned pages.
        let native_count = page.text_items.len();
        for r in &ocr_results {
            if r.confidence <= 0.1 {
                continue;
            }

            // Prefer the screen-space axis-aligned bbox derived from the polygon
            // (when present) so rotated detections carry a tight upright bbox.
            // The polygon also lets us recover an explicit rotation angle so the
            // projector can route rotated sidebar text through its rotation
            // reading-order handler instead of mistaking it for body text.
            let (ocr_x, ocr_y, ocr_w, ocr_h, rotation) = match r.polygon {
                Some(poly) => {
                    let xs = [poly[0][0], poly[1][0], poly[2][0], poly[3][0]];
                    let ys = [poly[0][1], poly[1][1], poly[2][1], poly[3][1]];
                    let x_min = xs.iter().copied().fold(f32::INFINITY, f32::min);
                    let x_max = xs.iter().copied().fold(f32::NEG_INFINITY, f32::max);
                    let y_min = ys.iter().copied().fold(f32::INFINITY, f32::min);
                    let y_max = ys.iter().copied().fold(f32::NEG_INFINITY, f32::max);
                    let rot = polygon_rotation_deg(&poly);
                    (
                        x_min * scale_factor,
                        y_min * scale_factor,
                        (x_max - x_min) * scale_factor,
                        (y_max - y_min) * scale_factor,
                        rot,
                    )
                }
                None => (
                    r.bbox[0] * scale_factor,
                    r.bbox[1] * scale_factor,
                    (r.bbox[2] - r.bbox[0]) * scale_factor,
                    (r.bbox[3] - r.bbox[1]) * scale_factor,
                    0.0,
                ),
            };

            if overlaps_existing_text(
                &page.text_items[..native_count],
                ocr_x,
                ocr_y,
                ocr_w,
                ocr_h,
                2.0,
            ) {
                continue;
            }

            let cleaned = clean_ocr_table_artifacts(&r.text);
            if cleaned.is_empty() {
                continue;
            }

            // For native rotated text the font_size approximates line height,
            // which for 90/270° rotations corresponds to the *narrow* screen
            // dimension. Use the perpendicular extent for rotated OCR text so
            // downstream font-size heuristics stay sane.
            let font_size_hint = if rotation == 90.0 || rotation == 270.0 {
                ocr_w.max(1.0)
            } else {
                ocr_h
            };

            page.text_items.push(TextItem {
                text: cleaned,
                x: ocr_x,
                y: ocr_y,
                width: ocr_w,
                height: ocr_h,
                rotation,
                font_name: Some("OCR".to_string()),
                font_size: Some(font_size_hint),
                confidence: Some((r.confidence * 1000.0).round() / 1000.0),
                ..Default::default()
            });
        }
    }

    // If every OCR task failed *and* at least one of those failures was on a
    // sparse-text page (the same length/coverage predicate that sends pages to
    // OCR as text-poor in `render_pages_for_ocr`), treat it as a systemic
    // failure. Returning an error surfaces the root cause (e.g. missing language
    // data) instead of silently emitting an empty or mostly-empty page. We
    // deliberately do NOT fail when the only failures were on pages that already
    // had substantial native text and were merely rendered for image-based OCR
    // enrichment — a broken OCR setup must not abort an otherwise-good
    // native-text document.
    if total_tasks > 0 && failed_tasks == total_tasks && failed_sparse_text_page {
        let detail = first_error.unwrap_or_else(|| "unknown error".to_string());
        if ocr_failure_fatal {
            return Err(LiteParseError::Ocr(format!(
                "OCR failed for all {} page(s): {}",
                total_tasks, detail
            )));
        }
        // Non-fatal mode: the caller prefers partial results over a hard abort,
        // so keep whatever native text was extracted and continue. Surface the
        // root cause as a warning so a broken OCR setup is still visible.
        eprintln!(
            "[ocr] OCR failed for all {} page(s): {} — continuing with partial (native-text) results (ocr_failure_fatal=false)",
            total_tasks, detail
        );
    }

    // Surface a concise summary for partial failures without flooding stderr.
    if failed_tasks > 0 {
        eprintln!(
            "[ocr] {}/{} page(s) failed OCR; continuing with partial results",
            failed_tasks, total_tasks
        );
    }

    Ok(())
}

/// True when the page's native (already-extracted) text is sparse enough that
/// OCR is likely its primary text source. Mirrors the non-image predicates in
/// `render_pages_for_ocr` (`text_length < 20 || text_coverage < 0.15`) so the
/// systemic-failure guard matches the same pages that were rendered because
/// their native text was insufficient.
fn page_has_sparse_native_text(page: &Page) -> bool {
    let text_length: usize = page
        .text_items
        .iter()
        .filter(|item| !is_unusable_native(item))
        .map(|item| item.text.len())
        .sum();
    let page_area = page.page_width * page.page_height;
    let text_bbox_area: f32 = page
        .text_items
        .iter()
        .filter(|item| !is_unusable_native(item))
        .map(|item| item.width * item.height)
        .sum();
    let text_coverage = if page_area > 0.0 {
        text_bbox_area / page_area
    } else {
        0.0
    };

    text_length < 20 || text_coverage < 0.15
}

/// A native text item that cannot be trusted as a text source: either its
/// Unicode mapping failed outright (Type3 fonts with no ToUnicode — the text
/// is a char-code fallback and the bbox comes from deceptive declared
/// metrics), or its content looks substitution-cipher garbled.
fn is_unusable_native(item: &TextItem) -> bool {
    item.has_unicode_map_error || is_likely_garbled(&item.text)
}

/// Total area of filled vector paths not accounted for by native text items.
/// Glyph outlines drawn as paths produce filled regions with no overlapping
/// text item; rules and table borders are stroke-only and already excluded
/// upstream, and shading rects behind real text are subtracted away by the
/// text overlap. Coverage is approximated by summing per-item intersections
/// (clamped to the path's own area), which can only over-estimate coverage —
/// i.e. err toward not triggering OCR.
fn uncovered_path_area(paths: &[ImageBounds], items: &[TextItem]) -> f32 {
    let mut uncovered = 0.0f32;
    for p in paths {
        let p_area = p.width * p.height;
        if p_area <= 0.0 {
            continue;
        }
        let mut covered = 0.0f32;
        for item in items {
            let ix = (p.x + p.width).min(item.x + item.width) - p.x.max(item.x);
            let iy = (p.y + p.height).min(item.y + item.height) - p.y.max(item.y);
            if ix > 0.0 && iy > 0.0 {
                covered += ix * iy;
                if covered >= p_area {
                    break;
                }
            }
        }
        uncovered += (p_area - covered).max(0.0);
    }
    uncovered
}

/// Heuristic for substitution-cipher / broken-cmap garbling: real Latin-script
/// text has a vowel ratio of roughly 30–45%, but a substitution permutation
/// almost always maps the original A/E/I/O/U onto non-vowel letters, driving
/// the apparent vowel ratio to near zero. Texts without enough ASCII letters
/// to judge (non-Latin scripts, numbers, short labels) are treated as fine.
fn is_likely_garbled(text: &str) -> bool {
    let (letters, vowels) = count_letters_and_vowels(text);
    if letters < 10 {
        return false;
    }
    vowels * 10 < letters
}

fn count_letters_and_vowels(text: &str) -> (usize, usize) {
    let mut letters = 0usize;
    let mut vowels = 0usize;
    for ch in text.chars() {
        if ch.is_ascii_alphabetic() {
            letters += 1;
            if matches!(ch.to_ascii_lowercase(), 'a' | 'e' | 'i' | 'o' | 'u') {
                vowels += 1;
            }
        }
    }
    (letters, vowels)
}

/// Page-level garbled check: even when individual items are too short to judge
/// in isolation (e.g. "GDWH", "FXUUHQFB XVG"), a page whose aggregate vowel
/// ratio collapses to single digits is almost certainly substitution-encoded.
/// Used to drop all native items on the page before OCR merge, so short
/// garbled labels don't suppress overlapping OCR results.
fn page_is_garbled(page: &Page) -> bool {
    let mut total_letters = 0usize;
    let mut total_vowels = 0usize;
    for it in &page.text_items {
        let (l, v) = count_letters_and_vowels(&it.text);
        total_letters += l;
        total_vowels += v;
    }
    if total_letters < 30 {
        return false;
    }
    // Real Latin-script vowel ratios sit ~30–45% across English, Portuguese,
    // Spanish, French, etc. A page-wide ratio under 20% is well outside any
    // natural-language range and signals substitution-style corruption. (A
    // simple +3 Caesar shift still leaves some U/Y letters from the original
    // O/Y mapping, so a 10% bound is too tight to catch this in practice.)
    total_vowels * 5 < total_letters
}

/// Recover a discrete CCW rotation in degrees from a 4-point OCR polygon.
/// Returns one of 0.0, 90.0, 180.0, 270.0 — snapping to the nearest right
/// angle — or 0.0 for nearly-square/degenerate polygons.
///
/// Point ordering varies between OCR engines: some emit TL→TR→BR→BL in the
/// glyphs' upright reading frame (so poly[0]→poly[1] is always the reading
/// direction), but others (notably PaddleOCR 3.x with
/// `use_textline_orientation=True`) emit polygons in screen-axis order, where
/// poly[0]→poly[1] is always horizontal in screen space regardless of how the
/// text actually reads. To handle both, we pick the *longer* of the two
/// adjacent edges as the reading direction — the text always runs along the
/// long axis of its bounding quadrilateral.
fn polygon_rotation_deg(poly: &[[f32; 2]; 4]) -> f32 {
    let e0 = [poly[1][0] - poly[0][0], poly[1][1] - poly[0][1]];
    let e1 = [poly[2][0] - poly[1][0], poly[2][1] - poly[1][1]];
    let len0 = (e0[0] * e0[0] + e0[1] * e0[1]).sqrt();
    let len1 = (e1[0] * e1[0] + e1[1] * e1[1]).sqrt();
    if len0.max(len1) < 1.0 {
        return 0.0;
    }
    // Treat near-square polygons as un-rotated — there's no reliable reading
    // axis to pick from. Single-char/CJK detections fall in here.
    let (longer, shorter) = if len0 >= len1 {
        (len0, len1)
    } else {
        (len1, len0)
    };
    if shorter > 0.0 && longer / shorter < 1.3 {
        return 0.0;
    }
    let reading = if len0 >= len1 { e0 } else { e1 };
    // atan2 with screen-down y; negate to get the conventional CCW angle.
    let angle_ccw = -reading[1].atan2(reading[0]).to_degrees();
    let normalized = angle_ccw.rem_euclid(360.0);
    ((normalized / 90.0).round() as i32 * 90).rem_euclid(360) as f32
}

/// Check if an OCR bounding box overlaps with any existing text item.
fn overlaps_existing_text(
    items: &[TextItem],
    ocr_x: f32,
    ocr_y: f32,
    ocr_w: f32,
    ocr_h: f32,
    tolerance: f32,
) -> bool {
    for item in items {
        let item_right = item.x + item.width;
        let item_bottom = item.y + item.height;

        let overlap_x = ocr_x < item_right + tolerance && ocr_x + ocr_w > item.x - tolerance;
        let overlap_y = ocr_y < item_bottom + tolerance && ocr_y + ocr_h > item.y - tolerance;

        if overlap_x && overlap_y {
            return true;
        }
    }
    false
}

/// Clean common OCR artifacts from table border misreads.
/// OCR often misreads vertical table border lines as bracket-like characters.
fn clean_ocr_table_artifacts(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    // Strip leading/trailing border artifact characters: | [ ] ( ) { }
    let without_artifacts: &str = trimmed
        .trim_start_matches(['|', '[', ']', '(', ')', '{', '}'])
        .trim_end_matches(['|', '[', ']', '(', ')', '{', '}'])
        .trim();

    if without_artifacts.is_empty() {
        return trimmed.to_string();
    }

    // Only use cleaned version if core content looks numeric-ish
    // This avoids incorrectly stripping brackets from content like "(note)"
    let is_numeric_ish = without_artifacts
        .chars()
        .all(|c| c.is_ascii_digit() || matches!(c, ',' | '.' | ' ' | '%' | '-' | '+' | '*' | '/'))
        || without_artifacts == "N/A"
        || without_artifacts == "Z"
        || without_artifacts == "-";

    if is_numeric_ish {
        without_artifacts.to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Rect;

    fn leaf(n: usize) -> Region {
        Region {
            bbox: Rect::default(),
            kind: RegionKind::Leaf {
                item_indices: (0..n).collect(),
            },
        }
    }

    fn split(axis: CutAxis, children: Vec<Region>) -> Region {
        Region {
            bbox: Rect::default(),
            kind: RegionKind::Split { axis, children },
        }
    }

    #[test]
    fn test_count_columns_single_leaf() {
        assert_eq!(count_columns(&leaf(50), 50), 1);
    }

    #[test]
    fn test_count_columns_two_columns() {
        let root = split(CutAxis::Vertical, vec![leaf(40), leaf(40)]);
        assert_eq!(count_columns(&root, 80), 2);
    }

    #[test]
    fn test_count_columns_nested_three_columns() {
        // Three-column page cut as V(a, V(b, c)).
        let inner = split(CutAxis::Vertical, vec![leaf(30), leaf(30)]);
        let root = split(CutAxis::Vertical, vec![leaf(40), inner]);
        assert_eq!(count_columns(&root, 100), 3);
    }

    #[test]
    fn test_count_columns_header_over_two_columns() {
        // Full-width title band stacked above a two-column body: the page is
        // still two-column (horizontal splits take the max of their bands).
        let body = split(CutAxis::Vertical, vec![leaf(40), leaf(40)]);
        let root = split(CutAxis::Horizontal, vec![leaf(20), body]);
        assert_eq!(count_columns(&root, 100), 2);
    }

    #[test]
    fn test_count_columns_thin_sidebar_not_a_column() {
        // A 6-item sidebar next to a 94-item body fails both the absolute
        // (8-item) and fractional (15%) bars.
        let root = split(CutAxis::Vertical, vec![leaf(6), leaf(94)]);
        assert_eq!(count_columns(&root, 100), 1);
    }

    #[test]
    fn test_count_columns_narrow_table_slices_not_columns() {
        // Four narrow slices (10 items each of 100): each fails the 15%
        // fractional bar, so none count — table-ish V-splits stay quiet.
        let root = split(
            CutAxis::Vertical,
            vec![leaf(10), leaf(10), leaf(10), leaf(10)],
        );
        assert_eq!(count_columns(&root, 100), 0);
    }

    #[test]
    fn test_count_columns_empty_page() {
        assert_eq!(count_columns(&leaf(0), 0), 0);
    }

    #[test]
    fn test_polygon_rotation_horizontal() {
        let p = [[0.0, 0.0], [100.0, 0.0], [100.0, 20.0], [0.0, 20.0]];
        assert_eq!(polygon_rotation_deg(&p), 0.0);
    }

    #[test]
    fn test_polygon_rotation_90_ccw() {
        // Upright text rotated 90° CCW: TL→TR edge points upward (screen y decreasing).
        let p = [[10.0, 100.0], [10.0, 0.0], [30.0, 0.0], [30.0, 100.0]];
        assert_eq!(polygon_rotation_deg(&p), 90.0);
    }

    #[test]
    fn test_polygon_rotation_270_ccw() {
        // Upright text rotated 270° CCW (= 90° CW): TL→TR edge points downward.
        let p = [[10.0, 0.0], [10.0, 100.0], [30.0, 100.0], [30.0, 0.0]];
        assert_eq!(polygon_rotation_deg(&p), 270.0);
    }

    #[test]
    fn test_polygon_rotation_screen_axis_vertical() {
        // PaddleOCR-style: tall+narrow sidebar polygon in screen-axis order
        // (smallest-y first). poly[0]→poly[1] is the SHORT horizontal edge,
        // not the reading direction. The longer edge picks out the rotation.
        let p = [[20.0, 50.0], [50.0, 50.0], [50.0, 750.0], [20.0, 750.0]];
        let r = polygon_rotation_deg(&p);
        assert!(r == 90.0 || r == 270.0, "expected 90 or 270, got {r}");
    }

    #[test]
    fn test_polygon_rotation_near_square() {
        // Single-char detections (CJK glyphs, etc.) should not be classified
        // as rotated — there's no reliable reading axis.
        let p = [[0.0, 0.0], [20.0, 0.0], [20.0, 22.0], [0.0, 22.0]];
        assert_eq!(polygon_rotation_deg(&p), 0.0);
    }

    #[test]
    fn test_polygon_rotation_180() {
        let p = [[100.0, 20.0], [0.0, 20.0], [0.0, 0.0], [100.0, 0.0]];
        assert_eq!(polygon_rotation_deg(&p), 180.0);
    }

    #[test]
    fn test_clean_ocr_table_artifacts() {
        assert_eq!(clean_ocr_table_artifacts("44520]"), "44520");
        assert_eq!(clean_ocr_table_artifacts("|123"), "123");
        assert_eq!(clean_ocr_table_artifacts("0.3|"), "0.3");
        assert_eq!(clean_ocr_table_artifacts("(note)"), "(note)");
        assert_eq!(clean_ocr_table_artifacts("|hello|"), "|hello|");
        assert_eq!(clean_ocr_table_artifacts("N/A"), "N/A");
        assert_eq!(clean_ocr_table_artifacts(""), "");
        assert_eq!(clean_ocr_table_artifacts("|||"), "|||");
    }

    fn make_item(x: f32, y: f32, w: f32, h: f32) -> TextItem {
        TextItem {
            text: "x".into(),
            x,
            y,
            width: w,
            height: h,
            ..Default::default()
        }
    }

    #[test]
    fn test_overlaps_existing_text_inside() {
        let items = vec![make_item(10.0, 10.0, 20.0, 5.0)];
        assert!(overlaps_existing_text(&items, 12.0, 11.0, 5.0, 2.0, 2.0));
    }

    #[test]
    fn test_overlaps_existing_text_disjoint() {
        let items = vec![make_item(10.0, 10.0, 20.0, 5.0)];
        assert!(!overlaps_existing_text(&items, 100.0, 100.0, 5.0, 5.0, 2.0));
    }

    #[test]
    fn test_overlaps_existing_text_tolerance() {
        let items = vec![make_item(10.0, 10.0, 20.0, 5.0)];
        // Just outside but within tolerance
        assert!(overlaps_existing_text(&items, 31.0, 10.0, 5.0, 5.0, 2.0));
        // Beyond tolerance
        assert!(!overlaps_existing_text(&items, 35.0, 10.0, 5.0, 5.0, 2.0));
    }

    #[test]
    fn test_overlaps_empty() {
        assert!(!overlaps_existing_text(&[], 0.0, 0.0, 1.0, 1.0, 0.0));
    }

    fn pb(x: f32, y: f32, w: f32, h: f32) -> ImageBounds {
        ImageBounds {
            x,
            y,
            width: w,
            height: h,
        }
    }

    #[test]
    fn test_uncovered_path_area_no_text() {
        // A sentence-sized outlined region with no native text at all.
        let paths = vec![pb(50.0, 300.0, 200.0, 12.0)];
        let area = uncovered_path_area(&paths, &[]);
        assert!((area - 2400.0).abs() < 1.0);
        assert!(area >= UNCOVERED_VECTOR_AREA_THRESHOLD);
    }

    #[test]
    fn test_uncovered_path_area_fully_covered_by_text() {
        // Shading rect behind real text: fully covered, must not trigger.
        let paths = vec![pb(50.0, 300.0, 200.0, 12.0)];
        let items = vec![make_item(40.0, 295.0, 250.0, 25.0)];
        let area = uncovered_path_area(&paths, &items);
        assert_eq!(area, 0.0);
    }

    #[test]
    fn test_uncovered_path_area_partial_coverage() {
        // Half the outlined region is covered by a text item.
        let paths = vec![pb(0.0, 0.0, 100.0, 10.0)];
        let items = vec![make_item(0.0, 0.0, 50.0, 10.0)];
        let area = uncovered_path_area(&paths, &items);
        assert!((area - 500.0).abs() < 1.0);
    }

    #[test]
    fn test_uncovered_path_area_small_decoration_below_threshold() {
        // A few bullet-sized filled paths shouldn't reach the threshold.
        let paths = vec![pb(10.0, 10.0, 8.0, 8.0), pb(10.0, 30.0, 8.0, 8.0)];
        let area = uncovered_path_area(&paths, &[]);
        assert!(area < UNCOVERED_VECTOR_AREA_THRESHOLD);
    }

    #[test]
    fn test_unusable_native_unicode_map_error() {
        let mut item = make_item(0.0, 0.0, 10.0, 10.0);
        assert!(!is_unusable_native(&item));
        item.has_unicode_map_error = true;
        assert!(is_unusable_native(&item));
    }

    #[test]
    fn test_clean_ocr_keeps_whitespace_trimmed() {
        assert_eq!(clean_ocr_table_artifacts("   "), "");
        assert_eq!(clean_ocr_table_artifacts(" 123 "), "123");
    }

    // A mock OCR engine that always fails, simulating a systemic error such as
    // missing Tesseract language data (the root cause behind issue #253).
    struct FailingEngine;
    impl OcrEngine for FailingEngine {
        fn name(&self) -> &str {
            "failing"
        }
        fn recognize<'a, 'b: 'a, 'c: 'a>(
            &'a self,
            _image_data: &'c [u8],
            _width: u32,
            _height: u32,
            _options: &'b OcrOptions,
        ) -> std::pin::Pin<
            Box<
                dyn Future<
                        Output = Result<Vec<OcrResult>, Box<dyn std::error::Error + Send + Sync>>,
                    > + Send
                    + '_,
            >,
        > {
            Box::pin(async move { Err("Error opening data file tessdata/eng.traineddata".into()) })
        }
    }

    fn make_blank_page(page_number: usize) -> Page {
        Page {
            page_number,
            page_width: 100.0,
            page_height: 100.0,
            text_items: Vec::new(),
            graphics: Vec::new(),
            vector_graphics: None,
            struct_nodes: Vec::new(),
            image_refs: Vec::new(),
        }
    }

    fn make_rendered(idx: usize) -> RenderedPage {
        RenderedPage {
            idx,
            // 1x1 grayscale pixel; the engine never inspects it.
            pixels: vec![0u8],
            width: 1,
            height: 1,
        }
    }

    // A page that already has substantial native text coverage, as would be the
    // case for a native-text PDF page that was only rendered for OCR because it
    // also contains an image.
    fn make_native_text_page(page_number: usize) -> Page {
        Page {
            page_number,
            page_width: 100.0,
            page_height: 100.0,
            text_items: vec![TextItem {
                text: "this page already has real native text content".into(),
                x: 0.0,
                y: 0.0,
                width: 50.0,
                height: 50.0,
                ..Default::default()
            }],
            graphics: Vec::new(),
            vector_graphics: None,
            struct_nodes: Vec::new(),
            image_refs: Vec::new(),
        }
    }

    // A page with >20 bytes of native text but very low page coverage. These
    // are still text-poor enough that `render_pages_for_ocr` sends them to OCR
    // (`text_coverage < 0.15`), so a systemic OCR failure should not be silently
    // swallowed.
    fn make_low_coverage_text_page(page_number: usize) -> Page {
        Page {
            page_number,
            page_width: 100.0,
            page_height: 100.0,
            text_items: vec![TextItem {
                text: "small native header that is not enough".into(),
                x: 0.0,
                y: 0.0,
                width: 10.0,
                height: 5.0,
                ..Default::default()
            }],
            graphics: Vec::new(),
            vector_graphics: None,
            struct_nodes: Vec::new(),
            image_refs: Vec::new(),
        }
    }

    // When every OCR task fails (e.g. missing language data), the function must
    // return an error instead of silently reporting success with no OCR text.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_all_pages_fail_returns_error() {
        let mut pages = vec![make_blank_page(1), make_blank_page(2)];
        let rendered = vec![make_rendered(0), make_rendered(1)];
        let engine: Arc<dyn OcrEngine> = Arc::new(FailingEngine);

        let result =
            ocr_and_merge_rendered(&mut pages, rendered, 72.0, engine, "eng", 2, true).await;

        let err = result.expect_err("expected systemic OCR failure to be surfaced");
        let msg = err.to_string();
        assert!(
            msg.contains("OCR failed for all 2 page(s)"),
            "unexpected error message: {msg}"
        );
        assert!(
            msg.contains("traineddata"),
            "error should carry the underlying cause: {msg}"
        );
    }

    // With no rendered pages there is nothing to OCR; this must remain a no-op
    // success rather than tripping the all-failed guard.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_no_rendered_pages_is_ok() {
        let mut pages = vec![make_blank_page(1)];
        let engine: Arc<dyn OcrEngine> = Arc::new(FailingEngine);

        let result =
            ocr_and_merge_rendered(&mut pages, Vec::new(), 72.0, engine, "eng", 2, true).await;

        assert!(result.is_ok(), "empty OCR set should succeed: {result:?}");
    }

    // Regression guard: when OCR fails but every failing page already had native
    // text (it was only rendered for image-based enrichment), a broken OCR setup
    // must NOT abort the parse — the native text is still valid output.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_native_text_pages_not_failed_on_ocr_error() {
        let mut pages = vec![make_native_text_page(1), make_native_text_page(2)];
        let rendered = vec![make_rendered(0), make_rendered(1)];
        let engine: Arc<dyn OcrEngine> = Arc::new(FailingEngine);

        let result =
            ocr_and_merge_rendered(&mut pages, rendered, 72.0, engine, "eng", 2, true).await;

        assert!(
            result.is_ok(),
            "OCR failure on already-native-text pages must not abort the parse: {result:?}"
        );
        // Native text is preserved untouched.
        assert_eq!(pages[0].text_items.len(), 1);
        assert_eq!(pages[1].text_items.len(), 1);
    }

    // When failures span both a sparse-text page and a native-text page, the
    // sparse-text page lost its likely primary text source, so we still fail
    // loud.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_mixed_failure_with_sparse_text_page_returns_error() {
        let mut pages = vec![make_native_text_page(1), make_blank_page(2)];
        let rendered = vec![make_rendered(0), make_rendered(1)];
        let engine: Arc<dyn OcrEngine> = Arc::new(FailingEngine);

        let result =
            ocr_and_merge_rendered(&mut pages, rendered, 72.0, engine, "eng", 2, true).await;

        let err = result.expect_err("a text-starved page losing all OCR must surface an error");
        assert!(
            err.to_string().contains("OCR failed for all 2 page(s)"),
            "unexpected error message: {err}"
        );
    }

    // Regression guard for the review finding: low-coverage pages are rendered
    // for OCR even when their native text length is >20 bytes. A systemic OCR
    // failure on such pages must still surface as an error.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_low_coverage_text_page_failure_returns_error() {
        let mut pages = vec![make_low_coverage_text_page(1)];
        let rendered = vec![make_rendered(0)];
        let engine: Arc<dyn OcrEngine> = Arc::new(FailingEngine);

        let result =
            ocr_and_merge_rendered(&mut pages, rendered, 72.0, engine, "eng", 2, true).await;

        let err = result.expect_err("low-coverage text page losing OCR must surface an error");
        assert!(
            err.to_string().contains("OCR failed for all 1 page(s)"),
            "unexpected error message: {err}"
        );
    }

    // With `ocr_failure_fatal = false`, a systemic OCR failure that would
    // normally abort (every task failed, a sparse-text page among them) must
    // instead return Ok and preserve whatever native text was extracted, so a
    // caller with its own fallback gets partial results rather than nothing.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_non_fatal_systemic_failure_returns_partial() {
        let mut pages = vec![make_native_text_page(1), make_blank_page(2)];
        let rendered = vec![make_rendered(0), make_rendered(1)];
        let engine: Arc<dyn OcrEngine> = Arc::new(FailingEngine);

        let result =
            ocr_and_merge_rendered(&mut pages, rendered, 72.0, engine, "eng", 2, false).await;

        assert!(
            result.is_ok(),
            "non-fatal mode must not abort on systemic OCR failure: {result:?}"
        );
        // The native-text page keeps its text; the blank page simply has no OCR.
        assert_eq!(pages[0].text_items.len(), 1);
    }
}
