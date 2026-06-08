use std::sync::Arc;

use crate::error::LiteParseError;
use crate::ocr::{OcrEngine, OcrOptions, OcrResult};
use crate::types::{Page, TextItem};
use image::{ImageBuffer, Rgba};
use pdfium::Document;

/// Owned page bitmap prepared for OCR. Indices refer to positions in the `pages` slice.
pub(crate) struct RenderedPage {
    pub idx: usize,
    pub rgb_bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// Render pages that need OCR from an already-open document.
///
/// The pdfium `Document` holds raw pointers that are not `Send`, so callers must
/// drop it before awaiting the OCR engine.
pub(crate) fn render_pages_for_ocr(
    document: &Document,
    pages: &[Page],
    dpi: f32,
) -> Result<Vec<RenderedPage>, LiteParseError> {
    let mut rendered = Vec::new();
    for (idx, page) in pages.iter().enumerate() {
        // Count only non-garbled native text. Substitution-cipher-style corrupt
        // encodings (e.g. PDFs with a broken cmap) produce long "text" that looks
        // populated but is unreadable — without this, such pages bypass OCR
        // because text_length >= 20 and coverage looks fine.
        let text_length: usize = page
            .text_items
            .iter()
            .filter(|item| !is_likely_garbled(&item.text))
            .map(|item| item.text.len())
            .sum();
        let page_obj = document.page((page.page_number - 1) as i32)?;
        let has_images = !page_obj.image_bounds(25.0, 0.9).is_empty();

        let page_area = page.page_width * page.page_height;
        let text_bbox_area: f32 = page
            .text_items
            .iter()
            .filter(|item| !is_likely_garbled(&item.text))
            .map(|item| item.width * item.height)
            .sum();
        let text_coverage = if page_area > 0.0 {
            text_bbox_area / page_area
        } else {
            0.0
        };

        let needs_ocr =
            text_length < 20 || text_coverage < 0.15 || has_images || page_is_garbled(page);
        if !needs_ocr {
            continue;
        }

        let bitmap = page_obj.render(dpi)?;
        let width = bitmap.width() as u32;
        let height = bitmap.height() as u32;
        let rgba = bitmap.to_rgba();

        let img: ImageBuffer<Rgba<u8>, Vec<u8>> = ImageBuffer::from_raw(width, height, rgba)
            .ok_or(LiteParseError::Other(
                "failed to create image buffer".into(),
            ))?;
        let rgb_img = image::DynamicImage::ImageRgba8(img).to_rgb8();
        let rgb_bytes = rgb_img.into_raw();

        rendered.push(RenderedPage {
            idx,
            rgb_bytes,
            width,
            height,
        });
    }
    Ok(rendered)
}

/// Run OCR on pre-rendered page bitmaps and merge results into `pages`.
pub(crate) async fn ocr_and_merge_rendered(
    pages: &mut [Page],
    rendered: Vec<RenderedPage>,
    dpi: f32,
    ocr_engine: Arc<dyn OcrEngine>,
    ocr_language: &str,
    num_workers: usize,
    clean_artifacts: bool,
) -> Result<(), LiteParseError> {
    // Phase 1: spawn OCR tasks onto the tokio runtime so they run on
    // separate threads. A semaphore limits concurrency to `num_workers`.
    let num_workers = num_workers.max(1);
    let semaphore = Arc::new(tokio::sync::Semaphore::new(num_workers));
    let mut handles = Vec::with_capacity(rendered.len());

    let handle = tokio::runtime::Handle::current();

    for r in rendered {
        let engine = ocr_engine.clone();
        let sem = semaphore.clone();
        let language = ocr_language.to_string();
        let page_number = pages[r.idx].page_number;
        let rt_handle = handle.clone();

        handles.push((
            r.idx,
            page_number,
            tokio::task::spawn_blocking(move || {
                // Block this thread until a permit is available.
                let _permit = rt_handle
                    .block_on(sem.acquire_owned())
                    .expect("semaphore closed");
                let options = OcrOptions { language };
                rt_handle.block_on(engine.recognize(&r.rgb_bytes, r.width, r.height, &options))
            }),
        ));
    }

    // Phase 3: collect results and merge into pages.
    let scale_factor = 72.0 / dpi;

    // Track OCR task outcomes so we can distinguish a systemic failure (e.g.
    // missing Tesseract language data, which fails identically on every page)
    // from incidental per-page failures. Without this, every page logs the same
    // error and `parse()` still returns "success" with no OCR text.
    //
    // We additionally track whether any *sparse-text* page failed: a page is
    // rendered for OCR if it has sparse native text OR merely contains an image
    // (`needs_ocr = text_length < 20 || text_coverage < 0.15 || has_images`).
    // A native-text PDF with a logo on every page is rendered for OCR
    // enrichment but already has all its text. We must only fail loud when OCR
    // failure destroyed a sparse page's likely primary text source — otherwise
    // a broken OCR setup would abort perfectly good native-text documents.
    let total_tasks = handles.len();
    let mut failed_tasks = 0usize;
    let mut failed_sparse_text_page = false;
    let mut first_error: Option<String> = None;

    for (idx, page_number, handle) in handles {
        let ocr_results: Vec<OcrResult> = match handle.await {
            Ok(Ok(results)) => results,
            Ok(Err(e)) => {
                failed_tasks += 1;
                failed_sparse_text_page |= page_has_sparse_native_text(&pages[idx]);
                // Only log the first failure to avoid flooding stderr with an
                // identical message for every page.
                if first_error.is_none() {
                    let msg = e.to_string();
                    eprintln!("[ocr] failed for page {}: {}", page_number, msg);
                    first_error = Some(msg);
                }
                continue;
            }
            Err(e) => {
                failed_tasks += 1;
                failed_sparse_text_page |= page_has_sparse_native_text(&pages[idx]);
                if first_error.is_none() {
                    let msg = e.to_string();
                    eprintln!("[ocr] task panicked for page {}: {}", page_number, msg);
                    first_error = Some(msg);
                }
                continue;
            }
        };

        if ocr_results.is_empty() {
            continue;
        }

        let page = &mut pages[idx];
        // Drop garbled native items (e.g. substitution-cipher cmap corruption) so
        // OCR can replace them. Without this, garbled-but-spatially-present native
        // text suppresses every OCR result that overlaps it via the overlap check
        // below, leaving the output stuck with unreadable cipher text. We apply
        // both per-item and per-page checks: short garbled labels ("GDWH",
        // "XVG") can't be flagged alone, but their host page can.
        if page_is_garbled(page) {
            page.text_items.clear();
        } else {
            page.text_items
                .retain(|item| !is_likely_garbled(&item.text));
        }

        // Only check overlap against native (already-extracted) PDF text. Comparing
        // each OCR result against previously-accepted OCR results caused adjacent
        // OCR lines whose bounding boxes touched within tolerance to suppress each
        // other, dropping every second line on scanned pages.
        let native_count = page.text_items.len();
        for r in &ocr_results {
            if r.confidence <= 0.1 {
                continue;
            }

            let ocr_x = r.bbox[0] * scale_factor;
            let ocr_y = r.bbox[1] * scale_factor;
            let ocr_w = (r.bbox[2] - r.bbox[0]) * scale_factor;
            let ocr_h = (r.bbox[3] - r.bbox[1]) * scale_factor;

            if overlaps_existing_text(
                &page.text_items[..native_count],
                ocr_x,
                ocr_y,
                ocr_w,
                ocr_h,
                2.0,
            ) {
                continue;
            }

            let cleaned = if clean_artifacts {
                clean_ocr_table_artifacts(&r.text)
            } else {
                r.text.clone()
            };

            if cleaned.is_empty() {
                continue;
            }

            page.text_items.push(TextItem {
                text: cleaned,
                x: ocr_x,
                y: ocr_y,
                width: ocr_w,
                height: ocr_h,
                font_name: Some("OCR".to_string()),
                font_size: Some(ocr_h),
                confidence: Some((r.confidence * 1000.0).round() / 1000.0),
                ..Default::default()
            });
        }
    }

    // If every OCR task failed *and* at least one of those failures was on a
    // sparse-text page (the same length/coverage predicate that sends pages to
    // OCR as text-poor in `render_pages_for_ocr`), treat it as a systemic
    // failure. Returning an error surfaces the root cause (e.g. missing language
    // data) instead of silently emitting an empty or mostly-empty page. We
    // deliberately do NOT fail when the only failures were on pages that already
    // had substantial native text and were merely rendered for image-based OCR
    // enrichment — a broken OCR setup must not abort an otherwise-good
    // native-text document.
    if total_tasks > 0 && failed_tasks == total_tasks && failed_sparse_text_page {
        let detail = first_error.unwrap_or_else(|| "unknown error".to_string());
        return Err(LiteParseError::Ocr(format!(
            "OCR failed for all {} page(s): {}",
            total_tasks, detail
        )));
    }

    // Surface a concise summary for partial failures without flooding stderr.
    if failed_tasks > 0 {
        eprintln!(
            "[ocr] {}/{} page(s) failed OCR; continuing with partial results",
            failed_tasks, total_tasks
        );
    }

    Ok(())
}

/// True when the page's native (already-extracted) text is sparse enough that
/// OCR is likely its primary text source. Mirrors the non-image predicates in
/// `render_pages_for_ocr` (`text_length < 20 || text_coverage < 0.15`) so the
/// systemic-failure guard matches the same pages that were rendered because
/// their native text was insufficient.
fn page_has_sparse_native_text(page: &Page) -> bool {
    let text_length: usize = page
        .text_items
        .iter()
        .filter(|item| !is_likely_garbled(&item.text))
        .map(|item| item.text.len())
        .sum();
    let page_area = page.page_width * page.page_height;
    let text_bbox_area: f32 = page
        .text_items
        .iter()
        .filter(|item| !is_likely_garbled(&item.text))
        .map(|item| item.width * item.height)
        .sum();
    let text_coverage = if page_area > 0.0 {
        text_bbox_area / page_area
    } else {
        0.0
    };

    text_length < 20 || text_coverage < 0.15
}

/// Heuristic for substitution-cipher / broken-cmap garbling: real Latin-script
/// text has a vowel ratio of roughly 30–45%, but a substitution permutation
/// almost always maps the original A/E/I/O/U onto non-vowel letters, driving
/// the apparent vowel ratio to near zero. Texts without enough ASCII letters
/// to judge (non-Latin scripts, numbers, short labels) are treated as fine.
fn is_likely_garbled(text: &str) -> bool {
    let (letters, vowels) = count_letters_and_vowels(text);
    if letters < 10 {
        return false;
    }
    vowels * 10 < letters
}

fn count_letters_and_vowels(text: &str) -> (usize, usize) {
    let mut letters = 0usize;
    let mut vowels = 0usize;
    for ch in text.chars() {
        if ch.is_ascii_alphabetic() {
            letters += 1;
            if matches!(ch.to_ascii_lowercase(), 'a' | 'e' | 'i' | 'o' | 'u') {
                vowels += 1;
            }
        }
    }
    (letters, vowels)
}

/// Page-level garbled check: even when individual items are too short to judge
/// in isolation (e.g. "GDWH", "FXUUHQFB XVG"), a page whose aggregate vowel
/// ratio collapses to single digits is almost certainly substitution-encoded.
/// Used to drop all native items on the page before OCR merge, so short
/// garbled labels don't suppress overlapping OCR results.
fn page_is_garbled(page: &Page) -> bool {
    let mut total_letters = 0usize;
    let mut total_vowels = 0usize;
    for it in &page.text_items {
        let (l, v) = count_letters_and_vowels(&it.text);
        total_letters += l;
        total_vowels += v;
    }
    if total_letters < 30 {
        return false;
    }
    // Real Latin-script vowel ratios sit ~30–45% across English, Portuguese,
    // Spanish, French, etc. A page-wide ratio under 20% is well outside any
    // natural-language range and signals substitution-style corruption. (A
    // simple +3 Caesar shift still leaves some U/Y letters from the original
    // O/Y mapping, so a 10% bound is too tight to catch this in practice.)
    total_vowels * 5 < total_letters
}

/// Check if an OCR bounding box overlaps with any existing text item.
fn overlaps_existing_text(
    items: &[TextItem],
    ocr_x: f32,
    ocr_y: f32,
    ocr_w: f32,
    ocr_h: f32,
    tolerance: f32,
) -> bool {
    for item in items {
        let item_right = item.x + item.width;
        let item_bottom = item.y + item.height;

        let overlap_x = ocr_x < item_right + tolerance && ocr_x + ocr_w > item.x - tolerance;
        let overlap_y = ocr_y < item_bottom + tolerance && ocr_y + ocr_h > item.y - tolerance;

        if overlap_x && overlap_y {
            return true;
        }
    }
    false
}

/// Clean common OCR artifacts from table border misreads.
/// OCR often misreads vertical table border lines as bracket-like characters.
fn clean_ocr_table_artifacts(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    // Strip leading/trailing border artifact characters: | [ ] ( ) { }
    let without_artifacts: &str = trimmed
        .trim_start_matches(['|', '[', ']', '(', ')', '{', '}'])
        .trim_end_matches(['|', '[', ']', '(', ')', '{', '}'])
        .trim();

    if without_artifacts.is_empty() {
        return trimmed.to_string();
    }

    // Only use cleaned version if core content looks numeric-ish
    // This avoids incorrectly stripping brackets from content like "(note)"
    let is_numeric_ish = without_artifacts
        .chars()
        .all(|c| c.is_ascii_digit() || matches!(c, ',' | '.' | ' ' | '%' | '-' | '+' | '*' | '/'))
        || without_artifacts == "N/A"
        || without_artifacts == "Z"
        || without_artifacts == "-";

    if is_numeric_ish {
        without_artifacts.to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clean_ocr_table_artifacts() {
        assert_eq!(clean_ocr_table_artifacts("44520]"), "44520");
        assert_eq!(clean_ocr_table_artifacts("|123"), "123");
        assert_eq!(clean_ocr_table_artifacts("0.3|"), "0.3");
        assert_eq!(clean_ocr_table_artifacts("(note)"), "(note)");
        assert_eq!(clean_ocr_table_artifacts("|hello|"), "|hello|");
        assert_eq!(clean_ocr_table_artifacts("N/A"), "N/A");
        assert_eq!(clean_ocr_table_artifacts(""), "");
        assert_eq!(clean_ocr_table_artifacts("|||"), "|||");
    }

    fn make_item(x: f32, y: f32, w: f32, h: f32) -> TextItem {
        TextItem {
            text: "x".into(),
            x,
            y,
            width: w,
            height: h,
            ..Default::default()
        }
    }

    #[test]
    fn test_overlaps_existing_text_inside() {
        let items = vec![make_item(10.0, 10.0, 20.0, 5.0)];
        assert!(overlaps_existing_text(&items, 12.0, 11.0, 5.0, 2.0, 2.0));
    }

    #[test]
    fn test_overlaps_existing_text_disjoint() {
        let items = vec![make_item(10.0, 10.0, 20.0, 5.0)];
        assert!(!overlaps_existing_text(&items, 100.0, 100.0, 5.0, 5.0, 2.0));
    }

    #[test]
    fn test_overlaps_existing_text_tolerance() {
        let items = vec![make_item(10.0, 10.0, 20.0, 5.0)];
        // Just outside but within tolerance
        assert!(overlaps_existing_text(&items, 31.0, 10.0, 5.0, 5.0, 2.0));
        // Beyond tolerance
        assert!(!overlaps_existing_text(&items, 35.0, 10.0, 5.0, 5.0, 2.0));
    }

    #[test]
    fn test_overlaps_empty() {
        assert!(!overlaps_existing_text(&[], 0.0, 0.0, 1.0, 1.0, 0.0));
    }

    #[test]
    fn test_clean_ocr_keeps_whitespace_trimmed() {
        assert_eq!(clean_ocr_table_artifacts("   "), "");
        assert_eq!(clean_ocr_table_artifacts(" 123 "), "123");
    }

    // A mock OCR engine that always fails, simulating a systemic error such as
    // missing Tesseract language data (the root cause behind issue #253).
    struct FailingEngine;
    impl OcrEngine for FailingEngine {
        fn name(&self) -> &str {
            "failing"
        }
        fn recognize<'a, 'b: 'a, 'c: 'a>(
            &'a self,
            _image_data: &'c [u8],
            _width: u32,
            _height: u32,
            _options: &'b OcrOptions,
        ) -> std::pin::Pin<
            Box<
                dyn Future<
                        Output = Result<Vec<OcrResult>, Box<dyn std::error::Error + Send + Sync>>,
                    > + Send
                    + '_,
            >,
        > {
            Box::pin(async move { Err("Error opening data file tessdata/eng.traineddata".into()) })
        }
    }

    fn make_blank_page(page_number: usize) -> Page {
        Page {
            page_number,
            page_width: 100.0,
            page_height: 100.0,
            text_items: Vec::new(),
        }
    }

    fn make_rendered(idx: usize) -> RenderedPage {
        RenderedPage {
            idx,
            // 1x1 RGB pixel; the engine never inspects it.
            rgb_bytes: vec![0u8, 0u8, 0u8],
            width: 1,
            height: 1,
        }
    }

    // A page that already has substantial native text coverage, as would be the
    // case for a native-text PDF page that was only rendered for OCR because it
    // also contains an image.
    fn make_native_text_page(page_number: usize) -> Page {
        Page {
            page_number,
            page_width: 100.0,
            page_height: 100.0,
            text_items: vec![TextItem {
                text: "this page already has real native text content".into(),
                x: 0.0,
                y: 0.0,
                width: 50.0,
                height: 50.0,
                ..Default::default()
            }],
        }
    }

    // A page with >20 bytes of native text but very low page coverage. These
    // are still text-poor enough that `render_pages_for_ocr` sends them to OCR
    // (`text_coverage < 0.15`), so a systemic OCR failure should not be silently
    // swallowed.
    fn make_low_coverage_text_page(page_number: usize) -> Page {
        Page {
            page_number,
            page_width: 100.0,
            page_height: 100.0,
            text_items: vec![TextItem {
                text: "small native header that is not enough".into(),
                x: 0.0,
                y: 0.0,
                width: 10.0,
                height: 5.0,
                ..Default::default()
            }],
        }
    }

    // When every OCR task fails (e.g. missing language data), the function must
    // return an error instead of silently reporting success with no OCR text.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_all_pages_fail_returns_error() {
        let mut pages = vec![make_blank_page(1), make_blank_page(2)];
        let rendered = vec![make_rendered(0), make_rendered(1)];
        let engine: Arc<dyn OcrEngine> = Arc::new(FailingEngine);

        let result =
            ocr_and_merge_rendered(&mut pages, rendered, 72.0, engine, "eng", 2, true).await;

        let err = result.expect_err("expected systemic OCR failure to be surfaced");
        let msg = err.to_string();
        assert!(
            msg.contains("OCR failed for all 2 page(s)"),
            "unexpected error message: {msg}"
        );
        assert!(
            msg.contains("traineddata"),
            "error should carry the underlying cause: {msg}"
        );
    }

