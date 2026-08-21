#![cfg(target_arch = "wasm32")]
//! WebAssembly bindings for LiteParse.
//!
//! Exposes a small JS-facing API mirroring `packages/node`:
//!   - `LiteParse` class with `new(config)`, `parse(Uint8Array)`
//!   - JS-side OCR callback bridge (any object with an async `recognize` method)

mod wasi_stubs;

use std::collections::HashMap;
use std::pin::Pin;

use js_sys::{Function, Reflect, Uint8Array};
use serde::{Deserialize, Serialize};
use tsify_next::Tsify;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

use liteparse::config::{
    CropBox as CoreCropBox, ImageMode, LiteParseConfig as CoreConfig, OutputFormat,
};
use liteparse::ocr::{OcrEngine, OcrOptions, OcrResult as CoreOcrResult};
use liteparse::parser::LiteParse as CoreLiteParse;
use liteparse::search;
use liteparse::types::PdfInput;

// ---------------------------------------------------------------------------
// Setup
// ---------------------------------------------------------------------------

#[wasm_bindgen(start)]
pub fn __wasm_start() {
    #[cfg(feature = "panic_hook")]
    console_error_panic_hook::set_once();
}

// ---------------------------------------------------------------------------
// JS-facing config (camelCase to match the Node package)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default, Tsify)]
#[tsify(into_wasm_abi, from_wasm_abi)]
#[serde(rename_all = "camelCase", default)]
pub struct LiteParseConfig {
    ocr_language: Option<String>,
    ocr_enabled: Option<bool>,
    ocr_server_url: Option<String>,
    ocr_server_headers: Option<HashMap<String, String>>,
    tessdata_path: Option<String>,
    max_pages: Option<usize>,
    target_pages: Option<String>,
    /// Skip page-level PDF extraction failures and report them in
    /// `ParseResult.pageErrors`. Document-level failures remain fatal.
    continue_on_page_error: Option<bool>,
    /// Render parsed pages to PNG and return them in
    /// `ParseResult.screenshots`. Default false; PNG payloads can be large.
    extract_screenshots: Option<bool>,
    /// Scan rendered screenshots for solid rectangles/lines and attach
    /// them to each screenshot result. Default false.
    detect_screenshot_rects: Option<bool>,
    dpi: Option<f32>,
    #[tsify(type = "\"json\" | \"text\" | \"markdown\" | \"md\"")]
    output_format: Option<String>,
    #[tsify(type = "\"off\" | \"none\" | \"placeholder\" | \"embed\"")]
    image_mode: Option<String>,
    extract_images: Option<bool>,
    extract_links: Option<bool>,
    /// Keep running headers/footers in markdown output instead of stripping
    /// repeated page-band lines and page chrome. Default false.
    keep_headers_footers: Option<bool>,
    extract_annotations: Option<bool>,
    extract_form_fields: Option<bool>,
    extract_structure_tree: Option<bool>,
    /// Extract raw XFA packets (name + XML content) into
    /// `ParseResult.xfaPackets`. Default false.
    extract_xfa_packets: Option<bool>,
    /// Collect document provenance metadata into `ParseResult.docMeta`.
    /// Default false: it streams the whole source file once.
    extract_document_metadata: Option<bool>,
    /// Emit each page's `contentBounds` (union bbox of top-level content
    /// objects, viewport coords). Default false.
    extract_content_bounds: Option<bool>,
    ocr_failure_fatal: Option<bool>,
    ocr_hedge_delays_ms: Option<Vec<u64>>,
    preserve_very_small_text: Option<bool>,
    password: Option<String>,
    quiet: Option<bool>,
    emit_word_boxes: Option<bool>,
    /// Restrict output to a page sub-region. Each field is the fraction of the
    /// page cropped from that side; a text item survives only if it lies
    /// entirely inside the remaining rectangle. Unset keeps the whole page.
    crop_box: Option<CropBox>,
    /// Drop diagonal text (rotation >2° off the nearest right angle). Default
    /// false. Use to exclude rotated watermarks/stamps from the output.
    skip_diagonal_text: Option<bool>,
    /// Compute per-page complexity signals during parse and attach them to each
    /// page as `ParsedPage.complexity` (the same signals `isComplex` returns).
    /// Default false; enabling it runs an extra vector-text detection pass.
    include_complexity: Option<bool>,
    /// Expose page-scoped vector shapes and merged H/V line segments.
    extract_vector_graphics: Option<bool>,
    /// Include rich PDF text metadata on text items (font metrics/weight,
    /// buggy state, MCID, fill/stroke colors, raw char codes, generated
    /// trailing-space state). Default false.
    extract_text_metadata: Option<bool>,
    /// Draw AcroForm field appearances into OCR rasters (runs document
    /// open/JS actions). Default false.
    render_form_fields: Option<bool>,
    /// Attach the markdown classifier's block decomposition to each page as
    /// `ParsedPage.blocks`. Default false.
    extract_blocks: Option<bool>,
}

/// A page sub-region as the fraction cropped from each side (top-left origin,
/// each in `[0, 1]`).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, Tsify)]
#[tsify(into_wasm_abi, from_wasm_abi)]
#[serde(rename_all = "camelCase", default)]
pub struct CropBox {
    top: f32,
    right: f32,
    bottom: f32,
    left: f32,
}

