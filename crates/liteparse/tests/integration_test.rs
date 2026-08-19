use std::path::Path;

use liteparse::config::{
    OutputFormat, PngCompression, PngScreenshotOptions, ScreenshotFormat, ScreenshotOptions,
};
use liteparse::conversion::convert_data_to_pdf;
use liteparse::ocr_merge::ComplexityReason;
use liteparse::types::PdfInput;
use liteparse::{LiteParse, LiteParseConfig};
use serial_test::serial;

#[tokio::test]
#[serial]
async fn test_screenshot_image_integration() {
    let env_var = std::env::var("SKIP_INTEGRATION_TESTS");
    if let Ok(v) = env_var
        && v == "yes"
    {
        return;
    }
    let lit = LiteParse::new(LiteParseConfig::default());
    let results = lit
        .screenshot("../../integration_tests_data/receipt.png", None)
        .await
        .expect("Should be able to screenshot converted image");
    assert_eq!(results.len(), 1);
    assert!(results[0].width > 0);
    assert!(results[0].height > 0);
    assert!(!results[0].image_bytes.is_empty());
}

#[tokio::test]
#[serial]
async fn test_screenshot_pdf_integration() {
    let lit = LiteParse::new(LiteParseConfig::default());
    let results = lit
        .screenshot("../../integration_tests_data/sample.pdf", None)
        .await
        .expect("Should be able to screenshot PDF");
    assert_eq!(results.len(), 1);
    assert!(!results[0].image_bytes.is_empty());
}

#[tokio::test]
#[serial]
async fn test_parse_can_return_screenshots() {
    let lit = LiteParse::new(LiteParseConfig {
        ocr_enabled: false,
        extract_screenshots: true,
        ..LiteParseConfig::default()
    });
    let parsed = lit
        .parse("../../integration_tests_data/sample.pdf")
        .await
        .expect("Should parse and render PDF pages");

    assert_eq!(parsed.screenshots.len(), parsed.pages.len());
    assert_eq!(
        parsed.screenshots[0].page_num,
        parsed.pages[0].page_number as u32
    );
    assert!(
        parsed.screenshots[0]
            .image_bytes
            .starts_with(b"\x89PNG\r\n\x1a\n")
    );
    assert_eq!(parsed.screenshots[0].format, ScreenshotFormat::Png);
    assert_eq!(parsed.screenshots[0].stride, None);
}

#[tokio::test]
#[serial]
async fn test_png_compression_preserves_rendered_pixels() {
    let render = |compression| async move {
        LiteParse::new(LiteParseConfig {
            ocr_enabled: false,
            screenshot: ScreenshotOptions {
                format: ScreenshotFormat::Png,
                png: PngScreenshotOptions { compression },
            },
            ..LiteParseConfig::default()
        })
        .screenshot("../../integration_tests_data/sample.pdf", Some(vec![1]))
        .await
        .expect("Should render a PNG screenshot")
        .remove(0)
        .image_bytes
    };

    let fast = render(PngCompression::Fast).await;
    let best = render(PngCompression::Best).await;
    let fast_image = image::load_from_memory(&fast)
        .expect("Fast-compressed screenshot should decode")
        .to_rgb8();
    let best_image = image::load_from_memory(&best)
        .expect("Best-compressed screenshot should decode")
        .to_rgb8();

    assert_eq!(fast_image.dimensions(), best_image.dimensions());
    assert_eq!(fast_image.as_raw(), best_image.as_raw());
    assert_ne!(fast, best);
}

#[tokio::test]
#[serial]
async fn test_parse_can_return_tightly_packed_rgb8_screenshots() {
    let lit = LiteParse::new(LiteParseConfig {
        ocr_enabled: false,
        extract_screenshots: true,
        screenshot: ScreenshotOptions {
            format: ScreenshotFormat::Rgb8,
            ..ScreenshotOptions::default()
        },
        detect_screenshot_rects: true,
        ..LiteParseConfig::default()
    });
    let parsed = lit
        .parse("../../integration_tests_data/sample.pdf")
        .await
        .expect("Should parse and render RGB screenshots");
    let screenshot = &parsed.screenshots[0];
    let stride = screenshot.width.checked_mul(3).unwrap();
    assert_eq!(screenshot.format, ScreenshotFormat::Rgb8);
    assert_eq!(screenshot.stride, Some(stride));
    assert_eq!(
        screenshot.image_bytes.len(),
        stride as usize * screenshot.height as usize
    );
    assert!(!screenshot.is_solid_fill);
}

