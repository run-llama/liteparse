use serde::Serialize;
use std::collections::HashMap;

#[doc(hidden)]
#[derive(Debug, Clone)]
pub enum PdfInput {
    /// Path to a PDF file on disk.
    Path(String),
    /// Raw PDF bytes (e.g. from a network response or in-memory buffer).
    Bytes(Vec<u8>),
}

/// Represents a single text item extracted from a PDF page,
/// including its content, position, size, rotation, and font metadata.
#[derive(Debug, Clone, Default, Serialize)]
pub struct TextItem {
    pub text: String,
    /// Viewport-space coordinates (top-left origin, 72 DPI).
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    /// Rotation in degrees (counter-clockwise, adjusted for page rotation).
    pub rotation: f32,
    pub font_name: Option<String>,
    pub font_size: Option<f32>,
    /// Font size * scale_y from the text matrix — accounts for CTM scaling.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_height: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_ascent: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_descent: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_weight: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_flags: Option<i32>,
    /// Sum of glyph widths (using charcode-based lookup when possible).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_width: Option<f32>,
    /// Whether the font has buggy encoding (private-use codepoints, TT subset, etc.)
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub font_is_buggy: bool,
    /// Whether most characters in this item could not be mapped to Unicode
    /// (e.g. a Type3 font with no ToUnicode map). The text content is
    /// PDFium's char-code fallback and does not reflect the rendered glyphs.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub has_unicode_map_error: bool,
    /// Marked content ID from the PDF structure tree.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mcid: Option<i32>,
    /// Fill color as ARGB hex string (e.g. "ff000000").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fill_color: Option<String>,
    /// Stroke color as ARGB hex string.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stroke_color: Option<String>,
    /// OCR confidence score (0.0–1.0). None for native PDF text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
    /// Target URI when this item falls inside a hyperlink annotation's
    /// rectangle. Populated in `extract.rs`; consumed by the markdown emitter.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    /// Whether a thin horizontal stroke/rect crosses this item's vertical middle
    /// band (a strikethrough line). Populated in `extract.rs`; consumed by the
    /// markdown emitter to wrap the text in `~~…~~`.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub strike: bool,
    /// Per-word sub-boxes within this item, split on the inter-word spaces seen
    /// during segment building. A segment groups several words together (it only
    /// breaks at line/column boundaries), so this exposes the finer word-level
    /// geometry needed for bbox attribution. Empty for items that produced no
    /// word split (e.g. OCR-sourced or single-token items). Internal/attribution
    /// use only — `#[serde(skip)]` keeps it out of the JSON output but it is
    /// marshalled across the napi boundary.
    #[serde(skip)]
    pub words: Vec<WordBox>,
}

