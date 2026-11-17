//! Word spaces survive when projection rejoins split items whose boxes touch.
use liteparse::types::PdfInput;
use liteparse::{LiteParse, LiteParseConfig};

/// Text drawn at `1 Tf` and sized by a sheared text matrix (a synthetic
/// oblique), with the tight TJ kerning some report generators emit. The shear
/// widens each glyph's loose box until neighbouring words touch across their
/// spaces, whether a space glyph or a TJ displacement (`(Lauderdale,) -278`).
fn sheared_text_matrix_pdf() -> Vec<u8> {
    let content = "BT /F1 1 Tf 7.125 0 1.5145 7.125 20 60 Tm \
[(110 ) -28 (SE ) -28 (6th ) -28 (Str) 18 (eet, ) -28 (Suite ) -28 (2800)] TJ ET\n\
BT /F1 1 Tf 7.125 0 1.5145 7.125 20 40 Tm \
[(Fort ) -28 (Lauderdale,) -278 (Florida ) -28 (33301)] TJ ET";
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>".to_string(),
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

#[tokio::test]
async fn sheared_text_matrix_keeps_word_spaces() {
    let parser = LiteParse::new(LiteParseConfig {
        ocr_enabled: false,
        quiet: true,
        ..LiteParseConfig::default()
    });
    let result = parser
        .parse_input(PdfInput::Bytes(sheared_text_matrix_pdf()))
        .await
        .unwrap();
    let lines: Vec<&str> = result.pages[0]
        .text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    assert_eq!(
        lines,
        [
            "110 SE 6th Street, Suite 2800",
            "Fort Lauderdale, Florida 33301"
        ]
    );
}
