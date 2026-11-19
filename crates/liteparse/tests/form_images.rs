//! Scans drawn inside a form XObject still count as full-page images.
use liteparse::stages::{self, ComplexityReason, PageComplexityStats};
use liteparse::types::PdfInput;
use pdfium::Library;

/// 600x800 page with image `/Im1` and form `/Fx1` (drawing `/Im1`).
fn pdf(content: &str, form_matrix: &str) -> Vec<u8> {
    let form_content = "/Im1 Do";
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 600 800] /Contents 4 0 R /Resources << /Font << /F1 7 0 R >> /XObject << /Im1 5 0 R /Fx1 6 0 R >> >> >>".to_string(),
        stream("", content),
        stream(
            "/Type /XObject /Subtype /Image /Width 2 /Height 2 /ColorSpace /DeviceGray /BitsPerComponent 8 /Filter /ASCIIHexDecode",
            "00FFFF00>",
        ),
        stream(
            &format!(
                "/Type /XObject /Subtype /Form /BBox [0 0 1 1] /Matrix [{form_matrix}] /Resources << /XObject << /Im1 5 0 R >> >>"
            ),
            form_content,
        ),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Courier >>".to_string(),
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

fn stream(dict: &str, content: &str) -> String {
    format!(
        "<< {dict} /Length {} >>\nstream\n{content}\nendstream",
        content.len()
    )
}

fn complexity(bytes: Vec<u8>) -> PageComplexityStats {
    let lib = Library::init();
    let document = stages::open(&lib, &PdfInput::Bytes(bytes), None, &[]).unwrap();
    let pages = stages::extract(&document, &stages::ExtractRequest::default())
        .unwrap()
        .pages;
    stages::page_complexity(&document, &pages[0]).unwrap()
}

const STAMP: &str = "BT /F1 8 Tf 20 20 Td (Stamp 01) Tj ET";

#[test]
fn scan_inside_form_is_a_full_page_image() {
    let stats = complexity(pdf(&format!("q /Fx1 Do Q {STAMP}"), "600 0 0 800 0 0"));
    assert!(stats.full_page_image);
    assert!(
        stats.reasons.contains(&ComplexityReason::Scanned),
        "{:?}",
        stats.reasons
    );
}

#[test]
fn small_image_inside_form_is_not_a_full_page_image() {
    let stats = complexity(pdf(&format!("q /Fx1 Do Q {STAMP}"), "200 0 0 100 50 50"));
    assert!(!stats.full_page_image);
    assert!(!stats.reasons.contains(&ComplexityReason::Scanned));
    assert!(!stats.reasons.contains(&ComplexityReason::EmbeddedImages));
}