/// One word's bounding box within a `TextItem`, in the same viewport space
/// (top-left origin, 72 DPI) as the parent item. `text` is the word's content
/// with inter-word spaces excluded.
#[derive(Debug, Clone, Default, Serialize)]
pub struct WordBox {
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[doc(hidden)]
#[derive(Debug, Serialize)]
pub struct Page {
    pub page_number: usize,
    pub page_width: f32,
    pub page_height: f32,
    pub text_items: Vec<TextItem>,
    /// Vector graphics on the page, distilled from PDFium path objects.
    /// Not emitted in JSON/text outputs — consumed by the markdown layout pass.
    #[serde(skip)]
    pub graphics: Vec<GraphicPrimitive>,
    /// Structure-tree nodes for this page when the PDF is tagged. Each node
    /// carries its role, marked-content ids, and the union bbox of its tagged
    /// content. Empty for untagged PDFs.
    #[serde(skip)]
    pub struct_nodes: Vec<StructNode>,
    /// Raster image objects detected on the page. Empty when the page has no
    /// images. Threaded through to `ParsedPage.image_refs`.
    #[serde(skip)]
    pub image_refs: Vec<ImageRef>,
}

/// One entry in the document outline (bookmarks). Coordinates are in PDF
/// user space (origin bottom-left) — convert to viewport with
/// `page_height - y` once you know the page.
#[doc(hidden)]
#[derive(Debug, Clone)]
pub struct OutlineTarget {
    /// Hierarchy depth, 1-based.
    pub level: u8,
    pub title: String,
    /// Zero-based page index of the destination.
    pub page_index: i32,
    /// Y in PDF user space, top of the target location. `None` when the
    /// destination doesn't specify a Y.
    pub y_pdf: Option<f32>,
}

/// One node from the structure tree of a page. Pre-flattened in pre-order
/// (parent before children).
#[doc(hidden)]
#[derive(Debug, Clone)]
pub struct StructNode {
    pub role: String,
    pub mcids: Vec<i32>,
    /// Union bbox of page objects tagged with `mcids`, in viewport coords.
    /// `None` when none of the mcids resolved to a bbox.
    pub bbox: Option<Rect>,
    pub alt_text: Option<String>,
}

/// Represents a fully parsed page with projected text layout.
#[derive(Debug, Serialize)]
pub struct ParsedPage {
    pub page_number: usize,
    pub page_width: f32,
    pub page_height: f32,
    pub text: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub markdown: String,
    pub text_items: Vec<TextItem>,
    /// Per-line structural metadata used by the markdown emitter. Not part of
    /// the JSON/text outputs (consumed internally) so it is `#[serde(skip)]`.
    #[serde(skip)]
    pub projected_lines: Vec<ProjectedLine>,
    /// Root of the XY-cut region tree for this page. Leaves correspond to the
    /// `region_path` on each `ProjectedLine`. Internal-only.
    #[serde(skip)]
    pub regions: Region,
    /// Vector graphics on the page (decomposed paths) used by the markdown
    /// emitter for ruled-table / HR / figure-cluster detection. Not part of
    /// the JSON/text output.
    #[serde(skip)]
    pub graphics: Vec<GraphicPrimitive>,
    /// Figure-region bounding rectangles derived from `graphics`. Pre-computed
    /// in `to_parsed_pages` so the XY-cut layout pass can treat them as
    /// obstacles, and reused downstream for figure classification.
    #[serde(skip)]
    pub figures: Vec<Rect>,
    /// Structure-tree nodes for this page (tagged PDFs only). Pre-flattened in
    /// pre-order. Consumed by the markdown classifier for highest-priority
    /// heading / figure / table detection.
    #[serde(skip)]
    pub struct_nodes: Vec<StructNode>,
    /// Raster image objects on the page. Bbox in viewport coords. Populated
    /// during extraction; consumed by the markdown emitter to interleave
    /// `Block::Figure` references at the right y position. Empty when the
    /// page has no embedded images. Not part of JSON/text output.
    #[serde(skip)]
    pub image_refs: Vec<ImageRef>,
}

/// One embedded raster image on a page. `id` is a stable, page-scoped slug
/// used as the markdown link target (e.g. `image_p1_0.png`). `obj_index` is
/// the image's position among image page-objects, so a later embed pass can
/// re-open the document and pull pixel bytes with `render_image_object`.
#[doc(hidden)]
#[derive(Debug, Clone)]
pub struct ImageRef {
    pub id: String,
    pub bbox: Rect,
    pub obj_index: usize,
    pub format: String,
    pub pixel_width: u32,
    pub pixel_height: u32,
    pub rotation: f32,
    pub jpeg_bytes: Option<Vec<u8>>,
    pub raw_bytes: Option<Vec<u8>>,
    pub bits_per_pixel: u32,
    pub colorspace: i32,
}

/// A raster image extracted from a page along with its pixel bytes. Surfaced
/// on `ParseResult.images` only when `ImageMode::Embed` is configured —
/// otherwise the extraction step skips the render and only `ImageRef`s are
/// produced. Other images are encoded as PNG from PDFium's rendered bitmap.
/// JPEG streams are preserved without re-encoding when PDFium
/// exposes a valid directly decoded DCT stream.
#[derive(Debug, Clone, Serialize)]
pub struct ExtractedImage {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub page: u32,
    pub bbox: Rect,
    pub width: u32,
    pub height: u32,
    pub rotation: f32,
    pub format: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duplicate_of: Option<String>,
    #[serde(skip)]
    pub bytes: Vec<u8>,
}

#[doc(hidden)]
#[derive(Debug, Clone, Default, Serialize)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Lightweight vector-graphic primitive derived from PDFium path objects.
/// Only the shapes useful to the markdown emitter (ruled tables, HRs, figure
/// clusters) are kept — bezier curves and complex paths are decomposed into
/// straight strokes, or dropped.
#[doc(hidden)]
#[derive(Debug, Clone)]
pub enum GraphicPrimitive {
    /// A single straight line segment in viewport coords. Used for HR/table
    /// border detection.
    Stroke {
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        color: Option<String>,
        width: f32,
    },
    /// An axis-aligned rectangle — typically a filled cell background, banner,
    /// or fully-stroked table border drawn as a single path.
    Rect {
        bbox: Rect,
        fill: Option<String>,
        stroke: Option<String>,
    },
}

impl GraphicPrimitive {
    /// Bbox of the primitive in viewport coords.
    pub fn bbox(&self) -> Rect {
        match self {
            GraphicPrimitive::Stroke { x1, y1, x2, y2, .. } => {
                let x = x1.min(*x2);
                let y = y1.min(*y2);
                Rect {
                    x,
                    y,
                    width: (x2 - x1).abs(),
                    height: (y2 - y1).abs(),
                }
            }
            GraphicPrimitive::Rect { bbox, .. } => bbox.clone(),
        }
    }
}

/// Per-line structural metadata derived during grid projection. Used by the
/// markdown emitter; not surfaced in JSON/text output.
#[doc(hidden)]
#[derive(Debug, Clone, Serialize)]
pub struct ProjectedLine {
    pub text: String,
    pub bbox: Rect,
    pub anchor: Anchor,
    pub indent_x: f32,
    pub dominant_font_size: f32,
    /// True when `dominant_font_size` was derived from bbox height (PDFium
    /// reported the font size baked into the text matrix, ~1.0) rather than a
    /// real font-size value. Height-derived sizes jitter ±1pt line-to-line
    /// based on glyph content (descenders, parens, capitals), so heading
    /// detection must use a wider margin over body for these lines.
    pub font_size_is_estimated: bool,
    /// Precise matrix-derived size (`Tf_size × text_matrix_scale`) for
    /// matrix-baked-size lines, when a glyph exposed a text matrix. Used
    /// *only* by heading detection (body-size + heading map), where the
    /// jitter-free value beats the bbox-height estimate in `dominant_font_size`.
    /// Deliberately NOT consumed by table/paragraph grouping, which stay on
    /// `dominant_font_size` to avoid perturbing well-tuned line grouping.
    pub heading_font_size: Option<f32>,
    pub dominant_font_name: Option<String>,
    pub all_bold: bool,
    pub all_italic: bool,
    pub all_mono: bool,
    pub all_strike: bool,
    pub spans: Vec<TextItem>,
    /// Path from the page's region-tree root to the leaf containing this line.
    /// Equality means "same leaf"; prefix relationship means "one contains the
    /// other". Replaces the prior flat `column_id` scheme so nested layouts
    /// (banded splits with sub-columns) survive paragraph/table grouping.
    pub region_path: Vec<u16>,
    pub mcid: Option<i32>,
    /// True when the line's original-coordinate bbox falls inside a detected
    /// figure/chart region. Chart text (axis labels, legends, category titles)
    /// is often set in a font larger than real headings; if it reached the
    /// heading-size histogram it would hijack the top heading levels and get
    /// promoted itself. Heading detection skips these lines.
    pub in_figure: bool,
}

/// XY-cut region tree node. A page's root region recursively splits along H or
/// V axes until each leaf holds a coherent block of items.
#[doc(hidden)]
#[derive(Debug, Clone, Default)]
pub struct Region {
    pub bbox: Rect,
    pub kind: RegionKind,
}

#[doc(hidden)]
#[derive(Debug, Clone)]
pub enum RegionKind {
    Leaf {
        item_indices: Vec<usize>,
    },
    Split {
        axis: CutAxis,
        children: Vec<Region>,
    },
}

impl Default for RegionKind {
    fn default() -> Self {
        RegionKind::Leaf {
            item_indices: Vec::new(),
        }
    }
}

#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CutAxis {
    Horizontal,
    Vertical,
}

