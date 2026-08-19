use serde::{Deserialize, Serialize};

/// Configuration for LiteParse document parsing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiteParseConfig {
    /// OCR language code (Tesseract format: "eng", "fra", "deu", etc.).
    pub ocr_language: String,
    /// Whether OCR is enabled. When true, runs on text-sparse pages and embedded images.
    pub ocr_enabled: bool,
    /// HTTP OCR server URL (uses Tesseract if not provided)
    pub ocr_server_url: Option<String>,
    /// Extra HTTP headers sent with every request to `ocr_server_url`, as
    /// `(name, value)` pairs. Use for auth, e.g. `("Authorization", "Bearer …")`.
    /// Ignored when `ocr_server_url` is None.
    pub ocr_server_headers: Vec<(String, String)>,
    /// Path to tessdata directory. Falls back to TESSDATA_PREFIX env var if not set.
    pub tessdata_path: Option<String>,
    /// Maximum number of pages to parse.
    pub max_pages: usize,
    /// Specific pages to parse (e.g., "1-5,10,15-20"). None means all pages.
    pub target_pages: Option<String>,
    /// Render parsed pages and return them in `ParseResult.screenshots`.
    /// Default `false`; screenshot payloads can be large.
    #[serde(default)]
    pub extract_screenshots: bool,
    /// Screenshot output and format-specific encoder options.
    #[serde(default)]
    pub screenshot: ScreenshotOptions,
    /// Continue parsing after a page-level PDFium extraction failure and
    /// report it in `ParseResult.page_errors`. Default `false` preserves the
    /// fail-fast behavior. Document-open and document-level failures remain
    /// fatal regardless of this setting.
    #[serde(default)]
    pub continue_on_page_error: bool,
    /// DPI for rendering pages (used for OCR and screenshots).
    pub dpi: f32,
    /// Output format.
    pub output_format: OutputFormat,
    /// Keep very small text that would normally be filtered out.
    pub preserve_very_small_text: bool,
    /// Password for encrypted/protected documents.
    pub password: Option<String>,
    /// Suppress progress output.
    pub quiet: bool,
    /// Number of concurrent OCR workers. Defaults to (number of CPU cores - 1), minimum 1.
    pub num_workers: usize,
    /// Controls how raster image references are surfaced in markdown output.
    /// This does not enable embedded-image extraction.
    pub image_mode: ImageMode,
    /// Extract embedded image bytes and metadata into `ParseResult.images`.
    /// Defaults to false. `ImageMode::Embed` also enables extraction for
    /// backwards compatibility (see [`LiteParseConfig::effective_extract_images`]).
    pub extract_images: bool,
    /// Directory where extracted embedded images are written. Requires
    /// `extract_images` to be true.
    pub image_output_dir: Option<String>,
    /// Extract hyperlink annotations and render them as `[text](url)` in
    /// markdown output. Default on. Disable for benchmark parity with
    /// plain-text ground truth (the GT corpora never use link syntax).
    pub extract_links: bool,
    /// Keep running header/footer chrome in markdown output instead of
    /// stripping it. By default the markdown renderer removes lines that
    /// repeat in the top/bottom page bands across pages (and single-page
    /// chrome like `Page N of M`). Set `true` to retain everything —
    /// useful for extraction pipelines that would rather deduplicate
    /// downstream than risk losing content. Only affects Markdown output.
    #[serde(default)]
    pub keep_headers_footers: bool,
    /// Extract all PDF annotations into each parsed page. Default `false`.
    /// This is independent of `extract_links`, which only controls Markdown
    /// link reconstruction.
    pub extract_annotations: bool,
    /// Extract AcroForm widget fields and values. Default `false`.
    #[serde(default)]
    pub extract_form_fields: bool,
    /// Extract the tagged-PDF logical structure tree. Default `false`.
    #[serde(default)]
    pub extract_structure_tree: bool,
    /// Emit each page's classified layout blocks (headings, paragraphs, list
    /// items, tables with per-cell boxes, code, rules, figures) with bounding
    /// boxes. Default `false`.
    ///
    /// This is the same decomposition the Markdown renderer consumes, exposed
    /// as data. It is independent of `output_format`: blocks are available in
    /// JSON and text modes too, and enabling it never changes the rendered
    /// Markdown.
    #[serde(default)]
    pub extract_blocks: bool,
    /// Emit each page's `content_bounds`: the union bbox of its top-level
    /// content objects in viewport coords (visible content extent). Default
    /// `false` to keep the default output shape unchanged. Content bounds are
    /// still computed internally for the white-fill heuristic whenever
    /// `extract_vector_graphics` is on.
    #[serde(default)]
    pub extract_content_bounds: bool,
    /// Extract raw XFA packets (name + XML content) from XFA form documents
    /// into `ParseResult.xfa_packets`. Default `false`. Non-XFA documents
    /// yield an empty list.
    #[serde(default)]
    pub extract_xfa_packets: bool,
    /// Collect document-level provenance metadata into `ParseResult.doc_meta`.
    /// Default `false`: `None` for inputs converted from a non-PDF format,
    /// since the facts would describe the intermediate PDF rather than the caller's file.
    #[serde(default)]
    pub extract_document_metadata: bool,
    /// Detect solid rectangles and thick lines in rendered page screenshots
    /// and attach them to each `ScreenshotResult.rects`. Works on the raster,
    /// so it also finds structure in scanned/flattened pages that have no
    /// vector paths. Default `false` (adds a full-bitmap scan per page).
    #[serde(default)]
    pub detect_screenshot_rects: bool,
    /// Draw AcroForm field appearances (filled values, checkbox states) into
    /// rendered rasters (screenshots and OCR inputs). This initializes a
    /// PDFium form-fill environment and runs the document's open/JS actions,
    /// so it is off by default: plain parses should neither execute document
    /// scripts nor change raster bytes for form-bearing PDFs.
    #[serde(default)]
    pub render_form_fields: bool,
    /// Whether a systemic OCR failure (every OCR task failed *and* at least one
    /// was a text-sparse page whose primary text source was OCR) aborts the
    /// whole parse. Default `true`: surface the root cause instead of silently
    /// emitting blank pages. Set `false` to keep already-recovered native text
    /// and return partial results when OCR is unavailable — useful for callers
    /// that prefer a degraded document over a hard failure (e.g. when the host
    /// has its own OCR fallback or treats OCR as best-effort enrichment).
    pub ocr_failure_fatal: bool,
    /// OCR request-hedging schedule (milliseconds) for the HTTP OCR engine.
    /// Empty (default) = no hedging. With multiple delays (e.g.
    /// `[0, 5000, 10000, 15000, 20000]`), each OCR attempt fires a duplicate
    /// request at every delay and takes the first to succeed — trading extra
    /// OCR-server load for lower tail latency on a slow/stuck pod. No effect on
    /// the Tesseract engine.
    pub ocr_hedge_delays_ms: Vec<u64>,
    /// Emit per-word sub-boxes on each `TextItem` (`TextItem.words`). Default
    /// `false`: a text item already carries its own box, and word boxes roughly
    /// double the text-item payload (size + napi marshalling), so they are only
    /// worth computing for callers doing word-level bbox attribution. When
    /// `false`, `TextItem.words` is always empty and the per-word tracking is
    /// skipped entirely (zero allocation).
    pub emit_word_boxes: bool,
    /// Include rich PDF text metadata on public text items: MCID, glyph width,
    /// font metrics/weight/buggy state, fill/stroke colors, raw character
    /// codes, and generated-trailing-space state. Default `false` to preserve
    /// LiteParse's lightweight response shape. Layout-required metrics remain
    /// available internally; source character codes and generated-space state
    /// are not computed unless this is enabled.
    #[serde(default)]
    pub extract_text_metadata: bool,
    /// Restrict output to a sub-region of every page. Each field is the
    /// fraction of the page to crop away from that side (e.g. `left = 0.5`
    /// discards the left half). A text item is kept only when it lies
    /// *entirely* inside the surviving rectangle
    /// `[left·W, (1-right)·W] × [top·H, (1-bottom)·H]` (top-left origin).
    /// `None` (default) keeps the whole page. Applied after OCR merge, so it
    /// also drops OCR-sourced text outside the region.
    pub crop_box: Option<CropBox>,
    /// Drop diagonal (skewed) text — items whose rotation is more than 2°
    /// off the nearest right angle (0/90/180/270). Default `false`.
    pub skip_diagonal_text: bool,
    /// Compute per-page complexity signals during `parse` and attach them to
    /// each `ParsedPage` (surfaced as a `complexity` object per page in JSON).
    /// These are the same signals the standalone `is_complex` API returns.
    /// Default `false`.
    pub include_complexity: bool,
    /// Expose page-scoped vector path data (`shapes` and merged horizontal /
    /// vertical `lines`) in parse results. Default `false`; path objects are
    /// still inspected internally for layout detection when disabled.
    pub extract_vector_graphics: bool,
}