impl LiteParseConfig {
    fn into_core(self) -> Result<CoreConfig, JsError> {
        let mut cfg = CoreConfig::default();
        if let Some(v) = self.ocr_language {
            cfg.ocr_language = v;
        }
        if let Some(v) = self.ocr_enabled {
            cfg.ocr_enabled = v;
        }
        if self.ocr_server_url.is_some() {
            cfg.ocr_server_url = self.ocr_server_url;
        }
        if let Some(v) = self.ocr_server_headers {
            cfg.ocr_server_headers = v.into_iter().collect();
        }
        if self.tessdata_path.is_some() {
            cfg.tessdata_path = self.tessdata_path;
        }
        if let Some(v) = self.max_pages {
            cfg.max_pages = v;
        }
        if self.target_pages.is_some() {
            cfg.target_pages = self.target_pages;
        }
        if let Some(v) = self.continue_on_page_error {
            cfg.continue_on_page_error = v;
        }
        if let Some(v) = self.extract_screenshots {
            cfg.extract_screenshots = v;
        }
        if let Some(v) = self.detect_screenshot_rects {
            cfg.detect_screenshot_rects = v;
        }
        if let Some(v) = self.dpi {
            cfg.dpi = v;
        }
        if let Some(v) = self.output_format {
            cfg.output_format = match v.as_str() {
                "json" => OutputFormat::Json,
                "text" => OutputFormat::Text,
                "markdown" | "md" => OutputFormat::Markdown,
                other => {
                    return Err(JsError::new(&format!(
                        "invalid outputFormat: {} (expected 'json', 'text', or 'markdown')",
                        other
                    )));
                }
            };
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
        if let Some(v) = self.extract_links {
            cfg.extract_links = v;
        }
        if let Some(v) = self.keep_headers_footers {
            cfg.keep_headers_footers = v;
        }
        if let Some(v) = self.extract_annotations {
            cfg.extract_annotations = v;
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
        if let Some(v) = self.ocr_failure_fatal {
            cfg.ocr_failure_fatal = v;
        }
        if let Some(v) = self.ocr_hedge_delays_ms {
            cfg.ocr_hedge_delays_ms = v;
        }
        if let Some(v) = self.preserve_very_small_text {
            cfg.preserve_very_small_text = v;
        }
        if self.password.is_some() {
            cfg.password = self.password;
        }
        if let Some(v) = self.quiet {
            cfg.quiet = v;
        }
        if let Some(v) = self.emit_word_boxes {
            cfg.emit_word_boxes = v;
        }
        if let Some(v) = self.crop_box {
            cfg.crop_box = Some(CoreCropBox {
                top: v.top,
                right: v.right,
                bottom: v.bottom,
                left: v.left,
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
        if let Some(v) = self.extract_text_metadata {
            cfg.extract_text_metadata = v;
        }
        if let Some(v) = self.render_form_fields {
            cfg.render_form_fields = v;
        }
        if let Some(v) = self.extract_blocks {
            cfg.extract_blocks = v;
        }
        cfg.num_workers = 1;
        Ok(cfg)
    }

    /// Rebuild the JS-facing config from the resolved core config. Core's
    /// `extract_blocks` word-box forcing never mutates the config (it applies
    /// at extraction time), so `cfg.emit_word_boxes` is always the caller's
    /// own `emitWordBoxes` request — which is also what the serialized output
    /// honors.
    fn from_core(cfg: &CoreConfig) -> Self {
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
            max_pages: Some(cfg.max_pages),
            target_pages: cfg.target_pages.clone(),
            continue_on_page_error: Some(cfg.continue_on_page_error),
            extract_screenshots: Some(cfg.extract_screenshots),
            detect_screenshot_rects: Some(cfg.detect_screenshot_rects),
            dpi: Some(cfg.dpi),
            output_format: Some(match cfg.output_format {
                OutputFormat::Json => "json".into(),
                OutputFormat::Text => "text".into(),
                OutputFormat::Markdown => "markdown".into(),
            }),
            image_mode: Some(match cfg.image_mode {
                ImageMode::Off => "off".into(),
                ImageMode::Placeholder => "placeholder".into(),
                ImageMode::Embed => "embed".into(),
            }),
            extract_images: Some(cfg.extract_images),
            extract_links: Some(cfg.extract_links),
            keep_headers_footers: Some(cfg.keep_headers_footers),
            extract_annotations: Some(cfg.extract_annotations),
            extract_form_fields: Some(cfg.extract_form_fields),
            extract_structure_tree: Some(cfg.extract_structure_tree),
            extract_xfa_packets: Some(cfg.extract_xfa_packets),
            extract_document_metadata: Some(cfg.extract_document_metadata),
            extract_content_bounds: Some(cfg.extract_content_bounds),
            ocr_failure_fatal: Some(cfg.ocr_failure_fatal),
            ocr_hedge_delays_ms: Some(cfg.ocr_hedge_delays_ms.clone()),
            preserve_very_small_text: Some(cfg.preserve_very_small_text),
            password: cfg.password.clone(),
            quiet: Some(cfg.quiet),
            emit_word_boxes: Some(cfg.emit_word_boxes),
            crop_box: cfg.crop_box.map(|c| CropBox {
                top: c.top,
                right: c.right,
                bottom: c.bottom,
                left: c.left,
            }),
            skip_diagonal_text: Some(cfg.skip_diagonal_text),
            include_complexity: Some(cfg.include_complexity),
            extract_vector_graphics: Some(cfg.extract_vector_graphics),
            extract_text_metadata: Some(cfg.extract_text_metadata),
            render_form_fields: Some(cfg.render_form_fields),
            extract_blocks: Some(cfg.extract_blocks),
        }
    }
}

// ---------------------------------------------------------------------------
// JS-facing parse result
// ---------------------------------------------------------------------------

#[derive(Serialize, Tsify)]
#[tsify(into_wasm_abi)]
#[serde(rename_all = "camelCase")]
pub struct WordBox {
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Serialize, Tsify)]
#[tsify(into_wasm_abi)]
#[serde(rename_all = "camelCase")]
pub struct TextItem {
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_size: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
    /// Rotation in degrees (viewport space).
    pub rotation: f32,
    // Rich text metadata; present only when `extractTextMetadata` is set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_height: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_ascent: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_descent: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_weight: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_width: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_is_buggy: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mcid: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fill_color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stroke_color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub char_codes: Option<Vec<u32>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trailing_space_generated: Option<bool>,
    /// Per-word sub-boxes for attribution. Omitted when empty (the default —
    /// only populated when parsing with `emitWordBoxes: true`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub words: Option<Vec<WordBox>>,
}

#[derive(Serialize, Tsify)]
#[tsify(into_wasm_abi)]
#[serde(rename_all = "camelCase")]
pub struct ParsedPage {
    pub page_num: usize,
    pub width: f32,
    pub height: f32,
    /// Union bbox of the page's top-level content objects in viewport
    /// coords (visible content extent). Absent for empty pages.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_bounds: Option<VectorRect>,
    pub text: String,
    pub markdown: String,
    pub text_items: Vec<TextItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub complexity: Option<PageComplexityStats>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vector_graphics: Option<VectorGraphics>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotations: Option<Vec<DocumentAnnotation>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub form_fields: Option<Vec<FormField>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub structure_tree: Option<StructureTree>,
    /// Classified layout blocks in reading order; present only when
    /// `extractBlocks` is enabled.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocks: Option<Vec<LayoutBlock>>,
}

#[derive(Serialize, Tsify)]
#[tsify(into_wasm_abi)]
#[serde(rename_all = "camelCase")]
pub struct VectorGraphics {
    pub shapes: Vec<VectorShape>,
    pub lines: Vec<VectorLine>,
}

#[derive(Serialize, Tsify)]
#[tsify(into_wasm_abi)]
#[serde(rename_all = "camelCase")]
pub struct VectorShape {
    pub bbox: VectorRect,
    pub stroke: bool,
    pub stroke_color: Option<String>,
    pub fill: bool,
    pub fill_color: Option<String>,
    pub has_curve: bool,
}

#[derive(Serialize, Tsify)]
#[tsify(into_wasm_abi)]
#[serde(rename_all = "camelCase")]
pub struct VectorRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Serialize, Tsify)]
#[tsify(into_wasm_abi)]
#[serde(rename_all = "camelCase")]
pub struct AnnotationRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Serialize, Tsify)]
#[tsify(into_wasm_abi)]
#[serde(rename_all = "camelCase")]
pub struct VectorLine {
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
    pub stroke: bool,
    pub stroke_width: Option<f32>,
    pub stroke_color: Option<String>,
    pub fill: bool,
    pub fill_color: Option<String>,
}

