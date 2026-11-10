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
    if rgba[3] == 255 {
        image::RgbImage::from_pixel(width, height, image::Rgb([rgba[0], rgba[1], rgba[2]]))
            .save(&path)
            .unwrap();
    } else {
        image::RgbaImage::from_pixel(width, height, image::Rgba(rgba))
            .save(&path)
            .unwrap();
    }
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
        use_encoded_images: true,
        page_images: HashMap::from([(1, source.clone())]),
        max_rasters: 1,
        ..Default::default()
    };
    let (rasters, next) = stages::render_for_ocr(&document, &pages, 0, &options).unwrap();
    assert_eq!(next, 1);
    assert_eq!(rasters[0].pixels, vec![255; 72 * 144]);
    options.grayscale = false;
    let (rasters, _) = stages::render_for_ocr(&document, &pages, 0, &options).unwrap();
    assert_eq!(rasters[0].pixels, vec![255; 72 * 144 * 3]);
    options.grayscale = true;
    options.use_encoded_images = false;
    let gray = dir.path().join("gray.png");
    image::GrayImage::from_pixel(72, 144, image::Luma([37]))
        .save(&gray)
        .unwrap();
    options.page_images.insert(
        1,
        OcrPageImage {
            path: gray.to_string_lossy().into_owned(),
            dpi: 72.0,
        },
    );
    let (rasters, _) = stages::render_for_ocr(&document, &pages, 0, &options).unwrap();
    assert_eq!(rasters[0].pixels, vec![37; 72 * 144]);
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

#[tokio::test]
async fn http_ocr_preserves_original_jpeg_and_png_bytes_mime_and_retry_payloads() {
    use liteparse::ocr::http_simple::{HttpOcrEngine, OcrRetryConfig};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let dir = tempfile::tempdir().unwrap();
    let png = image(dir.path(), "page.png", 72, 144, [255, 0, 0, 255], 72.0);
    let jpeg_path = dir.path().join("page-image.bin");
    let rgb = image::RgbImage::from_pixel(72, 144, image::Rgb([0, 0, 255]));
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 90)
        .encode_image(&rgb)
        .unwrap();
    std::fs::write(&jpeg_path, &jpeg).unwrap();
    // A tEXt chunk survives forwarding but would be lost in pixel re-encoding.
    let mut png_bytes = std::fs::read(&png.path).unwrap();
    let text = b"Comment\0encoded-byte-regression";
    let mut chunk = (text.len() as u32).to_be_bytes().to_vec();
    chunk.extend_from_slice(b"tEXt");
    chunk.extend_from_slice(text);
    chunk.extend_from_slice(&crc32fast::hash(&chunk[4..]).to_be_bytes());
    png_bytes.splice(png_bytes.len() - 12..png_bytes.len() - 12, chunk);
    std::fs::write(&png.path, &png_bytes).unwrap();
    let expected = [
        (jpeg, "image/jpeg", "image.jpg", "JPEG"),
        (
            std::fs::read(&png.path).unwrap(),
            "image/png",
            "image.png",
            "PNG",
        ),
    ];
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        for (bytes, mime, file, text) in expected {
            for attempt in 0..2 {
                let (mut stream, _) =
                    tokio::time::timeout(std::time::Duration::from_secs(5), listener.accept())
                        .await
                        .unwrap()
                        .unwrap();
                let mut request = Vec::new();
                let mut buffer = [0; 4096];
                let header_end = loop {
                    let n = stream.read(&mut buffer).await.unwrap();
                    assert!(n > 0);
                    request.extend_from_slice(&buffer[..n]);
                    if let Some(end) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                        break end + 4;
                    }
                };
                let headers = std::str::from_utf8(&request[..header_end]).unwrap();
                let length: usize = headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse().unwrap())
                    })
                    .unwrap();
                while request.len() < header_end + length {
                    let n = stream.read(&mut buffer).await.unwrap();
                    assert!(n > 0);
                    request.extend_from_slice(&buffer[..n]);
                }
                let body = &request[header_end..header_end + length];
                let data_start = body
                    .windows(4)
                    .position(|part| part == b"\r\n\r\n")
                    .unwrap()
                    + 4;
                let part_headers = std::str::from_utf8(&body[..data_start]).unwrap();
                assert!(part_headers.contains(mime), "{part_headers}");
                assert!(
                    part_headers.contains(&format!("filename=\"{file}\"")),
                    "{part_headers}"
                );
                assert_eq!(
                    &body[data_start..data_start + bytes.len()],
                    bytes.as_slice()
                );
                assert!(body[data_start + bytes.len()..].starts_with(b"\r\n--"));
                let (status, payload) = if attempt == 0 {
                    ("500 Internal Server Error", "retry".to_string())
                } else {
                    (
                        "200 OK",
                        format!(
                            r#"{{"results":[{{"text":"{text}","bbox":[40,20,60,30],"confidence":0.93}}]}}"#
                        ),
                    )
                };
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
                    payload.len()
                );
                stream.write_all(response.as_bytes()).await.unwrap();
            }
        }
    });
    let mut cfg = config(HashMap::from([
        (
            1,
            OcrPageImage {
                path: jpeg_path.to_string_lossy().into_owned(),
                dpi: 72.0,
            },
        ),
        (3, png),
    ]));
    cfg.target_pages = Some("1,3".into());
    cfg.num_workers = 1;
    cfg.dpi = 0.0;
    let engine = HttpOcrEngine::new(url).with_retry(OcrRetryConfig {
        max_attempts: 2,
        base_backoff_ms: 0,
        jitter_ms: 0,
        ..Default::default()
    });
    let result = LiteParse::new(cfg)
        .with_ocr_engine(Arc::new(engine))
        .parse_input(blank_pdf())
        .await
        .unwrap();
    server.await.unwrap();
    assert_eq!(
        result
            .pages
            .iter()
            .map(|page| page.page_number)
            .collect::<Vec<_>>(),
        vec![1, 3]
    );
    for (page, text) in result.pages.iter().zip(["JPEG", "PNG"]) {
        let item = page
            .text_items
            .iter()
            .find(|item| item.text == text)
            .unwrap();
        assert_eq!((item.x, item.y, item.width), (40.0, 20.0, 20.0));
        assert_eq!(item.confidence, Some(0.93));
    }
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
        use_encoded_images: true,
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
    assert!(reused.encoded_image.is_none());
    assert_eq!(reused.pixels, baseline.pixels);
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
    let lib = Library::init();
    let document = stages::open(&lib, &blank_pdf(), None, &[]).unwrap();
    let pages = stages::extract(&document, &stages::ExtractRequest::default())
        .unwrap()
        .pages;
    let options = stages::OcrRenderOptions {
        max_rasters: 1,
        use_encoded_images: true,
        page_images: HashMap::from([(
            1,
            OcrPageImage {
                path: path.to_string_lossy().into_owned(),
                dpi: 72.0,
            },
        )]),
        ..Default::default()
    };
    let raster = stages::render_for_ocr(&document, &pages, 0, &options)
        .unwrap()
        .0
        .remove(0);
    assert!(raster.encoded_image.is_none());
    assert_eq!((raster.width, raster.height), (72, 144));
    assert_eq!(
        raster.pixels,
        image::load_from_memory(&jpeg)
            .unwrap()
            .into_rgb8()
            .into_raw()
    );
}
