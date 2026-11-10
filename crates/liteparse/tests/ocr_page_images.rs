//! Caller images must bypass rendering without changing OCR page coordinates.
use std::collections::HashMap;
use std::sync::Arc;

use liteparse::config::{CropBox, OcrPageImage, PageOrientationCorrection};
use liteparse::ocr::{OcrEngine, OcrImage, OcrOptions, OcrResult};
use liteparse::stages::{self, Library};
use liteparse::types::PdfInput;
use liteparse::{LiteParse, LiteParseConfig};

fn blank_pdf() -> PdfInput {
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R 4 0 R 5 0 R] /Count 3 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 72 144] >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 72 144] >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 72 144] >>",
    ];
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", index + 1).as_bytes());
    }
    let xref = bytes.len();
    bytes.extend_from_slice(b"xref\n0 6\n0000000000 65535 f \n");
    for offset in offsets {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!("trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF").as_bytes(),
    );
    PdfInput::Bytes(bytes)
}

fn image(
    dir: &std::path::Path,
    name: &str,
    width: u32,
    height: u32,
    rgb: [u8; 3],
    dpi: f32,
) -> OcrPageImage {
    let path = dir.join(name);
    image::RgbImage::from_pixel(width, height, image::Rgb(rgb))
        .save(&path)
        .unwrap();
    OcrPageImage {
        path: path.to_string_lossy().into_owned(),
        dpi,
    }
}

struct PixelOcr {
    grayscale: bool,
}

impl OcrEngine for PixelOcr {
    fn name(&self) -> &str {
        "pixel-test"
    }
    fn prefers_grayscale(&self) -> bool {
        self.grayscale
    }
    fn recognize<'a, 'b: 'a, 'c: 'a>(
        &'a self,
        pixels: &'c [u8],
        width: u32,
        height: u32,
        options: &'b OcrOptions,
    ) -> std::pin::Pin<
        Box<
            dyn Future<Output = Result<Vec<OcrResult>, Box<dyn std::error::Error + Send + Sync>>>
                + Send
                + '_,
        >,
    > {
        Box::pin(async move {
            assert_eq!(
                pixels.len(),
                width as usize * height as usize * if self.grayscale { 1 } else { 3 }
            );
            let text = match pixels[0] {
                37 => "ONE",
                211 => "THREE",
                other => panic!("unexpected image pixel: {other}"),
            };
            let scale = options.dpi / 72.0;
            Ok(vec![
                OcrResult {
                    text: text.into(),
                    bbox: [40.0, 20.0, 60.0, 30.0].map(|v| v * scale),
                    confidence: 0.93,
                    polygon: None,
                },
                OcrResult {
                    text: "DROP".into(),
                    bbox: [0.0, 0.0, 5.0 * scale, 5.0 * scale],
                    confidence: 0.9,
                    polygon: None,
                },
            ])
        })
    }
}

fn config(images: HashMap<u32, OcrPageImage>) -> LiteParseConfig {
    LiteParseConfig {
        ocr_enabled: true,
        quiet: true,
        dpi: 72.0,
        emit_word_boxes: true,
        ocr_page_images: images,
        ..Default::default()
    }
}

#[tokio::test]
async fn legacy_rgb_and_grayscale_engines_receive_selected_images_in_page_coordinates() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = config(HashMap::from([
        (
            1,
            image(dir.path(), "one.png", 288, 144, [37, 37, 37], 144.0),
        ),
        (
            3,
            image(dir.path(), "three.png", 72, 144, [211, 211, 211], 72.0),
        ),
    ]));
    cfg.target_pages = Some("3,1".into());
    cfg.page_orientation_corrections = vec![PageOrientationCorrection { page: 1, angle: 90 }];
    cfg.crop_box = Some(CropBox {
        left: 0.1,
        top: 0.0,
        right: 0.0,
        bottom: 0.0,
    });
    // Zero DPI makes any discarded PDF render fail. These engines implement
    // only the existing raw-pixel method, including its grayscale preference.
    cfg.dpi = 0.0;
    for grayscale in [false, true] {
        let result = LiteParse::new(cfg.clone())
            .with_ocr_engine(Arc::new(PixelOcr { grayscale }))
            .parse_input(blank_pdf())
            .await
            .unwrap();
        assert_eq!(
            result
                .pages
                .iter()
                .map(|p| p.page_number)
                .collect::<Vec<_>>(),
            vec![1, 3]
        );
        assert_eq!(
            (result.pages[0].page_width, result.pages[0].page_height),
            (144.0, 72.0)
        );
        for (page, text) in result.pages.iter().zip(["ONE", "THREE"]) {
            let item = page
                .text_items
                .iter()
                .find(|item| item.text == text)
                .unwrap();
            assert_eq!((item.x, item.y, item.width), (40.0, 20.0, 20.0));
            assert_eq!(item.words.len(), 1);
            assert_eq!(item.confidence, Some(0.93));
            assert!(!page.text.contains("DROP"));
        }
    }
}

