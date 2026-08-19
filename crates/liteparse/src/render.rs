use crate::config::{PngCompression, ScreenshotFormat, ScreenshotOptions};
use crate::error::LiteParseError;
use crate::extract::{encode_png_with_compression, load_document_from_input};
use crate::types::{PdfInput, ScreenshotRect};
use pdfium::Library;
use serde::Serialize;

/// A single rendered page image, plus raster-derived signals.
#[derive(Debug, Clone)]
pub struct RenderedPage {
    pub page_num: u32,
    pub width: u32,
    pub height: u32,
    pub image_bytes: Vec<u8>,
    pub format: ScreenshotFormat,
    /// Bytes per row for raw formats; absent for encoded images.
    pub stride: Option<u32>,
    /// True when every pixel has the same color (blank page after render).
    pub is_solid_fill: bool,
    /// Solid rectangles/lines detected in the raster (viewport coords).
    /// Empty unless rect detection was requested; also empty for
    /// solid-fill pages, where detection is skipped.
    pub rects: Vec<ScreenshotRect>,
}

struct RenderedScreenshot {
    image_bytes: Vec<u8>,
    stride: Option<u32>,
    is_solid_fill: bool,
    rects: Vec<ScreenshotRect>,
}

/// Render selected pages from a PDF input to PNG bytes.
///
/// With `render_form_fields`, form-field appearances (filled values, checkbox
/// states) are drawn on top of the page raster; this runs the document's
/// open/JS actions, so it is opt-in. With `detect_rects`, each page's raster
/// is also scanned for solid rectangles and lines.
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
    detect_rects: bool,
    render_form_fields: bool,
) -> Result<Vec<RenderedPage>, LiteParseError> {
    let lib = Library::init();
    let document = load_document_from_input(&lib, input, password)?;
    render_document_pages(
        &document,
        page_numbers,
        dpi,
        detect_rects,
        render_form_fields,
        false,
        &ScreenshotOptions::default(),
    )
}

/// Render selected pages using the requested output format.
pub fn render_pages(
    input: &PdfInput,
    page_numbers: Option<&[u32]>,
    dpi: f32,
    password: Option<&str>,
    detect_rects: bool,
    render_form_fields: bool,
    options: &ScreenshotOptions,
) -> Result<Vec<RenderedPage>, LiteParseError> {
    let lib = Library::init();
    let document = load_document_from_input(&lib, input, password)?;
    render_document_pages(
        &document,
        page_numbers,
        dpi,
        detect_rects,
        render_form_fields,
        false,
        options,
    )
}

pub(crate) fn render_document_pages(
    document: &pdfium::Document,
    page_numbers: Option<&[u32]>,
    dpi: f32,
    detect_rects: bool,
    render_form_fields: bool,
    continue_on_page_error: bool,
    options: &ScreenshotOptions,
) -> Result<Vec<RenderedPage>, LiteParseError> {
    let page_count = document.page_count() as u32;
    let pages: Vec<u32> = match page_numbers {
        Some(nums) => nums.to_vec(),
        None => (1..=page_count).collect(),
    };

    let form = render_form_fields
        .then(|| document.form_environment())
        .flatten();
    if let Some(form) = form.as_ref() {
        form.run_document_actions();
    }

    let mut results = Vec::with_capacity(pages.len());
    for page_num in pages {
        let page_render = (|| -> Result<RenderedPage, LiteParseError> {
            if page_num < 1 || page_num > page_count {
                return Err(LiteParseError::Other(format!(
                    "page {page_num} out of range (document has {page_count} pages)"
                )));
            }

            let page = document.page((page_num - 1) as i32)?;
            let bitmap = page.render_with_form(dpi, form.as_ref())?;
            let width = bitmap.width() as u32;
            let height = bitmap.height() as u32;
            let format = options.format;
            let screenshot = match format {
                ScreenshotFormat::Png => {
                    render_png_screenshot(&bitmap, &page, detect_rects, options.png.compression)
                }
                ScreenshotFormat::Rgb8 => render_rgb8_screenshot(&bitmap, &page, detect_rects),
            }?;

            Ok(RenderedPage {
                page_num,
                width,
                height,
                image_bytes: screenshot.image_bytes,
                format,
                stride: screenshot.stride,
                is_solid_fill: screenshot.is_solid_fill,
                rects: screenshot.rects,
            })
        })();

        match page_render {
            Ok(rendered) => results.push(rendered),
            // The page's text is already extracted; a tolerant parse keeps
            // it and just forgoes this page's screenshot.
            Err(error) if continue_on_page_error => eprintln!(
                "[render] page {page_num} failed: {error} — skipping its screenshot (continue_on_page_error)"
            ),
            Err(error) => return Err(error),
        }
    }

    Ok(results)
}