#[derive(Serialize, Tsify)]
#[tsify(into_wasm_abi)]
#[serde(rename_all = "camelCase")]
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
    pub rect: Option<AnnotationRect>,
    pub quadpoint_rects: Vec<AnnotationRect>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
}

impl DocumentAnnotation {
    fn from_rust(annotation: &liteparse::types::DocumentAnnotation) -> Self {
        let to_rect = |rect: &liteparse::types::Rect| AnnotationRect {
            x: rect.x,
            y: rect.y,
            width: rect.width,
            height: rect.height,
        };
        Self {
            subtype: annotation.subtype.clone(),
            contents: annotation.contents.clone(),
            created: annotation.created.clone(),
            modified: annotation.modified.clone(),
            title: annotation.title.clone(),
            rect: annotation.rect.as_ref().map(to_rect),
            quadpoint_rects: annotation.quadpoint_rects.iter().map(to_rect).collect(),
            uri: annotation.uri.clone(),
        }
    }
}

/// One table cell: its rendered text and the region it occupied.
///
/// `bbox` is absent for cells with no ink behind them — padding inserted to
/// square off a ragged grid, or halves of a merged run split at an estimated
/// position.
#[derive(Serialize, Tsify)]
#[tsify(into_wasm_abi)]
#[serde(rename_all = "camelCase")]
pub struct LayoutCell {
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bbox: Option<AnnotationRect>,
    /// Indices into the page's `textItems` array (the order the caller
    /// receives it), in reading order. Always present; `[]` for padding
    /// cells inserted to square off a ragged grid. An index never repeats
    /// within one cell but may appear in several cells (a merged span split
    /// across column tracks attributes each fragment to the whole span).
    pub text_item_indices: Vec<u32>,
}

/// A classified block plus where it sits on the page.
///
/// `kind` discriminates the block and every field that doesn't apply to that
/// kind is omitted, so a heading serializes as `{kind, text, level, bbox}`.
/// Blocks appear in reading order, matching the page's markdown.
#[derive(Serialize, Tsify)]
#[tsify(into_wasm_abi)]
#[serde(rename_all = "camelCase")]
pub struct LayoutBlock {
    /// One of `heading`, `paragraph`, `list_item`, `code`, `table`,
    /// `grid_fallback`, `rule`, `figure`.
    pub kind: String,
    /// Indices into the page's `textItems` array (the order the caller
    /// receives it), sorted and deduped. Always present; `[]` for text-less
    /// blocks (vector `rule`s, `figure`s). For a `table` block this is the
    /// union of all cell indices.
    pub text_item_indices: Vec<u32>,
    /// Rendered text for the text-bearing kinds (`heading`, `paragraph`,
    /// `list_item`). Table text lives in `header`/`rows`; code and grid text
    /// in `lines`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Heading level (1–6), or list nesting depth for `list_item`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<u8>,
    /// Whether the block's text is uniformly bold / italic. `paragraph` and
    /// `list_item` only; omitted when false.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub bold: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub italic: bool,
    /// `list_item`: whether the list is ordered, and the original marker as it
    /// appeared on the page (`138.`, `iii)`, `•`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ordered: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub marker: Option<String>,
    /// Verbatim source lines for `code` and `grid_fallback`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lines: Option<Vec<String>>,
    /// Best-effort language hint for `code`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lang: Option<String>,
    /// `table`: the header row, when one was detected.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub header: Option<Vec<LayoutCell>>,
    /// `table`: the body rows.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rows: Option<Vec<Vec<LayoutCell>>>,
    /// `figure`: the image's page-scoped id and encoded format, matching the
    /// `img_{id}.{format}` target the markdown renderer emits.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    /// Region of the page this block occupies, in the same top-left, 72-DPI
    /// viewport space as `textItems`. Absent when the block has no page
    /// geometry behind it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bbox: Option<AnnotationRect>,
}

fn to_annotation_rect(rect: &liteparse::types::Rect) -> AnnotationRect {
    AnnotationRect {
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
    }
}

/// Narrow core `usize` provenance indices to the `u32` the JS DTO carries.
/// Lossless in practice: indices address a page's `textItems` array, which
/// never approaches `u32::MAX` entries.
fn to_js_indices(indices: &[usize]) -> Vec<u32> {
    indices.iter().map(|&i| i as u32).collect()
}