#[test]
fn supplied_images_reject_invalid_dpi_geometry_and_zero_area() {
    let dir = tempfile::tempdir().unwrap();
    let source = image(dir.path(), "page.png", 72, 144, [37, 37, 37], 72.0);
    let lib = Library::init();
    let document = stages::open(&lib, &blank_pdf(), None, &[]).unwrap();
    let pages = stages::extract(&document, &stages::ExtractRequest::default())
        .unwrap()
        .pages;
    let mut options = stages::OcrRenderOptions {
        page_images: HashMap::from([(1, source)]),
        max_rasters: 1,
        ..Default::default()
    };
    for dpi in [0.0, f32::NAN, 144.0] {
        options.page_images.get_mut(&1).unwrap().dpi = dpi;
        assert!(
            stages::render_for_ocr(&document, &pages, 0, &options).is_err(),
            "dpi {dpi}"
        );
    }
    // PNM permits zero-width images. This one also fits the one-pixel
    // rounding tolerance at low DPI, but OCR still requires actual pixels.
    let empty = dir.path().join("empty.ppm");
    std::fs::write(&empty, b"P6\n0 1\n255\n").unwrap();
    options.page_images.insert(
        1,
        OcrPageImage {
            path: empty.to_string_lossy().into_owned(),
            dpi: 0.5,
        },
    );
    assert!(stages::render_for_ocr(&document, &pages, 0, &options).is_err());
}

#[test]
fn encoded_images_stay_decoded_when_native_text_must_be_masked() {
    let lib = Library::init();
    let document = stages::open(
        &lib,
        &PdfInput::Path("../../integration_tests_data/sample.pdf".into()),
        None,
        &[],
    )
    .unwrap();
    let pages = stages::extract(
        &document,
        &stages::ExtractRequest {
            target_pages: Some(&[1]),
            ..Default::default()
        },
    )
    .unwrap()
    .pages;
    let mut options = stages::OcrRenderOptions {
        dpi: 72.0,
        selection: Some([1].into_iter().collect()),
        ..Default::default()
    };
    let baseline = stages::render_for_ocr(&document, &pages, 0, &options)
        .unwrap()
        .0
        .remove(0);
    assert!(baseline.has_native_text);
    let bitmap = document.page(0).unwrap().render(72.0).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("native.png");
    image::save_buffer(
        &path,
        &bitmap.to_rgb(),
        bitmap.width() as u32,
        bitmap.height() as u32,
        image::ColorType::Rgb8,
    )
    .unwrap();
    options.page_images.insert(
        1,
        OcrPageImage {
            path: path.to_string_lossy().into_owned(),
            dpi: 72.0,
        },
    );
    let reused = stages::render_for_ocr(&document, &pages, 0, &options)
        .unwrap()
        .0
        .remove(0);
    assert!(matches!(reused.image, OcrImage::Pixels { .. }));
    assert_eq!(
        reused.image.pixels(false).unwrap(),
        baseline.image.pixels(false).unwrap()
    );
}

#[test]
fn encoded_images_with_exif_rotation_use_pixels_to_preserve_page_coordinates() {
    let dir = tempfile::tempdir().unwrap();
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 90)
        .encode_image(&image::RgbImage::from_pixel(
            72,
            144,
            image::Rgb([0, 0, 255]),
        ))
        .unwrap();
    // JPEG APP1: Exif, little-endian TIFF, one SHORT orientation entry = 6
    // (rotate 90 degrees). Pixel coordinates themselves remain 72×144.
    let exif = b"Exif\0\0II\x2a\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0";
    let mut segment = vec![0xff, 0xe1];
    segment.extend_from_slice(&((exif.len() + 2) as u16).to_be_bytes());
    segment.extend_from_slice(exif);
    jpeg.splice(2..2, segment);
    let path = dir.path().join("oriented.jpg");
    std::fs::write(&path, &jpeg).unwrap();
    let image = OcrImage::from_file(path.to_str().unwrap()).unwrap();
    assert!(matches!(image, OcrImage::Pixels { .. }));
    assert_eq!(image.dimensions(), (72, 144));
    assert_eq!(
        image.pixels(false).unwrap().as_ref(),
        image::load_from_memory(&jpeg).unwrap().into_rgb8().as_raw()
    );
}