/// A page sub-region expressed as the fraction cropped from each side.
/// All fields are in `[0, 1]`; `left + right < 1` and `top + bottom < 1`
/// for a non-empty region. Origin is top-left, matching `TextItem` coordinates.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct CropBox {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

/// Image handling for the markdown emitter.
///
/// * `Off` — strip image references entirely.
/// * `Placeholder` (default) — emit `![](img_pN_K.png)` references in
///   reading order at each image's y position, but do **not** extract or
///   return pixel bytes. Keeps response size small while letting the LLM see
///   where figures live in the document.
/// * `Embed` — emit the same references as `Placeholder`, and extract the
///   embedded pixel bytes into `ParseResult.images` (equivalent to setting
///   `LiteParseConfig::extract_images`).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ImageMode {
    Off,
    Placeholder,
    Embed,
}

/// Supported output formats.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    Json,
    Text,
    Markdown,
}

impl LiteParseConfig {
    /// Whether embedded-image extraction should run. True when
    /// `extract_images` is set, or when the legacy `ImageMode::Embed` is
    /// configured (`Embed` implies byte extraction for backwards
    /// compatibility).
    pub fn effective_extract_images(&self) -> bool {
        self.extract_images || self.image_mode == ImageMode::Embed
    }
}

impl Default for LiteParseConfig {
    fn default() -> Self {
        Self {
            ocr_language: "eng".to_string(),
            // OCR is on by default only when a built-in engine is compiled in
            // (the `tesseract` feature). Builds without a built-in engine
            // (`tesseract` disabled, or WASM) default to OFF, so a consumer that
            // just wants native text extraction works out of the box instead of
            // hitting "OCR enabled but no engine available". OCR is still fully
            // available in those builds by opting in explicitly: set
            // `ocr_enabled = true` together with an `ocr_server_url` (or, on
            // WASM, an `ocrEngine` callback). That explicit request still fails
            // fast if no engine is reachable, so a real misconfiguration is
            // never silently swallowed.
            ocr_enabled: cfg!(feature = "tesseract"),
            ocr_server_url: None,
            ocr_server_headers: Vec::new(),
            tessdata_path: None,
            max_pages: 1000,
            target_pages: None,
            extract_screenshots: false,
            screenshot: ScreenshotOptions::default(),
            continue_on_page_error: false,
            dpi: 150.0,
            output_format: OutputFormat::Json,
            preserve_very_small_text: false,
            password: None,
            quiet: false,
            num_workers: default_num_workers(),
            image_mode: ImageMode::Placeholder,
            extract_images: false,
            image_output_dir: None,
            extract_links: true,
            keep_headers_footers: false,
            extract_annotations: false,
            extract_form_fields: false,
            extract_structure_tree: false,
            extract_blocks: false,
            extract_content_bounds: false,
            extract_xfa_packets: false,
            extract_document_metadata: false,
            detect_screenshot_rects: false,
            render_form_fields: false,
            ocr_failure_fatal: true,
            ocr_hedge_delays_ms: Vec::new(),
            emit_word_boxes: false,
            extract_text_metadata: false,
            crop_box: None,
            skip_diagonal_text: false,
            include_complexity: false,
            extract_vector_graphics: false,
        }
    }
}