#[tokio::test]
async fn test_screenshot_rejects_text_file() {
    let dir = tempfile::tempdir().unwrap();
    let txt_path = dir.path().join("notes.txt");
    std::fs::write(&txt_path, "hello").unwrap();
    let lit = LiteParse::new(LiteParseConfig::default());
    let err = lit
        .screenshot(txt_path.to_str().unwrap(), None)
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("Cannot screenshot text-based format"));
}

#[tokio::test]
#[serial]
async fn test_convert_data_to_pdf_integration() {
    let env_var = std::env::var("SKIP_INTEGRATION_TESTS");
    if let Ok(v) = env_var
        && v == "yes"
    {
        return;
    }
    let fixture_path = "../../integration_tests_data/receipt.png";
    let data = tokio::fs::read(fixture_path)
        .await
        .expect("Should be able to read file");
    let (converted, _temps) = convert_data_to_pdf(data, None)
        .await
        .expect("Should be able to convert data to PDF");
    assert!(Path::new(&converted.pdf_path).exists());
}

#[tokio::test]
#[serial]
async fn test_parse_bytes_image_integration() {
    let env_var = std::env::var("SKIP_INTEGRATION_TESTS");
    if let Ok(v) = env_var
        && v == "yes"
    {
        return;
    }
    let fixture_path = "../../integration_tests_data/receipt.png";
    let lit = LiteParse::new(LiteParseConfig::default());
    let data = tokio::fs::read(fixture_path)
        .await
        .expect("Should be able to read file");
    let input = PdfInput::Bytes(data);
    let parsed = lit
        .parse_input(input)
        .await
        .expect("Should be able to parse");
    assert_eq!(parsed.pages.len(), 1);
    assert_eq!(parsed.total_pages, 1);
}

#[tokio::test]
#[serial]
async fn test_total_pages_precedes_target_page_filtering() {
    let lit = LiteParse::new(LiteParseConfig {
        ocr_enabled: false,
        quiet: true,
        target_pages: Some("2".into()),
        ..LiteParseConfig::default()
    });
    let parsed = lit
        .parse("../../integration_tests_data/filled_acroform.pdf")
        .await
        .expect("Should be able to parse a selected page");

    assert_eq!(parsed.total_pages, 3);
    assert_eq!(parsed.pages.len(), 1);
    assert_eq!(parsed.pages[0].page_number, 2);
}

/// Batching must not change what any individual page parses to — only how many
/// pages are materialized at once.
#[tokio::test]
#[serial]
async fn test_batch_parse_matches_whole_document() {
    let config = || LiteParseConfig {
        ocr_enabled: false,
        quiet: true,
        ..LiteParseConfig::default()
    };
    let path = "../../integration_tests_data/filled_acroform.pdf";

    let whole = LiteParse::new(config())
        .parse(path)
        .await
        .expect("whole-document parse should succeed");

    let mut session = LiteParse::new(config())
        .open_batch_session(PdfInput::Path(path.to_string()), 2)
        .await
        .expect("should open a session");
    assert_eq!(session.total_pages(), whole.total_pages);

    let mut batches = Vec::new();
    while let Some(batch) = session.next_batch().await.expect("batch should parse") {
        batches.push(batch);
    }

    // 3 pages at 2 per batch: [1-2], [3-3].
    assert_eq!(batches.len(), 2);
    assert_eq!((batches[0].start_page, batches[0].end_page), (1, 2));
    assert_eq!((batches[1].start_page, batches[1].end_page), (3, 3));

    let batched: Vec<_> = batches.iter().flat_map(|b| b.result.pages.iter()).collect();
    assert_eq!(batched.len(), whole.pages.len());
    for (got, want) in batched.iter().zip(&whole.pages) {
        assert_eq!(got.page_number, want.page_number);
        assert_eq!(got.text, want.text);
    }
    for batch in &batches {
        assert_eq!(batch.result.total_pages, whole.total_pages);
    }
}

