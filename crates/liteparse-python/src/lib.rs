use std::collections::HashMap;

use pyo3::prelude::*;
use pyo3::types::PyBytes;

use liteparse::config::{CropBox, ImageMode, LiteParseConfig, OutputFormat};
use liteparse::types::PdfInput;

mod cli;

// ---------------------------------------------------------------------------
// Python type wrappers
// ---------------------------------------------------------------------------

#[pyclass(frozen, from_py_object)]
#[derive(Clone)]
struct PyWordBox {
    #[pyo3(get)]
    text: String,
    #[pyo3(get)]
    x: f64,
    #[pyo3(get)]
    y: f64,
    #[pyo3(get)]
    width: f64,
    #[pyo3(get)]
    height: f64,
}

#[pymethods]
impl PyWordBox {
    fn __repr__(&self) -> String {
        format!(
            "WordBox(text={:?}, x={}, y={}, width={}, height={})",
            self.text, self.x, self.y, self.width, self.height
        )
    }
}

impl PyWordBox {
    fn from_rust(word: liteparse::types::WordBox) -> Self {
        Self {
            text: word.text,
            x: word.x as f64,
            y: word.y as f64,
            width: word.width as f64,
            height: word.height as f64,
        }
    }
}

#[pyclass(frozen, from_py_object)]
#[derive(Clone)]
struct PyTextItem {
    #[pyo3(get)]
    text: String,
    #[pyo3(get)]
    x: f64,
    #[pyo3(get)]
    y: f64,
    #[pyo3(get)]
    width: f64,
    #[pyo3(get)]
    height: f64,
    #[pyo3(get)]
    font_name: Option<String>,
    #[pyo3(get)]
    font_size: Option<f64>,
    #[pyo3(get)]
    confidence: Option<f64>,
    /// Rotation in degrees (viewport space). Defaults to 0.
    #[pyo3(get)]
    rotation: f64,
    /// Per-word sub-boxes for attribution. Empty unless the parse was
    /// configured with `emit_word_boxes=True`.
    #[pyo3(get)]
    words: Vec<PyWordBox>,
}

#[pymethods]
impl PyTextItem {
    fn __repr__(&self) -> String {
        format!(
            "TextItem(text={:?}, x={}, y={}, width={}, height={})",
            self.text, self.x, self.y, self.width, self.height
        )
    }
}

#[pyclass(frozen, from_py_object)]
#[derive(Clone)]
struct PyAnnotationRect {
    #[pyo3(get)]
    x: f64,
    #[pyo3(get)]
    y: f64,
    #[pyo3(get)]
    width: f64,
    #[pyo3(get)]
    height: f64,
}

impl PyAnnotationRect {
    fn from_rust(rect: liteparse::types::Rect) -> Self {
        Self {
            x: rect.x as f64,
            y: rect.y as f64,
            width: rect.width as f64,
            height: rect.height as f64,
        }
    }
}

#[pyclass(frozen, from_py_object)]
#[derive(Clone)]
struct PyDocumentAnnotation {
    #[pyo3(get)]
    subtype: String,
    #[pyo3(get)]
    contents: Option<String>,
    #[pyo3(get)]
    created: Option<String>,
    #[pyo3(get)]
    modified: Option<String>,
    #[pyo3(get)]
    title: Option<String>,
    #[pyo3(get)]
    rect: Option<PyAnnotationRect>,
    #[pyo3(get)]
    quadpoint_rects: Vec<PyAnnotationRect>,
    #[pyo3(get)]
    uri: Option<String>,
}

impl PyDocumentAnnotation {
    fn from_rust(annotation: liteparse::types::DocumentAnnotation) -> Self {
        Self {
            subtype: annotation.subtype,
            contents: annotation.contents,
            created: annotation.created,
            modified: annotation.modified,
            title: annotation.title,
            rect: annotation.rect.map(PyAnnotationRect::from_rust),
            quadpoint_rects: annotation
                .quadpoint_rects
                .into_iter()
                .map(PyAnnotationRect::from_rust)
                .collect(),
            uri: annotation.uri,
        }
    }
}