impl LayoutCell {
    fn from_rust(cell: &liteparse::layout::LayoutCell) -> Self {
        Self {
            text: cell.text.clone(),
            bbox: cell.bbox.as_ref().map(to_annotation_rect),
            text_item_indices: to_js_indices(&cell.text_item_indices),
        }
    }
}

impl LayoutBlock {
    fn from_rust(block: &liteparse::layout::LayoutBlock) -> Self {
        let cells = |row: &Vec<liteparse::layout::LayoutCell>| {
            row.iter().map(LayoutCell::from_rust).collect::<Vec<_>>()
        };
        Self {
            kind: block.kind.to_string(),
            text_item_indices: to_js_indices(&block.text_item_indices),
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
            bbox: block.bbox.as_ref().map(to_annotation_rect),
        }
    }
}

#[derive(Serialize, Tsify)]
#[tsify(into_wasm_abi)]
#[serde(rename_all = "camelCase")]
pub struct FormField {
    pub id: String,
    #[serde(rename = "type")]
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
    pub rect: Option<AnnotationRect>,
    pub options: Vec<String>,
    pub selected_options: Vec<String>,
}

impl FormField {
    fn from_rust(field: &liteparse::types::FormField) -> Self {
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
            rect: field.rect.as_ref().map(|rect| AnnotationRect {
                x: rect.x,
                y: rect.y,
                width: rect.width,
                height: rect.height,
            }),
            options: field.options.clone(),
            selected_options: field.selected_options.clone(),
        }
    }
}

/// Scalar value from a tagged-PDF structure element's `/A` dictionary.
#[derive(Serialize, Tsify)]
#[tsify(into_wasm_abi)]
#[serde(untagged)]
pub enum StructureAttributeValue {
    Boolean(bool),
    Number(f32),
    String(String),
}

#[derive(Serialize, Tsify)]
#[tsify(into_wasm_abi)]
#[serde(rename_all = "camelCase")]
pub struct StructureTree {
    pub roots: Vec<StructureTreeElement>,
}

#[derive(Serialize, Tsify)]
#[tsify(into_wasm_abi)]
#[serde(rename_all = "camelCase")]
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
    pub attributes: std::collections::BTreeMap<String, StructureAttributeValue>,
    pub marked_content_ids: Vec<i32>,
    pub children: Vec<StructureTreeElement>,
    pub annotations: Vec<DocumentAnnotation>,
}

impl StructureTree {
    fn from_rust(tree: &liteparse::types::StructureTree) -> Self {
        Self {
            roots: tree
                .roots
                .iter()
                .map(StructureTreeElement::from_rust)
                .collect(),
        }
    }
}

impl StructureTreeElement {
    fn from_rust(element: &liteparse::types::StructureTreeElement) -> Self {
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
                    let value = match value {
                        liteparse::types::StructureAttributeValue::Boolean(v) => {
                            StructureAttributeValue::Boolean(*v)
                        }
                        liteparse::types::StructureAttributeValue::Number(v) => {
                            StructureAttributeValue::Number(*v)
                        }
                        liteparse::types::StructureAttributeValue::String(v) => {
                            StructureAttributeValue::String(v.clone())
                        }
                    };
                    (name.clone(), value)
                })
                .collect(),
            marked_content_ids: element.marked_content_ids.clone(),
            children: element
                .children
                .iter()
                .map(StructureTreeElement::from_rust)
                .collect(),
            annotations: element
                .annotations
                .iter()
                .map(DocumentAnnotation::from_rust)
                .collect(),
        }
    }
}

#[derive(Serialize, Tsify)]
#[tsify(into_wasm_abi)]
#[serde(rename_all = "camelCase")]
pub struct ParseResult {
    /// Total source-document pages before target/max-page filtering.
    pub total_pages: u32,
    pub pages: Vec<ParsedPage>,
    pub text: String,
    pub images: Vec<ExtractedImage>,
    /// Page screenshots encoded as PNG. Empty unless `extractScreenshots`
    /// is enabled.
    pub screenshots: Vec<ScreenshotResult>,
    pub image_error_count: u32,
    pub page_errors: Vec<PageError>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub form_type: Option<i32>,
    /// The document's `/Info` `Creator` entry, when present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub creator: Option<String>,
    /// The document's `/Info` `Producer` entry, when present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub producer: Option<String>,
    /// Document-level provenance metadata; present only when
    /// `extractDocumentMetadata` is enabled and the input was a real PDF.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc_meta: Option<DocumentMetadata>,
    /// Raw XFA packets; present only when `extractXfaPackets` is enabled.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub xfa_packets: Option<Vec<XfaPacket>>,
}

#[derive(Serialize, Tsify)]
#[tsify(into_wasm_abi)]
#[serde(rename_all = "camelCase")]
pub struct PageError {
    pub page_num: u32,
    pub message: String,
}

/// One page rendered to PNG, plus raster-derived signals.
#[derive(Serialize, Tsify)]
#[tsify(into_wasm_abi)]
#[serde(rename_all = "camelCase")]
pub struct ScreenshotResult {
    pub page_num: u32,
    pub width: u32,
    pub height: u32,
    /// PNG-encoded image bytes.
    pub image_bytes: Vec<u8>,
    /// True when every pixel has the same color (blank page after render).
    pub is_solid_fill: bool,
    /// Solid rectangles/lines detected in the raster (viewport coords).
    /// Empty unless `detectScreenshotRects` is enabled.
    pub rects: Vec<ScreenshotRect>,
}

/// A solid rectangle or line detected in a rendered page.
#[derive(Serialize, Tsify)]
#[tsify(into_wasm_abi)]
#[serde(rename_all = "camelCase")]
pub struct ScreenshotRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    /// Fill color as ARGB hex string (e.g. "ff1a2b3c").
    pub color: String,
    /// True when the rect is thin enough to be a rule/divider line.
    pub is_line: bool,
}

