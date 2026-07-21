use crate::types::{ExtractedImage, ParsedPage, Rect};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub(crate) struct JsonTextItem {
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_size: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
}

#[derive(Debug, Serialize)]
pub(crate) struct JsonPage {
    pub page: usize,
    pub width: f32,
    pub height: f32,
    pub text: String,
    pub text_items: Vec<JsonTextItem>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ParseResultJson {
    pub pages: Vec<JsonPage>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<JsonImage>,
    #[serde(skip_serializing_if = "is_zero")]
    pub image_error_count: u32,
}

fn is_zero(value: &u32) -> bool {
    *value == 0
}

#[derive(Debug, Serialize)]
pub(crate) struct JsonImage {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub page: u32,
    pub bbox: Rect,
    pub width: u32,
    pub height: u32,
    pub rotation: f32,
    pub format: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duplicate_of: Option<String>,
}

/// Build structured JSON output from parsed pages.
pub(crate) fn build_json(pages: &[ParsedPage]) -> ParseResultJson {
    ParseResultJson {
        images: Vec::new(),
        image_error_count: 0,
        pages: pages
            .iter()
            .map(|page| JsonPage {
                page: page.page_number,
                width: page.page_width,
                height: page.page_height,
                text: page.text.clone(),
                text_items: page
                    .text_items
                    .iter()
                    .map(|item| JsonTextItem {
                        text: item.text.clone(),
                        x: item.x,
                        y: item.y,
                        width: item.width,
                        height: item.height,
                        font_name: item.font_name.clone(),
                        font_size: item.font_size,
                        confidence: item.confidence.or(Some(1.0)),
                    })
                    .collect(),
            })
            .collect(),
    }
}

/// Format complete parse output, including extracted-image metadata. Pixel
/// bytes are written separately by the CLI's `--image-output-dir` option.
pub fn format_json_result(
    pages: &[ParsedPage],
    images: &[ExtractedImage],
    image_error_count: u32,
) -> Result<String, serde_json::Error> {
    let mut result = build_json(pages);
    result.images = images
        .iter()
        .map(|image| JsonImage {
            id: image.id.clone(),
            name: image.name.clone(),
            path: image.path.clone(),
            page: image.page,
            bbox: image.bbox.clone(),
            width: image.width,
            height: image.height,
            rotation: image.rotation,
            format: image.format.clone(),
            duplicate_of: image.duplicate_of.clone(),
        })
        .collect();
    result.image_error_count = image_error_count;
    serde_json::to_string_pretty(&result)
}

/// Format parsed pages as pretty-printed JSON string.
pub fn format_json(pages: &[ParsedPage]) -> Result<String, serde_json::Error> {
    let result = build_json(pages);
    serde_json::to_string_pretty(&result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ParsedPage, TextItem};

    fn item(text: &str, conf: Option<f32>) -> TextItem {
        TextItem {
            text: text.into(),
            x: 1.0,
            y: 2.0,
            width: 3.0,
            height: 4.0,
            font_name: Some("Helv".into()),
            font_size: Some(10.0),
            confidence: conf,
            ..Default::default()
        }
    }

    fn page(items: Vec<TextItem>) -> ParsedPage {
        ParsedPage {
            page_number: 1,
            page_width: 612.0,
            page_height: 792.0,
            text: "txt".into(),
            markdown: String::new(),
            text_items: items,
            projected_lines: vec![],
            regions: crate::types::Region::default(),
            graphics: vec![],
            figures: vec![],
            struct_nodes: vec![],
            image_refs: vec![],
        }
    }

    #[test]
    fn test_build_json_native_text_defaults_confidence_to_one() {
        let j = build_json(&[page(vec![item("hi", None)])]);
        assert_eq!(j.pages.len(), 1);
        assert_eq!(j.pages[0].page, 1);
        assert_eq!(j.pages[0].text_items[0].confidence, Some(1.0));
        assert_eq!(j.pages[0].text_items[0].font_name.as_deref(), Some("Helv"));
    }

    #[test]
    fn test_build_json_preserves_ocr_confidence() {
        let j = build_json(&[page(vec![item("hi", Some(0.42))])]);
        assert_eq!(j.pages[0].text_items[0].confidence, Some(0.42));
    }

    #[test]
    fn test_format_json_pretty() {
        let s = format_json(&[page(vec![item("hi", None)])]).unwrap();
        assert!(s.contains("\n"));
        assert!(s.contains("\"text\": \"hi\""));
        assert!(s.contains("\"page\": 1"));
    }

    #[test]
    fn test_build_json_empty() {
        let j = build_json(&[]);
        assert!(j.pages.is_empty());
    }

    #[test]
    fn test_format_json_result_includes_image_metadata_and_errors() {
        let image = ExtractedImage {
            id: "p2_0".into(),
            name: "image_p2_0.jpg".into(),
            path: Some("/tmp/images/image_p2_0.jpg".into()),
            page: 2,
            bbox: Rect {
                x: 10.0,
                y: 20.0,
                width: 30.0,
                height: 40.0,
            },
            width: 640,
            height: 480,
            rotation: 90.0,
            format: "jpg".into(),
            duplicate_of: Some("p1_0".into()),
            bytes: vec![1, 2, 3],
        };
        let value: serde_json::Value =
            serde_json::from_str(&format_json_result(&[], &[image], 2).unwrap()).unwrap();
        assert_eq!(value["images"][0]["bbox"]["x"], 10.0);
        assert_eq!(value["images"][0]["width"], 640);
        assert_eq!(value["images"][0]["rotation"], 90.0);
        assert_eq!(value["images"][0]["duplicate_of"], "p1_0");
        assert!(value["images"][0].get("bytes").is_none());
        assert_eq!(value["image_error_count"], 2);
    }
}
