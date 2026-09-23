//! Gate for the `stages` module: `parse()` must equal the stages composed by
//! hand.
//!
//! `LiteParse::parse` is written only in terms of `liteparse::stages`. This
//! test re-implements that sequence here, from the public stage functions,
//! and asserts the two produce identical output over the fixture corpus and
//! several configurations (markdown with every extraction option, OCR in
//! bounded rounds through a mock engine, orientation corrections, content
//! filters, AcroForm repair). If a step is added to `parse()` without a
//! public stage, this test is what breaks.
//!
//! Along the way the composed pipeline serializes and deserializes the page
//! set at two stage boundaries (after extract, after projection), so the
//! same test proves the boundary types round-trip losslessly.

use std::sync::Arc;

use liteparse::config::{CropBox, ImageMode, OutputFormat, PageOrientationCorrection};
use liteparse::ocr::{OcrEngine, OcrOptions, OcrResult};
use liteparse::stages::{self, Library};
use liteparse::types::{Page, ParsedPage, PdfInput};
use liteparse::{LiteParse, LiteParseConfig, ParseResult};
use serial_test::serial;

const FIXTURES: &str = "../../integration_tests_data";

fn skip_integration() -> bool {
    std::env::var("SKIP_INTEGRATION_TESTS").as_deref() == Ok("yes")
}

fn fixture(name: &str) -> String {
    format!("{FIXTURES}/{name}")
}

/// Every PDF in the fixture corpus, sorted for stable failure output.
fn corpus_pdfs() -> Vec<String> {
    let mut paths: Vec<String> = std::fs::read_dir(FIXTURES)
        .expect("fixture directory")
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            (path.extension()? == "pdf").then(|| path.to_string_lossy().into_owned())
        })
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "no fixture PDFs found under {FIXTURES}");
    paths
}

/// Deterministic mock OCR: one box per raster whose text encodes the raster
/// size, so a raster routed to the wrong page is visible in the output.
struct MockOcr;

impl OcrEngine for MockOcr {
    fn name(&self) -> &str {
        "mock"
    }
    fn recognize<'a, 'b: 'a, 'c: 'a>(
        &'a self,
        _image_data: &'c [u8],
        width: u32,
        height: u32,
        _options: &'b OcrOptions,
    ) -> std::pin::Pin<
        Box<
            dyn Future<Output = Result<Vec<OcrResult>, Box<dyn std::error::Error + Send + Sync>>>
                + Send
                + '_,
        >,
    > {
        Box::pin(async move {
            Ok(vec![
                OcrResult {
                    text: format!("MOCK {width}x{height}"),
                    bbox: [10.0, 10.0, 260.0, 40.0],
                    confidence: 0.99,
                    polygon: None,
                },
                OcrResult {
                    text: "rotated".into(),
                    bbox: [300.0, 100.0, 330.0, 400.0],
                    confidence: 0.9,
                    polygon: Some([
                        [330.0, 100.0],
                        [330.0, 400.0],
                        [300.0, 400.0],
                        [300.0, 100.0],
                    ]),
                },
            ])
        })
    }
}

/// Everything `ParseResult` reports, in a comparable form. Pages go through
/// their (lossless) serde form so every field is compared, not just the
/// rendered strings.
#[derive(Debug, PartialEq)]
struct Snapshot {
    total_pages: u32,
    pages: Vec<serde_json::Value>,
    page_errors: Vec<(u32, String)>,
    text: String,
    outline: String,
    images: Vec<serde_json::Value>,
    screenshots: Vec<(u32, u32, u32, bool, usize, Vec<u8>)>,
    image_error_count: u32,
    form_type: Option<i32>,
    creator: Option<String>,
    producer: Option<String>,
    doc_meta: Option<serde_json::Value>,
    xfa_packets: Option<serde_json::Value>,
}

