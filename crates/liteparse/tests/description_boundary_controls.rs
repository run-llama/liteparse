//! Invented PDF controls for description-table boundaries and row recovery.
use liteparse::types::PdfInput;
use liteparse::{LiteParse, LiteParseConfig, OutputFormat};

async fn parse(pdf: Vec<u8>) -> liteparse::ParseResult {
    LiteParse::new(LiteParseConfig {
        ocr_enabled: false,
        output_format: OutputFormat::Markdown,
        extract_blocks: true,
        quiet: true,
        ..LiteParseConfig::default()
    })
    .parse_input(PdfInput::Bytes(pdf))
    .await
    .unwrap()
}

fn pdf_from_objects(objects: &[String]) -> Vec<u8> {
    let mut data = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, object) in objects.iter().enumerate() {
        offsets.push(data.len());
        data.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", i + 1).as_bytes());
    }
    let xref = data.len();
    data.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for offset in offsets {
        data.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    data.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    data
}

fn pdf_with_content(content: &str) -> Vec<u8> {
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Resources << /Font << /F1 4 0 R /F2 5 0 R >> >> /Contents 6 0 R >>".to_string(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold >>".to_string(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_two_fit_bookmarks(content: &str, first: &str, second: &str) -> Vec<u8> {
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R /Outlines 7 0 R /PageMode /UseOutlines >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Resources << /Font << /F1 4 0 R /F2 5 0 R >> >> /Contents 6 0 R >>".to_string(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold >>".to_string(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
        "<< /Type /Outlines /First 8 0 R /Last 9 0 R /Count 2 >>".to_string(),
        format!("<< /Title ({first}) /Parent 7 0 R /Dest [3 0 R /Fit] /Next 9 0 R >>"),
        format!("<< /Title ({second}) /Parent 7 0 R /Dest [3 0 R /Fit] /Prev 8 0 R >>"),
    ];
    pdf_from_objects(&objects)
}

fn text(x: i32, y: i32, font: &str, size: i32, value: &str) -> String {
    format!("BT /{font} {size} Tf 1 0 0 1 {x} {y} Tm ({value}) Tj ET\n")
}

fn merged_text(x: i32, y: i32, value: &str) -> String {
    text(x, y, "F1", 10, value)
}

fn assert_rows(result: &liteparse::ParseResult, expected: &[&[&str]]) {
    let blocks = result.pages[0].blocks.as_deref().unwrap();
    let tables: Vec<_> = blocks
        .iter()
        .filter(|block| block.kind == "table")
        .collect();
    assert_eq!(tables.len(), 1, "blocks: {blocks:#?}");
    let rows: Vec<Vec<&str>> = tables[0]
        .rows
        .as_ref()
        .unwrap()
        .iter()
        .map(|row| row.iter().map(|cell| cell.text.as_str()).collect())
        .collect();
    assert_eq!(
        rows,
        expected.iter().map(|row| row.to_vec()).collect::<Vec<_>>(),
        "blocks: {blocks:#?}"
    );
}

fn assert_reconstructed_cells_have_no_bbox(result: &liteparse::ParseResult, row_indexes: &[usize]) {
    let table = result.pages[0]
        .blocks
        .as_deref()
        .unwrap()
        .iter()
        .find(|block| block.kind == "table")
        .unwrap();
    let rows = table.rows.as_ref().unwrap();
    for &index in row_indexes {
        assert!(
            rows[index].iter().all(|cell| cell.bbox.is_none()),
            "cells reconstructed from merged spans should not have fabricated bboxes: {:#?}",
            rows[index]
        );
    }
}

#[tokio::test]
async fn heading_and_prose_before_description_table_remain_ordered() {
    let mut content = String::new();
    content += &text(48, 782, "F2", 24, "DEVICE DETAILS");
    content += &text(
        48,
        750,
        "F1",
        10,
        "This invented paragraph introduces the device settings.",
    );
    content += &text(48, 732, "F1", 10, "Mode");
    content += &text(
        180,
        732,
        "F1",
        10,
        "Description of the fictional machine mode",
    );
    content += &text(48, 714, "F1", 10, "Size");
    content += &text(
        180,
        714,
        "F1",
        10,
        "Description of the size of the fictional machine",
    );
    content += &text(48, 696, "F1", 10, "Type");
    content += &text(
        180,
        696,
        "F1",
        10,
        "Description of the type of the fictional machine",
    );
    let result = parse(pdf_with_content(&content)).await;
    assert_rows(
        &result,
        &[
            &["Mode", "Description of the fictional machine mode"],
            &["Size", "Description of the size of the fictional machine"],
            &["Type", "Description of the type of the fictional machine"],
        ],
    );
    let blocks = result.pages[0].blocks.as_deref().unwrap();
    let heading = blocks
        .iter()
        .position(|b| {
            b.kind == "heading" && b.text.as_deref().unwrap_or("").contains("DEVICE DETAILS")
        })
        .unwrap();
    let prose = blocks
        .iter()
        .position(|b| {
            b.kind == "paragraph"
                && b.text
                    .as_deref()
                    .unwrap_or("")
                    .contains("invented paragraph introduces")
        })
        .unwrap();
    let table = blocks.iter().position(|b| b.kind == "table").unwrap();
    assert!(
        heading < prose && prose < table,
        "heading, prose, table order changed: {blocks:#?}"
    );
}

#[tokio::test]
async fn numbered_bold_row_and_split_merged_rows_stay_in_one_table() {
    let mut content = String::new();
    content += &text(100, 782, "F2", 24, "DEVICE DESCRIPTION");
    // The marker and title are separate cells, as in the table's first row.
    content += &text(100, 722, "F2", 16, "1");
    content += &text(143, 722, "F2", 16, "DEVICE CONFIGURATION");
    // These rows are real two-span rows and establish the column tracks.
    content += &text(100, 695, "F1", 10, "Mode");
    content += &text(143, 695, "F1", 10, "automatic operation");
    content += &text(100, 683, "F1", 10, "Type");
    content += &text(143, 683, "F1", 10, "standard equipment");
    // Each following row is deliberately emitted as one PDF text object/span.
    // The inferred whitespace boundary at x=143 must recover both cells.
    content += &merged_text(100, 671, "Voltage nominal power rating");
    content += &merged_text(100, 659, "Current nominal current rating");
    let result = parse(pdf_with_content(&content)).await;
    let text_items = &result.pages[0].text_items;
    for merged in [
        "Voltage nominal power rating",
        "Current nominal current rating",
    ] {
        assert_eq!(
            text_items.iter().filter(|item| item.text == merged).count(),
            1,
            "the invented PDF must yield {merged:?} as one PDFium text item; items: {text_items:#?}"
        );
    }
    assert_rows(
        &result,
        &[
            &["1", "DEVICE CONFIGURATION"],
            &["Mode", "automatic operation"],
            &["Type", "standard equipment"],
            &["Voltage", "nominal power rating"],
            &["Current", "nominal current rating"],
        ],
    );
    assert_reconstructed_cells_have_no_bbox(&result, &[3, 4]);
}

#[tokio::test]
async fn numeric_observed_rows_support_reconstructed_alpha_rows() {
    let mut content = String::new();
    content += &text(100, 782, "F2", 24, "DEVICE DESCRIPTION");
    // Numeric labels provide two genuine observed cell boundaries.
    content += &text(100, 722, "F1", 10, "1");
    content += &text(143, 722, "F1", 10, "UNIT MODE");
    content += &text(100, 704, "F1", 10, "2");
    content += &text(143, 704, "F1", 10, "UNIT TYPE");
    // Alphabetic label/value pairs are each one PDFium text item.
    content += &merged_text(100, 686, "Voltage nominal power rating");
    content += &merged_text(100, 668, "Current nominal current rating");
    let result = parse(pdf_with_content(&content)).await;
    let text_items = &result.pages[0].text_items;
    for merged in [
        "Voltage nominal power rating",
        "Current nominal current rating",
    ] {
        assert_eq!(
            text_items.iter().filter(|item| item.text == merged).count(),
            1,
            "the invented PDF must yield {merged:?} as one PDFium text item; items: {text_items:#?}"
        );
    }
    assert_rows(
        &result,
        &[
            &["1", "UNIT MODE"],
            &["2", "UNIT TYPE"],
            &["Voltage", "nominal power rating"],
            &["Current", "nominal current rating"],
        ],
    );
    assert_reconstructed_cells_have_no_bbox(&result, &[2, 3]);
}

#[tokio::test]
async fn bookmarked_numeric_rows_and_reconstructed_alpha_rows_stay_one_table() {
    let mut content = String::new();
    content += &text(100, 782, "F2", 24, "DEVICE DESCRIPTION");
    // The bold observed rows are structurally a table, with outline titles
    // matching each full numbered heading on this same page.
    content += &text(100, 722, "F2", 16, "1");
    content += &text(143, 722, "F2", 16, "GENERAL INFORMATION");
    content += &text(100, 704, "F2", 16, "2");
    content += &text(143, 704, "F2", 16, "DOCUMENT PURPOSE");
    // Non-bold alpha rows each arrive as one text span and need reconstruction.
    content += &merged_text(100, 686, "Voltage nominal power rating");
    content += &merged_text(100, 668, "Current nominal current rating");
    let pdf = pdf_with_two_fit_bookmarks(&content, "1 GENERAL INFORMATION", "2 DOCUMENT PURPOSE");
    let result = parse(pdf).await;
    for merged in [
        "Voltage nominal power rating",
        "Current nominal current rating",
    ] {
        assert_eq!(
            result.pages[0]
                .text_items
                .iter()
                .filter(|item| item.text == merged)
                .count(),
            1,
            "expected {merged:?} to be one PDFium text item"
        );
    }
    assert_rows(
        &result,
        &[
            &["1", "GENERAL INFORMATION"],
            &["2", "DOCUMENT PURPOSE"],
            &["Voltage", "nominal power rating"],
            &["Current", "nominal current rating"],
        ],
    );
    assert_reconstructed_cells_have_no_bbox(&result, &[2, 3]);
}
