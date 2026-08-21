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
use crate::render;
use crate::types::{
    DocumentMetadata, ExtractedImage, OutlineTarget, Page, PageError, ParsedPage, PdfInput,
    ScreenshotRect, XfaPacket,
};
use pdfium::Library;

/// Result of parsing a document.
pub struct ParseResult {
    /// Total number of pages in the source document, before `target_pages` or
    /// `max_pages` limits are applied.
    pub total_pages: u32,
    /// Parsed pages with projected text layout.
    pub pages: Vec<ParsedPage>,
    /// Page-level PDFium extraction failures collected when
    /// `continue_on_page_error` is enabled.
    pub page_errors: Vec<PageError>,
    /// Full document text, concatenated from all pages.
    pub text: String,
    /// Document outline (bookmarks) when present. Used by the markdown
    /// emitter as a high-priority heading source on untagged PDFs.
    pub outline: Vec<OutlineTarget>,
    /// Raster images extracted from the document. Empty unless the parser
    /// was configured with `extract_images`. Each entry carries the same
    /// `id` and `format` the markdown emitter referenced, so the caller can
    /// match them up without parsing markdown.
    pub images: Vec<ExtractedImage>,
    /// Page screenshots encoded as PNG. Empty unless `extract_screenshots`
    /// is enabled.
    pub screenshots: Vec<ScreenshotResult>,
    /// Number of embedded image objects that could not be extracted. A bad
    /// image does not fail the rest of the document parse.
    pub image_error_count: u32,
    /// PDFium form type (0 none, 1 AcroForm, 2 XFA full, 3 XFA foreground),
    /// present only when form-field extraction is enabled.
    pub form_type: Option<i32>,
    /// The document's `/Info` `Creator` entry, when present.
    pub creator: Option<String>,
    /// The document's `/Info` `Producer` entry, when present.
    pub producer: Option<String>,
    /// Document provenance metadata (dates, version/security, signatures,
    /// incremental-save markers, trailer IDs, raw XMP, and source size).
    /// Present only when `extract_document_metadata` is enabled, and `None`
    /// for inputs converted from a non-PDF format.
    pub doc_meta: Option<DocumentMetadata>,
    /// Raw XFA packets, present only when `extract_xfa_packets` is enabled.
    /// `Some([])` means extraction ran on a non-XFA document.
    pub xfa_packets: Option<Vec<XfaPacket>>,
}

/// Result of rendering a single page screenshot.
#[derive(Debug, Clone)]
pub struct ScreenshotResult {
    pub page_num: u32,
    pub width: u32,
    pub height: u32,
    pub image_bytes: Vec<u8>,
    /// True when every pixel has the same color (blank page after render).
    pub is_solid_fill: bool,
    /// Solid rectangles/lines detected in the raster (viewport coords).
    /// Populated only when `LiteParseConfig::detect_screenshot_rects` is on.
    pub rects: Vec<ScreenshotRect>,
}

/// Env var pointing at a fragmented glyph-outline → unicode font database
/// directory (`%02x%02x.msgpack` shards). When set, [`LiteParse::new`]
/// auto-wires a [`crate::FontDbResolver`] so buggy/obfuscated-font glyphs are
/// recovered without any extra wiring. Unset (default) leaves the hook dormant.
#[cfg(not(target_arch = "wasm32"))]
const FONT_DB_DIR_ENV: &str = "LITEPARSE_FONT_DB_DIR";

#[cfg(not(target_arch = "wasm32"))]
fn write_extracted_images(
    output_dir: &str,
    images: &mut [ExtractedImage],
) -> Result<(), LiteParseError> {
    use std::collections::HashMap;
    use std::path::Path;

    std::fs::create_dir_all(output_dir)?;
    // Platform contract (mirrors the LlamaParse C extractor, which the worker
    // pipeline is built around): only the canonical file is written; every
    // duplicate placement keeps its own `name` but points `path` at the
    // canonical file. Markdown figure references are rewritten to the
    // canonical name (`rewrite_duplicate_image_refs`) so they only ever
    // reference files that exist.
    let mut written: HashMap<String, String> = HashMap::new();
    for image in images {
        if let Some(canonical) = image.duplicate_of.as_ref()
            && let Some(path) = written.get(canonical)
        {
            image.path = Some(path.clone());
            continue;
        }

        let path = Path::new(output_dir).join(&image.name);
        std::fs::write(&path, image.bytes.as_slice())?;
        let path = path.to_string_lossy().into_owned();
        image.path = Some(path.clone());
        written.insert(image.id.clone(), path);
    }
    Ok(())
}