fn render_png_screenshot(
    bitmap: &pdfium::Bitmap<'_>,
    page: &pdfium::Page<'_, '_>,
    detect_rects: bool,
    compression: PngCompression,
) -> Result<RenderedScreenshot, LiteParseError> {
    let width = bitmap.width() as u32;
    let height = bitmap.height() as u32;
    let rgba = bitmap.to_rgba();
    let is_solid_fill = is_solid_fill_rgba(&rgba, width as usize, height as usize);
    let rects = screenshot_rects(
        &rgba,
        width,
        height,
        page.width(),
        page.height(),
        detect_rects,
        is_solid_fill,
    );

    Ok(RenderedScreenshot {
        image_bytes: encode_png_with_compression(&rgba, width, height, compression)?,
        stride: None,
        is_solid_fill,
        rects,
    })
}

fn render_rgb8_screenshot(
    bitmap: &pdfium::Bitmap<'_>,
    page: &pdfium::Page<'_, '_>,
    detect_rects: bool,
) -> Result<RenderedScreenshot, LiteParseError> {
    let width = bitmap.width() as u32;
    let height = bitmap.height() as u32;
    let (rgb, is_solid_fill, rects) = if detect_rects {
        let rgba = bitmap.to_rgba();
        let is_solid_fill = is_solid_fill_rgba(&rgba, width as usize, height as usize);
        let rects = screenshot_rects(
            &rgba,
            width,
            height,
            page.width(),
            page.height(),
            true,
            is_solid_fill,
        );
        (rgba_to_rgb(&rgba), is_solid_fill, rects)
    } else {
        let rgb = bitmap.to_rgb();
        let is_solid_fill = is_solid_fill_rgb(&rgb, width as usize, height as usize);
        (rgb, is_solid_fill, Vec::new())
    };

    let stride = width
        .checked_mul(3)
        .ok_or_else(|| LiteParseError::Other("RGB screenshot stride overflow".into()))?;
    let expected_len = (stride as usize)
        .checked_mul(height as usize)
        .ok_or_else(|| LiteParseError::Other("RGB screenshot size overflow".into()))?;
    if rgb.len() != expected_len {
        return Err(LiteParseError::Other(format!(
            "invalid RGB screenshot length: expected {expected_len}, got {}",
            rgb.len()
        )));
    }

    Ok(RenderedScreenshot {
        image_bytes: rgb,
        stride: Some(stride),
        is_solid_fill,
        rects,
    })
}

fn rgba_to_rgb(rgba: &[u8]) -> Vec<u8> {
    rgba.chunks_exact(4)
        .flat_map(|pixel| [pixel[0], pixel[1], pixel[2]])
        .collect()
}

fn is_solid_fill_rgb(rgb: &[u8], width: usize, height: usize) -> bool {
    if width == 0 || height == 0 || rgb.len() < width * height * 3 {
        return false;
    }
    let first = &rgb[..3];
    rgb[3..].chunks_exact(3).all(|pixel| pixel == first)
}

fn screenshot_rects(
    rgba: &[u8],
    width: u32,
    height: u32,
    page_width: f32,
    page_height: f32,
    detect_rects: bool,
    is_solid_fill: bool,
) -> Vec<ScreenshotRect> {
    if !detect_rects || is_solid_fill {
        return Vec::new();
    }
    find_solid_rects_rgba(
        rgba,
        width as usize,
        height as usize,
        page_width,
        page_height,
    )
}

