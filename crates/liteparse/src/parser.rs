use crate::config::{LiteParseConfig, parse_target_pages};
#[cfg(not(target_arch = "wasm32"))]
use crate::conversion;
use crate::error::LiteParseError;
use crate::extract;
use crate::ocr::OcrEngine;
#[cfg(not(target_arch = "wasm32"))]
use crate::ocr::http_simple::HttpOcrEngine;
#[cfg(feature = "tesseract")]
use crate::ocr::tesseract::TesseractOcrEngine;
use crate::ocr_merge;
use crate::output::markdown;
use crate::projection;
#[cfg(not(target_arch = "wasm32"))]
use crate::render;
use crate::types::{ExtractedImage, OutlineTarget, Page, ParsedPage, PdfInput};
use base64::Engine;
use pdfium::Library;

/// Result of parsing a document.
pub struct ParseResult {
    /// Parsed pages with projected text layout.
    pub pages: Vec<ParsedPage>,
    /// Full document text, concatenated from all pages.
    pub text: String,
    /// Document outline (bookmarks) when present. Used by the markdown
    /// emitter as a high-priority heading source on untagged PDFs.
    pub outline: Vec<OutlineTarget>,
    /// Raster images extracted from the document. Empty unless the parser
    /// was configured with `ImageMode::Embed`. Each entry carries the same
    /// `id` the markdown emitter referenced in `![](image_{id}.png)`, so the
    /// caller can match them up without parsing markdown.
    pub images: Vec<ExtractedImage>,
}

/// Result of rendering a single page screenshot.
#[derive(Debug, Clone)]
pub struct ScreenshotResult {
    pub page_num: u32,
    pub width: u32,
    pub height: u32,
    pub image_bytes: Vec<u8>,
}

/// Main LiteParse orchestrator.
///
/// ### Thread safety
///
/// `LiteParse` is `Send + Sync` and safe to share across threads (e.g.
/// behind an `Arc`, or used concurrently from a multi-threaded `tokio`
/// runtime).
///
/// PDFium itself is **not** thread-safe, so all PDFium FFI work — document
/// loading, page rendering, text extraction — is serialized through a
/// process-global lock held by [`pdfium::Library`]. From a caller's
/// perspective, this means concurrent `parse_*` / `screenshot*` calls are
/// safe but their PDFium portions run sequentially. The OCR pass and grid
/// projection (which dominate runtime for OCR-heavy documents) run outside
/// the lock and remain fully concurrent.
pub struct LiteParse {
    config: LiteParseConfig,
    /// Optional caller-provided OCR engine. When set, this overrides the
    /// built-in selection logic (HTTP OCR / Tesseract). This is the primary
    /// mechanism for plugging an OCR engine in environments without the
    /// built-ins (e.g. WASM, where the JS side supplies a callback engine).
    ocr_engine_override: Option<std::sync::Arc<dyn OcrEngine>>,
}

impl LiteParse {
    pub fn new(config: LiteParseConfig) -> Self {
        Self {
            config,
            ocr_engine_override: None,
        }
    }

    /// Override the OCR engine. When set, the engine is used regardless of
    /// `ocr_server_url` / built-in Tesseract availability.
    pub fn with_ocr_engine(mut self, engine: std::sync::Arc<dyn OcrEngine>) -> Self {
        self.ocr_engine_override = Some(engine);
        self
    }

    /// Parse a document from a file path, returning structured results.
    ///
    /// Non-PDF files are automatically converted to PDF first (requires
    /// LibreOffice/ImageMagick on the system).
    ///
    /// Not available on `wasm32` — the browser has no filesystem. Use
    /// [`LiteParse::parse_input`] with [`PdfInput::Bytes`] instead.
    #[cfg(not(target_arch = "wasm32"))]
    pub async fn parse(&self, input: &str) -> Result<ParseResult, LiteParseError> {
        self.parse_input(PdfInput::Path(input.to_string())).await
    }