/// Rewrite markdown figure references for deduplicated images to the
/// canonical entry's file name. The markdown emitter references each figure
/// by its own placement id (`![](img_p2_1.jpg)`), but only the canonical
/// file is written to disk (see `write_extracted_images`), so duplicate
/// placements must reference the canonical name, matching the resolution the
/// LlamaParse worker applies via `resolveOutputImageName`.
fn rewrite_duplicate_image_refs(
    pages: &mut [ParsedPage],
    full_text: &mut String,
    images: &[ExtractedImage],
) {
    use std::collections::HashMap;

    let by_id: HashMap<&str, &ExtractedImage> = images
        .iter()
        .map(|image| (image.id.as_str(), image))
        .collect();
    let renames: Vec<(String, String)> = images
        .iter()
        .filter_map(|image| {
            let canonical = by_id.get(image.duplicate_of.as_ref()?.as_str())?;
            Some((
                format!("![](img_{}.{})", image.id, image.format),
                format!("![]({})", canonical.name),
            ))
        })
        .collect();
    if renames.is_empty() {
        return;
    }

    for markdown in pages
        .iter_mut()
        .map(|page| &mut page.markdown)
        .chain(std::iter::once(full_text))
    {
        for (from, to) in &renames {
            if markdown.contains(from.as_str()) {
                *markdown = markdown.replace(from.as_str(), to);
            }
        }
    }
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

/// A document input already converted to PDF, if it needed converting.
///
/// Holds the [`conversion::PdfInputGuard`] so the temporary file produced for
/// a DOCX/XLSX/PPTX/image source stays alive for as long as the resolved input
/// is usable. Reusing one of these across several parses is what keeps batch
/// parsing from re-running LibreOffice for every batch.
pub(crate) struct ResolvedInput {
    input: PdfInput,
    #[cfg(not(target_arch = "wasm32"))]
    guard: conversion::PdfInputGuard,
}

impl ResolvedInput {
    /// True when `input` points at a temporary PDF we produced, so raw-file
    /// provenance describes the intermediate rather than the caller's document.
    fn is_converted(&self) -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.guard.is_converted()
        }
        #[cfg(target_arch = "wasm32")]
        {
            false
        }
    }
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
#[derive(Clone)]
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

