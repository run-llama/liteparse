use crate::types::ParsedPage;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub(crate) struct JsonImageItem {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub mime_type: String,
    pub base64: String,
}

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
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<JsonImageItem>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ParseResultJson {
    pub pages: Vec<JsonPage>,
}

/// Build structured JSON output from parsed pages.
pub(crate) fn build_json(pages: &[ParsedPage]) -> ParseResultJson {
    ParseResultJson {
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
                images: page
                    .images
                    .iter()
                    .map(|image| JsonImageItem {
                        x: image.x,
                        y: image.y,
                        width: image.width,
                        height: image.height,
                        mime_type: image.mime_type.clone(),
                        base64: image.base64.clone(),
                    })
                    .collect(),
            })
            .collect(),
    }
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
            text_items: items,
            images: vec![],
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
}
