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
    /// `id` and `format` the markdown emitter referenced, so the caller can
    /// match them up without parsing markdown.
    pub images: Vec<ExtractedImage>,
    /// Number of embedded image objects that could not be extracted. A bad
    /// image does not fail the rest of the document parse.
    pub image_error_count: u32,
}

/// Result of rendering a single page screenshot.
#[derive(Debug, Clone)]
pub struct ScreenshotResult {
    pub page_num: u32,
    pub width: u32,
    pub height: u32,
    pub image_bytes: Vec<u8>,
}

/// Env var pointing at a fragmented glyph-outline → unicode font database
/// directory (`%02x%02x.msgpack` shards). When set, [`LiteParse::new`]
/// auto-wires a [`crate::FontDbResolver`] so buggy/obfuscated-font glyphs are
/// recovered without any extra wiring. Unset (default) leaves the hook dormant.
#[cfg(not(target_arch = "wasm32"))]
const FONT_DB_DIR_ENV: &str = "LITEPARSE_FONT_DB_DIR";

fn should_extract_images(config: &LiteParseConfig) -> bool {
    config.extract_images
        || matches!(config.image_mode, crate::config::ImageMode::Embed)
        || config.image_output_dir.is_some()
}

#[cfg(not(target_arch = "wasm32"))]
fn write_extracted_images(
    output_dir: &str,
    images: &mut [ExtractedImage],
) -> Result<(), LiteParseError> {
    use std::collections::HashMap;
    use std::path::Path;

    std::fs::create_dir_all(output_dir)?;
    let mut written: HashMap<String, (String, String)> = HashMap::new();
    for image in images {
        if let Some(canonical) = image.duplicate_of.as_ref()
            && let Some((name, path)) = written.get(canonical)
        {
            image.name = name.clone();
            image.path = Some(path.clone());
            continue;
        }

        let path = Path::new(output_dir).join(&image.name);
        std::fs::write(&path, &image.bytes)?;
        let path = path.to_string_lossy().into_owned();
        image.path = Some(path.clone());
        written.insert(image.id.clone(), (image.name.clone(), path));
    }
    Ok(())
}

/// Build the default glyph resolver from the environment, if configured.
#[cfg(not(target_arch = "wasm32"))]
fn default_glyph_resolver() -> Option<std::sync::Arc<dyn crate::GlyphResolver>> {
    let dir = std::env::var_os(FONT_DB_DIR_ENV)?;
    if dir.is_empty() {
        return None;
    }
    Some(std::sync::Arc::new(crate::FontDbResolver::new(dir)))
}

#[cfg(target_arch = "wasm32")]
fn default_glyph_resolver() -> Option<std::sync::Arc<dyn crate::GlyphResolver>> {
    None
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
    /// Optional caller-provided glyph recovery hook. When set, it is consulted
    /// as a last resort for buggy/obfuscated-font glyphs that liteparse's
    /// built-in cmap/AGL recovery could not decode. The published package ships
    /// none; the platform build injects an outline → unicode font-DB resolver.
    glyph_resolver: Option<std::sync::Arc<dyn crate::GlyphResolver>>,
}

impl LiteParse {
    pub fn new(config: LiteParseConfig) -> Self {
        Self {
            config,
            ocr_engine_override: None,
            glyph_resolver: default_glyph_resolver(),
        }
    }

    /// Override the OCR engine. When set, the engine is used regardless of
    /// `ocr_server_url` / built-in Tesseract availability.
    pub fn with_ocr_engine(mut self, engine: std::sync::Arc<dyn OcrEngine>) -> Self {
        self.ocr_engine_override = Some(engine);
        self
    }

    /// Inject a glyph recovery hook. When set, glyphs that liteparse considers
    /// untrusted and cannot decode with its built-in cmap/AGL recovery are
    /// passed to the resolver as vector-outline segments for a final attempt.
    pub fn with_glyph_resolver(
        mut self,
        resolver: std::sync::Arc<dyn crate::GlyphResolver>,
    ) -> Self {
        self.glyph_resolver = Some(resolver);
        self
    }

    /// Parse the configured `target_pages` string (e.g. `"1-5,10"`) into an
    /// explicit page list, or `None` when no selection was configured.
    fn resolve_target_pages(&self) -> Result<Option<Vec<u32>>, LiteParseError> {
        self.config
            .target_pages
            .as_ref()
            .map(|s| parse_target_pages(s))
            .transpose()
            .map_err(|e| format!("invalid --target-pages: {}", e).into())
    }