#[doc(hidden)]
#[derive(Debug, Serialize)]
pub enum Snap {
    Left,
    Right,
    Center,
}

#[doc(hidden)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum Anchor {
    Left,
    Right,
    Center,
    /// Inline span that does not snap to a column edge — used by lines whose
    /// dominant items couldn't be classified as Left/Right/Center.
    Floating,
}

#[doc(hidden)]
#[derive(Debug, Serialize)]
pub struct ProjectedTextItem {
    pub item: TextItem,
    pub snap: Snap,
    pub anchor: Anchor,
    pub is_dup: bool,
    pub rendered: bool,
    pub num_spaces: usize,
    pub force_unsnapped: bool,
    pub is_margin_line_number: bool,
    pub rotated: bool,
    pub d: f32,
    pub orig_x: f32,
    pub orig_y: f32,
    pub orig_width: f32,
    pub orig_height: f32,
    pub orig_rotation: f32,
}

#[doc(hidden)]
pub type AnchorMap = HashMap<i32, Vec<(usize, usize)>>;

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_item() -> TextItem {
        TextItem {
            text: "hi".into(),
            x: 1.0,
            y: 2.0,
            width: 10.0,
            height: 4.0,
            font_name: Some("Arial".into()),
            font_size: Some(12.0),
            ..Default::default()
        }
    }

    #[test]
    fn text_item_skips_none_fields() {
        let item = sample_item();
        let s = serde_json::to_string(&item).unwrap();
        assert!(!s.contains("font_height"));
        assert!(!s.contains("confidence"));
        assert!(!s.contains("font_is_buggy"));
        assert!(s.contains("\"text\":\"hi\""));
    }

    #[test]
    fn text_item_includes_buggy_flag_when_true() {
        let mut item = sample_item();
        item.font_is_buggy = true;
        let s = serde_json::to_string(&item).unwrap();
        assert!(s.contains("font_is_buggy"));
    }

    #[test]
    fn page_serializes() {
        let p = Page {
            page_number: 1,
            page_width: 100.0,
            page_height: 200.0,
            text_items: vec![sample_item()],
            graphics: vec![],
            struct_nodes: vec![],
            image_refs: vec![],
        };
        let s = serde_json::to_string(&p).unwrap();
        assert!(s.contains("\"page_number\":1"));
    }

    #[test]
    fn anchor_map_basic() {
        let mut m: AnchorMap = HashMap::new();
        m.entry(5).or_default().push((1, 2));
        assert_eq!(m.get(&5).unwrap()[0], (1, 2));
    }
}
