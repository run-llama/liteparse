//! Real Apple Vision integration tests. These run on the macOS CI runners,
//! including in builds without Tesseract; no mock OCR or external server.
#![cfg(target_os = "macos")]

use liteparse::config::{MacOcrRecognition, OcrBackend, OutputFormat};
use liteparse::ocr::{OcrEngine, OcrOptions, OcrResult, macos_vision::MacOcrEngine};
use liteparse::types::PdfInput;
use liteparse::{LiteParse, LiteParseConfig};
use lopdf::{Document, Object, Stream, dictionary};

const FIXTURE: &[u8] = include_bytes!("../../../integration_tests_data/macos_ocr.png");
const WIDTH: u32 = 1200;
const HEIGHT: u32 = 720;
const DPI: f32 = 150.0;

fn config() -> LiteParseConfig {
    LiteParseConfig {
        ocr_enabled: true,
        ocr_backend: OcrBackend::MacosVision,
        mac_ocr_languages: vec!["en-US".into()],
        emit_word_boxes: true,
        output_format: OutputFormat::Text,
        num_workers: 2,
        quiet: true,
        dpi: DPI,
        ..Default::default()
    }
}

fn word<'a>(results: &'a [OcrResult], text: &str) -> &'a OcrResult {
    results
        .iter()
        .find(|result| result.text == text)
        .unwrap_or_else(|| panic!("missing word {text:?} in {results:?}"))
}

#[tokio::test]
async fn vision_returns_words_in_raster_coordinates_in_both_modes() {
    let image = image::load_from_memory(FIXTURE).unwrap().to_rgb8();
    for recognition in [MacOcrRecognition::Accurate, MacOcrRecognition::Fast] {
        let engine = MacOcrEngine::new(vec!["en-US".into()], recognition).unwrap();
        let results = engine
            .recognize(
                image.as_raw(),
                WIDTH,
                HEIGHT,
                &OcrOptions {
                    language: "eng".into(),
                    dpi: DPI,
                },
            )
            .await
            .unwrap();
        for result in &results {
            assert!((0.0..=1.0).contains(&result.confidence), "{result:?}");
            let [left, top, right, bottom] = result.bbox;
            assert!(
                left >= 0.0 && top >= 0.0 && right <= WIDTH as f32 && bottom <= HEIGHT as f32,
                "{result:?}"
            );
            assert!(left < right && top < bottom, "{result:?}");
            assert_eq!(
                result.text.split_whitespace().count(),
                1,
                "line returned as a word: {result:?}"
            );
        }
        // Independent word boxes are required by overlap suppression and
        // emit_word_boxes; a line-sized box for each word would fail here.
        let native = word(&results, "Native");
        let vision = word(&results, "Vision");
        let invoice = word(&results, "Invoice");
        word(&results, "OCR");
        word(&results, "12345");
        word(&results, "Total:");
        word(&results, "$1,234.56");
        assert!(native.bbox[2] <= vision.bbox[0] + 10.0);
        assert!((60.0..120.0).contains(&native.bbox[1]), "{native:?}");
        assert!((270.0..340.0).contains(&invoice.bbox[1]), "{invoice:?}");
        assert!((60.0..110.0).contains(&invoice.bbox[0]), "{invoice:?}");
    }
}

