//! A document whose viewer preferences say `/Direction /R2L` makes PDFium hand back the
//! pieces of any left-to-right line that needs bidi reordering (a "™" is enough) in
//! right-to-left order, with the brackets it moved mirrored. The text still reads left to
//! right (issue #503).
use liteparse::types::PdfInput;
use liteparse::{LiteParse, LiteParseConfig};

fn pdf(direction: &str) -> Vec<u8> {
    let content = "BT /F1 1 Tf 12 0 0 12 72 700 Tm \
[(W) -0.6 (.I.S) -0.6 (.E.-CS) -0.6 (I 300 Ch) -0.6 (ina Tracker\\231,)] TJ \
12 0 0 12 72 670 Tm [(Charges \\(Note 2\\) Tr) -0.6 (ust\\231 \\(40\\))] TJ ET";
    let objects = [
        format!("<< /Type /Catalog /Pages 2 0 R /ViewerPreferences << /Direction /{direction} >> >>"),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>".to_string(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold /Encoding /WinAnsiEncoding >>".to_string(),
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

async fn lines(bytes: Vec<u8>) -> Vec<String> {
    let parser = LiteParse::new(LiteParseConfig {
        ocr_enabled: false,
        quiet: true,
        ..LiteParseConfig::default()
    });
    let result = parser.parse_input(PdfInput::Bytes(bytes)).await.unwrap();
    result.pages[0]
        .text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|l| l.replace("TM", "™"))
        .collect()
}

#[tokio::test]
async fn r2l_viewer_direction_keeps_ltr_lines_in_order() {
    let expected = [
        "W.I.S.E.-CSI 300 China Tracker™,",
        "Charges (Note 2) Trust™ (40)",
    ];
    assert_eq!(lines(pdf("L2R")).await, expected);
    assert_eq!(lines(pdf("R2L")).await, expected);
}