impl PyTextItem {
    fn from_rust(item: liteparse::types::TextItem) -> Self {
        Self {
            text: item.text,
            x: item.x as f64,
            y: item.y as f64,
            width: item.width as f64,
            height: item.height as f64,
            font_name: item.font_name,
            font_size: item.font_size.map(|v| v as f64),
            confidence: item.confidence.map(|v| v as f64).or(Some(1.0)),
            rotation: item.rotation as f64,
            words: item.words.into_iter().map(PyWordBox::from_rust).collect(),
        }
    }
}

#[pyclass(frozen, from_py_object)]
#[derive(Clone)]
struct PyParsedPage {
    #[pyo3(get)]
    page_num: u32,
    #[pyo3(get)]
    width: f64,
    #[pyo3(get)]
    height: f64,
    #[pyo3(get)]
    text: String,
    #[pyo3(get)]
    markdown: String,
    #[pyo3(get)]
    text_items: Vec<PyTextItem>,
    #[pyo3(get)]
    complexity: Option<PyPageComplexityStats>,
    #[pyo3(get)]
    annotations: Option<Vec<PyDocumentAnnotation>>,
}

#[pymethods]
impl PyParsedPage {
    fn __repr__(&self) -> String {
        format!(
            "ParsedPage(page_num={}, width={}, height={}, text_items={})",
            self.page_num,
            self.width,
            self.height,
            self.text_items.len()
        )
    }
}

impl PyParsedPage {
    fn from_rust(page: liteparse::types::ParsedPage) -> Self {
        Self {
            page_num: page.page_number as u32,
            width: page.page_width as f64,
            height: page.page_height as f64,
            text: page.text,
            markdown: page.markdown,
            text_items: page
                .text_items
                .into_iter()
                .map(PyTextItem::from_rust)
                .collect(),
            complexity: page
                .complexity
                .as_ref()
                .map(PyPageComplexityStats::from_rust),
            annotations: page.annotations.map(|annotations| {
                annotations
                    .into_iter()
                    .map(PyDocumentAnnotation::from_rust)
                    .collect()
            }),
        }
    }
}

#[pyclass(frozen, from_py_object)]
#[derive(Clone)]
struct PyParseResult {
    #[pyo3(get)]
    pages: Vec<PyParsedPage>,
    #[pyo3(get)]
    text: String,
    #[pyo3(get)]
    images: Vec<PyExtractedImage>,
}

#[pymethods]
impl PyParseResult {
    #[getter]
    fn num_pages(&self) -> usize {
        self.pages.len()
    }

    fn get_page(&self, page_num: u32) -> Option<PyParsedPage> {
        self.pages.iter().find(|p| p.page_num == page_num).cloned()
    }

    fn __repr__(&self) -> String {
        format!(
            "ParseResult(pages={}, text_len={}, images={})",
            self.pages.len(),
            self.text.len(),
            self.images.len()
        )
    }
}

impl PyParseResult {
    fn from_rust(result: liteparse::parser::ParseResult) -> Self {
        Self {
            pages: result
                .pages
                .into_iter()
                .map(PyParsedPage::from_rust)
                .collect(),
            text: result.text,
            images: result
                .images
                .into_iter()
                .map(PyExtractedImage::from_rust)
                .collect(),
        }
    }
}

#[pyclass(frozen, from_py_object)]
#[derive(Clone)]
struct PyExtractedImage {
    #[pyo3(get)]
    id: String,
    #[pyo3(get)]
    page: u32,
    #[pyo3(get)]
    format: String,
    bytes_buffer: Vec<u8>,
}

#[pymethods]
impl PyExtractedImage {
    #[getter]
    fn bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.bytes_buffer)
    }

    fn __repr__(&self) -> String {
        format!(
            "ExtractedImage(id='{}', page={}, format='{}', bytes_len={})",
            self.id,
            self.page,
            self.format,
            self.bytes_buffer.len()
        )
    }
}

impl PyExtractedImage {
    fn from_rust(img: liteparse::types::ExtractedImage) -> Self {
        Self {
            id: img.id,
            page: img.page,
            format: img.format,
            bytes_buffer: img.bytes,
        }
    }
}

#[pyclass(frozen, from_py_object)]
#[derive(Clone)]
struct PyScreenshotResult {
    #[pyo3(get)]
    page_num: u32,
    #[pyo3(get)]
    width: u32,
    #[pyo3(get)]
    height: u32,
    image_buffer: Vec<u8>,
}