fn snapshot(result: &ParseResult) -> Snapshot {
    Snapshot {
        total_pages: result.total_pages,
        pages: result
            .pages
            .iter()
            .map(|p| serde_json::to_value(p).unwrap())
            .collect(),
        page_errors: result
            .page_errors
            .iter()
            .map(|e| (e.page_number, e.message.clone()))
            .collect(),
        text: result.text.clone(),
        outline: format!("{:?}", result.outline),
        images: result
            .images
            .iter()
            .map(|i| serde_json::to_value(i).unwrap())
            .collect(),
        screenshots: result
            .screenshots
            .iter()
            .map(|s| {
                (
                    s.page_num,
                    s.width,
                    s.height,
                    s.is_solid_fill,
                    s.rects.len(),
                    s.image_bytes.clone(),
                )
            })
            .collect(),
        image_error_count: result.image_error_count,
        form_type: result.form_type,
        creator: result.creator.clone(),
        producer: result.producer.clone(),
        doc_meta: result
            .doc_meta
            .as_ref()
            .map(|m| serde_json::to_value(m).unwrap()),
        xfa_packets: result
            .xfa_packets
            .as_ref()
            .map(|x| serde_json::to_value(x).unwrap()),
    }
}

/// Serialize and deserialize `value`, asserting the round trip is lossless
/// (re-serializing the deserialized value yields the same JSON).
fn round_trip<T: serde::Serialize + serde::de::DeserializeOwned>(value: T, what: &str) -> T {
    let json = serde_json::to_string(&value).unwrap();
    let back: T = serde_json::from_str(&json).unwrap_or_else(|e| panic!("{what}: {e}"));
    assert_eq!(
        serde_json::to_string(&back).unwrap(),
        json,
        "{what}: serde round trip changed the value"
    );
    back
}

