use crate::error::LiteParseError;
use crate::extract::{encode_png, load_document_from_input};
use crate::types::PdfInput;
use pdfium::Library;
use serde::Serialize;

/// A single rendered page as PNG bytes.
#[derive(Debug, Clone)]
pub struct RenderedPage {
    pub page_num: u32,
    pub width: u32,
    pub height: u32,
    pub png_bytes: Vec<u8>,
}

/// Render selected pages from a PDF input to PNG bytes.
///
/// Acquires the process-global PDFium lock for the entire render. The lock
/// is held until this function returns — PNG encoding happens inside the
/// critical section, which is fine because it is pure CPU work with no
/// `.await` points.
pub fn render_pages_to_png(
    input: &PdfInput,
    page_numbers: Option<&[u32]>,
    dpi: f32,
    password: Option<&str>,
) -> Result<Vec<RenderedPage>, LiteParseError> {
    let lib = Library::init();
    let document = load_document_from_input(&lib, input, password)?;
    render_document_pages(&document, page_numbers, dpi)
}

fn render_document_pages(
    document: &pdfium::Document,
    page_numbers: Option<&[u32]>,
    dpi: f32,
) -> Result<Vec<RenderedPage>, LiteParseError> {
    let page_count = document.page_count() as u32;
    let pages: Vec<u32> = match page_numbers {
        Some(nums) => nums.to_vec(),
        None => (1..=page_count).collect(),
    };

    let mut results = Vec::with_capacity(pages.len());
    for page_num in pages {
        if page_num < 1 || page_num > page_count {
            return Err(LiteParseError::Other(format!(
                "page {page_num} out of range (document has {page_count} pages)"
            )));
        }

        let page = document.page((page_num - 1) as i32)?;
        let bitmap = page.render(dpi)?;
        let width = bitmap.width() as u32;
        let height = bitmap.height() as u32;
        let rgba = bitmap.to_rgba();
        let png_bytes = encode_png(&rgba, width, height)?;

        results.push(RenderedPage {
            page_num,
            width,
            height,
            png_bytes,
        });
    }

    Ok(results)
}

/// Render a single page to a PNG file.
pub fn screenshot(
    pdf_path: &str,
    page_num: u32,
    dpi: f32,
    output_path: &str,
    password: Option<&str>,
) -> Result<(), LiteParseError> {
    let input = PdfInput::Path(pdf_path.to_string());
    let pages = render_pages_to_png(&input, Some(&[page_num]), dpi, password)?;
    let page = pages
        .into_iter()
        .next()
        .ok_or_else(|| LiteParseError::Other("no page rendered".into()))?;

    std::fs::write(output_path, &page.png_bytes)?;

    eprintln!(
        "[rust-bin] rendered page {} at {dpi} DPI → {output_path} ({}×{})",
        page_num, page.width, page.height
    );

    Ok(())
}

