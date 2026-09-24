use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::error::LiteParseError;
use crate::ocr::{OcrEngine, OcrOptions, OcrResult};
use crate::types::{CutAxis, Page, ParsedPage, Rect, Region, RegionKind, TextItem};
use pdfium::{Document, ImageBounds};
use serde::{Deserialize, Serialize};

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

/// Owned page bitmap prepared for OCR, plus the page facts the merge needs
/// to judge the engine's output. Produced by [`render_pages_for_ocr`],
/// consumed by [`recognize_rasters`]; the non-pixel facts are carried into
/// the [`PageOcrOutcome`] so [`merge_ocr_results`] never needs the raster or
/// the document again.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OcrRaster {
    /// 1-based source page number. Outcomes are matched back to pages by
    /// this number, so a raster can be merged into any page slice that
    /// contains its page.
    pub page_number: usize,
    /// Tightly-packed pixels: 1 byte/px (grayscale) or 3 (RGB). Serialized
    /// as a base64 string, since a JSON array of numbers is ~3.5× the size.
    #[serde(with = "base64_bytes")]
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// DPI this page was actually rendered at (the long-edge cap can lower it
    /// below the requested DPI). Pixel → point scale is `72 / dpi`.
    pub dpi: f32,
    /// The page has real native text (it was not sent to OCR as scanned,
    /// blank, or garbled). OCR is then an enrichment pass over native text,
    /// and the merge applies the stricter artifact filters.
    pub has_native_text: bool,
    /// Embedded raster figure rects (viewport pt, excluding full-page
    /// backgrounds), so the merge can judge each figure's OCR output as a
    /// group — a chart's axis/legend labels are dropped together.
    pub image_rects: Vec<Rect>,
}

/// How [`render_pages_for_ocr`] picks and rasterizes pages.
///
/// `Default` renders every page that needs OCR at
/// [`DEFAULT_DPI`](crate::config::DEFAULT_DPI) in RGB, in one round, with
/// no form rendering or re-flattening.
#[derive(Debug, Clone)]
pub struct OcrRenderOptions {
    /// Stop after this many rasters (`0` = no limit), so a long document can
    /// be processed in bounded rounds; the function returns where to resume.
    pub max_rasters: usize,
    /// Requested render resolution; clamped per page by the long-edge cap.
    pub dpi: f32,
    /// Emit 1 byte/px luma instead of RGB (see `OcrEngine::prefers_grayscale`).
    pub grayscale: bool,
    /// Paint form-field appearances into the raster (runs document actions).
    pub render_form_fields: bool,
    /// Keep going when one page fails to render (it keeps its native text).
    pub continue_on_page_error: bool,
    /// Pages extraction flattened widget annotations on, which must be
    /// flattened again on this (reopened) document so the raster shows what
    /// extraction saw. See `ExtractedPages::flattened_page_numbers`.
    pub reflatten_pages: HashSet<u32>,
    /// Explicit 1-based page selection. `None` lets the page-complexity
    /// scoring decide which pages need OCR (the default parse behaviour);
    /// `Some` renders exactly these pages, whatever the scoring says, so a
    /// caller with its own routing can override it.
    pub selection: Option<HashSet<u32>>,
}

impl Default for OcrRenderOptions {
    fn default() -> Self {
        Self {
            max_rasters: 0,
            dpi: crate::config::DEFAULT_DPI,
            grayscale: false,
            render_form_fields: false,
            continue_on_page_error: false,
            reflatten_pages: HashSet::new(),
            selection: None,
        }
    }
}

/// Base64 (de)serialization for raster pixel buffers.
mod base64_bytes {
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&STANDARD.encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        let encoded = String::deserialize(deserializer)?;
        STANDARD.decode(encoded).map_err(serde::de::Error::custom)
    }
}

