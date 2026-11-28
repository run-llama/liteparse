//! A word space drawn as its own text object survives when PDFium skips that object.
use liteparse::types::PdfInput;
use liteparse::{LiteParse, LiteParseConfig};

/// Headings the way Word exports them: the number, a lone `( )Tj` space and the title
/// are separate text objects, and the title starts only 0.16 em after the space's origin,
/// so the gap is narrower than a normal word space. Helvetica's space glyph has no
/// outline, so PDFium drops the space object from the text page. The second line draws
/// one word as two objects with no space between them and must stay one word.
fn heading_pdf() -> Vec<u8> {
    let content = "BT /F1 1 Tf 12 0 0 12 72 700 Tm (51-8.1)Tj 2.84 0 Td ( )Tj 0.16 0 Td \
(Savings Provisions)Tj ET\n\
BT /F1 1 Tf 12 0 0 12 72 670 Tm (51-9.1 Auth)Tj (ority)Tj ET";
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>".to_string(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold >>".to_string(),
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

#[tokio::test]
async fn lone_space_object_keeps_word_break() {
    let parser = LiteParse::new(LiteParseConfig {
        ocr_enabled: false,
        quiet: true,
        ..LiteParseConfig::default()
    });
    let result = parser
        .parse_input(PdfInput::Bytes(heading_pdf()))
        .await
        .unwrap();
    let lines: Vec<&str> = result.pages[0]
        .text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    assert_eq!(lines, ["51-8.1 Savings Provisions", "51-9.1 Authority"]);
}
