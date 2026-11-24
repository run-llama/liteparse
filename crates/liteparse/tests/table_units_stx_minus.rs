//! pdfium reports U+0002 for a font that draws a minus as code 0x02 with no ToUnicode map
//! (some publishers' math fonts). The table model's units read it as '-', as the text output
//! does, instead of dropping it and flipping the sign of the value (issue #620).
use liteparse::table_model::units::pdf_units;
use pdfium::Library;

fn pdf(content: &str) -> Vec<u8> {
    // F2: a Type3 font whose only glyph, code 0x02, is a bar; no ToUnicode map
    let glyph = "584 0 40 230 544 290 d1\n40 230 504 60 re f";
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 600 800] /Resources << /Font << /F1 4 0 R /F2 5 0 R >> >> /Contents 6 0 R >>".to_string(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Courier >>".to_string(),
        "<< /Type /Font /Subtype /Type3 /FontBBox [0 0 600 700] /FontMatrix [0.001 0 0 0.001 0 0] /CharProcs << /g2 7 0 R >> /Encoding << /Type /Encoding /Differences [2 /g2] >> /FirstChar 2 /LastChar 2 /Widths [584] /Resources << >> >>".to_string(),
        stream(content),
        stream(glyph),
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

fn stream(content: &str) -> String {
    format!(
        "<< /Length {} >>\nstream\n{content}\nendstream",
        content.len()
    )
}

fn unit_texts(bytes: &[u8]) -> Vec<String> {
    let lib = Library::init();
    let doc = lib.load_document_from_bytes(bytes, None).unwrap();
    let page = doc.page(0).unwrap();
    pdf_units(&page).0.into_iter().map(|u| u.text).collect()
}

#[test]
fn stx_minus_reads_as_dash() {
    let content = "BT /F1 9 Tf 50 700 Td (log) Tj ET \
                   BT /F2 9 Tf 150 700 Td <02> Tj ET BT /F1 9 Tf 155.26 700 Td (0.09) Tj ET \
                   BT /F1 9 Tf 210 700 Td ([) Tj ET BT /F2 9 Tf 215.4 700 Td <02> Tj ET \
                   BT /F1 9 Tf 220.66 700 Td (2.29]) Tj ET";
    let texts = unit_texts(&pdf(content));
    let joined = texts.join("|");
    assert!(!joined.contains('\u{2}'), "control code leaked: {joined:?}");
    assert!(texts.iter().any(|t| t == "-0.09"), "minus lost: {joined:?}");
    assert!(
        texts.iter().any(|t| t == "[-2.29]"),
        "minus lost: {joined:?}"
    );
}

#[test]
fn line_end_hyphen_stays_a_word_break() {
    let content = "BT /F1 10 Tf 50 700 Td (Used improperly, technolo-) Tj 0 -12 Td (gies can and do result) Tj ET";
    let joined = unit_texts(&pdf(content)).join("|");
    assert!(!joined.contains('\u{2}'), "control code leaked: {joined:?}");
    assert!(
        !joined.contains("technolo-"),
        "line-end hyphen kept: {joined:?}"
    );
}