#[derive(Serialize, Tsify)]
#[tsify(into_wasm_abi)]
#[serde(rename_all = "camelCase")]
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
    pub permissions: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eof_section_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub startxref_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trailer_id_pair_differs: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_file_size: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub xmp: Option<String>,
    /// True when the catalog's XMP stream exceeded the 64 KiB cap. WASM
    /// builds never populate `xmp`, so this is always absent there.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub xmp_truncated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature_byte_range_reaches_eof: Option<bool>,
}

impl From<&liteparse::types::DocumentMetadata> for DocumentMetadata {
    fn from(metadata: &liteparse::types::DocumentMetadata) -> Self {
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
#[derive(Serialize, Tsify)]
#[tsify(into_wasm_abi)]
#[serde(rename_all = "camelCase")]
pub struct XfaPacket {
    pub index: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub content_length: u32,
    /// Packet content (usually XML), lossily decoded as UTF-8.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
}

#[derive(Serialize, Tsify)]
#[tsify(into_wasm_abi)]
#[serde(rename_all = "camelCase")]
pub struct ExtractedImage {
    pub id: String,
    pub name: String,
    pub path: Option<String>,
    pub page: u32,
    pub bbox: ImageRect,
    pub width: u32,
    pub height: u32,
    pub rotation: f32,
    pub format: String,
    pub duplicate_of: Option<String>,
    /// Raw image bytes, serialized as a JS `number[]`. Callers that want a
    /// Uint8Array can wrap with `new Uint8Array(image.bytes)`.
    pub bytes: Vec<u8>,
}

#[derive(Serialize, Tsify)]
#[tsify(into_wasm_abi)]
#[serde(rename_all = "camelCase")]
pub struct ImageRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

// ---------------------------------------------------------------------------
// JS OCR engine bridge
// ---------------------------------------------------------------------------

/// Wraps a JS object that exposes an async `recognize(imageData, width, height, language)`
/// method, returning `Promise<Array<{text, bbox, confidence}>>`.
///
/// `JsValue` is `!Send`, but on `wasm32` (single-threaded) the trait does not
/// require `Send + Sync`, so this works.
struct JsOcrEngine {
    name: String,
    obj: JsValue,
}

impl JsOcrEngine {
    fn new(obj: JsValue) -> Self {
        Self {
            name: "js-callback".into(),
            obj,
        }
    }
}

impl OcrEngine for JsOcrEngine {
    fn name(&self) -> &str {
        &self.name
    }

    fn recognize<'a, 'b: 'a, 'c: 'a>(
        &'a self,
        image_data: &'c [u8],
        width: u32,
        height: u32,
        options: &'b OcrOptions,
    ) -> Pin<
        Box<
            dyn Future<
                    Output = Result<Vec<CoreOcrResult>, Box<dyn std::error::Error + Send + Sync>>,
                > + '_,
        >,
    > {
        // Copy bytes into a JS Uint8Array up-front (must happen on the
        // current thread anyway in wasm).
        let arr = Uint8Array::new_with_length(image_data.len() as u32);
        arr.copy_from(image_data);
        let language = options.language.clone();

        Box::pin(async move {
            let recognize: JsValue = Reflect::get(&self.obj, &JsValue::from_str("recognize"))
                .map_err(|e| format!("ocrEngine.recognize lookup failed: {:?}", e))?;
            let recognize: Function = recognize
                .dyn_into::<Function>()
                .map_err(|_| "ocrEngine.recognize is not a function".to_string())?;

            let args = js_sys::Array::new();
            args.push(&arr);
            args.push(&JsValue::from_f64(width as f64));
            args.push(&JsValue::from_f64(height as f64));
            args.push(&JsValue::from_str(&language));

            let promise = recognize
                .apply(&self.obj, &args)
                .map_err(|e| format!("ocrEngine.recognize threw: {:?}", e))?;
            let promise: js_sys::Promise = promise
                .dyn_into::<js_sys::Promise>()
                .map_err(|_| "ocrEngine.recognize did not return a Promise".to_string())?;

            let resolved = JsFuture::from(promise)
                .await
                .map_err(|e| format!("ocrEngine.recognize rejected: {:?}", e))?;

            let parsed: Vec<OcrResult> = serde_wasm_bindgen::from_value(resolved)
                .map_err(|e| format!("ocrEngine.recognize result decode failed: {:?}", e))?;

            Ok(parsed
                .into_iter()
                .map(|r| CoreOcrResult {
                    text: r.text,
                    bbox: r.bbox,
                    confidence: r.confidence,
                    polygon: r.polygon,
                })
                .collect())
        })
    }
}

#[derive(Deserialize, Tsify)]
#[tsify(from_wasm_abi)]
#[serde(rename_all = "camelCase")]
pub struct OcrResult {
    pub text: String,
    pub bbox: [f32; 4],
    pub confidence: f32,
    #[serde(default)]
    pub polygon: Option<[[f32; 2]; 4]>,
}

// ---------------------------------------------------------------------------
// LiteParse class (JS-facing)
// ---------------------------------------------------------------------------

// Hand-written TS types that tsify can't derive: the JS-implemented OCR engine
// interface, and the constructor init object (the config plus an optional
// `ocrEngine`). `LiteParseConfig` and `OcrResult` are generated by tsify.
#[wasm_bindgen(typescript_custom_section)]
const TS_EXTRA: &'static str = r#"
export interface OcrEngine {
  recognize(imageData: Uint8Array, width: number, height: number, language: string): Promise<OcrResult[]>;
}

export interface LiteParseInit extends LiteParseConfig {
  ocrEngine?: OcrEngine;
}
"#;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "LiteParseInit")]
    pub type LiteParseInit;
}

#[wasm_bindgen]
pub struct LiteParse {
    inner: CoreLiteParse,
    /// The resolved config as the caller requested it. Core's
    /// `extract_blocks` word-box forcing happens at extraction time without
    /// mutating the config, so `config.emit_word_boxes` here is always the
    /// caller's own `emitWordBoxes` value — see [`Self::serialize_words`].
    config: CoreConfig,
}