#[tokio::test]
async fn vision_handles_grayscale_blank_images_and_invalid_buffers() {
    let engine = MacOcrEngine::new(vec!["en-US".into()], MacOcrRecognition::Accurate).unwrap();
    let options = OcrOptions {
        language: "eng".into(),
        dpi: DPI,
    };
    let gray = image::load_from_memory(FIXTURE).unwrap().to_luma8();
    let results = engine
        .recognize(gray.as_raw(), WIDTH, HEIGHT, &options)
        .await
        .unwrap();
    word(&results, "Invoice");
    assert!(
        engine
            .recognize(&vec![255; 200 * 100], 200, 100, &options)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(engine.recognize(&[], 0, 0, &options).await.is_err());
    assert!(engine.recognize(&[0; 5], 2, 2, &options).await.is_err());
}

#[test]
fn language_support_is_queried_for_the_running_os() {
    for recognition in [MacOcrRecognition::Accurate, MacOcrRecognition::Fast] {
        let supported = MacOcrEngine::supported_languages(recognition).unwrap();
        assert!(supported.iter().any(|language| language == "en-US"));
        let error = MacOcrEngine::new(vec!["xx-INVALID".into()], recognition)
            .err()
            .unwrap()
            .to_string();
        assert!(
            error.contains("xx-INVALID") && error.contains("supported languages"),
            "{error}"
        );
    }
}

#[tokio::test]
async fn scanned_pdf_recovers_text_word_boxes_and_point_coordinates() {
    let parser = LiteParse::new(config());
    let result = parser
        .parse_input(PdfInput::Bytes(raster_pdf(false, 2)))
        .await
        .unwrap();
    assert_eq!(result.pages.len(), 2);
    for page in &result.pages {
        assert!(page.text.contains("Native Vision OCR"), "{}", page.text);
        assert!(page.text.contains("Invoice 12345"), "{}", page.text);
        let words: Vec<_> = page
            .text_items
            .iter()
            .flat_map(|item| &item.words)
            .collect();
        let invoice = words.iter().find(|word| word.text == "Invoice").unwrap();
        let scale = 72.0 / DPI;
        assert!((invoice.x - 80.0 * scale).abs() < 10.0, "{invoice:?}");
        assert!((invoice.y - 290.0 * scale).abs() < 15.0, "{invoice:?}");
        assert!(words.iter().any(|word| word.text == "12345"));
        assert!(page.text_items.iter().all(|item| item.confidence.is_some()));
    }
}

#[tokio::test]
async fn mixed_pdf_keeps_native_text_and_suppresses_ocr_duplicates() {
    let parser = LiteParse::new(config());
    let result = parser
        .parse_input(PdfInput::Bytes(raster_pdf(true, 1)))
        .await
        .unwrap();
    let page = &result.pages[0];
    assert_eq!(page.text.matches("Native").count(), 1, "{}", page.text);
    assert!(page.text.contains("Invoice 12345"), "{}", page.text);
    assert!(
        page.text_items
            .iter()
            .any(|item| item.confidence.is_none() && item.text.contains("Native"))
    );
    assert!(
        page.text_items
            .iter()
            .any(|item| item.confidence.is_some() && item.text.contains("Invoice"))
    );
    assert!(
        !page
            .text_items
            .iter()
            .any(|item| item.confidence.is_some() && item.text.contains("Native"))
    );
}

#[tokio::test]
async fn image_conversion_and_batch_sessions_use_the_selected_backend() {
    // Empty language preferences exercise the public default: automatic
    // language detection on macOS 13+, Vision defaults on older systems.
    let parser = LiteParse::new(LiteParseConfig {
        mac_ocr_languages: Vec::new(),
        ..config()
    });
    let fixture = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../integration_tests_data/macos_ocr.png"
    );
    let result = parser.parse(fixture).await.unwrap();
    assert!(result.text.contains("Invoice 12345"));

    let mut session = parser
        .open_batch_session(PdfInput::Bytes(raster_pdf(false, 2)), 1)
        .await
        .unwrap();
    for _ in 0..2 {
        let batch = session.next_batch().await.unwrap().unwrap();
        assert!(batch.result.text.contains("Invoice 12345"));
    }
    assert!(session.next_batch().await.unwrap().is_none());
}

#[test]
fn rust_cli_selects_vision() {
    let fixture = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../integration_tests_data/macos_ocr.png"
    );
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_lit"))
        .args([
            "parse",
            fixture,
            "--ocr-backend",
            "macos-vision",
            "--mac-ocr-language",
            "en-US",
            "--mac-ocr-recognition",
            "fast",
            "--format",
            "text",
            "--quiet",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(String::from_utf8_lossy(&result.stdout).contains("Invoice 12345"));
}

/// A raster-only PDF, optionally replacing the image's header with native
/// Helvetica text at the same position. Generated in memory to exercise
/// PDFium extraction, selective rendering, Vision, merging and projection.
fn raster_pdf(native_header: bool, page_count: usize) -> Vec<u8> {
    let image = image::load_from_memory(FIXTURE).unwrap().to_rgb8();
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let image_id = doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Image", "Width" => WIDTH,
            "Height" => HEIGHT, "ColorSpace" => "DeviceRGB", "BitsPerComponent" => 8,
        },
        image.into_raw(),
    ));
    let font_id = doc.add_object(
        dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" },
    );
    let scale = 72.0 / DPI;
    let width = WIDTH as f32 * scale;
    let height = HEIGHT as f32 * scale;
    let mut kids = Vec::new();
    for _ in 0..page_count {
        let mut content = format!("q {width} 0 0 {height} 0 0 cm /Image Do Q\n");
        if native_header {
            content += &format!(
                "q 1 1 1 rg 0 {} {width} {} re f Q\nBT /F1 {} Tf {} {} Td (Native Vision OCR) Tj ET\n",
                height - 180.0 * scale,
                160.0 * scale,
                64.0 * scale,
                80.0 * scale,
                height - 120.0 * scale,
            );
        }
        let content_id = doc.add_object(Stream::new(dictionary! {}, content.into_bytes()));
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page", "Parent" => pages_id,
            "MediaBox" => vec![Object::from(0), Object::from(0), Object::from(width), Object::from(height)],
            "Resources" => dictionary! { "XObject" => dictionary! { "Image" => image_id }, "Font" => dictionary! { "F1" => font_id } },
            "Contents" => content_id,
        });
        kids.push(Object::Reference(page_id));
    }
    doc.objects.insert(
        pages_id,
        dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => page_count as i64 }.into(),
    );
    let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog_id);
    doc.compress();
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).unwrap();
    bytes
}
