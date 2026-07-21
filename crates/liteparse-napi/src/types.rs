use std::collections::HashMap;

use napi_derive::napi;

use liteparse::config::{CropBox, ImageMode, LiteParseConfig, OutputFormat};
use liteparse::parser::ParseResult;
use liteparse::types::{GraphicPrimitive, Page, ParsedPage, Rect, TextItem, WordBox};

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
    /// (default — emits `![](image_pN_K.png)` references with no bytes), or
    /// "embed" (also returns each image's PNG bytes on `images`).
    pub image_mode: Option<String>,
    /// Render hyperlink annotations as `[text](url)` in markdown output
    /// (default true). Set false for plain anchor text.
    pub extract_links: Option<bool>,
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
    /// for word-level bbox attribution.
    pub emit_word_boxes: Option<bool>,
    /// Include rich PDF text metadata on returned text items. Default false.
    pub include_text_metadata: Option<bool>,
    /// Restrict output to a page sub-region. Each field is the fraction of the
    /// page cropped from that side; a text item survives only if it lies
    /// entirely inside the remaining rectangle. Unset keeps the whole page.
    pub crop_box: Option<JsCropBox>,
    /// Drop diagonal text (rotation >2° off the nearest right angle). Default
    /// false. Use to exclude rotated watermarks/stamps from the output.
    pub skip_diagonal_text: Option<bool>,
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
        if let Some(v) = self.extract_links {
            cfg.extract_links = v;
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
        if let Some(v) = self.include_text_metadata {
            cfg.include_text_metadata = v;
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
            extract_links: Some(cfg.extract_links),
            ocr_failure_fatal: Some(cfg.ocr_failure_fatal),
            ocr_hedge_delays_ms: Some(
                cfg.ocr_hedge_delays_ms
                    .iter()
                    .map(|&v| u32::try_from(v).unwrap_or(u32::MAX))
                    .collect(),
            ),
            emit_word_boxes: Some(cfg.emit_word_boxes),
            include_text_metadata: Some(cfg.include_text_metadata),
            crop_box: cfg.crop_box.map(|c| JsCropBox {
                top: c.top as f64,
                right: c.right as f64,
                bottom: c.bottom as f64,
                left: c.left as f64,
            }),
            skip_diagonal_text: Some(cfg.skip_diagonal_text),
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
    pub tsg: Option<bool>,
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
            tsg: self.tsg.unwrap_or(false),
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
            tsg: Some(item.tsg),
            confidence: item.confidence.map(|v| v as f64).or(Some(1.0)),
            words: item.words.iter().map(JsWordBox::from_rust).collect(),
        }
    }

    fn from_rust_for_output(item: &TextItem, include_text_metadata: bool) -> Self {
        let mut output = Self::from_rust(item);
        if !include_text_metadata {
            output.font_height = None;
            output.font_ascent = None;
            output.font_descent = None;
            output.font_weight = None;
            output.text_width = None;
            output.font_is_buggy = None;
            output.mcid = None;
            output.fill_color = None;
            output.stroke_color = None;
            output.char_codes = None;
            output.tsg = None;
        }
        output
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
            text_items: self.text_items.iter().map(JsTextItem::to_rust).collect(),
            graphics: self
                .graphics
                .as_ref()
                .map(|gs| gs.iter().filter_map(JsGraphic::to_rust).collect())
                .unwrap_or_default(),
            struct_nodes: Vec::new(),
            image_refs: Vec::new(),
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
    pub markdown: String,
    pub text_items: Vec<JsTextItem>,
}

impl JsParsedPage {
    pub fn from_rust(page: &ParsedPage, include_text_metadata: bool) -> Self {
        Self {
            page_num: page.page_number as u32,
            width: page.page_width as f64,
            height: page.page_height as f64,
            text: page.text.clone(),
            markdown: page.markdown.clone(),
            text_items: page
                .text_items
                .iter()
                .map(|item| JsTextItem::from_rust_for_output(item, include_text_metadata))
                .collect(),
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
    pub images: Vec<JsExtractedImage>,
}

#[napi(object)]
#[derive(Clone)]
pub struct JsExtractedImage {
    pub id: String,
    pub page: u32,
    pub format: String,
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
}

#[napi(object)]
#[derive(Clone)]
pub struct JsPageComplexityStats {
    pub page_number: u32,
    pub text_length: u32,
    pub text_coverage: f64,
    pub has_substantial_images: bool,
    pub image_block_count: u32,
    pub image_coverage: f64,
    pub largest_image_coverage: f64,
    pub full_page_image: bool,
    pub uncovered_vector_area: Option<f64>,
    pub is_garbled: bool,
    pub page_area: f64,
    pub needs_ocr: bool,
    pub reasons: Vec<String>,
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
        }
    }
}

impl JsParseResult {
    pub fn from_rust(result: &ParseResult, config: &LiteParseConfig) -> Self {
        Self {
            pages: result
                .pages
                .iter()
                .map(|page| JsParsedPage::from_rust(page, config.include_text_metadata))
                .collect(),
            text: result.text.clone(),
            images: result
                .images
                .iter()
                .map(|img| JsExtractedImage {
                    id: img.id.clone(),
                    page: img.page,
                    format: img.format.clone(),
                    bytes: img.bytes.clone().into(),
                })
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
            tsg: true,
            ..Default::default()
        };

        let js = JsTextItem::from_rust(&item);
        assert_eq!(js.char_codes, Some(vec![65, 32]));
        assert_eq!(js.tsg, Some(true));
        assert_eq!(js.fill_color.as_deref(), Some("ff112233"));

        let lightweight = JsTextItem::from_rust_for_output(&item, false);
        assert_eq!(lightweight.font_height, None);
        assert_eq!(lightweight.font_is_buggy, None);
        assert_eq!(lightweight.char_codes, None);
        assert_eq!(lightweight.tsg, None);

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
        assert!(round_trip.tsg);
    }

    #[test]
    fn text_metadata_config_defaults_off_and_round_trips() {
        let mut js = JsLiteParseConfig::from_rust(&LiteParseConfig::default());
        assert_eq!(js.include_text_metadata, Some(false));
        js.include_text_metadata = Some(true);
        assert!(js.into_rust().include_text_metadata);
    }
}
