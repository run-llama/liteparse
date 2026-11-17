//! Invented PDFs reproducing numbered prose misclassified as a table.
use liteparse::types::PdfInput;
use liteparse::{LiteParse, LiteParseConfig, OutputFormat};

#[tokio::test]
async fn word_positions_keep_numbered_sections_and_prose_out_of_tables_by_default() {
    let basic_headings: &[&str] = &["1 GENERAL INFORMATION", "2 DOCUMENT PURPOSE"];
    let basic_content: &[&str] = &[
        "1 GENERAL INFORMATION",
        "maintenance schedule",
        "2 DOCUMENT PURPOSE",
        "inspected each day",
    ];
    let manual_headings: &[&str] = &[
        "1 GENERAL INFORMATION",
        "1.1 USING THIS GUIDE",
        "2 DOCUMENT PURPOSE",
    ];
    let manual_content: &[&str] = &[
        "1 GENERAL INFORMATION",
        "This handbook explains",
        "1.1 USING THIS GUIDE",
        "This section describes",
        "Review the sample checklist",
        "Record any unexpected condition",
        "2 DOCUMENT PURPOSE",
        "This document is an invented example",
    ];
    // Each invented page separates section numbers from titles but keeps
    // paragraph lines together, mimicking a heading that suggests columns.
    let fixtures = [
        (
            include_bytes!("fixtures/description_lists/numeric_headings_prose.pdf").as_slice(),
            basic_headings,
            basic_content,
        ),
        (
            include_bytes!("fixtures/description_lists/numeric_headings_prose_outline.pdf")
                .as_slice(),
            basic_headings,
            basic_content,
        ),
        (
            include_bytes!("fixtures/description_lists/manual_like_sections.pdf").as_slice(),
            manual_headings,
            manual_content,
        ),
    ];
    for output_format in [OutputFormat::Markdown, OutputFormat::Json] {
        let parser = LiteParse::new(LiteParseConfig {
            ocr_enabled: false,
            output_format,
            extract_blocks: true,
            quiet: true,
            ..LiteParseConfig::default()
        });
        for (pdf, expected_headings, expected_content) in fixtures {
            let result = parser
                .parse_input(PdfInput::Bytes(pdf.to_vec()))
                .await
                .unwrap();
            let blocks = result.pages[0].blocks.as_deref().unwrap();
            assert!(
                blocks
                    .iter()
                    .all(|block| block.kind != "table" && block.kind != "merged_table"),
                "{blocks:#?}"
            );
            for heading in expected_headings {
                assert!(
                    blocks.iter().any(|block| {
                        block.kind == "heading" && block.text.as_deref() == Some(*heading)
                    }),
                    "missing heading {heading:?}: {blocks:#?}"
                );
            }
            let text = blocks
                .iter()
                .filter_map(|block| block.text.as_deref())
                .collect::<Vec<_>>()
                .join(" ");
            let mut offset = 0;
            for phrase in expected_content {
                offset += text[offset..]
                    .find(*phrase)
                    .expect("missing or reordered content")
                    + phrase.len();
            }
        }
    }
}