    /// Determine the complexity of each page in a document, returning a vector
    /// of `PageComplexityStats` for each page. This is useful for deciding
    /// whether to enable OCR on a per-page basis, or for other heuristics.
    pub async fn is_complex(
        &self,
        input: PdfInput,
    ) -> Result<Vec<ocr_merge::PageComplexityStats>, LiteParseError> {
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
        let target_pages = self.resolve_target_pages()?;

        // Load the document and extract text items. Complexity signals derive
        // from the text layer and page objects only — embedded image rasters
        // and hyperlinks are irrelevant here, so both are skipped to keep this
        // pass fast (its whole purpose is a cheap pre-OCR check).
        let password = self.config.password.as_deref();

        let lib = Library::init();
        let document = extract::load_document_from_input(&lib, &validated_input, password)?;

        let (pages, _, _) = extract::extract_pages_and_images(
            &document,
            target_pages.as_deref(),
            self.config.max_pages,
            false, // render_images: image rasters not needed for complexity
            false, // extract_links: irrelevant for complexity stats
            self.glyph_resolver.as_deref(),
            false, // emit_word_boxes: word boxes not needed for complexity stats
        )?;
        let t_extract = web_time::Instant::now();
        log(&format!(
            "[liteparse] extract: {:.1}ms ({} pages)",
            t_extract.duration_since(t0).as_secs_f64() * 1000.0,
            pages.len()
        ));

        let t_complexity = web_time::Instant::now();
        let page_complexities = pages
            .iter()
            .map(|page| {
                let page_obj = document.page((page.page_number - 1) as i32)?;
                ocr_merge::calculate_page_complexity(page, &page_obj)
            })
            .collect::<Result<Vec<_>, _>>()?;
        log(&format!(
            "[liteparse] complexity: {:.1}ms",
            t_complexity.duration_since(t_extract).as_secs_f64() * 1000.0
        ));

        Ok(page_complexities)
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
        let target_pages = self.resolve_target_pages()?;

        // Extract text (and pre-render OCR pages in one PDF load when OCR is on).
        // The PDFium lock is acquired for this entire critical section and
        // released before any `.await` below — OCR (network / CPU) and grid
        // projection (pure Rust) do not touch PDFium, so they can run
        // concurrently with other `LiteParse` calls.
        let password = self.config.password.as_deref();
        let render_images = should_extract_images(&self.config);

        // Build the OCR engine up front so the renderer knows whether to emit a
        // grayscale buffer (cheaper, for engines that binarize internally) or RGB.
        let ocr_engine: Option<std::sync::Arc<dyn OcrEngine>> = if self.config.ocr_enabled {
            Some(if let Some(e) = self.ocr_engine_override.clone() {
                e
            } else {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    if let Some(ref url) = self.config.ocr_server_url {
                        std::sync::Arc::new(
                            HttpOcrEngine::with_headers(
                                url.clone(),
                                self.config.ocr_server_headers.clone(),
                            )
                            .with_retry(
                                crate::ocr::http_simple::OcrRetryConfig {
                                    hedge_delays_ms: self.config.ocr_hedge_delays_ms.clone(),
                                    ..Default::default()
                                },
                            ),
                        )
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
            })
        } else {
            None
        };
        let ocr_grayscale = ocr_engine.as_ref().is_some_and(|e| e.prefers_grayscale());

        let (pages, ocr_rendered, outline, mut images, image_error_count) = {
            let lib = Library::init();
            let document = extract::load_document_from_input(&lib, &validated_input, password)?;
            let outline = extract::extract_outline(&document);
            let (pages, images, image_error_count) = extract::extract_pages_and_images(
                &document,
                target_pages.as_deref(),
                self.config.max_pages,
                render_images,
                self.config.extract_links
                    && self.config.output_format == crate::config::OutputFormat::Markdown,
                self.glyph_resolver.as_deref(),
                self.config.emit_word_boxes,
            )?;
            let t_extract = web_time::Instant::now();
            log(&format!(
                "[liteparse] extract: {:.1}ms ({} pages)",
                t_extract.duration_since(t0).as_secs_f64() * 1000.0,
                pages.len()
            ));
            let rendered = if self.config.ocr_enabled {
                let r = ocr_merge::render_pages_for_ocr(
                    &document,
                    &pages,
                    self.config.dpi,
                    ocr_grayscale,
                )?;
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
            (pages, rendered, outline, images, image_error_count)
        };
        let mut pages = pages;
        let t1 = web_time::Instant::now();

        // OCR pass (engine resolved before the render block above).
        if let Some(engine) = ocr_engine {
            ocr_merge::ocr_and_merge_rendered(
                &mut pages,
                ocr_rendered,
                self.config.dpi,
                engine,
                &self.config.ocr_language,
                self.config.num_workers,
                self.config.ocr_failure_fatal,
            )
            .await?;
        }
        let t_ocr = web_time::Instant::now();
        log(&format!(
            "[liteparse] ocr: {:.1}ms",
            t_ocr.duration_since(t1).as_secs_f64() * 1000.0
        ));

        // Caller-requested content filters (page-region crop, diagonal-text
        // removal). Runs after OCR merge so it also drops OCR text outside the
        // crop region, and before projection so filtered items never surface.
        extract::apply_content_filters(
            &mut pages,
            self.config.crop_box.as_ref(),
            self.config.skip_diagonal_text,
        );

        // Grid projection
        let mut parsed_pages = projection::project_pages_to_grid(pages);
        let t2 = web_time::Instant::now();
        log(&format!(
            "[liteparse] project: {:.1}ms",
            t2.duration_since(t_ocr).as_secs_f64() * 1000.0
        ));

        let full_text = if self.config.output_format == crate::config::OutputFormat::Markdown {
            let page_md =
                markdown::format_markdown_pages(&parsed_pages, &outline, self.config.image_mode);
            let md = page_md.join("\n\n-----\n\n");
            for (page, md) in parsed_pages.iter_mut().zip(page_md) {
                page.markdown = md;
            }
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

        #[cfg(not(target_arch = "wasm32"))]
        if let Some(output_dir) = self.config.image_output_dir.as_deref() {
            write_extracted_images(output_dir, &mut images)?;
        }

        Ok(ParseResult {
            pages: parsed_pages,
            text: full_text,
            outline,
            images,
            image_error_count,
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
        let mut parsed_pages = projection::project_pages_to_grid(pages);

        let full_text = if self.config.output_format == crate::config::OutputFormat::Markdown {
            let page_md =
                markdown::format_markdown_pages(&parsed_pages, &outline, self.config.image_mode);
            let md = page_md.join("\n\n-----\n\n");
            for (page, md) in parsed_pages.iter_mut().zip(page_md) {
                page.markdown = md;
            }
            md
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
            image_error_count: 0,
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
    fn image_extraction_is_opt_in_with_compatibility_implications() {
        let default = LiteParseConfig::default();
        assert!(!should_extract_images(&default));

        let explicit = LiteParseConfig {
            extract_images: true,
            ..Default::default()
        };
        assert!(should_extract_images(&explicit));

        let embed = LiteParseConfig {
            image_mode: crate::config::ImageMode::Embed,
            ..Default::default()
        };
        assert!(should_extract_images(&embed));

        let output_dir = LiteParseConfig {
            image_output_dir: Some("images".into()),
            ..Default::default()
        };
        assert!(should_extract_images(&output_dir));
    }

    #[test]
    fn image_output_reuses_canonical_file_for_duplicates() {
        fn image(id: &str, duplicate_of: Option<&str>, bytes: &[u8]) -> ExtractedImage {
            ExtractedImage {
                id: id.into(),
                name: format!("image_{id}.png"),
                path: None,
                page: 1,
                bbox: crate::types::Rect::default(),
                width: 2,
                height: 2,
                rotation: 0.0,
                format: "png".into(),
                duplicate_of: duplicate_of.map(str::to_owned),
                bytes: bytes.to_vec(),
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let mut images = vec![
            image("p1_0", None, b"canonical"),
            image("p2_0", Some("p1_0"), b"canonical"),
        ];
        write_extracted_images(dir.path().to_str().unwrap(), &mut images).unwrap();

        assert_eq!(images[0].name, images[1].name);
        assert_eq!(images[0].path, images[1].path);
        assert_eq!(
            std::fs::read(images[0].path.as_ref().unwrap()).unwrap(),
            b"canonical"
        );
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}
