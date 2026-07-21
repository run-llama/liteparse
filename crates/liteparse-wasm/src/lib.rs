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
    dpi: Option<f32>,
    #[tsify(type = "\"json\" | \"text\" | \"markdown\" | \"md\"")]
    output_format: Option<String>,
    #[tsify(type = "\"off\" | \"none\" | \"placeholder\" | \"embed\"")]
    image_mode: Option<String>,
    extract_links: Option<bool>,
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
        if let Some(v) = self.extract_links {
            cfg.extract_links = v;
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
        cfg.num_workers = 1;
        Ok(cfg)
    }

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
            extract_links: Some(cfg.extract_links),
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
    pub text: String,
    pub markdown: String,
    pub text_items: Vec<TextItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub complexity: Option<PageComplexityStats>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vector_graphics: Option<VectorGraphics>,
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
pub struct ParseResult {
    pub pages: Vec<ParsedPage>,
    pub text: String,
    pub images: Vec<ExtractedImage>,
}

#[derive(Serialize, Tsify)]
#[tsify(into_wasm_abi)]
#[serde(rename_all = "camelCase")]
pub struct ExtractedImage {
    pub id: String,
    pub page: u32,
    pub format: String,
    /// Raw image bytes, serialized as a JS `number[]`. Callers that want a
    /// Uint8Array can wrap with `new Uint8Array(image.bytes)`.
    pub bytes: Vec<u8>,
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

        let pages: Vec<ParsedPage> = result
            .pages
            .iter()
            .map(|p| ParsedPage {
                page_num: p.page_number,
                width: p.page_width,
                height: p.page_height,
                text: p.text.clone(),
                markdown: p.markdown.clone(),
                text_items: p
                    .text_items
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
            })
            .collect();

        let images: Vec<ExtractedImage> = result
            .images
            .iter()
            .map(|img| ExtractedImage {
                id: img.id.clone(),
                page: img.page,
                format: img.format.clone(),
                bytes: img.bytes.clone(),
            })
            .collect();

        Ok(ParseResult {
            pages,
            text: result.text.clone(),
            images,
        })
    }

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
    image_block_count: usize,
    image_coverage: f32,
    largest_image_coverage: f32,
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
