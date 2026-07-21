use std::path::Path;

use liteparse::conversion::convert_data_to_pdf;
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
    assert_eq!(parsed.pages.len(), 3);
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
#[cfg(feature = "native-docx")]
async fn test_parse_docx_doc_native_integration() {
    let env_var = std::env::var("SKIP_INTEGRATION_TESTS");
    if let Ok(v) = env_var
        && v == "yes"
    {
        return;
    }
    let lit = LiteParse::new(LiteParseConfig::default());
    let parsed = lit
        .parse("../../integration_tests_data/sample.docx")
        .await
        .expect("Should be able to parse");
    assert_eq!(parsed.pages.len(), 4);
}

#[tokio::test]
#[serial]
#[cfg(feature = "native-docx")]
async fn test_parse_bytes_docx_native_integration() {
    let env_var = std::env::var("SKIP_INTEGRATION_TESTS");
    if let Ok(v) = env_var
        && v == "yes"
    {
        return;
    }
    let fixture_path = "../../integration_tests_data/sample.docx";
    let lit = LiteParse::new(LiteParseConfig::default());
    let data = tokio::fs::read(fixture_path)
        .await
        .expect("Should be able to read file");
    let input = PdfInput::Bytes(data);
    let parsed = lit
        .parse_input(input)
        .await
        .expect("Should be able to parse");
    assert_eq!(parsed.pages.len(), 4);
}

#[tokio::test]
#[serial]
async fn test_parse_pdf_integration() {
    let lit = LiteParse::new(LiteParseConfig::default());
    let parsed = lit
        .parse("../../integration_tests_data/sample.pdf")
        .await
        .expect("Should be able to parse");
    assert_eq!(parsed.pages.len(), 1);
}

#[tokio::test]
#[serial]
async fn test_parse_bytes_pdf_integration() {
    let fixture_path = "../../integration_tests_data/sample.pdf";
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