/// Minimum rect dimension as a fraction of page height: 0.5%. Small renders
/// (<400px tall) fall back to a 2px floor.
const FIND_RECTS_MIN_DIMENSION_DENOM: usize = 200;

#[inline]
fn rgb_at(rgba: &[u8], pixel_index: usize) -> u32 {
    let base = pixel_index * 4;
    ((rgba[base] as u32) << 16) | ((rgba[base + 1] as u32) << 8) | (rgba[base + 2] as u32)
}

/// True when every pixel matches the first pixel's RGB (alpha ignored; the
/// render always starts from an opaque white fill).
pub(crate) fn is_solid_fill_rgba(rgba: &[u8], width: usize, height: usize) -> bool {
    if width == 0 || height == 0 || rgba.len() < width * height * 4 {
        return false;
    }
    let first = rgb_at(rgba, 0);
    (1..width * height).all(|i| rgb_at(rgba, i) == first)
}

/// Find solid same-color rectangles and lines in a rendered page bitmap.
/// Port of the LlamaParse extract binary's `ParseImage_findSolidRects`
/// (rectangles + lines modes; whitespace mode is not ported, so white areas
/// are skipped). Returned coordinates are scaled from pixels to the page's
/// viewport space (`page_width`/`page_height` in PDF points).
pub(crate) fn find_solid_rects_rgba(
    rgba: &[u8],
    width: usize,
    height: usize,
    page_width: f32,
    page_height: f32,
) -> Vec<ScreenshotRect> {
    let mut out = Vec::new();
    if width == 0 || height == 0 || rgba.len() < width * height * 4 {
        return out;
    }

    let mut visited = vec![false; width * height];
    // For small renders (<200px edge) count a rectangle as at least 2x2
    // pixels, a line as 1x2.
    let min_dimension = (height / FIND_RECTS_MIN_DIMENSION_DENOM).max(2);

    for y in 0..height {
        for x in 0..width {
            if visited[y * width + x] {
                continue;
            }
            let color = rgb_at(rgba, y * width + x);
            visited[y * width + x] = true;

            // White pixels are page background, not content.
            if color == 0xffffff {
                continue;
            }

            let mut min_dimension_matches = 0;
            if x + min_dimension - 1 < width
                && rgb_at(rgba, y * width + x + min_dimension - 1) == color
            {
                min_dimension_matches += 1;
            }
            if y + min_dimension - 1 < height
                && rgb_at(rgba, (y + min_dimension - 1) * width + x) == color
            {
                min_dimension_matches += 1;
            }
            if min_dimension_matches < 1 {
                continue;
            }

            // Grow the same-color region down and right; the rect width is
            // the narrowest row so partial rows don't inflate it.
            let mut rect_width: Option<usize> = None;
            let mut yy = y;
            while yy < height {
                if yy != y && (visited[yy * width + x] || rgb_at(rgba, yy * width + x) != color) {
                    break;
                }
                let mut xx = x + 1;
                while xx < width
                    && !visited[yy * width + xx]
                    && rgb_at(rgba, yy * width + xx) == color
                {
                    xx += 1;
                }
                let row_width = xx - x;
                rect_width = Some(rect_width.map_or(row_width, |w| w.min(row_width)));
                yy += 1;
            }
            let rect_height = yy - y;
            let rect_width = rect_width.unwrap_or(0);

            for vy in y..y + rect_height {
                for vx in x..x + rect_width {
                    visited[vy * width + vx] = true;
                }
            }

            let (found, is_line) = if min_dimension_matches == 1 {
                (
                    rect_height >= min_dimension || rect_width >= min_dimension,
                    true,
                )
            } else {
                (
                    rect_height >= min_dimension && rect_width >= min_dimension,
                    false,
                )
            };
            if found {
                out.push(ScreenshotRect {
                    x: x as f32 / width as f32 * page_width,
                    y: y as f32 / height as f32 * page_height,
                    width: rect_width as f32 / width as f32 * page_width,
                    height: rect_height as f32 / height as f32 * page_height,
                    color: format!("ff{:06x}", color),
                    is_line,
                });
            }
        }
    }

    out
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
    let pages = render_pages_to_png(&input, Some(&[page_num]), dpi, password, false, false)?;
    let page = pages
        .into_iter()
        .next()
        .ok_or_else(|| LiteParseError::Other("no page rendered".into()))?;

    std::fs::write(output_path, &page.image_bytes)?;

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
        let bounds = page.image_bounds(25.0, 0.9);

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

    /// Build an RGBA buffer from rows of 24-bit RGB pixel values.
    fn rgba_from_rows(rows: &[&[u32]]) -> (Vec<u8>, usize, usize) {
        let height = rows.len();
        let width = rows[0].len();
        let mut buf = Vec::with_capacity(width * height * 4);
        for row in rows {
            for &px in *row {
                buf.push((px >> 16) as u8);
                buf.push((px >> 8) as u8);
                buf.push(px as u8);
                buf.push(0xff);
            }
        }
        (buf, width, height)
    }

    #[test]
    fn solid_fill_detects_uniform_and_rejects_mixed() {
        let uniform_rows = [[0xffffffu32; 4]; 4];
        let uniform_refs: Vec<&[u32]> = uniform_rows.iter().map(|r| &r[..]).collect();
        let (uniform, w, h) = rgba_from_rows(&uniform_refs);
        assert!(is_solid_fill_rgba(&uniform, w, h));

        let mut rows = [[0x336699u32; 4]; 4];
        rows[3][3] = 0x336698;
        let row_refs: Vec<&[u32]> = rows.iter().map(|r| &r[..]).collect();
        let (mixed, w, h) = rgba_from_rows(&row_refs);
        assert!(!is_solid_fill_rgba(&mixed, w, h));
    }

    #[test]
    fn find_solid_rects_finds_colored_block_and_skips_white() {
        // 8x8 white page with a 4x4 colored block at (2,2). min_dimension
        // floors at 2, so the block qualifies as a rectangle.
        const W: u32 = 0xffffff;
        const C: u32 = 0x112233;
        let mut rows = [[W; 8]; 8];
        for row in rows.iter_mut().skip(2).take(4) {
            for px in row.iter_mut().skip(2).take(4) {
                *px = C;
            }
        }
        let row_refs: Vec<&[u32]> = rows.iter().map(|r| &r[..]).collect();
        let (buf, w, h) = rgba_from_rows(&row_refs);

        // Page is 80x80pt, so pixel coords scale 10x.
        let rects = find_solid_rects_rgba(&buf, w, h, 80.0, 80.0);
        assert_eq!(rects.len(), 1);
        let rect = &rects[0];
        assert_eq!(rect.x, 20.0);
        assert_eq!(rect.y, 20.0);
        assert_eq!(rect.width, 40.0);
        assert_eq!(rect.height, 40.0);
        assert_eq!(rect.color, "ff112233");
        assert!(!rect.is_line);
    }

    #[test]
    fn find_solid_rects_flags_thin_line_as_line() {
        // 8x8 white page with a 1px-tall, 6px-wide dark line: only the
        // horizontal min-dimension probe matches, so it is a line.
        const W: u32 = 0xffffff;
        const C: u32 = 0x000000;
        let mut rows = [[W; 8]; 8];
        for px in rows[4].iter_mut().skip(1).take(6) {
            *px = C;
        }
        let row_refs: Vec<&[u32]> = rows.iter().map(|r| &r[..]).collect();
        let (buf, w, h) = rgba_from_rows(&row_refs);

        let rects = find_solid_rects_rgba(&buf, w, h, 8.0, 8.0);
        assert_eq!(rects.len(), 1);
        assert!(rects[0].is_line);
        assert_eq!(rects[0].color, "ff000000");
        assert_eq!(rects[0].width, 6.0);
        assert_eq!(rects[0].height, 1.0);
    }
}
