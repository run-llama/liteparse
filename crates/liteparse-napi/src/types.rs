use std::collections::HashMap;

use napi_derive::napi;

use liteparse::config::{CropBox, ImageMode, LiteParseConfig, OutputFormat};
use liteparse::layout::{LayoutBlock, LayoutCell};
use liteparse::parser::ParseResult;
use liteparse::types::{
    DocumentAnnotation, FormField, GraphicPrimitive, Page, ParsedPage, Rect, ScreenshotRect,
    StructureAttributeValue, StructureTree, StructureTreeElement, TextItem, VectorGraphics,
    WordBox, XfaPacket,
};

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

#[napi(object)]
#[derive(Clone)]
pub struct JsLiteParseConfig {
    /// OCR language code (e.g., "eng", "fra").
    pub ocr_language: Option<String>,
    /// Whether OCR is enabled.
    pub ocr_enabled: Option<bool>,
    /// HTTP OCR server URL. If set, uses HTTP OCR instead of Tesseract.
    pub ocr_server_url: Option<String>,
    /// Extra HTTP headers sent with every request to `ocrServerUrl`
    /// (e.g. `{ Authorization: "Bearer <token>" }`).
    pub ocr_server_headers: Option<HashMap<String, String>>,
    /// Path to tessdata directory for Tesseract.
    pub tessdata_path: Option<String>,
    /// Maximum number of pages to parse.
    pub max_pages: Option<u32>,
    /// Specific pages to parse (e.g., "1-5,10,15-20").
    pub target_pages: Option<String>,
    /// Render parsed pages to PNG and return them in `ParseResult.screenshots`.
    /// Default false; PNG payloads can be large.
    pub extract_screenshots: Option<bool>,
    /// Continue after page-level extraction failures and return them in
    /// `ParseResult.pageErrors`. Default false.
    pub continue_on_page_error: Option<bool>,
    /// DPI for rendering pages (used for OCR and screenshots).
    pub dpi: Option<f64>,
    /// Output format: "json", "text", or "markdown".
    pub output_format: Option<String>,
    /// Keep very small text that would normally be filtered out.
    pub preserve_very_small_text: Option<bool>,
    /// Password for encrypted/protected documents.
    pub password: Option<String>,
    /// Suppress progress output.
    pub quiet: Option<bool>,
    /// Number of concurrent OCR workers (default: CPU cores - 1).
    pub num_workers: Option<u32>,
    /// How to surface raster images in markdown output: "off", "placeholder"
    /// (default; emits `![](img_pN_K.png)` references with no bytes), or
    /// "embed" (same presentation as placeholder; extraction is independent).
    pub image_mode: Option<String>,
    /// Extract embedded image bytes and metadata (default false).
    pub extract_images: Option<bool>,
    /// Directory where embedded image files are written. Requires
    /// `extractImages` to be true.
    pub image_output_dir: Option<String>,
    /// Render hyperlink annotations as `[text](url)` in markdown output
    /// (default true). Set false for plain anchor text.
    pub extract_links: Option<bool>,
    /// Keep running headers/footers in markdown output instead of stripping
    /// repeated page-band lines and page chrome (default false).
    pub keep_headers_footers: Option<bool>,
    /// Extract all PDF annotations as page-scoped structured data.
    pub extract_annotations: Option<bool>,
    /// Emit each page's classified layout blocks (headings, paragraphs, list
    /// items, tables with per-cell boxes, code, rules, figures) with bounding
    /// boxes as `ParsedPage.blocks`. Default false.
    pub extract_blocks: Option<bool>,
    /// Extract AcroForm widget fields and values.
    pub extract_form_fields: Option<bool>,
    /// Extract the tagged-PDF logical structure tree.
    pub extract_structure_tree: Option<bool>,
    /// Extract raw XFA packets (name + XML content) into
    /// `ParseResult.xfaPackets`. Default false.
    pub extract_xfa_packets: Option<bool>,
    /// Collect document provenance metadata into `ParseResult.docMeta`.
    /// Default false: it streams the whole source file once. Absent for
    /// inputs converted from a non-PDF format.
    pub extract_document_metadata: Option<bool>,
    /// Emit each page's `contentBounds` (union bbox of top-level content
    /// objects, viewport coords). Default false.
    pub extract_content_bounds: Option<bool>,
    /// Detect solid rectangles/lines in rendered page screenshots and attach
    /// them to each screenshot result. Default false.
    pub detect_screenshot_rects: Option<bool>,
    /// Draw AcroForm field appearances into rendered rasters (screenshots and
    /// OCR inputs). Runs the document's open/JS actions. Default false.
    pub render_form_fields: Option<bool>,
    /// Whether a systemic OCR failure aborts the whole parse (default true).
    /// Set false to keep already-recovered native text and return partial
    /// results when OCR is unavailable, instead of rejecting.
    pub ocr_failure_fatal: Option<bool>,
    /// OCR request-hedging schedule (ms). Empty/unset = no hedging. Multiple
    /// delays (e.g. `[0, 5000, 10000]`) fire duplicate requests per attempt and
    /// take the first success — lower tail latency at the cost of extra load.
    pub ocr_hedge_delays_ms: Option<Vec<u32>>,
    /// Emit per-word sub-boxes on each text item (`TextItem.words`). Default
    /// false. Word boxes roughly double the text-item payload, so enable only
    /// for word-level bbox attribution. With `extractBlocks` on, word geometry
    /// is always computed internally as a table-detection input, but `words`
    /// is only returned when this is set.
    pub emit_word_boxes: Option<bool>,
    /// Include rich PDF text metadata on returned text items. Default false.
    pub extract_text_metadata: Option<bool>,
    /// Restrict output to a page sub-region. Each field is the fraction of the
    /// page cropped from that side; a text item survives only if it lies
    /// entirely inside the remaining rectangle. Unset keeps the whole page.
    pub crop_box: Option<JsCropBox>,
    /// Drop diagonal text (rotation >2° off the nearest right angle). Default
    /// false. Use to exclude rotated watermarks/stamps from the output.
    pub skip_diagonal_text: Option<bool>,
    /// Compute per-page complexity signals during parse and attach them to each
    /// page as `ParsedPage.complexity` (the same signals `isComplex` returns).
    /// Default false; enabling it runs an extra vector-text detection pass.
    pub include_complexity: Option<bool>,
    /// Expose page-scoped vector path extraction. Default false.
    pub extract_vector_graphics: Option<bool>,
}

/// A page sub-region as the fraction cropped from each side (top-left origin,
/// each in `[0, 1]`).
#[napi(object)]
#[derive(Clone)]
pub struct JsCropBox {
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
    pub left: f64,
}

