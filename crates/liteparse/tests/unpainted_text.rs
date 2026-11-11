//! Synthetic PDFs for text that is drawn but never changes a pixel of the page.
use liteparse::stages;
use pdfium::Library;

fn pdf(content: &str) -> Vec<u8> {
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 600 800] /Resources << /Font << /F1 4 0 R >> /ExtGState << /Half 6 0 R >> /Pattern << /Grad 7 0 R >> >> /Contents 5 0 R >>".to_string(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
        format!(
            "<< /Length {} >>\nstream\n{content}\nendstream",
            content.len()
        ),
        "<< /ca 0.5 >>".to_string(),
        "<< /PatternType 2 /Shading << /ShadingType 2 /ColorSpace /DeviceRGB /Coords [0 0 600 0] /Function << /FunctionType 2 /Domain [0 1] /C0 [0 0 1] /C1 [1 0 0] /N 1 >> >> >>".to_string(),
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

fn extracted(content: &str) -> Vec<String> {
    let bytes = pdf(content);
    let lib = Library::init();
    let doc = lib.load_document_from_bytes(&bytes, None).unwrap();
    let result = stages::extract(&doc, &stages::ExtractRequest::default()).unwrap();
    result.pages[0]
        .text_items
        .iter()
        .map(|i| i.text.clone())
        .collect()
}

fn text(label: &str, y: u32, color: &str) -> String {
    format!("BT /F1 14 Tf {color} 60 {y} Td ({label}) Tj ET\n")
}

#[test]
fn white_text_on_an_unpainted_page_is_dropped() {
    let content = text("SHOWN", 700, "0 g") + &text("HIDDEN", 650, "1 g");
    assert_eq!(extracted(&content), ["SHOWN"]);
}

#[test]
fn text_filled_like_the_rectangle_beneath_it_is_dropped() {
    let content = "1 0 0 rg 50 590 400 26 re f\n".to_string()
        + &text("HIDDEN", 598, "1 0 0 rg")
        + &text("SHOWN", 550, "0 g");
    assert_eq!(extracted(&content), ["SHOWN"]);
}

#[test]
fn white_text_on_a_white_rectangle_is_dropped() {
    let content = "1 g 50 590 400 26 re f\n".to_string()
        + &text("HIDDEN", 598, "1 g")
        + &text("SHOWN", 550, "0 g");
    assert_eq!(extracted(&content), ["SHOWN"]);
}

#[test]
fn white_text_on_a_dark_rectangle_is_kept() {
    let content = "0 g 50 590 400 26 re f\n".to_string() + &text("SHOWN", 598, "1 g");
    assert_eq!(extracted(&content), ["SHOWN"]);
}

#[test]
fn white_text_over_a_rectangle_it_extends_past_is_kept() {
    let content = "0 g 50 590 100 26 re f\n".to_string() + &text("SHOWN", 598, "1 g");
    assert_eq!(extracted(&content), ["SHOWN"]);
}

#[test]
fn the_rectangle_directly_beneath_decides_not_an_earlier_one() {
    let content = "0 g 50 580 400 60 re f\n1 g 50 590 400 26 re f\n".to_string()
        + &text("HIDDEN", 598, "1 g");
    assert!(extracted(&content).is_empty());
    let content =
        "1 g 50 580 400 60 re f\n0 g 50 590 400 26 re f\n".to_string() + &text("SHOWN", 598, "1 g");
    assert_eq!(extracted(&content), ["SHOWN"]);
}

#[test]
fn translucent_or_stroked_backdrops_and_stroked_text_are_kept() {
    let translucent =
        "/Half gs 1 0 0 rg 50 590 400 26 re f\n".to_string() + &text("SHOWN", 598, "1 0 0 rg");
    assert_eq!(extracted(&translucent), ["SHOWN"]);
    let stroked =
        "1 0 0 rg 0 0 1 RG 50 590 400 26 re B\n".to_string() + &text("SHOWN", 598, "1 0 0 rg");
    assert_eq!(extracted(&stroked), ["SHOWN"]);
    let outlined = text("SHOWN", 700, "1 g 0 G 1 Tr");
    assert_eq!(extracted(&outlined), ["SHOWN"]);
}

#[test]
fn white_text_over_earlier_text_is_kept() {
    let content = text("UNDER", 700, "0 g") + &text("OVER", 700, "1 g");
    assert!(extracted(&content).iter().any(|t| t == "OVER"));
}

#[test]
fn text_filled_with_a_pattern_is_kept() {
    let content = text("SHOWN", 700, "/Pattern cs /Grad scn");
    assert_eq!(extracted(&content), ["SHOWN"]);
}

#[test]
fn white_text_over_a_gradient_rectangle_is_kept() {
    let content =
        "/Pattern cs /Grad scn 40 600 300 100 re f\n".to_string() + &text("SHOWN", 650, "1 g");
    assert_eq!(extracted(&content), ["SHOWN"]);
}
