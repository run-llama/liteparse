use std::collections::HashMap;

use pyo3::prelude::*;
use pyo3::types::PyBytes;

use liteparse::config::{ImageMode, LiteParseConfig, OutputFormat};
use liteparse::types::PdfInput;

mod cli;

// ---------------------------------------------------------------------------
// Python type wrappers
// ---------------------------------------------------------------------------

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

impl PyTextItem {
    fn to_rust(&self) -> liteparse::types::TextItem {
        liteparse::types::TextItem {
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
    text_items: Vec<PyTextItem>,
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
            text_items: page
                .text_items
                .into_iter()
                .map(PyTextItem::from_rust)
                .collect(),
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
    inline_images: bool,
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
            inline_images: cfg.inline_images,
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
        inline_images = None,
        extract_links = None,
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
        inline_images: Option<bool>,
        extract_links: Option<bool>,
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
        if let Some(v) = inline_images {
            cfg.inline_images = v;
        }
        if let Some(v) = extract_links {
            cfg.extract_links = v;
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

/// Search text items for phrase matches, returning merged items with combined bounding boxes.
#[pyfunction]
#[pyo3(signature = (items, phrase, *, case_sensitive = false))]
fn search_items(items: Vec<PyTextItem>, phrase: String, case_sensitive: bool) -> Vec<PyTextItem> {
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
    m.add_class::<PyScreenshotResult>()?;
    m.add_function(wrap_pyfunction!(run_cli, m)?)?;
    m.add_function(wrap_pyfunction!(search_items, m)?)?;
    Ok(())
}