/// `parse()` written out by hand from the public stages. Mirrors
/// `LiteParse::parse_resolved` step for step; the point is that nothing in
/// that function is unreachable from here.
async fn compose(parser: &LiteParse, input: PdfInput) -> ParseResult {
    let config = parser.config();
    let password = config.password.as_deref();
    let markdown = config.output_format == OutputFormat::Markdown;
    let target_pages = config
        .target_pages
        .as_ref()
        .map(|s| liteparse::config::parse_target_pages(s).unwrap());

    // Non-PDF inputs are converted before the pipeline starts; a PDF passes
    // through untouched.
    let (input, _guard) = liteparse::conversion::resolve_pdf_input(input, password, false)
        .await
        .unwrap();
    let want_doc_meta = config.extract_document_metadata && !_guard.is_converted();

    let engine = parser.ocr_engine().unwrap();
    let grayscale = engine.as_ref().is_some_and(|e| e.prefers_grayscale());

    // ── pdfium-bound: open, document facts, extract, complexity, screenshots
    let lib = Library::init();
    let repaired = config
        .extract_form_fields
        .then(|| stages::repair_acroform(&lib, &input, password))
        .flatten();
    let document_input = repaired.as_ref().unwrap_or(&input);
    let document = stages::open(
        &lib,
        document_input,
        password,
        &config.page_orientation_corrections,
    )
    .unwrap();
    let total_pages = document.page_count().max(0) as u32;
    let form_type = config.extract_form_fields.then(|| document.form_type());
    let creator = document.meta_text("Creator").filter(|v| !v.is_empty());
    let producer = document.meta_text("Producer").filter(|v| !v.is_empty());
    let doc_meta = want_doc_meta.then(|| {
        if repaired.is_some() {
            let source =
                stages::open(&lib, &input, password, &config.page_orientation_corrections).unwrap();
            return stages::document_metadata(&input, &source);
        }
        stages::document_metadata(&input, &document)
    });
    let xfa_packets = config
        .extract_xfa_packets
        .then(|| stages::xfa_packets(&document));
    let outline = stages::outline(&document);
    let extracted = stages::extract(
        &document,
        &parser.extract_request(target_pages.as_deref(), config.max_pages),
    )
    .unwrap();
    let ocr_render_options = parser.ocr_render_options(grayscale, &extracted);
    let screenshot_options = parser.screenshot_options(config.continue_on_page_error);
    // Extraction may have flattened form widgets into the open document;
    // screenshots that paint form fields need a document it did not touch.
    let pristine = (config.extract_screenshots
        && stages::screenshots_need_pristine_document(&extracted, &screenshot_options))
    .then(|| {
        stages::open(
            &lib,
            document_input,
            password,
            &config.page_orientation_corrections,
        )
        .unwrap()
    });
    let analysis_document = pristine.as_ref().unwrap_or(&document);
    let complexity: Vec<stages::PageComplexityStats> = if config.include_complexity {
        extracted
            .pages
            .iter()
            .map(|page| stages::page_complexity(analysis_document, page).unwrap())
            .collect()
    } else {
        Vec::new()
    };
    let screenshots = if config.extract_screenshots {
        let page_numbers: Vec<u32> = extracted
            .pages
            .iter()
            .map(|p| p.page_number as u32)
            .collect();
        stages::screenshots(analysis_document, Some(&page_numbers), &screenshot_options).unwrap()
    } else {
        Vec::new()
    };
    drop(pristine);
    drop(document);
    drop(lib);

    // Boundary 1: the whole extraction result crosses a (simulated) process
    // boundary before OCR. Image payloads are the documented exception (they
    // travel out of band, keyed by id), so they are carried across by hand.
    let image_bytes: Vec<_> = extracted.images.iter().map(|i| i.bytes.clone()).collect();
    let extracted = round_trip(extracted, "ExtractedPages after extract");
    let stages::ExtractedPages {
        pages,
        page_errors,
        mut images,
        image_error_count,
        ..
    } = extracted;
    for (image, bytes) in images.iter_mut().zip(image_bytes) {
        image.bytes = bytes;
    }
    let mut pages: Vec<Page> = pages;

    // ── OCR: render rounds (pdfium) → recognize (async) → merge (pure)
    if let Some(engine) = engine {
        let options = ocr_render_options;
        let ocr_input = repaired.as_ref().unwrap_or(&input);
        let mut start = 0;
        while start < pages.len() {
            let (rasters, next) = {
                let lib = Library::init();
                let document = stages::open(
                    &lib,
                    ocr_input,
                    password,
                    &config.page_orientation_corrections,
                )
                .unwrap();
                stages::render_for_ocr(&document, &pages, start, &options).unwrap()
            };
            start = next;
            if rasters.is_empty() {
                continue;
            }
            let rasters = round_trip(rasters, "Vec<OcrRaster>");
            let outcomes = stages::recognize(
                rasters,
                engine.clone(),
                &config.ocr_language,
                config.num_workers,
            )
            .await;
            let outcomes = round_trip(outcomes, "Vec<PageOcrOutcome>");
            stages::merge_ocr(&mut pages, outcomes, config.ocr_failure_fatal).unwrap();
        }
    }

    // ── pure: filters, projection, complexity layout half
    stages::apply_content_filters(&mut pages, &parser.content_filters());
    let parsed = stages::project(pages);

    // Boundary 2: projected pages cross before the markdown stages.
    let mut parsed: Vec<ParsedPage> = round_trip(parsed, "Vec<ParsedPage> after project");

    let mut complexity = complexity.into_iter().peekable();
    for page in parsed.iter_mut() {
        if let Some(mut stats) = complexity.next_if(|s| s.page_number == page.page_number) {
            stats.layout = Some(stages::layout_complexity(page));
            page.complexity = Some(stats);
        }
    }

    // ── markdown: document signals once, then per page
    if markdown || config.extract_blocks {
        let signals = stages::document_signals(&parsed, config.keep_headers_footers);
        // `header_footer` is a set, so its JSON order is not stable across
        // serializations; compare the parts rather than the bytes.
        let back: stages::DocumentSignals =
            serde_json::from_str(&serde_json::to_string(&signals).unwrap()).unwrap();
        assert_eq!(back.body_size, signals.body_size);
        assert_eq!(back.heading_map, signals.heading_map);
        assert_eq!(back.header_footer, signals.header_footer);
        let signals = back;
        let options = parser.block_options(&outline);
        for page in parsed.iter_mut() {
            let blocks = stages::extract_blocks(page, &signals, &options);
            let blocks = blocks.map(|b| round_trip(b, "Vec<PositionedBlock>"));
            if config.extract_blocks {
                page.blocks = Some(stages::layout_blocks(
                    page,
                    blocks.as_deref().unwrap_or_default(),
                ));
            }
            if markdown {
                page.markdown = stages::render_page_markdown(page, blocks.as_deref());
            }
        }
    }
    let mut text = if markdown {
        parsed
            .iter()
            .map(|p| p.markdown.as_str())
            .collect::<Vec<_>>()
            .join("\n\n-----\n\n")
    } else {
        parsed
            .iter()
            .map(|p| p.text.as_str())
            .collect::<Vec<_>>()
            .join("\n\n")
    };
    if markdown {
        stages::canonicalize_image_refs(&mut parsed, &mut text, &images);
    }
    // `parse()` writes image files only with `image_output_dir`; none of the
    // configurations here set it, so `path` stays unset on both sides.
    for image in images.iter_mut() {
        image.path = None;
    }

    ParseResult {
        total_pages,
        pages: parsed,
        page_errors,
        text,
        outline,
        images,
        screenshots,
        image_error_count,
        form_type,
        creator,
        producer,
        doc_meta,
        xfa_packets,
    }
}

