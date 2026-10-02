//! `extract_blocks` must report the same tables under every output format.
use liteparse::config::OutputFormat;
use liteparse::types::PdfInput;
use liteparse::{LiteParse, LiteParseConfig};

async fn tables(output_format: OutputFormat) -> serde_json::Value {
    let parser = LiteParse::new(LiteParseConfig {
        ocr_enabled: false,
        quiet: true,
        extract_blocks: true,
        output_format,
        target_pages: Some("39".into()),
        ..LiteParseConfig::default()
    });
    let result = parser
        .parse_input(PdfInput::Path(
            "../../demo/docs/apple-10k-2024.pdf".to_string(),
        ))
        .await
        .unwrap();
    let blocks = result.pages[0].blocks.as_ref().unwrap();
    serde_json::to_value(
        blocks
            .iter()
            .filter(|b| b.kind.contains("table"))
            .collect::<Vec<_>>(),
    )
    .unwrap()
}

/// Page 39's fair-value table packs several amounts into single PDFium runs;
/// splitting them right needs word boxes, which markdown output always had.
#[tokio::test]
async fn json_and_markdown_report_the_same_tables() {
    let json = tables(OutputFormat::Json).await;
    assert!(!json.as_array().unwrap().is_empty());
    assert_eq!(json, tables(OutputFormat::Markdown).await);
}