/// Run block classification once and fan the result out to everything that
/// needs it: the rendered per-page markdown (when Markdown is the output
/// format) and each page's `blocks` (when `extract_blocks` is on).
///
/// Classification is the expensive part of the markdown pipeline, so the two
/// consumers share a single pass rather than each triggering their own. Returns
/// the joined document markdown when that is the output format.
fn apply_layout(
    config: &crate::config::LiteParseConfig,
    parsed_pages: &mut [ParsedPage],
    outline: &[OutlineTarget],
) -> Option<String> {
    let wants_markdown = config.output_format == crate::config::OutputFormat::Markdown;
    if !wants_markdown && !config.extract_blocks {
        return None;
    }
    let classified = markdown::classify_document(
        parsed_pages,
        outline,
        config.image_mode,
        config.keep_headers_footers,
    );
    if config.extract_blocks {
        for (page, blocks) in parsed_pages.iter_mut().zip(&classified) {
            // A page with no structural decomposition reports an empty list,
            // not `None` — extraction *was* enabled, there was just nothing to
            // decompose.
            let mut layout_blocks: Vec<crate::layout::LayoutBlock> = blocks
                .as_deref()
                .unwrap_or_default()
                .iter()
                .map(crate::layout::LayoutBlock::from)
                .collect();
            // Bounds safety at the public boundary: every provenance index
            // must address this page's returned text_items. Should never
            // fire (indices are recorded against the same vector the page
            // returns); in release an out-of-range index is dropped rather
            // than shipped.
            let n_items = page.text_items.len();
            for block in &mut layout_blocks {
                debug_assert!(
                    block.text_item_indices.iter().all(|&i| i < n_items),
                    "block provenance index out of range (page {}: {} items)",
                    page.page_number,
                    n_items
                );
                block.text_item_indices.retain(|&i| i < n_items);
                for cell in block
                    .header
                    .iter_mut()
                    .flatten()
                    .chain(block.rows.iter_mut().flatten().flatten())
                {
                    debug_assert!(
                        cell.text_item_indices.iter().all(|&i| i < n_items),
                        "cell provenance index out of range (page {}: {} items)",
                        page.page_number,
                        n_items
                    );
                    cell.text_item_indices.retain(|&i| i < n_items);
                }
            }
            page.blocks = Some(layout_blocks);
        }
    }
    if !wants_markdown {
        return None;
    }
    let page_md = markdown::render_classified(parsed_pages, &classified);
    let md = page_md.join("\n\n-----\n\n");
    for (page, page_md) in parsed_pages.iter_mut().zip(page_md) {
        page.markdown = page_md;
    }
    Some(md)
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

    fn validate_output_config(&self) -> Result<(), LiteParseError> {
        if self.config.image_output_dir.is_some() && !self.config.effective_extract_images() {
            return Err(LiteParseError::Config(
                "image_output_dir requires extract_images = true (or image_mode = embed)"
                    .to_string(),
            ));
        }
        Ok(())
    }

    /// Determine the complexity of each page in a document, returning a vector
    /// of `PageComplexityStats` for each page. This is useful for deciding
    /// whether to enable OCR on a per-page basis, or for other heuristics.
    ///
    /// Besides the OCR-need signals, each entry carries `layout` signals
    /// (multi-column, ruled tables, dense graphics) computed by running the
    /// real grid-projection pass — useful for routing pages to a
    /// higher-accuracy pipeline even when no OCR is needed.
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

        let (pages, mut page_complexities) = {
            let lib = Library::init();
            let document = extract::load_document_from_input(&lib, &validated_input, password)?;

            // Complexity deliberately runs against the flattened document: once
            // widget text lives in the content stream it is genuinely within
            // PDFium's reach, so it should count toward the page's text budget
            // instead of routing the page to OCR to recover text we already
            // have. `AnnotationText` still fires for the non-widget appearance
            // text it was introduced for.
            let pages = extract::extract_pages_and_images(
                &document,
                target_pages.as_deref(),
                self.config.max_pages,
                false, // extract_links: irrelevant for complexity stats
                self.glyph_resolver.as_deref(),
                extract::ExtractionOutputOptions {
                    continue_on_page_error: self.config.continue_on_page_error,
                    ..Default::default()
                },
            )?
            .pages;
            let t_extract = web_time::Instant::now();
            log(&format!(
                "[liteparse] extract: {:.1}ms ({} pages)",
                t_extract.duration_since(t0).as_secs_f64() * 1000.0,
                pages.len()
            ));

            // In tolerant mode a page whose stats fail is dropped from both
            // vectors (consumers match stats to pages by `page_number`), so
            // the layout zip below stays aligned.
            let mut kept_pages = Vec::with_capacity(pages.len());
            let mut page_complexities = Vec::with_capacity(pages.len());
            for page in pages {
                let stats = document
                    .page((page.page_number - 1) as i32)
                    .map_err(LiteParseError::from)
                    .and_then(|page_obj| ocr_merge::calculate_page_complexity(&page, &page_obj));
                match stats {
                    Ok(stats) => {
                        page_complexities.push(stats);
                        kept_pages.push(page);
                    }
                    Err(error) if self.config.continue_on_page_error => log(&format!(
                        "[liteparse] complexity failed on page {}: {}",
                        page.page_number, error
                    )),
                    Err(error) => return Err(error),
                }
            }
            let pages = kept_pages;
            log(&format!(
                "[liteparse] complexity: {:.1}ms",
                web_time::Instant::now()
                    .duration_since(t_extract)
                    .as_secs_f64()
                    * 1000.0
            ));
            // `lib` is dropped here, releasing the PDFium lock; the layout
            // pass below is pure CPU over the already-extracted items.
            (pages, page_complexities)
        };

        // Layout signals come from the real projection pass so they match
        // what a full parse will decide.
        let t_layout = web_time::Instant::now();
        let parsed_pages = projection::project_pages_to_grid(pages);
        for (stats, page) in page_complexities.iter_mut().zip(&parsed_pages) {
            stats.layout = Some(ocr_merge::calculate_layout_complexity(page));
        }
        log(&format!(
            "[liteparse] layout: {:.1}ms",
            web_time::Instant::now()
                .duration_since(t_layout)
                .as_secs_f64()
                * 1000.0
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
        self.validate_output_config()?;
        let resolved = self.resolve_input(input).await?;
        let target_pages = self.resolve_target_pages()?;
        self.parse_resolved(
            &resolved,
            target_pages.as_deref(),
            self.config.max_pages,
            None,
        )
        .await
    }

    /// Convert a non-PDF input to PDF (if needed) and return it alongside the
    /// guard that keeps any temporary file alive.
    ///
    /// Split out of [`LiteParse::parse_input`] so [`ParseSession`] can pay the
    /// conversion cost once and reuse the result for every page batch.
    async fn resolve_input(&self, input: PdfInput) -> Result<ResolvedInput, LiteParseError> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let (input, guard) =
                conversion::resolve_pdf_input(input, self.config.password.as_deref(), false)
                    .await?;
            Ok(ResolvedInput { input, guard })
        }
        #[cfg(target_arch = "wasm32")]
        {
            Ok(ResolvedInput { input })
        }
    }

    /// Parse an already-resolved input over an explicit page selection.
    ///
    /// `target_pages` and `max_pages` are parameters rather than config reads
    /// so batch parsing can narrow the selection per batch.
    ///
    /// `outline` lets a caller that already walked the bookmark tree supply it
    /// instead of paying for it again. The walk resolves a page destination per
    /// entry, which costs several times more than opening the document, so a
    /// batch parse that recomputed it per batch would spend most of its
    /// overhead there.
    async fn parse_resolved(
        &self,
        resolved: &ResolvedInput,
        target_pages: Option<&[u32]>,
        max_pages: usize,
        outline: Option<Vec<OutlineTarget>>,
    ) -> Result<ParseResult, LiteParseError> {
        let log = |msg: &str| {
            if !self.config.quiet {
                eprintln!("{}", msg);
            }
        };

        let t0 = web_time::Instant::now();

        // Provenance facts describe the file on disk, so they are meaningless
        // for a PDF we generated ourselves from a DOCX/XLSX/image.
        let want_doc_meta = self.config.extract_document_metadata && !resolved.is_converted();

        let validated_input = &resolved.input;

        // Extract text (and pre-render OCR pages in one PDF load when OCR is on).
        // The PDFium lock is acquired for this entire critical section and
        // released before any `.await` below — OCR (network / CPU) and grid
        // projection (pure Rust) do not touch PDFium, so they can run
        // concurrently with other `LiteParse` calls.
        let password = self.config.password.as_deref();
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

        #[allow(unused_mut)] // mutated only by the native image-output writer
        let (
            pages,
            page_errors,
            total_pages,
            outline,
            mut images,
            screenshots,
            image_error_count,
            complexity,
            form_type,
            creator,
            producer,
            doc_meta,
            xfa_packets,
            flattened_form_widgets,
            flattened_page_numbers,
            repaired_input,
        ) = {
            let lib = Library::init();
            #[cfg(not(target_arch = "wasm32"))]
            let repaired_input = self
                .config
                .extract_form_fields
                .then(|| {
                    crate::acroform_repair::repair_orphaned_widgets(&lib, validated_input, password)
                })
                .flatten();
            #[cfg(not(target_arch = "wasm32"))]
            let document_input = repaired_input.as_ref().unwrap_or(validated_input);
            #[cfg(target_arch = "wasm32")]
            let document_input = validated_input;
            let document = extract::load_document_from_input(&lib, document_input, password)?;
            let total_pages = document.page_count().max(0) as u32;
            let form_type = self
                .config
                .extract_form_fields
                .then(|| document.form_type());
            let creator = document.meta_text("Creator");
            let producer = document.meta_text("Producer");
            let doc_meta = want_doc_meta.then(|| {
                // AcroForm repair rewrites the file, so provenance has to come
                // from the original document; fall back if it no longer loads.
                #[cfg(not(target_arch = "wasm32"))]
                if repaired_input.is_some()
                    && let Ok(source) =
                        extract::load_document_from_input(&lib, validated_input, password)
                {
                    return crate::document_metadata::extract(validated_input, &source);
                }
                crate::document_metadata::extract(validated_input, &document)
            });
            let xfa_packets = self.config.extract_xfa_packets.then(|| {
                document
                    .xfa_packets()
                    .into_iter()
                    .map(|packet| XfaPacket {
                        index: packet.index.max(0) as u32,
                        name: packet.name,
                        content_length: packet
                            .content
                            .as_ref()
                            .map_or(0, |content| content.len() as u32),
                        content: packet
                            .content
                            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned()),
                    })
                    .collect::<Vec<_>>()
            });
            let outline = outline.unwrap_or_else(|| extract::extract_outline(&document));
            let extracted = extract::extract_pages_and_images(
                &document,
                target_pages,
                max_pages,
                self.config.extract_links
                    && self.config.output_format == crate::config::OutputFormat::Markdown,
                self.glyph_resolver.as_deref(),
                extract::ExtractionOutputOptions {
                    continue_on_page_error: self.config.continue_on_page_error,
                    extract_content_bounds: self.config.extract_content_bounds,
                    extract_images: self.config.effective_extract_images(),
                    // The table detector splits PDFium's merged multi-cell
                    // runs on real word geometry, so it needs word boxes even
                    // when the caller didn't ask for them. Markdown output
                    // always runs it; `extract_blocks` runs the same
                    // classifier under any output format, so without the
                    // forcing a JSON caller would get blocks classified by a
                    // materially weaker (char-estimate) table splitter than
                    // the markdown they can see. Detection-only: `self.config`
                    // is left untouched, so `config()` still reports what the
                    // caller asked for, and bindings that expose
                    // `TextItem::words` gate serialization on that request.
                    emit_word_boxes: self.config.emit_word_boxes
                        || self.config.output_format == crate::config::OutputFormat::Markdown
                        || self.config.extract_blocks,
                    extract_text_metadata: self.config.extract_text_metadata,
                    extract_vector_graphics: self.config.extract_vector_graphics,
                    extract_annotations: self.config.extract_annotations,
                    extract_form_fields: self.config.extract_form_fields,
                    extract_structure_tree: self.config.extract_structure_tree,
                },
            )?;
            let extract::ExtractedPages {
                pages,
                page_errors,
                images,
                image_error_count,
                flattened_form_widgets,
                flattened_page_numbers,
            } = extracted;
            // Reopening the input costs a full parse, so it is confined to the
            // one consumer that genuinely needs live widget annotations: the
            // opt-in form renderer, which initializes the form environment to
            // run document actions and paint computed field appearances that
            // have no appearance stream to flatten.
            //
            // Plain rendering (screenshots) does not need it — flattening
            // promotes the widget appearances into page content, so the
            // raster is the same either way. Complexity likewise runs on the
            // flattened document by design (see `is_complex`). OCR rasters
            // render in bounded rounds after this section, each against a
            // freshly reopened (hence pristine) document.
            let needs_pristine_document = flattened_form_widgets
                && self.config.extract_screenshots
                && self.config.render_form_fields;
            let pristine_document = needs_pristine_document
                .then(|| extract::load_document_from_input(&lib, document_input, password))
                .transpose()?;
            let analysis_document = pristine_document.as_ref().unwrap_or(&document);
            let t_extract = web_time::Instant::now();
            log(&format!(
                "[liteparse] extract: {:.1}ms ({} pages)",
                t_extract.duration_since(t0).as_secs_f64() * 1000.0,
                pages.len()
            ));
            let complexity = if self.config.include_complexity {
                let mut complexity = Vec::with_capacity(pages.len());
                for page in &pages {
                    let stats = analysis_document
                        .page((page.page_number - 1) as i32)
                        .map_err(LiteParseError::from)
                        .and_then(|page_obj| ocr_merge::calculate_page_complexity(page, &page_obj));
                    match stats {
                        Ok(stats) => complexity.push(stats),
                        // The page's text is already extracted; a tolerant
                        // parse keeps it and just leaves `complexity` unset
                        // (stats attach by page number below).
                        Err(error) if self.config.continue_on_page_error => log(&format!(
                            "[liteparse] complexity failed on page {}: {}",
                            page.page_number, error
                        )),
                        Err(error) => return Err(error),
                    }
                }
                complexity
            } else {
                Vec::new()
            };
            let screenshots = if self.config.extract_screenshots {
                let page_numbers = pages
                    .iter()
                    .map(|page| page.page_number as u32)
                    .collect::<Vec<_>>();
                render::render_document_pages(
                    analysis_document,
                    Some(&page_numbers),
                    self.config.dpi,
                    self.config.detect_screenshot_rects,
                    self.config.render_form_fields,
                    self.config.continue_on_page_error,
                )?
                .into_iter()
                .map(|page| ScreenshotResult {
                    page_num: page.page_num,
                    width: page.width,
                    height: page.height,
                    image_bytes: page.png_bytes,
                    is_solid_fill: page.is_solid_fill,
                    rects: page.rects,
                })
                .collect()
            } else {
                Vec::new()
            };
            #[cfg(target_arch = "wasm32")]
            let repaired_input: Option<crate::types::PdfInput> = None;
            // `lib` is dropped here, releasing the PDFium lock.
            (
                pages,
                page_errors,
                total_pages,
                outline,
                images,
                screenshots,
                image_error_count,
                complexity,
                form_type,
                creator,
                producer,
                doc_meta,
                xfa_packets,
                flattened_form_widgets,
                flattened_page_numbers,
                repaired_input,
            )
        };
        let mut pages = pages;
        let t1 = web_time::Instant::now();

        if let Some(engine) = ocr_engine {
            let round_rasters = self.config.num_workers.max(1);
            // Extraction may have flattened SOME pages' form widgets into
            // page content in its (now dropped) document instance; re-apply
            // per round on exactly those pages so the rasters match what
            // extraction saw. With `render_form_fields` the form environment
            // paints the widgets instead, so no re-flatten is needed.
            let reflatten_pages: std::collections::HashSet<u32> =
                if flattened_form_widgets && !self.config.render_form_fields {
                    flattened_page_numbers.iter().copied().collect()
                } else {
                    std::collections::HashSet::new()
                };
            let ocr_input = repaired_input.as_ref().unwrap_or(validated_input);
            let mut round_start = 0usize;
            while round_start < pages.len() {
                let (rendered, next_start) = {
                    let lib = Library::init();
                    let document = extract::load_document_from_input(&lib, ocr_input, password)?;
                    ocr_merge::render_pages_for_ocr(
                        &document,
                        &pages,
                        round_start,
                        round_rasters,
                        self.config.dpi,
                        ocr_grayscale,
                        self.config.render_form_fields,
                        self.config.continue_on_page_error,
                        &reflatten_pages,
                    )?
                    // `lib` drops here, releasing the PDFium lock before the
                    // engine's async recognition below.
                };
                round_start = next_start;
                if rendered.is_empty() {
                    // The scan reached the end without finding another page
                    // that needs OCR.
                    continue;
                }
                // `RenderedPage::idx` is absolute, so the whole slice is
                // passed regardless of where this round started.
                ocr_merge::ocr_and_merge_rendered(
                    &mut pages,
                    rendered,
                    engine.clone(),
                    &self.config.ocr_language,
                    self.config.num_workers,
                    self.config.ocr_failure_fatal,
                )
                .await?;
            }
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

        // Attach per-page complexity signals, including the layout signals
        // that need the projected page (same as `is_complex()` reports).
        // Matched by page number, not position: a tolerant parse may have no
        // stats for a page whose complexity pass failed.
        let mut complexity = complexity.into_iter().peekable();
        for page in parsed_pages.iter_mut() {
            let Some(mut stats) = complexity.next_if(|stats| stats.page_number == page.page_number)
            else {
                continue;
            };
            stats.layout = Some(ocr_merge::calculate_layout_complexity(page));
            page.complexity = Some(stats);
        }
        let t2 = web_time::Instant::now();
        log(&format!(
            "[liteparse] project: {:.1}ms",
            t2.duration_since(t_ocr).as_secs_f64() * 1000.0
        ));

        let laid_out = apply_layout(&self.config, &mut parsed_pages, &outline);
        let mut full_text = if let Some(md) = laid_out {
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
        if self.config.output_format == crate::config::OutputFormat::Markdown {
            rewrite_duplicate_image_refs(&mut parsed_pages, &mut full_text, &images);
        }

        let total = web_time::Instant::now().duration_since(t0).as_secs_f64() * 1000.0;
        log(&format!("[liteparse] total: {:.1}ms", total));

        #[cfg(not(target_arch = "wasm32"))]
        if self.config.effective_extract_images()
            && let Some(output_dir) = self.config.image_output_dir.as_deref()
        {
            write_extracted_images(output_dir, &mut images)?;
        }

        Ok(ParseResult {
            total_pages,
            pages: parsed_pages,
            page_errors,
            text: full_text,
            outline,
            images,
            screenshots,
            image_error_count,
            form_type,
            creator,
            producer,
            doc_meta,
            xfa_packets,
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
        let total_pages = pages.len().min(u32::MAX as usize) as u32;
        let mut parsed_pages = projection::project_pages_to_grid(pages);

        let full_text = if let Some(md) = apply_layout(&self.config, &mut parsed_pages, &outline) {
            md
        } else {
            parsed_pages
                .iter()
                .map(|p| p.text.as_str())
                .collect::<Vec<_>>()
                .join("\n\n")
        };

        ParseResult {
            total_pages,
            pages: parsed_pages,
            page_errors: Vec::new(),
            text: full_text,
            outline,
            images: Vec::new(),
            screenshots: Vec::new(),
            image_error_count: 0,
            form_type: None,
            creator: None,
            producer: None,
            doc_meta: None,
            xfa_packets: None,
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
            self.config.detect_screenshot_rects,
            self.config.render_form_fields,
        )?;

        Ok(rendered
            .into_iter()
            .map(|page| ScreenshotResult {
                page_num: page.page_num,
                width: page.width,
                height: page.height,
                image_bytes: page.png_bytes,
                is_solid_fill: page.is_solid_fill,
                rects: page.rects,
            })
            .collect())
    }

    pub fn config(&self) -> &LiteParseConfig {
        &self.config
    }

    /// Open a document for bounded-memory batch parsing.
    ///
    /// Returns an error if `target_pages` is configured — an explicit page
    /// selection and generated batch ranges would be ambiguous together.
    pub async fn open_batch_session(
        &self,
        input: PdfInput,
        batch_size: usize,
    ) -> Result<ParseSession, LiteParseError> {
        self.validate_output_config()?;
        if self.config.target_pages.is_some() {
            return Err(LiteParseError::Config(
                "batch parsing cannot be combined with target_pages".to_string(),
            ));
        }
        if batch_size == 0 {
            return Err(LiteParseError::Config(
                "batch size must be at least 1".to_string(),
            ));
        }

        let input = self.resolve_input(input).await?;
        // One cheap open (a few ms even for a 100 MB file — PDFium maps the
        // file and parses the xref rather than reading it) so the caller knows
        // the page count before the first batch is parsed, and so the
        // document-level bookmark walk is paid once instead of per batch.
        let (total_pages, outline) = {
            let lib = Library::init();
            let document = extract::load_document_from_input(
                &lib,
                &input.input,
                self.config.password.as_deref(),
            )?;
            (
                document.page_count().max(0) as u32,
                extract::extract_outline(&document),
            )
        };

        Ok(ParseSession {
            page_limit: total_pages.min(self.config.max_pages.min(u32::MAX as usize) as u32),
            parser: self.clone(),
            input,
            total_pages,
            outline,
            next_page: 1,
            batch_size,
        })
    }
}

/// A document opened once and parsed in bounded page batches.
pub struct ParseSession {
    parser: LiteParse,
    input: ResolvedInput,
    total_pages: u32,
    /// Walked once at open. Document-level, so every batch reports the same
    /// outline, and re-walking it per batch would dominate batching overhead.
    outline: Vec<OutlineTarget>,
    /// Last source page this session will parse: `min(total_pages, max_pages)`.
    page_limit: u32,
    /// Next source page to parse, 1-based.
    next_page: u32,
    batch_size: usize,
}

/// One batch of pages from a [`ParseSession`].
pub struct ParseBatch {
    /// First source page in this batch, 1-based.
    pub start_page: u32,
    /// Last source page in this batch, 1-based and inclusive.
    pub end_page: u32,
    /// The pages in `start_page..=end_page`, parsed as an ordinary result.
    pub result: ParseResult,
}

impl ParseSession {
    /// Total pages in the source document, before `max_pages` or batching.
    pub fn total_pages(&self) -> u32 {
        self.total_pages
    }

    /// Parse and return the next batch, or `None` once every page within
    /// `max_pages` has been yielded.
    pub async fn next_batch(&mut self) -> Result<Option<ParseBatch>, LiteParseError> {
        if self.next_page > self.page_limit {
            return Ok(None);
        }
        let start_page = self.next_page;
        let end_page = start_page
            .saturating_add(self.batch_size.min(u32::MAX as usize) as u32 - 1)
            .min(self.page_limit);
        let targets: Vec<u32> = (start_page..=end_page).collect();

        let result = self
            .parser
            .parse_resolved(
                &self.input,
                Some(&targets),
                targets.len(),
                Some(self.outline.clone()),
            )
            .await?;

        self.next_page = end_page.saturating_add(1);
        Ok(Some(ParseBatch {
            start_page,
            end_page,
            result,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Page, TextItem};

    fn page_with_text_metadata() -> Page {
        Page {
            page_number: 1,
            page_width: 100.0,
            page_height: 100.0,
            content_bounds: None,
            text_items: vec![TextItem {
                text: "hello".into(),
                width: 20.0,
                height: 10.0,
                font_name: Some("Helvetica".into()),
                font_size: Some(10.0),
                font_height: Some(10.0),
                font_ascent: Some(8.0),
                font_descent: Some(-2.0),
                font_weight: Some(700),
                text_width: Some(19.0),
                font_is_buggy: true,
                mcid: Some(3),
                fill_color: Some("ff112233".into()),
                stroke_color: Some("ff445566".into()),
                char_codes: vec![104, 101, 108, 108, 111],
                trailing_space_generated: true,
                ..Default::default()
            }],
            graphics: vec![],
            vector_graphics: None,
            struct_nodes: vec![],
            image_refs: vec![],
            annotations: None,
            form_fields: None,
            structure_tree: None,
        }
    }

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
    fn parse_from_pages_preserves_internal_text_metadata() {
        let result = LiteParse::new(LiteParseConfig::default())
            .parse_from_pages(vec![page_with_text_metadata()], vec![]);
        let item = &result.pages[0].text_items[0];
        assert_eq!(item.font_name.as_deref(), Some("Helvetica"));
        assert_eq!(item.font_size, Some(10.0));
        assert_eq!(item.font_height, Some(10.0));
        assert_eq!(item.font_ascent, Some(8.0));
        assert_eq!(item.font_descent, Some(-2.0));
        assert_eq!(item.font_weight, Some(700));
        assert_eq!(item.text_width, Some(19.0));
        assert!(item.font_is_buggy);
        assert_eq!(item.mcid, Some(3));
        assert_eq!(item.fill_color.as_deref(), Some("ff112233"));
        assert_eq!(item.stroke_color.as_deref(), Some("ff445566"));
        assert_eq!(item.char_codes, vec![104, 101, 108, 108, 111]);
        assert!(item.trailing_space_generated);
    }

    #[test]
    fn image_extraction_is_opt_in_but_embed_mode_implies_it() {
        let default = LiteParseConfig::default();
        assert!(!default.effective_extract_images());

        // `image_mode = embed` predates `extract_images` and must keep
        // extracting bytes for existing callers.
        let embed = LiteParseConfig {
            image_mode: crate::config::ImageMode::Embed,
            ..Default::default()
        };
        assert!(embed.effective_extract_images());

        let explicit = LiteParseConfig {
            extract_images: true,
            ..Default::default()
        };
        assert!(explicit.effective_extract_images());
    }

    #[test]
    fn image_output_dir_requires_image_extraction() {
        let parser = LiteParse::new(LiteParseConfig {
            image_output_dir: Some("images".into()),
            ..Default::default()
        });
        assert_eq!(
            parser.validate_output_config().unwrap_err().to_string(),
            "invalid config: image_output_dir requires extract_images = true (or image_mode = embed)"
        );

        let embed = LiteParse::new(LiteParseConfig {
            image_mode: crate::config::ImageMode::Embed,
            image_output_dir: Some("images".into()),
            ..Default::default()
        });
        assert!(embed.validate_output_config().is_ok());
    }

    #[test]
    fn image_output_writes_duplicates_from_canonical_bytes() {
        fn image(id: &str, duplicate_of: Option<&str>, bytes: &[u8]) -> ExtractedImage {
            ExtractedImage {
                id: id.into(),
                name: format!("img_{id}.png"),
                path: None,
                page: 1,
                bbox: crate::types::Rect::default(),
                width: 2,
                height: 2,
                rotation: 0.0,
                format: "png".into(),
                duplicate_of: duplicate_of.map(str::to_owned),
                bytes: std::sync::Arc::new(bytes.to_vec()),
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let mut images = vec![
            image("p1_1", None, b"canonical"),
            image("p2_1", Some("p1_1"), b"canonical"),
        ];
        write_extracted_images(dir.path().to_str().unwrap(), &mut images).unwrap();

        // Platform contract: one file on disk; the duplicate keeps its own
        // placement `name` but shares the canonical file's `path`.
        assert_eq!(images[0].name, "img_p1_1.png");
        assert_eq!(images[1].name, "img_p2_1.png");
        assert_eq!(images[0].path, images[1].path);
        assert_eq!(
            std::fs::read(images[0].path.as_ref().unwrap()).unwrap(),
            b"canonical"
        );
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn duplicate_image_markdown_refs_are_rewritten_to_canonical() {
        fn image(id: &str, format: &str, duplicate_of: Option<&str>) -> ExtractedImage {
            ExtractedImage {
                id: id.into(),
                name: format!("img_{id}.{format}"),
                path: None,
                page: 1,
                bbox: crate::types::Rect::default(),
                width: 2,
                height: 2,
                rotation: 0.0,
                format: format.into(),
                duplicate_of: duplicate_of.map(str::to_owned),
                bytes: std::sync::Arc::new(Vec::new()),
            }
        }

        let mut pages = vec![ParsedPage {
            page_number: 1,
            page_width: 612.0,
            page_height: 792.0,
            content_bounds: None,
            text: String::new(),
            markdown: "intro\n\n![](img_p2_1.jpg)\n\noutro".into(),
            text_items: vec![],
            projected_lines: vec![],
            regions: crate::types::Region::default(),
            graphics: vec![],
            vector_graphics: None,
            figures: vec![],
            struct_nodes: vec![],
            image_refs: vec![],
            complexity: None,
            annotations: None,
            form_fields: None,
            structure_tree: None,
            blocks: None,
        }];
        let mut full_text = pages[0].markdown.clone();
        let images = vec![
            image("p1_1", "jpg", None),
            image("p2_1", "jpg", Some("p1_1")),
        ];

        rewrite_duplicate_image_refs(&mut pages, &mut full_text, &images);

        // The duplicate's ref now points at the canonical file; canonical
        // refs and surrounding text are untouched.
        assert_eq!(pages[0].markdown, "intro\n\n![](img_p1_1.jpg)\n\noutro");
        assert_eq!(full_text, pages[0].markdown);
    }

    /// A non-PDF source is converted exactly once, when the session opens:
    /// the converted temporary PDF outlives every batch and is only removed
    /// when the session is dropped. Lives here rather than in the integration
    /// tests because it asserts against [`ResolvedInput`] internals.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_batch_session_owns_converted_source_for_its_lifetime() {
        if std::env::var("SKIP_INTEGRATION_TESTS").as_deref() == Ok("yes") {
            return;
        }
        let parser = LiteParse::new(LiteParseConfig {
            ocr_enabled: false,
            quiet: true,
            ..LiteParseConfig::default()
        });
        let mut session = parser
            .open_batch_session(
                PdfInput::Path("../../integration_tests_data/sample3.doc".to_string()),
                1,
            )
            .await
            .expect("should convert and open a .doc");

        assert!(session.input.is_converted());
        let converted_path = match &session.input.input {
            PdfInput::Path(p) => p.clone(),
            PdfInput::Bytes(_) => panic!("a converted .doc should resolve to a temp file path"),
        };

        let mut pages = 0;
        while let Some(batch) = session.next_batch().await.expect("batch should parse") {
            pages += batch.result.pages.len();
            assert!(
                std::path::Path::new(&converted_path).exists(),
                "converted temp PDF should outlive every batch — a missing file \
                 would mean it was re-resolved or cleaned up per batch"
            );
        }
        assert_eq!(pages, 2);

        drop(session);
        assert!(
            !std::path::Path::new(&converted_path).exists(),
            "dropping the session should clean up the converted temp PDF"
        );
    }
}