/// Parse `path` both ways, assert equality, and hand back the result so a
/// caller can check the configuration actually exercised what it meant to.
async fn assert_parse_equals_composition(
    config: LiteParseConfig,
    path: &str,
    with_ocr: bool,
) -> ParseResult {
    let mut parser = LiteParse::new(config);
    if with_ocr {
        parser = parser.with_ocr_engine(Arc::new(MockOcr));
    }
    let input = PdfInput::Path(path.to_string());
    let via_parse = parser
        .parse_input(input.clone())
        .await
        .unwrap_or_else(|e| panic!("parse() failed on {path}: {e}"));
    let via_stages = compose(&parser, input).await;
    assert_eq!(
        snapshot(&via_parse),
        snapshot(&via_stages),
        "parse() and the composed stages diverged on {path}"
    );
    via_parse
}

/// Every extraction option on, no OCR: exercises extract, complexity,
/// screenshots, projection, blocks and markdown, and the duplicate-image
/// rewrite.
fn everything_config() -> LiteParseConfig {
    LiteParseConfig {
        ocr_enabled: false,
        quiet: true,
        output_format: OutputFormat::Markdown,
        image_mode: ImageMode::Embed,
        extract_images: true,
        extract_blocks: true,
        include_complexity: true,
        extract_content_bounds: true,
        emit_word_boxes: true,
        extract_text_metadata: true,
        extract_vector_graphics: true,
        extract_annotations: true,
        extract_structure_tree: true,
        extract_document_metadata: true,
        extract_xfa_packets: true,
        extract_screenshots: true,
        detect_screenshot_rects: true,
        extract_links: true,
        ..LiteParseConfig::default()
    }
}

#[tokio::test]
#[serial]
async fn parse_equals_composed_stages_markdown_with_every_option() {
    let (mut with_images, mut with_blocks, mut with_screenshots) = (0, 0, 0);
    for path in corpus_pdfs() {
        let result = assert_parse_equals_composition(everything_config(), &path, false).await;
        with_images += usize::from(!result.images.is_empty());
        with_blocks += usize::from(
            result
                .pages
                .iter()
                .any(|p| p.blocks.as_ref().is_some_and(|b| !b.is_empty())),
        );
        with_screenshots += usize::from(!result.screenshots.is_empty());
    }
    // The equality is only meaningful if the branches ran. None of the
    // fixture PDFs embeds a raster image; the demo 10-K does on three pages,
    // so those pages (plus one without) cover image extraction, the figure
    // block and the duplicate-image rewrite.
    assert!(with_blocks > 0, "no fixture produced blocks");
    assert!(with_screenshots > 0, "no fixture produced screenshots");
    let config = LiteParseConfig {
        target_pages: Some("1,2,108,119".into()),
        ..everything_config()
    };
    let result =
        assert_parse_equals_composition(config, "../../demo/docs/apple-10k-2024.pdf", false).await;
    with_images += usize::from(!result.images.is_empty());
    assert!(with_images > 0, "no fixture produced images");
}