#[wasm_bindgen]
impl LiteParse {
    /// Construct a new parser. `config` is a JS object (all fields optional).
    /// If `config.ocrEngine` is present, it is wired up as the OCR backend.
    #[wasm_bindgen(constructor)]
    pub fn new(config: LiteParseInit) -> Result<LiteParse, JsError> {
        let config: JsValue = config.into();
        let ocr_engine_js = if config.is_object() {
            Reflect::get(&config, &JsValue::from_str("ocrEngine"))
                .ok()
                .filter(|v| !v.is_undefined() && !v.is_null())
        } else {
            None
        };

        let js_cfg: LiteParseConfig = if config.is_undefined() || config.is_null() {
            LiteParseConfig::default()
        } else {
            serde_wasm_bindgen::from_value(config)
                .map_err(|e| JsError::new(&format!("invalid config: {}", e)))?
        };
        let core_cfg = js_cfg.into_core()?;
        // With `extractBlocks` on, core forces word-box extraction internally
        // as a table-detection input (word-anchored straddle splits) without
        // mutating the config; the DTO mapping still omits `words` unless the
        // caller requested them — see `serialize_words`.
        let mut parser = CoreLiteParse::new(core_cfg.clone());
        if let Some(js_engine) = ocr_engine_js {
            parser = parser.with_ocr_engine(std::sync::Arc::new(JsOcrEngine::new(js_engine)));
        }
        Ok(LiteParse {
            inner: parser,
            config: core_cfg,
        })
    }

    /// Return the resolved config (camelCase JS object).
    #[wasm_bindgen(getter)]
    pub fn config(&self) -> LiteParseConfig {
        LiteParseConfig::from_core(&self.config)
    }

    /// Parse PDF bytes. Returns `Promise<ParseResult>`.
    pub async fn parse(&self, data: Vec<u8>) -> Result<ParseResult, JsError> {
        let result = self
            .inner
            .parse_input(PdfInput::Bytes(data))
            .await
            .map_err(|e| JsError::new(&format!("parse failed: {}", e)))?;

        Ok(to_js_result(
            &result,
            self.inner.config().extract_text_metadata,
            self.serialize_words(),
        ))
    }
}

impl LiteParse {
    /// Whether `TextItem::words` goes out to JS: always when the caller asked
    /// for word boxes; with `extractBlocks` off, whatever core populated
    /// (stock behavior — byte-identity contract); with `extractBlocks` on and
    /// no request, never — core forces word-box extraction as a
    /// table-detection input, but serializing those unrequested boxes
    /// measurably bloats the `textItems` payload (2.3x on an 80-page text
    /// PDF).
    fn serialize_words(&self) -> bool {
        self.config.emit_word_boxes || !self.config.extract_blocks
    }
}

/// Convert a core [`CoreParseResult`] into the wasm-bindgen view.
///
/// Shared by `parse()` and `ParseSession::nextBatch()` so a batch is mapped
/// exactly the same way a whole-document result is.
///
/// `emit_words` is the caller's own `emitWordBoxes` request: core may have
/// populated `TextItem::words` anyway (markdown output and `extractBlocks`
/// both enable extraction as a table-detection input), but shipping them
/// unrequested is pure payload.
fn to_js_result(
    result: &liteparse::ParseResult,
    extract_text_metadata: bool,
    emit_words: bool,
) -> ParseResult {
    let pages: Vec<ParsedPage> = result
        .pages
        .iter()
        .map(|p| ParsedPage {
            page_num: p.page_number,
            width: p.page_width,
            height: p.page_height,
            content_bounds: p.content_bounds.as_ref().map(|b| VectorRect {
                x: b.x,
                y: b.y,
                width: b.width,
                height: b.height,
            }),
            text: p.text.clone(),
            markdown: p.markdown.clone(),
            text_items: p
                .text_items
                .iter()
                .map(|i| {
                    // Core-gated metadata view; `TextMetadata` defines
                    // which fields `extractTextMetadata` covers.
                    let meta = i.text_metadata(extract_text_metadata);
                    TextItem {
                        text: i.text.clone(),
                        x: i.x,
                        y: i.y,
                        width: i.width,
                        height: i.height,
                        font_name: i.font_name.clone(),
                        font_size: i.font_size,
                        confidence: i.confidence,
                        rotation: i.rotation,
                        font_height: meta.font_height,
                        font_ascent: meta.font_ascent,
                        font_descent: meta.font_descent,
                        font_weight: meta.font_weight,
                        text_width: meta.text_width,
                        font_is_buggy: meta.font_is_buggy,
                        mcid: meta.mcid,
                        fill_color: meta.fill_color.map(str::to_owned),
                        stroke_color: meta.stroke_color.map(str::to_owned),
                        char_codes: meta
                            .char_codes
                            .filter(|codes| !codes.is_empty())
                            .map(<[u32]>::to_vec),
                        trailing_space_generated: meta.trailing_space_generated,
                        words: if !emit_words || i.words.is_empty() {
                            None
                        } else {
                            Some(
                                i.words
                                    .iter()
                                    .map(|w| WordBox {
                                        text: w.text.clone(),
                                        x: w.x,
                                        y: w.y,
                                        width: w.width,
                                        height: w.height,
                                    })
                                    .collect(),
                            )
                        },
                    }
                })
                .collect(),
            complexity: p.complexity.as_ref().map(PageComplexityStats::from_rust),
            vector_graphics: p.vector_graphics.as_ref().map(|v| VectorGraphics {
                shapes: v
                    .shapes
                    .iter()
                    .map(|s| VectorShape {
                        bbox: VectorRect {
                            x: s.bbox.x,
                            y: s.bbox.y,
                            width: s.bbox.width,
                            height: s.bbox.height,
                        },
                        stroke: s.stroke,
                        stroke_color: s.stroke_color.clone(),
                        fill: s.fill,
                        fill_color: s.fill_color.clone(),
                        has_curve: s.has_curve,
                    })
                    .collect(),
                lines: v
                    .lines
                    .iter()
                    .map(|l| VectorLine {
                        x1: l.x1,
                        y1: l.y1,
                        x2: l.x2,
                        y2: l.y2,
                        stroke: l.stroke,
                        stroke_width: l.stroke_width,
                        stroke_color: l.stroke_color.clone(),
                        fill: l.fill,
                        fill_color: l.fill_color.clone(),
                    })
                    .collect(),
            }),
            annotations: p.annotations.as_ref().map(|annotations| {
                annotations
                    .iter()
                    .map(DocumentAnnotation::from_rust)
                    .collect()
            }),
            form_fields: p
                .form_fields
                .as_ref()
                .map(|fields| fields.iter().map(FormField::from_rust).collect()),
            structure_tree: p.structure_tree.as_ref().map(StructureTree::from_rust),
            blocks: p
                .blocks
                .as_ref()
                .map(|blocks| blocks.iter().map(LayoutBlock::from_rust).collect()),
        })
        .collect();

    let images: Vec<ExtractedImage> = result
        .images
        .iter()
        .map(|img| ExtractedImage {
            id: img.id.clone(),
            name: img.name.clone(),
            path: img.path.clone(),
            page: img.page,
            bbox: ImageRect {
                x: img.bbox.x,
                y: img.bbox.y,
                width: img.bbox.width,
                height: img.bbox.height,
            },
            width: img.width,
            height: img.height,
            rotation: img.rotation,
            format: img.format.clone(),
            duplicate_of: img.duplicate_of.clone(),
            bytes: img.bytes.as_slice().to_vec(),
        })
        .collect();

    let screenshots: Vec<ScreenshotResult> = result
        .screenshots
        .iter()
        .map(|shot| ScreenshotResult {
            page_num: shot.page_num,
            width: shot.width,
            height: shot.height,
            image_bytes: shot.image_bytes.clone(),
            is_solid_fill: shot.is_solid_fill,
            rects: shot
                .rects
                .iter()
                .map(|rect| ScreenshotRect {
                    x: rect.x,
                    y: rect.y,
                    width: rect.width,
                    height: rect.height,
                    color: rect.color.clone(),
                    is_line: rect.is_line,
                })
                .collect(),
        })
        .collect();

    ParseResult {
        total_pages: result.total_pages,
        pages,
        text: result.text.clone(),
        images,
        screenshots,
        image_error_count: result.image_error_count,
        page_errors: result
            .page_errors
            .iter()
            .map(|error| PageError {
                page_num: error.page_number,
                message: error.message.clone(),
            })
            .collect(),
        form_type: result.form_type,
        creator: result.creator.clone(),
        producer: result.producer.clone(),
        doc_meta: result.doc_meta.as_ref().map(DocumentMetadata::from),
        xfa_packets: result.xfa_packets.as_ref().map(|packets| {
            packets
                .iter()
                .map(|packet| XfaPacket {
                    index: packet.index,
                    name: packet.name.clone(),
                    content_length: packet.content_length,
                    content: packet.content.clone(),
                })
                .collect()
        }),
    }
}