impl JsLiteParseConfig {
    pub fn into_rust(self) -> LiteParseConfig {
        let mut cfg = LiteParseConfig::default();
        if let Some(v) = self.ocr_language {
            cfg.ocr_language = v;
        }
        if let Some(v) = self.ocr_enabled {
            cfg.ocr_enabled = v;
        }
        if let Some(v) = self.ocr_server_url {
            cfg.ocr_server_url = Some(v);
        }
        if let Some(v) = self.ocr_server_headers {
            cfg.ocr_server_headers = v.into_iter().collect();
        }
        if let Some(v) = self.tessdata_path {
            cfg.tessdata_path = Some(v);
        }
        if let Some(v) = self.max_pages {
            cfg.max_pages = v as usize;
        }
        if let Some(v) = self.target_pages {
            cfg.target_pages = Some(v);
        }
        if let Some(v) = self.extract_screenshots {
            cfg.extract_screenshots = v;
        }
        if let Some(v) = self.continue_on_page_error {
            cfg.continue_on_page_error = v;
        }
        if let Some(v) = self.dpi {
            cfg.dpi = v as f32;
        }
        if let Some(v) = self.output_format {
            cfg.output_format = match v.as_str() {
                "text" => OutputFormat::Text,
                "markdown" | "md" => OutputFormat::Markdown,
                _ => OutputFormat::Json,
            };
        }
        if let Some(v) = self.preserve_very_small_text {
            cfg.preserve_very_small_text = v;
        }
        if let Some(v) = self.password {
            cfg.password = Some(v);
        }
        if let Some(v) = self.quiet {
            cfg.quiet = v;
        }
        if let Some(v) = self.num_workers {
            cfg.num_workers = v as usize;
        }
        if let Some(v) = self.image_mode {
            cfg.image_mode = match v.as_str() {
                "off" | "none" => ImageMode::Off,
                "embed" => ImageMode::Embed,
                _ => ImageMode::Placeholder,
            };
        }
        if let Some(v) = self.extract_images {
            cfg.extract_images = v;
        }
        if let Some(v) = self.image_output_dir {
            cfg.image_output_dir = Some(v);
        }
        if let Some(v) = self.extract_links {
            cfg.extract_links = v;
        }
        if let Some(v) = self.keep_headers_footers {
            cfg.keep_headers_footers = v;
        }
        if let Some(v) = self.extract_annotations {
            cfg.extract_annotations = v;
        }
        if let Some(v) = self.extract_blocks {
            cfg.extract_blocks = v;
        }
        if let Some(v) = self.extract_form_fields {
            cfg.extract_form_fields = v;
        }
        if let Some(v) = self.extract_structure_tree {
            cfg.extract_structure_tree = v;
        }
        if let Some(v) = self.extract_xfa_packets {
            cfg.extract_xfa_packets = v;
        }
        if let Some(v) = self.extract_document_metadata {
            cfg.extract_document_metadata = v;
        }
        if let Some(v) = self.extract_content_bounds {
            cfg.extract_content_bounds = v;
        }
        if let Some(v) = self.detect_screenshot_rects {
            cfg.detect_screenshot_rects = v;
        }
        if let Some(v) = self.render_form_fields {
            cfg.render_form_fields = v;
        }
        if let Some(v) = self.ocr_failure_fatal {
            cfg.ocr_failure_fatal = v;
        }
        if let Some(v) = self.ocr_hedge_delays_ms {
            cfg.ocr_hedge_delays_ms = v.into_iter().map(u64::from).collect();
        }
        if let Some(v) = self.emit_word_boxes {
            cfg.emit_word_boxes = v;
        }
        if let Some(v) = self.extract_text_metadata {
            cfg.extract_text_metadata = v;
        }
        if let Some(v) = self.crop_box {
            cfg.crop_box = Some(CropBox {
                top: v.top as f32,
                right: v.right as f32,
                bottom: v.bottom as f32,
                left: v.left as f32,
            });
        }
        if let Some(v) = self.skip_diagonal_text {
            cfg.skip_diagonal_text = v;
        }
        if let Some(v) = self.include_complexity {
            cfg.include_complexity = v;
        }
        if let Some(v) = self.extract_vector_graphics {
            cfg.extract_vector_graphics = v;
        }
        cfg
    }

    pub fn from_rust(cfg: &LiteParseConfig) -> Self {
        Self {
            ocr_language: Some(cfg.ocr_language.clone()),
            ocr_enabled: Some(cfg.ocr_enabled),
            ocr_server_url: cfg.ocr_server_url.clone(),
            ocr_server_headers: if cfg.ocr_server_headers.is_empty() {
                None
            } else {
                Some(cfg.ocr_server_headers.iter().cloned().collect())
            },
            tessdata_path: cfg.tessdata_path.clone(),
            max_pages: Some(cfg.max_pages as u32),
            target_pages: cfg.target_pages.clone(),
            extract_screenshots: Some(cfg.extract_screenshots),
            continue_on_page_error: Some(cfg.continue_on_page_error),
            dpi: Some(cfg.dpi as f64),
            output_format: Some(match cfg.output_format {
                OutputFormat::Json => "json".to_string(),
                OutputFormat::Text => "text".to_string(),
                OutputFormat::Markdown => "markdown".to_string(),
            }),
            preserve_very_small_text: Some(cfg.preserve_very_small_text),
            password: cfg.password.clone(),
            quiet: Some(cfg.quiet),
            num_workers: Some(cfg.num_workers as u32),
            image_mode: Some(match cfg.image_mode {
                ImageMode::Off => "off".to_string(),
                ImageMode::Placeholder => "placeholder".to_string(),
                ImageMode::Embed => "embed".to_string(),
            }),
            extract_images: Some(cfg.extract_images),
            image_output_dir: cfg.image_output_dir.clone(),
            extract_links: Some(cfg.extract_links),
            keep_headers_footers: Some(cfg.keep_headers_footers),
            extract_annotations: Some(cfg.extract_annotations),
            extract_blocks: Some(cfg.extract_blocks),
            extract_form_fields: Some(cfg.extract_form_fields),
            extract_structure_tree: Some(cfg.extract_structure_tree),
            extract_xfa_packets: Some(cfg.extract_xfa_packets),
            extract_document_metadata: Some(cfg.extract_document_metadata),
            extract_content_bounds: Some(cfg.extract_content_bounds),
            detect_screenshot_rects: Some(cfg.detect_screenshot_rects),
            render_form_fields: Some(cfg.render_form_fields),
            ocr_failure_fatal: Some(cfg.ocr_failure_fatal),
            ocr_hedge_delays_ms: Some(
                cfg.ocr_hedge_delays_ms
                    .iter()
                    .map(|&v| u32::try_from(v).unwrap_or(u32::MAX))
                    .collect(),
            ),
            emit_word_boxes: Some(cfg.emit_word_boxes),
            extract_text_metadata: Some(cfg.extract_text_metadata),
            crop_box: cfg.crop_box.map(|c| JsCropBox {
                top: c.top as f64,
                right: c.right as f64,
                bottom: c.bottom as f64,
                left: c.left as f64,
            }),
            skip_diagonal_text: Some(cfg.skip_diagonal_text),
            include_complexity: Some(cfg.include_complexity),
            extract_vector_graphics: Some(cfg.extract_vector_graphics),
        }
    }
}

