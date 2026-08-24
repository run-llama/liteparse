use serde::Serialize;
use std::collections::{BTreeMap, HashMap};

#[doc(hidden)]
#[derive(Debug, Clone)]
pub enum PdfInput {
    /// Path to a PDF file on disk.
    Path(String),
    /// Raw PDF bytes (e.g. from a network response or in-memory buffer).
    Bytes(Vec<u8>),
}

/// Document-level provenance metadata extracted from PDFium plus a bounded
/// streaming scan of the source PDF. Fields stay optional so malformed
/// metadata never prevents the document itself from being parsed.
#[derive(Debug, Clone, Default, Serialize)]
pub struct DocumentMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub creation_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mod_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_version: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_encrypted: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub security_handler_revision: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permissions: Option<u64>,
    /// Literal `%%EOF` markers in the file. A rough incremental-update signal:
    /// embedded PDF attachments and content-stream text inflate it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eof_section_count: Option<u32>,
    /// Literal `startxref` markers in the file, with the same caveat.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub startxref_count: Option<u32>,
    /// Whether the two halves of the last trailer `/ID` array differ, which
    /// usually means the file was updated after creation. `None` when no
    /// hex-string `/ID` pair was found in the last 1 MiB.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trailer_id_pair_differs: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_file_size: Option<u64>,
    /// The document catalog's `/Metadata` XMP packet, capped at 64 KiB.
    /// `None` when the document has none, when it is too large to resolve
    /// cheaply (see `extract_document_metadata`), or in WASM builds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub xmp: Option<String>,
    /// True when the catalog's XMP stream exceeded the 64 KiB cap and `xmp`
    /// holds only its first 64 KiB.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub xmp_truncated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature_byte_range_reaches_eof: Option<bool>,
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
    /// Raw character codes from the PDF content stream. These correspond to
    /// source glyphs rather than Unicode scalar values, so ligature expansion
    /// can produce more text characters than entries in this array.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub char_codes: Vec<u32>,
    /// Whether the trailing source space was synthesized by PDFium rather than
    /// represented by a real space glyph in the PDF content stream.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub trailing_space_generated: bool,
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

/// The `TextItem` fields governed by `extract_text_metadata`, pre-gated by
/// [`TextItem::text_metadata`]. This struct defines which fields count as
/// rich text metadata: every output surface (CLI JSON, napi, python, wasm)
/// builds its public items from it, so a new metadata field added here is a
/// compile error in each surface until it is wired through.
///
/// All fields are `Option`/slice so "extraction disabled" is representable
/// even for the plain-`bool` fields on `TextItem`: `None`/empty means the
/// caller did not opt in and the field must be omitted from output.
pub struct TextMetadata<'a> {
    pub font_height: Option<f32>,
    pub font_ascent: Option<f32>,
    pub font_descent: Option<f32>,
    pub font_weight: Option<i32>,
    pub text_width: Option<f32>,
    pub font_is_buggy: Option<bool>,
    pub mcid: Option<i32>,
    pub fill_color: Option<&'a str>,
    pub stroke_color: Option<&'a str>,
    pub char_codes: Option<&'a [u32]>,
    pub trailing_space_generated: Option<bool>,
}