#[pymethods]
impl PyScreenshotResult {
    #[getter]
    fn image_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.image_buffer)
    }

    fn __repr__(&self) -> String {
        format!(
            "ScreenshotResult(page_num={}, width={}, height={})",
            self.page_num, self.width, self.height
        )
    }
}

#[pyclass(frozen, from_py_object)]
#[derive(Clone)]
struct PyLayoutComplexityStats {
    #[pyo3(get)]
    column_count: usize,
    #[pyo3(get)]
    ruled_table_count: usize,
    #[pyo3(get)]
    ruled_table_coverage: f32,
    #[pyo3(get)]
    text_table_run_count: usize,
    #[pyo3(get)]
    figure_count: usize,
    #[pyo3(get)]
    figure_coverage: f32,
    #[pyo3(get)]
    is_complex: bool,
    #[pyo3(get)]
    reasons: Vec<String>,
}

#[pymethods]
impl PyLayoutComplexityStats {
    fn __repr__(&self) -> String {
        format!(
            "LayoutComplexityStats(column_count={}, ruled_table_count={}, figure_coverage={:.2}, is_complex={})",
            self.column_count, self.ruled_table_count, self.figure_coverage, self.is_complex
        )
    }
}

impl PyLayoutComplexityStats {
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

#[pyclass(frozen, from_py_object)]
#[derive(Clone)]
struct PyPageComplexityStats {
    #[pyo3(get)]
    page_number: usize,
    #[pyo3(get)]
    text_length: usize,
    #[pyo3(get)]
    text_coverage: f32,
    #[pyo3(get)]
    has_substantial_images: bool,
    #[pyo3(get)]
    image_block_count: usize,
    #[pyo3(get)]
    image_coverage: f32,
    #[pyo3(get)]
    largest_image_coverage: f32,
    #[pyo3(get)]
    full_page_image: bool,
    #[pyo3(get)]
    uncovered_vector_area: Option<f32>,
    #[pyo3(get)]
    is_garbled: bool,
    #[pyo3(get)]
    page_area: f32,
    #[pyo3(get)]
    needs_ocr: bool,
    #[pyo3(get)]
    reasons: Vec<String>,
    #[pyo3(get)]
    layout: Option<PyLayoutComplexityStats>,
}

#[pymethods]
impl PyPageComplexityStats {
    fn __repr__(&self) -> String {
        format!(
            "PageComplexityStats(page_number={}, text_length={}, text_coverage={:.2}, needs_ocr={})",
            self.page_number, self.text_length, self.text_coverage, self.needs_ocr
        )
    }
}

impl PyPageComplexityStats {
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
            layout: stats
                .layout
                .as_ref()
                .map(PyLayoutComplexityStats::from_rust),
        }
    }
}

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

#[pyclass(frozen, from_py_object)]
#[derive(Clone)]
struct PyLiteParseConfig {
    #[pyo3(get)]
    ocr_language: String,
    #[pyo3(get)]
    ocr_enabled: bool,
    #[pyo3(get)]
    ocr_server_url: Option<String>,
    #[pyo3(get)]
    ocr_server_headers: Option<HashMap<String, String>>,
    #[pyo3(get)]
    tessdata_path: Option<String>,
    #[pyo3(get)]
    max_pages: usize,
    #[pyo3(get)]
    target_pages: Option<String>,
    #[pyo3(get)]
    dpi: f32,
    #[pyo3(get)]
    output_format: String,
    #[pyo3(get)]
    preserve_very_small_text: bool,
    #[pyo3(get)]
    password: Option<String>,
    #[pyo3(get)]
    quiet: bool,
    #[pyo3(get)]
    num_workers: usize,
    #[pyo3(get)]
    image_mode: String,
    #[pyo3(get)]
    extract_links: bool,
    #[pyo3(get)]
    extract_annotations: bool,
    #[pyo3(get)]
    ocr_failure_fatal: bool,
    #[pyo3(get)]
    ocr_hedge_delays_ms: Vec<u64>,
    #[pyo3(get)]
    emit_word_boxes: bool,
    #[pyo3(get)]
    crop_box: Option<(f32, f32, f32, f32)>,
    #[pyo3(get)]
    skip_diagonal_text: bool,
    #[pyo3(get)]
    include_complexity: bool,
}

