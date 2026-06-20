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
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

use liteparse::config::{ImageMode, LiteParseConfig, OutputFormat};
use liteparse::ocr::{OcrEngine, OcrOptions, OcrResult};
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct JsLiteParseConfig {
    ocr_language: Option<String>,
    ocr_enabled: Option<bool>,
    ocr_server_url: Option<String>,
    ocr_server_headers: Option<HashMap<String, String>>,
    tessdata_path: Option<String>,
    max_pages: Option<usize>,
    target_pages: Option<String>,
    dpi: Option<f32>,
    output_format: Option<String>,
    image_mode: Option<String>,
    inline_images: Option<bool>,
    extract_links: Option<bool>,
    preserve_very_small_text: Option<bool>,
    password: Option<String>,
    quiet: Option<bool>,
}

impl JsLiteParseConfig {
    fn into_core(self) -> Result<LiteParseConfig, JsError> {
        let mut cfg = LiteParseConfig::default();
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
        if let Some(v) = self.inline_images {
            cfg.inline_images = v;
        }
        if let Some(v) = self.extract_links {
            cfg.extract_links = v;
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
        cfg.num_workers = 1;
        Ok(cfg)
    }

    fn from_core(cfg: &LiteParseConfig) -> Self {
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
            inline_images: Some(cfg.inline_images),
            extract_links: Some(cfg.extract_links),
            preserve_very_small_text: Some(cfg.preserve_very_small_text),
            password: cfg.password.clone(),
            quiet: Some(cfg.quiet),
        }
    }
}

// ---------------------------------------------------------------------------
// JS-facing parse result
// ---------------------------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsTextItem<'a> {
    text: &'a str,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    font_name: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    font_size: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    confidence: Option<f32>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsParsedPage<'a> {
    page_num: usize,
    width: f32,
    height: f32,
    text: &'a str,
    text_items: Vec<JsTextItem<'a>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsParseResult<'a> {
    pages: Vec<JsParsedPage<'a>>,
    text: &'a str,
    images: Vec<JsExtractedImage<'a>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsExtractedImage<'a> {
    id: &'a str,
    page: u32,
    format: &'a str,
    /// Serialized as a JS `number[]`. Callers that want a Uint8Array can
    /// wrap with `new Uint8Array(image.bytes)`. (Could be upgraded to a real
    /// Uint8Array later by switching to a hand-rolled to_value path.)
    bytes: &'a [u8],
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
            dyn Future<Output = Result<Vec<OcrResult>, Box<dyn std::error::Error + Send + Sync>>>
                + '_,
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

            let parsed: Vec<JsOcrResult> = serde_wasm_bindgen::from_value(resolved)
                .map_err(|e| format!("ocrEngine.recognize result decode failed: {:?}", e))?;

            Ok(parsed
                .into_iter()
                .map(|r| OcrResult {
                    text: r.text,
                    bbox: r.bbox,
                    confidence: r.confidence,
                    polygon: r.polygon,
                })
                .collect())
        })
    }
}

#[derive(Deserialize)]
struct JsOcrResult {
    text: String,
    bbox: [f32; 4],
    confidence: f32,
    #[serde(default)]
    polygon: Option<[[f32; 2]; 4]>,
}

// ---------------------------------------------------------------------------
// LiteParse class (JS-facing)
// ---------------------------------------------------------------------------

#[wasm_bindgen]
pub struct LiteParse {
    inner: CoreLiteParse,
    config: LiteParseConfig,
}

#[wasm_bindgen]
impl LiteParse {
    /// Construct a new parser. `config` is a JS object (all fields optional).
    /// If `config.ocrEngine` is present, it is wired up as the OCR backend.
    #[wasm_bindgen(constructor)]
    pub fn new(config: JsValue) -> Result<LiteParse, JsError> {
        let ocr_engine_js = if config.is_object() {
            Reflect::get(&config, &JsValue::from_str("ocrEngine"))
                .ok()
                .filter(|v| !v.is_undefined() && !v.is_null())
        } else {
            None
        };

        let js_cfg: JsLiteParseConfig = if config.is_undefined() || config.is_null() {
            JsLiteParseConfig::default()
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
    pub fn config(&self) -> Result<JsValue, JsError> {
        let cfg = JsLiteParseConfig::from_core(&self.config);
        serde_wasm_bindgen::to_value(&cfg)
            .map_err(|e| JsError::new(&format!("serialize config failed: {}", e)))
    }

    /// Parse PDF bytes. Returns `Promise<ParseResult>`.
    pub async fn parse(&self, data: Vec<u8>) -> Result<JsValue, JsError> {
        let result = self
            .inner
            .parse_input(PdfInput::Bytes(data))
            .await
            .map_err(|e| JsError::new(&format!("parse failed: {}", e)))?;

        let js_pages: Vec<JsParsedPage<'_>> = result
            .pages
            .iter()
            .map(|p| JsParsedPage {
                page_num: p.page_number,
                width: p.page_width,
                height: p.page_height,
                text: &p.text,
                text_items: p
                    .text_items
                    .iter()
                    .map(|i| JsTextItem {
                        text: &i.text,
                        x: i.x,
                        y: i.y,
                        width: i.width,
                        height: i.height,
                        font_name: i.font_name.as_deref(),
                        font_size: i.font_size,
                        confidence: i.confidence,
                    })
                    .collect(),
            })
            .collect();

        let js_images: Vec<JsExtractedImage> = result
            .images
            .iter()
            .map(|img| JsExtractedImage {
                id: &img.id,
                page: img.page,
                format: &img.format,
                bytes: &img.bytes,
            })
            .collect();
        let js_result = JsParseResult {
            pages: js_pages,
            text: &result.text,
            images: js_images,
        };

        serde_wasm_bindgen::to_value(&js_result)
            .map_err(|e| JsError::new(&format!("serialize result failed: {}", e)))
    }
}

// ---------------------------------------------------------------------------
// searchItems (standalone function)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct JsSearchTextItem {
    text: String,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    #[serde(default)]
    font_name: Option<String>,
    #[serde(default)]
    font_size: Option<f32>,
    #[serde(default)]
    confidence: Option<f32>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct JsSearchOptions {
    phrase: String,
    case_sensitive: bool,
}

impl Default for JsSearchOptions {
    fn default() -> Self {
        Self {
            phrase: String::new(),
            case_sensitive: false,
        }
    }
}

/// Search text items for phrase matches, returning merged items with combined bounding boxes.
#[wasm_bindgen(js_name = "searchItems")]
pub fn search_items(items: JsValue, options: JsValue) -> Result<JsValue, JsError> {
    let js_items: Vec<JsSearchTextItem> = serde_wasm_bindgen::from_value(items)
        .map_err(|e| JsError::new(&format!("invalid items: {}", e)))?;
    let js_opts: JsSearchOptions = serde_wasm_bindgen::from_value(options)
        .map_err(|e| JsError::new(&format!("invalid options: {}", e)))?;

    let rust_items: Vec<liteparse::types::TextItem> = js_items
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
        phrase: js_opts.phrase,
        case_sensitive: js_opts.case_sensitive,
    };

    let results = search::search_items(&rust_items, &options);
    let js_results: Vec<JsTextItem<'_>> = results
        .iter()
        .map(|i| JsTextItem {
            text: &i.text,
            x: i.x,
            y: i.y,
            width: i.width,
            height: i.height,
            font_name: i.font_name.as_deref(),
            font_size: i.font_size,
            confidence: i.confidence,
        })
        .collect();

    serde_wasm_bindgen::to_value(&js_results)
        .map_err(|e| JsError::new(&format!("serialize results failed: {}", e)))
}