/// `max_pages` bounds the session the same way it bounds a whole parse, and a
/// batch size larger than the document collapses to a single batch.
#[tokio::test]
#[serial]
async fn test_batch_parse_respects_max_pages() {
    let mut session = LiteParse::new(LiteParseConfig {
        ocr_enabled: false,
        quiet: true,
        max_pages: 2,
        ..LiteParseConfig::default()
    })
    .open_batch_session(
        PdfInput::Path("../../integration_tests_data/filled_acroform.pdf".to_string()),
        100,
    )
    .await
    .expect("should open a session");

    let first = session
        .next_batch()
        .await
        .expect("batch should parse")
        .expect("a first batch");
    assert_eq!(session.total_pages(), 3, "reports the source page count");
    assert_eq!((first.start_page, first.end_page), (1, 2));
    assert_eq!(first.result.pages.len(), 2);
    assert!(
        session.next_batch().await.expect("clean end").is_none(),
        "max_pages should end the session before page 3"
    );
}

/// An explicit page selection and generated batch ranges are ambiguous
/// together, so the combination is rejected up front.
#[tokio::test]
#[serial]
async fn test_batch_parse_rejects_target_pages() {
    let opened = LiteParse::new(LiteParseConfig {
        ocr_enabled: false,
        quiet: true,
        target_pages: Some("1-2".into()),
        ..LiteParseConfig::default()
    })
    .open_batch_session(
        PdfInput::Path("../../integration_tests_data/filled_acroform.pdf".to_string()),
        25,
    )
    .await;
    assert!(
        opened.is_err(),
        "target_pages + batching should be rejected"
    );
}

#[tokio::test]
#[serial]
async fn test_parse_bytes_office_integration() {
    let env_var = std::env::var("SKIP_INTEGRATION_TESTS");
    if let Ok(v) = env_var
        && v == "yes"
    {
        return;
    }
    let fixture_path = "../../integration_tests_data/sample3.doc";
    let lit = LiteParse::new(LiteParseConfig::default());
    let data = tokio::fs::read(fixture_path)
        .await
        .expect("Should be able to read file");
    let input = PdfInput::Bytes(data);
    let parsed = lit
        .parse_input(input)
        .await
        .expect("Should be able to parse");
    assert_eq!(parsed.pages.len(), 2);
}

#[tokio::test]
#[serial]
async fn test_parse_image_integration() {
    let env_var = std::env::var("SKIP_INTEGRATION_TESTS");
    if let Ok(v) = env_var
        && v == "yes"
    {
        return;
    }
    let lit = LiteParse::new(LiteParseConfig::default());
    let parsed = lit
        .parse("../../integration_tests_data/receipt.png")
        .await
        .expect("Should be able to parse");
    assert_eq!(parsed.pages.len(), 1);
}

#[tokio::test]
#[serial]
async fn test_parse_office_doc_integration() {
    let env_var = std::env::var("SKIP_INTEGRATION_TESTS");
    if let Ok(v) = env_var
        && v == "yes"
    {
        return;
    }
    let lit = LiteParse::new(LiteParseConfig::default());
    let parsed = lit
        .parse("../../integration_tests_data/sample3.doc")
        .await
        .expect("Should be able to parse");
    assert_eq!(parsed.pages.len(), 2);
}

