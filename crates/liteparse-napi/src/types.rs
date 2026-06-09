use napi_derive::napi;

use liteparse::config::{LiteParseConfig, OutputFormat};
use liteparse::parser::ParseResult;
use liteparse::types::{ChartItem, ChartType, ImageItem, ParsedPage, TextItem};

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
    /// Path to tessdata directory for Tesseract.
    pub tessdata_path: Option<String>,
    /// Maximum number of pages to parse.
    pub max_pages: Option<u32>,
    /// Specific pages to parse (e.g., "1-5,10,15-20").
    pub target_pages: Option<String>,
    /// DPI for rendering pages (used for OCR and screenshots).
    pub dpi: Option<f64>,
    /// Output format: "json" or "text".
    pub output_format: Option<String>,
    /// Keep very small text that would normally be filtered out.
    pub preserve_very_small_text: Option<bool>,
    /// Password for encrypted/protected documents.
    pub password: Option<String>,
    /// Suppress progress output.
    pub quiet: Option<bool>,
    /// Number of concurrent OCR workers (default: CPU cores - 1).
    pub num_workers: Option<u32>,
    /// Inline embedded PDF images into text as base64 markdown data URI images.
    pub inline_images: Option<bool>,
    /// Detect supported vector charts and render them as ASCII charts.
    pub detect_charts: Option<bool>,
    /// DPI for chart detection rendering.
    pub chart_detection_dpi: Option<f64>,
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
        if let Some(v) = self.tessdata_path {
            cfg.tessdata_path = Some(v);
        }
        if let Some(v) = self.max_pages {
            cfg.max_pages = v as usize;
        }
        if let Some(v) = self.target_pages {
            cfg.target_pages = Some(v);
        }
        if let Some(v) = self.dpi {
            cfg.dpi = v as f32;
        }
        if let Some(v) = self.output_format {
            cfg.output_format = match v.as_str() {
                "text" => OutputFormat::Text,
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
        if let Some(v) = self.inline_images {
            cfg.inline_images = v;
        }
        if let Some(v) = self.detect_charts {
            cfg.detect_charts = v;
        }
        if let Some(v) = self.chart_detection_dpi {
            cfg.chart_detection_dpi = v as f32;
        }
        cfg
    }

    pub fn from_rust(cfg: &LiteParseConfig) -> Self {
        Self {
            ocr_language: Some(cfg.ocr_language.clone()),
            ocr_enabled: Some(cfg.ocr_enabled),
            ocr_server_url: cfg.ocr_server_url.clone(),
            tessdata_path: cfg.tessdata_path.clone(),
            max_pages: Some(cfg.max_pages as u32),
            target_pages: cfg.target_pages.clone(),
            dpi: Some(cfg.dpi as f64),
            output_format: Some(match cfg.output_format {
                OutputFormat::Json => "json".to_string(),
                OutputFormat::Text => "text".to_string(),
            }),
            preserve_very_small_text: Some(cfg.preserve_very_small_text),
            password: cfg.password.clone(),
            quiet: Some(cfg.quiet),
            num_workers: Some(cfg.num_workers as u32),
            inline_images: Some(cfg.inline_images),
            detect_charts: Some(cfg.detect_charts),
            chart_detection_dpi: Some(cfg.chart_detection_dpi as f64),
        }
    }
}

// ---------------------------------------------------------------------------
// TextItem
// ---------------------------------------------------------------------------

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
    pub confidence: Option<f64>,
}

impl JsTextItem {
    pub fn to_rust(&self) -> TextItem {
        TextItem {
            text: self.text.clone(),
            x: self.x as f32,
            y: self.y as f32,
            width: self.width as f32,
            height: self.height as f32,
            font_name: self.font_name.clone(),
            font_size: self.font_size.map(|v| v as f32),
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
            font_name: item.font_name.clone(),
            font_size: item.font_size.map(|v| v as f64),
            confidence: item.confidence.map(|v| v as f64).or(Some(1.0)),
        }
    }
}

// ---------------------------------------------------------------------------
// ImageItem
// ---------------------------------------------------------------------------

#[napi(object)]
#[derive(Clone)]
pub struct JsImageItem {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub mime_type: String,
    pub base64: String,
}

impl JsImageItem {
    pub fn from_rust(item: &ImageItem) -> Self {
        Self {
            x: item.x as f64,
            y: item.y as f64,
            width: item.width as f64,
            height: item.height as f64,
            mime_type: item.mime_type.clone(),
            base64: item.base64.clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// ChartItem
// ---------------------------------------------------------------------------

#[napi(object)]
#[derive(Clone)]
pub struct JsChartItem {
    pub chart_type: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub ascii: String,
    pub series: Vec<String>,
    pub groups: Vec<String>,
    pub values: Vec<Vec<f64>>,
}

impl JsChartItem {
    pub fn from_rust(item: &ChartItem) -> Self {
        Self {
            chart_type: match &item.chart_type {
                ChartType::Bar => "bar",
                ChartType::Line => "line",
                ChartType::Pie => "pie",
                ChartType::Radar => "radar",
            }
            .to_string(),
            x: item.x as f64,
            y: item.y as f64,
            width: item.width as f64,
            height: item.height as f64,
            ascii: item.ascii.clone(),
            series: item.series.clone(),
            groups: item.groups.clone(),
            values: item
                .values
                .iter()
                .map(|row| row.iter().map(|v| *v as f64).collect())
                .collect(),
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
    pub text: String,
    pub text_items: Vec<JsTextItem>,
    pub images: Vec<JsImageItem>,
    pub charts: Vec<JsChartItem>,
}

impl JsParsedPage {
    pub fn from_rust(page: &ParsedPage) -> Self {
        Self {
            page_num: page.page_number as u32,
            width: page.page_width as f64,
            height: page.page_height as f64,
            text: page.text.clone(),
            text_items: page.text_items.iter().map(JsTextItem::from_rust).collect(),
            images: page.images.iter().map(JsImageItem::from_rust).collect(),
            charts: page.charts.iter().map(JsChartItem::from_rust).collect(),
        }
    }
}

// ---------------------------------------------------------------------------
// ParseResult
// ---------------------------------------------------------------------------

#[napi(object)]
#[derive(Clone)]
pub struct JsParseResult {
    pub pages: Vec<JsParsedPage>,
    pub text: String,
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
}

impl JsParseResult {
    pub fn from_rust(result: &ParseResult, _config: &LiteParseConfig) -> Self {
        Self {
            pages: result.pages.iter().map(JsParsedPage::from_rust).collect(),
            text: result.text.clone(),
        }
    }
}