impl TextItem {
    /// The rich-metadata view of this item: real values when `enabled`, all
    /// absent otherwise. See [`TextMetadata`].
    pub fn text_metadata(&self, enabled: bool) -> TextMetadata<'_> {
        if enabled {
            TextMetadata {
                font_height: self.font_height,
                font_ascent: self.font_ascent,
                font_descent: self.font_descent,
                font_weight: self.font_weight,
                text_width: self.text_width,
                font_is_buggy: Some(self.font_is_buggy),
                mcid: self.mcid,
                fill_color: self.fill_color.as_deref(),
                stroke_color: self.stroke_color.as_deref(),
                char_codes: Some(&self.char_codes),
                trailing_space_generated: Some(self.trailing_space_generated),
            }
        } else {
            TextMetadata {
                font_height: None,
                font_ascent: None,
                font_descent: None,
                font_weight: None,
                text_width: None,
                font_is_buggy: None,
                mcid: None,
                fill_color: None,
                stroke_color: None,
                char_codes: None,
                trailing_space_generated: None,
            }
        }
    }
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
    /// Union bbox of the page's top-level content objects in viewport
    /// coords (visible content extent). `None` for empty pages.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_bounds: Option<Rect>,
    pub text_items: Vec<TextItem>,
    /// Vector graphics on the page, distilled from PDFium path objects.
    /// Not emitted in JSON/text outputs — consumed by the markdown layout pass.
    #[serde(skip)]
    pub graphics: Vec<GraphicPrimitive>,
    /// Lossless-enough, PDFium-compatible path output requested by the caller.
    /// Kept separate from the lossy internal `graphics` layout primitives.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vector_graphics: Option<VectorGraphics>,
    /// Structure-tree nodes for this page when the PDF is tagged. Each node
    /// carries its role, marked-content ids, and the union bbox of its tagged
    /// content. Empty for untagged PDFs.
    #[serde(skip)]
    pub struct_nodes: Vec<StructNode>,
    /// Raster image objects detected on the page. Empty when the page has no
    /// images. Threaded through to `ParsedPage.image_refs`.
    #[serde(skip)]
    pub image_refs: Vec<ImageRef>,
    /// Public annotation data when explicitly requested. `None` distinguishes
    /// disabled extraction from an enabled page with no annotations.
    #[serde(skip)]
    pub annotations: Option<Vec<DocumentAnnotation>>,
    /// AcroForm widgets when explicitly requested.
    #[serde(skip)]
    pub form_fields: Option<Vec<FormField>>,
    /// Tagged-PDF logical structure when explicitly requested.
    #[serde(skip)]
    pub structure_tree: Option<StructureTree>,
}

/// One PDF page annotation. Coordinates use the same top-left, 72-DPI
/// viewport space as [`TextItem`].
#[derive(Debug, Clone, Serialize)]
pub struct DocumentAnnotation {
    pub subtype: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contents: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modified: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rect: Option<Rect>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub quadpoint_rects: Vec<Rect>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
}

/// Scalar value from a tagged-PDF structure element's `/A` dictionary.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(untagged)]
pub enum StructureAttributeValue {
    Boolean(bool),
    Number(f32),
    String(String),
}

/// A complete page-scoped tagged-PDF logical structure tree.
#[derive(Debug, Clone, Serialize)]
pub struct StructureTree {
    pub roots: Vec<StructureTreeElement>,
}

/// One tagged-PDF structure element. Field names follow the
/// repository's snake_case JSON convention rather than PDFium's C spellings.
#[derive(Debug, Clone, Serialize)]
pub struct StructureTreeElement {
    #[serde(rename = "type")]
    pub element_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alt_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub attributes: BTreeMap<String, StructureAttributeValue>,
    pub marked_content_ids: Vec<i32>,
    pub children: Vec<StructureTreeElement>,
    pub annotations: Vec<DocumentAnnotation>,
}

/// One AcroForm widget and its resolved field metadata.
#[derive(Debug, Clone, Serialize)]
pub struct FormField {
    pub id: String,
    #[serde(rename = "type")]
    pub field_type: String,
    pub page: u32,
    pub annotation_index: i32,
    pub widget_index: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object_number: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alternate_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub export_value: Option<String>,
    pub field_flags: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub control_count: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub control_index: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checked: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rect: Option<Rect>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub selected_options: Vec<String>,
}

/// One raw packet from an XFA form document's `/XFA` array. Surfaced on
/// `ParseResult.xfa_packets` when `extract_xfa_packets` is enabled.
#[derive(Debug, Clone, Serialize)]
pub struct XfaPacket {
    /// Zero-based index in the XFA array.
    pub index: u32,
    /// Packet name (e.g. `template`, `datasets`), when present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Decoded content length in bytes.
    pub content_length: u32,
    /// Packet content (usually XML), lossily decoded as UTF-8. `None` when
    /// the packet stream could not be read.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
}

