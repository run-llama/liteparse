use napi::bindgen_prelude::*;
use napi_derive::napi;

mod types;

use types::{
    JsLiteParseConfig, JsPageComplexityStats, JsPageInput, JsParseBatch, JsParseResult,
    JsScreenshotResult, JsTextItem,
};

/// Main LiteParse parser class.
#[napi]
pub struct LiteParse {
    inner: liteparse::parser::LiteParse,
    config: liteparse::config::LiteParseConfig,
}

#[napi]
impl LiteParse {
    /// Create a new LiteParse instance with optional configuration.
    /// Any fields not provided will use defaults.
    #[napi(constructor)]
    pub fn new(config: Option<JsLiteParseConfig>) -> Self {
        let rust_config = config.map(|c| c.into_rust()).unwrap_or_default();
        let inner = liteparse::parser::LiteParse::new(rust_config.clone());
        Self {
            inner,
            config: rust_config,
        }
    }

    /// Parse a document. Accepts a file path (string) or raw PDF bytes (Buffer).
    #[napi]
    pub async fn parse(&self, input: Either<String, Buffer>) -> Result<JsParseResult> {
        use liteparse::types::PdfInput;

        let pdf_input = match input {
            Either::A(path) => PdfInput::Path(path),
            Either::B(buf) => PdfInput::Bytes(buf.to_vec()),
        };

        let result = self
            .inner
            .parse_input(pdf_input)
            .await
            .map_err(|e| Error::from_reason(e.to_string()))?;

        Ok(JsParseResult::from_rust(&result, &self.config))
    }

    /// Open a document for bounded-memory batch parsing. Internal plumbing
    /// for the JS wrapper's `parseBatches()` — prefer that; it also closes
    /// the session for you.
    ///
    /// Converts a non-PDF source once, then yields `batchSize` pages at a time
    /// via `nextBatch()` (default 25). Cross-page passes (repeated
    /// header/footer removal, image deduplication) see only the pages in their
    /// own batch, so output can differ from a whole-document `parse()`.
    #[napi]
    pub async fn open_batch_session(
        &self,
        input: Either<String, Buffer>,
        batch_size: Option<u32>,
    ) -> Result<ParseSession> {
        use liteparse::types::PdfInput;

        let pdf_input = match input {
            Either::A(path) => PdfInput::Path(path),
            Either::B(buf) => PdfInput::Bytes(buf.to_vec()),
        };
        let batch_size = batch_size
            .map(|v| v as usize)
            .unwrap_or(liteparse::DEFAULT_PAGE_BATCH_SIZE);

        let session = self
            .inner
            .open_batch_session(pdf_input, batch_size)
            .await
            .map_err(|e| Error::from_reason(e.to_string()))?;

        Ok(ParseSession {
            total_pages: session.total_pages(),
            inner: std::sync::Arc::new(tokio::sync::Mutex::new(Some(session))),
            config: self.config.clone(),
        })
    }

    /// Parse from pre-extracted pages, skipping PDFium text extraction.
    ///
    /// The caller supplies pages already populated with text items in viewport
    /// space (top-left origin, 72 DPI). Runs only grid projection + the
    /// configured output formatter, so it never loads PDFium. Use when an
    /// external extractor owns text extraction (e.g. to keep its own
    /// font-recovery pipeline).
    #[napi]
    pub fn parse_pages(&self, pages: Vec<JsPageInput>) -> Result<JsParseResult> {
        let rust_pages: Vec<_> = pages.iter().map(JsPageInput::to_rust).collect();
        let result = self.inner.parse_from_pages(rust_pages, Vec::new());
        Ok(JsParseResult::from_rust(&result, &self.config))
    }

    /// Determine per-page complexity. Returns one entry per parsed page with
    /// signals (text coverage, images, garbled text, vector area) and a
    /// `needsOcr` verdict — a cheap pre-OCR check to decide whether a document
    /// needs advanced parsing. Accepts a file path (string) or raw PDF bytes.
    #[napi]
    pub async fn is_complex(
        &self,
        input: Either<String, Buffer>,
    ) -> Result<Vec<JsPageComplexityStats>> {
        use liteparse::types::PdfInput;

        let pdf_input = match input {
            Either::A(path) => PdfInput::Path(path),
            Either::B(buf) => PdfInput::Bytes(buf.to_vec()),
        };

        let stats = self
            .inner
            .is_complex(pdf_input)
            .await
            .map_err(|e| Error::from_reason(e.to_string()))?;

        Ok(stats.iter().map(JsPageComplexityStats::from_rust).collect())
    }