#[derive(Debug, Serialize)]
struct ImageBoundsOutput {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

/// Default minimum side length (in points) for a raster image object to be
/// counted; smaller objects such as rule lines, bullets, and icons are ignored.
pub const DEFAULT_MIN_IMAGE_SIZE_PT: f32 = 25.0;

/// Default upper bound on a single image's page coverage; an object at or above
/// this fraction of the page is treated as a full-page background and ignored.
pub const DEFAULT_MAX_IMAGE_PAGE_COVERAGE: f32 = 0.9;

/// Raster-image coverage for one page.
#[derive(Debug, Clone, Serialize)]
pub struct PageImageCoverage {
    /// 1-based page number.
    pub page_num: u32,
    /// Number of raster image objects counted on the page (after filtering).
    pub image_block_count: usize,
    /// Combined area of those images over the page area, clamped to 1.0.
    pub image_coverage: f32,
    /// Area of the single largest image over the page area, clamped to 1.0.
    pub largest_image_coverage: f32,
}

/// Report how much of each page is covered by raster images.
///
/// This walks every page and aggregates the image-object bounds that
/// `pdfium::Page::image_bounds` already exposes, returning structured data
/// rather than the stdout JSON that `image_bounds` prints. It is useful for
/// telling scanned or image-only pages apart from born-digital text pages: a
/// page dominated by one large image with little extractable text is likely a
/// scan, whereas a text page with small inline figures is not.
///
/// `min_image_size_pt` and `max_image_page_coverage` control which objects are
/// counted; `DEFAULT_MIN_IMAGE_SIZE_PT` and `DEFAULT_MAX_IMAGE_PAGE_COVERAGE`
/// are the values the `image_bounds` CLI uses. The call is read-only and
/// returns one entry per page, in page order.
pub fn page_image_coverage(
    input: &PdfInput,
    min_image_size_pt: f32,
    max_image_page_coverage: f32,
    password: Option<&str>,
) -> Result<Vec<PageImageCoverage>, LiteParseError> {
    let lib = Library::init();
    let document = load_document_from_input(&lib, input, password)?;
    let page_count = document.page_count();

    let mut coverage = Vec::with_capacity(page_count.max(0) as usize);
    for page_index in 0..page_count {
        let page = document.page(page_index)?;
        let page_area = (page.width() * page.height()).max(1.0);
        let bounds = page.image_bounds(min_image_size_pt, max_image_page_coverage);

        let mut total_area = 0.0_f32;
        let mut largest_area = 0.0_f32;
        for b in &bounds {
            let area = b.width.max(0.0) * b.height.max(0.0);
            total_area += area;
            largest_area = largest_area.max(area);
        }

        coverage.push(PageImageCoverage {
            page_num: page_index as u32 + 1,
            image_block_count: bounds.len(),
            image_coverage: (total_area / page_area).min(1.0),
            largest_image_coverage: (largest_area / page_area).min(1.0),
        });
    }

    Ok(coverage)
}

/// Extract image bounding boxes and print as JSON to stdout.
pub fn image_bounds(pdf_path: &str, page_num: Option<u32>) -> Result<(), LiteParseError> {
    let lib = Library::init();
    let document = load_document_from_input(&lib, &PdfInput::Path(pdf_path.to_string()), None)?;
    let page_count = document.page_count();

    for page_index in 0..page_count {
        if let Some(target) = page_num
            && page_index as u32 + 1 != target
        {
            continue;
        }

        let page = document.page(page_index)?;
        let bounds = page.image_bounds(DEFAULT_MIN_IMAGE_SIZE_PT, DEFAULT_MAX_IMAGE_PAGE_COVERAGE);

        let output: Vec<ImageBoundsOutput> = bounds
            .iter()
            .map(|b| ImageBoundsOutput {
                x: b.x,
                y: b.y,
                width: b.width,
                height: b.height,
            })
            .collect();

        let json = serde_json::json!({
            "page_number": page_index + 1,
            "images": output,
        });
        println!("{}", serde_json::to_string(&json)?);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_image_bounds_output_serializes() {
        let b = ImageBoundsOutput {
            x: 1.0,
            y: 2.0,
            width: 3.0,
            height: 4.0,
        };
        let s = serde_json::to_string(&b).unwrap();
        assert!(s.contains("\"x\":1"));
        assert!(s.contains("\"width\":3"));
    }

    #[test]
    fn test_screenshot_missing_file_errors() {
        let r = screenshot(
            "/nonexistent/path/does_not_exist.pdf",
            1,
            72.0,
            "/tmp/out.png",
            None,
        );
        assert!(r.is_err());
    }

    #[test]
    fn test_image_bounds_missing_file_errors() {
        let r = image_bounds("/nonexistent/path/does_not_exist.pdf", None);
        assert!(r.is_err());
    }

    #[test]
    fn test_page_image_coverage_serializes() {
        let c = PageImageCoverage {
            page_num: 1,
            image_block_count: 2,
            image_coverage: 0.5,
            largest_image_coverage: 0.4,
        };
        let s = serde_json::to_string(&c).unwrap();
        assert!(s.contains("\"page_num\":1"));
        assert!(s.contains("\"image_block_count\":2"));
    }

    #[test]
    fn test_page_image_coverage_missing_file_errors() {
        let r = page_image_coverage(
            &PdfInput::Path("/nonexistent/path/does_not_exist.pdf".to_string()),
            DEFAULT_MIN_IMAGE_SIZE_PT,
            DEFAULT_MAX_IMAGE_PAGE_COVERAGE,
            None,
        );
        assert!(r.is_err());
    }
}