/// Why a page was flagged as needing more than the cheap text-only path.
/// Multiple reasons can apply to one page (e.g. a sparse page whose little
/// text is also garbled). Empty exactly when `needs_ocr` is false.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ComplexityReason {
    /// A single raster covers essentially the whole page and there is little or
    /// no extractable text behind it — a scanned/photographed page.
    Scanned,
    /// Almost no extractable native text, and no full-page raster behind it
    /// (a blank page, or a near-empty cover/divider). Check for a companion
    /// `AnnotationText` before treating the page as blank — it means the text
    /// is there, just outside the extractable surface.
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
    /// The page's own content stream has (almost) no text, but an annotation
    /// paints text through its appearance stream. PDFium extracts the content
    /// stream only, so this text is invisible to the text-only path even
    /// though the page renders — and OCR does recover it. Always accompanied
    /// by `NoText` or `Scanned`; it tells a genuinely blank page apart from
    /// one whose body text simply lives out of reach.
    AnnotationText,
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
            ComplexityReason::AnnotationText => "annotation-text",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    if text_length < MIN_NATIVE_TEXT_LEN {
        // Too little text to be the page's content. A full-page raster behind
        // it means a scan; otherwise it's effectively blank.
        reasons.push(if full_page_image {
            ComplexityReason::Scanned
        } else {
            ComplexityReason::NoText
        });
        // ...unless the text is there and simply out of PDFium's reach: text
        // painted by an annotation appearance stream renders (so OCR gets it)
        // but is never tokenized by the text API. Only checked on pages that
        // already look empty, so the annotation walk stays off the hot path.
        if page_obj.has_annotation_text() {
            reasons.push(ComplexityReason::AnnotationText);
        }
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

/// Native-office analogue of [`calculate_page_complexity`] +
/// [`calculate_layout_complexity`] in one: the facts a PDF object walk digs
/// out arrive here as source-document data. `image_rects` are the page's
/// placed media rects (viewport pt), `table_count` the page's table blocks,
/// `column_count` the section-declared column count.
///
/// Deliberate divergences from the PDF pair, all following from "the source
/// states it":
/// - `uncovered_vector_area` is `Some(0.0)` when no cheaper predicate fired —
///   text painted as vector outlines cannot exist on this path (every glyph
///   the layout draws is in the text stream), so the area is a known zero,
///   not an unmeasured `None`.
/// - No `AnnotationText` reason: there are no PDF annotations to paint text.
/// - `ruled_table_coverage` stays 0.0 and `text_table_run_count` 0: blocks
///   carry no geometry, and the ruled/borderless split is a PDF-detector
///   concept — `ruled_table_count` carries the (exact) table count and drives
///   `TableLikely` on its own.
/// - `figure_count`/`figure_coverage` count placed images rather than
///   vector-graphic clusters — the closest native analogue.
pub fn calculate_native_page_complexity(
    page: &Page,
    image_rects: &[crate::types::Rect],
    table_count: usize,
    column_count: usize,
) -> PageComplexityStats {
    let pw = page.page_width;
    let ph = page.page_height;
    let page_area = pw * ph;

    // Same "usable text only" filter as the PDF path. Native text should
    // never trip it, but sharing the predicate keeps the two paths honest.
    let text_length: usize = page
        .text_items
        .iter()
        .filter(|item| !is_unusable_native(item))
        .map(|item| item.text.len())
        .sum();

    // Same filters as the PDF path's `image_bounds(MIN_IMAGE_SIZE_PT, ∞)` +
    // full-page split: tiny decorations dropped, full-page backgrounds
    // excluded from inline-figure coverage but remembered as the scan signal.
    let sized: Vec<&crate::types::Rect> = image_rects
        .iter()
        .filter(|r| r.width >= MIN_IMAGE_SIZE_PT && r.height >= MIN_IMAGE_SIZE_PT)
        .collect();
    let is_full_page = |r: &crate::types::Rect| {
        r.width > pw * MAX_IMAGE_PAGE_COVERAGE && r.height > ph * MAX_IMAGE_PAGE_COVERAGE
    };
    let full_page_image = sized.iter().any(|r| is_full_page(r));
    let counted: Vec<&crate::types::Rect> =
        sized.iter().copied().filter(|r| !is_full_page(r)).collect();
    let has_images = !counted.is_empty();

    let (image_area_sum, largest_image_area) =
        counted.iter().fold((0.0_f32, 0.0_f32), |(sum, max), r| {
            let area = r.width.max(0.0) * r.height.max(0.0);
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

    let sparse_text = text_length < 2000 && text_coverage < 0.15;
    // Native text comes straight from the source XML — a garbled result would
    // be an engine bug, not a font-encoding casualty. Run the shared detector
    // anyway so the two paths can never silently diverge on the predicate.
    let is_garbled = page_is_garbled(page);

    let mut reasons = Vec::new();
    if text_length < MIN_NATIVE_TEXT_LEN {
        reasons.push(if full_page_image {
            ComplexityReason::Scanned
        } else {
            ComplexityReason::NoText
        });
    } else if sparse_text {
        reasons.push(ComplexityReason::SparseText);
    }
    if has_images {
        reasons.push(ComplexityReason::EmbeddedImages);
    }
    if is_garbled {
        reasons.push(ComplexityReason::Garbled);
    }
    let needs_ocr = !reasons.is_empty();
    let uncovered_vector_area = (!needs_ocr).then_some(0.0);

    // Layout half. `figure` coverage from ALL placed images (the PDF layout
    // half counts every figure region, full-page ones included).
    let all_image_area: f32 = image_rects
        .iter()
        .map(|r| r.width.max(0.0) * r.height.max(0.0))
        .sum();
    let figure_coverage = if page_area > 0.0 && all_image_area > 0.0 {
        (all_image_area / page_area).min(1.0)
    } else {
        0.0
    };
    let column_count = column_count.max(1);
    let mut layout_reasons = Vec::new();
    if column_count >= 2 {
        layout_reasons.push(LayoutComplexityReason::MultiColumn);
    }
    if table_count > 0 {
        layout_reasons.push(LayoutComplexityReason::TableLikely);
    }
    if figure_coverage >= DENSE_GRAPHICS_MIN_COVERAGE {
        layout_reasons.push(LayoutComplexityReason::DenseGraphics);
    }
    let layout = LayoutComplexityStats {
        column_count,
        ruled_table_count: table_count,
        ruled_table_coverage: 0.0,
        text_table_run_count: 0,
        figure_count: image_rects.len(),
        figure_coverage,
        is_complex: !layout_reasons.is_empty(),
        reasons: layout_reasons,
    };

    PageComplexityStats {
        page_number: page.page_number,
        text_length,
        text_coverage,
        has_substantial_images: has_images,
        image_block_count: counted.len(),
        image_coverage,
        largest_image_coverage,
        full_page_image,
        uncovered_vector_area,
        is_garbled,
        page_area,
        needs_ocr,
        reasons,
        layout: Some(layout),
    }
}

/// Why a page's layout is expected to be hard to reconstruct faithfully.
/// Orthogonal to `ComplexityReason`: none of these imply OCR — they signal
/// that the text-only path may mangle reading order or structure, so a caller
/// can route the page to a higher-accuracy pipeline. Same leniency contract
/// as `ComplexityReason`: new variants will be added, treat unknowns kindly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// Cap on the rendered long edge (in pixels) for OCR page rasters
/// to avoid excessive memory usage.
pub(crate) const MAX_OCR_RENDER_LONG_EDGE_PX: f32 = 4096.0;

/// Render the pages in `pages[start..]` that need OCR from an already-open
/// document, stopping once `options.max_rasters` of them have been rendered.
/// Returns the rasters plus the index to resume scanning from, so a caller
/// can process a long document in bounded rounds.
///
/// The pdfium `Document` holds raw pointers that are not `Send`, so callers
/// must drop it before awaiting the OCR engine.
pub fn render_pages_for_ocr(
    document: &Document,
    pages: &[Page],
    start: usize,
    options: &OcrRenderOptions,
) -> Result<(Vec<OcrRaster>, usize), LiteParseError> {
    let OcrRenderOptions {
        max_rasters,
        dpi,
        grayscale,
        render_form_fields,
        continue_on_page_error,
        reflatten_pages: flatten_page_numbers,
        selection,
    } = options;
    let (max_rasters, dpi, grayscale, render_form_fields, continue_on_page_error) = (
        *max_rasters,
        *dpi,
        *grayscale,
        *render_form_fields,
        *continue_on_page_error,
    );
    let mut rendered = Vec::new();
    // With `render_form_fields`, draw form-field appearances into the OCR
    // raster so filled-in form values are visible to the OCR engine (matches
    // the LlamaParse extract binary's screenshot rendering). Opt-in because
    // the form environment runs the document's open/JS actions.
    let form = render_form_fields
        .then(|| document.form_environment())
        .flatten();
    if let Some(form) = form.as_ref() {
        form.run_document_actions();
    }
    let mut next_start = pages.len();
    for (idx, page) in pages.iter().enumerate().skip(start) {
        if let Some(selection) = selection
            && !selection.contains(&(page.page_number as u32))
        {
            continue;
        }
        let page_render = (|| -> Result<Option<OcrRaster>, LiteParseError> {
            let page_index = (page.page_number - 1) as i32;
            // Text extraction may have flattened THIS page's widget
            // annotations into page content. When the caller hands us a
            // freshly reopened document, re-apply the flatten on exactly the
            // pages extraction flattened, so the raster shows the same
            // content extraction saw. Never flatten any other page:
            // flattening hides a page's non-widget annotations (stamps,
            // highlights, free text) from the raster, losing their text.
            let page_obj = if flatten_page_numbers.contains(&(page.page_number as u32)) {
                match document.flatten_form_widgets(page_index)? {
                    Some(flattened_page) => flattened_page,
                    None => document.page(page_index)?,
                }
            } else {
                document.page(page_index)?
            };
            // `has_native_text` drives the merge's artifact filters: true
            // when the page was not sent to OCR as scanned, blank, garbled or
            // annotation-only. With an explicit selection the OCR gate is the
            // caller's decision, so the page-object walk in
            // `calculate_page_complexity` is skipped and the signal is derived
            // from the text items alone (which is all those reasons read).
            let has_native_text = match selection {
                Some(_) => {
                    native_text_length(page) >= MIN_NATIVE_TEXT_LEN && !page_is_garbled(page)
                }
                None => {
                    let page_complexity = calculate_page_complexity(page, &page_obj)?;
                    if !page_complexity.needs_ocr {
                        return Ok(None);
                    }
                    !page_complexity.reasons.iter().any(|r| {
                        matches!(
                            r,
                            ComplexityReason::Scanned
                                | ComplexityReason::NoText
                                | ComplexityReason::Garbled
                                | ComplexityReason::AnnotationText
                        )
                    })
                }
            };

            // Clamp the render DPI so the long edge stays within the raster budget.
            let long_edge_pt = page.page_width.max(page.page_height);
            let mut eff_dpi = dpi;
            if long_edge_pt > 0.0 {
                let max_dpi = MAX_OCR_RENDER_LONG_EDGE_PX * 72.0 / long_edge_pt;
                if eff_dpi > max_dpi {
                    eff_dpi = max_dpi;
                }
            }

            let bitmap = page_obj.render_with_form(eff_dpi, form.as_ref())?;
            let width = bitmap.width() as u32;
            let height = bitmap.height() as u32;
            // Grayscale or RGB per the engine; see `OcrEngine::prefers_grayscale`.
            let pixels = if grayscale {
                bitmap.to_luma()
            } else {
                bitmap.to_rgb()
            };

            let image_rects = if has_native_text {
                let pw = page.page_width;
                let ph = page.page_height;
                page_obj
                    .image_bounds(MIN_IMAGE_SIZE_PT, f32::INFINITY)
                    .into_iter()
                    .filter(|b| {
                        !(b.width > pw * MAX_IMAGE_PAGE_COVERAGE
                            && b.height > ph * MAX_IMAGE_PAGE_COVERAGE)
                    })
                    .map(|b| Rect {
                        x: b.x,
                        y: b.y,
                        width: b.width,
                        height: b.height,
                    })
                    .collect()
            } else {
                Vec::new()
            };

            Ok(Some(OcrRaster {
                page_number: page.page_number,
                pixels,
                width,
                height,
                dpi: eff_dpi,
                has_native_text,
                image_rects,
            }))
        })();
        match page_render {
            Ok(Some(render)) => {
                rendered.push(render);
                if max_rasters > 0 && rendered.len() >= max_rasters {
                    next_start = idx + 1;
                    break;
                }
            }
            Ok(None) => {}
            // The page already extracted successfully, so a tolerant parse
            // keeps its native text and only forgoes the OCR enrichment.
            Err(error) if continue_on_page_error => eprintln!(
                "[ocr] render failed for page {}: {} — keeping native text without OCR (continue_on_page_error)",
                page.page_number, error
            ),
            Err(error) => return Err(error),
        }
    }
    Ok((rendered, next_start))
}

/// One page's recognition outcome: what the engine returned for one
/// [`OcrRaster`], plus the raster's page facts so the merge is a pure
/// function of `(pages, outcomes)`. The pixels are gone — this is the small
/// payload that crosses a stage boundary, not the bitmap.
///
/// `error` is set when the engine failed on this page (`results` is then
/// empty); the merge counts failures to tell a systemic engine problem from
/// a flaky page.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageOcrOutcome {
    /// 1-based source page number the raster came from.
    pub page_number: usize,
    /// DPI the raster was rendered at; pixel → point scale is `72 / dpi`.
    pub dpi: f32,
    pub has_native_text: bool,
    pub image_rects: Vec<Rect>,
    /// Engine output in raster pixel coordinates. Empty when the engine
    /// failed (see `error`) or found nothing.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub results: Vec<OcrResult>,
    /// The engine's error message when recognition failed on this page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Run `ocr_engine` over pre-rendered page bitmaps, at most `num_workers`
/// pages at a time, returning one outcome per raster in the order given.
/// Pure recognition: nothing is merged, and a failed page is reported as an
/// `Err` outcome rather than aborting the batch.
pub async fn recognize_rasters(
    rendered: Vec<OcrRaster>,
    ocr_engine: Arc<dyn OcrEngine>,
    ocr_language: &str,
    num_workers: usize,
) -> Vec<PageOcrOutcome> {
    type OcrTaskResult = Result<Vec<OcrResult>, Box<dyn std::error::Error + Send + Sync>>;

    // Browser WASM uses the JavaScript event loop. It has no Tokio runtime or
    // blocking thread pool. Run each JavaScript OCR callback directly so the
    // returned Promise can make progress on the browser event loop.
    #[cfg(target_arch = "wasm32")]
    let task_results: Vec<(usize, f32, (bool, Vec<Rect>), OcrTaskResult)> = {
        let _ = num_workers;
        let mut results = Vec::with_capacity(rendered.len());
        for r in rendered {
            let page_number = r.page_number;
            let page_dpi = r.dpi;
            let native = (r.has_native_text, r.image_rects.clone());
            let options = OcrOptions {
                language: ocr_language.to_string(),
                dpi: page_dpi,
            };
            let result = ocr_engine
                .recognize(&r.pixels, r.width, r.height, &options)
                .await;
            results.push((page_number, page_dpi, native, result));
        }
        results
    };

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
    #[cfg(not(target_arch = "wasm32"))]
    let task_results: Vec<(usize, f32, (bool, Vec<Rect>), OcrTaskResult)> = {
        let num_workers = num_workers.max(1);
        let semaphore = Arc::new(tokio::sync::Semaphore::new(num_workers));
        let mut handles = Vec::with_capacity(rendered.len());

        let handle = tokio::runtime::Handle::current();

        for r in rendered {
            let engine = ocr_engine.clone();
            let sem = semaphore.clone();
            let language = ocr_language.to_string();
            let page_number = r.page_number;
            let rt_handle = handle.clone();

            handles.push((
                page_number,
                r.dpi,
                (r.has_native_text, r.image_rects.clone()),
                tokio::spawn(async move {
                    // Park the task (not an OS thread) until a permit is available.
                    let _permit = sem.acquire_owned().await.expect("semaphore closed");
                    let options = OcrOptions {
                        language,
                        dpi: r.dpi,
                    };
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

        let mut results = Vec::with_capacity(handles.len());
        for (page_number, page_dpi, native, handle) in handles {
            let result = match handle.await {
                Ok(result) => result,
                Err(join_err) => {
                    Err(Box::new(join_err) as Box<dyn std::error::Error + Send + Sync>)
                }
            };
            results.push((page_number, page_dpi, native, result));
        }
        results
    };

    task_results
        .into_iter()
        .map(
            |(page_number, dpi, (has_native_text, image_rects), result)| {
                let (results, error) = match result {
                    Ok(results) => (results, None),
                    Err(e) => (Vec::new(), Some(e.to_string())),
                };
                PageOcrOutcome {
                    page_number,
                    dpi,
                    has_native_text,
                    image_rects,
                    results,
                    error,
                }
            },
        )
        .collect()
}

/// Merge recognition outcomes into `pages`, in place. Pure: needs neither the
/// document nor the rasters, only the outcomes [`recognize_rasters`] produced
/// (or an equivalent a caller built from its own OCR engine's output).
///
/// Per page, unusable native text (garbled fonts, unmappable Type3) is dropped
/// first so OCR can replace it; then each result is filtered against rule
/// artifacts, chart figures and overlap with surviving native text before
/// being appended as an `OCR`-font [`TextItem`] in viewport points.
///
/// Outcomes are matched to pages by `page_number`; an outcome for a page not
/// in `pages` is an error rather than a panic, so a caller can merge a
/// subset of pages or hand-built outcomes safely.
pub fn merge_ocr_results(
    pages: &mut [Page],
    outcomes: Vec<PageOcrOutcome>,
    ocr_failure_fatal: bool,
) -> Result<(), LiteParseError> {
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
    let total_tasks = outcomes.len();
    let mut failed_tasks = 0usize;
    let mut failed_sparse_text_page = false;
    let mut first_error: Option<String> = None;

    let index_by_page_number: HashMap<usize, usize> = pages
        .iter()
        .enumerate()
        .map(|(idx, page)| (page.page_number, idx))
        .collect();

    for outcome in outcomes {
        let PageOcrOutcome {
            page_number,
            dpi: page_dpi,
            has_native_text,
            image_rects,
            results: ocr_results,
            error,
        } = outcome;
        let Some(&idx) = index_by_page_number.get(&page_number) else {
            return Err(LiteParseError::Other(format!(
                "OCR outcome for page {page_number} has no matching page among the {} page(s) being merged",
                pages.len()
            )));
        };
        if let Some(msg) = error {
            failed_tasks += 1;
            failed_sparse_text_page |= page_has_sparse_native_text(&pages[idx]);
            if first_error.is_none() {
                eprintln!("[ocr] failed for page {}: {}", page_number, msg);
                first_error = Some(msg);
            }
            continue;
        }

        if ocr_results.is_empty() {
            continue;
        }

        // Pixel -> PDF-point scale for the DPI this page was actually rendered
        // at (the long-edge cap can lower it below the configured DPI).
        let scale_factor = 72.0 / page_dpi;

        let page = &mut pages[idx];
        // Drop unusable native items (substitution-cipher cmap corruption, or
        // unmappable Type3 text) so OCR can replace them. Without this,
        // garbled-but-spatially-present native text suppresses every OCR
        // result that overlaps it via the overlap check below, leaving the
        // output stuck with unreadable text. We apply per-item, per-font, and
        // per-page checks: short garbled labels ("GDWH", "XVG") can't be
        // flagged alone, but their font group or host page can.
        match garbled_scope(page) {
            GarbledScope::None => page.text_items.retain(|item| !is_unusable_native(item)),
            GarbledScope::WholePage => page.text_items.clear(),
            // Only the corrupt font's items go; text in the page's healthy
            // fonts is still the best source for its part of the page, and
            // keeping it lets the overlap check below suppress redundant OCR
            // there while OCR fills in what was dropped.
            GarbledScope::Fonts(fonts) => page
                .text_items
                .retain(|item| !fonts.contains(&item.font_name) && !is_unusable_native(item)),
        }

        // Only check overlap against native (already-extracted) PDF text. Comparing
        // each OCR result against previously-accepted OCR results caused adjacent
        // OCR lines whose bounding boxes touched within tolerance to suppress each
        // other, dropping every second line on scanned pages.
        let native_count = page.text_items.len();
        let debug_ocr = std::env::var_os("LITEPARSE_DEBUG_OCR").is_some();
        // Judge each embedded figure's OCR output as a group: a figure whose
        // recognised text is mostly numbers and short tokens is a chart
        // (axes, ticks, legend keys), and none of it belongs in the text flow.
        let chart_images: Vec<bool> = if has_native_text {
            chart_like_images(&image_rects, &ocr_results, scale_factor)
        } else {
            Vec::new()
        };
        if debug_ocr {
            eprintln!(
                "[ocr-merge] page {} native_items={} ocr_results={} dpi={}",
                page_number,
                native_count,
                ocr_results.len(),
                page_dpi
            );
        }
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

            // On pages that already have native text, OCR only enriches:
            // a bar-shaped single glyph is a table border / rule misread
            // (Tesseract reads vertical rules as `|`/`I`), and a result with
            // no letters or digits is decoration. Both would otherwise be
            // glued onto the adjacent native line by the projection's
            // item-merge pass, corrupting cell text.
            if has_native_text && is_rule_artifact(&r.text, ocr_w, ocr_h) {
                if debug_ocr {
                    eprintln!(
                        "[ocr-merge]   skip(rule-artifact) {:?} w={:.1} h={:.1}",
                        r.text, ocr_w, ocr_h
                    );
                }
                continue;
            }
            if has_native_text
                && chart_images
                    .iter()
                    .zip(&image_rects)
                    .any(|(is_chart, rect)| {
                        *is_chart && box_center_in(rect, ocr_x, ocr_y, ocr_w, ocr_h)
                    })
            {
                if debug_ocr {
                    eprintln!("[ocr-merge]   skip(chart-figure) {:?}", r.text);
                }
                continue;
            }
            let overlaps = overlaps_existing_text(
                &page.text_items[..native_count],
                ocr_x,
                ocr_y,
                ocr_w,
                ocr_h,
                2.0,
            );
            if debug_ocr {
                eprintln!(
                    "[ocr-merge]   {} conf={:.2} x={:.1} y={:.1} w={:.1} h={:.1} {:?}",
                    if overlaps { "skip(overlap)" } else { "ADD" },
                    r.confidence,
                    ocr_x,
                    ocr_y,
                    ocr_w,
                    ocr_h,
                    r.text
                );
            }
            if overlaps {
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
    let text_length = native_text_length(page);
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

    text_length < MIN_NATIVE_TEXT_LEN || text_coverage < 0.15
}

/// Below this many bytes of usable native text a page counts as having no
/// text of its own (`NoText` / `Scanned`), so OCR is its primary source.
const MIN_NATIVE_TEXT_LEN: usize = 20;

/// Bytes of native text on the page, excluding items OCR would replace
/// anyway (see `is_unusable_native`).
fn native_text_length(page: &Page) -> usize {
    page.text_items
        .iter()
        .filter(|item| !is_unusable_native(item))
        .map(|item| item.text.len())
        .sum()
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

/// Minimum ASCII letters before a group of text is worth judging for garbling.
/// Below this the vowel ratio is too noisy — an acronym run or a short label
/// can sit near zero without being corrupt.
const GARBLE_MIN_LETTERS: usize = 30;

/// A font group must hold at least this share of the page's letters before its
/// own vowel ratio can flag the page. Corruption travels with a font, so a
/// broken map affects a substantial block of the page; requiring a real share
/// keeps a stray caption or ticker run in an odd font from flagging a healthy
/// page.
const GARBLE_MIN_FONT_SHARE: f32 = 0.15;

/// True when a body of text's vowel ratio is below the natural-language floor.
/// Real Latin-script vowel ratios sit ~30–45% across English, Portuguese,
/// Spanish, French, etc. A ratio under 20% is well outside any natural-language
/// range and signals substitution-style corruption. (A simple +3 Caesar shift
/// still leaves some U/Y letters from the original O/Y mapping, so a 10% bound
/// is too tight to catch this in practice.)
fn ratio_is_garbled(letters: usize, vowels: usize) -> bool {
    letters >= GARBLE_MIN_LETTERS && vowels * 5 < letters
}

/// Which of a page's native text items are untrustworthy because they decode
/// to garbage. Corruption is a property of a *font's* ToUnicode map, not of the
/// page, so this is resolved per font group as well as page-wide.
pub(crate) enum GarbledScope {
    /// Nothing looks garbled.
    None,
    /// The page's text as a whole decodes to garbage — every native item is
    /// suspect, including items too short to judge on their own.
    WholePage,
    /// Specific font groups are garbled while the rest of the page extracts
    /// fine. Holds the `font_name` of each flagged group.
    Fonts(HashSet<Option<String>>),
}

/// Page-level garbled check: even when individual items are too short to judge
/// in isolation (e.g. "GDWH", "FXUUHQFB XVG"), text whose aggregate vowel ratio
/// collapses is almost certainly substitution-encoded. Used to drop untrusted
/// native items before OCR merge, so short garbled labels don't suppress
/// overlapping OCR results.
///
/// Judged per font group, not only page-wide: a page mixing a healthy standard
/// font with an embedded font whose ToUnicode map is wrong averages out to a
/// natural-looking ratio, so a page-wide sum alone reports the page as clean
/// while half its text is corrupt. The page-wide check is still applied first,
/// since it catches pages whose fonts are individually too small to judge (and
/// pages whose items carry no font name at all).
fn garbled_scope(page: &Page) -> GarbledScope {
    let mut total_letters = 0usize;
    let mut total_vowels = 0usize;
    // Keyed by font name, so text sharing a ToUnicode map is judged together.
    let mut per_font: HashMap<Option<&str>, (usize, usize)> = HashMap::new();
    for it in &page.text_items {
        let (l, v) = count_letters_and_vowels(&it.text);
        total_letters += l;
        total_vowels += v;
        let entry = per_font.entry(it.font_name.as_deref()).or_insert((0, 0));
        entry.0 += l;
        entry.1 += v;
    }

    if ratio_is_garbled(total_letters, total_vowels) {
        return GarbledScope::WholePage;
    }

    let min_share = (total_letters as f32 * GARBLE_MIN_FONT_SHARE).ceil() as usize;
    let fonts: HashSet<Option<String>> = per_font
        .into_iter()
        .filter(|&(_, (letters, vowels))| letters >= min_share && ratio_is_garbled(letters, vowels))
        .map(|(font, _)| font.map(str::to_string))
        .collect();

    if fonts.is_empty() {
        GarbledScope::None
    } else {
        GarbledScope::Fonts(fonts)
    }
}

fn page_is_garbled(page: &Page) -> bool {
    !matches!(garbled_scope(page), GarbledScope::None)
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
/// Maximum width/height ratio for a single-glyph OCR result to count as a
/// misread rule: real narrow glyphs ("1", "l", "I") in a tight OCR box sit
/// around 0.3–0.5 of their height; a table border is a hairline.
const RULE_ARTIFACT_MAX_ASPECT: f32 = 0.35;

/// An OCR result that is a misread of a rule/border or pure decoration, not
/// text: no letters or digits at all, or a single bar-like glyph in a box far
/// narrower than it is tall.
fn is_rule_artifact(text: &str, w: f32, h: f32) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() || !trimmed.chars().any(|c| c.is_alphanumeric()) {
        return true;
    }
    let mut chars = trimmed.chars();
    let (first, rest) = (chars.next(), chars.next());
    if rest.is_none() {
        if let Some(c) = first {
            if matches!(
                c,
                '|' | 'I'
                    | 'l'
                    | '1'
                    | '!'
                    | 'i'
                    | 'j'
                    | '['
                    | ']'
                    | '('
                    | ')'
                    | '{'
                    | '}'
                    | '/'
                    | '\\'
            ) && h > 0.0
                && w < RULE_ARTIFACT_MAX_ASPECT * h
            {
                return true;
            }
        }
    }
    false
}

/// Minimum OCR results inside a figure before it is judged as a chart.
const CHART_MIN_RESULTS: usize = 4;
/// Fraction of a figure's OCR results that must be numeric or very short
/// tokens for the figure to count as a chart.
const CHART_LABEL_FRACTION: f32 = 0.5;

fn box_center_in(rect: &Rect, x: f32, y: f32, w: f32, h: f32) -> bool {
    let cx = x + w / 2.0;
    let cy = y + h / 2.0;
    cx >= rect.x && cx <= rect.x + rect.width && cy >= rect.y && cy <= rect.y + rect.height
}

/// A chart label: a number (with separators/units) or a token of ≤ 3 chars.
fn is_chart_label(text: &str) -> bool {
    let t = text.trim();
    if t.chars().count() <= 3 {
        return true;
    }
    t.chars().any(|c| c.is_ascii_digit())
        && t.chars().all(|c| {
            c.is_ascii_digit()
                || matches!(
                    c,
                    ',' | '.' | '%' | '-' | '+' | '$' | '€' | '£' | ' ' | 'k' | 'K' | 'M' | 'x'
                )
        })
}

/// For each figure rect, whether its OCR results look like a chart's
/// axes/ticks/legend rather than text worth keeping.
fn chart_like_images(rects: &[Rect], results: &[OcrResult], scale_factor: f32) -> Vec<bool> {
    rects
        .iter()
        .map(|rect| {
            let (mut n, mut labels) = (0usize, 0usize);
            for r in results {
                let (x, y, w, h) = (
                    r.bbox[0] * scale_factor,
                    r.bbox[1] * scale_factor,
                    (r.bbox[2] - r.bbox[0]) * scale_factor,
                    (r.bbox[3] - r.bbox[1]) * scale_factor,
                );
                if box_center_in(rect, x, y, w, h) {
                    n += 1;
                    if is_chart_label(&r.text) {
                        labels += 1;
                    }
                }
            }
            n >= CHART_MIN_RESULTS && labels as f32 >= CHART_LABEL_FRACTION * n as f32
        })
        .collect()
}

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
        // Parentheses directly around a number mark it negative, e.g. "(78,939)",
        // so keep that pair; only what lies outside it is a border misread.
        if let Some((before, after)) = trimmed.split_once(without_artifacts)
            && before.ends_with('(')
            && after.starts_with(')')
        {
            return format!("({without_artifacts})");
        }
        without_artifacts.to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Rect;

    #[test]
    fn rule_artifact_catches_bars_and_decoration() {
        // Tesseract reading a table border / hairline.
        assert!(is_rule_artifact("|", 4.3, 12.5));
        assert!(is_rule_artifact("I", 1.0, 7.2));
        assert!(is_rule_artifact("|  |", 3.0, 12.0));
        assert!(is_rule_artifact("—", 20.0, 3.0));
        assert!(is_rule_artifact("  ", 1.0, 1.0));
        // Real narrow glyphs sit well above the hairline aspect.
        assert!(!is_rule_artifact("1", 5.0, 12.0));
        assert!(!is_rule_artifact("I", 4.0, 10.0));
        // Anything with letters/digits and more than one char is text.
        assert!(!is_rule_artifact("If", 1.0, 12.0));
        assert!(!is_rule_artifact("N/A", 12.0, 8.0));
    }

    #[test]
    fn chart_labels_and_chart_like_figures() {
        assert!(is_chart_label("140,000"));
        assert!(is_chart_label("2016"));
        assert!(is_chart_label("$1.5M"));
        assert!(is_chart_label("0"));
        assert!(is_chart_label("Yr"));
        assert!(!is_chart_label("Year"));
        assert!(!is_chart_label("Fruit Production in British Columbia"));

        let rect = Rect {
            x: 50.0,
            y: 100.0,
            width: 200.0,
            height: 150.0,
        };
        let mk = |text: &str, x: f32, y: f32| OcrResult {
            text: text.to_string(),
            bbox: [x, y, x + 20.0, y + 8.0],
            confidence: 0.9,
            polygon: None,
        };
        // Axis ticks + one title: mostly labels -> chart.
        let chart = vec![
            mk("100", 60.0, 110.0),
            mk("80", 60.0, 130.0),
            mk("60", 60.0, 150.0),
            mk("2016", 100.0, 240.0),
            mk("Fruit Production", 120.0, 105.0),
        ];
        assert_eq!(chart_like_images(&[rect.clone()], &chart, 1.0), vec![true]);
        // An infographic with sentences: not a chart.
        let prose = vec![
            mk("Wash your hands", 60.0, 110.0),
            mk("for twenty seconds", 60.0, 130.0),
            mk("before every meal", 60.0, 150.0),
            mk("2020", 100.0, 240.0),
        ];
        assert_eq!(chart_like_images(&[rect.clone()], &prose, 1.0), vec![false]);
        // Too few results to judge.
        assert_eq!(chart_like_images(&[rect], &chart[..3], 1.0), vec![false]);
    }

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

    mod native {
        use super::super::*;
        use crate::types::Rect;

        fn item(text: &str, x: f32, y: f32, w: f32, h: f32) -> TextItem {
            TextItem {
                text: text.to_string(),
                x,
                y,
                width: w,
                height: h,
                ..TextItem::default()
            }
        }

        fn page(items: Vec<TextItem>) -> Page {
            Page {
                page_number: 1,
                page_label: None,
                page_width: 612.0,
                page_height: 792.0,
                content_bounds: None,
                text_items: items,
                graphics: Vec::new(),
                vector_graphics: None,
                struct_nodes: Vec::new(),
                image_refs: Vec::new(),
                annotations: None,
                form_fields: None,
                structure_tree: None,
            }
        }

        fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
            Rect {
                x,
                y,
                width: w,
                height: h,
            }
        }

        #[test]
        fn text_dense_page_is_simple_with_known_zero_vector_area() {
            // 40 wide items -> text_length >= 20 and coverage >= 15%.
            let items: Vec<TextItem> = (0..40)
                .map(|i| item("some meaningful text", 40.0, 20.0 * i as f32, 500.0, 18.0))
                .collect();
            let stats = calculate_native_page_complexity(&page(items), &[], 0, 1);
            assert!(!stats.needs_ocr);
            assert!(stats.reasons.is_empty());
            // Known zero, not unmeasured: vector text cannot exist natively.
            assert_eq!(stats.uncovered_vector_area, Some(0.0));
            let layout = stats.layout.expect("native stats always carry layout");
            assert_eq!(layout.column_count, 1);
            assert!(!layout.is_complex);
        }

        #[test]
        fn tables_columns_and_images_drive_layout_reasons() {
            let items = vec![item(
                "cell text and more cell text",
                40.0,
                40.0,
                500.0,
                700.0,
            )];
            // One large inline figure (not full-page).
            let figures = [rect(100.0, 100.0, 400.0, 300.0)];
            let stats = calculate_native_page_complexity(&page(items), &figures, 2, 2);
            assert!(
                stats.reasons.contains(&ComplexityReason::EmbeddedImages),
                "sized inline image counts"
            );
            let layout = stats.layout.unwrap();
            assert_eq!(layout.ruled_table_count, 2);
            assert_eq!(layout.column_count, 2);
            assert!(
                layout
                    .reasons
                    .contains(&LayoutComplexityReason::TableLikely)
            );
            assert!(
                layout
                    .reasons
                    .contains(&LayoutComplexityReason::MultiColumn)
            );
            assert!(
                layout
                    .reasons
                    .contains(&LayoutComplexityReason::DenseGraphics),
                "400x300 over 612x792 is ~25% coverage"
            );
            assert!(layout.is_complex);
        }

        #[test]
        fn full_page_image_with_no_text_reads_as_scanned() {
            let figures = [rect(0.0, 0.0, 612.0, 792.0)];
            let stats = calculate_native_page_complexity(&page(Vec::new()), &figures, 0, 1);
            assert!(stats.reasons.contains(&ComplexityReason::Scanned));
            assert!(stats.full_page_image);
            assert_eq!(
                stats.image_block_count, 0,
                "full-page background is not an inline figure"
            );
            // The cheap predicate fired, so the (zero) vector walk is skipped,
            // mirroring the PDF flow's None.
            assert_eq!(stats.uncovered_vector_area, None);
        }

        #[test]
        fn tiny_decorations_are_ignored() {
            let items: Vec<TextItem> = (0..40)
                .map(|i| item("some meaningful text", 40.0, 20.0 * i as f32, 500.0, 18.0))
                .collect();
            // A 10pt icon: below MIN_IMAGE_SIZE_PT on both axes.
            let figures = [rect(40.0, 40.0, 10.0, 10.0)];
            let stats = calculate_native_page_complexity(&page(items), &figures, 0, 1);
            assert!(!stats.has_substantial_images);
            assert!(!stats.needs_ocr);
        }
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

    #[test]
    fn test_clean_ocr_keeps_accounting_negatives() {
        // Parentheses around a number mark it negative (issue #468).
        assert_eq!(clean_ocr_table_artifacts("(78,939)"), "(78,939)");
        assert_eq!(clean_ocr_table_artifacts("(12.5%)"), "(12.5%)");
        // Border misreads next to them are still removed.
        assert_eq!(clean_ocr_table_artifacts("(78,939)|"), "(78,939)");
        assert_eq!(clean_ocr_table_artifacts("|(78,939)]"), "(78,939)");
        // A lone parenthesis is still treated as a border misread.
        assert_eq!(clean_ocr_table_artifacts("78,939)"), "78,939");
        assert_eq!(clean_ocr_table_artifacts("(78,939"), "78,939");
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
            page_label: None,
            page_width: 100.0,
            page_height: 100.0,
            content_bounds: None,
            text_items: Vec::new(),
            graphics: Vec::new(),
            vector_graphics: None,
            struct_nodes: Vec::new(),
            image_refs: Vec::new(),
            annotations: None,
            form_fields: None,
            structure_tree: None,
        }
    }

    /// A page whose items carry per-item `(font_name, text)` pairs. Geometry is
    /// irrelevant to the garble checks, so every item shares one bbox.
    fn make_font_page(items: &[(&str, &str)]) -> Page {
        let mut page = make_blank_page(1);
        page.text_items = items
            .iter()
            .map(|(font, text)| TextItem {
                text: (*text).into(),
                font_name: Some((*font).to_string()),
                width: 10.0,
                height: 10.0,
                ..Default::default()
            })
            .collect();
        page
    }

    /// Caesar-shifted table text: the +11 letter / +3 digit map from the
    /// mixed-font fixture in issue #413, applied to the visible row text.
    const GARBLED_ROWS: &[&str] = &[
        "Tepx Nzop Opdnctaetzy Bflyetej Cpgtph Olep",
        "KI-334 Cpawlnpxpye Qtwepc Lddpxmwj 44 5359-30-34",
        "KI-335 Azcelmwp Dpydzc Wzfdtyr 45 5359-30-35",
        "KI-336 Nlwtmcletzy Mclnvpe 46 5359-30-36",
    ];

    const HEALTHY_PARAGRAPH: &[&str] = &[
        "This fictional report is a public parser-routing fixture.",
        "The upper section uses a standard font and extracts normally.",
        "All names, identifiers, quantities, and dates are fabricated.",
        "A PDF viewer still renders both sections as readable text.",
    ];

    #[test]
    fn test_garbled_scope_clean_page() {
        let items: Vec<(&str, &str)> = HEALTHY_PARAGRAPH
            .iter()
            .map(|t| ("Helvetica", *t))
            .collect();
        let page = make_font_page(&items);
        assert!(matches!(garbled_scope(&page), GarbledScope::None));
        assert!(!page_is_garbled(&page));
    }

    #[test]
    fn test_garbled_scope_whole_page() {
        let items: Vec<(&str, &str)> = GARBLED_ROWS.iter().map(|t| ("BadFont", *t)).collect();
        let page = make_font_page(&items);
        assert!(matches!(garbled_scope(&page), GarbledScope::WholePage));
    }

    /// Issue #413: healthy text in one font must not mask a corrupt ToUnicode
    /// map in another. Page-wide the vowel ratio lands ~0.28 (natural-looking),
    /// while the corrupt font group alone sits ~0.19.
    #[test]
    fn test_garbled_scope_mixed_fonts_flags_only_bad_font() {
        let mut items: Vec<(&str, &str)> = HEALTHY_PARAGRAPH
            .iter()
            .map(|t| ("Helvetica", *t))
            .collect();
        items.extend(GARBLED_ROWS.iter().map(|t| ("BitstreamVeraSans-Roman", *t)));
        let page = make_font_page(&items);

        // The page-wide sum alone does not trip: this is the regression.
        let (letters, vowels) = page.text_items.iter().fold((0, 0), |(l, v), it| {
            let (il, iv) = count_letters_and_vowels(&it.text);
            (l + il, v + iv)
        });
        assert!(!ratio_is_garbled(letters, vowels));

        match garbled_scope(&page) {
            GarbledScope::Fonts(fonts) => {
                assert_eq!(fonts.len(), 1);
                assert!(fonts.contains(&Some("BitstreamVeraSans-Roman".to_string())));
            }
            _ => panic!("expected only the corrupt font group to be flagged"),
        }
        assert!(page_is_garbled(&page));
    }

    /// A small run of low-vowel text (acronyms, tickers, part numbers) in its
    /// own font must not flag an otherwise healthy page.
    #[test]
    fn test_garbled_scope_ignores_small_low_vowel_font() {
        let mut items: Vec<(&str, &str)> = HEALTHY_PARAGRAPH
            .iter()
            .map(|t| ("Helvetica", *t))
            .collect();
        items.push(("TickerFont", "NYSE NASDAQ FTSE DAX SPX"));
        let page = make_font_page(&items);
        assert!(matches!(garbled_scope(&page), GarbledScope::None));
    }

    /// Items with no font name still fall back to the page-wide verdict.
    #[test]
    fn test_garbled_scope_without_font_names() {
        let mut page = make_blank_page(1);
        page.text_items = GARBLED_ROWS
            .iter()
            .map(|text| TextItem {
                text: (*text).into(),
                font_name: None,
                ..Default::default()
            })
            .collect();
        assert!(matches!(garbled_scope(&page), GarbledScope::WholePage));
    }

    fn make_rendered(idx: usize) -> OcrRaster {
        OcrRaster {
            has_native_text: false,
            image_rects: Vec::new(),
            page_number: idx + 1,
            // 1x1 grayscale pixel; the engine never inspects it.
            pixels: vec![0u8],
            width: 1,
            height: 1,
            dpi: 72.0,
        }
    }

    /// The pre-split entry point, kept for these tests: recognition followed
    /// by the merge, exactly as `parse()` composes the two stages.
    async fn ocr_and_merge_rendered(
        pages: &mut [Page],
        rendered: Vec<OcrRaster>,
        engine: Arc<dyn OcrEngine>,
        language: &str,
        num_workers: usize,
        ocr_failure_fatal: bool,
    ) -> Result<(), LiteParseError> {
        let outcomes = recognize_rasters(rendered, engine, language, num_workers).await;
        merge_ocr_results(pages, outcomes, ocr_failure_fatal)
    }

    // A page that already has substantial native text coverage, as would be the
    // case for a native-text PDF page that was only rendered for OCR because it
    // also contains an image.
    fn make_native_text_page(page_number: usize) -> Page {
        Page {
            page_number,
            page_label: None,
            page_width: 100.0,
            page_height: 100.0,
            content_bounds: None,
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
            annotations: None,
            form_fields: None,
            structure_tree: None,
        }
    }

    // A page with >20 bytes of native text but very low page coverage. These
    // are still text-poor enough that `render_pages_for_ocr` sends them to OCR
    // (`text_coverage < 0.15`), so a systemic OCR failure should not be silently
    // swallowed.
    fn make_low_coverage_text_page(page_number: usize) -> Page {
        Page {
            page_number,
            page_label: None,
            page_width: 100.0,
            page_height: 100.0,
            content_bounds: None,
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
            annotations: None,
            form_fields: None,
            structure_tree: None,
        }
    }

    // When every OCR task fails (e.g. missing language data), the function must
    // return an error instead of silently reporting success with no OCR text.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_all_pages_fail_returns_error() {
        let mut pages = vec![make_blank_page(1), make_blank_page(2)];
        let rendered = vec![make_rendered(0), make_rendered(1)];
        let engine: Arc<dyn OcrEngine> = Arc::new(FailingEngine);

        let result = ocr_and_merge_rendered(&mut pages, rendered, engine, "eng", 2, true).await;

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

        let result = ocr_and_merge_rendered(&mut pages, Vec::new(), engine, "eng", 2, true).await;

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

        let result = ocr_and_merge_rendered(&mut pages, rendered, engine, "eng", 2, true).await;

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

        let result = ocr_and_merge_rendered(&mut pages, rendered, engine, "eng", 2, true).await;

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

        let result = ocr_and_merge_rendered(&mut pages, rendered, engine, "eng", 2, true).await;

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

        let result = ocr_and_merge_rendered(&mut pages, rendered, engine, "eng", 2, false).await;

        assert!(
            result.is_ok(),
            "non-fatal mode must not abort on systemic OCR failure: {result:?}"
        );
        // The native-text page keeps its text; the blank page simply has no OCR.
        assert_eq!(pages[0].text_items.len(), 1);
    }
}