/// Output used for rendered page screenshots.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScreenshotFormat {
    #[default]
    Png,
    Rgb8,
}

/// Screenshot output and format-specific encoder options.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ScreenshotOptions {
    /// Screenshot representation returned to the caller.
    pub format: ScreenshotFormat,
    /// PNG encoder settings, used when `format` is [`ScreenshotFormat::Png`].
    pub png: PngScreenshotOptions,
}

/// PNG-specific screenshot encoder settings.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PngScreenshotOptions {
    pub compression: PngCompression,
}

/// PNG DEFLATE compression preset. `Fast` preserves the existing behavior.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PngCompression {
    #[default]
    Fast,
    Default,
    Best,
}

impl PngCompression {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Fast => "fast",
            Self::Default => "default",
            Self::Best => "best",
        }
    }
}

impl ScreenshotFormat {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Rgb8 => "rgb8",
        }
    }
}

/// Returns the default number of OCR workers: CPU cores - 1, minimum 1.
fn default_num_workers() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get().saturating_sub(1).max(1))
        .unwrap_or(1)
}

/// Upper bound on the number of pages a `--target-pages` argument may expand
/// to. Ranges are materialised eagerly into a `Vec<u32>`, so without a cap a
/// tiny argument like `1-4294967295` would try to allocate ~17 GB and the
/// process would be OOM-killed before the document is even opened. No real
/// document approaches this many pages, so the cap only ever rejects nonsense
/// input.
const MAX_TARGET_PAGES: u64 = 100_000;