    // With no rendered pages there is nothing to OCR; this must remain a no-op
    // success rather than tripping the all-failed guard.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_no_rendered_pages_is_ok() {
        let mut pages = vec![make_blank_page(1)];
        let engine: Arc<dyn OcrEngine> = Arc::new(FailingEngine);

        let result =
            ocr_and_merge_rendered(&mut pages, Vec::new(), 72.0, engine, "eng", 2, true).await;

        assert!(result.is_ok(), "empty OCR set should succeed: {result:?}");
    }

    // Regression guard: when OCR fails but every failing page already had native
    // text (it was only rendered for image-based enrichment), a broken OCR setup
    // must NOT abort the parse — the native text is still valid output.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_native_text_pages_not_failed_on_ocr_error() {
        let mut pages = vec![make_native_text_page(1), make_native_text_page(2)];
        let rendered = vec![make_rendered(0), make_rendered(1)];
        let engine: Arc<dyn OcrEngine> = Arc::new(FailingEngine);

        let result =
            ocr_and_merge_rendered(&mut pages, rendered, 72.0, engine, "eng", 2, true).await;

        assert!(
            result.is_ok(),
            "OCR failure on already-native-text pages must not abort the parse: {result:?}"
        );
        // Native text is preserved untouched.
        assert_eq!(pages[0].text_items.len(), 1);
        assert_eq!(pages[1].text_items.len(), 1);
    }

    // When failures span both a sparse-text page and a native-text page, the
    // sparse-text page lost its likely primary text source, so we still fail
    // loud.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_mixed_failure_with_sparse_text_page_returns_error() {
        let mut pages = vec![make_native_text_page(1), make_blank_page(2)];
        let rendered = vec![make_rendered(0), make_rendered(1)];
        let engine: Arc<dyn OcrEngine> = Arc::new(FailingEngine);

        let result =
            ocr_and_merge_rendered(&mut pages, rendered, 72.0, engine, "eng", 2, true).await;

        let err = result.expect_err("a text-starved page losing all OCR must surface an error");
        assert!(
            err.to_string().contains("OCR failed for all 2 page(s)"),
            "unexpected error message: {err}"
        );
    }

    // Regression guard for the review finding: low-coverage pages are rendered
    // for OCR even when their native text length is >20 bytes. A systemic OCR
    // failure on such pages must still surface as an error.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_low_coverage_text_page_failure_returns_error() {
        let mut pages = vec![make_low_coverage_text_page(1)];
        let rendered = vec![make_rendered(0)];
        let engine: Arc<dyn OcrEngine> = Arc::new(FailingEngine);

        let result =
            ocr_and_merge_rendered(&mut pages, rendered, 72.0, engine, "eng", 2, true).await;

        let err = result.expect_err("low-coverage text page losing OCR must surface an error");
        assert!(
            err.to_string().contains("OCR failed for all 1 page(s)"),
            "unexpected error message: {err}"
        );
    }
}
