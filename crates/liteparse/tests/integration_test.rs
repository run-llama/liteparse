use std::path::Path;

use liteparse::config::OutputFormat;
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

    // Provenance (fork): every block attributes to the page's returned
    // text_items — indices in bounds, sorted+deduped, non-empty for
    // text-bearing kinds, and never shared between two different blocks.
    let n_items = with.pages[0].text_items.len();
    let mut seen: std::collections::HashSet<usize> = std::collections::HashSet::new();
    for b in blocks {
        assert!(
            b.text_item_indices.iter().all(|&i| i < n_items),
            "index out of bounds in {:?}",
            b.kind
        );
        assert!(
            b.text_item_indices.windows(2).all(|w| w[0] < w[1]),
            "block indices must be strictly ascending (sorted+deduped)"
        );
        if !matches!(b.kind, "rule" | "figure") {
            assert!(
                !b.text_item_indices.is_empty(),
                "text-bearing block {:?} carries no provenance",
                b.kind
            );
        }
        for &i in &b.text_item_indices {
            assert!(
                seen.insert(i),
                "item {i} attributed to two different blocks"
            );
        }
    }
}

/// `extract_blocks` needs real word geometry for table detection (word-anchored
/// straddle splits), so it forces word-box extraction internally — under any
/// output format, exactly like markdown output always has. The forcing is a
/// detection input only: the parser's reported config still says what the
/// caller asked for, and with `extract_blocks` off nothing changes.
#[tokio::test]
#[serial]
async fn test_extract_blocks_forces_word_geometry_json_output() {
    let base = LiteParseConfig {
        ocr_enabled: false,
        output_format: OutputFormat::Json,
        ..LiteParseConfig::default()
    };

    // Control: JSON output without extract_blocks populates no word boxes.
    let without = LiteParse::new(base.clone());
    let parsed = without
        .parse("../../integration_tests_data/sample.pdf")
        .await
        .expect("Should be able to parse");
    assert!(
        parsed
            .pages
            .iter()
            .flat_map(|p| &p.text_items)
            .all(|i| i.words.is_empty()),
        "no word boxes should be extracted when neither the caller nor a \
         feature asks for them"
    );

    let with = LiteParse::new(LiteParseConfig {
        extract_blocks: true,
        ..base
    });
    let parsed = with
        .parse("../../integration_tests_data/sample.pdf")
        .await
        .expect("Should be able to parse");
    assert!(
        parsed
            .pages
            .iter()
            .flat_map(|p| &p.text_items)
            .any(|i| !i.words.is_empty()),
        "extract_blocks should force word-box extraction as a table-detection \
         input even under JSON output"
    );
    // The forcing is internal: the resolved config still reports the caller's
    // own request, which is what binding layers echo and gate `words`
    // serialization on.
    assert!(!with.config().emit_word_boxes);
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

/// A blank multi-page PDF: no text, so every page is text-poor and routes to
/// OCR. Pages take distinct sizes so each page's raster is uniquely
/// identifiable by the dimensions the OCR engine receives.
fn blank_pdf(page_sizes: &[(u32, u32)]) -> Vec<u8> {
    let kids: Vec<String> = (0..page_sizes.len())
        .map(|i| format!("{} 0 R", i + 3))
        .collect();
    let mut objects: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        format!(
            "<< /Type /Pages /Kids [{}] /Count {} >>",
            kids.join(" "),
            page_sizes.len()
        )
        .into_bytes(),
    ];
    for (width, height) in page_sizes {
        objects.push(
            format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {width} {height}] >>")
                .into_bytes(),
        );
    }
    let mut pdf = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::with_capacity(objects.len());
    for (index, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        pdf.extend_from_slice(object);
        pdf.extend_from_slice(b"\nendobj\n");
    }
    let xref = pdf.len();
    pdf.extend_from_slice(format!("xref\n0 {}\n", objects.len() + 1).as_bytes());
    pdf.extend_from_slice(b"0000000000 65535 f \n");
    for offset in offsets {
        pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF",
            objects.len() + 1,
            xref
        )
        .as_bytes(),
    );
    pdf
}

/// An OCR engine that reports each raster's dimensions as its recognized
/// text (so misrouted rasters are detectable per page), counts calls, and
/// tracks the concurrent-recognition high-water mark (so the round structure
/// itself is observable: serialized rounds of one page cap it at 1).
struct ProbeEngine {
    calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    in_flight: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    peak_in_flight: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl liteparse::ocr::OcrEngine for ProbeEngine {
    fn name(&self) -> &str {
        "probe"
    }
    fn recognize<'a, 'b: 'a, 'c: 'a>(
        &'a self,
        _image_data: &'c [u8],
        width: u32,
        height: u32,
        _options: &'b liteparse::ocr::OcrOptions,
    ) -> std::pin::Pin<
        Box<
            dyn Future<
                    Output = Result<
                        Vec<liteparse::ocr::OcrResult>,
                        Box<dyn std::error::Error + Send + Sync>,
                    >,
                > + Send
                + '_,
        >,
    > {
        use std::sync::atomic::Ordering;
        self.calls.fetch_add(1, Ordering::SeqCst);
        let now_in_flight = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak_in_flight
            .fetch_max(now_in_flight, Ordering::SeqCst);
        let in_flight = self.in_flight.clone();
        Box::pin(async move {
            // Hold the slot briefly so overlapping recognitions overlap
            // observably; a serialized round of one page keeps the peak at 1.
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            in_flight.fetch_sub(1, Ordering::SeqCst);
            Ok(vec![liteparse::ocr::OcrResult {
                text: format!("DIM{width}x{height}"),
                bbox: [10.0, 10.0, 200.0, 40.0],
                confidence: 0.99,
                polygon: None,
            }])
        })
    }
}

