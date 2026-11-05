use liteparse::config::OutputFormat;
use liteparse::{LiteParse, LiteParseConfig};
use serial_test::serial;

const PDF_PATH: &str = "../../integration_tests_data/wrapped_outline_heading_546.pdf";
const HEADING: &str = "2 CONFIGURATION OF MACHINE AND OPERATING PROCEDURES";

fn config(target_pages: Option<&str>) -> LiteParseConfig {
    LiteParseConfig {
        ocr_enabled: false,
        quiet: true,
        output_format: OutputFormat::Markdown,
        extract_blocks: true,
        target_pages: target_pages.map(str::to_owned),
        ..LiteParseConfig::default()
    }
}

#[tokio::test]
#[serial]
async fn wrapped_outline_heading_stays_one_h1_in_full_and_targeted_parse() {
    let full = LiteParse::new(config(None))
        .parse(PDF_PATH)
        .await
        .expect("whole-document parse should succeed");
    let targeted = LiteParse::new(config(Some("2")))
        .parse(PDF_PATH)
        .await
        .expect("targeted page parse should succeed");

    assert_eq!(full.total_pages, 2);
    assert_eq!(full.pages.len(), 2);
    assert_eq!(targeted.pages.len(), 1);
    assert_eq!(targeted.pages[0].page_number, 2);

    let page_two = &full.pages[1];
    let matching_headings: Vec<_> = page_two
        .blocks
        .as_ref()
        .expect("blocks should be available")
        .iter()
        .filter(|block| {
            block.kind == "heading"
                && block
                    .text
                    .as_deref()
                    .is_some_and(|t| t.contains("CONFIGURATION"))
        })
        .collect();
    assert_eq!(
        matching_headings.len(),
        1,
        "the wrapped title should be one heading block"
    );
    assert_eq!(matching_headings[0].text.as_deref(), Some(HEADING));
    assert_eq!(matching_headings[0].level, Some(1));
    let bbox = matching_headings[0]
        .bbox
        .as_ref()
        .expect("the merged heading block should retain its source geometry");
    assert!(bbox.width > 0.0 && bbox.height > 0.0);

    let h1 = format!("# {HEADING}");
    let h2 = "## OPERATING PROCEDURES";
    assert_eq!(page_two.markdown.matches(&h1).count(), 1);
    assert!(
        !page_two.markdown.contains(h2),
        "the wrapped line must not become an H2"
    );
    assert_eq!(targeted.pages[0].markdown, page_two.markdown);
    assert_eq!(targeted.text, page_two.markdown);
    assert!(full.text.contains(&full.pages[0].markdown));
    assert!(full.text.contains(&page_two.markdown));
}