// ---------------------------------------------------------------------------
// TextItem
// ---------------------------------------------------------------------------

/// One word's sub-box within a `JsTextItem`, in the same viewport space.
#[napi(object)]
#[derive(Clone)]
pub struct JsWordBox {
    pub text: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl JsWordBox {
    pub fn from_rust(word: &WordBox) -> Self {
        Self {
            text: word.text.clone(),
            x: word.x as f64,
            y: word.y as f64,
            width: word.width as f64,
            height: word.height as f64,
        }
    }
}

#[napi(object)]
#[derive(Clone)]
pub struct JsTextItem {
    pub text: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub font_name: Option<String>,
    pub font_size: Option<f64>,
    pub font_height: Option<f64>,
    pub font_ascent: Option<f64>,
    pub font_descent: Option<f64>,
    pub font_weight: Option<i32>,
    pub text_width: Option<f64>,
    pub font_is_buggy: Option<bool>,
    pub mcid: Option<i32>,
    /// Fill color as an eight-character ARGB hex string.
    pub fill_color: Option<String>,
    /// Stroke color as an eight-character ARGB hex string.
    pub stroke_color: Option<String>,
    /// Raw PDF content-stream character codes for the source glyphs.
    pub char_codes: Option<Vec<u32>>,
    /// True when the trailing source space was synthesized by PDFium.
    pub trailing_space_generated: Option<bool>,
    /// OCR confidence score (0.0-1.0). Undefined for native PDF text.
    pub confidence: Option<f64>,
    /// Rotation in degrees (viewport space). Defaults to 0 when omitted.
    pub rotation: Option<f64>,
    /// Per-word sub-boxes for attribution. Empty for items with no word split
    /// (e.g. OCR-sourced or single-token items).
    pub words: Vec<JsWordBox>,
}

impl JsTextItem {
    pub fn to_rust(&self) -> TextItem {
        TextItem {
            text: self.text.clone(),
            x: self.x as f32,
            y: self.y as f32,
            width: self.width as f32,
            height: self.height as f32,
            rotation: self.rotation.unwrap_or(0.0) as f32,
            font_name: self.font_name.clone(),
            font_size: self.font_size.map(|v| v as f32),
            font_height: self.font_height.map(|v| v as f32),
            font_ascent: self.font_ascent.map(|v| v as f32),
            font_descent: self.font_descent.map(|v| v as f32),
            font_weight: self.font_weight,
            text_width: self.text_width.map(|v| v as f32),
            font_is_buggy: self.font_is_buggy.unwrap_or(false),
            mcid: self.mcid,
            fill_color: self.fill_color.clone(),
            stroke_color: self.stroke_color.clone(),
            char_codes: self.char_codes.clone().unwrap_or_default(),
            trailing_space_generated: self.trailing_space_generated.unwrap_or(false),
            confidence: self.confidence.map(|v| v as f32),
            ..Default::default()
        }
    }

    pub fn from_rust(item: &TextItem) -> Self {
        Self {
            text: item.text.clone(),
            x: item.x as f64,
            y: item.y as f64,
            width: item.width as f64,
            height: item.height as f64,
            rotation: Some(item.rotation as f64),
            font_name: item.font_name.clone(),
            font_size: item.font_size.map(|v| v as f64),
            font_height: item.font_height.map(|v| v as f64),
            font_ascent: item.font_ascent.map(|v| v as f64),
            font_descent: item.font_descent.map(|v| v as f64),
            font_weight: item.font_weight,
            text_width: item.text_width.map(|v| v as f64),
            font_is_buggy: Some(item.font_is_buggy),
            mcid: item.mcid,
            fill_color: item.fill_color.clone(),
            stroke_color: item.stroke_color.clone(),
            char_codes: Some(item.char_codes.clone()),
            trailing_space_generated: Some(item.trailing_space_generated),
            confidence: item.confidence.map(|v| v as f64),
            words: item.words.iter().map(JsWordBox::from_rust).collect(),
        }
    }