/// One solid rectangle (or thick line) detected in a rendered page bitmap.
/// Coordinates are in the same top-left, 72-DPI viewport space as text
/// items. Detection runs on the raster, so it also covers scanned/flattened
/// pages that carry no vector paths.
#[derive(Debug, Clone, Serialize)]
pub struct ScreenshotRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    /// Fill color as ARGB hex string (e.g. "ff1a2b3c").
    pub color: String,
    /// True when only one dimension reaches the minimum rectangle size
    /// (a solid line rather than a filled area).
    pub is_line: bool,
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
    /// Union bbox of the page's top-level content objects in viewport
    /// coords (visible content extent). `None` for empty pages.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_bounds: Option<Rect>,
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
    /// Public vector path extraction. Absent unless explicitly enabled.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vector_graphics: Option<VectorGraphics>,
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
    /// Per-page complexity signals (the same the `is_complex` API returns).
    /// Populated only when `LiteParseConfig::include_complexity` is set;
    /// `None` otherwise. Surfaced as a per-page `complexity` object in JSON.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub complexity: Option<crate::ocr_merge::PageComplexityStats>,
    /// Page annotations when `LiteParseConfig::extract_annotations` is true.
    /// `None` means extraction was disabled; `Some([])` means enabled with no
    /// annotations on this page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotations: Option<Vec<DocumentAnnotation>>,
    /// AcroForm widgets when `extract_form_fields` is true.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub form_fields: Option<Vec<FormField>>,
    /// Tagged-PDF logical structure when `extract_structure_tree` is true.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub structure_tree: Option<StructureTree>,
}

/// One embedded raster image on a page. `id` is a stable, page-scoped slug
/// used as the markdown link target (e.g. `img_p1_1.png`). `obj_index` is
/// the image's depth-first position among image placements (including Form
/// XObjects), so a later embed pass can re-open the document and pull pixel
/// bytes with `render_image_object`.
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
/// on `ParseResult.images` only when `extract_images` is enabled; otherwise
/// the extraction step skips the render and only `ImageRef`s are
/// produced. JPEG streams are preserved without re-encoding when PDFium
/// exposes a valid directly decoded DCT stream; other images are encoded as
/// PNG from PDFium's rendered bitmap.
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
    /// Encoded image bytes. Shared (`Arc`) so duplicate entries reference the
    /// canonical image's buffer instead of copying it per occurrence.
    #[serde(skip)]
    pub bytes: std::sync::Arc<Vec<u8>>,
}

#[doc(hidden)]
#[derive(Debug, Clone, Default, Serialize)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Page-scoped vector path output. Coordinates use the same top-left,
/// 72-DPI viewport space as text items.
#[derive(Debug, Clone, Default, Serialize)]
pub struct VectorGraphics {
    pub shapes: Vec<VectorShape>,
    pub lines: Vec<VectorLine>,
}

/// One PDF path object's paint state and viewport bounding box.
#[derive(Debug, Clone, Serialize)]
pub struct VectorShape {
    pub bbox: Rect,
    pub stroke: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stroke_color: Option<String>,
    pub fill: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fill_color: Option<String>,
    pub has_curve: bool,
}

/// A strict horizontal or vertical path segment after adjacent compatible
/// segments have been merged, matching LlamaParse PDFium path semantics.
#[derive(Debug, Clone, Serialize)]
pub struct VectorLine {
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
    pub stroke: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stroke_width: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stroke_color: Option<String>,
    pub fill: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fill_color: Option<String>,
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
    /// True when the line's base direction is right-to-left (strong RTL
    /// characters outnumber strong LTR ones). `spans` is always in x-ascending
    /// order; consumers that rebuild the line's *text* must walk it in reverse
    /// when this is set, or labels detach from their values.
    pub rtl: bool,
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
            content_bounds: None,
            text_items: vec![sample_item()],
            graphics: vec![],
            vector_graphics: None,
            struct_nodes: vec![],
            image_refs: vec![],
            annotations: None,
            form_fields: None,
            structure_tree: None,
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