/// Text output with blocks: the non-markdown branch of the layout stage.
#[tokio::test]
#[serial]
async fn parse_equals_composed_stages_text_output() {
    let config = LiteParseConfig {
        ocr_enabled: false,
        quiet: true,
        output_format: OutputFormat::Text,
        extract_blocks: true,
        ..LiteParseConfig::default()
    };
    for path in corpus_pdfs() {
        assert_parse_equals_composition(config.clone(), &path, false).await;
    }
}

/// OCR through the stage split, one page per round (`num_workers: 1`) so the
/// render → recognize → merge loop runs several times, plus the
/// selection-by-complexity gate and the native-text artifact filters.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn parse_equals_composed_stages_with_ocr_rounds() {
    let config = LiteParseConfig {
        ocr_enabled: true,
        num_workers: 1,
        quiet: true,
        output_format: OutputFormat::Markdown,
        include_complexity: true,
        ..LiteParseConfig::default()
    };
    let mut ocr_pages = 0;
    for path in corpus_pdfs() {
        let result = assert_parse_equals_composition(config.clone(), &path, true).await;
        ocr_pages += result
            .pages
            .iter()
            .filter(|p| p.text_items.iter().any(|i| i.text.starts_with("MOCK ")))
            .count();
    }
    assert!(ocr_pages > 0, "no fixture page had mock OCR text merged in");
}

/// A converted image input is a scanned page: OCR is its only text.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn parse_equals_composed_stages_on_converted_image() {
    if skip_integration() {
        return;
    }
    let config = LiteParseConfig {
        ocr_enabled: true,
        quiet: true,
        output_format: OutputFormat::Markdown,
        ..LiteParseConfig::default()
    };
    let result = assert_parse_equals_composition(config, &fixture("receipt.png"), true).await;
    assert!(
        result.text.contains("MOCK "),
        "a converted image should be OCR-only: {}",
        result.text
    );
}

/// Orientation corrections applied at open, a page selection, and the
/// content filters.
#[tokio::test]
#[serial]
async fn parse_equals_composed_stages_with_orientation_selection_and_filters() {
    let config = LiteParseConfig {
        ocr_enabled: false,
        quiet: true,
        output_format: OutputFormat::Markdown,
        page_orientation_corrections: vec![PageOrientationCorrection { page: 1, angle: 90 }],
        target_pages: Some("1".into()),
        max_pages: 1,
        crop_box: Some(CropBox {
            top: 0.05,
            bottom: 0.05,
            left: 0.0,
            right: 0.0,
        }),
        skip_diagonal_text: true,
        ..LiteParseConfig::default()
    };
    for name in ["sample_rotated_90cw.pdf", "diagonal_text.pdf"] {
        assert_parse_equals_composition(config.clone(), &fixture(name), false).await;
    }
}

/// Form-field extraction: AcroForm repair before open, widget flattening
/// during extract, and the re-flatten on the reopened OCR document.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn parse_equals_composed_stages_with_form_fields() {
    let config = LiteParseConfig {
        ocr_enabled: true,
        quiet: true,
        output_format: OutputFormat::Markdown,
        extract_form_fields: true,
        extract_document_metadata: true,
        ..LiteParseConfig::default()
    };
    assert_parse_equals_composition(config, &fixture("filled_acroform.pdf"), true).await;
}

/// Form fields plus screenshots that paint form fields: the one combination
/// where `parse()` reopens a pristine document after extraction flattened
/// widgets, so the composed pipeline must do the same.
#[tokio::test]
#[serial]
async fn parse_equals_composed_stages_with_form_fields_and_rendered_screenshots() {
    let config = LiteParseConfig {
        ocr_enabled: false,
        quiet: true,
        output_format: OutputFormat::Markdown,
        extract_form_fields: true,
        extract_screenshots: true,
        render_form_fields: true,
        include_complexity: true,
        ..LiteParseConfig::default()
    };
    let result =
        assert_parse_equals_composition(config, &fixture("filled_acroform.pdf"), false).await;
    assert!(
        !result.screenshots.is_empty(),
        "fixture should produce screenshots"
    );
}