#[tokio::test]
#[serial]
async fn test_parse_pdf_integration() {
    let lit = LiteParse::new(LiteParseConfig {
        ocr_enabled: false,
        extract_document_metadata: true,
        ..LiteParseConfig::default()
    });
    let parsed = lit
        .parse("../../integration_tests_data/sample.pdf")
        .await
        .expect("Should be able to parse");
    assert_eq!(parsed.pages.len(), 1);
    let doc_meta = parsed.doc_meta.expect("doc_meta requested");
    assert!(doc_meta.file_version.is_some());
    assert_eq!(doc_meta.is_encrypted, Some(false));
    assert!(doc_meta.raw_file_size.is_some_and(|size| size > 0));
    assert!(doc_meta.eof_section_count.is_some_and(|count| count > 0));
    assert_eq!(doc_meta.signature_count, Some(0));
}

/// Provenance is opt-in and stays absent on the default path.
#[tokio::test]
#[serial]
async fn test_doc_meta_absent_unless_requested() {
    let lit = LiteParse::new(LiteParseConfig {
        ocr_enabled: false,
        ..LiteParseConfig::default()
    });
    let parsed = lit
        .parse("../../integration_tests_data/sample.pdf")
        .await
        .expect("Should be able to parse");
    assert!(parsed.doc_meta.is_none());
}

#[tokio::test]
#[serial]
async fn test_blocks_absent_unless_requested() {
    let lit = LiteParse::new(LiteParseConfig {
        ocr_enabled: false,
        ..LiteParseConfig::default()
    });
    let parsed = lit
        .parse("../../integration_tests_data/sample.pdf")
        .await
        .expect("Should be able to parse");
    assert!(parsed.pages.iter().all(|p| p.blocks.is_none()));
}

/// Blocks are a parallel view of the same classification the Markdown renderer
/// uses, so turning them on must not perturb the rendered Markdown, and every
/// block must report where it came from.
#[tokio::test]
#[serial]
async fn test_extract_blocks_carries_geometry_without_changing_markdown() {
    let base = LiteParseConfig {
        ocr_enabled: false,
        ..LiteParseConfig::default()
    };
    let without = LiteParse::new(base.clone())
        .parse("../../integration_tests_data/sample.pdf")
        .await
        .expect("Should be able to parse");
    let with = LiteParse::new(LiteParseConfig {
        extract_blocks: true,
        ..base
    })
    .parse("../../integration_tests_data/sample.pdf")
    .await
    .expect("Should be able to parse");

    assert_eq!(without.text, with.text, "markdown must be unchanged");

    let blocks = with.pages[0]
        .blocks
        .as_ref()
        .expect("blocks should be populated");
    assert!(!blocks.is_empty());
    assert!(
        blocks.iter().all(|b| b.bbox.is_some()),
        "every block on a text page should be located"
    );
    // Boxes describe real page regions, not placeholders.
    assert!(blocks.iter().all(|b| {
        let r = b.bbox.as_ref().unwrap();
        r.width > 0.0 && r.height > 0.0
    }));
    // Blocks come back in reading order (top-to-bottom on a single-column page).
    let ys: Vec<f32> = blocks.iter().map(|b| b.bbox.as_ref().unwrap().y).collect();
    assert!(
        ys.windows(2).all(|w| w[0] <= w[1]),
        "blocks should be in reading order, got {ys:?}"
    );
}

#[tokio::test]
#[serial]
async fn test_parse_bytes_pdf_integration() {
    let fixture_path = "../../integration_tests_data/sample.pdf";
    let lit = LiteParse::new(LiteParseConfig {
        ocr_enabled: false,
        extract_document_metadata: true,
        ..LiteParseConfig::default()
    });
    let data = tokio::fs::read(fixture_path)
        .await
        .expect("Should be able to read file");
    let expected_size = data.len() as u64;
    let input = PdfInput::Bytes(data);
    let parsed = lit
        .parse_input(input)
        .await
        .expect("Should be able to parse");
    assert_eq!(parsed.pages.len(), 1);
    assert_eq!(
        parsed.doc_meta.and_then(|meta| meta.raw_file_size),
        Some(expected_size)
    );
}