    /// Take screenshots of document pages in the configured format.
    ///
    /// Non-PDF files are automatically converted to PDF before rendering when
    /// LibreOffice/ImageMagick are available.
    #[napi]
    pub async fn screenshot(
        &self,
        input: Either<String, Buffer>,
        page_numbers: Option<Vec<u32>>,
    ) -> Result<Vec<JsScreenshotResult>> {
        use liteparse::types::PdfInput;

        let pdf_input = match input {
            Either::A(path) => PdfInput::Path(path),
            Either::B(buf) => PdfInput::Bytes(buf.to_vec()),
        };

        let results = self
            .inner
            .screenshot_input(pdf_input, page_numbers)
            .await
            .map_err(|e| Error::from_reason(e.to_string()))?;

        Ok(results
            .into_iter()
            .map(|r| JsScreenshotResult {
                page_num: r.page_num,
                width: r.width,
                height: r.height,
                image_buffer: r.image_bytes.into(),
                format: r.format.into(),
                stride: r.stride,
                is_solid_fill: r.is_solid_fill,
                rects: r
                    .rects
                    .iter()
                    .map(crate::types::JsScreenshotRect::from_rust)
                    .collect(),
            })
            .collect())
    }

    /// Get the current configuration.
    #[napi(getter)]
    pub fn config(&self) -> JsLiteParseConfig {
        JsLiteParseConfig::from_rust(&self.config)
    }
}

/// A document opened once and parsed in bounded page batches. Internal
/// plumbing for the JS wrapper's `parseBatches()` — prefer that.
///
/// Created by `LiteParse.openBatchSession()`. The converted-PDF temporary
/// file for a non-PDF source lives as long as the session, so conversion is
/// paid once no matter how many batches are consumed. Call `close()` when
/// abandoning the session early — otherwise that temp file waits for GC.
#[napi]
pub struct ParseSession {
    /// The core session is `&mut` per batch, but napi hands out `&self`, so
    /// the mutation is serialized here. Concurrent `nextBatch()` calls queue
    /// rather than interleave, which also keeps batch order well-defined.
    /// `None` after `close()`.
    inner: std::sync::Arc<tokio::sync::Mutex<Option<liteparse::ParseSession>>>,
    config: liteparse::config::LiteParseConfig,
    total_pages: u32,
}

#[napi]
impl ParseSession {
    /// Total pages in the source document, before `maxPages` or batching.
    #[napi(getter)]
    pub fn total_pages(&self) -> u32 {
        self.total_pages
    }

    /// Parse and return the next batch, or `null` once every page within
    /// `maxPages` has been yielded. Rejects if the session is closed.
    #[napi]
    pub async fn next_batch(&self) -> Result<Option<JsParseBatch>> {
        let inner = self.inner.clone();
        let mut session = inner.lock().await;
        let batch = session
            .as_mut()
            .ok_or_else(|| Error::from_reason("session is closed"))?
            .next_batch()
            .await
            .map_err(|e| Error::from_reason(e.to_string()))?;

        Ok(batch.map(|batch| JsParseBatch {
            start_page: batch.start_page,
            end_page: batch.end_page,
            result: JsParseResult::from_rust(&batch.result, &self.config),
        }))
    }

    /// Release the session's resources now — most importantly the converted
    /// temporary PDF for a non-PDF source, which otherwise lives until the
    /// JS object is garbage collected. Idempotent; `nextBatch()` rejects
    /// afterwards.
    #[napi]
    pub async fn close(&self) -> Result<()> {
        let inner = self.inner.clone();
        let mut session = inner.lock().await;
        // Dropping the core session drops the conversion guard, which
        // removes the temp file.
        session.take();
        Ok(())
    }
}

/// Search text items for phrase matches, returning merged items with combined bounding boxes.
#[napi]
pub fn search_items(
    items: Vec<JsTextItem>,
    phrase: String,
    case_sensitive: Option<bool>,
) -> Vec<JsTextItem> {
    let rust_items: Vec<_> = items.iter().map(|i| i.to_rust()).collect();
    let options = liteparse::search::SearchOptions {
        phrase,
        case_sensitive: case_sensitive.unwrap_or(false),
    };
    liteparse::search::search_items(&rust_items, &options)
        .iter()
        .map(JsTextItem::from_rust)
        .collect()
}