/// OCR runs in render→recognize rounds of `num_workers` pages. Whatever the
/// round size, every page must be recognized exactly once and each page's OCR
/// text must land on the page whose raster produced it — distinct page sizes
/// make a misroute visible, which is the failure mode the per-round document
/// reopen and form-widget re-flatten could introduce. `num_workers: 1` forces
/// one page per round (four rounds over four pages) and must also serialize
/// recognition; `num_workers: 4` covers the whole document in a single round
/// with overlapping recognition. Both must agree page-for-page.
#[tokio::test]
#[serial]
async fn test_ocr_rounds_cover_every_page_once() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    // Distinct sizes so a raster delivered to the wrong page is detectable.
    let page_sizes: [(u32, u32); 4] = [(612, 792), (400, 500), (300, 300), (500, 900)];
    let expected_texts: Vec<String> = page_sizes
        .iter()
        .map(|(width, height)| {
            let scale = 150.0 / 72.0;
            format!(
                "DIM{}x{}",
                (*width as f32 * scale).round() as u32,
                (*height as f32 * scale).round() as u32
            )
        })
        .collect();

    let run = |num_workers: usize| {
        let calls = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let engine = ProbeEngine {
            calls: calls.clone(),
            in_flight: Arc::new(AtomicUsize::new(0)),
            peak_in_flight: peak.clone(),
        };
        let parser = LiteParse::new(LiteParseConfig {
            ocr_enabled: true,
            num_workers,
            dpi: 150.0,
            quiet: true,
            ..Default::default()
        })
        .with_ocr_engine(std::sync::Arc::new(engine));
        (parser, calls, peak)
    };

    let assert_pages_carry_own_rasters = |result: &liteparse::ParseResult, label: &str| {
        assert_eq!(result.pages.len(), page_sizes.len(), "{label}: page count");
        for (page, expected) in result.pages.iter().zip(&expected_texts) {
            let text: String = page
                .text_items
                .iter()
                .map(|item| item.text.as_str())
                .collect();
            assert!(
                text.contains(expected.as_str()),
                "{label}: page {} carries {text:?}, expected {expected:?} — OCR text landed on the wrong page",
                page.page_number
            );
        }
    };

    // One page per round: four rounds, recognition fully serialized.
    let (parser, calls, peak) = run(1);
    let serialized = parser
        .parse_input(PdfInput::Bytes(blank_pdf(&page_sizes)))
        .await
        .expect("single-page-round OCR parse should succeed");
    assert_pages_carry_own_rasters(&serialized, "rounds of 1");
    assert_eq!(
        calls.load(Ordering::SeqCst),
        page_sizes.len(),
        "one recognition per page"
    );
    assert_eq!(
        peak.load(Ordering::SeqCst),
        1,
        "num_workers=1 must serialize recognition"
    );

    // Round wide enough for the whole document: one round, overlapping
    // recognition, identical routing.
    let (parser, calls, peak) = run(4);
    let overlapped = parser
        .parse_input(PdfInput::Bytes(blank_pdf(&page_sizes)))
        .await
        .expect("single-round OCR parse should succeed");
    assert_pages_carry_own_rasters(&overlapped, "rounds of 4");
    assert_eq!(calls.load(Ordering::SeqCst), page_sizes.len());
    assert!(
        peak.load(Ordering::SeqCst) > 1,
        "num_workers=4 over 4 pages must overlap recognition within the round"
    );
}

/// A round is bounded by rasters rendered, not by page span, so OCR-needing
/// pages that are sparsely scattered through a mostly-native-text document
/// still fill a round and recognize concurrently.
///
/// This guards a real regression: bounding the round by page span instead
/// made each round contain only the OCR-needing pages that happened to fall
/// inside its span — often one or two — which starved the worker pool and
/// serialized recognition. On this document that was a 2.4x wall-clock loss
/// at realistic OCR latency, with no test failing.
#[tokio::test]
#[serial]
async fn test_ocr_rounds_fill_across_sparse_pages() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let calls = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let parser = LiteParse::new(LiteParseConfig {
        ocr_enabled: true,
        num_workers: 8,
        dpi: 72.0, // Small rasters: this test is about scheduling, not pixels.
        quiet: true,
        ..Default::default()
    })
    .with_ocr_engine(std::sync::Arc::new(ProbeEngine {
        calls: calls.clone(),
        in_flight: Arc::new(AtomicUsize::new(0)),
        peak_in_flight: peak.clone(),
    }));

    let result = parser
        .parse("../../demo/docs/apple-10k-2024.pdf")
        .await
        .expect("should parse the 10-K with OCR enabled");

    let recognized = calls.load(Ordering::SeqCst);
    assert!(
        recognized > 8,
        "expected the fixture to need OCR on more than one round's worth of pages, got {recognized}"
    );
    assert!(
        result.pages.len() > recognized,
        "fixture should be mostly native text, so OCR pages ({recognized}) must be sparse \
         among its {} pages",
        result.pages.len()
    );
    // The scan-ahead must gather a full round even though the OCR-needing
    // pages are interleaved with native-text pages it skips.
    assert_eq!(
        peak.load(Ordering::SeqCst),
        8,
        "rounds must fill to num_workers across skipped pages; a lower peak means \
         rounds are being cut short by page span"
    );
}