    /// Parse a document from either a file path or raw bytes.
    ///
    /// Use `PdfInput::Path` for files on disk or `PdfInput::Bytes` for
    /// in-memory PDF data (e.g. from a network response or Node.js Buffer).
    pub async fn parse_input(&self, input: PdfInput) -> Result<ParseResult, LiteParseError> {
        let log = |msg: &str| {
            if !self.config.quiet {
                eprintln!("{}", msg);
            }
        };

        let t0 = web_time::Instant::now();

        #[cfg(not(target_arch = "wasm32"))]
        let (validated_input, _guard) =
            conversion::resolve_pdf_input(input, self.config.password.as_deref(), false).await?;

        #[cfg(target_arch = "wasm32")]
        let validated_input = input;

        // Determine which pages to extract
        let target_pages = self
            .config
            .target_pages
            .as_ref()
            .map(|s| parse_target_pages(s))
            .transpose()
            .map_err(|e| format!("invalid --target-pages: {}", e))?;

        // Extract text (and pre-render OCR pages in one PDF load when OCR is on).
        // The PDFium lock is acquired for this entire critical section and
        // released before any `.await` below — OCR (network / CPU) and grid
        // projection (pure Rust) do not touch PDFium, so they can run
        // concurrently with other `LiteParse` calls.
        let password = self.config.password.as_deref();
        let render_images = matches!(self.config.image_mode, crate::config::ImageMode::Embed)
            || self.config.inline_images;
        let (pages, ocr_rendered, outline, images) = {
            let lib = Library::init();
            let document = extract::load_document_from_input(&lib, &validated_input, password)?;
            let outline = extract::extract_outline(&document);
            let (pages, images) = extract::extract_pages_and_images(
                &document,
                target_pages.as_deref(),
                self.config.max_pages,
                render_images,
                self.config.extract_links
                    && self.config.output_format == crate::config::OutputFormat::Markdown,
            )?;
            let t_extract = web_time::Instant::now();
            log(&format!(
                "[liteparse] extract: {:.1}ms ({} pages)",
                t_extract.duration_since(t0).as_secs_f64() * 1000.0,
                pages.len()
            ));
            let rendered = if self.config.ocr_enabled {
                let r = ocr_merge::render_pages_for_ocr(&document, &pages, self.config.dpi)?;
                log(&format!(
                    "[liteparse] ocr render: {:.1}ms ({} pages)",
                    web_time::Instant::now()
                        .duration_since(t_extract)
                        .as_secs_f64()
                        * 1000.0,
                    r.len()
                ));
                r
            } else {
                Vec::new()
            };
            // `lib` is dropped here, releasing the PDFium lock.
            (pages, rendered, outline, images)
        };
        let mut pages = pages;
        let t1 = web_time::Instant::now();

        // OCR pass
        if self.config.ocr_enabled {
            let engine: std::sync::Arc<dyn OcrEngine> = if let Some(e) =
                self.ocr_engine_override.clone()
            {
                e
            } else {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    if let Some(ref url) = self.config.ocr_server_url {
                        std::sync::Arc::new(HttpOcrEngine::with_headers(
                            url.clone(),
                            self.config.ocr_server_headers.clone(),
                        ))
                    } else {
                        #[cfg(feature = "tesseract")]
                        {
                            std::sync::Arc::new(TesseractOcrEngine::new(
                                self.config.tessdata_path.clone(),
                            ))
                        }
                        #[cfg(not(feature = "tesseract"))]
                        {
                            return Err("OCR enabled but no --ocr-server-url provided and tesseract feature is disabled".into());
                        }
                    }
                }
                #[cfg(target_arch = "wasm32")]
                {
                    return Err(
                        "OCR enabled but no `ocrEngine` callback was provided (WASM builds have no built-in OCR engine)".into(),
                    );
                }
            };
            ocr_merge::ocr_and_merge_rendered(
                &mut pages,
                ocr_rendered,
                self.config.dpi,
                engine,
                &self.config.ocr_language,
                self.config.num_workers,
            )
            .await?;
        }
        let t_ocr = web_time::Instant::now();
        log(&format!(
            "[liteparse] ocr: {:.1}ms",
            t_ocr.duration_since(t1).as_secs_f64() * 1000.0
        ));

        // Grid projection
        let parsed_pages = projection::project_pages_to_grid(pages);
        let t2 = web_time::Instant::now();
        log(&format!(
            "[liteparse] project: {:.1}ms",
            t2.duration_since(t_ocr).as_secs_f64() * 1000.0
        ));

        let full_text = if self.config.output_format == crate::config::OutputFormat::Markdown
            || self.config.inline_images
        {
            let image_mode = if self.config.inline_images {
                crate::config::ImageMode::Placeholder
            } else {
                self.config.image_mode
            };
            let md = markdown::format_markdown(&parsed_pages, &outline, image_mode);
            let md = if self.config.inline_images {
                inline_markdown_images(md, &images)
            } else {
                md
            };
            let t3 = web_time::Instant::now();
            log(&format!(
                "[liteparse] markdown: {:.1}ms",
                t3.duration_since(t2).as_secs_f64() * 1000.0
            ));
            md
        } else {
            parsed_pages
                .iter()
                .map(|p| p.text.as_str())
                .collect::<Vec<_>>()
                .join("\n\n")
        };