#[wasm_bindgen]
impl LiteParse {
    /// Determine per-page complexity for the given PDF bytes. Returns
    /// `Promise<PageComplexityStats[]>` — a cheap pre-OCR check with per-page
    /// signals and a `needsOcr` verdict.
    #[wasm_bindgen(js_name = isComplex)]
    pub async fn is_complex(&self, data: Vec<u8>) -> Result<Vec<PageComplexityStats>, JsError> {
        let stats = self
            .inner
            .is_complex(PdfInput::Bytes(data))
            .await
            .map_err(|e| JsError::new(&format!("is_complex failed: {}", e)))?;

        Ok(stats.iter().map(PageComplexityStats::from_rust).collect())
    }

    /// Open PDF bytes for bounded-memory batch parsing. Returns
    /// `Promise<ParseSession>`.
    ///
    /// Yields `batchSize` pages at a time (default 25), which bounds both the
    /// Rust-side result and the JS objects built from it — the wasm heap has a
    /// hard ceiling, so a large document parsed whole can exhaust it.
    ///
    /// Cross-page passes (repeated header/footer removal, image deduplication)
    /// see only the pages in their own batch, so output can differ from a
    /// whole-document `parse()`.
    #[wasm_bindgen(js_name = openBatchSession)]
    pub async fn open_batch_session(
        &self,
        data: Vec<u8>,
        batch_size: Option<usize>,
    ) -> Result<ParseSession, JsError> {
        let batch_size = batch_size.unwrap_or(liteparse::DEFAULT_PAGE_BATCH_SIZE);
        let session = self
            .inner
            .open_batch_session(PdfInput::Bytes(data), batch_size)
            .await
            .map_err(|e| JsError::new(&format!("open failed: {}", e)))?;

        Ok(ParseSession {
            extract_text_metadata: self.inner.config().extract_text_metadata,
            serialize_words: self.serialize_words(),
            inner: session,
        })
    }
}

/// One batch of pages from a [`ParseSession`].
#[derive(Serialize, Tsify)]
#[tsify(into_wasm_abi)]
#[serde(rename_all = "camelCase")]
pub struct ParseBatch {
    /// First source page in this batch, 1-indexed.
    pub start_page: u32,
    /// Last source page in this batch, 1-indexed and inclusive.
    pub end_page: u32,
    /// The pages in `startPage..=endPage`, as an ordinary parse result.
    pub result: ParseResult,
}

/// A document opened once and parsed in bounded page batches.
///
/// Created by `LiteParse.openBatchSession()`. Call `nextBatch()` until it
/// returns `undefined`, then `free()` the session to release its wasm-side
/// memory promptly (wasm-bindgen objects are not garbage collected).
#[wasm_bindgen]
pub struct ParseSession {
    inner: liteparse::ParseSession,
    extract_text_metadata: bool,
    /// See [`LiteParse::serialize_words`] — resolved once at session open.
    serialize_words: bool,
}

#[wasm_bindgen]
impl ParseSession {
    /// Total pages in the source document, before `maxPages` or batching.
    #[wasm_bindgen(getter, js_name = totalPages)]
    pub fn total_pages(&self) -> u32 {
        self.inner.total_pages()
    }

