//! LI-8306 (test-only): Form XObject /BBox white-render regression fixtures.
//!
//! Renders a synthetic PDF whose Form XObject /BBox contains an out-of-range
//! integer (2^32). Unpatched PDFium collapses the coordinate to 0, producing
//! an empty clip and a blank page; the provenance-preserving build recovers
//! the saturated coordinate and renders visibly. Zero and valid controls pin
//! the unchanged legacy behavior.
//!
//! Selecting the candidate library is done entirely outside this test via the
//! existing `PDFIUM_LIB_PATH` / `PDFIUM_INCLUDE_PATH` build-time override in
//! pdfium-sys; no production code or lock behavior changes.

use liteparse_pdfium::Library;

const PAGE_WIDTH: u32 = 200;
const PAGE_HEIGHT: u32 = 100;
const RENDER_DPI: f32 = 144.0;
const OVERFLOW_COORD: &str = "4294967296";

fn build_pdf(objects: &[(u32, String)]) -> Vec<u8> {
    let mut doc: Vec<u8> = b"%PDF-1.7\n%\xff\xff\xff\xff\n".to_vec();
    let mut offsets = Vec::new();
    for (num, body) in objects {
        offsets.push((*num, doc.len()));
        doc.extend_from_slice(format!("{num} 0 obj\n{body}\nendobj\n").as_bytes());
    }
    let xref = doc.len();
    let size = objects.iter().map(|(n, _)| n).max().unwrap() + 1;
    doc.extend_from_slice(format!("xref\n0 {size}\n0000000000 65535 f \n").as_bytes());
    for (num, offset) in &offsets {
        let _ = num;
        doc.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    doc.extend_from_slice(
        format!("trailer\n<< /Size {size} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    doc
}

fn form_pdf(bbox: &str) -> Vec<u8> {
    let form = "q\n0 0 1 rg\n0 0 200 100 re\nf\nQ\nBT\n/F1 14 Tf\n20 50 Td\n(LI-8306) Tj\nET";
    let page = "q /Fm1 Do Q";
    build_pdf(&[
        (1, "<< /Type /Catalog /Pages 2 0 R >>".to_string()),
        (2, "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string()),
        (
            3,
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {PAGE_WIDTH} {PAGE_HEIGHT}] \
             /Resources << /XObject << /Fm1 5 0 R >> >> /Contents 4 0 R >>"
            ),
        ),
        (
            4,
            format!("<< /Length {} >>\nstream\n{page}\nendstream", page.len()),
        ),
        (
            5,
            format!(
                "<< /Type /XObject /Subtype /Form /FormType 1 /BBox [{bbox}] \
             /Resources << /Font << /F1 6 0 R >> >> /Length {} >>\nstream\n{form}\nendstream",
                form.len()
            ),
        ),
        (
            6,
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
        ),
    ])
}

fn non_white_ratio(pdf: &[u8]) -> f64 {
    let lib = Library::init();
    let doc = lib
        .load_document_from_bytes(pdf, None)
        .expect("synthetic fixture must parse");
    assert_eq!(doc.page_count(), 1);
    let page = doc.page(0).expect("page 0 must load");
    let bitmap = page.render(RENDER_DPI).expect("page must render");
    let rgb = bitmap.to_rgb();
    let pixels = (bitmap.width() as usize) * (bitmap.height() as usize);
    assert_eq!(rgb.len(), pixels * 3);
    let non_white = rgb
        .chunks_exact(3)
        .filter(|px| px[0] < 245 || px[1] < 245 || px[2] < 245)
        .count();
    non_white as f64 / pixels as f64
}

// Requires a pdfium-binaries release carrying the LI-8306 provenance fix
// (PDFIUM_RELEASE_TAG bump past chromium/7897). Until then the stock library
// collapses the out-of-range /BBox to an empty clip and this assertion fails
// by design; it was verified green against the patched candidate build.
#[test]
#[ignore = "needs pdfium-binaries release with the LI-8306 Form-/BBox fix"]
fn overflow_bbox_renders_visibly() {
    let pdf = form_pdf(&format!(
        "-{OVERFLOW_COORD} -{OVERFLOW_COORD} {OVERFLOW_COORD} {OVERFLOW_COORD}"
    ));
    let ratio = non_white_ratio(&pdf);
    assert!(
        ratio > 0.01,
        "out-of-range /BBox must not collapse to an empty clip (non-white ratio {ratio})"
    );
}

#[test]
fn zero_bbox_stays_blank() {
    let ratio = non_white_ratio(&form_pdf("0 0 0 0"));
    assert_eq!(ratio, 0.0, "genuine zero /BBox must remain an empty clip");
}

#[test]
fn valid_bbox_renders_visibly() {
    let ratio = non_white_ratio(&form_pdf("0 0 200 100"));
    assert!(
        ratio > 0.01,
        "valid /BBox control must render (ratio {ratio})"
    );
}

/// Evidence hook: report which libpdfium the process actually mapped, and
/// verify it when `LI8306_EXPECT_LIBPDFIUM_BLAKE3` is set. Linux-only: reads
/// the mapped path from /proc/self/maps. No-op elsewhere.
#[test]
#[cfg(target_os = "linux")]
fn loaded_library_identity_is_reported() {
    // Force the library to load before inspecting the map.
    let _ = non_white_ratio(&form_pdf("0 0 200 100"));

    let maps = std::fs::read_to_string("/proc/self/maps").expect("readable /proc/self/maps");
    let path = maps
        .lines()
        .filter_map(|line| line.split_whitespace().nth(5))
        .find(|p| p.contains("libpdfium.so"))
        .expect("libpdfium.so must be mapped after rendering")
        .to_string();
    let bytes = std::fs::read(&path).expect("mapped libpdfium.so must be readable");
    let hash = blake3::hash(&bytes).to_hex().to_string();
    println!("LI-8306 loaded libpdfium: {path} blake3={hash}");

    if let Ok(expected) = std::env::var("LI8306_EXPECT_LIBPDFIUM_BLAKE3") {
        assert_eq!(
            hash, expected,
            "mapped libpdfium.so does not match the staged candidate hash"
        );
    }
}
