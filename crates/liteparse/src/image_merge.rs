use base64::Engine as _;
use image::ImageEncoder;

use crate::error::LiteParseError;
use crate::types::{ImageItem, Page, TextItem};

const MIN_IMAGE_SIZE_PT: f32 = 25.0;
const MAX_PAGE_COVERAGE: f32 = 0.9;
const IMAGE_PLACEHOLDER_FONT: &str = "IMAGE";

/// Extract embedded PDF images into parsed pages.
pub(crate) fn extract_images_into_pages(
    document: &pdfium::Document,
    pages: &mut [Page],
) -> Result<(), LiteParseError> {
    for page in pages {
        let pdf_page = document.page((page.page_number - 1) as i32)?;
        let image_objects = pdf_page.image_objects(MIN_IMAGE_SIZE_PT, MAX_PAGE_COVERAGE);

        for (image_idx, info) in image_objects.into_iter().enumerate() {
            let bitmap = pdf_page.render_image_object(info.image_obj_index)?;
            if bitmap.width() <= 0 || bitmap.height() <= 0 {
                continue;
            }
            let width = bitmap.width() as u32;
            let height = bitmap.height() as u32;
            let rgba = bitmap.to_rgba();
            let png_bytes = encode_png(&rgba, width, height)?;
            let base64 = base64::engine::general_purpose::STANDARD.encode(png_bytes);
            let placeholder = image_placeholder(page.page_number, image_idx);

            page.images.push(ImageItem {
                x: info.bounds.x,
                y: info.bounds.y,
                width: info.bounds.width,
                height: info.bounds.height,
                mime_type: "image/png".to_string(),
                base64,
                placeholder: Some(placeholder.clone()),
            });
        }
    }

    Ok(())
}

/// Add short synthetic text placeholders so grid projection can place images
/// in page text. Call this after OCR merging so placeholders do not suppress
/// OCR text that overlaps image bounds.
pub(crate) fn add_inline_image_placeholders(pages: &mut [Page]) {
    for page in pages {
        for image in &page.images {
            let Some(placeholder) = &image.placeholder else {
                continue;
            };
            page.text_items.push(TextItem {
                text: placeholder.clone(),
                x: image.x,
                y: image.y,
                // Use a normal text-sized synthetic box so the placeholder
                // does not skew grid median character/line sizing.
                width: placeholder.chars().count() as f32 * 6.0,
                height: 12.0,
                font_name: Some(IMAGE_PLACEHOLDER_FONT.to_string()),
                font_size: Some(12.0),
                ..Default::default()
            });
        }
    }
}

pub(crate) fn is_inline_image_placeholder(item: &TextItem) -> bool {
    item.font_name.as_deref() == Some(IMAGE_PLACEHOLDER_FONT)
        && item.text.starts_with("[[LITEPARSE_IMAGE_")
        && item.text.ends_with("]]")
}

pub(crate) fn replace_image_placeholders(mut text: String, images: &[ImageItem]) -> String {
    for image in images {
        if let Some(placeholder) = &image.placeholder {
            let markdown = format!("![](data:{};base64,{})", image.mime_type, image.base64);
            text = text.replace(placeholder, &markdown);
        }
    }

    // Grid projection may indent the synthetic image placeholder based on its
    // x coordinate. Inline image data URI lines should not keep that leading
    // whitespace because consumers may treat it as preformatted markdown/code.
    text.lines()
        .map(|line| {
            let trimmed = line.trim_start();
            if trimmed.starts_with("![](data:image/") {
                trimmed
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn image_placeholder(page_number: usize, image_idx: usize) -> String {
    format!("[[LITEPARSE_IMAGE_{}_{}]]", page_number, image_idx)
}

fn encode_png(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>, LiteParseError> {
    let mut png_buf = Vec::new();
    let encoder = image::codecs::png::PngEncoder::new(&mut png_buf);
    encoder.write_image(rgba, width, height, image::ColorType::Rgba8.into())?;
    Ok(png_buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_replace_image_placeholders_uses_empty_alt_markdown() {
        let images = vec![ImageItem {
            mime_type: "image/png".into(),
            base64: "abc123".into(),
            placeholder: Some("[[LITEPARSE_IMAGE_1_0]]".into()),
            ..Default::default()
        }];

        let text =
            replace_image_placeholders("before [[LITEPARSE_IMAGE_1_0]] after".into(), &images);
        assert_eq!(text, "before ![](data:image/png;base64,abc123) after");
    }

    #[test]
    fn test_replace_image_placeholders_trims_leading_spaces_on_image_lines() {
        let images = vec![ImageItem {
            mime_type: "image/png".into(),
            base64: "abc123".into(),
            placeholder: Some("[[LITEPARSE_IMAGE_1_0]]".into()),
            ..Default::default()
        }];

        let text = replace_image_placeholders(
            "before\n    [[LITEPARSE_IMAGE_1_0]]\nafter".into(),
            &images,
        );
        assert_eq!(text, "before\n![](data:image/png;base64,abc123)\nafter");
    }
}