        let total = web_time::Instant::now().duration_since(t0).as_secs_f64() * 1000.0;
        log(&format!("[liteparse] total: {:.1}ms", total));

        Ok(ParseResult {
            pages: parsed_pages,
            text: full_text,
            outline,
            images,
        })
    }

    /// Parse from pre-extracted pages, skipping PDFium text extraction.
    ///
    /// The caller supplies `Page`s already populated with text items (and,
    /// optionally, graphics / struct nodes / image refs) in viewport space
    /// (top-left origin, 72 DPI). This runs only grid projection and the
    /// configured output formatter, so it touches neither PDFium nor OCR and
    /// is fully synchronous. Used when an external extractor (e.g. with its
    /// own font-recovery pipeline) owns text extraction.
    pub fn parse_from_pages(&self, pages: Vec<Page>, outline: Vec<OutlineTarget>) -> ParseResult {
        let parsed_pages = projection::project_pages_to_grid(pages);

        let full_text = if self.config.output_format == crate::config::OutputFormat::Markdown {
            markdown::format_markdown(&parsed_pages, &outline, self.config.image_mode)
        } else {
            parsed_pages
                .iter()
                .map(|p| p.text.as_str())
                .collect::<Vec<_>>()
                .join("\n\n")
        };

        ParseResult {
            pages: parsed_pages,
            text: full_text,
            outline,
            images: Vec::new(),
        }
    }

    /// Generate screenshots of document pages as PNG bytes.
    ///
    /// Non-PDF files are automatically converted to PDF first (requires
    /// LibreOffice/ImageMagick on the system). Plain-text formats cannot be
    /// rendered and return a clear error.
    #[cfg(not(target_arch = "wasm32"))]
    pub async fn screenshot(
        &self,
        input: &str,
        page_numbers: Option<Vec<u32>>,
    ) -> Result<Vec<ScreenshotResult>, LiteParseError> {
        self.screenshot_input(PdfInput::Path(input.to_string()), page_numbers)
            .await
    }

    /// Generate screenshots from a file path or raw bytes.
    #[cfg(not(target_arch = "wasm32"))]
    pub async fn screenshot_input(
        &self,
        input: PdfInput,
        page_numbers: Option<Vec<u32>>,
    ) -> Result<Vec<ScreenshotResult>, LiteParseError> {
        let log = |msg: &str| {
            if !self.config.quiet {
                eprintln!("{}", msg);
            }
        };

        let (validated_input, _guard) =
            conversion::resolve_pdf_input(input, self.config.password.as_deref(), true).await?;

        if let PdfInput::Path(ref path) = validated_input
            && !conversion::is_pdf(path)
        {
            log("[liteparse] converted input to PDF for screenshot rendering");
        }

        let rendered = render::render_pages_to_png(
            &validated_input,
            page_numbers.as_deref(),
            self.config.dpi,
            self.config.password.as_deref(),
        )?;

        Ok(rendered
            .into_iter()
            .map(|page| ScreenshotResult {
                page_num: page.page_num,
                width: page.width,
                height: page.height,
                image_bytes: page.png_bytes,
            })
            .collect())
    }

    pub fn config(&self) -> &LiteParseConfig {
        &self.config
    }
}

fn inline_markdown_images(mut markdown: String, images: &[ExtractedImage]) -> String {
    for image in images {
        let mime_type = match image.format.as_str() {
            "jpg" | "jpeg" => "image/jpeg",
            "webp" => "image/webp",
            _ => "image/png",
        };
        let encoded = base64::engine::general_purpose::STANDARD.encode(&image.bytes);
        let replacement = format!("![](data:{mime_type};base64,{encoded})");
        markdown = markdown.replace(
            &format!("![](image_{}.{})", image.id, image.format),
            &replacement,
        );
    }
    markdown
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(clippy::field_reassign_with_default)]
    fn test_new_stores_config() {
        let mut cfg = LiteParseConfig::default();
        cfg.ocr_enabled = false;
        cfg.max_pages = 7;
        let lp = LiteParse::new(cfg);
        assert!(!lp.config().ocr_enabled);
        assert_eq!(lp.config().max_pages, 7);
    }

    #[test]
    fn inline_markdown_images_replaces_image_refs_with_data_uris() {
        let markdown = "before ![](image_p1_0.png) after".to_string();
        let images = vec![ExtractedImage {
            id: "p1_0".into(),
            page: 1,
            bbox: crate::types::Rect {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            },
            format: "png".into(),
            bytes: b"hi".to_vec(),
        }];
        let out = inline_markdown_images(markdown, &images);
        assert_eq!(out, "before ![](data:image/png;base64,aGk=) after");
    }
}
