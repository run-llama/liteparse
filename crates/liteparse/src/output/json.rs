use crate::types::ParsedPage;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub(crate) struct JsonTextItem {
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rotation: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_size: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_height: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_ascent: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_descent: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_weight: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_width: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_is_buggy: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mcid: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fill_color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stroke_color: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub char_codes: Vec<u32>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub tsg: bool,
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
}

/// Build structured JSON output from parsed pages.
pub(crate) fn build_json(pages: &[ParsedPage], include_text_metadata: bool) -> ParseResultJson {
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
                        rotation: include_text_metadata.then_some(item.rotation),
                        font_name: item.font_name.clone(),
                        font_size: item.font_size,
                        font_height: include_text_metadata.then_some(item.font_height).flatten(),
                        font_ascent: include_text_metadata.then_some(item.font_ascent).flatten(),
                        font_descent: include_text_metadata.then_some(item.font_descent).flatten(),
                        font_weight: include_text_metadata.then_some(item.font_weight).flatten(),
                        text_width: include_text_metadata.then_some(item.text_width).flatten(),
                        font_is_buggy: include_text_metadata.then_some(item.font_is_buggy),
                        mcid: include_text_metadata.then_some(item.mcid).flatten(),
                        fill_color: include_text_metadata
                            .then(|| item.fill_color.clone())
                            .flatten(),
                        stroke_color: include_text_metadata
                            .then(|| item.stroke_color.clone())
                            .flatten(),
                        char_codes: if include_text_metadata {
                            item.char_codes.clone()
                        } else {
                            Vec::new()
                        },
                        tsg: include_text_metadata && item.tsg,
                        confidence: item.confidence.or(Some(1.0)),
                    })
                    .collect(),
            })
            .collect(),
    }
}

/// Format parsed pages as pretty-printed JSON string.
pub fn format_json(pages: &[ParsedPage]) -> Result<String, serde_json::Error> {
    format_json_with_text_metadata(pages, false)
}

/// Format parsed pages as JSON, optionally including rich PDF text metadata.
pub fn format_json_with_text_metadata(
    pages: &[ParsedPage],
    include_text_metadata: bool,
) -> Result<String, serde_json::Error> {
    let result = build_json(pages, include_text_metadata);
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
        let j = build_json(&[page(vec![item("hi", None)])], false);
        assert_eq!(j.pages.len(), 1);
        assert_eq!(j.pages[0].page, 1);
        assert_eq!(j.pages[0].text_items[0].confidence, Some(1.0));
        assert_eq!(j.pages[0].text_items[0].font_name.as_deref(), Some("Helv"));
    }

    #[test]
    fn test_build_json_preserves_ocr_confidence() {
        let j = build_json(&[page(vec![item("hi", Some(0.42))])], false);
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
    fn test_build_json_preserves_text_metadata() {
        let mut text_item = item("hi", None);
        text_item.font_height = Some(11.0);
        text_item.font_ascent = Some(8.0);
        text_item.font_descent = Some(-2.0);
        text_item.font_weight = Some(700);
        text_item.text_width = Some(9.5);
        text_item.font_is_buggy = true;
        text_item.mcid = Some(4);
        text_item.fill_color = Some("ff112233".into());
        text_item.stroke_color = Some("ff445566".into());
        text_item.char_codes = vec![104, 105, 32];
        text_item.tsg = true;

        let value: serde_json::Value = serde_json::from_str(
            &format_json_with_text_metadata(&[page(vec![text_item])], true).unwrap(),
        )
        .unwrap();
        let item = &value["pages"][0]["text_items"][0];
        assert_eq!(item["font_height"], 11.0);
        assert_eq!(item["font_ascent"], 8.0);
        assert_eq!(item["font_descent"], -2.0);
        assert_eq!(item["font_weight"], 700);
        assert_eq!(item["text_width"], 9.5);
        assert_eq!(item["font_is_buggy"], true);
        assert_eq!(item["mcid"], 4);
        assert_eq!(item["fill_color"], "ff112233");
        assert_eq!(item["stroke_color"], "ff445566");
        assert_eq!(item["char_codes"], serde_json::json!([104, 105, 32]));
        assert_eq!(item["tsg"], true);
        assert_eq!(item["rotation"], 0.0);
    }

    #[test]
    fn test_build_json_empty() {
        let j = build_json(&[], false);
        assert!(j.pages.is_empty());
    }

    #[test]
    fn test_text_metadata_is_omitted_by_default() {
        let mut text_item = item("hi", None);
        text_item.font_height = Some(11.0);
        text_item.font_is_buggy = true;
        text_item.mcid = Some(4);
        text_item.char_codes = vec![104, 105];
        text_item.tsg = true;

        let value: serde_json::Value =
            serde_json::from_str(&format_json(&[page(vec![text_item])]).unwrap()).unwrap();
        let item = &value["pages"][0]["text_items"][0];
        assert!(item.get("rotation").is_none());
        assert!(item.get("font_height").is_none());
        assert!(item.get("font_is_buggy").is_none());
        assert!(item.get("mcid").is_none());
        assert!(item.get("char_codes").is_none());
        assert!(item.get("tsg").is_none());
    }
}
