//! Synthetic PDFs keep table-geometry regressions independent of customer documents.
use liteparse::types::{PdfInput, Rect};
use liteparse::{LiteParse, LiteParseConfig};

/// A one-page 595x842 PDF holding a ruled 3x3 table whose last column is
/// stroked as a single rect running to x=5600, far past the right page edge.
fn offpage_table_pdf() -> Vec<u8> {
    let mut content = String::from("0.5 w\n");
    for y in [700, 680, 660, 640] {
        content += &format!("100 {y} m 400 {y} l S\n");
    }
    for x in [100, 250, 400] {
        content += &format!("{x} 640 m {x} 700 l S\n");
    }
    content += "400 640 5200 60 re S\n";
    let rows = [
        ["Item", "Qty", "Notes"],
        ["Apples", "3", "fresh"],
        ["Pears", "5", "ripe"],
    ];
    for (r, row) in rows.iter().enumerate() {
        let y = 686 - 20 * r;
        for (x, text) in [104, 254, 404].iter().zip(row) {
            content += &format!("BT /F1 10 Tf {x} {y} Td ({text}) Tj ET\n");
        }
    }
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>".to_string(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
    ];
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

fn assert_on_page(r: &Rect, width: f32, height: f32) {
    assert!(
        r.x >= 0.0 && r.y >= 0.0 && r.x + r.width <= width && r.y + r.height <= height,
        "{r:?} escapes the {width}x{height} page"
    );
}

#[tokio::test]
async fn table_boxes_stay_on_the_page() {
    let parser = LiteParse::new(LiteParseConfig {
        ocr_enabled: false,
        quiet: true,
        extract_blocks: true,
        ..LiteParseConfig::default()
    });
    let result = parser
        .parse_input(PdfInput::Bytes(offpage_table_pdf()))
        .await
        .unwrap();
    let page = &result.pages[0];
    let blocks = page.blocks.as_ref().unwrap();
    let table = blocks.iter().find(|b| b.kind == "table").unwrap();
    let (width, height) = (page.page_width, page.page_height);
    assert_on_page(table.bbox.as_ref().unwrap(), width, height);
    let cells = table
        .header
        .iter()
        .flatten()
        .chain(table.rows.iter().flatten().flatten());
    let mut count = 0;
    for cell in cells {
        assert_on_page(cell.bbox.as_ref().unwrap(), width, height);
        count += 1;
    }
    assert_eq!(count, 9);
}