#[pymethods]
impl PyLiteParseConfig {
    fn __repr__(&self) -> String {
        format!(
            "LiteParseConfig(ocr_enabled={}, dpi={}, max_pages={})",
            self.ocr_enabled, self.dpi, self.max_pages
        )
    }
}

impl PyLiteParseConfig {
    fn from_rust(cfg: &LiteParseConfig) -> Self {
        Self {
            ocr_language: cfg.ocr_language.clone(),
            ocr_enabled: cfg.ocr_enabled,
            ocr_server_url: cfg.ocr_server_url.clone(),
            ocr_server_headers: if cfg.ocr_server_headers.is_empty() {
                None
            } else {
                Some(cfg.ocr_server_headers.iter().cloned().collect())
            },
            tessdata_path: cfg.tessdata_path.clone(),
            max_pages: cfg.max_pages,
            target_pages: cfg.target_pages.clone(),
            dpi: cfg.dpi,
            output_format: match cfg.output_format {
                OutputFormat::Json => "json".to_string(),
                OutputFormat::Text => "text".to_string(),
                OutputFormat::Markdown => "markdown".to_string(),
            },
            preserve_very_small_text: cfg.preserve_very_small_text,
            password: cfg.password.clone(),
            quiet: cfg.quiet,
            num_workers: cfg.num_workers,
            image_mode: match cfg.image_mode {
                ImageMode::Off => "off".to_string(),
                ImageMode::Placeholder => "placeholder".to_string(),
                ImageMode::Embed => "embed".to_string(),
            },
            extract_links: cfg.extract_links,
            extract_annotations: cfg.extract_annotations,
            ocr_failure_fatal: cfg.ocr_failure_fatal,
            ocr_hedge_delays_ms: cfg.ocr_hedge_delays_ms.clone(),
            emit_word_boxes: cfg.emit_word_boxes,
            crop_box: cfg
                .crop_box
                .as_ref()
                .map(|c| (c.top, c.right, c.bottom, c.left)),
            skip_diagonal_text: cfg.skip_diagonal_text,
            include_complexity: cfg.include_complexity,
        }
    }
}

// ---------------------------------------------------------------------------
// Main LiteParse class
// ---------------------------------------------------------------------------

#[pyclass]
struct LiteParse {
    inner: liteparse::parser::LiteParse,
    config: LiteParseConfig,
    runtime: tokio::runtime::Runtime,
}