/// Pages per batch when a caller of
/// [`crate::parser::LiteParse::open_batch_session`] does
/// not pick a size. Small enough to keep the materialized result bounded;
/// large enough that the per-batch document reopen stays modest (it costs
/// roughly 13% at this size on a 457-page document, ~4% at 50).
pub const DEFAULT_PAGE_BATCH_SIZE: usize = 25;

#[doc(hidden)]
pub fn parse_target_pages(s: &str) -> Result<Vec<u32>, String> {
    let mut pages = Vec::new();
    for part in s.split(',') {
        let part = part.trim();
        if part.contains('-') {
            let bounds: Vec<&str> = part.splitn(2, '-').collect();
            let start: u32 = bounds[0]
                .trim()
                .parse()
                .map_err(|_| format!("invalid page number: {}", bounds[0]))?;
            let end: u32 = bounds[1]
                .trim()
                .parse()
                .map_err(|_| format!("invalid page number: {}", bounds[1]))?;
            if start > end {
                return Err(format!("invalid range: {}-{}", start, end));
            }
            // Reject before expanding so an oversized range can never allocate
            // gigabytes. `end >= start`, so the span cannot underflow.
            let span = end as u64 - start as u64 + 1;
            if pages.len() as u64 + span > MAX_TARGET_PAGES {
                return Err(format!(
                    "too many target pages: {}-{} exceeds the limit of {}",
                    start, end, MAX_TARGET_PAGES
                ));
            }
            for p in start..=end {
                pages.push(p);
            }
        } else {
            let p: u32 = part
                .parse()
                .map_err(|_| format!("invalid page number: {}", part))?;
            if pages.len() as u64 + 1 > MAX_TARGET_PAGES {
                return Err(format!(
                    "too many target pages: exceeds the limit of {}",
                    MAX_TARGET_PAGES
                ));
            }
            pages.push(p);
        }
    }
    pages.sort();
    pages.dedup();
    Ok(pages)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_target_pages() {
        assert_eq!(
            parse_target_pages("1-5,10,15-20").unwrap(),
            vec![1, 2, 3, 4, 5, 10, 15, 16, 17, 18, 19, 20]
        );
        assert_eq!(parse_target_pages("3").unwrap(), vec![3]);
        assert_eq!(parse_target_pages("1,1,2").unwrap(), vec![1, 2]);
        assert!(parse_target_pages("5-3").is_err());
        assert!(parse_target_pages("abc").is_err());
    }

    #[test]
    fn test_parse_target_pages_with_whitespace() {
        assert_eq!(parse_target_pages(" 1 , 2 - 4 ").unwrap(), vec![1, 2, 3, 4]);
    }

    #[test]
    fn test_parse_target_pages_single_range() {
        assert_eq!(parse_target_pages("2-2").unwrap(), vec![2]);
    }

    #[test]
    fn test_parse_target_pages_rejects_oversized_range() {
        // The headline OOM repro from #269: a 12-character argument must not
        // be allowed to expand into a multi-gigabyte allocation.
        assert!(parse_target_pages("1-4294967295").is_err());
        assert!(parse_target_pages("0-4294967295").is_err());
        // The cap is on the total page count, so multiple ranges that
        // together exceed the limit are rejected too.
        assert!(parse_target_pages("1-60000,60001-120000").is_err());
        // A selection within the limit is still parsed normally.
        assert_eq!(parse_target_pages("1-1000").unwrap().len(), 1000);
    }

    #[test]
    fn test_default_config() {
        let c = LiteParseConfig::default();
        assert_eq!(c.ocr_language, "eng");
        // OCR defaults on only when a built-in engine is compiled in.
        assert_eq!(c.ocr_enabled, cfg!(feature = "tesseract"));
        assert_eq!(c.max_pages, 1000);
        assert!(!c.extract_screenshots);
        assert!(!c.continue_on_page_error);
        assert_eq!(c.dpi, 150.0);
        assert_eq!(c.output_format, OutputFormat::Json);
        assert!(!c.preserve_very_small_text);
        assert!(!c.quiet);
        assert!(!c.extract_text_metadata);
        assert!(c.password.is_none());
        assert!(!c.extract_images);
        assert!(!c.extract_structure_tree);
    }

    #[test]
    fn test_output_format_lowercase_serde() {
        let s = serde_json::to_string(&OutputFormat::Json).unwrap();
        assert_eq!(s, "\"json\"");
        let back: OutputFormat = serde_json::from_str("\"text\"").unwrap();
        assert_eq!(back, OutputFormat::Text);
    }

    #[test]
    fn test_config_roundtrip() {
        let c = LiteParseConfig::default();
        let s = serde_json::to_string(&c).unwrap();
        let back: LiteParseConfig = serde_json::from_str(&s).unwrap();
        assert_eq!(back.ocr_language, c.ocr_language);
        assert_eq!(back.output_format, c.output_format);
        assert_eq!(back.screenshot, ScreenshotOptions::default());
    }

    #[test]
    fn screenshot_format_uses_stable_wire_names() {
        assert_eq!(
            serde_json::to_string(&ScreenshotFormat::Png).unwrap(),
            "\"png\""
        );
        assert_eq!(
            serde_json::to_string(&ScreenshotFormat::Rgb8).unwrap(),
            "\"rgb8\""
        );
        assert_eq!(
            serde_json::from_str::<ScreenshotFormat>("\"rgb8\"").unwrap(),
            ScreenshotFormat::Rgb8
        );
    }

    #[test]
    fn screenshot_options_are_additive_and_default_nested_fields() {
        let options: ScreenshotOptions = serde_json::from_str(r#"{"format":"rgb8"}"#).unwrap();
        assert_eq!(options.format, ScreenshotFormat::Rgb8);
        assert_eq!(options.png.compression, PngCompression::Fast);

        let options: ScreenshotOptions =
            serde_json::from_str(r#"{"png":{"compression":"best"}}"#).unwrap();
        assert_eq!(options.format, ScreenshotFormat::Png);
        assert_eq!(options.png.compression, PngCompression::Best);
    }
}