/// Stress test: many concurrent `parse_input` calls on a multi-threaded
/// tokio runtime through a single `Arc<LiteParse>`. Before the PDFium
/// process-global lock was introduced, this scenario caused malloc
/// double-free / heap corruption because PDFium FFI is not thread-safe.
///
/// We intentionally do **not** use `#[serial]` here — this test must run
/// concurrently with itself (across tasks within the test) to exercise the
/// lock. Other tests in this file are `#[serial]` so they won't race.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_concurrent_parse_does_not_crash() {
    use std::sync::Arc;
    use tokio::task::JoinSet;

    let env_var = std::env::var("SKIP_INTEGRATION_TESTS");
    if let Ok(v) = env_var
        && v == "yes"
    {
        return;
    }

    let lit = Arc::new(LiteParse::new(LiteParseConfig {
        ocr_enabled: false,
        quiet: true,
        ..LiteParseConfig::default()
    }));

    let bytes = tokio::fs::read("../../integration_tests_data/sample.pdf")
        .await
        .expect("fixture exists");

    let mut set: JoinSet<usize> = JoinSet::new();
    for _ in 0..16 {
        let lit = lit.clone();
        let bytes = bytes.clone();
        set.spawn(async move {
            let parsed = lit
                .parse_input(PdfInput::Bytes(bytes))
                .await
                .expect("parse should succeed");
            parsed.pages.len()
        });
    }

    let mut total = 0;
    while let Some(joined) = set.join_next().await {
        total += joined.expect("task panicked");
    }
    // 16 tasks × 1 page each
    assert_eq!(total, 16);
}

/// A page whose only text is painted by an annotation's `/AP /N` appearance
/// stream extracts as empty (PDFium tokenizes the page content stream only),
/// so it must be distinguishable from a genuinely blank page. See issue #378.
#[tokio::test]
#[serial]
async fn test_annotation_text_complexity_reason() {
    let lit = LiteParse::new(LiteParseConfig::default());
    let stats = lit
        .is_complex(PdfInput::Path(
            "../../integration_tests_data/annotation_text.pdf".into(),
        ))
        .await
        .expect("is_complex should succeed");

    assert_eq!(stats.len(), 1);
    let page = &stats[0];
    assert_eq!(page.text_length, 0, "annotation text is not extractable");
    assert!(page.needs_ocr);
    assert!(page.reasons.contains(&ComplexityReason::NoText));
    assert!(
        page.reasons.contains(&ComplexityReason::AnnotationText),
        "expected annotation-text, got {:?}",
        page.reasons
    );
}