#[pymethods]
impl LiteParse {
    #[new]
    #[pyo3(signature = (
        *,
        ocr_language = None,
        ocr_enabled = None,
        ocr_server_url = None,
        ocr_server_headers = None,
        tessdata_path = None,
        max_pages = None,
        target_pages = None,
        dpi = None,
        output_format = None,
        preserve_very_small_text = None,
        password = None,
        quiet = None,
        num_workers = None,
        image_mode = None,
        extract_links = None,
        extract_annotations = None,
        ocr_failure_fatal = None,
        ocr_hedge_delays_ms = None,
        emit_word_boxes = None,
        crop_box = None,
        skip_diagonal_text = None,
        include_complexity = None,
    ))]
    fn new(
        ocr_language: Option<String>,
        ocr_enabled: Option<bool>,
        ocr_server_url: Option<String>,
        ocr_server_headers: Option<HashMap<String, String>>,
        tessdata_path: Option<String>,
        max_pages: Option<usize>,
        target_pages: Option<String>,
        dpi: Option<f32>,
        output_format: Option<String>,
        preserve_very_small_text: Option<bool>,
        password: Option<String>,
        quiet: Option<bool>,
        num_workers: Option<usize>,
        image_mode: Option<String>,
        extract_links: Option<bool>,
        extract_annotations: Option<bool>,
        ocr_failure_fatal: Option<bool>,
        ocr_hedge_delays_ms: Option<Vec<u64>>,
        emit_word_boxes: Option<bool>,
        crop_box: Option<(f32, f32, f32, f32)>,
        skip_diagonal_text: Option<bool>,
        include_complexity: Option<bool>,
    ) -> PyResult<Self> {
        let mut cfg = LiteParseConfig::default();
        if let Some(v) = ocr_language {
            cfg.ocr_language = v;
        }
        if let Some(v) = ocr_enabled {
            cfg.ocr_enabled = v;
        }
        if let Some(v) = ocr_server_url {
            cfg.ocr_server_url = Some(v);
        }
        if let Some(v) = ocr_server_headers {
            cfg.ocr_server_headers = v.into_iter().collect();
        }
        if let Some(v) = tessdata_path {
            cfg.tessdata_path = Some(v);
        }
        if let Some(v) = max_pages {
            cfg.max_pages = v;
        }
        if let Some(v) = target_pages {
            cfg.target_pages = Some(v);
        }
        if let Some(v) = dpi {
            cfg.dpi = v;
        }
        if let Some(v) = output_format {
            cfg.output_format = match v.as_str() {
                "text" => OutputFormat::Text,
                "markdown" | "md" => OutputFormat::Markdown,
                _ => OutputFormat::Json,
            };
        }
        if let Some(v) = preserve_very_small_text {
            cfg.preserve_very_small_text = v;
        }
        if let Some(v) = password {
            cfg.password = Some(v);
        }
        if let Some(v) = quiet {
            cfg.quiet = v;
        }
        if let Some(v) = num_workers {
            cfg.num_workers = v;
        }
        if let Some(v) = image_mode {
            cfg.image_mode = match v.as_str() {
                "off" | "none" => ImageMode::Off,
                "embed" => ImageMode::Embed,
                _ => ImageMode::Placeholder,
            };
        }
        if let Some(v) = extract_links {
            cfg.extract_links = v;
        }
        if let Some(v) = extract_annotations {
            cfg.extract_annotations = v;
        }
        if let Some(v) = ocr_failure_fatal {
            cfg.ocr_failure_fatal = v;
        }
        if let Some(v) = ocr_hedge_delays_ms {
            cfg.ocr_hedge_delays_ms = v;
        }
        if let Some(v) = emit_word_boxes {
            cfg.emit_word_boxes = v;
        }
        if let Some((top, right, bottom, left)) = crop_box {
            cfg.crop_box = Some(CropBox {
                top,
                right,
                bottom,
                left,
            });
        }
        if let Some(v) = skip_diagonal_text {
            cfg.skip_diagonal_text = v;
        }
        if let Some(v) = include_complexity {
            cfg.include_complexity = v;
        }

        let inner = liteparse::parser::LiteParse::new(cfg.clone());
        let runtime = tokio::runtime::Runtime::new()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;

        Ok(Self {
            inner,
            config: cfg,
            runtime,
        })
    }

    /// Parse a document from a file path.
    fn parse(&self, py: Python<'_>, input: String) -> PyResult<PyParseResult> {
        let pdf_input = PdfInput::Path(input);
        let result = py
            .detach(|| self.runtime.block_on(self.inner.parse_input(pdf_input)))
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;
        Ok(PyParseResult::from_rust(result))
    }

    /// Parse a document from raw bytes.
    fn parse_bytes(&self, py: Python<'_>, data: Vec<u8>) -> PyResult<PyParseResult> {
        let pdf_input = PdfInput::Bytes(data);
        let result = py
            .detach(|| self.runtime.block_on(self.inner.parse_input(pdf_input)))
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;
        Ok(PyParseResult::from_rust(result))
    }

    /// Determine per-page complexity for a document at the given path. Returns
    /// a list of PageComplexityStats — a cheap pre-OCR check with per-page
    /// signals and a `needs_ocr` verdict.
    fn is_complex(&self, py: Python<'_>, input: String) -> PyResult<Vec<PyPageComplexityStats>> {
        let pdf_input = PdfInput::Path(input);
        let stats = py
            .detach(|| self.runtime.block_on(self.inner.is_complex(pdf_input)))
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;
        Ok(stats.iter().map(PyPageComplexityStats::from_rust).collect())
    }

    /// Determine per-page complexity for a document from raw bytes.
    fn is_complex_bytes(
        &self,
        py: Python<'_>,
        data: Vec<u8>,
    ) -> PyResult<Vec<PyPageComplexityStats>> {
        let pdf_input = PdfInput::Bytes(data);
        let stats = py
            .detach(|| self.runtime.block_on(self.inner.is_complex(pdf_input)))
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;
        Ok(stats.iter().map(PyPageComplexityStats::from_rust).collect())
    }

    /// Take screenshots of document pages. Returns a list of ScreenshotResult.
    ///
    /// Non-PDF files are automatically converted to PDF before rendering when
    /// LibreOffice/ImageMagick are available.
    #[pyo3(signature = (input, page_numbers = None))]
    fn screenshot(
        &self,
        py: Python<'_>,
        input: String,
        page_numbers: Option<Vec<u32>>,
    ) -> PyResult<Vec<PyScreenshotResult>> {
        py.detach(|| {
            let results = self
                .runtime
                .block_on(self.inner.screenshot(&input, page_numbers))
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;

            Ok(results
                .into_iter()
                .map(|r| PyScreenshotResult {
                    page_num: r.page_num,
                    width: r.width,
                    height: r.height,
                    image_buffer: r.image_bytes,
                })
                .collect())
        })
    }

    /// Get the resolved configuration.
    #[getter]
    fn config(&self) -> PyLiteParseConfig {
        PyLiteParseConfig::from_rust(&self.config)
    }

    fn __repr__(&self) -> String {
        format!(
            "LiteParse(ocr_enabled={}, dpi={}, max_pages={})",
            self.config.ocr_enabled, self.config.dpi, self.config.max_pages
        )
    }
}