    /// Parse and return the next batch, or `undefined` once every page within
    /// `maxPages` has been yielded.
    ///
    /// Batches are parsed one at a time: await each call before making the
    /// next (a concurrent call throws wasm-bindgen's recursive-borrow error).
    #[wasm_bindgen(js_name = nextBatch)]
    pub async fn next_batch(&mut self) -> Result<Option<ParseBatch>, JsError> {
        let batch = self
            .inner
            .next_batch()
            .await
            .map_err(|e| JsError::new(&format!("batch parse failed: {}", e)))?;

        Ok(batch.map(|batch| ParseBatch {
            start_page: batch.start_page,
            end_page: batch.end_page,
            result: to_js_result(&batch.result, self.extract_text_metadata, self.serialize_words),
        }))
    }
}

#[derive(Serialize, Tsify)]
#[tsify(into_wasm_abi)]
#[serde(rename_all = "camelCase")]
pub struct LayoutComplexityStats {
    column_count: usize,
    ruled_table_count: usize,
    ruled_table_coverage: f32,
    text_table_run_count: usize,
    figure_count: usize,
    figure_coverage: f32,
    is_complex: bool,
    reasons: Vec<String>,
}

impl LayoutComplexityStats {
    fn from_rust(stats: &liteparse::ocr_merge::LayoutComplexityStats) -> Self {
        Self {
            column_count: stats.column_count,
            ruled_table_count: stats.ruled_table_count,
            ruled_table_coverage: stats.ruled_table_coverage,
            text_table_run_count: stats.text_table_run_count,
            figure_count: stats.figure_count,
            figure_coverage: stats.figure_coverage,
            is_complex: stats.is_complex,
            reasons: stats
                .reasons
                .iter()
                .map(|r| r.as_str().to_string())
                .collect(),
        }
    }
}

#[derive(Serialize, Tsify)]
#[tsify(into_wasm_abi)]
#[serde(rename_all = "camelCase")]
pub struct PageComplexityStats {
    page_number: usize,
    text_length: usize,
    text_coverage: f32,
    has_substantial_images: bool,
    /// Number of counted raster images — inline figures only; full-page
    /// backgrounds are excluded (see `fullPageImage`).
    image_block_count: usize,
    /// Summed image-bbox area over page area, clamped to 1. Counts inline
    /// figures only: a full-page scan raster contributes 0 here — check
    /// `fullPageImage` for that.
    image_coverage: f32,
    /// Largest single counted image's area over page area, clamped to 1. Same
    /// exclusion as `imageCoverage`: a full-page raster contributes 0.
    largest_image_coverage: f32,
    /// A single raster covering ≥90% of the page is present. Such full-page
    /// backgrounds are excluded from `imageCoverage`/`largestImageCoverage`
    /// (they're not inline figures), so this flag is the only signal that
    /// distinguishes a scan from a genuinely blank page — both otherwise
    /// report no text and no counted images.
    full_page_image: bool,
    uncovered_vector_area: Option<f32>,
    is_garbled: bool,
    page_area: f32,
    needs_ocr: bool,
    reasons: Vec<String>,
    layout: Option<LayoutComplexityStats>,
}

impl PageComplexityStats {
    fn from_rust(stats: &liteparse::ocr_merge::PageComplexityStats) -> Self {
        Self {
            page_number: stats.page_number,
            text_length: stats.text_length,
            text_coverage: stats.text_coverage,
            has_substantial_images: stats.has_substantial_images,
            image_block_count: stats.image_block_count,
            image_coverage: stats.image_coverage,
            largest_image_coverage: stats.largest_image_coverage,
            full_page_image: stats.full_page_image,
            uncovered_vector_area: stats.uncovered_vector_area,
            is_garbled: stats.is_garbled,
            page_area: stats.page_area,
            needs_ocr: stats.needs_ocr,
            reasons: stats
                .reasons
                .iter()
                .map(|r| r.as_str().to_string())
                .collect(),
            layout: stats.layout.as_ref().map(LayoutComplexityStats::from_rust),
        }
    }
}

// ---------------------------------------------------------------------------
// searchItems (standalone function)
// ---------------------------------------------------------------------------

#[derive(Deserialize, Tsify)]
#[tsify(from_wasm_abi)]
#[serde(rename_all = "camelCase")]
pub struct SearchTextItem {
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    #[serde(default)]
    pub font_name: Option<String>,
    #[serde(default)]
    pub font_size: Option<f32>,
    #[serde(default)]
    pub confidence: Option<f32>,
}

#[derive(Deserialize, Tsify)]
#[tsify(from_wasm_abi)]
#[serde(rename_all = "camelCase", default)]
pub struct SearchOptions {
    pub phrase: String,
    pub case_sensitive: bool,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            phrase: String::new(),
            case_sensitive: false,
        }
    }
}

/// Search text items for phrase matches, returning merged items with combined bounding boxes.
#[wasm_bindgen(js_name = "searchItems")]
pub fn search_items(items: Vec<SearchTextItem>, options: SearchOptions) -> Vec<TextItem> {
    let rust_items: Vec<liteparse::types::TextItem> = items
        .into_iter()
        .map(|i| liteparse::types::TextItem {
            text: i.text,
            x: i.x,
            y: i.y,
            width: i.width,
            height: i.height,
            font_name: i.font_name,
            font_size: i.font_size,
            confidence: i.confidence,
            ..Default::default()
        })
        .collect();

    let options = search::SearchOptions {
        phrase: options.phrase,
        case_sensitive: options.case_sensitive,
    };

    let results = search::search_items(&rust_items, &options);
    results
        .iter()
        .map(|i| TextItem {
            text: i.text.clone(),
            x: i.x,
            y: i.y,
            width: i.width,
            height: i.height,
            font_name: i.font_name.clone(),
            font_size: i.font_size,
            confidence: i.confidence,
            rotation: i.rotation,
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
            words: if i.words.is_empty() {
                None
            } else {
                Some(
                    i.words
                        .iter()
                        .map(|w| WordBox {
                            text: w.text.clone(),
                            x: w.x,
                            y: w.y,
                            width: w.width,
                            height: w.height,
                        })
                        .collect(),
                )
            },
        })
        .collect()
}