/// Filled AcroForm values are visible page content even though PDFium's text
/// API does not expose widget appearance streams until they are flattened.
#[tokio::test]
#[serial]
async fn test_filled_acroform_values_are_extracted_as_text() {
    let lit = LiteParse::new(LiteParseConfig {
        ocr_enabled: false,
        output_format: OutputFormat::Markdown,
        ..Default::default()
    });
    let parsed = lit
        .parse("../../integration_tests_data/filled_acroform.pdf")
        .await
        .expect("filled form should parse");

    // See scripts/generate_filled_acroform_fixture.py for what each widget
    // is meant to exercise.
    for (expected, case) in [
        (
            "ACROFORM-CUSTOMER-7319",
            "painted directly by the appearance",
        ),
        ("2026-07-28", "painted through a nested form XObject"),
        ("50.00", "painted by the appearance and the content stream"),
    ] {
        assert_eq!(
            parsed.text.matches(expected).count(),
            1,
            "visible form value should appear exactly once ({case}): {expected}"
        );
        assert!(
            parsed.pages[0]
                .text_items
                .iter()
                .any(|item| item.text.contains(expected)),
            "form value should be a positioned text item ({case}): {expected}"
        );
    }
    assert!(
        !parsed.text.contains("DEFAULT-ONLY-SHOULD-NOT-APPEAR"),
        "an unpainted default choice must not be treated as a filled value"
    );
    assert!(
        !parsed.text.contains("ANNOTATION-ONLY-SHOULD-NOT-APPEAR"),
        "non-widget annotation appearances must not become page text"
    );
    assert!(
        !parsed.text.contains("HIDDEN-SHOULD-NOT-APPEAR"),
        "a hidden widget is never rendered, so its value is not visible text"
    );
    // Flattening replaces the page content under a widget rect with that
    // widget's appearance, so page text drawn there is dropped unless it is
    // put back. This label sits inside the `amount` rect and no appearance
    // reproduces it.
    assert_eq!(
        parsed.text.matches("PREPRINTED-LABEL").count(),
        1,
        "page text under a widget rect must survive flattening exactly once"
    );
    assert!(
        parsed.pages[0].form_fields.is_none(),
        "default text extraction must not enable structured form metadata"
    );
    // Page 3's only annotation paints its value through a nested form XObject.
    // Nothing else on that page would trigger a flatten, so this fails unless
    // the appearance walk descends into form objects.
    assert!(
        parsed.pages[2]
            .text_items
            .iter()
            .any(|item| item.text.contains("NESTED-ONLY-VALUE")),
        "a widget whose value is painted only through a nested form XObject \
         must still be detected and flattened"
    );

    let with_metadata = LiteParse::new(LiteParseConfig {
        ocr_enabled: false,
        output_format: OutputFormat::Markdown,
        extract_annotations: true,
        extract_form_fields: true,
        ..Default::default()
    })
    .parse("../../integration_tests_data/filled_acroform.pdf")
    .await
    .expect("filled form should parse with structured metadata");

    assert_eq!(
        with_metadata.pages[0].annotations.as_ref().unwrap().len(),
        6
    );
    assert!(
        with_metadata.pages[0]
            .annotations
            .as_ref()
            .unwrap()
            .iter()
            .any(|annotation| {
                annotation.subtype == "freetext"
                    && annotation.contents.as_deref() == Some("ANNOTATION-ONLY-SHOULD-NOT-APPEAR")
            }),
        "non-widget annotation metadata should remain available when requested"
    );
    let fields = with_metadata.pages[0].form_fields.as_ref().unwrap();
    assert_eq!(fields.len(), 5);
    assert!(fields.iter().any(|field| {
        field.name.as_deref() == Some("customer_name")
            && field.value.as_deref() == Some("ACROFORM-CUSTOMER-7319")
    }));
    assert!(fields.iter().any(|field| {
        field.name.as_deref() == Some("default_only_choice")
            && field.value.as_deref() == Some("DEFAULT-ONLY-SHOULD-NOT-APPEAR")
    }));
    assert_eq!(
        with_metadata.pages[1].annotations.as_ref().unwrap().len(),
        1
    );
    let second_page_fields = with_metadata.pages[1].form_fields.as_ref().unwrap();
    assert_eq!(second_page_fields.len(), 1);
    assert_eq!(
        second_page_fields[0].name.as_deref(),
        Some("complexity_sentinel")
    );
    assert_eq!(second_page_fields[0].value.as_deref(), Some("OK"));
    let third_page_fields = with_metadata.pages[2].form_fields.as_ref().unwrap();
    assert_eq!(third_page_fields.len(), 1);
    assert_eq!(third_page_fields[0].name.as_deref(), Some("nested_only"));

    // Complexity sees the flattened text. `AnnotationText` means "the text is
    // there, just outside the extractable surface" — once a widget value has
    // been promoted into page content that no longer holds, so the reason must
    // not fire and the page must not be routed to OCR to recover text the
    // parser already returned. `test_annotation_text_complexity_reason` covers
    // the non-widget appearance text the reason still exists for.
    let complexity = LiteParse::new(LiteParseConfig {
        ocr_enabled: false,
        ..Default::default()
    })
    .is_complex(PdfInput::Path(
        "../../integration_tests_data/filled_acroform.pdf".into(),
    ))
    .await
    .expect("complexity analysis should run on the flattened document");
    assert_eq!(complexity.len(), 3);
    assert!(
        !complexity[1]
            .reasons
            .contains(&ComplexityReason::AnnotationText),
        "widget text is extractable after flattening, so it is not annotation-only text"
    );
}
