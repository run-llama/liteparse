//! Caller images must bypass rendering without changing OCR page coordinates.
use std::collections::HashMap;
use std::sync::Arc;

use liteparse::config::{CropBox, OcrPageImage, PageOrientationCorrection};
use liteparse::ocr::{OcrEngine, OcrOptions, OcrResult};
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
    rgba: [u8; 4],
    dpi: f32,
) -> OcrPageImage {
    let path = dir.join(name);
    image::RgbaImage::from_pixel(width, height, image::Rgba(rgba))
        .save(&path)
        .unwrap();
    OcrPageImage {
        path: path.to_string_lossy().into_owned(),
        dpi,
    }
}

struct PixelOcr;

impl OcrEngine for PixelOcr {
    fn name(&self) -> &str {
        "pixel-test"
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
            assert_eq!(pixels.len(), width as usize * height as usize * 3);
            let text = match &pixels[..3] {
                [255, 0, 0] => "RED",
                [0, 0, 255] => "BLUE",
                [255, 255, 255] => "WHITE",
                other => panic!("unexpected image pixel: {other:?}"),
            };
            let scale = options.dpi / 72.0;
            Ok(vec![OcrResult {
                text: text.into(),
                bbox: [40.0, 20.0, 60.0, 30.0].map(|v| v * scale),
                confidence: 0.93,
                polygon: None,
            }])
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
async fn selected_images_bypass_invalid_render_dpi_and_preserve_word_coordinates() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = config(HashMap::from([
        (
            1,
            image(dir.path(), "one.png", 144, 288, [255, 0, 0, 255], 144.0),
        ),
        (
            3,
            image(dir.path(), "three.png", 72, 144, [0, 0, 255, 255], 72.0),
        ),
    ]));
    cfg.target_pages = Some("3,1".into());
    // A render at zero DPI fails. Successful OCR proves the renderer was
    // skipped, even when selected pages are not a leading document slice.
    cfg.dpi = 0.0;
    let parser = LiteParse::new(cfg.clone()).with_ocr_engine(Arc::new(PixelOcr));
    let result = parser.parse_input(blank_pdf()).await.unwrap();
    assert_eq!(
        result
            .pages
            .iter()
            .map(|p| p.page_number)
            .collect::<Vec<_>>(),
        vec![1, 3]
    );
    for (page, text) in result.pages.iter().zip(["RED", "BLUE"]) {
        let item = page
            .text_items
            .iter()
            .find(|item| item.text == text)
            .unwrap();
        assert!((item.x - 40.0).abs() < 0.01);
        assert!((item.y - 20.0).abs() < 0.01);
        assert!((item.width - 20.0).abs() < 0.01);
        assert_eq!(item.words.len(), 1);
        assert_eq!(item.confidence, Some(0.93));
    }
    cfg.ocr_page_images.clear();
    assert!(
        LiteParse::new(cfg)
            .with_ocr_engine(Arc::new(PixelOcr))
            .parse_input(blank_pdf())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn batches_use_absolute_page_keys_and_render_only_missing_images() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = config(HashMap::from([
        (
            1,
            image(dir.path(), "one.png", 144, 288, [255, 0, 0, 255], 144.0),
        ),
        (
            3,
            image(dir.path(), "three.png", 72, 144, [0, 0, 255, 255], 72.0),
        ),
    ]));
    cfg.extract_screenshots = true;
    cfg.crop_box = Some(CropBox {
        left: 0.5,
        top: 0.0,
        right: 0.0,
        bottom: 0.0,
    });
    let parser = LiteParse::new(cfg).with_ocr_engine(Arc::new(PixelOcr));
    let mut session = parser.open_batch_session(blank_pdf(), 1).await.unwrap();
    for expected in ["RED", "WHITE", "BLUE"] {
        let result = session.next_batch().await.unwrap().unwrap().result;
        assert!(result.text.contains(expected), "{}", result.text);
        assert!((result.pages[0].text_items[0].x - 40.0).abs() < 0.01);
        // The requested screenshot is rendered at cfg.dpi, not copied from
        // the higher-resolution caller image. Crop filtering follows OCR.
        assert_eq!(
            (result.screenshots[0].width, result.screenshots[0].height),
            (72, 144)
        );
    }
    assert!(session.next_batch().await.unwrap().is_none());
}

#[tokio::test]
async fn caller_images_match_corrected_orientation_and_crop_filters_follow_ocr() {
    let dir = tempfile::tempdir().unwrap();
    let source = image(dir.path(), "upright.png", 288, 144, [255, 0, 0, 255], 144.0);
    let mut cfg = config(HashMap::from([(1, source)]));
    cfg.target_pages = Some("1".into());
    cfg.page_orientation_corrections = vec![PageOrientationCorrection { page: 1, angle: 90 }];
    cfg.crop_box = Some(CropBox {
        left: 0.5,
        top: 0.0,
        right: 0.0,
        bottom: 0.0,
    });
    let result = LiteParse::new(cfg.clone())
        .with_ocr_engine(Arc::new(PixelOcr))
        .parse_input(blank_pdf())
        .await
        .unwrap();
    assert_eq!(
        (result.pages[0].page_width, result.pages[0].page_height),
        (144.0, 72.0)
    );
    assert!(result.pages[0].text_items.is_empty());
    cfg.page_orientation_corrections.clear();
    let error = LiteParse::new(cfg)
        .with_ocr_engine(Arc::new(PixelOcr))
        .parse_input(blank_pdf())
        .await
        .err()
        .unwrap();
    assert!(error.to_string().contains("full-page image"), "{error}");
}

#[test]
fn image_validation_rejects_bad_scale_and_unreadable_sources_without_rendering() {
    let dir = tempfile::tempdir().unwrap();
    let source = image(dir.path(), "page.png", 72, 144, [0, 0, 0, 0], 72.0);
    let lib = Library::init();
    let document = stages::open(&lib, &blank_pdf(), None, &[]).unwrap();
    let pages = stages::extract(&document, &stages::ExtractRequest::default())
        .unwrap()
        .pages;
    let mut options = stages::OcrRenderOptions {
        grayscale: true,
        page_images: HashMap::from([(1, source.clone())]),
        max_rasters: 1,
        ..Default::default()
    };
    let (rasters, next) = stages::render_for_ocr(&document, &pages, 0, &options).unwrap();
    assert_eq!(next, 1);
    assert_eq!(rasters[0].pixels, vec![255; 72 * 144]);
    // The PNM decoder accepts a zero-width raster. At low DPI this also
    // fits the one-pixel rounding tolerance, but it cannot be sent to OCR.
    let empty = dir.path().join("empty.ppm");
    std::fs::write(&empty, b"P6\n0 1\n255\n").unwrap();
    let decoded = image::open(&empty).unwrap();
    assert_eq!((decoded.width(), decoded.height()), (0, 1));
    options.page_images.insert(
        1,
        OcrPageImage {
            path: empty.to_string_lossy().into_owned(),
            dpi: 0.5,
        },
    );
    assert!(stages::render_for_ocr(&document, &pages, 0, &options).is_err());
    options.page_images.insert(1, source);
    for dpi in [0.0, -1.0, f32::NAN, f32::INFINITY, 144.0] {
        options.page_images.get_mut(&1).unwrap().dpi = dpi;
        assert!(
            stages::render_for_ocr(&document, &pages, 0, &options).is_err(),
            "dpi {dpi}"
        );
    }
    for path in [dir.path().join("absent.png"), dir.path().join("broken.png")] {
        std::fs::write(dir.path().join("broken.png"), b"not an image").unwrap();
        options.page_images.insert(
            1,
            OcrPageImage {
                path: path.to_string_lossy().into_owned(),
                dpi: 72.0,
            },
        );
        assert!(stages::render_for_ocr(&document, &pages, 0, &options).is_err());
    }
}
