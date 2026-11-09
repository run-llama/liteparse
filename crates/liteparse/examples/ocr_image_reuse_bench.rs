//! Compare OCR raster preparation from PDFium and existing PNG/JPEG images.
//! Run with --release; this measures raster preparation, not OCR or total parse time.
use std::collections::HashMap;
use std::hint::black_box;
use std::time::Instant;

use image::codecs::jpeg::JpegEncoder;
use liteparse::config::OcrPageImage;
use liteparse::stages::{self, Library, OcrRenderOptions};
use liteparse::types::PdfInput;

fn scanned_pdf(jpeg: &[u8], width: u32, height: u32, pw: f32, ph: f32) -> Vec<u8> {
    let content = format!("q {pw} 0 0 {ph} 0 0 cm /Im0 Do Q");
    let mut image = format!("<< /Type /XObject /Subtype /Image /Width {width} /Height {height} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode /Length {} >>\nstream\n", jpeg.len()).into_bytes();
    image.extend_from_slice(jpeg);
    image.extend_from_slice(b"\nendstream");
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {pw} {ph}] /Resources << /XObject << /Im0 5 0 R >> >> /Contents 4 0 R >>").into_bytes(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()).into_bytes(),
        image,
    ];
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, object) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        bytes.extend_from_slice(object);
        bytes.extend_from_slice(b"\nendobj\n");
    }
    let xref = bytes.len();
    bytes.extend_from_slice(b"xref\n0 6\n0000000000 65535 f \n");
    for offset in offsets {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!("trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF").as_bytes(),
    );
    bytes
}

fn measure(input: &PdfInput, images: &[(String, OcrPageImage)], label: &str) {
    let lib = Library::init();
    let document = stages::open(&lib, input, None, &[]).unwrap();
    let pages = stages::extract(
        &document,
        &stages::ExtractRequest {
            target_pages: Some(&[1]),
            ..Default::default()
        },
    )
    .unwrap()
    .pages;
    let mut cases = vec![(
        "render".to_string(),
        OcrRenderOptions {
            dpi: 200.0,
            selection: Some([1].into_iter().collect()),
            ..Default::default()
        },
    )];
    for (name, image) in images {
        cases.push((
            name.clone(),
            OcrRenderOptions {
                dpi: 200.0,
                selection: Some([1].into_iter().collect()),
                page_images: HashMap::from([(1, image.clone())]),
                ..Default::default()
            },
        ));
    }
    let mut samples = vec![Vec::new(); cases.len()];
    for round in 0..24 {
        for offset in 0..cases.len() {
            let index = (round + offset) % cases.len();
            let start = Instant::now();
            let raster = stages::render_for_ocr(&document, &pages, 0, &cases[index].1).unwrap();
            let elapsed = start.elapsed().as_secs_f64() * 1000.0;
            black_box(&raster);
            if round >= 4 {
                samples[index].push(elapsed);
            }
        }
    }
    for ((name, _), values) in cases.iter().zip(&mut samples) {
        values.sort_by(f64::total_cmp);
        println!(
            "{label}\t{name}\tmedian_ms={:.3}\tmin_ms={:.3}\tn={}",
            values[values.len() / 2],
            values[0],
            values.len()
        );
    }
}

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "integration_tests_data/sample.pdf".into());
    let input = PdfInput::Path(path);
    let dir = tempfile::tempdir().unwrap();
    let (rgb, rgba, width, height, pw, ph) = {
        let lib = Library::init();
        let document = stages::open(&lib, &input, None, &[]).unwrap();
        let page = document.page(0).unwrap();
        let extracted = stages::extract(
            &document,
            &stages::ExtractRequest {
                target_pages: Some(&[1]),
                ..Default::default()
            },
        )
        .unwrap();
        let bitmap = page.render(200.0).unwrap();
        (
            bitmap.to_rgb(),
            bitmap.to_rgba(),
            bitmap.width() as u32,
            bitmap.height() as u32,
            extracted.pages[0].page_width,
            extracted.pages[0].page_height,
        )
    };
    let mut jpeg = Vec::new();
    JpegEncoder::new_with_quality(&mut jpeg, 90)
        .encode(&rgb, width, height, image::ExtendedColorType::Rgb8)
        .unwrap();
    let jpg = dir.path().join("page.jpg");
    let png = dir.path().join("page-rgb.png");
    let alpha = dir.path().join("page-rgba.png");
    std::fs::write(&jpg, &jpeg).unwrap();
    image::save_buffer(&png, &rgb, width, height, image::ColorType::Rgb8).unwrap();
    image::save_buffer(&alpha, &rgba, width, height, image::ColorType::Rgba8).unwrap();
    println!(
        "pixels={width}x{height}; dpi=200; warmups=4; samples=20; OCR and extraction excluded"
    );
    let images = [
        ("jpeg".into(), jpg),
        ("png-rgb".into(), png),
        ("png-rgba".into(), alpha),
    ]
    .map(|(name, path)| {
        (
            name,
            OcrPageImage {
                path: path.to_string_lossy().into_owned(),
                dpi: 200.0,
            },
        )
    });
    measure(&input, &images, "native-forced-ocr");
    measure(
        &PdfInput::Bytes(scanned_pdf(&jpeg, width, height, pw, ph)),
        &images,
        "scanned",
    );
}