    /// `from_rust` with the rich-metadata fields taken from the core-gated
    /// [`liteparse::types::TextMetadata`] view, so the "what counts as text
    /// metadata" list lives in one place instead of per binding.
    ///
    /// `serialize_words` is whether `TextItem::words` goes out to JS: core
    /// forces word-box extraction when `extract_blocks` is on (a
    /// table-detection input), but those unrequested boxes roughly double the
    /// text-item payload, so they are only returned when the caller asked —
    /// see [`JsParseResult::from_rust`].
    fn from_rust_for_output(
        item: &TextItem,
        extract_text_metadata: bool,
        serialize_words: bool,
    ) -> Self {
        let meta = item.text_metadata(extract_text_metadata);
        Self {
            words: if serialize_words {
                item.words.iter().map(JsWordBox::from_rust).collect()
            } else {
                Vec::new()
            },
            font_height: meta.font_height.map(|v| v as f64),
            font_ascent: meta.font_ascent.map(|v| v as f64),
            font_descent: meta.font_descent.map(|v| v as f64),
            font_weight: meta.font_weight,
            text_width: meta.text_width.map(|v| v as f64),
            font_is_buggy: meta.font_is_buggy,
            mcid: meta.mcid,
            fill_color: meta.fill_color.map(str::to_owned),
            stroke_color: meta.stroke_color.map(str::to_owned),
            char_codes: meta.char_codes.map(<[u32]>::to_vec),
            trailing_space_generated: meta.trailing_space_generated,
            ..Self::from_rust(item)
        }
    }
}

// ---------------------------------------------------------------------------
// Graphic primitive (pre-extracted vector graphics)
// ---------------------------------------------------------------------------

/// A vector-graphic primitive supplied by an external extractor. `kind` selects
/// the variant: `"stroke"` (uses `x1/y1/x2/y2`) or `"rect"` (uses
/// `x/y/width/height`). Coordinates are viewport space (top-left origin, 72
/// DPI), matching the text items. `has_fill`/`has_stroke` carry the paint
/// intent even when no color is known, so ruled-table edge detection still
/// treats a colorless stroked rect as stroked.
#[napi(object)]
#[derive(Clone)]
pub struct JsGraphic {
    /// "stroke" or "rect". Anything else is dropped.
    pub kind: String,
    // Stroke endpoints (used when kind == "stroke").
    pub x1: Option<f64>,
    pub y1: Option<f64>,
    pub x2: Option<f64>,
    pub y2: Option<f64>,
    // Rect bbox top-left + size (used when kind == "rect").
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub width: Option<f64>,
    pub height: Option<f64>,
    /// Whether the path is filled. Drives Rect `fill` presence.
    pub has_fill: Option<bool>,
    /// Whether the path is stroked. Drives Rect `stroke` presence.
    pub has_stroke: Option<bool>,
    /// Fill color as ARGB hex (e.g. "ff000000"). May be absent even when filled.
    pub fill_color: Option<String>,
    /// Stroke color as ARGB hex. May be absent even when stroked.
    pub stroke_color: Option<String>,
    /// Stroke line width in points.
    pub line_width: Option<f64>,
}

impl JsGraphic {
    pub fn to_rust(&self) -> Option<GraphicPrimitive> {
        match self.kind.as_str() {
            "stroke" => Some(GraphicPrimitive::Stroke {
                x1: self.x1.unwrap_or(0.0) as f32,
                y1: self.y1.unwrap_or(0.0) as f32,
                x2: self.x2.unwrap_or(0.0) as f32,
                y2: self.y2.unwrap_or(0.0) as f32,
                color: self.stroke_color.clone(),
                width: self.line_width.unwrap_or(0.0) as f32,
            }),
            "rect" => Some(GraphicPrimitive::Rect {
                bbox: Rect {
                    x: self.x.unwrap_or(0.0) as f32,
                    y: self.y.unwrap_or(0.0) as f32,
                    width: self.width.unwrap_or(0.0) as f32,
                    height: self.height.unwrap_or(0.0) as f32,
                },
                fill: if self.has_fill.unwrap_or(false) {
                    Some(self.fill_color.clone().unwrap_or_default())
                } else {
                    None
                },
                stroke: if self.has_stroke.unwrap_or(false) {
                    Some(self.stroke_color.clone().unwrap_or_default())
                } else {
                    None
                },
            }),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Page input (pre-extracted)
// ---------------------------------------------------------------------------

/// A page of pre-extracted text supplied by an external extractor. Coordinates
/// are viewport space (top-left origin, 72 DPI). `graphics` enables ruled-table
/// and horizontal-rule detection; struct nodes are still unsupported on this
/// path, so tagged-heading detection remains unavailable until they are added.
#[napi(object)]
#[derive(Clone)]
pub struct JsPageInput {
    pub page_number: u32,
    pub page_width: f64,
    pub page_height: f64,
    pub text_items: Vec<JsTextItem>,
    pub graphics: Option<Vec<JsGraphic>>,
}

impl JsPageInput {
    pub fn to_rust(&self) -> Page {
        Page {
            page_number: self.page_number as usize,
            page_width: self.page_width as f32,
            page_height: self.page_height as f32,
            content_bounds: None,
            text_items: self.text_items.iter().map(JsTextItem::to_rust).collect(),
            graphics: self
                .graphics
                .as_ref()
                .map(|gs| gs.iter().filter_map(JsGraphic::to_rust).collect())
                .unwrap_or_default(),
            vector_graphics: None,
            struct_nodes: Vec::new(),
            image_refs: Vec::new(),
            annotations: None,
            form_fields: None,
            structure_tree: None,
        }
    }
}

// ---------------------------------------------------------------------------
// ParsedPage
// ---------------------------------------------------------------------------

#[napi(object)]
#[derive(Clone)]
pub struct JsParsedPage {
    pub page_num: u32,
    pub width: f64,
    pub height: f64,
    /// Union bbox of the page's top-level content objects in viewport
    /// coords (visible content extent). Absent for empty pages.
    pub content_bounds: Option<JsRect>,
    pub text: String,
    pub markdown: String,
    pub text_items: Vec<JsTextItem>,
    pub complexity: Option<JsPageComplexityStats>,
    pub vector_graphics: Option<JsVectorGraphics>,
    pub annotations: Option<Vec<JsDocumentAnnotation>>,
    /// Classified layout blocks in reading order; present only when
    /// `extractBlocks` is enabled.
    pub blocks: Option<Vec<JsLayoutBlock>>,
    pub form_fields: Option<Vec<JsFormField>>,
    pub structure_tree: Option<JsStructureTree>,
}

#[napi(object)]
#[derive(Clone)]
pub struct JsStructureAttribute {
    pub name: String,
    pub boolean_value: Option<bool>,
    pub number_value: Option<f64>,
    pub string_value: Option<String>,
}

#[napi(object)]
#[derive(Clone)]
pub struct JsStructureTree {
    pub roots: Vec<JsStructureTreeElement>,
}

#[napi(object)]
#[derive(Clone)]
pub struct JsStructureTreeElement {
    pub element_type: String,
    pub id: Option<String>,
    pub actual_text: Option<String>,
    pub alt_text: Option<String>,
    pub title: Option<String>,
    pub attributes: Vec<JsStructureAttribute>,
    pub marked_content_ids: Vec<i32>,
    pub children: Vec<JsStructureTreeElement>,
    pub annotations: Vec<JsDocumentAnnotation>,
}

impl JsStructureTree {
    fn from_rust(tree: &StructureTree) -> Self {
        Self {
            roots: tree
                .roots
                .iter()
                .map(JsStructureTreeElement::from_rust)
                .collect(),
        }
    }
}

impl JsStructureTreeElement {
    fn from_rust(element: &StructureTreeElement) -> Self {
        Self {
            element_type: element.element_type.clone(),
            id: element.id.clone(),
            actual_text: element.actual_text.clone(),
            alt_text: element.alt_text.clone(),
            title: element.title.clone(),
            attributes: element
                .attributes
                .iter()
                .map(|(name, value)| {
                    let (boolean_value, number_value, string_value) = match value {
                        StructureAttributeValue::Boolean(value) => (Some(*value), None, None),
                        StructureAttributeValue::Number(value) => {
                            (None, Some(f64::from(*value)), None)
                        }
                        StructureAttributeValue::String(value) => (None, None, Some(value.clone())),
                    };
                    JsStructureAttribute {
                        name: name.clone(),
                        boolean_value,
                        number_value,
                        string_value,
                    }
                })
                .collect(),
            marked_content_ids: element.marked_content_ids.clone(),
            children: element.children.iter().map(Self::from_rust).collect(),
            annotations: element
                .annotations
                .iter()
                .map(JsDocumentAnnotation::from_rust)
                .collect(),
        }
    }
}

#[napi(object)]
#[derive(Clone)]
pub struct JsVectorShape {
    pub bbox: JsRect,
    pub stroke: bool,
    pub stroke_color: Option<String>,
    pub fill: bool,
    pub fill_color: Option<String>,
    pub has_curve: bool,
}

#[napi(object)]
#[derive(Clone)]
pub struct JsRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[napi(object)]
#[derive(Clone)]
pub struct JsAnnotationRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[napi(object)]
#[derive(Clone)]
pub struct JsVectorLine {
    pub x1: f64,
    pub y1: f64,
    pub x2: f64,
    pub y2: f64,
    pub stroke: bool,
    pub stroke_width: Option<f64>,
    pub stroke_color: Option<String>,
    pub fill: bool,
    pub fill_color: Option<String>,
}

#[napi(object)]
#[derive(Clone)]
pub struct JsVectorGraphics {
    pub shapes: Vec<JsVectorShape>,
    pub lines: Vec<JsVectorLine>,
}

impl JsVectorGraphics {
    fn from_rust(value: &VectorGraphics) -> Self {
        Self {
            shapes: value
                .shapes
                .iter()
                .map(|s| JsVectorShape {
                    bbox: JsRect {
                        x: s.bbox.x as f64,
                        y: s.bbox.y as f64,
                        width: s.bbox.width as f64,
                        height: s.bbox.height as f64,
                    },
                    stroke: s.stroke,
                    stroke_color: s.stroke_color.clone(),
                    fill: s.fill,
                    fill_color: s.fill_color.clone(),
                    has_curve: s.has_curve,
                })
                .collect(),
            lines: value
                .lines
                .iter()
                .map(|l| JsVectorLine {
                    x1: l.x1 as f64,
                    y1: l.y1 as f64,
                    x2: l.x2 as f64,
                    y2: l.y2 as f64,
                    stroke: l.stroke,
                    stroke_width: l.stroke_width.map(f64::from),
                    stroke_color: l.stroke_color.clone(),
                    fill: l.fill,
                    fill_color: l.fill_color.clone(),
                })
                .collect(),
        }
    }
}

impl JsAnnotationRect {
    fn from_rust(rect: &Rect) -> Self {
        Self {
            x: rect.x as f64,
            y: rect.y as f64,
            width: rect.width as f64,
            height: rect.height as f64,
        }
    }
}

#[napi(object)]
#[derive(Clone)]
pub struct JsDocumentAnnotation {
    pub subtype: String,
    pub contents: Option<String>,
    pub created: Option<String>,
    pub modified: Option<String>,
    pub title: Option<String>,
    pub rect: Option<JsAnnotationRect>,
    pub quadpoint_rects: Vec<JsAnnotationRect>,
    pub uri: Option<String>,
}

#[napi(object)]
#[derive(Clone)]
pub struct JsFormField {
    pub id: String,
    pub field_type: String,
    pub page: u32,
    pub annotation_index: i32,
    pub widget_index: i32,
    pub object_number: Option<i32>,
    pub name: Option<String>,
    pub alternate_name: Option<String>,
    pub value: Option<String>,
    pub export_value: Option<String>,
    pub field_flags: i32,
    pub control_count: Option<i32>,
    pub control_index: Option<i32>,
    pub checked: Option<bool>,
    pub rect: Option<JsAnnotationRect>,
    pub options: Vec<String>,
    pub selected_options: Vec<String>,
}

impl JsFormField {
    fn from_rust(field: &FormField) -> Self {
        Self {
            id: field.id.clone(),
            field_type: field.field_type.clone(),
            page: field.page,
            annotation_index: field.annotation_index,
            widget_index: field.widget_index,
            object_number: field.object_number,
            name: field.name.clone(),
            alternate_name: field.alternate_name.clone(),
            value: field.value.clone(),
            export_value: field.export_value.clone(),
            field_flags: field.field_flags,
            control_count: field.control_count,
            control_index: field.control_index,
            checked: field.checked,
            rect: field.rect.as_ref().map(JsAnnotationRect::from_rust),
            options: field.options.clone(),
            selected_options: field.selected_options.clone(),
        }
    }
}

impl JsDocumentAnnotation {
    fn from_rust(annotation: &DocumentAnnotation) -> Self {
        Self {
            subtype: annotation.subtype.clone(),
            contents: annotation.contents.clone(),
            created: annotation.created.clone(),
            modified: annotation.modified.clone(),
            title: annotation.title.clone(),
            rect: annotation.rect.as_ref().map(JsAnnotationRect::from_rust),
            quadpoint_rects: annotation
                .quadpoint_rects
                .iter()
                .map(JsAnnotationRect::from_rust)
                .collect(),
            uri: annotation.uri.clone(),
        }
    }
}

/// One table cell: its rendered text and the region it occupied. `bbox` is
/// absent for cells with no ink behind them (padding for a ragged grid, or a
/// merged run split at an estimated boundary).
#[napi(object)]
#[derive(Clone)]
pub struct JsLayoutCell {
    pub text: String,
    pub bbox: Option<JsAnnotationRect>,
    /// Indices into the page's returned `textItems`, in reading order, never
    /// repeating within one cell; empty for padding cells.
    pub text_item_indices: Vec<u32>,
}

/// A classified block plus where it sits on the page. Flat by design — `kind`
/// discriminates the block and every field that doesn't apply to that kind is
/// absent.
#[napi(object)]
#[derive(Clone)]
pub struct JsLayoutBlock {
    /// One of `heading`, `paragraph`, `list_item`, `code`, `table`,
    /// `grid_fallback`, `rule`, `figure`.
    pub kind: String,
    /// Indices into the page's returned `textItems`, sorted and deduped;
    /// empty for text-less blocks. For a `table` block, the union of its
    /// cells' indices.
    pub text_item_indices: Vec<u32>,
    /// Rendered text for the text-bearing kinds (`heading`, `paragraph`,
    /// `list_item`). Table text lives in `header`/`rows`; code and grid text in
    /// `lines`.
    pub text: Option<String>,
    /// Heading level (1–6), or list nesting depth for `list_item`.
    pub level: Option<u8>,
    /// Whether the block's text is uniformly bold / italic. `paragraph` and
    /// `list_item` only; false otherwise.
    pub bold: bool,
    pub italic: bool,
    /// `list_item`: whether the list is ordered, and the original marker as it
    /// appeared on the page (`138.`, `iii)`, `•`).
    pub ordered: Option<bool>,
    pub marker: Option<String>,
    /// Verbatim source lines for `code` and `grid_fallback`.
    pub lines: Option<Vec<String>>,
    /// Best-effort language hint for `code`.
    pub lang: Option<String>,
    /// `table`: the header row, when one was detected.
    pub header: Option<Vec<JsLayoutCell>>,
    /// `table`: the body rows.
    pub rows: Option<Vec<Vec<JsLayoutCell>>>,
    /// `figure`: the image's page-scoped id and encoded format, matching the
    /// `img_{id}.{format}` target the markdown renderer emits.
    pub id: Option<String>,
    pub format: Option<String>,
    /// Region of the page this block occupies, in the same viewport space as
    /// `textItems`. Absent when the block has no page geometry behind it.
    pub bbox: Option<JsAnnotationRect>,
}

impl JsLayoutCell {
    fn from_rust(cell: &LayoutCell) -> Self {
        Self {
            text: cell.text.clone(),
            bbox: cell.bbox.as_ref().map(JsAnnotationRect::from_rust),
            text_item_indices: cell.text_item_indices.iter().map(|&i| i as u32).collect(),
        }
    }
}

impl JsLayoutBlock {
    fn from_rust(block: &LayoutBlock) -> Self {
        let cells = |row: &Vec<LayoutCell>| row.iter().map(JsLayoutCell::from_rust).collect();
        Self {
            kind: block.kind.to_string(),
            text_item_indices: block.text_item_indices.iter().map(|&i| i as u32).collect(),
            text: block.text.clone(),
            level: block.level,
            bold: block.bold,
            italic: block.italic,
            ordered: block.ordered,
            marker: block.marker.clone(),
            lines: block.lines.clone(),
            lang: block.lang.clone(),
            header: block.header.as_ref().map(&cells),
            rows: block
                .rows
                .as_ref()
                .map(|rows| rows.iter().map(&cells).collect()),
            id: block.id.clone(),
            format: block.format.clone(),
            bbox: block.bbox.as_ref().map(JsAnnotationRect::from_rust),
        }
    }
}

impl JsParsedPage {
    pub fn from_rust(
        page: &ParsedPage,
        extract_text_metadata: bool,
        serialize_words: bool,
    ) -> Self {
        Self {
            page_num: page.page_number as u32,
            width: page.page_width as f64,
            height: page.page_height as f64,
            content_bounds: page.content_bounds.as_ref().map(|b| JsRect {
                x: b.x as f64,
                y: b.y as f64,
                width: b.width as f64,
                height: b.height as f64,
            }),
            text: page.text.clone(),
            markdown: page.markdown.clone(),
            text_items: page
                .text_items
                .iter()
                .map(|item| {
                    JsTextItem::from_rust_for_output(item, extract_text_metadata, serialize_words)
                })
                .collect(),
            complexity: page
                .complexity
                .as_ref()
                .map(JsPageComplexityStats::from_rust),
            vector_graphics: page
                .vector_graphics
                .as_ref()
                .map(JsVectorGraphics::from_rust),
            annotations: page.annotations.as_ref().map(|annotations| {
                annotations
                    .iter()
                    .map(JsDocumentAnnotation::from_rust)
                    .collect()
            }),
            blocks: page
                .blocks
                .as_ref()
                .map(|blocks| blocks.iter().map(JsLayoutBlock::from_rust).collect()),
            form_fields: page
                .form_fields
                .as_ref()
                .map(|fields| fields.iter().map(JsFormField::from_rust).collect()),
            structure_tree: page.structure_tree.as_ref().map(JsStructureTree::from_rust),
        }
    }
}

// ---------------------------------------------------------------------------
// ParseResult
// ---------------------------------------------------------------------------

#[napi(object)]
#[derive(Clone)]
pub struct JsParseResult {
    /// Total source-document pages before target/max-page filtering.
    pub total_pages: u32,
    pub pages: Vec<JsParsedPage>,
    pub page_errors: Vec<JsPageError>,
    pub text: String,
    pub images: Vec<JsExtractedImage>,
    pub screenshots: Vec<JsScreenshotResult>,
    pub image_error_count: u32,
    pub form_type: Option<i32>,
    /// The document's `/Info` `Creator` entry, when present.
    pub creator: Option<String>,
    /// The document's `/Info` `Producer` entry, when present.
    pub producer: Option<String>,
    /// Document-level provenance metadata; present only when
    /// `extractDocumentMetadata` is enabled and the input was a real PDF.
    pub doc_meta: Option<JsDocumentMetadata>,
    /// Raw XFA packets; present only when `extractXfaPackets` is enabled.
    pub xfa_packets: Option<Vec<JsXfaPacket>>,
}

/// One batch of pages from a `ParseSession`.
#[napi(object)]
pub struct JsParseBatch {
    /// First source page in this batch, 1-indexed.
    pub start_page: u32,
    /// Last source page in this batch, 1-indexed and inclusive.
    pub end_page: u32,
    /// The pages in `startPage..=endPage`, as an ordinary parse result.
    pub result: JsParseResult,
}

#[napi(object)]
#[derive(Clone)]
pub struct JsPageError {
    pub page_num: u32,
    pub message: String,
}

#[napi(object)]
#[derive(Clone)]
pub struct JsDocumentMetadata {
    pub creation_date: Option<String>,
    pub mod_date: Option<String>,
    pub file_version: Option<i32>,
    pub is_encrypted: Option<bool>,
    pub security_handler_revision: Option<i32>,
    pub permissions: Option<f64>,
    pub eof_section_count: Option<u32>,
    pub startxref_count: Option<u32>,
    pub trailer_id_pair_differs: Option<bool>,
    pub raw_file_size: Option<f64>,
    pub xmp: Option<String>,
    /// True when the catalog's XMP stream exceeded the 64 KiB cap.
    pub xmp_truncated: Option<bool>,
    pub signature_count: Option<u32>,
    pub signature_byte_range_reaches_eof: Option<bool>,
}

impl JsDocumentMetadata {
    fn from_rust(metadata: &liteparse::types::DocumentMetadata) -> Self {
        Self {
            creation_date: metadata.creation_date.clone(),
            mod_date: metadata.mod_date.clone(),
            file_version: metadata.file_version,
            is_encrypted: metadata.is_encrypted,
            security_handler_revision: metadata.security_handler_revision,
            permissions: metadata.permissions.map(|value| value as f64),
            eof_section_count: metadata.eof_section_count,
            startxref_count: metadata.startxref_count,
            trailer_id_pair_differs: metadata.trailer_id_pair_differs,
            raw_file_size: metadata.raw_file_size.map(|value| value as f64),
            xmp: metadata.xmp.clone(),
            xmp_truncated: metadata.xmp_truncated,
            signature_count: metadata.signature_count,
            signature_byte_range_reaches_eof: metadata.signature_byte_range_reaches_eof,
        }
    }
}

/// One raw packet from an XFA form document's `/XFA` array.
#[napi(object)]
#[derive(Clone)]
pub struct JsXfaPacket {
    pub index: u32,
    pub name: Option<String>,
    pub content_length: u32,
    /// Packet content (usually XML), lossily decoded as UTF-8.
    pub content: Option<String>,
}

impl JsXfaPacket {
    fn from_rust(packet: &XfaPacket) -> Self {
        Self {
            index: packet.index,
            name: packet.name.clone(),
            content_length: packet.content_length,
            content: packet.content.clone(),
        }
    }
}

#[napi(object)]
#[derive(Clone)]
pub struct JsImageRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[napi(object)]
#[derive(Clone)]
pub struct JsExtractedImage {
    pub id: String,
    pub name: String,
    pub path: Option<String>,
    pub page: u32,
    pub bbox: JsImageRect,
    pub width: u32,
    pub height: u32,
    pub rotation: f64,
    pub format: String,
    pub duplicate_of: Option<String>,
    pub bytes: napi::bindgen_prelude::Buffer,
}

// ---------------------------------------------------------------------------
// ScreenshotResult
// ---------------------------------------------------------------------------

#[napi(object)]
#[derive(Clone)]
pub struct JsScreenshotResult {
    pub page_num: u32,
    pub width: u32,
    pub height: u32,
    pub image_buffer: napi::bindgen_prelude::Buffer,
    /// True when every pixel has the same color (blank page after render).
    pub is_solid_fill: bool,
    /// Solid rectangles/lines detected in the raster (viewport coords).
    /// Populated only when `detectScreenshotRects` is enabled.
    pub rects: Vec<JsScreenshotRect>,
}

/// One solid rectangle (or line) detected in a rendered page bitmap, in
/// viewport coords (top-left origin, 72 DPI).
#[napi(object)]
#[derive(Clone)]
pub struct JsScreenshotRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    /// Fill color as ARGB hex string (e.g. "ff1a2b3c").
    pub color: String,
    /// True when the region is a solid line rather than a filled area.
    pub is_line: bool,
}

impl JsScreenshotRect {
    pub fn from_rust(rect: &ScreenshotRect) -> Self {
        Self {
            x: rect.x as f64,
            y: rect.y as f64,
            width: rect.width as f64,
            height: rect.height as f64,
            color: rect.color.clone(),
            is_line: rect.is_line,
        }
    }
}

impl JsScreenshotResult {
    pub fn from_rust(result: &liteparse::parser::ScreenshotResult) -> Self {
        Self {
            page_num: result.page_num,
            width: result.width,
            height: result.height,
            image_buffer: result.image_bytes.clone().into(),
            is_solid_fill: result.is_solid_fill,
            rects: result
                .rects
                .iter()
                .map(JsScreenshotRect::from_rust)
                .collect(),
        }
    }
}

#[napi(object)]
#[derive(Clone)]
pub struct JsLayoutComplexityStats {
    pub column_count: u32,
    pub ruled_table_count: u32,
    pub ruled_table_coverage: f64,
    pub text_table_run_count: u32,
    pub figure_count: u32,
    pub figure_coverage: f64,
    pub is_complex: bool,
    pub reasons: Vec<String>,
}

impl JsLayoutComplexityStats {
    pub fn from_rust(stats: &liteparse::ocr_merge::LayoutComplexityStats) -> Self {
        Self {
            column_count: stats.column_count as u32,
            ruled_table_count: stats.ruled_table_count as u32,
            ruled_table_coverage: stats.ruled_table_coverage as f64,
            text_table_run_count: stats.text_table_run_count as u32,
            figure_count: stats.figure_count as u32,
            figure_coverage: stats.figure_coverage as f64,
            is_complex: stats.is_complex,
            reasons: stats
                .reasons
                .iter()
                .map(|r| r.as_str().to_string())
                .collect(),
        }
    }
}

#[napi(object)]
#[derive(Clone)]
pub struct JsPageComplexityStats {
    pub page_number: u32,
    pub text_length: u32,
    pub text_coverage: f64,
    pub has_substantial_images: bool,
    /// Number of counted raster images — inline figures only; full-page
    /// backgrounds are excluded (see `fullPageImage`).
    pub image_block_count: u32,
    /// Summed image-bbox area over page area, clamped to 1. Counts inline
    /// figures only: a full-page scan raster contributes 0 here — check
    /// `fullPageImage` for that.
    pub image_coverage: f64,
    /// Largest single counted image's area over page area, clamped to 1. Same
    /// exclusion as `imageCoverage`: a full-page raster contributes 0.
    pub largest_image_coverage: f64,
    /// A single raster covering ≥90% of the page is present. Such full-page
    /// backgrounds are excluded from `imageCoverage`/`largestImageCoverage`
    /// (they're not inline figures), so this flag is the only signal that
    /// distinguishes a scan from a genuinely blank page — both otherwise
    /// report no text and no counted images.
    pub full_page_image: bool,
    pub uncovered_vector_area: Option<f64>,
    pub is_garbled: bool,
    pub page_area: f64,
    pub needs_ocr: bool,
    pub reasons: Vec<String>,
    pub layout: Option<JsLayoutComplexityStats>,
}

impl JsPageComplexityStats {
    pub fn from_rust(stats: &liteparse::ocr_merge::PageComplexityStats) -> Self {
        Self {
            page_number: stats.page_number as u32,
            text_length: stats.text_length as u32,
            text_coverage: stats.text_coverage as f64,
            has_substantial_images: stats.has_substantial_images,
            image_block_count: stats.image_block_count as u32,
            image_coverage: stats.image_coverage as f64,
            largest_image_coverage: stats.largest_image_coverage as f64,
            full_page_image: stats.full_page_image,
            uncovered_vector_area: stats.uncovered_vector_area.map(|v| v as f64),
            is_garbled: stats.is_garbled,
            page_area: stats.page_area as f64,
            needs_ocr: stats.needs_ocr,
            reasons: stats
                .reasons
                .iter()
                .map(|r| r.as_str().to_string())
                .collect(),
            layout: stats
                .layout
                .as_ref()
                .map(JsLayoutComplexityStats::from_rust),
        }
    }
}

impl JsParseResult {
    pub fn from_rust(result: &ParseResult, config: &LiteParseConfig) -> Self {
        // Whether `TextItem::words` goes out to JS: always when the caller
        // asked for word boxes; with `extract_blocks` off, whatever core
        // populated (stock behavior); with `extract_blocks` on and no
        // request, never — core forces word-box extraction as a
        // table-detection input, and shipping those unrequested boxes is
        // pure payload. `config` is the caller's requested config (core's
        // forcing never mutates it), so this reads the caller's own value.
        let serialize_words = config.emit_word_boxes || !config.extract_blocks;
        Self {
            total_pages: result.total_pages,
            pages: result
                .pages
                .iter()
                .map(|page| {
                    JsParsedPage::from_rust(page, config.extract_text_metadata, serialize_words)
                })
                .collect(),
            page_errors: result
                .page_errors
                .iter()
                .map(|error| JsPageError {
                    page_num: error.page_number,
                    message: error.message.clone(),
                })
                .collect(),
            text: result.text.clone(),
            image_error_count: result.image_error_count,
            form_type: result.form_type,
            creator: result.creator.clone(),
            producer: result.producer.clone(),
            doc_meta: result.doc_meta.as_ref().map(JsDocumentMetadata::from_rust),
            xfa_packets: result
                .xfa_packets
                .as_ref()
                .map(|packets| packets.iter().map(JsXfaPacket::from_rust).collect()),
            images: result
                .images
                .iter()
                .map(|img| JsExtractedImage {
                    id: img.id.clone(),
                    name: img.name.clone(),
                    path: img.path.clone(),
                    page: img.page,
                    bbox: JsImageRect {
                        x: img.bbox.x as f64,
                        y: img.bbox.y as f64,
                        width: img.bbox.width as f64,
                        height: img.bbox.height as f64,
                    },
                    width: img.width,
                    height: img.height,
                    rotation: img.rotation as f64,
                    format: img.format.clone(),
                    duplicate_of: img.duplicate_of.clone(),
                    bytes: img.bytes.as_slice().to_vec().into(),
                })
                .collect(),
            screenshots: result
                .screenshots
                .iter()
                .map(JsScreenshotResult::from_rust)
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_metadata_round_trips_through_napi_type() {
        let item = TextItem {
            text: "A".into(),
            font_height: Some(12.0),
            font_ascent: Some(9.0),
            font_descent: Some(-3.0),
            font_weight: Some(700),
            text_width: Some(8.0),
            font_is_buggy: true,
            mcid: Some(2),
            fill_color: Some("ff112233".into()),
            stroke_color: Some("ff445566".into()),
            char_codes: vec![65, 32],
            trailing_space_generated: true,
            ..Default::default()
        };

        let js = JsTextItem::from_rust(&item);
        assert_eq!(js.char_codes, Some(vec![65, 32]));
        assert_eq!(js.trailing_space_generated, Some(true));
        assert_eq!(js.fill_color.as_deref(), Some("ff112233"));

        let lightweight = JsTextItem::from_rust_for_output(&item, false, true);
        assert_eq!(lightweight.font_height, None);
        assert_eq!(lightweight.font_is_buggy, None);
        assert_eq!(lightweight.char_codes, None);
        assert_eq!(lightweight.trailing_space_generated, None);

        let round_trip = js.to_rust();
        assert_eq!(round_trip.font_height, Some(12.0));
        assert_eq!(round_trip.font_ascent, Some(9.0));
        assert_eq!(round_trip.font_descent, Some(-3.0));
        assert_eq!(round_trip.font_weight, Some(700));
        assert_eq!(round_trip.text_width, Some(8.0));
        assert!(round_trip.font_is_buggy);
        assert_eq!(round_trip.mcid, Some(2));
        assert_eq!(round_trip.stroke_color.as_deref(), Some("ff445566"));
        assert_eq!(round_trip.char_codes, vec![65, 32]);
        assert!(round_trip.trailing_space_generated);
    }

    /// Core forces word-box extraction when `extract_blocks` is on (a
    /// table-detection input), so the DTO layer must strip the unrequested
    /// boxes: `words` goes out only when the caller asked, or when
    /// `extract_blocks` is off (stock behavior — core then populates words
    /// only on request or for markdown output, and whatever it populated is
    /// passed through).
    #[test]
    fn words_serialized_only_when_requested_under_extract_blocks() {
        let item = TextItem {
            text: "two words".into(),
            words: vec![
                liteparse::types::WordBox {
                    text: "two".into(),
                    x: 0.0,
                    y: 0.0,
                    width: 10.0,
                    height: 5.0,
                },
                liteparse::types::WordBox {
                    text: "words".into(),
                    x: 12.0,
                    y: 0.0,
                    width: 16.0,
                    height: 5.0,
                },
            ],
            ..Default::default()
        };

        let stripped = JsTextItem::from_rust_for_output(&item, false, false);
        assert!(
            stripped.words.is_empty(),
            "forced word boxes must not ship when the caller didn't ask"
        );

        let kept = JsTextItem::from_rust_for_output(&item, false, true);
        assert_eq!(kept.words.len(), 2);
        assert_eq!(kept.words[0].text, "two");
        assert_eq!(kept.words[1].text, "words");
    }

    /// The requested-vs-forced resolution: `words` is serialized unless
    /// `extract_blocks` forced them on an unwilling caller.
    #[test]
    fn serialize_words_resolution_matches_wasm_binding() {
        let case = |emit_word_boxes: bool, extract_blocks: bool| {
            let config = LiteParseConfig {
                emit_word_boxes,
                extract_blocks,
                ..Default::default()
            };
            // The same expression `JsParseResult::from_rust` computes.
            config.emit_word_boxes || !config.extract_blocks
        };
        assert!(case(false, false), "stock path: pass through core output");
        assert!(case(true, false), "explicit request honored");
        assert!(case(true, true), "explicit request honored under blocks");
        assert!(!case(false, true), "forced-only word boxes are stripped");
    }

    #[test]
    fn text_metadata_config_defaults_off_and_round_trips() {
        let mut js = JsLiteParseConfig::from_rust(&LiteParseConfig::default());
        assert_eq!(js.extract_text_metadata, Some(false));
        js.extract_text_metadata = Some(true);
        assert!(js.into_rust().extract_text_metadata);
    }

    #[test]
    fn converts_vector_graphics_to_js_shape() {
        let rust = VectorGraphics {
            shapes: vec![liteparse::types::VectorShape {
                bbox: Rect {
                    x: 1.0,
                    y: 2.0,
                    width: 3.0,
                    height: 4.0,
                },
                stroke: true,
                stroke_color: Some("ff112233".into()),
                fill: false,
                fill_color: None,
                has_curve: true,
            }],
            lines: vec![liteparse::types::VectorLine {
                x1: 1.0,
                y1: 2.0,
                x2: 3.0,
                y2: 2.0,
                stroke: true,
                stroke_width: Some(0.5),
                stroke_color: Some("ff112233".into()),
                fill: false,
                fill_color: None,
            }],
        };
        let js = JsVectorGraphics::from_rust(&rust);
        assert_eq!(js.shapes[0].bbox.width, 3.0);
        assert!(js.shapes[0].has_curve);
        assert_eq!(js.lines[0].stroke_width, Some(0.5));
    }
}
