use super::*;

#[test]
fn writes_a_parseable_two_page_pdf() {
    let dir = std::env::temp_dir().join(format!("pptx-read-pdf-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let img = dir.join("p.png");
    let mut rgb = image::RgbImage::new(4, 3);
    rgb.put_pixel(1, 1, image::Rgb([255, 0, 0]));
    rgb.save(&img).unwrap();
    let out = dir.join("out.pdf");
    let page = |w| PdfPage {
        width_pt: w,
        height_pt: 540.0,
        image_path: img.to_string_lossy().into_owned(),
    };
    write_pdf(out.to_str().unwrap(), &[page(960.0), page(720.0)]).unwrap();
    let bytes = std::fs::read(&out).unwrap();
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.starts_with("%PDF-1.5"));
    assert!(text.contains("/Count 2"));
    assert!(text.contains("/MediaBox [0 0 960.000 540.000]"));
    // xref offsets point at object headers.
    let xref = text.find("xref\n").unwrap();
    let first_entry = &text[xref..].lines().nth(3).unwrap()[..10];
    let off: usize = first_entry.parse().unwrap();
    // Offsets count raw bytes (the binary header comment is not UTF-8).
    assert!(bytes[off..].starts_with(b"1 0 obj"));
    std::fs::remove_dir_all(&dir).ok();
}