/// An explicit OCR page selection bypasses the complexity gate: a text-dense
/// page that scoring would never send to OCR is rastered anyway, and pages
/// outside the selection are skipped.
#[tokio::test]
#[serial]
async fn render_for_ocr_selection_overrides_complexity_gate() {
    let parser = LiteParse::new(everything_config());
    let lib = Library::init();
    let document = stages::open(&lib, &PdfInput::Path(fixture("sample.pdf")), None, &[]).unwrap();
    let pages = stages::extract(&document, &parser.extract_request(None, usize::MAX))
        .unwrap()
        .pages;
    let first = pages[0].page_number as u32;
    let gated = stages::render_for_ocr(&document, &pages, 0, &stages::OcrRenderOptions::default())
        .unwrap()
        .0;
    assert!(
        gated.iter().all(|r| r.page_number as u32 != first),
        "sample.pdf page 1 has native text and should not be rastered by the scoring gate"
    );
    let options = stages::OcrRenderOptions {
        selection: Some([first].into_iter().collect()),
        ..stages::OcrRenderOptions::default()
    };
    let (rasters, next) = stages::render_for_ocr(&document, &pages, 0, &options).unwrap();
    assert_eq!(next, pages.len());
    assert_eq!(rasters.len(), 1);
    assert_eq!(rasters[0].page_number as u32, first);
    assert!(rasters[0].has_native_text);
    assert!(rasters[0].width > 0 && rasters[0].height > 0);
    // The raster's pixels survive the boundary as base64, not a number array.
    let json = serde_json::to_value(&rasters[0]).unwrap();
    assert!(json["pixels"].is_string());
}

/// A hand-built outcome for a page that is not being merged is an error,
/// not a panic.
#[test]
fn merge_ocr_rejects_outcome_for_unknown_page() {
    let mut pages: Vec<Page> = Vec::new();
    let outcome = stages::PageOcrOutcome {
        page_number: 7,
        dpi: 150.0,
        has_native_text: false,
        image_rects: Vec::new(),
        results: vec![OcrResult {
            text: "x".into(),
            bbox: [0.0, 0.0, 10.0, 10.0],
            confidence: 1.0,
            polygon: None,
        }],
        error: None,
    };
    let err = stages::merge_ocr(&mut pages, vec![outcome], false).unwrap_err();
    assert!(err.to_string().contains("page 7"), "{err}");
}

/// Sanity check on the round-trip helper itself: a page with every optional
/// field populated survives serde unchanged, including the fields the public
/// JSON output omits (word boxes, graphics, structure nodes, image refs).
#[tokio::test]
#[serial]
async fn extracted_pages_round_trip_keeps_internal_fields() {
    let parser = LiteParse::new(everything_config());
    let lib = Library::init();
    let document = stages::open(&lib, &PdfInput::Path(fixture("sample.pdf")), None, &[]).unwrap();
    let pages = stages::extract(&document, &parser.extract_request(None, usize::MAX))
        .unwrap()
        .pages;
    let has_words = pages
        .iter()
        .any(|p| p.text_items.iter().any(|i| !i.words.is_empty()));
    assert!(has_words, "fixture should produce word boxes");
    let back = round_trip(pages.clone(), "Vec<Page>");
    for (a, b) in pages.iter().zip(&back) {
        assert_eq!(a.text_items.len(), b.text_items.len());
        assert_eq!(a.graphics.len(), b.graphics.len());
        assert_eq!(a.struct_nodes.len(), b.struct_nodes.len());
        assert_eq!(a.image_refs.len(), b.image_refs.len());
        for (x, y) in a.text_items.iter().zip(&b.text_items) {
            assert_eq!(x.words.len(), y.words.len());
        }
    }
}