// ---------------------------------------------------------------------------
// Module
// ---------------------------------------------------------------------------

/// Duck-typed input for [`search_items`]. Extracts by attribute name, so it
/// accepts both the native `PyTextItem` and the pure-Python `TextItem`
/// dataclass that `LiteParse.parse()` hands back (see `parser.py`,
/// `_convert_native_result`).
#[derive(FromPyObject)]
struct SearchInputItem {
    text: String,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    #[pyo3(default)]
    font_name: Option<String>,
    #[pyo3(default)]
    font_size: Option<f64>,
    #[pyo3(default)]
    confidence: Option<f64>,
    #[pyo3(default)]
    rotation: f64,
}

impl SearchInputItem {
    fn to_rust(&self) -> liteparse::types::TextItem {
        liteparse::types::TextItem {
            text: self.text.clone(),
            x: self.x as f32,
            y: self.y as f32,
            width: self.width as f32,
            height: self.height as f32,
            rotation: self.rotation as f32,
            font_name: self.font_name.clone(),
            font_size: self.font_size.map(|v| v as f32),
            confidence: self.confidence.map(|v| v as f32),
            ..Default::default()
        }
    }
}

/// Search text items for phrase matches, returning merged items with combined bounding boxes.
#[pyfunction]
#[pyo3(signature = (items, phrase, *, case_sensitive = false))]
fn search_items(
    items: Vec<SearchInputItem>,
    phrase: String,
    case_sensitive: bool,
) -> Vec<PyTextItem> {
    let rust_items: Vec<_> = items.iter().map(|i| i.to_rust()).collect();
    let options = liteparse::search::SearchOptions {
        phrase,
        case_sensitive,
    };
    liteparse::search::search_items(&rust_items, &options)
        .into_iter()
        .map(PyTextItem::from_rust)
        .collect()
}

/// Run the `lit` CLI with the given arguments.
#[pyfunction]
fn run_cli(args: Vec<String>) -> PyResult<()> {
    cli::run_cli(args).map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))
}

#[pymodule]
fn _liteparse(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<LiteParse>()?;
    m.add_class::<PyLiteParseConfig>()?;
    m.add_class::<PyParseResult>()?;
    m.add_class::<PyExtractedImage>()?;
    m.add_class::<PyParsedPage>()?;
    m.add_class::<PyTextItem>()?;
    m.add_class::<PyWordBox>()?;
    m.add_class::<PyAnnotationRect>()?;
    m.add_class::<PyDocumentAnnotation>()?;
    m.add_class::<PyScreenshotResult>()?;
    m.add_class::<PyPageComplexityStats>()?;
    m.add_class::<PyLayoutComplexityStats>()?;
    m.add_function(wrap_pyfunction!(run_cli, m)?)?;
    m.add_function(wrap_pyfunction!(search_items, m)?)?;
    Ok(())
}
